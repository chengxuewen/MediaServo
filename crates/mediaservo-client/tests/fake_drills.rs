//! K11 批0 确定性演练——FakeEngine + mock WS 信令全链（sfu_surface 的假引擎姿态）。
//!
//! 与 sfu_surface.rs 的分工：那边钉「上行信令形状」（真 libwebrtc 本地协商），
//! 这边钉「引擎行为 → SDK 语义」映射（帧注入/ack 回程/故障/DC 态），CI 秒级、
//! 无 OS 门（fake 不触 libwebrtc 运行时；crate 链接依赖与默认姿态相同）。
//!
//! 演练面（ticket 批0 验收 ②）：
//! a) join→consume→帧回调→ack 往返全链
//! b) 引擎故障 → ClientError::WebRtc 正确映射，会话续用不 panic
//! c) DC open 前后 send 的状态拒绝（+ drop_dc_mid_send 假送达语义）
//! d) 双 consumer 互不连坐（consume 面多路；ack 泵单路 = 批1 前置观察，见测试尾注）

#![cfg(feature = "engine-fake")]

use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;

use mediaservo_client::engine::Engine;
use mediaservo_client::engine::fake::FakeEngine;
use mediaservo_client::error::ClientError;
use mediaservo_client::{ClientConfig, RoomSession};
use mediaservo_common::protocol::{
    ControlAck, DtlsParameters, Fingerprint, IceParameters, MediaKind, PeerRole,
    SctpStreamParameters, SignalingMessage, TransportDirection,
};

const ROOM: &str = "veh-fake";
const PSK: &str = "mock-psk";
const FAKE_FP: &str = "01:02:03:04:05:06:07:08:09:0A:0B:0C:0D:0E:0F:10:11:12:13:14:15:16:17:18:19:1A:1B:1C:1D:1E:1F:20";

type MockWs = WebSocketStream<tokio::net::TcpStream>;

fn text(msg: &SignalingMessage) -> Message {
    serde_json::to_string(msg).expect("serialize").into()
}

async fn send_msg(ws: &mut MockWs, msg: &SignalingMessage) {
    ws.send(text(msg)).await.expect("mock: 写失败");
}

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
            password: "pwdpwdpwdpwdpwdpwdpwdpwdpwdpwdpwd".into(),
        },
        dtls_parameters: DtlsParameters {
            fingerprints: vec![Fingerprint { algorithm: "sha-256".into(), value: FAKE_FP.into() }],
            role: "auto".into(),
        },
        ice_candidates: None,
        sctp_parameters: None,
    }
}

struct Harness {
    session: RoomSession,
    fake: FakeEngine,
    /// 上行信令观察通道。
    obs: mpsc::UnboundedReceiver<SignalingMessage>,
    /// 下行推送通道（server 主动广播 NewProducer/NewDataProducer 用）。
    push: mpsc::UnboundedSender<SignalingMessage>,
}

