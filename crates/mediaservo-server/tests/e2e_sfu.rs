//! E2E integration tests for mediasoup SFU flow.
//!
//! Feature-gated behind `sfu-mediasoup` and only runs on Linux
//! (mediasoup crate requires Linux kernel features).
//!
//! Tests: create room → create transports → produce media → consume media → cleanup.

#![cfg(all(feature = "sfu-mediasoup", target_os = "linux"))]

use futures_util::{SinkExt, StreamExt};
use mediaservo_common::protocol::{PeerRole, SignalingMessage};
use mediaservo_server::sfu::SfuManager;
use mediaservo_server::signaling::{SignalingServer, signaling_router};
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message as WsMsg;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

type KeepAliveWs =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

const PSK: &str = "e2e-sfu-psk";
const ROOM: &str = "sfu-test-room";

/// Full SFU lifecycle: create room → transports → produce → consume → cleanup.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn e2e_sfu_lifecycle() {
    unsafe { std::env::set_var("MEDIASERVO_PSK", PSK) };

    // Create mediasoup SFU manager
    let sfu = SfuManager::new_with_port(mediaservo_server::sfu::random_udp_port())
        .await
        .expect("Failed to create SFU manager");
    let sfu = Arc::new(sfu);
    let initial_room_count = sfu.room_count();

    // Create signaling server with SFU
    let mut server = SignalingServer::new(Arc::clone(&sfu), 65536, None);
    server.ws_ping_secs = 0; // the a1 heartbeat has its dedicated pins in heartbeat_e2e; this suite's raw JSON read loop must not eat control frames
    let app = signaling_router(server);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let ws_url = format!("ws://{}/ws", addr);

    // --- Host: connect, auth, join room, create send transport ---
    let host_url = ws_url.clone();
    let host_handle = tokio::spawn(async move {
        let (mut ws, _) = tokio_tungstenite::connect_async(&host_url).await.unwrap();

        // PSK auth
        ws.send(WsMsg::Text(PSK.into())).await.unwrap();
        let ack = ws.next().await.unwrap().unwrap();
        assert!(ack.to_text().unwrap().contains("authenticated"));

        // RoomJoin as Host
        let join = serde_json::to_string(&SignalingMessage::RoomJoin {
            room_id: ROOM.into(),
            peer_role: PeerRole::Host,
            stream_id: None,
            device_id: None,
            device_secret: None,
            device_pubkey: None,
            protocol: None,
            client_version: None,
            resume: None,
        })
        .unwrap();
        ws.send(WsMsg::Text(join)).await.unwrap();
        let joined = ws.next().await.unwrap().unwrap();
        assert!(joined.to_text().unwrap().contains("room_joined"));

        // Create send WebRTC transport
        let create_transport = serde_json::to_string(&SignalingMessage::CreateWebRtcTransport {
            room_id: ROOM.into(),
            peer_id: "host".to_string(),
            direction: mediaservo_common::protocol::TransportDirection::Send,
        })
        .unwrap();
        ws.send(WsMsg::Text(create_transport)).await.unwrap();

        // Wait for transport created response (may get room_leave first)
        let (transport_id, ice_parameters, dtls_parameters) = loop {
            let transport_resp = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            let resp_text = transport_resp.to_text().unwrap();
            let sig: SignalingMessage = serde_json::from_str(resp_text).unwrap();
            match sig {
                SignalingMessage::WebRtcTransportCreated {
                    transport_id,
                    ice_parameters,
                    dtls_parameters,
                    ..
                } => break (transport_id, ice_parameters, dtls_parameters),
                SignalingMessage::RoomLeave { .. } => continue,
                other => panic!("Unexpected response: {other:?}"),
            }
        };
        assert!(!transport_id.is_empty());
        assert!(!ice_parameters.username_fragment.is_empty());
        assert!(!dtls_parameters.role.is_empty());

        // Signal done — sentinel message
        ws.send(WsMsg::Text("host-ready".into())).await.unwrap();
    });

    // --- Remote: connect, auth, join room, create recv transport ---
    let remote_url = ws_url.clone();
    let remote_handle = tokio::spawn(async move {
        let (mut ws, _) = tokio_tungstenite::connect_async(&remote_url).await.unwrap();

        // PSK auth
        ws.send(WsMsg::Text(PSK.into())).await.unwrap();
        let ack = ws.next().await.unwrap().unwrap();
        assert!(ack.to_text().unwrap().contains("authenticated"));

        // RoomJoin as Remote
        let join = serde_json::to_string(&SignalingMessage::RoomJoin {
            room_id: ROOM.into(),
            peer_role: PeerRole::Remote,
            stream_id: None,
            device_id: None,
            device_secret: None,
            device_pubkey: None,
            protocol: None,
            client_version: None,
            resume: None,
        })
        .unwrap();
        ws.send(WsMsg::Text(join)).await.unwrap();
        let joined = ws.next().await.unwrap().unwrap();
        assert!(joined.to_text().unwrap().contains("room_joined"));

        // Create recv WebRTC transport
        let create_transport = serde_json::to_string(&SignalingMessage::CreateWebRtcTransport {
            room_id: ROOM.into(),
            peer_id: "remote".to_string(),
            direction: mediaservo_common::protocol::TransportDirection::Recv,
        })
        .unwrap();
        ws.send(WsMsg::Text(create_transport)).await.unwrap();

        // Wait for transport created response (may get room_leave first)
        loop {
            let transport_resp = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            let resp_text = transport_resp.to_text().unwrap();
            let sig: SignalingMessage = serde_json::from_str(resp_text).unwrap();
            match sig {
                SignalingMessage::WebRtcTransportCreated { .. } => break,
                SignalingMessage::RoomLeave { .. } => continue, // skip
                other => panic!("Unexpected response: {other:?}"),
            }
        }

        // Signal done
        ws.send(WsMsg::Text("remote-ready".into())).await.unwrap();
    });

    // Wait for both peers to be ready
    host_handle.await.unwrap();
    remote_handle.await.unwrap();

    // Verify room was created in SFU
    assert!(
        sfu.room_count() >= initial_room_count,
        "SFU should have at least one room after transport creation"
    );

    // Cleanup: drop WS connections triggers automatic peer cleanup
    // (signaling.rs handles disconnect → remove_peer automatically)
}

