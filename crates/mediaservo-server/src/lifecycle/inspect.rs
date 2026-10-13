//! 检视面（T18）：status（oxmgr 行 + /ready 探针列 + 退出码契约）、doctor（PATH/yaml/dist/
//! announced，退出码=失败数）、logs（oxmgr logs 转发）+ 端口/进程竞争探测辅助。

use std::ffi::OsStr;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use super::templates::{
    DEFAULT_WEB_PORT, WEAKNET_APP, effective_announced, parse_listen_port, parse_web_port,
    parse_web_root, resolve_port, server_namespace, server_product, server_web_app,
};
use super::{absolute, oxmgr_bin, oxmgr_list, parse_dir, run_oxmgr, which};

// ── status / doctor / logs ───────────────────────────────────────────────────

/// status 退出码映射（纯函数单测锚点）：
/// server 行缺失/未跑 → 2；探针非健康 → 1；web/weaknet 在簇但未跑 → 1；全绿 → 0。
/// web=None / weaknet=None = "not in cluster"（--no-web 形态 / bin 缺位条件渲染形态 [M-1]），
/// 不计异常（design dev 闭环）。BA-6 四点同步①（②status 行 ③logs 映射 ④apply 文案）。
pub(super) fn map_status_exit(
    server_status: Option<&str>,
    web_status: Option<Option<&str>>,
    weaknet_status: Option<Option<&str>>,
    probe: Option<bool>,
) -> i32 {
    let Some(s) = server_status else {
        return 2;
    };
    if !is_up(s) {
        return 2;
    }
    if probe != Some(true) {
        return 1;
    }
    if let Some(Some(w)) = web_status
        && !is_up(w)
    {
        return 1;
    }
    if let Some(Some(w)) = weaknet_status
        && !is_up(w)
    {
        return 1;
    }
    0
}

fn is_up(status: &str) -> bool {
    matches!(status, "running" | "online")
}

pub(super) fn cmd_status(args: &mut dyn Iterator<Item = String>) -> i32 {
    let dir = match parse_dir(args) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("{e}");
            return 2;
        }
    };
    if !dir.join("etc").join("server.yaml").exists()
        && !dir.join("run").join("oxfile.toml").exists()
    {
        eprintln!("status: {} 非实例目录（无 etc/server.yaml 与 run/oxfile.toml）", dir.display());
        return 2;
    }
    let rows = match oxmgr_list(&dir) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("status: {e}");
            return 2;
        }
    };
    let srv_name = server_product();
    let web_name = server_web_app();
    let wnet_name = WEAKNET_APP;
    let server =
        rows.iter().find(|p| p.get("name").and_then(|v| v.as_str()) == Some(srv_name.as_str()));
    let web =
        rows.iter().find(|p| p.get("name").and_then(|v| v.as_str()) == Some(web_name.as_str()));
    let wnet = rows.iter().find(|p| p.get("name").and_then(|v| v.as_str()) == Some(wnet_name));
    let port = std::fs::read_to_string(dir.join("etc").join("server.yaml"))
        .ok()
        .and_then(|c| parse_listen_port(&c));
    let probe = port.map(probe_ready);
    let probe_txt = match probe {
        Some(true) => "ok",
        Some(false) => "UNHEALTHY",
        None => "unreachable",
    };
    let status_of = |row: Option<&serde_json::Value>| -> String {
        row.and_then(|p| p.get("status").and_then(|s| s.as_str())).unwrap_or("?").to_string()
    };
    let pid_of = |row: Option<&serde_json::Value>| -> String {
        row.and_then(|p| p.get("pid").and_then(|v| v.as_u64()))
            .map_or_else(|| "-".to_string(), |n| n.to_string())
    };
    let server_status = status_of(server);
    let web_status = web.map(|_| status_of(web));
    let weaknet_status = wnet.map(|_| status_of(wnet));
    println!("{:<24} {:<22} {:<9} READY", "NAME", "STATUS", "PID");
    println!(
        "{:<24} {:<22} {:<9} {probe_txt}",
        server_product(),
        if server.is_some() { server_status.as_str() } else { "— (not in cluster)" },
        pid_of(server)
    );
    println!(
        "{:<24} {:<22} {:<9} —",
        server_web_app(),
        web_status.as_deref().unwrap_or("— (not in cluster)"),
        pid_of(web)
    );
    println!(
        "{:<24} {:<22} {:<9} —",
        wnet_name,
        weaknet_status.as_deref().unwrap_or("— (not in cluster)"),
        pid_of(wnet)
    );
    map_status_exit(
        server.map(|_| server_status.as_str()),
        web_status.as_deref().map(Some),
        weaknet_status.as_deref().map(Some),
        probe,
    )
}

