//! start conflict doctoring (start-conflict-doctor T3 -- host-side mirror (same shape as the server conflict.rs, F3 does not abstract it).
//!
//! Data sources (Linux /proc, read-only):
//!   (1) reverse-lookup the socket inode of `port` from `/proc/net/tcp|tcp6|udp|udp6`
//!   (2) scan `/proc/<pid>/fd/*` for `socket:[inode]` -> the set of occupying pids
//!   (3) `/proc/pid/{exe,cmdline,status}` -> the report body (exe containing "(deleted)" = hard evidence of an upgrade leftover)
//!
//! Principle (F1): **name it, do not kill it** -- only produce a human-readable report; disposition is left to the caller/person.
//! Non-Linux (/proc missing) -> empty report (callers keep the current degraded semantics, F6).

use std::collections::BTreeSet;
use std::path::PathBuf;

/// Doctoring report for a single occupying process.
#[derive(Debug, Clone)]
pub struct ConflictReport {
    pub pid: u32,
    /// `readlink /proc/pid/exe` (may carry the kernel suffix "(deleted)").
    pub exe: String,
    /// First segment of cmdline (truncated to 80 chars).
    pub cmd: String,
    /// exe deleted = the process survived a binary replacement (hard evidence of an upgrade leftover, F4).
    pub deleted: bool,
    /// (ppid, first segment of the parent process cmd) -- daemon ownership identification.
    pub parent: Option<(u32, String)>,
}

impl ConflictReport {
    /// Human-readable line (for eprintln).
    pub fn line(&self) -> String {
        let mark = if self.deleted { " ⚠ upgrade leftover (exe deleted)" } else { "" };
        let parent = match &self.parent {
            Some((ppid, pcmd)) => format!(", parent={pcmd}({ppid})"),
            None => String::new(),
        };
        format!("pid {}  {}{mark}  cmd={}{}", self.pid, self.exe, self.cmd, parent)
    }
}

/// Diagnose `port` (tcp+udp both, IPv4+IPv6). Unoccupied / non-Linux -> empty Vec.
pub fn diagnose_port(port: u16) -> Vec<ConflictReport> {
    let inodes = port_inodes(port);
    if inodes.is_empty() {
        return Vec::new();
    }
    let mut out: BTreeSet<u32> = BTreeSet::new();
    for pid in proc_pids() {
        if proc_pid_owns_inodes(pid, &inodes) {
            out.insert(pid);
        }
    }
    out.into_iter().filter_map(report_of_pid).collect()
}

/// The set of socket inodes in `/proc/net/{tcp,tcp6,udp,udp6}` whose local port == `port`.
fn port_inodes(port: u16) -> BTreeSet<u64> {
    let mut inodes = BTreeSet::new();
    for f in ["tcp", "tcp6", "udp", "udp6"] {
        let Ok(text) = std::fs::read_to_string(format!("/proc/net/{f}")) else { continue };
        for line in text.lines().skip(1) {
            // format: sl local_address rem_address st ... inode
            let cols: Vec<&str> = line.split_whitespace().collect();
            if cols.len() < 10 {
                continue;
            }
            // local_address = "ADDR:PORT" hex
            let local = cols[1];
            let Some((_, p)) = local.rsplit_once(':') else { continue };
            if u16::from_str_radix(p, 16) != Ok(port) {
                continue;
            }
            if let Ok(ino) = cols[9].parse::<u64>() {
                inodes.insert(ino);
            }
        }
    }
    inodes
}

fn proc_pids() -> Vec<u32> {
    let mut out = Vec::new();
    let Ok(dir) = std::fs::read_dir("/proc") else { return out };
    for e in dir.flatten() {
        if let Ok(pid) = e.file_name().to_string_lossy().parse::<u32>() {
            out.push(pid);
        }
    }
    out
}

fn proc_pid_owns_inodes(pid: u32, inodes: &BTreeSet<u64>) -> bool {
    let fd_dir = PathBuf::from(format!("/proc/{pid}/fd"));
    let Ok(fds) = std::fs::read_dir(&fd_dir) else { return false };
    for fd in fds.flatten() {
        let Ok(target) = std::fs::read_link(fd.path()) else { continue };
        let t = target.to_string_lossy();
        if let Some(rest) = t.strip_prefix("socket:[")
            && let Ok(ino) = rest.trim_end_matches(']').parse::<u64>()
            && inodes.contains(&ino)
        {
            return true;
        }
    }
    false
}

