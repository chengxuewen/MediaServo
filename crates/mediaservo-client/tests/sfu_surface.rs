//! client v2 SFU 面信令形状测试——mock server WS（直连形：PSK 文本帧 + 裸
//! SignalingMessage JSON，非网关 LocalEnvelope）。复用 host controller_e2e 的
//! TCP mock 模式；无外部 server / 无 mediasoup。
//!
//! 断言（S2 简报第 9 项）：
//! ① consume_video → 上行 CreateWebRtcTransport(Recv) → Consume{transport_id 绑定}
//!    → ConnectWebRtcTransport{role=client, 真指纹}
//! ② open_control → CreateWebRtcTransport(Send) → Connect → CreateDataProducer×label
//!    （protocol=sctp / sctp_stream_parameters 与 channel_init 同源 / transport_id 显式）
//! ③ CreateDataProducer 遇 Error{4012} → ClientError::ControlDenied（typed 终态）
//! ④ RoomJoined 无 protocol（旧 server）→ negotiated=1 → open_control 本地预拒
//!    ProtocolTooLow，不发任何 transport 请求
//!
//! 前置：backend-webrtc-sys 真 libwebrtc 本地协商 → Linux 门（controller_e2e 同款）。

#![cfg(target_os = "linux")]

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use mediaservo_client::error::ClientError;
use mediaservo_client::{ClientConfig, RoomSession};
use mediaservo_common::protocol::{
    DtlsParameters, Fingerprint, IceParameters, MediaKind, PeerRole, SctpStreamParameters,
    SignalingMessage, TransportDirection,
};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;

const ROOM: &str = "veh-test";
const PSK: &str = "mock-psk";
/// libwebrtc 拒畸形指纹——32 字节冒号十六进制（永不与 mock 完成 DTLS）。
const FAKE_FP: &str = "01:02:03:04:05:06:07:08:09:0A:0B:0C:0D:0E:0F:10:11:12:13:14:15:16:17:18:19:1A:1B:1C:1D:1E:1F:20";

type MockWs = WebSocketStream<tokio::net::TcpStream>;

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    /// SFU 全正向应答（TransportCreated / Consumed / DataProducerCreated）。
    Respond,
    /// CreateDataProducer 一律 4012 拒。
    DenyControl,
}

fn text(msg: &SignalingMessage) -> Message {
    serde_json::to_string(msg).expect("serialize").into()
}

async fn send_msg(ws: &mut MockWs, msg: &SignalingMessage) {
    ws.send(text(msg)).await.expect("mock: 写失败");
}

/// 收一条上行信令（8s 超时 panic 定位）。
async fn read_msg(ws: &mut MockWs) -> SignalingMessage {
    let m = tokio::time::timeout(Duration::from_secs(8), ws.next())
        .await
        .expect("mock: 等上行消息超时")
        .expect("mock: ws 流结束")
        .expect("mock: ws 读错误");
    serde_json::from_str(m.to_text().expect("mock: 非文本帧")).expect("mock: 消息解析")
}

fn transport_created(id: &str) -> SignalingMessage {
    SignalingMessage::WebRtcTransportCreated {
        room_id: ROOM.into(),
        peer_id: "consumer".into(),
        transport_id: id.into(),
        ice_parameters: IceParameters {
            username_fragment: "ufrag1234".into(),
            password: "pwdpwdpwdpwdpwdpwdpwdpwdpwdpwdpwd".into(), // libwebrtc 下限 22
        },
        dtls_parameters: DtlsParameters {
            fingerprints: vec![Fingerprint {
                algorithm: "sha-256".into(),
                value: FAKE_FP.into(),
            }],
            role: "auto".into(),
        },
        // None = 无远端候选 → ICE 不发检查，测试窗内必不 Failed（controller_e2e 同法）
        ice_candidates: None,
        sctp_parameters: None,
    }
}

