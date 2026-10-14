//! device-enroll T4/T5: the verify-challenge state machine (WS end-to-end) + admin pending/approve.
//!
//! Dual posture (mirroring admin_psk_test): the ALLOW_DEV_ENROLL gate is injected via `srv.allow_dev_enroll`
//! (= the same-source field after main.rs reads the env at startup). The byte constants reuse the batch-1 sig_vector
//! (seed=bytes(0..=31) / nonce=bytes(0x40..=0x5f) / DEV+ROOM), the same anchor the batch-3 host cross-verifies.
//! Regression-isolation pin for the existing secret path: this file only adds; it never touches old-branch tests.

use base64::Engine as _;
use ed25519_dalek::{Signer, SigningKey};
use futures_util::{SinkExt, StreamExt};
use mediaservo_common::protocol::{PeerRole, SignalingMessage};
use mediaservo_server::devices::{DeviceRegistry, Entry};
use mediaservo_server::signaling::{SignalingServer, signaling_router};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message as WsMsg;

const DEV: &str = "ms-0a1b2c3d4e5f";
const ROOM: &str = "vehicle_cam0";
/// The sig_vector constant vk (derived from seed=bytes(0..=31)).
const VK: &str = "A6EHv/POEL4dcN0Y50vAmWfk1jCbpQ1fHdyGZBJVMbg=";

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

