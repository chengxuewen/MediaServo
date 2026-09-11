//! Phase 1b: SignalClient 测试（本地 mock WS server）。

use futures_util::{SinkExt, StreamExt};
use mediaservo_common::protocol::{PeerRole, SignalingMessage};
use base64::Engine as _;
use mediaservo_link::{DeviceIdentity, LinkError, SignalClient, SignalEvent};
use tokio_tungstenite::tungstenite::Message;

#[tokio::test]
async fn connect_auth_join_and_roundtrip() {
    // 本地 mock WS server：PSK 认证 → RoomJoin → RoomJoined → echo
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();

        // 1) 收 PSK（首条文本）
        let psk_msg = ws.next().await.unwrap().unwrap();
        assert!(matches!(psk_msg, Message::Text(_)), "首条应为 PSK 文本");

        // 2) 发认证确认 Error{code:0}
        let ack = SignalingMessage::Error { code: 0, message: String::new() };
        ws.send(Message::Text(serde_json::to_string(&ack).unwrap().into()))
            .await
            .unwrap();

        // 3) 收 RoomJoin → 发 RoomJoined
        let join_msg = ws.next().await.unwrap().unwrap();
        let join: SignalingMessage =
            serde_json::from_str(join_msg.to_text().unwrap()).unwrap();
        let room_id = match join {
            SignalingMessage::RoomJoin { room_id, .. } => room_id,
            _ => panic!("expected RoomJoin"),
        };
        let joined = SignalingMessage::RoomJoined { room_id, peer_id: "peer-1".to_string() };
        ws.send(Message::Text(serde_json::to_string(&joined).unwrap().into()))
            .await
            .unwrap();

        // 4) echo loop：收一条回一条
        while let Some(Ok(msg)) = ws.next().await {
            if ws.send(msg).await.is_err() {
                break;
            }
        }
    });

    // 客户端
    let client = SignalClient::new(
        &format!("ws://{addr}/ws"),
        "test-psk",
        "test-room",
        PeerRole::Host,
    );
    let session = client.connect().await.expect("connect");
    assert_eq!(session.room_id(), "test-room");
    assert_eq!(session.peer_id(), "peer-1", "RoomJoined 的 peer_id 应可访问");
    assert_eq!(session.room_id(), "test-room");
    let mut events = session.events();

    // 发一条 Sdp，期待 server echo 回来
    session
        .send(SignalingMessage::Sdp {
            room_id: "test-room".to_string(),
            target: None,
            sdp: "v=0".to_string(),
        })
        .await
        .expect("send");

    // 收事件（先 Connected，后 echo 的 Message）
    let echoed = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            match events.recv().await.unwrap() {
                SignalEvent::Message(SignalingMessage::Sdp { sdp, .. }) => return sdp,
                _ => continue, // Connected 等事件跳过
            }
        }
    })
    .await
    .expect("echo timeout");
    assert_eq!(echoed, "v=0");

    session.close().await.expect("close");
    server.await.unwrap();
}

// ── D2: 本地网关模式（LocalEnvelope 信封 wire，无 PSK 挑战）──────────────

#[tokio::test]
async fn gateway_mode_connects_with_envelope_wire() {
    // mock 网关：首条消息必须是 LocalEnvelope 包 RoomJoin（无 PSK 挑战！），
    // 回复 LocalEnvelope{RoomJoined}，随后信封回显。
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();

        // 1) 首条必须是信封（网关本地 wire 无 PSK 挑战）
        let first = ws.next().await.unwrap().unwrap();
        let env: mediaservo_link::LocalEnvelope =
            serde_json::from_str(first.to_text().unwrap()).expect("首条应为 LocalEnvelope");
        assert_eq!(env.src, "child-1");
        let room_id = match env.msg {
            SignalingMessage::RoomJoin { room_id, .. } => room_id,
            other => panic!("expected RoomJoin in envelope, got {other:?}"),
        };

        // 2) 信封回 RoomJoined
        let joined = mediaservo_link::LocalEnvelope {
            src: "server".into(),
            msg: SignalingMessage::RoomJoined { room_id, peer_id: "veh-peer".into() },
        };
        ws.send(Message::Text(serde_json::to_string(&joined).unwrap().into()))
            .await
            .unwrap();

        // 3) echo loop（信封）
        while let Some(Ok(msg)) = ws.next().await {
            if ws.send(msg).await.is_err() {
                break;
            }
        }
    });

    let client = SignalClient::new_gateway(
        &format!("ws://{addr}/ws"),
        "child-1",
        "stream-s0",
        PeerRole::Host,
    );
    let session = client.connect().await.expect("gateway connect");
    assert_eq!(session.room_id(), "stream-s0");
    assert_eq!(session.peer_id(), "veh-peer", "合成 RoomJoined 的整车 peer_id");
    let mut events = session.events();

    session
        .send(SignalingMessage::Sdp {
            room_id: "stream-s0".to_string(),
            target: None,
            sdp: "v=0".to_string(),
        })
        .await
        .expect("send");
    let echoed = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            match events.recv().await.unwrap() {
                SignalEvent::Message(SignalingMessage::Sdp { sdp, .. }) => return sdp,
                _ => continue,
            }
        }
    })
    .await
    .expect("echo timeout");
    assert_eq!(echoed, "v=0");

    session.close().await.expect("close");
    server.await.unwrap();
}

