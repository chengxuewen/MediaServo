//! a2 session resume (S0.5) integration verification: the device form (auto-enroll) + the retention window + ticket rotation/one-shot.
//! Scaffolding replicates device_enroll_test (seed constants and the sig contract share a source).
use base64::Engine as _;
use ed25519_dalek::{Signer, SigningKey};
use futures_util::{SinkExt, StreamExt};
use mediaservo_common::protocol::{PeerRole, SignalingMessage};
use mediaservo_server::devices::DeviceRegistry;
use mediaservo_server::signaling::{SignalingServer, signaling_router};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message as WsMsg;

const DEV: &str = "ms-0a1b2c3d4e5f";
const ROOM: &str = "vehicle_cam0";

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

fn signing_key() -> SigningKey {
    SigningKey::from_bytes(&std::array::from_fn::<u8, 32, _>(|i| i as u8))
}

fn sign(nonce_b64: &str, device_id: &str, room_id: &str) -> String {
    let mut msg = base64::engine::general_purpose::STANDARD.decode(nonce_b64).unwrap();
    msg.extend_from_slice(device_id.as_bytes());
    msg.extend_from_slice(room_id.as_bytes());
    base64::engine::general_purpose::STANDARD.encode(signing_key().sign(&msg).to_bytes())
}

async fn spawn_server(allow_enroll: bool, hold_secs: u64) -> (String, SignalingServer) {
    #[cfg(feature = "sfu-mediasoup")]
    let server = {
        let sfu = Arc::new(
            mediaservo_server::sfu::SfuManager::new_with_port(
                mediaservo_server::sfu::random_udp_port(),
            )
            .await
            .unwrap(),
        );
        SignalingServer::new(sfu, 65536, None)
    };
    #[cfg(not(feature = "sfu-mediasoup"))]
    let server = SignalingServer::new(65536, None);
    let mut server = server;
    server.ws_ping_secs = 0; // this suite's raw JSON read loop must not eat control frames (the heartbeat lives in heartbeat_e2e)
    server.ws_resume_hold_secs = hold_secs;
    server.allow_dev_enroll = Arc::new(AtomicBool::new(allow_enroll));
    server.device_registry = Arc::new(DeviceRegistry::empty());
    server.devices_path =
        Arc::from(format!("/tmp/ms-resume-{}.yaml", uuid::Uuid::new_v4()).as_str());
    let app = signaling_router(server.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("ws://{addr}/ws"), server)
}

async fn connect(url: &str) -> Ws {
    let (mut ws, _) = tokio_tungstenite::connect_async(url).await.unwrap();
    match recv(&mut ws).await {
        SignalingMessage::Error { code: 0, .. } => {}
        other => panic!("expected auth ack, got {other:?}"),
    }
    ws
}

async fn recv(ws: &mut Ws) -> SignalingMessage {
    let m = tokio::time::timeout(Duration::from_secs(8), ws.next())
        .await
        .expect("recv timeout")
        .expect("stream ended")
        .expect("ws error");
    let text = m.to_text().expect("non-text frame").to_string();
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("unparsable frame {text}: {e}"))
}

async fn send(ws: &mut Ws, msg: &SignalingMessage) {
    ws.send(WsMsg::Text(serde_json::to_string(msg).unwrap())).await.unwrap();
}

/// pubkey-form join completing the verify-challenge chain -> RoomJoined. The resume ticket is optionally carried (accepted only on v3).
async fn device_join(ws: &mut Ws, protocol: u32, resume: Option<String>) -> SignalingMessage {
    send(
        ws,
        &SignalingMessage::RoomJoin {
            room_id: ROOM.into(),
            peer_role: PeerRole::Host,
            stream_id: None,
            device_id: Some(DEV.into()),
            device_secret: None,
            device_pubkey: Some(
                base64::engine::general_purpose::STANDARD
                    .encode(signing_key().verifying_key().to_bytes()),
            ),
            protocol: Some(protocol),
            client_version: None,
            resume,
        },
    )
    .await;
    match recv(ws).await {
        SignalingMessage::DeviceAuthChallenge { nonce } => {
            send(
                ws,
                &SignalingMessage::DeviceAuthResponse {
                    room_id: ROOM.into(),
                    sig: sign(&nonce, DEV, ROOM),
                },
            )
            .await;
        }
        other => panic!("expected challenge, got {other:?}"),
    }
    recv(ws).await
}

