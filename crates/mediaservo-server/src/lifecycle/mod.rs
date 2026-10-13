//! `msrtc-server` 实例生命周期（frontend-process-split T15-T19，单二进制双角色之管理面）。
//!
//! 引擎 = oxmgr（与 host 同构，design.md「Server 进程管理：修正版」）。拓扑 2~3 进程
//! （server + caddy web + weaknet-serve 条件并管），oxfile 静态模板免翻译层。所有 oxmgr 调用走**实例 daemon**
//! （OXMGR_HOME/OXMGR_DAEMON_ADDR 派生自实例目录——C32 隔离，基数 18500 避开 host 系）。
//!
//! 向后兼容硬门：main.rs 仅在首参命中 LIFECYCLE_CMDS 时进入本模块；
//! `run`/无参/`--config`/未知参数 = 守护模式原样（systemd/compose/docker 直启零破坏）。

mod conflict;
mod inspect;
mod reap;
mod startup;
pub mod templates;

use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use reap::{DirectReason, LiveProc, ProcKind, ReapStep, StepOutcome};

use templates::{
    DEFAULT_WEB_PORT, SFU_ENV_KEYS, WEAKNET_APP, WEAKNET_SERVE_PORT, gen_secret,
    instance_daemon_port, looks_like_oxfile, manual_env_keys, merge_manual_env, oxfile_has_weaknet,
    parse_listen_port, parse_weaknet_port, parse_web_port, render_admin_account, render_caddyfile,
    render_oxfile, render_server_yaml, resolve_port, server_product, server_web_app,
    weaknet_listen,
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

    // ④ Caddyfile（config-unification P1 读序：yaml > env MEDIASERVO_WEB_PORT > 缺省 8080）
    let caddy_path = etc.join("Caddyfile");
    let yaml_cfg = std::fs::read_to_string(&cfg_path)
        .ok()
        .and_then(|c| serde_yaml::from_str::<mediaservo_common::config::ServerConfig>(&c).ok());
    let (web_port, web_root_override, weaknet_port) = resolved_ports(yaml_cfg.as_ref());
    let backend_port =
        std::fs::read_to_string(&cfg_path).ok().and_then(|c| parse_listen_port(&c)).unwrap_or(9800);
    if caddy_path.exists() {
        eprintln!("init: {} 已存在，跳过", caddy_path.display());
    } else {
        let root = web_root_override.unwrap_or_else(|| absolute(&dir.join("web")));
        std::fs::write(
            &caddy_path,
            render_caddyfile(web_port, backend_port, &root, &weaknet_listen(weaknet_port)),
        )
        .map_err(|e| format!("写入 Caddyfile 失败: {e}"))?;
        println!("已生成 {}（:{web_port} → 127.0.0.1:{backend_port}）", caddy_path.display());
    }

    // ⑤ oxfile（静态 2~3 条目，weaknet 条件渲染 [M-1]；存在不覆盖——手工 env 编辑保留，
    //    陈旧迁移=Python _drop_stale_server_oxfile 单点 [bF3]）
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

/// 统一端口读序（config-unification P1）：yaml(Option 字段) > env > 常量缺省。
/// init ④ 与 write_oxfile 共用——消灭双份读序漂移。yaml_cfg=None（缺文件/解析失败）= 全兜底。
fn resolved_ports(
    yaml_cfg: Option<&mediaservo_common::config::ServerConfig>,
) -> (u16, Option<std::path::PathBuf>, u16) {
    let env_u16 = |k: &str| std::env::var(k).ok().and_then(|v| v.parse::<u16>().ok());
    let web = yaml_cfg.map(|c| &c.web);
    let sfu = yaml_cfg.map(|c| &c.sfu);
    let web_port =
        resolve_port(web.and_then(|w| w.port), env_u16("MEDIASERVO_WEB_PORT"), DEFAULT_WEB_PORT);
    let weaknet_port = resolve_port(sfu.and_then(|s| s.weaknet_port), None, WEAKNET_SERVE_PORT);
    let root = web.and_then(|w| w.static_root.clone()).map(std::path::PathBuf::from);
    (web_port, root, weaknet_port)
}

/// 写 run/oxfile.toml（init 与 start-缺文件兜底共用——Rust 侧唯一 oxfile 写点）。
/// PIT-160 根修（PLAN §2.4）：重渲染不从零——合并源 = live oxfile 在场用之，否则
/// .bak（上次成功渲染快照，覆盖「用户删文件后 start 重渲」与「Python deploy stale
/// 备份后 init 重渲」两路），双缺 = 首渲纯渲染；写毕以终态刷新 .bak（多轮重渲 N 不
/// 翻倍——快照随合并结果推进）。.bak 对 oxmgr 惰性（只读显式 oxfile 路径）。
fn write_oxfile(run_dir: &Path) -> Result<(), String> {
    let dir = run_dir.parent().ok_or("oxfile 渲染需实例根目录")?;
    let bin = std::env::current_exe().map_err(|e| format!("current_exe 失败: {e}"))?;
    let yaml_cfg = std::fs::read_to_string(dir.join("etc").join("server.yaml"))
        .ok()
        .and_then(|c| serde_yaml::from_str::<mediaservo_common::config::ServerConfig>(&c).ok());
    let (_, _, weaknet_port) = resolved_ports(yaml_cfg.as_ref());
    let new_text =
        render_oxfile(&absolute(dir), &bin, &caddy_command(), &sfu_env_passthrough(), weaknet_port);
    let path = run_dir.join("oxfile.toml");
    let bak = run_dir.join("oxfile.toml.bak");
    let (old, source) = match std::fs::read_to_string(&path) {
        Ok(t) => (Some(t), "run/oxfile.toml"),
        Err(_) => match std::fs::read_to_string(&bak) {
            Ok(t) => (Some(t), "run/oxfile.toml.bak"),
            Err(_) => (None, ""),
        },
    };
    let corrupt = old.as_deref().is_some_and(|o| !o.trim().is_empty() && !looks_like_oxfile(o));
    let text = match old.as_deref() {
        Some(o) if !corrupt => merge_manual_env(o, &new_text),
        _ => new_text,
    };
    let (merged, lost) = match old.as_deref() {
        Some(o) if !corrupt => {
            let want = manual_env_keys(o);
            let have = manual_env_keys(&text);
            let m = want.iter().filter(|k| have.contains(k)).count();
            (m, want.len() - m)
        }
        _ => (0, 0),
    };
    std::fs::write(&path, &text).map_err(|e| format!("写入 {} 失败: {e}", path.display()))?;
    println!("已生成 {}", path.display());
    if let Err(e) = std::fs::write(&bak, &text) {
        eprintln!("警告: {} 快照写入失败（{e}）——下次重渲染无手工 env 可回吸收", bak.display());
    }
    if corrupt {
        eprintln!(
            "警告: 旧 {source} 非本程序渲染形——本次退化为纯渲染，手工 env 未回吸收（PIT-160 兜底，不阻塞 start）"
        );
    }
    eprintln!("{}", pit160_note(merged, lost));
    Ok(())
}

/// PIT-160 提示行（纯函数可测）：无手工条目 → 旧指引文本逐字保留；有则「已合并 N 条
/// 手工 env」变体 + 未并（新渲染无对应 app 条目）计数。渲染器受管键（MEDIASERVO_SFU_*
/// 与 WEAKNET_STATEDIR）恒以本次渲染为准（baked-wins）、不计入 N。
fn pit160_note(merged: usize, lost: usize) -> String {
    if merged == 0 && lost == 0 {
        return "提示: 若本树此前存在手工 [apps.env] 行（ALLOW_DEV_*/RUST_LOG/WEAKNET_ADMIN_PASS），本次渲染不含它们——按 msrtc.sh -h「环境变量 [B]」回补".to_string();
    }
    let tail = if lost > 0 {
        format!("；另有 {lost} 条因新渲染无对应 app 条目未并入——按 msrtc.sh -h「环境变量 [B]」回补")
    } else {
        String::new()
    };
    format!(
        "提示: 已合并 {merged} 条手工 env（来源=上次渲染快照；受管键 MEDIASERVO_SFU_*/WEAKNET_STATEDIR 以本次渲染为准）{tail}"
    )
}

/// weaknet 探针口 = oxfile 反解（`serve --listen 127.0.0.1:NNNN`）——与渲染产物同源；
/// 反解失败（旧 oxfile 无条目/手工损形）回退缺省常量。
fn weaknet_probe_port(oxfile: &Path) -> u16 {
    std::fs::read_to_string(oxfile)
        .ok()
        .and_then(|t| parse_weaknet_port(&t))
        .unwrap_or(WEAKNET_SERVE_PORT)
}

/// init 时把 SFU 相关 env 烘进 oxfile [apps.env]（drill/多实例端口隔离入口）。
/// 透传键集 = SFU_ENV_KEYS 唯一真源；merge 排除集为 MANAGED_ENV_KEYS（本集 + weaknet
/// 烘值键——尾刀 baked-wins 裁决：env 透传与模板烘值两族受管键分集命名，各自单源）。
fn sfu_env_passthrough() -> Vec<(String, String)> {
    SFU_ENV_KEYS.iter().filter_map(|k| std::env::var(k).ok().map(|v| (k.to_string(), v))).collect()
}

/// caddy 命令探测链（2026-09-29 自适应化）：① PATH ② 本二进制同目录（out/server/bin
/// 自包含部署——caddy 随包）③ 仓内 pixi 树（源码机）。全落空 = 字面量（保持现降级语义）。
fn caddy_command() -> String {
    caddy_probe().unwrap_or_else(|| "caddy".to_string())
}

fn caddy_probe() -> Option<String> {
    if let Some(p) = which("caddy") {
        return Some(p);
    }
    // ② exe 同目录（部署树 bin/ 三件套形态）
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let p = dir.join("caddy");
        if p.is_file() {
            return Some(p.to_string_lossy().into_owned());
        }
    }
    // ③ 仓内 pixi 树（源码机直跑 target/debug 形态；构建期路径，跨机部署由 ② 接管）
    // （单候选——原数组循环形，1.97 clippy single_element_loop 直线化，语义零差）
    let cand = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.pixi/envs/default/bin/caddy");
    let p = cand.canonicalize().unwrap_or(cand.clone());
    if p.is_file() {
        return Some(p.to_string_lossy().into_owned());
    }
    None
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
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("读取 accounts.yaml 失败: {e}"))?;
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
    let mut f =
        std::fs::File::create(path).map_err(|e| format!("创建 {} 失败: {e}", path.display()))?;
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
    if !oxfile.exists()
        && let Err(e) = write_oxfile(&dir.join("run"))
    {
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
    let (no_web, caddy_missing) = decide_no_web(no_web_flag, caddy_probe().is_some());
    if caddy_missing {
        eprintln!("{verb}: caddy 不在 PATH——warn 自动降级 --no-web（dev 形态；严格检查用 doctor）");
    }
    let web_port = std::fs::read_to_string(dir.join("etc").join("Caddyfile"))
        .ok()
        .and_then(|cf| parse_web_port(&cf))
        .unwrap_or(DEFAULT_WEB_PORT);
    // weaknet 条目在场（oxfile 文本判据——与 render_oxfile 条件渲染同源 [BA-9]）：
    // 决定 9810 探针 + --no-web 白名单是否保留 weaknet-serve。
    let weaknet_cluster = std::fs::read_to_string(&oxfile).is_ok_and(|t| oxfile_has_weaknet(&t));
    // 端口竞争防护（host start 同模式：交互 y 接管 / 非 tty 退出）
    let mut busy: Option<(&str, u16)> = None;
    if inspect::port_in_use(port) {
        busy = Some(("后端监听口", port));
    } else if !no_web && inspect::port_in_use(web_port) {
        busy = Some(("web 口", web_port));
    } else if weaknet_cluster && inspect::port_in_use(weaknet_probe_port(&oxfile)) {
        // BA-9：外部 dev serve 占 weaknet 口时簇带 crash-loop unit 假「running」——被占报因拒启
        // 探针口 = oxfile 反解（与渲染同源；反解失败回退常量——config-unification P1）
        busy = Some(("weaknet 面板口", weaknet_probe_port(&oxfile)));
    }
    if let Some((what, p)) = busy {
        return contention_flow(verb, dir, what, p, no_web);
    }
    apply_oxfile(dir, &oxfile, no_web, verb)
}

