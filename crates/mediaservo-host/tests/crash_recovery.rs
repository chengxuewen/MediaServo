//! Task C5: crash-restart fault-injection e2e - validating the architecture's core value (Momus MEDIUM-3).
//!
//! Kill the capturer (SIGKILL = crash, not a SIGTERM graceful exit) -> oxmgr auto-respawns per
//! `restart_policy = "always"` -> **republishing on the same topic succeeds**
//! (max_publishers(1) + leftover iceoryx2 services do not block - the C25 root-cause path) ->
//! subscribers (the in-process subscriber + host-recorder) resume receiving frames.
//!
//! The all-production path: `host init` -> `host token issue` -> `host start` (translate ->
//! oxfile -> `oxmgr apply`) -> `oxmgr` manages the capturer -> kill -9 -> oxmgr restarts ->
//! FrameBus subscribers prove frame recovery. No streamer (needs an external SFU server; covered by C2/C4).
//!
//! Prerequisites: oxmgr 0.5.0 on PATH (~/.local/bin) with the daemon running; C25: clean
//! `/tmp/iceoryx2` + `/dev/shm/iox2_*` before running (this test cleans up itself, see `cleanup_iceoryx`).

#![cfg(target_os = "linux")]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use mediaservo_link::{
    CapabilityToken, Ed25519SigningKey, Ed25519VerifyingKey, FrameBus, FrameTopic, NodeAcl, NodeId,
    Role,
};

// Test Ed25519 keypair (generated with openssl, test-only; same source as link/deck/capturer tests).
const PRIV_PEM: &str = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIObCg8b+Le6kKOI/+pE+4+YhXUlr6X6h7q8p/MjvHmXT\n-----END PRIVATE KEY-----\n";
const PUB_PEM: &str = "-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VwAyEAgXprEbnahCZoZtLpiUqR0ruqtzEfRXk/Gl/6F6PEm4o=\n-----END PUBLIC KEY-----\n";

fn token(role: Role, node_id: &str) -> (CapabilityToken, Ed25519VerifyingKey) {
    let sk = Ed25519SigningKey::from_pem(PRIV_PEM.as_bytes());
    let vk = Ed25519VerifyingKey::from_pem(PUB_PEM.as_bytes());
    let acl = NodeAcl::for_role(NodeId::new(node_id), role);
    (CapabilityToken::sign(&acl, 3600, &sk).unwrap(), vk)
}

