//! S1: host-controller SFU-DC 信令面 e2e —— mock 本地网关 WS（夹具复用
//! gateway_e2e.rs 的 accept_async + LocalEnvelope 形，无外部 server / 无 mediasoup）。
//!
//! 拓扑：controller 走生产路径（SignalClient::new_gateway → control_loop），其
//! "网关"由测试内 mock WS 扮演：断言上行请求形状、按 mediasoup 语义回响应。
//!
//! 断言（信令层 = 无 worker 环境下的可达最高层，实盘 DTLS/SCTP 归活体一圈）：
//! ① RoomJoin/RoomJoined 信封握手（D2 网关形）
//! ② CreateWebRtcTransport(Send) → ConnectWebRtcTransport（DTLS 指纹 role=client）
//!    → CreateDataProducer ×{chassis,gimbal,light,ack}：protocol=sctp、
//!    transport_id 显式绑定、gimbal ordered=false/max_retransmits=5（D-H3 与
//!    channel_init 同源）、stream_id 唯一（= libwebrtc 实配 DC id）
//! ③ CreateWebRtcTransport(Recv) → Connect → 注入 NewDataProducer →
//!    ConsumeData{dp, transport_id=recv}；自建 producer 的广播回声不得触发消费
//! ④ DataConsumed 后主循环存活（不因未知消息崩环）；mock 关链 → 自愈退出码 1
//!
//! 前置：mediaservo-webrtc backend-webrtc-sys（真 libwebrtc 本地协商，无网络 IO）。

#![cfg(target_os = "linux")]

use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use mediaservo_common::protocol::{
    DtlsParameters, Fingerprint, PeerRole, SctpStreamParameters, SignalingMessage,
    TransportDirection,
};
use mediaservo_host::control::StubActuator;
use mediaservo_host::controller::{ControllerConfig, control_loop};
use mediaservo_link::{LocalEnvelope, SignalClient};
use tokio::net::TcpListener;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;

const ROOM: &str = "control";
/// libwebrtc 对畸形 fingerprint 直接拒绝 setRemoteDescription——给足 32 字节
/// 冒号十六进制（值本身无意义，DTLS 永远不会与 mock 完成握手）。
const FAKE_FP: &str = "01:02:03:04:05:06:07:08:09:0A:0B:0C:0D:0E:0F:10:11:12:13:14:15:16:17:18:19:1A:1B:1C:1D:1E:1F:20";

fn env_text(msg: SignalingMessage) -> Message {
    let text = serde_json::to_string(&LocalEnvelope {
        src: "server".into(),
        msg,
    })
    .expect("信封序列化");
    // (useless_conversion 钉：tungstenite From<String> for Message)
    text.into()
}

fn transport_created(transport_id: &str, ufrag: &str) -> SignalingMessage {
    SignalingMessage::WebRtcTransportCreated {
        room_id: ROOM.into(),
        peer_id: "veh-peer".into(),
        transport_id: transport_id.into(),
        sctp_parameters: None,
        ice_parameters: mediaservo_common::protocol::IceParameters {
            username_fragment: ufrag.into(),
            password: "pwdpwdpwdpwdpwdpwdpwdpwdpwdpwdpwd".into(), // libwebrtc 下限 22 字符
        },
        dtls_parameters: DtlsParameters {
            fingerprints: vec![Fingerprint {
                algorithm: "sha-256".into(),
                value: FAKE_FP.into(),
            }],
            role: "auto".into(),
        },
        // candidates=None：合成 SDP 无远端候选 → ICE 不发起检查 → 测试窗内必不
        // Failed（真 server 内联候选的实盘路径归活体验收，见文件头④）。
        ice_candidates: None,
    }
}

/// 收一条上行信封消息（10s 超时，超时即 panic 定位）。
async fn read_msg(ws: &mut WebSocketStream<tokio::net::TcpStream>) -> SignalingMessage {
    let m = tokio::time::timeout(Duration::from_secs(10), ws.next())
        .await
        .expect("mock: 等待上行消息超时")
        .expect("mock: ws 流结束")
        .expect("mock: ws 读错误");
    let env: LocalEnvelope = serde_json::from_str(m.to_text().expect("mock: 非文本帧"))
        .expect("mock: 信封解析");
    env.msg
}

