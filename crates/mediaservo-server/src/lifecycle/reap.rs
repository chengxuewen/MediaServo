//! stop fallback-sweep pure-function group (server-stop-global-reap T1, PLAN S2.1/S2.2/S2.5).
//!
//! Duty boundary (S2.5): **judgment and planning**, zero IO -- /proc scanning, environ reads, signals,
//! oxmgr calls all live in the T2/T3 executor thin shells. Cross-user (kill EPERM) and daemon-already-dead degradation
//! are runtime concerns, not expressed in the pure plan.
//!
//! Contract (`reap_plan`): the caller first resolves "which daemon owns which dir's registry snapshot"
//! and passes (daemon_data_dir, entries) pairs; pid first, miss falls back to cwd ownership
//! (same method as `inspect::strays_under`); both missing = orphan direct kill.
//!
//! T2 scan/execute thin shells (bottom of this file): `scan_live_procs` fills a LiveProc per pid
//! (argv NUL split / exe-cwd readlink / environ reverse lookup of OXMGR_HOME|OXMGR_DATA_DIR);
//! unreadable entries are skipped silently (cross-user), self pid excluded; judgment stays in the pure functions.

use std::path::{Path, PathBuf};

use super::inspect::ManagedEntry;
// -- Process identity signature (S2.1; the only ground truth = /proc; this function only judges) --

/// Live process snapshot (what the T2 scan layer fills; T1 defines the shape + test constructors).
#[derive(Debug, Clone)]
pub(super) struct LiveProc {
    pub pid: u32,
    pub argv: Vec<String>,
    pub exe: Option<String>,
    pub cwd: Option<PathBuf>,
    /// The OXMGR_HOME/OXMGR_DATA_DIR value in /proc/<pid>/environ -- Some = under an instance daemon.
    pub environ_oxmgr_home: Option<PathBuf>,
}

impl LiveProc {
    #[cfg(test)]
    fn new(pid: u32, argv: &[&str]) -> Self {
        Self {
            pid,
            argv: argv.iter().map(|s| s.to_string()).collect(),
            exe: None,
            cwd: None,
            environ_oxmgr_home: None,
        }
    }
}

/// Process identity three-way classification (signature output; caddy's disk marker is a separate IO step, see `caddy_marker_ok`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ProcKind {
    Server,
    Weaknet,
    /// `config` = the value of `--config` in argv (the instance root derivation source, S2.1 m3).
    Caddy {
        config: PathBuf,
    },
}

fn basename(p: &str) -> Option<&str> {
    Path::new(p).file_name().and_then(|s| s.to_str())
}

/// exe path contains `target/` as a path **component** (not a substring) -> a cargo test/dev binary, excluded (the Q1 ruling).
fn exe_is_target(exe: &str) -> bool {
    Path::new(exe).components().any(|c| c.as_os_str() == "target")
}

/// argv signature judgment (pure, no IO). Empty argv / not this product's name / argv[1] not run|serve / exe
/// with a target/ component -> None. caddy candidates are produced here (including the --config value); the disk marker
/// is verified by the caller via `caddy_marker_ok` -- IO and pure judgment are cut apart here; both sides are unit-testable without /proc.
pub(super) fn proc_kind(argv: &[String], exe: Option<&str>) -> Option<ProcKind> {
    if let Some(e) = exe
        && exe_is_target(e)
    {
        return None;
    }
    let argv0 = basename(argv.first()?)?;
    let sub = argv.get(1)?;
    if SERVER_NAMES.contains(&argv0) && sub == "run" {
        return Some(ProcKind::Server);
    }
    if WEAKNET_NAMES.contains(&argv0) && sub == "serve" {
        return Some(ProcKind::Weaknet);
    }
    if basename(exe?) == Some("caddy") {
        let mut it = argv.iter().skip(1);
        while let Some(a) = it.next() {
            if a == "--config"
                && let Some(v) = it.next()
                && v.ends_with("/etc/Caddyfile")
            {
                return Some(ProcKind::Caddy { config: PathBuf::from(v) });
            }
        }
    }
    None
}

