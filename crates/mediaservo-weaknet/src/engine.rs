//! engine.rs —— TcEngine：通道探测（local-root / docker NET_ADMIN sidecar）+ 骨架 argv 全链
//! （逐字对齐 bash build_skeleton，add-or-change 2015 兼容）+ spec 区分性回读指纹 +
//! leaf Sent 双采样 verify / counter-reset 探测 + fail-closed guard + teardown 计划/执行。
//!
//! 真值源：scripts/weaknet.sh（机制）+ design.md §dir 表（腿）+ §ifb 合同（T9：物理口
//! 上行走 ifb0 镜像 ingress，链路步骤见 plan_ifb；能力判定单点在 ifb.rs）。
//!
//! 退出码沿用：2 环境不足/施加失败 · 3 状态冲突（他方 qdisc/锁）· 4 参数非法（C15 全分支报因）。

use std::process::Command;
use std::time::Duration;

use crate::fuse;
use crate::ifb;
use crate::spec::{
    self, Dir, IfaceKind, ImpairSpec, LegFilter, MatchKind, ScopeSel, build_filter_legs,
    render_netem_spec_leg, render_rate_arg,
};
use crate::state::{self, ChannelSer, Dirs, SidecarRef, State, Teardown};

/// 统一失败载荷（code=bash 退出码语义）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fail {
    pub code: u32,
    pub msg: String,
}

impl Fail {
    #[must_use]
    pub fn env(msg: impl Into<String>) -> Self {
        Self { code: 2, msg: msg.into() }
    }
    #[must_use]
    pub fn conflict(msg: impl Into<String>) -> Self {
        Self { code: 3, msg: msg.into() }
    }
    #[must_use]
    pub fn bad_param(msg: impl Into<String>) -> Self {
        Self { code: 4, msg: msg.into() }
    }
}

impl std::fmt::Display for Fail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.msg)
    }
}

impl From<String> for Fail {
    fn from(msg: String) -> Self {
        Self::env(msg)
    }
}

pub type Wn<T> = Result<T, Fail>;

// ---------- 运行环境（env 读取集中一次，测试可手构） ----------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Env {
    /// WEAKNET_CHANNEL = auto|local|sidecar（design §engine 自有词注记）
    pub channel_pref: String,
    /// sidecar 容器名（bash SIDE=weaknet-ctrl 缺省；C20=文档化常量+env 覆写形）
    pub sidecar: String,
    /// iproute2 特权镜像（gaiadocker/iproute2——2015 语义版，坑见 bash 头注）
    pub image: String,
}

pub const DEF_SIDECAR: &str = "weaknet-ctrl";
pub const DEF_IMAGE: &str = "gaiadocker/iproute2";
/// bash `DEV="${WEAKNET_DEV:-lo}"` 同构缺省（C20 豁免形=文档化常量 + env/flag 覆写）。
pub const DEFAULT_IFACE: &str = "lo";

/// iface 统一解析链（CLI 与 serve 共用；T6 自 main.rs 上提，单一真值源）。
#[must_use]
pub fn resolve_iface(cli: Option<&str>) -> String {
    cli.map(str::to_owned)
        .or_else(|| std::env::var("WEAKNET_DEV").ok().filter(|s| !s.is_empty()))
        .unwrap_or_else(|| DEFAULT_IFACE.to_string())
}

impl Default for Env {
    fn default() -> Self {
        Self {
            channel_pref: "auto".into(),
            sidecar: DEF_SIDECAR.into(),
            image: DEF_IMAGE.into(),
        }
    }
}

impl Env {
    #[must_use]
    pub fn from_env() -> Self {
        let var = |k: &str, d: &str| std::env::var(k).unwrap_or_else(|_| d.to_string());
        Self {
            channel_pref: var("WEAKNET_CHANNEL", "auto").to_lowercase(),
            sidecar: var("WEAKNET_SIDECAR", DEF_SIDECAR),
            image: var("WEAKNET_IMAGE", DEF_IMAGE),
        }
    }
}

// ---------- 进程执行 ----------

fn run(argv: &[&str]) -> Wn<String> {
    let out = Command::new(argv[0])
        .args(&argv[1..])
        .output()
        .map_err(|e| Fail::env(format!("启动 `{}` 失败: {e}", argv[0])))?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(Fail::env(format!(
            "`{}` 失败（rc={:?}）: {}",
            argv.join(" "),
            out.status.code(),
            stderr.trim()
        )));
    }
    Ok(stdout)
}

/// root 权限探测（/proc/self/status Uid 行 euid 字段，零 libc）。Err=判据不可用（非 root）。
fn euid_is_root() -> Result<bool, String> {
    let raw = std::fs::read_to_string("/proc/self/status")
        .map_err(|e| format!("/proc/self/status 不可读: {e}"))?;
    for line in raw.lines() {
        if let Some(rest) = line.strip_prefix("Uid:") {
            let euid = rest
                .split_whitespace()
                .nth(1)
                .ok_or_else(|| format!("Uid 行字段缺失: {line}"))?;
            return Ok(euid == "0");
        }
    }
    Err("Uid 行不存在".to_string())
}

// ---------- 通道 ----------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Channel {
    LocalRoot,
    Sidecar { container: String, image: String },
}

impl Channel {
    /// tc 子命令形（不含程序名）→ 含程序名全 argv。
    fn tc_argv(&self, args: &[&str]) -> Vec<String> {
        let mut full = vec!["tc".to_string()];
        full.extend(args.iter().map(|s| (*s).to_string()));
        self.prefixed(&full)
    }

    /// 通道实执行前缀：local 原样，sidecar 经 `docker exec <c>`（任意 iproute2 程序同构）。
    fn prefixed(&self, argv: &[String]) -> Vec<String> {
        match self {
            Channel::LocalRoot => argv.to_vec(),
            Channel::Sidecar { container, .. } => {
                let mut v = vec!["docker".to_string(), "exec".to_string(), container.clone()];
                v.extend(argv.iter().cloned());
                v
            }
        }
    }

    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Channel::LocalRoot => "local-root".into(),
            Channel::Sidecar { container, .. } => format!("sidecar({container})"),
        }
    }
}

/// tc 实执行（args = 不含 `tc` 前缀的 argv）。
pub fn tc_exec(channel: &Channel, args: &[&str]) -> Wn<String> {
    let argv = channel.tc_argv(args);
    let refs: Vec<&str> = argv.iter().map(String::as_str).collect();
    run(&refs)
}

/// 任意 iproute2 程序实执行（argv[0]=程序名，如 `["ip","link","show","ifb0"]`；
/// T9 ifb 路径用 `ip`，sidecar 通道同 docker exec 前缀）。
pub fn exec_prog(channel: &Channel, argv: &[&str]) -> Wn<String> {
    let owned: Vec<String> = argv.iter().map(|s| (*s).to_string()).collect();
    let full = channel.prefixed(&owned);
    let refs: Vec<&str> = full.iter().map(String::as_str).collect();
    run(&refs)
}

/// 探测/建立通道（auto：local 判据=euid0∧tc -V 可用；否则 sidecar ensure；都失败 exit2 双因并报）。
pub fn probe_channel(env: &Env, iface: &str) -> Wn<Channel> {
    match env.channel_pref.as_str() {
        "local" => local_channel().map_err(|cause| {
            Fail::env(format!("WEAKNET_CHANNEL=local 指定但本地根通道不可用: {cause}"))
        }),
        "sidecar" => ensure_sidecar(env, iface)
            .map_err(Fail::env)
            .map(|()| Channel::Sidecar {
                container: env.sidecar.clone(),
                image: env.image.clone(),
            }),
        "auto" => {
            let local_cause = match local_channel() {
                Ok(c) => return Ok(c),
                Err(e) => e,
            };
            match ensure_sidecar(env, iface) {
                Ok(()) => Ok(Channel::Sidecar {
                    container: env.sidecar.clone(),
                    image: env.image.clone(),
                }),
                Err(side_cause) => Err(Fail::env(format!(
                    "无可用特权通道：local({local_cause}) / sidecar({side_cause})——本机 root+iproute2 或 docker(docker group) 二选一就位后重试"
                ))),
            }
        }
        other => Err(Fail::bad_param(format!(
            "WEAKNET_CHANNEL 需 auto|local|sidecar，得 {other:?}"
        ))),
    }
}

fn local_channel() -> Result<Channel, String> {
    if !euid_is_root().unwrap_or_else(|e| {
        eprintln!("weaknet(engine): root 判据: {e}");
        false
    }) {
        return Err("非 root（tc 需 CAP_NET_ADMIN）".to_string());
    }
    run(&["tc", "-V"]).map(|_| Channel::LocalRoot).map_err(|e| e.msg)
}

/// sidecar 生命周期（bash ensure_sidecar 逐支平移：running→start→pull→run→秒退取证→
/// tc 可达→netem 模块只查/载不试挂 PIT-184 防呆）。
pub fn ensure_sidecar(env: &Env, iface: &str) -> Result<(), String> {
    run(&["docker", "--version"]).map_err(|e| format!("docker 不可用（weaknet 特权通道缺失）: {}", e.msg))?;
    let running = run(&["docker", "ps", "-q", "-f", &format!("name=^{}$", env.sidecar)])
        .map_err(|e| e.msg)?;
    if !running.trim().is_empty() {
        return Ok(());
    }
    let existed = run(&["docker", "ps", "-aq", "-f", &format!("name=^{}$", env.sidecar)])
        .map_err(|e| e.msg)?;
    if !existed.trim().is_empty() {
        run(&["docker", "start", &env.sidecar]).map_err(|e| {
            format!("sidecar 启动失败（docker 权限/守护进程？）: {}", e.msg)
        })?;
        return sidecar_ready_check(env, iface);
    }
    if run(&["docker", "image", "inspect", &env.image]).is_err() {
        eprintln!("weaknet(engine): 镜像缺失，尝试拉取：docker pull {}", env.image);
        run(&["docker", "pull", &env.image]).map_err(|e| {
            format!("拉取 {} 失败（内网代理？手动 pull 后重试）: {}", env.image, e.msg)
        })?;
    }
    run(&[
        "docker", "run", "-d", "--name", &env.sidecar,
        "--cap-add", "NET_ADMIN", "--net", "host",
        "--entrypoint", "sh", &env.image,
        "-c", "while :; do sleep 3600; done",
    ])
    .map_err(|e| format!("sidecar 创建失败: {}", e.msg))?;
    sidecar_ready_check(env, iface)
}

