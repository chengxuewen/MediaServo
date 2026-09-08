//! mediaservo-weaknet — tc/netem 弱网模拟 agent（双端面单二进制）
//!
//! T1 = 全子命令空壳（flag 面按 rev-2.2 design 逐项定义，逻辑随 T2-T8 接线）。
//! T2 = 隐藏子命令 `__watchdog` 到场（fuse 自我 re-exec 入口）；其余仍为壳。
//! 契约源 = 主仓 docs/plans/weaknet-agent/（§dir 腿定义表 / §API / §ifb 合同）。

use clap::{Parser, Subcommand};
use mediaservo_weaknet::spec::Dir as SpecDir;

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
    /// 覆盖字段（服务端 state 合并，全量重放；无活跃 spec → 409/exit3 语义）
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
    /// 运行剧本（文件按 二进制同级 weaknet.d/scenarios > 内嵌 顺序解析，basename-only 于 REST 面）
    Run {
        #[arg(long)]
        file: Option<String>,
        /// 场景根覆盖（缺省见 §车端面寻径）
        #[arg(long)]
        dir: Option<String>,
    },
    /// 列出可用剧本
    List,
    /// 运行中的剧本停止（孤儿 flag 直接清并 200/exit0）
    Stop,
}

#[derive(Debug, clap::Args)]
struct ApplyArgs {
    /// 命名 profile（weaknet.d/profiles）
    #[arg(long)]
    profile: Option<String>,
    #[arg(long)]
    rtt: Option<u64>,
    #[arg(long)]
    jitter: Option<u64>,
    /// 丢包（"2%" 或 "2"，gemodel 时剥%）
    #[arg(long)]
    loss: Option<String>,
    #[arg(long, default_value = "simple")]
    loss_mode: String,
    /// Gilbert-Elliott 参数组（r h k，% 自动剥除）
    #[arg(long, num_args = 3, value_names = ["R", "H", "K"])]
    gemodel: Option<Vec<String>>,
    #[arg(long)]
    reorder: Option<String>,
    #[arg(long)]
    rate: Option<f64>,
    #[arg(long)]
    seed: Option<u64>,
    /// 保险丝时长（秒，缺省 300——apply 唯一存活承诺，0 不接受）
    #[arg(long)]
    duration: Option<u64>,
    /// 目标网卡（缺省 lo=文档化常量，C20 豁免形；物理口需 ports 枚举）
    #[arg(long)]
    iface: Option<String>,
    /// 方向（缺省 both；腿定义 = design §dir 表）
    #[arg(long, value_enum, default_value = "both")]
    dir: Dir,
    /// 定向：房间名（=流），逗号分隔
    #[arg(long)]
    stream: Option<String>,
    /// 定向：设备（owner 全部流端口并集）
    #[arg(long)]
    device: Option<String>,
    /// 逃生门：显式 RTP 端口集，逗号分隔
    #[arg(long)]
    rtp_port: Option<String>,
    /// 信令 TCP 腿端口（可选，bash --signaling 承接位）
    #[arg(long)]
    signaling_port: Option<u16>,
    /// 参数文件（车端面：iface/ports/duration/spec）
    #[arg(long)]
    config: Option<String>,
    /// 只打印等价命令序列（W1 快照测通路，零内核触达）
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
    match cli.cmd {
        Cmd::Watchdog { state_path } => {
            let path = std::path::PathBuf::from(&state_path);
            let rc = mediaservo_weaknet::fuse::watchdog_run(&path, |p| {
                // TODO(T3/T4): 接 engine teardown（读 state.teardown.channel/sidecar 照单
                // 执行 + sidecar 三层兜底）与 timeline 事件；本版仅占位播报（C15 禁静默）。
                println!(
                    "weaknet watchdog: {} 到期——teardown 接线随 T3/T4",
                    p.display()
                );
            });
            if let Err(e) = rc {
                eprintln!("weaknet watchdog: {e}");
                std::process::exit(2);
            }
        }
        // T1 空壳：其余子命令 print+exit0（flag 面先行冻结，逻辑随 T2-T8 接线）
        other => {
            println!("mediaservo-weaknet(T1 空壳): 收到子命令 {other:?}——实现随里程碑接线");
        }
    }
}
