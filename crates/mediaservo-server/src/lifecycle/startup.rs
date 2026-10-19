//! `startup on|off|status` -- the boot anchor (frontend-process-split T19).
//!
//! Fully isomorphic with host: a systemd user unit brings up the **instance oxmgr daemon** (Restart=always),
//! the daemon persists app state -> the whole cluster (server + caddy) revives automatically at boot. Non-Linux defers to oxmgr service.
//!
//! startup-hardening S1: conflict gate v2 -- identity checks (brand-aware prefix, contains-matching forbidden --
//! the TalkServo same-family unit misidentification incident surface) + port comparison (three-source intersection) + interactive takeover discipline (PLAN S2.1).

use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::templates::{
    instance_daemon_port, parse_listen_port, parse_weaknet_port, parse_web_port, server_namespace,
    server_product,
};
use super::{absolute, oxmgr_bin, oxmgr_list, parse_dir, run_oxmgr};

// -- S1: identity check (unit identity classification -- brand-aware prefix, exact match, contains forbidden) --

/// Unit identity classification (pure function; tests mirror the host-side semantics).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum UnitIdentity {
    /// This product's server domain (current brand ns prefix union the default brand `oxmgr-mediaservo-server-` -- S6 minor#2 union rule).
    Server,
    /// This product's host domain (brand unit_prefix covers both the legacy `oxmgr-host-` and branded `oxmgr-<brand>-` forms).
    Host,
    /// This product's unknown prefix `oxmgr-*.service` falling into neither domain above (TalkServo same-family/leftovers) -- WARN list,
    /// never silent, never a takeover candidate.
    Unknown,
    /// Non-oxmgr unit -- unrelated, ignored.
    Foreign,
}

/// Known brand set (cross-brand mutual lookup union -- ruling 1): server domain `{bin}-server-`, host domain
/// both `host-`/`<brand>-` forms. TalkServo same-family is not in the set -> Unknown semantics unchanged.
const KNOWN_BRANDS: &[&str] = &["mediaservo", "msrtc"];

/// Known brands' host-domain prefixes (deduped): `host-` (legacy/default form) union `{brand}-` per brand.
fn known_host_forms(brand: &mediaservo_common::brand::Brand) -> Vec<String> {
    let mut forms: Vec<String> = KNOWN_BRANDS
        .iter()
        .map(|b| {
            let b = mediaservo_common::brand::media_brand_from(Some(b));
            b.unit_prefix.trim_start_matches("oxmgr-").to_string()
        })
        .chain(std::iter::once(brand.unit_prefix.trim_start_matches("oxmgr-").to_string()))
        .collect();
    forms.sort();
    forms.dedup();
    forms
}

/// Known brands' server-domain prefixes (deduped): `{bin_prefix}server-` per brand.
/// The ns form (`oxmgr-{ns}-` = bin_prefix+server) is always a subset of that set (derived from bin_prefix).
fn known_server_forms(brand: &mediaservo_common::brand::Brand) -> Vec<String> {
    let mut forms: Vec<String> = KNOWN_BRANDS
        .iter()
        .map(|b| {
            format!("{}server-", mediaservo_common::brand::media_brand_from(Some(b)).bin_prefix)
        })
        .collect();
    forms.sort();
    forms.dedup();
    let _ = brand; // parameter shape kept (seam symmetry; the current set enumerates across brands, no current-brand slot)
    forms
}

/// unit file name -> identity (the `.service` suffix may be omitted -- both states accepted; callers pass the full name).
pub(super) fn classify_unit(name: &str) -> UnitIdentity {
    classify_unit_with(name, &mediaservo_common::brand::media_brand())
}

/// Same as above (the brand parameter seam -- for M1 pin tests; production goes through the `classify_unit` thin shell).
fn classify_unit_with(name: &str, brand: &mediaservo_common::brand::Brand) -> UnitIdentity {
    let stem = name.strip_suffix(".service").unwrap_or(name);
    let Some(rest) = stem.strip_prefix("oxmgr-").or((stem == "oxmgr").then_some("")) else {
        return UnitIdentity::Foreign;
    };
    if known_server_forms(brand).iter().any(|f| rest.starts_with(f.as_str())) {
        return UnitIdentity::Server;
    }
    if known_host_forms(brand).iter().any(|f| rest.starts_with(f.as_str())) {
        return UnitIdentity::Host;
    }
    UnitIdentity::Unknown
}

/// unit text -> instance dir (env dual keys, OXMGR_HOME= first, miss falls to OXMGR_DATA_DIR= -- reap.rs precedent).
/// Value = `<instance>/run/oxmgr`, two levels up is the instance root; parse failure -> None.
pub(super) fn unit_instance_dir(unit_text: &str) -> Option<PathBuf> {
    let home = unit_env_value(unit_text, "OXMGR_HOME");
    let dir = home.or_else(|| unit_env_value(unit_text, "OXMGR_DATA_DIR"))?;
    let p = PathBuf::from(dir);
    // `<dir>/run/oxmgr` -> `<dir>`; non-two-hop shapes (manually mangled) return the original value (callers can probe run/oxfile for liveness)
    Some(p.parent().and_then(Path::parent).map(Path::to_path_buf).unwrap_or(p))
}

/// The value of an `Environment=KEY=value` line in the unit text (trimmed; first hit).
fn unit_env_value(unit_text: &str, key: &str) -> Option<String> {
    let prefix = format!("Environment={key}=");
    unit_text.lines().find_map(|l| l.trim().strip_prefix(&prefix).map(|v| v.trim().to_string()))
}

// -- S1: port comparison (three sources -- parse functions shared with stop; a failed parse skips that source) --

/// The instance's three probe ports (server domain: yaml listen + Caddyfile web + oxfile weaknet).
/// Extracted to be shared with stop_ports (stop and the startup conflict gate share a source -- PLAN S2.1).
pub(super) fn instance_ports(dir: &Path) -> Vec<u16> {
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
    if let Ok(ox) = std::fs::read_to_string(dir.join("run").join("oxfile.toml"))
        && let Some(p) = parse_weaknet_port(&ox)
    {
        ports.push(p);
    }
    ports.sort();
    ports.dedup();
    ports
}

/// The intersection of two port sets (ascending).
fn overlap(a: &[u16], b: &[u16]) -> Vec<u16> {
    let mut out: Vec<u16> = a.iter().filter(|p| b.contains(p)).copied().collect();
    out.sort();
    out.dedup();
    out
}

// -- S1: decision (pure functions -- interactive I/O stays with the caller) -------------------------

