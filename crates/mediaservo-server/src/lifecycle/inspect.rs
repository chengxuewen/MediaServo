//! 检视面（T18）：status（oxmgr 行 + /ready 探针列 + 退出码契约）、doctor（PATH/yaml/dist/
//! announced，退出码=失败数）、logs（oxmgr logs 转发）+ 端口/进程竞争探测辅助。

use std::ffi::OsStr;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use super::templates::{
    effective_announced, parse_listen_port, parse_web_port, parse_web_root, server_namespace,
    server_product, server_web_app,
};
use super::{absolute, oxmgr_bin, oxmgr_list, parse_dir, run_oxmgr, which};

// ── status / doctor / logs ───────────────────────────────────────────────────

/// status 退出码映射（纯函数单测锚点）：
/// server 行缺失/未跑 → 2；探针非健康 → 1；web 在簇但未跑 → 1；全绿 → 0。
/// web=None = "not in cluster"（--no-web 形态），不计异常（design dev 闭环）。
pub(super) fn map_status_exit(
    server_status: Option<&str>,
    web_status: Option<Option<&str>>,
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
    if let Some(Some(w)) = web_status && !is_up(w) {
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
    if !dir.join("etc").join("server.yaml").exists() && !dir.join("run").join("oxfile.toml").exists() {
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
    let server = rows.iter().find(|p| p.get("name").and_then(|v| v.as_str()) == Some(srv_name.as_str()));
    let web = rows.iter().find(|p| p.get("name").and_then(|v| v.as_str()) == Some(web_name.as_str()));
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
        row.and_then(|p| p.get("status").and_then(|s| s.as_str()))
            .unwrap_or("?")
            .to_string()
    };
    let pid_of = |row: Option<&serde_json::Value>| -> String {
        row.and_then(|p| p.get("pid").and_then(|v| v.as_u64()))
            .map_or_else(|| "-".to_string(), |n| n.to_string())
    };
    let server_status = status_of(server);
    let web_status = web.map(|_| status_of(web));
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
    map_status_exit(
        server.map(|_| server_status.as_str()),
        web_status.as_deref().map(Some),
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
            if let Some(d) = parse_server_cmdline(&cmdline) && absolute(&d) != mine {
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
    let ours = [server_product(), server_web_app()];
    let ns = server_namespace();
    let rows = oxmgr_list(dir)?;
    Ok(rows
        .iter()
        .filter(|p| {
            p.get("namespace").and_then(|n| n.as_str()) == Some(ns.as_str())
                || p.get("name").and_then(|n| n.as_str()).is_some_and(|n| ours.iter().any(|o| o == n))
        })
        .filter_map(|p| p.get("name").and_then(|n| n.as_str()).map(String::from))
        .collect())
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
    check(which("caddy").is_some(), "caddy 在 PATH", "裸机需安装 caddy（dev --no-web 形态可忽略本项）");

    let cfg_path = dir.join("etc").join("server.yaml");
    let cfg = std::fs::read_to_string(&cfg_path).ok();
    check(
        cfg.as_deref().is_some_and(|c| parse_listen_port(c).is_some()),
        "server.yaml 可解析",
        "缺失或 listen.port 解析失败——先 init",
    );
    let parsed = cfg_path.exists().then(|| mediaservo_server::config::load(&cfg_path).ok()).flatten();
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

    let announced_in_yaml = parsed.is_some_and(|c| !c.sfu.announced_ips.is_empty());
    check(
        effective_announced(announced_in_yaml),
        "announced IP 已配置",
        "env MEDIASERVO_SFU_ANNOUNCED_IP 与 sfu.announced_ips 均缺省——多网卡/容器环境黑帧高危（C38 层①）",
    );

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

/// `logs [server|web|all] [<dir>] [-f|--lines N]`：oxmgr logs 转发（app 名映射品牌）。
pub(super) fn cmd_logs(args: &mut dyn Iterator<Item = String>) -> i32 {
    let mut target: Option<String> = None;
    let mut dir_token: Option<String> = None;
    let mut flags: Vec<String> = Vec::new();
    while let Some(a) = args.next() {
        match a.as_str() {
            "server" if target.is_none() && dir_token.is_none() => target = Some(server_product()),
            "web" if target.is_none() && dir_token.is_none() => target = Some(server_web_app()),
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
        assert_eq!(map_status_exit(Some("running"), Some(Some("running")), Some(true)), 0);
        // --no-web: web 不在簇，不计异常
        assert_eq!(map_status_exit(Some("running"), None, Some(true)), 0);
        // web 在簇但停了 = 1（降级）
        assert_eq!(map_status_exit(Some("running"), Some(Some("stopped")), Some(true)), 1);
        // 探针失败（worker 死/503）= 1
        assert_eq!(map_status_exit(Some("running"), None, Some(false)), 1);
        assert_eq!(map_status_exit(Some("running"), Some(Some("running")), None), 1);
        // server 未跑/缺行 = 2
        assert_eq!(map_status_exit(Some("stopped"), Some(Some("stopped")), None), 2);
        assert_eq!(map_status_exit(None, None, None), 2);
    }


    #[test]
    fn server_cmdline_probe_brand_compatible() {
        let dir =
            parse_server_cmdline("/opt/ms/bin/mediaservo-server run --config /opt/ms/etc/server.yaml");
        assert_eq!(dir.as_deref(), Some(Path::new("/opt/ms")));
        let branded =
            parse_server_cmdline("/opt/ms/bin/msrtc-server run --config /opt/ms/etc/server.yaml");
        assert_eq!(branded.as_deref(), Some(Path::new("/opt/ms")));
        assert!(parse_server_cmdline("caddy run --config /opt/ms/etc/Caddyfile").is_none());
        assert!(parse_server_cmdline("/opt/x/bin/msrtc-host run --config /opt/x/etc/host.yaml").is_none());
        assert!(parse_server_cmdline("/opt/x/bin/msrtc-server --config /opt/x/server.yaml").is_none());
    }
}
