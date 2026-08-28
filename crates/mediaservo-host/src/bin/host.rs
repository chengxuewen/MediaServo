//! host CLI: 业务视图运维入口（Phase A）。
//!
//! 子命令（`std::env::args` 手工解析，Phase A 不引入 clap）：
//! - `host init [<dir>]`        — 生成 `etc/host.yaml` 模板 + `etc/link/signing.pem`
//!   （Ed25519 PKCS#8 keypair，0600）+ `etc/link/ros_bridge.yaml`（B3：ROS 节点
//!   配置单一来源——topic 清单 + 令牌路径，从 host.yaml 相机/流清单导出）
//!   + 标准车辆令牌集自动签发（G1：`host token issue --all`，幂等）
//! - `host start [<dir>]` — 读 etc/host.yaml → 校验 → 翻译 → 写 run/oxfile.toml
//!   → `oxmgr apply run/oxfile.toml` 拉起全部 host 进程
//! - `host apply [<dir>]`  — 同上（E4 云端配置闭环本地等价物；增量: 新增 Start/
//!   变更 Recreate/未变 Noop）
//! - `host restart [<dir>]`— `oxmgr stop <oxfile>` 后重新 apply（全量重启）
//! - `host stop [<dir>]`  — `oxmgr stop run/oxfile.toml` + `oxmgr delete run/oxfile.toml`
//! OxMgr 动词核对（C11/C18，来源 .refinfo/OxMgr/docs/CLI.md + SKILL.md）：
//! `apply <config>` / `list [--json]` / `stop <name|id|config>` / `delete <name|id|config>`
//! （**无** `stop/delete --namespace` 旗标；config 目标自动解析 oxfile 内全部 app，
//! 见 CLI.md "Lifecycle" 段）。

use std::path::{Path, PathBuf};
use std::process::Command;

use mediaservo_link::{
    CapabilityToken, Ed25519SigningKey, Ed25519VerifyingKey, FrameTopic, NodeAcl, NodeId, Role, TokenFile,
};
use ed25519_dalek::pkcs8::{DecodePrivateKey, EncodePublicKey};
use pkcs8::LineEnding;

/// `host init` 生成的配置模板（host.yaml 初版 schema，A1）。
const HOST_TOML_TEMPLATE: &str = include_str!("../host.yaml.template");

const USAGE: &str = "用法: mediaservo-host <init|start|apply|restart|stop|status|doctor|token|startup|monit|ps|logs|version> [<dir>]（目录为位置参数，默认 .host/）
子命令:
  init [<dir>]       生成实例：host.yaml 模板 + signing.pem(0600) + identity.json + 标准令牌集
  start [<dir>]      读 host.yaml → 校验 → 翻译 oxfile → oxmgr apply 拉起全部进程
  apply [<dir>]      配置变更生效（增量: 新增 Start/变更 Recreate/未变 Noop）
  restart [<dir>]    全量重启（oxmgr stop 后重新 apply）
  stop [<dir>]       全部停止（oxmgr stop + delete）
  status [<dir>]     查看进程状态表（oxmgr list 过滤 host 命名空间）
  doctor [<dir>]     环境诊断（oxmgr/配置/翻译三检查，退出码=失败数）
  token issue --role R --node N [--topic T] --out O [<dir>]   签发链路令牌
  token --all [<dir>]  签发标准令牌集（相机/流/recorder/agent）
  startup on/off/status [<dir>]             开机自启（on=启用/off=停用，三端: systemd/launchd/Task Scheduler）
  monit               打开 oxmgr TUI（进程监控/日志/重启——需 oxmgr 在 PATH）
  ps                  进程列表（oxmgr list——含 CPU/RAM/uptime 列）
  logs [<proc>]       oxmgr 日志查看（logs all = 全部进程）
  version            版本信息

示例:
  mediaservo-host start /opt/mediaservo-host    启动部署实例
  mediaservo-host -h                            本帮助";

fn print_usage() {
    let b = mediaservo_common::brand::media_brand();
    // 默认品牌 product == "mediaservo-host"——replace 为 no-op（零行为变化门禁）
    println!("{}", USAGE.replace("mediaservo-host", b.product));
}

fn eprint_usage() {
    let b = mediaservo_common::brand::media_brand();
    eprintln!("{}", USAGE.replace("mediaservo-host", b.product));
}

fn main() {
    let mut args = std::env::args();
    let _prog = args.next();
    let Some(cmd) = args.next() else {
        print_usage();
        return;
    };
    if cmd == "-h" || cmd == "--help" {
        print_usage();
        return;
    }
    let code = match cmd.as_str() {
        "init" => cmd_init(&mut args),
        "start" => cmd_apply_impl(&mut args, "start"),
        "apply" => cmd_apply_impl(&mut args, "apply"),
        "restart" => cmd_restart(&mut args),
        "stop" => cmd_stop(&mut args),
        "status" => cmd_status(&mut args),
        "doctor" => cmd_doctor(&mut args),
        "token" => cmd_token(&mut args),
        // oxmgr 代理（方式 D: host 目录内统一入口）— oxmgr 需在 PATH（install 打包于 bin/）
        "startup" => cmd_startup(&mut args),
        "monit" => cmd_oxmgr(&mut args, &["ui"]),
        "ps" => cmd_oxmgr(&mut args, &["list"]),
        "logs" => cmd_oxmgr(&mut args, &["logs"]),
        "version" => {
            println!("{} {}", mediaservo_common::brand::media_brand().product, env!("CARGO_PKG_VERSION"));
            0
        }
        _ => {
            eprintln!("未知子命令: {cmd}");
            eprint_usage();
            2
        }
    };
    std::process::exit(code);
}

/// 解析子命令的实例目录参数（**位置参数**，缺省 `.host/`）。
///
/// 守卫: 目录是位置参数而非 flag——任何以 `--` 开头的参数直接拒绝。
/// （`host init --dir` 事故: "--dir" 曾被当作路径创建目录；根因是 flag/位置
/// 参数不一致，目录统一位置参数后 -- 前缀必为用户笔误。）
fn parse_dir(args: &mut impl Iterator<Item = String>) -> Result<PathBuf, String> {
    let Some(arg) = args.next() else {
        // 智能默认（安装即用）: ① cwd 有实例（etc/host.yaml）→ "."；
        // ② 二进制同目录有实例（install 布局: bin/ 旁 etc/host.yaml）→ 二进制目录；
        // ③ 否则 .host/（旧式根目录实例兼容）
        if PathBuf::from("etc/host.yaml").exists() {
            return Ok(PathBuf::from("."));
        }
        // ② 二进制同目录/上级有实例（install 布局: <inst>/bin/msrtc-host + <inst>/etc/host.yaml）
        //    → 定位安装根，实现「任意目录执行都指向本机实例」
        if let Some(exe_dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())) {
            if exe_dir.join("etc").join("host.yaml").exists() {
                return Ok(exe_dir.clone());
            }
            if let Some(inst_root) = exe_dir.parent() {
                if inst_root.join("etc").join("host.yaml").exists() {
                    return Ok(inst_root.to_path_buf());
                }
            }
        }
        return Ok(PathBuf::from(".host"));
    };
    if arg.starts_with("--") {
        return Err(format!("实例目录参数不能以 -- 开头（目录是位置参数）: {arg}"));
    }
    if args.next().is_some() {
        return Err("实例目录仅接受一个位置参数".to_string());
    }
    Ok(PathBuf::from(arg))
}