/// A single conflict peer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Conflict {
    pub peer_unit: String,
    pub peer_dir: PathBuf,
    pub peer_kind: PeerKind,
    pub overlapping: Vec<u16>,
}

/// Peer domain: same kind (same CLI domain) / cross domain (server vs host).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PeerKind {
    SameDomain,
    CrossDomain,
}

/// The four decision states (decide_startup v2 -- the S2.1 pinned shape). Coexist carries a coexistence note line.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Decision {
    Install,
    Coexist(String),
    PromptUnenrollPeer,
    PromptCrossDomainWarn,
    RegisterWithWarning,
}

/// Aggregate decision (S2.1 + R3-B startup-pure-registration): a same-kind collision prompts a peer
/// REGISTRATION un-enroll (listing all), otherwise a cross-domain collision warns; all-empty sets coexist-install.
/// Non-tty collisions warn and register anyway -- registration never blocks on a human and never touches processes.
pub(super) fn decide_v2(conflicts: &[Conflict], mine: &[u16], tty: bool) -> Decision {
    let _ = mine; // the intersection is already budgeted in Conflict.overlapping
    let same: Vec<&Conflict> =
        conflicts.iter().filter(|c| c.peer_kind == PeerKind::SameDomain).collect();
    let cross: Vec<&Conflict> =
        conflicts.iter().filter(|c| c.peer_kind == PeerKind::CrossDomain).collect();
    let same_hit = same.iter().any(|c| !c.overlapping.is_empty());
    let cross_hit = cross.iter().any(|c| !c.overlapping.is_empty());
    if same_hit {
        return if tty { Decision::PromptUnenrollPeer } else { Decision::RegisterWithWarning };
    }
    if cross_hit {
        return if tty { Decision::PromptCrossDomainWarn } else { Decision::RegisterWithWarning };
    }
    let mut peers: Vec<String> = conflicts
        .iter()
        .map(|c| format!("{} (no port intersection)", c.peer_dir.display()))
        .collect();
    peers.sort();
    peers.dedup();
    match peers.len() {
        0 => Decision::Install,
        1 => {
            Decision::Coexist(format!("coexists with instance {} (no port intersection)", peers[0]))
        }
        _ => Decision::Coexist(format!(
            "coexists with instances {} (no port intersection)",
            peers.join(", ")
        )),
    }
}

/// Unit name (derived from the dir slug -- multiple instances can each install; global-uniqueness detection reverse-looks-up by prefix).
pub fn startup_unit_name(dir: &Path) -> String {
    let raw = dir.to_string_lossy().replace(['/', '\\', ' '], "-");
    let raw = raw.trim_start_matches('-');
    format!("oxmgr-{}-{raw}.service", server_namespace())
}

/// Unit content (ExecStart=oxmgr daemon run + instance env -- design revised model: the anchor brings up the daemon,
/// the daemon persists app state and revives the whole cluster at boot; Restart=always watches the daemon.
/// Note: the task brief once proposed ExecStart=`bin start <dir>` -- start exits quickly x Restart=always = a restart storm,
/// and the host production precedent (oxmgr-*-host unit) is exactly daemon run; follow the design.md revised model).
pub fn render_startup_unit(dir: &Path, oxmgr: &str, cli: &str) -> String {
    let home = absolute(&dir.join("run").join("oxmgr")).to_string_lossy().into_owned();
    let port = instance_daemon_port(Path::new(&home));
    format!(
        "[Unit]
Description={product} oxmgr daemon ({dir})
After=network.target
ConditionPathExists={oxmgr}

[Service]
Type=simple
Environment=OXMGR_HOME={home}
Environment=OXMGR_DAEMON_ADDR=127.0.0.1:{port}
Environment=OXMGR_API_ADDR=127.0.0.1:{api}
Environment=OXMGR_LOG_MAX_SIZE_MB=10
Environment=OXMGR_LOG_MAX_FILES=3
Environment=OXMGR_LOG_MAX_DAYS=7
ExecStart={oxmgr} daemon run
Restart=always
RestartSec=2
TimeoutStartSec=240
{post}

[Install]
WantedBy=default.target
",
        product = server_product(),
        dir = dir.to_string_lossy(),
        api = port + 1000,
        post = if cli.is_empty() {
            String::new()
        } else {
            // k19 boot hook (host same cut, PIT-236): cold daemon may start before NTP
            // sync and drop the registry — our idempotent apply revives the fleet;
            // always exit 0 (failing ExecStartPost x Restart=always = restart storm).
            format!(
                "ExecStartPost=/bin/sh -c 'for i in $(seq 1 90); do if [ \"$i\" -ge 60 ] || timedatectl show-properties --value -p NTPSynchronized 2>/dev/null | grep -q yes; then \"{}\" apply \"{}\" && break; fi; sleep 2; done; exit 0'",
                cli,
                dir.to_string_lossy()
            )
        },
    )
}

fn units_dir() -> PathBuf {
    Path::new(&std::env::var("HOME").unwrap_or_default()).join(".config/systemd/user")
}

/// Scan other instances' product anchor units (S1 v2: identity check + dual-key env reverse derivation of the instance dir).
/// Returns (unit path, instance dir, identity). unknown/foreign filtered out (unknown is WARNed by the caller --
/// this function only accepts the Server/Host domains; the unknown set goes through `scan_unknown_units` separately).
fn scan_product_units(dir: &Path) -> (Vec<(PathBuf, PathBuf, UnitIdentity)>, Vec<String>) {
    let mine = startup_unit_name(dir);
    let mut found = Vec::new();
    let mut unknown = Vec::new();
    let Ok(entries) = std::fs::read_dir(units_dir()) else {
        return (found, unknown);
    };
    for e in entries.flatten() {
        let name = e.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !name.ends_with(".service") || name == mine {
            continue;
        }
        let id = classify_unit(name);
        match id {
            UnitIdentity::Foreign => continue,
            UnitIdentity::Unknown => {
                unknown.push(name.to_string());
                continue;
            }
            other => {
                let path = e.path();
                let d = std::fs::read_to_string(&path)
                    .ok()
                    .and_then(|c| unit_instance_dir(&c))
                    .unwrap_or_default();
                found.push((path, d, other));
            }
        }
    }
    (found, unknown)
}

/// The first whitespace-delimited token of the unit text's ExecStart= (the bin path -- used by the status liveness check).
fn first_token(unit_text: &str) -> Option<String> {
    unit_text
        .lines()
        .find_map(|l| l.trim().strip_prefix("ExecStart="))
        .and_then(|v| v.split_whitespace().next().map(String::from))
}