fn report_of_pid(pid: u32) -> Option<ConflictReport> {
    let exe = std::fs::read_link(format!("/proc/{pid}/exe")).ok()?.to_string_lossy().into_owned();
    let cmd = std::fs::read_to_string(format!("/proc/{pid}/cmdline"))
        .unwrap_or_default()
        .split('\0')
        .next()
        .unwrap_or("")
        .chars()
        .take(80)
        .collect::<String>();
    let deleted = exe.contains("(deleted)");
    let ppid = std::fs::read_to_string(format!("/proc/{pid}/status")).ok().and_then(|s| {
        s.lines().find_map(|l| l.strip_prefix("PPid:")).and_then(|v| v.trim().parse::<u32>().ok())
    });
    let parent = ppid.and_then(|pp| {
        let cmd = std::fs::read_to_string(format!("/proc/{pp}/cmdline"))
            .unwrap_or_default()
            .split('\0')
            .next()
            .unwrap_or("")
            .chars()
            .take(60)
            .collect::<String>();
        (!cmd.is_empty()).then_some((pp, cmd))
    });
    Some(ConflictReport { pid, exe, cmd, deleted, parent })
}

/// stop self-verification (T4): whether the processes of this family for the app name have reached zero.
/// Relying on oxmgr ps semantics would be too heavy -- lightweight form: anything in the process table whose cmdline contains `msrtc-<app>` or
/// `<app>` and whose exe is not this test process counts as a leftover. Nothing found = zeroed.
pub fn diagnose_app_gone(app: &str) -> bool {
    let needles = [format!("msrtc-{app}"), format!("mediaservo-{app}"), app.to_string()];
    for pid in proc_pids() {
        if pid == std::process::id() {
            continue;
        }
        let Ok(cmd) = std::fs::read_to_string(format!("/proc/{pid}/cmdline")) else { continue };
        let first = cmd.split('\0').next().unwrap_or("");
        if needles.iter().any(|n| first.contains(n.as_str())) {
            return false; // leftovers present
        }
    }
    true
}

/// k0 (host-ops-hardening F15): brand-aware family name list for the cmd_stop
/// self-proof. The old hardcoded `host-*` list was vacuously true on branded
/// trees (processes run as `msrtc-agent` — no needle matched). Pass brand-derived
/// names PLUS the legacy `host-` forms so current-brand, renamed-legacy, and
/// default-brand (CI) deployments are all covered; `diagnose_app_gone` layers its
/// own prefix variants per supplied name.
pub fn stop_sweep_names(app_prefix: &str) -> Vec<String> {
    const BASES: [&str; 7] =
        ["agent", "streamer", "capturer", "recorder", "controller", "emergency", "audio"];
    BASES.iter().flat_map(|b| [format!("{app_prefix}{b}"), format!("host-{b}")]).collect()
}

/// T1-k10 (host-ops-hardening R16): a family process that outlived cmd_stop's sweep.
#[derive(Debug, Clone)]
pub struct SurvivorInfo {
    pub pid: u32,
    /// Cmdline first segment (full path; comm-truncation-safe per PIT-15).
    pub first_arg: String,
    /// OXMGR_HOME from the process environ (same-uid readable), else "owner unknown".
    pub owner: String,
}

/// Pure name predicate: basename of the cmdline first segment starts with ANY family
/// name (covers instance names like msrtc-capturer-cam0); dev/test binaries under a
/// `/target/` path component never match. Self-pid exclusion is caller-side.
fn survivor_match(first_arg: &str, family: &[String]) -> bool {
    if first_arg.contains("/target/") {
        return false;
    }
    let base = first_arg.rsplit('/').next().unwrap_or(first_arg);
    family.iter().any(|n| base.starts_with(n.as_str()))
}

/// Extract OXMGR_HOME from a NUL-separated /proc/<pid>/environ blob (KEY=VALUE entries).
fn parse_oxmgr_home(environ: &[u8]) -> Option<String> {
    environ.split(|b| *b == 0).find_map(|kv| {
        let v = kv.strip_prefix(b"OXMGR_HOME=")?;
        (!v.is_empty()).then(|| String::from_utf8_lossy(v).into_owned())
    })
}