fn sidecar_ready_check(env: &Env, iface: &str) -> Result<(), String> {
    // bash `sleep 0.5` 后秒退取证（镜像 entrypoint while-loop 保活是 2015 版无 sleep infinity 的平移）
    std::thread::sleep(Duration::from_millis(500));
    let running = run(&["docker", "ps", "-q", "-f", &format!("name=^{}$", env.sidecar)])
        .map_err(|e| e.msg)?;
    if running.trim().is_empty() {
        let logs = run(&["docker", "logs", "--tail", "2", &env.sidecar])
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|e| format!("<logs 读取失败: {}>", e.msg));
        return Err(format!("sidecar 秒退（docker logs 尾部）: {logs}"));
    }
    let chan = Channel::Sidecar { container: env.sidecar.clone(), image: env.image.clone() };
    tc_exec(&chan, &["qdisc", "show", "dev", iface])
        .map_err(|e| format!("sidecar 内 tc 不可达 {iface}: {}", e.msg))?;
    // 模块存活探测：只查/载，不试挂（root 槽常被我方占用——PIT-184 同族防呆）
    run(&[
        "docker", "exec", &env.sidecar, "sh", "-c",
        "lsmod | grep -q netem || modprobe sch_netem",
    ])
    .map_err(|e| format!("sch_netem 不可载（宿主 root 执行 modprobe sch_netem 后重试）: {}", e.msg))?;
    Ok(())
}

/// seed 版本门（bash seed_gate：iproute2 ≥ 6.6 才语义有效；日期码版=非语义版本直接报因）。
pub fn version_gate_seed(channel: &Channel) -> Wn<()> {
    let v = tc_exec(channel, &["-V"])?;
    let Some((maj, min)) = parse_iproute2_semver(&v) else {
        return Err(Fail::bad_param(format!(
            "tc 非语义版本（{}），seed 需 iproute2≥6.6",
            v.trim()
        )));
    };
    if (maj, min) >= (6, 6) {
        Ok(())
    } else {
        Err(Fail::bad_param(format!(
            "tc={maj}.{min} < 6.6 不支持 seed（seed 亦仅 GE loss 可复现，bash 头注 5）"
        )))
    }
}

pub(crate) fn parse_iproute2_semver(out: &str) -> Option<(u32, u32)> {
    // 真实形两种：`iproute2-6.11.0`（新，连字符）/ `iproute2 ss150831`（旧，空格）——分隔符均容。
    let after = out.split("iproute2").nth(1)?;
    let mut it = after.trim_start_matches(['-', ' ', ':']).split('.');
    let maj = it.next()?.parse().ok()?;
    let min = it.next()?.split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()?;
    Some((maj, min))
}

// ---------- 纯规划（dry-run / 快照测判据源，零内核触达） ----------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RootAction {
    /// root 缺席/默认类 qdisc → del(best-effort) + add htb
    Add,
    /// htb 1: 已在（我方现场延续）→ 跳过 root 步走增量 change
    Change,
    /// 裸 root netem（--all 形态兼容位，M0 CLI 未暴露）→ replace root netem
    Replace,
}

/// 首根行判据（bash：无 htb 走 add；`^qdisc htb 1:` 在则 change-only）。
#[must_use]
pub fn root_action(qdisc_show: &str) -> RootAction {
    let first_root = qdisc_show.lines().find(|l| l.starts_with("qdisc "));
    match first_root {
        Some(l) if l.starts_with("qdisc htb") => RootAction::Change,
        Some(l) if l.starts_with("qdisc netem") && l.contains("root") => RootAction::Replace,
        _ => RootAction::Add,
    }
}

/// 执行程序（T9：ifb 链路含 `ip link` 步骤，与 tc 共用 StepPlan/通道前缀机器）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Prog {
    Tc,
    Ip,
}

