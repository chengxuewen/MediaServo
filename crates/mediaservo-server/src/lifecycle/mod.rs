//! `msrtc-server` 实例生命周期（frontend-process-split T15-T19，单二进制双角色之管理面）。
//!
//! 引擎 = oxmgr（与 host 同构，design.md「Server 进程管理：修正版」）。拓扑固定 2 进程
//! （server + caddy web），oxfile 静态模板免翻译层。所有 oxmgr 调用走**实例 daemon**
//! （OXMGR_HOME/OXMGR_DAEMON_ADDR 派生自实例目录——C32 隔离，基数 18500 避开 host 系）。
//!
//! 向后兼容硬门：main.rs 仅在首参命中 LIFECYCLE_CMDS 时进入本模块；
//! `run`/无参/`--config`/未知参数 = 守护模式原样（systemd/compose/docker 直启零破坏）。

mod inspect;
mod startup;
pub mod templates;

use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use templates::{
    DEFAULT_WEB_PORT, gen_secret, instance_daemon_port, parse_listen_port, parse_web_port,
    render_admin_account, render_caddyfile, render_oxfile, render_server_yaml, server_product,
    server_web_app,
};

/// 管理面子命令表（main.rs 派发判据）。`run` 不在此列——它走守护模式（显式形态）。
pub const LIFECYCLE_CMDS: &[&str] = &[
    "init", "start", "stop", "restart", "status", "doctor", "logs", "startup", "monit", "ps",
    "version",
];

pub fn is_lifecycle_cmd(c: &str) -> bool {
    LIFECYCLE_CMDS.contains(&c)
}

pub fn dispatch(cmd: &str, args: &mut dyn Iterator<Item = String>) -> i32 {
    match cmd {
        "init" => cmd_init(args),
        "start" => cmd_start(args, "start"),
        "stop" => cmd_stop(args),
        "restart" => cmd_restart(args),
        "status" => inspect::cmd_status(args),
        "doctor" => inspect::cmd_doctor(args),
        "logs" => inspect::cmd_logs(args),
        "startup" => startup::cmd_startup(args),
        "monit" => cmd_oxmgr_proxy(&["ui"]),
        "ps" => cmd_oxmgr_proxy(&["list"]),
        "version" => {
            println!("{} {}", server_product(), env!("CARGO_PKG_VERSION"));
            0
        }
        other => {
            eprintln!("未知子命令: {other}");
            2
        }
    }
}

/// 解析实例目录（位置参数，智能默认同 host：cwd 有实例→"."，二进制同级/上级→安装根，否则 .server）。
pub(super) fn parse_dir(args: &mut dyn Iterator<Item = String>) -> Result<PathBuf, String> {
    let Some(arg) = args.next() else {
        if PathBuf::from("etc/server.yaml").exists() {
            return Ok(PathBuf::from("."));
        }
        if let Some(exe_dir) =
            std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf))
        {
            if exe_dir.join("etc").join("server.yaml").exists() {
                return Ok(exe_dir);
            }
            if let Some(root) = exe_dir.parent()
                && root.join("etc").join("server.yaml").exists()
            {
                return Ok(root.to_path_buf());
            }
        }
        return Ok(PathBuf::from(".server"));
    };
    if arg.starts_with("--") {
        return Err(format!("实例目录参数不能以 -- 开头（目录是位置参数）: {arg}"));
    }
    if args.next().is_some() {
        return Err("实例目录仅接受一个位置参数".to_string());
    }
    Ok(PathBuf::from(arg))
}

// ── init ─────────────────────────────────────────────────────────────────────

/// `init [<dir>]`：实例目录生成（PIT-160：已存在文件**永不覆盖**）。
/// etc/{server,devices,accounts}.yaml + Caddyfile + run/oxfile.toml + secret 自举（0600 幂等，
/// compose entrypoint 逻辑归位——双轨同语义）。env MEDIASERVO_ADMIN_PASSWORD 引导首账号（entrypoint ⑤）。
fn cmd_init(args: &mut dyn Iterator<Item = String>) -> i32 {
    let dir = match parse_dir(args) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("{e}");
            return 2;
        }
    };
    match init_instance(&dir) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("init: {e}");
            1
        }
    }
}