/// `host init <dir>`: 生成 etc/host.yaml 模板 + etc/link/signing.pem（Ed25519，0600）
/// + etc/link/ros_bridge.yaml（topic 清单 + 令牌路径，从 host.yaml 导出）。
fn cmd_init(args: &mut impl Iterator<Item = String>) -> i32 {
    let dir = match parse_dir(args) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("{e}");
            return 2;
        }
    };
    let etc = dir.join("etc");
    let link = etc.join("link");
    if let Err(e) = std::fs::create_dir_all(&link) {
        eprintln!("init: 创建 {} 失败: {e}", link.display());
        return 1;
    }

    let cfg_path = etc.join("host.yaml");
    if cfg_path.exists() {
        eprintln!("init: {} 已存在，跳过", cfg_path.display());
    } else if let Err(e) = std::fs::write(&cfg_path, HOST_TOML_TEMPLATE) {
        eprintln!("init: 写入 {} 失败: {e}", cfg_path.display());
        return 1;
    } else {
        println!("已生成 {}", cfg_path.display());
    }

    let pem_path = link.join("signing.pem");
    if pem_path.exists() {
        eprintln!("init: {} 已存在，跳过", pem_path.display());
    } else {
        match gen_signing_pem() {
            Ok(pem) => {
                if let Err(e) = write_private_pem(&pem_path, pem.as_bytes()) {
                    eprintln!("init: 写入 {} 失败: {e}", pem_path.display());
                    return 1;
                }
                println!("已生成 {}（Ed25519 私钥，0600）", pem_path.display());
            }
            Err(e) => {
                eprintln!("init: 生成 Ed25519 密钥失败: {e}");
                return 1;
            }
        }
    }

    // G4: 设备身份 identity.json（D-H13 实例根，0600；幂等——仅缺失时生成，
    // 覆盖会使 server 侧注册失效）。损坏文件显式报错（C15），不静默覆盖。
    match mediaservo_host::identity::ensure_identity(&dir) {
        Ok(cred) => {
            let p = dir.join(mediaservo_host::identity::IDENTITY_FILE);
            if p.exists() {
                eprintln!("init: {} 已存在，跳过", p.display());
            } else {
                println!("已生成 {}（device_id={}，0600）", p.display(), cred.device_id);
            }
        }
        Err(e) => {
            eprintln!("init: {e}");
            return 1;
        }
    }

    // 生成 ros_bridge.yaml（B3）：topic 清单 + 令牌路径，ROS 节点配置单一来源。
    // 从已存在的 host.yaml 解析（init 刚写入模板或用户已编辑），解析失败即报错——
    // 静默写空清单会让 ROS 节点连不上任何 topic（C15）。
    let cfg_text = match std::fs::read_to_string(&cfg_path) {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!("init: 读取 {} 失败: {e}", cfg_path.display());
            return 1;
        }
    };
    let (sources, streams) = match mediaservo_host::translate::camera_and_stream_ids(&cfg_text) {
        Ok(ids) => ids,
        Err(e) => {
            eprintln!("init: {e}");
            return 1;
        }
    };
    let token_path = std::path::absolute(link.join("ros-vision.token"))
        .unwrap_or_else(|_| link.join("ros-vision.token"))
        .to_string_lossy()
        .into_owned();
    let yaml = mediaservo_link::bridge::ros_bridge(&sources, &streams, &token_path);
    let ros_path = link.join("ros_bridge.yaml");
    if let Err(e) = std::fs::write(&ros_path, &yaml) {
        eprintln!("init: 写入 {} 失败: {e}", ros_path.display());
        return 1;
    }
    println!("已生成 {}", ros_path.display());
    // G1: 签发标准车辆令牌集（幂等——D-H10 固定令牌，已存在跳过）
    if issue_all(&dir, DEFAULT_TOKEN_TTL_SECS) != 0 {
        return 1;
    }
    0
}

/// 生成 Ed25519 PKCS#8 PEM 私钥（link 令牌签名消费，D238）。
fn gen_signing_pem() -> Result<String, String> {
    use ed25519_dalek::pkcs8::EncodePrivateKey;
    let mut csprng = rand_core::OsRng;
    let signing = ed25519_dalek::SigningKey::generate(&mut csprng);
    let doc = signing
        .to_pkcs8_pem(pkcs8::LineEnding::LF)
        .map_err(|e| e.to_string())?;
    Ok(doc.to_string())
}

/// 写凭据文件（私钥/能力令牌）并设 0600 权限。
fn write_private_pem(path: &Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut f = std::fs::File::create(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    f.write_all(data)
}

/// `host start [<dir>]` / `host apply [<dir>]`（共享实现）: 读 host.yaml → 校验 →
/// 翻译 → 原子写 run/oxfile.toml → `oxmgr apply`（增量: 新增 Start/变更 Recreate/
/// 未变 Noop）。verb = "start"|"apply"（日志前缀）。
fn cmd_apply_impl(args: &mut impl Iterator<Item = String>, verb: &str) -> i32 {
    let dir = match parse_dir(args) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("{e}");
            return 2;
        }
    };
    // 多实例竞争检测: host-agent 本地网关端口被占用 = 另一 host 实例在运行。
    // 交互式二选一（非交互环境默认退出）: 退出 / 接管（停旧实例启当前）。
    if verb == "start" {
        if let Some(port) = agent_port_in_use(&dir) {
            let old_dir = find_other_instance_dir();
            eprintln!("检测到另一 host 实例在运行（本地信令网关端口 {port} 被占用）");
            if let Some(od) = &old_dir {
                eprintln!("  旧实例目录: {}", od.display());
            } else {
                eprintln!("  （未能定位旧实例目录——端口被其他程序占用？）");
            }
            if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
                eprintln!("  非交互环境——退出（有意多实例: 配置不同 [signaling] local_port + room 共存）");
                return 1;
            }
            eprint!("  输入 y 接管（停止旧实例并启动当前）/ 其他键退出: ");
            let _ = std::io::Write::flush(&mut std::io::stdout());
            let mut ans = String::new();
            let _ = std::io::stdin().read_line(&mut ans);
            if !ans.trim().eq_ignore_ascii_case("y") {
                return 1;
            }
            if let Some(od) = old_dir {
                let oxfile = od.join("run").join("oxfile.toml");
                let ox = oxfile.to_str().unwrap_or_default().to_string();
                eprintln!("接管: 停止旧实例 {}", od.display());
                let _ = run_oxmgr_in(Some(&od), &["stop", &ox]);
                let _ = run_oxmgr_in(Some(&od), &["delete", &ox]);
            } else {
                // PIT-155: 品牌化部署下定位旧实例失败（进程名 msrtc-agent 非 host-agent）——
                // 若继续清 SHM + apply 会与存活旧进程混战（相机 EBUSY/SHM 断链→web 黑屏）。
                eprintln!("接管中止: 未定位旧实例目录——旧进程可能仍在运行");
                eprintln!("  直接启动会造成资源竞争（相机 EBUSY / SHM 断链）");
                eprintln!("  请先手动停止旧实例（如 kill <旧进程> 或重启）后重试");
                return 1;
            }
        }
    }
    // C25: 全量启动前清 SHM 残留；apply（热更新）不清（进程在跑，SHM 在用）
    if verb == "start" {
        clear_iceoryx_residue("start");
    }
    match mediaservo_host::translate::apply_config(&dir) {
        Ok(()) => {
            println!("{verb}: 已应用 {}", dir.join("run").join("oxfile.toml").display());
            // 日志同步: OxMgr 0.5.0 [apps.logs] override 未接线（upstream）—
            // 日志在 oxmgr log_dir；把 host 进程日志 symlink 到实例 run/logs/（D-H13 布局）
            sync_host_logs(&dir);
            0
        }
        Err(e) => {
            eprintln!("{verb}: {e}");
            1
        }
    }
}

