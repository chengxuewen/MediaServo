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
        let fake = dir.join("msrtc-agent");
        std::fs::copy("/bin/sleep", &fake).unwrap();
        let mut child = std::process::Command::new(&fake).arg("30").spawn().expect("spawn fixture");
        let gone = diagnose_app_gone("msrtc-agent");
        let _ = child.kill();
        let _ = child.wait();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(!gone, "branded survivor must count as leftover (found => gone=false)");
    }
}