/// 起 mock server：握手（PSK→auth_ok→RoomJoin→RoomJoined{protocol}）后转入
/// 脚本应答循环；上行消息经观察通道回传测试断言。ws 由后台任务持有至测试结束。
async fn pair(
    protocol: Option<u32>,
    mode: Mode,
) -> (RoomSession, mpsc::UnboundedReceiver<SignalingMessage>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let cfg = ClientConfig {
        signaling_url: format!("ws://{addr}/ws"),
        room_id: ROOM.into(),
        psk: Some(PSK.into()),
        jwt: None,
        role: PeerRole::Consumer,
    };
    let (tx, rx) = mpsc::unbounded_channel();
    let server = tokio::spawn(async move {
        let (sock, _) = listener.accept().await.unwrap();
        let mut ws: MockWs = tokio_tungstenite::accept_async(sock).await.unwrap();

        // ── 握手（与 RoomSession::connect 交错：mock 必须先答，双方才不互等）
        let frame = ws.next().await.unwrap().unwrap();
        assert_eq!(frame.to_text().unwrap(), PSK, "直连形第一帧 = PSK 明文");
        send_msg(&mut ws, &SignalingMessage::Error { code: 0, message: "auth_ok".into() }).await;
        let join = read_msg(&mut ws).await;
        match join {
            SignalingMessage::RoomJoin { room_id, peer_role, protocol: claim, .. } => {
                assert_eq!(room_id, ROOM);
                assert_eq!(peer_role, PeerRole::Consumer);
                assert_eq!(
                    claim,
                    Some(mediaservo_common::protocol::SIGNALING_PROTOCOL_VERSION),
                    "S0：join 必带方言声明"
                );
            }
            other => panic!("期望 RoomJoin, got {other:?}"),
        }
        send_msg(
            &mut ws,
            &SignalingMessage::RoomJoined {
                room_id: ROOM.into(),
                peer_id: "consumer-test".into(),
                protocol,
                server_version: None,
                session_nonce: None,
            },
        )
        .await;

        // ── 脚本应答循环（对端 drop 收敛）
        while let Some(m) = ws.next().await {
            let Ok(m) = m else { break };
            let Some(t) = m.to_text().ok() else { continue };
            let Ok(msg) = serde_json::from_str::<SignalingMessage>(t) else { continue };
            let _ = tx.send(msg.clone());
            match msg {
                SignalingMessage::CreateWebRtcTransport { direction, .. } => {
                    let id = match direction {
                        TransportDirection::Recv => "t-recv",
                        TransportDirection::Send => "t-send",
                    };
                    send_msg(&mut ws, &transport_created(id)).await;
                }
                SignalingMessage::GetRouterRtpCapabilities { .. } => {
                    send_msg(
                        &mut ws,
                        &SignalingMessage::RouterRtpCapabilities {
                            room_id: ROOM.into(),
                            capabilities: serde_json::json!({
                                "codecs": [{"mimeType": "video/VP8", "clockRate": 90000,
                                           "kind": "video", "payloadTypes": [96]}],
                                "headerExtensions": [],
                            }),
                        },
                    )
                    .await;
                }
                SignalingMessage::Consume { .. } => {
                    send_msg(
                        &mut ws,
                        &SignalingMessage::Consumed {
                            room_id: ROOM.into(),
                            consumer_id: "cons-1".into(),
                            producer_id: "prod-1".into(),
                            kind: MediaKind::Video,
                            rtp_parameters: serde_json::json!({
                                "codecs": [{"mimeType": "video/VP8", "payloadType": 96,
                                           "clockRate": 90000, "parameters": {}}],
                                "encodings": [{"ssrc": 11112222}]
                            }),
                        },
                    )
                    .await;
                }
                SignalingMessage::ConnectWebRtcTransport { .. } => {
                    send_msg(
                        &mut ws,
                        &SignalingMessage::Error {
                            code: 0,
                            message: "transport_connected".into(),
                        },
                    )
                    .await;
                }
                SignalingMessage::CreateDataProducer { label, .. } => {
                    let reply = if mode == Mode::DenyControl {
                        SignalingMessage::Error {
                            code: 4012,
                            message: format!("control_denied: {label}"),
                        }
                    } else {
                        SignalingMessage::DataProducerCreated {
                            room_id: ROOM.into(),
                            data_producer_id: format!("dp-{label}"),
                        }
                    };
                    send_msg(&mut ws, &reply).await;
                }
                other => panic!("mock: 意外上行消息 {other:?}"),
            }
        }
    });
    let session = match RoomSession::connect(&cfg).await {
        Ok(s) => s,
        Err(e) => {
            // mock 侧 panic 经 JoinHandle 浮出（不吞第二现场）
            let reason = server.await.expect_err("mock 任务应 panic");
            panic!("connect 失败: {e:?}（mock panic: {reason}）");
        }
    };
    (session, rx)
}

/// 取 n 条已观察上行（5s/条超时兜底）。
async fn take(
    rx: &mut mpsc::UnboundedReceiver<SignalingMessage>,
    n: usize,
) -> Vec<SignalingMessage> {
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        out.push(
            tokio::time::timeout(Duration::from_secs(5), rx.recv())
                .await
                .expect("等上行消息超时")
                .expect("观察通道关闭"),
        );
    }
    out
}

// ───────── 测试 ─────────

