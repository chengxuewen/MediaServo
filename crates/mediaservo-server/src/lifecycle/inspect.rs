//! Inspection face (T18): status (oxmgr rows + /ready probe column + exit-code contract), doctor (PATH/yaml/dist/
//! announced, exit code = failure count), logs (oxmgr logs passthrough) + port/process contention probe helpers.

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

/// status exit-code mapping (pure-function unit-test anchor):
/// server row missing/not running -> 2; probe unhealthy -> 1; web/weaknet in the cluster but not running -> 1; all green -> 0.
/// web=None / weaknet=None = "not in cluster" (the --no-web form / the bin-missing conditional-render form [M-1]),
/// not counted as anomalies (design dev loop). BA-6 four-point sync 1 (2 the status rows, 3 the logs mapping, 4 the apply text).
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
        eprintln!(
            "status: {} is not an instance dir (no etc/server.yaml and no run/oxfile.toml)",
            dir.display()
        );
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

/// One-shot /ready probe (raw TCP HTTP/1.1 -- no http client dependency; ponytail: the probe only needs the status code).
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

/// /proc scan for other server instance dirs (brand compat: exe names ending in "-server" -- same lesson as PIT-155).
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

/// cmdline -> instance dir (exe ends in -server AND contains `--config <x>/etc/server.yaml`). Pure function, unit-testable.
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