/// Post-stop ground truth (report-only, NEVER kills — F1 name-don-t-kill): anything in
/// the process table whose cmdline first segment hits the brand-aware family set after
/// the sweep was "verified". Closes the branded-sweep false-green (W1 evidence round).
/// `_dir` is kept for the instance-scoping follow-up (batch spec signature); the current
/// check is namespace-wide via the family set + target/ exclusion.
pub fn stop_survivors(_dir: &std::path::Path) -> Vec<SurvivorInfo> {
    let family = stop_sweep_names(mediaservo_common::brand::media_brand().app_prefix);
    let mut out = Vec::new();
    for pid in proc_pids() {
        if pid == std::process::id() {
            continue;
        }
        let Ok(cmd) = std::fs::read_to_string(format!("/proc/{pid}/cmdline")) else {
            continue;
        };
        let first = cmd.split('\0').next().unwrap_or("");
        if !survivor_match(first, &family) {
            continue;
        }
        // exe under cargo target/ = a dev/test binary, not an instance survivor
        let exe = std::fs::read_link(format!("/proc/{pid}/exe"))
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        if exe.contains("/target/") {
            continue;
        }
        let owner = std::fs::read(format!("/proc/{pid}/environ"))
            .ok()
            .and_then(|env| parse_oxmgr_home(&env))
            .unwrap_or_else(|| "owner unknown".to_string());
        out.push(SurvivorInfo { pid, first_arg: first.to_string(), owner });
    }
    out
}