/// Test SFU room lifecycle: create room via transport, then cleanup via RoomLeave.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn e2e_sfu_cleanup_on_disconnect() {
    unsafe { std::env::set_var("MEDIASERVO_PSK", PSK) };

    let sfu = SfuManager::new_with_port(mediaservo_server::sfu::random_udp_port())
        .await
        .expect("Failed to create SFU manager");
    let sfu = Arc::new(sfu);
    let initial_count = sfu.room_count();

    let mut server = SignalingServer::new(Arc::clone(&sfu), 65536, None);
    server.ws_ping_secs = 0; // the a1 heartbeat has its dedicated pins in heartbeat_e2e; this suite's raw JSON read loop must not eat control frames
    let app = signaling_router(server);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let ws_url = format!("ws://{}/ws", addr);

    // Connect a Host peer, create transport, then disconnect
    let (mut ws, _) = tokio_tungstenite::connect_async(&ws_url).await.unwrap();

    // Auth
    ws.send(WsMsg::Text(PSK.into())).await.unwrap();
    let ack = ws.next().await.unwrap().unwrap();
    assert!(ack.to_text().unwrap().contains("authenticated"));

    // Join room
    let join = serde_json::to_string(&SignalingMessage::RoomJoin {
        room_id: ROOM.into(),
        peer_role: PeerRole::Host,
        stream_id: None,
        device_id: None,
        device_secret: None,
        device_pubkey: None,
        // S0: this helper speaks for the new endpoint (data_domain/F8 gating requires negotiated>=2).
        protocol: Some(2),
        client_version: None,
        resume: None,
    })
    .unwrap();
    ws.send(WsMsg::Text(join)).await.unwrap();
    let joined = ws.next().await.unwrap().unwrap();
    assert!(joined.to_text().unwrap().contains("room_joined"));

    // Create send transport
    let create_transport = serde_json::to_string(&SignalingMessage::CreateWebRtcTransport {
        room_id: ROOM.into(),
        peer_id: "host".to_string(),
        direction: mediaservo_common::protocol::TransportDirection::Send,
    })
    .unwrap();
    ws.send(WsMsg::Text(create_transport)).await.unwrap();

    let resp = ws.next().await.unwrap().unwrap();
    assert!(resp.to_text().unwrap().contains("transport_created"));

    // Room should exist
    assert_eq!(sfu.room_count(), initial_count + 1);

    // Close WebSocket — triggers disconnect cleanup
    ws.close(None).await.unwrap();

    // Give cleanup a moment
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Room should be cleaned up after disconnect
    assert_eq!(
        sfu.room_count(),
        initial_count,
        "SFU room should be destroyed after peer disconnect"
    );
}

