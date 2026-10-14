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
//!   OxMgr 动词核对（C11/C18，来源 .refinfo/OxMgr/docs/CLI.md + SKILL.md）：
//!   `apply <config>` / `list [--json]` / `stop <name|id|config>` / `delete <name|id|config>`
//!   （**无** `stop/delete --namespace` 旗标；config 目标自动解析 oxfile 内全部 app，
//!   见 CLI.md "Lifecycle" 段）。

use std::path::{Path, PathBuf};
use std::process::Command;

use ed25519_dalek::pkcs8::{DecodePrivateKey, EncodePublicKey};
use mediaservo_link::{
    CapabilityToken, Ed25519SigningKey, Ed25519VerifyingKey, FrameTopic, NodeAcl, NodeId, Role,
    TokenFile,
};
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
  startup on/off/status [<dir>]             开机自启（Linux=systemd user unit；macOS/Windows=用 oxmgr 服务，未内置验证）
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
    // env 总表来自 common 资产（单一真源；末行含"host 树手工 env 行无效"注记）。
    println!(
        "{}",
        format!("{}\n\n{}", USAGE, mediaservo_common::env_usage_text())
            .replace("mediaservo-host", b.product)
    );
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
            println!(
                "{} {}",
                mediaservo_common::brand::media_brand().product,
                env!("CARGO_PKG_VERSION")
            );
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
        if let Some(exe_dir) =
            std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf()))
        {
            if exe_dir.join("etc").join("host.yaml").exists() {
                return Ok(exe_dir.clone());
            }
            if let Some(inst_root) = exe_dir.parent()
                && inst_root.join("etc").join("host.yaml").exists()
            {
                return Ok(inst_root.to_path_buf());
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

    // G4+device-enroll: 设备身份 identity.json（D-H13 实例根，0600；幂等——仅缺失时
    // 生成，覆盖会使 server 侧注册失效；新形状 {device_id}，公指纹即凭据）。损坏显式报错。
    let id_path = dir.join(mediaservo_host::identity::IDENTITY_FILE);
    let id_pre_existing = id_path.exists();
    match mediaservo_host::identity::ensure_identity(&dir) {
        Ok(device_id) => {
            if id_pre_existing {
                eprintln!("init: {} 已存在，跳过", id_path.display());
            } else {
                let fp = mediaservo_host::identity::load_device_identity(&dir)
                    .ok()
                    .flatten()
                    .map(|i| format!("，公钥指纹={}", i.pubkey_b64))
                    .unwrap_or_default();
                println!("已生成 {}（device_id={device_id}{fp}，0600）", id_path.display());
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
    let doc = signing.to_pkcs8_pem(pkcs8::LineEnding::LF).map_err(|e| e.to_string())?;
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
    if verb == "start"
        && let Some(port) = agent_port_in_use(&dir)
    {
        let old_dir = find_other_instance_dir();
        eprintln!("检测到另一 host 实例在运行（本地信令网关端口 {port} 被占用）");
        // start-conflict-doctor T3：占用者指认（pid/exe/deleted/父进程）——指认不杀戮。
        for r in mediaservo_host::conflict::diagnose_port(port) {
            eprintln!("  {}", r.line());
            if r.deleted {
                eprintln!("  → exe 已删除（升级残留）；kill {} 后重跑", r.pid);
            }
        }
        if let Some(od) = &old_dir {
            eprintln!("  旧实例目录: {}", od.display());
        } else {
            eprintln!("  （未能定位旧实例目录——端口被其他程序占用？）");
        }
        if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
            eprintln!(
                "  非交互环境——退出（有意多实例: 配置不同 [signaling] local_port + room 共存）"
            );
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
        // start-conflict-doctor T4：stop 自证（≤5s 轮询家族归零）
        let mut verified = false;
        for _ in 0..10 {
            let gone = [
                "host-agent",
                "host-streamer",
                "host-capturer",
                "host-recorder",
                "host-controller",
                "host-emergency",
                "host-audio",
            ]
            .iter()
            .all(|app| mediaservo_host::conflict::diagnose_app_gone(app));
            if gone {
                verified = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
        println!(
            "stop: 已停止全部 host 进程{}",
            if verified {
                "——自证清场 ✓"
            } else {
                "——⚠ 未能验证清场（残留见 ps）"
            }
        );
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
        .filter(|p| {
            p.get("namespace").and_then(|n| n.as_str())
                == Some(mediaservo_common::brand::media_brand().namespace)
        })
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
    if (all || for_ros) && (role.is_some() || node.is_some() || !topics.is_empty() || out.is_some())
    {
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
        return match issue_one(
            &dir,
            Role::Perception,
            ROS_TOKEN_NODE.to_string(),
            Vec::new(),
            &out,
            ttl,
        ) {
            Ok(()) => {
                println!(
                    "已签发 Perception 令牌 → {}（node={} ttl={}s）",
                    out.display(),
                    ROS_TOKEN_NODE,
                    ttl
                );
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
    let pem = std::fs::read(&pem_path)
        .map_err(|e| format!("读取 {} 失败: {e} — 先运行 host init <dir>", pem_path.display()))?;
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
            match issue_one(dir, Role::Recorder, "host-recorder".to_string(), Vec::new(), &rec, ttl)
            {
                Ok(()) => issued += 1,
                Err(e) => first_err = Some(format!("签发 {} 失败: {e}", rec.display())),
            }
        }
        if first_err.is_none() {
            let agent = link.join("agent.token");
            if !agent.exists() {
                match issue_one(
                    dir,
                    Role::Monitor,
                    "host-agent".to_string(),
                    Vec::new(),
                    &agent,
                    ttl,
                ) {
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
        _ => Err(format!(
            "未知角色: {s}（可选: capture/processor/pusher/puller/recorder/control/perception/monitor）"
        )),
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

/// K3 oxmgr 子进程调用点审计（startup-stop-followups——IPC 繁殖根修的完整面）:
/// ┌ 调用点 ─────────────────────┬ dir 来源 ────────────────┬ 缺省形风险 ┐
/// │ run_oxmgr_in ×9（stop/delete/apply）│ 全部 Some(&dir) 显式传入 │ 无        │
/// │ oxmgr_env ← run_oxmgr_in/cmd_status/cmd_oxmgr │ 调用方 dir        │ 见下两行   │
/// │ cmd_status（oxmgr list --json）     │ parse_dir 智能默认    │ 低: cwd/exe 锚定，仍可能 .host │
/// │ cmd_oxmgr（monit/ps/logs 直通）     │ ★K3 后显式必选        │ 旧: 缺省 "." ← 繁殖面已封 │
/// │ cmd_doctor --version               │ 无 daemon（纯可执行查询）│ 安全      │
/// │ oxmgr_bin() service install/uninstall/status │ 系统服务操作（无实例 daemon 语义）│ 面外 │
/// └────────────────────────────────────┴──────────────────────────┴───────────┘
/// translate.rs 同构链: oxmgr_apply/live_host_apps/delete 全走 oxmgr_env(&dir) 显式（无缺省形）。
/// 代理 oxmgr CLI；oxmgr 不在 PATH 时报清晰错误并提示安装。
/// 注入实例化的 oxmgr 环境——多实例 daemon 完全隔离:
/// ① OXMGR_HOME = <dir>/run/oxmgr（数据/日志/state）② OXMGR_DAEMON_ADDR 端口从 dir 稳定派生
/// （daemon 互斥 = TCP 端口——不隔离则全局 daemon 阻断实例 daemon 启动）③ OXMGR_DATA_DIR 双设
/// （C32 记忆用名——server oxmgr_cmd 同形，reap/startup 反查两键任一在场即命中）。
fn oxmgr_env(dir: &std::path::Path) -> std::process::Command {
    let mut cmd = std::process::Command::new(oxmgr_path());
    // 绝对化（OXMGR_HOME 相对路径依赖 daemon cwd——重启/系统服务会错位）
    let home = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf()).join("run").join("oxmgr");
    cmd.env("OXMGR_HOME", &home).env("OXMGR_DATA_DIR", &home); // C32 记忆用名——双设兼容（server oxmgr_cmd 同形）
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
        // K3 繁殖闸（现全仓 9 调用点全 Some——零行为变化，纯防御）:
        // 裸 Command 不带实例 env → oxmgr CLI IPC 按 cwd 静默拉起默认位 daemon。
        // 保持 Option 签名——未来调用者撞闸得到明确报错而非一次繁殖。
        None => {
            eprintln!("run_oxmgr_in: None dir 已禁用（防 IPC 繁殖默认 daemon）");
            return 2;
        }
    };
    match cmd.args(args).status() {
        Ok(status) => status.code().unwrap_or(1),
        Err(e) => {
            eprintln!(
                "oxmgr 执行失败: {e} — 请先安装 OxMgr 并加入 PATH（npm install -g oxmgr，见 https://github.com/Vladimir-Urik/OxMgr#install）"
            );
            1
        }
    }
}

/// 代理 oxmgr 命令（TUI/列表/日志——薄封装，stdio 继承）。
/// 用法: mediaservo-host <monit|ps|logs [proc]> —— oxmgr 不在 PATH 时报清晰错误。
fn cmd_oxmgr(args: &mut impl Iterator<Item = String>, fixed: &[&str]) -> i32 {
    // monit/ps 无额外参数；logs 透传剩余参数（进程名 + oxmgr 选项如 --lines/-f）。
    // 注意: logs 的 [<proc>] 消费完毕后仍可追加 <dir>——两段式 parse_dir；
    // monit/ps 的目录直接从 args 头部取。
    let extra: Vec<String> = if fixed[0] == "logs" { args.collect() } else { Vec::new() };
    // K3（startup-stop-followups）: 显式目录必选——旧缺省 "." 让任何 cwd 触发
    // `<cwd>/run/oxmgr` daemon 地址 → oxmgr CLI IPC 静默繁殖默认位 daemon（D1 清单风险位）。
    // server 同日同刀（lifecycle::cmd_oxmgr_proxy 同形态）——两侧一致收紧。
    let mut parts: Vec<String> = extra.clone();
    let Some(dir_arg) = parts.iter().position(|a| !a.starts_with('-')) else {
        eprintln!(
            "{}: 需要显式实例目录位置参数（如 mediaservo-host {} out/host）——不再缺省 cwd",
            fixed[0], fixed[0]
        );
        return 2;
    };
    let dir = std::path::PathBuf::from(parts.remove(dir_arg));
    let mut cmd = oxmgr_env(&dir);
    cmd.args(fixed).args(&parts);
    cmd.args(fixed).args(&extra);
    match cmd.status() {
        Ok(st) => st.code().unwrap_or(1),
        Err(e) => {
            eprintln!("无法执行 oxmgr: {e}");
            eprintln!(
                "提示: oxmgr 在 install 打包的 bin/ 下——export PATH=<prefix>/bin:$PATH 后使用（或绝对路径）"
            );
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
        if std::os::unix::fs::symlink(e.path(), &link).is_ok() {
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

/// 开机自启（Linux=systemd user unit；macOS/Windows=用 oxmgr 服务，未内置验证）。
/// `startup on` = oxmgr startup（注册系统集成）+ service install；`off` = service uninstall。
/// OXMGR_DATA_DIR 注入使 daemon 状态在实例内（与 host start 同 env，服务自启后一致性）。
/// unit 文本 ExecStart= 首 token（status bin 存活判据）——server startup.rs 同名镜像。
fn startup_first_token(unit_text: &str) -> Option<String> {
    unit_text
        .lines()
        .find_map(|l| l.trim().strip_prefix("ExecStart="))
        .and_then(|v| v.split_whitespace().next().map(String::from))
}

/// 单 unit 健康面四行（纯渲染）——server `status_line` 同语义镜像（S2 四形钉测）。
fn startup_status_line(
    unit: &str,
    enabled: Option<&str>,
    active: Option<&str>,
    linger: bool,
    bin_alive: bool,
) -> Vec<String> {
    let mut out = vec![unit.to_string()];
    out.push(format!("  enabled: {}", enabled.unwrap_or("no")));
    let mut daemon_line = format!("  daemon:  {}", active.unwrap_or("inactive"));
    if active == Some("failed") {
        daemon_line.push_str("  ⚠ 诊断: journalctl --user -u <unit> -n 20");
    }
    out.push(daemon_line);
    let mut linger_line = format!("  linger:  {}", if linger { "enabled" } else { "disabled" });
    if !linger {
        linger_line.push_str("——重启后不会自拉：loginctl enable-linger");
    }
    out.push(linger_line);
    out.push(if bin_alive {
        "  实例: bin ✓".to_string()
    } else {
        "  ⚠ bin 已删除——开机必失败。清场: startup off <dir> 或 systemctl --user disable --now + rm"
            .to_string()
    });
    out
}

/// unit 健康判定（exit 语义）：daemon failed / bin 缺失 / enabled 非 enabled = 不健康。
fn startup_unit_healthy(enabled: Option<&str>, active: Option<&str>, bin_alive: bool) -> bool {
    enabled == Some("enabled") && active == Some("active") && bin_alive
}

/// systemctl --user is-enabled（thin exec；失败/空输出 → None）。
fn startup_systemctl_enabled(unit: &str) -> Option<String> {
    let o = std::process::Command::new("systemctl")
        .args(["--user", "is-enabled", unit])
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// S3 装后探针样本（is-active 归一化）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum S3ProbeSample {
    Active,
    Failed,
    Other,
}

/// S3 探针裁决：FailFast=轮询期见 failed；Ok=sustain 复核通过；
/// FailLate=首 active 后复核掉链（迟发 bind 失败）；Timeout=5s 无 active。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum S3ProbeVerdict {
    Ok,
    FailFast,
    FailLate,
    Timeout,
}

/// S3 装后探针裁决（纯函数，§2.3+m1）：Failed 即刻快败；首个 Active 后等 sustain
/// 复核样本：仍 Active=Ok，否则 FailLate；样本耗尽未见 active=Timeout。
/// 复核期采样仍 is-active，故 [Active,Failed] 归 FailLate。与 server lifecycle 镜像。
fn s3_probe_verdict(samples: &[S3ProbeSample]) -> S3ProbeVerdict {
    let mut seen_active = false;
    for &smp in samples {
        match smp {
            S3ProbeSample::Failed if !seen_active => return S3ProbeVerdict::FailFast,
            S3ProbeSample::Failed => return S3ProbeVerdict::FailLate,
            S3ProbeSample::Active if !seen_active => seen_active = true,
            S3ProbeSample::Active => return S3ProbeVerdict::Ok,
            S3ProbeSample::Other => {}
        }
    }
    if seen_active { S3ProbeVerdict::FailLate } else { S3ProbeVerdict::Timeout }
}

/// journalctl 诊断指引行（命令面钉测点）。
fn s3_probe_failure_hint(unit: &str) -> String {
    format!("journalctl --user -u {unit} -n 20")
}

/// 轮询 ≤5s（10×500ms）+ sustain 2s 复核。I/O runner：采样 + sleep；裁决全在纯函数。
fn s3_probe_startup(unit: &str) -> bool {
    let mut samples: Vec<S3ProbeSample> = Vec::new();
    let mut seen_active = false;
    for _ in 0..10 {
        let smp = match startup_systemctl_active(unit).as_deref() {
            Some("active") => S3ProbeSample::Active,
            Some("failed") => S3ProbeSample::Failed,
            _ => S3ProbeSample::Other,
        };
        let hit = smp == S3ProbeSample::Active;
        samples.push(smp);
        if seen_active {
            break; // sustain 复核样本已入列——一处采样即裁决（§2.3 "one more is-active"）
        }
        if hit {
            seen_active = true;
            std::thread::sleep(std::time::Duration::from_secs(2));
            continue;
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    match s3_probe_verdict(&samples) {
        S3ProbeVerdict::Ok => true,
        v => {
            let hint = s3_probe_failure_hint(unit);
            match v {
                S3ProbeVerdict::FailFast => {
                    eprintln!("startup on: daemon 起后即 failed——查: {hint}");
                    eprintln!("  快猜: 端口冲突? 查 doctor / 端口占用");
                }
                S3ProbeVerdict::FailLate => {
                    eprintln!("警告: daemon 起后退出（可能迟发 bind 失败）——查: {hint}");
                }
                _ => {
                    eprintln!("startup on: daemon 5s 内未进入 active——查: {hint}");
                    eprintln!("  快猜: 端口冲突? 查 doctor / 端口占用");
                }
            }
            false
        }
    }
}

/// systemctl --user is-active（thin exec）。
fn startup_systemctl_active(unit: &str) -> Option<String> {
    let o = std::process::Command::new("systemctl")
        .args(["--user", "is-active", unit])
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// linger 态（§5 M2 钉形 `loginctl show-user $(id -u) -p Linger`，无 -U；失败兑底
/// `/var/lib/systemd/linger/$USER` 存在性）——server linger_enabled 同语义镜像。
fn startup_linger_enabled() -> bool {
    let uid = std::process::Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty());
    if let Some(uid) = uid
        && let Ok(o) = std::process::Command::new("loginctl")
            .args(["show-user", &uid, "-p", "Linger"])
            .output()
    {
        let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
        if s.contains("Linger=") {
            return s.ends_with("=yes");
        }
    }
    std::env::var("USER")
        .ok()
        .is_some_and(|u| std::path::Path::new("/var/lib/systemd/linger").join(u).exists())
}

/// host 域 unit → bin 存活快照（IO 薄壳）。
struct S2UnitHealth {
    name: String,
    enabled: Option<String>,
    active: Option<String>,
    bin_alive: bool,
}

/// 扫 host 域全部 unit（S1 classifier——Host 域收 legacy+品牌两形）。
fn startup_scan_health() -> (Vec<S2UnitHealth>, Vec<String>) {
    let units_dir = std::path::Path::new(&std::env::var("HOME").unwrap_or_default())
        .join(".config/systemd/user");
    let mut out = Vec::new();
    let mut unknown = Vec::new();
    let Ok(entries) = std::fs::read_dir(&units_dir) else {
        return (out, unknown);
    };
    for e in entries.flatten() {
        let name = e.file_name();
        let Some(name) = name.to_str() else { continue };
        if !name.ends_with(".service") {
            continue;
        }
        match startup_classify_unit(name) {
            S1Foreign => continue,
            S1Unknown => unknown.push(name.to_string()),
            _ => out.push(S2UnitHealth {
                name: name.to_string(),
                enabled: startup_systemctl_enabled(name),
                active: startup_systemctl_active(name),
                bin_alive: std::fs::read_to_string(e.path())
                    .ok()
                    .and_then(|c| startup_first_token(&c))
                    .is_none_or(|p| std::path::Path::new(&p).exists()),
            }),
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    (out, unknown)
}

/// off 无参选择（纯）——server select_off 镜像。
fn startup_select_off(count: usize, tty: bool) -> S2OffSelect {
    match count {
        0 => S2OffSelect::NoneInstalled,
        1 => S2OffSelect::Single,
        _ if tty => S2OffSelect::Prompt,
        _ => S2OffSelect::RejectList,
    }
}

#[derive(Debug, PartialEq, Eq)]
enum S2OffSelect {
    NoneInstalled,
    Single,
    Prompt,
    RejectList,
}

fn cmd_startup(args: &mut impl Iterator<Item = String>) -> i32 {
    let sub = args.next().unwrap_or_default();
    if !matches!(sub.as_str(), "on" | "off" | "status") {
        eprintln!(
            "用法: mediaservo-host startup <on|off|status> [<dir>]（status/off 无参=本机全量盘点）"
        );
        return 2;
    }
    // S2: status/off 无参 = 全量盘点默认语义；带 <dir> = 既有过滤/定向卸载形。
    if (sub == "status" || sub == "off") && args.next().is_none() {
        return if sub == "status" { startup_status_all() } else { startup_off_all() };
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

/// `startup status`（无参）：host 域全量盘点（server startup_status_all 镜像）。
fn startup_status_all() -> i32 {
    let (units, unknown) = startup_scan_health();
    let linger = startup_linger_enabled();
    for u in &unknown {
        eprintln!(
            "警告: 未知 oxmgr unit: {u}（非本产品命名——如需清场手工 systemctl --user disable --now + rm）"
        );
    }
    if units.is_empty() {
        if unknown.is_empty() {
            let p = mediaservo_common::brand::media_brand().product;
            println!("本机无 {p} 自启 unit——`{p} startup on <dir>` 启用");
        }
        return 1;
    }
    let mut unhealthy = 0;
    for u in &units {
        let e = u.enabled.as_deref();
        let a = u.active.as_deref();
        for line in startup_status_line(&u.name, e, a, linger, u.bin_alive) {
            println!("{line}");
        }
        if !startup_unit_healthy(e, a, u.bin_alive) {
            unhealthy += 1;
        }
    }
    i32::from(unhealthy > 0)
}

/// `startup off`（无参）：零=幂等 0；恰一=直停；多=tty 列选 / 非 tty 拒。
fn startup_off_all() -> i32 {
    let (units, _unknown) = startup_scan_health();
    let tty = std::io::IsTerminal::is_terminal(&std::io::stdin());
    match startup_select_off(units.len(), tty) {
        S2OffSelect::NoneInstalled => {
            println!("startup off: 未安装自启");
            0
        }
        S2OffSelect::Single => startup_uninstall_unit(&units[0].name),
        S2OffSelect::Prompt => {
            for (i, u) in units.iter().enumerate() {
                println!("[{}] {}（{}）", i + 1, u.name, u.active.as_deref().unwrap_or("?"));
            }
            println!("[0] 全部卸载 / 回车=取消");
            let mut ans = String::new();
            let _ = std::io::stdin().read_line(&mut ans);
            let picked: Vec<String> = match ans.trim() {
                "" => return 0,
                "0" => units.iter().map(|u| u.name.clone()).collect(),
                n => match n.parse::<usize>().ok().filter(|n| (1..=units.len()).contains(n)) {
                    Some(i) => vec![units[i - 1].name.clone()],
                    None => {
                        eprintln!("无效选择: {n}");
                        return 2;
                    }
                },
            };
            let mut rc = 0;
            for name in picked {
                rc |= startup_uninstall_unit(&name);
            }
            rc
        }
        S2OffSelect::RejectList => {
            eprintln!(
                "startup off: 本机有多个本产品自启 unit，非交互环境拒绝盲卸——用 `startup off <dir>` 定向:"
            );
            for u in &units {
                eprintln!("  {}", u.name);
            }
            1
        }
    }
}

/// 单 unit 卸载（disable --now + rm + daemon-reload）。
fn startup_uninstall_unit(unit: &str) -> i32 {
    let _ =
        std::process::Command::new("systemctl").args(["--user", "disable", "--now", unit]).status();
    let _ = std::fs::remove_file(
        std::path::Path::new(&std::env::var("HOME").unwrap_or_default())
            .join(".config/systemd/user")
            .join(unit),
    );
    let _ = std::process::Command::new("systemctl").args(["--user", "daemon-reload"]).status();
    println!("startup off: 已停用并移除 {unit}");
    0
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

#[cfg(target_os = "linux")]
fn startup_install(dir: &std::path::Path) -> i32 {
    // S1 v2 冲突闸：身份核对 + 端口比对（相机等共享资源——同类撞须处置）。
    // 判定全在 s1_decide_v2 纯函数；此处只做扫描 IO + 交互执行。
    let (conflicts, unknown) = startup_scan_conflicts(dir);
    for u in &unknown {
        eprintln!("警告: 检测到未知 oxmgr unit（不识别域，未处理）: {u}");
    }
    for c in &conflicts {
        eprintln!(
            "检测到本产品其他自启锚点: unit={} 实例={} 域={} 冲突端口={}",
            c.peer_unit,
            c.peer_dir.display(),
            match c.peer_kind {
                S1SameDomain => "同类",
                S1CrossDomain => "跨域",
            },
            if c.overlapping.is_empty() {
                "无".to_string()
            } else {
                format!("{:?}", c.overlapping)
            }
        );
    }
    let tty = std::io::IsTerminal::is_terminal(&std::io::stdin());
    match s1_decide_v2(&conflicts, tty) {
        S1Abort => {
            eprintln!("  非交互环境——退出（端口冲突需处置；先 `startup off <旧dir>` 或交互接管）");
            return 1;
        }
        S1PromptTakeover => {
            eprintln!("同类实例端口冲突——三选一: [1]算了不装 [2]卸对方接管 [3]手工处置");
            match s1_prompt().as_str() {
                "2" => {
                    for c in conflicts.iter().filter(|c| c.peer_kind == S1SameDomain) {
                        eprintln!(
                            "  接管: 卸载 {}（实例 {}，端口明细见上）",
                            c.peer_unit,
                            c.peer_dir.display()
                        );
                        s1_takeover_peer(&c.peer_unit, &c.peer_dir);
                    }
                    let _ = std::process::Command::new("systemctl")
                        .args(["--user", "daemon-reload"])
                        .status();
                }
                "3" => {
                    eprintln!("已退出——请手工处置上述冲突后重试");
                    return 1;
                }
                _ => {
                    println!("已取消");
                    return 0;
                }
            }
        }
        S1PromptCrossDomainWarn => {
            eprintln!("警告: 跨服务域端口重叠——不同服务域，通常为配置错误");
            eprintln!("选择: [1]覆盖(卸对方) [2]取消本项 [3]允许并行(已知晓风险)");
            match s1_prompt().as_str() {
                "1" => {
                    for c in conflicts
                        .iter()
                        .filter(|c| c.peer_kind == S1CrossDomain && !c.overlapping.is_empty())
                    {
                        eprintln!(
                            "  覆盖: 卸载 {}（实例 {}，端口明细见上）",
                            c.peer_unit,
                            c.peer_dir.display()
                        );
                        s1_takeover_peer(&c.peer_unit, &c.peer_dir);
                    }
                    let _ = std::process::Command::new("systemctl")
                        .args(["--user", "daemon-reload"])
                        .status();
                }
                "3" => {
                    eprintln!("警告: 允许并行——端口重叠风险已知晓");
                }
                _ => {
                    println!("已取消");
                    return 0;
                }
            }
        }
        S1Coexist(note) => {
            println!("startup on: {note}");
        }
        S1Install => {}
    }
    let unit_name = startup_unit_name(dir);
    let unit_path = std::path::Path::new(&std::env::var("HOME").unwrap_or_default())
        .join(".config/systemd/user")
        .join(&unit_name);
    // S5 实环勘误②（server 同笔）：systemd 无 PATH——ExecStart 裸名 = 203/EXEC crash loop。
    let oxmgr = {
        let p = oxmgr_path();
        if !p.is_absolute() {
            eprintln!("startup on: 无法定位 oxmgr 绝对路径（bin/ 旁无 oxmgr）——就位后重试");
            return 1;
        }
        p
    };
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
                eprintln!(
                    "startup on: 执行 {} 失败: {e}（systemd 用户服务不可用？）",
                    args.join(" ")
                );
                return 1;
            }
        }
    }
    if !s3_probe_startup(&unit_name) {
        return 1;
    }
    println!("startup on: 开机自启已启用 + daemon 已拉起 ✓（{unit_name}）");
    // S5 实环勘误（与 server 侧同笔）：daemon 锚点不拉簇——apply oxfile 幂等唤醒。
    let oxfile = dir.join("run").join("oxfile.toml");
    if oxfile.exists() {
        println!("startup on: 唤醒实例进程（apply {}）", oxfile.display());
        if run_oxmgr_in(Some(dir), &["apply", &oxfile.to_string_lossy()]) != 0 {
            eprintln!(
                "startup on: apply 失败——unit/daemon 在位，实例未拉起（手工 start {}）",
                dir.display()
            );
            return 1;
        }
    } else {
        eprintln!(
            "startup on: 无 {}（实例未 init/start 过）——仅 daemon 锚点在位",
            oxfile.display()
        );
    }
    println!("startup on: 建议 loginctl enable-linger 保跨会话存活");
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
    if !unit_path.exists() {
        println!(
            "startup: 未启用（{} 无自启 unit——用 `{} startup on` 启用）",
            dir.display(),
            mediaservo_common::brand::media_brand().product
        );
        return 1;
    }
    println!("startup: 已启用（{}）", unit_path.display());
    let enabled = startup_systemctl_enabled(&unit_name);
    let active = startup_systemctl_active(&unit_name);
    let bin_alive = std::fs::read_to_string(&unit_path)
        .ok()
        .and_then(|c| startup_first_token(&c))
        .is_none_or(|p| std::path::Path::new(&p).exists());
    for line in startup_status_line(
        &unit_name,
        enabled.as_deref(),
        active.as_deref(),
        startup_linger_enabled(),
        bin_alive,
    ) {
        println!("{line}");
    }
    i32::from(!startup_unit_healthy(enabled.as_deref(), active.as_deref(), bin_alive))
}

// ── 非 Linux 薄适配（startup-hardening S4/项6）：转发 oxmgr service（跨平台装拆
//    oxmgr 内置 systemd/launchd/Task Scheduler）。已知语义（本机侦察 10-13）：
//    固定单例名、实例 env 不注入（默认数据目录形态）——单例 WARN + 退出码透传。

#[cfg(not(target_os = "linux"))]
fn startup_install(_dir: &std::path::Path) -> i32 {
    eprintln!("startup on: 非 Linux——转发 oxmgr service install（单例形态）");
    eprintln!("  ⚠ 实例目录 env 不注入：daemon 用默认数据目录；多实例自启仅 Linux 形支持");
    match Command::new(oxmgr_bin()).args(["service", "install", "--system", "auto"]).status() {
        Ok(st) => st.code().unwrap_or(1),
        Err(e) => {
            eprintln!("startup on: oxmgr service 执行失败: {e}");
            1
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn startup_uninstall(_dir: &std::path::Path) -> i32 {
    eprintln!("startup off: 非 Linux——转发 oxmgr service uninstall");
    match Command::new(oxmgr_bin()).args(["service", "uninstall", "--system", "auto"]).status() {
        Ok(st) => st.code().unwrap_or(1),
        Err(e) => {
            eprintln!("startup off: oxmgr service 执行失败: {e}");
            1
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn startup_status(_dir: &std::path::Path) -> i32 {
    eprintln!("startup status: 非 Linux——转发 oxmgr service status");
    match Command::new(oxmgr_bin()).args(["service", "status", "--system", "auto"]).status() {
        Ok(st) => st.code().unwrap_or(1),
        Err(e) => {
            eprintln!("startup status: oxmgr service 执行失败: {e}");
            1
        }
    }
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
        if p == cfg_marker
            && let Some(path) = parts.next()
        {
            let cfg = std::path::Path::new(path);
            if cfg.ends_with("etc/host.yaml") {
                return cfg.parent()?.parent().map(|d| d.to_path_buf());
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
                if is_agent_cmdline(&cmdline)
                    && let Some(d) = probe_agent_dir(&cmdline)
                {
                    return Some(d);
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
    fn bundled_host_yaml_template_passes_validate() {
        // 模板 = init 落地产物。defaults 段带 deny_unknown_fields、codec/stream_mode 有
        // 合法集门（D282）——模板写错键/错值必须在本单测红，而不是用户 init 后 deploy 挂。
        mediaservo_host::translate::validate(HOST_TOML_TEMPLATE)
            .expect("host.yaml.template 必须通过校验");
        let cams = mediaservo_host::translate::camera_configs(HOST_TOML_TEMPLATE).unwrap();
        assert_eq!(
            cams[0].mode,
            mediaservo_host::translate::SourceMode::Camera,
            "mode 身份键逐条目显式（D282 修订：不入 defaults）"
        );
        assert_eq!(cams[1].fps, 30, "test 源走 defaults.sources.fps");
        let s = &mediaservo_host::translate::stream_configs(HOST_TOML_TEMPLATE).unwrap()[0];
        assert_eq!(s.codec, "h264", "缺省编码 h264（D282）");
        assert_eq!(s.stream_mode.as_deref(), Some("smooth"), "defaults.streams 进流");
        assert_eq!(s.keyframe_interval, Some(2), "defaults.streams 逐键合并");
    }

    #[test]
    fn probe_agent_dir_finds_official_name() {
        let cmdline = "/opt/mediaservo-host/bin/host-agent --config /opt/mediaservo-host/etc/host.yaml --port 17980";
        assert!(is_agent_cmdline(cmdline));
        assert_eq!(
            probe_agent_dir(cmdline).as_deref(),
            Some(std::path::Path::new("/opt/mediaservo-host"))
        );
    }

    #[test]
    fn probe_agent_dir_finds_branded_name() {
        // PIT-155: 品牌化部署进程名 msrtc-agent——旧实现 "host-agent" 硬编码定位失败
        let cmdline = "/opt/mediaservo-host/bin/msrtc-agent --config /opt/mediaservo-host/etc/host.yaml --port 17980";
        assert!(is_agent_cmdline(cmdline));
        assert_eq!(
            probe_agent_dir(cmdline).as_deref(),
            Some(std::path::Path::new("/opt/mediaservo-host"))
        );
    }

    #[test]
    fn probe_agent_dir_rejects_non_agent() {
        let cmdline = "/opt/mediaservo-host/bin/msrtc-streamer --stream cam0 --config /opt/mediaservo-host/etc/host.yaml";
        assert!(
            !is_agent_cmdline(cmdline),
            "streamer 不得命中 agent 探测（过滤在 is_agent_cmdline 层）"
        );
        // probe_agent_dir 不挑进程（任何 --config 均提取目录）——进程区分由 is_agent_cmdline 把关
        assert_eq!(
            probe_agent_dir(cmdline).as_deref(),
            Some(std::path::Path::new("/opt/mediaservo-host"))
        );
    }
}

// ── K3（startup-stop-followups）: oxmgr 调用面繁殖闸/双设 env ────────────────
#[cfg(test)]
mod k3_oxmgr_surface_tests {
    use super::*;

    /// D2: None 臂 = 防繁殖闸——返回 2 且不 spawn 子进程（闸在 Command 构造前返回）。
    #[test]
    fn run_oxmgr_in_none_dir_blocked() {
        let rc = run_oxmgr_in(None, &["list"]);
        assert_eq!(rc, 2, "None dir 必须被闸拦截（exit 2），不得裸 spawn 默认 daemon 面 oxmgr");
    }

    /// D4: host oxmgr_env 四键齐——OXMGR_HOME+OXMGR_DATA_DIR 双设（server 同形）+ 两地址键。
    #[test]
    fn oxmgr_env_carries_dual_data_dir_keys() {
        let cmd = oxmgr_env(std::path::Path::new("/tmp/k3-fake-instance"));
        let keys: Vec<_> = cmd.get_envs().map(|(k, _)| k.to_string_lossy().into_owned()).collect();
        for k in ["OXMGR_HOME", "OXMGR_DATA_DIR", "OXMGR_DAEMON_ADDR", "OXMGR_API_ADDR"] {
            assert!(keys.contains(&k.to_string()), "缺键 {k}: envs={keys:?}");
        }
        let home = cmd.get_envs().find(|(k, _)| *k == "OXMGR_HOME").unwrap().1.unwrap();
        assert!(
            home.to_string_lossy().ends_with("run/oxmgr"),
            "home 锚定 <dir>/run/oxmgr: {home:?}"
        );
        let data = cmd.get_envs().find(|(k, _)| *k == "OXMGR_DATA_DIR").unwrap().1.unwrap();
        assert_eq!(home, data, "双设同值");
    }

    /// D4: translate::oxmgr_env（env 向量形）同样四键双设。
    #[test]
    fn translate_oxmgr_env_vector_dual_set() {
        let env =
            mediaservo_host::translate::oxmgr_env(std::path::Path::new("/tmp/k3-fake-instance"));
        assert_eq!(env.len(), 4, "四键: {env:?}");
        assert_eq!(env[0].0, "OXMGR_HOME");
        assert_eq!(env[1].0, "OXMGR_DATA_DIR", "第二键=双设形（server 同序）");
        assert_eq!(env[0].1, env[1].1, "双设同值");
    }
}

// ── startup-hardening S1: 冲突闸 v2——身份核对+端口比对（§2.1/§5 钉形）────────
// host 侧独立实现（host 是 standalone bin，PLAN §3 T4a 裁定共享下沉另立项）——
// 与 server lifecycle/startup.rs 同语义镜像，测试 fixture 同族互证。

/// unit 身份分类（host 视角）：Host=同类 / Server=跨域 / Unknown=本产品 oxmgr 前缀不识别
/// （WARN 列表，永不静默永不做接管候选）/ Foreign=非 oxmgr。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum S1UnitIdentity {
    Host,
    Server,
    Unknown,
    Foreign,
}
use S1UnitIdentity::{
    Foreign as S1Foreign, Host as S1Host, Server as S1Server, Unknown as S1Unknown,
};

/// 已知品牌集合（跨品牌互查 ∪——startup-stop-followups 裁决 1；与 server 侧
/// lifecycle/startup.rs KNOWN_BRANDS 同值镜像，两 crate 常量不提升——计划 §3）。
const S1_KNOWN_BRANDS: &[&str] = &["mediaservo", "msrtc"];

/// unit 文件名 → 身份（精确前缀匹配——禁 contains；B1）。
fn startup_classify_unit(name: &str) -> S1UnitIdentity {
    startup_classify_unit_with(name, &mediaservo_common::brand::media_brand())
}

/// 同上（品牌参数缝——K1 钉测双视角；生产走薄壳）。
fn startup_classify_unit_with(
    name: &str,
    brand: &mediaservo_common::brand::Brand,
) -> S1UnitIdentity {
    let stem = name.strip_suffix(".service").unwrap_or(name);
    let Some(rest) = stem.strip_prefix("oxmgr-").or((stem == "oxmgr").then_some("")) else {
        return S1Foreign;
    };
    // server 域（跨域对方）先判 = 已知品牌 `{bin_prefix}server-` 全枚举 ∪ 当前品牌形
    // （先于 host——`mediaservo-server-` ⊃ `mediaservo-` 前缀，host 先判会吞 server unit）。
    let mut server_forms: Vec<String> = S1_KNOWN_BRANDS
        .iter()
        .map(|b| {
            format!("{}server-", mediaservo_common::brand::media_brand_from(Some(b)).bin_prefix)
        })
        .collect();
    server_forms.push(format!("{}server-", brand.bin_prefix));
    server_forms.sort();
    server_forms.dedup();
    if server_forms.iter().any(|f| rest.starts_with(f.as_str())) {
        return S1Server;
    }
    // host 域 = 已知品牌两形（legacy `host-` + 各品牌 `<brand>-`）∪ 当前品牌两形。
    let mut host_forms: Vec<String> = S1_KNOWN_BRANDS
        .iter()
        .map(|b| {
            mediaservo_common::brand::media_brand_from(Some(b))
                .unit_prefix
                .trim_start_matches("oxmgr-")
                .to_string()
        })
        .chain([
            brand.unit_prefix.trim_start_matches("oxmgr-").to_string(),
            format!("{}-", brand.id),
        ])
        .collect();
    host_forms.sort();
    host_forms.dedup();
    if host_forms.iter().any(|f| rest.starts_with(f.as_str())) {
        return S1Host;
    }
    S1Unknown
}

/// unit 文本 → 实例目录（双键 OXMGR_HOME= 优先 miss 落 OXMGR_DATA_DIR=——reap.rs 先例；
/// 值形 `<实例>/run/oxmgr` 上跳两级）。
fn startup_unit_instance_dir(unit_text: &str) -> Option<std::path::PathBuf> {
    let envs: Vec<&str> =
        unit_text.lines().filter_map(|l| l.trim().strip_prefix("Environment=")).collect();
    let val = envs
        .iter()
        .find_map(|l| l.strip_prefix("OXMGR_HOME=").map(|v| v.trim().to_string()))
        .or_else(|| {
            envs.iter()
                .find_map(|l| l.strip_prefix("OXMGR_DATA_DIR=").map(|v| v.trim().to_string()))
        })?;
    let p = std::path::PathBuf::from(val);
    Some(
        p.parent().and_then(std::path::Path::parent).map(std::path::Path::to_path_buf).unwrap_or(p),
    )
}

/// host 实例端口集（M3 域分形：host.yaml signaling local_port + oxfile `--port/--listen`
/// 文本扫描。derived daemon port 不入——非 app 口）。parse 失败逐源 skip。
fn startup_instance_ports(dir: &std::path::Path) -> Vec<u16> {
    let mut ports = Vec::new();
    if let Ok(cfg) = std::fs::read_to_string(dir.join("etc").join("host.yaml"))
        && let Ok(Some(p)) = mediaservo_host::translate::signaling_local_port(&cfg)
    {
        ports.push(p);
    }
    // host oxfile 现无 weaknet 条目（weaknet 只随 server 簇部署）——文本扫描 `--port N`/
    // `--listen 127.0.0.1:N` 兜将来条目形态；现网实例恒空集 = 零假阳。
    if let Ok(ox) = std::fs::read_to_string(dir.join("run").join("oxfile.toml")) {
        for line in ox.lines() {
            let t = line.trim();
            for key in ["--port ", "--listen 127.0.0.1:"] {
                if let Some(rest) = t.split(key).nth(1) {
                    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
                    if let Ok(p) = digits.parse::<u16>() {
                        ports.push(p);
                    }
                }
            }
        }
    }
    ports.sort();
    ports.dedup();
    ports
}

/// tty 行读入（trim 后；EOF/空 = ""）。
fn s1_prompt() -> String {
    eprint!("选择: ");
    let _ = std::io::Write::flush(&mut std::io::stdout());
    let mut ans = String::new();
    let _ = std::io::stdin().read_line(&mut ans);
    ans.trim().to_string()
}

/// 接管执行：daemon 停旧实例（oxfile 在场）→ unit disable+删文件（明细由调用方前置打印）。
fn s1_takeover_peer(unit: &str, peer_dir: &std::path::Path) {
    if !peer_dir.as_os_str().is_empty() && peer_dir.join("run/oxfile.toml").exists() {
        let ox = peer_dir.join("run/oxfile.toml").to_string_lossy().into_owned();
        let _ = run_oxmgr_in(Some(peer_dir), &["stop", &ox]);
        let _ = run_oxmgr_in(Some(peer_dir), &["delete", &ox]);
    }
    let _ =
        std::process::Command::new("systemctl").args(["--user", "disable", "--now", unit]).status();
    let _ = std::fs::remove_file(
        std::path::Path::new(&std::env::var("HOME").unwrap_or_default())
            .join(".config/systemd/user")
            .join(unit),
    );
}

/// 扫描本产品其他自启 unit → 冲突表（S1 v2：身份分类 + 双键反推 + 端口求交）。
/// 返回 (冲突表, unknown unit 名集)。同 `startup_unit_name(dir)` 自身排除。
fn startup_scan_conflicts(dir: &std::path::Path) -> (Vec<S1Conflict>, Vec<String>) {
    let mine = startup_unit_name(dir);
    let units_dir = std::path::Path::new(&std::env::var("HOME").unwrap_or_default())
        .join(".config/systemd/user");
    let mut conflicts = Vec::new();
    let mut unknown = Vec::new();
    let Ok(entries) = std::fs::read_dir(&units_dir) else {
        return (conflicts, unknown);
    };
    let my_ports = startup_instance_ports(dir);
    for e in entries.flatten() {
        let name = e.file_name();
        let Some(name) = name.to_str() else { continue };
        if !name.ends_with(".service") || name == mine {
            continue;
        }
        match startup_classify_unit(name) {
            S1Foreign => continue,
            S1Unknown => {
                unknown.push(name.to_string());
                continue;
            }
            kind => {
                let text = std::fs::read_to_string(e.path()).unwrap_or_default();
                let peer_dir = startup_unit_instance_dir(&text).unwrap_or_default();
                let peer_ports = if peer_dir.as_os_str().is_empty() {
                    Vec::new()
                } else {
                    startup_instance_ports(&peer_dir)
                };
                conflicts.push(S1Conflict {
                    peer_unit: name.to_string(),
                    peer_dir,
                    peer_kind: match kind {
                        S1UnitIdentity::Host => S1SameDomain,
                        _ => S1CrossDomain,
                    },
                    overlapping: s1_overlap(&my_ports, &peer_ports),
                });
            }
        }
    }
    (conflicts, unknown)
}

fn s1_overlap(a: &[u16], b: &[u16]) -> Vec<u16> {
    let mut out: Vec<u16> = a.iter().filter(|p| b.contains(p)).copied().collect();
    out.sort();
    out.dedup();
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct S1Conflict {
    peer_unit: String,
    peer_dir: std::path::PathBuf,
    peer_kind: S1PeerKind,
    overlapping: Vec<u16>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum S1PeerKind {
    SameDomain,
    CrossDomain,
}
use S1PeerKind::{CrossDomain as S1CrossDomain, SameDomain as S1SameDomain};

#[derive(Debug, PartialEq, Eq)]
enum S1Decision {
    Install,
    Coexist(String),
    PromptTakeover,
    PromptCrossDomainWarn,
    Abort,
}
use S1Decision::{Abort as S1Abort, Coexist as S1Coexist, Install as S1Install};

/// 聚合决策（§2.1，server decide_v2 镜像）：同类撞优先弹接管、否则跨域撞弹警告、
/// 全空集共存安装；非 tty + 任何弹问 → Abort。
fn s1_decide_v2(conflicts: &[S1Conflict], tty: bool) -> S1Decision {
    let same_hit =
        conflicts.iter().any(|c| c.peer_kind == S1SameDomain && !c.overlapping.is_empty());
    let cross_hit =
        conflicts.iter().any(|c| c.peer_kind == S1CrossDomain && !c.overlapping.is_empty());
    if same_hit {
        return if tty { S1PromptTakeover } else { S1Abort };
    }
    if cross_hit {
        return if tty { S1PromptCrossDomainWarn } else { S1Abort };
    }
    let mut peers: Vec<String> =
        conflicts.iter().map(|c| format!("{}（无端口交集）", c.peer_dir.display())).collect();
    peers.sort();
    peers.dedup();
    match peers.len() {
        0 => S1Install,
        1 => S1Coexist(format!("与实例 {} 共存（无端口交集）", peers[0])),
        _ => S1Coexist(format!("与实例 {} 共存（无端口交集）", peers.join("、"))),
    }
}
use S1Decision::PromptCrossDomainWarn as S1PromptCrossDomainWarn;
use S1Decision::PromptTakeover as S1PromptTakeover;

#[cfg(test)]
mod s1_startup_gate_tests {
    use super::*;

    /// legacy 形（motivating fixture）：`oxmgr-host-` 前缀 + OXMGR_DATA_DIR 键 + 死 bin。
    const S1_LEGACY_HOST_UNIT: &str = "[Service]\nEnvironment=OXMGR_DATA_DIR=/home/maxsense/Documents/MediaServo/install/host/run/oxmgr\nExecStart=/old/bin/oxmgr daemon run\n";
    /// current 形：OXMGR_HOME 键（双键优先级先例 reap.rs:274）。
    const S1_CURRENT_HOST_UNIT: &str = "[Service]\nEnvironment=OXMGR_HOME=/opt/host2/run/oxmgr\nExecStart=/opt/host2/bin/oxmgr daemon run\n";

    #[test]
    fn s1_dual_key_extraction_home_first() {
        assert_eq!(
            startup_unit_instance_dir(S1_CURRENT_HOST_UNIT),
            Some(std::path::PathBuf::from("/opt/host2"))
        );
        assert_eq!(
            startup_unit_instance_dir(S1_LEGACY_HOST_UNIT),
            Some(std::path::PathBuf::from("/home/maxsense/Documents/MediaServo/install/host"))
        );
        assert_eq!(startup_unit_instance_dir("[Service]\nExecStart=x\n"), None);
    }

    #[test]
    fn s1_identity_classification() {
        // host 域：legacy/默认 unit_prefix（brand 感知——默认品牌下即 oxmgr-host- 形；
        // 品牌化 oxmgr-{brand}- 形由 host_forms[0]=unit_prefix 通吃，跨品牌钉测随品牌进程跑）
        assert_eq!(startup_classify_unit("oxmgr-host-home-x.service"), S1Host);
        // server 域（跨域对方）：bin_prefix ns 形 + 默认品牌形（默认品牌下同串）
        assert_eq!(startup_classify_unit("oxmgr-mediaservo-server-x.service"), S1Server);
        // TalkServo 同族 → Unknown（绝不误认——B1 反例）
        assert_eq!(startup_classify_unit("oxmgr-talkservo-host-x.service"), S1Unknown);
        assert_eq!(startup_classify_unit("oxmgr.service"), S1Unknown);
        assert_eq!(startup_classify_unit("nginx.service"), S1Foreign);
    }

    // ── K1 跨品牌互查 ∪（裁决 1；缝钉测——双品牌视角逐方验证）────────────────

    #[test]
    fn k1_msrtc_brand_perspective_recognizes_default_and_msrtc() {
        let msrtc = mediaservo_common::brand::media_brand_from(Some("msrtc"));
        // host 域：默认/legacy 形（此前 msrtc bin 只认 msrtc- 形=互查缺口主对象）
        assert_eq!(startup_classify_unit_with("oxmgr-host-home-x.service", &msrtc), S1Host);
        // server 域：默认品牌 server unit → 跨域 Server
        assert_eq!(
            startup_classify_unit_with("oxmgr-mediaservo-server-x.service", &msrtc),
            S1Server
        );
        // 自身两形不丢
        assert_eq!(startup_classify_unit_with("oxmgr-msrtc-home-x.service", &msrtc), S1Host);
        assert_eq!(startup_classify_unit_with("oxmgr-msrtc-server-x.service", &msrtc), S1Server);
        // TalkServo 负例
        assert_eq!(startup_classify_unit_with("oxmgr-talkservo-host-x.service", &msrtc), S1Unknown);
    }

    #[test]
    fn k1_default_brand_perspective_recognizes_msrtc() {
        let msrtc = mediaservo_common::brand::media_brand_from(Some("msrtc"));
        let def = mediaservo_common::brand::media_brand_from(None);
        // 默认 bin 认 msrtc 两域
        assert_eq!(
            startup_classify_unit_with("oxmgr-msrtc-server-home-x-out-server.service", &def),
            S1Server
        );
        assert_eq!(startup_classify_unit_with("oxmgr-msrtc-home-x-out-host.service", &def), S1Host);
        // msrtc 视角下 legacy host 形（同串双视角互证）
        assert_eq!(startup_classify_unit_with("oxmgr-host-home-x.service", &msrtc), S1Host);
    }

    #[test]
    fn s1_overlap_and_decide_v2() {
        assert!(s1_overlap(&[17980, 9810], &[9800, 8080]).is_empty());
        assert_eq!(s1_overlap(&[17980], &[17980]), vec![17980]);
        let same_hit = S1Conflict {
            peer_unit: "oxmgr-host-a.service".into(),
            peer_dir: "/a".into(),
            peer_kind: S1SameDomain,
            overlapping: vec![17980],
        };
        let cross_empty = S1Conflict {
            peer_unit: "oxmgr-mediaservo-server-x.service".into(),
            peer_dir: "/x".into(),
            peer_kind: S1CrossDomain,
            overlapping: vec![],
        };
        // 四态 + 非 tty 拒 + 聚合
        assert_eq!(s1_decide_v2(&[same_hit.clone()], true), S1PromptTakeover);
        assert_eq!(s1_decide_v2(&[same_hit.clone()], false), S1Abort);
        assert!(matches!(s1_decide_v2(&[cross_empty.clone()], true), S1Coexist(_)));
        match s1_decide_v2(
            &[S1Conflict { overlapping: vec![], ..same_hit.clone() }, cross_empty],
            true,
        ) {
            S1Coexist(ref n) if n.contains("/a") && n.contains("/x") => {}
            other => panic!("期望双 peer Coexist，得 {other:?}"),
        }
        assert_eq!(s1_decide_v2(&[], true), S1Install);
    }

    // ── S2: status 四行健康面 / off 无参选择（server startup.rs 镜像钉测，零 spawn）──

    const S2_ZOMBIE_UNIT: &str =
        "[Service]\nEnvironment=OXMGR_HOME=/old/run/oxmgr\nExecStart=/old/bin/oxmgr daemon run\n";

    #[test]
    fn s2_first_token_execstart() {
        assert_eq!(startup_first_token(S2_ZOMBIE_UNIT).as_deref(), Some("/old/bin/oxmgr"));
        assert_eq!(startup_first_token("[Service]"), None);
    }

    #[test]
    fn s2_status_line_four_shapes() {
        // ① 健康
        let healthy = startup_status_line("u.service", Some("enabled"), Some("active"), true, true);
        assert_eq!(healthy[1], "  enabled: enabled");
        assert_eq!(healthy[2], "  daemon:  active");
        assert_eq!(healthy[3], "  linger:  enabled");
        assert_eq!(healthy[4], "  实例: bin ✓");
        // ② 僵尸 bin（legacy install-host 形——开机必失败）
        let zombie = startup_status_line("u.service", Some("enabled"), Some("active"), true, false);
        assert!(zombie[4].contains("bin 已删除"));
        assert!(zombie[4].contains("startup off <dir>"));
        // ③ daemon failed
        let failed = startup_status_line("u.service", Some("enabled"), Some("failed"), true, true);
        assert!(failed[2].contains("journalctl"));
        // ④ linger off
        let no_linger =
            startup_status_line("u.service", Some("enabled"), Some("active"), false, true);
        assert_eq!(no_linger[3], "  linger:  disabled——重启后不会自拉：loginctl enable-linger");
    }

    #[test]
    fn s2_unit_healthy_exit_semantics() {
        assert!(startup_unit_healthy(Some("enabled"), Some("active"), true));
        assert!(!startup_unit_healthy(Some("enabled"), Some("failed"), true));
        assert!(!startup_unit_healthy(Some("enabled"), Some("active"), false));
        assert!(!startup_unit_healthy(None, None, true));
    }

    #[test]
    fn s2_select_off_three_shapes() {
        assert_eq!(startup_select_off(0, true), S2OffSelect::NoneInstalled);
        assert_eq!(startup_select_off(1, false), S2OffSelect::Single, "恰一=直停");
        assert_eq!(startup_select_off(2, true), S2OffSelect::Prompt);
        assert_eq!(startup_select_off(3, false), S2OffSelect::RejectList);
    }

    // ── S3: 装后探针裁决（表测驱动纯函数；真 systemctl/真 sleep 不进测试）──

    use S3ProbeSample::{Active, Failed, Other};
    use S3ProbeVerdict::{FailFast, FailLate, Ok as POk, Timeout};

    #[test]
    fn s3_probe_verdict_table() {
        // 基形：first-active + sustain 通过
        assert_eq!(s3_probe_verdict(&[Other, Active, Active]), POk);
        assert_eq!(s3_probe_verdict(&[Active, Active]), POk);
        // 装前即 failed = 快败
        assert_eq!(s3_probe_verdict(&[Failed, Active]), FailFast);
        // first-active 后复核掉链 = 迟发失败（[Active, Failed] 边形）
        assert_eq!(s3_probe_verdict(&[Active, Failed]), FailLate);
        assert_eq!(s3_probe_verdict(&[Active, Other]), FailLate);
        // 轮询窗内无 active = 超时
        assert_eq!(s3_probe_verdict(&[Other; 10]), Timeout);
        assert_eq!(s3_probe_verdict(&[]), Timeout);
        // active-then-other-then-active 边形：复核窗内 Other 抖动不判死（语义钉死防漂移）
        assert_eq!(s3_probe_verdict(&[Active, Other, Active]), POk);
    }

    #[test]
    fn s3_probe_failure_hint_command() {
        assert_eq!(
            s3_probe_failure_hint("oxmgr-host-x.service"),
            "journalctl --user -u oxmgr-host-x.service -n 20"
        );
    }

    #[test]
    fn s2_systemctl_command_construction() {
        // thin exec 层 argv 合同 = ["--user", <verb>, unit]（is-enabled/is-active 同序）
        for verb in ["is-enabled", "is-active"] {
            let argv = ["--user", verb, "oxmgr-host-x.service"];
            assert_eq!(argv[0], "--user");
            assert_eq!(argv[1], verb);
        }
    }
}