/// /ready 一次性探测（裸 TCP HTTP/1.1——不引入 http client 依赖，ponytail: 探针只需状态码）。
fn probe_ready(port: u16) -> bool {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let Ok(mut s) = std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(500)) else {
        return false;
    };
    let _ = s.set_read_timeout(Some(Duration::from_millis(1500)));
    let _ = s.set_write_timeout(Some(Duration::from_millis(500)));
    if s.write_all(b"GET /ready HTTP/1.1\r\nHost: probe\r\nConnection: close\r\n\r\n").is_err() {
        return false;
    }
    let mut buf = [0u8; 16];
    match s.read(&mut buf) {
        Ok(n) if n >= 12 => String::from_utf8_lossy(&buf[..n]).get(9..12) == Some("200"),
        _ => false,
    }
}

pub(super) fn port_in_use(port: u16) -> bool {
    std::net::TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
        Duration::from_millis(300),
    )
    .is_ok()
}

/// `/proc` 扫描其他 server 实例目录（品牌兼容：exe 名以 "-server" 结尾——PIT-155 同训）。
pub(super) fn find_other_server_dir(my_dir: &Path) -> Option<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        let mine = absolute(my_dir);
        let entries = std::fs::read_dir("/proc").ok()?;
        for e in entries.flatten() {
            let Ok(pid) = e.file_name().to_string_lossy().parse::<u32>() else {
                continue;
            };
            let Ok(cmdline) = std::fs::read_to_string(format!("/proc/{pid}/cmdline")) else {
                continue;
            };
            let cmdline = cmdline.replace('\0', " ");
            if let Some(d) = parse_server_cmdline(&cmdline)
                && absolute(&d) != mine
            {
                return Some(d);
            }
        }
        None
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = my_dir;
        None
    }
}

/// cmdline → 实例目录（exe 以 -server 结尾 且含 `--config <x>/etc/server.yaml`）。纯函数可单测。
pub fn parse_server_cmdline(cmdline: &str) -> Option<PathBuf> {
    let mut parts = cmdline.split_whitespace();
    let exe = parts.next()?;
    let fname = Path::new(exe).file_name()?.to_str()?;
    if !fname.ends_with("-server") {
        return None;
    }
    while let Some(p) = parts.next() {
        if p != "--config" {
            continue;
        }
        let path = parts.next()?;
        let cfg = Path::new(path);
        if cfg.file_name() == Some(OsStr::new("server.yaml"))
            && cfg.parent().is_some_and(|d| d.file_name() == Some(OsStr::new("etc")))
        {
            return cfg.parent()?.parent().map(Path::to_path_buf);
        }
    }
    None
}

/// 本实例 daemon 中属于本簇的 app 名（namespace 或期望名匹配）。
pub(super) fn list_registered_apps(dir: &Path) -> Result<Vec<String>, String> {
    // bF7：三条目——weaknet-serve 品牌无关字面量 [mn10]，否则 status/doctor 判 alien、
    // stop 漏收敛（面板簇残留进程停不掉）。
    let ours = [server_product(), server_web_app(), WEAKNET_APP.to_string()];
    let ns = server_namespace();
    let rows = oxmgr_list(dir)?;
    Ok(rows
        .iter()
        .filter(|p| {
            p.get("namespace").and_then(|n| n.as_str()) == Some(ns.as_str())
                || p.get("name")
                    .and_then(|n| n.as_str())
                    .is_some_and(|n| ours.iter().any(|o| o == n))
        })
        .filter_map(|p| p.get("name").and_then(|n| n.as_str()).map(String::from))
        .collect())
}

