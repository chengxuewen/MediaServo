//! MediaServo E2E integration tests.
//!
//! Exercises all crate boundaries: config parsing, metrics, protocol serialization,
//! PSK auth, room management, and a live axum health endpoint.

use mediaservo_common::auth::{AuthResult, PskAuthenticator, SimplePskAuth};
use mediaservo_common::config::{HostConfig, RemoteConfig, ServerConfig};
use mediaservo_common::metrics::CoreMetrics;
use mediaservo_common::protocol::{PeerRole, SignalingMessage};

// ── Config parsing ───────────────────────────────────────────────────────────

#[test]
fn parse_host_config_minimal() {
    let yaml = r#"
version: 1
server:
  signaling_url: "ws://10.0.0.1:9800/ws"
capture:
  source: "screen"
encoder:
  backend: "auto"
"#;
    let config: HostConfig = serde_yaml::from_str(yaml).expect("valid yaml");
    assert_eq!(config.version, 1);
    assert_eq!(config.server.signaling_url, "ws://10.0.0.1:9800/ws");
    assert!(config.server.ice_servers.is_empty());
    assert_eq!(config.capture.source, "screen");
    assert_eq!(config.capture.resolution, "1280x720"); // default
    assert_eq!(config.capture.framerate, 30); // default
    assert!(config.capture.device.is_none());
    assert_eq!(config.encoder.backend, "auto");
}

#[test]
fn parse_host_config_full() {
    let yaml = r#"
version: 1
server:
  signaling_url: "wss://relay.example.com:9801/ws"
  ice_servers:
    - "stun:stun.l.google.com:19302"
    - "turn:turn.example.com:3478"
capture:
  source: "camera"
  resolution: "1920x1080"
  framerate: 60
  device: "/dev/video0"
encoder:
  backend: "nvenc"
  bitrate_kbps: 4000
  keyframe_interval: 120
webrtc:
  ice_timeout_secs: 45
psk: "my-pre-shared-key"
"#;
    let config: HostConfig = serde_yaml::from_str(yaml).expect("valid yaml");
    assert_eq!(config.server.ice_servers.len(), 2);
    assert_eq!(config.capture.resolution, "1920x1080");
    assert_eq!(config.capture.framerate, 60);
    assert_eq!(config.capture.device.as_deref(), Some("/dev/video0"));
    assert_eq!(config.encoder.bitrate_kbps, 4000);
    assert_eq!(config.encoder.keyframe_interval, 120);
    let webrtc_cfg = config.webrtc.expect("should have webrtc section");
    assert_eq!(webrtc_cfg.ice_timeout_secs, 45);
    assert_eq!(config.psk.as_deref(), Some("my-pre-shared-key"));
}

#[test]
fn parse_server_config_minimal() {
    let yaml = r#"
version: 1
listen:
  host: "0.0.0.0"
  port: 9800
"#;
    let config: ServerConfig = serde_yaml::from_str(yaml).expect("valid yaml");
    assert_eq!(config.listen.port, 9800);
    assert_eq!(config.room_capacity, 10); // default
    assert_eq!(config.rate_limit, 100); // default
    assert!(config.psk.is_none());
}

#[test]
fn parse_server_config_full() {
    let yaml = r#"
version: 1
listen:
  host: "127.0.0.1"
  port: 9800
room_capacity: 50
psk: "s3cret"
rate_limit: 200
"#;
    let config: ServerConfig = serde_yaml::from_str(yaml).expect("valid yaml");
    assert_eq!(config.room_capacity, 50);
    assert_eq!(config.psk.as_deref(), Some("s3cret"));
    assert_eq!(config.rate_limit, 200);
}

#[test]
fn parse_remote_config_minimal() {
    let yaml = r#"
version: 1
server:
  signaling_url: "ws://server:9800/ws"
"#;
    let config: RemoteConfig = serde_yaml::from_str(yaml).expect("valid yaml");
    assert_eq!(config.server.signaling_url, "ws://server:9800/ws");
    assert!(config.psk.is_none());
    assert!(config.render.is_none());
}