fn init_instance(dir: &Path) -> Result<(), String> {
    let etc = dir.join("etc");
    let run = dir.join("run");
    std::fs::create_dir_all(&etc).map_err(|e| format!("创建 {} 失败: {e}", etc.display()))?;
    std::fs::create_dir_all(run.join("logs")).map_err(|e| format!("创建 run 目录失败: {e}"))?;

    // ① server.yaml（含凭据 → 0600；存在即整体跳过——secret 不可再生覆盖，entrypoint ② 幂等语义）
    let cfg_path = etc.join("server.yaml");
    if cfg_path.exists() {
        eprintln!("init: {} 已存在，跳过（凭据保留）", cfg_path.display());
    } else {
        let yaml = render_server_yaml(&gen_secret(), &gen_secret());
        write_private(&cfg_path, yaml.as_bytes())?;
        println!("已生成 {}（PSK/JWT 自举，0600）", cfg_path.display());
    }

    // ② 注册表空模板（管理面 C33；逐文件缺失才写）
    let registry_templates = [
        ("devices.yaml", templates::DEVICES_TEMPLATE),
        ("accounts.yaml", templates::ACCOUNTS_TEMPLATE),
    ];
    for (name, content) in registry_templates {
        let p = etc.join(name);
        if p.exists() {
            eprintln!("init: {} 已存在，跳过", p.display());
        } else {
            std::fs::write(&p, content).map_err(|e| format!("写入 {} 失败: {e}", p.display()))?;
            println!("已生成 {}", p.display());
        }
    }

    // ③ admin 引导账号（env 提供且尚无任何 admin 时追加——entrypoint ⑤ 归位）
    bootstrap_admin_from_env(&etc.join("accounts.yaml"))?;

    // ④ Caddyfile（web 端口 env MEDIASERVO_WEB_PORT 可覆盖——多实例/drill 友好）
    let caddy_path = etc.join("Caddyfile");
    let web_port = std::env::var("MEDIASERVO_WEB_PORT")
        .ok()
        .and_then(|v| v.parse::<u16>().ok())
        .unwrap_or(DEFAULT_WEB_PORT);
    let backend_port = std::fs::read_to_string(&cfg_path)
        .ok()
        .and_then(|c| parse_listen_port(&c))
        .unwrap_or(9800);
    if caddy_path.exists() {
        eprintln!("init: {} 已存在，跳过", caddy_path.display());
    } else {
        let root = absolute(&dir.join("web"));
        std::fs::write(&caddy_path, render_caddyfile(web_port, backend_port, &root))
            .map_err(|e| format!("写入 Caddyfile 失败: {e}"))?;
        println!("已生成 {}（:{web_port} → 127.0.0.1:{backend_port}）", caddy_path.display());
    }

    // ⑤ oxfile（静态 2 条目；存在不覆盖——手工 env 编辑保留）
    let oxfile = run.join("oxfile.toml");
    if oxfile.exists() {
        eprintln!("init: {} 已存在，跳过", oxfile.display());
    } else {
        write_oxfile(&run)?;
    }
    println!(
        "init: 实例就绪 {}（start 后用 server 日志 setup-token 建号，或 MEDIASERVO_ADMIN_PASSWORD 重跑 init）",
        dir.display()
    );
    Ok(())
}

/// 写 run/oxfile.toml（init 与 start-缺文件兜底共用）。
fn write_oxfile(run_dir: &Path) -> Result<(), String> {
    let dir = run_dir.parent().ok_or("oxfile 渲染需实例根目录")?;
    let bin = std::env::current_exe().map_err(|e| format!("current_exe 失败: {e}"))?;
    let ox = render_oxfile(&absolute(dir), &bin, &caddy_command(), &sfu_env_passthrough());
    let path = run_dir.join("oxfile.toml");
    std::fs::write(&path, ox).map_err(|e| format!("写入 {} 失败: {e}", path.display()))?;
    println!("已生成 {}", path.display());
    Ok(())
}

/// init 时把 SFU 相关 env 烘进 oxfile [apps.env]（drill/多实例端口隔离入口）。
fn sfu_env_passthrough() -> Vec<(String, String)> {
    ["MEDIASERVO_SFU_PORT", "MEDIASERVO_SFU_ANNOUNCED_IP"]
        .iter()
        .filter_map(|k| std::env::var(k).ok().map(|v| (k.to_string(), v)))
        .collect()
}

/// caddy 命令：PATH 命中则绝对化（oxmgr daemon 继承的 PATH 与 CLI 可能不同），否则字面量。
fn caddy_command() -> String {
    which("caddy").unwrap_or_else(|| "caddy".to_string())
}