/// web 端口漂移判据（T6b①，D-A 改判后漂移的唯一报警通道=doctor）。intent = yaml > env >
/// DEFAULT_WEB_PORT（与 init 渲染 resolved_ports 同链）；产物 = Caddyfile 反解（start 探活真值源）。
/// 不一致 → Some((caddy, intent))。契约 §2.6：Caddyfile 缺席/parse 失败（古董/手破）→ None = skip 不告警。
pub(super) fn web_port_drift(
    cfg_web_port: Option<u16>,
    env_web: Option<u16>,
    caddyfile: Option<&str>,
) -> Option<(u16, u16)> {
    let caddy = caddyfile.and_then(parse_web_port)?;
    let intent = resolve_port(cfg_web_port, env_web, DEFAULT_WEB_PORT);
    (caddy != intent).then_some((caddy, intent))
}

/// announced_ips 腐烂判据（T6b④，纯函数：吃 yaml ips + 本机接口集，IO 留调用方）。
/// 裸机/容器判定复用 use_bare_metal_listen（sfu.rs 同源，first-only）：
/// false → 容器姿态 → None（INFO 豁免）；true → 裸机 → Some(腐烂项)，全健康 = Some(空)。
/// ponytail: v1 盲区（PLAN §2.6 显式接受）——多 IP 裸机且首项已腐烂时 first-only 误判
/// "容器"整体豁免；闭合判据 = announced ∩ 本机 = ∅ 时全检，偏离复用条款需用户裁。
pub(super) fn announced_stale_items(
    announced: &[String],
    local: &std::collections::HashSet<String>,
) -> Option<Vec<String>> {
    if !mediaservo_server::sfu::use_bare_metal_listen(announced, local) {
        return None;
    }
    Some(announced.iter().filter(|ip| !local.contains(*ip)).cloned().collect())
}

// ── T5 oxmgr 双 daemon 注册表分裂检测（指认不杀戮——PLAN §2.5-3，PIT-98）──────────

/// 收口指引句（doctor/contention 两处同源——同一句话只写一次）
pub(super) const SPLIT_GUIDANCE: &str = "簇操作先固定一个 OXMGR_DATA_DIR 并 `oxmgr list` 对账；收口方式：经共享 daemon 停簇或迁移注册后重启（本检测不自动杀戮/迁移）";

/// oxmgr `state.json` processes[] 条目的最小关注面（未知字段忽略；pid 在 stopped 形为 null）。
#[derive(Debug, Clone, serde::Deserialize)]
pub(super) struct ManagedEntry {
    #[serde(default)]
    pub id: u64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub pid: Option<u64>,
    #[serde(default)]
    pub cwd: PathBuf,
}

#[derive(serde::Deserialize)]
struct OxmgrState {
    #[serde(default)]
    processes: Vec<ManagedEntry>,
}

/// 共享/旧 daemon 数据目录：$XDG_DATA_HOME（非空）否则 $HOME/.local/share，接 "oxmgr"。
/// 计划原文 `~/.oxmgr` 系误引——实机 oxmgr 注册表落 XDG（2026-10-12 事故复盘亲验）。
pub(super) fn default_data_dir() -> Option<PathBuf> {
    default_data_dir_from(
        std::env::var("XDG_DATA_HOME").ok().as_deref(),
        std::env::var("HOME").ok().as_deref(),
    )
}

fn default_data_dir_from(xdg: Option<&str>, home: Option<&str>) -> Option<PathBuf> {
    // ponytail: Windows 无 HOME/XDG 语义 → None（检测整体静默）；此生命周期面是
    // Linux/macOS 生产，读注册不为 Windows 引入 dirs 依赖。
    let base = xdg.filter(|s| !s.is_empty()).map(PathBuf::from).or_else(|| {
        home.filter(|s| !s.is_empty()).map(|h| PathBuf::from(h).join(".local/share"))
    })?;
    Some(base.join("oxmgr"))
}

