//! start conflict check (start-conflict-doctor T1) -- port occupant attribution.
//!
//! Data sources (Linux /proc, read-only):
//!   1. `/proc/net/tcp|tcp6|udp|udp6` reverse lookup of the socket inodes for `port`
//!   2. `/proc/<pid>/fd/*` scan for `socket:[inode]` -> set of occupying pids
//!   3. `/proc/pid/{exe,cmdline,status}` -> report body (exe containing "(deleted)" = hard evidence of upgrade residue)
//!
//! Principle (F1): **attribute, do not kill** -- produce a plain-language report only; process handling is left to the caller/human.
//! Non-Linux (/proc missing) -> empty report (caller keeps the existing degraded semantics, F6).

use std::collections::BTreeSet;
use std::path::PathBuf;

/// Health-check report for a single occupying process.
#[derive(Debug, Clone)]
pub struct ConflictReport {
    pub pid: u32,
    /// `readlink /proc/pid/exe` (may carry the kernel suffix "(deleted)").
    pub exe: String,
    /// First segment of cmdline (truncated to 80 chars).
    pub cmd: String,
    /// exe deleted = the binary was replaced while the process stayed alive (hard evidence of upgrade residue, F4).
    pub deleted: bool,
    /// (ppid, first segment of the parent's cmd) -- daemon ownership attribution.
    pub parent: Option<(u32, String)>,
}

impl ConflictReport {
    /// Plain-language line (for eprintln).
    pub fn line(&self) -> String {
        let mark = if self.deleted { " [!] upgrade residue (exe deleted)" } else { "" };
        let parent = match &self.parent {
            Some((ppid, pcmd)) => format!(", parent={pcmd}({ppid})"),
            None => String::new(),
        };
        format!("pid {}  {}{mark}  cmd={}{}", self.pid, self.exe, self.cmd, parent)
    }
}

/// Diagnose `port` (tcp+udp, IPv4+IPv6). Unoccupied/non-Linux -> empty Vec.
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

/// Socket inodes in `/proc/net/{tcp,tcp6,udp,udp6}` whose local port == `port`.
fn port_inodes(port: u16) -> BTreeSet<u64> {
    let mut inodes = BTreeSet::new();
    for f in ["tcp", "tcp6", "udp", "udp6"] {
        let Ok(text) = std::fs::read_to_string(format!("/proc/net/{f}")) else { continue };
        for line in text.lines().skip(1) {
            // Format: sl local_address rem_address st ... inode
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

/// stop self-verification (T4): have the family processes for app name gone to zero.
/// Going through oxmgr ps semantics is too heavy -- lightweight form: a process counts as remaining if its cmdline contains
/// `msrtc-<app>` or `<app>` and its exe is not this test process. None found = zero.
/// (Since global-reap T2 the stop main path uses B2 ownership-attribution self-verification; no other caller today --
/// keep `#[allow(dead_code)]` for one observation cycle, then delete.)
#[allow(dead_code)]
pub fn diagnose_app_gone(app: &str) -> bool {
    let needles = [format!("msrtc-{app}"), format!("mediaservo-{app}"), app.to_string()];
    for pid in proc_pids() {
        if pid == std::process::id() {
            continue;
        }
        let Ok(cmd) = std::fs::read_to_string(format!("/proc/{pid}/cmdline")) else { continue };
        let first = cmd.split('\0').next().unwrap_or("");
        if needles.iter().any(|n| first.contains(n.as_str())) {
            return false; // remaining processes
        }
    }
    true
}

/// Plain-language report output (eprintln; offers handling advice when an occupant carries upgrade residue, F1 attribute-don't-kill).
pub fn print_report(reports: &[ConflictReport], port: u16) {
    if reports.is_empty() {
        return;
    }
    eprintln!("port {port} is occupied -- conflict check:");
    for r in reports {
        eprintln!("  {}", r.line());
        if r.deleted {
            eprintln!(
                "  -> this process is running a deleted old binary (upgrade residue); kill {} and rerun start",
                r.pid
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Unoccupied port -> empty report (quick test; picks a high cold port, collision probability negligible).
    #[test]
    fn unoccupied_port_yields_empty() {
        // Find a port that is truly not listened on: temporary bind then drop, diagnose immediately
        let l = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = l.local_addr().unwrap().port();
        drop(l);
        assert!(diagnose_port(port).is_empty(), "no occupant after drop");
    }

    /// This test process occupies a port itself (TCP+UDP) -> exactly 1 report with pid=self, deleted=false.
    #[test]
    fn self_occupied_port_reports_self() {
        let l = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let u = std::net::UdpSocket::bind(("127.0.0.1", 0)).unwrap();
        let tcp_port = l.local_addr().unwrap().port();
        let udp_port = u.local_addr().unwrap().port();
        let tcp = diagnose_port(tcp_port);
        assert_eq!(tcp.len(), 1, "tcp self-occupancy should yield exactly 1 entry: {tcp:?}");
        assert_eq!(tcp[0].pid, std::process::id());
        assert!(!tcp[0].deleted);
        assert!(
            tcp[0].cmd.contains("conflict")
                || tcp[0].cmd.contains("mediaservo")
                || !tcp[0].cmd.is_empty()
        );
        let udp = diagnose_port(udp_port);
        assert_eq!(udp.len(), 1, "udp self-occupancy should yield exactly 1 entry: {udp:?}");
        assert_eq!(udp[0].pid, std::process::id());
    }

    /// Plain-language line format pin (deleted marker and advice wording).
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
                && line.contains("upgrade residue")
        );
        assert!(line.contains("oxmgr daemon run(7)"));
    }
}