pub(super) fn which(bin: &str) -> Option<String> {
    std::env::split_paths(&std::env::var("PATH").unwrap_or_default()).find_map(|d| {
        let p = d.join(bin);
        (p.is_file() && is_executable(&p)).then(|| p.to_string_lossy().into_owned())
    })
}

#[cfg(unix)]
fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(_p: &Path) -> bool {
    true
}

/// entrypoint ⑤：env MEDIASERVO_ADMIN_PASSWORD 存在时引导 admin（env 读取壳，逻辑在 bootstrap_admin）。
fn bootstrap_admin_from_env(path: &Path) -> Result<(), String> {
    let pass = std::env::var("MEDIASERVO_ADMIN_PASSWORD").unwrap_or_default();
    if pass.trim().is_empty() {
        return Ok(());
    }
    bootstrap_admin(path, &pass)
}

/// 账号表无 admin 时追加引导 admin（幂等；密码显式入参——避免测试 env 竞争）。
pub fn bootstrap_admin(path: &Path, pass: &str) -> Result<(), String> {
    if pass.trim().is_empty() {
        return Ok(());
    }
    let text = std::fs::read_to_string(path).map_err(|e| format!("读取 accounts.yaml 失败: {e}"))?;
    if text.lines().any(|l| l.trim() == "admin:") {
        eprintln!("init: admin 账号已存在，跳过引导");
        return Ok(());
    }
    let mut body = text.replace("accounts: {}", "accounts:");
    if !body.ends_with('\n') {
        body.push('\n');
    }
    body.push_str(&render_admin_account("admin", pass));
    std::fs::write(path, body).map_err(|e| format!("写入 accounts.yaml 失败: {e}"))?;
    println!("init: 已创建引导 admin 账号（bootstrap）");
    Ok(())
}

/// 凭据文件写入 + 0600（unix）。
fn write_private(path: &Path, data: &[u8]) -> Result<(), String> {
    let mut f = std::fs::File::create(path).map_err(|e| format!("创建 {} 失败: {e}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        f.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("{} 权限设置失败: {e}", path.display()))?;
    }
    f.write_all(data).map_err(|e| format!("写入 {} 失败: {e}", path.display()))
}

// ── start / stop / restart ───────────────────────────────────────────────────

/// dev 闭环决策（design「dev 模式闭环」）：显式 --no-web 或 caddy 不在 PATH → 降级仅后端。
/// 返回 (降级后 no_web, 是否因 caddy 缺失降级[warn 标记])。纯函数可单测。
pub fn decide_no_web(no_web_flag: bool, caddy_found: bool) -> (bool, bool) {
    if no_web_flag {
        (true, false)
    } else if !caddy_found {
        (true, true)
    } else {
        (false, false)
    }
}

/// 解析 `start/restart` 参数：`--no-web` flag + 单位置目录。Err(i32) = 已打印的退出码。
fn parse_start_args(args: &mut dyn Iterator<Item = String>) -> Result<(PathBuf, bool), i32> {
    let mut no_web = false;
    let mut positional: Vec<String> = Vec::new();
    for a in &mut *args {
        match a.as_str() {
            "--no-web" => no_web = true,
            s if s.starts_with("--") => {
                eprintln!("未知参数: {s}");
                return Err(2);
            }
            s => positional.push(s.to_string()),
        }
    }
    if positional.len() > 1 {
        eprintln!("实例目录仅接受一个位置参数");
        return Err(2);
    }
    let mut it = positional.into_iter();
    parse_dir(&mut it).inspect_err(|e| eprintln!("{e}")).map_err(|_| 2).map(|d| (d, no_web))
}

fn cmd_start(args: &mut dyn Iterator<Item = String>, verb: &str) -> i32 {
    let (dir, no_web_flag) = match parse_start_args(args) {
        Ok(v) => v,
        Err(code) => return code,
    };
    start_impl(&dir, no_web_flag, verb)
}

fn start_impl(dir: &Path, no_web_flag: bool, verb: &str) -> i32 {
    if !dir.join("etc").join("server.yaml").exists() {
        eprintln!("{verb}: {} 无实例配置——先 `{} init <dir>`", dir.display(), server_product());
        return 2;
    }
    // oxfile：init 产物为准（手工编辑保留——drill 的 SFU 端口注入路径）；缺失才重渲染
    let oxfile = dir.join("run").join("oxfile.toml");
    if !oxfile.exists() && let Err(e) = write_oxfile(&dir.join("run")) {
        eprintln!("{verb}: {e}");
        return 1;
    }
    let cfg = match std::fs::read_to_string(dir.join("etc").join("server.yaml")) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{verb}: 读 server.yaml 失败: {e}");
            return 1;
        }
    };
    let port = match parse_listen_port(&cfg) {
        Some(p) => p,
        None => {
            eprintln!("{verb}: server.yaml 解析失败（listen.port 不可读）——先跑 doctor");
            return 1;
        }
    };
    let (no_web, caddy_missing) = decide_no_web(no_web_flag, which("caddy").is_some());
    if caddy_missing {
        eprintln!("{verb}: caddy 不在 PATH——warn 自动降级 --no-web（dev 形态；严格检查用 doctor）");
    }
    let web_port = std::fs::read_to_string(dir.join("etc").join("Caddyfile"))
        .ok()
        .and_then(|cf| parse_web_port(&cf))
        .unwrap_or(DEFAULT_WEB_PORT);
    // 端口竞争防护（host start 同模式：交互 y 接管 / 非 tty 退出）
    let mut busy: Option<(&str, u16)> = None;
    if inspect::port_in_use(port) {
        busy = Some(("后端监听口", port));
    } else if !no_web && inspect::port_in_use(web_port) {
        busy = Some(("web 口", web_port));
    }
    if let Some((what, p)) = busy {
        return contention_flow(verb, dir, what, p, no_web);
    }
    apply_oxfile(dir, &oxfile, no_web, verb)
}

