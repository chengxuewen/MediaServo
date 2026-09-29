//! start 冲突体检（start-conflict-doctor T1）——端口占用者指认。
//!
//! 数据源（Linux /proc，纯只读）：
//!   ① `/proc/net/tcp|tcp6|udp|udp6` 反查 `port` 的 socket inode
//!   ② `/proc/<pid>/fd/*` 扫 `socket:[inode]` → 占用 pid 集
//!   ③ `/proc/pid/{exe,cmdline,status}` → 报告体（exe 含 "(deleted)" = 升级残留铁证）
//!
//! 原则（F1）：**指认不杀戮**——只产出人话报告，进程处置留给调用方/人。
//! 非 Linux（/proc 缺失）→ 空报告（调用方维持现降级语义，F6）。

use std::collections::BTreeSet;
use std::path::PathBuf;

/// 单个占用进程的体检报告。
#[derive(Debug, Clone)]
pub struct ConflictReport {
    pub pid: u32,
    /// `readlink /proc/pid/exe`（可能带内核后缀 "(deleted)"）。
    pub exe: String,
    /// cmdline 首段（截断 80 字符）。
    pub cmd: String,
    /// exe 已删除 = 二进制被替换后进程仍存活（升级残留铁证，F4）。
    pub deleted: bool,
    /// (ppid, 父进程 cmd 首段)——daemon 归属指认。
    pub parent: Option<(u32, String)>,
}

impl ConflictReport {
    /// 人话行（eprintln 用）。
    pub fn line(&self) -> String {
        let mark = if self.deleted { " ⚠ 升级残留(exe 已删除)" } else { "" };
        let parent = match &self.parent {
            Some((ppid, pcmd)) => format!("，父={pcmd}({ppid})"),
            None => String::new(),
        };
        format!(
            "pid {}  {}{mark}  cmd={}{}",
            self.pid, self.exe, self.cmd, parent
        )
    }
}

/// 诊断 `port`（tcp+udp 双面，IPv4+IPv6）。无占用/非 Linux → 空 Vec。
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

/// `/proc/net/{tcp,tcp6,udp,udp6}` 中本地端口 == `port` 的 socket inode 集。
fn port_inodes(port: u16) -> BTreeSet<u64> {
    let mut inodes = BTreeSet::new();
    for f in ["tcp", "tcp6", "udp", "udp6"] {
        let Ok(text) = std::fs::read_to_string(format!("/proc/net/{f}")) else { continue };
        for line in text.lines().skip(1) {
            // 格式: sl local_address rem_address st ... inode
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
        if let Some(rest) = t.strip_prefix("socket:[") {
            if let Ok(ino) = rest.trim_end_matches(']').parse::<u64>()
                && inodes.contains(&ino)
            {
                return true;
            }
        }
    }
    false
}

fn report_of_pid(pid: u32) -> Option<ConflictReport> {
    let exe = std::fs::read_link(format!("/proc/{pid}/exe"))
        .ok()?
        .to_string_lossy()
        .into_owned();
    let cmd = std::fs::read_to_string(format!("/proc/{pid}/cmdline"))
        .unwrap_or_default()
        .split('\0')
        .next()
        .unwrap_or("")
        .chars()
        .take(80)
        .collect::<String>();
    let deleted = exe.contains("(deleted)");
    let ppid = std::fs::read_to_string(format!("/proc/{pid}/status"))
        .ok()
        .and_then(|s| {
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
        (!cmd.is_empty()).then(|| (pp, cmd))
    });
    Some(ConflictReport { pid, exe, cmd, deleted, parent })
}

/// stop 自证（T4）：app 名对应的本族进程是否已归零。
/// 依 oxmgr ps 语义过重——轻量形：进程表里 cmdline 含 `msrtc-<app>` 或
/// `<app>` 且 exe 非本测试进程即视为残存。查无 = 归零。
pub fn diagnose_app_gone(app: &str) -> bool {
    let needles = [format!("msrtc-{app}"), format!("mediaservo-{app}"), app.to_string()];
    for pid in proc_pids() {
        if pid == std::process::id() {
            continue;
        }
        let Ok(cmd) = std::fs::read_to_string(format!("/proc/{pid}/cmdline")) else { continue };
        let first = cmd.split('\0').next().unwrap_or("");
        if needles.iter().any(|n| first.contains(n.as_str())) {
            return false; // 有残存
        }
    }
    true
}

/// 人话报告输出（eprintln；占用者含升级残留时给处置建议，F1 指认不杀戮）。
pub fn print_report(reports: &[ConflictReport], port: u16) {
    if reports.is_empty() {
        return;
    }
    eprintln!("端口 {port} 被占用——冲突体检:");
    for r in reports {
        eprintln!("  {}", r.line());
        if r.deleted {
            eprintln!("  → 该进程运行的是已删除的旧二进制（升级残留）；kill {} 后重跑 start", r.pid);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 未占用端口 → 空报告（快测；选高位冷端口，撞占用概率可忽略）。
    #[test]
    fn unoccupied_port_yields_empty() {
        // 找一个确实没被监听的端口：临时 bind 再 drop 后立即诊断
        let l = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = l.local_addr().unwrap().port();
        drop(l);
        assert!(diagnose_port(port).is_empty(), "drop 后无占用者");
    }

    /// 本测试进程自占端口（TCP+UDP）→ 恰 1 条报告且 pid=自己、deleted=false。
    #[test]
    fn self_occupied_port_reports_self() {
        let l = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let u = std::net::UdpSocket::bind(("127.0.0.1", 0)).unwrap();
        let tcp_port = l.local_addr().unwrap().port();
        let udp_port = u.local_addr().unwrap().port();
        let tcp = diagnose_port(tcp_port);
        assert_eq!(tcp.len(), 1, "tcp 自占应恰 1 条: {tcp:?}");
        assert_eq!(tcp[0].pid, std::process::id());
        assert!(!tcp[0].deleted);
        assert!(tcp[0].cmd.contains("conflict") || tcp[0].cmd.contains("mediaservo") || !tcp[0].cmd.is_empty());
        let udp = diagnose_port(udp_port);
        assert_eq!(udp.len(), 1, "udp 自占应恰 1 条: {udp:?}");
        assert_eq!(udp[0].pid, std::process::id());
    }

    /// 人话行格式钉（deleted 标记与建议语）。
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
        assert!(line.contains("pid 42") && line.contains("(deleted)") && line.contains("升级残留"));
        assert!(line.contains("oxmgr daemon run(7)"));
    }
}
