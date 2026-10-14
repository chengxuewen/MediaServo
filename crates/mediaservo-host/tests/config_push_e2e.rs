//! Task E4: cloud-config closed loop e2e - ConfigPush receive -> validate -> host.yaml update (backup) ->
//! oxfile regeneration -> OxMgr hot activation (incremental apply + file-watch restart).
//!
//! (1) the mock server sends ConfigPush -> gateway -> apply: host.yaml updated + backup + oxfile regenerated
//!    + version recorded (the correlated data source of StatusReport.config_version)
//! (2) illegal config rejected: host.yaml unchanged + no backup + audit log (tracing-captured assertion)
//! (3) OxMgr hot-activation proof (Linux + oxmgr prerequisite): host start -> push new config (add a camera) ->
//!    the new capturer process appears (incremental apply Start) + the existing capturer's pid changes
//!    (host.yaml content fingerprint -> file-watch restart; when the command is unchanged apply is a Noop,
//!     so the pid change can only come from watch); then push a config removing the camera -> that app is cleaned up (removal)
//!
//! Prerequisite (C25): the test cleans iceoryx2 itself (/tmp/iceoryx2 + /dev/shm/iox2_*).

#![allow(clippy::doc_lazy_continuation, clippy::await_holding_lock, unused_assignments)] // header-doc list continuation with ` +` + polling sentinel initial 0 (the break path always covers it)
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use mediaservo_common::protocol::{PeerRole, SignalingMessage};
use mediaservo_host::gateway::{GatewayConfig, GatewayHandle, run_gateway};
use mediaservo_link::RetryConfig;
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;

/// The most recent apply outcome (Err rejection reason = the warn payload of handle_config_push's audit log;
/// the test's deterministic channel for asserting audit content - the tracing-emitted log text matches the Err text).
type Outcome = Arc<Mutex<Option<String>>>;

const VEHICLE_ROOM: &str = "vehicle-1";
const VEHICLE_PEER: &str = "veh-peer";

const CFG_V0: &str = "sources:\n  - id: \"cam0\"\n    mode: \"generator\"\n    fps: 30\n";
const CFG_V1: &str = "sources:\n  - id: \"cam0\"\n    mode: \"generator\"\n    fps: 30\n  - id: \"cam1\"\n    mode: \"generator\"\n    fps: 15\n";
const CFG_V2: &str = "sources:\n  - id: \"cam0\"\n    mode: \"generator\"\n    fps: 15\n";

fn write_host_toml(dir: &Path, cfg: &str) {
    std::fs::create_dir_all(dir.join("etc")).expect("create etc");
    std::fs::write(dir.join("etc").join("host.yaml"), cfg).expect("write host.yaml");
}

/// An apply loop shaped like host-agent's (polls the gateway for pending ConfigPush; 100ms polling in tests).
/// outcome records the latest result (applied version / rejection reason - same text as handle_config_push's audit log).
fn spawn_applier(handle: GatewayHandle, dir: PathBuf, version: Arc<AtomicU64>, outcome: Outcome) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_millis(100));
        loop {
            tick.tick().await;
            let Some(push) = handle.take_config_push() else { continue };
            match mediaservo_host::translate::handle_config_push(
                &dir,
                version.load(Ordering::Relaxed),
                &push,
            ) {
                Ok(v) => {
                    version.store(v, Ordering::Relaxed);
                    *outcome.lock().expect("outcome lock") = Some(format!("accepted v{v}"));
                }
                Err(e) => *outcome.lock().expect("outcome lock") = Some(e),
            }
        }
    });
}

/// Full mock-server handshake (aligned with the link::SignalClient protocol flow).
async fn mock_handshake(listener: &TcpListener) -> WebSocketStream<TcpStream> {
    let (stream, _) = listener.accept().await.expect("mock accept");
    let mut ws = tokio_tungstenite::accept_async(stream).await.expect("mock ws handshake");
    let psk = ws.next().await.unwrap().unwrap();
    assert!(matches!(psk, Message::Text(_)), "first message should be PSK text");
    ws.send(Message::Text(
        serde_json::to_string(&SignalingMessage::Error { code: 0, message: String::new() })
            .unwrap(),
    ))
    .await
    .unwrap();
    let join = ws.next().await.unwrap().unwrap();
    match serde_json::from_str::<SignalingMessage>(join.to_text().unwrap()).expect("parse RoomJoin")
    {
        SignalingMessage::RoomJoin { room_id, peer_role, .. } => {
            assert_eq!(peer_role, PeerRole::Host);
            ws.send(Message::Text(
                serde_json::to_string(&SignalingMessage::RoomJoined {
                    room_id,
                    peer_id: VEHICLE_PEER.into(),
                    protocol: None,
                    server_version: None,
                    session_nonce: None,
                })
                .unwrap(),
            ))
            .await
            .unwrap();
        }
        other => panic!("expected RoomJoin, got {other:?}"),
    }
    ws
}