impl Prog {
    #[must_use]
    pub fn str(self) -> &'static str {
        match self {
            Prog::Tc => "tc",
            Prog::Ip => "ip",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepPlan {
    /// 主 argv（不含程序名；执行/展示时按 prog 拼接）
    pub run: Vec<String>,
    pub prog: Prog,
    /// 2015 兼容：change 失败回落 add（bash `change 2>/dev/null || add`）
    pub alt: Option<Vec<String>>,
    /// best-effort 步失败仅注记不中断（del root / filter flush）
    pub best_effort: bool,
    pub what: String,
}

impl StepPlan {
    /// dry-run 展示行（bash do_apply --dry-run 口径：主形展示，best_effort 步省略）。
    #[must_use]
    pub fn display_lines(steps: &[StepPlan]) -> Vec<String> {
        steps
            .iter()
            .filter(|s| !s.best_effort)
            .map(|s| format!("{} {}", s.prog.str(), s.run.join(" ")))
            .collect()
    }
}

/// 施加骨架步骤序（bash build_skeleton 精确顺序）：root(add 形含前置 best-effort del) →
/// 1:99 default 类哨兵 → 1:10 类 rate_arg → netem 叶 spec → filters 全量重建（flush+legs）→
/// 可选信令腿（prio 2 / protocol 6）。
#[must_use]
pub fn plan_skeleton(
    dev: &str,
    add_root: bool,
    netem_spec: &str,
    rate_arg: &str,
    legs: &[LegFilter],
    sig_port: Option<u16>,
) -> Vec<StepPlan> {
    let mut steps = Vec::new();
    if add_root {
        steps.push(StepPlan {
            run: vec!["qdisc".into(), "del".into(), "dev".into(), dev.into(), "root".into()],
            prog: Prog::Tc,
            alt: None,
            best_effort: true,
            what: "前置清 root（--all 残留容忍）".into(),
        });
        steps.push(StepPlan {
            run: vec!["qdisc".into(), "add".into(), "dev".into(), dev.into(),
                      "root".into(), "handle".into(), "1:".into(), "htb".into(), "default".into(), "99".into()],
            prog: Prog::Tc,
            alt: None,
            best_effort: false,
            what: "htb root 建立".into(),
        });
    }
    steps.push(change_or_add(
        "default 类 1:99 哨兵",
        dev,
        &["parent", "1:", "classid", "1:99", "htb", "rate", spec::SENTINEL_RATE],
    ));
    steps.push(change_or_add(
        &format!("netem 承载类 1:10 rate={rate_arg}"),
        dev,
        &["parent", "1:", "classid", "1:10", "htb", "rate", rate_arg, "ceil", spec::SENTINEL_RATE],
    ));
    let mut leaf: Vec<String> = vec![
        "qdisc".into(), "change".into(), "dev".into(), dev.into(),
        "parent".into(), "1:10".into(), "handle".into(), "10:".into(), "netem".into(),
    ];
    leaf.extend(netem_spec.split_whitespace().map(str::to_string));
    let add_leaf: Vec<String> = {
        let mut v = leaf.clone();
        v[1] = "add".into();
        v
    };
    steps.push(StepPlan {
        run: leaf,
        prog: Prog::Tc,
        alt: Some(add_leaf),
        best_effort: false,
        what: "netem 叶 10:".into(),
    });
    steps.push(StepPlan {
        run: vec!["filter".into(), "del".into(), "dev".into(), dev.into(), "parent".into(), "1:".into()],
            prog: Prog::Tc,
        alt: None,
        best_effort: true,
        what: "filters 全量重建前置 flush（幂等）".into(),
    });
    for leg in legs {
        steps.push(StepPlan {
            run: filter_add_argv(dev, "1", leg),
            prog: Prog::Tc,
            alt: None,
            best_effort: false,
            what: format!("媒体腿 filter {:?}", leg.matches),
        });
    }
    if let Some(port) = sig_port {
        let leg = LegFilter {
            protocol: 6,
            matches: vec![(MatchKind::Dport, port)],
            flowid: spec::MEDIA_FLOWID.to_string(),
        };
        steps.push(StepPlan {
            run: filter_add_argv(dev, "2", &leg),
            prog: Prog::Tc,
            alt: None,
            best_effort: false,
            what: format!("信令腿 filter dport={port}（TCP）"),
        });
    }
    steps
}

fn change_or_add(what: &str, dev: &str, rest: &[&str]) -> StepPlan {
    let mk = |verb: &str| -> Vec<String> {
        let mut v = vec![
            "class".to_string(),
            verb.to_string(),
            "dev".to_string(),
            dev.to_string(),
        ];
        v.extend(rest.iter().map(|s| s.to_string()));
        v
    };
    StepPlan {
        run: mk("change"),
        prog: Prog::Tc,
        alt: Some(mk("add")),
        best_effort: false,
        what: what.into(),
    }
}

fn filter_add_argv(dev: &str, prio: &str, leg: &LegFilter) -> Vec<String> {
    let mut fixed: Vec<String> = vec![
        "filter".into(), "add".into(), "dev".into(), dev.into(),
        "parent".into(), "1:".into(), "protocol".into(), "ip".into(),
        "prio".into(), prio.into(), "u32".into(),
        "match".into(), "ip".into(), "protocol".into(), leg.protocol.to_string(), "0xff".into(),
    ];
    for (kind, port) in &leg.matches {
        let k = match kind {
            MatchKind::Sport => "sport",
            MatchKind::Dport => "dport",
        };
        fixed.extend(["match".into(), "ip".into(), k.into(), port.to_string(), "0xffff".into()]);
    }
    fixed.extend(["flowid".into(), leg.flowid.clone()]);
    fixed
}

/// 裸 root netem（--all 兼容形，M0 CLI 未暴露——T10 前 bash 现场对照用）。
#[must_use]
pub fn plan_all_root(dev: &str, netem_spec: &str) -> Vec<StepPlan> {
    let mut run: Vec<String> = vec![
        "qdisc".into(), "replace".into(), "dev".into(), dev.into(), "root".into(), "netem".into(),
    ];
    run.extend(netem_spec.split_whitespace().map(str::to_string));
    vec![StepPlan {
        run,
        prog: Prog::Tc,
        alt: None,
        best_effort: false,
        what: "--all 裸 root netem".into(),
    }]
}

// ---------- T9: ifb ingress 镜像（design §ifb 合同） ----------

/// 物理口 × dir∈{In,Both} → 上行腿必须走 ifb0 镜像路径（§dir 表「物理网卡 dir=in：
/// 端口腿改走 ifb」）。lo 恒纯端口腿（dir_lo 能力恒真，§capability 表）。
#[must_use]
pub fn ifb_needed(kind: IfaceKind, dir: Dir) -> bool {
    kind == IfaceKind::Physical && matches!(dir, Dir::In | Dir::Both)
}

/// fail-closed 门（错误分类判据）：需 ifb 而探测不过 → exit2 报因，禁静默降级为
/// 「无镜像直挂 ingress」（=黑洞三连前罪）。纯判定，供单测矩阵。
pub fn guard_ifb_required(kind: IfaceKind, dir: Dir, probe: (bool, String)) -> Wn<bool> {
    if !ifb_needed(kind, dir) {
        return Ok(false);
    }
    if probe.0 {
        return Ok(true);
    }
    Err(Fail::env(format!(
        "物理口上行（dir={:?}）需 {IFB_NOTE}，能力探测未过：{} —— 补救=宿主 root modprobe ifb（netns 非特权装不了模块）",
        dir, probe.1
    )))
}

const IFB_NOTE: &str = "ifb0 镜像";

/// ifb 镜像链步骤（§ifb 合同施加序，含 link up——缺 up 步 = mirred 重定向到 DOWN 设备
/// = ENETDOWN 全丢、netem 零命中而回读「行存在」的假绿源）。叶形决定（本轮记录）：
/// **netem 直挂 ifb0 root**（合同原文 "tc qdisc add dev ifb0 root netem…" 最小形），
/// 不把 htb 骨架镜像上 ifb0——代价 = rate 墙在 ingress 腿不生效，replay 侧 WARN 报出；
/// 升级路径 = 需要 ifb 侧限速时移植 plan_skeleton 的 root/class 段。
/// `need_link` 由 apply 先读后判（ifb0 已存在=他人/系统建的不重发 add、teardown 不删）。
#[must_use]
pub fn plan_ifb(dev: &str, netem_spec: &str, legs: &[LegFilter], need_link: bool) -> Vec<StepPlan> {
    let mut steps = Vec::new();
    if need_link {
        steps.push(StepPlan {
            run: vec!["link".into(), "add".into(), ifb::IFB_DEV.into(), "type".into(), "ifb".into()],
            prog: Prog::Ip,
            alt: None,
            best_effort: false,
            what: format!("{} 建立（本次会话为 owner）", ifb::IFB_DEV),
        });
    }
    steps.push(StepPlan {
        run: vec!["link".into(), "set".into(), ifb::IFB_DEV.into(), "up".into()],
        prog: Prog::Ip,
        alt: None,
        best_effort: false,
        what: format!("{} link up（缺此步=ENETDOWN 黑洞，§ifb 合同）", ifb::IFB_DEV),
    });
    let mut root: Vec<String> = vec![
        "qdisc".into(), "change".into(), "dev".into(), ifb::IFB_DEV.into(),
        "root".into(), "netem".into(),
    ];
    root.extend(netem_spec.split_whitespace().map(str::to_string));
    let add_root: Vec<String> = {
        let mut v = root.clone();
        v[1] = "add".into();
        v
    };
    steps.push(StepPlan {
        run: root,
        prog: Prog::Tc,
        alt: Some(add_root),
        best_effort: false,
        what: format!("{} root netem 叶", ifb::IFB_DEV),
    });
    steps.push(StepPlan {
        run: vec!["qdisc".into(), "add".into(), "dev".into(), dev.into(), "ingress".into()],
        prog: Prog::Tc,
        alt: None,
        best_effort: true, // 已存在（set 重放）= File exists 容忍；真建不起由下一步 filter 报因
        what: "iface ingress qdisc（镜像挂载点）".into(),
    });
    steps.push(StepPlan {
        run: vec!["filter".into(), "del".into(), "dev".into(), dev.into(), "ingress".into()],
        prog: Prog::Tc,
        alt: None,
        best_effort: true,
        what: "ingress filters 全量重建前置 flush（幂等）".into(),
    });
    for leg in legs {
        steps.push(StepPlan {
            run: mirred_filter_argv(dev, leg),
            prog: Prog::Tc,
            alt: None,
            best_effort: false,
            what: format!("上行镜像腿 filter {:?} → {}", leg.matches, ifb::IFB_DEV),
        });
    }
    steps
}

/// ingress 镜像 filter：与 egress 腿同合取形（protocol 17 + 端口 AND 单 filter），action 换
/// `mirred egress redirect dev ifb0`（§ifb 合同；无 parent 1:/flowid——ingress qdisc 无类）。
fn mirred_filter_argv(dev: &str, leg: &LegFilter) -> Vec<String> {
    let mut fixed: Vec<String> = vec![
        "filter".into(), "add".into(), "dev".into(), dev.into(), "ingress".into(),
        "protocol".into(), "ip".into(), "u32".into(),
        "match".into(), "ip".into(), "protocol".into(), leg.protocol.to_string(), "0xff".into(),
    ];
    for (kind, port) in &leg.matches {
        let k = match kind {
            MatchKind::Sport => "sport",
            MatchKind::Dport => "dport",
        };
        fixed.extend(["match".into(), "ip".into(), k.into(), port.to_string(), "0xffff".into()]);
    }
    fixed.extend([
        "action".into(), "mirred".into(), "egress".into(), "redirect".into(),
        "dev".into(), ifb::IFB_DEV.into(),
    ]);
    fixed
}

pub fn iface_kind_of(dev: &str) -> IfaceKind {
    if dev == "lo" { IfaceKind::Loopback } else { IfaceKind::Physical }
}

// ---------- fail-closed 守卫（纯函数于回读文本） ----------

/// bash guard_foreign_root：root 槽白名单（noqueue/fq_codel/"fq "/pfifo_fast）放行；
/// htb/netem 需我方 state 自证；其余=他方占用 exit3。
pub fn guard_foreign_root(qdisc_show: &str, state_exists: bool) -> Wn<()> {
    let Some(line) = qdisc_show.lines().find(|l| l.starts_with("qdisc ")) else {
        return Ok(());
    };
    if ["noqueue", "fq_codel", "pfifo_fast"].iter().any(|p| line.contains(p))
        || line.contains(" fq ")
    {
        return Ok(());
    }
    if line.starts_with("qdisc htb") {
        return if state_exists {
            Ok(())
        } else {
            Err(Fail::conflict("lo 已有 htb root 但无 weaknet state（他方/残留）——先 weaknet clear 复位"))
        };
    }
    if line.starts_with("qdisc netem") {
        return Err(Fail::conflict(format!(
            "root 的 netem 非我方所辖（他方/--all 残留/手挂）——拒绝动它: {line}"
        )));
    }
    Err(Fail::conflict(format!("root 槽被他方占用: {line}（weaknet 拒绝动别家 qdisc）")))
}

// ---------- 回读指纹 ----------

fn norm_words(line: &str) -> Vec<String> {
    line.split_whitespace()
        .map(|w| {
            let w = w.trim_matches(|c: char| c == '(' || c == ')' || c == '%');
            normalize_num_token(w)
        })
        .collect()
}

/// 数值 token 归一：旧 tc 回显「delay 40.0ms」（浮点带尾零）而 spec 形「delay 40ms」——两侧同法
/// 剥小数尾零（"40.0ms"→"40ms"、"2.5%"→"2.5%"、"60.00"→"60"）。非数值段原样。
#[must_use]
fn normalize_num_token(w: &str) -> String {
    let unit_at = w.find(|c: char| !c.is_ascii_digit() && c != '.');
    let (num, unit) = match unit_at {
        Some(i) => (&w[..i], &w[i..]),
        None => (w, ""),
    };
    if !num.contains('.') {
        return w.to_string();
    }
    let trimmed = num.trim_end_matches('0').trim_end_matches('.');
    format!("{trimmed}{unit}")
}

/// spec 区分性 token 集：数字/单位 token + 模式关键词（gemodel/loss/reorder/seed/delay/limit）。
/// 「distribution normal」刻意排除——旧 tc 不回显分布词（2015 镜像实证面），jitter 由 "{j}ms"
/// 数值 token 承载。两侧归一与 [`normalize_num_token`] 同步（旧镜像 delay 回显浮点形）。
#[must_use]
pub fn fingerprint_tokens(netem_spec: &str) -> Vec<String> {
    netem_spec
        .split_whitespace()
        .map(|w| normalize_num_token(w.trim_end_matches('%')))
        .filter(|w| w.chars().any(|c| c.is_ascii_digit()) || SPEC_KEYWORDS.contains(&w.as_str()))
        .collect()
}

const SPEC_KEYWORDS: &[&str] = &["limit", "delay", "loss", "gemodel", "reorder", "seed"];

/// 叶行形（T9：ifb 路径的 netem 挂 ifb0 root，无 parent 1:10）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeafForm {
    /// egress 骨架：`qdisc netem … parent 1:10`。
    Parent110,
    /// ifb0 ingress 路径（§ifb 合同最小形）：`qdisc netem … root`。
    Root,
}

fn leaf_line_matches(l: &str, form: LeafForm) -> bool {
    l.contains("netem")
        && match form {
            LeafForm::Parent110 => l.contains("parent 1:10"),
            LeafForm::Root => l.contains(" root"),
        }
}

/// 叶行 = 按形匹配的首行。
#[must_use]
pub fn find_leaf_line_form(qdisc_show: &str, form: LeafForm) -> Option<&str> {
    qdisc_show.lines().find(|l| leaf_line_matches(l, form))
}

/// 叶行 = 含 netem 且 parent 1:10 的首行（egress 骨架形）。
#[must_use]
pub fn find_leaf_line(qdisc_show: &str) -> Option<&str> {
    find_leaf_line_form(qdisc_show, LeafForm::Parent110)
}

pub fn assert_fingerprint(qdisc_show: &str, netem_spec: &str) -> Wn<()> {
    assert_fingerprint_form(qdisc_show, netem_spec, LeafForm::Parent110)
}

/// 指纹断言：spec 的区分 token 必须逐个出现在叶行（归一化词集）——缺失列表 got/want 报因。
pub fn assert_fingerprint_form(qdisc_show: &str, netem_spec: &str, form: LeafForm) -> Wn<()> {
    let leaf = find_leaf_line_form(qdisc_show, form).ok_or_else(|| {
        Fail::env(match form {
            LeafForm::Parent110 => "回读：1:10 叶无 netem（施加未生效=PIT-184 同族假绿判据）".to_string(),
            LeafForm::Root => format!("回读：{} root 无 netem（ifb 镜像未生效）", ifb::IFB_DEV),
        })
    })?;
    let words = norm_words(leaf);
    let want = fingerprint_tokens(netem_spec);
    let missing: Vec<&str> = want
        .iter()
        .filter(|t| !words.iter().any(|w| w == *t))
        .map(String::as_str)
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(Fail::env(format!(
            "回读指纹不符：缺 {:?}（叶行现值: {leaf}）",
            missing
        )))
    }
}