/// C25: iceoryx2 SHM 残留（SystemInFlux）会让 capturer/streamer 订阅 open 失败——
/// 全量停进程后（start/restart）必须清理；apply（热更新）不清（进程在跑，SHM 在用）。
fn clear_iceoryx_residue(verb: &str) {
    for p in ["/tmp/iceoryx2", "/dev/shm/iox2_*"] {
        let _ = std::process::Command::new("rm").args(["-rf", p]).status();
    }
    println!("{verb}: 已清理 iceoryx2 SHM 残留（C25）");
}

/// `host restart [<dir>]`: `oxmgr stop <oxfile>`（停全部 app）后重新 apply（全量重启）。
fn cmd_restart(args: &mut impl Iterator<Item = String>) -> i32 {
    let dir = match parse_dir(args) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("{e}");
            return 2;
        }
    };
    let oxfile = dir.join("run").join("oxfile.toml");
    if !oxfile.exists() {
        eprintln!("restart: 无 {} — 先 host start/apply", oxfile.display());
        return 1;
    }
    let code = run_oxmgr_in(Some(&dir), &["stop", oxfile.to_str().expect("oxfile path utf8")]);
    if code != 0 {
        return code;
    }

    // 全量重启 = 停后再起：与 start 同样清 SHM 残留（restart 漏清理 → SystemInFlux 黑屏）
    clear_iceoryx_residue("restart");
    match mediaservo_host::translate::apply_config(&dir) {
        Ok(()) => {
            println!("restart: 已全量重启（{}", dir.join("run").join("oxfile.toml").display());
            0
        }
        Err(e) => {
            eprintln!("restart: {e}");
            1
        }
    }
}

/// `host stop [<dir>]`: `oxmgr stop <oxfile>` + `oxmgr delete <oxfile>` + 兜底清
/// 残留 host 命名空间 app（历史 rename/中断 apply 遗留）——停后命名空间必空。
///
/// 无 run/oxfile.toml（从未 apply 或已删除）→ 仍清残留，无残留则直接成功。
fn cmd_stop(args: &mut impl Iterator<Item = String>) -> i32 {
    let dir = match parse_dir(args) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("{e}");
            return 2;
        }
    };
    let mut failed = 0;
    let oxfile = dir.join("run").join("oxfile.toml");
    if oxfile.exists() {
        let oxfile = oxfile.to_str().expect("oxfile path utf8");
        if run_oxmgr_in(Some(&dir), &["stop", oxfile]) != 0 {
            failed = 1;
        }
        if run_oxmgr_in(Some(&dir), &["delete", oxfile]) != 0 {
            failed = 1;
        }
    } else {
        println!("stop: 无 {}，无已管理进程", oxfile.display());
    }
    // 兜底: 残留 host 命名空间 app（oxfile 外遗留）逐个删除
    // （同一 per-instance daemon env——不带 env 查默认 daemon 为空，残留清理失效）
    let env = mediaservo_host::translate::oxmgr_env(&dir);
    match mediaservo_host::translate::live_host_apps(&env) {
        Ok(leftovers) => {
            for name in leftovers {
                eprintln!("stop: 清理残留 app {name}");
                if run_oxmgr_in(Some(&dir), &["delete", &name]) != 0 {
                    failed = 1;
                }
            }
        }
        Err(e) => {
            eprintln!("stop: 查询残留失败: {e}");
            failed = 1;
        }
    }
    if failed == 0 {
        println!("stop: 已停止全部 host 进程");
    }
    failed
}

/// `host status [<dir>]`: `oxmgr list --json` 过滤 host 命名空间，输出状态表。
fn cmd_status(args: &mut impl Iterator<Item = String>) -> i32 {
    let dir = match parse_dir(args) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("{e}");
            return 2;
        }
    };
    let out = match oxmgr_env(&dir).args(["list", "--json"]).output() {
        Ok(out) => out,
        Err(e) => {
            eprintln!("oxmgr 执行失败: {e} — 请先安装 OxMgr 并加入 PATH");
            return 1;
        }
    };
    if !out.status.success() {
        eprintln!("oxmgr list 失败: {}", String::from_utf8_lossy(&out.stderr).trim());
        return 1;
    }
    let procs: serde_json::Value = match serde_json::from_slice(&out.stdout) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("status: 解析 oxmgr list --json 输出失败: {e}");
            return 1;
        }
    };
    let Some(rows) = procs.as_array() else {
        eprintln!("status: oxmgr list --json 输出非数组");
        return 1;
    };
    let host_procs: Vec<&serde_json::Value> = rows
        .iter()
        .filter(|p| p.get("namespace").and_then(|n| n.as_str()) == Some(mediaservo_common::brand::media_brand().namespace))
        .collect();
    if host_procs.is_empty() {
        println!("host 命名空间无已管理进程（先 host start [<dir>]）");
        return 0;
    }
    println!("{:<28} {:<10} PID", "NAME", "STATUS");
    for p in &host_procs {
        let name = p.get("name").and_then(|v| v.as_str()).unwrap_or("?");
        let status = p.get("status").and_then(|v| v.as_str()).unwrap_or("?");
        let pid = p.get("pid").and_then(|v| v.as_u64()).map_or("-".to_string(), |v| v.to_string());
        println!("{name:<28} {status:<10} {pid}");
    }
    0
}

/// `host token issue`: 用 `etc/link/signing.pem`（host init 生成，PKCS#8 Ed25519）签发
/// 能力令牌（C4 最小签发 → G1 全量签发：--all/--for-ros/--ttl/校验/审计）。
///
/// 令牌写为 TokenFile 单文件（内嵌公钥 + JWT，MSTK 格式）——与 translate.rs
/// oxfile `--token` 引用的 `<cam>.token`/`<stream>.token`/`recorder.token`/`agent.token`
/// 同构。缺省 TTL 10 年（D-H10 固定令牌策略）。
///
/// 每次签发写审计（etc/link/issuance.jsonl JSONL，D-H10 审计纪律）。
const TOKEN_USAGE: &str = "用法:\n  host token issue --role <capture|processor|pusher|puller|recorder|control|perception|monitor> --node <id> [--topic <T>]... --out <path> [--ttl <secs>] [<dir>]\n  host token issue --all [--ttl <secs>] [<dir>]      # 从 host.yaml 签发标准车辆令牌集（幂等，跳过已存在）\n  host token issue --for-ros [--ttl <secs>] [<dir>]  # Perception 预设（node=ros-vision, out=etc/link/ros-vision.token，与 ros_bridge.yaml 一致）\nROS 令牌示例: host token issue --role perception --node ros-vision --out etc/link/ros-vision.token <dir>";
/// D-H10 固定令牌策略: 令牌长期有效，不随部署轮换。
const DEFAULT_TOKEN_TTL_SECS: u64 = 10 * 365 * 24 * 3600;
/// --for-ros 预设: ROS 视觉节点身份（与 ros_bridge.yaml token_path 同文件）。
const ROS_TOKEN_NODE: &str = "ros-vision";
const ROS_TOKEN_FILE: &str = "ros-vision.token";

