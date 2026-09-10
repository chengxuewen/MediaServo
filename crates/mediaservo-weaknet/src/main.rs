//! mediaservo-weaknet — tc/netem 弱网模拟 agent（双端面单二进制）
//!
//! T4 = CLI 装配：`apply|set|status|clear` 全链 + `up|down` 别名经 engine/state；
//! T12 = 车端面到场：`--config weaknet.yaml`（config.rs 缺省供给源）+ `status --watch`（watch.rs 一屏重绘）。
//! 措辞/退出码真值 = `scripts/weaknet.sh`（至退役日）；契约源 = 主仓 docs/plans/weaknet-agent/。
//! 退出码：0 OK / 2 环境不足·施加失败 / 3 状态冲突（锁·他方 qdisc·state 背离）/ 4 参数非法。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use clap::{Parser, Subcommand};
use serde_json::{Value, json};

use mediaservo_weaknet::config;
use mediaservo_weaknet::engine::{self, ApplyRequest, Env, Fail, Replay, Verify, Wn};
use mediaservo_weaknet::fuse;
use mediaservo_weaknet::scenario;
use mediaservo_weaknet::scope::{self, Targeting};
use mediaservo_weaknet::server;
use mediaservo_weaknet::spec::{self, Dir as SpecDir, IfaceKind, ImpairSpec, LossSpec, ScopeSel};
use mediaservo_weaknet::state::{self, Dirs, State};
use mediaservo_weaknet::watch;

/// bash DEFAULT_DURATION——apply 唯一存活承诺。
const DEFAULT_DURATION: u64 = 300;

#[derive(Debug, Parser)]
#[command(
    name = "mediaservo-weaknet",
    version,
    about = "弱网模拟 agent：server 面(CLI+serve+面板) 与 车端面(up/down/watch+yaml) 同二进制"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Debug, Subcommand)]
enum Cmd {
    /// 施加损伤（缺省 dir=both = bash 双腿等价；缺省 duration=300s 保险丝）
    Apply(Box<ApplyArgs>),
    /// 覆盖字段（state 基底合并，全量重放；不续命；无活跃 spec → exit4）
    Set(Box<ApplyArgs>),
    /// 时间轴剧本
    Scenario {
        #[command(subcommand)]
        step: ScenarioCmd,
    },
    /// 当前状态（--watch = ANSI 一屏重绘，top 式）
    Status {
        #[arg(long)]
        watch: bool,
    },
    /// 清除全部损伤（免锁可达，救火通道）
    Clear,
    /// 控制面：REST+SSE+内嵌面板（缺省 127.0.0.1:9810）
    Serve(ServeArgs),
    /// = apply <profile>（车端别名糖，缺省 duration 300）
    Up {
        profile: String,
        #[arg(long)]
        iface: Option<String>,
        #[arg(long)]
        duration: Option<u64>,
    },
    /// = clear（车端别名糖）
    Down,
    /// = status --watch
    Watch,
    /// 隐藏子命令：watchdog 自我 re-exec 执行体（design §fuse；spawn 方传 current_exe + 本命令）
    #[command(name = "__watchdog", hide = true)]
    Watchdog {
        /// 含到期时刻（expires_at_ms）的 state 文件路径
        state_path: String,
    },
}

#[derive(Debug, Subcommand)]
enum ScenarioCmd {
    /// 运行剧本（job 独占 + 每步瞬持锁；bash 旗标 --no-baseline/--keep 平移）
    Run {
        #[arg(long)]
        file: Option<String>,
        /// 场景根覆盖（缺省见 §车端面寻径）
        #[arg(long)]
        dir: Option<String>,
        /// 跳过基线窗（judge 窗退化为 start+10s 缺省——bash 同义）
        #[arg(long, default_value_t = false)]
        no_baseline: bool,
        /// 收尾保留现场（不 clear；bash 同义）
        #[arg(long, default_value_t = false)]
        keep: bool,
        /// Ctrl-C/SIGTERM 等价触发既有 stop 旗标路径（不发明第二套撤损通道），
        /// 由 runner 步末检查收尾（scenario-end aborted="user" + 既有 teardown，exit0）
        #[arg(long, default_value_t = false)]
        auto_clear: bool,
    },
    /// 列出可用剧本
    List,
    /// 运行中的剧本停止（cancel 旗标/陈旧旗标自清；恒 exit0 幂等）
    Stop,
}