fn b64enc(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn signing_key() -> SigningKey {
    SigningKey::from_bytes(&std::array::from_fn::<u8, 32, _>(|i| i as u8))
}

/// S3 byte contract: sig = sign(nonce_raw(32B) || device_id || room_id).
fn sign(nonce_b64: &str, device_id: &str, room_id: &str) -> String {
    let mut msg = base64::engine::general_purpose::STANDARD.decode(nonce_b64).unwrap();
    msg.extend_from_slice(device_id.as_bytes());
    msg.extend_from_slice(room_id.as_bytes());
    b64enc(&signing_key().sign(&msg).to_bytes())
}

// -- server harness (dual cfg, mirroring admin_psk_test::make_state) --

async fn new_signaling() -> SignalingServer {
    #[cfg(feature = "sfu-mediasoup")]
    {
        let sfu = Arc::new(
            mediaservo_server::sfu::SfuManager::new_with_port(
                mediaservo_server::sfu::random_udp_port(),
            )
            .await
            .unwrap(),
        );
        SignalingServer::new(sfu, 65536, None)
    }
    #[cfg(not(feature = "sfu-mediasoup"))]
    SignalingServer::new(65536, None)
}

async fn spawn_server(reg: Arc<DeviceRegistry>, allow_enroll: bool) -> (String, SignalingServer) {
    let mut srv = new_signaling().await;
    let devices_path = format!("/tmp/ms-enroll-{}.yaml", uuid::Uuid::new_v4());
    srv.device_registry = reg;
    srv.allow_dev_enroll = Arc::new(AtomicBool::new(allow_enroll));
    srv.devices_path = Arc::from(devices_path.as_str());
    let app = signaling_router(srv.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("ws://{addr}/ws"), srv)
}

// ── WS helpers ──────────────────────────────────────────────────────────────

async fn connect(url: &str) -> Ws {
    let (mut ws, _) = tokio_tungstenite::connect_async(url).await.unwrap();
    // psk unconfigured -> the connection gets an auth ack right away (Error code=0 "authenticated"), same as the old path.
    let ack = recv(&mut ws).await;
    assert!(
        matches!(&ack, SignalingMessage::Error { code: 0, .. }),
        "expected auth ack, got {ack:?}"
    );
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

async fn send_raw(ws: &mut Ws, raw: &str) {
    ws.send(WsMsg::Text(raw.into())).await.unwrap();
}

async fn join_pubkey(ws: &mut Ws, device: Option<&str>, secret: Option<&str>) {
    send(
        ws,
        &SignalingMessage::RoomJoin {
            room_id: ROOM.into(),
            peer_role: PeerRole::Host,
            stream_id: None,
            device_id: device.map(str::to_string),
            device_secret: secret.map(str::to_string),
            device_pubkey: Some(VK.into()),
            protocol: None,
            client_version: None,
            resume: None,
        },
    )
    .await;
}

async fn expect_closed(ws: &mut Ws) {
    loop {
        match tokio::time::timeout(Duration::from_secs(9), ws.next()).await {
            Err(_) => panic!("expected connection close, socket still open"),
            Ok(None) | Ok(Some(Err(_))) => return,
            Ok(Some(Ok(WsMsg::Close(_)))) => return,
            Ok(Some(Ok(_))) => continue, // Error frames are allowed before the close
        }
    }
}

/// Asserts a DeviceAuthChallenge is received and returns the nonce.
async fn get_challenge(ws: &mut Ws) -> String {
    match recv(ws).await {
        SignalingMessage::DeviceAuthChallenge { nonce } => {
            assert_eq!(
                base64::engine::general_purpose::STANDARD.decode(&nonce).unwrap().len(),
                32,
                "the nonce must be 32B (S3 OsRng)"
            );
            nonce
        }
        other => panic!("expected DeviceAuthChallenge, got {other:?}"),
    }
}

// -- T4: the state machine (auto gate / pending / replay / timeout / shape refusal / deferred queue) --

/// S5.2 auto on: an unknown pubkey passing verification -> admitted (memory + disk) + Joined passes right after.
#[tokio::test]
async fn auto_enroll_unknown_pubkey_join_passes_and_persists() {
    let reg = Arc::new(DeviceRegistry::empty());
    let (url, srv) = spawn_server(Arc::clone(&reg), true).await;
    let mut ws = connect(&url).await;
    join_pubkey(&mut ws, Some(DEV), None).await;
    let nonce = get_challenge(&mut ws).await;
    send(
        &mut ws,
        &SignalingMessage::DeviceAuthResponse {
            room_id: ROOM.into(),
            sig: sign(&nonce, DEV, ROOM),
        },
    )
    .await;
    let ack = recv(&mut ws).await;
    assert!(matches!(ack, SignalingMessage::RoomJoined { .. }), "{ack:?}");
    assert!(
        matches!(reg.entry_of(DEV), Some(Entry::PublicKey { vk, .. }) if vk == VK),
        "auto admission should be the public_key form: {:?}",
        reg.entry_of(DEV)
    );
    let saved = std::fs::read_to_string(srv.devices_path.as_ref()).unwrap();
    assert!(saved.contains("ed25519:"), "{saved}");
}

/// S5.2 default (manual) mode: an unknown pubkey passing verification -> pending{verified} + DeviceAuthPending + disconnect; not admitted.
#[tokio::test]
async fn manual_mode_unknown_pubkey_goes_pending() {
    let reg = Arc::new(DeviceRegistry::empty());
    let (url, srv) = spawn_server(Arc::clone(&reg), false).await;
    let mut ws = connect(&url).await;
    join_pubkey(&mut ws, Some(DEV), None).await;
    let nonce = get_challenge(&mut ws).await;
    send(
        &mut ws,
        &SignalingMessage::DeviceAuthResponse {
            room_id: ROOM.into(),
            sig: sign(&nonce, DEV, ROOM),
        },
    )
    .await;
    match recv(&mut ws).await {
        SignalingMessage::DeviceAuthPending { device_id } => assert_eq!(device_id, DEV),
        other => panic!("expected DeviceAuthPending, got {other:?}"),
    }
    expect_closed(&mut ws).await;
    let list = srv.pending_devices.list();
    assert_eq!(list.len(), 1, "{list:?}");
    assert_eq!(list[0].0, DEV);
    assert_eq!(list[0].1.vk, VK);
    assert!(list[0].1.verified, "the pending entry should be marked verified=true");
    assert!(reg.entry_of(DEV).is_none(), "manual mode passing verification != admitted");
}

/// nonce one-shot per connection + nonce binding (D-E7): replaying an old sig across connections -> 4010 without polluting pending.
#[tokio::test]
async fn replayed_sig_from_other_connection_rejected() {
    let reg = Arc::new(DeviceRegistry::empty());
    let (url, srv) = spawn_server(Arc::clone(&reg), false).await;

    // connection A: a normal answer -> pending (nonce A burned).
    let mut a = connect(&url).await;
    join_pubkey(&mut a, Some(DEV), None).await;
    let nonce_a = get_challenge(&mut a).await;
    let sig_a = sign(&nonce_a, DEV, ROOM);
    send(
        &mut a,
        &SignalingMessage::DeviceAuthResponse { room_id: ROOM.into(), sig: sig_a.clone() },
    )
    .await;
    assert!(matches!(recv(&mut a).await, SignalingMessage::DeviceAuthPending { .. }));
    expect_closed(&mut a).await;
    let first_seen = srv.pending_devices.get(DEV).unwrap().first_seen_ms;

    // connection B: a new nonce, replaying sig_a -> verification fails -> 4010 + disconnect (not entered into pending / first_seen not overwritten).
    let mut b = connect(&url).await;
    join_pubkey(&mut b, Some(DEV), None).await;
    let _nonce_b = get_challenge(&mut b).await;
    send(&mut b, &SignalingMessage::DeviceAuthResponse { room_id: ROOM.into(), sig: sig_a }).await;
    match recv(&mut b).await {
        SignalingMessage::Error { code, .. } => assert_eq!(code, 4010, "a replay must get 4010"),
        other => panic!("expected Error 4010, got {other:?}"),
    }
    expect_closed(&mut b).await;
    let e = srv.pending_devices.get(DEV).expect("a failed verification must not clear pending");
    assert_eq!(e.first_seen_ms, first_seen, "a replay failure must not overwrite pending");
}

/// S5.2 "5s timeout without an answer = disconnect WARN": not answering -> 4010 + close, zero pending residue.
#[tokio::test]
async fn challenge_timeout_disconnects() {
    let reg = Arc::new(DeviceRegistry::empty());
    let (url, srv) = spawn_server(Arc::clone(&reg), false).await;
    let mut ws = connect(&url).await;
    join_pubkey(&mut ws, Some(DEV), None).await;
    let _nonce = get_challenge(&mut ws).await;
    // stay silent: the server's 5s watchdog sends the uniform 4010 and disconnects.
    match recv(&mut ws).await {
        SignalingMessage::Error { code, .. } => assert_eq!(code, 4010),
        other => panic!("expected Error 4010 on timeout, got {other:?}"),
    }
    expect_closed(&mut ws).await;
    assert!(
        srv.pending_devices.is_empty(),
        "a timeout without verification must not enter pending"
    );
}

/// S5.2 "a registered secret-form device connecting in pubkey form -> refused, no challenge issued" (the S8 migration handling goes to the log).
#[tokio::test]
async fn secret_form_device_rejects_pubkey_join_without_challenge() {
    let reg = Arc::new(DeviceRegistry::empty());
    reg.register_with_secret(DEV, Some("abcdefgh")).unwrap();
    let (url, srv) = spawn_server(Arc::clone(&reg), true).await;
    let mut ws = connect(&url).await;
    join_pubkey(&mut ws, Some(DEV), None).await;
    // the first frame is 4010 -- not a DeviceAuthChallenge (saves an RTT, branch 2).
    match recv(&mut ws).await {
        SignalingMessage::Error { code, .. } => assert_eq!(code, 4010),
        other => panic!("expected immediate Error 4010 (no challenge), got {other:?}"),
    }
    expect_closed(&mut ws).await;
    assert!(srv.pending_devices.is_empty(), "a secret-form refusal must not enter pending");
    assert!(matches!(srv.device_registry.entry_of(DEV), Some(Entry::Secret(_))));
}

/// Registered public-key form: every connection verifies as usual (D-E8); a different-key signature -> 4010 (a key change is refused, S5.2 branch 1).
#[tokio::test]
async fn registered_pubkey_passes_and_other_key_rejected() {
    let yaml =
        format!("devices:\n  {DEV}:\n    public_key: \"ed25519:{VK}\"\n    name: jetson-7\n");
    let reg = Arc::new(DeviceRegistry::from_yaml(&yaml).unwrap());
    let (url, _srv) = spawn_server(Arc::clone(&reg), false).await;

    // correct key: challenge -> answer -> RoomJoined (manual mode is transparent for registered devices -- passing verification is enough).
    let mut ws = connect(&url).await;
    join_pubkey(&mut ws, Some(DEV), None).await;
    let nonce = get_challenge(&mut ws).await;
    send(
        &mut ws,
        &SignalingMessage::DeviceAuthResponse {
            room_id: ROOM.into(),
            sig: sign(&nonce, DEV, ROOM),
        },
    )
    .await;
    assert!(matches!(recv(&mut ws).await, SignalingMessage::RoomJoined { .. }));

    // different key: a new private key's signature -> verify_strict(registered vk) fails -> 4010 + disconnect.
    let mut ws2 = connect(&url).await;
    join_pubkey(&mut ws2, Some(DEV), None).await;
    let nonce2 = get_challenge(&mut ws2).await;
    let rogue = SigningKey::from_bytes(&std::array::from_fn::<u8, 32, _>(|i| 0xF0 ^ i as u8));
    let mut msg = base64::engine::general_purpose::STANDARD.decode(&nonce2).unwrap();
    msg.extend_from_slice(DEV.as_bytes());
    msg.extend_from_slice(ROOM.as_bytes());
    let bad_sig = b64enc(&rogue.sign(&msg).to_bytes());
    send(&mut ws2, &SignalingMessage::DeviceAuthResponse { room_id: ROOM.into(), sig: bad_sig })
        .await;
    match recv(&mut ws2).await {
        SignalingMessage::Error { code, .. } => assert_eq!(code, 4010),
        other => panic!("expected Error 4010 (key change), got {other:?}"),
    }
    expect_closed(&mut ws2).await;
}

/// Shape discipline: both forms present / pubkey without device_id -> a 4000 parameter-class refusal + disconnect (no challenge).
#[tokio::test]
async fn credential_shape_violations_rejected_4000() {
    let reg = Arc::new(DeviceRegistry::empty());
    let (url, _srv) = spawn_server(Arc::clone(&reg), true).await;

    // both forms present
    let mut ws = connect(&url).await;
    join_pubkey(&mut ws, Some(DEV), Some("legacy-secret")).await;
    match recv(&mut ws).await {
        SignalingMessage::Error { code, .. } => {
            assert_eq!(code, 4000, "both forms present must get 4000")
        }
        other => panic!("expected Error 4000, got {other:?}"),
    }
    expect_closed(&mut ws).await;

    // pubkey without device_id
    let mut ws2 = connect(&url).await;
    send(
        &mut ws2,
        &SignalingMessage::RoomJoin {
            room_id: ROOM.into(),
            peer_role: PeerRole::Host,
            stream_id: None,
            device_id: None,
            device_secret: None,
            device_pubkey: Some(VK.into()),
            protocol: None,
            client_version: None,
            resume: None,
        },
    )
    .await;
    match recv(&mut ws2).await {
        SignalingMessage::Error { code, .. } => assert_eq!(code, 4000),
        other => panic!("expected Error 4000, got {other:?}"),
    }
    expect_closed(&mut ws2).await;
}

/// Regression-isolation pin (D-E3): the secret-form path is byte-identical -- correct/wrong secrets get no challenge frame in between.
#[tokio::test]
async fn secret_legacy_path_regression() {
    let reg = Arc::new(DeviceRegistry::empty());
    reg.register_with_secret(DEV, Some("abcdefgh")).unwrap();
    let (url, _srv) = spawn_server(Arc::clone(&reg), true).await;

    // correct secret -> RoomJoined directly (no DeviceAuthChallenge).
    let mut ws = connect(&url).await;
    send(
        &mut ws,
        &SignalingMessage::RoomJoin {
            room_id: ROOM.into(),
            peer_role: PeerRole::Host,
            stream_id: None,
            device_id: Some(DEV.into()),
            device_secret: Some("abcdefgh".into()),
            device_pubkey: None,
            protocol: None,
            client_version: None,
            resume: None,
        },
    )
    .await;
    assert!(matches!(recv(&mut ws).await, SignalingMessage::RoomJoined { .. }));

    // wrong secret -> the uniform 4010 (the old semantics as-is: a single family message against enumeration).
    let mut ws2 = connect(&url).await;
    send(
        &mut ws2,
        &SignalingMessage::RoomJoin {
            room_id: ROOM.into(),
            peer_role: PeerRole::Host,
            stream_id: None,
            device_id: Some(DEV.into()),
            device_secret: Some("wrong-secret".into()),
            device_pubkey: None,
            protocol: None,
            client_version: None,
            resume: None,
        },
    )
    .await;
    match recv(&mut ws2).await {
        SignalingMessage::Error { code, message } => {
            assert_eq!(code, 4010);
            assert!(message.contains("invalid device credentials"), "{message}");
        }
        other => panic!("expected Error 4010, got {other:?}"),
    }
    expect_closed(&mut ws2).await;
}

/// S5.3 deferred-queue flush: a StatusReport inside the challenge window must not enter the pipeline before auth; after passing it is flushed in order.
#[tokio::test]
async fn deferred_messages_flushed_after_auth() {
    let reg = Arc::new(DeviceRegistry::empty());
    let (url, srv) = spawn_server(Arc::clone(&reg), true).await;
    let mut ws = connect(&url).await;
    join_pubkey(&mut ws, Some(DEV), None).await;
    let nonce = get_challenge(&mut ws).await;
    // fire a StatusReport while the challenge is in flight (unauthenticated messages must not be stored).
    let status_raw = r#"{"type":"status_report","room_id":"vehicle_cam0","topics":[],"streams":[],"processes":[],"signal":{"remote_connected":true,"remote_peer_id":"child","children":[],"agent_uptime_secs":1},"ts":1,"config_version":0}"#;
    send_raw(&mut ws, status_raw).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        srv.status_registry.get(ROOM).is_none(),
        "unauthenticated messages must not enter the store"
    );
    send(
        &mut ws,
        &SignalingMessage::DeviceAuthResponse {
            room_id: ROOM.into(),
            sig: sign(&nonce, DEV, ROOM),
        },
    )
    .await;
    assert!(matches!(recv(&mut ws).await, SignalingMessage::RoomJoined { .. }));
    // after the flush the same pipeline consumes the deferred messages (polling absorbs the relay-loop start jitter).
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while srv.status_registry.get(ROOM).is_none() && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        srv.status_registry.get(ROOM).is_some(),
        "deferred messages must be flushed into the pipeline after auth"
    );
}