#[tokio::test]
async fn gateway_mode_room_join_denied_returns_error() {
    // 网关未连上远端 server → RoomJoin 拦截回 Error 5001（信封内）
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
        let _env = ws.next().await.unwrap().unwrap();
        let deny = mediaservo_link::LocalEnvelope {
            src: "server".into(),
            msg: SignalingMessage::Error { code: 5001, message: "gateway not connected to server".into() },
        };
        ws.send(Message::Text(serde_json::to_string(&deny).unwrap().into()))
            .await
            .unwrap();
    });

    let client = SignalClient::new_gateway(
        &format!("ws://{addr}/ws"),
        "child-1",
        "r",
        PeerRole::Host,
    );
    let err = client.connect().await.unwrap_err();
    assert!(err.to_string().contains("5001"), "应报 5001，got: {err}");
    server.await.unwrap();
}

#[tokio::test]
async fn auth_denied_returns_error() {
    // mock server：认证拒绝
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
        let _psk = ws.next().await.unwrap().unwrap();
        let deny = SignalingMessage::Error { code: 4003, message: "PSK authentication failed".to_string() };
        ws.send(Message::Text(serde_json::to_string(&deny).unwrap().into()))
            .await
            .unwrap();
    });

    let client = SignalClient::new(&format!("ws://{addr}/ws"), "wrong-psk", "r", PeerRole::Host);
    let err = client.connect().await.unwrap_err();
    assert!(err.to_string().contains("auth denied [4003]"), "应报认证拒绝，got: {err}");

    server.await.unwrap();
}

// ---- G4: 设备凭证（D-H11）——RoomJoin 携带（additive；缺省 = PSK 路径）----

#[tokio::test]
async fn room_join_carries_device_credentials() {
    // mock server：PSK 确认后，断言 RoomJoin 携带 device_id/device_secret
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
        let _psk = ws.next().await.unwrap().unwrap();
        let ack = SignalingMessage::Error { code: 0, message: String::new() };
        ws.send(Message::Text(serde_json::to_string(&ack).unwrap().into())).await.unwrap();
        let join_msg = ws.next().await.unwrap().unwrap();
        let join: SignalingMessage = serde_json::from_str(join_msg.to_text().unwrap()).unwrap();
        match join {
            SignalingMessage::RoomJoin { device_id, device_secret, .. } => {
                assert_eq!(device_id.as_deref(), Some("ms-001122334455"), "RoomJoin 应携带 device_id");
                assert_eq!(device_secret.as_deref(), Some("s3cr3t"), "RoomJoin 应携带 device_secret");
            }
            other => panic!("expected RoomJoin, got {other:?}"),
        }
        let joined = SignalingMessage::RoomJoined { room_id: "r".into(), peer_id: "peer-1".into() };
        ws.send(Message::Text(serde_json::to_string(&joined).unwrap().into())).await.unwrap();
    });
    let client = SignalClient::new(&format!("ws://{addr}/ws"), "test-psk", "r", PeerRole::Host)
        .with_device_credentials(mediaservo_link::DeviceCredential {
            device_id: "ms-001122334455".into(),
            device_secret: "s3cr3t".into(),
        });
    let session = client.connect().await.expect("connect with device credentials");
    assert_eq!(session.peer_id(), "peer-1");
    session.close().await.expect("close");
    server.await.unwrap();
}