#[derive(Debug, clap::Args)]
struct ApplyArgs {
    /// 命名 profile（profiles/<name>.yaml；显式 flag > profile 优先级同 bash）
    #[arg(long)]
    profile: Option<String>,
    #[arg(long)]
    rtt: Option<u64>,
    #[arg(long)]
    jitter: Option<u64>,
    /// 丢包（"2%" 或 "2"，gemodel 时剥%）
    #[arg(long)]
    loss: Option<String>,
    /// simple（默认）| gemodel（须配 --gemodel）
    #[arg(long)]
    loss_mode: Option<String>,
    /// Gilbert-Elliott 参数组（r h k，% 自动剥除）
    #[arg(long, num_args = 3, value_names = ["R", "H", "K"])]
    gemodel: Option<Vec<String>>,
    #[arg(long)]
    reorder: Option<String>,
    #[arg(long)]
    rate: Option<f64>,
    #[arg(long)]
    seed: Option<u64>,
    /// 保险丝时长（秒，缺省 300——apply 唯一存活承诺，<5 不接受）
    #[arg(long)]
    duration: Option<u64>,
    /// 目标网卡（缺省 lo=文档化常量，C20 豁免形；env WEAKNET_DEV 覆写）
    #[arg(long)]
    iface: Option<String>,
    /// 方向（缺省 both；腿定义 = design §dir 表；set 缺省不覆盖基底 dir）
    #[arg(long, value_enum)]
    dir: Option<Dir>,
    /// 定向：房间名（=流），逗号分隔；"all"=显式回段级（T5 scope.rs 实现）
    #[arg(long)]
    stream: Option<String>,
    /// 定向：设备 ID（owner 分组全部流；旧 server 经 peer_id 兤底 WARN）（T5 实现）
    #[arg(long)]
    device: Option<String>,
    /// 逃生门：显式 RTP 端口集，逗号分隔（零 server 可用；优先级最高）
    #[arg(long)]
    rtp_port: Option<String>,
    /// 信令 TCP 腿端口（可选，bash --signaling 承接位）
    #[arg(long)]
    signaling_port: Option<u16>,
    /// stats 控制面 URL（缺省链：flag > env WEAKNET_SERVER_URL > 探测 out/server/etc/server.yaml）
    #[arg(long)]
    server_url: Option<String>,
    /// 参数文件 weaknet.yaml（车端面；缺省链 flag > env WEAKNET_CONFIG > 二进制同级/cwd 探测）
    #[arg(long)]
    config: Option<String>,
    /// 只打印等价命令序列（含通道前缀，零内核/docker 触达）exit0
    #[arg(long, default_value_t = false)]
    dry_run: bool,
}

#[derive(Debug, clap::Args)]
struct ServeArgs {
    /// 绑定地址（缺省 127.0.0.1:9810；非 loopback 需 --lan，否则启动拒绝）
    #[arg(long)]
    listen: Option<String>,
    /// 放宽至 LAN：Origin 白名单 + token 必填硬闸 + 横幅只出 token
    #[arg(long, default_value_t = false)]
    lan: bool,
    /// token 覆写（优先 WEAKNET_TOKEN env；缺省启动 CSPRNG≥128bit）
    #[arg(long)]
    token: Option<String>,
    /// server 面 stats/admin URL（缺省 flag > env WEAKNET_SERVER_URL > 探测 out/server/etc/）
    #[arg(long)]
    server_url: Option<String>,
    /// token 文件：存在则读（强制 0600）、缺则生成（0600）——优先级 --token flag > env WEAKNET_TOKEN
    /// > 本文件 > CSPRNG（design §D2 sF5；unit 侧传 {dir}/run/weaknet.token = T4）
    #[arg(long)]
    token_file: Option<std::path::PathBuf>,
}

#[derive(clap::ValueEnum, Clone, Copy, Debug)]
enum Dir {
    Out,
    In,
    Both,
}

impl From<Dir> for SpecDir {
    fn from(d: Dir) -> Self {
        match d {
            Dir::Out => SpecDir::Out,
            Dir::In => SpecDir::In,
            Dir::Both => SpecDir::Both,
        }
    }
}

fn main() {
    let cli = Cli::parse();
    if let Err(e) = dispatch(cli.cmd) {
        eprintln!("weaknet: {}", e.msg);
        std::process::exit(i32::try_from(e.code).unwrap_or(2));
    }
}