/// 起 mock WS server（握手 + 全 SFU 脚本应答 + 主动推送合流），fake 引擎入房。
async fn pair_fake() -> Harness {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let cfg = ClientConfig {
        signaling_url: format!("ws://{addr}/ws"),
        room_id: ROOM.into(),
        psk: Some(PSK.into()),
        jwt: None,
        role: PeerRole::Consumer,
        hmac_key: None,
    };
    let (obs_tx, obs_rx) = mpsc::unbounded_channel();
    let (push_tx, mut push_rx) = mpsc::unbounded_channel::<SignalingMessage>();
    tokio::spawn(async move {
        let (sock, _) = listener.accept().await.unwrap();
        let mut ws: MockWs = tokio_tungstenite::accept_async(sock).await.unwrap();

        // ── 握手（sfu_surface 同形）
        let frame = ws.next().await.unwrap().unwrap();
        assert_eq!(frame.to_text().unwrap(), PSK, "直连形第一帧 = PSK 明文");
        send_msg(&mut ws, &SignalingMessage::Error { code: 0, message: "auth_ok".into() }).await;
        match read_msg(&mut ws).await {
            SignalingMessage::RoomJoin { room_id, peer_role, .. } => {
                assert_eq!(room_id, ROOM);
                assert_eq!(peer_role, PeerRole::Consumer);
            }
            other => panic!("期望 RoomJoin, got {other:?}"),
        }
        send_msg(
            &mut ws,
            &SignalingMessage::RoomJoined {
                room_id: ROOM.into(),
                peer_id: "consumer-test".into(),
                protocol: Some(3),
                server_version: None,
                session_nonce: None,
            },
        )
        .await;

        // ── 脚本应答 + 推送合流（对端 drop 收敛）
        let mut tseq = 0u32;
        loop {
            tokio::select! {
                m = ws.next() => {
                    let Some(m) = m else { break };
                    let Ok(m) = m else { break };
                    let Some(t) = m.to_text().ok() else { continue };
                    let Ok(msg) = serde_json::from_str::<SignalingMessage>(t) else { continue };
                    let _ = obs_tx.send(msg.clone());
                    let reply = match msg {
                        SignalingMessage::CreateWebRtcTransport { .. } => {
                            tseq += 1;
                            transport_created(&format!("t-{tseq}"))
                        }
                        SignalingMessage::GetRouterRtpCapabilities { .. } => {
                            SignalingMessage::RouterRtpCapabilities {
                                room_id: ROOM.into(),
                                capabilities: serde_json::json!({
                                    "codecs": [{"mimeType": "video/VP8", "clockRate": 90000,
                                               "kind": "video", "payloadTypes": [96]}],
                                    "headerExtensions": [],
                                }),
                            }
                        }
                        SignalingMessage::Consume { .. } => SignalingMessage::Consumed {
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
                        SignalingMessage::ConnectWebRtcTransport { .. } => {
                            SignalingMessage::Error { code: 0, message: "transport_connected".into() }
                        }
                        SignalingMessage::CreateDataProducer { label, .. } => {
                            SignalingMessage::DataProducerCreated {
                                room_id: ROOM.into(),
                                data_producer_id: format!("dp-{label}"),
                            }
                        }
                        SignalingMessage::ConsumeData { data_producer_id, .. } => {
                            SignalingMessage::DataConsumed {
                                room_id: ROOM.into(),
                                data_consumer_id: "dc-ack".into(),
                                data_producer_id,
                                sctp_stream_parameters: Some(SctpStreamParameters {
                                    stream_id: 100,
                                    ordered: true,
                                    max_packet_life_time: None,
                                    max_retransmits: None,
                                }),
                                label: "ack".into(),
                                protocol: "sctp".into(),
                            }
                        }
                        other => panic!("mock: 意外上行消息 {other:?}"),
                    };
                    send_msg(&mut ws, &reply).await;
                }
                Some(p) = push_rx.recv() => {
                    send_msg(&mut ws, &p).await;
                }
            }
        }
    });

    let fake = FakeEngine::new();
    let engine: Arc<dyn Engine> = Arc::new(fake.clone());
    let session =
        RoomSession::connect_with_engine(&cfg, engine).await.expect("fake 姿态 connect 应成功");
    Harness { session, fake, obs: obs_rx, push: push_tx }
}

/// 观察上行直到命中谓词（中间消息跳过）。
async fn wait_uplink<F>(obs: &mut mpsc::UnboundedReceiver<SignalingMessage>, what: &str, pred: F)
where
    F: Fn(&SignalingMessage) -> bool,
{
    for _ in 0..64 {
        let m = tokio::time::timeout(Duration::from_secs(5), obs.recv())
            .await
            .unwrap_or_else(|_| panic!("等上行超时（{what}）"))
            .expect("观察通道关闭");
        if pred(&m) {
            return;
        }
    }
    panic!("未观察到 {what}（跳过窗耗尽）");
}

/// 向 fake 世界投递 DC 入程消息，直到 ack 泵的 consumer DC 建立（重试消化竞态）。
async fn deliver_until_hit(fake: &FakeEngine, label: &str, bytes: Vec<u8>) {
    for _ in 0..100 {
        if fake.deliver_dc_message(label, bytes.clone()) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("label={label} 的 consumer DC 始终未建立");
}

// ───────── 演练 ─────────

/// a) join→consume→帧回调→ack 往返全链（fake 引擎 + mock 信令，零真 webrtc）。
#[tokio::test]
async fn drill_a_join_consume_frames_ack_roundtrip() {
    let h = pair_fake().await;
    let Harness { mut session, fake, mut obs, push } = h;

    // 发现 producer（server 广播 NewProducer）
    push.send(SignalingMessage::NewProducer {
        room_id: ROOM.into(),
        producer_id: "prod-1".into(),
        peer_id: "host-1".into(),
        kind: MediaKind::Video,
    })
    .unwrap();
    assert_eq!(session.wait_video_producer(Duration::from_secs(5)).await.unwrap(), "prod-1");

    // consume → 引擎协商（fake）→ 注入 track + 帧 → VideoFrame 出流
    let mut frames = session.consume_video("prod-1").await.expect("consume_video");
    fake.inject_video_track();
    fake.inject_frame(640, 360);
    let f = tokio::time::timeout(Duration::from_secs(2), frames.recv())
        .await
        .expect("帧回调超时")
        .expect("帧流关闭");
    assert_eq!((f.width, f.height), (640, 360));
    assert_eq!(f.data.len(), 640 * 360 * 3 / 2, "I420 尺寸");
    let s = session.video_stats_summary();
    assert_eq!(s.frames_decoded, 1, "注入帧应计入 stats");
    assert_eq!(s.frame_width, 640);

    // 控制面：出程 DC announce + ack 泵消费回程
    let mut ctl = session.open_control(&["chassis"]).await.expect("open_control");
    push.send(SignalingMessage::NewDataProducer {
        room_id: ROOM.into(),
        data_producer_id: "dp-ack".into(),
        peer_id: "host-1".into(),
        label: "ack".into(),
        protocol: "sctp".into(),
    })
    .unwrap();
    wait_uplink(&mut obs, "ConsumeData", |m| matches!(m, SignalingMessage::ConsumeData { .. }))
        .await;

    let env_json = serde_json::json!({"seq": 7, "cmd": "steer", "payload": {"deg": 10}});
    ctl.send("chassis", 7, "steer", env_json["payload"].clone()).await.expect("send");
    assert!(
        fake.sent_texts("chassis").first().is_some_and(|t| t.contains("\"seq\":7")),
        "出程信封应被 fake 记账: {:?}",
        fake.sent_texts("chassis")
    );

    let ack_bytes =
        serde_json::to_vec(&ControlAck { ack: 7, result: serde_json::json!({"ok": true}) })
            .unwrap();
    deliver_until_hit(&fake, "ack", ack_bytes).await;
    let ack = ctl.recv_ack_for(7, Duration::from_secs(5)).await.expect("recv_ack_for");
    assert_eq!(ack.ack, 7);
    assert_eq!(ack.result["ok"], serde_json::json!(true));
}

/// b) 引擎建连故障 → WebRtc typed 错误；会话不 panic、后续操作可成（无泄漏连坐）。
#[tokio::test]
async fn drill_b_engine_fault_maps_error_and_session_survives() {
    let h = pair_fake().await;
    let Harness { mut session, fake, obs: _obs, push: _push } = h;

    fake.fail_next_connect(1);
    let e = session.consume_video("prod-x").await.unwrap_err();
    assert!(
        matches!(&e, ClientError::WebRtc(m) if m.contains("故障注入")),
        "引擎故障应映射 WebRtc, got {e:?}"
    );

    // 旗标一次性消耗：同会话二次 consume 成功（无脏状态连坐）
    let mut frames = session.consume_video("prod-x").await.expect("二次 consume 应成功");
    fake.inject_video_track();
    fake.inject_frame(320, 240);
    assert!(tokio::time::timeout(Duration::from_secs(2), frames.recv()).await.is_ok());

    // 控制面同点位故障：失败发生在 create_pc（transport 应答已消费），不 spawn 泵
    fake.fail_next_connect(1);
    let e2 = session.open_control(&["chassis"]).await.unwrap_err();
    assert!(matches!(e2, ClientError::WebRtc(_)), "got {e2:?}");
}

/// c) DC open 前 send 拒、open 后放行；drop_dc_mid_send = 假送达（Ok 但不出账）。
#[tokio::test]
async fn drill_c_dc_send_state_and_mid_drop() {
    let h = pair_fake().await;
    let Harness { mut session, fake, obs: _obs, push: _push } = h;

    fake.set_dc_auto_open(false);
    let ctl = session.open_control(&["chassis"]).await.expect("open_control");

    let e = ctl.send("chassis", 1, "steer", serde_json::json!({})).await.unwrap_err();
    assert!(
        matches!(&e, ClientError::WebRtc(m) if m.contains("not open")),
        "未 open 应拒发, got {e:?}"
    );
    assert!(fake.sent_texts("chassis").is_empty());

    fake.open_data_channels();
    ctl.send("chassis", 2, "steer", serde_json::json!({})).await.expect("open 后应可发");
    assert_eq!(fake.sent_texts("chassis").len(), 1);

    fake.drop_dc_mid_send();
    ctl.send("chassis", 3, "steer", serde_json::json!({})).await.expect("假送达 = Ok");
    assert_eq!(fake.sent_texts("chassis").len(), 1, "丢弃注入不得入账");
}

/// d) 双 consumer 独立：各自帧流互不连坐；drop 一路不影响另一路。
///
/// 批1 前置观察（在册，不硬造）：consume 面天然多路（_pcs 逐路 push），但
/// **ack 泵每会话一次性**（pump_events.take()）——第二路控制域需批1 解除单路闸。
#[tokio::test]
async fn drill_d_two_consumers_independent() {
    let h = pair_fake().await;
    let Harness { mut session, fake, obs: _obs, push: _push } = h;

    let mut f1 = session.consume_video("p1").await.expect("consume p1");
    let mut f2 = session.consume_video("p2").await.expect("consume p2");
    fake.inject_video_track(); // 两路 pc 各得一 track
    fake.inject_frame(160, 90);
    assert!(tokio::time::timeout(Duration::from_secs(2), f1.recv()).await.is_ok());
    assert!(tokio::time::timeout(Duration::from_secs(2), f2.recv()).await.is_ok());

    drop(f1); // 一路消费者撤走
    fake.inject_frame(160, 90); // 双 pc 都再投帧：p1 的 sink try_send 静默失败
    let f = tokio::time::timeout(Duration::from_secs(2), f2.recv())
        .await
        .expect("f2 不得被 f1 连坐")
        .expect("f2 流关闭");
    assert_eq!((f.width, f.height), (160, 90));
    // stats 跨 pc 折叠：两路各 2 帧 = 4（帧投了两路 sink，drop 只影响出流不影响计数）
    assert_eq!(session.video_stats_summary().frames_decoded, 4);
}