#[test]
fn parse_remote_config_with_render() {
    let yaml = r#"
version: 1
server:
  signaling_url: "ws://server:9800/ws"
psk: "key123"
render:
  backend: "metal"
"#;
    let config: RemoteConfig = serde_yaml::from_str(yaml).expect("valid yaml");
    let render = config.render.expect("should have render section");
    assert_eq!(render.backend, "metal");
}

// ── CoreMetrics counter/gauge/encode ──────────────────────────────────────────

#[test]
fn metrics_counter_and_gauge() {
    let m1 = CoreMetrics::new();
    let m2 = CoreMetrics::new();

    // Two independent instances should not interfere.
    m1.active_connections.inc();
    m1.active_connections.inc();
    m1.relayed_bytes.inc_by(1024);
    m1.signaling_latency_us.set(350);
    m1.error_count.inc();

    m2.active_connections.set(7);
    m2.relayed_bytes.inc_by(512);
    m2.signaling_latency_us.set(120);

    // m1 state unchanged by m2
    assert_eq!(m1.active_connections.get(), 2);
    assert_eq!(m1.relayed_bytes.get(), 1024);
    assert_eq!(m1.signaling_latency_us.get(), 350);
    assert_eq!(m1.error_count.get(), 1);

    // m2 state independent
    assert_eq!(m2.active_connections.get(), 7);
    assert_eq!(m2.relayed_bytes.get(), 512);
    assert_eq!(m2.signaling_latency_us.get(), 120);
    assert_eq!(m2.error_count.get(), 0);
}

#[test]
fn metrics_encode_contains_metric_names() {
    let m = CoreMetrics::new();
    m.active_connections.set(3);
    m.relayed_bytes.inc_by(2048);
    m.error_count.inc();

    let encoded = m.encode();
    assert!(encoded.contains("active_connections"), "encode: {}", encoded);
    assert!(encoded.contains("relayed_bytes"), "encode: {}", encoded);
    assert!(encoded.contains("signaling_latency_us"), "encode: {}", encoded);
    assert!(encoded.contains("error_count"), "encode: {}", encoded);
}

#[test]
fn metrics_gauge_default_zero() {
    let m = CoreMetrics::new();
    assert_eq!(m.active_connections.get(), 0);
    assert_eq!(m.signaling_latency_us.get(), 0);
    assert_eq!(m.relayed_bytes.get(), 0);
    assert_eq!(m.error_count.get(), 0);
}

// ── Protocol message round-trip (all SignalingMessage variants) ───────────────

#[test]
fn protocol_roundtrip_room_join() {
    let msg = SignalingMessage::RoomJoin {
        room_id: "room-1".into(),
        peer_role: PeerRole::Host,
        stream_id: None,
        device_id: None,
        device_secret: None,
    };
    let json = serde_json::to_string(&msg).unwrap();
    let parsed: SignalingMessage = serde_json::from_str(&json).unwrap();
    match parsed {
        SignalingMessage::RoomJoin { room_id, peer_role, .. } => {
            assert_eq!(room_id, "room-1");
            assert_eq!(peer_role, PeerRole::Host);
        }
        _ => panic!("expected RoomJoin"),
    }
}