fn cmd_token(args: &mut impl Iterator<Item = String>) -> i32 {
    let Some(sub) = args.next() else {
        eprintln!("{TOKEN_USAGE}");
        return 2;
    };
    if sub != "issue" {
        eprintln!("未知 token 子命令: {sub}");
        eprintln!("{TOKEN_USAGE}");
        return 2;
    }
    cmd_token_issue(args)
}

fn cmd_token_issue(args: &mut impl Iterator<Item = String>) -> i32 {
    let mut role: Option<Role> = None;
    let mut node: Option<String> = None;
    let mut topics: Vec<String> = Vec::new();
    let mut out: Option<PathBuf> = None;
    let mut ttl: u64 = DEFAULT_TOKEN_TTL_SECS;
    let mut all = false;
    let mut for_ros = false;
    let mut dir: Option<PathBuf> = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--role" => {
                let Some(v) = args.next() else {
                    eprintln!("--role 缺值");
                    return 2;
                };
                match parse_role(&v) {
                    Ok(r) => role = Some(r),
                    Err(e) => {
                        eprintln!("{e}");
                        return 2;
                    }
                }
            }
            "--node" => {
                let Some(v) = args.next() else {
                    eprintln!("--node 缺值");
                    return 2;
                };
                if let Err(e) = check_node_id(&v) {
                    eprintln!("{e}");
                    return 2;
                }
                node = Some(v);
            }
            "--topic" => {
                let Some(v) = args.next() else {
                    eprintln!("--topic 缺值");
                    return 2;
                };
                if v.is_empty() {
                    eprintln!("--topic 不能为空");
                    return 2;
                }
                topics.push(v);
            }
            "--out" => {
                let Some(v) = args.next() else {
                    eprintln!("--out 缺值");
                    return 2;
                };
                out = Some(PathBuf::from(v));
            }
            "--ttl" => {
                let Some(v) = args.next() else {
                    eprintln!("--ttl 缺值");
                    return 2;
                };
                match v.parse::<u64>() {
                    Ok(t) if t > 0 => ttl = t,
                    _ => {
                        eprintln!("--ttl 必须为正整数秒: {v}");
                        return 2;
                    }
                }
            }
            "--all" => all = true,
            "--for-ros" => for_ros = true,
            _ => {
                if arg.starts_with("--") {
                    eprintln!("实例目录参数不能以 -- 开头（目录是位置参数）: {arg}");
                    eprintln!("{TOKEN_USAGE}");
                    return 2;
                }
                if dir.is_some() {
                    eprintln!("未知参数: {arg}");
                    eprintln!("{TOKEN_USAGE}");
                    return 2;
                }
                dir = Some(PathBuf::from(arg));
            }
        }
    }
    let dir = dir.unwrap_or_else(|| PathBuf::from(".host"));
    if all && for_ros {
        eprintln!("--all 与 --for-ros 互斥");
        return 2;
    }
    if (all || for_ros) && (role.is_some() || node.is_some() || !topics.is_empty() || out.is_some()) {
        let preset = if all { "--all" } else { "--for-ros" };
        eprintln!("{preset} 不能与 --role/--node/--topic/--out 同用（参数从 host.yaml/预设推导）");
        return 2;
    }
    if all {
        return issue_all(&dir, ttl);
    }
    if for_ros {
        let out = std::path::absolute(dir.join("etc").join("link").join(ROS_TOKEN_FILE))
            .unwrap_or_else(|_| dir.join("etc").join("link").join(ROS_TOKEN_FILE));
        return match issue_one(&dir, Role::Perception, ROS_TOKEN_NODE.to_string(), Vec::new(), &out, ttl) {
            Ok(()) => {
                println!("已签发 Perception 令牌 → {}（node={} ttl={}s）", out.display(), ROS_TOKEN_NODE, ttl);
                0
            }
            Err(e) => {
                eprintln!("token: {e}");
                1
            }
        };
    }
    let (Some(role), Some(node), Some(out)) = (role, node, out) else {
        eprintln!("缺少必填参数: --role/--node/--out");
        eprintln!("{TOKEN_USAGE}");
        return 2;
    };
    // G1 加固: 显式 --topic 必须落在角色 ACL 矩阵允许方向/范围内（越权拒绝）。
    if let Err(e) = validate_topics(role, &topics) {
        eprintln!("{e}");
        return 2;
    }
    match issue_one(&dir, role, node.clone(), topics, &out, ttl) {
        Ok(()) => {
            println!("已签发 {:?} 令牌 → {}（node={} ttl={}s）", role, out.display(), node, ttl);
            0
        }
        Err(e) => {
            eprintln!("token: {e}");
            1
        }
    }
}

/// G1 加固: 显式 --topic 必须匹配角色 ACL 矩阵对应方向的允许模式。
/// 单方向角色（capture/pusher/recorder 订阅向/monitor）允许显式收窄；
/// 双方向/无方向角色（processor/perception/control/puller）显式 --topic 拒绝——
/// 矩阵缺省是其唯一授权面。
fn validate_topics(role: Role, topics: &[String]) -> Result<(), String> {
    if topics.is_empty() {
        return Ok(()); // 无显式 topic → 矩阵缺省，无需校验
    }
    let base = NodeAcl::for_role(NodeId::new(""), role);
    // --topic 匹配订阅方向（pusher/recorder/monitor/processor）
    if !base.subscribe_allow.is_empty() {
        let allowed = &base.subscribe_allow;
        for t in topics {
            if !allowed.iter().any(|p| FrameTopic::new(t).matches(p)) {
                return Err(format!("topic {t:?} 超出角色 {:?} 订阅允许模式: {:?}", role, allowed));
            }
        }
        return Ok(());
    }
    // --topic 匹配发布方向（capture）
    if !base.publish_allow.is_empty() {
        let allowed = &base.publish_allow;
        for t in topics {
            if !allowed.iter().any(|p| FrameTopic::new(t).matches(p)) {
                return Err(format!("topic {t:?} 超出角色 {:?} 发布允许模式: {:?}", role, allowed));
            }
        }
        return Ok(());
    }
    Err(format!(
        "role {:?} has no direction — explicit --topic only supported for single-direction roles (capture/pusher/recorder); omit --topic for matrix defaults",
        role,
    ))
}

/// G1 加固: node id 字符集守卫（与 host.yaml id 同规则 [A-Za-z0-9_-]+，
/// 防畸形 claims/路径穿越）。
fn check_node_id(id: &str) -> Result<(), String> {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err(format!("--node 非法: {id:?}（仅允许 [A-Za-z0-9_-]+）"));
    }
    Ok(())
}