/// 宽容解析：坏 json / 非 OxMgrState 形 → 空表（消费侧视同"无分裂"，不产噪音）。
pub(super) fn parse_state(text: &str) -> Vec<ManagedEntry> {
    serde_json::from_str::<OxmgrState>(text).map(|s| s.processes).unwrap_or_default()
}

pub(super) fn read_registry(path: &Path) -> Vec<ManagedEntry> {
    std::fs::read_to_string(path).map(|t| parse_state(&t)).unwrap_or_default()
}

/// 共享 daemon 注册表（default_data_dir 缺位 / state.json 缺失 = 空表）。
pub(super) fn read_default_registry() -> Vec<ManagedEntry> {
    default_data_dir().map(|d| read_registry(&d.join("state.json"))).unwrap_or_default()
}

/// entries 中 cwd 位于 my_dir 之下的条目（双侧绝对化后按 Path 组件前缀——
/// /dir/server-2 这类兄弟形不误入）。纯函数，doctor 与 contention 共用。
pub(super) fn strays_under(entries: &[ManagedEntry], my_dir: &Path) -> Vec<ManagedEntry> {
    let mine = absolute(my_dir);
    entries.iter().filter(|e| absolute(&e.cwd).starts_with(&mine)).cloned().collect()
}

#[derive(Debug)]
pub(super) struct SplitVerdict {
    /// 共享/旧 daemon 名下、cwd 属本实例的行
    pub strays: Vec<ManagedEntry>,
    /// 本实例注册表条目数（文案对照用）
    pub instance_count: usize,
}

/// 分裂判据：default 表含 cwd ∈ 我方目录的行即判。status 词汇无关——盘上仅观测到
/// "running"，而注册在错误 daemon 名下本身就是混乱源（实例 CLI stop 管不到它
/// = 2026-10-12"说 stopped 但端口仍被占"事故形）。**实例表为空不豁免**：事故恰是
/// 实例表空 + default 持全部，"两表皆非空"规则必漏。
pub(super) fn split_registry(
    default: &[ManagedEntry],
    instance: &[ManagedEntry],
    my_dir: &Path,
) -> Option<SplitVerdict> {
    let strays = strays_under(default, my_dir);
    (!strays.is_empty()).then_some(SplitVerdict { strays, instance_count: instance.len() })
}

