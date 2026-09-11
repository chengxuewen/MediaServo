//! device-enroll T4/T5: 验签挑战状态机（WS 端到端）+ admin pending/approve。
//!
//! 双姿态（仿 admin_psk_test）：ALLOW_DEV_ENROLL 门经 `srv.allow_dev_enroll` 注入
//! （= main.rs 启动读 env 后同源字段）。字节常量复用批1 sig_vector
//! （seed=bytes(0..=31) / nonce=bytes(0x40..=0x5f) / DEV+ROOM），批3 host 交叉复验同锚。
//! 既有 secret 路径回归隔离钉：本文件仅新增，不触碰旧分支测试。

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
/// sig_vector 常量 vk（seed=bytes(0..=31) 派生）。
const VK: &str = "A6EHv/POEL4dcN0Y50vAmWfk1jCbpQ1fHdyGZBJVMbg=";

type Ws = tokio_tungstenite::WebSocketStream<
    tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
>;

fn b64enc(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn signing_key() -> SigningKey {
    SigningKey::from_bytes(&std::array::from_fn::<u8, 32, _>(|i| i as u8))
}

/// §3 字节合同: sig = sign(nonce_raw(32B) ‖ device_id ‖ room_id)。
fn sign(nonce_b64: &str, device_id: &str, room_id: &str) -> String {
    let mut msg = base64::engine::general_purpose::STANDARD
        .decode(nonce_b64)
        .unwrap();
    msg.extend_from_slice(device_id.as_bytes());
    msg.extend_from_slice(room_id.as_bytes());
    b64enc(&signing_key().sign(&msg).to_bytes())
}

// ── server harness（双 cfg 仿 admin_psk_test::make_state）──────────────────

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
    // psk 未配置 → 建连即 auth ack（Error code=0 "authenticated"），与旧路径一致。
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
    ws.send(WsMsg::Text(serde_json::to_string(msg).unwrap()))
        .await
        .unwrap();
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
            Ok(Some(Ok(_))) => continue, // 关闭前允许有 Error 帧
        }
    }
}

/// 断言收到 DeviceAuthChallenge 并取回 nonce。
async fn get_challenge(ws: &mut Ws) -> String {
    match recv(ws).await {
        SignalingMessage::DeviceAuthChallenge { nonce } => {
            assert_eq!(
                base64::engine::general_purpose::STANDARD.decode(&nonce).unwrap().len(),
                32,
                "nonce 必须 32B（§3 OsRng）"
            );
            nonce
        }
        other => panic!("expected DeviceAuthChallenge, got {other:?}"),
    }
}

// ── T4: 状态机（auto 门 / pending / 重放 / 超时 / 形态拒 / 延后队列）────────

/// §5.2 auto 开: 陌生 pubkey 验签过 → 收录（内存+落盘）+ Joined 即过。
#[tokio::test]
async fn auto_enroll_unknown_pubkey_join_passes_and_persists() {
    let reg = Arc::new(DeviceRegistry::empty());
    let (url, srv) = spawn_server(Arc::clone(&reg), true).await;
    let mut ws = connect(&url).await;
    join_pubkey(&mut ws, Some(DEV), None).await;
    let nonce = get_challenge(&mut ws).await;
    send(
        &mut ws,
        &SignalingMessage::DeviceAuthResponse { room_id: ROOM.into(), sig: sign(&nonce, DEV, ROOM) },
    )
    .await;
    let ack = recv(&mut ws).await;
    assert!(matches!(ack, SignalingMessage::RoomJoined { .. }), "{ack:?}");
    assert!(
        matches!(reg.entry_of(DEV), Some(Entry::PublicKey { vk, .. }) if vk == VK),
        "auto 收录应为 public_key 形: {:?}",
        reg.entry_of(DEV)
    );
    let saved = std::fs::read_to_string(srv.devices_path.as_ref()).unwrap();
    assert!(saved.contains("ed25519:"), "{saved}");
}

