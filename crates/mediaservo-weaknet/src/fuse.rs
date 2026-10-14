//! fuse.rs -- the process-primitive layer of the four-layer fuse (T2 scope = clock + pidfile + spawn/kill primitives).
//!
//! Design contract source = docs/plans/weaknet-agent/design.md §fuse (rev-2.2):
//! - the expiry lazy check's **pure predicate** (any command entry + the serve state-frame tick call the same function);
//! - the watchdog is an independent process: spawn uses `CommandExt::process_group(0)`, asserting pgid==pid right after;
//!   pidfile protocol `"<pid> <starttime>"` (starttime = /proc/<pid>/stat field 22);
//!   validate starttime before the group kill, mismatch = stale, refuse to kill; **pgid==own pgid refuses to kill**;
//!   not-exist / process-already-dead = idempotent Ok (the CLI and serve watchdogs fire concurrently, the second must hit nothing -- noise guard).
//!
//! This module **never executes tc / never touches the state.json write path** (engine/state = T3/T4).
//! state file reads exist only for the `watchdog_run` skeleton's expiry parsing (JSON, tolerant of unknown fields).
//! Zero deps: /proc parsing entirely via std::fs (libc deliberately not introduced); paths always parameterized (C20).

use std::io::ErrorKind;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Unified semantics for missing pidfile/state fields: 0 = never expires (bash `EXPIRES_AT=0 || empty -> return 0`).
pub const NEVER_EXPIRES: u64 = 0;

/// Sleep shard cap. ponytail: the sharded loop guards against Duration overflow/hang from a single `sleep(huge)`;
/// swap to a tokio timer at T4 (watchdog_run is already an independent process; thread sleep is acceptable).
const SLEEP_CHUNK_MAX: Duration = Duration::from_secs(3600);

/// Expiry predicate (pure function, table-tested). Semantics inherited from bash autoheal_check:
/// `expires_at_ms` None / 0 (= the forever tier, --forever --i-know) -> never expires;
/// boundary `now == expires` **does not count as expired** (bash `-gt` is strictly greater).
#[must_use]
pub fn is_expired(expires_at_ms: Option<u64>, now_ms: u64) -> bool {
    match expires_at_ms {
        Some(e) if e != NEVER_EXPIRES => now_ms > e,
        _ => false,
    }
}

/// Read the expiry ms from the state file. JSON `{... "expires_at_ms": N ...}`, tolerant of unknown fields (the full
/// schema belongs to state.rs at T4). Root-shape tolerance: an object or a `{"state": {...}}` wrapper.
/// Missing field / null / 0 -> None (never expires); unreadable file / invalid JSON / non-numeric / negative -> Err (C15, never silent).
pub fn expiry_at_ms(state_path: &Path) -> Result<Option<u64>, String> {
    let raw = std::fs::read_to_string(state_path)
        .map_err(|e| format!("failed to read state {}: {e}", state_path.display()))?;
    let val: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|e| format!("invalid state JSON {}: {e}", state_path.display()))?;
    let obj = match &val {
        serde_json::Value::Object(m) => Ok(m),
        other => Err(format!("state root is not an object: {}", short_type(other))),
    }?;
    let field = obj.get("expires_at_ms").or(obj.get("expires_at"));
    Ok(match field {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::Number(n)) => match n.as_u64() {
            Some(u) if u != NEVER_EXPIRES => Some(u),
            Some(_) => None,
            None => {
                let i =
                    n.as_i64().ok_or_else(|| format!("cannot interpret expires_at number: {n}"))?;
                if i < 0 {
                    return Err(format!("expires_at negative: {i}"));
                }
                None
            }
        },
        Some(other) => return Err(format!("expires_at not a number: {other}")),
    })
}

fn short_type(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "bool",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

/// Current epoch ms (the production clock source; the pure predicate doesn't depend on it, tests inject their own now).
#[must_use]
pub fn now_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::from_secs(0))
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

// ---------- /proc primitives (field order per proc(5); comm may contain spaces/parens -> split at the last ')') ----------