/// The app names in this instance daemon that belong to this cluster (namespace or expected-name match).
pub(super) fn list_registered_apps(dir: &Path) -> Result<Vec<String>, String> {
    // bF7: three entries -- weaknet-serve is a brand-independent literal [mn10], otherwise status/doctor judge it alien and
    // stop misses its convergence (panel-cluster leftover processes cannot be stopped).
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

/// web port drift predicate (T6b 1; after the D-A re-judgment, drift's only alarm channel = doctor). intent = yaml > env >
/// DEFAULT_WEB_PORT (same chain as init's rendered resolved_ports); product = Caddyfile reverse parse (the start probe's ground truth).
/// Mismatch -> Some((caddy, intent)). Contract S2.6: Caddyfile absent/parse failure (antique/hand-broken) -> None = skip, no alarm.
pub(super) fn web_port_drift(
    cfg_web_port: Option<u16>,
    env_web: Option<u16>,
    caddyfile: Option<&str>,
) -> Option<(u16, u16)> {
    let caddy = caddyfile.and_then(parse_web_port)?;
    let intent = resolve_port(cfg_web_port, env_web, DEFAULT_WEB_PORT);
    (caddy != intent).then_some((caddy, intent))
}

/// announced_ips rot predicate (T6b 4, pure: takes yaml ips + the local interface set; IO stays with the caller).
/// Bare-metal/container judgment reuses use_bare_metal_listen (same source as sfu.rs, first-only):
/// false -> container posture -> None (INFO exemption); true -> bare metal -> Some(rotted items); all healthy = Some(empty).
/// ponytail: v1 blind spot (explicitly accepted in PLAN S2.6) -- a multi-IP bare metal whose first item has rotted is misjudged
/// "container" and exempted wholesale; the closing criterion = full check when announced intersect local = empty; deviating from the reuse clause needs a user ruling.
pub(super) fn announced_stale_items(
    announced: &[String],
    local: &std::collections::HashSet<String>,
) -> Option<Vec<String>> {
    if !mediaservo_server::sfu::use_bare_metal_listen(announced, local) {
        return None;
    }
    Some(announced.iter().filter(|ip| !local.contains(*ip)).cloned().collect())
}

// -- T5 oxmgr dual-daemon registry split detection (attribute, do not kill -- PLAN S2.5-3, PIT-98) --

/// Convergence guidance sentence (same source in doctor/contention -- written once).
pub(super) const SPLIT_GUIDANCE: &str = "cluster operations must pin one OXMGR_DATA_DIR and reconcile with `oxmgr list`; convergence: stop the cluster via the shared daemon, or migrate the registration and restart (this detection never auto-kills/migrates)";

/// The minimal surface of interest of an oxmgr `state.json` processes[] entry (unknown fields ignored; pid is null in the stopped shape).
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

/// Shared/old daemon data dir: $XDG_DATA_HOME (non-empty) else $HOME/.local/share, joined with "oxmgr".
/// The plan text's `~/.oxmgr` was a misquote -- the real oxmgr registry lives under XDG (verified in the 2026-10-12 incident retro).
pub(super) fn default_data_dir() -> Option<PathBuf> {
    default_data_dir_from(
        std::env::var("XDG_DATA_HOME").ok().as_deref(),
        std::env::var("HOME").ok().as_deref(),
    )
}

fn default_data_dir_from(xdg: Option<&str>, home: Option<&str>) -> Option<PathBuf> {
    // ponytail: Windows has no HOME/XDG semantics -> None (the whole detection stays silent); this lifecycle face is
    // Linux/macOS production; no dirs dependency is added for Windows to read the registry.
    let base = xdg.filter(|s| !s.is_empty()).map(PathBuf::from).or_else(|| {
        home.filter(|s| !s.is_empty()).map(|h| PathBuf::from(h).join(".local/share"))
    })?;
    Some(base.join("oxmgr"))
}

/// Tolerant parsing: bad json / a non-OxMgrState shape -> empty table (the consumer treats it as "no split", no noise).
pub(super) fn parse_state(text: &str) -> Vec<ManagedEntry> {
    serde_json::from_str::<OxmgrState>(text).map(|s| s.processes).unwrap_or_default()
}

pub(super) fn read_registry(path: &Path) -> Vec<ManagedEntry> {
    std::fs::read_to_string(path).map(|t| parse_state(&t)).unwrap_or_default()
}

/// The shared daemon registry (default_data_dir missing / state.json missing = empty table).
pub(super) fn read_default_registry() -> Vec<ManagedEntry> {
    default_data_dir().map(|d| read_registry(&d.join("state.json"))).unwrap_or_default()
}

/// Entries in `entries` whose cwd sits under my_dir (after absolute-canonicalizing both sides, by Path component prefix --
/// sibling shapes like /dir/server-2 do not slip in). Pure function, shared by doctor and contention.
pub(super) fn strays_under(entries: &[ManagedEntry], my_dir: &Path) -> Vec<ManagedEntry> {
    let mine = absolute(my_dir);
    entries.iter().filter(|e| absolute(&e.cwd).starts_with(&mine)).cloned().collect()
}

#[derive(Debug)]
pub(super) struct SplitVerdict {
    /// Rows under the shared/old daemon whose cwd belongs to this instance
    pub strays: Vec<ManagedEntry>,
    /// This instance's registry entry count (for text cross-reference)
    pub instance_count: usize,
}

/// Split predicate: any row in the default table whose cwd is in our dir = split. status vocabulary is irrelevant -- only
/// "running" is observed on disk, but being registered under the wrong daemon is itself the chaos source (the instance CLI's stop cannot reach it
/// = the 2026-10-12 "said stopped but the port is still held" incident shape). **An empty instance table does not exempt**: the incident was exactly
/// an empty instance table + the default table holding everything; a "both tables non-empty" rule would miss it.
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
        "oxmgr available",
        "not in PATH (deploy pins it to <prefix>/bin -- export PATH or re-deploy)",
    );
    check(
        which("caddy").is_some(),
        "caddy in PATH",
        "bare metal needs caddy installed (ignorable in the dev --no-web form)",
    );

    let cfg_path = dir.join("etc").join("server.yaml");
    let cfg = std::fs::read_to_string(&cfg_path).ok();
    check(
        cfg.as_deref().is_some_and(|c| parse_listen_port(c).is_some()),
        "server.yaml parseable",
        "missing or listen.port parse failed -- run init first",
    );
    let parsed =
        cfg_path.exists().then(|| mediaservo_server::config::load(&cfg_path).ok()).flatten();
    check(
        parsed.is_some(),
        "server.yaml fully valid",
        "ServerConfig deserialization failed (start would refuse)",
    );
    for f in ["devices.yaml", "accounts.yaml"] {
        let p = dir.join("etc").join(f);
        let ok = std::fs::read_to_string(&p)
            .ok()
            .is_some_and(|t| serde_yaml::from_str::<serde_yaml::Value>(&t).is_ok());
        check(ok, &format!("{f} parseable"), "missing or corrupt YAML");
    }

    // web dist (design: bundling is a delivery convenience, not a hard coupling -- in the dev --no-web form this failure is expected; noted in wording)
    let cf = std::fs::read_to_string(dir.join("etc").join("Caddyfile")).ok();
    let dist_ok = cf.as_deref().and_then(parse_web_root).is_some_and(|r| {
        Path::new(&r).is_dir() && std::fs::read_dir(&r).is_ok_and(|mut d| d.next().is_some())
    });
    check(
        dist_ok,
        "web dist non-empty",
        "Caddyfile root dir missing/empty (build web after changing TS; ignorable in the dev --no-web form)",
    );

    let announced_in_yaml = parsed.as_ref().is_some_and(|c| !c.sfu.announced_ips.is_empty());
    check(
        effective_announced(announced_in_yaml),
        "announced IP configured",
        "env MEDIASERVO_SFU_ANNOUNCED_IP and sfu.announced_ips both unset -- high black-frame risk in multi-NIC/container environments (C38 layer 1)",
    );

    // T6b 1 4 drift/rot inspection -- [warn]/[info] lines do not count into failed (the doctor exit-code = failure-count contract unchanged)
    if let Some(c) = parsed.as_ref() {
        let env_web = std::env::var("MEDIASERVO_WEB_PORT").ok().and_then(|v| v.parse::<u16>().ok());
        if let Some((caddy, intent)) = web_port_drift(c.web.port, env_web, cf.as_deref()) {
            println!(
                "[warn] Caddyfile(:{caddy}) and server.yaml/env(:{intent}) drifted -- the start probe follows the Caddyfile; to make yaml win, delete etc/Caddyfile and re-init"
            );
        }
        // The rot check only runs when the yaml value is in effect (under an env override the yaml is dead config; WARNing at a non-effective value = misleading)
        if announced_in_yaml && std::env::var("MEDIASERVO_SFU_ANNOUNCED_IP").is_err() {
            let local: std::collections::HashSet<String> =
                mediaservo_server::sfu::detect_all_ips().into_iter().collect();
            match announced_stale_items(&c.sfu.announced_ips, &local) {
                None => println!(
                    "[info] announced rot check exempted: first item not in local interfaces = container posture (first-only criterion; multi-IP bare metal with a rotted first item is an accepted blind spot)"
                ),
                Some(stale) if stale.is_empty() => println!(
                    "[ok] announced_ips all still belong to local interfaces ({} items)",
                    c.sfu.announced_ips.len()
                ),
                Some(stale) => println!(
                    "[warn] announced_ips rotted: {stale:?} are no longer local interface IPs -- two exits: bare metal = clear sfu.announced_ips for auto-detect / container = update to currently reachable IPs (announcing rotted IPs = client ICE total failure, C38 layer 1)"
                ),
            }
        }
    }

    // T5 dual-daemon registry split inspection -- [warn] not counted in failed (T6b precedent; the exit code = failure-count contract unchanged)
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
                "[warn] oxmgr dual-daemon split: {} of this instance's apps registered under a shared/old daemon: {} (this instance's registry has {n} entries) -- {g}",
                v.strays.len(),
                items.join(", "),
                n = v.instance_count,
                g = SPLIT_GUIDANCE
            );
        }
    }

    if let Some(p) = cfg.as_deref().and_then(parse_listen_port) {
        println!(
            "[info] backend port {p}: {}",
            if port_in_use(p) {
                "occupied (this instance running, or someone else)"
            } else {
                "idle"
            }
        );
    }
    if let Some(wp) = cf.as_deref().and_then(parse_web_port) {
        println!("[info] web port {wp}: {}", if port_in_use(wp) { "occupied" } else { "idle" });
    }
    if failed == 0 {
        println!("doctor: all passed ({})", dir.display());
    }
    i32::try_from(failed).unwrap_or(125).min(125)
}