/// §5.2 默认(手动)档: 陌生 pubkey 验签过 → pending{verified} + DeviceAuthPending + 断连；不入册。
#[tokio::test]
async fn manual_mode_unknown_pubkey_goes_pending() {
    let reg = Arc::new(DeviceRegistry::empty());
    let (url, srv) = spawn_server(Arc::clone(&reg), false).await;
    let mut ws = connect(&url).await;
    join_pubkey(&mut ws, Some(DEV), None).await;
    let nonce = get_challenge(&mut ws).await;
    send(
        &mut ws,
        &SignalingMessage::DeviceAuthResponse { room_id: ROOM.into(), sig: sign(&nonce, DEV, ROOM) },
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
    assert!(list[0].1.verified, "pending 条目应标 verified=true");
    assert!(reg.entry_of(DEV).is_none(), "手动档验签过 ≠ 入册");
}

/// nonce 一连接一次性 + nonce 绑定（D-E7）: 跨连接重放旧 sig → 4010 且不污染 pending。
#[tokio::test]
async fn replayed_sig_from_other_connection_rejected() {
    let reg = Arc::new(DeviceRegistry::empty());
    let (url, srv) = spawn_server(Arc::clone(&reg), false).await;

    // 连接 A: 正常应答 → pending（nonce A 已焚）。
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

    // 连接 B: 新 nonce，重放 sig_a → 验签败 → 4010 + 断连（不入 pending / 不覆盖首见）。
    let mut b = connect(&url).await;
    join_pubkey(&mut b, Some(DEV), None).await;
    let _nonce_b = get_challenge(&mut b).await;
    send(
        &mut b,
        &SignalingMessage::DeviceAuthResponse { room_id: ROOM.into(), sig: sig_a },
    )
    .await;
    match recv(&mut b).await {
        SignalingMessage::Error { code, .. } => assert_eq!(code, 4010, "重放必 4010"),
        other => panic!("expected Error 4010, got {other:?}"),
    }
    expect_closed(&mut b).await;
    let e = srv.pending_devices.get(DEV).expect("验签败不得清除 pending");
    assert_eq!(e.first_seen_ms, first_seen, "重放失败不得覆盖 pending");
}

/// §5.2 「5s 超时未答 = 断连 WARN」：不应答 → 4010 + 关闭，pending 零残留。
#[tokio::test]
async fn challenge_timeout_disconnects() {
    let reg = Arc::new(DeviceRegistry::empty());
    let (url, srv) = spawn_server(Arc::clone(&reg), false).await;
    let mut ws = connect(&url).await;
    join_pubkey(&mut ws, Some(DEV), None).await;
    let _nonce = get_challenge(&mut ws).await;
    // 保持沉默：server 5s watchdog 发统一 4010 后断连。
    match recv(&mut ws).await {
        SignalingMessage::Error { code, .. } => assert_eq!(code, 4010),
        other => panic!("expected Error 4010 on timeout, got {other:?}"),
    }
    expect_closed(&mut ws).await;
    assert!(srv.pending_devices.is_empty(), "超时未验签不得入 pending");
}

/// §5.2 「已登记 secret 形设备以 pubkey 形来连 → 拒 + 不发挑战」（§8 迁移处置进日志）。
#[tokio::test]
async fn secret_form_device_rejects_pubkey_join_without_challenge() {
    let reg = Arc::new(DeviceRegistry::empty());
    reg.register_with_secret(DEV, Some("abcdefgh")).unwrap();
    let (url, srv) = spawn_server(Arc::clone(&reg), true).await;
    let mut ws = connect(&url).await;
    join_pubkey(&mut ws, Some(DEV), None).await;
    // 首帧即 4010 —— 不是 DeviceAuthChallenge（省 RTT，分支②）。
    match recv(&mut ws).await {
        SignalingMessage::Error { code, .. } => assert_eq!(code, 4010),
        other => panic!("expected immediate Error 4010 (no challenge), got {other:?}"),
    }
    expect_closed(&mut ws).await;
    assert!(srv.pending_devices.is_empty(), "secret 形拒绝不得入 pending");
    assert!(matches!(srv.device_registry.entry_of(DEV), Some(Entry::Secret(_))));
}

/// 已登记公钥形: 每次连接照常验签（D-E8）；异钥签名 → 4010（换钥拒，§5.2 分支①）。
#[tokio::test]
async fn registered_pubkey_passes_and_other_key_rejected() {
    let yaml = format!("devices:\n  {DEV}:\n    public_key: \"ed25519:{VK}\"\n    name: jetson-7\n");
    let reg = Arc::new(DeviceRegistry::from_yaml(&yaml).unwrap());
    let (url, _srv) = spawn_server(Arc::clone(&reg), false).await;

    // 正钥: 挑战→应答→RoomJoined（手动档对已登记设备无感——验签过即放行）。
    let mut ws = connect(&url).await;
    join_pubkey(&mut ws, Some(DEV), None).await;
    let nonce = get_challenge(&mut ws).await;
    send(
        &mut ws,
        &SignalingMessage::DeviceAuthResponse { room_id: ROOM.into(), sig: sign(&nonce, DEV, ROOM) },
    )
    .await;
    assert!(matches!(recv(&mut ws).await, SignalingMessage::RoomJoined { .. }));

    // 异钥: 新私钥签名 → verify_strict(登记 vk) 败 → 4010 + 断连。
    let mut ws2 = connect(&url).await;
    join_pubkey(&mut ws2, Some(DEV), None).await;
    let nonce2 = get_challenge(&mut ws2).await;
    let rogue = SigningKey::from_bytes(&std::array::from_fn::<u8, 32, _>(|i| 0xF0 ^ i as u8));
    let mut msg = base64::engine::general_purpose::STANDARD.decode(&nonce2).unwrap();
    msg.extend_from_slice(DEV.as_bytes());
    msg.extend_from_slice(ROOM.as_bytes());
    let bad_sig = b64enc(&rogue.sign(&msg).to_bytes());
    send(
        &mut ws2,
        &SignalingMessage::DeviceAuthResponse { room_id: ROOM.into(), sig: bad_sig },
    )
    .await;
    match recv(&mut ws2).await {
        SignalingMessage::Error { code, .. } => assert_eq!(code, 4010),
        other => panic!("expected Error 4010 (换钥), got {other:?}"),
    }
    expect_closed(&mut ws2).await;
}

/// 形态纪律: 两形同现 / pubkey 缺 device_id → 4000 参数类拒 + 断连（不发挑战）。
#[tokio::test]
async fn credential_shape_violations_rejected_4000() {
    let reg = Arc::new(DeviceRegistry::empty());
    let (url, _srv) = spawn_server(Arc::clone(&reg), true).await;

    // 两形同现
    let mut ws = connect(&url).await;
    join_pubkey(&mut ws, Some(DEV), Some("legacy-secret")).await;
    match recv(&mut ws).await {
        SignalingMessage::Error { code, .. } => assert_eq!(code, 4000, "两形同现必 4000"),
        other => panic!("expected Error 4000, got {other:?}"),
    }
    expect_closed(&mut ws).await;

    // pubkey 无 device_id
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
        },
    )
    .await;
    match recv(&mut ws2).await {
        SignalingMessage::Error { code, .. } => assert_eq!(code, 4000),
        other => panic!("expected Error 4000, got {other:?}"),
    }
    expect_closed(&mut ws2).await;
}