fn apply_oxfile(dir: &Path, oxfile: &Path, no_web: bool, verb: &str) -> i32 {
    let mut oxargs: Vec<String> = vec!["apply".into(), oxfile.to_string_lossy().into_owned()];
    if no_web {
        oxargs.extend(["--only".into(), server_product()]);
    }
    let refs: Vec<&str> = oxargs.iter().map(String::as_str).collect();
    let code = run_oxmgr(Some(dir), &refs);
    if code == 0 {
        let cluster = if no_web {
            format!("仅后端（--no-web）: {svc}", svc = server_product())
        } else {
            format!("整簇: {svc} + {web}", svc = server_product(), web = server_web_app())
        };
        println!("{verb}: 已应用 {} — {cluster}", oxfile.display());
    }
    code
}

/// 端口占用 → 定位旧 server 实例 → 交互接管（stop 旧簇）/ 非 tty 退出（host PIT-155 模式）。
fn contention_flow(verb: &str, dir: &Path, what: &str, port: u16, no_web: bool) -> i32 {
    eprintln!("{verb}: {what} {port} 被占用——检测到另一进程在监听");
    let old = inspect::find_other_server_dir(dir);
    match &old {
        Some(od) => eprintln!("  旧实例目录: {}", od.display()),
        None => eprintln!("  （未能定位旧实例目录——端口可能被其他程序占用）"),
    }
    if !std::io::stdin().is_terminal() {
        eprintln!("  非交互环境——退出（多实例共存: 各实例配不同 listen.port + MEDIASERVO_WEB_PORT 后重新 init）");
        return 1;
    }
    eprint!("  输入 y 接管（停止旧实例并启动当前）/ 其他键退出: ");
    let _ = std::io::stdout().flush();
    let mut ans = String::new();
    let _ = std::io::stdin().read_line(&mut ans);
    if !ans.trim().eq_ignore_ascii_case("y") {
        return 1;
    }
    let Some(od) = old else {
        eprintln!("接管中止: 未定位旧实例目录——继续启动会与存活进程混战（PIT-155 教训）");
        return 1;
    };
    eprintln!("接管: 停止旧实例 {}", od.display());
    stop_cluster_in(&od);
    apply_oxfile(dir, &dir.join("run").join("oxfile.toml"), no_web, verb)
}

/// `stop [<dir>]`：按注册名 stop+delete + 实例 daemon 收敛（stop 后本实例无常驻进程）。
fn cmd_stop(args: &mut dyn Iterator<Item = String>) -> i32 {
    let dir = match parse_dir(args) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("{e}");
            return 2;
        }
    };
    stop_cluster(&dir)
}