/// `-s` 叶行块统计（Sent pkt + dropped）——serve 状态帧 tc 字段取数源。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LeafStats {
    pub sent_pkt: u64,
    pub dropped: u64,
}

/// `-s` 输出叶行块的 Sent X bytes Y pkt → Y（bash leaf_pkt 平移；扫叶行及其后一行）。
#[must_use]
pub fn leaf_sent(qdisc_s_show: &str) -> Option<u64> {
    leaf_stats(qdisc_s_show).map(|s| s.sent_pkt)
}

/// 叶行及其后一行窗口解析 `Sent _ bytes N pkt (dropped M, …)`；dropped 缺形=0（旧 tc 容错）。
#[must_use]
pub fn leaf_stats(qdisc_s_show: &str) -> Option<LeafStats> {
    leaf_stats_form(qdisc_s_show, LeafForm::Parent110)
}

/// 形感知版（T9：ifb0 root netem 叶同机制取数）。
#[must_use]
pub fn leaf_stats_form(qdisc_s_show: &str, form: LeafForm) -> Option<LeafStats> {
    let idx = qdisc_s_show.lines().enumerate().find_map(|(i, l)| {
        if leaf_line_matches(l, form) { Some(i) } else { None }
    })?;
    let window: Vec<&str> = qdisc_s_show.lines().skip(idx).take(2).collect();
    for line in window {
        let toks: Vec<&str> = line.split_whitespace().collect();
        for w in toks.windows(5) {
            if w[0] == "Sent" && w[2] == "bytes" && w[4] == "pkt"
                && let Ok(n) = w[3].parse::<u64>()
            {
                let dropped = toks
                    .iter()
                    .position(|t| *t == "(dropped")
                    .and_then(|i| toks.get(i + 1))
                    .and_then(|v| v.trim_end_matches(',').parse::<u64>().ok())
                    .unwrap_or(0);
                return Some(LeafStats { sent_pkt: n, dropped });
            }
        }
    }
    None
}

// ---------- verify / counter-reset（活流量双采样） ----------

/// 双采样命中覆盖断言（bash verify_measure：1s 窗 leaf Sent 无增量=媒体未命中过滤）。
/// 返回 (pre, post)；post<pre（set 路径）= 计数重置（本内核实测不重置，头注 6——降 advisory 注记）。
pub fn verify_measure(channel: &Channel, dev: &str, pre: Option<u64>) -> Wn<(u64, u64)> {
    verify_measure_form(channel, dev, pre, LeafForm::Parent110)
}

/// 形感知版（T9：上行铁证 = ifb0 leaf Sent 增长，design §ifb「命中验证以 ifb0 leaf
/// Sent 计数增长为包真穿过 netem 的证」）。
pub fn verify_measure_form(
    channel: &Channel,
    dev: &str,
    pre: Option<u64>,
    form: LeafForm,
) -> Wn<(u64, u64)> {
    let read_leaf = |show: &str| leaf_stats_form(show, form).map(|s| s.sent_pkt);
    let pre = match pre {
        Some(p) => p,
        None => read_leaf(&tc_exec(channel, &["-s", "qdisc", "show", "dev", dev])?)
            .ok_or_else(|| Fail::env("verify：读不到叶 Sent（叶不在？）"))?,
    };
    std::thread::sleep(Duration::from_secs(1));
    let post = read_leaf(&tc_exec(channel, &["-s", "qdisc", "show", "dev", dev])?)
        .ok_or_else(|| Fail::env("verify：二次采样叶 Sent 丢失"))?;
    if pre == post {
        return Err(Fail::env(format!(
            "verify：leaf 1s 无包增量（{pre}→{post}）——媒体未命中过滤（查流是否在产 / --rtp-port）"
        )));
    }
    Ok((pre, post))
}

#[must_use]
pub fn counters_reset(pre: u64, post: u64) -> bool {
    post < pre
}

// ---------- teardown 计划/执行 ----------

/// apply 时落盘的撤除计划（design §ifb 合同，顺序敏感）：
/// 1) iface ingress 先删（断镜像于先，防 redirect 悬空黑洞）→ 2) ifb0 root →
/// 3) **仅本次所建** ifb0 才 del 链路（别人/系统建的不碰——state 记录 owner）→ 4) iface root。
///
/// **root-del 恒常无条件**（bash do_clear parity）：clear=救火通道，步失败经 is_not_exist
/// 幂等化（双发互踩可接受）；外米 qdisc 在 apply 时已被 guard_foreign_root 拦在门外，
/// 能活到 clear 阶段的 root 必是我方或我方残留——都该删。前版 created_root 门控
/// （救援轮「比 bash 更严」）即 **PIT-187 clear 留树 bug 根因**（T7 实盘三连复现）。
/// 纯 ingress 会话虽未在 apply 动 iface root，root-del 仍恒在（回落内核缺省=幂等无害，
/// 维持单一无条件形防 created_* 门控复辟）。
///
/// steps 形约定：首元素 `ip` = 链路类命令（run_step_resilient 路由到 ip 程序），
/// 否则为 tc argv（不含程序名）。
#[must_use]
pub fn plan_teardown(iface: &str, ifb_used: bool, ifb_created: bool) -> Vec<Vec<String>> {
    let mut steps = Vec::new();
    if ifb_used {
        steps.push(vec!["qdisc".into(), "del".into(), "dev".into(), iface.into(), "ingress".into()]);
        steps.push(vec!["qdisc".into(), "del".into(), "dev".into(), ifb::IFB_DEV.into(), "root".into()]);
    }
    if ifb_created {
        steps.push(vec!["ip".into(), "link".into(), "del".into(), ifb::IFB_DEV.into()]);
    }
    steps.push(vec!["qdisc".into(), "del".into(), "dev".into(), iface.into(), "root".into()]);
    steps
}

/// not-exist 类错误 = 幂等成功（design rev-2.2：双 watchdog 并发到点第二发必踩空，防噪声）。
#[must_use]
pub fn is_not_exist(msg: &str) -> bool {
    let m = msg.to_lowercase();
    m.contains("no such file")
        || m.contains("not found")
        || m.contains("cannot find")
        // tc 5.15 形："Cannot delete qdisc with handle of zero" = 设备上无实体 root qdisc
        // （noqueue 不可删）——纯 ifb 向的无条件 root-del 步必踩，语义即幂等无物可删。
        || m.contains("handle of zero")
}

/// 照单执行 teardown（clear/watchdog/autoheal 共用）。返回逐步失败清单（C15 由调用方落
/// timeline——watchdog 上下文写 watchdog-clear-failed）。Sidecar 通道三层兜底平移自
/// del_root_besteffort：exec → docker start+exec → `run --rm` 一次性。
pub fn run_teardown(state: &State) -> Vec<String> {
    let mut failures = Vec::new();
    for step in &state.teardown.steps {
        if let Err(e) = run_step_resilient(&state.teardown, step) {
            eprintln!("weaknet(engine): teardown 步失败 [{:?}]: {e}", step);
            failures.push(format!("{}: {e}", step.join(" ")));
        }
    }
    failures
}

