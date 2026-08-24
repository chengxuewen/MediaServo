//! Task E2: 数据流监控 e2e — FrameBus 订阅统计 + streamer 推流状态。
//!
//! ① `flow_stats_from_simulated_publisher`: 进程内模拟发布者 30fps（ts_mono 步进）
//!   → FlowSnapshot fps/bps 正确（容差）+ 停止发布后 stalled 出现（数据面事实，
//!   无 grace）；
//! ② `streamer_stats_ingestion`: 假 stats topic（Recorder 角色发布——Pusher 单方向
//!   契约不可发布 stats，streamer 令牌缺省 Recorder，C2 遗留；`StreamerStats` JSON）
//!   → snapshot.streams 反映 bytes_sent/frames_encoded/connected；
//! ③ `real_capturer_flow_then_stall`: 真实 host-capturer 二进制 → monitor 见流
//!   （fps≈30）；杀进程 → 默认 2s floor 后 stalled。
//!
//! 前置（C25）: `rm -rf /tmp/iceoryx2 /dev/shm/iox2_*`。测试内 topic 含
//! `std::process::id()` 隔离并发，但跨 run 残留仍需外部清理。

use std::process::{Command, Stdio};
use std::time::Duration;

use mediaservo_host::monitor::flow::{FlowMonitor, StreamerStats};
use mediaservo_link::{
    CapabilityToken, Ed25519SigningKey, Ed25519VerifyingKey, FrameBus, FrameMeta, FrameTopic,
    NodeAcl, NodeId, Role, TokenFile,
};

// 测试用 Ed25519 密钥对（openssl 生成，仅测试用；与 link/deck/capturer 测试同源）。
const PRIV_PEM: &str = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIObCg8b+Le6kKOI/+pE+4+YhXUlr6X6h7q8p/MjvHmXT\n-----END PRIVATE KEY-----\n";
const PUB_PEM: &str = "-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VwAyEAgXprEbnahCZoZtLpiUqR0ruqtzEfRXk/Gl/6F6PEm4o=\n-----END PUBLIC KEY-----\n";

fn token(role: Role, node_id: &str) -> (CapabilityToken, Ed25519VerifyingKey) {
    let sk = Ed25519SigningKey::from_pem(PRIV_PEM.as_bytes());
    let vk = Ed25519VerifyingKey::from_pem(PUB_PEM.as_bytes());
    let acl = NodeAcl::for_role(NodeId::new(node_id), role);
    (CapabilityToken::sign(&acl, 3600, &sk).unwrap(), vk)
}

/// C25: 清 iceoryx2 0.9.3 运行时残留（/tmp/iceoryx2 + /dev/shm/iox2_*）。
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

/// ① 模拟发布者变速 → 帧率曲线 + 停滞告警。
#[tokio::test]
async fn flow_stats_from_simulated_publisher() {
    cleanup_iceoryx();
    let pid = std::process::id();
    let cam_id = format!("e2flow-{pid}");
    let host_toml = format!("sources:\n  - id: \"{cam_id}\"\n    mode: \"generator\"\n    fps: 30\n");

    // monitor attach（Monitor 角色 → camera/* + stats/* 订阅）
    let (mon_tok, mon_vk) = token(Role::Monitor, &format!("monitor-{pid}"));
    let monitor =
        FlowMonitor::attach_with_stall(host_toml, &mon_tok, &mon_vk, Duration::from_millis(300))
            .expect("monitor attach");

    // 模拟发布者（Capture 角色）：30fps × 45 帧，ts_mono 33.33ms 步进（C17 单调）
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
    // 排空 monitor 的 drain 任务
    tokio::time::sleep(Duration::from_millis(250)).await;

    let snap = monitor.collect();
    let tf = snap
        .topics
        .iter()
        .find(|t| t.topic == topic.as_str())
        .unwrap_or_else(|| panic!("应见 topic flow: {snap:#?}"));
    assert!(tf.frames >= 20, "窗口内应有大量帧, got {}", tf.frames);
    assert!(
        (tf.fps - 30.0).abs() < 3.0,
        "fps 应≈30（ts_mono 步进 33.33ms）, got {}",
        tf.fps
    );
    assert!(
        (1500..4500).contains(&tf.bps),
        "bps 应≈100B×30=3000, got {}",
        tf.bps
    );
    assert!(!tf.stalled, "发布中不应 stalled: {snap:#?}");

    // 停止发布 → 短 floor(300ms) 后 stalled（数据面事实，无 grace）
    tokio::time::sleep(Duration::from_millis(700)).await;
    let snap2 = monitor.collect();
    let tf2 = snap2
        .topics
        .iter()
        .find(|t| t.topic == topic.as_str())
        .expect("topic flow 仍在");
    assert!(tf2.stalled, "停止后应 stalled: {snap2:#?}");
    assert_eq!(tf2.frames, 0, "窗口内应无新帧");
    assert_eq!(tf2.fps, 0.0);
}