/// 单次签发流水线：读 signing.pem → build_acl → 签名 → 审计 → 写 TokenFile（0600）。
/// 审计先行：无审计记录的签发拒绝落盘（D-H10 审计纪律——令牌不脱离审计存在）。
fn issue_one(
    dir: &Path,
    role: Role,
    node: String,
    topics: Vec<String>,
    out: &Path,
    ttl: u64,
) -> Result<(), String> {
    let pem_path = dir.join("etc").join("link").join("signing.pem");
    let pem = std::fs::read(&pem_path).map_err(|e| {
        format!("读取 {} 失败: {e} — 先运行 host init <dir>", pem_path.display())
    })?;
    let signing = ed25519_dalek::SigningKey::from_pkcs8_pem(&String::from_utf8_lossy(&pem))
        .map_err(|e| format!("{} 不是有效 PKCS#8 Ed25519 私钥: {e}", pem_path.display()))?;
    let acl = build_acl(NodeId::new(node.clone()), role, topics.clone())?;
    let sk = Ed25519SigningKey::from_pem(&pem);
    let token = CapabilityToken::sign(&acl, ttl, &sk).map_err(|e| format!("签名失败: {e}"))?;
    // TokenFile 内嵌 verifying key PEM（派生自私钥，同一密钥对）
    let vk_pem = signing
        .verifying_key()
        .to_public_key_pem(LineEnding::LF)
        .map_err(|e| format!("导出公钥失败: {e}"))?;
    let vk = Ed25519VerifyingKey::from_pem(vk_pem.as_bytes());
    let bytes = TokenFile::encode(&token, &vk);
    append_audit(dir, &role, &node, &topics, out, ttl)?;
    write_private_pem(out, &bytes).map_err(|e| format!("写入 {} 失败: {e}", out.display()))?;
    Ok(())
}

/// 签发审计（D-H10）: JSONL 追加 etc/link/issuance.jsonl，
/// 每条含 ts/role/node/topics/out/ttl。文件恒 0600（签发凭据登记簿）。
fn append_audit(
    dir: &Path,
    role: &Role,
    node: &str,
    topics: &[String],
    out: &Path,
    ttl: u64,
) -> Result<(), String> {
    use std::io::Write;
    let log = dir.join("etc").join("link").join("issuance.jsonl");
    let entry = serde_json::json!({
        "ts": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
        "role": role_name(role),
        "node": node,
        "topics": topics,
        "out": out.to_string_lossy(),
        "ttl": ttl,
    });
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log)
        .map_err(|e| format!("打开 {} 失败: {e}", log.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        f.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("审计 {} 权限设置失败: {e}", log.display()))?;
    }
    writeln!(f, "{entry}").map_err(|e| format!("写入 {} 失败: {e}", log.display()))
}

fn role_name(r: &Role) -> &'static str {
    match r {
        Role::Capture => "capture",
        Role::Processor => "processor",
        Role::Pusher => "pusher",
        Role::Puller => "puller",
        Role::Recorder => "recorder",
        Role::Control => "control",
        Role::Perception => "perception",
        Role::Monitor => "monitor",
        _ => "unknown",
    }
}

/// G1: `host token issue --all` — 从 host.yaml 签发标准车辆令牌集（幂等，
/// 已存在跳过 — D-H10 固定令牌）：
///   etc/link/<cam>.token     Capture  publish camera/<cam>（host-capturer-<cam>）
///   etc/link/<stream>.token  Pusher   subscribe camera/<cam> + vision/<cam> + publish stats/*
///   etc/link/recorder.token  Recorder 矩阵缺省（subscribe camera/video/vision + publish stats）
///   etc/link/agent.token     Monitor  矩阵缺省（subscribe camera/* + stats/* — E2 数据流监控）
/// ROS 令牌（--for-ros）不自动签发——ROS 存在性可选。
fn issue_all(dir: &Path, ttl: u64) -> i32 {
    let cfg_path = dir.join("etc").join("host.yaml");
    let cfg = match std::fs::read_to_string(&cfg_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("token: 读取 {} 失败: {e} — 先运行 host init <dir>", cfg_path.display());
            return 1;
        }
    };
    let sources = match mediaservo_host::translate::camera_configs(&cfg) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("token: {e}");
            return 1;
        }
    };
    let streams = match mediaservo_host::translate::stream_configs(&cfg) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("token: {e}");
            return 1;
        }
    };
    let link = dir.join("etc").join("link");
    let mut issued = 0usize;
    let mut first_err: Option<String> = None;
    'issue: {
        for src in &sources {
            let out = link.join(format!("{}.token", src.id));
            if out.exists() {
                continue;
            }
            match issue_one(
                dir,
                Role::Capture,
                format!("host-capturer-{}", src.id),
                vec![format!("camera/{}", src.id)],
                &out,
                ttl,
            ) {
                Ok(()) => issued += 1,
                Err(e) => {
                    first_err = Some(format!("签发 {} 失败: {e}", out.display()));
                    break 'issue;
                }
            }
        }
        for s in &streams {
            // <stream>-stream.token（Pusher）与 sources 的 <id>.token（Capture）分离——
            // 同名会因幂等 skip 被 Capture 占位，streamer 订阅被 ACL 拒（PIT-139）
            let out = link.join(format!("{}-stream.token", s.id));
            if out.exists() {
                continue;
            }
            let src = s.source.clone();
            match issue_one(
                dir,
                Role::Pusher,
                format!("host-streamer-{}", s.id),
                vec![format!("camera/{src}"), format!("vision/{src}")],
                &out,
                ttl,
            ) {
                Ok(()) => issued += 1,
                Err(e) => {
                    first_err = Some(format!("签发 {} 失败: {e}", out.display()));
                    break 'issue;
                }
            }
        }
        let rec = link.join("recorder.token");
        if !rec.exists() {
            match issue_one(dir, Role::Recorder, "host-recorder".to_string(), Vec::new(), &rec, ttl) {
                Ok(()) => issued += 1,
                Err(e) => first_err = Some(format!("签发 {} 失败: {e}", rec.display())),
            }
        }
        if first_err.is_none() {
            let agent = link.join("agent.token");
            if !agent.exists() {
                match issue_one(dir, Role::Monitor, "host-agent".to_string(), Vec::new(), &agent, ttl) {
                    Ok(()) => issued += 1,
                    Err(e) => first_err = Some(format!("签发 {} 失败: {e}", agent.display())),
                }
            }
        }
    }
    match first_err {
        Some(e) => {
            eprintln!("token: {e}");
            1
        }
        None => {
            println!("已签发 {issued} 个链路令牌（{}）", link.display());
            0
        }
    }
}

fn parse_role(s: &str) -> Result<Role, String> {
    match s.to_ascii_lowercase().as_str() {
        "capture" => Ok(Role::Capture),
        "processor" => Ok(Role::Processor),
        "pusher" => Ok(Role::Pusher),
        "puller" => Ok(Role::Puller),
        "recorder" => Ok(Role::Recorder),
        "control" => Ok(Role::Control),
        "perception" => Ok(Role::Perception),
        "monitor" => Ok(Role::Monitor),
        _ => Err(format!("未知角色: {s}（可选: capture/processor/pusher/puller/recorder/control/perception/monitor）")),
    }
}