fn expect_create_transport(
    msg: SignalingMessage,
    direction: TransportDirection,
) {
    match msg {
        SignalingMessage::CreateWebRtcTransport {
            room_id,
            peer_id,
            direction: d,
        } => {
            assert_eq!(room_id, ROOM, "CreateWebRtcTransport 房间");
            assert_eq!(peer_id, "host", "SFU peer 键 = host（C1 惯例）");
            assert_eq!(d, direction, "transport 方向");
        }
        other => panic!("期望 CreateWebRtcTransport({direction:?}), got {other:?}"),
    }
}

fn expect_connect(msg: SignalingMessage, transport_id: &str) {
    match msg {
        SignalingMessage::ConnectWebRtcTransport {
            transport_id: tid,
            dtls_parameters,
            peer_id,
            ..
        } => {
            assert_eq!(tid, transport_id, "Connect 绑定的 transport_id");
            assert_eq!(peer_id, "host");
            assert_eq!(dtls_parameters.role, "client", "端点=DTLS client（mediasoup server 侧）");
            assert_eq!(dtls_parameters.fingerprints.len(), 1);
            assert!(
                dtls_parameters.fingerprints[0].value.len() > 32,
                "指纹应为本地 PC 真值（sha-256 冒号十六进制）"
            );
        }
        other => panic!("期望 ConnectWebRtcTransport, got {other:?}"),
    }
}