/// 1. core: disconnect -> resume within the retention window -> peer takeover + ticket rotation; the old delayed cleanup is voided (the seq guard).
#[tokio::test]
async fn resume_rebinds_peer_and_rotates_nonce() {
    let hold = 2u64;
    let (url, srv) = spawn_server(true, hold).await;
    let mut ws = connect(&url).await;
    let j1 = device_join(&mut ws, 3, None).await;
    let SignalingMessage::RoomJoined {
        peer_id: peer1,
        session_nonce: Some(nonce1),
        protocol: Some(3),
        ..
    } = &j1
    else {
        panic!(
            "a v3 vehicle device session must negotiate 3 and issue a re-attach ticket, got {j1:?}"
        )
    };
    let (peer1, nonce1) = (peer1.clone(), nonce1.clone());
    assert!(
        srv.device_bindings.contains_key(&peer1),
        "there should be a connection-level binding after join"
    );

    // disconnect (drop = the server's read end sees EOF) -> enters the retention window
    drop(ws);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        srv.device_bindings.contains_key(&peer1),
        "within the retention window the binding must not be cleared (the delayed cleanup is waiting on the ticket)"
    );

    // re-attach: the auth chain fully re-run (a new challenge, new signature) + the resume ticket
    let mut ws2 = connect(&url).await;
    let j2 = device_join(&mut ws2, 3, Some(nonce1.clone())).await;
    let (peer2, nonce2) = match &j2 {
        SignalingMessage::RoomJoined { peer_id, session_nonce, .. } => (
            peer_id.clone(),
            session_nonce.clone().expect("the re-attach session ticket must be rotated"),
        ),
        other => panic!("expected RoomJoined, got {other:?}"),
    };
    assert_eq!(peer2, peer1, "resume = the old peer identity taking over");
    assert_ne!(nonce2, nonce1, "tickets are one-shot: they must rotate after a re-attach");

    // past the retention window: the delayed task must be voided by the seq mismatch (after rotation the new ticket is present = not stale)
    tokio::time::sleep(Duration::from_secs(hold + 1)).await;
    assert!(
        srv.device_bindings.contains_key(&peer1),
        "a2: a live session after a re-attach must not be killed by the previous connection's delayed cleanup"
    );
}

/// 2. replay refusal: a burned ticket presented again -> miss -> full join + the old pending peer taken over and cleared immediately (the peer must change).
#[tokio::test]
async fn burned_nonce_replay_falls_back_to_full_join() {
    let (url, _srv) = spawn_server(true, 30).await;
    let mut ws = connect(&url).await;
    let j1 = device_join(&mut ws, 3, None).await;
    let nonce1 = match &j1 {
        SignalingMessage::RoomJoined { session_nonce: Some(n), .. } => n.clone(),
        other => panic!("{other:?}"),
    };
    let peer1 = match &j1 {
        SignalingMessage::RoomJoined { peer_id, .. } => peer_id.clone(),
        _ => unreachable!(),
    };
    drop(ws);
    // first: a valid ticket -> takeover
    let mut ws2 = connect(&url).await;
    let j2 = device_join(&mut ws2, 3, Some(nonce1.clone())).await;
    let SignalingMessage::RoomJoined { peer_id: p2, session_nonce: Some(n2), .. } = j2 else {
        panic!("expected armed join, got {j2:?}")
    };
    assert_eq!(p2, peer1);
    drop(ws2);
    // second: replaying the same old ticket -> must miss (burn-on-verify), the takeover path clears the old and a new peer is born
    let mut ws3 = connect(&url).await;
    let j3 = device_join(&mut ws3, 3, Some(nonce1.clone())).await;
    let SignalingMessage::RoomJoined { peer_id: p3, .. } = j3 else {
        panic!("a replay should fall back to a full join (takeover frees the slot), got {j3:?}")
    };
    assert_ne!(p3, peer1, "a replayed ticket must not take over (burn-and-burn failed)");
    assert!(n2 != nonce1);
}

/// 3. the protocol gate: n=2 carrying resume -> ignored; n=2 vehicles do not attach tickets (v3 opens the domain).
#[tokio::test]
async fn resume_ignored_below_v3() {
    let (url, _srv) = spawn_server(true, 30).await;
    let mut ws = connect(&url).await;
    let j = device_join(&mut ws, 2, Some("bogus".into())).await;
    match j {
        SignalingMessage::RoomJoined { protocol: Some(2), session_nonce: None, .. } => {}
        other => panic!("a v2 session must not see tickets / must not negotiate v3: {other:?}"),
    }
}

/// 4. the identity gate: a Legacy PSK vehicle (no device credentials) does not attach even on v3 (the v1 scope = device sessions).
#[tokio::test]
async fn legacy_host_not_armed() {
    let (url, _srv) = spawn_server(false, 30).await;
    let mut ws = connect(&url).await;
    send(
        &mut ws,
        &SignalingMessage::RoomJoin {
            room_id: "legacy-room".into(),
            peer_role: PeerRole::Host,
            stream_id: None,
            device_id: None,
            device_secret: None,
            device_pubkey: None,
            protocol: Some(3),
            client_version: None,
            resume: None,
        },
    )
    .await;
    match recv(&mut ws).await {
        SignalingMessage::RoomJoined { protocol: Some(3), session_nonce: None, .. } => {}
        other => panic!("a Legacy vehicle should not be issued a re-attach ticket: {other:?}"),
    }
}