/// The remaining field stream after the comm field (from field 3 = state onward).
fn stat_fields_after_comm(pid: u32) -> Result<Vec<String>, String> {
    let raw = std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .map_err(|e| format!("/proc/{pid}/stat unreadable: {e}"))?;
    let tail = raw
        .rsplit_once(')')
        .ok_or_else(|| format!("/proc/{pid}/stat missing the comm close paren (malformed)"))?
        .1;
    Ok(tail.split_whitespace().map(str::to_owned).collect())
}

/// /proc/<pid>/stat field 22 = starttime (jiffies since boot). Process missing/malformed -> None.
#[must_use]
pub fn read_proc_starttime(pid: u32) -> Option<u64> {
    // after_comm[0] = field 3 (state), so field 22 = after_comm[19]
    match stat_fields_after_comm(pid) {
        Ok(f) => f.get(19).and_then(|s| s.parse().ok()),
        Err(e) => {
            eprintln!("weaknet(fuse): {e}");
            None
        }
    }
}

/// /proc/<pid>/stat field 5 = pgid.
#[must_use]
pub fn proc_pgid(pid: u32) -> Option<u64> {
    // after_comm[0] = field 3 (ppid), so field 5 (pgid) = after_comm[2]
    match stat_fields_after_comm(pid) {
        Ok(f) => f.get(2).and_then(|s| s.parse().ok()),
        Err(e) => {
            eprintln!("weaknet(fuse): {e}");
            None
        }
    }
}

/// This process's pid (resolved via the /proc/self link, zero libc).
fn current_pid() -> Result<u32, String> {
    let target =
        std::fs::read_link("/proc/self").map_err(|e| format!("/proc/self link unreadable: {e}"))?;
    let name = target
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| format!("/proc/self target unexpected: {}", target.display()))?;
    name.parse().map_err(|e| format!("/proc/self pid parse failed for {name}: {e}"))
}

// ---------- pidfile protocol "<pid> <starttime>" ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PidEntry {
    pub pid: u32,
    pub starttime: u64,
}

pub fn write_pidfile(path: &Path, entry: PidEntry) -> Result<(), String> {
    std::fs::write(path, format!("{} {}\n", entry.pid, entry.starttime))
        .map_err(|e| format!("failed to write pidfile {}: {e}", path.display()))
}

/// Read the pidfile. Missing file -> Ok(None) (idempotent); malformed content -> Err (protocol violations must be visible, C15).
pub fn read_pidfile(path: &Path) -> Result<Option<PidEntry>, String> {
    let raw = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("failed to read pidfile {}: {e}", path.display())),
    };
    let mut it = raw.split_whitespace();
    let pid = it.next().and_then(|s| s.parse::<u32>().ok());
    let st = it.next().and_then(|s| s.parse::<u64>().ok());
    match (pid, st) {
        (Some(p), Some(s)) => Ok(Some(PidEntry { pid: p, starttime: s })),
        _ => Err(format!(
            "pidfile {} protocol violation (expected \"<pid> <starttime>\"): {:?}",
            path.display(),
            raw.trim()
        )),
    }
}

// ---------- watchdog spawn / kill ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KillOutcome {
    /// group-kill command dispatched (TERM to -pgid)
    Killed,
    /// no pidfile / target process dead / ENOENT class -- idempotent success
    Idempotent,
    /// pidfile present but starttime mismatched (pid reused) -- refused, stale record cleared
    Stale,
}

/// spawn spec for self re-exec (program parameterized = the test-injection path for /bin/sleep;
/// production passes `current_exe()` + `["__watchdog", state_path]` from main.rs).
#[cfg(unix)]
pub fn spawn_watchdog(
    program: &Path,
    args: &[&str],
    pidfile: &Path,
) -> Result<std::process::Child, String> {
    use std::os::unix::process::CommandExt;
    let mut cmd = std::process::Command::new(program);
    cmd.args(args)
        .stdin(std::process::Stdio::null())
        // inheriting stdout/stderr makes pipe-based callers (apply ... | tee/sed) block until the watchdog's sleep ends
        // before seeing EOF (measured: duration=180 blocks 180s); the bash prototype's setsid is the same as `>/dev/null 2>&1`.
        // diagnostics aren't lost: watchdog events land in the timeline (watchdog-clear / watchdog-clear-failed).
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .process_group(0);
    let mut child =
        cmd.spawn().map_err(|e| format!("watchdog spawn {} failed: {e}", program.display()))?;
    let pid = child.id();
    match proc_pgid(pid) {
        Some(g) if g == u64::from(pid) => {}
        other => {
            let _ = child.kill();
            return Err(format!(
                "watchdog pgid invariant broken: pid={pid} pgid={other:?} (expected equal)"
            ));
        }
    }
    let st = match read_proc_starttime(pid) {
        Some(s) => s,
        None => {
            let _ = child.kill();
            return Err(format!("watchdog pid={pid} starttime read failed (died in a race?)"));
        }
    };
    write_pidfile(pidfile, PidEntry { pid, starttime: st })?;
    Ok(child)
}

