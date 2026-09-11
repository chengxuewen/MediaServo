//! D1 host-agent 信令网关测试（本地 mock WS server，无 Docker 依赖）。
//!
//! 覆盖契约（Momus HIGH-1 协议语义）:
//! ① 多本地客户端 → 单远端 WS，双向转发 + 房间重写（整车房间 ↔ 子进程房间）
//! ② RoomJoin 拦截：子进程 RoomJoin 不上行（agent 单次 join）；子进程本地
//!    合成 RoomJoined（携带整车 peer_id）
//! ③ 并发协商：两路 CreateWebRtcTransport 在途 → 响应按 FIFO 路由不串
//!    （NewProducer 广播夹在响应之间不消耗 FIFO）
//! ④ 断线重连：远端 WS 断开 → B1 重连 → 转发恢复；断线在途请求清空
//! ⑤ P2P Sdp/ICE 单协商路由：回显去重 + 协商归属切换

use std::net::SocketAddr;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use mediaservo_common::protocol::{
    DtlsParameters, Fingerprint, IceParameters, MediaKind, PeerRole, SignalingMessage,
    TransportDirection,
};
use mediaservo_host::gateway::{run_gateway, GatewayConfig, LocalEnvelope};
use mediaservo_link::{DeviceIdentity, RetryConfig};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

/// mock server 侧（accept_async 原始 TCP 流）。
type WsServer = WebSocketStream<TcpStream>;
/// 本地子进程侧（connect_async 可能 TLS 流）。
type WsClient = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// 任一 WS 流（client/server 共用 helper 的泛型约束）。
trait WsIo: AsyncRead + AsyncWrite + Unpin + Send {}
impl WsIo for TcpStream {}
impl WsIo for MaybeTlsStream<TcpStream> {}

type Ws = WebSocketStream<TcpStream>;

const VEHICLE_ROOM: &str = "vehicle-1";
const VEHICLE_PEER: &str = "veh-peer";

fn cfg(remote: SocketAddr) -> GatewayConfig {
    GatewayConfig {
        local_port: 0, // 临时端口
        remote_url: format!("ws://{remote}/ws"),
        psk: "test-psk".into(),
        device: None,
        room: VEHICLE_ROOM.into(),
        retry: RetryConfig {
            max_retries: 3,
            base_delay: Duration::from_millis(50),
            max_delay: Duration::from_millis(500),
        },
    }
}

fn env(src: &str, msg: SignalingMessage) -> String {
    serde_json::to_string(&LocalEnvelope { src: src.into(), msg }).unwrap()
}

/// mock server 完整握手：PSK → Error{0} 确认 → RoomJoin → RoomJoined。
/// 返回 (ws, 请求的房间, 角色)。
async fn mock_handshake(listener: &TcpListener) -> (WsServer, String, PeerRole) {
    let (stream, _) = listener.accept().await.expect("mock accept");
    let mut ws = tokio_tungstenite::accept_async(stream).await.expect("mock ws handshake");
    let psk = ws.next().await.unwrap().unwrap();
    assert!(matches!(psk, Message::Text(_)), "首条应为 PSK 文本");
    ws.send(Message::Text(
        serde_json::to_string(&SignalingMessage::Error { code: 0, message: String::new() })
            .unwrap()
            .into(),
    ))
    .await
    .unwrap();
    let join = ws.next().await.unwrap().unwrap();
    let (room, role) = match serde_json::from_str::<SignalingMessage>(join.to_text().unwrap())
        .expect("mock 解析 RoomJoin")
    {
        SignalingMessage::RoomJoin { room_id, peer_role, .. } => (room_id, peer_role),
        other => panic!("expected RoomJoin, got {other:?}"),
    };
    ws.send(Message::Text(
        serde_json::to_string(&SignalingMessage::RoomJoined {
            room_id: room.clone(),
            peer_id: VEHICLE_PEER.into(),
        })
        .unwrap()
        .into(),
    ))
    .await
    .unwrap();
    (ws, room, role)
}