/// Deferred-queue cap: flooding >16 parseable non-answer messages during the challenge window -> disconnect (prevents unauthenticated memory squatting).
#[tokio::test]
async fn deferred_overflow_disconnects() {
    let reg = Arc::new(DeviceRegistry::empty());
    let (url, _srv) = spawn_server(Arc::clone(&reg), false).await;
    let mut ws = connect(&url).await;
    join_pubkey(&mut ws, Some(DEV), None).await;
    get_challenge(&mut ws).await;
    let filler = r#"{"type":"device_auth_pending","device_id":"filler"}"#;
    for _ in 0..17 {
        send_raw(&mut ws, filler).await;
    }
    expect_closed(&mut ws).await;
}

// -- T5: admin pending/approve (tower::oneshot, mirroring admin_device_test) --

use axum::body::Body;
use http::{Method, Request, StatusCode};
use mediaservo_common::auth::JwtClaims;
use mediaservo_server::accounts::AccountRegistry;
use mediaservo_server::admin::{AdminState, admin_router};
use tower::util::ServiceExt;

async fn make_admin_state(devices_path: String) -> AdminState {
    let reg = Arc::new(DeviceRegistry::empty());
    let (event_tx, _) = tokio::sync::broadcast::channel(256);
    #[cfg(feature = "sfu-mediasoup")]
    let (mut signaling, sfu) = {
        let sfu = Arc::new(
            mediaservo_server::sfu::SfuManager::new_with_port(
                mediaservo_server::sfu::random_udp_port(),
            )
            .await
            .unwrap(),
        );
        (SignalingServer::new(Arc::clone(&sfu), 65536, None), sfu)
    };
    #[cfg(not(feature = "sfu-mediasoup"))]
    let mut signaling = SignalingServer::new(65536, None);
    signaling.device_registry = Arc::clone(&reg);
    signaling.devices_path = Arc::from(devices_path.as_str());
    AdminState {
        signaling,
        event_tx,
        admin_jwt_secret: Some("test-secret-min-32-bytes!!!".into()),
        listen_host: "0.0.0.0".into(),
        listen_port: 9800,
        rate_limit: 100,
        room_capacity: 10,
        consumer_limit_per_stream: 50,
        accounts: Arc::new(AccountRegistry::empty()),
        accounts_path: format!("/tmp/ms-enroll-acc-{}.yaml", uuid::Uuid::new_v4()),
        psk_state: Arc::new(std::sync::RwLock::new(None)),
        config_path: "/tmp/ms-enroll-server.yaml".into(),
        device_registry: reg,
        devices_path,
        api_registry: Arc::new(mediaservo_server::apikeys::ApiKeyRegistry::empty()),
        api_keys_path: format!("/tmp/ms-apikeys-{}.yaml", uuid::Uuid::new_v4()),
        api_token_ttl_secs: 3600,
        #[cfg(feature = "sfu-mediasoup")]
        sfu_manager: sfu,
    }
}