/// C25: clean iceoryx2 0.9.3 runtime residue (/tmp/iceoryx2 + /dev/shm/iox2_*).
fn cleanup_iceoryx() {
    let _ = std::fs::remove_dir_all("/tmp/iceoryx2");
    if let Ok(entries) = std::fs::read_dir("/dev/shm") {
        for e in entries.flatten() {
            if e.file_name().to_string_lossy().starts_with("iox2_") {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

/// oxmgr is on PATH (the host CLI invokes `oxmgr` internally; the test process may lack ~/.local/bin).
fn path_with_oxmgr() -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    let mut path = format!("{home}/.local/bin");
    if let Ok(existing) = std::env::var("PATH") {
        path.push(':');
        path.push_str(&existing);
    }
    path
}

/// `oxmgr list --json` -> the host-namespace process list (name/status/pid).
/// The oxmgr instance-daemon env (same source as translate::oxmgr_apply: OXMGR_HOME derives port-isolated
/// daemons - `oxmgr list` without this env connects the default daemon (empty) and cannot see instance processes).
fn oxmgr_env(dir: &std::path::Path) -> Vec<(String, String)> {
    let home = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf()).join("run").join("oxmgr");
    let sum: u32 = home.to_string_lossy().bytes().map(u32::from).sum();
    let port = 18000 + (sum % 400);
    vec![
        ("OXMGR_HOME".to_string(), home.to_string_lossy().into_owned()),
        ("OXMGR_DAEMON_ADDR".to_string(), format!("127.0.0.1:{port}")),
        ("OXMGR_API_ADDR".to_string(), format!("127.0.0.1:{}", port + 1000)),
    ]
}
fn oxmgr_host_procs(dir: &std::path::Path) -> Vec<(String, String, u64)> {
    let out = Command::new("oxmgr")
        .env("PATH", path_with_oxmgr())
        .envs(oxmgr_env(dir))
        .args(["list", "--json"])
        .output()
        .expect("oxmgr list");
    assert!(
        out.status.success(),
        "oxmgr list failed (oxmgr 0.5.0 must be on PATH with the daemon running): {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let procs: serde_json::Value = serde_json::from_slice(&out.stdout).expect("oxmgr json");
    procs
        .as_array()
        .expect("oxmgr list should be an array")
        .iter()
        .filter(|p| p.get("namespace").and_then(|n| n.as_str()) == Some("mediaservo-host"))
        .map(|p| {
            let name = p.get("name").and_then(|v| v.as_str()).unwrap_or("?").to_string();
            let status = p.get("status").and_then(|v| v.as_str()).unwrap_or("?").to_string();
            let pid = p.get("pid").and_then(|v| v.as_u64()).unwrap_or(0);
            (name, status, pid)
        })
        .collect()
}

/// Idempotent test start: clears host processes a previous crashed run may have left (stop+delete by name).
fn cleanup_oxmgr_host(dir: &std::path::Path) {
    let names: Vec<String> = oxmgr_host_procs(dir).into_iter().map(|(n, _, _)| n).collect();
    for name in names {
        let _ = Command::new("oxmgr")
            .env("PATH", path_with_oxmgr())
            .envs(oxmgr_env(dir))
            .args(["stop", &name])
            .status();
        let _ = Command::new("oxmgr")
            .env("PATH", path_with_oxmgr())
            .envs(oxmgr_env(dir))
            .args(["delete", &name])
            .status();
    }
}

/// Runs the host CLI (PATH gets ~/.local/bin so oxmgr resolves) and returns the exit code.
fn host_cli(args: &[&str]) -> i32 {
    let out = Command::new(env!("CARGO_BIN_EXE_mediaservo-host"))
        .env("PATH", path_with_oxmgr())
        .args(args)
        .output()
        .expect("spawn host CLI");
    let code = out.status.code().unwrap_or(-1);
    if code != 0 {
        eprintln!(
            "host {} failed (exit {code}): {}",
            args[0],
            String::from_utf8_lossy(&out.stderr)
        );
    }
    code
}

/// Drop guarantee: also runs `host stop [<dir>]` when the test fails/panics (no orphan processes left in oxmgr).
struct OxmgrGuard {
    dir: PathBuf,
    done: bool,
}

impl Drop for OxmgrGuard {
    fn drop(&mut self) {
        if !self.done {
            let _ = host_cli(&["stop", self.dir.to_str().expect("dir utf8")]);
        }
    }
}

/// Waits for host.yaml to appear (rewritten right after host init generates it).
fn write_host_toml(dir: &Path) {
    // the recorder stays resident ([record] enabled + out_dir inside the instance) -> a real long-lived subscriber
    // (host-recorder is designed to keep recording across publisher crashes); during the capturer crash-restart the recorder
    // must survive throughout (same pid) and resume receiving frames on the same handle.
    let cfg = format!(
        "sources:\n  - id: \"cam0\"\n    mode: \"generator\"\n    fps: 30\nrecord:\n  enabled: true\n  out_dir: \"{}\"\n",
        dir.join("recordings").display()
    );
    std::fs::write(dir.join("etc").join("host.yaml"), cfg).expect("write host.yaml");
}

/// Waits until the subscriber has received >= n frames, returning the max seq seen meanwhile.
async fn wait_frames(stream: &mediaservo_link::FrameStream, n: u32, timeout: Duration) -> u64 {
    let deadline = std::time::Instant::now() + timeout;
    let mut seen = 0u32;
    let mut max_seq = 0u64;
    while std::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(500), stream.recv()).await {
            Ok(Some(frame)) => {
                max_seq = max_seq.max(frame.meta().seq);
                seen += 1;
                if seen >= n {
                    return max_seq;
                }
            }
            Ok(None) => panic!("the subscription stream closed"),
            Err(_) => {} // keep waiting on timeout
        }
    }
    panic!("only {seen} frames received within {timeout:?} (need >= {n})");
}

/// E2E core: kill the capturer (SIGKILL) -> oxmgr restarts -> republish on the same topic -> subscription recovers.
///
/// Empirical conclusion (2026-08-19): after an iceoryx2 0.9.3 publisher crashes and restarts, old subscribers
/// rebuild their connections automatically (seq resets at the restart point and runs continuous afterwards); the so-called "stale old subscriber"
/// is a test-assertion artifact - the latest-slot semantics + restart-probe polling (500ms) miss the first frames after the restart (seq 0/1),
/// and everything fetched afterwards has seq >= baseline. Discriminate instead: wait >= 30 frames before the kill (baseline seq >= 29),
/// a background drainer records seq throughout, and after the restart assert a seq < baseline appears (the zero-reset must be catchable).
#[tokio::test]
#[allow(clippy::await_holding_lock)] // the polling lock window is tiny with an explicit drop; the await only sleeps (test domain)
async fn capturer_kill9_restart_resumes_frames_to_subscribers() {
    cleanup_iceoryx();
    // enable tracing inside the test process so the FrameBus subscriber threads' receive errors are visible (debug)
    mediaservo_common::logging::init(mediaservo_common::logging::LoggingConfig::default());
    let dir = tempfile::tempdir().expect("tempdir");
    let dir_path = dir.path().to_path_buf();
    cleanup_oxmgr_host(&dir_path);

    // (1) host init (positional form) + rewrite host.yaml (cam0 + record enabled, no streams
    //    - the streamer needs an external SFU server, covered by C2/C4) + issue tokens
    assert_eq!(host_cli(&["init", dir_path.to_str().expect("dir utf8")]), 0);
    write_host_toml(&dir_path);
    assert_eq!(
        host_cli(&[
            "token",
            "issue",
            "--role",
            "capture",
            "--node",
            "capture-cam0",
            "--out",
            dir_path.join("etc/link/cam0.token").to_str().expect("tok utf8"),
            dir_path.to_str().expect("dir utf8"),
        ]),
        0
    );
    assert_eq!(
        host_cli(&[
            "token",
            "issue",
            "--role",
            "recorder",
            "--node",
            "recorder-cam0",
            "--out",
            dir_path.join("etc/link/recorder.token").to_str().expect("tok utf8"),
            dir_path.to_str().expect("dir utf8"),
        ]),
        0
    );

    // (2) in-process subscriber (the Recorder role may subscribe camera/*) - attached before the capturer
    //    (same sequencing as the recorder process; verifies crash-restart recovery under the subscribe-first/publish-later order)
    let (sub_tok, sub_vk) = token(Role::Recorder, "crash-test-sub");
    let bus = FrameBus::attach("", &sub_tok, &sub_vk).expect("subscriber attach");
    let stream = bus.subscribe(&FrameTopic::new("camera/cam0")).expect("subscribe");

    // (3) host start (translate -> run/oxfile.toml -> oxmgr apply) -> all processes running
    assert_eq!(host_cli(&["start", dir_path.to_str().expect("dir utf8")]), 0);
    let mut guard = OxmgrGuard { dir: dir_path.clone(), done: false };
    let procs = oxmgr_host_procs(&dir_path);
    assert_eq!(procs.len(), 6, "expected 6 processes (5 fixed + capturer), got: {procs:?}");
    let capturer_pid = procs
        .iter()
        .find(|(n, _, _)| n == "host-capturer-cam0")
        .map(|(_, _, pid)| *pid)
        .expect("host-capturer is in oxmgr");
    let recorder_pid = procs
        .iter()
        .find(|(n, _, _)| n == "host-recorder")
        .map(|(_, _, pid)| *pid)
        .expect("host-recorder is in oxmgr");
    let recorder_running = recorder_pid != 0;
    assert!(capturer_pid != 0, "the pid should be non-zero: {procs:?}");

    // (4) pre-kill baseline: wait >= 30 frames (>= 1s @30fps) -> baseline seq >= 29. After the kill the restarted instance
    //    starts seq from 0 again; the higher the baseline, the longer the window where post-restart "zero-reset frames"
    //    stay distinguishable (latest-slot skips some low-seq frames; discrimination relies on the drainer's full record, see (7)).
    let last_seq_before = wait_frames(&stream, 30, Duration::from_secs(20)).await;
    eprintln!("[crash_recovery] last seq before kill={last_seq_before}");

    // (4b) background drainer: from this point on, record every seq on the stream (under latest-slot consumers miss frames,
    //     the continuous drainer guarantees catching the restart-point zero-reset frames)
    let drained: std::sync::Arc<std::sync::Mutex<Vec<u64>>> = std::sync::Arc::default();
    {
        let drain_stream = stream.clone();
        let drained2 = std::sync::Arc::clone(&drained);
        tokio::spawn(async move {
            loop {
                match tokio::time::timeout(Duration::from_millis(500), drain_stream.recv()).await {
                    Ok(Some(f)) => drained2.lock().expect("drain lock").push(f.meta().seq),
                    Ok(None) => break,
                    Err(_) => continue,
                }
            }
        });
    }

    // (5) SIGKILL (the crash path, not a SIGTERM graceful exit)
    unsafe { libc::kill(capturer_pid as i32, libc::SIGKILL) };
    eprintln!("[crash_recovery] SIGKILL capturer pid={capturer_pid}");

    // (6) wait for the oxmgr restart (new pid + running)
    let mut new_pid = 0u64;
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    while std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(500));
        let cur = oxmgr_host_procs(&dir_path)
            .into_iter()
            .find(|(n, _, _)| n == "host-capturer-cam0")
            .map(|(_, s, pid)| (s, pid))
            .unwrap_or_default();
        if cur.1 != 0 && cur.1 != capturer_pid {
            new_pid = cur.1;
            eprintln!(
                "[crash_recovery] oxmgr restarted the capturer: pid={} status={}",
                cur.1, cur.0
            );
            break;
        }
    }
    assert!(
        new_pid != 0,
        "oxmgr did not restart the capturer within 20s (pid still {capturer_pid})"
    );

    // (7) core assertion: republishing on the same topic succeeds - the same subscriber handle resumes frames.
    //    criterion: the drainer's records show seq < last_seq_before (the restarted instance zero-reset).
    //    ("whether the old handle still emits frames within 20s" is unreliable: under latest-slot a consumer may
    //     forever fetch seq>=baseline frames and be misjudged as stale - proven 2026-08-19.)
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    while std::time::Instant::now() < deadline {
        let drained_guard = drained.lock().expect("drain lock");
        if drained_guard.iter().any(|&s| s < last_seq_before) {
            break;
        }
        drop(drained_guard);
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let reset_seen = drained.lock().expect("drain lock").iter().any(|&s| s < last_seq_before);
    eprintln!(
        "[crash_recovery] resumed={reset_seen} drained_total={} (baseline seq >= {last_seq_before})",
        drained.lock().expect("drain lock").len()
    );
    if !reset_seen {
        // probe: create a second subscriber - discriminate "connection not recovered" vs "the publisher is not publishing at all"
        let probe = bus.subscribe(&FrameTopic::new("camera/cam0")).expect("probe subscribe");
        match tokio::time::timeout(Duration::from_secs(5), probe.recv()).await {
            Ok(Some(f)) => eprintln!(
                "[crash_recovery] PROBE: the new subscriber received a frame seq={}",
                f.meta().seq
            ),
            Ok(None) => eprintln!("[crash_recovery] PROBE: the new subscriber's stream closed"),
            Err(_) => eprintln!("[crash_recovery] PROBE: the new subscriber saw no frames in 5s"),
        }
    }
    assert!(
        reset_seen,
        "the same subscriber handle saw no zero-reset frames within 20s after the restart (seq should be < {last_seq_before}) - \
         publisher not restarted / not republished? log: see oxmgr logs/host-capturer-cam0.out.log"
    );

    // (7) crash isolation: the recorder survives throughout (same pid + running) - no crash-loop (only while the recorder resides)
    if recorder_running {
        let recorder_now = oxmgr_host_procs(&dir_path)
            .into_iter()
            .find(|(n, _, _)| n == "host-recorder")
            .expect("the recorder is still in oxmgr");
        assert_eq!(
            recorder_now.2, recorder_pid,
            "the recorder must not be restarted because of the capturer crash"
        );
        assert_eq!(recorder_now.1, "running", "the recorder should stay running: {recorder_now:?}");
    }
    eprintln!(
        "[crash_recovery] OK: capturer {capturer_pid}->{new_pid} republished on the same topic after the restart, \
         the subscriber resumed receiving frames, recorder {} survived",
        recorder_pid
    );

    // (8) cleanup: host stop (oxmgr stop + delete) -> the host namespace empties
    assert_eq!(host_cli(&["stop", dir_path.to_str().expect("dir utf8")]), 0);
    guard.done = true;
    let remaining = oxmgr_host_procs(&dir_path);
    assert!(remaining.is_empty(), "no host processes should remain after stop: {remaining:?}");
}