async fn local(port: u16) -> WsClient {
    let (ws, _) = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/ws"))
        .await
        .expect("local client connect");
    ws
}

async fn read_env<S: WsIo>(ws: &mut WebSocketStream<S>) -> (String, SignalingMessage) {
    let msg = tokio::time::timeout(Duration::from_secs(5), ws.next())
        .await
        .expect("读超时")
        .expect("ws 关闭")
        .expect("ws 错误");
    let text = msg.to_text().expect("应为文本").to_string();
    let env: LocalEnvelope = serde_json::from_str(&text).expect("信封解析");
    (env.src, env.msg)
}

/// 子进程 RoomJoin（拦截语义）：网关未就绪时 Error 5001 → 重试。
/// 返回合成 RoomJoined 的 peer_id。
async fn join<S: WsIo>(ws: &mut WebSocketStream<S>, room: &str) -> String {
    for _ in 0..100 {
        ws.send(Message::Text(env(
            "child",
            SignalingMessage::RoomJoin {
                device_id: None,
                device_secret: None,
                device_pubkey: None,
                room_id: room.into(),
                peer_role: PeerRole::Host,
                stream_id: None,
            },
        )))
        .await
        .unwrap();
        let (_src, msg) = read_env(ws).await;
        match msg {
            SignalingMessage::RoomJoined { room_id, peer_id } => {
                assert_eq!(room_id, room, "合成 RoomJoined 应回显子进程房间");
                return peer_id;
            }
            SignalingMessage::Error { code: 5001, .. } => {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            other => panic!("join 意外应答: {other:?}"),
        }
    }
    panic!("join 重试耗尽");
}

/// 读 2 条消息：transport 响应 + NewProducer 广播（任意顺序）。
/// 返回 (transport_id, 响应 peer_id)——mock 回显请求标识（peer_id 作标记），
/// 串线即可检测（A 收到 req-b 的响应 = 错配）。
async fn collect_transport(ws: &mut WsClient, room: &str) -> (String, String) {
    let mut transport: Option<String> = None;
    let mut peer: Option<String> = None;
    for _ in 0..2 {
        let (_, m) = read_env(ws).await;
        match m {
            SignalingMessage::WebRtcTransportCreated { transport_id, room_id, peer_id, .. } => {
                assert_eq!(room_id, room, "transport 响应房间应改写为 {room}");
                transport = Some(transport_id);
                peer = Some(peer_id);
            }
            SignalingMessage::NewProducer { room_id, .. } => {
                assert_eq!(room_id, room, "NewProducer 广播房间应改写为 {room}");
            }
            other => panic!("{room} 意外消息: {other:?}"),
        }
    }
    (transport.expect("应收到 transport 响应"), peer.expect("响应应携带身份回显"))
}

fn transport_created_for(transport_id: &str, peer_id: &str) -> SignalingMessage {
    SignalingMessage::WebRtcTransportCreated {
        room_id: VEHICLE_ROOM.into(),
        peer_id: peer_id.into(),
        transport_id: transport_id.into(),
        ice_parameters: IceParameters {
            username_fragment: "ufrag".into(),
            password: "pwd".into(),
        },
        dtls_parameters: DtlsParameters {
            fingerprints: vec![Fingerprint {
                algorithm: "sha-256".into(),
                value: "AA:BB:CC".into(),
            }],
            role: "auto".into(),
        },
        ice_candidates: None,
    }
}

/// mock 响应：transport 创建确认（peer_id 身份回显）。
fn transport_created(transport_id: &str) -> SignalingMessage {
    transport_created_for(transport_id, VEHICLE_PEER)
}

/// CreateWebRtcTransport 请求（peer_id 作请求标识；room 由网关重写）。
fn create(room: &str, peer_id: &str) -> SignalingMessage {
    SignalingMessage::CreateWebRtcTransport {
        room_id: room.into(),
        peer_id: peer_id.into(),
        direction: TransportDirection::Send,
    }
}

// ── ① 多本地客户端转发正确（双向房间重写 + src=server 下发）────────────────

#[tokio::test]
async fn multiple_clients_forward_with_room_rewrite() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut ws, room, role) = mock_handshake(&listener).await;
        assert_eq!(room, VEHICLE_ROOM, "agent 应以整车身份单次 join");
        assert_eq!(role, PeerRole::Host);

        // 两条上行（来自不同子进程）：房间均重写为整车房间
        let m1: SignalingMessage =
            serde_json::from_str(ws.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
        assert!(
            matches!(&m1, SignalingMessage::Frame { room_id, .. } if room_id == VEHICLE_ROOM),
            "Frame 房间应重写为整车房间, got {m1:?}"
        );
        let m2: SignalingMessage =
            serde_json::from_str(ws.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
        assert!(
            matches!(&m2, SignalingMessage::EncoderStatus { room_id, .. } if room_id == VEHICLE_ROOM),
            "EncoderStatus 房间应重写为整车房间, got {m2:?}"
        );

        // 下行房间级广播：每个子进程收到自己房间的改写
        ws.send(Message::Text(
            serde_json::to_string(&SignalingMessage::NewProducer {
                room_id: VEHICLE_ROOM.into(),
                producer_id: "p1".into(),
                peer_id: VEHICLE_PEER.into(),
                kind: MediaKind::Video,
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
        tokio::time::sleep(Duration::from_secs(1)).await; // 保持连接至断言完成
    });

    let (port, _handle) = run_gateway(cfg(addr)).await.unwrap();
    let mut a = local(port).await;
    let mut b = local(port).await;
    join(&mut a, "room-a").await;
    join(&mut b, "room-b").await;

    a.send(Message::Text(env(
        "a",
        SignalingMessage::Frame {
            room_id: "room-a".into(),
            codec: "vp8".into(),
            sequence: 1,
            is_keyframe: true,
            data_base64: "AA==".into(),
        },
    )))
    .await
    .unwrap();
    b.send(Message::Text(env(
        "b",
        SignalingMessage::EncoderStatus {
            room_id: "room-b".into(),
            peer_id: "host".into(),
            codec: "vp8".into(),
            encoder_backend: "software".into(),
            encoder_implementation: None,
            frames_per_second: 30.0,
            frame_width: 640,
            frame_height: 360,
            avg_encode_ms: None,
        },
    )))
    .await
    .unwrap();

    // 下行广播：各子进程收到自己的房间改写 + src=server
    let (src, m) = read_env(&mut a).await;
    assert_eq!(src, "server");
    assert!(
        matches!(&m, SignalingMessage::NewProducer { room_id, .. } if room_id == "room-a"),
        "子进程 a 应收到房间 room-a 的改写, got {m:?}"
    );
    let (src, m) = read_env(&mut b).await;
    assert_eq!(src, "server");
    assert!(
        matches!(&m, SignalingMessage::NewProducer { room_id, .. } if room_id == "room-b"),
        "子进程 b 应收到房间 room-b 的改写, got {m:?}"
    );
    server.await.unwrap();
}

// ── ② RoomJoin 拦截：不上行 + 本地合成 RoomJoined ──────────────────────────

#[tokio::test]
async fn room_join_intercepted_not_forwarded() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut ws, room, role) = mock_handshake(&listener).await;
        assert_eq!(room, VEHICLE_ROOM);
        assert_eq!(role, PeerRole::Host);
        // 子进程 RoomJoin 绝不上行：2s 内不应收到任何消息（更不是 RoomJoin）
        let leaked = tokio::time::timeout(Duration::from_secs(2), ws.next()).await;
        assert!(
            leaked.is_err(),
            "子进程 RoomJoin 泄漏到远端: {:?}",
            leaked.ok().flatten().map(|m| format!("{m:?}"))
        );
    });

    let (port, _handle) = run_gateway(cfg(addr)).await.unwrap();
    let mut a = local(port).await;
    let peer_id = join(&mut a, "child-room").await;
    assert_eq!(peer_id, VEHICLE_PEER, "合成 RoomJoined 应携带整车 peer_id");
    server.await.unwrap();
}