fn admin_token(state: &AdminState) -> String {
    role_token(state, "admin")
}

fn role_token(state: &AdminState, role: &str) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()
        as usize;
    let claims = JwtClaims {
        sub: role.into(),
        iat: now,
        exp: now + 3600,
        role: Some(role.into()),
        vehicles: None,
    };
    jsonwebtoken::encode(
        &jsonwebtoken::Header::default(),
        &claims,
        &jsonwebtoken::EncodingKey::from_secret(
            state.admin_jwt_secret.as_deref().unwrap().as_bytes(),
        ),
    )
    .unwrap()
}

fn auth_request(method: Method, uri: &str, tk: Option<&str>, body: Body) -> Request<Body> {
    let is_post = method == Method::POST;
    let mut b = Request::builder().method(method).uri(uri);
    if let Some(t) = tk {
        b = b.header("Authorization", format!("Bearer {t}"));
    }
    if is_post {
        b = b.header("content-type", "application/json");
    }
    b.body(body).unwrap()
}

async fn json_of(response: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(response.into_body(), 8192).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

/// Role gate (the existing C33 method = auth_middleware): no credential 401; dispatcher POST approve is not read-only -> denied
/// (the existing middleware answers 401 for dispatcher writes; the same-source semantics as admin_device_test devices_dispatcher_read_only).
#[tokio::test]
async fn pending_endpoints_auth_gates() {
    let state =
        make_admin_state(format!("/tmp/ms-enroll-gate-{}.yaml", uuid::Uuid::new_v4())).await;
    let app = admin_router(state.clone());
    let resp = app
        .clone()
        .oneshot(auth_request(Method::GET, "/api/admin/devices/pending", None, Body::empty()))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    let resp = app
        .oneshot(auth_request(
            Method::POST,
            "/api/admin/devices/approve",
            Some(&role_token(&state, "dispatcher")),
            Body::from(r#"{"device_id":"ms-x"}"#),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

/// approve missing pending -> 404.
#[tokio::test]
async fn approve_unknown_returns_404() {
    let state = make_admin_state(format!("/tmp/ms-enroll-404-{}.yaml", uuid::Uuid::new_v4())).await;
    let app = admin_router(state.clone());
    let tk = admin_token(&state);
    let resp = app
        .oneshot(auth_request(
            Method::POST,
            "/api/admin/devices/approve",
            Some(&tk),
            Body::from(r#"{"device_id":"ms-ghost"}"#),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

/// approve full chain + hot effect: a pending hit -> admit (public_key form + name) + save + kick from pending;
/// the new vk can immediately complete one pubkey verification (verify_pubkey = the same entry as the access plane).
#[tokio::test]
async fn approve_full_chain_hot_effective() {
    let devices_path = format!("/tmp/ms-enroll-approve-{}.yaml", uuid::Uuid::new_v4());
    let state = make_admin_state(devices_path.clone()).await;
    // pre-seed pending (simulating a manual-mode unknown device after passing verification, entering the table).
    state.signaling.pending_devices.insert(DEV, VK, true);
    let app = admin_router(state.clone());
    let tk = admin_token(&state);

    // 1) GET pending lists it (the public_key echoed in the ed25519: prefix form).
    let resp = app
        .clone()
        .oneshot(auth_request(Method::GET, "/api/admin/devices/pending", Some(&tk), Body::empty()))
        .await
        .unwrap();
    let body = json_of(resp).await;
    assert_eq!(body["count"], 1, "{body}");
    assert_eq!(body["pending"][0]["device_id"], DEV);
    assert_eq!(body["pending"][0]["public_key"], format!("ed25519:{VK}"));
    assert_eq!(body["pending"][0]["verified"], true);

    // 2) POST approve (with name).
    let resp = app
        .clone()
        .oneshot(auth_request(
            Method::POST,
            "/api/admin/devices/approve",
            Some(&tk),
            Body::from(format!(r#"{{"device_id":"{DEV}","name":"jetson-7"}}"#)),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    // 3) pending emptied + admitted + the disk write-back preserves the form.
    let resp = app
        .oneshot(auth_request(Method::GET, "/api/admin/devices/pending", Some(&tk), Body::empty()))
        .await
        .unwrap();
    assert_eq!(json_of(resp).await["count"], 0);
    assert!(
        matches!(state.device_registry.entry_of(DEV),
            Some(Entry::PublicKey { vk, name })
                if vk == VK && name.as_deref() == Some("jetson-7")),
        "{:?}",
        state.device_registry.entry_of(DEV)
    );
    let saved = std::fs::read_to_string(&devices_path).unwrap();
    assert!(saved.contains("ed25519:") && saved.contains("jetson-7"), "{saved}");

    // 4) hot effect: the newly registered vk immediately passes one sig_vector challenge verification (the next connection passes).
    let nonce_b64 = b64enc(&(0x40u8..0x60).collect::<Vec<u8>>());
    let mut msg = (0x40u8..0x60).collect::<Vec<u8>>();
    msg.extend_from_slice(DEV.as_bytes());
    msg.extend_from_slice(ROOM.as_bytes());
    let sig = b64enc(&signing_key().sign(&msg).to_bytes());
    assert_eq!(state.device_registry.verify_pubkey(DEV, &nonce_b64, ROOM, &sig), Ok(()));
}

/// Race surface: on approve the device_id is already registered -> 409 and pending is restored (the device re-report window is not swallowed).
#[tokio::test]
async fn approve_conflict_409_restores_pending() {
    let devices_path = format!("/tmp/ms-enroll-conf-{}.yaml", uuid::Uuid::new_v4());
    let state = make_admin_state(devices_path).await;
    state.device_registry.register_with_secret(DEV, Some("abcdefgh")).unwrap();
    state.signaling.pending_devices.insert(DEV, VK, true);
    let app = admin_router(state.clone());
    let tk = admin_token(&state);
    let resp = app
        .oneshot(auth_request(
            Method::POST,
            "/api/admin/devices/approve",
            Some(&tk),
            Body::from(format!(r#"{{"device_id":"{DEV}"}}"#)),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
    assert!(
        state.signaling.pending_devices.get(DEV).is_some(),
        "after 409 pending must be restored"
    );
    // the registered entry untouched (still the secret form).
    assert!(matches!(state.device_registry.entry_of(DEV), Some(Entry::Secret(_))));
}