/// caddy disk marker (B1, the only IO facet): recognized only if `<root>/bin/<server-bin-name>` exists --
/// root = one level above the Caddyfile (etc/) (S2.1 second-review pin: config_path.parent().parent()).
/// Guards against mis-killing TalkServo-style isomorphic layouts. The bin name set is two literals (no brand read: test processes and deployed
/// processes may be compiled with different brands; the marker predicate must be process-independent -- anchored live this session).
pub(super) fn caddy_marker_ok(config: &Path) -> bool {
    let Some(root) = config.parent().and_then(Path::parent) else { return false };
    SERVER_NAMES.iter().any(|n| root.join("bin").join(n).is_file())
}

const SERVER_NAMES: &[&str] = &["msrtc-server", "mediaservo-server"];
const WEAKNET_NAMES: &[&str] = &["msrtc-weaknet", "mediaservo-weaknet"];

// -- Reap plan (S2.2 pure shape: via which daemon / direct kill) -------------------------------

/// A single process's reap step. `ViaDaemon.daemon_data_dir` = the target daemon's OXMGR_DATA_DIR
/// (the executor sets env and calls oxmgr stop+delete from it); direct kill remains only for NoRegistry -- the daemon-dead
/// degradation is a runtime concern, not part of the pure plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ReapStep {
    ViaDaemon { daemon_data_dir: PathBuf, name: String, pid: u32 },
    DirectKill { pid: u32, reason: DirectReason },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum DirectReason {
    /// Both instance/default tables miss on pid+cwd (orphan / nohup direct start).
    NoRegistry,
}

/// The reap plan (pure function). **Contract**: `registries` is pre-resolved by the caller into (daemon_data_dir,
/// registry snapshot read under that dir) pairs -- this function does no disk reads and no default_data_dir()
/// env resolution. Match rule: exact pid match first; on miss, cwd-ownership fallback (entry cwd absolutized,
/// then starts_with against the process cwd, same method as `inspect::strays_under`, M2: the stale-pid shape after a daemon restart).
/// Both lookups miss -> `DirectKill(NoRegistry)`.
pub(super) fn reap_plan(
    procs: &[LiveProc],
    registries: &[(PathBuf, &[ManagedEntry])],
) -> Vec<ReapStep> {
    procs
        .iter()
        .map(|p| {
            // environ attribution (the /proc ground truth): the process claims membership in some daemon -> that table first (prevents the default
            // table's cwd fallback from swallowing an instance-daemon-owned process -- the overlapping-cwd shape of the two tables).
            let ordered: Vec<&(PathBuf, &[ManagedEntry])> = match &p.environ_oxmgr_home {
                Some(home) => {
                    let mut v: Vec<_> = registries.iter().collect();
                    v.sort_by_key(|(d, _)| !matches_home(d, home));
                    v
                }
                None => registries.iter().collect(),
            };
            for (dir, entries) in ordered {
                if let Some(e) = lookup(entries, p) {
                    return ReapStep::ViaDaemon {
                        daemon_data_dir: (*dir).clone(),
                        name: e.name.clone(),
                        pid: p.pid,
                    };
                }
            }
            ReapStep::DirectKill { pid: p.pid, reason: DirectReason::NoRegistry }
        })
        .collect()
}

/// environ home vs registry dir identity (compared after absolute normalization).
fn matches_home(dir: &Path, home: &Path) -> bool {
    super::absolute(dir) == super::absolute(home)
}

/// Registry reverse lookup: pid first, cwd ownership on miss (both sides absolutized, component-level prefix).
fn lookup<'a>(entries: &'a [ManagedEntry], p: &LiveProc) -> Option<&'a ManagedEntry> {
    if entries.iter().any(|e| e.pid == Some(p.pid as u64)) {
        return entries.iter().find(|e| e.pid == Some(p.pid as u64));
    }
    let Some(cwd) = &p.cwd else { return None };
    entries.iter().find(|e| e.cwd.starts_with(cwd))
}

// -- T3: -g global selection + tally counts (pure judgment; the marker predicate is injected to keep the IO seam) --

/// Single-step execution outcome (produced by the executor, consumed by `tally`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StepOutcome {
    /// stop+delete via the owning daemon succeeded.
    ViaDaemon,
    /// Killed directly and exit observed (including the daemon-unreachable degraded direct kill).
    DirectKill,
    /// Skipped by the direct-kill gate (daemon alive/entry present -- killing it would revive; report only, never overstep).
    /// Not a failure: not counted in failed, does not affect RC (a clean final state = RC 0).
    Skipped,
    /// Kill signal sent but the process still lives (EPERM/D-state etc.).
    Failed,
}