fn stop_cluster(dir: &Path) -> i32 {
    let names = match inspect::list_registered_apps(dir) {
        Ok(n) => n,
        Err(e) => {
            // daemon 未起/oxmgr 缺失 → 无受管进程可停（幂等成功）
            eprintln!("stop: {e}");
            println!("stop: 视为无运行中实例（无已管理进程）");
            return 0;
        }
    };
    let mut failed = 0;
    for name in &names {
        if run_oxmgr(Some(dir), &["stop", name]) != 0 {
            failed = 1;
        }
        if run_oxmgr(Some(dir), &["delete", name]) != 0 {
            failed = 1;
        }
    }
    // 实例 daemon 收敛（best-effort——从未 apply 则无 daemon，正常）
    if !names.is_empty() && run_oxmgr(Some(dir), &["daemon", "stop"]) != 0 {
        eprintln!("stop: 实例 daemon 停止命令未成功（可能已自行退出）");
    }
    if failed == 0 {
        println!(
            "stop: 已停止 {n} 个进程并收敛实例 daemon（{prod} 簇）",
            n = names.len(),
            prod = server_product()
        );
    }
    failed
}

fn stop_cluster_in(od: &Path) {
    if let Ok(names) = inspect::list_registered_apps(od) {
        for name in &names {
            let _ = run_oxmgr(Some(od), &["stop", name]);
            let _ = run_oxmgr(Some(od), &["delete", name]);
        }
        let _ = run_oxmgr(Some(od), &["daemon", "stop"]);
    }
}

/// `restart [<dir>] [--no-web]`：stop 后重新 apply（守卫复用 start 路径）。
fn cmd_restart(args: &mut dyn Iterator<Item = String>) -> i32 {
    let (dir, no_web_flag) = match parse_start_args(args) {
        Ok(v) => v,
        Err(code) => return code,
    };
    if stop_cluster(&dir) != 0 {
        eprintln!("restart: 停止阶段有失败，仍继续重起（oxmgr 对存活 app 幂等）");
    }
    start_impl(&dir, no_web_flag, "restart")
}

/// monit/ps——oxmgr TUI/列表直通（实例 env；目录走智能默认）。
fn cmd_oxmgr_proxy(fixed: &[&str]) -> i32 {
    let dir = parse_dir(&mut std::iter::empty()).unwrap_or_else(|_| PathBuf::from("."));
    let mut cmd = oxmgr_cmd(&dir);
    cmd.args(fixed);
    match cmd.status() {
        Ok(st) => st.code().unwrap_or(1),
        Err(e) => {
            eprintln!("无法执行 oxmgr: {e}");
            eprintln!("提示: deploy 实例 oxmgr 在 <prefix>/bin——export PATH 后重试");
            1
        }
    }
}

// ── oxmgr 管线（实例 daemon env——C32 隔离，与 host translate::oxmgr_env 同源派生）──

pub(super) fn oxmgr_bin() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("oxmgr")))
        .filter(|p| p.is_file())
        .unwrap_or_else(|| PathBuf::from("oxmgr"))
}

fn oxmgr_cmd(dir: &Path) -> Command {
    let mut cmd = Command::new(oxmgr_bin());
    // 绝对化（相对路径依赖 daemon cwd——锚点/重启错位）
    let home = absolute(&dir.join("run").join("oxmgr"));
    let port = instance_daemon_port(&home);
    cmd.env("OXMGR_HOME", &home)
        .env("OXMGR_DATA_DIR", &home) // C32 记忆用名——双设兼容
        .env("OXMGR_DAEMON_ADDR", format!("127.0.0.1:{port}"))
        .env("OXMGR_API_ADDR", format!("127.0.0.1:{}", port + 1000));
    cmd
}

pub(super) fn run_oxmgr(dir: Option<&Path>, args: &[&str]) -> i32 {
    let mut cmd = match dir {
        Some(d) => oxmgr_cmd(d),
        None => Command::new(oxmgr_bin()),
    };
    match cmd.args(args).status() {
        Ok(st) => st.code().unwrap_or(1),
        Err(e) => {
            eprintln!("oxmgr 执行失败: {e} — 安装 OxMgr（GitHub Releases 预编译 / cargo install，见 https://github.com/Vladimir-Urik/OxMgr#install）");
            1
        }
    }
}