/// `--no-web` 的 apply 白名单（纯函数单测锚点，lck-F8）：**只剔 web，绝不连坐剔 weaknet**
/// （D5-6）——oxmgr 0.5.0 `--only` = 白名单形制实测：非白名单条目被 stop（不删除），
/// 三条目下缺省白名单必须含 weaknet-serve（条目在场时）。逗号并列形实测同义（`--only a,b`）。
pub fn no_web_only_list(weaknet_in_oxfile: bool) -> String {
    if weaknet_in_oxfile {
        format!("{},{}", server_product(), WEAKNET_APP)
    } else {
        server_product()
    }
}

fn apply_oxfile(dir: &Path, oxfile: &Path, no_web: bool, verb: &str) -> i32 {
    let has_wnet = std::fs::read_to_string(oxfile).is_ok_and(|t| oxfile_has_weaknet(&t));
    let mut oxargs: Vec<String> = vec!["apply".into(), oxfile.to_string_lossy().into_owned()];
    if no_web {
        oxargs.extend(["--only".into(), no_web_only_list(has_wnet)]);
    }
    let refs: Vec<&str> = oxargs.iter().map(String::as_str).collect();
    let code = run_oxmgr(Some(dir), &refs);
    if code == 0 {
        // BA-6 四点同步之 apply_oxfile 文案：三条目形如实枚举（weaknet 在场才报）
        let wnet_part = if has_wnet { format!(" + {WEAKNET_APP}") } else { String::new() };
        let cluster = if no_web {
            format!("仅后端（--no-web）: {svc}{wnet_part}", svc = server_product())
        } else {
            format!(
                "整簇: {svc} + {web}{wnet_part}",
                svc = server_product(),
                web = server_web_app()
            )
        };
        println!("{verb}: 已应用 {} — {cluster}", oxfile.display());
    }
    code
}