/// `-g` global selection (PLAN S2.4, pure judgment): a full /proc snapshot -> signature hits, **no dir filter,
/// no cwd assumption**; caddy candidates must pass the disk-marker predicate (B1 -- generic matching was vetoed; the TalkServo
/// isomorphic layout is on record). Returns (selected, marker-rejected caddy candidates) -- the rejection list is for printing explanations, not for the kill set.
pub(super) fn select_global(
    procs: &[LiveProc],
    caddy_ok: impl Fn(&Path) -> bool,
) -> (Vec<LiveProc>, Vec<LiveProc>) {
    let mut selected = Vec::new();
    let mut rejected = Vec::new();
    for p in procs {
        match proc_kind(&p.argv, p.exe.as_deref()) {
            Some(ProcKind::Caddy { config }) if caddy_ok(&config) => selected.push(p.clone()),
            Some(ProcKind::Caddy { .. }) => rejected.push(p.clone()),
            Some(_) => selected.push(p.clone()),
            None => {}
        }
    }
    (selected, rejected)
}

/// `-g` tally counts (pure function): returns (via daemon N, direct kill M, skipped S, failed K).
pub(super) fn tally(outcomes: &[StepOutcome]) -> (usize, usize, usize, usize) {
    (
        outcomes.iter().filter(|o| **o == StepOutcome::ViaDaemon).count(),
        outcomes.iter().filter(|o| **o == StepOutcome::DirectKill).count(),
        outcomes.iter().filter(|o| **o == StepOutcome::Skipped).count(),
        outcomes.iter().filter(|o| **o == StepOutcome::Failed).count(),
    )
}

// -- T2 scan thin shell (the IO face; all judgment in the pure functions above) ------------------

/// Full /proc scan -> live process snapshots (argv NUL split, exe/cwd readlink, environ reverse lookup).
/// Unreadable entries skipped silently (cross-user); self pid excluded (stop never self-harms). Non-Linux -> empty.
pub(super) fn scan_live_procs() -> Vec<LiveProc> {
    #[cfg(target_os = "linux")]
    {
        let mut out = Vec::new();
        let Ok(dir) = std::fs::read_dir("/proc") else { return out };
        for e in dir.flatten() {
            let Ok(pid) = e.file_name().to_string_lossy().parse::<u32>() else { continue };
            if pid == std::process::id() {
                continue;
            }
            if let Some(p) = fill_from_proc(pid) {
                out.push(p);
            }
        }
        out
    }
    #[cfg(not(target_os = "linux"))]
    {
        Vec::new()
    }
}

/// Fill one pid's snapshot. cmdline unreadable (cross-user/kernel thread) -> None, skipped silently.
fn fill_from_proc(pid: u32) -> Option<LiveProc> {
    #[cfg(target_os = "linux")]
    {
        let argv: Vec<String> = std::fs::read_to_string(format!("/proc/{pid}/cmdline"))
            .ok()?
            .split('\0')
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect();
        if argv.is_empty() {
            return None;
        }
        let exe = std::fs::read_link(format!("/proc/{pid}/exe"))
            .ok()
            .map(|p| p.to_string_lossy().into_owned());
        let cwd = std::fs::read_link(format!("/proc/{pid}/cwd")).ok();
        let environ_oxmgr_home = read_oxmgr_home(pid);
        Some(LiveProc { pid, argv, exe, cwd, environ_oxmgr_home })
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid;
        None
    }
}

/// The OXMGR_HOME / OXMGR_DATA_DIR value in /proc/<pid>/environ (the first present).
fn read_oxmgr_home(pid: u32) -> Option<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        let env = std::fs::read_to_string(format!("/proc/{pid}/environ")).ok()?;
        env.split('\0').find_map(|kv| kv.strip_prefix("OXMGR_HOME=").map(PathBuf::from)).or_else(
            || env.split('\0').find_map(|kv| kv.strip_prefix("OXMGR_DATA_DIR=").map(PathBuf::from)),
        )
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid;
        None
    }
}