#[tokio::test]
async fn room_join_denied_surfaces_device_auth_error() {
    // 凭证被 server 拒绝（G2 起：Error 4010 设备认证失败）→ 客户端必须明确报错（C15/C16）
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
        let _psk = ws.next().await.unwrap().unwrap();
        let ack = SignalingMessage::Error { code: 0, message: String::new() };
        ws.send(Message::Text(serde_json::to_string(&ack).unwrap().into())).await.unwrap();
        let _join = ws.next().await.unwrap().unwrap();
        let deny = SignalingMessage::Error { code: 4010, message: "device authentication failed".to_string() };
        ws.send(Message::Text(serde_json::to_string(&deny).unwrap().into())).await.unwrap();
    });
    let client = SignalClient::new(&format!("ws://{addr}/ws"), "test-psk", "r", PeerRole::Host)
        .with_device_credentials(mediaservo_link::DeviceCredential {
            device_id: "ms-bad".into(),
            device_secret: "wrong".into(),
        });
    let err = client.connect().await.unwrap_err();
    assert!(err.to_string().contains("4010") && err.to_string().contains("device authentication failed"),
        "应明确报设备认证失败，got: {err}");
    server.await.unwrap();
}

// ---- Phase B (B1): 重连（指数退避 + jitter）与断线通知 ----

/// mock server：前 `refuse` 次连接 TCP 立断（WS 握手失败），之后完整认证+入房；
/// 累计接受 `total` 次连接后退出（防客户端放弃后 accept 永久阻塞）。
async fn refuse_then_serve(refuse: usize, total: usize) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let attempts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let server = tokio::spawn({
        let attempts = attempts.clone();
        async move {
            let mut conn = 0usize;
            while conn < total {
                let (stream, _) = listener.accept().await.unwrap();
                conn += 1;
                attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if conn <= refuse {
                    drop(stream); // 拒绝：TCP 立即关闭 → connect_async 报错
                    continue;
                }
                let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
                let psk_msg = ws.next().await.unwrap().unwrap();
                assert!(matches!(psk_msg, Message::Text(_)));
                let ack = SignalingMessage::Error { code: 0, message: String::new() };
                ws.send(Message::Text(serde_json::to_string(&ack).unwrap().into())).await.unwrap();
                let join_msg = ws.next().await.unwrap().unwrap();
                let join: SignalingMessage = serde_json::from_str(join_msg.to_text().unwrap()).unwrap();
                let room_id = match join {
                    SignalingMessage::RoomJoin { room_id, .. } => room_id,
                    _ => panic!("expected RoomJoin"),
                };
                let joined = SignalingMessage::RoomJoined { room_id, peer_id: "peer-1".to_string() };
                ws.send(Message::Text(serde_json::to_string(&joined).unwrap().into())).await.unwrap();
                return; // 本轮服务完成，剩余连接不再处理
            }
        }
    });
    (addr, server, attempts)
}

#[tokio::test]
async fn connect_with_retry_refuses_then_succeeds() {
    // 先拒 2 次（重试 2 轮），第 3 次连接成功 → 会话可用
    let (addr, server, attempts) = refuse_then_serve(2, 3).await;
    let client = SignalClient::new(&format!("ws://{addr}/ws"), "test-psk", "test-room", PeerRole::Host);
    let session = client
        .connect_with_retry(mediaservo_link::RetryConfig {
            max_retries: 3,
            base_delay: std::time::Duration::from_millis(50),
            max_delay: std::time::Duration::from_secs(1),
        })
        .await
        .expect("retry should succeed");
    assert_eq!(session.room_id(), "test-room");
    assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 3, "应尝试 3 次（2 拒 + 1 收）");
    session.close().await.expect("close");
    server.await.unwrap();
}

#[tokio::test]
async fn connect_with_retry_exhausts_max_retries() {
    // 永远拒绝 → max_retries 次重试后返回错误，且每次等待指数退避
    let (addr, server, attempts) = refuse_then_serve(3, 3).await;
    let client = SignalClient::new(&format!("ws://{addr}/ws"), "test-psk", "test-room", PeerRole::Host);
    let started = std::time::Instant::now();
    let err = client
        .connect_with_retry(mediaservo_link::RetryConfig {
            max_retries: 2,
            base_delay: std::time::Duration::from_millis(50),
            max_delay: std::time::Duration::from_secs(1),
        })
        .await
        .unwrap_err();
    assert!(err.to_string().contains("after 2 retries"), "应报告重试次数，got: {err}");
    assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 3, "应尝试 3 次（1 初 + 2 重试）");
    // ±25% jitter：两次退避 50ms/100ms 的 75% 下限合计 ≥ 100ms
    assert!(started.elapsed() >= std::time::Duration::from_millis(100), "应等待退避，elapsed={:?}", started.elapsed());
    server.await.unwrap();
}

