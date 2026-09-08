//! engine.rs —— TcEngine：通道探测（local-root / docker NET_ADMIN sidecar）+ 骨架 argv 全链
//! （逐字对齐 bash build_skeleton，add-or-change 2015 兼容）+ spec 区分性回读指纹 +
//! leaf Sent 双采样 verify / counter-reset 探测 + fail-closed guard + teardown 计划/执行。
//!
//! 真值源：scripts/weaknet.sh（机制）+ design.md §dir 表（腿）。ifb ingress = T9（本文件仅
//! 在 teardown 计划形制中预留位，M0 不产生 ifb 步骤）。
//!
//! 退出码沿用：2 环境不足/施加失败 · 3 状态冲突（他方 qdisc/锁）· 4 参数非法（C15 全分支报因）。

use std::process::Command;
use std::time::Duration;

use crate::fuse;
use crate::spec::{
    self, IfaceKind, ImpairSpec, LegFilter, MatchKind, ScopeSel, build_filter_legs,
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
    fn tc_argv(&self, args: &[&str]) -> Vec<String> {
        match self {
            Channel::LocalRoot => {
                let mut v = vec!["tc".to_string()];
                v.extend(args.iter().map(|s| (*s).to_string()));
                v
            }
            Channel::Sidecar { container, .. } => {
                let mut v = vec!["docker".to_string(), "exec".to_string(), container.clone(), "tc".to_string()];
                v.extend(args.iter().map(|s| (*s).to_string()));
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

fn parse_iproute2_semver(out: &str) -> Option<(u32, u32)> {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepPlan {
    /// 主 argv（不含 `tc` 前缀）
    pub run: Vec<String>,
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
            .map(|s| format!("tc {}", s.run.join(" ")))
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
            alt: None,
            best_effort: true,
            what: "前置清 root（--all 残留容忍）".into(),
        });
        steps.push(StepPlan {
            run: vec!["qdisc".into(), "add".into(), "dev".into(), dev.into(),
                      "root".into(), "handle".into(), "1:".into(), "htb".into(), "default".into(), "99".into()],
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
        alt: Some(add_leaf),
        best_effort: false,
        what: "netem 叶 10:".into(),
    });
    steps.push(StepPlan {
        run: vec!["filter".into(), "del".into(), "dev".into(), dev.into(), "parent".into(), "1:".into()],
        alt: None,
        best_effort: true,
        what: "filters 全量重建前置 flush（幂等）".into(),
    });
    for leg in legs {
        steps.push(StepPlan {
            run: filter_add_argv(dev, "1", leg),
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
    vec![StepPlan { run, alt: None, best_effort: false, what: "--all 裸 root netem".into() }]
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

/// 叶行 = 含 netem 且 parent 1:10 的首行。
#[must_use]
pub fn find_leaf_line(qdisc_show: &str) -> Option<&str> {
    qdisc_show.lines().find(|l| l.contains("netem") && l.contains("parent 1:10"))
}

/// 指纹断言：spec 的区分 token 必须逐个出现在叶行（归一化词集）——缺失列表 got/want 报因。
pub fn assert_fingerprint(qdisc_show: &str, netem_spec: &str) -> Wn<()> {
    let leaf = find_leaf_line(qdisc_show).ok_or_else(|| {
        Fail::env("回读：1:10 叶无 netem（施加未生效=PIT-184 同族假绿判据）")
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

/// `-s` 输出叶行块的 Sent X bytes Y pkt → Y（bash leaf_pkt 平移；扫叶行及其后一行）。
#[must_use]
pub fn leaf_sent(qdisc_s_show: &str) -> Option<u64> {
    let idx = qdisc_s_show.lines().enumerate().find_map(|(i, l)| {
        if l.contains("netem") && l.contains("parent 1:10") { Some(i) } else { None }
    })?;
    let window: Vec<&str> = qdisc_s_show.lines().skip(idx).take(2).collect();
    for line in window {
        let toks: Vec<&str> = line.split_whitespace().collect();
        for w in toks.windows(5) {
            if w[0] == "Sent" && w[2] == "bytes" && w[4] == "pkt"
                && let Ok(n) = w[3].parse::<u64>()
            {
                return Some(n);
            }
        }
    }
    None
}

// ---------- verify / counter-reset（活流量双采样） ----------

/// 双采样命中覆盖断言（bash verify_measure：1s 窗 leaf Sent 无增量=媒体未命中过滤）。
/// 返回 (pre, post)；post<pre（set 路径）= 计数重置（本内核实测不重置，头注 6——降 advisory 注记）。
pub fn verify_measure(channel: &Channel, dev: &str, pre: Option<u64>) -> Wn<(u64, u64)> {
    let pre = match pre {
        Some(p) => p,
        None => leaf_sent(&tc_exec(channel, &["-s", "qdisc", "show", "dev", dev])?)
            .ok_or_else(|| Fail::env("verify：读不到 1:10 叶 Sent（叶不在？）"))?,
    };
    std::thread::sleep(Duration::from_secs(1));
    let post = leaf_sent(&tc_exec(channel, &["-s", "qdisc", "show", "dev", dev])?)
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

/// apply 时落盘的撤除计划（顺序敏感：§ifb 合同 T9 形制先 ingress 后 root；created_ifb 恒
/// false 于 M0——真值到场随 T9，此分支先行入计划语义钉）。
#[must_use]
pub fn plan_teardown(
    _channel: &Channel,
    iface: &str,
    created_root: bool,
    created_ifb: bool,
) -> Vec<Vec<String>> {
    let mut steps = Vec::new();
    if created_ifb {
        // ponytail: ifb 真路径随 T9（§ifb teardown 序）——此处先钉形不执行
        steps.push(vec!["qdisc".into(), "del".into(), "dev".into(), iface.into(), "ingress".into()]);
        steps.push(vec!["qdisc".into(), "del".into(), "dev".into(), "ifb0".into(), "root".into()]);
    }
    if created_root {
        steps.push(vec!["qdisc".into(), "del".into(), "dev".into(), iface.into(), "root".into()]);
    }
    steps
}

/// not-exist 类错误 = 幂等成功（design rev-2.2：双 watchdog 并发到点第二发必踩空，防噪声）。
#[must_use]
pub fn is_not_exist(msg: &str) -> bool {
    let m = msg.to_lowercase();
    m.contains("no such file") || m.contains("not found") || m.contains("cannot find")
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

fn run_step_resilient(teardown: &Teardown, step: &[String]) -> Result<(), String> {
    let chan_args: Vec<&str> = step.iter().map(String::as_str).collect();
    let channel = match (teardown.channel, &teardown.sidecar) {
        (ChannelSer::LocalRoot, _) => Channel::LocalRoot,
        (ChannelSer::Sidecar, Some(sc)) => Channel::Sidecar {
            container: sc.name.clone(),
            image: sc.image.clone(),
        },
        (ChannelSer::Sidecar, None) => {
            return Err("teardown 计划为 sidecar 但缺 sidecar 指元（state 损坏）".to_string());
        }
    };
    match tc_exec(&channel, &chan_args) {
        Ok(_) => Ok(()),
        Err(e) if is_not_exist(&e.msg) => Ok(()),
        Err(first) => match &channel {
            // sidecar 可能被 docker rm——qdisc 在宿主 netns 持久，一次性容器兜底
            Channel::Sidecar { container, image } => {
                let _ = run(&["docker", "start", container]);
                match tc_exec(&channel, &chan_args) {
                    Ok(_) => Ok(()),
                    Err(e2) if is_not_exist(&e2.msg) => Ok(()),
                    Err(second) => match run(&[
                        "docker", "run", "--rm", "--net", "host",
                        "--cap-add", "NET_ADMIN", "--entrypoint", "tc", image,
                    ].into_iter().chain(step.iter().map(String::as_str)).collect::<Vec<_>>()) {
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

#[derive(Debug, Clone)]
pub struct ApplyRequest {
    pub spec: ImpairSpec,
    pub scope: ScopeSel,
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
pub fn replay(req: &ApplyRequest, mode: Replay, dirs: &Dirs, env: &Env) -> Wn<ReplayOutcome> {
    req.spec.validate().map_err(Fail::bad_param)?;
    let legs = build_filter_legs(
        req.scope,
        iface_kind_of(&req.iface),
        req.spec.dir,
        &req.ports,
        &req.pairs,
    )
    .map_err(Fail::env)?;
    let channel = probe_channel(env, &req.iface)?;
    if req.spec.seed.is_some() {
        version_gate_seed(&channel)?;
    }
    let show = tc_exec(&channel, &["qdisc", "show", "dev", &req.iface])?;
    let state_exists = dirs.state_json().is_file();
    guard_foreign_root(&show, state_exists)?;
    let add_root = matches!(root_action(&show), RootAction::Add | RootAction::Replace);
    let created_root_pre = match &mode {
        Replay::Apply { .. } => add_root,
        Replay::Set { prior, .. } => prior.created_root,
    };
    let spec_string = render_netem_spec_leg(&req.spec);
    let rate = render_rate_arg(req.spec.rate_mbps);
    let steps = plan_skeleton(&req.iface, add_root, &spec_string, &rate, &legs, req.sig_port);

    let pre_sent = match &mode {
        Replay::Set { .. } => {
            let s = tc_exec(&channel, &["-s", "qdisc", "show", "dev", &req.iface])?;
            leaf_sent(&s)
        }
        Replay::Apply { .. } => None,
    };

    if let Err(e) = run_steps(&channel, &steps) {
        if matches!(mode, Replay::Apply { .. }) {
            rollback_root(&channel, &req.iface);
        }
        return Err(e);
    }
    let sshow = tc_exec(&channel, &["-s", "qdisc", "show", "dev", &req.iface])?;
    if let Err(e) = assert_fingerprint(&sshow, &spec_string)
        .and_then(|()| assert_class_1_10(&channel, &req.iface))
    {
        let e = Fail::env(match &mode {
            Replay::Apply { .. } => format!("{}（root 已回滚）", e.msg),
            Replay::Set { .. } => format!("{}（set 未回滚防断流；重跑 set 恢复）", e.msg),
        });
        if matches!(mode, Replay::Apply { .. }) {
            rollback_root(&channel, &req.iface);
        }
        return Err(e);
    }
    let sent1 = leaf_sent(&sshow);
    let verify = verify_measure(&channel, &req.iface, sent1);
    let leaf = match verify {
        Ok(pair) => pair,
        Err(e) => {
            if matches!(mode, Replay::Apply { .. }) {
                rollback_root(&channel, &req.iface);
                return Err(Fail::env(format!("{}（root 已回滚）", e.msg)));
            }
            // bash do_set：verify 失败 die 2 但不回滚
            return Err(e);
        }
    };
    let counters_reset = match (&mode, pre_sent) {
        (Replay::Set { .. }, Some(pre)) => Some(counters_reset(pre, leaf.1)),
        _ => None,
    };

    let expires_at_ms = match &mode {
        Replay::Apply { duration_secs } => fuse::now_epoch_ms() + duration_secs * 1000,
        Replay::Set { prior, .. } => prior.expires_at_ms,
    };
    let st = State {
        schema: state::STATE_SCHEMA.to_string(),
        spec: req.spec.clone(),
        dir: req.spec.dir,
        scope: req.scope,
        iface: req.iface.clone(),
        ports: req.ports.clone(),
        sig_port: req.sig_port,
        expires_at_ms,
        created_root: created_root_pre,
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
            steps: plan_teardown(&channel, &req.iface, created_root_pre, false),
        },
    };
    st.write_to(&dirs.state_json()).map_err(Fail::env)?;
    match &mode {
        Replay::Apply { .. } => {
            state::timeline_append(dirs, state::apply_event(&req.spec, &req.ports))
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
        match tc_exec(channel, &s.run.iter().map(String::as_str).collect::<Vec<_>>()) {
            Ok(_) => {}
            Err(e) => {
                if let Some(alt) = &s.alt {
                    match tc_exec(channel, &alt.iter().map(String::as_str).collect::<Vec<_>>()) {
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
                    return Err(Fail::env(format!("{}失败 [tc {}]: {}", s.what, s.run.join(" "), e.msg)));
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
    let docker = || format!("docker exec {} tc", env.sidecar);
    match env.channel_pref.as_str() {
        "local" => "tc".to_string(),
        "sidecar" => docker(),
        // auto：仅按 root 判据分支（零探测副作用）。
        _ => {
            if euid_is_root().unwrap_or(false) {
                "tc".to_string()
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
        // §ifb 撤除序：先断镜像 ingress → ifb0 root → 我方 iface root（仅 created 才入计划）。
        let steps = plan_teardown(&Channel::LocalRoot, "eth0", true, true);
        assert_eq!(
            steps,
            vec![
                v(&["qdisc", "del", "dev", "eth0", "ingress"]),
                v(&["qdisc", "del", "dev", "ifb0", "root"]),
                v(&["qdisc", "del", "dev", "eth0", "root"]),
            ]
        );
        assert_eq!(
            plan_teardown(&Channel::LocalRoot, "lo", true, false),
            vec![v(&["qdisc", "del", "dev", "lo", "root"])]
        );
        assert!(plan_teardown(&Channel::LocalRoot, "lo", false, false).is_empty());
    }

    #[test]
    fn is_not_exist_covers_rtnetlink_and_docker_forms() {
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
