//! mediaservo-weaknet — tc/netem 弱网模拟 agent（双端面单二进制）
//!
//! T4 = CLI 装配：`apply|set|status|clear` 全链 + `up|down` 别名经 engine/state；
//! 未到场里程碑（serve=T6 / scenario run=T8 / --watch·config=T12）报因 exit2——C15 禁静默。
//! 措辞/退出码真值 = `scripts/weaknet.sh`（至退役日）；契约源 = 主仓 docs/plans/weaknet-agent/。
//! 退出码：0 OK / 2 环境不足·施加失败 / 3 状态冲突（锁·他方 qdisc·state 背离）/ 4 参数非法。

use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};
use serde_json::{Value, json};

use mediaservo_weaknet::engine::{self, ApplyRequest, Env, Fail, Replay, Verify, Wn};
use mediaservo_weaknet::fuse;
use mediaservo_weaknet::scope::{self, Targeting};
use mediaservo_weaknet::server;
use mediaservo_weaknet::spec::{self, Dir as SpecDir, ImpairSpec, LossSpec, ScopeSel};
use mediaservo_weaknet::state::{self, Dirs, State};

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
    /// 运行剧本（steps 调度/job 独占随 T8）
    Run {
        #[arg(long)]
        file: Option<String>,
        /// 场景根覆盖（缺省见 §车端面寻径）
        #[arg(long)]
        dir: Option<String>,
    },
    /// 列出可用剧本
    List,
    /// 运行中的剧本停止（job 状态面随 T8）
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
    /// 参数文件 weaknet.yaml（车端面，T12 到场）
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
        Cmd::Apply(a) => {
            let _lk = engine::take_write_lock(&dirs)?;
            do_apply(&a, &dirs, &env)
        }
        Cmd::Set(a) => {
            let _lk = engine::take_write_lock(&dirs)?;
            engine::autoheal_if_expired(&dirs, &env)?; // apply/clear 免检、其余开场先验（bash）
            do_set(&a, &dirs, &env)
        }
        Cmd::Scenario { step } => do_scenario(step),
        Cmd::Status { watch } => {
            if watch {
                return Err(Fail::env("status --watch 一屏重绘随 T12（watch.rs）到场"));
            }
            engine::autoheal_if_expired(&dirs, &env)?;
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
        Cmd::Watch => Err(Fail::env("watch 一屏重绘随 T12（status --watch 同路径）到场")),
        Cmd::Watchdog { state_path } => {
            fuse::watchdog_run(&PathBuf::from(state_path), watchdog_expire).map_err(Fail::env)
        }
    }
}

// ---------- apply / set ----------