/// --topic 缺省 = ACL 矩阵缺省（NodeAcl::for_role）；显式 --topic 覆盖角色
/// **单方向** ACL 列表（发布型角色 → publish_allow，订阅型角色 → subscribe_allow）。
/// 双方向/无方向角色（processor/perception/control/puller）显式 --topic 报错——
/// C 阶段最小签发只服务单方向角色，双方向留 G1 全量签发。
fn build_acl(node_id: NodeId, role: Role, topics: Vec<String>) -> Result<NodeAcl, String> {
    if topics.is_empty() {
        return Ok(NodeAcl::for_role(node_id, role));
    }
    let base = NodeAcl::for_role(node_id, role);
    // --topic controls subscribe direction for subscribe-oriented roles
    if !base.subscribe_allow.is_empty() {
        return Ok(NodeAcl { subscribe_allow: topics, ..base });
    }
    // --topic controls publish direction for publish-oriented roles (Capture)
    if !base.publish_allow.is_empty() {
        return Ok(NodeAcl { publish_allow: topics, ..base });
    }
    Err(format!(
        "role {:?} has no direction — explicit --topic only supported for single-direction roles (capture/pusher/recorder); omit --topic for matrix defaults",
        role,
    ))
}

/// `host doctor [<dir>]`: 环境诊断。三项检查：
/// ① oxmgr 可执行（PATH 内）② etc/host.yaml 可解析 ③ host.yaml → oxfile 可生成。
/// 退出码 = 失败检查数（0..=3）。
fn cmd_doctor(args: &mut impl Iterator<Item = String>) -> i32 {
    let dir = match parse_dir(args) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("{e}");
            return 2;
        }
    };
    let mut failed = 0;

    match Command::new(oxmgr_path()).arg("--version").output() {
        Ok(_) => println!("[ok] oxmgr 可用"),
        Err(e) => {
            println!("[fail] oxmgr 不可用: {e} — 请先安装并加入 PATH（npm install -g oxmgr）");
            failed += 1;
        }
    }

    let cfg_path = dir.join("etc").join("host.yaml");
    let cfg = match std::fs::read_to_string(&cfg_path) {
        Ok(cfg) => cfg,
        Err(e) => {
            println!("[fail] 读取 {} 失败: {e} — 先运行 host init <dir>", cfg_path.display());
            println!("[fail] oxfile 生成失败: 无配置可翻译（host.yaml 不可读）");
            return failed + 2; // ②③ 均因无配置失败，各打一条 [fail] 与计数一致
        }
    };
    // ② 配置解析与各消费进程同源（translate::validate——serde_yaml + 字段语义校验）
    match mediaservo_host::translate::validate(&cfg) {
        Ok(_) => println!("[ok] host.yaml 可解析"),
        Err(e) => {
            println!("[fail] host.yaml 解析失败: {e}");
            failed += 1;
        }
    }
    match mediaservo_host::translate::to_oxfile(&cfg) {
        Ok(_) => println!("[ok] oxfile 生成成功"),
        Err(e) => {
            println!("[fail] oxfile 生成失败: {e}");
            failed += 1;
        }
    }
    if failed == 0 {
        println!("doctor: 全部通过（{}）", cfg_path.display());
    }
    failed
}


/// 代理 oxmgr CLI；oxmgr 不在 PATH 时报清晰错误并提示安装。
/// 注入实例化的 oxmgr 数据目录（<dir>/run/oxmgr——oxmgr 用 OXMGR_HOME env）——多实例 daemon 状态隔离
/// 注入实例化的 oxmgr 环境——多实例 daemon 完全隔离:
/// ① OXMGR_HOME = <dir>/run/oxmgr（数据/日志/state）② OXMGR_DAEMON_ADDR 端口从 dir 稳定派生
/// （daemon 互斥 = TCP 端口——不隔离则全局 daemon 阻断实例 daemon 启动）
fn oxmgr_env(dir: &std::path::Path) -> std::process::Command {
    let mut cmd = std::process::Command::new(oxmgr_path());
    // 绝对化（OXMGR_HOME 相对路径依赖 daemon cwd——重启/系统服务会错位）
    let home = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf()).join("run").join("oxmgr");
    cmd.env("OXMGR_HOME", &home);
    let port = instance_daemon_port(&home);
    cmd.env("OXMGR_DAEMON_ADDR", format!("127.0.0.1:{port}"));
    // 第三个互斥点: webhook API 端口（默认从全局 daemon 端口派生——不隔离则撞全局 53081）
    cmd.env("OXMGR_API_ADDR", format!("127.0.0.1:{}", port + 1000));
    let _ = &home; // 保持 home 生命周期（后续 env 使用）
    cmd
}

/// 实例 daemon 端口: 18000 + dir 路径字节和 % 400（稳定——跨进程一致）
fn instance_daemon_port(dir: &std::path::Path) -> u16 {
    let sum: u32 = dir.to_string_lossy().bytes().map(u32::from).sum();
    18000u16 + (sum % 400) as u16
}



fn run_oxmgr_in(dir: Option<&std::path::Path>, args: &[&str]) -> i32 {
    let mut cmd = match dir {
        Some(d) => oxmgr_env(d),
        None => std::process::Command::new(oxmgr_path()),
    };
    match cmd.args(args).status() {
        Ok(status) => status.code().unwrap_or(1),
        Err(e) => {
            eprintln!("oxmgr 执行失败: {e} — 请先安装 OxMgr 并加入 PATH（npm install -g oxmgr，见 https://github.com/Vladimir-Urik/OxMgr#install）");
            1
        }
    }
}

/// 代理 oxmgr 命令（TUI/列表/日志——薄封装，stdio 继承）。
/// 用法: mediaservo-host <monit|ps|logs [proc]> —— oxmgr 不在 PATH 时报清晰错误。
fn cmd_oxmgr(args: &mut impl Iterator<Item = String>, fixed: &[&str]) -> i32 {
    // monit/ps 无额外参数；logs 透传全部参数（进程名 + oxmgr 选项如 --lines/-f）
    let extra: Vec<String> = if fixed[0] == "logs" {
        args.collect()
    } else {
        Vec::new()
    };
    let dir = parse_dir(&mut std::iter::empty()).unwrap_or_else(|_| std::path::PathBuf::from("."));
    let mut cmd = oxmgr_env(&dir);
    cmd.args(fixed).args(&extra);
    match cmd.status() {
        Ok(st) => st.code().unwrap_or(1),
        Err(e) => {
            eprintln!("无法执行 oxmgr: {e}");
            eprintln!("提示: oxmgr 在 install 打包的 bin/ 下——export PATH=<prefix>/bin:$PATH 后使用（或绝对路径）");
            1
        }
    }
}

/// 把 oxmgr 日志目录中本实例的进程日志 symlink 到 <dir>/run/logs/（D-H13 布局）。
/// 匹配品牌前缀（默认 host-*，品牌化 msrtc-*）——D252 品牌化遗漏修复。
/// 注: OxMgr 0.5.0 [apps.logs] 已接线（实测直写实例 run/logs），本函数为
/// oxmgr log_dir 兜底路径（如 daemon 忽略 override 的老版本）。
fn sync_host_logs(dir: &std::path::Path) {
    let log_dir = dirs_log_dir();
    let run_logs = dir.join("run").join("logs");
    let _ = std::fs::create_dir_all(&run_logs);
    let Ok(entries) = std::fs::read_dir(&log_dir) else {
        return;
    };
    let prefix = mediaservo_common::brand::media_brand().app_prefix;
    let mut n = 0;
    for e in entries.flatten() {
        let name = e.file_name();
        let Some(name) = name.to_str() else { continue };
        if !name.starts_with(prefix) {
            continue;
        }
        let link = run_logs.join(name);
        // 已存在且指向同一文件 → 跳过；否则重建（轮转/改名后更新）
        if let Ok(md) = std::fs::symlink_metadata(&link) {
            if md.file_type().is_symlink()
                && std::fs::read_link(&link).map(|p| p == e.path()).unwrap_or(false)
            {
                continue;
            }
            let _ = std::fs::remove_file(&link);
        }
        if std::os::unix::fs::symlink(&e.path(), &link).is_ok() {
            n += 1;
        }
    }
    if n > 0 {
        println!("日志同步: {n} 个进程日志 → {}", run_logs.display());
    }
}

