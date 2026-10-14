//! T3: the serve SIGTERM/SIGINT graceful-exit five-step sequence -- a **real-signal e2e** (design §D2 [sF2 lck-F1]).
//!
//! PIT-189 iron rule: tokio::signal handlers are **process-level** resources; an in-process self-kill under libtest concurrency broadcasts
//! spurious wakeups to other cases -> real-signal cases must run in an **independent child process** (here we spawn the compiled `mediaservo-weaknet` bin,
//! SIGTERM only the child's pid, never touching the test process).
//!
//! Zero real-touch surface (neither case produces real pressure): A = no state (the clear branch not entered) + the `--token-file` generation path;
//! B = one lazy state planted (teardown channel=sidecar AND sidecar=null AND steps=[]) -- the startup cross-check's
//! "channel cannot be rebuilt" skips, `engine::clear(quiet=true)` executes nothing for empty steps, only remove_file, never
//! exec tc / never starts docker along the way (the five-step 3-4 order is separately pinned by `shutdown_plan` unit tests). Fixtures all under
//! `/tmp/wnet-t3-*` (WEAKNET_STATEDIR injected), self-cleaned after use.

use std::io::Read;
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_mediaservo-weaknet");

fn free_port() -> u16 {
    let a = ("127.0.0.1", 0u16).to_socket_addrs().unwrap().next().unwrap();
    let l = std::net::TcpListener::bind(a).unwrap();
    l.local_addr().unwrap().port()
}

/// Lazy state: sidecar=null -> channel_of=None -> startup skips the tc cross-check; steps=[] -> clear executes nothing.
const INERT_STATE: &str = r#"{
  "schema": "weaknet-state/1",
  "spec": {"rtt_ms": 100, "jitter_ms": 0, "loss": null, "reorder_pct": null,
           "rate_mbps": null, "seed": null, "limit": 100000, "dir": "both"},
  "dir": "both",
  "scope": "media",
  "iface": "lo",
  "expires_at_ms": 99999999999999,
  "created_root": false,
  "teardown": {"channel": "sidecar", "sidecar": null, "steps": []}
}"#;

fn spawn_serve(statedir: &Path, extra: &[&str]) -> (Child, u16) {
    let port = free_port();
    let child = Command::new(BIN)
        .arg("serve")
        .arg("--listen")
        .arg(format!("127.0.0.1:{port}"))
        .args(extra)
        .env("WEAKNET_STATEDIR", statedir)
        // no WEAKNET_TOKEN/--token injected: takes the --token-file branch or CSPRNG; the auto channel does not probe at startup.
        .env_remove("WEAKNET_TOKEN")
        .env_remove("WEAKNET_CHANNEL")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn mediaservo-weaknet serve");
    (child, port)
}

fn wait_listening(child: &mut Child, port: u16) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(st) = child.try_wait().unwrap() {
            panic!("the serve process exited before becoming ready (exit={st:?})");
        }
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return; // listener bound -> the signal listener is established (the serve future is in select)
        }
        if Instant::now() > deadline {
            panic!("serve did not listen on :{port} within 10s");
        }
        std::thread::sleep(Duration::from_millis(30));
    }
}