pub(super) fn oxmgr_list(dir: &Path) -> Result<Vec<serde_json::Value>, String> {
    let out = oxmgr_cmd(dir)
        .args(["list", "--json"])
        .output()
        .map_err(|e| format!("oxmgr 执行失败: {e} — 请先安装 OxMgr 并加入 PATH"))?;
    if !out.status.success() {
        return Err(format!("oxmgr list 失败: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    serde_json::from_slice(&out.stdout).map_err(|e| format!("解析 oxmgr list --json 失败: {e}"))
}

pub(super) fn absolute(p: &Path) -> PathBuf {
    std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_web_downgrade_decision() {
        assert_eq!(decide_no_web(true, true), (true, false)); // 显式 --no-web
        assert_eq!(decide_no_web(true, false), (true, false)); // 显式优先，无 warn 标记
        assert_eq!(decide_no_web(false, false), (true, true)); // caddy 缺失自动降级 + warn
        assert_eq!(decide_no_web(false, true), (false, false)); // 整簇
    }

    #[test]
    fn init_is_idempotent_and_secret_safe() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path();
        init_instance(dir).expect("first init");
        let cfg = dir.join("etc").join("server.yaml");
        let ox_first =
            std::fs::read_to_string(dir.join("run").join("oxfile.toml")).expect("oxfile");
        // 篡改后二次 init 必须不覆盖（PIT-160）
        std::fs::write(&cfg, "edited by user").expect("tamper");
        init_instance(dir).expect("second init");
        assert_eq!(std::fs::read_to_string(&cfg).expect("read"), "edited by user");
        assert_eq!(
            std::fs::read_to_string(dir.join("run").join("oxfile.toml")).expect("read"),
            ox_first
        );
        for f in [
            "etc/server.yaml",
            "etc/devices.yaml",
            "etc/accounts.yaml",
            "etc/Caddyfile",
            "run/oxfile.toml",
        ] {
            assert!(dir.join(f).exists(), "{f} 缺失");
        }
    }

    #[cfg(unix)]
    #[test]
    fn server_yaml_is_0600_and_secrets_fresh() {
        use std::os::unix::fs::PermissionsExt;
        let a = tempfile::tempdir().expect("tempdir a");
        init_instance(a.path()).expect("init a");
        let md = std::fs::metadata(a.path().join("etc/server.yaml")).expect("md");
        assert_eq!(md.permissions().mode() & 0o777, 0o600);
        let b = tempfile::tempdir().expect("tempdir b");
        init_instance(b.path()).expect("init b");
        let sa = std::fs::read_to_string(a.path().join("etc/server.yaml")).expect("read a");
        let sb = std::fs::read_to_string(b.path().join("etc/server.yaml")).expect("read b");
        assert_ne!(sa, sb, "两次 init 的随机凭据不得相同");
        // jwt 与 admin_jwt 同值（pairing fail-fast 门 C35）
        let jwt = sa.lines().find(|l| l.starts_with("jwt_secret:")).expect("jwt line");
        let adjwt = sa.lines().find(|l| l.starts_with("admin_jwt_secret:")).expect("admin line");
        assert_eq!(jwt.split(':').nth(1).expect("v1"), adjwt.split(':').nth(1).expect("v2"));
    }

    #[test]
    fn admin_bootstrap_is_idempotent() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let acc = tmp.path().join("accounts.yaml");
        std::fs::write(&acc, templates::ACCOUNTS_TEMPLATE).expect("write");
        // 空密码：不动
        bootstrap_admin(&acc, "").expect("empty-pass no-op");
        assert!(!std::fs::read_to_string(&acc).expect("read").contains("role: admin"));
        // 引导一次，二次不重复（显式入参，无 env 竞争）
        bootstrap_admin(&acc, "pw-test").expect("bootstrap 1");
        bootstrap_admin(&acc, "pw-test").expect("bootstrap 2");
        let after = std::fs::read_to_string(&acc).expect("read");
        assert_eq!(after.matches("role: admin").count(), 1, "admin 引导幂等");
        assert!(after.contains("  admin:"));
        serde_yaml::from_str::<serde_yaml::Value>(&after).expect("bootstrap 后仍是合法 YAML");
    }

    #[test]
    fn dir_flag_guard_rejects_dashdash() {
        let mut it = ["--dir".to_string(), "/tmp/x".to_string()].into_iter();
        assert!(parse_dir(&mut it).is_err(), "--dir 类笔误必须拒绝（host init --dir 事故同训）");
    }
}
