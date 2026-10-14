//! Task E2: stream-flow monitor e2e - FrameBus subscription stats + streamer push state.
//!
//! (1) `flow_stats_from_simulated_publisher`: in-process simulated publisher at 30fps (ts_mono stepping)
//!   -> FlowSnapshot fps/bps correct (with tolerance) + stalled appears after publishing stops (data-plane fact,
//!   no grace);
//! (2) `streamer_stats_ingestion`: fake stats topic (published with the Recorder role - the Pusher one-way
//!   contract forbids publishing stats; the streamer token defaults to Recorder, C2 legacy; `StreamerStats` JSON)
//!   -> snapshot.streams reflects bytes_sent/frames_encoded/connected;
//! (3) `real_capturer_flow_then_stall`: real host-capturer binary -> monitor sees the flow
//!   (fps~30); kill the process -> stalled after the default 2s floor.
//!
//! Prerequisite (C25): `rm -rf /tmp/iceoryx2 /dev/shm/iox2_*`. Test topics include
//! `std::process::id()` to isolate concurrency, but cross-run residue still needs external cleanup.

use std::process::{Command, Stdio};
use std::time::Duration;

use mediaservo_host::monitor::flow::{FlowMonitor, StreamerStats};
use mediaservo_link::{
    CapabilityToken, Ed25519SigningKey, Ed25519VerifyingKey, FrameBus, FrameMeta, FrameTopic,
    NodeAcl, NodeId, Role, TokenFile,
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

/// (1) simulated publisher rate change -> frame-rate curve + stall warning.
#[tokio::test]
async fn flow_stats_from_simulated_publisher() {
    cleanup_iceoryx();
    let pid = std::process::id();
    let cam_id = format!("e2flow-{pid}");
    let host_toml =
        format!("sources:\n  - id: \"{cam_id}\"\n    mode: \"generator\"\n    fps: 30\n");

    // monitor attach (Monitor role -> subscribes camera/* + stats/*)
    let (mon_tok, mon_vk) = token(Role::Monitor, &format!("monitor-{pid}"));
    let monitor =
        FlowMonitor::attach_with_stall(host_toml, &mon_tok, &mon_vk, Duration::from_millis(300))
            .expect("monitor attach");

    // simulated publisher (Capture role): 30fps x 45 frames, ts_mono stepping 33.33ms (C17 monotonic)
    let (pub_tok, pub_vk) = token(Role::Capture, &format!("capture-{pid}"));
    let bus = FrameBus::attach("", &pub_tok, &pub_vk).expect("publisher attach");
    let topic = FrameTopic::new(format!("camera/{cam_id}"));
    let payload = vec![7u8; 100];
    let mut ts = 1_000_000_000u64;
    for _ in 0..45 {
        let meta = FrameMeta {
            seq: ts,
            width: 320,
            height: 240,
            format: 1,
            ts_mono_ns: ts,
            ..Default::default()
        };
        bus.publish(&topic, &payload, &meta).expect("publish frame");
        ts += 33_333_333;
        tokio::time::sleep(Duration::from_millis(33)).await;
    }
    // drain the monitor's drain task
    tokio::time::sleep(Duration::from_millis(250)).await;

    let snap = monitor.collect();
    let tf = snap
        .topics
        .iter()
        .find(|t| t.topic == topic.as_str())
        .unwrap_or_else(|| panic!("topic flow should be visible: {snap:#?}"));
    assert!(tf.frames >= 20, "many frames expected within the window, got {}", tf.frames);
    assert!(
        (tf.fps - 30.0).abs() < 3.0,
        "fps should be ~30 (ts_mono steps 33.33ms), got {}",
        tf.fps
    );
    assert!((1500..4500).contains(&tf.bps), "bps should be ~100B x 30 = 3000, got {}", tf.bps);
    assert!(!tf.stalled, "while publishing it must not be stalled: {snap:#?}");

    // stop publishing -> stalled after the short floor(300ms) (data-plane fact, no grace)
    tokio::time::sleep(Duration::from_millis(700)).await;
    let snap2 = monitor.collect();
    let tf2 =
        snap2.topics.iter().find(|t| t.topic == topic.as_str()).expect("topic flow still present");
    assert!(tf2.stalled, "should be stalled after stopping: {snap2:#?}");
    assert_eq!(tf2.frames, 0, "no new frames expected within the window");
    assert_eq!(tf2.fps, 0.0);
}

/// (2) streamer push-state ingestion: fake stats topic -> snapshot.streams reflects it.
#[tokio::test]
async fn streamer_stats_ingestion() {
    cleanup_iceoryx();
    let pid = std::process::id();
    let stream_id = format!("s{pid}");
    let host_toml =
        format!("streams:\n  - id: \"{stream_id}\"\n    source: \"cam0\"\n    codec: \"vp8\"\n");

    let (mon_tok, mon_vk) = token(Role::Monitor, &format!("monitor-{pid}"));
    let monitor = FlowMonitor::attach(host_toml, &mon_tok, &mon_vk).expect("monitor attach");

    // fake stats publisher (Recorder role -> publishes stats/*; the streamer token defaults to Recorder, C2 legacy)
    let (st_tok, st_vk) = token(Role::Recorder, &format!("streamer-{pid}"));
    let bus = FrameBus::attach("", &st_tok, &st_vk).expect("stats publisher attach");
    let topic = FrameTopic::new(format!("stats/stream-{stream_id}"));
    let stats = StreamerStats {
        bytes_sent: 12345,
        frames_encoded: 678,
        frame_width: 1280,
        frame_height: 720,
        codec: "h264".into(),
        avg_encode_ms: None,
        encoder_implementation: None,
    };
    let payload = serde_json::to_vec(&stats).expect("stats json");
    bus.publish(
        &topic,
        &payload,
        &FrameMeta { format: FrameMeta::FORMAT_JSON, ts_mono_ns: 1, ..Default::default() },
    )
    .expect("publish stats");
    tokio::time::sleep(Duration::from_millis(250)).await;

    let snap = monitor.collect();
    let sf = snap
        .streams
        .iter()
        .find(|s| s.id == stream_id)
        .unwrap_or_else(|| panic!("stream flow should be visible: {snap:#?}"));
    assert_eq!(sf.bytes_sent, 12345, "bytes_sent should come from the stats JSON");
    assert_eq!(sf.frames_encoded, 678);
    assert_eq!(sf.frame_width, 1280);
    assert_eq!(sf.frame_height, 720);
    assert!(sf.connected, "just receiving stats should mean connected: {snap:#?}");
}

/// (3) integration: real capturer running -> monitor sees the flow; kill -> stalled appears.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn real_capturer_flow_then_stall() {
    cleanup_iceoryx();
    let pid = std::process::id();
    let cam_id = format!("e2cap-{pid}");
    let dir = tempfile::tempdir().expect("tempdir");
    let host_toml =
        format!("sources:\n  - id: \"{cam_id}\"\n    mode: \"generator\"\n    fps: 30\n");
    let cfg_path = dir.path().join("host.yaml");
    std::fs::write(&cfg_path, &host_toml).expect("write host.yaml");

    // capturer token file (Capture role)
    let (cap_tok, cap_vk) = token(Role::Capture, &format!("capture-{pid}"));
    let tok_path = dir.path().join("cap.token");
    std::fs::write(&tok_path, TokenFile::encode(&cap_tok, &cap_vk)).expect("write token file");

    // monitor attaches first (flow visible as soon as the capturer starts)
    let (mon_tok, mon_vk) = token(Role::Monitor, &format!("monitor-{pid}"));
    let monitor = FlowMonitor::attach(host_toml, &mon_tok, &mon_vk).expect("monitor attach");

    let log = tempfile::NamedTempFile::new().expect("log file");
    let mut child = Command::new(env!("CARGO_BIN_EXE_host-capturer"))
        .args([
            "--camera",
            &cam_id,
            "--config",
            cfg_path.to_str().expect("cfg path utf8"),
            "--token",
            tok_path.to_str().expect("token path utf8"),
        ])
        .stdout(Stdio::from(log.reopen().expect("reopen log")))
        .stderr(Stdio::from(log.reopen().expect("reopen log")))
        .spawn()
        .expect("spawn host-capturer");

    // 3s of frame production -> flow visible
    tokio::time::sleep(Duration::from_secs(3)).await;
    let snap = monitor.collect();
    let tf =
        snap.topics.iter().find(|t| t.topic == format!("camera/{cam_id}")).unwrap_or_else(|| {
            panic!("the real capturer's topic flow should be visible: {snap:#?}")
        });
    assert!(
        tf.frames > 20,
        "many frames expected in the real capturer's window, got {}",
        tf.frames
    );
    assert!(
        (tf.fps - 30.0).abs() < 8.0,
        "real capturer fps should be ~30 (startup jitter tolerance), got {}",
        tf.fps
    );
    assert!(!tf.stalled, "while the capturer runs it must not be stalled: {snap:#?}");

    // kill the capturer -> stalled after the default 2s floor
    child.kill().expect("kill capturer");
    let _ = child.wait();
    tokio::time::sleep(Duration::from_secs(3)).await;
    let snap2 = monitor.collect();
    let tf2 = snap2
        .topics
        .iter()
        .find(|t| t.topic == format!("camera/{cam_id}"))
        .expect("topic flow still present");
    assert!(tf2.stalled, "should be stalled after the capturer is killed: {snap2:#?}");
    assert_eq!(tf2.frames, 0, "no frames expected within the window after the kill");
}