#[tokio::test]
async fn on_disconnect_fires_when_server_closes() {
    // mock server：完整握手后，等客户端 ready 消息再主动 Close
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
        let _psk = ws.next().await.unwrap().unwrap();
        let ack = SignalingMessage::Error { code: 0, message: String::new() };
        ws.send(Message::Text(serde_json::to_string(&ack).unwrap().into())).await.unwrap();
        let _join = ws.next().await.unwrap().unwrap();
        let joined = SignalingMessage::RoomJoined { room_id: "r".to_string(), peer_id: "peer-1".to_string() };
        ws.send(Message::Text(serde_json::to_string(&joined).unwrap().into())).await.unwrap();
        let _ready = ws.next().await.unwrap().unwrap(); // 等客户端就绪
        ws.close(None).await.unwrap();
    });

    let client = SignalClient::new(&format!("ws://{addr}/ws"), "test-psk", "r", PeerRole::Host);
    let session = client.connect().await.expect("connect");
    let (tx, mut rx) = tokio::sync::watch::channel(());
    session.on_disconnect(Box::new(move || {
        let _ = tx.send(());
    }));
    // 通知 server 关闭
    session
        .send(SignalingMessage::Sdp {
            room_id: "r".to_string(),
            target: None,
            sdp: "v=0".to_string(),
        })
        .await
        .expect("send ready");
    tokio::time::timeout(std::time::Duration::from_secs(3), rx.changed())
        .await
        .expect("on_disconnect 应在 server 关闭时触发（3s 超时）")
        .expect("watch channel 不应关闭");
    server.await.unwrap();
}

// ── device-enroll T7: 公钥指纹验签链（challenge→应答→终态）+ 单锚交叉复验 ─────
// 常量与期望值 = server devices.rs::sig_vector（批2 钉死）同一锚点，逐字抄录不重算。

const SIG_VECTOR_VK_B64: &str = "A6EHv/POEL4dcN0Y50vAmWfk1jCbpQ1fHdyGZBJVMbg=";
const SIG_VECTOR_SIG_CAM0_B64: &str =
    "Gnz2kGCFH6igsOfv5QW0+8aRyu/lP5ytAa8fJA0CPYP3fIX5UsYr6uTjFjqOEEFBUB2scnDffIZ1WfP9O2ECCg==";

fn sig_nonce() -> Vec<u8> {
    (0x40u8..0x60).collect()
}

/// sig_vector 身份：seed = bytes(0..=31)，device_id = ms-0a1b2c3d4e5f。
fn ident_seed0() -> DeviceIdentity {
    let seed: [u8; 32] = std::array::from_fn(|i| i as u8);
    DeviceIdentity::new("ms-0a1b2c3d4e5f", ed25519_dalek::SigningKey::from_bytes(&seed))
}

#[test]
fn device_auth_sig_cross_anchor_matches_server_vector() {
    // 单锚交叉复验：本 crate 签名函数 × server 钉期望值 —— 同输入必出同字节。
    let ident = ident_seed0();
    assert_eq!(ident.pubkey_b64, SIG_VECTOR_VK_B64, "公钥指纹必须 = server 钉值");
    assert_eq!(
        ident.sign_device_auth(&sig_nonce(), "vehicle_cam0"),
        SIG_VECTOR_SIG_CAM0_B64,
        "同 seed/nonce/device_id/room 必出同 sig（字节合同 §3 两侧一致）"
    );
}