/// A single unit's health-face line group (pure rendering -- the sole criterion of the S2 four-shape pin tests).
/// enabled: None = systemctl did not answer (treated as no); linger: false appends the enable-linger hint;
/// bin_alive: false = ExecStart points at a deleted binary (zombie -- boot will fail + cleanup hint).
fn status_line(
    unit: &str,
    tag: &str,
    path: Option<(&Path, bool)>,
    enabled: Option<&str>,
    active: Option<&str>,
    linger: bool,
    bin_alive: bool,
) -> Vec<String> {
    let head = if tag.is_empty() { unit.to_string() } else { format!("{tag} {unit}") };
    let mut out = vec![head];
    out.push(match path {
        Some((p, false)) => format!("  path:      {}", p.display()),
        Some((p, true)) => format!("  path:      {} (from ExecStart)", p.display()),
        None => "  path:      <unknown: no OXMGR_HOME in this unit>".to_string(),
    });
    out.push(format!("  enabled: {}", enabled.unwrap_or("no")));
    let mut daemon_line = format!("  daemon:  {}", active.unwrap_or("inactive"));
    if active == Some("failed") {
        daemon_line.push_str("  [!] diagnose: journalctl --user -u <unit> -n 20");
    }
    out.push(daemon_line);
    let mut linger_line = format!("  linger:  {}", if linger { "enabled" } else { "disabled" });
    if !linger {
        linger_line.push_str(" -- will not self-start after reboot: loginctl enable-linger");
    }
    out.push(linger_line);
    out.push(if bin_alive {
        "  instance: bin OK".to_string()
    } else {
        "  [!] bin deleted -- boot will fail. Clean up: startup off <dir> or systemctl --user disable --now + rm"
            .to_string()
    });
    out
}

/// Unit health verdict (exit-code semantics pure function): daemon failed or bin missing = unhealthy.
fn unit_healthy(enabled: Option<&str>, active: Option<&str>, bin_alive: bool) -> bool {
    enabled == Some("enabled") && active == Some("active") && bin_alive
}