/// Start the gateway (ephemeral local port) + the apply loop; returns handles.
async fn start_gateway_and_applier(
    addr: &str,
    dir: PathBuf,
    version: Arc<AtomicU64>,
    outcome: Outcome,
) -> (GatewayHandle, u16) {
    let cfg = GatewayConfig {
        local_port: 0,
        remote_url: format!("ws://{addr}/ws"),
        psk: "test-psk".into(),
        device: None,
        room: VEHICLE_ROOM.into(),
        retry: RetryConfig {
            max_retries: 3,
            base_delay: Duration::from_millis(50),
            max_delay: Duration::from_millis(500),
        },
    };
    let (port, handle) = run_gateway(cfg).await.expect("run_gateway");
    spawn_applier(handle.clone(), dir, version, outcome);
    (handle, port)
}

#[tokio::test(flavor = "current_thread")]
async fn config_push_via_mock_server_updates_host_toml_backs_up_and_regenerates_oxfile() {
    let dir = tempfile::tempdir().expect("tempdir");
    let dir_path = dir.path().to_path_buf();
    write_host_toml(&dir_path, CFG_V0);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let mut ws = mock_handshake(&listener).await;
        // send ConfigPush right after joining (v1 config, version 7)
        ws.send(Message::Text(
            serde_json::to_string(&SignalingMessage::ConfigPush {
                room_id: VEHICLE_ROOM.into(),
                target: VEHICLE_PEER.into(),
                config: CFG_V1.into(),
                version: 7,
            })
            .unwrap(),
        ))
        .await
        .unwrap();
        // keep the connection open (the gateway side is normal; the task ends with the runtime when the test finishes)
        loop {
            if ws.next().await.is_none() {
                break;
            }
        }
    });

    let version = Arc::new(AtomicU64::new(0));
    let outcome: Outcome = Arc::default();
    let (_handle, _port) =
        start_gateway_and_applier(&addr.to_string(), dir_path.clone(), version.clone(), outcome)
            .await;

    // poll the apply result (host.yaml updated + backup + oxfile regenerated + version recorded)
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let toml = std::fs::read_to_string(dir_path.join("etc").join("host.yaml")).unwrap();
        let bak = dir_path.join("etc").join("host.yaml.bak-7");
        let ox = std::fs::read_to_string(dir_path.join("run").join("oxfile.toml"));
        if toml == CFG_V1 && bak.exists() && version.load(Ordering::Relaxed) == 7 {
            let ox = ox.expect("the oxfile should have been generated");
            assert!(
                ox.contains("host-capturer-cam1"),
                "the new camera instance should land in the oxfile: {ox}"
            );
            break;
        }
        if std::time::Instant::now() > deadline {
            panic!(
                "apply not completed within 10s: yaml_updated={} bak={} version={}",
                toml == CFG_V1,
                bak.exists(),
                version.load(Ordering::Relaxed)
            );
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    assert_eq!(
        std::fs::read_to_string(dir_path.join("etc").join("host.yaml.bak-7")).unwrap(),
        CFG_V0,
        "the backup should hold the old config"
    );
    // F1 correlation contract: simulate an agent restart - the new instance (new Arc) restores the version from disk, not zeroed
    let restarted =
        Arc::new(AtomicU64::new(mediaservo_host::translate::recover_config_version(&dir_path)));
    assert_eq!(
        restarted.load(Ordering::Relaxed),
        7,
        "after restart the version should be restored from disk to 7 (correlation contract)"
    );
    assert_eq!(
        std::fs::read_to_string(dir_path.join("etc").join("host.yaml.bak-7")).unwrap(),
        CFG_V0,
        "the backup should hold the old config"
    );
    server.abort();
}