/// mock server：PSK ack → 断言 Join 为 pubkey 形 → challenge(sig_vector nonce) →
/// 断言应答 verify_strict 过 → 回 `final_msg` 终态。
async fn spawn_pubkey_server(final_msg: SignalingMessage) -> std::net::SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
        let _psk = ws.next().await.unwrap().unwrap();
        let ack = SignalingMessage::Error { code: 0, message: String::new() };
        ws.send(Message::Text(serde_json::to_string(&ack).unwrap()))
            .await
            .unwrap();
        let join_msg = ws.next().await.unwrap().unwrap();
        let room_id =
            match serde_json::from_str::<SignalingMessage>(join_msg.to_text().unwrap()).unwrap() {
                SignalingMessage::RoomJoin { room_id, device_id, device_secret, device_pubkey, .. } => {
                    assert_eq!(device_id.as_deref(), Some("ms-0a1b2c3d4e5f"));
                    assert_eq!(device_secret, None, "pubkey 形 Join 不得携带 secret");
                    assert_eq!(
                        device_pubkey.as_deref(),
                        Some(SIG_VECTOR_VK_B64),
                        "Join 应带 device_pubkey（= sig_vector vk）"
                    );
                    room_id
                }
                other => panic!("expected RoomJoin, got {other:?}"),
            };
        let nonce_raw = sig_nonce();
        let ch = SignalingMessage::DeviceAuthChallenge {
            nonce: base64::engine::general_purpose::STANDARD.encode(&nonce_raw),
        };
        ws.send(Message::Text(serde_json::to_string(&ch).unwrap()))
            .await
            .unwrap();
        // server 同款纪律验签（verify_strict，§3）+ 同锚 sig 比对
        let resp_msg = ws.next().await.unwrap().unwrap();
        match serde_json::from_str::<SignalingMessage>(resp_msg.to_text().unwrap()).unwrap() {
            SignalingMessage::DeviceAuthResponse { room_id: r, sig } => {
                assert_eq!(r, room_id, "应答 room 必须回显 Join 房间");
                assert_eq!(sig, SIG_VECTOR_SIG_CAM0_B64, "nonce/ids/room 同锚必出同 sig");
                let vk_arr: [u8; 32] = base64::engine::general_purpose::STANDARD
                    .decode(SIG_VECTOR_VK_B64)
                    .unwrap()
                    .as_slice()
                    .try_into()
                    .unwrap();
                let sig_arr: [u8; 64] = base64::engine::general_purpose::STANDARD
                    .decode(&sig)
                    .unwrap()
                    .as_slice()
                    .try_into()
                    .unwrap();
                let mut msg = nonce_raw;
                msg.extend_from_slice(b"ms-0a1b2c3d4e5f");
                msg.extend_from_slice(room_id.as_bytes());
                ed25519_dalek::VerifyingKey::from_bytes(&vk_arr)
                    .unwrap()
                    .verify_strict(&msg, &ed25519_dalek::Signature::from_bytes(&sig_arr))
                    .expect("host 应答必须通过验签");
            }
            other => panic!("expected DeviceAuthResponse, got {other:?}"),
        }
        ws.send(Message::Text(serde_json::to_string(&final_msg).unwrap()))
            .await
            .unwrap();
    });
    addr
}

fn pubkey_client(addr: std::net::SocketAddr) -> SignalClient {
    SignalClient::new(&format!("ws://{addr}/ws"), "test-psk", "vehicle_cam0", PeerRole::Host)
        .with_device_identity(ident_seed0())
}

#[tokio::test]
async fn pubkey_join_challenge_answered_receives_joined() {
    // 全链过：Join(pubkey) → challenge → 应答 → RoomJoined → 会话可用
    let addr = spawn_pubkey_server(SignalingMessage::RoomJoined {
        room_id: "vehicle_cam0".into(),
        peer_id: "peer-vk".into(),
    })
    .await;
    let session = pubkey_client(addr).connect().await.expect("pubkey 全链应过");
    assert_eq!(session.peer_id(), "peer-vk");
    session.close().await.expect("close");
}

#[tokio::test]
async fn pubkey_join_pending_maps_to_enroll_pending_error() {
    // 手动档未批准：DeviceAuthPending → typed EnrollPending（host 日志面可判别）
    let addr = spawn_pubkey_server(SignalingMessage::DeviceAuthPending {
        device_id: "ms-0a1b2c3d4e5f".into(),
    })
    .await;
    let err = pubkey_client(addr).connect().await.expect_err("pending 必须报 typed 错误");
    assert!(
        matches!(&err, LinkError::EnrollPending { device_id } if device_id == "ms-0a1b2c3d4e5f"),
        "应为 EnrollPending{{device_id}}，got: {err:?}"
    );
    assert!(err.to_string().contains("device enroll pending"), "Display 含家族串: {err}");
}

#[tokio::test]
async fn pubkey_join_rejected_after_answer_surfaces_error() {
    // 验签被拒（4010 统一防枚举消息）→ 错误上抛，会话不建
    let addr = spawn_pubkey_server(SignalingMessage::Error {
        code: 4010,
        message: "device authentication failed: invalid device credentials".into(),
    })
    .await;
    let err = pubkey_client(addr).connect().await.expect_err("拒签必须报错");
    assert!(
        err.to_string().contains("4010") && err.to_string().contains("device authentication failed"),
        "got: {err}"
    );
}

#[tokio::test]
async fn pubkey_join_silence_hits_five_second_disconnect() {
    // §1 合同：pubkey 形 Join 后服务端静默 → 5s 超时断连错误（start_paused 瞬时推进）
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
        let _psk = ws.next().await.unwrap().unwrap();
        let ack = SignalingMessage::Error { code: 0, message: String::new() };
        ws.send(Message::Text(serde_json::to_string(&ack).unwrap()))
            .await
            .unwrap();
        let _join = ws.next().await; // 此后静默
        tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
    });
    let err = pubkey_client(addr).connect().await.expect_err("静默必须超时");
    assert!(err.to_string().contains("no server response within 5s"), "got: {err}");
    server.abort();
}