/// 回归隔离钉（D-E3）: secret 形路径逐字节不变——正确/错误 secret 均无挑战帧介入。
#[tokio::test]
async fn secret_legacy_path_regression() {
    let reg = Arc::new(DeviceRegistry::empty());
    reg.register_with_secret(DEV, Some("abcdefgh")).unwrap();
    let (url, _srv) = spawn_server(Arc::clone(&reg), true).await;

    // 正确 secret → 直接 RoomJoined（无 DeviceAuthChallenge）。
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
        },
    )
    .await;
    assert!(matches!(recv(&mut ws).await, SignalingMessage::RoomJoined { .. }));

    // 错误 secret → 统一 4010（旧语义原样：单一家族消息防枚举）。
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

/// §5.3 延后队列冲刷: 挑战窗口内的 StatusReport 鉴权前不得入管线，通过后按序冲刷。
#[tokio::test]
async fn deferred_messages_flushed_after_auth() {
    let reg = Arc::new(DeviceRegistry::empty());
    let (url, srv) = spawn_server(Arc::clone(&reg), true).await;
    let mut ws = connect(&url).await;
    join_pubkey(&mut ws, Some(DEV), None).await;
    let nonce = get_challenge(&mut ws).await;
    // 挑战在途时抢发 StatusReport（未鉴权不得被 store）。
    let status_raw = r#"{"type":"status_report","room_id":"vehicle_cam0","topics":[],"streams":[],"processes":[],"signal":{"remote_connected":true,"remote_peer_id":"child","children":[],"agent_uptime_secs":1},"ts":1,"config_version":0}"#;
    send_raw(&mut ws, status_raw).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(srv.status_registry.get(ROOM).is_none(), "未鉴权消息不得入 store");
    send(
        &mut ws,
        &SignalingMessage::DeviceAuthResponse { room_id: ROOM.into(), sig: sign(&nonce, DEV, ROOM) },
    )
    .await;
    assert!(matches!(recv(&mut ws).await, SignalingMessage::RoomJoined { .. }));
    // 冲刷后同一管线消费延后消息（轮询吸收 relay loop 启动抖）。
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while srv.status_registry.get(ROOM).is_none() && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(srv.status_registry.get(ROOM).is_some(), "延后消息必须在鉴权后冲刷入管线");
}

/// 延后队列封顶: 挑战窗口洪泛 >16 条可解析非应答消息 → 断连（防未鉴权占内存）。
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