#[test]
fn protocol_roundtrip_room_joined() {
    let msg = SignalingMessage::RoomJoined {
        room_id: "room-x".into(),
        peer_id: "peer-abc".into(),
    };
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains(r#""type":"room_joined""#));
    let parsed: SignalingMessage = serde_json::from_str(&json).unwrap();
    match parsed {
        SignalingMessage::RoomJoined { room_id, peer_id } => {
            assert_eq!(room_id, "room-x");
            assert_eq!(peer_id, "peer-abc");
        }
        _ => panic!("expected RoomJoined"),
    }
}

#[test]
fn protocol_roundtrip_room_leave() {
    let msg = SignalingMessage::RoomLeave {
        room_id: "room-x".into(),
        peer_id: "peer-abc".into(),
    };
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains(r#""type":"room_leave""#));
    let parsed: SignalingMessage = serde_json::from_str(&json).unwrap();
    match parsed {
        SignalingMessage::RoomLeave { room_id, peer_id } => {
            assert_eq!(room_id, "room-x");
            assert_eq!(peer_id, "peer-abc");
        }
        _ => panic!("expected RoomLeave"),
    }
}

#[test]
fn protocol_roundtrip_sdp() {
    let msg = SignalingMessage::Sdp {
        room_id: "room-2".into(),
        target: Some("peer-1".into()),
        sdp: "v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=-\r\n".into(),
    };
    let json = serde_json::to_string(&msg).unwrap();
    let parsed: SignalingMessage = serde_json::from_str(&json).unwrap();
    match parsed {
        SignalingMessage::Sdp {
            room_id,
            target,
            sdp,
        } => {
            assert_eq!(room_id, "room-2");
            assert_eq!(target.as_deref(), Some("peer-1"));
            assert!(sdp.starts_with("v=0"));
        }
        _ => panic!("expected Sdp"),
    }
}

#[test]
fn protocol_roundtrip_ice_candidate() {
    let msg = SignalingMessage::RTCIceCandidate {
        room_id: "r1".into(),
        target: None,
        candidate: "candidate:1 1 UDP 2130706431 10.0.0.1 8000 typ host".into(),
        sdp_mid: Some("0".into()),
        sdp_mline_index: Some(0),
    };
    let json = serde_json::to_string(&msg).unwrap();
    let parsed: SignalingMessage = serde_json::from_str(&json).unwrap();
    match parsed {
        SignalingMessage::RTCIceCandidate {
            sdp_mid,
            sdp_mline_index,
            ..
        } => {
            assert_eq!(sdp_mid.as_deref(), Some("0"));
            assert_eq!(sdp_mline_index, Some(0));
        }
        _ => panic!("expected RTCIceCandidate"),
    }
}

#[test]
fn protocol_roundtrip_error() {
    let msg = SignalingMessage::Error {
        code: 4003,
        message: "PSK authentication failed".into(),
    };
    let json = serde_json::to_string(&msg).unwrap();
    let parsed: SignalingMessage = serde_json::from_str(&json).unwrap();
    match parsed {
        SignalingMessage::Error { code, message } => {
            assert_eq!(code, 4003);
            assert_eq!(message, "PSK authentication failed");
        }
        _ => panic!("expected Error"),
    }
}

#[test]
fn protocol_deserialize_unknown_type_is_error() {
    // serde(tag="type") with externally-tagged enum: unknown tag is an error
    let result: Result<SignalingMessage, _> =
        serde_json::from_str(r#"{"type":"unknown","room_id":"r"}"#);
    assert!(result.is_err());
}

// ── Auth flow ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn auth_sign_then_verify_success() {
    let auth = SimplePskAuth::new("super-secret-key");
    let challenge = b"random-challenge-bytes-12345";
    let signature = auth.sign(challenge);

    let result = auth.verify_challenge(challenge, &signature).await.unwrap();
    assert_eq!(result, AuthResult::Success);
}

#[tokio::test]
async fn auth_verify_denied_wrong_key() {
    let correct = SimplePskAuth::new("correct-key");
    let attacker = SimplePskAuth::new("wrong-key");

    let challenge = b"server-challenge";
    let forged_sig = attacker.sign(challenge);

    let result = correct
        .verify_challenge(challenge, &forged_sig)
        .await
        .unwrap();
    assert_eq!(result, AuthResult::Denied);
}

#[tokio::test]
async fn auth_different_challenge_denied() {
    let auth = SimplePskAuth::new("shared-key");
    let sig = auth.sign(b"challenge-1");
    let result = auth.verify_challenge(b"challenge-2", &sig).await.unwrap();
    assert_eq!(result, AuthResult::Denied);
}

// ── RoomManager ───────────────────────────────────────────────────────────────

#[test]
fn room_manager_join_and_leave() {
    use mediaservo_server::room::RoomManager;

    let rm = RoomManager::new();
    assert_eq!(rm.active_rooms(), 0);

    // Host joins → creates room
    rm.join_room("room-1", "host-1", &PeerRole::Host).unwrap();
    assert_eq!(rm.active_rooms(), 1);
    assert_eq!(rm.get_peer_count(), 1);

    // Remote joins same room
    rm.join_room("room-1", "remote-1", &PeerRole::Remote)
        .unwrap();
    assert_eq!(rm.active_rooms(), 1);
    assert_eq!(rm.get_peer_count(), 2);

    // Second Host in same room → error RoomFull
    let result = rm.join_room("room-1", "host-2", &PeerRole::Host);
    assert!(result.is_err());
    assert_eq!(rm.get_peer_count(), 2); // unchanged

    // 全 SFU（2026-08-25）: 多 Remote（舱端 consumer）可共存——无 P2P single-slot
    rm.join_room("room-1", "remote-2", &PeerRole::Remote).unwrap();
    assert_eq!(rm.get_peer_count(), 3);

    // Leave host → room stays (remotes still there)
    rm.leave_room("room-1", "host-1");
    assert_eq!(rm.get_peer_count(), 2);

    // Leave remotes → room removed (empty)
    rm.leave_room("room-1", "remote-1");
    assert_eq!(rm.get_peer_count(), 1);
    rm.leave_room("room-1", "remote-2");
    assert_eq!(rm.active_rooms(), 0);
    assert_eq!(rm.get_peer_count(), 0);
}

#[test]
fn room_manager_multiple_rooms() {
    use mediaservo_server::room::RoomManager;

    let rm = RoomManager::new();
    rm.join_room("room-a", "h-a", &PeerRole::Host).unwrap();
    rm.join_room("room-b", "h-b", &PeerRole::Host).unwrap();
    rm.join_room("room-c", "h-c", &PeerRole::Host).unwrap();

    assert_eq!(rm.active_rooms(), 3);
    assert_eq!(rm.get_peer_count(), 3);

    rm.leave_room("room-b", "h-b");
    assert_eq!(rm.active_rooms(), 2);
    assert_eq!(rm.get_peer_count(), 2);
}

// ── Axum health endpoint (live server on port 0) ──────────────────────────────

#[tokio::test]
async fn e2e_server_health_endpoint() {
    use mediaservo_server::axum::Router;
    use mediaservo_server::axum::routing::get;
    use mediaservo_server::tokio::io::{AsyncReadExt, AsyncWriteExt};
    use mediaservo_server::tokio::net::TcpListener;
    use mediaservo_server::tokio::net::TcpStream;

    let app = Router::new().route(
        "/health",
        get(|| async { "OK" }),
    );

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        mediaservo_server::axum::serve(listener, app).await.unwrap();
    });

    // Brief wait for the server to start
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let mut stream = TcpStream::connect(addr).await.unwrap();
    stream
        .write_all(
            b"GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        )
        .await
        .unwrap();

    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();
    let response = String::from_utf8_lossy(&buf);

    assert!(
        response.contains("200 OK"),
        "expected 200 OK, got:\n{}",
        response
    );
    assert!(
        response.contains("OK"),
        "expected body 'OK', got:\n{}",
        response
    );
}

#[tokio::test]
async fn e2e_monitor_router_health_endpoint() {
    use mediaservo_server::monitor::monitor_router;
    use mediaservo_server::signaling::SignalingServer;
    use mediaservo_server::tokio::io::{AsyncReadExt, AsyncWriteExt};
    use mediaservo_server::tokio::net::TcpListener;
    use mediaservo_server::tokio::net::TcpStream;

    #[cfg(feature = "sfu-mediasoup")]
    let signaling = {
        let sfu = std::sync::Arc::new(
            mediaservo_server::sfu::SfuManager::new_with_port(
                mediaservo_server::sfu::random_udp_port(),
            )
            .await
            .unwrap());
        SignalingServer::new(sfu, 65536, None)
    };
    #[cfg(not(feature = "sfu-mediasoup"))]
    let signaling = SignalingServer::new(65536, None);
    let app = monitor_router(signaling);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        mediaservo_server::axum::serve(listener, app).await.unwrap();
    });

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let mut stream = TcpStream::connect(addr).await.unwrap();
    stream
        .write_all(
            b"GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        )
        .await
        .unwrap();

    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();
    let response = String::from_utf8_lossy(&buf);

    assert!(response.contains("200 OK"), "got:\n{}", response);
    assert!(response.contains("OK"), "got:\n{}", response);
}