// ── ③ 并发协商：两路 Create 在途 → FIFO 响应路由不串 ───────────────────────

#[tokio::test]
async fn concurrent_sfu_negotiation_routes_responses() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut ws, _room, _role) = mock_handshake(&listener).await;

        // 两路 Create（顺序 = 响应顺序）：提取请求标识（peer_id 作标记）
        let mut markers = Vec::new();
        for _ in 0..2 {
            let c: SignalingMessage =
                serde_json::from_str(ws.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
            match &c {
                SignalingMessage::CreateWebRtcTransport { peer_id, room_id, direction, .. } => {
                    assert_eq!(room_id, VEHICLE_ROOM, "Create 房间应重写为整车房间");
                    assert_eq!(direction, &TransportDirection::Send);
                    markers.push(peer_id.clone());
                }
                other => panic!("期望 CreateWebRtcTransport, got {other:?}"),
            }
        }
        // 响应序列：t1(markers[0]) → NewProducer 广播（夹在响应之间）→ t2(markers[1])
        ws.send(Message::Text(serde_json::to_string(&transport_created_for("t1", &markers[0])).unwrap().into()))
            .await
            .unwrap();
        ws.send(Message::Text(
            serde_json::to_string(&SignalingMessage::NewProducer {
                room_id: VEHICLE_ROOM.into(),
                producer_id: "p9".into(),
                peer_id: VEHICLE_PEER.into(),
                kind: MediaKind::Video,
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
        ws.send(Message::Text(serde_json::to_string(&transport_created_for("t2", &markers[1])).unwrap().into()))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_secs(1)).await;
    });

    let (port, _handle) = run_gateway(cfg(addr)).await.unwrap();
    let mut a = local(port).await;
    let mut b = local(port).await;
    join(&mut a, "room-a").await;
    join(&mut b, "room-b").await;

    // 两路同时发送 Create（在途；peer_id 携带各自请求标识）
    a.send(Message::Text(env("a", create("room-a", "req-a")))).await.unwrap();
    b.send(Message::Text(env("b", create("room-b", "req-b")))).await.unwrap();

    // 各读 2 条：自己的 transport 响应 + NewProducer 广播（顺序不定——server 的
    // 广播通道与直连响应并发，广播可能先于第二个响应到达）
    let (ta, pa) = collect_transport(&mut a, "room-a").await;
    let (tb, pb) = collect_transport(&mut b, "room-b").await;
    assert_eq!(pa, "req-a", "A 必须收到自己请求的响应（身份回显）");
    assert_eq!(pb, "req-b", "B 必须收到自己请求的响应（身份回显）");
    assert_ne!(ta, tb, "两路 transport_id 不得串线");
    assert!(ta == "t1" || ta == "t2");
    assert!(tb == "t1" || tb == "t2");
    server.await.unwrap();
}

// ── ④ 断线重连：转发恢复 + 在途请求清空 ────────────────────────────────────

#[tokio::test]
async fn reconnect_resumes_forwarding() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        // 连接 1：完整握手 → 收一条消息 → 关闭（模拟远端断线）
        let (mut ws1, _r, _role) = mock_handshake(&listener).await;
        let m1 = ws1.next().await.unwrap().unwrap();
        assert!(
            m1.to_text().unwrap().contains("\"type\":\"sdp\""),
            "连接 1 应收到子进程 Sdp"
        );
        drop(ws1);

        // 连接 2：握手 → 收 Create → 响应
        let (mut ws2, _r2, _role2) = mock_handshake(&listener).await;
        let c = ws2.next().await.unwrap().unwrap();
        let c: SignalingMessage = serde_json::from_str(c.to_text().unwrap()).unwrap();
        assert!(
            matches!(&c, SignalingMessage::CreateWebRtcTransport { room_id, .. }
                if room_id == VEHICLE_ROOM),
            "重连后 Create 房间应为整车房间, got {c:?}"
        );
        ws2.send(Message::Text(serde_json::to_string(&transport_created("t-after")).unwrap().into()))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_secs(1)).await;
    });

    let (port, _handle) = run_gateway(cfg(addr)).await.unwrap();
    let mut a = local(port).await;
    join(&mut a, "room-a").await;

    // 断线前：Sdp 转发（mock 连接 1 收后关闭）
    a.send(Message::Text(env(
        "a",
        SignalingMessage::Sdp {
            room_id: "room-a".into(),
            target: None,
            sdp: "v=0 offer".into(),
        },
    )))
    .await
    .unwrap();

    // 重连后：Create 在途（5001 或断线窗口无应答 → 重试）→ 响应路由回本连接
    let response = loop {
        a.send(Message::Text(env(
            "a",
            SignalingMessage::CreateWebRtcTransport {
                room_id: "room-a".into(),
                peer_id: "host".into(),
                direction: TransportDirection::Send,
            },
        )))
        .await
        .unwrap();
        match tokio::time::timeout(Duration::from_millis(300), read_env(&mut a)).await {
            Ok((_, SignalingMessage::Error { code: 5001, .. })) => {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            Ok((_, m)) => break m,
            Err(_) => {} // 断线窗口：请求无应答，重发
        }
    };
    assert!(
        matches!(&response, SignalingMessage::WebRtcTransportCreated { transport_id, room_id, .. }
            if transport_id == "t-after" && room_id == "room-a"),
        "重连后响应应路由到本连接且房间改写, got {response:?}"
    );
    server.await.unwrap();
}