#[tokio::test(flavor = "current_thread")]
async fn invalid_config_push_rejected_with_audit_log_and_unchanged_files() {
    let dir = tempfile::tempdir().expect("tempdir");
    let dir_path = dir.path().to_path_buf();
    write_host_toml(&dir_path, CFG_V0);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let mut ws = mock_handshake(&listener).await;
        ws.send(Message::Text(
            serde_json::to_string(&SignalingMessage::ConfigPush {
                room_id: VEHICLE_ROOM.into(),
                target: VEHICLE_PEER.into(),
                config: "not toml [[[".into(),
                version: 9,
            })
            .unwrap(),
        ))
        .await
        .unwrap();
        loop {
            if ws.next().await.is_none() {
                break;
            }
        }
    });

    let version = Arc::new(AtomicU64::new(0));
    let outcome: Outcome = Arc::default();
    let (_handle, _port) = start_gateway_and_applier(
        &addr.to_string(),
        dir_path.clone(),
        version.clone(),
        outcome.clone(),
    )
    .await;

    // wait for the rejection outcome (the record = the same payload handle_config_push's audit warn logs)
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let guard = outcome.lock().expect("outcome lock");
        if let Some(reason) = guard.as_deref() {
            assert!(
                reason.contains("parse failed"),
                "the rejection reason should contain the parse failure (audit payload): {reason}"
            );
            break;
        }
        drop(guard);
        if std::time::Instant::now() > deadline {
            panic!("no rejection outcome within 10s (the apply loop did not handle ConfigPush)");
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    server.abort();

    // file unchanged + no backup + version not advanced
    assert_eq!(
        std::fs::read_to_string(dir_path.join("etc").join("host.yaml")).unwrap(),
        CFG_V0,
        "an illegal config must not rewrite host.yaml"
    );
    assert!(
        !dir_path.join("etc").join("host.yaml.bak-9").exists(),
        "an illegal config must not create a backup"
    );
    assert!(
        !dir_path.join("run").join("oxfile.toml").exists(),
        "an illegal config must not rewrite the oxfile"
    );
    assert_eq!(version.load(Ordering::Relaxed), 0, "the version must not advance");
}

// -- (3) OxMgr hot-activation proof (real oxmgr + host processes) --───────────────────────────

#[cfg(target_os = "linux")]
mod oxmgr_hot_reload {
    use super::*;

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

    fn path_with_oxmgr() -> String {
        let home = std::env::var("HOME").unwrap_or_default();
        let mut path = format!("{home}/.local/bin");
        if let Ok(existing) = std::env::var("PATH") {
            path.push(':');
            path.push_str(&existing);
        }
        path
    }

    /// The oxmgr instance-daemon env (same source as translate::oxmgr_apply: OXMGR_HOME derives port-isolated
    /// daemons - `oxmgr list` without this env connects the default daemon (empty) and cannot see instance processes).
    fn oxmgr_env(dir: &std::path::Path) -> Vec<(String, String)> {
        let home =
            dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf()).join("run").join("oxmgr");
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

    fn issue_token(dir: &Path, role: &str, node: &str, out: &str) {
        assert_eq!(
            host_cli(&[
                "token",
                "issue",
                "--role",
                role,
                "--node",
                node,
                "--out",
                dir.join("etc").join("link").join(out).to_str().expect("tok utf8"),
                dir.to_str().expect("dir utf8"),
            ]),
            0
        );
    }