/// A local unit's enabled state (thin exec layer -- failure/exception -> None).
fn systemctl_enabled(unit: &str) -> Option<String> {
    let o = Command::new("systemctl").args(["--user", "is-enabled", unit]).output().ok()?;
    let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// A local unit's is-active state (thin exec layer).
fn systemctl_active(unit: &str) -> Option<String> {
    let o = Command::new("systemctl").args(["--user", "is-active", unit]).output().ok()?;
    let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// Linger state (S5 M2 pinned shape: `loginctl show-user $(id -u) -p Linger`, no -U -- the local
/// `invalid option -- U` evidence; overall loginctl failure -> falls back to the existence of `/var/lib/systemd/linger/$USER`).
/// If no ground truth is readable at all -> false (better to hint than to under-report).
fn linger_enabled() -> bool {
    // S5 M2 pinned shape: `loginctl show-user $(id -u) -p Linger` (no -U -- the local `invalid option -- U` evidence).
    let uid = Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty());
    if let Some(uid) = uid
        && let Ok(o) = Command::new("loginctl").args(["show-user", &uid, "-p", "Linger"]).output()
    {
        let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
        if s.contains("Linger=") {
            return s.ends_with("=yes");
        }
    }
    // Fallback (M2): overall loginctl failure -> the existence of /var/lib/systemd/linger/<USER>.
    std::env::var("USER")
        .ok()
        .is_some_and(|u| Path::new("/var/lib/systemd/linger").join(u).exists())
}

/// off no-arg selection (pure function): zero units = already installed, idempotent; exactly one = stop it directly; many = tty list-select
/// / non-tty reject.
fn select_off(count: usize, tty: bool) -> OffSelect {
    match count {
        0 => OffSelect::NoneInstalled,
        1 => OffSelect::Single,
        _ if tty => OffSelect::Prompt,
        _ => OffSelect::RejectList,
    }
}

/// The four states of `select_off`.
#[derive(Debug, PartialEq, Eq)]
enum OffSelect {
    NoneInstalled,
    Single,
    Prompt,
    RejectList,
}

pub(super) fn cmd_startup(args: &mut dyn Iterator<Item = String>) -> i32 {
    let sub = args.next().unwrap_or_default();
    if !matches!(sub.as_str(), "on" | "off" | "status") {
        eprintln!(
            "Usage: {} startup <on|off|status> [<dir>] (status/off with no arg = full local inventory)",
            server_product()
        );
        return 2;
    }
    // S2: status/off with no arg = full inventory default semantics; with <dir> = the existing filter/targeted-uninstall form.
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
    let dir = absolute(&dir);
    match sub.as_str() {
        "on" => startup_install(&dir),
        "off" => startup_uninstall(&dir),
        _ => startup_status(&dir),
    }
}

/// Conflict aggregation (thin IO shell: read the peers' instance ports + intersect). Returns (conflict table, unknown unit name set).
fn collect_conflicts(dir: &Path) -> (Vec<Conflict>, Vec<String>) {
    let (units, unknown) = scan_product_units(dir);
    let mine = instance_ports(dir);
    let mut conflicts = Vec::new();
    for (path, peer_dir, id) in units {
        let kind = match id {
            UnitIdentity::Server => PeerKind::SameDomain,
            _ => PeerKind::CrossDomain,
        };
        let peer_ports =
            if peer_dir.as_os_str().is_empty() { Vec::new() } else { instance_ports(&peer_dir) };
        let overlapping = overlap(&mine, &peer_ports);
        conflicts.push(Conflict {
            peer_unit: path.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string(),
            peer_dir,
            peer_kind: kind,
            overlapping,
        });
    }
    (conflicts, unknown)
}

/// Conflict/peer detail lines (detail-first discipline -- both actions and refusals print this first).
fn conflict_lines(conflicts: &[Conflict]) -> Vec<String> {
    conflicts
        .iter()
        .map(|c| {
            format!(
                "unit={} instance={} domain={} conflicting ports={}",
                c.peer_unit,
                c.peer_dir.display(),
                match c.peer_kind {
                    PeerKind::SameDomain => "same kind",
                    PeerKind::CrossDomain => "cross domain",
                },
                if c.overlapping.is_empty() {
                    "none".to_string()
                } else {
                    format!("{:?}", c.overlapping)
                }
            )
        })
        .collect()
}

#[cfg(target_os = "linux")]
fn startup_install(dir: &Path) -> i32 {
    let (conflicts, unknown) = collect_conflicts(dir);
    for u in &unknown {
        eprintln!(
            "warning: detected unknown oxmgr units (domain not recognized, not handled): {u}"
        );
    }
    for line in conflict_lines(&conflicts) {
        eprintln!("detected other product auto-start anchors: {line}");
    }
    let tty = std::io::stdin().is_terminal();
    match decide_v2(&conflicts, &instance_ports(dir), tty) {
        Decision::RegisterWithWarning => {
            eprintln!(
                "warning: port overlap with another registered instance above -- registration proceeds; clear the clash (stop / startup off one side) before the next boot"
            );
        }
        Decision::PromptUnenrollPeer => {
            eprintln!(
                "same-kind instance port conflict -- pick one: [1] give up installing [2] un-enroll the peer's autostart REGISTRATION (its running processes stay untouched) [3] handle manually"
            );
            eprint!("choice: ");
            let _ = std::io::Write::flush(&mut std::io::stdout());
            let mut ans = String::new();
            let _ = std::io::stdin().read_line(&mut ans);
            match ans.trim() {
                "2" => {
                    for c in &conflicts {
                        if c.peer_kind == PeerKind::SameDomain {
                            eprintln!(
                                "  un-enroll: removing registration of {} (instance {}, its processes stay up)",
                                c.peer_unit,
                                c.peer_dir.display()
                            );
                            let _ = Command::new("systemctl")
                                .args(["--user", "disable", &c.peer_unit])
                                .status();
                            let _ = std::fs::remove_file(units_dir().join(&c.peer_unit));
                        }
                    }
                    let _ = Command::new("systemctl").args(["--user", "daemon-reload"]).status();
                }
                "3" => {
                    eprintln!("exited -- handle the conflicts above manually and retry (exit 1)");
                    return 1;
                }
                _ => {
                    println!("cancelled");
                    return 0;
                }
            }
        }
        Decision::PromptCrossDomainWarn => {
            eprintln!(
                "warning: cross-service-domain port overlap -- different service domains, usually a misconfiguration"
            );
            eprintln!(
                "choice: [1] override (un-enroll peer registration; processes stay up) [2] cancel this item [3] allow in parallel (risk acknowledged)"
            );
            eprint!("choice: ");
            let _ = std::io::Write::flush(&mut std::io::stdout());
            let mut ans = String::new();
            let _ = std::io::stdin().read_line(&mut ans);
            match ans.trim() {
                "1" => {
                    for c in &conflicts {
                        if c.peer_kind == PeerKind::CrossDomain && !c.overlapping.is_empty() {
                            eprintln!(
                                "  overwrite: un-enrolling {} (instance {}, its processes stay up)",
                                c.peer_unit,
                                c.peer_dir.display()
                            );
                            let _ = Command::new("systemctl")
                                .args(["--user", "disable", &c.peer_unit])
                                .status();
                            let _ = std::fs::remove_file(units_dir().join(&c.peer_unit));
                        }
                    }
                    let _ = Command::new("systemctl").args(["--user", "daemon-reload"]).status();
                }
                "3" => {
                    eprintln!(
                        "warning: allowing parallel operation -- the port-overlap risk is acknowledged"
                    );
                }
                _ => {
                    println!("cancelled");
                    return 0;
                }
            }
        }
        Decision::Coexist(note) => {
            println!("startup on: {note}");
        }
        Decision::Install => {}
    }
    let unit_name = startup_unit_name(dir);
    let unit_path = units_dir().join(&unit_name);
    if let Some(parent) = unit_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    // S5 live correction 2: systemd has no login PATH -- a bare `oxmgr` in ExecStart = unit 203/EXEC
    // crash loop (journalctl evidence). oxmgr_bin()'s final fallback is the bare name; force absolutization here:
    // no oxmgr beside the exe -> refuse to install and point the way (more honest than installing a unit guaranteed to explode).
    let oxmgr_path = oxmgr_bin();
    if !oxmgr_path.is_absolute() {
        eprintln!(
            "startup on: cannot locate an absolute oxmgr path (no oxmgr beside {} and PATH is unreliable) -- place bin/oxmgr and retry",
            std::env::current_exe()
                .map(|p| p.parent().map(|d| d.display().to_string()).unwrap_or_default())
                .unwrap_or_default()
        );
        return 1;
    }
    let oxmgr = oxmgr_path.to_string_lossy().into_owned();
    let cli = std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    if let Err(e) = std::fs::write(&unit_path, render_startup_unit(dir, &oxmgr, &cli)) {
        eprintln!("startup on: failed to write the unit: {e}");
        return 1;
    }
    println!("startup on: installed {} (daemon anchor, Restart=always)", unit_path.display());
    for args in [
        vec!["systemctl", "--user", "daemon-reload"],
        vec!["systemctl", "--user", "enable", unit_name.as_str()],
    ] {
        match Command::new(args[0]).args(&args[1..]).status() {
            Ok(s) if s.success() => {}
            Ok(s) => {
                eprintln!("startup on: {} exited {}", args.join(" "), s.code().unwrap_or(-1));
                return 1;
            }
            Err(e) => {
                eprintln!(
                    "startup on: failed to run {}: {e} (systemd user services unavailable?)",
                    args.join(" ")
                );
                return 1;
            }
        }
    }
    println!("startup on: recommend loginctl enable-linger for cross-session survival");
    let me = std::env::current_exe()
        .map(|x| x.display().to_string())
        .unwrap_or_else(|_| format!("{}-server", server_product()));
    println!(
        "startup on: registered for boot autostart ({unit_name}) -- no processes were started; to run now: {} start {}",
        me,
        dir.display()
    );
    0
}

#[cfg(target_os = "linux")]
fn startup_uninstall(dir: &Path) -> i32 {
    let unit_name = startup_unit_name(dir);
    let unit_path = units_dir().join(&unit_name);
    if !unit_path.exists() {
        println!("startup off: auto-start not installed ({} does not exist)", unit_path.display());
        return 0;
    }
    // R2 (startup-pure-registration): disable WITHOUT --now -- off un-registers only.
    let _ = Command::new("systemctl").args(["--user", "disable", &unit_name]).status();
    let _ = std::fs::remove_file(&unit_path);
    let _ = Command::new("systemctl").args(["--user", "daemon-reload"]).status();
    println!(
        "startup off: un-registered {} -- any running processes stay up (use `stop {}` for runtime)",
        unit_path.display(),
        dir.display()
    );
    0
}

#[cfg(target_os = "linux")]
fn startup_status(dir: &Path) -> i32 {
    let unit_name = startup_unit_name(dir);
    let unit_path = units_dir().join(&unit_name);
    if !unit_path.exists() {
        println!(
            "startup: not enabled ({} has no anchor unit -- enable with `{} startup on`)",
            dir.display(),
            server_product()
        );
        return 1;
    }
    println!("startup: installed ({})", unit_path.display());
    let enabled = systemctl_enabled(&unit_name);
    let active = systemctl_active(&unit_name);
    let bin_alive = std::fs::read_to_string(&unit_path)
        .ok()
        .and_then(|c| first_token(&c))
        .is_none_or(|p| Path::new(&p).exists());
    for line in status_line(
        &unit_name,
        "",
        Some((dir, false)),
        enabled.as_deref(),
        active.as_deref(),
        linger_enabled(),
        bin_alive,
    ) {
        println!("{line}");
    }
    i32::from(!unit_healthy(enabled.as_deref(), active.as_deref(), bin_alive))
}

/// Attribution scope (startup-status-attribution; host mirror of the same face).
#[derive(Debug, PartialEq, Eq)]
enum SrvScope {
    Own,
    Other,
    Stale,
    Unknown,
}

/// Instance roots = nearest ancestors of the exe or cwd carrying etc/server.yaml.
fn own_anchor_set(exe: Option<&Path>, cwd: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = Vec::new();
    for start in [exe, Some(cwd)].into_iter().flatten() {
        let mut cur = Some(start);
        while let Some(path) = cur {
            if path.join("etc").join("server.yaml").is_file() {
                if !v.contains(&path.to_path_buf()) {
                    v.push(path.to_path_buf());
                }
                break;
            }
            cur = path.parent();
        }
    }
    v
}

fn scope_matches_anchor(inst: Option<&Path>, anchors: &[PathBuf]) -> bool {
    let Some(i) = inst else { return false };
    let i_canon = i.canonicalize().unwrap_or_else(|_| i.to_path_buf());
    anchors.iter().any(|a| a == i || a.canonicalize().unwrap_or_else(|_| a.clone()) == i_canon)
}

fn scope_of(instance: Option<&Path>, bin_alive: bool, anchors: &[PathBuf]) -> (SrvScope, bool) {
    if !bin_alive {
        return (SrvScope::Stale, scope_matches_anchor(instance, anchors));
    }
    match instance {
        Some(i) if scope_matches_anchor(Some(i), anchors) => (SrvScope::Own, false),
        Some(_) => (SrvScope::Other, false),
        None => (SrvScope::Unknown, false),
    }
}

fn scope_tag(s: &SrvScope) -> &'static str {
    match s {
        SrvScope::Own => "[own]",
        SrvScope::Other => "[other]",
        SrvScope::Stale => "[stale]",
        SrvScope::Unknown => "[?]",
    }
}