/// `logs [server|web|weaknet|all] [<dir>] [-f|--lines N]`: oxmgr logs passthrough (app names map to brand;
/// weaknet-serve is brand-independent [mn10]; the logs anchor = run/logs/weaknet-serve.*.log).
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
                eprintln!("logs: extra arguments: {s}");
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
        // all green = 0
        assert_eq!(
            map_status_exit(
                Some("running"),
                Some(Some("running")),
                Some(Some("running")),
                Some(true)
            ),
            0
        );
        // --no-web: web not in cluster, not an anomaly; weaknet None likewise (the bin-missing conditional-render form [M-1])
        assert_eq!(map_status_exit(Some("running"), None, None, Some(true)), 0);
        // web in cluster but stopped = 1 (degraded)
        assert_eq!(map_status_exit(Some("running"), Some(Some("stopped")), None, Some(true)), 1);
        // weaknet in cluster but stopped = 1 (BA-6 new case: same semantics as web; None = not in cluster, not counted)
        assert_eq!(
            map_status_exit(
                Some("running"),
                Some(Some("running")),
                Some(Some("stopped")),
                Some(true)
            ),
            1
        );
        // web removed while weaknet alive (the correct --no-web shape lck-F8) = 0: web=None does not drag it along
        assert_eq!(map_status_exit(Some("running"), None, Some(Some("running")), Some(true)), 0);
        // probe failure (worker dead/503) = 1
        assert_eq!(map_status_exit(Some("running"), None, None, Some(false)), 1);
        assert_eq!(
            map_status_exit(Some("running"), Some(Some("running")), Some(Some("running")), None),
            1
        );
        // server not running/row missing = 2
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
        // consistent (yaml == Caddyfile) -> None, no drift
        assert_eq!(web_port_drift(Some(8080), None, Some(&cf_text(8080))), None);
        // default chain consistent: neither yaml nor env set = 8080 default, Caddyfile 8080 -> None
        assert_eq!(web_port_drift(None, None, Some(&cf_text(8080))), None);
        // drift: yaml 8093 vs Caddyfile 8080 -> Some((caddy, intent))
        assert_eq!(web_port_drift(Some(8093), None, Some(&cf_text(8080))), Some((8080, 8093)));
        // intent chain yaml > env: env 8091 fallback (yaml None)
        assert_eq!(web_port_drift(None, Some(8091), Some(&cf_text(8080))), Some((8080, 8091)));
        // yaml overrides env: yaml 8092 env 8091 vs Caddyfile 8092 -> None
        assert_eq!(web_port_drift(Some(8092), Some(8091), Some(&cf_text(8092))), None);
        // contract S2.6: antique/hand-broken Caddyfile (parse None) -> skip, no WARN
        assert_eq!(web_port_drift(Some(8093), None, Some("no site block here")), None);
        // Caddyfile absent (--no-web / never init-ed) -> skip
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
        // bare metal (first item in local interfaces) with rotted items -> Some(the rot list)
        assert_eq!(
            announced_stale_items(
                &ips(&["192.168.2.127", "203.0.113.9"]),
                &ipset(&["192.168.2.127"])
            ),
            Some(ips(&["203.0.113.9"]))
        );
        // bare metal healthy: all local -> Some(empty) (pass)
        assert_eq!(
            announced_stale_items(&ips(&["192.168.2.127"]), &ipset(&["192.168.2.127"])),
            Some(vec![])
        );
        // container posture (first item not local = a host-reachable IP shape) -> None INFO exemption, no WARN
        assert_eq!(announced_stale_items(&ips(&["192.168.2.127"]), &ipset(&["172.18.0.2"])), None);
        // ponytail v1 blind-spot pin (first-only criterion, the PLAN S2.6 exemption branch): multi-IP bare metal with a rotted first item -> misjudged container exemption
        assert_eq!(
            announced_stale_items(
                &ips(&["203.0.113.9", "192.168.2.127"]),
                &ipset(&["192.168.2.127"])
            ),
            None
        );
        // empty config -> None (use_bare_metal_listen([]) = false)
        assert_eq!(announced_stale_items(&[], &ipset(&["192.168.2.127"])), None);
    }

    // -- T5 dual-daemon registry split detection (fixture = static json strings; no daemon/disk touched) --
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
        // corrupt json / a non-oxmgr shape -> empty table (silent pass, no noise)
        assert!(parse_state("{oops").is_empty());
        assert!(parse_state("{}").is_empty());
        // unknown fields ignored + missing/null pid tolerated (the stopped shape)
        let loose = parse_state(
            r#"{"processes":[{"id":1,"name":"x","status":"stopped","pid":null,"cwd":"/a","whatever":42}]}"#,
        );
        assert_eq!(loose.len(), 1);
        assert_eq!(loose[0].pid, None);
    }

    #[test]
    fn split_registry_catches_incident_shape_default_holds_ours_instance_empty() {
        // the 2026-10-12 incident shape: this instance's apps all under the shared daemon, instance table empty --
        // a "judge only when both tables are non-empty" rule would miss it. The predicate = cwd ownership, independent of instance-table content.
        let default = parse_state(INCIDENT_STATE);
        let v = split_registry(&default, &[], Path::new("/deploy/out/server"))
            .expect("the incident shape must be detected");
        assert_eq!(v.strays.len(), 2);
        assert_eq!(v.instance_count, 0);
    }

    #[test]
    fn split_registry_clean_single_registry_none() {
        // normal shape: this instance's apps in the instance table, no rows of ours in the default table
        let default = vec![entry(1, "other-app", "running", "/opt/otherproject")];
        let instance = vec![entry(2, "msrtc-server", "running", "/deploy/out/server")];
        assert!(split_registry(&default, &instance, Path::new("/deploy/out/server")).is_none());
        // default table empty -> None (no noise)
        assert!(split_registry(&[], &instance, Path::new("/deploy/out/server")).is_none());
    }

    #[test]
    fn split_registry_foreign_prefix_lookalike_not_stray() {
        // Path component-level prefix predicate: /deploy/out/server-2 is not under /deploy/out/server
        let default = vec![entry(9, "sibling-instance", "running", "/deploy/out/server-2")];
        assert!(split_registry(&default, &[], Path::new("/deploy/out/server")).is_none());
    }

    #[test]
    fn split_registry_status_agnostic_pins_cwd_membership_rule() {
        // rule pin: any row in the default table with cwd in our dir = split; the status vocabulary is irrelevant
        // (only "running" is seen on disk; a stopped ghost is just as much a registry-ownership chaos source)
        let default = vec![entry(3, "msrtc-server", "stopped", "/deploy/out/server")];
        let v = split_registry(&default, &[], Path::new("/deploy/out/server"))
            .expect("a stopped ghost is judged the same");
        assert_eq!(v.strays[0].status, "stopped");
    }

    #[test]
    fn strays_under_pure_match_for_contention() {
        // the contention-side predicate: port occupied + /proc miss -> look for rows of our dir in the default table
        let default = parse_state(INCIDENT_STATE);
        let strays = strays_under(&default, Path::new("/deploy/out/server"));
        assert_eq!(strays[0].name, "msrtc-server");
        assert_eq!(strays[0].id, 124);
        assert_eq!(strays[0].pid, Some(3419236));
        assert!(strays_under(&default, Path::new("/somewhere/else")).is_empty());
    }

    #[test]
    fn default_data_dir_prefers_xdg_then_home() {
        // pure-arg shape pins the predicate (set_var is unsafe under edition 2024 -- process env untouched)
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