// ── T5: admin pending/approve（tower::oneshot，仿 admin_device_test）────────

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
        #[cfg(feature = "sfu-mediasoup")]
        sfu_manager: sfu,
    }
}

fn admin_token(state: &AdminState) -> String {
    role_token(state, "admin")
}

fn role_token(state: &AdminState, role: &str) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as usize;
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

/// 角色门（C33 现有法=auth_middleware）: 无凭证 401；dispatcher POST approve 非只读 → 拒
/// （既有 middleware 对 dispatcher 写操作回 401，admin_device_test devices_dispatcher_read_only 同源语义）。
#[tokio::test]
async fn pending_endpoints_auth_gates() {
    let state =
        make_admin_state(format!("/tmp/ms-enroll-gate-{}.yaml", uuid::Uuid::new_v4())).await;
    let app = admin_router(state.clone());
    let resp = app
        .clone()
        .oneshot(auth_request(Method::GET, "/api/admin/devices/pending", None, Body::empty()))
        .await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    let resp = app
        .oneshot(auth_request(
            Method::POST,
            "/api/admin/devices/approve",
            Some(&role_token(&state, "dispatcher")),
            Body::from(r#"{"device_id":"ms-x"}"#),
        ))
        .await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

/// approve 未命中 pending → 404。
#[tokio::test]
async fn approve_unknown_returns_404() {
    let state =
        make_admin_state(format!("/tmp/ms-enroll-404-{}.yaml", uuid::Uuid::new_v4())).await;
    let app = admin_router(state.clone());
    let tk = admin_token(&state);
    let resp = app
        .oneshot(auth_request(
            Method::POST,
            "/api/admin/devices/approve",
            Some(&tk),
            Body::from(r#"{"device_id":"ms-ghost"}"#),
        ))
        .await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

/// approve 全链 + 热生效: pending 命中 → 入册(public_key 形+name)+save+踢出；
/// 新 vk 立即可完成一次 pubkey 验签（verify_pubkey = 接入面同一入口）。
#[tokio::test]
async fn approve_full_chain_hot_effective() {
    let devices_path = format!("/tmp/ms-enroll-approve-{}.yaml", uuid::Uuid::new_v4());
    let state = make_admin_state(devices_path.clone()).await;
    // 预置 pending（模拟手动档陌生设备验签过后入表）。
    state.signaling.pending_devices.insert(DEV, VK, true);
    let app = admin_router(state.clone());
    let tk = admin_token(&state);

    // 1) GET pending 列出（public_key 回显 ed25519: 前缀形）。
    let resp = app
        .clone()
        .oneshot(auth_request(
            Method::GET,
            "/api/admin/devices/pending",
            Some(&tk),
            Body::empty(),
        ))
        .await.unwrap();
    let body = json_of(resp).await;
    assert_eq!(body["count"], 1, "{body}");
    assert_eq!(body["pending"][0]["device_id"], DEV);
    assert_eq!(body["pending"][0]["public_key"], format!("ed25519:{VK}"));
    assert_eq!(body["pending"][0]["verified"], true);

    // 2) POST approve（带 name）。
    let resp = app
        .clone()
        .oneshot(auth_request(
            Method::POST,
            "/api/admin/devices/approve",
            Some(&tk),
            Body::from(format!(r#"{{"device_id":"{DEV}","name":"jetson-7"}}"#)),
        ))
        .await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    // 3) pending 清空 + 入册 + 落盘保形。
    let resp = app
        .oneshot(auth_request(
            Method::GET,
            "/api/admin/devices/pending",
            Some(&tk),
            Body::empty(),
        ))
        .await.unwrap();
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

    // 4) 热生效: 新登记 vk 立即通过一次 sig_vector 挑战验签（下次连接即放行）。
    let nonce_b64 = b64enc(&(0x40u8..0x60).collect::<Vec<u8>>());
    let mut msg = (0x40u8..0x60).collect::<Vec<u8>>();
    msg.extend_from_slice(DEV.as_bytes());
    msg.extend_from_slice(ROOM.as_bytes());
    let sig = b64enc(&signing_key().sign(&msg).to_bytes());
    assert_eq!(state.device_registry.verify_pubkey(DEV, &nonce_b64, ROOM, &sig), Ok(()));
}

/// 竞态面: approve 时 device_id 已在册 → 409 且 pending 恢复（不吞设备重报窗口）。
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
        .await.unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
    assert!(
        state.signaling.pending_devices.get(DEV).is_some(),
        "409 后 pending 必须恢复"
    );
    // 已注册条目未被触碰（仍是 secret 形）。
    assert!(matches!(state.device_registry.entry_of(DEV), Some(Entry::Secret(_))));
}