fn scope_rank(s: &SrvScope) -> u8 {
    match s {
        SrvScope::Own => 0,
        SrvScope::Other => 1,
        SrvScope::Stale => 2,
        SrvScope::Unknown => 3,
    }
}

/// Scan all units of this domain -> health snapshot (thin IO shell).
struct UnitHealth {
    name: String,
    enabled: Option<String>,
    active: Option<String>,
    bin_alive: bool,
    instance: Option<PathBuf>,
    instance_derived: bool,
}

fn scan_health() -> (Vec<UnitHealth>, Vec<String>) {
    let mut out = Vec::new();
    let mut unknown = Vec::new();
    let Ok(entries) = std::fs::read_dir(units_dir()) else {
        return (out, unknown);
    };
    for e in entries.flatten() {
        let name = e.file_name();
        let Some(name) = name.to_str() else { continue };
        if !name.ends_with(".service") {
            continue;
        }
        match classify_unit(name) {
            UnitIdentity::Foreign => continue,
            UnitIdentity::Unknown => unknown.push(name.to_string()),
            _ => {
                // attribution (startup-status-attribution, host mirror): the unit file is
                // already read here for bin liveness -- parse the instance dir in same pass.
                let text = std::fs::read_to_string(e.path()).ok();
                let (bin_alive, instance, instance_derived) = match &text {
                    Some(t) => {
                        let alive = first_token(t).is_none_or(|x| Path::new(&x).exists());
                        let env_dir = unit_instance_dir(t);
                        let inst = env_dir.clone().or_else(|| {
                            first_token(t).and_then(|x| {
                                PathBuf::from(x)
                                    .parent()
                                    .and_then(Path::parent)
                                    .map(Path::to_path_buf)
                            })
                        });
                        (alive, inst, env_dir.is_none())
                    }
                    None => (true, None, false),
                };
                out.push(UnitHealth {
                    name: name.to_string(),
                    enabled: systemctl_enabled(name),
                    active: systemctl_active(name),
                    bin_alive,
                    instance,
                    instance_derived,
                })
            }
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    (out, unknown)
}

/// `startup status` (no arg): full four-line health-face inventory (S2). Script-consumable exit codes:
/// 0 = all healthy (enabled+active+bin OK); 1 = any zombie (bin missing) or daemon failed.
fn startup_status_all() -> i32 {
    let (units, unknown) = scan_health();
    let linger = linger_enabled();
    for u in &unknown {
        eprintln!(
            "warning: unknown oxmgr unit: {u} (not this product's naming -- to clean up manually run systemctl --user disable --now + rm)"
        );
    }
    if units.is_empty() {
        if unknown.is_empty() {
            println!(
                "no {} auto-start unit on this machine -- enable with `{} startup on <dir>`",
                server_product(),
                server_product()
            );
        }
        return 1;
    }
    println!(
        "scope: [own]=this instance tree | [other]=same brand, another instance | [stale]=binary gone (boot skips it via its ConditionPathExists guard) | [?]=instance path unresolvable"
    );
    let exe = std::env::current_exe().ok();
    let cwd = std::env::current_dir().unwrap_or_default();
    let anchors = own_anchor_set(exe.as_deref(), &cwd);
    let mut rows: Vec<(u8, &UnitHealth, String)> = units
        .iter()
        .map(|u| {
            let (sc, note) = scope_of(u.instance.as_deref(), u.bin_alive, &anchors);
            let tag = if sc == SrvScope::Stale && note {
                format!("{} (this instance)", scope_tag(&sc))
            } else {
                scope_tag(&sc).to_string()
            };
            (scope_rank(&sc), u, tag)
        })
        .collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.name.cmp(&b.1.name)));
    let mut unhealthy = 0;
    for (_, u, tag) in &rows {
        let e = u.enabled.as_deref();
        let a = u.active.as_deref();
        let path = u.instance.as_deref().map(|p| (p, u.instance_derived));
        for line in status_line(&u.name, tag, path, e, a, linger, u.bin_alive) {
            println!("{line}");
        }
        if !unit_healthy(e, a, u.bin_alive) {
            unhealthy += 1;
        }
    }
    i32::from(unhealthy > 0)
}