/// teardown 计划 → 通道重建（run_teardown / serve 状态帧共用单一构造源；
/// sidecar 形缺指元 = None，调用方按 state 损坏处理）。
#[must_use]
pub fn channel_of(teardown: &Teardown) -> Option<Channel> {
    match (teardown.channel, &teardown.sidecar) {
        (ChannelSer::LocalRoot, _) => Some(Channel::LocalRoot),
        (ChannelSer::Sidecar, Some(sc)) => Some(Channel::Sidecar {
            container: sc.name.clone(),
            image: sc.image.clone(),
        }),
        (ChannelSer::Sidecar, None) => None,
    }
}

/// argv[0]=程序名（tc|ip）→ 通道实执行。run_steps（施加）与 teardown 路由共用单点。
fn exec_argv(channel: &Channel, prog: &str, args: &[&str]) -> Wn<String> {
    if prog == Prog::Ip.str() {
        let mut full = vec![Prog::Ip.str()];
        full.extend_from_slice(args);
        exec_prog(channel, &full)
    } else {
        tc_exec(channel, args)
    }
}

/// teardown step 约定（plan_teardown 头注）：首元素 `ip` = 链路命令（含程序名），
/// 否则 tc argv（不含程序名）。拆分为 (prog, args)。
fn split_prog(step: &[String]) -> (&'static str, Vec<&str>) {
    let args: Vec<&str> = step.iter().map(String::as_str).collect();
    if matches!(args.first(), Some(p) if *p == Prog::Ip.str()) {
        (Prog::Ip.str(), args[1..].to_vec())
    } else {
        (Prog::Tc.str(), args)
    }
}

fn exec_step(channel: &Channel, step: &[String]) -> Wn<String> {
    let (prog, tail) = split_prog(step);
    exec_argv(channel, prog, &tail)
}

fn run_step_resilient(teardown: &Teardown, step: &[String]) -> Result<(), String> {
    let channel = channel_of(teardown)
        .ok_or_else(|| "teardown 计划为 sidecar 但缺 sidecar 指元（state 损坏）".to_string())?;
    let (prog, tail) = split_prog(step);
    match exec_argv(&channel, prog, &tail) {
        Ok(_) => Ok(()),
        Err(e) if is_not_exist(&e.msg) => Ok(()),
        Err(first) => match &channel {
            // sidecar 可能被 docker rm——qdisc 在宿主 netns 持久，一次性容器兜底
            Channel::Sidecar { container, image } => {
                let _ = run(&["docker", "start", container]);
                match exec_argv(&channel, prog, &tail) {
                    Ok(_) => Ok(()),
                    Err(e2) if is_not_exist(&e2.msg) => Ok(()),
                    Err(second) => match run(&[
                        "docker", "run", "--rm", "--net", "host",
                        "--cap-add", "NET_ADMIN", "--entrypoint", prog, image,
                    ].into_iter().chain(tail.iter().copied()).collect::<Vec<_>>()) {
                        Ok(_) => Ok(()),
                        Err(third) => Err(format!(
                            "三层兜底尽墨：exec({}) / start+exec({}) / run --rm({})",
                            first.msg, second.msg, third.msg
                        )),
                    },
                }
            }
            Channel::LocalRoot => Err(first.msg),
        },
    }
}

/// 惰性自愈主保险（bash autoheal_check：字段无进程可死，SIGKILL/OOM 也兜得住）。
/// 在 set/status/scenario 入口调用（apply/clear 免检——bash 同规）。返回是否发生了自愈。
pub fn autoheal_if_expired(dirs: &Dirs, env: &Env) -> Wn<bool> {
    let Some(st) = State::read_from(&dirs.state_json()).map_err(Fail::env)? else {
        return Ok(false);
    };
    if !fuse::is_expired(Some(st.expires_at_ms), fuse::now_epoch_ms()) {
        return Ok(false);
    }
    eprintln!(
        "weaknet: 过期自愈：apply 已超期（expires_at_ms={} < now），惰性清道",
        st.expires_at_ms
    );
    match fuse::kill_watchdog(&dirs.watchdog_pid()) {
        Ok(o) => eprintln!("weaknet: watchdog 回收: {o:?}"),
        Err(e) => eprintln!("weaknet: WARN watchdog 回收失败（继续清道）: {e}"),
    }
    let failures = run_teardown(&st);
    for f in &failures {
        eprintln!("weaknet: WARN self-heal teardown: {f}");
    }
    // bash 同规：teardown 尽力而为后必 rm state（防死循环重试半现场）
    if let Err(e) = std::fs::remove_file(dirs.state_json())
        && e.kind() != std::io::ErrorKind::NotFound
    {
        return Err(Fail::env(format!("清 state 失败: {e}")));
    }
    let ev = if failures.is_empty() {
        serde_json::json!({"ev": "self-heal-clear"})
    } else {
        serde_json::json!({"ev": "watchdog-clear-failed", "err": failures.join("; ")})
    };
    state::timeline_append(dirs, ev).map_err(Fail::env)?;
    let _ = env; // iface 通道已入 state.teardown——自愈不重探
    Ok(true)
}

// ---------- apply / set 编排 ----------

#[derive(Debug, Clone, Default)]
pub struct ScopeNames {
    /// Stream 定向房间集（= bash STREAM_SEL 拆分形）
    pub rooms: Vec<String>,
    /// Device 定向设备 ID 集
    pub devices: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ApplyRequest {
    pub spec: ImpairSpec,
    pub scope: ScopeSel,
    /// E1/E2（T8）：定向名字（入 state + apply 事件 stream 字段）。
    pub names: ScopeNames,
    pub iface: String,
    /// Media：本地口集；Stream/Device 走 pairs（与 ports 二选一非空）
    pub ports: Vec<u16>,
    pub pairs: Vec<(u16, u16)>,
    pub sig_port: Option<u16>,
}

#[derive(Debug)]
pub enum Replay {
    Apply { duration_secs: u64 },
    Set { prior: Box<State>, before: String },
}

/// 实测 verify 模式（design §server「verify 移出响应路径」）：CLI=Inline（bash 语义：
/// apply verify 失败→root 回滚 exit2）；REST=Deferred——200 仅要求回读指纹过，双采样
/// 移入后台任务，结果落 timeline `verify` 事件（SSE 状态帧 ev 透传播报）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verify {
    Inline,
    Deferred,
}

#[derive(Debug)]
pub struct ReplayOutcome {
    pub spec_string: String,
    pub filter_count: usize,
    pub expires_at_ms: u64,
    pub channel: String,
    /// Set 路径的 counter-reset 探测结果（apply=None，set=Some(bool)，bash echo 同位）
    pub counters_reset: Option<bool>,
    pub leaf: (u64, u64),
}