/// ② streamer 推流状态摄入：假 stats topic → snapshot.streams 反映。
#[tokio::test]
async fn streamer_stats_ingestion() {
    cleanup_iceoryx();
    let pid = std::process::id();
    let stream_id = format!("s{pid}");
    let host_toml = format!(
        "streams:\n  - id: \"{stream_id}\"\n    source: \"cam0\"\n    codec: \"vp8\"\n"
    );

    let (mon_tok, mon_vk) = token(Role::Monitor, &format!("monitor-{pid}"));
    let monitor = FlowMonitor::attach(host_toml, &mon_tok, &mon_vk).expect("monitor attach");

    // 假 stats 发布者（Recorder 角色 → stats/* 发布；streamer 令牌缺省 Recorder，C2 遗留）
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
        &FrameMeta {
            format: FrameMeta::FORMAT_JSON,
            ts_mono_ns: 1,
            ..Default::default()
        },
    )
    .expect("publish stats");
    tokio::time::sleep(Duration::from_millis(250)).await;

    let snap = monitor.collect();
    let sf = snap
        .streams
        .iter()
        .find(|s| s.id == stream_id)
        .unwrap_or_else(|| panic!("应见 stream flow: {snap:#?}"));
    assert_eq!(sf.bytes_sent, 12345, "bytes_sent 应来自 stats JSON");
    assert_eq!(sf.frames_encoded, 678);
    assert_eq!(sf.frame_width, 1280);
    assert_eq!(sf.frame_height, 720);
    assert!(sf.connected, "刚收到 stats 应 connected: {snap:#?}");
}

/// ③ 集成：真实 capturer 运行 → monitor 见流；杀 → stalled 出现。
#[cfg(target_os = "linux")]
#[tokio::test]
async fn real_capturer_flow_then_stall() {
    cleanup_iceoryx();
    let pid = std::process::id();
    let cam_id = format!("e2cap-{pid}");
    let dir = tempfile::tempdir().expect("tempdir");
    let host_toml = format!("sources:\n  - id: \"{cam_id}\"\n    mode: \"generator\"\n    fps: 30\n");
    let cfg_path = dir.path().join("host.yaml");
    std::fs::write(&cfg_path, &host_toml).expect("write host.yaml");

    // capturer 令牌文件（Capture 角色）
    let (cap_tok, cap_vk) = token(Role::Capture, &format!("capture-{pid}"));
    let tok_path = dir.path().join("cap.token");
    std::fs::write(&tok_path, TokenFile::encode(&cap_tok, &cap_vk)).expect("write token file");

    // monitor 先 attach（capturer 起后即见流）
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

    // 3s 产帧 → 见流
    tokio::time::sleep(Duration::from_secs(3)).await;
    let snap = monitor.collect();
    let tf = snap
        .topics
        .iter()
        .find(|t| t.topic == format!("camera/{cam_id}"))
        .unwrap_or_else(|| panic!("应见真实 capturer 的 topic flow: {snap:#?}"));
    assert!(tf.frames > 20, "真实 capturer 窗口内应有大量帧, got {}", tf.frames);
    assert!(
        (tf.fps - 30.0).abs() < 8.0,
        "真实 capturer fps 应≈30（启动抖动容差）, got {}",
        tf.fps
    );
    assert!(!tf.stalled, "capturer 运行中不应 stalled: {snap:#?}");

    // 杀 capturer → 默认 2s floor 后 stalled
    child.kill().expect("kill capturer");
    let _ = child.wait();
    tokio::time::sleep(Duration::from_secs(3)).await;
    let snap2 = monitor.collect();
    let tf2 = snap2
        .topics
        .iter()
        .find(|t| t.topic == format!("camera/{cam_id}"))
        .expect("topic flow 仍在");
    assert!(tf2.stalled, "capturer 被杀后应 stalled: {snap2:#?}");
    assert_eq!(tf2.frames, 0, "杀后窗口内应无帧");
}