fn expect_data_producer(
    msg: SignalingMessage,
    label: &str,
    ordered: bool,
    max_retransmits: Option<u16>,
    seen_stream_ids: &mut Vec<u16>,
) -> SctpStreamParameters {
    let dp = match msg {
        SignalingMessage::CreateDataProducer {
            room_id,
            transport_direction,
            label: l,
            protocol,
            sctp_stream_parameters,
            transport_id,
            ..
        } => {
            assert_eq!(room_id, ROOM);
            assert_eq!(transport_direction, TransportDirection::Send, "{label} 必须走 send transport");
            assert_eq!(l, label, "announce label");
            assert_eq!(protocol, "sctp", "v1 子协议名");
            assert_eq!(transport_id.as_deref(), Some("t-send"), "C1 显式绑定 send transport");
            sctp_stream_parameters.expect("sctp_stream_parameters 必填（server 缺之必拒）")
        }
        other => panic!("期望 CreateDataProducer({label}), got {other:?}"),
    };
    assert_eq!(dp.ordered, ordered, "{label} ordered 与 channel_init 同源");
    assert_eq!(dp.max_retransmits, max_retransmits, "{label} 重传参数与 channel_init 同源");
    assert!(!seen_stream_ids.contains(&dp.stream_id), "stream_id 唯一（DC id 不重号）: {seen_stream_ids:?} + {}", dp.stream_id);
    seen_stream_ids.push(dp.stream_id);
    dp
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn controller_sfu_dc_signaling_sequence() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "warn,mediaservo_host=debug".into()),
        )
        .try_init();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    // ── mock 网关：按 S1 契约应答并逐项断言上行形状 ──
    let mock = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("mock accept");
        let mut ws = tokio_tungstenite::accept_async(stream).await.expect("mock ws");

        // ① RoomJoin → RoomJoined（网关本地合成，protocol=3 S0.5 方言）
        match read_msg(&mut ws).await {
            SignalingMessage::RoomJoin { room_id, peer_role, .. } => {
                assert_eq!(room_id, ROOM);
                assert_eq!(peer_role, PeerRole::Host);
            }
            other => panic!("期望 RoomJoin, got {other:?}"),
        }
        ws.send(env_text(SignalingMessage::RoomJoined {
            room_id: ROOM.into(),
            peer_id: "veh-peer".into(),
            protocol: Some(3),
            server_version: None,
            session_nonce: None,
        }))
        .await
        .unwrap();

        // ② 出程：Send transport → connect → 4× CreateDataProducer announce
        expect_create_transport(read_msg(&mut ws).await, TransportDirection::Send);
        ws.send(env_text(transport_created("t-send", "ufragSend1")))
            .await
            .unwrap();
        expect_connect(read_msg(&mut ws).await, "t-send");
        ws.send(env_text(SignalingMessage::Error {
            code: 0,
            message: "transport_connected".into(),
        }))
        .await
        .unwrap();

        let mut stream_ids: Vec<u16> = Vec::new();
        let cases = [
            ("chassis", true, None),
            ("gimbal", false, Some(5u16)),
            ("light", true, None),
            ("ack", true, None),
        ];
        let mut own_ids: Vec<String> = Vec::new();
        for (i, (label, ordered, retrans)) in cases.iter().enumerate() {
            expect_data_producer(read_msg(&mut ws).await, label, *ordered, *retrans, &mut stream_ids);
            let id = format!("dp-{i}-{label}");
            own_ids.push(id.clone());
            ws.send(env_text(SignalingMessage::DataProducerCreated {
                room_id: ROOM.into(),
                data_producer_id: id,
            }))
            .await
            .unwrap();
        }

        // ③ 入程：Recv transport → connect → 注入舱端 producer + 自建回声
        expect_create_transport(read_msg(&mut ws).await, TransportDirection::Recv);
        ws.send(env_text(transport_created("t-recv", "ufragRecv1")))
            .await
            .unwrap();
        expect_connect(read_msg(&mut ws).await, "t-recv");
        ws.send(env_text(SignalingMessage::Error {
            code: 0,
            message: "transport_connected".into(),
        }))
        .await
        .unwrap();

        // 自建 announce 的房间广播回声（server 语义广播给全房含自己）——
        // controller 必须按 id 跳过，绝不自消费
        for id in &own_ids {
            ws.send(env_text(SignalingMessage::NewDataProducer {
                room_id: ROOM.into(),
                data_producer_id: id.clone(),
                peer_id: "host".into(),
                label: "echo".into(),
                protocol: "sctp".into(),
            }))
            .await
            .unwrap();
        }
        // 舱端 control producer（v1 单 label 约定）
        ws.send(env_text(SignalingMessage::NewDataProducer {
            room_id: ROOM.into(),
            data_producer_id: "dp-cockpit".into(),
            peer_id: "veh-peer".into(),
            label: "control".into(),
            protocol: "sctp".into(),
        }))
        .await
        .unwrap();

        // 恰有一条 ConsumeData：舱端 producer、Recv、绑定 t-recv
        let mut consumed_seen = false;
        for _ in 0..8 {
            let msg = read_msg(&mut ws).await;
            match msg {
                SignalingMessage::ConsumeData {
                    room_id,
                    transport_direction,
                    data_producer_id,
                    transport_id,
                    ..
                } => {
                    assert_eq!(room_id, ROOM);
                    assert_eq!(transport_direction, TransportDirection::Recv);
                    assert_eq!(data_producer_id, "dp-cockpit", "只消费舱端 producer（自建回声不自消费）");
                    assert_eq!(transport_id.as_deref(), Some("t-recv"), "C1 显式绑定 recv transport");
                    consumed_seen = true;
                    ws.send(env_text(SignalingMessage::DataConsumed {
                        room_id: ROOM.into(),
                        data_consumer_id: "dc-cockpit".into(),
                        data_producer_id,
                        // S2d: 带外 negotiated DC 参数（真机由 worker 分配；此处夹具值）。
                        sctp_stream_parameters: Some(SctpStreamParameters {
                            stream_id: 9,
                            ordered: true,
                            max_packet_life_time: None,
                            max_retransmits: None,
                        }),
                        label: "chassis".into(),
                        protocol: String::new(),
                    }))
                    .await
                    .unwrap();
                    break;
                }
                // 建立期 DataProducerCreated 的网关广播回声可能后到——忽略
                SignalingMessage::DataProducerCreated { .. } => continue,
                other => panic!("期望 ConsumeData, got {other:?}"),
            }
        }
        assert!(consumed_seen, "NewDataProducer 注入后必须收到 ConsumeData");

        // ④ 存活一拍（DataConsumed 后主循环不得崩环），再断链触发自愈退出
        tokio::time::sleep(Duration::from_millis(300)).await;
        ws.close(None).await.expect("mock close");
    });

    // ── controller：生产客户端链（new_gateway + control_loop）──
    let signal = SignalClient::new_gateway(
        &format!("ws://127.0.0.1:{port}/ws"),
        "host-controller",
        ROOM,
        PeerRole::Host,
    )
    .connect()
    .await
    .expect("controller 信令连接（mock 网关）");

    let code = tokio::time::timeout(
        Duration::from_secs(30),
        control_loop(signal, ControllerConfig::default(), Arc::new(StubActuator), None, Default::default()),
    )
    .await
    .expect("control_loop 30s 未退出");
    // mock 主动断链 = 生产自愈语义（退出非 0 待 restart_policy 拉起）
    assert_eq!(code, 1, "信令断开应走自愈退出码 1（PIT-87 惯例）");
    mock.await.expect("mock 网关断言失败（见 panic 溯源）");
}