/// apply/set 共用的全量重放（bash：两形态都是 build_skeleton 全套重建）。
/// Apply：新保险丝 + spawn watchdog + apply 事件；失败回滚 root（bash L522-533）。
/// Set：保原到期、不动 watchdog、回读失败不回滚（防断流——bash do_set L315-317 同规）；
/// 携 counter-reset 探测（change 前后 leaf Sent）。
pub fn replay(
    req: &ApplyRequest,
    mode: Replay,
    dirs: &Dirs,
    env: &Env,
    verify: Verify,
) -> Wn<ReplayOutcome> {
    req.spec.validate().map_err(Fail::bad_param)?;
    let kind = iface_kind_of(&req.iface);
    let use_ifb_want = ifb_needed(kind, req.spec.dir);
    // 腿切分（§dir 表：物理口上行腿走 ifb 镜像）：ifb 会话的 egress 只留 Out 形
    // （dir=Both）或空集（纯 In）；非 ifb = 全腿装 root（lo 现状零变更）。
    let legs = if !use_ifb_want {
        build_filter_legs(req.scope, kind, req.spec.dir, &req.ports, &req.pairs)
            .map_err(Fail::env)?
    } else if req.spec.dir == Dir::Both {
        build_filter_legs(req.scope, kind, Dir::Out, &req.ports, &req.pairs).map_err(Fail::env)?
    } else {
        Vec::new()
    };
    let ingress_legs = if use_ifb_want {
        build_filter_legs(req.scope, kind, Dir::In, &req.ports, &req.pairs).map_err(Fail::env)?
    } else {
        Vec::new()
    };
    let channel = probe_channel(env, &req.iface)?;
    // T9 capability 门（fail-closed）：物理口上行需 ifb0，探测不过 exit2 报因，禁静默降级。
    let use_ifb = guard_ifb_required(kind, req.spec.dir, ifb::probe(&channel))?;
    // 先读后判（§ifb 合同）：ifb0 已在（他人/系统/前会话建）→ 不重发 add、撤除不删链。
    let created_ifb_new = use_ifb && !ifb::link_present(&channel);
    if use_ifb && req.spec.rate_mbps.is_some() {
        eprintln!(
            "weaknet(engine): WARN rate 墙只在 egress htb 生效——{} root=netem 最小形（§ifb 合同裁量），上行腿不受限速",
            ifb::IFB_DEV
        );
    }
    if req.spec.seed.is_some() {
        version_gate_seed(&channel)?;
    }
    let show = tc_exec(&channel, &["qdisc", "show", "dev", &req.iface])?;
    let state_exists = dirs.state_json().is_file();
    // iface root 守卫对 ifb 会话同样保留（fail-closed：root 被外米占用 = 该口现场不干净，
    // 整单拒绝进住，而非只验 ingress）。
    guard_foreign_root(&show, state_exists)?;
    // 纯 In 物理会话 = 不动 iface root（跳骨架）；Both / 有信令腿照常建。
    let install_skeleton = !use_ifb || !legs.is_empty() || req.sig_port.is_some();
    let add_root =
        install_skeleton && matches!(root_action(&show), RootAction::Add | RootAction::Replace);
    let created_root_pre = match &mode {
        Replay::Apply { .. } => add_root,
        Replay::Set { prior, .. } => prior.created_root,
    };
    // Set 反向翻向（上行→无 ifb）：旧镜像链即时撤——全量重放语义，不留旧损伤。
    if let Replay::Set { prior, .. } = &mode
        && prior.ifb_used
        && !use_ifb
    {
            eprintln!(
                "weaknet(engine): set 关闭上行（dir={:?}）——撤 {} 旧镜像链",
                req.spec.dir, prior.iface
            );
            rollback_ifb(&channel, &prior.iface, false);
    }
    let spec_string = render_netem_spec_leg(&req.spec);
    let rate = render_rate_arg(req.spec.rate_mbps);
    let mut steps = Vec::new();
    if install_skeleton {
        steps.extend(plan_skeleton(
            &req.iface, add_root, &spec_string, &rate, &legs, req.sig_port,
        ));
    }
    if use_ifb {
        steps.extend(plan_ifb(&req.iface, &spec_string, &ingress_legs, created_ifb_new));
    }
    // 回读/verify 观测点：纯 In = ifb0 root 形（上行铁证）；其余 = iface 1:10（Both 观测出向腿）。
    let (rd_dev, rd_form) = if use_ifb && !install_skeleton {
        (ifb::IFB_DEV.to_string(), LeafForm::Root)
    } else {
        (req.iface.clone(), LeafForm::Parent110)
    };

    let pre_sent = match &mode {
        Replay::Set { .. } => {
            let s = tc_exec(&channel, &["-s", "qdisc", "show", "dev", &rd_dev])?;
            leaf_stats_form(&s, rd_form).map(|l| l.sent_pkt)
        }
        Replay::Apply { .. } => None,
    };

    if let Err(e) = run_steps(&channel, &steps) {
        if matches!(mode, Replay::Apply { .. }) {
            rollback_apply_failure(&channel, &req.iface, use_ifb, created_ifb_new, install_skeleton);
        }
        return Err(e);
    }
    let sshow = tc_exec(&channel, &["-s", "qdisc", "show", "dev", &rd_dev])?;
    if let Err(e) = assert_fingerprint_form(&sshow, &spec_string, rd_form).and_then(|()| {
        if rd_form == LeafForm::Parent110 {
            assert_class_1_10(&channel, &req.iface)
        } else {
            Ok(())
        }
    }) {
        let e = Fail::env(match &mode {
            Replay::Apply { .. } => format!("{}（施加现场已回滚）", e.msg),
            Replay::Set { .. } => format!("{}（set 未回滚防断流；重跑 set 恢复）", e.msg),
        });
        if matches!(mode, Replay::Apply { .. }) {
            rollback_apply_failure(&channel, &req.iface, use_ifb, created_ifb_new, install_skeleton);
        }
        return Err(e);
    }
    let sent1 = leaf_stats_form(&sshow, rd_form).map(|l| l.sent_pkt);
    let leaf = match verify {
        // Deferred：响应路径零等待；pre 原样带出，后台以同一 pre 二次采样（含 counters-reset 探测）。
        Verify::Deferred => (sent1.unwrap_or(0), sent1.unwrap_or(0)),
        Verify::Inline => match verify_measure_form(&channel, &rd_dev, sent1, rd_form) {
            Ok(pair) => pair,
            Err(e) => {
                if matches!(mode, Replay::Apply { .. }) {
                    rollback_apply_failure(&channel, &req.iface, use_ifb, created_ifb_new, install_skeleton);
                    return Err(Fail::env(format!("{}（施加现场已回滚）", e.msg)));
                }
                // bash do_set：verify 失败 die 2 但不回滚
                return Err(e);
            }
        },
    };
    let counters_reset = match (&mode, pre_sent) {
        (Replay::Set { .. }, Some(pre)) if matches!(verify, Verify::Inline) => {
            Some(counters_reset(pre, leaf.1))
        }
        _ => None,
    };

    let expires_at_ms = match &mode {
        Replay::Apply { duration_secs } => fuse::now_epoch_ms() + duration_secs * 1000,
        Replay::Set { prior, .. } => prior.expires_at_ms,
    };
    // teardown 所有权（Set 翻向只增不减：前 in/both 会话遗留的 ifb 链必须进撤除计划）。
    let (ifb_used_plan, ifb_created_plan) = match &mode {
        Replay::Apply { .. } => (use_ifb, created_ifb_new),
        Replay::Set { prior, .. } => (
            use_ifb || prior.ifb_used,
            created_ifb_new || prior.created_ifb,
        ),
    };
    let st = State {
        schema: state::STATE_SCHEMA.to_string(),
        spec: req.spec.clone(),
        dir: req.spec.dir,
        scope: req.scope,
        iface: req.iface.clone(),
        ports: req.ports.clone(),
        pairs: req.pairs.clone(),
        rooms: req.names.rooms.clone(),
        devices: req.names.devices.clone(),
        sig_port: req.sig_port,
        expires_at_ms,
        created_root: created_root_pre,
        ifb_used: ifb_used_plan,
        created_ifb: ifb_created_plan,
        job: match &mode {
            Replay::Set { prior, .. } => prior.job.clone(),
            Replay::Apply { .. } => None,
        },
        teardown: Teardown {
            channel: match &channel {
                Channel::LocalRoot => ChannelSer::LocalRoot,
                Channel::Sidecar { .. } => ChannelSer::Sidecar,
            },
            sidecar: match &channel {
                Channel::Sidecar { container, image } => Some(SidecarRef {
                    name: container.clone(),
                    image: image.clone(),
                }),
                Channel::LocalRoot => None,
            },
            steps: plan_teardown(&req.iface, ifb_used_plan, ifb_created_plan),
        },
    };
    st.write_to(&dirs.state_json()).map_err(Fail::env)?;
    match &mode {
        Replay::Apply { .. } => {
            let stream_sel = if !req.names.rooms.is_empty() {
                req.names.rooms.join(",")
            } else {
                req.names.devices.join(",")
            };
            state::timeline_append(dirs, state::apply_event(&req.spec, &req.ports, &stream_sel))
                .map_err(Fail::env)?;
            spawn_watchdog(dirs);
        }
        Replay::Set { before, .. } => {
            state::timeline_append(
                dirs,
                serde_json::json!({
                    "ev": "set",
                    "before": before,
                    "after": state::param_summary(&req.spec),
                    "counters": match counters_reset { Some(true) => "reset", _ => "ok" },
                }),
            )
            .map_err(Fail::env)?;
        }
    }
    Ok(ReplayOutcome {
        spec_string,
        filter_count: steps.iter().filter(|s| s.run[0] == "filter" && s.run[1] == "add").count(),
        expires_at_ms,
        channel: channel.describe(),
        counters_reset,
        leaf,
    })
}

/// watchdog spawn（自我 re-exec；失败仅 WARN——主保险=state 过期字段惰性自愈，fuse 第一层）。
pub fn spawn_watchdog(dirs: &Dirs) {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("weaknet(engine): WARN 无法定位自身二进制，watchdog 缺席（惰性自愈仍在）: {e}");
            return;
        }
    };
    let state_path = dirs.state_json();
    let state_arg = match state_path.to_str() {
        Some(s) => s.to_string(),
        None => {
            eprintln!("weaknet(engine): WARN state 路径非 UTF-8，watchdog 缺席: {}", state_path.display());
            return;
        }
    };
    if let Err(e) = fuse::kill_watchdog(&dirs.watchdog_pid()) {
        eprintln!("weaknet(engine): WARN 旧 watchdog 回收异常（继续重挂）: {e}");
    }
    if let Err(e) = fuse::spawn_watchdog(&exe, &["__watchdog", &state_arg], &dirs.watchdog_pid()) {
        eprintln!("weaknet(engine): WARN watchdog spawn 失败（惰性自愈层仍在）: {e}");
    }
}

fn run_steps(channel: &Channel, steps: &[StepPlan]) -> Wn<()> {
    for s in steps {
        match exec_argv(channel, s.prog.str(), &s.run.iter().map(String::as_str).collect::<Vec<_>>()) {
            Ok(_) => {}
            Err(e) => {
                if let Some(alt) = &s.alt {
                    match exec_argv(channel, s.prog.str(), &alt.iter().map(String::as_str).collect::<Vec<_>>()) {
                        Ok(_) => {}
                        Err(e2) if s.best_effort => {
                            eprintln!("weaknet(engine): best-effort 步 [{what}] 双形皆败（容忍）: {e2}", what = s.what);
                        }
                        Err(e2) => {
                            return Err(Fail::env(format!(
                                "{}失败 [{} || {}]: {e2}",
                                s.what,
                                s.run.join(" "),
                                alt.join(" ")
                            )));
                        }
                    }
                } else if s.best_effort {
                    eprintln!("weaknet(engine): best-effort 步 [{}] 失败（容忍）: {}", s.what, e.msg);
                } else {
                    return Err(Fail::env(format!(
                        "{}失败 [{} {}]: {}",
                        s.what,
                        s.prog.str(),
                        s.run.join(" "),
                        e.msg
                    )));
                }
            }
        }
    }
    Ok(())
}

fn rollback_root(channel: &Channel, dev: &str) {
    if let Err(e) = tc_exec(channel, &["qdisc", "del", "dev", dev, "root"]) {
        eprintln!("weaknet(engine): WARN root 回滚失败（需人工 clear）: {e}");
    }
}

/// apply 失败统一回滚（bash L522-533 + §ifb：镜像链与 root 同单清）。
fn rollback_apply_failure(
    channel: &Channel,
    dev: &str,
    use_ifb: bool,
    created_ifb: bool,
    install_skeleton: bool,
) {
    // 纯 ifb 向（未装 root 骨架）时 root-del 必踩 "handle of zero" 噪声——按现场实态回滚
    if install_skeleton {
        rollback_root(channel, dev);
    }
    if use_ifb {
        rollback_ifb(channel, dev, created_ifb);
    }
}