#[tokio::test]
async fn e2e_monitor_router_stats_endpoint() {
    use mediaservo_server::monitor::monitor_router;
    use mediaservo_server::signaling::SignalingServer;
    use mediaservo_server::tokio::io::{AsyncReadExt, AsyncWriteExt};
    use mediaservo_server::tokio::net::TcpListener;
    use mediaservo_server::tokio::net::TcpStream;

    #[cfg(feature = "sfu-mediasoup")]
    let signaling = {
        let sfu = std::sync::Arc::new(
            mediaservo_server::sfu::SfuManager::new_with_port(
                mediaservo_server::sfu::random_udp_port(),
            )
            .await
            .unwrap());
        SignalingServer::new(sfu, 65536, None)
    };
    #[cfg(not(feature = "sfu-mediasoup"))]
    let signaling = SignalingServer::new(65536, None);
    let app = monitor_router(signaling);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        mediaservo_server::axum::serve(listener, app).await.unwrap();
    });

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let mut stream = TcpStream::connect(addr).await.unwrap();
    stream
        .write_all(
            b"GET /stats HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        )
        .await
        .unwrap();

    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();
    let response = String::from_utf8_lossy(&buf);

    assert!(response.contains("200 OK"), "got:\n{}", response);
    assert!(
        response.contains("active_rooms"),
        "stats should contain 'active_rooms', got:\n{}",
        response
    );
    assert!(
        response.contains("connected_peers"),
        "got:\n{}",
        response
    );
    assert!(
        response.contains("uptime_seconds"),
        "got:\n{}",
        response
    );
}

