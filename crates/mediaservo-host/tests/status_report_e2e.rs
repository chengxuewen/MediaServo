//! Task E3: StatusReport upload e2e - host-agent gateway + status reporter task -> mock server.
//!
//! Covers: (1) after the gateway joins remotely, the reporter periodically aggregates three snapshots -> StatusReport upstream;
//! (2) content assertions: room_id/ts/config_version + topology processes (expected union) + signaling plane
//! (remote_connected/peer_id) + empty stream sections (no token -> FlowMonitor absent).
//!
//! Prerequisite (C25): `rm -rf /tmp/iceoryx2 /dev/shm/iox2_*` (FrameBus discovery residue).

use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use mediaservo_common::protocol::{PeerRole, SignalingMessage};
use mediaservo_host::gateway::{GatewayConfig, run_gateway};
use mediaservo_host::monitor::signal::spawn_status_reporter;
use mediaservo_link::RetryConfig;
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;

const VEHICLE_ROOM: &str = "vehicle-1";
const VEHICLE_PEER: &str = "veh-peer";
const HOST_TOML: &str = r#"
sources:
  - id: "cam0"
    mode: "generator"
    fps: 30
"#;

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
    let room = match serde_json::from_str::<SignalingMessage>(join.to_text().unwrap())
        .expect("parse RoomJoin")
    {
        SignalingMessage::RoomJoin { room_id, peer_role, .. } => {
            assert_eq!(peer_role, PeerRole::Host);
            room_id
        }
        other => panic!("expected RoomJoin, got {other:?}"),
    };
    ws.send(Message::Text(
        serde_json::to_string(&SignalingMessage::RoomJoined {
            room_id: room.clone(),
            peer_id: VEHICLE_PEER.into(),
            protocol: None,
            server_version: None,
            session_nonce: None,
        })
        .unwrap(),
    ))
    .await
    .unwrap();
    ws
}

#[tokio::test]
async fn status_report_reaches_mock_server_with_expected_content() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let mut ws = mock_handshake(&listener).await;
        // The first app-layer message after joining should be StatusReport (the reporter's first cycle)
        let msg = tokio::time::timeout(Duration::from_secs(10), ws.next())
            .await
            .expect("StatusReport should arrive within 10s")
            .expect("ws closed")
            .expect("ws error");
        let msg: SignalingMessage =
            serde_json::from_str(msg.to_text().unwrap()).expect("StatusReport parses");
        match msg {
            SignalingMessage::StatusReport {
                room_id,
                topics,
                streams,
                processes,
                signal,
                ts,
                config_version,
            } => {
                assert_eq!(
                    room_id, VEHICLE_ROOM,
                    "the reported room_id should be the vehicle room"
                );
                assert!(
                    topics.is_empty(),
                    "no token -> the stream section should be empty: {topics:?}"
                );
                assert!(
                    streams.is_empty(),
                    "no token -> the stream section should be empty: {streams:?}"
                );
                assert!(
                    processes.iter().any(|p| p.name == "host-agent" && p.expected),
                    "the fixed process host-agent should be in the expected union: {processes:?}"
                );
                assert!(
                    processes.iter().any(|p| p.name == "host-capturer-cam0" && p.expected),
                    "host.yaml video-source instances should be in the expected union (single source: no id suffix): {processes:?}"
                );
                assert!(
                    signal.remote_connected,
                    "the gateway joined the room, remote should be connected"
                );
                assert_eq!(
                    signal.remote_peer_id, VEHICLE_PEER,
                    "peer_id should come from RoomJoined"
                );
                assert!(ts > 0, "ts should be unix seconds: {ts}");
                assert_eq!(config_version, 0, "config_version is always 0 before E4");
            }
            other => panic!("expected StatusReport, got {other:?}"),
        }
    });

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
    let (_port, handle) = run_gateway(cfg).await.unwrap();
    spawn_status_reporter(
        HOST_TOML.into(),
        Instant::now(),
        None,
        handle,
        Duration::from_millis(300),
        std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
        None, // T1b3: no ledger dir -> telemetry monitors stay pinned (report-only test)
    );
    server.await.unwrap();
}