#[cfg(not(unix))]
pub fn spawn_watchdog(
    _program: &Path,
    _args: &[&str],
    _pidfile: &Path,
) -> Result<std::process::Child, String> {
    Err("non-unix platforms have no process_group primitive (reported at runtime; no compile_error -- same rule as design §engine)".into())
}

/// Group-kill the watchdog per the pidfile. Verdict order (design §fuse):
/// no file/process dead -> Idempotent; starttime mismatch -> Stale (clear the file, refuse);
/// target pgid == own pgid -> Err refusing to kill (self-kill defense); otherwise `sh -c "kill -TERM -<pid>"`.
#[cfg(unix)]
pub fn kill_watchdog(pidfile: &Path) -> Result<KillOutcome, String> {
    let entry = match read_pidfile(pidfile)? {
        Some(e) => e,
        None => return Ok(KillOutcome::Idempotent),
    };
    let alive = Path::new(&format!("/proc/{}", entry.pid)).exists();
    if !alive {
        clear_pidfile(pidfile);
        return Ok(KillOutcome::Idempotent);
    }
    match read_proc_starttime(entry.pid) {
        Some(now_st) if now_st == entry.starttime => {}
        Some(now_st) => {
            eprintln!(
                "weaknet(fuse): pidfile recorded pid={} starttime={}, current starttime={} -- pid reused, refusing to kill, clearing the stale pidfile",
                entry.pid, entry.starttime, now_st
            );
            clear_pidfile(pidfile);
            return Ok(KillOutcome::Stale);
        }
        None => {
            clear_pidfile(pidfile);
            return Ok(KillOutcome::Idempotent);
        }
    }
    let own = current_pid()?;
    let target_pgid = proc_pgid(entry.pid).ok_or_else(|| {
        format!("failed to read the target pid={} pgid (died in a race? re-run clear)", entry.pid)
    })?;
    let own_pgid = proc_pgid(own).map(|g| g as u32).unwrap_or_else(|| {
        eprintln!(
            "weaknet(fuse): failed to read own pgid, conservatively comparing against own pid {own}"
        );
        own
    });
    if target_pgid == u64::from(own_pgid) {
        return Err(format!(
            "refusing to kill: watchdog target group pgid={target_pgid} == this process's pgid={own_pgid} (self-group defense)"
        ));
    }
    let out = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("kill -TERM -{}", entry.pid))
        .output()
        .map_err(|e| format!("failed to spawn kill: {e}"))?;
    if !out.status.success() {
        // the group already dispersed in a race (kill: No such process) = idempotent-success semantics; WARN on record (C15)
        eprintln!(
            "weaknet(fuse): kill -TERM -{} exited non-zero (treated as already dead): {}",
            entry.pid,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    clear_pidfile(pidfile);
    Ok(KillOutcome::Killed)
}

#[cfg(not(unix))]
pub fn kill_watchdog(_pidfile: &Path) -> Result<KillOutcome, String> {
    Err("non-unix platforms have no group-kill primitive (reported at runtime)".into())
}

fn clear_pidfile(pidfile: &Path) {
    if let Err(e) = std::fs::remove_file(pidfile)
        && e.kind() != ErrorKind::NotFound
    {
        eprintln!("weaknet(fuse): failed to clear pidfile {}: {e}", pidfile.display());
    }
}

/// watchdog executor skeleton: read the state expiry -> sleep until it fires -> callback (the engine teardown wiring belongs to T3/T4).
/// Missing/None expiry -> Err (nothing to guard = config error, never silently resident).
pub fn watchdog_run(state_path: &Path, on_expire: impl Fn(&Path)) -> Result<(), String> {
    let expiry = expiry_at_ms(state_path)?.ok_or_else(|| {
        format!("watchdog: state {} has no expiry moment (nothing to guard)", state_path.display())
    })?;
    let start_wall = now_epoch_ms();
    if expiry <= start_wall {
        on_expire(state_path);
        return Ok(());
    }
    let deadline = Instant::now() + Duration::from_millis(expiry - start_wall);
    loop {
        let now = Instant::now();
        if now >= deadline {
            break;
        }
        std::thread::sleep((deadline - now).min(SLEEP_CHUNK_MAX));
    }
    on_expire(state_path);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------- clock table (>=4 cases: missing / future / past / boundary==) ----------

    #[test]
    fn clock_table() {
        // missing (None / 0 = forever tier) -> never expires
        assert!(!is_expired(None, 1_000));
        assert!(!is_expired(Some(NEVER_EXPIRES), u64::MAX));
        // future -> not expired
        assert!(!is_expired(Some(2_000), 1_000));
        // past -> expired
        assert!(is_expired(Some(1_000), 1_001));
        // boundary now == expires -> not expired (bash -gt strict)
        assert!(!is_expired(Some(1_000), 1_000));
    }

    fn tmp_state(name: &str, body: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("weaknet-fuse-{}-{name}", std::process::id()));
        std::fs::write(&p, body).expect("write tmp state");
        p
    }

    #[test]
    fn expiry_reader_missing_field_is_never() {
        let p = tmp_state("miss", r#"{"spec": {"rtt_ms": 80}}"#);
        assert_eq!(expiry_at_ms(&p).unwrap(), None);
        let p0 = tmp_state("zero", r#"{"expires_at_ms": 0, "unknown_top": true}"#);
        assert_eq!(expiry_at_ms(&p0).unwrap(), None);
        let pn = tmp_state("null", r#"{"expires_at_ms": null}"#);
        assert_eq!(expiry_at_ms(&pn).unwrap(), None);
        let pall = tmp_state("alias", r#"{"expires_at": 123, "job": {"pid": 1}}"#);
        assert_eq!(expiry_at_ms(&pall).unwrap(), Some(123));
        std::fs::remove_file(&p).ok();
        std::fs::remove_file(&p0).ok();
        std::fs::remove_file(&pn).ok();
        std::fs::remove_file(&pall).ok();
    }

    #[test]
    fn expiry_reader_errors_not_silent() {
        let bad = tmp_state("bad", "not json");
        assert!(expiry_at_ms(&bad).is_err());
        let wrong_type = tmp_state("wt", r#"{"expires_at_ms": "123"}"#);
        assert!(expiry_at_ms(&wrong_type).is_err());
        let neg = tmp_state("neg", r#"{"expires_at_ms": -5}"#);
        assert!(expiry_at_ms(&neg).is_err());
        let root = tmp_state("root", "[1,2]");
        assert!(expiry_at_ms(&root).is_err());
        assert!(expiry_at_ms(Path::new("/nonexistent/weaknet-test-state")).is_err());
        for f in ["bad", "wt", "neg", "root"] {
            std::fs::remove_file(tmp_state_path(f)).ok();
        }
    }

    fn tmp_state_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("weaknet-fuse-{}-{name}", std::process::id()))
    }

    #[test]
    fn watchdog_run_fires_callback_at_expiry() {
        // a small wheel expiring 30ms out: a thread runs watchdog_run, the channel receives the callback
        let expiry = now_epoch_ms() + 30;
        let p = tmp_state("fire", &format!(r#"{{"expires_at_ms": {expiry}}}"#));
        let (tx, rx) = std::sync::mpsc::channel();
        let path = p.clone();
        std::thread::spawn(move || {
            let _ = watchdog_run(&path, |_| {
                tx.send(()).unwrap();
            });
        });
        rx.recv_timeout(Duration::from_secs(2))
            .expect("watchdog should fire the callback when it expires");
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn watchdog_run_without_expiry_is_err() {
        let p = tmp_state("noexp", r#"{"spec": {}}"#);
        assert!(watchdog_run(&p, |_| unreachable!("must not fire the callback")).is_err());
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn pidfile_roundtrip_and_protocol_violation() {
        let mut p = std::env::temp_dir();
        p.push(format!("weaknet-fuse-pid-{}", std::process::id()));
        let missing = read_pidfile(&p).unwrap();
        assert_eq!(missing, None, "missing = Ok(None) idempotent");
        write_pidfile(&p, PidEntry { pid: 4242, starttime: 99 }).unwrap();
        assert_eq!(read_pidfile(&p).unwrap(), Some(PidEntry { pid: 4242, starttime: 99 }));
        std::fs::write(&p, "garbage-line").unwrap();
        assert!(read_pidfile(&p).is_err(), "malformed content must report a protocol violation");
        std::fs::remove_file(&p).ok();
    }

    // ---------- /proc and spawn/kill (Linux-only) ----------

    #[cfg(target_os = "linux")]
    #[test]
    fn proc_readers_sane_on_self() {
        let pid = current_pid().unwrap();
        assert!(pid > 0);
        let st = read_proc_starttime(pid).expect("own starttime must be readable");
        assert!(st > 0, "starttime non-zero (jiffies since boot)");
        let g = proc_pgid(pid).expect("own pgid must be readable");
        assert!(g > 0);
        assert!(read_proc_starttime(u32::MAX).is_none(), "missing process -> None, no panic");
        assert!(proc_pgid(u32::MAX).is_none());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn spawn_pgid_invariant_then_kill_and_idempotent() {
        let pf = unique_path("pgid-pidfile");
        // inject /bin/sleep 30 (production = current_exe + __watchdog, same function path)
        let mut child = spawn_watchdog(Path::new("/bin/sleep"), &["30"], &pf)
            .expect("spawning /bin/sleep should succeed");
        let pid = child.id();
        // invariant: pgid == pid (the effect of process_group(0))
        assert_eq!(proc_pgid(pid), Some(u64::from(pid)));
        let entry = read_pidfile(&pf).unwrap().expect("pidfile must be present after spawn");
        assert_eq!(entry.pid, pid);

        // group kill (kill_watchdog's kill -TERM -<pid> channel)
        assert_eq!(kill_watchdog(&pf).unwrap(), KillOutcome::Killed);
        let _ = child.wait();
        // killing again = idempotent (file cleared + process dead, double insurance)
        assert_eq!(kill_watchdog(&pf).unwrap(), KillOutcome::Idempotent);
        std::fs::remove_file(&pf).ok();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn kill_refuses_self_group_and_stale_starttime() {
        // self-group defense: forge the pidfile = this process's pid + real starttime, but first align the pgid --
        // a test process's own pgid==itself (usually true when bash runs cargo test directly); if not, assert only not-Killed.
        let pid = current_pid().unwrap();
        let st = read_proc_starttime(pid).unwrap();
        let pf = unique_path("self-pidfile");
        write_pidfile(&pf, PidEntry { pid, starttime: st }).unwrap();
        let r = kill_watchdog(&pf);
        if proc_pgid(pid) == Some(u64::from(pid)) {
            assert!(r.is_err(), "pgid==pid must refuse to kill: {r:?}");
            assert!(r.unwrap_err().contains("refusing"));
        } else {
            assert!(!matches!(r, Ok(KillOutcome::Killed)), "never allowed to kill our own group");
        }
        std::fs::remove_file(&pf).ok();

        // stale starttime: live pid + wrong starttime -> Stale refuses and clears the file
        let pf2 = unique_path("stale-pidfile");
        write_pidfile(&pf2, PidEntry { pid, starttime: st ^ 0xFFFF_FFFF }).unwrap();
        assert_eq!(kill_watchdog(&pf2).unwrap(), KillOutcome::Stale);
        assert!(!pf2.exists(), "the Stale path must reap the stale pidfile");
    }

    fn unique_path(tag: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("weaknet-fuse-{}-{}-{}", std::process::id(), tag, now_epoch_ms()));
        p
    }
}