/// oxmgr 日志目录（默认 ~/.local/share/oxmgr/logs；OXMGR_DATA_DIR 自定义时按 env）
fn dirs_log_dir() -> std::path::PathBuf {
    if let Ok(d) = std::env::var("OXMGR_DATA_DIR") {
        return std::path::PathBuf::from(d).join("logs");
    }
    std::env::var("HOME")
        .map(|h| std::path::PathBuf::from(h).join(".local/share/oxmgr/logs"))
        .unwrap_or_else(|_| std::path::PathBuf::from(".oxmgr/logs"))
}

/// 开机自启（三端: Linux systemd / macOS launchd / Windows Task Scheduler）。
/// `startup on` = oxmgr startup（注册系统集成）+ service install；`off` = service uninstall。
/// OXMGR_DATA_DIR 注入使 daemon 状态在实例内（与 host start 同 env，服务自启后一致性）。
fn cmd_startup(args: &mut impl Iterator<Item = String>) -> i32 {
    let sub = args.next().unwrap_or_default();
    if !matches!(sub.as_str(), "on" | "off" | "status") {
        eprintln!("用法: mediaservo-host startup <on|off|status> [<dir>]");
        return 2;
    }
    let dir = match parse_dir(args) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("{e}");
            return 2;
        }
    };
    // 绝对路径——unit 名按实例目录派生必须稳定（不同 cwd 同实例同名）
    let dir = dir.canonicalize().unwrap_or(dir);
    match sub.as_str() {
        "on" => startup_install(&dir),
        "off" => startup_uninstall(&dir),
        _ => startup_status(&dir),
    }
}

/// 自启 unit 名（按实例目录派生——多实例各自自启，天然无覆盖竞争）
fn startup_unit_name(dir: &std::path::Path) -> String {
    let raw = dir.to_string_lossy().replace(['/', '\\', ' '], "-");
    let raw = raw.trim_start_matches('-');
    // 默认品牌保持 legacy "oxmgr-host-*"（brand.rs 映射）；品牌化 "oxmgr-<brand>-*"
    format!("{}{raw}.service", mediaservo_common::brand::media_brand().unit_prefix)
}

/// 实例 daemon 的 oxmgr 二进制路径（host CLI 同目录）
fn oxmgr_path() -> std::path::PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("oxmgr")))
        .filter(|p| p.exists())
        .unwrap_or_else(|| std::path::PathBuf::from("oxmgr"))
}

/// 扫描已安装的其他实例自启 unit（返回 (旧 unit 路径, 旧实例目录)）。
/// 从 unit 的 Environment=OXMGR_DATA_DIR=<dir>/run/oxmgr 反推实例目录。
fn other_startup_units(dir: &std::path::Path) -> Vec<(std::path::PathBuf, std::path::PathBuf)> {
    let my_name = startup_unit_name(dir);
    let units_dir = std::path::Path::new(&std::env::var("HOME").unwrap_or_default())
        .join(".config/systemd/user");
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir(&units_dir) else { return found };
    for e in entries.flatten() {
        let name = e.file_name();
        let Some(name) = name.to_str() else { continue };
        let is_ours = name.starts_with("oxmgr-host-")
            || name.starts_with(mediaservo_common::brand::media_brand().unit_prefix);
        if !is_ours || !name.ends_with(".service") || name == my_name {
            continue;
        }
        let path = e.path();
        let dir = std::fs::read_to_string(&path)
            .ok()
            .and_then(|c| {
                c.lines().find_map(|l| {
                    l.trim().strip_prefix("Environment=OXMGR_DATA_DIR=").map(|v| v.trim().to_string())
                })
            })
            .map(|d| std::path::PathBuf::from(d).join("..").join(".."))
            .unwrap_or_default();
        found.push((path, dir));
    }
    found
}

#[cfg(target_os = "linux")]
fn startup_install(dir: &std::path::Path) -> i32 {
    // 全局唯一自启（相机等共享资源——多实例自启会抢设备）: 检测其他实例 unit → 交互接管
    let others = other_startup_units(dir);
    if !others.is_empty() {
        for (p, od) in &others {
            eprintln!("检测到其他实例已开启自启: {}（实例目录: {}）", p.display(), od.display());
        }
        if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
            eprintln!("  非交互环境——退出（只允许一个自启实例；先 `mediaservo-host startup off <旧dir>` 或手动接管）");
            return 1;
        }
        eprint!("  输入 y 接管（停旧实例并改为当前实例自启）/ 其他键退出: ");
        let _ = std::io::Write::flush(&mut std::io::stdout());
        let mut ans = String::new();
        let _ = std::io::stdin().read_line(&mut ans);
        if !ans.trim().eq_ignore_ascii_case("y") {
            return 1;
        }
        // 停旧实例（OXMGR_DATA_DIR 反推的 dir 可用则 stop+delete；unit 删除）
        for (p, od) in &others {
            if !od.as_os_str().is_empty() && od.join("run/oxfile.toml").exists() {
                let ox = od.join("run/oxfile.toml").to_string_lossy().into_owned();
                eprintln!("接管: 停止旧实例 {}", od.display());
                let _ = run_oxmgr_in(Some(od), &["stop", &ox]);
                let _ = run_oxmgr_in(Some(od), &["delete", &ox]);
            }
            let _ = std::fs::remove_file(p);
        }
        let _ = std::process::Command::new("systemctl").args(["--user", "daemon-reload"]).status();
    }
    let unit_name = startup_unit_name(dir);
    let unit_path = std::path::Path::new(&std::env::var("HOME").unwrap_or_default())
        .join(".config/systemd/user")
        .join(&unit_name);
    let oxmgr = oxmgr_path();
    let data_dir = dir.join("run").join("oxmgr");
    // 幂等: 覆盖自身 unit（同实例重跑 on = 刷新）
    let unit = format!(
        "[Unit]
         Description=MediaServo host oxmgr daemon ({})
         After=network.target
         
         [Service]
         Type=simple
         Environment=OXMGR_HOME={}
         Environment=OXMGR_DAEMON_ADDR=127.0.0.1:{}
         Environment=OXMGR_API_ADDR=127.0.0.1:{}
         ExecStart={} daemon run
         Restart=always
         RestartSec=2
         
         [Install]
         WantedBy=default.target
",
        dir.display(),
        data_dir.display(),
        instance_daemon_port(dir),
        instance_daemon_port(dir) + 1000,
        oxmgr.display()
    );
    if let Some(parent) = unit_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(e) = std::fs::write(&unit_path, unit) {
        eprintln!("startup on: 写入 unit 失败: {e}");
        return 1;
    }
    println!("startup on: 已安装 {}（OXMGR_DATA_DIR={}）", unit_path.display(), data_dir.display());
    for args in [
        vec!["systemctl", "--user", "daemon-reload"],
        vec!["systemctl", "--user", "enable", "--now", unit_name.as_str()],
    ] {
        let st = std::process::Command::new(args[0]).args(&args[1..]).status();
        match st {
            Ok(s) if s.success() => {}
            Ok(s) => {
                eprintln!("startup on: {} 退出码 {}", args.join(" "), s.code().unwrap_or(-1));
                return 1;
            }
            Err(e) => {
                eprintln!("startup on: 执行 {} 失败: {e}（systemd 用户服务不可用？）", args.join(" "));
                return 1;
            }
        }
    }
    println!("startup on: 开机自启已启用（systemd 用户服务 {}）", unit_name);
    0
}