pub(super) fn cmd_doctor(args: &mut dyn Iterator<Item = String>) -> i32 {
    let dir = match parse_dir(args) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("{e}");
            return 2;
        }
    };
    let mut failed = 0usize;
    let mut check = |ok: bool, name: &str, note: &str| {
        if ok {
            println!("[ok] {name}");
        } else {
            println!("[fail] {name}: {note}");
            failed += 1;
        }
    };

    check(
        Command::new(oxmgr_bin()).arg("--version").output().is_ok(),
        "oxmgr 可用",
        "不在 PATH（deploy 会锁定于 <prefix>/bin——export PATH 或重新 deploy）",
    );
    check(
        which("caddy").is_some(),
        "caddy 在 PATH",
        "裸机需安装 caddy（dev --no-web 形态可忽略本项）",
    );

    let cfg_path = dir.join("etc").join("server.yaml");
    let cfg = std::fs::read_to_string(&cfg_path).ok();
    check(
        cfg.as_deref().is_some_and(|c| parse_listen_port(c).is_some()),
        "server.yaml 可解析",
        "缺失或 listen.port 解析失败——先 init",
    );
    let parsed =
        cfg_path.exists().then(|| mediaservo_server::config::load(&cfg_path).ok()).flatten();
    check(parsed.is_some(), "server.yaml 全字段合法", "ServerConfig 反序列化失败（start 会拒绝）");
    for f in ["devices.yaml", "accounts.yaml"] {
        let p = dir.join("etc").join(f);
        let ok = std::fs::read_to_string(&p)
            .ok()
            .is_some_and(|t| serde_yaml::from_str::<serde_yaml::Value>(&t).is_ok());
        check(ok, &format!("{f} 可解析"), "缺失或 YAML 损坏");
    }

    // web dist（design：一体是交付非强制耦合——dev --no-web 形态此失败为预期，措辞已注）
    let cf = std::fs::read_to_string(dir.join("etc").join("Caddyfile")).ok();
    let dist_ok = cf.as_deref().and_then(parse_web_root).is_some_and(|r| {
        Path::new(&r).is_dir() && std::fs::read_dir(&r).is_ok_and(|mut d| d.next().is_some())
    });
    check(
        dist_ok,
        "web dist 非空",
        "Caddyfile root 目录缺失/为空（改 TS 后 build web；dev --no-web 形态可忽略本项）",
    );

    let announced_in_yaml = parsed.as_ref().is_some_and(|c| !c.sfu.announced_ips.is_empty());
    check(
        effective_announced(announced_in_yaml),
        "announced IP 已配置",
        "env MEDIASERVO_SFU_ANNOUNCED_IP 与 sfu.announced_ips 均缺省——多网卡/容器环境黑帧高危（C38 层①）",
    );

    // T6b①④ 漂移/腐烂巡检——[warn]/[info] 行不计入 failed（doctor 退出码=失败数契约不变）
    if let Some(c) = parsed.as_ref() {
        let env_web = std::env::var("MEDIASERVO_WEB_PORT").ok().and_then(|v| v.parse::<u16>().ok());
        if let Some((caddy, intent)) = web_port_drift(c.web.port, env_web, cf.as_deref()) {
            println!(
                "[warn] Caddyfile(:{caddy}) 与 server.yaml/env(:{intent}) 漂移——start 探活以 Caddyfile 为准；欲 yaml 生效删 etc/Caddyfile 重 init"
            );
        }
        // 腐烂检查仅在 yaml 值生效时跑（env 覆写态下 yaml=死配置，WARN 指向非生效值=误导）
        if announced_in_yaml && std::env::var("MEDIASERVO_SFU_ANNOUNCED_IP").is_err() {
            let local: std::collections::HashSet<String> =
                mediaservo_server::sfu::detect_all_ips().into_iter().collect();
            match announced_stale_items(&c.sfu.announced_ips, &local) {
                None => println!(
                    "[info] announced 腐烂检查豁免：首项 ∉ 本机接口 = 容器姿态（first-only 判据，多 IP 裸机首项腐烂属已接受盲区）"
                ),
                Some(stale) if stale.is_empty() => println!(
                    "[ok] announced_ips 全部仍属本机接口（{} 项）",
                    c.sfu.announced_ips.len()
                ),
                Some(stale) => println!(
                    "[warn] announced_ips 腐烂: {stale:?} 已非本机接口 IP——两出口：裸机=清空 sfu.announced_ips 走自动探测 / 容器=更新为当前可达 IP（公告腐烂 IP=客户端 ICE 全败，C38 层①）"
                ),
            }
        }
    }

    // T5 双 daemon 注册表分裂巡检——[warn] 不计 failed（T6b 先例，退出码=失败数契约不变）
    let default_reg = read_default_registry();
    if !default_reg.is_empty() {
        let instance_reg = read_registry(&dir.join("run").join("oxmgr").join("state.json"));
        if let Some(v) = split_registry(&default_reg, &instance_reg, &dir) {
            let items: Vec<String> = v
                .strays
                .iter()
                .map(|e| {
                    format!(
                        "{}(id={} status={} pid={})",
                        e.name,
                        e.id,
                        e.status,
                        e.pid.map_or_else(|| "?".into(), |p| p.to_string())
                    )
                })
                .collect();
            println!(
                "[warn] oxmgr 双 daemon 分裂: {} 个本实例 app 注册在共享/旧 daemon 名下: {}（本实例注册表 {n} 条）—— {g}",
                v.strays.len(),
                items.join(", "),
                n = v.instance_count,
                g = SPLIT_GUIDANCE
            );
        }
    }

    if let Some(p) = cfg.as_deref().and_then(parse_listen_port) {
        println!(
            "[info] 后端端口 {p}: {}",
            if port_in_use(p) { "占用（本实例在跑或他人）" } else { "空闲" }
        );
    }
    if let Some(wp) = cf.as_deref().and_then(parse_web_port) {
        println!("[info] web 端口 {wp}: {}", if port_in_use(wp) { "占用" } else { "空闲" });
    }
    if failed == 0 {
        println!("doctor: 全部通过（{}）", dir.display());
    }
    i32::try_from(failed).unwrap_or(125).min(125)
}