fn do_apply(a: &ApplyArgs, dirs: &Dirs, env: &Env) -> Wn<()> {
    reject_deferred(a)?;
    let iface = resolve_iface(a.iface.as_deref());
    let base = match a.profile.as_deref() {
        Some(p) => load_profile(p)?,
        None => ImpairSpec::default(),
    };
    let spec = merge_spec(&base, a)?;
    let duration = a.duration.unwrap_or(DEFAULT_DURATION);
    spec::validate_duration(duration).map_err(Fail::bad_param)?;
    let ports_flag = parse_ports(a.rtp_port.as_deref())?;
    if a.dry_run {
        return print_dry_run(env, &spec, &iface, &ports_flag, a.signaling_port,
            &format!("{duration}s（惰性自愈+watchdog；--forever 豁免位未暴露）"));
    }
    let targeting = resolve_targeting(a, ports_flag)?;
    let req = ApplyRequest {
        spec: spec.clone(),
        scope: targeting.scope,
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
    reject_deferred(a)?;
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

fn reject_deferred(a: &ApplyArgs) -> Wn<()> {
    // T5 到场：--stream/--device 已由 scope.rs 承接（resolve_targeting），此处仅剩 --config。
    if a.config.is_some() {
        return Err(Fail::env(
            "--config（weaknet.yaml 车端面）随 T12（config.rs）到场",
        ));
    }
    Ok(())
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
    let legs = spec::build_filter_legs(
        ScopeSel::Media,
        engine::iface_kind_of(iface),
        spec.dir,
        ports,
        &[],
    )
    .unwrap_or_default();
    let spec_string = spec::render_netem_spec_leg(spec);
    let rate = spec::render_rate_arg(spec.rate_mbps);
    let steps = engine::plan_skeleton(iface, true, &spec_string, &rate, &legs, sig);
    let prefix = engine::dryrun_prefix(env);
    println!("[dry] 等效命令序列（root 缺席走 add 形展示；change || add = 2015 兼容；# = best-effort）：");
    for s in &steps {
        let alt = s
            .alt
            .as_ref()
            .map_or_else(String::new, |a| format!(" || {}", a.join(" ")));
        let note = if s.best_effort { "   # best-effort" } else { "" };
        println!("{prefix} {}{alt}{note}", s.run.join(" "));
    }
    if legs.is_empty() {
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
    let show = engine::tc_exec(&chan, &["qdisc", "show", "dev", &dev])?;
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
            if let Err(e) = engine::assert_fingerprint(&show, &spec::render_netem_spec_leg(&s.spec))
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

fn watchdog_state(dirs: &Dirs) -> String {
    match fuse::read_pidfile(&dirs.watchdog_pid()) {
        Ok(None) => "none".into(),
        Ok(Some(e)) => {
            let alive = Path::new(&format!("/proc/{}", e.pid)).exists()
                && fuse::read_proc_starttime(e.pid) == Some(e.starttime);
            if alive { "ok".into() } else { "dead（仅剩惰性自愈层）".into() }
        }
        Err(e) => {
            eprintln!("weaknet: WARN {e}");
            "协议违例".into()
        }
    }
}

// ---------- scenario ----------

fn do_scenario(sub: ScenarioCmd) -> Wn<()> {
    match sub {
        ScenarioCmd::Run { .. } => Err(Fail::env(
            "scenario run（steps 调度/job 独占/baseline 配对）随 T8 到场",
        )),
        ScenarioCmd::Stop => Err(Fail::env("scenario stop 随 T8（job 状态面）到场")),
        ScenarioCmd::List => scenario_list(),
    }
}

fn scenario_list() -> Wn<()> {
    let root = weaknet_d_root().ok_or_else(|| {
        Fail::env(
            "无 scenario 资产目录（探测链：WEAKNET_ASSETS_DIR > 二进制同级 weaknet.d > cwd/二进制祖先 scripts/weaknet.d）",
        )
    })?;
    let dir = root.join("scenarios");
    if !dir.is_dir() {
        return Err(Fail::env(format!(
            "scenario 目录不存在：{}（设 WEAKNET_ASSETS_DIR 指向 weaknet.d 资产根）",
            dir.display()
        )));
    }
    let mut names: Vec<String> = Vec::new();
    let rd = std::fs::read_dir(&dir)
        .map_err(|e| Fail::env(format!("读 {} 失败: {e}", dir.display())))?;
    for e in rd {
        let e = e.map_err(|er| Fail::env(format!("read_dir 条目失败: {er}")))?;
        let p = e.path();
        let is_yaml = p
            .extension()
            .is_some_and(|x| x == "yaml" || x == "yml");
        if is_yaml && p.is_file() && let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
            names.push(stem.to_string());
        }
    }
    names.sort();
    if names.is_empty() {
        println!("（{} 无 scenario yaml）", dir.display());
    } else {
        for n in names {
            println!("{n}");
        }
    }
    Ok(())
}

/// profile/scenario 资产寻径（T6 上提 lib 面 scope::weaknet_d_root——CLI/serve 单一真值源）。
fn weaknet_d_root() -> Option<PathBuf> {
    scope::weaknet_d_root()
}

// ---------- profile（emit.py 白名单语义的 serde_yaml 平移） ----------

fn load_profile(name: &str) -> Wn<ImpairSpec> {
    let path: PathBuf = if Path::new(name).extension().is_some_and(|e| {
        e == "yaml" || e == "yml"
    }) {
        let p = PathBuf::from(name);
        if p.is_file() {
            p
        } else {
            return Err(Fail::bad_param(format!("无 profile 文件: {name}")));
        }
    } else {
        let root = weaknet_d_root().ok_or_else(|| {
            Fail::env(format!(
                "无 profile '{name}'（且无资产目录：WEAKNET_ASSETS_DIR / 二进制同级 weaknet.d / scripts/weaknet.d）"
            ))
        })?;
        let pdir = root.join("profiles");
        let p = pdir.join(format!("{name}.yaml"));
        if !p.is_file() {
            let avail = std::fs::read_dir(&pdir)
                .map(|rd| {
                    rd.filter_map(|e| {
                        e.ok().and_then(|e| {
                            let x = e.path();
                            (x.extension().is_some_and(|s| s == "yaml")
                                .then(|| x.file_stem().and_then(|s| s.to_str().map(str::to_string))))
                            .flatten()
                        })
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
                })
                .unwrap_or_default();
            return Err(Fail::bad_param(format!(
                "无 profile: {name}（可用: {avail}；目录 {}）",
                pdir.display()
            )));
        }
        p
    };
    let raw = std::fs::read_to_string(&path)
        .map_err(|e| Fail::env(format!("读 profile {} 失败: {e}", path.display())))?;
    let doc: Value = serde_yaml::from_str(&raw)
        .map_err(|e| Fail::bad_param(format!("profile YAML 非法 {}: {e}", path.display())))?;
    profile_to_spec(&doc).map_err(Fail::bad_param)
}

/// emit.py `profile` 消费形 → from_flat_json 键形：`reorder_pct→reorder`、`gemodel{r,h,k}→[r,h,k]`、
/// loss 数值 stringify；`scope`/`clear` 非参数字段剥离（signaling=true 时 WARN——T5 消费）。
fn profile_to_spec(doc: &Value) -> Result<ImpairSpec, String> {
    if doc
        .get("scope")
        .and_then(|s| s.get("signaling"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        eprintln!("weaknet: WARN profile scope.signaling=true——信令腿消费随 T5 到场（当前用 --signaling-port 显式）");
    }
    let mut flat = serde_json::Map::new();
    for k in ["rtt_ms", "jitter_ms", "rate_mbps", "seed"] {
        if let Some(v) = doc.get(k) {
            flat.insert(k.to_string(), v.clone());
        }
    }
    if let Some(l) = doc.get("loss") {
        flat.insert("loss".to_string(), stringify(l));
    }
    if let Some(r) = doc.get("reorder_pct") {
        flat.insert("reorder".to_string(), stringify(r));
    }
    if let Some(g) = doc.get("gemodel") {
        let trio: Vec<Value> = ["r", "h", "k"]
            .iter()
            .filter_map(|k| g.get(*k))
            .map(stringify)
            .collect();
        if trio.len() != 3 {
            return Err("gemodel 需 dict{r,h,k}".to_string());
        }
        flat.insert("gemodel".to_string(), Value::Array(trio));
        flat.insert("loss_mode".to_string(), Value::String("gemodel".into()));
    }
    ImpairSpec::from_flat_json(&Value::Object(flat))
}

fn stringify(v: &Value) -> Value {
    match v {
        Value::String(s) => Value::String(s.clone()),
        other => Value::String(other.to_string()),
    }
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