fn dispatch(cmd: Cmd) -> Wn<()> {
    let dirs = Dirs::from_env();
    let env = Env::from_env();
    match cmd {
        // bash 同规：flock 覆盖 apply|set|scenario；clear/status 免锁可达（救火）。
        // T8 独占门（rev-2.2 Momus-B2）：scenario job 存活期外部 apply/set 先行 409/exit3（锁无关）。
        Cmd::Apply(a) => {
            scenario::job_gate(&dirs)?;
            let _lk = engine::take_write_lock(&dirs)?;
            do_apply(&a, &dirs, &env)
        }
        Cmd::Set(a) => {
            scenario::job_gate(&dirs)?;
            let _lk = engine::take_write_lock(&dirs)?;
            engine::autoheal_if_expired(&dirs, &env)?; // apply/clear 免检、其余开场先验（bash）
            do_set(&a, &dirs, &env)
        }
        Cmd::Scenario { step } => {
            engine::autoheal_if_expired(&dirs, &env)?; // bash：scenario 亦先验过期
            do_scenario(step, &dirs, &env)
        }
        Cmd::Status { watch: is_watch } => {
            engine::autoheal_if_expired(&dirs, &env)?;
            if is_watch {
                return watch::run(&dirs, &env);
            }
            do_status(&dirs, &env)
        }
        Cmd::Clear => {
            println!("{}", engine::clear(&dirs, &env, false)?);
            Ok(())
        }
        Cmd::Serve(args) => {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .map_err(|e| Fail::env(format!("tokio runtime 建立失败: {e}")))?;
            rt.block_on(server::serve_main(
                args.listen.as_deref(),
                args.lan,
                args.token.as_deref(),
                args.token_file.as_deref(),
                args.server_url.as_deref(),
            ))
        }
        Cmd::Up {
            profile,
            iface,
            duration,
        } => {
            let a = ApplyArgs {
                profile: Some(profile),
                rtt: None,
                jitter: None,
                loss: None,
                loss_mode: None,
                gemodel: None,
                reorder: None,
                rate: None,
                seed: None,
                duration,
                iface,
                dir: None,
                stream: None,
                device: None,
                rtp_port: None,
                signaling_port: None,
                server_url: None,
                config: None,
                dry_run: false,
            };
            let _lk = engine::take_write_lock(&dirs)?;
            do_apply(&a, &dirs, &env)
        }
        Cmd::Down => {
            println!("{}", engine::clear(&dirs, &env, false)?);
            Ok(())
        }
        Cmd::Watch => {
            engine::autoheal_if_expired(&dirs, &env)?;
            watch::run(&dirs, &env)
        }
        Cmd::Watchdog { state_path } => {
            fuse::watchdog_run(&PathBuf::from(state_path), watchdog_expire).map_err(Fail::env)
        }
    }
}

// ---------- apply / set ----------