/// Instance root derivation + ownership (dir-ownership comparisons always go through `absolute` normalization, S2.1/second-review pin 3).
/// server/caddy per `--config` (root = one level above etc/ = parent().parent()); weaknet per
/// `--token-file <root>/run/weaknet.token`. Underivable (missing args) = not owned.
pub(super) fn kind_dir_within(kind: &ProcKind, argv: &[String], mine: &Path) -> bool {
    let root = match kind {
        ProcKind::Server => {
            config_flag_value(argv, "--config").and_then(|c| derive_root_from_config(Path::new(&c)))
        }
        // m3: weaknet derives from --token-file <root>/run/weaknet.token
        ProcKind::Weaknet => config_flag_value(argv, "--token-file")
            .and_then(|c| derive_root_from_config(Path::new(&c))),
        ProcKind::Caddy { config } => derive_root_from_config(config),
    };
    let Some(root) = root else { return false };
    let mine = super::absolute(mine);
    super::absolute(&root).starts_with(&mine)
}

/// `--token-file <root>/run/weaknet.token` -> root (run -> root is two hops, the token sits directly in run/
/// -- equivalent to the etc/ shape of server.yaml/Caddyfile); the server.yaml/Caddyfile shape -> two levels up.
fn derive_root_from_config(path: &Path) -> Option<PathBuf> {
    path.parent().and_then(Path::parent).map(Path::to_path_buf)
}

/// The value of `--flag` in argv (None if absent).
fn config_flag_value(argv: &[String], flag: &str) -> Option<String> {
    argv.iter().position(|a| a == flag).and_then(|i| argv.get(i + 1)).cloned()
}

#[cfg(test)]
mod tests {
    use super::super::absolute;
    use super::*;

    // ── proc_kind: server / weaknet ─────────────────────────────────────────

    #[test]
    fn server_signature_both_brands() {
        for name in SERVER_NAMES {
            let argv = vec![format!("/opt/x/bin/{name}"), "run".into()];
            assert_eq!(proc_kind(&argv, Some(&argv[0])), Some(ProcKind::Server), "{name}");
        }
    }

    #[test]
    fn weaknet_signature_both_brands() {
        for name in WEAKNET_NAMES {
            let argv = vec![format!("/opt/x/bin/{name}"), "serve".into()];
            assert_eq!(proc_kind(&argv, Some(&argv[0])), Some(ProcKind::Weaknet), "{name}");
        }
    }

    #[test]
    fn wrong_subcommand_is_none() {
        let a = vec!["/opt/x/bin/msrtc-server".to_string(), "serve".into()];
        assert_eq!(proc_kind(&a, None), None, "a server process using serve = no match");
        let b = vec!["/opt/x/bin/msrtc-weaknet".to_string(), "run".into()];
        assert_eq!(proc_kind(&b, None), None);
        // no argv[1]
        let c = vec!["/opt/x/bin/msrtc-server".to_string()];
        assert_eq!(proc_kind(&c, None), None);
    }

    #[test]
    fn non_server_products_are_none() {
        for name in ["msrtc-host", "mediaservo-host", "msrtc-agent", "msrtc-streamer", "oxmgr"] {
            let argv = vec![format!("/opt/x/bin/{name}"), "run".into()];
            assert_eq!(proc_kind(&argv, Some(&argv[0])), None, "{name} must not hit");
        }
    }

    #[test]
    fn empty_argv_is_none() {
        assert_eq!(proc_kind(&[], None), None);
    }

    #[test]
    fn trailing_slash_argv0_still_matches() {
        let argv = vec!["/opt/x/bin/msrtc-server/".to_string(), "run".into()];
        assert_eq!(proc_kind(&argv, None), Some(ProcKind::Server));
    }

    #[test]
    fn target_component_exe_excluded_but_lookalike_kept() {
        // Component shape: target/ appears in the path -> excluded (a cargo test binary)
        let argv = vec!["/home/u/ms/target/debug/msrtc-server".to_string(), "run".into()];
        assert_eq!(proc_kind(&argv, Some(argv[0].as_str())), None);
        // Look-alike substring: /opt/targets/bin is not a target/ component -> not excluded
        let lookalike = vec!["/opt/targets/bin/msrtc-server".to_string(), "run".into()];
        assert_eq!(proc_kind(&lookalike, Some(lookalike[0].as_str())), Some(ProcKind::Server));
    }