#[tokio::test]
async fn e2e_monitor_router_metrics_endpoint() {
    use mediaservo_server::monitor::monitor_router;
    use mediaservo_server::signaling::SignalingServer;
    use mediaservo_server::tokio::io::{AsyncReadExt, AsyncWriteExt};
    use mediaservo_server::tokio::net::TcpListener;
    use mediaservo_server::tokio::net::TcpStream;

    #[cfg(feature = "sfu-mediasoup")]
    let signaling = {
        let sfu = std::sync::Arc::new(
            mediaservo_server::sfu::SfuManager::new_with_port(
                mediaservo_server::sfu::random_udp_port(),
            )
            .await
            .unwrap());
        SignalingServer::new(sfu, 65536, None)
    };
    #[cfg(not(feature = "sfu-mediasoup"))]
    let signaling = SignalingServer::new(65536, None);
    let app = monitor_router(signaling);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        mediaservo_server::axum::serve(listener, app).await.unwrap();
    });

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let mut stream = TcpStream::connect(addr).await.unwrap();
    stream
        .write_all(
            b"GET /metrics HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        )
        .await
        .unwrap();

    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();
    let response = String::from_utf8_lossy(&buf);

    assert!(response.contains("200 OK"), "got:\n{}", response);
    assert!(
        response.contains("active_connections"),
        "metrics should contain metric names, got:\n{}",
        response
    );
}