#[tokio::test]
async fn consume_video_drives_recv_sfu_sequence() {
    let (mut session, mut obs) = pair(Some(3), Mode::Respond).await;
    assert_eq!(session.negotiated(), 3);

    session.consume_video("prod-1").await.expect("consume_video 应建立");

    let msgs = take(&mut obs, 4).await;
    match &msgs[0] {
        SignalingMessage::CreateWebRtcTransport { room_id, peer_id, direction } => {
            assert_eq!(room_id, ROOM);
            assert_eq!(peer_id, "consumer", "SFU peer 键 = role 派生（C1 惯例）");
            assert_eq!(direction, &TransportDirection::Recv);
        }
        other => panic!("① 期望 CreateWebRtcTransport(Recv), got {other:?}"),
    }
    match &msgs[1] {
        // S2b：consume 前必须先查 router caps（手拼 caps 被真 mediasoup 拒——勿回退）
        SignalingMessage::GetRouterRtpCapabilities { room_id } => assert_eq!(room_id, ROOM),
        other => panic!("① 期望 GetRouterRtpCapabilities, got {other:?}"),
    }
    match &msgs[2] {
        SignalingMessage::Consume { producer_id, transport_id, rtp_capabilities, .. } => {
            assert_eq!(producer_id, "prod-1");
            assert_eq!(transport_id.as_deref(), Some("t-recv"), "C1 显式绑 recv transport");
            assert!(
                rtp_capabilities["codecs"].to_string().contains("VP8"),
                "Consume 必须携带 router 回包 caps，实得 {rtp_capabilities}"
            );
        }
        other => panic!("① 期望 Consume, got {other:?}"),
    }
    match &msgs[3] {
        SignalingMessage::ConnectWebRtcTransport { transport_id, dtls_parameters, .. } => {
            assert_eq!(transport_id, "t-recv");
            assert_eq!(dtls_parameters.role, "client", "端点 = DTLS client");
            assert_eq!(dtls_parameters.fingerprints.len(), 1);
            assert!(
                dtls_parameters.fingerprints[0].value.len() > 32,
                "指纹应为本地 PC 真值"
            );
        }
        other => panic!("① 期望 ConnectWebRtcTransport, got {other:?}"),
    }
}

#[tokio::test]
async fn open_control_announces_data_producers() {
    let (mut session, mut obs) = pair(Some(3), Mode::Respond).await;
    let ctl = session.open_control(&["chassis", "gimbal"]).await.expect("open_control 应建立");
    assert_eq!(ctl.labels(), ["chassis", "gimbal"]);
    assert_eq!(ctl.producer_ids(), ["dp-chassis", "dp-gimbal"]);

    let msgs = take(&mut obs, 4).await;
    match &msgs[0] {
        SignalingMessage::CreateWebRtcTransport { direction, .. } => {
            assert_eq!(direction, &TransportDirection::Send);
        }
        other => panic!("② 期望 CreateWebRtcTransport(Send), got {other:?}"),
    }
    assert!(
        matches!(msgs[1], SignalingMessage::ConnectWebRtcTransport { .. }),
        "② Connect 必须先于 CreateDataProducer（S1 时序合同）"
    );
    let mut stream_ids = Vec::new();
    for (i, (want_label, want_ordered, want_retrans)) in [
        ("chassis", true, None),
        ("gimbal", false, Some(5u16)),
    ]
    .iter()
    .enumerate()
    {
        match &msgs[2 + i] {
            SignalingMessage::CreateDataProducer {
                label,
                protocol,
                sctp_stream_parameters,
                transport_id,
                transport_direction,
                ..
            } => {
                assert_eq!(label, *want_label);
                assert_eq!(protocol, "sctp");
                assert_eq!(transport_direction, &TransportDirection::Send);
                assert_eq!(transport_id.as_deref(), Some("t-send"), "C1 显式绑 send transport");
                let sp: &SctpStreamParameters = sctp_stream_parameters
                    .as_ref()
                    .unwrap_or_else(|| panic!("② {want_label} 缺 sctp_stream_parameters"));
                assert_eq!(sp.ordered, *want_ordered, "{want_label} ordered（D-H3 同源）");
                assert_eq!(sp.max_retransmits, *want_retrans, "{want_label} retransmits");
                stream_ids.push(sp.stream_id);
            }
            other => panic!("② 期望 CreateDataProducer, got {other:?}"),
        }
    }
    assert_ne!(stream_ids[0], stream_ids[1], "stream_id = libwebrtc 实配 DC id，必互异");
}

#[tokio::test]
async fn control_denied_4012_maps_typed_terminal() {
    let (mut session, _obs) = pair(Some(3), Mode::DenyControl).await;
    let e = session.open_control(&["chassis"]).await.unwrap_err();
    assert!(
        matches!(&e, ClientError::ControlDenied(m) if m.contains("chassis")),
        "got {e:?}"
    );
}

#[tokio::test]
async fn old_server_negotiated_1_pre_refuses_control() {
    let (mut session, mut obs) = pair(None, Mode::Respond).await;
    assert_eq!(session.negotiated(), 1, "RoomJoined 无 protocol = 旧 server 方言 1");
    let e = session.open_control(&["chassis"]).await.unwrap_err();
    assert!(
        matches!(e, ClientError::ProtocolTooLow { need: 2, got: 1 }),
        "I5 门前置本地预拒，got {e:?}"
    );
    // 未发出任何 transport 请求（观察通道保持空）
    assert!(
        tokio::time::timeout(Duration::from_millis(300), obs.recv())
            .await
            .is_err(),
        "④ 预拒不得产生上行"
    );
}