/// ifb 镜像链撤除/回滚（best-effort，ENOENT 幂等；iface root 归 rollback_root）。
/// 也供 set 翻向即时清场（created 所有权位留在 teardown 计划，此处不删链）。
fn rollback_ifb(channel: &Channel, dev: &str, created_ifb: bool) {
    let mut steps = vec![
        vec!["qdisc".into(), "del".into(), "dev".into(), dev.into(), "ingress".into()],
        vec!["qdisc".into(), "del".into(), "dev".into(), ifb::IFB_DEV.into(), "root".into()],
    ];
    if created_ifb {
        steps.push(vec!["ip".into(), "link".into(), "del".into(), ifb::IFB_DEV.into()]);
    }
    for st in &steps {
        if let Err(e) = exec_step(channel, st)
            && !is_not_exist(&e.msg)
        {
            eprintln!(
                "weaknet(engine): WARN ifb 链撤除失败 [{:?}]（残留由 clear 兜底）: {}",
                st, e.msg
            );
        }
    }
}

fn assert_class_1_10(channel: &Channel, dev: &str) -> Wn<()> {
    let out = tc_exec(channel, &["class", "show", "dev", dev])?;
    if out.contains("class htb 1:10") {
        Ok(())
    } else {
        Err(Fail::env("回读：1:10 class 缺失"))
    }
}

/// clear 主体（bash do_clear：kill_watchdog → ensure 通道 → del root 照单/最佳努力 →
/// rm state → timeline clear）。免锁可达（救火通道）。watchdog 上下文用 `quiet`（不再 ensure
/// 打扰、事件名不同）。
pub fn clear(dirs: &Dirs, env: &Env, quiet: bool) -> Wn<String> {
    match fuse::kill_watchdog(&dirs.watchdog_pid()) {
        Ok(o) => eprintln!("weaknet: watchdog 回收: {o:?}"),
        Err(e) => eprintln!("weaknet: WARN watchdog 回收: {e}"),
    }
    let state_path = dirs.state_json();
    let st = State::read_from(&state_path).map_err(Fail::env)?;
    if !quiet {
        // 通道可达性检查（非 --all 现场需要 tc 说话）
        let iface = st.as_ref().map(|s| s.iface.clone()).unwrap_or_else(|| "lo".into());
        probe_channel(env, &iface)?;
    }
    let mut failures = Vec::new();
    match &st {
        Some(s) => failures = run_teardown(s),
        None => {
            // 无 state：最佳努力 del root（外部手挂也照清——clear 是救火通道），ENOENT 幂等
            // iface 无法从 state 得——bash 同规走 WEAKNET_DEV:-lo
            let dev = std::env::var("WEAKNET_DEV").unwrap_or_else(|_| "lo".into());
            let chan = probe_channel(env, &dev)?;
            let step = ["qdisc", "del", "dev", dev.as_str(), "root"];
            if let Err(e) = tc_exec(&chan, &step)
                && !is_not_exist(&e.msg)
            {
                failures.push(format!("{}: {}", step.join(" "), e.msg));
            }
        }
    }
    for f in &failures {
        eprintln!("weaknet: WARN clear teardown: {f}");
    }
    if let Err(e) = std::fs::remove_file(&state_path)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        return Err(Fail::env(format!("删 state 失败: {e}")));
    }
    state::timeline_append(dirs, serde_json::json!({"ev": "clear"})).map_err(Fail::env)?;
    Ok(if failures.is_empty() {
        "cleared（残留检测见 status）".to_string()
    } else {
        format!("cleared-with-warnings（{} 步失败，见上）", failures.len())
    })
}

/// 写命令入口锁（apply/set/scenario——bash flock 同域；busy=exit3）；clear/status 免锁由调用方自律。
pub fn take_write_lock(dirs: &Dirs) -> Wn<state::WriteLock> {
    state::acquire_write_lock(dirs).map_err(Fail::conflict)
}

/// dry-run 通道前缀（纯判定零副作用——不探测/不建容器：local 指定或 euid0 → tc，否则 sidecar 形）。
#[must_use]
pub fn dryrun_prefix(env: &Env) -> String {
    dryrun_prog_prefix(env, Prog::Tc)
}

