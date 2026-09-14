//! a2 会话续期（S0.5）集成验证：设备形（auto-enroll）+ 保留窗 + 票轮换/一次性。
//! 脚手架复刻 device_enroll_test（seed 常量与 sig 合同同源）。
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

type Ws = tokio_tungstenite::WebSocketStream<
    tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
>;

fn signing_key() -> SigningKey {
    SigningKey::from_bytes(&std::array::from_fn::<u8, 32, _>(|i| i as u8))
}

fn sign(nonce_b64: &str, device_id: &str, room_id: &str) -> String {
    let mut msg = base64::engine::general_purpose::STANDARD
        .decode(nonce_b64)
        .unwrap();
    msg.extend_from_slice(device_id.as_bytes());
    msg.extend_from_slice(room_id.as_bytes());
    base64::engine::general_purpose::STANDARD.encode(&signing_key().sign(&msg).to_bytes())
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
    server.ws_ping_secs = 0; // 本套件裸 JSON 读环不吃控制帧（心跳归 heartbeat_e2e）
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
    ws.send(WsMsg::Text(serde_json::to_string(msg).unwrap()))
        .await
        .unwrap();
}

/// pubkey 形 join + 验签挑战链走完 → RoomJoined。resume 票可选携带（仅 v3 受理）。
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

/// ① 核心：断链 → 保留窗内 resume → peer 接管 + 票轮换；旧延迟清理作废（seq 守卫）。
#[tokio::test]
async fn resume_rebinds_peer_and_rotates_nonce() {
    let hold = 2u64;
    let (url, srv) = spawn_server(true, hold).await;
    let mut ws = connect(&url).await;
    let j1 = device_join(&mut ws, 3, None).await;
    let SignalingMessage::RoomJoined { peer_id: peer1, session_nonce: Some(nonce1), protocol: Some(3), .. } = &j1 else {
        panic!("v3 车端设备会话必须谈成 3 并下发重挂票, got {j1:?}")
    };
    let (peer1, nonce1) = (peer1.clone(), nonce1.clone());
    assert!(srv.device_bindings.contains_key(&peer1), "join 后应有连接级绑定");

    // 断链（drop = 服务端读端 EOF）→ 进入保留窗
    drop(ws);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        srv.device_bindings.contains_key(&peer1),
        "保留窗内绑定不得被清（延迟清理在等票）"
    );

    // 重挂：认证链全量重跑（新挑战新签）+ resume 票
    let mut ws2 = connect(&url).await;
    let j2 = device_join(&mut ws2, 3, Some(nonce1.clone())).await;
    let (peer2, nonce2) = match &j2 {
        SignalingMessage::RoomJoined { peer_id, session_nonce, .. } => {
            (peer_id.clone(), session_nonce.clone().expect("重挂会话票须轮换"))
        }
        other => panic!("expected RoomJoined, got {other:?}"),
    };
    assert_eq!(peer2, peer1, "resume = 旧 peer 身份接管");
    assert_ne!(nonce2, nonce1, "票一次性：重挂后必须轮换");

    // 越过保留窗：延迟任务须因 seq 不匹配作废（轮换后新票在场 = 非 stale）
    tokio::time::sleep(Duration::from_secs(hold + 1)).await;
    assert!(
        srv.device_bindings.contains_key(&peer1),
        "a2: 重挂后的活会话不得被上一连接的延迟清理误杀"
    );
}

/// ② 重放拒绝：已焚票再次携带 → miss → 全量 join + 旧挂起 peer 即刻接管清理（peer 必换）。
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
    // 第一次：有效票 → 接管
    let mut ws2 = connect(&url).await;
    let j2 = device_join(&mut ws2, 3, Some(nonce1.clone())).await;
    let SignalingMessage::RoomJoined { peer_id: p2, session_nonce: Some(n2), .. } = j2 else {
        panic!("expected armed join, got {j2:?}")
    };
    assert_eq!(p2, peer1);
    drop(ws2);
    // 第二次：同一旧票重放 → 必须 miss（即焚验证），接管路径清旧、新 peer 诞生
    let mut ws3 = connect(&url).await;
    let j3 = device_join(&mut ws3, 3, Some(nonce1.clone())).await;
    let SignalingMessage::RoomJoined { peer_id: p3, .. } = j3 else {
        panic!("重放应回落全量 join（takeover 腾位），got {j3:?}")
    };
    assert_ne!(p3, peer1, "重放票不得接管（burn-and-burn 失守）");
    assert!(n2 != nonce1);
}

/// ③ 协议门：n=2 携带 resume → 忽略；n=2 车端不挂载票（v3 才开域）。
#[tokio::test]
async fn resume_ignored_below_v3() {
    let (url, _srv) = spawn_server(true, 30).await;
    let mut ws = connect(&url).await;
    let j = device_join(&mut ws, 2, Some("bogus".into())).await;
    match j {
        SignalingMessage::RoomJoined { protocol: Some(2), session_nonce: None, .. } => {}
        other => panic!("v2 会话不得见票/不得谈 v3: {other:?}"),
    }
}

/// ④ 身份门：Legacy PSK 车端（无设备凭证）即便 v3 也不挂载（v1 范围=设备会话）。
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
        other => panic!("Legacy 车端不应下发重挂票: {other:?}"),
    }
}