fn do_apply(a: &ApplyArgs, dirs: &Dirs, env: &Env) -> Wn<()> {
    // T12 车端面：weaknet.yaml = 缺省供给源（优先级 CLI flag > env > yaml > 文档化缺省）。
    let cfg = config::load(a.config.as_deref())?;
    let iface = resolve_iface(
        a.iface
            .as_deref()
            .or_else(|| cfg.as_ref().and_then(|c| c.iface_or())),
    );
    let base = match a.profile.as_deref() {
        Some(p) => load_profile(p)?,
        None => cfg
            .as_ref()
            .and_then(|c| c.spec.clone())
            .unwrap_or_default(),
    };
    let spec = merge_spec(&base, a)?;
    let duration = a
        .duration
        .or_else(|| cfg.as_ref().and_then(|c| c.duration))
        .unwrap_or(DEFAULT_DURATION);
    spec::validate_duration(duration).map_err(Fail::bad_param)?;
    let mut ports_flag = parse_ports(a.rtp_port.as_deref())?;
    if ports_flag.is_empty() && let Some(p) = cfg.as_ref().and_then(|c| c.ports.clone()) {
        ports_flag = p;
    }
    if a.dry_run {
        return print_dry_run(env, &spec, &iface, &ports_flag, a.signaling_port,
            &format!("{duration}s（惰性自愈+watchdog；--forever 豁免位未暴露）"),
        );
    }
    let kind = engine::iface_kind_of(&iface);
    let targeting = match resolve_targeting(a, ports_flag.clone()) {
        Ok(t) => t,
        Err(mut e) => {
            // 反锁死规则层：物理口 + 无显式端口供给（flag/yaml 皆空、stats 也未解析成）→ 枚举指引报因。
            if let Some(hint) =
                config::physical_ports_hint(kind == IfaceKind::Physical, !ports_flag.is_empty())
            {
                e.msg.push_str(hint);
            }
            return Err(e);
        }
    };
    // 操作层反锁死（design §车端面③）：物理口施加前打印影响面。
    if kind == IfaceKind::Physical {
        let impact = if targeting.pairs.is_empty() {
            targeting
                .ports
                .iter()
                .map(u16::to_string)
                .collect::<Vec<_>>()
                .join(",")
        } else {
            format!(
                "pairs {}",
                targeting
                    .pairs
                    .iter()
                    .map(|(l, r)| format!("{l}:{r}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            )
        };
        println!("影响面: {iface}/UDP/{impact}/倒计时 {duration}s（撤损: weaknet down）");
    }
    let req = ApplyRequest {
        spec: spec.clone(),
        scope: targeting.scope,
        names: targeting.scope_names(),
        iface: iface.clone(),
        ports: targeting.ports.clone(),
        pairs: targeting.pairs.clone(),
        sig_port: a.signaling_port,
    };
    let out = engine::replay(
        &req,
        Replay::Apply { duration_secs: duration },
        dirs,
        env,
        Verify::Inline,
    )?;
    let scope_detail = match req.scope {
        ScopeSel::Media => format!(
            "ports=[{}]",
            req.ports.iter().map(u16::to_string).collect::<Vec<_>>().join(" "),
        ),
        _ => format!(
            "pairs=[{}]",
            req.pairs
                .iter()
                .map(|(l, r)| format!("{l}:{r}"))
                .collect::<Vec<_>>()
                .join(" "),
        ),
    };
    println!(
        "applied: dev={iface} spec=[{}] {scope_detail} filters={} channel={} 回读指纹过 ✓",
        out.spec_string,
        out.filter_count,
        out.channel,
    );
    println!(
        "verify leaf {}→{} 包 ✓ 生效RTT≈{}ms（iface=lo 时 2× 折算）保险丝={duration}s（惰性自愈+watchdog）",
        out.leaf.0, out.leaf.1, spec.rtt_ms
    );
    Ok(())
}

fn do_set(a: &ApplyArgs, dirs: &Dirs, env: &Env) -> Wn<()> {
    if a.config.is_some() {
        return Err(Fail::bad_param(
            "set 不消费 --config（weaknet.yaml 是 apply 的缺省供给源；set 基底=活跃 state）",
        ));
    }
    let prior = State::read_from(&dirs.state_json())
        .map_err(Fail::env)?
        .ok_or_else(|| Fail::bad_param("无 state（先 apply；set 不创建新现场）"))?;
    if !any_override(a) {
        return Err(Fail::bad_param("set 需至少一个覆盖参数（-h 看帮助）"));
    }
    let iface = a.iface.clone().unwrap_or_else(|| prior.iface.clone());
    let spec = merge_spec(&prior.spec, a)?;
    let dry_ports = if a.rtp_port.is_some() {
        parse_ports(a.rtp_port.as_deref())?
    } else {
        prior.ports.clone()
    };
    let sig = a.signaling_port.or(prior.sig_port);
    if a.dry_run {
        return print_dry_run(env, &spec, &iface, &dry_ports, sig, "不变(set 不续命)");
    }
    // 重定向（--rtp-port/--stream/--device 任一）= 重新解析；否则沿用基底 state 的
    // scope/ports/pairs（T5：解析结果已入盘，replay 免重复拉 stats）。
    let targeting =
        if a.rtp_port.is_some() || a.stream.is_some() || a.device.is_some() {
            resolve_targeting(a, parse_ports(a.rtp_port.as_deref())?)?
        } else {
            Targeting {
                scope: prior.scope,
                ports: prior.ports.clone(),
                pairs: prior.pairs.clone(),
                names: match prior.scope {
                    ScopeSel::Stream => prior.rooms.clone(),
                    ScopeSel::Device => prior.devices.clone(),
                    ScopeSel::Media => vec![],
                },
            }
        };
    if targeting.ports.is_empty() && targeting.pairs.is_empty() {
        return Err(Fail::env(
            "无可用媒体口/配对（基底 state 为空且未给 --rtp-port/--stream/--device）——重新 apply",
        ));
    }
    let before = state::param_summary(&prior.spec);
    let req = ApplyRequest {
        spec: spec.clone(),
        scope: targeting.scope,
        names: targeting.scope_names(),
        iface,
        ports: targeting.ports,
        pairs: targeting.pairs,
        sig_port: sig,
    };
    let out = engine::replay(
        &req,
        Replay::Set {
            prior: Box::new(prior),
            before: before.clone(),
        },
        dirs,
        env,
        Verify::Inline,
    )?;
    println!(
        "set: [{before}] → [{}]（counters_reset={}，root 未回滚·保险丝不续命）",
        state::param_summary(&spec),
        if matches!(out.counters_reset, Some(true)) { "yes" } else { "no" }
    );
    Ok(())
}

fn any_override(a: &ApplyArgs) -> bool {
    a.rtt.is_some()
        || a.jitter.is_some()
        || a.loss.is_some()
        || a.gemodel.is_some()
        || a.reorder.is_some()
        || a.rate.is_some()
        || a.seed.is_some()
        || a.dir.is_some()
        || a.signaling_port.is_some()
        || a.profile.is_some()
        || a.rtp_port.is_some()
        || a.stream.is_some()
        || a.device.is_some()
        || a.jitter.is_some()
        || a.loss.is_some()
        || a.gemodel.is_some()
        || a.reorder.is_some()
        || a.rate.is_some()
        || a.seed.is_some()
        || a.dir.is_some()
        || a.signaling_port.is_some()
        || a.profile.is_some()
}

/// 基底 + 显式 flag 覆盖（bash 优先级：flag > profile；值域校验镜像 exit-4 表）。
fn merge_spec(base: &ImpairSpec, a: &ApplyArgs) -> Wn<ImpairSpec> {
    let mut s = base.clone();
    if let Some(v) = a.rtt {
        s.rtt_ms = v;
    }
    if let Some(v) = a.jitter {
        s.jitter_ms = v;
    }
    if let Some(v) = &a.reorder {
        s.reorder_pct = Some(v.clone());
    }
    if let Some(v) = a.rate {
        s.rate_mbps = Some(v);
    }
    if let Some(v) = a.seed {
        s.seed = Some(v);
    }
    if let Some(v) = a.dir {
        s.dir = v.into();
    }
    let mode = a.loss_mode.as_deref().unwrap_or("simple");
    if let Some(g) = &a.gemodel {
        let loss = a.loss.clone().or_else(|| match &s.loss {
            Some(LossSpec::GeModel { loss, .. }) => Some(loss.clone()),
            Some(LossSpec::Simple(p)) if !p.is_empty() => Some(p.clone()),
            _ => None,
        });
        let Some(loss) = loss else {
            return Err(Fail::bad_param(
                "gemodel 需配 --loss（p13，profile 或 flag 均可）",
            ));
        };
        s.loss = Some(LossSpec::GeModel {
            loss,
            r: g[0].clone(),
            h: g[1].clone(),
            k: g[2].clone(),
        });
    } else if let Some(p) = &a.loss {
        match mode {
            "simple" | "uniform" => s.loss = Some(LossSpec::Simple(p.clone())),
            "gemodel" => {
                return Err(Fail::bad_param("loss_mode=gemodel 需配 --gemodel r h k 三参"));
            }
            other => {
                return Err(Fail::bad_param(format!(
                    "loss_mode 需 simple|gemodel，得 {other:?}"
                )));
            }
        }
    }
    s.validate().map_err(Fail::bad_param)?;
    Ok(s)
}


fn resolve_iface(cli: Option<&str>) -> String {
    engine::resolve_iface(cli)
}

fn parse_ports(s: Option<&str>) -> Wn<Vec<u16>> {
    let Some(raw) = s.map(str::trim).filter(|t| !t.is_empty()) else {
        return Ok(vec![]);
    };
    raw.split(',')
        .map(|p| {
            p.trim()
                .parse::<u16>()
                .map_err(|e| Fail::bad_param(format!("--rtp-port 端口非法 {p:?}: {e}")))
        })
        .collect()
}

/// 作用域裁决（T6 提取：核心入 scope::resolve_targeting 与 serve REST 共源，此处仅字段搬运）。
fn resolve_targeting(a: &ApplyArgs, ports_flag: Vec<u16>) -> Wn<Targeting> {
    scope::resolve_targeting(
        a.server_url.as_deref(),
        a.stream.as_deref(),
        a.device.as_deref(),
        &ports_flag,
    )
}


fn print_dry_run(
    env: &Env,
    spec: &ImpairSpec,
    iface: &str,
    ports: &[u16],
    sig: Option<u16>,
    duration_note: &str,
) -> Wn<()> {
    // T9：与 replay 同构的腿切分（物理口 in/both = egress 只留 Out 形，上行腿走 ifb 展示段）。
    let kind = engine::iface_kind_of(iface);
    let use_ifb = engine::ifb_needed(kind, spec.dir);
    let legs = if !use_ifb {
        spec::build_filter_legs(ScopeSel::Media, kind, spec.dir, ports, &[]).unwrap_or_default()
    } else if spec.dir == SpecDir::Both {
        spec::build_filter_legs(ScopeSel::Media, kind, SpecDir::Out, ports, &[]).unwrap_or_default()
    } else {
        Vec::new()
    };
    let ingress_legs = if use_ifb {
        spec::build_filter_legs(ScopeSel::Media, kind, SpecDir::In, ports, &[]).unwrap_or_default()
    } else {
        Vec::new()
    };
    let spec_string = spec::render_netem_spec_leg(spec);
    let rate = spec::render_rate_arg(spec.rate_mbps);
    let install_skeleton = !use_ifb || !legs.is_empty() || sig.is_some();
    let mut steps = Vec::new();
    if install_skeleton {
        steps.extend(engine::plan_skeleton(iface, true, &spec_string, &rate, &legs, sig));
    }
    if use_ifb {
        // 先读后判不可有副作用——展示全链形（need_link=true 含建链+up 步）。
        steps.extend(engine::plan_ifb(iface, &spec_string, &ingress_legs, true));
    }
    let prefix = engine::dryrun_prefix(env);
    let ip_prefix = engine::dryrun_prog_prefix(env, engine::Prog::Ip);
    println!("[dry] 等效命令序列（root 缺席走 add 形展示；change || add = 2015 兼容；# = best-effort）：");
    for s in &steps {
        let alt = s
            .alt
            .as_ref()
            .map_or_else(String::new, |a| format!(" || {}", a.join(" ")));
        let note = if s.best_effort { "   # best-effort" } else { "" };
        let pfx = if s.prog == engine::Prog::Ip { &ip_prefix } else { &prefix };
        println!("{pfx} {}{alt}{note}", s.run.join(" "));
    }
    if legs.is_empty() && ingress_legs.is_empty() {
        println!("（媒体腿/配对在执行时解析：--stream/--device/--rtp-port 或 stats 观测）");
    }
    println!("[dry] 保险丝: {duration_note} auto-clear=0（信号层默认关）");
    Ok(())
}

// ---------- status ----------

fn do_status(dirs: &Dirs, env: &Env) -> Wn<()> {
    let st = State::read_from(&dirs.state_json()).map_err(Fail::env)?;
    let dev = st
        .as_ref()
        .map(|s| s.iface.clone())
        .unwrap_or_else(|| resolve_iface(None));
    // 假绿防线（bash do_status）：tc 通道不可读 ≠ 无 qdisc——probe/exec 失败自带 exit2 报因。
    let chan = engine::probe_channel(env, &dev)?;
    // T9 回读路由：ifb 纯上行会话的现场在 ifb0 root（iface root 未动）。
    let (rd_dev, rd_form) = match &st {
        Some(s) if s.ifb_used && s.dir == SpecDir::In && s.sig_port.is_none() => {
            (
                mediaservo_weaknet::ifb::IFB_DEV.to_string(),
                engine::LeafForm::Root,
            )
        }
        _ => (dev.clone(), engine::LeafForm::Parent110),
    };
    let show = engine::tc_exec(&chan, &["qdisc", "show", "dev", &rd_dev])?;
    let observed: Vec<&str> = show
        .lines()
        .filter(|l| l.contains("netem") || l.contains("htb"))
        .take(3)
        .collect();
    match (&st, observed.is_empty()) {
        (None, true) => println!("inactive：{dev} 无 weaknet qdisc"),
        (Some(_), true) => {
            return Err(Fail::conflict(
                "state 在但 qdisc 不在（外部清道？watchdog？）——先 weaknet clear 复位",
            ));
        }
        (None, false) => {
            return Err(Fail::conflict(format!(
                "qdisc 残留而 state 缺（外部/手挂 或 猝死超期）：{} —— clear 前自证",
                observed[0]
            )));
        }
        (Some(s), false) => {
            // 交叉判定（design §Error handling）：state × tc 指纹，分歧 = 被外部改写 WARN。
            if let Err(e) = engine::assert_fingerprint_form(
                &show,
                &spec::render_netem_spec_leg(&s.spec),
                rd_form,
            )
            {
                eprintln!(
                    "weaknet: WARN qdisc 被外部改写（另一实例/手工 tc/另一 clone）：{}",
                    e.msg
                );
            }
            let rem = if s.is_forever() {
                "forever".to_string()
            } else {
                let now = fuse::now_epoch_ms();
                if now >= s.expires_at_ms {
                    "已过期（惰性自愈将清）".to_string()
                } else {
                    format!("{}s", (s.expires_at_ms - now) / 1000)
                }
            };
            println!(
                "== status（state 摘要，channel={} iface={dev} 剩余 {rem}，watchdog {}）==",
                chan.describe(),
                watchdog_state(dirs)
            );
            println!("{}", state::param_summary(&s.spec));
            let scope_detail = if s.pairs.is_empty() {
                format!(
                    "ports=[{}]",
                    s.ports.iter().map(u16::to_string).collect::<Vec<_>>().join(" "),
                )
            } else {
                format!(
                    "pairs=[{}]",
                    s.pairs
                        .iter()
                        .map(|(l, r)| format!("{l}:{r}"))
                        .collect::<Vec<_>>()
                        .join(" "),
                )
            };
            println!(
                "scope={:?} dir={:?} {scope_detail} teardown {} 步",
                s.scope,
                s.dir,
                s.teardown.steps.len()
            );
            println!("== tc 实况（{dev}）==");
            for l in &observed {
                println!("{l}");
            }
            if let Ok(classes) = engine::tc_exec(&chan, &["class", "show", "dev", &dev]) {
                for l in classes.lines().take(3) {
                    println!("{l}");
                }
            }
            if let Ok(v) = engine::tc_exec(&chan, &["-V"]) {
                println!("== tc 版本 ==\n{}", v.trim());
            }
            if let Ok(raw) = std::fs::read_to_string(dirs.timeline()) {
                let lines: Vec<&str> = raw.lines().collect();
                println!("== timeline 尾 3 ==");
                for l in lines.iter().rev().take(3).rev() {
                    println!("{l}");
                }
            }
        }
    }
    Ok(())
}

/// watchdog 存活摘要（真值源已上提 watch.rs，与 --watch 共用同一判据）。
fn watchdog_state(dirs: &Dirs) -> String {
    watch::watchdog_value(dirs).into()
}

// ---------- scenario ----------

fn do_scenario(sub: ScenarioCmd, dirs: &Dirs, env: &Env) -> Wn<()> {
    match sub {
        ScenarioCmd::Run {
            file,
            dir,
            no_baseline,
            keep,
            auto_clear,
        } => {
            let file = file.ok_or_else(|| {
                Fail::bad_param("scenario run 需 --file（<名|*.yaml> [--no-baseline|--keep]；-h 看帮助）")
            })?;
            let (raw, name, disp) = scenario::read_run_yaml(&file, dir.as_deref())?;
            let plan = scenario::parse_plan_yaml(&raw).map_err(Fail::bad_param)?;
            // 定向 = 段级 stats（bash do_scenario 不给 --stream；缺省同 do_apply）
            let targeting = scope::resolve_targeting(None, None, None, &[])?;
            let rs = scenario::RunSpec {
                name,
                file_disp: disp,
                plan,
                no_baseline,
                keep,
                targeting,
                iface: engine::resolve_iface(None),
                sig_port: None,
                recovery_dwell_secs: None,
            };
            let exec: Arc<dyn scenario::StepExec> =
                Arc::new(scenario::EngineExec::new(dirs.clone(), env.clone()));
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| Fail::env(format!("tokio runtime 建立失败: {e}")))?;
            // serve 侧 /v1/scenario/run 不经信号层（长驻进程，停止面已有 /v1/scenario/stop
            // 端点）——--auto-clear 仅作用于本 CLI 前台路径（out of scope 注记，票面裁 4）。
            let out = if auto_clear {
                rt.block_on(scenario::run_full_auto_clear(rs, exec, dirs.clone()))
            } else {
                rt.block_on(scenario::run_full(rs, exec, dirs.clone()))
            };
            println!("{}", out.summary);
            if out.exit_code != 0 {
                return Err(Fail {
                    code: out.exit_code,
                    msg: out.aborted.unwrap_or_else(|| "scenario 失败（见上）".into()),
                });
            }
            Ok(())
        }
        ScenarioCmd::List => scenario_list(),
        ScenarioCmd::Stop => {
            println!("{}", scenario::stop(dirs)?);
            Ok(())
        }
    }
}

fn scenario_list() -> Wn<()> {
    // T13 §车端面：fs 探测链 ∪ 编译期内嵌（单一源 = scope::list_assets，与 serve /v1/scenarios 同形）。
    let names = scope::list_assets("scenarios");
    if names.is_empty() {
        return Err(Fail::env(
            "无 scenario 资产（WEAKNET_ASSETS_DIR > 二进制同级 weaknet.d > 祖先 scripts/weaknet.d > 内嵌 皆空）",
        ));
    }
    for n in names {
        println!("{n}");
    }
    Ok(())
}

/// profile/scenario 资产寻径（T6 上提 lib 面 scope::weaknet_d_root——CLI/serve 单一真值源）。
fn weaknet_d_root() -> Option<PathBuf> {
    scope::weaknet_d_root()
}

// ---------- profile（emit.py 白名单语义的 serde_yaml 平移） ----------

fn load_profile(name: &str) -> Wn<ImpairSpec> {
    let parse = |raw: &str, disp: &str| -> Wn<ImpairSpec> {
        let doc: Value = serde_yaml::from_str(raw)
            .map_err(|e| Fail::bad_param(format!("profile YAML 非法 {disp}: {e}")))?;
        config::spec_from_profile_doc(&doc).map_err(Fail::bad_param)
    };
    // 1) 文件形（带扩展名）：直读 fs（bash 同位）。
    if Path::new(name).extension().is_some_and(|e| e == "yaml" || e == "yml") {
        let p = PathBuf::from(name);
        if !p.is_file() {
            return Err(Fail::bad_param(format!("无 profile 文件: {name}")));
        }
        let raw = std::fs::read_to_string(&p)
            .map_err(|e| Fail::env(format!("读 profile {} 失败: {e}", p.display())))?;
        return parse(&raw, &p.display().to_string());
    }
    // 2) 名形：fs 资产目录优先（§车端面寻径单函数 = scope::read_asset）。
    let fs_hit = weaknet_d_root()
        .map(|root| root.join("profiles").join(format!("{name}.yaml")))
        .filter(|p| p.is_file());
    if let Some(p) = fs_hit {
        let raw = std::fs::read_to_string(&p)
            .map_err(|e| Fail::env(format!("读 profile {} 失败: {e}", p.display())))?;
        return parse(&raw, &p.display().to_string());
    }
    // 3) 内嵌兜底（T13：车端 scp 单文件 `up smoke` 零资产依赖；名形才允许，路径穿越不入内嵌）。
    if scenario::is_safe_asset_name(name) {
        let rel = scenario::asset_rel("profiles", name);
        if let Some(raw) = scope::read_asset(&rel)? {
            return parse(&raw, &format!("内嵌 {rel}"));
        }
    }
    Err(Fail::bad_param(format!(
        "无 profile: {name}（可用: {}；源: WEAKNET_ASSETS_DIR / 二进制同级 weaknet.d / scripts/weaknet.d / 内嵌）",
        scope::list_assets("profiles").join(" ")
    )))
}


// ---------- __watchdog 执行体（design §fuse rev-2.2 跨通道合同） ----------

/// 到点照单执行：读 state.teardown.channel/sidecar → run_teardown（sidecar 亡走三层兜底）→
/// rm state → timeline `watchdog-clear`；失败 → `watchdog-clear-failed`+err（C15，禁静默）；
/// not-exist 类已在 run_teardown 内视同幂等成功（双 watchdog 并发到点防噪声）。
fn watchdog_expire(state_path: &Path) {
    let dirs = Dirs {
        statedir: state_path.parent().unwrap_or(Path::new(".")).to_path_buf(),
    };
    match State::read_from(state_path) {
        Ok(Some(st)) => {
            let failures = engine::run_teardown(&st);
            for f in &failures {
                eprintln!("weaknet watchdog: WARN teardown: {f}");
            }
            if let Err(e) = std::fs::remove_file(state_path)
                && e.kind() != std::io::ErrorKind::NotFound
            {
                eprintln!("weaknet watchdog: WARN 删 state 失败: {e}");
            }
            let ev = if failures.is_empty() {
                json!({"ev": "watchdog-clear"})
            } else {
                json!({"ev": "watchdog-clear-failed", "err": failures.join("; ")})
            };
            if let Err(e) = state::timeline_append(&dirs, ev) {
                eprintln!("weaknet watchdog: WARN timeline 失败: {e}");
            }
        }
        Ok(None) => {
            eprintln!("weaknet watchdog: state 不存在（已被 clear？）——幂等退场");
        }
        Err(e) => {
            eprintln!("weaknet watchdog: WARN {e}");
            if let Err(te) = state::timeline_append(
                &dirs,
                json!({"ev": "watchdog-clear-failed", "err": e}),
            ) {
                eprintln!("weaknet watchdog: WARN timeline 失败: {te}");
            }
        }
    }
}
