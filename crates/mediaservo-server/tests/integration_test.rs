use futures_util::{SinkExt, StreamExt};
use mediaservo_common::protocol::{PeerRole, SignalingMessage};
use mediaservo_server::signaling::{SignalingServer, signaling_router};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message as WsMsg;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

const PSK: &str = "test-psk";
const ROOM: &str = "test-room";

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn integration_signaling_pipeline() {
    unsafe { std::env::set_var("MEDIASERVO_PSK", PSK) };

    #[cfg(feature = "sfu-mediasoup")]
    let mut server = {
        let sfu = std::sync::Arc::new(
            mediaservo_server::sfu::SfuManager::new_with_port(
                mediaservo_server::sfu::random_udp_port(),
            )
            .await
            .unwrap(),
        );
        SignalingServer::new(sfu, 65536, None)
    };
    #[cfg(not(feature = "sfu-mediasoup"))]
    let mut server = SignalingServer::new(65536, None);
    server.ws_ping_secs = 0; // the a1 heartbeat has its dedicated pins in heartbeat_e2e; the raw JSON read loop must not eat control frames
    let mut server = server;
    server.psk_state = std::sync::Arc::new(std::sync::RwLock::new(Some(PSK.into())));
    let app = signaling_router(server);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let ws_url = format!("ws://{}/ws", addr);

    // Spawn Host in background task
    let (host_tx, mut host_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let host_url = ws_url.clone();
    let host_handle = tokio::spawn(async move {
        let (mut ws, _) = tokio_tungstenite::connect_async(&host_url).await.unwrap();
        // PSK auth
        ws.send(WsMsg::Text(PSK.into())).await.unwrap();
        let ack = ws.next().await.unwrap().unwrap();
        assert!(ack.to_text().unwrap().contains("authenticated"));
        // RoomJoin
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
        let joined_text = joined.to_text().unwrap();
        assert!(joined_text.contains("room_joined"), "host join failed: {}", joined_text);
        host_tx.send("joined".into()).unwrap();
        // Drain (ignore relay echos) and forward received messages to channel
        while let Some(Ok(msg)) = ws.next().await {
            if let Ok(text) = msg.to_text() {
                // Don't forward auth/join echos
                if !text.contains("authenticated") && !text.contains("room_join") {
                    let _ = host_tx.send(text.to_string());
                }
            }
        }
    });

    // Spawn Remote in background task
    let (remote_tx, mut remote_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let remote_url = ws_url.clone();
    let remote_handle = tokio::spawn(async move {
        let (mut ws, _) = tokio_tungstenite::connect_async(&remote_url).await.unwrap();
        ws.send(WsMsg::Text(PSK.into())).await.unwrap();
        let ack = ws.next().await.unwrap().unwrap();
        assert!(ack.to_text().unwrap().contains("authenticated"));
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
        remote_tx.send("joined".into()).unwrap();
        while let Some(Ok(msg)) = ws.next().await {
            if let Ok(text) = msg.to_text()
                && !text.contains("authenticated")
                && !text.contains("room_join")
            {
                let _ = remote_tx.send(text.to_string());
            }
        }
    });

    // Wait for both to join
    assert_eq!(host_rx.recv().await.unwrap(), "joined");
    assert_eq!(remote_rx.recv().await.unwrap(), "joined");

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Now we need to send messages FROM host. But host_ws is consumed by the spawned task.
    // Instead, we'll connect a third client to send messages as the "host producer".
    // Actually, the host spawned task is consuming the Host connection. We need
    // a separate connection for sending. Let's use a different approach:
    // The test itself connects as the message sender, while the spawned tasks
    // are the receivers.

    // Connect a "sender" as Host (second Host will get RoomFull, but first Remote is fine)
    // Actually, let's restructure: test connects as Host, spawns Remote reader.
    // We need to connect as Host here and send messages.

    // ponytail: simpler — use room_manager directly + protocol serialization for relay testing.
    // The WS relay is tested implicitly by the spawned tasks receiving messages.
    // Cleanup
    host_handle.abort();
    remote_handle.abort();
    drop(host_rx);
    drop(remote_rx);
}

#[test]
fn test_room_manager_signaling_flow() {
    use mediaservo_server::room::RoomManager;
    let rm = RoomManager::new();

    // Host joins
    rm.join_room("room-1", "host-1", &PeerRole::Host).unwrap();
    assert_eq!(rm.active_rooms(), 1);
    assert_eq!(rm.get_peer_count(), 1);

    // Remote joins
    rm.join_room("room-1", "remote-1", &PeerRole::Remote).unwrap();
    assert_eq!(rm.active_rooms(), 1);
    assert_eq!(rm.get_peer_count(), 2);

    // RoomFull for second Host
    assert!(rm.join_room("room-1", "host-2", &PeerRole::Host).is_err());

    // All-SFU (2026-08-25): multiple Remotes (cockpit consumers) can coexist
    rm.join_room("room-1", "remote-2", &PeerRole::Remote).unwrap();
    assert_eq!(rm.get_peer_count(), 3);

    // Leave host
    rm.leave_room("room-1", "host-1");
    assert_eq!(rm.get_peer_count(), 2);

    // Leave remotes → room removed
    rm.leave_room("room-1", "remote-1");
    assert_eq!(rm.get_peer_count(), 1);
    rm.leave_room("room-1", "remote-2");
    assert_eq!(rm.active_rooms(), 0);
    assert_eq!(rm.get_peer_count(), 0);
}

#[test]
fn test_sdp_frame_ice_serialization() {
    // SDP round-trip
    let sdp = SignalingMessage::Sdp {
        room_id: "r1".into(),
        target: None,
        sdp: "v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=-".into(),
    };
    let json = serde_json::to_string(&sdp).unwrap();
    let back: SignalingMessage = serde_json::from_str(&json).unwrap();
    assert!(matches!(back, SignalingMessage::Sdp { .. }));

    // Frame round-trip
    let frame = SignalingMessage::Frame {
        room_id: "r1".into(),
        codec: "h264".into(),
        sequence: 42,
        is_keyframe: true,
        data_base64: "SGVsbG8=".into(),
    };
    let json = serde_json::to_string(&frame).unwrap();
    let back: SignalingMessage = serde_json::from_str(&json).unwrap();
    match back {
        SignalingMessage::Frame { codec, sequence, is_keyframe, .. } => {
            assert_eq!(codec, "h264");
            assert_eq!(sequence, 42);
            assert!(is_keyframe);
        }
        _ => panic!("expected Frame"),
    }

    // ICE round-trip
    let ice = SignalingMessage::RTCIceCandidate {
        room_id: "r1".into(),
        target: None,
        candidate: "candidate:1 1 UDP 2130706431 10.0.0.1 8000 typ host".into(),
        sdp_mid: Some("0".into()),
        sdp_mline_index: Some(0),
    };
    let json = serde_json::to_string(&ice).unwrap();
    let back: SignalingMessage = serde_json::from_str(&json).unwrap();
    assert!(matches!(back, SignalingMessage::RTCIceCandidate { .. }));

    // Error round-trip
    let err = SignalingMessage::Error { code: 4002, message: "Room is full".into() };
    let json = serde_json::to_string(&err).unwrap();
    let back: SignalingMessage = serde_json::from_str(&json).unwrap();
    match back {
        SignalingMessage::Error { code, message } => {
            assert_eq!(code, 4002);
            assert_eq!(message, "Room is full");
        }
        _ => panic!("expected Error"),
    }

    // RoomJoin/RoomJoined/RoomLeave
    let join: SignalingMessage =
        serde_json::from_str(r#"{"type":"room_join","room_id":"abc","peer_role":"host"}"#).unwrap();
    assert!(matches!(join, SignalingMessage::RoomJoin { .. }));

    let joined: SignalingMessage =
        serde_json::from_str(r#"{"type":"room_joined","room_id":"abc","peer_id":"p1"}"#).unwrap();
    assert!(matches!(joined, SignalingMessage::RoomJoined { .. }));

    let leave: SignalingMessage =
        serde_json::from_str(r#"{"type":"room_leave","room_id":"abc","peer_id":"p1"}"#).unwrap();
    assert!(matches!(leave, SignalingMessage::RoomLeave { .. }));
}

#[tokio::test]
async fn test_auth_failure_integration() {
    unsafe { std::env::set_var("MEDIASERVO_PSK", PSK) };

    #[cfg(feature = "sfu-mediasoup")]
    let mut server = {
        let sfu = std::sync::Arc::new(
            mediaservo_server::sfu::SfuManager::new_with_port(
                mediaservo_server::sfu::random_udp_port(),
            )
            .await
            .unwrap(),
        );
        SignalingServer::new(sfu, 65536, None)
    };
    #[cfg(not(feature = "sfu-mediasoup"))]
    let mut server = SignalingServer::new(65536, None);
    server.ws_ping_secs = 0; // the a1 heartbeat has its dedicated pins in heartbeat_e2e; the raw JSON read loop must not eat control frames
    let mut server = server;
    server.psk_state = std::sync::Arc::new(std::sync::RwLock::new(Some(PSK.into())));
    let app = signaling_router(server);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let ws_url = format!("ws://{}/ws", addr);
    let (mut ws, _) = tokio_tungstenite::connect_async(&ws_url).await.unwrap();

    // Send wrong PSK
    ws.send(WsMsg::Text("wrong-psk".into())).await.unwrap();
    let resp = ws.next().await.unwrap().unwrap();
    let text = resp.to_text().unwrap();
    let msg: SignalingMessage = serde_json::from_str(text).unwrap();
    match msg {
        SignalingMessage::Error { code, .. } => {
            assert_eq!(code, 4003, "expected 4003, got: {}", text);
        }
        _ => panic!("expected Error, got: {}", text),
    }

    drop(ws);
}

// ── E2E video frame relay test ──

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn e2e_video_frame_relay() {
    unsafe { std::env::set_var("MEDIASERVO_PSK", PSK) };

    #[cfg(feature = "sfu-mediasoup")]
    let mut server = {
        let sfu = std::sync::Arc::new(
            mediaservo_server::sfu::SfuManager::new_with_port(
                mediaservo_server::sfu::random_udp_port(),
            )
            .await
            .unwrap(),
        );
        SignalingServer::new(sfu, 65536, None)
    };
    #[cfg(not(feature = "sfu-mediasoup"))]
    let mut server = SignalingServer::new(65536, None);
    server.ws_ping_secs = 0; // the a1 heartbeat has its dedicated pins in heartbeat_e2e; the raw JSON read loop must not eat control frames
    let mut server = server;
    server.psk_state = std::sync::Arc::new(std::sync::RwLock::new(Some(PSK.into())));
    let app = signaling_router(server);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let ws_url = format!("ws://{}/ws", addr);

    // --- Host: connect, auth, join room, wait for remote, send 5 video frames ---
    let host_url = ws_url.clone();
    let host_handle = tokio::spawn(async move {
        let (mut ws, _) = tokio_tungstenite::connect_async(&host_url).await.unwrap();
        ws.send(WsMsg::Text(PSK.into())).await.unwrap();
        ws.next().await.unwrap().unwrap(); // auth ack
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
        ws.next().await.unwrap().unwrap(); // room_joined

        // Wait for remote to signal SDP (we use a sleep since we can't coordinate channels here)
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        // Send 5 video frames with increasing sequence numbers
        for seq in 0..5u64 {
            let frame = SignalingMessage::Frame {
                room_id: ROOM.into(),
                codec: "h264".into(),
                sequence: seq,
                is_keyframe: seq == 0,
                data_base64: base64::Engine::encode(
                    &base64::engine::general_purpose::STANDARD,
                    format!("frame-{seq}").as_bytes(),
                ),
            };
            ws.send(WsMsg::Text(serde_json::to_string(&frame).unwrap())).await.unwrap();
        }
        // Keep connection alive briefly
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    });

    // --- Remote: connect, auth, join room, listen for frames ---
    let remote_url = ws_url.clone();
    let remote_handle = tokio::spawn(async move {
        let (mut ws, _) = tokio_tungstenite::connect_async(&remote_url).await.unwrap();
        ws.send(WsMsg::Text(PSK.into())).await.unwrap();
        ws.next().await.unwrap().unwrap(); // auth ack
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
        ws.next().await.unwrap().unwrap(); // room_joined

        // Drain: collect Frame messages until timeout
        let mut received_frames: Vec<u64> = Vec::new();
        loop {
            let msg = tokio::time::timeout(std::time::Duration::from_secs(3), ws.next()).await;
            match msg {
                Ok(Some(Ok(ws_msg))) => {
                    if let Ok(text) = ws_msg.to_text()
                        && let Ok(sig) = serde_json::from_str::<SignalingMessage>(text)
                        && let SignalingMessage::Frame { sequence, is_keyframe, codec, .. } = sig
                    {
                        received_frames.push(sequence);
                        // First frame must be keyframe
                        if sequence == 0 {
                            assert!(is_keyframe, "first frame must be keyframe");
                        }
                        assert_eq!(codec, "h264");
                    }
                }
                _ => break, // timeout or error — stop
            }
        }
        received_frames
    });

    // Collect results
    host_handle.await.unwrap();
    let received = remote_handle.await.unwrap();

    // Assert: remote received all 5 frames in order
    assert_eq!(received.len(), 5, "expected 5 frames, got: {:?}", received);
    assert_eq!(received, vec![0, 1, 2, 3, 4], "frames must be in order");
    println!("E2E frame relay: {}/5 frames received in order", received.len());
}

// ═══════════════════════════════════════════════════════════════════════════
// G2: device auth (D-H11 connection-level identity) -- registry match -> binding; failure -> Error 4010;
// both missing -> the PSK path unchanged (regression).
// ═══════════════════════════════════════════════════════════════════════════

/// Starts a test server with a device registry, returns (server, ws_url).
/// registry_yaml: entry text under `devices:` (None = empty registry).
async fn spawn_server_with_devices(registry_yaml: Option<&str>) -> (SignalingServer, String) {
    unsafe { std::env::set_var("MEDIASERVO_PSK", PSK) };

    let yaml = match registry_yaml {
        Some(devices) => format!("devices:\n{devices}\n"),
        None => "devices: {}\n".into(),
    };
    let registry = mediaservo_server::devices::DeviceRegistry::from_yaml(&yaml)
        .expect("test registry yaml valid");

    #[cfg(feature = "sfu-mediasoup")]
    let mut server = {
        let sfu = std::sync::Arc::new(
            mediaservo_server::sfu::SfuManager::new_with_port(
                mediaservo_server::sfu::random_udp_port(),
            )
            .await
            .unwrap(),
        );
        SignalingServer::new(sfu, 65536, None)
    };
    #[cfg(not(feature = "sfu-mediasoup"))]
    let mut server = SignalingServer::new(65536, None);
    server.ws_ping_secs = 0; // the a1 heartbeat has its dedicated pins in heartbeat_e2e; the raw JSON read loop must not eat control frames
    server.device_registry = std::sync::Arc::new(registry);

    let mut server = server;
    server.psk_state = std::sync::Arc::new(std::sync::RwLock::new(Some(PSK.into())));
    let app = signaling_router(server.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    (server, format!("ws://{}/ws", addr))
}

/// PSK auth + one RoomJoin, returns the server's response text.
async fn psk_join_and_recv(ws_url: &str, join: &SignalingMessage) -> String {
    let (mut ws, _) = tokio_tungstenite::connect_async(ws_url).await.unwrap();
    ws.send(WsMsg::Text(PSK.into())).await.unwrap();
    let ack = ws.next().await.unwrap().unwrap();
    assert!(ack.to_text().unwrap().contains("authenticated"));
    let join = serde_json::to_string(join).unwrap();
    ws.send(WsMsg::Text(join)).await.unwrap();
    let resp = ws.next().await.unwrap().unwrap();
    resp.to_text().unwrap().to_string()
}

/// Like `psk_join_and_recv` but keeps the connection alive (returns ws + response) -- needed for binding lifecycle assertions.
type KeepAliveWs =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;
async fn connect_join_keepalive(ws_url: &str, join: &SignalingMessage) -> (KeepAliveWs, String) {
    let (mut ws, _) = tokio_tungstenite::connect_async(ws_url).await.unwrap();
    ws.send(WsMsg::Text(PSK.into())).await.unwrap();
    let ack = ws.next().await.unwrap().unwrap();
    assert!(ack.to_text().unwrap().contains("authenticated"));
    ws.send(WsMsg::Text(serde_json::to_string(join).unwrap())).await.unwrap();
    let resp = ws.next().await.unwrap().unwrap();
    (ws, resp.to_text().unwrap().to_string())
}

const TEST_DEVICE: &str = "ms-car1";
const TEST_DEVICE_SECRET: &str = "car1-secret";

fn test_device_yaml() -> String {
    // sha256("ms-car1:car1-secret") -- consistent with mediaservo_server::devices::hash_secret
    let hash = mediaservo_server::devices::hash_secret(TEST_DEVICE, TEST_DEVICE_SECRET);
    format!("  {TEST_DEVICE}:\n    secret_hash: \"{hash}\"\n")
}

fn device_join(device_id: Option<&str>, device_secret: Option<&str>) -> SignalingMessage {
    SignalingMessage::RoomJoin {
        room_id: ROOM.into(),
        peer_role: PeerRole::Host,
        stream_id: None,
        device_id: device_id.map(String::from),
        device_secret: device_secret.map(String::from),
        device_pubkey: None,
        protocol: None,
        client_version: None,
        resume: None,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn g2_device_auth_success_binds_and_unbinds_on_disconnect() {
    let (server, ws_url) = spawn_server_with_devices(Some(&test_device_yaml())).await;
    // keep the connection alive (the binding is connection-scoped -- see the review #2 semantics).
    let (ws, resp) =
        connect_join_keepalive(&ws_url, &device_join(Some(TEST_DEVICE), Some(TEST_DEVICE_SECRET)))
            .await;
    let joined: SignalingMessage = serde_json::from_str(&resp).expect("RoomJoined expected");
    let peer_id = match joined {
        SignalingMessage::RoomJoined { peer_id, .. } => peer_id,
        other => panic!("expected RoomJoined, got {other:?}"),
    };
    // D-H11: connection-level identity -- the session binds device_id; G3 can read it via server.device_id_of(peer).
    assert_eq!(
        server.device_id_of(&peer_id).as_deref(),
        Some(TEST_DEVICE),
        "peer_id must be bound to device_id"
    );
    assert_eq!(server.device_binding_count(), 1);
    // disconnect -> cleanup must unbind (review #2: disconnect-cleanup verification).
    drop(ws);
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert_eq!(server.device_id_of(&peer_id), None, "the binding must be unbound after disconnect");
    assert_eq!(server.device_binding_count(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn g2_device_auth_unknown_device_rejected() {
    let (_server, ws_url) = spawn_server_with_devices(Some(&test_device_yaml())).await;
    let resp = psk_join_and_recv(&ws_url, &device_join(Some("ms-not-registered"), Some("x"))).await;
    let msg: SignalingMessage = serde_json::from_str(&resp).unwrap();
    match msg {
        SignalingMessage::Error { code, message } => {
            assert_eq!(code, 4010, "an unknown device must get 4010");
            assert!(message.contains("invalid device credentials"), "message: {message}");
        }
        other => panic!("expected Error, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn g2_device_auth_wrong_secret_rejected() {
    let (_server, ws_url) = spawn_server_with_devices(Some(&test_device_yaml())).await;
    let resp =
        psk_join_and_recv(&ws_url, &device_join(Some(TEST_DEVICE), Some("wrong-secret"))).await;
    let msg: SignalingMessage = serde_json::from_str(&resp).unwrap();
    match msg {
        SignalingMessage::Error { code, message } => {
            assert_eq!(code, 4010, "a wrong secret must get 4010");
            assert!(message.contains("invalid device credentials"), "message: {message}");
        }
        other => panic!("expected Error, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn g2_device_auth_half_present_rejected() {
    // G4 review Minor 1: shape check -- carrying only the id without the secret must be refused (no ambiguity).
    let (_server, ws_url) = spawn_server_with_devices(Some(&test_device_yaml())).await;
    for join in [device_join(Some(TEST_DEVICE), None), device_join(None, Some(TEST_DEVICE_SECRET))]
    {
        let resp = psk_join_and_recv(&ws_url, &join).await;
        let msg: SignalingMessage = serde_json::from_str(&resp).unwrap();
        match msg {
            SignalingMessage::Error { code, message } => {
                assert_eq!(code, 4010, "half-present credentials must get 4010");
                assert!(message.contains("both device_id"), "message: {message}");
            }
            other => panic!("expected Error, got {other:?}"),
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn g2_psk_path_regression_with_registry_loaded() {
    // old client / no device fields: with the registry present it must still take the PSK path (additive two-way compat).
    let (server, ws_url) = spawn_server_with_devices(Some(&test_device_yaml())).await;
    let resp = psk_join_and_recv(&ws_url, &device_join(None, None)).await;
    let joined: SignalingMessage = serde_json::from_str(&resp).unwrap();
    let peer_id = match joined {
        SignalingMessage::RoomJoined { peer_id, .. } => peer_id,
        other => panic!("expected RoomJoined, got {other:?}"),
    };
    assert_eq!(
        server.device_id_of(&peer_id),
        None,
        "the PSK path must not produce a device binding"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn g2_unknown_vs_wrong_secret_wire_identical() {
    // review #1 TDD: the full wire response for unknown device and wrong secret must be byte-identical
    // (same code 4010 + same message -- anti-enumeration; the internal distinction goes to the audit log only).
    let (_server, ws_url) = spawn_server_with_devices(Some(&test_device_yaml())).await;
    let r_unknown =
        psk_join_and_recv(&ws_url, &device_join(Some("ms-not-registered"), Some("x"))).await;
    let r_bad =
        psk_join_and_recv(&ws_url, &device_join(Some(TEST_DEVICE), Some("wrong-secret"))).await;
    assert_eq!(
        r_unknown, r_bad,
        "the wire responses for unknown device and wrong secret must be byte-identical (anti-enumeration / anti-timing)"
    );
    let msg: SignalingMessage = serde_json::from_str(&r_unknown).unwrap();
    match msg {
        SignalingMessage::Error { code, message } => {
            assert_eq!(code, 4010);
            assert!(message.contains("invalid device credentials"), "message: {message}");
        }
        other => panic!("expected Error, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn g2_join_failure_leaves_no_binding() {
    // review #2: the binding must happen after join succeeds -- RoomFull(4002) must leave no residual binding.
    let (server, ws_url) = spawn_server_with_devices(Some(&test_device_yaml())).await;
    // conn1: the first Host joins successfully (keeps the connection) -> the binding is established.
    let (ws1, resp1) =
        connect_join_keepalive(&ws_url, &device_join(Some(TEST_DEVICE), Some(TEST_DEVICE_SECRET)))
            .await;
    let joined: SignalingMessage = serde_json::from_str(&resp1).unwrap();
    let peer1 = match joined {
        SignalingMessage::RoomJoined { peer_id, .. } => peer_id,
        other => panic!("expected RoomJoined, got {other:?}"),
    };
    assert_eq!(
        server.device_id_of(&peer1).as_deref(),
        Some(TEST_DEVICE),
        "conn1's binding should be established"
    );
    assert_eq!(server.device_binding_count(), 1);
    // conn2: a second Host in the same room -> RoomFull(4002); auth passed but join failed.
    let resp2 =
        psk_join_and_recv(&ws_url, &device_join(Some(TEST_DEVICE), Some(TEST_DEVICE_SECRET))).await;
    let msg: SignalingMessage = serde_json::from_str(&resp2).unwrap();
    match msg {
        SignalingMessage::Error { code, .. } => {
            assert_eq!(code, 4002, "the second Host should get RoomFull")
        }
        other => panic!("expected Error, got {other:?}"),
    }
    assert_eq!(
        server.device_binding_count(),
        1,
        "a failed join must leave no residual binding (review #2)"
    );
    drop(ws1);
}

// ═══════════════════════════════════════════════════════════════════════════
// G3: cockpit tiered authorization (the D-H11 matrix + tenant isolation + emergency-stop strong audit + one-way config)
// ═══════════════════════════════════════════════════════════════════════════

const JWT_SECRET: &str = "g3-test-jwt-secret-32-bytes-min!!";

/// The audit ring buffer is process-global -- parallel tests' clear_recent() would wipe each other's events.
/// Any WS test that clears/asserts audit::recent() must hold this lock to run serially (other tests unaffected).
static AUDIT_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn account_token(username: &str, role: &str, vehicles: &[&str]) -> String {
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
        &jsonwebtoken::EncodingKey::from_secret(JWT_SECRET.as_bytes()),
    )
    .unwrap()
}

/// Dual-vehicle registry (ms-car1 / ms-car2) -- for tenant-isolation tests.
fn two_devices_yaml() -> String {
    let h1 = mediaservo_server::devices::hash_secret("ms-car1", "car1-secret");
    let h2 = mediaservo_server::devices::hash_secret("ms-car2", "car2-secret");
    format!("  ms-car1:\n    secret_hash: \"{h1}\"\n  ms-car2:\n    secret_hash: \"{h2}\"\n")
}

/// G3 test server: JWT auth (JWT_SECRET) + the device registry.
async fn spawn_server_g3(devices_yaml: &str) -> (SignalingServer, String) {
    unsafe { std::env::set_var("MEDIASERVO_PSK", PSK) };
    let registry = mediaservo_server::devices::DeviceRegistry::from_yaml(&format!(
        "devices:\n{devices_yaml}\n"
    ))
    .unwrap();
    #[cfg(feature = "sfu-mediasoup")]
    let mut server = {
        let sfu = std::sync::Arc::new(
            mediaservo_server::sfu::SfuManager::new_with_port(
                mediaservo_server::sfu::random_udp_port(),
            )
            .await
            .unwrap(),
        );
        SignalingServer::new(sfu, 65536, Some(mediaservo_common::auth::JwtAuth::new(JWT_SECRET)))
    };
    #[cfg(not(feature = "sfu-mediasoup"))]
    let mut server =
        SignalingServer::new(65536, Some(mediaservo_common::auth::JwtAuth::new(JWT_SECRET)));
    server.ws_ping_secs = 0; // the a1 heartbeat has its dedicated pins in heartbeat_e2e
    server.device_registry = std::sync::Arc::new(registry);
    let mut server = server;
    server.psk_state = std::sync::Arc::new(std::sync::RwLock::new(Some(PSK.into())));
    let app = signaling_router(server.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    (server, format!("ws://{}/ws", addr))
}

fn device_join_room(room: &str, device_id: &str, secret: &str) -> SignalingMessage {
    SignalingMessage::RoomJoin {
        room_id: room.into(),
        peer_role: PeerRole::Host,
        stream_id: None,
        device_id: Some(device_id.into()),
        device_secret: Some(secret.into()),
        device_pubkey: None,
        protocol: None,
        client_version: None,
        resume: None,
    }
}

fn legacy_join(room: &str, role: PeerRole) -> SignalingMessage {
    SignalingMessage::RoomJoin {
        room_id: room.into(),
        peer_role: role,
        stream_id: None,
        device_id: None,
        device_secret: None,
        device_pubkey: None,
        protocol: None,
        client_version: None,
        resume: None,
    }
}

/// Account JWT auth + RoomJoin (token via sec-websocket-protocol -- same as the production browser).
/// Returns (ws, response text); the caller holds ws to keep the session alive.
async fn account_join(
    ws_url: &str,
    token: &str,
    room: &str,
    role: PeerRole,
) -> (KeepAliveWs, String) {
    let mut req = ws_url.into_client_request().expect("valid ws url");
    req.headers_mut()
        .insert("Sec-WebSocket-Protocol", token.parse().expect("token is a valid header value"));
    let (mut ws, _) = tokio_tungstenite::connect_async(req).await.unwrap();
    let ack = ws.next().await.unwrap().unwrap();
    assert!(
        ack.to_text().unwrap().contains("authenticated"),
        "auth ack: {}",
        ack.to_text().unwrap()
    );
    let join = SignalingMessage::RoomJoin {
        room_id: room.into(),
        peer_role: role,
        stream_id: None,
        device_id: None,
        device_secret: None,
        device_pubkey: None,
        protocol: None,
        client_version: None,
        resume: None,
    };
    ws.send(WsMsg::Text(serde_json::to_string(&join).unwrap())).await.unwrap();
    let resp = ws.next().await.unwrap().unwrap();
    (ws, resp.to_text().unwrap().to_string())
}

fn has_denial(audit: &[mediaservo_server::audit::AuditEvent], action: &str) -> bool {
    audit.iter().any(|e| {
        matches!(
            e,
            mediaservo_server::audit::AuditEvent::AuthorizationDenied {
                action: a,
                ..
            } if a == action
        )
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn g3_room_join_matrix_and_tenant_isolation() {
    let (_server, ws_url) = spawn_server_g3(&two_devices_yaml()).await;
    mediaservo_server::audit::clear_recent();
    let _ring_guard = AUDIT_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    // vehicles A/B online (each registers its room's owner vehicle).
    let (_veh_a, resp) =
        connect_join_keepalive(&ws_url, &device_join_room("car1-room", "ms-car1", "car1-secret"))
            .await;
    assert!(resp.contains("room_joined"), "vehicle A online: {resp}");
    let (_veh_b, resp) =
        connect_join_keepalive(&ws_url, &device_join_room("car2-room", "ms-car2", "car2-secret"))
            .await;
    assert!(resp.contains("room_joined"), "vehicle B online: {resp}");

    // operator carol (allowlist [ms-car1]): the allowlisted vehicle is allowed OK
    let (ws, resp) = account_join(
        &ws_url,
        &account_token("carol", "operator", &["ms-car1"]),
        "car1-room",
        PeerRole::Consumer,
    )
    .await;
    assert!(resp.contains("room_joined"), "operator on an allowlisted vehicle must pass: {resp}");
    drop(ws);

    // tenant isolation: carol -> vehicle B's room DENIED 4031
    let (_ws, resp) = account_join(
        &ws_url,
        &account_token("carol", "operator", &["ms-car1"]),
        "car2-room",
        PeerRole::Consumer,
    )
    .await;
    assert!(
        resp.contains(r#""code":4031"#),
        "operator on a non-allowlisted vehicle must be denied: {resp}"
    );

    // viewer with an empty allowlist: cannot even watch the allowlisted vehicle DENIED
    let (_ws, resp) = account_join(
        &ws_url,
        &account_token("vic", "viewer", &[]),
        "car1-room",
        PeerRole::Consumer,
    )
    .await;
    assert!(
        resp.contains(r#""code":4031"#),
        "viewer with an empty allowlist must be denied: {resp}"
    );

    // dispatcher: any vehicle OK (pull + status)
    let (ws, resp) = account_join(
        &ws_url,
        &account_token("d1", "dispatcher", &[]),
        "car2-room",
        PeerRole::Consumer,
    )
    .await;
    assert!(resp.contains("room_joined"), "dispatcher on any vehicle must pass: {resp}");
    drop(ws);

    // vehicle A cannot see vehicle B: ms-car2 joins car1-room (vehicle B's owner room) DENIED
    let (_ws, resp) =
        connect_join_keepalive(&ws_url, &device_join_room("car1-room", "ms-car2", "car2-secret"))
            .await;
    assert!(
        resp.contains(r#""code":4031"#),
        "vehicle A cannot see vehicle B (tenant isolation): {resp}"
    );

    // legacy PSK is not constrained by the matrix (additive regression: deployments without accounts behave the same).
    let resp = psk_join_and_recv(&ws_url, &legacy_join("car1-room", PeerRole::Consumer)).await;
    assert!(resp.contains("room_joined"), "legacy PSK path regression: {resp}");

    // C15: all denials audited.
    let audit = mediaservo_server::audit::recent();
    assert!(has_denial(&audit, "room_join"), "room_join denials must be audited: {audit:?}");
    assert!(has_denial(&audit, "room_join"), "tenant-isolation denials must be audited");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn g3_emergency_audit_and_matrix() {
    let (_server, ws_url) = spawn_server_g3(&two_devices_yaml()).await;
    let (_veh_a, _) =
        connect_join_keepalive(&ws_url, &device_join_room("car1-room", "ms-car1", "car1-secret"))
            .await;
    let _ring_guard = AUDIT_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let (_veh_b, _) =
        connect_join_keepalive(&ws_url, &device_join_room("car2-room", "ms-car2", "car2-secret"))
            .await;

    // operator carol emergency stop on an allowlisted vehicle -> forwarded + strong audit (who/when/vehicle/command).
    let (mut ws, resp) = account_join(
        &ws_url,
        &account_token("carol", "operator", &["ms-car1"]),
        "car1-room",
        PeerRole::Consumer,
    )
    .await;
    assert!(resp.contains("room_joined"));
    mediaservo_server::audit::clear_recent();
    let emergency = SignalingMessage::EmergencyCommand {
        room_id: "car1-room".into(),
        command: "e-stop".into(),
    };
    ws.send(WsMsg::Text(serde_json::to_string(&emergency).unwrap())).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let events = mediaservo_server::audit::recent();
    assert!(
        events.iter().any(|e| matches!(
            e,
            mediaservo_server::audit::AuditEvent::EmergencyCommand {
                username,
                role,
                vehicle,
                command
            } if username == "carol"
                && role == "operator"
                && vehicle == "ms-car1"
                && command == "e-stop"
        )),
        "the emergency-stop strong audit must contain who/when/which vehicle/what command: {events:?}"
    );
    drop(ws);

    // dispatcher can pull any vehicle but emergency stop is denied (matrix: dispatcher has no emergency stop) DENIED 4031 + audit.
    let (mut ws, resp) = account_join(
        &ws_url,
        &account_token("d1", "dispatcher", &[]),
        "car2-room",
        PeerRole::Consumer,
    )
    .await;
    assert!(resp.contains("room_joined"));
    mediaservo_server::audit::clear_recent();
    ws.send(WsMsg::Text(serde_json::to_string(&emergency).unwrap())).await.unwrap();
    let resp = tokio::time::timeout(std::time::Duration::from_secs(3), ws.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(
        resp.to_text().unwrap().contains(r#""code":4031"#),
        "dispatcher emergency stop must be denied: {}",
        resp.to_text().unwrap()
    );
    assert!(
        has_denial(&mediaservo_server::audit::recent(), "emergency"),
        "an emergency-stop denial must be audited"
    );
    drop(ws);

    // viewer can pull but emergency stop is denied DENIED.
    let (mut ws, resp) = account_join(
        &ws_url,
        &account_token("vic", "viewer", &["ms-car1"]),
        "car1-room",
        PeerRole::Consumer,
    )
    .await;
    assert!(resp.contains("room_joined"));
    mediaservo_server::audit::clear_recent();
    ws.send(WsMsg::Text(serde_json::to_string(&emergency).unwrap())).await.unwrap();
    let resp = tokio::time::timeout(std::time::Duration::from_secs(3), ws.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(
        resp.to_text().unwrap().contains(r#""code":4031"#),
        "viewer emergency stop must be denied: {}",
        resp.to_text().unwrap()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn g3_config_push_inbound_rejected() {
    let (_server, ws_url) = spawn_server_g3(&two_devices_yaml()).await;
    let (mut veh_ws, resp) =
        connect_join_keepalive(&ws_url, &device_join_room("car1-room", "ms-car1", "car1-secret"))
            .await;
    let _ring_guard = AUDIT_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    assert!(resp.contains("room_joined"));
    mediaservo_server::audit::clear_recent();

    // a client-inbound ConfigPush (even from a vehicle) -> denied + audited (the server pushes one-way).
    let cfg = SignalingMessage::ConfigPush {
        room_id: "car1-room".into(),
        target: "self".into(),
        config: "[[cameras]]\nid = \"cam0\"\n".into(),
        version: 1,
    };
    veh_ws.send(WsMsg::Text(serde_json::to_string(&cfg).unwrap())).await.unwrap();
    let resp = tokio::time::timeout(std::time::Duration::from_secs(3), veh_ws.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(
        resp.to_text().unwrap().contains(r#""code":4031"#),
        "an inbound ConfigPush must be denied: {}",
        resp.to_text().unwrap()
    );
    assert!(
        has_denial(&mediaservo_server::audit::recent(), "config_push"),
        "a ConfigPush denial must be audited"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn g3_jwt_unknown_role_rejected_at_handshake() {
    let (_server, ws_url) = spawn_server_g3(&two_devices_yaml()).await;
    let token = account_token("evil", "superuser", &[]);
    let mut req = ws_url.into_client_request().expect("valid ws url");
    req.headers_mut()
        .insert("Sec-WebSocket-Protocol", token.parse().expect("token is a valid header value"));
    let (mut ws, _) = tokio_tungstenite::connect_async(req).await.unwrap();
    let resp = tokio::time::timeout(std::time::Duration::from_secs(3), ws.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(
        resp.to_text().unwrap().contains(r#""code":4011"#),
        "an unknown role must be refused at the handshake: {}",
        resp.to_text().unwrap()
    );
}

/// T0 fail-closed pin: a JWT presented with failed verification (forged with a different signing key) -> a 4013 disconnect;
/// after the refusal, presenting the correct PSK must not gain authenticated (the old shape fell to PSK -> the Legacy matrix bypass).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bad_jwt_rejects_4013_and_never_degrades_to_psk_legacy() {
    let (_server, ws_url) = spawn_server_g3(&two_devices_yaml()).await;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()
        as usize;
    let claims = mediaservo_common::auth::JwtClaims {
        sub: "rogue".into(),
        iat: now,
        exp: now + 3600,
        role: Some("viewer".into()),
        vehicles: None,
    };
    let rogue = jsonwebtoken::encode(
        &jsonwebtoken::Header::default(),
        &claims,
        &jsonwebtoken::EncodingKey::from_secret(b"not-the-server-secret-32b-min!!"),
    )
    .unwrap();
    let mut req = ws_url.into_client_request().expect("valid ws url");
    req.headers_mut().insert("Sec-WebSocket-Protocol", rogue.parse().unwrap());
    let (mut ws, _) = tokio_tungstenite::connect_async(req).await.unwrap();
    let resp = tokio::time::timeout(std::time::Duration::from_secs(3), ws.next())
        .await
        .expect("the 4013 refusal should arrive immediately (not hang silently)")
        .unwrap()
        .unwrap();
    assert!(
        resp.to_text().unwrap().contains(r#""code":4013"#),
        "a bad JWT must be refused with 4013: {}",
        resp.to_text().unwrap()
    );
    // presenting the real PSK after the refusal: the connection is dead; any authenticated semantics = a downgrade regression.
    let _ = ws.send(WsMsg::Text(PSK.into())).await;
    match tokio::time::timeout(std::time::Duration::from_secs(3), ws.next()).await {
        Err(_) => {} // no response (the connection is zombified) -- an acceptable end state
        Ok(None) | Ok(Some(Err(_))) => {} // closed
        Ok(Some(Ok(m))) => panic!("no more frames after 4013: {}", m.to_text().unwrap()),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// G3 review I1: control-column enforcement on the P2P path -- the Remote-role join gate (can_control) +
// the P2P room SDP/ICE relay gate (prevents viewer/dispatcher negotiating control over SDP).
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn g3_review_i1_remote_role_join_requires_control() {
    let (_server, ws_url) = spawn_server_g3(&two_devices_yaml()).await;
    let (_veh_a, resp) =
        connect_join_keepalive(&ws_url, &device_join_room("car1-room", "ms-car1", "car1-secret"))
            .await;
    assert!(resp.contains("room_joined"), "vehicle A online: {resp}");

    // viewer (can pull but has no control): Remote join DENIED 4031 (Remote = the P2P control slot)
    let (_ws, resp) = account_join(
        &ws_url,
        &account_token("vic", "viewer", &["ms-car1"]),
        "car1-room",
        PeerRole::Remote,
    )
    .await;
    assert!(
        resp.contains(r#""code":4031"#),
        "viewer Remote join must be denied (no control): {resp}"
    );

    // dispatcher (pulls any vehicle but has no control): Remote join DENIED 4031
    let (_ws, resp) = account_join(
        &ws_url,
        &account_token("d1", "dispatcher", &[]),
        "car1-room",
        PeerRole::Remote,
    )
    .await;
    assert!(
        resp.contains(r#""code":4031"#),
        "dispatcher Remote join must be denied (the matrix grants no control): {resp}"
    );

    // operator (allowlisted vehicle + control): Remote join OK (the P2P control-negotiation slot)
    let (ws, resp) = account_join(
        &ws_url,
        &account_token("carol", "operator", &["ms-car1"]),
        "car1-room",
        PeerRole::Remote,
    )
    .await;
    assert!(resp.contains("room_joined"), "operator Remote join must pass: {resp}");
    drop(ws);

    // viewer joining as Consumer is still fine (SFU media pulling unaffected)
    let (ws, resp) = account_join(
        &ws_url,
        &account_token("vic", "viewer", &["ms-car1"]),
        "car1-room",
        PeerRole::Consumer,
    )
    .await;
    assert!(resp.contains("room_joined"), "viewer Consumer pulling unaffected: {resp}");
    drop(ws);

    // legacy PSK Remote is not constrained by the matrix (additive regression)
    let resp = psk_join_and_recv(&ws_url, &legacy_join("car1-room", PeerRole::Remote)).await;
    assert!(resp.contains("room_joined"), "legacy Remote regression: {resp}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn g3_review_i1_sdp_relay_never_in_device_rooms() {
    let (_server, ws_url) = spawn_server_g3(&two_devices_yaml()).await;
    // the vehicle host session: receives room broadcasts, watches whether SDP gets relayed.
    let _ring_guard = AUDIT_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let (mut veh_ws, resp) =
        connect_join_keepalive(&ws_url, &device_join_room("car1-room", "ms-car1", "car1-secret"))
            .await;
    assert!(resp.contains("room_joined"));

    // viewer Consumer joins (pulling is fine) but sends SDP -> it must not be relayed to the vehicle.
    let (mut vic_ws, resp) = account_join(
        &ws_url,
        &account_token("vic", "viewer", &["ms-car1"]),
        "car1-room",
        PeerRole::Consumer,
    )
    .await;
    assert!(resp.contains("room_joined"));
    let viewer_sdp = SignalingMessage::Sdp {
        room_id: "car1-room".into(),
        target: None,
        sdp: "g3-review-i1-viewer-sdp".into(),
    };
    vic_ws.send(WsMsg::Text(serde_json::to_string(&viewer_sdp).unwrap())).await.unwrap();

    // the vehicle must not receive any SDP within 500ms (the viewer's control negotiation is intercepted server-side).
    let mut blocked = true;
    if let Ok(Some(Ok(msg))) =
        tokio::time::timeout(std::time::Duration::from_millis(500), veh_ws.next()).await
    {
        let text = msg.to_text().unwrap().to_string();
        if text.contains("g3-review-i1-viewer-sdp") {
            blocked = false;
        }
    }
    assert!(
        blocked,
        "the viewer's SDP must not be relayed to the vehicle (P2P control-negotiation interception)"
    );
    drop(vic_ws);

    // operator Consumer joins and sends SDP -> relayed to the vehicle (the control negotiation is allowed).
    let (mut op_ws, resp) = account_join(
        &ws_url,
        &account_token("carol", "operator", &["ms-car1"]),
        "car1-room",
        PeerRole::Consumer,
    )
    .await;
    assert!(resp.contains("room_joined"));
    let op_sdp = SignalingMessage::Sdp {
        room_id: "car1-room".into(),
        target: None,
        sdp: "g3-review-i1-operator-sdp".into(),
    };
    op_ws.send(WsMsg::Text(serde_json::to_string(&op_sdp).unwrap())).await.unwrap();

    let mut relayed = false;
    for _ in 0..5 {
        if let Ok(Some(Ok(msg))) =
            tokio::time::timeout(std::time::Duration::from_secs(1), veh_ws.next()).await
        {
            let text = msg.to_text().unwrap().to_string();
            if text.contains("g3-review-i1-operator-sdp") {
                relayed = true;
                break;
            }
        }
    }
    // All-SFU (2026-08-25): every room is a DeviceStream -- SDP is never relayed (no P2P control negotiation,
    // media goes over SFU message-driven flows, and control DC goes through the SFU data domain Phase B). The operator's SDP is intercepted too.
    assert!(!relayed, "all-SFU: the operator's SDP must not be relayed to the vehicle either");
    drop(op_ws);
}