/// 端口占用 → 定位旧 server 实例 → 交互接管（stop 旧簇）/ 非 tty 退出（host PIT-155 模式）。
fn contention_flow(verb: &str, dir: &Path, what: &str, port: u16, no_web: bool) -> i32 {
    eprintln!("{verb}: {what} {port} 被占用——检测到另一进程在监听");
    // start-conflict-doctor T2：占用者指认（pid/exe/deleted/父 daemon）——指认不杀戮（F1）。
    let reports = conflict::diagnose_port(port);
    conflict::print_report(&reports, port);
    let old = inspect::find_other_server_dir(dir);
    match &old {
        Some(od) => eprintln!("  旧实例目录: {}", od.display()),
        None => {
            match inspect::strays_under(&inspect::read_default_registry(), dir).into_iter().next() {
                // T5: /proc 扫不到 → 占用者可能在共享/旧 daemon 名下（非本实例注册表管辖
                // ——2026-10-12 事故形）。指认不杀戮：接管路径仍拒（old=None 语义不变）。
                Some(e) => {
                    eprintln!(
                        "  端口占用者在旧共享 daemon 名下: name={} id={} status={} pid={}（非本实例注册表管辖）—— {}",
                        e.name,
                        e.id,
                        e.status,
                        e.pid.map_or_else(|| "?".into(), |p| p.to_string()),
                        inspect::SPLIT_GUIDANCE
                    );
                    // global-reap T4：文案闭环——T5 指认后给一键出口
                    eprintln!("  可用 msrtc-server stop -g 收口（全局按进程身份清场）");
                }
                None => eprintln!("  （未能定位旧实例目录——端口可能被其他程序占用）"),
            }
        }
    }
    if !std::io::stdin().is_terminal() {
        eprintln!(
            "  非交互环境——退出（多实例共存: 各实例配不同 listen.port + MEDIASERVO_WEB_PORT 后重新 init）"
        );
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

/// stop 参数形（纯解析，单测锚）：`stop` 默认目录 / `stop <dir>` / `stop -g|--all`；
/// `-g` 与目录并给 = 参数非法（exit 2）。
#[derive(Debug, PartialEq, Eq)]
pub(super) enum StopArgs {
    Dir(Option<String>),
    Global,
}

pub(super) fn parse_stop_args(args: &mut dyn Iterator<Item = String>) -> Result<StopArgs, String> {
    let mut global = false;
    let mut positional: Vec<String> = Vec::new();
    for a in &mut *args {
        match a.as_str() {
            "-g" | "--all" => global = true,
            s if s.starts_with('-') => {
                return Err(format!("未知参数: {s}"));
            }
            s => positional.push(s.to_string()),
        }
    }
    if global {
        if !positional.is_empty() {
            return Err("stop -g 为全局收口，不接受目录参数".to_string());
        }
        return Ok(StopArgs::Global);
    }
    if positional.len() > 1 {
        return Err("实例目录仅接受一个位置参数".to_string());
    }
    Ok(StopArgs::Dir(positional.into_iter().next()))
}

/// `stop [<dir>]`：按注册名 stop+delete + 实例 daemon 收敛（stop 后本实例无常驻进程）。
/// `stop -g|--all`：跨目录跨 daemon 按进程身份全局收口（PLAN §2.4）。
fn cmd_stop(args: &mut dyn Iterator<Item = String>) -> i32 {
    let parsed = match parse_stop_args(args) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            return 2;
        }
    };
    match parsed {
        StopArgs::Global => scan_all(),
        StopArgs::Dir(None) => stop_cluster(
            &parse_dir(&mut std::iter::empty()).unwrap_or_else(|_| PathBuf::from(".server")),
        ),
        StopArgs::Dir(Some(d)) => stop_cluster(&PathBuf::from(d)),
    }
}