/// `startup off` (no arg): zero units = idempotent 0; exactly one = stop it directly; many = tty list-select / non-tty rejects with a list.
fn startup_off_all() -> i32 {
    let (units, _unknown) = scan_health();
    let tty = std::io::IsTerminal::is_terminal(&std::io::stdin());
    match select_off(units.len(), tty) {
        OffSelect::NoneInstalled => {
            println!("startup off: auto-start not installed");
            0
        }
        OffSelect::Single => uninstall_unit(&units[0].name),
        OffSelect::Prompt => {
            for (i, u) in units.iter().enumerate() {
                println!("[{}] {} ({})", i + 1, u.name, u.active.as_deref().unwrap_or("?"));
            }
            println!("[0] uninstall all / Enter = cancel");
            eprint!("choice: ");
            let _ = std::io::Write::flush(&mut std::io::stdout());
            let mut ans = String::new();
            let _ = std::io::stdin().read_line(&mut ans);
            let picked: Vec<String> = match ans.trim() {
                "" => return 0,
                "0" => units.iter().map(|u| u.name.clone()).collect(),
                n => match n.parse::<usize>().ok().filter(|n| (1..=units.len()).contains(n)) {
                    Some(i) => vec![units[i - 1].name.clone()],
                    None => {
                        eprintln!("invalid choice: {n}");
                        return 2;
                    }
                },
            };
            let mut rc = 0;
            for name in picked {
                rc |= uninstall_unit(&name);
            }
            rc
        }
        OffSelect::RejectList => {
            eprintln!(
                "startup off: multiple product auto-start units on this machine; non-interactive environments refuse blind uninstalls -- use `startup off <dir>` to target:",
            );
            for u in &units {
                eprintln!("  {}", u.name);
            }
            1
        }
    }
}

/// Uninstall a single unit (disable without --now + rm + daemon-reload) -- registration only (R2).
fn uninstall_unit(unit: &str) -> i32 {
    let _ = Command::new("systemctl").args(["--user", "disable", unit]).status();
    let _ = std::fs::remove_file(units_dir().join(unit));
    let _ = Command::new("systemctl").args(["--user", "daemon-reload"]).status();
    println!(
        "startup off: un-registered {unit} -- any running processes stay up (use `stop` for runtime)"
    );
    0
}