    // -- proc_kind: caddy (argv face) + disk marker (tempdir IO face) --

    #[test]
    fn caddy_argv_candidate_without_config_value_is_none() {
        let a = vec!["/usr/bin/caddy".to_string(), "run".into()];
        assert_eq!(proc_kind(&a, Some("/usr/bin/caddy")), None);
        // the config value does not end with /etc/Caddyfile
        let b = vec![
            "/usr/bin/caddy".to_string(),
            "run".into(),
            "--config".into(),
            "/x/etc/Caddyfile.dev".into(),
        ];
        assert_eq!(proc_kind(&b, Some("/usr/bin/caddy")), None);
    }

    #[test]
    fn caddy_marker_present_accepts_absent_rejects() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join("bin")).unwrap();
        let cfg = root.join("etc").join("Caddyfile");
        // marker missing (the TalkServo isomorphic-layout shape) -> argv hits but the marker rejects
        std::fs::create_dir_all(root.join("etc")).unwrap();
        assert!(!caddy_marker_ok(&cfg));
        let argv = vec![
            "/usr/bin/caddy".to_string(),
            "run".into(),
            "--config".into(),
            cfg.to_string_lossy().into_owned(),
        ];
        assert_eq!(
            proc_kind(&argv, Some("/usr/bin/caddy")),
            Some(ProcKind::Caddy { config: cfg.clone() })
        );
        // marker present (both the brand form and the upstream form recognized) -> accepted
        std::fs::write(root.join("bin").join("msrtc-server"), b"").unwrap();
        assert!(caddy_marker_ok(&cfg));
        std::fs::remove_file(root.join("bin").join("msrtc-server")).unwrap();
        std::fs::write(root.join("bin").join("mediaservo-server"), b"").unwrap();
        assert!(caddy_marker_ok(&cfg));
    }

    // -- reap_plan four shapes + M2 stale-pid-cwd fallback --

    fn entry(id: u64, name: &str, pid: Option<u64>, cwd: &str) -> ManagedEntry {
        ManagedEntry { id, name: name.into(), status: "running".into(), pid, cwd: cwd.into() }
    }

    #[test]
    fn instance_daemon_managed_routes_via_instance_daemon() {
        let p = LiveProc {
            environ_oxmgr_home: Some(PathBuf::from("/inst/run/oxmgr")),
            ..LiveProc::new(100, &["/x/msrtc-server", "run"])
        };
        let inst = vec![entry(1, "msrtc-server", Some(100), "/inst")];
        let plan = reap_plan(
            &[p],
            &[(PathBuf::from("/inst/run/oxmgr"), &inst), (PathBuf::from("/default"), &[])],
        );
        assert_eq!(
            plan,
            vec![ReapStep::ViaDaemon {
                daemon_data_dir: PathBuf::from("/inst/run/oxmgr"),
                name: "msrtc-server".into(),
                pid: 100
            }]
        );
    }

    #[test]
    fn default_daemon_managed_routes_via_default_daemon() {
        let p = LiveProc::new(200, &["/x/msrtc-server", "run"]); // no OXMGR_* in environ
        let default = vec![entry(2, "msrtc-server", Some(200), "/deploy/out/server")];
        let plan = reap_plan(&[p], &[(PathBuf::from("/home/u/.local/share/oxmgr"), &default)]);
        assert_eq!(
            plan,
            vec![ReapStep::ViaDaemon {
                daemon_data_dir: PathBuf::from("/home/u/.local/share/oxmgr"),
                name: "msrtc-server".into(),
                pid: 200
            }]
        );
    }

    #[test]
    fn orphan_double_miss_direct_kill() {
        let p = LiveProc::new(300, &["/x/msrtc-server", "run"]);
        let plan = reap_plan(&[p], &[(PathBuf::from("/d"), &[])]);
        assert_eq!(plan, vec![ReapStep::DirectKill { pid: 300, reason: DirectReason::NoRegistry }]);
    }

    #[test]
    fn stale_pid_falls_back_to_cwd_membership() {
        // M2: after a daemon crash-restart the registry pid is stale (!= the live pid); cwd ownership still routes via the daemon
        let p = LiveProc {
            cwd: Some(PathBuf::from("/deploy/out/server")),
            ..LiveProc::new(999, &["/x/msrtc-server", "run"])
        };
        let stale = vec![entry(3, "msrtc-server", Some(3419236), "/deploy/out/server")];
        let plan = reap_plan(&[p], &[(PathBuf::from("/d/oxmgr"), &stale)]);
        assert_eq!(
            plan,
            vec![ReapStep::ViaDaemon {
                daemon_data_dir: PathBuf::from("/d/oxmgr"),
                name: "msrtc-server".into(),
                pid: 999
            }]
        );
    }

    #[test]
    fn cwd_sibling_prefix_not_matched() {
        // /deploy/out/server-2 is not under /deploy/out/server (same lesson as strays_under)
        let p = LiveProc {
            cwd: Some(PathBuf::from("/deploy/out/server")),
            ..LiveProc::new(400, &["/x/msrtc-server", "run"])
        };
        let reg = vec![entry(4, "sibling", Some(1), "/deploy/out/server-2")];
        let plan = reap_plan(&[p], &[(PathBuf::from("/d"), &reg)]);
        assert_eq!(plan, vec![ReapStep::DirectKill { pid: 400, reason: DirectReason::NoRegistry }]);
    }

    // -- T2: kind_dir_within / derive_root_from_config (pure judgment; tempdir IO only in the marker) --

    #[test]
    fn kind_dir_within_server_and_weaknet_and_caddy() {
        let tmp = tempfile::tempdir().unwrap();
        let root = absolute(tmp.path());
        // server shape: --config <root>/etc/server.yaml
        let srv_argv = vec![
            format!("{}/bin/msrtc-server", root.display()),
            "run".into(),
            "--config".into(),
            format!("{}/etc/server.yaml", root.display()),
        ];
        assert!(kind_dir_within(&ProcKind::Server, &srv_argv, &root));
        assert!(!kind_dir_within(&ProcKind::Server, &srv_argv, Path::new("/elsewhere")));
        // weaknet shape: --token-file <root>/run/weaknet.token (the kind signature is serve)
        let wn_argv = vec![
            format!("{}/bin/msrtc-weaknet", root.display()),
            "serve".into(),
            "--listen".into(),
            "127.0.0.1:9810".into(),
            "--token-file".into(),
            format!("{}/run/weaknet.token", root.display()),
        ];
        assert!(kind_dir_within(&ProcKind::Weaknet, &wn_argv, &root));
        // weaknet missing --token-file -> underivable -> not owned (conservative)
        let wn_bare = vec![format!("{}/bin/msrtc-weaknet", root.display()), "serve".into()];
        assert!(!kind_dir_within(&ProcKind::Weaknet, &wn_bare, &root));
        // caddy shape: --config <root>/etc/Caddyfile -> root = two levels up
        let caddy =
            ProcKind::Caddy { config: PathBuf::from(format!("{}/etc/Caddyfile", root.display())) };
        assert!(kind_dir_within(&caddy, &[], &root));
        // sibling-dir shape (/root-2) is not misattributed
        assert!(!kind_dir_within(
            &caddy,
            &[],
            &root
                .parent()
                .unwrap()
                .join(format!("{}-2", root.file_name().unwrap().to_string_lossy()))
        ));
    }

    #[test]
    fn kind_dir_within_relative_proc_paths_normalized() {
        // S2.1 pin: cmdline relative path vs an absolute arg -> still owned after absolute normalization
        let tmp = tempfile::tempdir().unwrap();
        let root = absolute(tmp.path());
        let rel_argv =
            vec!["msrtc-server".into(), "run".into(), "--config".into(), "etc/server.yaml".into()];
        // the relative "etc/server.yaml" resolves against cwd -- absolute() carries no cwd semantics, so root derivation
        // cannot go two levels above "etc" -> not owned (conservative direction is correct, no mis-kill)
        assert!(!kind_dir_within(&ProcKind::Server, &rel_argv, &root));
    }

    #[test]
    fn reap_plan_environ_attribution_prefers_own_daemon() {
        // the /proc environ ground truth: the process claims /inst/run/oxmgr -> the instance table first (even if the default
        // table's pid/cwd also hit -- the overlapping-cwd shape must not be swallowed by the default table)
        let p = LiveProc {
            cwd: Some(PathBuf::from("/inst")),
            environ_oxmgr_home: Some(PathBuf::from("/inst/run/oxmgr")),
            ..LiveProc::new(500, &["/x/msrtc-server", "run"])
        };
        let default_tab = vec![entry(9, "msrtc-server", Some(500), "/inst")];
        let inst_tab = vec![entry(8, "msrtc-server", Some(500), "/inst")];
        let plan = reap_plan(
            &[p],
            &[
                (PathBuf::from("/home/u/.local/share/oxmgr"), &default_tab),
                (PathBuf::from("/inst/run/oxmgr"), &inst_tab),
            ],
        );
        assert_eq!(
            plan[0],
            ReapStep::ViaDaemon {
                daemon_data_dir: PathBuf::from("/inst/run/oxmgr"),
                name: "msrtc-server".into(),
                pid: 500
            }
        );
    }

    // -- T3: select_global / tally (fixture-tested, zero /proc) --

    #[test]
    fn select_global_keeps_ours_rejects_markerless_caddy() {
        // marked caddy (this product's layout shape)
        let tmp = tempfile::tempdir().unwrap();
        let root = absolute(tmp.path());
        std::fs::create_dir_all(root.join("bin")).unwrap();
        std::fs::write(root.join("bin").join("msrtc-server"), b"").unwrap();
        let with_marker = LiveProc {
            argv: vec![
                "/usr/bin/caddy".to_string(),
                "run".into(),
                "--config".into(),
                format!("{}/etc/Caddyfile", root.display()),
            ],
            exe: Some("/usr/bin/caddy".into()),
            ..LiveProc::new(11, &[])
        };
        // unmarked caddy (the TalkServo isomorphic-layout shape) -- a subdirectory of the same tempdir without a faked bin
        let talkservo = LiveProc {
            argv: vec![
                "/usr/bin/caddy".to_string(),
                "run".into(),
                "--config".into(),
                format!("{}/talk/etc/Caddyfile", root.display()),
            ],
            exe: Some("/usr/bin/caddy".into()),
            ..LiveProc::new(12, &[])
        };
        let ours_server = LiveProc::new(10, &["/opt/x/bin/msrtc-server", "run"]);
        let ours_weaknet = LiveProc::new(15, &["/opt/x/bin/msrtc-weaknet", "serve"]);
        // the cargo test shape (exe contains a target/ component) and the host family -- outside the -g kill set
        let target_exe = LiveProc {
            exe: Some("/home/u/ms/target/debug/msrtc-server".into()),
            ..LiveProc::new(13, &["/home/u/ms/target/debug/msrtc-server", "run"])
        };
        let host_proc = LiveProc::new(14, &["/opt/x/bin/msrtc-host", "run"]);
        let procs = vec![ours_server, with_marker, talkservo, target_exe, host_proc, ours_weaknet];
        let (sel, rej) = select_global(&procs, caddy_marker_ok);
        let pids: Vec<u32> = sel.iter().map(|p| p.pid).collect();
        assert_eq!(pids, vec![10, 11, 15], "-g kill set = server/weaknet/marked caddy");
        let rej_pids: Vec<u32> = rej.iter().map(|p| p.pid).collect();
        assert_eq!(
            rej_pids,
            vec![12],
            "the unmarked caddy rejection list (printed, not in the kill set)"
        );
    }

    #[test]
    fn tally_counts_four_outcomes() {
        let out = vec![
            StepOutcome::ViaDaemon,
            StepOutcome::DirectKill,
            StepOutcome::Failed,
            StepOutcome::ViaDaemon,
            StepOutcome::DirectKill,
            StepOutcome::Skipped,
            StepOutcome::Skipped,
        ];
        assert_eq!(tally(&out), (2, 2, 2, 1));
        assert_eq!(tally(&[]), (0, 0, 0, 0));
    }

    #[test]
    fn scan_fill_unreadable_pid_is_none() {
        // cross-user/kernel pid (pid 1 usually has readable cmdline but unreadable environ -- fill must not panic;
        // assert: fill an unreadable-by-construction pid (u32::MAX) -> None silently
        assert!(fill_from_proc(u32::MAX).is_none());
    }
}