fn stop_cluster(dir: &Path) -> i32 {
    let names = match inspect::list_registered_apps(dir) {
        Ok(n) => n,
        Err(e) => {
            // 二审残余#1：daemon 未起/oxmgr 缺失**不早退**——该变体下残留进程恰可能
            // 注册在别的 daemon 名下，必须继续走扫尾（幂等成功语义在尾部统一判定）。
            eprintln!("stop: {e}");
            println!("stop: 视为无运行中实例（无已管理进程）——继续扫尾兜底");
            Vec::new()
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
    // 扫尾兜底（global-reap T2，PLAN §2.2/§2.3）：实例表停货后 /proc 全扫本实例
    // 残留（含注册在共享/默认 daemon 名下的分裂形），经所属 daemon 优雅收口防
    // restart_policy=always 复活；双查 miss 才直杀。
    let reaped_pids = reap_under(dir);
    // 实例 daemon 收敛（M1：挪扫尾后——先收敛会让实例名下残货被迫走裸杀）
    if !names.is_empty() && run_oxmgr(Some(dir), &["daemon", "stop"]) != 0 {
        eprintln!("stop: 实例 daemon 停止命令未成功（可能已自行退出）");
    }
    if failed == 0 {
        // B2 归属归因自证：轮询 ≤5s 三端口——我方残留=失败，陌生者占口=WARN+成功。
        let ports = stop_ports(dir);
        let verified = verify_ports_attribution(&ports, &reaped_pids, dir);
        println!(
            "stop: 已停止 {n} 个进程并收敛实例 daemon（{prod} 簇）{}",
            if verified {
                "——自证清场 ✓（端口级）"
            } else {
                "——⚠ 端口仍被本实例残留占用"
            },
            n = names.len(),
            prod = server_product()
        );
        // T4-a 实环勘误：自证未过 = 本实例残留在场，语义即失败（RC=0 曾令假绿复活——
        // 归因行如实报了 failed 但退出码没跟上）。failed 本身不为 0 的路径已按计数返 1。
        if !verified {
            failed = 1;
        }
    }
    failed
}

/// 本实例三探针口（parse 失败 → skip 该端口不假红，§2.3）。
fn stop_ports(dir: &Path) -> Vec<u16> {
    let mut ports = Vec::new();
    if let Some(p) = std::fs::read_to_string(dir.join("etc").join("server.yaml"))
        .ok()
        .and_then(|c| parse_listen_port(&c))
    {
        ports.push(p);
    }
    if let Some(p) = std::fs::read_to_string(dir.join("etc").join("Caddyfile"))
        .ok()
        .and_then(|cf| parse_web_port(&cf))
    {
        ports.push(p);
    }
    ports.push(weaknet_probe_port(&dir.join("run").join("oxfile.toml")));
    ports.sort();
    ports.dedup();
    ports
}

/// 扫尾执行：签名命中且 dir 归属 ∈ 本实例的存活进程 → reap_plan → 逐个执行。
/// 返回本轮收口的 pid 集（B2 归因用）。判定全在 reap 纯函数；此处只做 IO 薄壳。
fn reap_under(dir: &Path) -> Vec<u32> {
    let live = reap::scan_live_procs();
    let matched: Vec<(&LiveProc, ProcKind)> = live
        .iter()
        .filter_map(|p| {
            let kind = reap::proc_kind(&p.argv, p.exe.as_deref())?;
            if !reap::kind_dir_within(&kind, &p.argv, dir) {
                return None;
            }
            // caddy 磁盘标记（B1）——root 下须有本产品 bin
            if let ProcKind::Caddy { config } = &kind
                && !reap::caddy_marker_ok(config)
            {
                return None;
            }
            Some((p, kind))
        })
        .collect();
    if matched.is_empty() {
        println!("扫尾: 无残留进程");
        return Vec::new();
    }
    // 注册表快照：实例 daemon 表（若目录可读）+ 默认 daemon 表（B3 清洗后读取）。
    let mut registries: Vec<(PathBuf, Vec<inspect::ManagedEntry>)> = Vec::new();
    let inst_dir = dir.join("run").join("oxmgr");
    registries.push((inst_dir.clone(), inspect::read_registry(&inst_dir.join("state.json"))));
    if let Some(dd) = inspect::default_data_dir() {
        registries.push((dd.clone(), inspect::read_registry(&dd.join("state.json"))));
    }
    let refs: Vec<(PathBuf, &[inspect::ManagedEntry])> =
        registries.iter().map(|(d, e)| (d.clone(), e.as_slice())).collect();
    let plan =
        reap::reap_plan(&matched.iter().map(|(p, _)| (*p).clone()).collect::<Vec<_>>(), &refs);
    execute_steps(&plan).into_iter().map(|(pid, _)| pid).collect()
}

/// daemon 存活探测：DAEMON_ADDR 端口可连 = 活（§10 直杀闸判据①）。
fn daemon_alive(daemon_data_dir: &Path) -> bool {
    let home = absolute(daemon_data_dir);
    std::net::TcpStream::connect(("127.0.0.1", instance_daemon_port(&home))).is_ok()
}

/// 注册表条目在否（§10 直杀闸判据②）：name 命中且 pid 相符（或 pid 陈旧但 name 在场
/// ——daemon 活着重拉过 pid 的形态，M2 同源）。
fn registry_has_entry(daemon_data_dir: &Path, name: &str, pid: &u32) -> bool {
    let state = daemon_data_dir.join("state.json");
    inspect::read_registry(&state)
        .iter()
        .any(|e| e.name == name && (e.pid == Some((*pid).into()) || e.pid.is_none()))
}

/// 收口步骤执行（T2/T3 共用薄壳；语义与 T2 内联形逐字节一致——daemon 不可达降级
/// 直杀、直杀横幅、单次补杀上限）。返回 (pid, 结局) 序列供汇总计数。
fn execute_steps(plan: &[ReapStep]) -> Vec<(u32, StepOutcome)> {
    plan.iter()
        .map(|step| match step {
            ReapStep::ViaDaemon { daemon_data_dir, name, pid } => {
                println!(
                    "扫尾: pid {pid}（{name}）→ 经所属 daemon 收口 {}",
                    daemon_data_dir.display()
                );
                if run_oxmgr_for_daemon(daemon_data_dir, &["stop", name]) == 0
                    && run_oxmgr_for_daemon(daemon_data_dir, &["delete", name]) == 0
                {
                    return (*pid, StepOutcome::ViaDaemon);
                }
                // §10 直杀闸（10-13 二次事故）：收口失败 ≠ daemon 死——daemon 可能只是
                // 短暂不可达/被 stop 收敛但条目仍在拉新 pid。直杀许可 = daemon 确认死亡
                // 且注册表无该条目；否则只报告（-g/stop 均不越权，防 always 复活循环）。
                if daemon_alive(daemon_data_dir)
                    || registry_has_entry(daemon_data_dir, name, pid)
                {
                    eprintln!(
                        "扫尾: pid {pid}（{name}）daemon 收口未成功且 daemon 仍活/条目在——跳过直杀（可经 stop <dir> 或修复 daemon 后重试）"
                    );
                    return (*pid, StepOutcome::Failed);
                }
                eprintln!("扫尾: daemon 收口未成功且 daemon 确认死亡——绕过 daemon 直杀");
                kill_and_wait(*pid);
                let ok = !pid_alive(*pid);
                (*pid, if ok { StepOutcome::DirectKill } else { StepOutcome::Failed })
            }
            ReapStep::DirectKill { pid, reason: DirectReason::NoRegistry } => {
                println!("扫尾: pid {pid} → 绕过 daemon 直杀（无所属注册表）");
                kill_and_wait(*pid);
                let ok = !pid_alive(*pid);
                (*pid, if ok { StepOutcome::DirectKill } else { StepOutcome::Failed })
            }
        })
        .collect()
}

/// `stop -g`（PLAN §2.4）：/proc 全扫签名命中者（无 dir 过滤、无端口探查），逐个按
/// 所属 daemon 优雅收口（environ 反查——实例 daemon 名下进程经其自家 home 收口，
/// 注册表快照=默认 daemon 表 + 各 environ home 表），双查 miss 才直杀。汇总表 +
/// 签名命中 pid 终态行 + 「实例 daemon 未动」注记（m4）。退出码 = 失败数语义与
/// 既有 stop 一致（0/1）。
fn scan_all() -> i32 {
    println!("stop -g: 全局收口（按进程身份，跨目录跨 daemon；实例 daemon 本身不动）");
    let live = reap::scan_live_procs();
    let (matched, rejected) = reap::select_global(&live, reap::caddy_marker_ok);
    for p in &rejected {
        println!("stop -g: pid {} caddy 签名命中但磁盘标记缺失（非本产品布局）——跳过", p.pid);
    }
    if matched.is_empty() {
        println!("stop -g: 无本产品进程存活——收口汇总: 经 daemon 0 个 / 直杀 0 个 / 失败 0 个");
        println!("终态: 全部清场 ✓");
        println!("实例 daemon 未动（非目标）");
        return 0;
    }
    // 注册表快照：默认 daemon 表 + matched 进程各自 environ home 的实例表（reap_plan
    // 按 per-proc environ 排序配对——-g 传空实例列表即默认表，无需 dir 语义）。
    // B3 姊妹（T4-b 实环揪出）：default_data_dir() 按 CLI 自身 env 解析，而
    // run_oxmgr_default 给子进程剥 XDG_DATA_HOME——CLI 视角与执行视角的"默认 daemon"
    // 会分叉（XDG 沙箱壳里 -g 误扫真默认 daemon 并直杀生产进程=事故形）。统一为
    // 剥后语义：CLI 侧忽略 XDG_DATA_HOME，与执行子进程同源。
    // （不绕 env 突改——直接按"剥 XDG_DATA_HOME 后"的解析形计算：HOME/.local/share，
    //  与 default_data_dir 的 XDG 回退实现一致，零 unsafe）
    let default_dir = std::env::var_os("XDG_DATA_HOME")
        .filter(|v| !v.is_empty())
        .is_some()
        .then(|| {
            std::env::var_os("HOME")
                .map(|h| PathBuf::from(h).join(".local").join("share").join("oxmgr"))
        })
        .flatten()
        .or_else(inspect::default_data_dir);
    let mut registries: Vec<(PathBuf, Vec<inspect::ManagedEntry>)> = Vec::new();
    if let Some(dd) = default_dir {
        registries.push((dd.clone(), inspect::read_registry(&dd.join("state.json"))));
    }
    for home in matched.iter().filter_map(|p| p.environ_oxmgr_home.clone()) {
        let home = absolute(&home);
        if registries.iter().any(|(d, _)| *d == home) {
            continue;
        }
        registries.push((home.clone(), inspect::read_registry(&home.join("state.json"))));
    }
    let refs: Vec<(PathBuf, &[inspect::ManagedEntry])> =
        registries.iter().map(|(d, e)| (d.clone(), e.as_slice())).collect();
    let plan = reap::reap_plan(&matched, &refs);
    let steps = execute_steps(&plan);
    let (via, direct, mut failed) = reap::tally(&steps.iter().map(|(_, o)| *o).collect::<Vec<_>>());
    // 终态（归因型端口终态打印的 -g 形——ports 逐实例各异不探，按签名二扫裁决）：
    let still: Vec<u32> = reap::scan_live_procs()
        .into_iter()
        .filter(|p| match reap::proc_kind(&p.argv, p.exe.as_deref()) {
            Some(ProcKind::Caddy { config }) => reap::caddy_marker_ok(&config),
            Some(_) => true,
            None => false,
        })
        .map(|p| p.pid)
        .collect();
    if still.is_empty() {
        println!("终态: 全部清场 ✓");
    } else {
        println!("终态: 仍有签名命中存活 pid: {still:?}");
        // T4-b 勘误: 终态残货=未清干净，计入失败（曾只看 tally 的 Failed 步——
        // "杀令已发但进程仍在"之外的复活/漏网形态不被 tally 覆盖）
        failed += still.len();
    }
    println!("收口汇总: 经 daemon {via} 个 / 直杀 {direct} 个 / 失败 {failed} 个");
    println!("实例 daemon 未动（非目标）");
    i32::from(failed > 0)
}

/// 对**指定 daemon 数据目录**发 oxmgr stop/delete（env 四键按 oxmgr_cmd 同法派生）。
fn run_oxmgr_for_daemon(daemon_data_dir: &Path, args: &[&str]) -> i32 {
    let home = absolute(daemon_data_dir);
    let port = instance_daemon_port(&home);
    let mut cmd = Command::new(oxmgr_bin());
    cmd.env("OXMGR_HOME", &home)
        .env("OXMGR_DATA_DIR", &home)
        .env("OXMGR_DAEMON_ADDR", format!("127.0.0.1:{port}"))
        .env("OXMGR_API_ADDR", format!("127.0.0.1:{}", port + 1000));
    match cmd.args(args).status() {
        Ok(st) => st.code().unwrap_or(1),
        Err(_) => 1,
    }
}

/// SIGTERM → ≤3s 等退 → SIGKILL（§2.2 末路）。EPERM（跨用户）记失败不崩溃（m2）。
fn kill_and_wait(pid: u32) {
    unsafe {
        if libc::kill(pid as i32, libc::SIGTERM) != 0 {
            eprintln!(
                "扫尾: SIGTERM pid {pid} 失败（{}）——可能跨用户进程",
                std::io::Error::last_os_error()
            );
            return;
        }
    }
    for _ in 0..30 {
        if !pid_alive(pid) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    unsafe {
        if libc::kill(pid as i32, libc::SIGKILL) == 0 {
            eprintln!("扫尾: pid {pid} 3s 未退——已 SIGKILL");
        }
    }
}

fn pid_alive(pid: u32) -> bool {
    #[cfg(target_os = "linux")]
    {
        std::path::Path::new(&format!("/proc/{pid}")).exists()
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid;
        false
    }
}

/// B2 归属归因端口自证三态（纯判定，单测锚）：Ok(()) = 清场/陌生者 pass，
/// Err(残留报告集) = 本实例进程仍占口。
pub(super) fn port_verdict(
    listeners: &[conflict::ConflictReport],
    ours: &[u32],
) -> Result<(), Vec<conflict::ConflictReport>> {
    let residue: Vec<conflict::ConflictReport> =
        listeners.iter().filter(|r| ours.contains(&r.pid)).cloned().collect();
    if residue.is_empty() { Ok(()) } else { Err(residue) }
}

/// 轮询 ≤5s（10×500ms）三端口归属归因。true = 全部清场或陌生者占口（WARN）。
fn verify_ports_attribution(ports: &[u16], reaped: &[u32], dir: &Path) -> bool {
    // 签名命中本实例 dir 的存活 pid 也算"我方"（sweep 匹配集，B2 归因并集）
    let sweep_matched: Vec<u32> = reap::scan_live_procs()
        .into_iter()
        .filter(|p| {
            reap::proc_kind(&p.argv, p.exe.as_deref())
                .is_some_and(|k| reap::kind_dir_within(&k, &p.argv, dir))
        })
        .map(|p| p.pid)
        .collect();
    let ours: Vec<u32> = reaped.iter().chain(sweep_matched.iter()).copied().collect();
    let mut verified = false;
    for _ in 0..10 {
        let mut verdict_ok = true;
        for &port in ports {
            if !inspect::port_in_use(port) {
                continue;
            }
            let rep = conflict::diagnose_port(port);
            match port_verdict(&rep, &ours) {
                // B2：陌生者占口 ≠ 残留——WARN + pass（TalkServo 实录形不假失败）
                Ok(()) => {
                    for r in &rep {
                        eprintln!(
                            "[warn] 端口 {port} 由非本实例进程持有(pid {} exe {}；可能是另一目录本产品实例)",
                            r.pid, r.exe
                        );
                    }
                }
                Err(residue) => {
                    verdict_ok = false;
                    if !residue.is_empty() {
                        eprintln!("stop: 端口 {port} 仍被本实例残留占用:");
                        for r in &residue {
                            eprintln!("  {}", r.line());
                        }
                    }
                }
            }
        }
        if verdict_ok {
            verified = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    if !verified {
        eprintln!("stop: 端口级自证未过（本实例残留未清）——复核占用者后处置");
    }
    verified
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
        // B3（PLAN §2.2）：默认 daemon 语义 = 显式无 OXMGR_*——std Command 继承父 env，
        // 操作者 shell 残留会把"默认 daemon"操作静默打到实例 daemon（错向同族）。
        None => {
            let mut c = Command::new(oxmgr_bin());
            for k in [
                "OXMGR_HOME",
                "OXMGR_DATA_DIR",
                "OXMGR_DAEMON_ADDR",
                "OXMGR_API_ADDR",
                "XDG_DATA_HOME", // B3 姊妹（T4-a 实环揪出）：XDG 沙箱同样令 default_data_dir
                                 // 解析到非默认 daemon——默认语义=完全无覆盖形态
            ] {
                c.env_remove(k);
            }
            c
        }
    };
    match cmd.args(args).status() {
        Ok(st) => st.code().unwrap_or(1),
        Err(e) => {
            eprintln!(
                "oxmgr 执行失败: {e} — 安装 OxMgr（GitHub Releases 预编译 / cargo install，见 https://github.com/Vladimir-Urik/OxMgr#install）"
            );
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
    fn no_web_only_list_keeps_weaknet_never_web() {
        // lck-F8/D5-6：--no-web 只剔 web——weaknet 条目在场必入白名单（防连坐 stop）
        assert!(no_web_only_list(true).contains(&format!(",{WEAKNET_APP}")));
        assert!(no_web_only_list(true).starts_with(&format!("{},", server_product())));
        // 条目缺位（bin 未装配的条件渲染形）→ 纯 server，不带幽灵名（apply --only 未知名风险）
        assert_eq!(no_web_only_list(false), server_product());
        // 白名单永不含 web（剔除语义=只剔它）
        assert!(!no_web_only_list(true).contains(&server_web_app()));
        assert!(!no_web_only_list(false).contains(&server_web_app()));
    }

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

    // ---- config-unification P1: yaml 端口键贯通 init 渲染产物 ----

    #[test]
    fn init_caddyfile_uses_yaml_web_port() {
        // PIT-13 正对照：断言具体渲染值，非仅"无告警"
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path();
        init_instance(dir).expect("first init");
        // 覆盖 server.yaml 加 web.port，删 Caddyfile 后重 init（PIT-160 语义：缺失才写）
        let cfg = dir.join("etc").join("server.yaml");
        let text = std::fs::read_to_string(&cfg).expect("read");
        std::fs::write(&cfg, format!("{text}\nweb:\n  port: 9080\n")).expect("write");
        std::fs::remove_file(dir.join("etc").join("Caddyfile")).expect("rm caddyfile");
        init_instance(dir).expect("re-init");
        let cf = std::fs::read_to_string(dir.join("etc").join("Caddyfile")).expect("read cf");
        assert!(cf.contains(":9080 {"), "yaml web.port 必须渲染进 Caddyfile 站点地址");
        assert!(!cf.contains(":8080"), "缺省口不得残留在站点地址");
    }

    #[test]
    fn init_weaknet_yaml_port_flows_to_oxfile_and_caddyfile() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path();
        init_instance(dir).expect("first init");
        let cfg = dir.join("etc").join("server.yaml");
        let text = std::fs::read_to_string(&cfg).expect("read");
        std::fs::write(&cfg, format!("{text}\nsfu:\n  weaknet_port: 9811\n")).expect("write");
        // 重渲 oxfile（write_oxfile 单点——删掉让 init 重建）
        std::fs::remove_file(dir.join("run").join("oxfile.toml")).expect("rm oxfile");
        // oxfile 条件渲染需要 bin/weaknet 在场；无 bin 时只验 Caddyfile 侧
        std::fs::remove_file(dir.join("etc").join("Caddyfile")).expect("rm cf");
        init_instance(dir).expect("re-init");
        let cf = std::fs::read_to_string(dir.join("etc").join("Caddyfile")).expect("read cf");
        assert!(cf.contains("reverse_proxy 127.0.0.1:9811"), "weaknet_port 渲染进 Caddyfile 上游");
        // oxfile：手工造 bin 后直接调 write_oxfile 验证两产物同源
        std::fs::create_dir_all(dir.join("bin")).expect("mkdir bin");
        std::fs::write(dir.join("bin").join("mediaservo-weaknet"), b"#!/bin/sh\n")
            .expect("write bin");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                dir.join("bin").join("mediaservo-weaknet"),
                std::fs::Permissions::from_mode(0o755),
            )
            .expect("chmod");
        }
        write_oxfile(&dir.join("run")).expect("write oxfile");
        let ox = std::fs::read_to_string(dir.join("run").join("oxfile.toml")).expect("read ox");
        assert!(
            ox.contains("serve --listen 127.0.0.1:9811"),
            "weaknet_port 渲染进 oxfile --listen（两产物同源）"
        );
    }

    // ---- T4（PIT-160 根修）：write_oxfile 合并源 live→.bak + 提示行分叉 ----

    #[test]
    fn t4c_manual_env_survives_via_bak_when_live_deleted() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path();
        init_instance(dir).expect("first init");
        let ox = dir.join("run").join("oxfile.toml");
        let bak = dir.join("run").join("oxfile.toml.bak");
        let base = std::fs::read_to_string(&bak)
            .expect("write_oxfile 必须同产 .bak 快照（PIT-160 重渲源）");
        // 手工键注入 .bak（= Python deploy stale 备份后的形）；server 条目 env 表有/无形两路
        let tampered = if base.contains("[apps.env]") {
            base.replacen("[apps.env]", "[apps.env]\nALLOW_X = \"1\"", 1)
        } else {
            base.replacen("[apps.logs]", "[apps.env]\nALLOW_X = \"1\"\n[apps.logs]", 1)
        };
        assert_ne!(tampered, base, "fixture 注入失败");
        std::fs::write(&bak, &tampered).expect("tamper bak");
        std::fs::remove_file(&ox).expect("rm live oxfile");
        write_oxfile(&dir.join("run")).expect("re-render（live 缺 → bak 合并源）");
        let after = std::fs::read_to_string(&ox).expect("read ox");
        assert!(after.contains("ALLOW_X = \"1\""), ".bak 手工 env 必须合并回:\n{after}");
        assert_eq!(
            std::fs::read_to_string(&bak).expect("read bak"),
            after,
            ".bak 应随写终态推进（多轮 N 不翻倍）"
        );
        // live 与 bak 双缺 → 纯渲染（无任何手工键）
        std::fs::remove_file(&ox).expect("rm oxfile 2");
        std::fs::remove_file(&bak).expect("rm bak");
        write_oxfile(&dir.join("run")).expect("re-render（首渲形）");
        let pure = std::fs::read_to_string(&ox).expect("read ox 2");
        assert!(!pure.contains("ALLOW_X"), "live 与 bak 双缺 → 纯渲染");
    }

    // ---- global-reap T2: B3 env 清洗 + B2 归因三态 + 扫尾计划 fixture ----

    #[test]
    fn run_oxmgr_default_strips_oxmgr_env() {
        // B3：默认 daemon 语义 = 显式无 OXMGR_*。std Command 继承父 env——env_remove
        // 后 get_envs 必含 (key, None) 覆盖删除项（父 shell 残留也不得漏进子进程）。
        // （不 set_var 造父 env：edition 2024 unsafe + 并行测试 env 竞争——删除语义
        // 对"继承值"同样生效，覆盖项即证明。）
        let keys = ["OXMGR_HOME", "OXMGR_DATA_DIR", "OXMGR_DAEMON_ADDR", "OXMGR_API_ADDR"];
        let mut cmd = Command::new(oxmgr_bin());
        for k in keys {
            cmd.env_remove(k);
        }
        for k in keys {
            assert!(
                cmd.get_envs().any(|(key, v)| key == std::ffi::OsStr::new(k) && v.is_none()),
                "{k} 必须呈显式删除态"
            );
        }
    }

    #[test]
    fn port_verdict_three_states() {
        use super::conflict::ConflictReport;
        let rep = |pid: u32| ConflictReport {
            pid,
            exe: "/x/bin/msrtc-server".into(),
            cmd: "msrtc-server run".into(),
            deleted: false,
            parent: None,
        };
        // ① 空闲（无监听者）→ pass
        assert!(port_verdict(&[], &[1, 2]).is_ok());
        // ② 我方残留（pid ∈ 本轮 reap 名单）→ fail
        let ours = vec![rep(101), rep(102)];
        assert!(port_verdict(&ours, &[101]).is_err());
        // ③ 陌生者占口 → pass（B2：TalkServo 实录形不假失败）
        assert!(port_verdict(&ours, &[999]).is_ok());
        // 边界：两侧全空 → pass
        assert!(port_verdict(&[], &[]).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn sweep_plan_fixture_direct_kill_when_no_registry() {
        use super::reap::{LiveProc, ProcKind, ReapStep};
        // fixture：tempdir 实例 root + 伪造 bin/msrtc-server 标记（marker 判据面）
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = absolute(tmp.path());
        std::fs::create_dir_all(root.join("bin")).expect("mkdir bin");
        std::fs::write(root.join("bin").join("msrtc-server"), b"").expect("marker");
        // LiveProc 指向它：server 签名 + --config 归属本 root；注册表空 → DirectKill 计划
        let p = LiveProc {
            argv: vec![
                format!("{}/bin/msrtc-server", root.display()),
                "run".into(),
                "--config".into(),
                format!("{}/etc/server.yaml", root.display()),
            ],
            exe: Some(format!("{}/bin/msrtc-server", root.display())),
            cwd: Some(root.clone()),
            environ_oxmgr_home: None,
            pid: 424242,
        };
        assert_eq!(
            super::reap::proc_kind(&p.argv, p.exe.as_deref()),
            Some(ProcKind::Server),
            "fixture 必须过签名"
        );
        assert!(super::reap::kind_dir_within(&ProcKind::Server, &p.argv, &root));
        // 空注册表（双表快照）→ 直杀计划（断言计划，不执行——单元测试零真实信号）
        let plan = super::reap::reap_plan(&[p], &[(root.join("run").join("oxmgr"), &[])]);
        assert_eq!(
            plan,
            vec![ReapStep::DirectKill {
                pid: 424242,
                reason: super::reap::DirectReason::NoRegistry
            }]
        );
    }

    // ---- global-reap T3: stop 参数解析三形 ----

    #[test]
    fn stop_args_global_alone_and_with_dir_conflict() {
        let mut a = ["-g".to_string()].into_iter();
        assert_eq!(parse_stop_args(&mut a), Ok(StopArgs::Global));
        let mut b = ["--all".to_string()].into_iter();
        assert_eq!(parse_stop_args(&mut b), Ok(StopArgs::Global));
        // -g + dir = 非法（exit 2 由 cmd_stop 映射）
        let mut c = ["-g".to_string(), "/opt/x".to_string()].into_iter();
        assert_eq!(parse_stop_args(&mut c), Err("stop -g 为全局收口，不接受目录参数".to_string()));
        // dir 在前 -g 在后同拒（顺序无关）
        let mut d = ["/opt/x".to_string(), "--all".to_string()].into_iter();
        assert!(parse_stop_args(&mut d).is_err());
        // 未知旗标拒
        let mut e = ["--dir".to_string()].into_iter();
        assert!(parse_stop_args(&mut e).is_err());
    }

    #[test]
    fn stop_args_dir_forms_unchanged() {
        // 裸 stop = 默认目录（既有行为）；显式 dir；dir 与未知 -- 开头参数同拒。
        let mut a: Box<dyn Iterator<Item = String>> = Box::new(std::iter::empty());
        assert_eq!(parse_stop_args(&mut a), Ok(StopArgs::Dir(None)));
        let mut b = ["/opt/inst".to_string()].into_iter();
        assert_eq!(parse_stop_args(&mut b), Ok(StopArgs::Dir(Some("/opt/inst".into()))));
        let mut c = ["/a".to_string(), "/b".to_string()].into_iter();
        assert!(parse_stop_args(&mut c).is_err());
    }

    #[test]
    fn t4_pit160_note_variants() {
        assert_eq!(
            pit160_note(0, 0),
            "提示: 若本树此前存在手工 [apps.env] 行（ALLOW_DEV_*/RUST_LOG/WEAKNET_ADMIN_PASS），本次渲染不含它们——按 msrtc.sh -h「环境变量 [B]」回补",
            "N=0 必须逐字节保留旧指引文本"
        );
        let v = pit160_note(3, 0);
        assert!(v.contains("已合并 3 条手工 env"), "N>0 变体: {v}");
        let l = pit160_note(2, 1);
        assert!(l.contains("另有 1 条"), "未并条目必须报数: {l}");
    }
}