#[cfg(not(target_os = "linux"))]
fn startup_install(_dir: &Path) -> i32 {
    eprintln!(
        "startup on: non-Linux -- deferring to oxmgr service install (single-instance form: default data dir; multi-instance is Linux only)"
    );
    1
}
#[cfg(not(target_os = "linux"))]
fn startup_uninstall(_dir: &Path) -> i32 {
    eprintln!("startup off: non-Linux -- deferring to oxmgr service uninstall");
    1
}
#[cfg(not(target_os = "linux"))]
fn startup_status(_dir: &Path) -> i32 {
    eprintln!("startup status: non-Linux -- deferring to oxmgr service status");
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_unit_render_and_naming() {
        let u = render_startup_unit(
            Path::new("/opt/msr"),
            "/opt/msr/bin/oxmgr",
            "/opt/msr/bin/msrtc-server",
        );
        assert!(u.contains("ExecStart=/opt/msr/bin/oxmgr daemon run"));
        assert!(u.contains("After=network.target"));
        assert!(u.contains("Restart=always"));
        assert!(u.contains("Environment=OXMGR_HOME=/opt/msr/run/oxmgr"));
        // k19/k20: deleted-tree guard + boot apply hook (mirror of the host cut)
        assert!(u.contains("ConditionPathExists=/opt/msr/bin/oxmgr"));
        assert!(u.contains("ExecStartPost=/bin/sh -c"));
        assert!(u.contains("\"/opt/msr/bin/msrtc-server\" apply \"/opt/msr\""));
        assert!(u.contains("exit 0'"));
        assert!(u.contains("TimeoutStartSec=240"));
        let name = startup_unit_name(Path::new("/opt/msr"));
        assert!(name.starts_with("oxmgr-mediaservo-server-"), "{name}");
        assert!(name.ends_with("opt-msr.service"));
    }

    // -- S1: identity classification (S5 B1 mandate -- brand awareness + TalkServo negative + legacy fixture) --

    /// Legacy shape (the motivating fixture): `oxmgr-host-` prefix + OXMGR_DATA_DIR key + dead bin.
    const LEGACY_HOST_UNIT: &str = "\
[Unit]
Description=MediaServo host oxmgr daemon

[Service]
Environment=OXMGR_DATA_DIR=/home/maxsense/Documents/MediaServo/install/host/run/oxmgr
ExecStart=/old/bin/oxmgr daemon run
";

    /// current-server shape: the OXMGR_HOME key.
    const CURRENT_SERVER_UNIT: &str = "\
[Service]
Environment=OXMGR_HOME=/home/x/out/server/run/oxmgr
ExecStart=/home/x/out/server/bin/oxmgr daemon run
";

    // -- K1 cross-brand mutual lookup union (ruling 1; seam pin test -- each brand's view verified against the other) --

    #[test]
    fn k1_default_brand_perspective_recognizes_msrtc() {
        let msrtc = mediaservo_common::brand::media_brand_from(Some("msrtc"));
        // server domain: the msrtc bin's unit -> Server
        assert_eq!(
            classify_unit_with("oxmgr-msrtc-server-home-x-out-server.service", &msrtc),
            UnitIdentity::Server
        );
        // host domain: the msrtc unit -> Host (the default bin previously did not recognize oxmgr-msrtc-*)
        assert_eq!(
            classify_unit_with("oxmgr-msrtc-home-x-out-host.service", &msrtc),
            UnitIdentity::Host
        );
        // host domain: legacy default shape (mutating-view self-verification -- both brands recognize it)
        assert_eq!(
            classify_unit_with("oxmgr-host-home-x-out-host.service", &msrtc),
            UnitIdentity::Host
        );
        // TalkServo same-family still Unknown
        assert_eq!(
            classify_unit_with("oxmgr-talkservo-server-x.service", &msrtc),
            UnitIdentity::Unknown
        );
    }

    #[test]
    fn k1_msrtc_brand_perspective_recognizes_default() {
        let msrtc = mediaservo_common::brand::media_brand_from(Some("msrtc"));
        // the msrtc bin view recognizes the default-brand server unit
        assert_eq!(
            classify_unit_with("oxmgr-mediaservo-server-home-x-out-server.service", &msrtc),
            UnitIdentity::Server
        );
        // the msrtc bin view recognizes default/legacy host units (host.rs:1936-1939 once only recognized the msrtc- form)
        assert_eq!(classify_unit_with("oxmgr-host-home-x.service", &msrtc), UnitIdentity::Host);
        // the current brand's own form is not lost
        assert_eq!(classify_unit_with("oxmgr-msrtc-foo.service", &msrtc), UnitIdentity::Host);
        assert_eq!(
            classify_unit_with("oxmgr-msrtc-server-x.service", &msrtc),
            UnitIdentity::Server
        );
        // TalkServo negative
        assert_eq!(
            classify_unit_with("oxmgr-talkservo-host-x.service", &msrtc),
            UnitIdentity::Unknown
        );
    }

    #[test]
    fn k1_default_brand_perspective_recognizes_msrtc_via_default_param() {
        // the global seam under the current process brand (tests = default): both msrtc-domain units recognized
        assert_eq!(
            classify_unit("oxmgr-msrtc-server-home-x-out-server.service"),
            UnitIdentity::Server
        );
        assert_eq!(classify_unit("oxmgr-msrtc-home-x-out-host.service"), UnitIdentity::Host);
    }

    #[test]
    fn identity_classification_table() {
        // server domain: the current brand's ns form + the default brand's form
        assert_eq!(
            classify_unit("oxmgr-mediaservo-server-home-x-out-server.service"),
            UnitIdentity::Server,
            "default-brand ns form"
        );
        // the current brand's ns form -- this test process's brand = default, ns = mediaservo-server (same string, so merged assertion)
        assert_eq!(
            classify_unit(&format!("oxmgr-{}-foo.service", server_namespace())),
            UnitIdentity::Server,
            "ns={} (env may rebrand this test binary)",
            server_namespace()
        );
        // host domain: legacy/default `oxmgr-host-` (unit_prefix)
        assert_eq!(
            classify_unit("oxmgr-host-home-maxsense-Documents-MediaServo-install-host.service"),
            UnitIdentity::Host
        );
        // TalkServo same-family -> Unknown (the B1 negative -- must never be judged Server/Host)
        assert_eq!(classify_unit("oxmgr-talkservo-server-home-t.service"), UnitIdentity::Unknown);
        // bare leftover -> Unknown (stem == "oxmgr", empty rest falls into no domain)
        assert_eq!(classify_unit("oxmgr"), UnitIdentity::Unknown);
        assert_eq!(classify_unit("oxmgr.service"), UnitIdentity::Unknown);
        assert_eq!(classify_unit("oxmgr-mediaservo-server-x"), UnitIdentity::Server);
        // non-oxmgr -> Foreign
        assert_eq!(classify_unit("nginx.service"), UnitIdentity::Foreign);
        assert_eq!(classify_unit("myapp-oxmgr.service"), UnitIdentity::Foreign);
    }

    #[test]
    fn dual_key_extraction_home_first() {
        // HOME first (the reap.rs:274 precedent)
        assert_eq!(
            unit_instance_dir(CURRENT_SERVER_UNIT),
            Some(PathBuf::from("/home/x/out/server"))
        );
        // HOME miss -> falls to DATA_DIR; value = <instance>/run/oxmgr, two levels up is the instance root
        assert_eq!(
            unit_instance_dir(LEGACY_HOST_UNIT),
            Some(PathBuf::from("/home/maxsense/Documents/MediaServo/install/host"))
        );
        // both keys missing -> None
        assert_eq!(unit_instance_dir("[Service]\nExecStart=x\n"), None);
    }

    #[test]
    fn overlap_empty_and_nonempty() {
        assert!(overlap(&[9800, 8080], &[17980, 9810]).is_empty());
        assert_eq!(overlap(&[9800, 8080], &[8080, 1234]), vec![8080]);
        assert_eq!(overlap(&[8080], &[8080]), vec![8080]);
        assert!(overlap(&[], &[9800]).is_empty());
    }

    fn conflict(kind: PeerKind, ports: &[u16]) -> Conflict {
        Conflict {
            peer_unit: "oxmgr-x.service".into(),
            peer_dir: PathBuf::from("/tmp/peer"),
            peer_kind: kind,
            overlapping: ports.to_vec(),
        }
    }

    #[test]
    fn decide_v2_four_states() {
        // same-kind collision + tty -> takeover prompt; non-tty -> reject
        assert_eq!(
            decide_v2(&[conflict(PeerKind::SameDomain, &[9800])], &[9800], true),
            Decision::PromptUnenrollPeer
        );
        assert_eq!(
            decide_v2(&[conflict(PeerKind::SameDomain, &[9800])], &[9800], false),
            Decision::RegisterWithWarning
        );
        // cross-domain collision + tty -> warning confirm; non-tty -> reject
        assert_eq!(
            decide_v2(&[conflict(PeerKind::CrossDomain, &[8080])], &[8080], true),
            Decision::PromptCrossDomainWarn
        );
        assert_eq!(
            decide_v2(&[conflict(PeerKind::CrossDomain, &[8080])], &[8080], false),
            Decision::RegisterWithWarning
        );
    }

    #[test]
    fn decide_v2_coexist_and_clean_install() {
        // same kind, no intersection -> coexist (with note)
        match decide_v2(&[conflict(PeerKind::SameDomain, &[])], &[9800], false) {
            Decision::Coexist(note) => {
                assert!(note.contains("coexists with instance"), "{note}");
                assert!(note.contains("/tmp/peer"), "{note}");
            }
            other => panic!("expected Coexist, got {other:?}"),
        }
        // cross domain, no intersection -> coexist
        assert!(matches!(
            decide_v2(&[conflict(PeerKind::CrossDomain, &[])], &[9800], false),
            Decision::Coexist(_)
        ));
        // zero peers -> pure Install
        assert_eq!(decide_v2(&[], &[9800], false), Decision::Install);
    }

    #[test]
    fn decide_v2_aggregate_multi_conflict() {
        // same-kind + cross-domain collisions coexist -> same kind wins (takeover)
        assert_eq!(
            decide_v2(
                &[
                    conflict(PeerKind::SameDomain, &[9800]),
                    conflict(PeerKind::CrossDomain, &[8080])
                ],
                &[9800, 8080],
                true
            ),
            Decision::PromptUnenrollPeer
        );
        // only cross-domain, multiple collisions -> warning
        assert_eq!(
            decide_v2(
                &[conflict(PeerKind::CrossDomain, &[8080]), conflict(PeerKind::CrossDomain, &[])],
                &[8080],
                true
            ),
            Decision::PromptCrossDomainWarn
        );
    }

    /// contains("server") regression pin: a talkservo-server-domain unit never enters the conflict table even if its content carries ports
    /// (the classify layer already judged Unknown -- the scan layer filters it; this pins the classify negative against regressions).
    #[test]
    fn talkservo_family_never_same_domain() {
        assert_ne!(classify_unit("oxmgr-talkservo-server-x.service"), UnitIdentity::Server);
        assert_ne!(classify_unit("oxmgr-talkservo-server-x.service"), UnitIdentity::Host);
    }
    // -- S2: status four-line health face / off no-arg selection (pure-function pin tests, zero systemctl spawn) --

    const ZOMBIE_UNIT: &str =
        "[Service]\nEnvironment=OXMGR_HOME=/old/run/oxmgr\nExecStart=/old/bin/oxmgr daemon run\n";

    #[test]
    fn first_token_execstart() {
        assert_eq!(first_token(ZOMBIE_UNIT).as_deref(), Some("/old/bin/oxmgr"));
        assert_eq!(first_token("[Service]"), None);
    }

    #[test]
    fn status_line_four_shapes() {
        // 1. healthy
        let healthy = status_line(
            "u.service",
            "[own]",
            Some((Path::new("/srv/srv1"), false)),
            Some("enabled"),
            Some("active"),
            true,
            true,
        );
        assert_eq!(healthy[0], "[own] u.service");
        assert_eq!(healthy[1], "  path:      /srv/srv1");
        assert_eq!(healthy[2], "  enabled: enabled");
        assert_eq!(healthy[3], "  daemon:  active");
        assert_eq!(healthy[4], "  linger:  enabled");
        assert_eq!(healthy[5], "  instance: bin OK");
        assert!(!healthy.join("\n").contains("WARN"));
        // 2. zombie bin (boot will fail)
        let zombie = status_line(
            "u.service",
            "[stale]",
            Some((Path::new("/gone/srv1"), true)),
            Some("enabled"),
            Some("active"),
            true,
            false,
        );
        assert!(zombie[5].contains("bin deleted"));
        assert!(zombie[5].contains("boot will fail"));
        assert!(zombie[5].contains("startup off <dir>"));
        assert!(zombie[1].contains("(from ExecStart)"));
        // 3. daemon failed (with diagnosis hint)
        let failed =
            status_line("u.service", "", None, Some("enabled"), Some("failed"), true, true);
        assert!(failed[3].contains("failed"));
        assert!(failed[3].contains("journalctl"));
        assert!(failed[1].contains("<unknown"));
        // 4. linger off (with hint)
        let no_linger =
            status_line("u.service", "", None, Some("enabled"), Some("active"), false, true);
        assert_eq!(
            no_linger[4],
            "  linger:  disabled -- will not self-start after reboot: loginctl enable-linger"
        );
    }

    #[test]
    fn attribution_scope_matrix() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().to_path_buf();
        std::fs::create_dir_all(root.join("etc")).unwrap();
        std::fs::write(
            root.join("etc").join("server.yaml"),
            "sfu: {}
",
        )
        .unwrap();
        let other = PathBuf::from("/srv/other");
        let anchors = vec![root.clone()];
        assert_eq!(scope_of(Some(&root), true, &anchors), (SrvScope::Own, false));
        assert_eq!(scope_of(Some(&other), true, &anchors), (SrvScope::Other, false));
        assert_eq!(scope_of(Some(&root), false, &anchors), (SrvScope::Stale, true));
        assert_eq!(scope_of(None, true, &anchors), (SrvScope::Unknown, false));
        assert_eq!(scope_of(Some(&root), true, &[]), (SrvScope::Other, false));
        assert!(scope_rank(&SrvScope::Own) < scope_rank(&SrvScope::Other));
        assert!(scope_rank(&SrvScope::Other) < scope_rank(&SrvScope::Stale));
        assert_eq!(scope_tag(&SrvScope::Stale), "[stale]");
        // exe-anchor walk: bin/msrtc-server under a tree with etc/server.yaml
        let exe = root.join("bin").join("msrtc-server");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, b"x").unwrap();
        let found = own_anchor_set(Some(&exe), Path::new("/"));
        assert!(found.contains(&root), "{found:?}");
    }

    #[test]
    fn unit_healthy_exit_semantics() {
        assert!(unit_healthy(Some("enabled"), Some("active"), true));
        assert!(!unit_healthy(Some("enabled"), Some("failed"), true), "daemon failed = exit 1");
        assert!(!unit_healthy(Some("enabled"), Some("active"), false), "zombie bin = exit 1");
        assert!(!unit_healthy(Some("disabled"), Some("active"), true));
        // systemctl did not answer -> treated as unhealthy (better to report than miss)
        assert!(!unit_healthy(None, None, true));
    }

    #[test]
    fn select_off_three_shapes() {
        assert_eq!(select_off(0, true), OffSelect::NoneInstalled);
        assert_eq!(select_off(0, false), OffSelect::NoneInstalled);
        assert_eq!(
            select_off(1, false),
            OffSelect::Single,
            "exactly one = stop directly (not refused even on non-tty)"
        );
        assert_eq!(select_off(2, true), OffSelect::Prompt);
        assert_eq!(select_off(3, false), OffSelect::RejectList);
    }

    // -- S3: post-install probe verdict (table-driven pure function; no real systemctl/sleep in tests) --

    #[test]
    fn systemctl_command_construction() {
        // thin exec layer command-surface pin: systemctl_enabled/active argv contract = ["--user", <verb>, unit]
        // (is-enabled/is-active in the same order; the is-active form has been consumed by the startup_status path for years).
        let unit = startup_unit_name(Path::new("/opt/msr"));
        for verb in ["is-enabled", "is-active"] {
            let argv = ["--user", verb, unit.as_str()];
            assert_eq!(argv[0], "--user");
            assert_eq!(argv[1], verb);
            assert_eq!(argv[2], unit);
        }
    }
}