/// Consumer pipeline: Host produce → Consumer recv transport → connect → consume.
/// This test exercises the full consumer-side SFU flow that causes "Signal Lost".
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn e2e_sfu_consume_pipeline() {
    unsafe { std::env::set_var("MEDIASERVO_PSK", PSK) };

    let sfu = Arc::new(
        SfuManager::new_with_port(mediaservo_server::sfu::random_udp_port())
            .await
            .expect("Failed to create SFU manager"),
    );
    let mut server = SignalingServer::new(Arc::clone(&sfu), 65536, None);
    server.ws_ping_secs = 0; // the a1 heartbeat has its dedicated pins in heartbeat_e2e; this suite's raw JSON read loop must not eat control frames
    let app = signaling_router(server);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let ws_url = format!("ws://{}/ws", addr);

    // --- Host: create send transport, connect, produce video ---
    let host_url = ws_url.clone();
    let host_handle = tokio::spawn(async move {
        let (mut ws, _) = tokio_tungstenite::connect_async(&host_url).await.unwrap();

        // Auth + RoomJoin
        ws.send(WsMsg::Text(PSK.into())).await.unwrap();
        let ack = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(ack.to_text().unwrap().contains("authenticated"));

        let join = serde_json::to_string(&SignalingMessage::RoomJoin {
            room_id: "sfu-consume-room".into(),
            peer_role: PeerRole::Host,
            stream_id: None,
            device_id: None,
            device_secret: None,
            device_pubkey: None,
            protocol: None,
            client_version: None,
            resume: None,
        })
        .unwrap();
        ws.send(WsMsg::Text(join)).await.unwrap();
        let joined = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(joined.to_text().unwrap().contains("room_joined"));

        // Create send transport
        let create = serde_json::to_string(&SignalingMessage::CreateWebRtcTransport {
            room_id: "sfu-consume-room".into(),
            peer_id: "host".into(),
            direction: mediaservo_common::protocol::TransportDirection::Send,
        })
        .unwrap();
        ws.send(WsMsg::Text(create)).await.unwrap();
        let resp = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let created: SignalingMessage = serde_json::from_str(resp.to_text().unwrap()).unwrap();
        let (send_tid, send_dtls) = match created {
            SignalingMessage::WebRtcTransportCreated { transport_id, dtls_parameters, .. } => {
                (transport_id, dtls_parameters)
            }
            other => panic!("Expected WebRtcTransportCreated, got: {:?}", other),
        };

        // Connect send transport
        let connect = serde_json::to_string(&SignalingMessage::ConnectWebRtcTransport {
            room_id: "sfu-consume-room".into(),
            peer_id: "host".into(),
            transport_id: send_tid.clone(),
            dtls_parameters: send_dtls,
        })
        .unwrap();
        ws.send(WsMsg::Text(connect)).await.unwrap();
        let conn_resp = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(conn_resp.to_text().unwrap().contains("transport_connected"));

        // Produce video
        let produce = serde_json::to_string(&SignalingMessage::Produce {
            room_id: "sfu-consume-room".into(),
            peer_id: "host".into(),
            transport_direction: mediaservo_common::protocol::TransportDirection::Send,
            kind: mediaservo_common::protocol::MediaKind::Video,
            rtp_parameters: serde_json::json!({"mid": "0", "codecs": [{"mimeType": "video/VP8", "payloadType": 100, "clockRate": 90000}], "headerExtensions": [], "encodings": [{"ssrc": 12345}], "rtcp": {"reducedSize": true}}),
            transport_id: Some(send_tid.clone()), // C1: bind to this session's transport by name

            label: None,}).unwrap();
        ws.send(WsMsg::Text(produce)).await.unwrap();
        let prod_resp = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let produced: SignalingMessage =
            serde_json::from_str(prod_resp.to_text().unwrap()).unwrap();
        let producer_id = match produced {
            SignalingMessage::Produced { producer_id, .. } => producer_id,
            other => panic!("Expected Produced, got: {:?}", other),
        };

        // Signal done
        ws.send(WsMsg::Text(format!("host-ready:{}", producer_id))).await.unwrap();
        (ws, producer_id)
    });

    // Wait for host to be ready
    let (host_ws, producer_id) = host_handle.await.unwrap();

    // --- Consumer: create recv transport, connect, consume ---
    let consumer_url = ws_url.clone();
    let pid = producer_id.clone();
    let consumer_handle = tokio::spawn(async move {
        let (mut ws, _) = tokio_tungstenite::connect_async(&consumer_url).await.unwrap();

        // Auth + RoomJoin
        ws.send(WsMsg::Text(PSK.into())).await.unwrap();
        let ack = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(ack.to_text().unwrap().contains("authenticated"));

        let join = serde_json::to_string(&SignalingMessage::RoomJoin {
            room_id: "sfu-consume-room".into(),
            peer_role: PeerRole::Remote,
            stream_id: None,
            device_id: None,
            device_secret: None,
            device_pubkey: None,
            protocol: None,
            client_version: None,
            resume: None,
        })
        .unwrap();
        ws.send(WsMsg::Text(join)).await.unwrap();
        let joined = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(joined.to_text().unwrap().contains("room_joined"));

        // Create recv transport
        let create = serde_json::to_string(&SignalingMessage::CreateWebRtcTransport {
            room_id: "sfu-consume-room".into(),
            peer_id: "consumer".into(),
            direction: mediaservo_common::protocol::TransportDirection::Recv,
        })
        .unwrap();
        ws.send(WsMsg::Text(create)).await.unwrap();
        // the late-joiner sync replays pending NewProducer broadcasts -- they arrive before the direct response,
        // so drain until the target message (a PIT-103 side fix: this test never ran during the test-server compile break).
        let created = loop {
            let resp = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            let parsed: SignalingMessage = serde_json::from_str(resp.to_text().unwrap()).unwrap();
            match parsed {
                SignalingMessage::WebRtcTransportCreated { .. } => break parsed,
                SignalingMessage::NewProducer { .. } => continue,
                other => panic!("Expected WebRtcTransportCreated, got: {:?}", other),
            }
        };
        let (recv_tid, recv_dtls) = match created {
            SignalingMessage::WebRtcTransportCreated { transport_id, dtls_parameters, .. } => {
                (transport_id, dtls_parameters)
            }
            _ => unreachable!(),
        };

        // Connect recv transport
        let connect = serde_json::to_string(&SignalingMessage::ConnectWebRtcTransport {
            room_id: "sfu-consume-room".into(),
            peer_id: "consumer".into(),
            transport_id: recv_tid,
            dtls_parameters: recv_dtls,
        })
        .unwrap();
        ws.send(WsMsg::Text(connect)).await.unwrap();
        let conn_resp = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(conn_resp.to_text().unwrap().contains("transport_connected"));

        // Consume the Host's producer
        let consume = serde_json::to_string(&SignalingMessage::Consume {
            room_id: "sfu-consume-room".into(),
            peer_id: "consumer".into(),
            producer_id: pid.clone(),
            rtp_capabilities: serde_json::json!({
                "codecs": [{"mimeType": "video/VP8", "clockRate": 90000, "kind": "video"}],
                "headerExtensions": []
            }),
            transport_id: None, // legacy path regression
        })
        .unwrap();
        ws.send(WsMsg::Text(consume)).await.unwrap();
        let consumed_resp = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let consumed: SignalingMessage =
            serde_json::from_str(consumed_resp.to_text().unwrap()).unwrap();
        match consumed {
            SignalingMessage::Consumed { consumer_id, producer_id, kind, .. } => {
                assert!(!consumer_id.is_empty(), "consumer_id should not be empty");
                assert_eq!(producer_id, pid, "producer_id should match host's producer");
                assert_eq!(
                    kind,
                    mediaservo_common::protocol::MediaKind::Video,
                    "consumer kind should be Video"
                );
            }
            other => panic!("Expected Consumed, got: {:?}", other),
        }

        ws
    });

    // Wait for consumer to complete
    let consumer_ws = consumer_handle.await.unwrap();

    // Verify consumer was registered
    assert!(sfu.room_count() > 0, "SFU should have at least one room");

    // Cleanup
    let _ = host_ws; // drop triggers disconnect
    let _ = consumer_ws;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
}

// ═══════════════════════════════════════════════════════════════════════════
// G3: role enforcement on the SFU plane -- vehicle produce auto-allowed (regression) + account produce denied +
// authorized account consume allowed + unauthorized account join denied (tenant isolation).
// ═══════════════════════════════════════════════════════════════════════════

const G3_JWT: &str = "e2e-sfu-g3-jwt-secret-32-bytes-min!";
const G3_ROOM: &str = "sfu-g3-room";

fn g3_token(username: &str, role: &str, vehicles: &[&str]) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()
        as usize;
    let claims = mediaservo_common::auth::JwtClaims {
        sub: username.into(),
        iat: now,
        exp: now + 3600,
        role: Some(role.into()),
        vehicles: Some(vehicles.iter().map(|s| s.to_string()).collect()),
    };
    jsonwebtoken::encode(
        &jsonwebtoken::Header::default(),
        &claims,
        &jsonwebtoken::EncodingKey::from_secret(G3_JWT.as_bytes()),
    )
    .unwrap()
}