/// T9：按程序的展示前缀（ifb 链步骤是 `ip ...`，sidecar 形 = `docker exec <c> ip ...`）。
#[must_use]
pub fn dryrun_prog_prefix(env: &Env, prog: Prog) -> String {
    let bare = prog.str();
    let docker = || format!("docker exec {} {bare}", env.sidecar);
    match env.channel_pref.as_str() {
        "local" => bare.to_string(),
        "sidecar" => docker(),
        // auto：仅按 root 判据分支（零探测副作用）。
        _ => {
            if euid_is_root().unwrap_or(false) {
                bare.to_string()
            } else {
                docker()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{Dir, LossSpec};

    fn v(xs: &[&str]) -> Vec<String> {
        xs.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn root_action_pure_decision() {
        // bash：首根行判据——无 htb 走 add；`^qdisc htb 1:` 在则 change-only。
        assert_eq!(root_action("qdisc noqueue 0: root default"), RootAction::Add);
        assert_eq!(root_action("qdisc fq_codel 0: root"), RootAction::Add);
        assert_eq!(root_action(""), RootAction::Add);
        assert_eq!(root_action("qdisc htb 1: root handle 1: rmdefault"), RootAction::Change);
        assert_eq!(root_action("qdisc netem 8001: root limit 100"), RootAction::Replace);
        // ingress 行在前也不误判 Change（首 qdisc 行优先）
        assert_eq!(
            root_action("qdisc ingress ffff: dev lo ingress_block 2\nqdisc htb 1: root"),
            RootAction::Add
        );
    }

    #[test]
    fn guard_foreign_root_fail_closed_matrix() {
        // 白名单放行（bash *noqueue*|*fq_codel*|*fq*空|*pfifo_fast*）
        for l in [
            "qdisc noqueue 0: root",
            "qdisc fq_codel 0: root",
            "qdisc fq 8000: root",
            "qdisc pfifo_fast 0: root",
        ] {
            assert!(guard_foreign_root(l, false).is_ok(), "{l}");
        }
        // htb root：无我方 state → 冲突 exit3；有 → 自证放行
        assert_eq!(guard_foreign_root("qdisc htb 1: root", false).unwrap_err().code, 3);
        assert!(guard_foreign_root("qdisc htb 1: root", true).is_ok());
        // netem root：M0 无 --all 形态 → 一律拒动（严于 bash，fail-closed）
        assert_eq!(guard_foreign_root("qdisc netem 8001: root", true).unwrap_err().code, 3);
        // 其他他方占用 → exit3
        assert_eq!(guard_foreign_root("qdisc sfq 12: root", false).unwrap_err().code, 3);
        // 无 qdisc 行 → 放行（空输出）
        assert!(guard_foreign_root("", false).is_ok());
    }

    #[test]
    fn fingerprint_a_row_does_not_satisfy_spec_b() {
        // design §engine 负例判据：spec A 的叶行不满足 spec B 指纹。
        let spec_a = "limit 100000 delay 40ms loss 2%";
        let leaf_a = "qdisc netem 8001: parent 1:10 handle 10: limit 100000 delay 40ms loss 2%";
        assert!(assert_fingerprint(leaf_a, spec_a).is_ok());
        let spec_b = "limit 100000 delay 80ms loss 5%";
        let e = assert_fingerprint(leaf_a, spec_b).unwrap_err();
        assert_eq!(e.code, 2);
        assert!(e.msg.contains("80ms") && e.msg.contains("5"), "{e}");
        // 无 netem 叶（施加未生效）→ 报因而非静默过
        assert!(assert_fingerprint("qdisc noqueue 0: root", spec_a).is_err());
        // distribution/normal 不进指纹（旧 tc 不回显分布词——2015 镜像实证面）
        let toks = fingerprint_tokens("limit 100000 delay 40ms 7ms distribution normal");
        assert!(!toks.iter().any(|t| t == "distribution" || t == "normal"), "{toks:?}");
        // gemodel 模式词入指纹（loss 模式区分性）
        let g = fingerprint_tokens("limit 100000 loss gemodel 8 25 0.2 0.05");
        assert!(g.iter().any(|t| t == "gemodel"), "{g:?}");
    }

    #[test]
    fn plan_teardown_order_ifb_then_own_root() {
        // §ifb 撤除序（全真值）：先断镜像 ingress → ifb0 root → 仅本次所建才 del 链路 → iface root；
        // root-del 恒在（PIT-187 后不门控）。
        assert_eq!(
            plan_teardown("eth0", true, true),
            vec![
                v(&["qdisc", "del", "dev", "eth0", "ingress"]),
                v(&["qdisc", "del", "dev", "ifb0", "root"]),
                v(&["ip", "link", "del", "ifb0"]),
                v(&["qdisc", "del", "dev", "eth0", "root"]),
            ]
        );
        // 他人在场的 ifb0（created=false）：撤镜像与叶，但绝不 del 链路。
        assert_eq!(
            plan_teardown("eth0", true, false),
            vec![
                v(&["qdisc", "del", "dev", "eth0", "ingress"]),
                v(&["qdisc", "del", "dev", "ifb0", "root"]),
                v(&["qdisc", "del", "dev", "eth0", "root"]),
            ]
        );
        assert_eq!(
            plan_teardown("lo", false, false),
            vec![v(&["qdisc", "del", "dev", "lo", "root"])]
        );
        // 回归钉：root 非本次创建（旧 created_root=false 语境）计划也必须含 root-del——PIT-187
        assert_eq!(
            plan_teardown("wlan0", false, false),
            vec![v(&["qdisc", "del", "dev", "wlan0", "root"])]
        );
    }

    #[test]
    fn ifb_needed_matrix_and_guard() {
        // §dir 表：物理口 × {In, Both} 才需镜像；lo 恒 false；物理×Out 走 egress root。
        assert!(!ifb_needed(IfaceKind::Loopback, Dir::In));
        assert!(!ifb_needed(IfaceKind::Loopback, Dir::Both));
        assert!(!ifb_needed(IfaceKind::Physical, Dir::Out));
        assert!(ifb_needed(IfaceKind::Physical, Dir::In));
        assert!(ifb_needed(IfaceKind::Physical, Dir::Both));
        // 错误分类（unknown-device-type 归 Physical 面）：能力未过 = exit2 报因，禁静默降级。
        let e = guard_ifb_required(
            IfaceKind::Physical,
            Dir::In,
            (false, "内核无 ifb 模块".to_string()),
        )
        .unwrap_err();
        assert_eq!(e.code, 2);
        assert!(e.msg.contains("内核无 ifb 模块") && e.msg.contains("modprobe ifb"), "{e}");
        // lo 不需要 ifb → 能力 false 也不拦（dir_lo 恒真）。
        assert!(!guard_ifb_required(IfaceKind::Loopback, Dir::Both, (false, "x".into())).unwrap());
        assert!(guard_ifb_required(IfaceKind::Physical, Dir::Both, (true, String::new())).unwrap());
    }

    #[test]
    fn plan_ifb_apply_order_matches_contract() {
        // §ifb 施加序：link add（仅 need_link）→ link up → ifb0 root netem → iface ingress →
        // 前置 flush → mirred 镜像腿（protocol 17 合取 + dport AND，action redirect dev ifb0）。
        let legs = vec![LegFilter {
            protocol: 17,
            matches: vec![(MatchKind::Dport, 40010)],
            flowid: spec::MEDIA_FLOWID.to_string(),
        }];
        let steps = plan_ifb("ens32", "limit 100000 delay 80ms loss 10%", &legs, true);
        let shown: Vec<String> = steps
            .iter()
            .map(|t| format!("{} {}{}", t.prog.str(), t.run.join(" "), if t.best_effort { " [be]" } else { "" }))
            .collect();
        assert_eq!(
            shown,
            vec![
                "ip link add ifb0 type ifb",
                "ip link set ifb0 up",
                "tc qdisc change dev ifb0 root netem limit 100000 delay 80ms loss 10%",
                "tc qdisc add dev ens32 ingress [be]",
                "tc filter del dev ens32 ingress [be]",
                "tc filter add dev ens32 ingress protocol ip u32 match ip protocol 17 0xff match ip dport 40010 0xffff action mirred egress redirect dev ifb0",
            ]
        );
        // need_link=false（先读命中 ifb0 已在）：不重发 add，其余照常（幂等重放）。
        assert!(!plan_ifb("ens32", "limit 100000", &[], false)
            .iter()
            .any(|t| t.run.join(" ").contains("link add")));
        assert_eq!(plan_ifb("ens32", "limit 100000", &[], false)[0].run.join(" "), "link set ifb0 up");
        // 缺 up 步 = ENETDOWN 假绿源（合同红线）：up 必须在 netem 叶与 filter 之前。
        let idx_up = shown.iter().position(|l| l.contains("link set ifb0 up")).unwrap();
        let idx_leaf = shown.iter().position(|l| l.contains("ifb0 root netem")).unwrap();
        assert!(idx_up < idx_leaf, "{shown:?}");
    }

    #[test]
    fn leaf_form_root_parsing_parent_form_untouched() {
        // T9 形感知：ifb0 root 叶（无 parent 1:10）解析 + 指纹；Parent110 旧形不回归。
        let root_show = "qdisc netem 8001: root refcnt 2 limit 100000 delay 80ms loss 10%\n Sent 1000 bytes 20 pkt (dropped 2, overlimits 0 requeues 0)";
        assert!(find_leaf_line_form(root_show, LeafForm::Root).is_some());
        assert!(find_leaf_line_form(root_show, LeafForm::Parent110).is_none());
        assert!(assert_fingerprint_form(root_show, "limit 100000 delay 80ms loss 10%", LeafForm::Root).is_ok());
        // 负例：Parent110 spec 不满足 Root 叶（形间不互证）。
        let parent_show = "qdisc netem 8002: parent 1:10 handle 10: limit 100000 delay 80ms";
        assert!(assert_fingerprint_form(parent_show, "limit 100000 delay 80ms", LeafForm::Root).is_err());
        assert!(find_leaf_line(parent_show).is_some());
        let ls = leaf_stats_form(root_show, LeafForm::Root).unwrap();
        assert_eq!((ls.sent_pkt, ls.dropped), (20, 2));
        assert!(leaf_stats(parent_show).is_none());
    }

    #[test]
    fn is_not_exist_covers_rtnetlink_and_docker_forms() {
        assert!(is_not_exist("Error: Cannot delete qdisc with handle of zero.")); // 纯 ifb 向 root-del 噪声灭
        // 双 watchdog 并发到点第二发必踩空 → ENOENT 视同幂等成功（rev-2.2 防噪条款）。
        assert!(is_not_exist("RTNETLINK answers: No such file or directory"));
        assert!(is_not_exist("Error: Cannot find qdisc for parent 1:10"));
        assert!(is_not_exist("Error: Exiting (failing). qdisc not found"));
        assert!(!is_not_exist("RTNETLINK answers: Operation not permitted"));
    }

    #[test]
    fn counters_reset_matches_bash_sortv_judgement() {
        // bash：sent_now < sent_pre 且不等才 reset=yes；相等≠reset（本内核实测不重置）。
        assert!(counters_reset(100, 50));
        assert!(!counters_reset(100, 100));
        assert!(!counters_reset(100, 1_862));
    }

    #[test]
    fn tc_argv_channel_shapes() {
        assert_eq!(
            Channel::LocalRoot.tc_argv(&["qdisc", "show", "dev", "lo"]),
            v(&["tc", "qdisc", "show", "dev", "lo"])
        );
        let c = Channel::Sidecar {
            container: "wnet-t4-test".into(),
            image: "gaiadocker/iproute2".into(),
        };
        assert_eq!(
            c.tc_argv(&["-V"]),
            v(&["docker", "exec", "wnet-t4-test", "tc", "-V"])
        );
        assert_eq!(c.describe(), "sidecar(wnet-t4-test)");
    }

    #[test]
    fn leaf_sent_parses_dash_s_block() {
        let out = "qdisc htb 1: root handle 1: prio 1: ref 4\n Sent 0 bytes 0 pkt (dropped 0, overlimits 0 requeues 0)\nqdisc netem 8002: parent 1:10 handle 10: limit 100000 delay 40ms\n Sent 12345 bytes 67 pkt (dropped 3, overlimits 0 requeues 0)";
        assert_eq!(leaf_sent(out), Some(67));
        assert_eq!(leaf_sent("qdisc noqueue 0: root"), None);
    }

    #[test]
    fn parse_iproute2_semver_both_separator_forms() {
        assert_eq!(parse_iproute2_semver("tc, from iproute2-6.11.0"), Some((6, 11)));
        assert_eq!(parse_iproute2_semver("tc, from iproute2-4.9.0"), Some((4, 9)));
        assert_eq!(parse_iproute2_semver("tc, from iproute2 ss150831"), None);
        assert_eq!(parse_iproute2_semver("tc, from iproute2-6.6"), Some((6, 6)));
    }

    #[test]
    fn dryrun_prefix_preserves_pref() {
        // 显式指定形零探测直判（root 机上 sidecar 也必须展示 docker 前缀——修自旧 || 短路 bug）。
        let local = Env {
            channel_pref: "local".into(),
            ..Default::default()
        };
        assert_eq!(dryrun_prefix(&local), "tc");
        let side = Env {
            channel_pref: "sidecar".into(),
            sidecar: "wnet-t4".into(),
            ..Default::default()
        };
        assert_eq!(dryrun_prefix(&side), "docker exec wnet-t4 tc");
    }

    #[test]
    fn write_lock_busy_maps_to_exit3() {
        // flock 同域 busy 语义 = exit3（bash flock -n 失败 die 3）——fn 级可测形。
        let dir = std::env::temp_dir().join(format!("weaknet-enginelock-{}", std::process::id()));
        let dirs = Dirs { statedir: dir.clone() };
        let _lk = take_write_lock(&dirs).expect("首次必成");
        let e = take_write_lock(&dirs).unwrap_err();
        assert_eq!(e.code, 3, "{e}");
        drop(_lk);
        assert!(take_write_lock(&dirs).is_ok(), "释放后可重取");
        std::fs::remove_dir_all(&dir).ok();
    }
    #[test]
    fn seed_gate_pure_semver_judgement() {
        // 版本门纯判据（dir/spec 交叉用 LossSpec 钉住两形状序列化稳定）。
        let spec = ImpairSpec {
            rtt_ms: 80,
            jitter_ms: 0,
            loss: Some(LossSpec::GeModel {
                loss: "8%".into(),
                r: "25%".into(),
                h: "0.2".into(),
                k: "0.05".into(),
            }),
            dir: Dir::In,
            ..Default::default()
        };
        // 单腿 dir=In → 全额 rtt（§dir 表 rtt 标定规则），与 version gate 无交叉但同层钉形。
        assert_eq!(render_netem_spec_leg(&spec), "limit 100000 delay 80ms loss gemodel 8 25 0.2 0.05");
    }
}