/// Human-readable report output (eprintln; gives a disposition suggestion when an occupier is an upgrade leftover, F1 name-don-t-kill).
pub fn print_report(reports: &[ConflictReport], port: u16) {
    if reports.is_empty() {
        return;
    }
    eprintln!("port {port} is occupied -- conflict doctoring:");
    for r in reports {
        eprintln!("  {}", r.line());
        if r.deleted {
            eprintln!(
                "  -> this process runs a deleted old binary (upgrade leftover); rerun start after kill {}",
                r.pid
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Unoccupied port -> empty report (fast test; a high cold port is chosen, so collision odds are negligible).
    #[test]
    fn unoccupied_port_yields_empty() {
        // find a port that really is not listening: bind temporarily, drop, then diagnose immediately
        let l = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = l.local_addr().unwrap().port();
        drop(l);
        assert!(diagnose_port(port).is_empty(), "no occupier after drop");
    }

    /// This test process occupying its own port (TCP+UDP) -> exactly 1 report with pid=self, deleted=false.
    #[test]
    fn self_occupied_port_reports_self() {
        let l = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let u = std::net::UdpSocket::bind(("127.0.0.1", 0)).unwrap();
        let tcp_port = l.local_addr().unwrap().port();
        let udp_port = u.local_addr().unwrap().port();
        let tcp = diagnose_port(tcp_port);
        assert_eq!(tcp.len(), 1, "tcp self-occupancy should be exactly 1: {tcp:?}");
        assert_eq!(tcp[0].pid, std::process::id());
        assert!(!tcp[0].deleted);
        assert!(
            tcp[0].cmd.contains("conflict")
                || tcp[0].cmd.contains("mediaservo")
                || !tcp[0].cmd.is_empty()
        );
        let udp = diagnose_port(udp_port);
        assert_eq!(udp.len(), 1, "udp self-occupancy should be exactly 1: {udp:?}");
        assert_eq!(udp[0].pid, std::process::id());
    }

    /// Human-readable line format pin (deleted marker and suggestion wording).
    #[test]
    fn report_line_marks_deleted() {
        let r = ConflictReport {
            pid: 42,
            exe: "/old/path/mediaservo-server (deleted)".into(),
            cmd: "mediaservo-server run".into(),
            deleted: true,
            parent: Some((7, "oxmgr daemon run".into())),
        };
        let line = r.line();
        assert!(
            line.contains("pid 42")
                && line.contains("(deleted)")
                && line.contains("upgrade leftover")
        );
        assert!(line.contains("oxmgr daemon run(7)"));
    }

    /// k0 pin: branded prefix yields brand AND legacy forms (no single-shape list).
    #[test]
    fn stop_sweep_names_cover_both_families() {
        let names = stop_sweep_names("msrtc-");
        assert!(names.contains(&"msrtc-agent".to_string()));
        assert!(names.contains(&"host-agent".to_string()));
        assert!(names.contains(&"msrtc-capturer".to_string()));
        let legacy = stop_sweep_names("host-");
        assert!(legacy.iter().any(|n| n == "host-agent"));
    }

    /// F15 negative fixture: a live branded-named process MUST be detected — the
    /// old hardcoded list reported "sweep verified" regardless on branded boxes.
    /// (Linux-only: /proc scan.)
    #[cfg(target_os = "linux")]
    #[test]
    fn branded_survivor_is_not_vacuously_gone() {
        let dir = std::env::temp_dir().join(format!("k0-sweep-fixture-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // unique exec-mapped filename: a same-path copy right after a previous run's exiting
        // process ETXTBSYs (seen live 1/15 — stale sleep still mapped); suffixes are prefix-safe
        // for the family match.
        let fake = dir.join(format!("msrtc-agent-k0-{}", std::process::id()));
        std::fs::copy("/bin/sleep", &fake).unwrap();
        let mut child = std::process::Command::new(&fake).arg("30").spawn().expect("spawn fixture");
        // spawn→/proc visibility is not synchronous — poll the very predicate we assert on
        // (the k10 sibling fixture shares the needle basename; the asserted scan must be
        // the thing we waited on, not a re-scan after an exact-path poll).
        let mut gone = true;
        for _ in 0..100 {
            gone = diagnose_app_gone("msrtc-agent");
            if !gone {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let _ = child.kill();
        let _ = child.wait();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(!gone, "branded survivor must count as leftover (found => gone=false)");
    }

    /// k10 pin: brand basename hits (branded, legacy, instance name) and misses;
    /// /target/ component never matches (dev binaries are not survivors).
    #[test]
    fn survivor_match_covers_brand_family() {
        let family = stop_sweep_names("msrtc-");
        assert!(survivor_match("/opt/x/bin/msrtc-agent", &family));
        assert!(survivor_match("msrtc-capturer-cam0", &family), "instance names count");
        assert!(survivor_match("/opt/x/bin/host-agent", &family), "legacy form counts");
        assert!(survivor_match("/opt/x/bin/host-recorder", &family));
        assert!(!survivor_match("/usr/bin/nginx", &family));
        assert!(!survivor_match("/repo/target/debug/msrtc-agent", &family), "target/ excluded");
        assert!(!survivor_match("/repo/target/debug/oxmgr", &family));
    }

    /// k10 pin: OXMGR_HOME parsed from NUL-separated environ; absent/empty key -> None.
    #[test]
    fn parse_oxmgr_home_reads_environ() {
        let env = b"PATH=/usr/bin\0OXMGR_HOME=/srv/host/run/oxmgr\0RUST_LOG=info\0";
        assert_eq!(parse_oxmgr_home(env).as_deref(), Some("/srv/host/run/oxmgr"));
        assert_eq!(parse_oxmgr_home(b"PATH=/usr/bin\0HOME=/root\0"), None);
        assert_eq!(parse_oxmgr_home(b"OXMGR_HOME=\0"), None, "empty value is not an owner");
    }

    /// k10 Linux fixture: a live process named after the CURRENT brand's agent must be
    /// reported with owner unknown (spawned without oxmgr env) and vanish after kill.
    /// (Name derives from the process's own brand: media_brand() is OnceLock-cached, so
    /// a hardcoded "msrtc-" name would be red on the default-brand test box.)
    #[cfg(target_os = "linux")]
    #[test]
    fn stop_survivors_finds_and_loses_live_fixture() {
        let branded = format!("{}agent", mediaservo_common::brand::media_brand().app_prefix);
        let dir = std::env::temp_dir().join(format!("k10-survivor-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // unique exec-mapped filename (ETXTBSY defense, same as the k0 fixture above)
        let fake = dir.join(format!("{branded}-k10-{}", std::process::id()));
        std::fs::copy("/bin/sleep", &fake).unwrap();
        let mut child =
            std::process::Command::new(&fake).arg("30").spawn().expect("spawn k10 fixture");
        // spawn→/proc visibility is not synchronous; poll until our own fixture shows up
        // before asserting (pre-existing flake, 4/15 red on clean HEAD when run parallel).
        let mut found = Vec::new();
        let mut hit: Option<SurvivorInfo> = None;
        for _ in 0..100 {
            found = stop_survivors(&dir);
            if let Some(h) = found.iter().find(|s| s.first_arg == fake.to_string_lossy()) {
                hit = Some(h.clone());
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let _ = child.kill();
        let _ = child.wait();
        let _ = std::fs::remove_dir_all(&dir);
        let hit = hit.unwrap_or_else(|| {
            panic!("fixture {branded} must be reported within the poll window: {found:?}")
        });
        assert_eq!(hit.owner, "owner unknown", "no OXMGR_HOME in the fixture environ");
        assert!(stop_survivors(&dir).iter().all(|s| s.pid != hit.pid), "killed pid must be gone");
    }
}