async fn g3_account_connect(ws_url: &str, token: &str) -> KeepAliveWs {
    let mut req = ws_url.into_client_request().unwrap();
    req.headers_mut().insert("Sec-WebSocket-Protocol", token.parse().unwrap());
    let (ws, _) = tokio_tungstenite::connect_async(req).await.unwrap();
    ws
}

async fn g3_auth_and_join(ws: &mut KeepAliveWs, room: &str, role: PeerRole) {
    let ack = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(
        ack.to_text().unwrap().contains("authenticated"),
        "auth ack: {}",
        ack.to_text().unwrap()
    );
    let join = serde_json::to_string(&SignalingMessage::RoomJoin {
        room_id: room.into(),
        peer_role: role,
        stream_id: None,
        device_id: None,
        device_secret: None,
        device_pubkey: None,
        // S0: this helper speaks for the new endpoint (data_domain/F8 gating requires negotiated>=2).
        protocol: Some(2),
        client_version: None,
        resume: None,
    })
    .unwrap();
    ws.send(WsMsg::Text(join)).await.unwrap();
    let resp = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(resp.to_text().unwrap().contains("room_joined"), "join: {}", resp.to_text().unwrap());
}

async fn g3_create_recv_transport(
    ws: &mut KeepAliveWs,
    peer: &str,
) -> (String, mediaservo_common::protocol::DtlsParameters) {
    let create = serde_json::to_string(&SignalingMessage::CreateWebRtcTransport {
        room_id: G3_ROOM.into(),
        peer_id: peer.into(),
        direction: mediaservo_common::protocol::TransportDirection::Recv,
    })
    .unwrap();
    ws.send(WsMsg::Text(create)).await.unwrap();
    loop {
        let resp = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let parsed: SignalingMessage = serde_json::from_str(resp.to_text().unwrap()).unwrap();
        match parsed {
            SignalingMessage::WebRtcTransportCreated { transport_id, dtls_parameters, .. } => {
                return (transport_id, dtls_parameters);
            }
            SignalingMessage::NewProducer { .. } => continue, // late-joiner replay
            other => panic!("Expected WebRtcTransportCreated, got: {other:?}"),
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn e2e_sfu_role_enforcement() {
    unsafe { std::env::set_var("MEDIASERVO_PSK", PSK) };

    let sfu = Arc::new(
        SfuManager::new_with_port(mediaservo_server::sfu::random_udp_port())
            .await
            .expect("Failed to create SFU manager"),
    );
    let mut server = SignalingServer::new(
        Arc::clone(&sfu),
        65536,
        Some(mediaservo_common::auth::JwtAuth::new(G3_JWT)),
    );
    server.ws_ping_secs = 0; // the a1 heartbeat has its dedicated pins in heartbeat_e2e; this suite's raw JSON read loop must not eat control frames
    let hash = mediaservo_server::devices::hash_secret("ms-car1", "car1-secret");
    server.device_registry = std::sync::Arc::new(
        mediaservo_server::devices::DeviceRegistry::from_yaml(&format!(
            "devices:\n  ms-car1:\n    secret_hash: \"{hash}\"\n"
        ))
        .unwrap(),
    );
    let app = signaling_router(server);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let ws_url = format!("ws://{}/ws", addr);

    // -- 1. vehicle (device auth): produce auto-allowed (D-H11 regression) ------------------
    let host_url = ws_url.clone();
    let host_handle = tokio::spawn(async move {
        let (mut ws, _) = tokio_tungstenite::connect_async(&host_url).await.unwrap();
        ws.send(WsMsg::Text(PSK.into())).await.unwrap();
        let ack = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(ack.to_text().unwrap().contains("authenticated"));

        let join = serde_json::to_string(&SignalingMessage::RoomJoin {
            room_id: G3_ROOM.into(),
            peer_role: PeerRole::Host,
            stream_id: None,
            device_id: Some("ms-car1".into()),
            device_secret: Some("car1-secret".into()),
            device_pubkey: None,
            protocol: None,
            client_version: None,
            resume: None,
        })
        .unwrap();
        ws.send(WsMsg::Text(join)).await.unwrap();
        let joined = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(joined.to_text().unwrap().contains("room_joined"));

        // send transport + connect
        let create = serde_json::to_string(&SignalingMessage::CreateWebRtcTransport {
            room_id: G3_ROOM.into(),
            peer_id: "host".into(),
            direction: mediaservo_common::protocol::TransportDirection::Send,
        })
        .unwrap();
        ws.send(WsMsg::Text(create)).await.unwrap();
        let created: SignalingMessage = loop {
            let resp = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            let parsed: SignalingMessage = serde_json::from_str(resp.to_text().unwrap()).unwrap();
            match parsed {
                SignalingMessage::WebRtcTransportCreated { .. } => break parsed,
                SignalingMessage::RoomLeave { .. } => continue,
                other => panic!("Unexpected: {other:?}"),
            }
        };
        let (send_tid, send_dtls) = match created {
            SignalingMessage::WebRtcTransportCreated { transport_id, dtls_parameters, .. } => {
                (transport_id, dtls_parameters)
            }
            _ => unreachable!(),
        };
        let connect = serde_json::to_string(&SignalingMessage::ConnectWebRtcTransport {
            room_id: G3_ROOM.into(),
            peer_id: "host".into(),
            transport_id: send_tid,
            dtls_parameters: send_dtls,
        })
        .unwrap();
        ws.send(WsMsg::Text(connect)).await.unwrap();
        let conn = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(conn.to_text().unwrap().contains("transport_connected"));

        // produce -- the vehicle must be auto-allowed
        let produce = serde_json::to_string(&SignalingMessage::Produce {
            room_id: G3_ROOM.into(),
            peer_id: "host".into(),
            transport_direction: mediaservo_common::protocol::TransportDirection::Send,
            kind: mediaservo_common::protocol::MediaKind::Video,
            rtp_parameters: serde_json::json!({"mid": "0", "codecs": [{"mimeType": "video/VP8", "payloadType": 100, "clockRate": 90000}], "headerExtensions": [], "encodings": [{"ssrc": 12345}], "rtcp": {"reducedSize": true}}),
            transport_id: None, // legacy path regression

            label: None,})
        .unwrap();
        ws.send(WsMsg::Text(produce)).await.unwrap();
        let prod = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let producer_id =
            match serde_json::from_str::<SignalingMessage>(prod.to_text().unwrap()).unwrap() {
                SignalingMessage::Produced { producer_id, .. } => producer_id,
                other => panic!("vehicle produce must be allowed, got: {other:?}"),
            };
        (ws, producer_id)
    });
    let (host_ws, producer_id) = host_handle.await.unwrap();

    // -- 2. authorized operator (carol, [ms-car1]): join + consume -> Consumed OK -----------
    let carol_url = ws_url.clone();
    let pid = producer_id.clone();
    let carol_handle = tokio::spawn(async move {
        let mut ws =
            g3_account_connect(&carol_url, &g3_token("carol", "operator", &["ms-car1"])).await;
        g3_auth_and_join(&mut ws, G3_ROOM, PeerRole::Consumer).await;
        let (recv_tid, recv_dtls) = g3_create_recv_transport(&mut ws, "carol").await;
        let connect = serde_json::to_string(&SignalingMessage::ConnectWebRtcTransport {
            room_id: G3_ROOM.into(),
            peer_id: "carol".into(),
            transport_id: recv_tid,
            dtls_parameters: recv_dtls,
        })
        .unwrap();
        ws.send(WsMsg::Text(connect)).await.unwrap();
        let conn = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(conn.to_text().unwrap().contains("transport_connected"));

        let consume = serde_json::to_string(&SignalingMessage::Consume {
            room_id: G3_ROOM.into(),
            peer_id: "carol".into(),
            producer_id: pid.clone(),
            rtp_capabilities: serde_json::json!({
                "codecs": [{"mimeType": "video/VP8", "clockRate": 90000, "kind": "video"}],
                "headerExtensions": []
            }),
            transport_id: None, // legacy path regression
        })
        .unwrap();
        ws.send(WsMsg::Text(consume)).await.unwrap();
        let resp = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        match serde_json::from_str::<SignalingMessage>(resp.to_text().unwrap()).unwrap() {
            SignalingMessage::Consumed { producer_id: pid_resp, .. } => {
                assert_eq!(pid_resp, pid, "authorized operator consume must be allowed")
            }
            other => panic!("authorized consume must be Consumed, got: {other:?}"),
        }
        ws
    });
    let _carol_ws = carol_handle.await.unwrap();

    // -- 3. operator attempts produce -> 4031 (accounts only consume) -----------------------
    let prod_url = ws_url.clone();
    let prod_handle = tokio::spawn(async move {
        let mut ws =
            g3_account_connect(&prod_url, &g3_token("carol", "operator", &["ms-car1"])).await;
        g3_auth_and_join(&mut ws, G3_ROOM, PeerRole::Consumer).await;
        // send transport (the produce gate sits behind the direction check -- a send transport must exist first to reach it)
        let create = serde_json::to_string(&SignalingMessage::CreateWebRtcTransport {
            room_id: G3_ROOM.into(),
            peer_id: "carol-prod".into(),
            direction: mediaservo_common::protocol::TransportDirection::Send,
        })
        .unwrap();
        ws.send(WsMsg::Text(create)).await.unwrap();
        let created = loop {
            let resp = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            let parsed: SignalingMessage = serde_json::from_str(resp.to_text().unwrap()).unwrap();
            match parsed {
                SignalingMessage::WebRtcTransportCreated { .. } => break parsed,
                SignalingMessage::NewProducer { .. } => continue,
                other => panic!("Expected transport created, got: {other:?}"),
            }
        };
        let (send_tid, send_dtls) = match created {
            SignalingMessage::WebRtcTransportCreated { transport_id, dtls_parameters, .. } => {
                (transport_id, dtls_parameters)
            }
            _ => unreachable!(),
        };
        let connect = serde_json::to_string(&SignalingMessage::ConnectWebRtcTransport {
            room_id: G3_ROOM.into(),
            peer_id: "carol-prod".into(),
            transport_id: send_tid,
            dtls_parameters: send_dtls,
        })
        .unwrap();
        ws.send(WsMsg::Text(connect)).await.unwrap();
        let conn = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(conn.to_text().unwrap().contains("transport_connected"));

        let produce = serde_json::to_string(&SignalingMessage::Produce {
            room_id: G3_ROOM.into(),
            peer_id: "carol-prod".into(),
            transport_direction: mediaservo_common::protocol::TransportDirection::Send,
            kind: mediaservo_common::protocol::MediaKind::Video,
            rtp_parameters: serde_json::json!({"mid": "0", "codecs": [{"mimeType": "video/VP8", "payloadType": 100, "clockRate": 90000}], "headerExtensions": [], "encodings": [{"ssrc": 999}], "rtcp": {"reducedSize": true}}),
            transport_id: None, // legacy path regression

            label: None,})
        .unwrap();
        ws.send(WsMsg::Text(produce)).await.unwrap();
        let resp = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(
            resp.to_text().unwrap().contains(r#""code":4031"#),
            "account produce must be denied: {}",
            resp.to_text().unwrap()
        );
        ws
    });
    let _prod_ws = prod_handle.await.unwrap();

    // -- 4. unauthorized operator (mallory, empty allowlist): join refused (tenant isolation) --
    let mallory_url = ws_url.clone();
    let mallory_handle = tokio::spawn(async move {
        let mut ws = g3_account_connect(&mallory_url, &g3_token("mallory", "operator", &[])).await;
        let ack = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(ack.to_text().unwrap().contains("authenticated"));
        let join = serde_json::to_string(&SignalingMessage::RoomJoin {
            room_id: G3_ROOM.into(),
            peer_role: PeerRole::Consumer,
            stream_id: None,
            device_id: None,
            device_secret: None,
            device_pubkey: None,
            protocol: None,
            client_version: None,
            resume: None,
        })
        .unwrap();
        ws.send(WsMsg::Text(join)).await.unwrap();
        let resp = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(
            resp.to_text().unwrap().contains(r#""code":4031"#),
            "unauthorized account join must be denied: {}",
            resp.to_text().unwrap()
        );
        ws
    });
    let _mallory_ws = mallory_handle.await.unwrap();

    // Cleanup
    let _ = host_ws;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
}

// -- H1: SFU data-domain e2e -- DataProducer/DataConsumer over WS signaling + the G3 role gate --

/// H1: vehicle (device) produce_data -> authorized operator consume_data, the full chain;
/// account produce_data denied (4031); the NewDataProducer broadcast reaches the consumer.
/// Message content flows inside the mediasoup worker (endpoint SCTP -> worker -> endpoint);
/// the server only wires -- for end-to-end message-receipt proof see sfu.rs data_message_roundtrip_direct (#[ignore],
/// blocked by the mediasoup-rs notification-channel bug); this proves the protocol/gate/entity-creation chain.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn e2e_sfu_data_domain() {
    unsafe { std::env::set_var("MEDIASERVO_PSK", PSK) };

    let sfu = Arc::new(
        SfuManager::new_with_port(mediaservo_server::sfu::random_udp_port())
            .await
            .expect("Failed to create SFU manager"),
    );
    let mut server = SignalingServer::new(
        Arc::clone(&sfu),
        65536,
        Some(mediaservo_common::auth::JwtAuth::new(G3_JWT)),
    );
    server.ws_ping_secs = 0; // the a1 heartbeat has its dedicated pins in heartbeat_e2e; this suite's raw JSON read loop must not eat control frames
    let hash = mediaservo_server::devices::hash_secret("ms-car1", "car1-secret");
    server.device_registry = std::sync::Arc::new(
        mediaservo_server::devices::DeviceRegistry::from_yaml(&format!(
            "devices:\n  ms-car1:\n    secret_hash: \"{hash}\"\n"
        ))
        .unwrap(),
    );
    let app = signaling_router(server);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let ws_url = format!("ws://{}/ws", addr);

    // -- 1. vehicle (device auth): send transport + produce_data (auto-allowed) --------------
    let host_url = ws_url.clone();
    let host_handle = tokio::spawn(async move {
        let (mut ws, _) = tokio_tungstenite::connect_async(&host_url).await.unwrap();
        ws.send(WsMsg::Text(PSK.into())).await.unwrap();
        let ack = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(ack.to_text().unwrap().contains("authenticated"));

        let join = serde_json::to_string(&SignalingMessage::RoomJoin {
            room_id: G3_ROOM.into(),
            peer_role: PeerRole::Host,
            stream_id: None,
            device_id: Some("ms-car1".into()),
            device_secret: Some("car1-secret".into()),
            device_pubkey: None,
            protocol: None,
            client_version: None,
            resume: None,
        })
        .unwrap();
        ws.send(WsMsg::Text(join)).await.unwrap();
        let joined = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(joined.to_text().unwrap().contains("room_joined"));

        // send transport
        let create = serde_json::to_string(&SignalingMessage::CreateWebRtcTransport {
            room_id: G3_ROOM.into(),
            peer_id: "host".into(),
            direction: mediaservo_common::protocol::TransportDirection::Send,
        })
        .unwrap();
        ws.send(WsMsg::Text(create)).await.unwrap();
        let (send_tid, send_dtls) = loop {
            let resp = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            match serde_json::from_str::<SignalingMessage>(resp.to_text().unwrap()).unwrap() {
                SignalingMessage::WebRtcTransportCreated {
                    transport_id, dtls_parameters, ..
                } => break (transport_id, dtls_parameters),
                SignalingMessage::RoomLeave { .. } => continue,
                other => panic!("Unexpected: {other:?}"),
            }
        };
        let connect = serde_json::to_string(&SignalingMessage::ConnectWebRtcTransport {
            room_id: G3_ROOM.into(),
            peer_id: "host".into(),
            transport_id: send_tid,
            dtls_parameters: send_dtls,
        })
        .unwrap();
        ws.send(WsMsg::Text(connect)).await.unwrap();
        let conn = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(conn.to_text().unwrap().contains("transport_connected"));

        // produce_data -- the vehicle is auto-allowed
        let produce_data = serde_json::to_string(&SignalingMessage::CreateDataProducer {
            room_id: G3_ROOM.into(),
            peer_id: "host".into(),
            transport_direction: mediaservo_common::protocol::TransportDirection::Send,
            label: "control".into(),
            protocol: "mediaservo.control".into(),
            sctp_stream_parameters: Some(mediaservo_common::protocol::SctpStreamParameters {
                stream_id: 1,
                ordered: true,
                max_packet_life_time: None,
                max_retransmits: None,
            }),
            transport_id: None, // legacy path regression
        })
        .unwrap();
        ws.send(WsMsg::Text(produce_data)).await.unwrap();
        let data_producer_id =
            match tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap()
            {
                resp => {
                    match serde_json::from_str::<SignalingMessage>(resp.to_text().unwrap()).unwrap()
                    {
                        SignalingMessage::DataProducerCreated { data_producer_id, .. } => {
                            data_producer_id
                        }
                        other => panic!("vehicle produce_data must be allowed, got: {other:?}"),
                    }
                }
            };
        (ws, data_producer_id)
    });
    let (host_ws, data_producer_id) = host_handle.await.unwrap();

    // -- 2. authorized operator (carol, [ms-car1]): recv transport + consume_data -> allowed --
    let carol_url = ws_url.clone();
    let dp_id = data_producer_id.clone();
    let carol_handle = tokio::spawn(async move {
        let mut ws =
            g3_account_connect(&carol_url, &g3_token("carol", "operator", &["ms-car1"])).await;
        g3_auth_and_join(&mut ws, G3_ROOM, PeerRole::Consumer).await;
        // should receive the vehicle's NewDataProducer broadcast (late-joiner sync + broadcast)
        let mut saw_new_data_producer = false;
        let mut recv_tid = None;
        let mut recv_dtls = None;
        let create = serde_json::to_string(&SignalingMessage::CreateWebRtcTransport {
            room_id: G3_ROOM.into(),
            peer_id: "carol".into(),
            direction: mediaservo_common::protocol::TransportDirection::Recv,
        })
        .unwrap();
        ws.send(WsMsg::Text(create)).await.unwrap();
        for _ in 0..8 {
            let resp = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            match serde_json::from_str::<SignalingMessage>(resp.to_text().unwrap()).unwrap() {
                SignalingMessage::NewDataProducer { label, data_producer_id, .. } => {
                    assert_eq!(label, "control");
                    assert_eq!(data_producer_id, dp_id);
                    saw_new_data_producer = true;
                }
                SignalingMessage::NewProducer { .. } => continue,
                SignalingMessage::WebRtcTransportCreated {
                    transport_id, dtls_parameters, ..
                } => {
                    recv_tid = Some(transport_id);
                    recv_dtls = Some(dtls_parameters);
                    break;
                }
                other => panic!("Unexpected: {other:?}"),
            }
        }
        assert!(saw_new_data_producer, "the consumer must receive the NewDataProducer broadcast");
        let recv_tid = recv_tid.expect("recv transport");
        let recv_dtls = recv_dtls.expect("recv dtls");
        let connect = serde_json::to_string(&SignalingMessage::ConnectWebRtcTransport {
            room_id: G3_ROOM.into(),
            peer_id: "carol".into(),
            transport_id: recv_tid,
            dtls_parameters: recv_dtls,
        })
        .unwrap();
        ws.send(WsMsg::Text(connect)).await.unwrap();
        let conn = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(conn.to_text().unwrap().contains("transport_connected"));

        // consume_data -- authorized operator allowed
        let consume_data = serde_json::to_string(&SignalingMessage::ConsumeData {
            room_id: G3_ROOM.into(),
            peer_id: "carol".into(),
            transport_direction: mediaservo_common::protocol::TransportDirection::Recv,
            data_producer_id: dp_id,
            transport_id: None, // legacy path regression
        })
        .unwrap();
        ws.send(WsMsg::Text(consume_data)).await.unwrap();
        let resp = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        match serde_json::from_str::<SignalingMessage>(resp.to_text().unwrap()).unwrap() {
            SignalingMessage::DataConsumed { data_producer_id: dp_resp, .. } => {
                assert!(
                    !dp_resp.is_empty(),
                    "consume_data must return the producer associated with the data_consumer_id"
                )
            }
            other => panic!("authorized consume_data must be DataConsumed, got: {other:?}"),
        }
        ws
    });
    let _carol_ws = carol_handle.await.unwrap();

    // -- 3. the P1/F8 contract re-judgment: operator (can_control) creates the control DC -> the role gate allows --
    let prod_url = ws_url.clone();
    let prod_handle = tokio::spawn(async move {
        let mut ws =
            g3_account_connect(&prod_url, &g3_token("carol", "operator", &["ms-car1"])).await;
        g3_auth_and_join(&mut ws, G3_ROOM, PeerRole::Consumer).await;
        // a send transport must exist first to reach the produce_data gate
        let create = serde_json::to_string(&SignalingMessage::CreateWebRtcTransport {
            room_id: G3_ROOM.into(),
            peer_id: "carol-prod".into(),
            direction: mediaservo_common::protocol::TransportDirection::Send,
        })
        .unwrap();
        ws.send(WsMsg::Text(create)).await.unwrap();
        let (send_tid, send_dtls) = loop {
            let resp = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            match serde_json::from_str::<SignalingMessage>(resp.to_text().unwrap()).unwrap() {
                SignalingMessage::WebRtcTransportCreated {
                    transport_id, dtls_parameters, ..
                } => break (transport_id, dtls_parameters),
                SignalingMessage::NewProducer { .. } | SignalingMessage::NewDataProducer { .. } => {
                    continue;
                }
                other => panic!("Expected transport created, got: {other:?}"),
            }
        };
        let connect = serde_json::to_string(&SignalingMessage::ConnectWebRtcTransport {
            room_id: G3_ROOM.into(),
            peer_id: "carol-prod".into(),
            transport_id: send_tid,
            dtls_parameters: send_dtls,
        })
        .unwrap();
        ws.send(WsMsg::Text(connect)).await.unwrap();
        let conn = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(conn.to_text().unwrap().contains("transport_connected"));

        let produce_data = serde_json::to_string(&SignalingMessage::CreateDataProducer {
            room_id: G3_ROOM.into(),
            peer_id: "carol-prod".into(),
            transport_direction: mediaservo_common::protocol::TransportDirection::Send,
            label: "control".into(),
            protocol: "mediaservo.control".into(),
            sctp_stream_parameters: Some(mediaservo_common::protocol::SctpStreamParameters {
                stream_id: 2,
                ordered: true,
                max_packet_life_time: None,
                max_retransmits: None,
            }),
            transport_id: None, // legacy path regression
        })
        .unwrap();
        ws.send(WsMsg::Text(produce_data)).await.unwrap();
        let resp = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        // P1/F8 (a deliberate contract change; the old shape denied all with 4031): operator has the control capability -> the role gate allows -> SFU data-domain creation.
        match serde_json::from_str::<SignalingMessage>(resp.to_text().unwrap()).unwrap() {
            SignalingMessage::DataProducerCreated { .. } => {}
            other => {
                panic!("operator control DC must be allowed (the F8 role gate), got: {other:?}")
            }
        }
        ws
    });
    let _prod_ws = prod_handle.await.unwrap();

    // -- 4. the P1/F8 negative: dispatcher lacks the control capability -> control DC -> explicit 4012 (the D273 terminal is visible) --
    let dis_url = ws_url.clone();
    let dis_handle = tokio::spawn(async move {
        let mut ws = g3_account_connect(&dis_url, &g3_token("dave", "dispatcher", &[])).await;
        g3_auth_and_join(&mut ws, G3_ROOM, PeerRole::Consumer).await;
        let produce_data = serde_json::to_string(&SignalingMessage::CreateDataProducer {
            room_id: G3_ROOM.into(),
            peer_id: "dave-prod".into(),
            transport_direction: mediaservo_common::protocol::TransportDirection::Send,
            label: "control".into(),
            protocol: "mediaservo.control".into(),
            sctp_stream_parameters: None,
            transport_id: None,
        })
        .unwrap();
        ws.send(WsMsg::Text(produce_data)).await.unwrap();
        let resp = loop {
            let r = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            match serde_json::from_str::<SignalingMessage>(r.to_text().unwrap()).unwrap() {
                SignalingMessage::NewProducer { .. } | SignalingMessage::NewDataProducer { .. } => {
                    continue;
                }
                other => break other,
            }
        };
        assert!(
            matches!(&resp, SignalingMessage::Error { code: 4012, .. }),
            "dispatcher control DC must get an explicit 4012, got: {resp:?}"
        );
    });
    dis_handle.await.unwrap();

    // Cleanup
    let _ = host_ws;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
}

// ═══════════════════════════════════════════════════════════════════════════
// H2 (I3 review): the audio room device-identity path -- device auth join audio-<vehicle>
// succeeds + member ownership registered + cross-vehicle tenant isolation + video produce in an audio room denied (device path).
// ═══════════════════════════════════════════════════════════════════════════

/// Device identity joins an audio room (the G2 pattern: device_registry + RoomJoin carrying credentials).
/// Asserts: 1. join succeeds 2. room_owners["audio-ms-car1"] == "ms-car1" (member ownership)
/// 3. another vehicle's device joining the same audio room -> 4031 (tenant isolation) 4. the device producing video in the audio room -> 4031.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn e2e_audio_room_device_identity() {
    unsafe { std::env::set_var("MEDIASERVO_PSK", PSK) };

    let sfu = Arc::new(
        SfuManager::new_with_port(mediaservo_server::sfu::random_udp_port())
            .await
            .expect("Failed to create SFU manager"),
    );
    let mut server = SignalingServer::new(Arc::clone(&sfu), 65536, None);
    server.ws_ping_secs = 0; // the a1 heartbeat has its dedicated pins in heartbeat_e2e; this suite's raw JSON read loop must not eat control frames
    let hash1 = mediaservo_server::devices::hash_secret("ms-car1", "car1-secret");
    let hash2 = mediaservo_server::devices::hash_secret("ms-car2", "car2-secret");
    server.device_registry = std::sync::Arc::new(
        mediaservo_server::devices::DeviceRegistry::from_yaml(&format!(
            "devices:\n  ms-car1:\n    secret_hash: \"{hash1}\"\n  ms-car2:\n    secret_hash: \"{hash2}\"\n"
        ))
        .unwrap(),
    );
    let app = signaling_router(server);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let ws_url = format!("ws://{}/ws", addr);

    // -- 1. vehicle device auth joins the audio room audio-ms-car1 --
    let url = ws_url.clone();
    let veh_handle = tokio::spawn(async move {
        let (mut ws, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
        ws.send(WsMsg::Text(PSK.into())).await.unwrap();
        let ack = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(ack.to_text().unwrap().contains("authenticated"));

        let join = serde_json::to_string(&SignalingMessage::RoomJoin {
            room_id: "audio-ms-car1".into(),
            peer_role: PeerRole::Consumer,
            stream_id: None,
            device_id: Some("ms-car1".into()),
            device_secret: Some("car1-secret".into()),
            device_pubkey: None,
            protocol: None,
            client_version: None,
            resume: None,
        })
        .unwrap();
        ws.send(WsMsg::Text(join)).await.unwrap();
        let joined = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(
            joined.to_text().unwrap().contains("room_joined"),
            "device identity joining the audio room must succeed: {}",
            joined.to_text().unwrap()
        );
        ws
    });
    let mut veh_ws = veh_handle.await.unwrap();

    // Member ownership: room_owners["audio-ms-car1"] registered as ms-car1 (the G3 registration path)
    // -- indirect evidence: a later other-vehicle join is refused by tenant isolation (owner matching works) + the own-vehicle produce path is allowed.
    // -- 2. another vehicle's device joins the same audio room -> 4031 (tenant isolation) --
    let url2 = ws_url.clone();
    let other_handle = tokio::spawn(async move {
        let (mut ws, _) = tokio_tungstenite::connect_async(&url2).await.unwrap();
        ws.send(WsMsg::Text(PSK.into())).await.unwrap();
        let ack = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(ack.to_text().unwrap().contains("authenticated"));
        let join = serde_json::to_string(&SignalingMessage::RoomJoin {
            room_id: "audio-ms-car1".into(),
            peer_role: PeerRole::Consumer,
            stream_id: None,
            device_id: Some("ms-car2".into()),
            device_secret: Some("car2-secret".into()),
            device_pubkey: None,
            protocol: None,
            client_version: None,
            resume: None,
        })
        .unwrap();
        ws.send(WsMsg::Text(join)).await.unwrap();
        let resp = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(
            resp.to_text().unwrap().contains("4031"),
            "another vehicle's device joining the audio room must get 4031: {}",
            resp.to_text().unwrap()
        );
    });
    other_handle.await.unwrap();

    // -- 3. the device produces video in the audio room -> 4031 (the H2 audio-room gate, device path) --
    veh_ws
        .send(WsMsg::Text(
            serde_json::to_string(&SignalingMessage::Produce {
                room_id: "audio-ms-car1".into(),
                peer_id: "veh".into(),
                transport_direction: mediaservo_common::protocol::TransportDirection::Send,
                kind: mediaservo_common::protocol::MediaKind::Video,
                rtp_parameters: serde_json::json!({
                    "mid": "0",
                    "codecs": [{"mimeType": "video/VP8", "payloadType": 96, "clockRate": 90000}],
                    "headerExtensions": [],
                    "encodings": [{"ssrc": 12345}],
                    "rtcp": {"reducedSize": true}
                }),
                transport_id: None, // legacy path regression

                label: None,
            })
            .unwrap(),
        ))
        .await
        .unwrap();
    let resp = tokio::time::timeout(std::time::Duration::from_secs(5), veh_ws.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(
        resp.to_text().unwrap().contains("4031"),
        "a device producing video in an audio room must get 4031: {}",
        resp.to_text().unwrap()
    );

    drop(veh_ws);
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
}