/// Sends a real SIGTERM to the **child process** and asserts it exits 0 within the graceful budget (the counterexample to the oxmgr-grace-then-SIGKILL dead end).
fn sigterm_and_reap(child: &mut Child, stdout: &mut Option<std::process::ChildStdout>) -> String {
    let pid = child.id().to_string();
    let code = Command::new("kill")
        .args(["-TERM", &pid])
        .status()
        .expect("kill -TERM available (coreutils)")
        .code();
    assert_eq!(code, Some(0), "kill -TERM dispatch failed");
    let deadline = Instant::now() + Duration::from_secs(3);
    let status = loop {
        if let Some(st) = child.try_wait().unwrap() {
            break st;
        }
        if Instant::now() > deadline {
            let _ = child.kill(); // over budget = an lck-F1 violation; the test process reaps to prevent orphans
            panic!("did not exit within 3s of SIGTERM (unbounded blocking on the graceful path?)");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(status.success(), "non-0 SIGTERM exit code: {status:?}");
    let mut buf = String::new();
    if let Some(so) = stdout.as_mut() {
        so.read_to_string(&mut buf).ok();
    }
    buf
}

fn assert_seq(log: &str, markers: &[&str]) {
    let mut prev = 0usize;
    for m in markers {
        let pos = log
            .find(m)
            .unwrap_or_else(|| panic!("missing graceful-exit step marker {m:?}\n---log---\n{log}"));
        assert!(
            pos >= prev,
            "steps out of order: {m:?} at {pos} < previous {prev}\n---log---\n{log}"
        );
        prev = pos;
    }
}

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("wnet-t3-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A: no pressure -- SIGTERM takes steps 1-2-5 (never entering clear), token-file generated on-site at 0600.
#[test]
fn serve_sigterm_graceful_no_pressure() {
    let dir = tmp("a");
    let token = dir.join("weaknet.token");
    let (mut child, port) = spawn_serve(&dir, &["--token-file", token.to_str().unwrap()]);
    wait_listening(&mut child, port);
    let mut stdout = child.stdout.take();
    let log = sigterm_and_reap(&mut child, &mut stdout);

    assert_seq(
        &log,
        &[
            "graceful exit (1) stop flag set",
            "graceful exit (2) drive quiet",
            "graceful exit (5) exit 0",
        ],
    );
    assert!(
        !log.contains("graceful exit (3-4)"),
        "no state must not enter the clear branch:\n{log}"
    );
    // token-file generation path: present + 0600 (lck-F10/F11).
    assert!(token.exists(), "the token file should have been generated");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&token).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "the token file's mode must be 0600, got {mode:#o}");
    }
    // no state on disk (never applied).
    assert!(!dir.join("state.json").exists());
    std::fs::remove_dir_all(&dir).ok();
}

/// B: a lazy active state -- SIGTERM takes steps 1-2-3-4-5, the clear branch shows up and tears the state down cleanly (zero real tc/docker).
#[test]
fn serve_sigterm_graceful_clears_inert_state() {
    let dir = tmp("b");
    let state_path = dir.join("state.json");
    std::fs::write(&state_path, INERT_STATE).unwrap();
    let (mut child, port) = spawn_serve(&dir, &["--token", "e2e-token-abcdef0123456789"]);
    wait_listening(&mut child, port);
    let mut stdout = child.stdout.take();
    let log = sigterm_and_reap(&mut child, &mut stdout);

    assert_seq(
        &log,
        &[
            "graceful exit (1) stop flag set",
            "graceful exit (2) drive quiet",
            "graceful exit (3-4) clear",
            "graceful exit (5) exit 0",
        ],
    );
    assert!(
        !state_path.exists(),
        "five-step 3-4 should land inactive via engine::clear (remove state):\n{log}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// C: an existing token-file with loosened permissions (0644) -- startup refuses to use it, non-zero exit (the lck-F11 umask scenario).
#[test]
fn serve_rejects_world_readable_token_file() {
    #[cfg(not(unix))]
    {
        return; // POSIX mode judgment only meaningful on unix
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let dir = tmp("c");
        let token = dir.join("weaknet.token");
        std::fs::write(&token, "leaky-secret-token\n").unwrap();
        std::fs::set_permissions(&token, std::fs::Permissions::from_mode(0o644)).unwrap();
        let (mut child, _) = spawn_serve(&dir, &["--token-file", token.to_str().unwrap()]);
        // rejected before binding (token parsing precedes bind) -> the process should die short with a non-zero exit, never SIGTERM it.
        let deadline = Instant::now() + Duration::from_secs(10);
        let status = loop {
            if let Some(st) = child.try_wait().unwrap() {
                break st;
            }
            assert!(Instant::now() < deadline, "the refuse-to-use case did not exit within 10s");
            std::thread::sleep(Duration::from_millis(30));
        };
        assert!(!status.success(), "a 0644 token-file must be rejected -> non-zero exit");
        std::fs::remove_dir_all(&dir).ok();
    }
}