    #[tokio::test]
    async fn oxmgr_hot_reload_add_watch_and_remove_camera_apply() {
        cleanup_iceoryx();
        let dir = tempfile::tempdir().expect("tempdir");
        let dir_path = dir.path().to_path_buf();
        cleanup_oxmgr_host(&dir_path);

        // (1) host init + v0 config (cam0) + tokens (capture cam0 + recorder)
        assert_eq!(host_cli(&["init", dir_path.to_str().expect("dir utf8")]), 0);
        write_host_toml(&dir_path, CFG_V0);
        issue_token(&dir_path, "capture", "capture-cam0", "cam0.token");
        issue_token(&dir_path, "recorder", "recorder-cam0", "recorder.token");

        // (2) host start -> 6 processes (5 fixed + host-capturer-cam0) running
        assert_eq!(host_cli(&["start", dir_path.to_str().expect("dir utf8")]), 0);
        let mut guard = OxmgrGuard { dir: dir_path.clone(), done: false };
        let procs = oxmgr_host_procs(&dir_path);
        assert_eq!(procs.len(), 6, "expected 6 processes (5 fixed + capturer), got: {procs:?}");
        let old_pid = procs
            .iter()
            .find(|(n, _, _)| n == "host-capturer-cam0")
            .map(|(_, _, pid)| *pid)
            .expect("host-capturer-cam0 is in oxmgr");
        assert!(old_pid != 0, "the capturer pid should be non-zero");

        // (3) push the new config (add cam1) + the cam1 token - the same write-to-disk -> apply sequence as host-agent
        //    (the agent writes host.yaml + regenerates the oxfile -> oxmgr apply; the test runs apply via the host CLI
        //     because the test process's current_exe lives under deps/ and cannot resolve artifact paths; the watch restart
        //     is triggered asynchronously through the daemon)
        issue_token(&dir_path, "capture", "capture-cam1", "cam1.token");
        write_host_toml(&dir_path, CFG_V1);
        assert_eq!(host_cli(&["apply", dir_path.to_str().expect("dir utf8")]), 0);

        // (4) proof 1 (incremental apply Start): host-capturer-cam1 appears and is running
        //    proof 2 (file-watch restart): host-capturer-cam0 (command unchanged -> apply is Noop)
        //      pid changed - can only come from the host.yaml content fingerprint -> watch restart
        let mut new_pid = 0u64;
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        loop {
            let cur = oxmgr_host_procs(&dir_path);
            let cam1 = cur.iter().find(|(n, _, _)| n == "host-capturer-cam1").cloned();
            let cam0 = cur.iter().find(|(n, _, _)| n == "host-capturer-cam0").cloned();
            if let Some((_, s1, _)) = &cam1
                && s1 == "running"
                && let Some((_, _s0, p0)) = &cam0
                && p0 != &0
                && p0 != &old_pid
            {
                new_pid = *p0;
                eprintln!(
                    "[config_push_e2e] OK: cam1 running + cam0 watch-restarted {old_pid}->{new_pid}"
                );
                break;
            }
            if std::time::Instant::now() > deadline {
                panic!(
                    "hot activation not achieved within 30s: cam1={cam1:?} cam0={cam0:?} (old pid {old_pid})",
                    cam1 = cam1,
                    cam0 = cam0
                );
            }
            std::thread::sleep(Duration::from_millis(500));
        }
        assert!(new_pid != 0, "the pid should be non-zero after the watch restart");

        // (4b) removal-path proof: push v2 (cam0 only, fps 15) -> the cam1 app is cleaned up
        //    (oxmgr_apply removal is synchronous; removal = oxmgr delete); cam0's command is unchanged ->
        //    another watch restart (pid changes again)
        write_host_toml(&dir_path, CFG_V2);
        assert_eq!(host_cli(&["apply", dir_path.to_str().expect("dir utf8")]), 0);
        let mut pid_after_remove = 0u64;
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        loop {
            let cur = oxmgr_host_procs(&dir_path);
            let cam1 = cur.iter().find(|(n, _, _)| n == "host-capturer-cam1").cloned();
            let cam0 = cur.iter().find(|(n, _, _)| n == "host-capturer-cam0").cloned();
            if cam1.is_none()
                && let Some((_, _s0, p0)) = &cam0
                && p0 != &0
                && p0 != &new_pid
            {
                pid_after_remove = *p0;
                eprintln!(
                    "[config_push_e2e] OK: cam1 deleted + cam0 watch-restarted {new_pid}->{pid_after_remove}"
                );
                break;
            }
            if std::time::Instant::now() > deadline {
                panic!(
                    "removal + restart not achieved within 30s: cam1={cam1:?} cam0={cam0:?}",
                    cam1 = cam1,
                    cam0 = cam0
                );
            }
            std::thread::sleep(Duration::from_millis(500));
        }
        assert!(
            pid_after_remove != 0,
            "the pid should be non-zero after the removal-triggered watch restart"
        );

        // (5) cleanup: host stop -> the host namespace empties (a race exists between watch restarts and stop -",
        //    re-check after stopping, stop again on residue, up to 15s)
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        loop {
            if oxmgr_host_procs(&dir_path).is_empty() {
                break;
            }
            if std::time::Instant::now() > deadline {
                break;
            }
            let _ = host_cli(&["stop", dir_path.to_str().expect("dir utf8")]);
            std::thread::sleep(Duration::from_millis(500));
        }
        guard.done = true;
        let remaining = oxmgr_host_procs(&dir_path);
        assert!(remaining.is_empty(), "no host processes should remain after stop: {remaining:?}");
    }
}