#[cfg(target_os = "linux")]
fn startup_uninstall(dir: &std::path::Path) -> i32 {
    let unit_name = startup_unit_name(dir);
    let unit_path = std::path::Path::new(&std::env::var("HOME").unwrap_or_default())
        .join(".config/systemd/user")
        .join(&unit_name);
    if !unit_path.exists() {
        println!("startup off: 未安装自启（{} 不存在）", unit_path.display());
        return 0;
    }
    let _ = std::process::Command::new("systemctl")
        .args(["--user", "disable", "--now", &unit_name])
        .status();
    let _ = std::fs::remove_file(&unit_path);
    let _ = std::process::Command::new("systemctl").args(["--user", "daemon-reload"]).status();
    println!("startup off: 已停用并移除 {}", unit_path.display());
    0
}

#[cfg(target_os = "linux")]
fn startup_status(dir: &std::path::Path) -> i32 {
    let unit_name = startup_unit_name(dir);
    let unit_path = std::path::Path::new(&std::env::var("HOME").unwrap_or_default())
        .join(".config/systemd/user")
        .join(&unit_name);
    if unit_path.exists() {
        println!("startup: 已启用（{}）", unit_path.display());
        let st = std::process::Command::new("systemctl")
            .args(["--user", "is-active", unit_name.as_str()])
            .output();
        if let Ok(o) = st {
            println!("  daemon 状态: {}", String::from_utf8_lossy(&o.stdout).trim());
        }
        0
    } else {
        println!("startup: 未启用（{} 无自启 unit——用 `{} startup on` 启用）", dir.display(), mediaservo_common::brand::media_brand().product);
        1
    }
}

#[cfg(not(target_os = "linux"))]
fn startup_install(_dir: &std::path::Path) -> i32 {
    eprintln!("startup on: 非 Linux 平台——请用 oxmgr service install（macOS launchd / Windows Task Scheduler）");
    1
}

#[cfg(not(target_os = "linux"))]
fn startup_uninstall(_dir: &std::path::Path) -> i32 {
    eprintln!("startup off: 非 Linux 平台——请用 oxmgr service uninstall");
    1
}

#[cfg(not(target_os = "linux"))]
fn startup_status(_dir: &std::path::Path) -> i32 {
    eprintln!("startup status: 非 Linux 平台——请用 oxmgr service status");
    1
}

/// 探测 host-agent 本地网关端口是否被占用（[signaling] local_port 或默认 17980）。
fn agent_port_in_use(dir: &std::path::Path) -> Option<u16> {
    let cfg_path = dir.join("etc").join("host.yaml");
    let port = std::fs::read_to_string(&cfg_path)
        .ok()
        .and_then(|c| mediaservo_host::translate::signaling_local_port(&c).ok().flatten())
        .unwrap_or(17980);
    std::net::TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
        std::time::Duration::from_millis(300),
    )
    .map(|_| port)
    .ok()
}

/// 从进程 cmdline 提取 host 实例目录（品牌兼容：exe 名以 "-agent" 结尾即命中，PIT-155）。
fn probe_agent_dir(cmdline: &str) -> Option<std::path::PathBuf> {
    let cfg_marker = "--config";
    let mut parts = cmdline.split_whitespace();
    while let Some(p) = parts.next() {
        if p == cfg_marker {
            if let Some(path) = parts.next() {
                let cfg = std::path::Path::new(path);
                if cfg.ends_with("etc/host.yaml") {
                    return cfg.parent()?.parent().map(|d| d.to_path_buf());
                }
            }
        }
    }
    None
}

/// 判断进程 cmdline 是否为 host agent（品牌兼容：exe 名以 "-agent" 结尾）。
fn is_agent_cmdline(cmdline: &str) -> bool {
    cmdline
        .split_whitespace()
        .next()
        .and_then(|exe| std::path::Path::new(exe).file_name())
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.ends_with("-agent"))
}

/// 定位另一 host 实例目录：扫描 agent 进程 cmdline 的 --config <dir>/etc/host.yaml。
/// 品牌兼容（host-agent/msrtc-agent——PIT-155）。Linux /proc/*/cmdline；macOS ps；Windows None。
fn find_other_instance_dir() -> Option<std::path::PathBuf> {
    #[cfg(target_os = "linux")]
    {
        if let Ok(entries) = std::fs::read_dir("/proc") {
            for e in entries.flatten() {
                let pid = e.file_name();
                let Some(pid) = pid.to_str().and_then(|s| s.parse::<u32>().ok()) else { continue };
                let cmdline = std::fs::read_to_string(format!("/proc/{pid}/cmdline")).ok()?;
                let cmdline = cmdline.replace('\0', " ");
                if is_agent_cmdline(&cmdline) {
                    if let Some(d) = probe_agent_dir(&cmdline) {
                        return Some(d);
                    }
                }
            }
        }
        None
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

#[cfg(test)]
mod instance_probe_tests {
    use super::*;

    #[test]
    fn probe_agent_dir_finds_official_name() {
        let cmdline = "/opt/mediaservo-host/bin/host-agent --config /opt/mediaservo-host/etc/host.yaml --port 17980";
        assert!(is_agent_cmdline(cmdline));
        assert_eq!(probe_agent_dir(cmdline).as_deref(), Some(std::path::Path::new("/opt/mediaservo-host")));
    }

    #[test]
    fn probe_agent_dir_finds_branded_name() {
        // PIT-155: 品牌化部署进程名 msrtc-agent——旧实现 "host-agent" 硬编码定位失败
        let cmdline = "/opt/mediaservo-host/bin/msrtc-agent --config /opt/mediaservo-host/etc/host.yaml --port 17980";
        assert!(is_agent_cmdline(cmdline));
        assert_eq!(probe_agent_dir(cmdline).as_deref(), Some(std::path::Path::new("/opt/mediaservo-host")));
    }

    #[test]
    fn probe_agent_dir_rejects_non_agent() {
        let cmdline = "/opt/mediaservo-host/bin/msrtc-streamer --stream cam0 --config /opt/mediaservo-host/etc/host.yaml";
        assert!(!is_agent_cmdline(cmdline), "streamer 不得命中 agent 探测（过滤在 is_agent_cmdline 层）");
        // probe_agent_dir 不挑进程（任何 --config 均提取目录）——进程区分由 is_agent_cmdline 把关
        assert_eq!(probe_agent_dir(cmdline).as_deref(), Some(std::path::Path::new("/opt/mediaservo-host")));
    }
}