// ── ⑤ P2P Sdp/ICE 单协商路由：回显去重 + 归属切换 ──────────────────────────

#[tokio::test]
async fn p2p_sdp_ice_single_negotiation_routing() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut ws, _r, _role) = mock_handshake(&listener).await;

        // 收 A 的 offer → 原样回显（模拟 server 房间广播）+ 远端应答
        let offer = ws.next().await.unwrap().unwrap();
        assert!(offer.to_text().unwrap().contains("\"type\":\"sdp\""));
        ws.send(offer).await.unwrap();
        ws.send(Message::Text(
            serde_json::to_string(&SignalingMessage::Sdp {
                room_id: VEHICLE_ROOM.into(),
                target: Some("remote-peer".into()),
                sdp: "b-answer".into(),
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();

        // 收 B 的 ICE → 原样回显 + 远端候选
        let ice = ws.next().await.unwrap().unwrap();
        assert!(ice.to_text().unwrap().contains("\"type\":\"r_t_c_ice_candidate\""));
        ws.send(ice).await.unwrap();
        ws.send(Message::Text(
            serde_json::to_string(&SignalingMessage::RTCIceCandidate {
                room_id: VEHICLE_ROOM.into(),
                target: None,
                candidate: "peer-cand".into(),
                sdp_mid: Some("0".into()),
                sdp_mline_index: Some(0),
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
        tokio::time::sleep(Duration::from_secs(1)).await;
    });

    let (port, _handle) = run_gateway(cfg(addr)).await.unwrap();
    let mut a = local(port).await;
    let mut b = local(port).await;
    join(&mut a, "room-a").await;
    join(&mut b, "room-b").await;

    // A 发 offer（协商归属 = A）
    a.send(Message::Text(env(
        "a",
        SignalingMessage::Sdp {
            room_id: "room-a".into(),
            target: None,
            sdp: "a-offer".into(),
        },
    )))
    .await
    .unwrap();
    // A 收到的第一条必须是远端应答（自己的回显已被去重）
    let (_, m) = read_env(&mut a).await;
    assert!(
        matches!(&m, SignalingMessage::Sdp { sdp, room_id, .. }
            if sdp == "b-answer" && room_id == "room-a"),
        "A 应收到远端应答（非自身回显），房间改写, got {m:?}"
    );

    // B 发 ICE（归属切到 B）→ 收到远端候选（非自身回显）
    b.send(Message::Text(env(
        "b",
        SignalingMessage::RTCIceCandidate {
            room_id: "room-b".into(),
            target: None,
            candidate: "b-cand".into(),
            sdp_mid: Some("0".into()),
            sdp_mline_index: Some(0),
        },
    )))
    .await
    .unwrap();
    let (_, m) = read_env(&mut b).await;
    assert!(
        matches!(&m, SignalingMessage::RTCIceCandidate { candidate, room_id, .. }
            if candidate == "peer-cand" && room_id == "room-b"),
        "B 应收到远端候选（非自身回显），房间改写, got {m:?}"
    );
    server.await.unwrap();
}

// ── CRITICAL-1 回归：断线窗口的 5001 不得留下陈旧 pending 槽 ────────────────

#[tokio::test]
async fn disconnect_window_stale_pending_not_cross_routed() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    // 闸门：mock2 在测试放行前不 accept → agent 阻塞在 WS 握手，joined 保持
    // false（确定性断线窗口——connect 在途且无超时）
    let (gate_tx, gate_rx) = tokio::sync::watch::channel(false);
    let server = tokio::spawn(async move {
        // 连接 1：收 A 的在途请求 → 无响应关闭
        let (mut ws1, _r, _role) = mock_handshake(&listener).await;
        let m1 = ws1.next().await.unwrap().unwrap();
        let m1: SignalingMessage = serde_json::from_str(m1.to_text().unwrap()).unwrap();
        assert!(
            matches!(&m1, SignalingMessage::CreateWebRtcTransport { peer_id, .. } if peer_id == "req-a1"),
            "mock1 应收到 A 的在途请求, got {m1:?}"
        );
        drop(ws1);
        // 等测试放行 mock2
        let mut g = gate_rx.clone();
        while !*g.borrow() {
            g.changed().await.unwrap();
        }
        // 连接 2：收 B 的请求 → 响应（身份回显）
        let (mut ws2, _r2, _role2) = mock_handshake(&listener).await;
        let c2 = ws2.next().await.unwrap().unwrap();
        let c2: SignalingMessage = serde_json::from_str(c2.to_text().unwrap()).unwrap();
        assert!(
            matches!(&c2, SignalingMessage::CreateWebRtcTransport { peer_id, .. } if peer_id == "req-b"),
            "mock2 应收到 B 的请求, got {c2:?}"
        );
        ws2.send(Message::Text(
            serde_json::to_string(&transport_created_for("t-b", "req-b")).unwrap().into(),
        ))
        .await
        .unwrap();
        tokio::time::sleep(Duration::from_secs(1)).await;
    });

    let (port, _handle) = run_gateway(cfg(addr)).await.unwrap();
    let mut a = local(port).await;
    let mut b = local(port).await;
    join(&mut a, "room-a").await;
    join(&mut b, "room-b").await;

    // A 在断线前发出在途请求（mock1 收后无响应关闭 → 断线窗口开始）
    a.send(Message::Text(env("a", create("room-a", "req-a1")))).await.unwrap();

    // 窗口期：A 反复发 Create 直到 5001（修复后：未 join 不入 pending）
    let mut saw_5001 = false;
    for _ in 0..50 {
        a.send(Message::Text(env("a", create("room-a", "req-a2")))).await.unwrap();
        match tokio::time::timeout(Duration::from_millis(300), read_env(&mut a)).await {
            Ok((_, SignalingMessage::Error { code: 5001, .. })) => {
                saw_5001 = true;
                break;
            }
            Ok((_, m)) => panic!("断线窗口不应有响应: {m:?}"),
            Err(_) => {} // 转发已死连接：无应答，重试
        }
    }
    assert!(saw_5001, "A 应命中断线窗口（5001）");

    // 放行 mock2 → agent 重连完成
    gate_tx.send(true).unwrap();

    // B 发请求（5001/无应答重试，有界防挂起）→ 必须收到自己的响应
    let resp = 'b_loop: {
        for _ in 0..50 {
            b.send(Message::Text(env("b", create("room-b", "req-b")))).await.unwrap();
            match tokio::time::timeout(Duration::from_millis(300), read_env(&mut b)).await {
                Ok((_, SignalingMessage::Error { code: 5001, .. })) => {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                Ok((_, m)) => break 'b_loop m,
                Err(_) => {}
            }
        }
        panic!("B 重试耗尽未收到响应（陈旧 pending 串线）");
    };
    assert!(
        matches!(&resp, SignalingMessage::WebRtcTransportCreated { peer_id, transport_id, .. }
            if peer_id == "req-b" && transport_id == "t-b"),
        "B 应收到自己请求的响应, got {resp:?}"
    );

    // A 不得收到任何响应（修复前：陈旧槽弹出 → A 收到 B 的响应 = 串线）
    let extra = tokio::time::timeout(Duration::from_millis(300), read_env(&mut a)).await;
    assert!(
        extra.is_err(),
        "A 不应收到响应（陈旧 pending 串线）, got {:?}",
        extra.ok().map(|(_, m)| m)
    );
    server.await.unwrap();
}

// ── IMPORTANT-3 回归：relay 的 Frame 不得抢占 P2P 协商归属 ──────────────────

#[tokio::test]
async fn frame_relay_does_not_steal_p2p_ownership() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut ws, _r, _role) = mock_handshake(&listener).await;
        // 收 offer（A）+ Frame（B），任意顺序；随后回显两者 + 发远端应答
        let mut offer: Option<Message> = None;
        let mut frame: Option<Message> = None;
        for _ in 0..2 {
            let m = ws.next().await.unwrap().unwrap();
            let t = m.to_text().unwrap().to_string();
            if t.contains("\"type\":\"sdp\"") {
                offer = Some(m);
            } else if t.contains("\"type\":\"frame\"") {
                frame = Some(m);
            } else {
                panic!("mock 意外消息: {t}");
            }
        }
        ws.send(offer.unwrap()).await.unwrap();
        ws.send(frame.unwrap()).await.unwrap();
        ws.send(Message::Text(
            serde_json::to_string(&SignalingMessage::Sdp {
                room_id: VEHICLE_ROOM.into(),
                target: Some("remote-peer".into()),
                sdp: "b-answer".into(),
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
        tokio::time::sleep(Duration::from_secs(1)).await;
    });

    let (port, _handle) = run_gateway(cfg(addr)).await.unwrap();
    let mut a = local(port).await;
    let mut b = local(port).await;
    join(&mut a, "room-a").await;
    join(&mut b, "room-b").await;

    // A 发 offer（协商归属 = A）
    a.send(Message::Text(env(
        "a",
        SignalingMessage::Sdp {
            room_id: "room-a".into(),
            target: None,
            sdp: "a-offer".into(),
        },
    )))
    .await
    .unwrap();
    // B 发 Frame（relay 消息；修复前 is_relay_msg 会抢走归属）
    b.send(Message::Text(env(
        "b",
        SignalingMessage::Frame {
            room_id: "room-b".into(),
            codec: "vp8".into(),
            sequence: 1,
            is_keyframe: true,
            data_base64: "AA==".into(),
        },
    )))
    .await
    .unwrap();

    // A 收到远端应答（自身 offer 回显去重；归属未被 Frame 抢占）
    let (_, m) = read_env(&mut a).await;
    assert!(
        matches!(&m, SignalingMessage::Sdp { sdp, room_id, .. }
            if sdp == "b-answer" && room_id == "room-a"),
        "A 应收到远端应答（归属未被 Frame 抢占）, got {m:?}"
    );
    // B 不得收到任何消息（Frame 回显去重；应答不属于 B）
    let extra = tokio::time::timeout(Duration::from_millis(300), read_env(&mut b)).await;
    assert!(
        extra.is_err(),
        "B 不应收到消息, got {:?}",
        extra.ok().map(|(_, m)| m)
    );
    server.await.unwrap();
}

// ── device-enroll: 远端 Join 公钥指纹全链（challenge→应答→joined）────────────

#[tokio::test]
async fn remote_join_carries_device_credentials() {
    use base64::Engine as _;
    use ed25519_dalek::VerifyingKey;
    // 网关侧身份 = sig_vector 同锚 seed（批3 交叉复验同一常量系）
    let seed: [u8; 32] = std::array::from_fn(|i| i as u8);
    let ident = DeviceIdentity::new("ms-gw-device", ed25519_dalek::SigningKey::from_bytes(&seed));
    let expect_pk = ident.pubkey_b64.clone();
    // mock 远端 server：PSK 确认后断言 RoomJoin 带 device_pubkey（pubkey 形，无 secret），
    // 发 challenge → 断言应答验签过（server 同款 verify_strict）→ RoomJoined。
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("mock accept");
        let mut ws = tokio_tungstenite::accept_async(stream).await.expect("mock ws handshake");
        let psk = ws.next().await.unwrap().unwrap();
        assert!(matches!(psk, Message::Text(_)), "首条应为 PSK 文本");
        ws.send(Message::Text(
            serde_json::to_string(&SignalingMessage::Error { code: 0, message: String::new() })
                .unwrap(),
        ))
        .await
        .unwrap();
        let join = ws.next().await.unwrap().unwrap();
        let nonce_raw: Vec<u8> = (0x40u8..0x60).collect();
        match serde_json::from_str::<SignalingMessage>(join.to_text().unwrap()).unwrap() {
            SignalingMessage::RoomJoin { device_id, device_secret, device_pubkey, .. } => {
                assert_eq!(device_id.as_deref(), Some("ms-gw-device"), "远端 RoomJoin 应携带 device_id");
                assert_eq!(device_secret, None, "pubkey 形不携带 secret（D-E3 两形互斥）");
                assert_eq!(device_pubkey.as_deref(), Some(expect_pk.as_str()), "应携带装配出的公钥指纹");
            }
            other => panic!("expected RoomJoin, got {other:?}"),
        }
        ws.send(Message::Text(
            serde_json::to_string(&SignalingMessage::DeviceAuthChallenge {
                nonce: base64::engine::general_purpose::STANDARD.encode(&nonce_raw),
            })
            .unwrap(),
        ))
        .await
        .unwrap();
        let resp = ws.next().await.unwrap().unwrap();
        match serde_json::from_str::<SignalingMessage>(resp.to_text().unwrap()).unwrap() {
            SignalingMessage::DeviceAuthResponse { room_id, sig } => {
                assert_eq!(room_id, VEHICLE_ROOM, "应答 room 回显整车房间");
                let vk_arr: [u8; 32] = base64::engine::general_purpose::STANDARD
                    .decode(&expect_pk).unwrap().as_slice().try_into().unwrap();
                let sig_arr: [u8; 64] = base64::engine::general_purpose::STANDARD
                    .decode(&sig).unwrap().as_slice().try_into().unwrap();
                let mut msg = nonce_raw;
                msg.extend_from_slice(b"ms-gw-device");
                msg.extend_from_slice(VEHICLE_ROOM.as_bytes());
                VerifyingKey::from_bytes(&vk_arr)
                    .unwrap()
                    .verify_strict(&msg, &ed25519_dalek::Signature::from_bytes(&sig_arr))
                    .expect("网关应答必须通过验签");
            }
            other => panic!("expected DeviceAuthResponse, got {other:?}"),
        }
        ws.send(Message::Text(
            serde_json::to_string(&SignalingMessage::RoomJoined {
                room_id: VEHICLE_ROOM.into(),
                peer_id: VEHICLE_PEER.into(),
            })
            .unwrap(),
        ))
        .await
        .unwrap();
        tokio::time::sleep(Duration::from_millis(500)).await; // 保持连接至断言完成
    });

    // 网关携带设备身份（host-agent 从 identity.json + signing.pem 装配——device-enroll T6/T7）
    let mut gateway_cfg = cfg(addr);
    gateway_cfg.device = Some(ident);
    let (port, _handle) = run_gateway(gateway_cfg).await.expect("run_gateway");
    // 本地子进程 join：合成 RoomJoined 证明远端会话已就绪（连接全链路）
    let mut child = local(port).await;
    let peer = join(&mut child, "child-room").await;
    assert_eq!(peer, VEHICLE_PEER);
    server.await.unwrap();
}