/// `logs [server|web|weaknet|all] [<dir>] [-f|--lines N]`：oxmgr logs 转发（app 名映射品牌；
/// weaknet-serve 品牌无关 [mn10]，logs 锚点=run/logs/weaknet-serve.*.log）。
pub(super) fn cmd_logs(args: &mut dyn Iterator<Item = String>) -> i32 {
    let mut target: Option<String> = None;
    let mut dir_token: Option<String> = None;
    let mut flags: Vec<String> = Vec::new();
    while let Some(a) = args.next() {
        match a.as_str() {
            "server" if target.is_none() && dir_token.is_none() => target = Some(server_product()),
            "web" if target.is_none() && dir_token.is_none() => target = Some(server_web_app()),
            "weaknet" if target.is_none() && dir_token.is_none() => {
                target = Some(WEAKNET_APP.to_string())
            }
            "all" if target.is_none() && dir_token.is_none() => target = Some("all".to_string()),
            "--lines" => {
                flags.push(a.to_string());
                if let Some(v) = args.next() {
                    flags.push(v);
                }
            }
            s if s.starts_with('-') => flags.push(s.to_string()),
            s if dir_token.is_none() => dir_token = Some(s.to_string()),
            s => {
                eprintln!("logs: 多余参数: {s}");
                return 2;
            }
        }
    }
    let mut iter = dir_token.into_iter();
    let dir = match parse_dir(&mut iter) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("{e}");
            return 2;
        }
    };
    let mut oxargs: Vec<String> = vec!["logs".into(), target.unwrap_or_else(|| "all".into())];
    oxargs.extend(flags);
    let refs: Vec<&str> = oxargs.iter().map(String::as_str).collect();
    run_oxmgr(Some(&dir), &refs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_exit_contract() {
        // 全绿 = 0
        assert_eq!(
            map_status_exit(
                Some("running"),
                Some(Some("running")),
                Some(Some("running")),
                Some(true)
            ),
            0
        );
        // --no-web: web 不在簇，不计异常；weaknet None 同理（bin 缺位条件渲染形 [M-1]）
        assert_eq!(map_status_exit(Some("running"), None, None, Some(true)), 0);
        // web 在簇但停了 = 1（降级）
        assert_eq!(map_status_exit(Some("running"), Some(Some("stopped")), None, Some(true)), 1);
        // weaknet 在簇但停了 = 1（BA-6 新案：与 web 同语义，None=不在簇不计）
        assert_eq!(
            map_status_exit(
                Some("running"),
                Some(Some("running")),
                Some(Some("stopped")),
                Some(true)
            ),
            1
        );
        // web 剔 while weaknet 活（--no-web 正确形态 lck-F8）= 0：web=None 不连坐
        assert_eq!(map_status_exit(Some("running"), None, Some(Some("running")), Some(true)), 0);
        // 探针失败（worker 死/503）= 1
        assert_eq!(map_status_exit(Some("running"), None, None, Some(false)), 1);
        assert_eq!(
            map_status_exit(Some("running"), Some(Some("running")), Some(Some("running")), None),
            1
        );
        // server 未跑/缺行 = 2
        assert_eq!(map_status_exit(Some("stopped"), Some(Some("stopped")), None, None), 2);
        assert_eq!(map_status_exit(None, None, None, None), 2);
    }

    #[test]
    fn server_cmdline_probe_brand_compatible() {
        let dir = parse_server_cmdline(
            "/opt/ms/bin/mediaservo-server run --config /opt/ms/etc/server.yaml",
        );
        assert_eq!(dir.as_deref(), Some(Path::new("/opt/ms")));
        let branded =
            parse_server_cmdline("/opt/ms/bin/msrtc-server run --config /opt/ms/etc/server.yaml");
        assert_eq!(branded.as_deref(), Some(Path::new("/opt/ms")));
        assert!(parse_server_cmdline("caddy run --config /opt/ms/etc/Caddyfile").is_none());
        assert!(
            parse_server_cmdline("/opt/x/bin/msrtc-host run --config /opt/x/etc/host.yaml")
                .is_none()
        );
        assert!(
            parse_server_cmdline("/opt/x/bin/msrtc-server --config /opt/x/server.yaml").is_none()
        );
    }
    fn cf_text(port: u16) -> String {
        format!("state {{\n:{port} {{\n  root * /srv/web\n}}\n}}\n")
    }

    #[test]
    fn web_drift_match_mismatch_and_skip() {
        // 一致（yaml == Caddyfile）→ None 不漂移
        assert_eq!(web_port_drift(Some(8080), None, Some(&cf_text(8080))), None);
        // 缺省链一致：yaml/env 均无 = 8080 缺省，Caddyfile 8080 → None
        assert_eq!(web_port_drift(None, None, Some(&cf_text(8080))), None);
        // 漂移：yaml 8093 vs Caddyfile 8080 → Some((caddy, intent))
        assert_eq!(web_port_drift(Some(8093), None, Some(&cf_text(8080))), Some((8080, 8093)));
        // 意图链 yaml > env：env 8091 兜底（yaml None）
        assert_eq!(web_port_drift(None, Some(8091), Some(&cf_text(8080))), Some((8080, 8091)));
        // yaml 压制 env：yaml 8092 env 8091 vs Caddyfile 8092 → None
        assert_eq!(web_port_drift(Some(8092), Some(8091), Some(&cf_text(8092))), None);
        // 契约 §2.6：古董/手破 Caddyfile（parse None）→ skip 不 WARN
        assert_eq!(web_port_drift(Some(8093), None, Some("no site block here")), None);
        // Caddyfile 缺席（--no-web / 未 init）→ skip
        assert_eq!(web_port_drift(Some(8093), None, None), None);
    }

    fn ips(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn ipset(v: &[&str]) -> std::collections::HashSet<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn announced_staleness_verdicts() {
        // 裸机（首项 ∈ 本机）且有腐烂项 → Some(腐烂清单)
        assert_eq!(
            announced_stale_items(
                &ips(&["192.168.2.127", "203.0.113.9"]),
                &ipset(&["192.168.2.127"])
            ),
            Some(ips(&["203.0.113.9"]))
        );
        // 裸机健康：全属本机 → Some(空)（pass）
        assert_eq!(
            announced_stale_items(&ips(&["192.168.2.127"]), &ipset(&["192.168.2.127"])),
            Some(vec![])
        );
        // 容器姿态（首项 ∉ 本机 = 宿主可达 IP 形）→ None INFO 豁免，不 WARN
        assert_eq!(announced_stale_items(&ips(&["192.168.2.127"]), &ipset(&["172.18.0.2"])), None);
        // ponytail v1 盲区钉（first-only 判据，PLAN §2.6 豁免分支）：多 IP 裸机首项腐烂 → 误判容器豁免
        assert_eq!(
            announced_stale_items(
                &ips(&["203.0.113.9", "192.168.2.127"]),
                &ipset(&["192.168.2.127"])
            ),
            None
        );
        // 空配置 → None（use_bare_metal_listen([]) = false）
        assert_eq!(announced_stale_items(&[], &ipset(&["192.168.2.127"])), None);
    }

    // ── T5 双 daemon 注册表分裂检测（fixture = 静态 json 串，不触 daemon/磁盘）──
    fn entry(id: u64, name: &str, status: &str, cwd: &str) -> ManagedEntry {
        ManagedEntry {
            id,
            name: name.into(),
            status: status.into(),
            pid: if status == "running" { Some(id * 10) } else { None },
            cwd: cwd.into(),
        }
    }

    const INCIDENT_STATE: &str = r#"{"next_id":128,"processes":[
      {"id":124,"name":"msrtc-server","status":"running","pid":3419236,"cwd":"/deploy/out/server"},
      {"id":125,"name":"msrtc-web","status":"running","pid":3419240,"cwd":"/deploy/out/server"}]}"#;

    #[test]
    fn parse_state_fixture_shape() {
        let rows = parse_state(INCIDENT_STATE);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].name, "msrtc-server");
        assert_eq!(rows[0].pid, Some(3419236));
        // corrupt json / 非 oxmgr 形 → 空表（静默 pass，不产噪音）
        assert!(parse_state("{oops").is_empty());
        assert!(parse_state("{}").is_empty());
        // 未知字段忽略 + pid 缺省/null 容忍（stopped 形）
        let loose = parse_state(
            r#"{"processes":[{"id":1,"name":"x","status":"stopped","pid":null,"cwd":"/a","whatever":42}]}"#,
        );
        assert_eq!(loose.len(), 1);
        assert_eq!(loose[0].pid, None);
    }

    #[test]
    fn split_registry_catches_incident_shape_default_holds_ours_instance_empty() {
        // 2026-10-12 事故形：本实例 app 全在共享 daemon 名下、实例表空——
        // "两表皆非空才判"的规则会漏它。判据 = cwd 归属，与实例表内容无关。
        let default = parse_state(INCIDENT_STATE);
        let v =
            split_registry(&default, &[], Path::new("/deploy/out/server")).expect("事故形必须判出");
        assert_eq!(v.strays.len(), 2);
        assert_eq!(v.instance_count, 0);
    }

    #[test]
    fn split_registry_clean_single_registry_none() {
        // 正常态：本实例 app 在实例表、default 表无我方行
        let default = vec![entry(1, "other-app", "running", "/opt/otherproject")];
        let instance = vec![entry(2, "msrtc-server", "running", "/deploy/out/server")];
        assert!(split_registry(&default, &instance, Path::new("/deploy/out/server")).is_none());
        // default 表空 → None（无噪音）
        assert!(split_registry(&[], &instance, Path::new("/deploy/out/server")).is_none());
    }

    #[test]
    fn split_registry_foreign_prefix_lookalike_not_stray() {
        // Path 组件级前缀判据：/deploy/out/server-2 ∉ /deploy/out/server 之下
        let default = vec![entry(9, "sibling-instance", "running", "/deploy/out/server-2")];
        assert!(split_registry(&default, &[], Path::new("/deploy/out/server")).is_none());
    }

    #[test]
    fn split_registry_status_agnostic_pins_cwd_membership_rule() {
        // 规则钉：default 表含 cwd∈我方目录的行即判，status 词汇无关
        //（盘上仅见 "running"；stopped 残影同样是注册表归属混乱源）
        let default = vec![entry(3, "msrtc-server", "stopped", "/deploy/out/server")];
        let v = split_registry(&default, &[], Path::new("/deploy/out/server"))
            .expect("stopped 残影同判");
        assert_eq!(v.strays[0].status, "stopped");
    }

    #[test]
    fn strays_under_pure_match_for_contention() {
        // contention 侧判据：端口占用 + /proc 未命中 → default 表找我方目录行
        let default = parse_state(INCIDENT_STATE);
        let strays = strays_under(&default, Path::new("/deploy/out/server"));
        assert_eq!(strays[0].name, "msrtc-server");
        assert_eq!(strays[0].id, 124);
        assert_eq!(strays[0].pid, Some(3419236));
        assert!(strays_under(&default, Path::new("/somewhere/else")).is_empty());
    }

    #[test]
    fn default_data_dir_prefers_xdg_then_home() {
        // 纯参形钉判据（edition 2024 下 set_var 是 unsafe——不碰进程 env）
        assert_eq!(
            default_data_dir_from(Some("/x/data"), Some("/h")),
            Some(PathBuf::from("/x/data/oxmgr"))
        );
        assert_eq!(
            default_data_dir_from(None, Some("/h")),
            Some(PathBuf::from("/h/.local/share/oxmgr"))
        );
        assert_eq!(
            default_data_dir_from(Some(""), Some("/h")),
            Some(PathBuf::from("/h/.local/share/oxmgr"))
        );
        assert_eq!(default_data_dir_from(None, None), None);
    }
}
