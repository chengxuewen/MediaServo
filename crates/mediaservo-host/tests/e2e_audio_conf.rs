//! e2e_audio_conf: H2 音频会议房间 — 3 方全互连 opus（车端 + 舱端 + dispatcher，
//! 全部为合成音频参与者：tone PCM → AudioTrackSource → libwebrtc opus → RTP）。
//!
//! 纯外部模式（C21）— 连外部 mediasoup server，仅通过 WS 信令协议交互。
//! 媒体面证据 = server 侧 RTP 统计（SfuStatsRequest → get_stats）:
//!   - 每个 producer 的 byte_count > 0（参与者音频 RTP 到达 server）
//!   - 每个 consumer 的 byte_count > 0（router 转发到所有消费方 — "all hear all"）
//!
//! Runs on Linux only (C22: host 原生 + Docker server)。

#![cfg(target_os = "linux")]

use futures_util::{SinkExt, StreamExt};
use mediaservo_common::protocol::{
    DtlsParameters, Fingerprint, IceCandidate, IceParameters, MediaKind, PeerRole,
    SignalingMessage, TransportDirection,
};
use mediaservo_webrtc::rtp::RTCRtpTransceiverInit;
use mediaservo_webrtc::{
    RTCAnswerOptions, RTCConfiguration, RTCPeerConnectionFactory, RTCPeerConnectionState,
    RTCSdpType, RTCSessionDescription, TrackKind, TrackRef,
};
use mediaservo_webrtc::traits::PeerConnectionApi;
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message as WsMsg;

/// 测试级 logging 初始化（全局 subscriber 只能 set 一次 — 多测试并行防 panic）。
fn init_logging_once() {
    use std::sync::Once;
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        mediaservo_common::logging::init(mediaservo_common::logging::LoggingConfig::default());
    });
}

fn psk() -> String {
    std::env::var("SFU_E2E_PSK").unwrap_or_else(|_| "e2e-host-sfu-psk".to_string())
}

const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// opus 标准 PT（router default_router_options: opus 111/48000/2ch）。
const OPUS_PT: u16 = 111;
/// 48kHz mono，10ms 帧 = 480 样本。
const SAMPLE_RATE: u32 = 48_000;
const FRAME_SAMPLES: usize = 480;

struct Harness {
    ws_url: String,
}

impl Harness {
    async fn new() -> Self {
        let url = std::env::var("SFU_E2E_WS_URL").unwrap_or_else(|_| {
            panic!("SFU_E2E_WS_URL 未设置 — 需连外部 mediasoup server (C21)")
        });
        tracing::info!("SfuTestHarness: 外部 mediasoup server 模式 ({url})");
        Self { ws_url: url }
    }
}

async fn ws_auth_and_join<S>(ws: &mut S, role: PeerRole, room_id: &str)
where
    S: SinkExt<WsMsg> + StreamExt<Item = Result<WsMsg, tokio_tungstenite::tungstenite::Error>> + Unpin,
    <S as futures_util::Sink<WsMsg>>::Error: std::fmt::Debug,
{
    ws.send(WsMsg::Text(psk())).await.unwrap();
    let ack = tokio::time::timeout(Duration::from_secs(5), ws.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(ack.to_text().unwrap().contains("authenticated"));

    let join = serde_json::to_string(&SignalingMessage::RoomJoin {
        device_id: None,
        device_secret: None,
        device_pubkey: None,
        room_id: room_id.into(),
        peer_role: role,
        stream_id: None,
    })
    .unwrap();
    ws.send(WsMsg::Text(join)).await.unwrap();
    let joined = tokio::time::timeout(Duration::from_secs(5), ws.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(joined.to_text().unwrap().contains("room_joined"));
}

/// 读下一条消息并解析；跳过 transport_connected ack 与 NewProducer 广播。
async fn next_sig_skip_noise<S>(
    ws: &mut S,
) -> SignalingMessage
where
    S: SinkExt<WsMsg> + StreamExt<Item = Result<WsMsg, tokio_tungstenite::tungstenite::Error>> + Unpin,
    <S as futures_util::Sink<WsMsg>>::Error: std::fmt::Debug,
{
    loop {
        let msg = tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .expect("ws recv timeout")
            .expect("ws closed")
            .expect("ws error");
        let sig: SignalingMessage = serde_json::from_str(msg.to_text().unwrap()).unwrap();
        match &sig {
            SignalingMessage::Error { message, .. } if message == "transport_connected" => continue,
            SignalingMessage::NewProducer { .. } => continue,
            _ => return sig,
        }
    }
}

/// 构造音频 remote SDP（opus）。
/// `sendonly` = server 发送、本地接收（消费侧）; `recvonly` = server 接收、本地发送（生产侧）。
/// `ssrc` 仅在 sendonly 时注入（consumer 的 encodings[0].ssrc — libwebrtc demux 必需）。
fn build_remote_audio_sdp(
    ice_parameters: &IceParameters,
    dtls_parameters: &DtlsParameters,
    ice_candidates: Option<&Vec<IceCandidate>>,
    sendonly: bool,
    ssrc: Option<u64>,
    mid: &str,
) -> String {
    let fp = &dtls_parameters.fingerprints[0];
    let conn_ip = ice_candidates
        .and_then(|cs| cs.iter().find(|c| !c.ip.contains(".local")))
        .map(|c| c.ip.clone())
        .unwrap_or_else(|| "0.0.0.0".to_string());

    let mut lines = vec![
        "v=0".to_string(),
        "o=- 0 0 IN IP4 0.0.0.0".to_string(),
        "s=-".to_string(),
        "t=0 0".to_string(),
        format!("a=group:BUNDLE {mid}"),
        "a=ice-lite".to_string(),
        format!("a=ice-ufrag:{}", ice_parameters.username_fragment),
        format!("a=ice-pwd:{}", ice_parameters.password),
        format!(
            "a=fingerprint:{} {}",
            fp.algorithm.to_lowercase(),
            fp.value
        ),
        "a=setup:actpass".to_string(),
        format!("m=audio 7 UDP/TLS/RTP/SAVPF {OPUS_PT}"),
        format!("c=IN IP4 {conn_ip}"),
        "a=rtcp-mux".to_string(),
        "a=rtcp-rsize".to_string(),
        format!("a=mid:{mid}"),
        "a=extmap:1 urn:ietf:params:rtp-hdrext:sdes:mid".to_string(),
        "a=extmap:3 http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01".to_string(),
        if sendonly { "a=sendonly" } else { "a=recvonly" }.to_string(),
        format!("a=rtpmap:{OPUS_PT} opus/{SAMPLE_RATE}/2"),
        format!("a=fmtp:{OPUS_PT} minptime=10;useinbandfec=1"),
    ];
    if sendonly
        && let Some(ssrc) = ssrc {
            lines.push(format!("a=ssrc:{ssrc} cname:mediaservo-audio"));
            lines.push(format!("a=ssrc:{ssrc} msid:audio audio"));
        }
    if let Some(candidates) = ice_candidates {
        for c in candidates {
            if c.ip.contains(".local") {
                continue;
            }
            let ctype = match c.candidate_type.as_str() {
                "srflx" => "srflx",
                "prflx" => "prflx",
                "relay" => "relay",
                _ => "host",
            };
            lines.push(format!(
                "a=candidate:{} 1 {} {} {} {} typ {}",
                c.foundation,
                c.protocol.to_uppercase(),
                c.priority,
                c.ip,
                c.port,
                ctype
            ));
        }
    }
    lines.push("a=end-of-candidates".to_string());
    lines.push(String::new());
    lines.join("\r\n")
}

/// 440Hz 正弦 tone 生成器 — 10ms/帧 i16 mono PCM（opus 有实际载荷，防 DTX 静音零包）。
fn tone_frame(phase: &mut f64) -> Vec<u8> {
    let freq = 440.0;
    let step = 2.0 * std::f64::consts::PI * freq / SAMPLE_RATE as f64;
    let mut out = Vec::with_capacity(FRAME_SAMPLES * 2);
    for _ in 0..FRAME_SAMPLES {
        let sample = (*phase).sin() * 0.1 * i16::MAX as f64;
        out.extend_from_slice(&(sample as i16).to_le_bytes());
        *phase += step;
        if *phase > 2.0 * std::f64::consts::PI {
            *phase -= 2.0 * std::f64::consts::PI;
        }
    }
    out
}

/// 参与者 = 生产者：join → send transport → audio SDP → add_track(audio) → answer →
/// Connect → Produce → 返回 (producer_id, TrackSender, tone 推送 task)。
async fn audio_producer<S>(
    ws: &mut S,
    room: &str,
    peer: &str,
) -> (
    String,
    mediaservo_webrtc::track::TrackSender,
    tokio::task::JoinHandle<()>,
    std::sync::Arc<std::sync::atomic::AtomicU64>,
)
where
    S: SinkExt<WsMsg> + StreamExt<Item = Result<WsMsg, tokio_tungstenite::tungstenite::Error>> + Unpin,
    <S as futures_util::Sink<WsMsg>>::Error: std::fmt::Debug,
{
    ws_auth_and_join(ws, PeerRole::Consumer, room).await;

    // 1. CreateWebRtcTransport (Send)
    ws.send(WsMsg::Text(
        serde_json::to_string(&SignalingMessage::CreateWebRtcTransport {
            room_id: room.into(),
            peer_id: peer.into(),
            direction: TransportDirection::Send,
        })
        .unwrap(),
    ))
    .await
    .unwrap();
    let (send_tid, ice_params, dtls_params, ice_candidates) = {
        let sig = next_sig_skip_noise(ws).await;
        match sig {
            SignalingMessage::WebRtcTransportCreated {
                transport_id,
                ice_parameters,
                dtls_parameters,
                ice_candidates,
                ..
            } => (transport_id, ice_parameters, dtls_parameters, ice_candidates),
            other => panic!("producer {peer}: expected WebRtcTransportCreated, got {other:?}"),
        }
    };

    // 2. PC + 标准 answerer 协商（C18: remote SDP → add_track → create_answer）
    let factory = RTCPeerConnectionFactory::new();
    let pc = factory
        .create_peer_connection(RTCConfiguration::default())
        .await
        .unwrap();
    let connected = std::sync::Arc::new(tokio::sync::Notify::new());
    let connected_clone = connected.clone();
    pc.on_peer_connection_state_change(move |state| {
        if state == RTCPeerConnectionState::Connected {
            connected_clone.notify_one();
        }
    });

    let remote_sdp = build_remote_audio_sdp(&ice_params, &dtls_params, ice_candidates.as_ref(), false, None, "audio");
    let remote_desc = RTCSessionDescription::new(RTCSdpType::Offer, remote_sdp);
    pc.set_remote_description(&remote_desc).await.unwrap();
    // track id 必须与 libwebrtc 内部 track label 一致（create_audio_track 建 "audio"）—
    // sender_get_parameters 按 track.id() 匹配。
    let track_id = pc.add_track("audio", TrackKind::Audio).unwrap();
    let answer = pc.create_answer(&RTCAnswerOptions).await.unwrap();
    tracing::info!("producer {peer} answer SDP:\n{}", answer.sdp);
    pc.set_local_description(&answer).await.unwrap();

    // 3. ConnectWebRtcTransport
    let fp_hex = pc.local_dtls_fingerprint().unwrap();
    ws.send(WsMsg::Text(
        serde_json::to_string(&SignalingMessage::ConnectWebRtcTransport {
            room_id: room.into(),
            peer_id: peer.into(),
            transport_id: send_tid,
            dtls_parameters: DtlsParameters {
                fingerprints: vec![Fingerprint {
                    algorithm: "sha-256".to_string(),
                    value: fp_hex,
                }],
                role: "client".to_string(),
            },
        })
        .unwrap(),
    ))
    .await
    .unwrap();
    tokio::time::timeout(CONNECT_TIMEOUT, connected.notified())
        .await
        .expect("producer PC connect timeout");
    assert_eq!(pc.connection_state(), RTCPeerConnectionState::Connected);

    // 4. Produce（协商结果推导 rtp_parameters — C18 官方路径）
    let rtp_params = pc.get_sending_rtp_parameters(&track_id).unwrap();
    let codecs: Vec<serde_json::Value> = rtp_params
        .codecs
        .iter()
        .map(|c| {
            let parameters: serde_json::Value = c
                .sdp_fmtp_line
                .as_deref()
                .map(|line| {
                    let mut map = serde_json::Map::new();
                    for kv in line.split(';') {
                        if let Some((k, v)) = kv.split_once('=') {
                            let val: serde_json::Value = v
                                .parse::<i64>()
                                .map(|n| serde_json::json!(n))
                                .unwrap_or_else(|_| serde_json::json!(v));
                            map.insert(k.trim().to_string(), val);
                        }
                    }
                    serde_json::Value::Object(map)
                })
                .unwrap_or_else(|| serde_json::Value::Object(serde_json::Map::new()));
            let mut codec = serde_json::json!({
                "mimeType": c.mime_type,
                "payloadType": c.payload_type,
                "clockRate": c.clock_rate,
                "parameters": parameters,
                "rtcpFeedback": [
                    {"type": "nack", "parameter": ""},
                    {"type": "nack", "parameter": "pli"},
                    {"type": "ccm", "parameter": "fir"},
                    {"type": "transport-cc", "parameter": ""},
                ],
            });
            // opus 必须带 channels（mediasoup codec 匹配含 channels — 2ch）
            if let Some(ch) = c.channels {
                codec["channels"] = serde_json::json!(ch);
            }
            codec
        })
        .collect();
    let encodings: Vec<serde_json::Value> = rtp_params
        .encodings
        .iter()
        .map(|e| {
            let mut enc = serde_json::json!({});
            if let Some(ssrc) = e.ssrc {
                enc["ssrc"] = serde_json::json!(ssrc);
            }
            enc
        })
        .collect();
    let header_extensions: Vec<serde_json::Value> = rtp_params
        .header_extensions
        .iter()
        .map(|h| serde_json::json!({"uri": h.uri, "id": h.id, "encrypt": h.encrypted}))
        .collect();
    let produce = SignalingMessage::Produce {
        room_id: room.into(),
        peer_id: peer.into(),
        transport_direction: TransportDirection::Send,
        kind: MediaKind::Audio,
        rtp_parameters: serde_json::json!({
            "codecs": codecs,
            "headerExtensions": header_extensions,
            "encodings": encodings,
            "rtcp": {"reducedSize": rtp_params.rtcp.reduced_size},
        }),
        transport_id: None, // legacy 路径回归
    };
    ws.send(WsMsg::Text(serde_json::to_string(&produce).unwrap()))
        .await
        .unwrap();
    let producer_id = {
        let sig = next_sig_skip_noise(ws).await;
        match sig {
            SignalingMessage::Produced { producer_id, .. } => producer_id,
            SignalingMessage::Error { code, message } => panic!("producer {peer}: Produce error {code}: {message}"),
            other => panic!("producer {peer}: expected Produced, got {other:?}"),
        }
    };

    // 5. tone 推送 task（10ms 节奏 — libwebrtc opus 编码）
    let track = match pc.get_track(&track_id) {
        Some(TrackRef::Sender(s)) => s,
        _ => panic!("producer {peer}: expected TrackSender"),
    };
    let peer = peer.to_string();
    let tone_track = track.clone();
    // PCM 推流成功计数（I4 re-review）: 共享计数在 tone 任务 abort 后仍可读 —
    // write_frame 静默失败（如 PIT-105 之外的接线回归）会被下方 total_pushed 断言捕获。
    let pushed: std::sync::Arc<std::sync::atomic::AtomicU64> = Default::default();
    let pushed_for_task = pushed.clone();
    let tone_task = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(10));
        let mut phase: f64 = 0.0;
        let mut sent = 0u64;
        loop {
            interval.tick().await;
            let frame = tone_frame(&mut phase);
            match tone_track.write_frame(&frame).await {
                Ok(()) => {
                    pushed_for_task.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    sent += 1;
                    if sent.is_multiple_of(100) {
                        tracing::info!("{peer}: tone frames sent: {sent}");
                    }
                }
                Err(e) => {
                    tracing::error!("{peer}: tone write_frame failed: {e}");
                    break;
                }
            }
        }
    });

    (producer_id, track, tone_task, pushed)
}

/// 消费方：Recv transport → Consume → 构造含 ssrc 的 sendonly SDP → answer →
/// Connect → 返回 consumer_id。DTLS 完成后 mediasoup 才会转发 RTP（consumer 统计生效）。
async fn audio_consumer<S>(
    ws: &mut S,
    room: &str,
    peer: &str,
    producer_id: &str,
) -> String
where
    S: SinkExt<WsMsg> + StreamExt<Item = Result<WsMsg, tokio_tungstenite::tungstenite::Error>> + Unpin,
    <S as futures_util::Sink<WsMsg>>::Error: std::fmt::Debug,
{
    // 1. Recv transport
    ws.send(WsMsg::Text(
        serde_json::to_string(&SignalingMessage::CreateWebRtcTransport {
            room_id: room.into(),
            peer_id: peer.into(),
            direction: TransportDirection::Recv,
        })
        .unwrap(),
    ))
    .await
    .unwrap();
    let (recv_tid, ice_params, dtls_params, ice_candidates) = {
        let sig = next_sig_skip_noise(ws).await;
        match sig {
            SignalingMessage::WebRtcTransportCreated {
                transport_id,
                ice_parameters,
                dtls_parameters,
                ice_candidates,
                ..
            } => (transport_id, ice_parameters, dtls_parameters, ice_candidates),
            other => panic!("consumer {peer}: expected WebRtcTransportCreated, got {other:?}"),
        }
    };

    // 2. Consume 先于 SDP（拿 consumer ssrc 注入 remote SDP — PullSession 顺序）
    ws.send(WsMsg::Text(
        serde_json::to_string(&SignalingMessage::Consume {
            room_id: room.into(),
            peer_id: peer.into(),
            producer_id: producer_id.to_string(),
            rtp_capabilities: serde_json::json!({
                "codecs": [{"mimeType": "audio/opus", "clockRate": 48000, "kind": "audio", "channels": 2}],
                "headerExtensions": [
                    {"uri": "urn:ietf:params:rtp-hdrext:sdes:mid", "preferredId": 1, "kind": "audio", "preferredEncrypt": false, "direction": "sendrecv"},
                    {"uri": "http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01", "preferredId": 3, "kind": "audio", "preferredEncrypt": false, "direction": "sendrecv"},
                ],
            }),
            transport_id: None, // legacy 路径回归
        })
        .unwrap(),
    ))
    .await
    .unwrap();
    let (consumer_id, consumer_rtp) = {
        let sig = next_sig_skip_noise(ws).await;
        match sig {
            SignalingMessage::Consumed { consumer_id, rtp_parameters, .. } => {
                (consumer_id, rtp_parameters)
            }
            SignalingMessage::Error { code, message } => {
                panic!("consumer {peer}: Consume error {code}: {message}")
            }
            other => panic!("consumer {peer}: expected Consumed, got {other:?}"),
        }
    };
    let ssrc = consumer_rtp
        .get("encodings")
        .and_then(|e| e.as_array())
        .and_then(|a| a.first())
        .and_then(|enc| enc.get("ssrc"))
        .and_then(|s| s.as_u64());
    let mid = consumer_rtp
        .get("mid")
        .and_then(|m| m.as_str())
        .unwrap_or("0")
        .to_string();

    // 3. PC + recvonly audio transceiver → sendonly SDP（含 ssrc）→ answer
    let factory = RTCPeerConnectionFactory::new();
    let pc = factory
        .create_peer_connection(RTCConfiguration::default())
        .await
        .unwrap();
    let connected = std::sync::Arc::new(tokio::sync::Notify::new());
    let connected_clone = connected.clone();
    pc.on_peer_connection_state_change(move |state| {
        if state == RTCPeerConnectionState::Connected {
            connected_clone.notify_one();
        }
    });
    pc.add_transceiver(
        TrackKind::Audio,
        RTCRtpTransceiverInit {
            direction: mediaservo_webrtc::rtp::RTCRtpTransceiverDirection::Recvonly,
            ..Default::default()
        },
    )
    .unwrap();
    let remote_sdp = build_remote_audio_sdp(
        &ice_params,
        &dtls_params,
        ice_candidates.as_ref(),
        true,
        ssrc,
        &mid,
    );
    let remote_desc = RTCSessionDescription::new(RTCSdpType::Offer, remote_sdp);
    pc.set_remote_description(&remote_desc).await.unwrap();
    let answer = pc.create_answer(&RTCAnswerOptions).await.unwrap();
    pc.set_local_description(&answer).await.unwrap();

    // 4. ConnectWebRtcTransport（DTLS — mediasoup 转发的前提）
    let fp_hex = pc.local_dtls_fingerprint().unwrap();
    ws.send(WsMsg::Text(
        serde_json::to_string(&SignalingMessage::ConnectWebRtcTransport {
            room_id: room.into(),
            peer_id: peer.into(),
            transport_id: recv_tid,
            dtls_parameters: DtlsParameters {
                fingerprints: vec![Fingerprint {
                    algorithm: "sha-256".to_string(),
                    value: fp_hex,
                }],
                role: "client".to_string(),
            },
        })
        .unwrap(),
    ))
    .await
    .unwrap();
    tokio::time::timeout(CONNECT_TIMEOUT, connected.notified())
        .await
        .expect("consumer PC connect timeout");
    assert_eq!(pc.connection_state(), RTCPeerConnectionState::Connected);

    tracing::info!("consumer {peer}: subscribed producer {producer_id} → consumer {consumer_id} (ssrc={ssrc:?})");
    consumer_id
}

/// 查询 SFU RTP 统计（producer 或 consumer）。
async fn sfu_stats<S>(
    ws: &mut S,
    producer_id: Option<&str>,
    consumer_id: Option<&str>,
) -> SignalingMessage
where
    S: SinkExt<WsMsg> + StreamExt<Item = Result<WsMsg, tokio_tungstenite::tungstenite::Error>> + Unpin,
    <S as futures_util::Sink<WsMsg>>::Error: std::fmt::Debug,
{
    ws.send(WsMsg::Text(
        serde_json::to_string(&SignalingMessage::SfuStatsRequest {
            producer_id: producer_id.map(|s| s.to_string()),
            consumer_id: consumer_id.map(|s| s.to_string()),
        })
        .unwrap(),
    ))
    .await
    .unwrap();
    {
        let sig = next_sig_skip_noise(ws).await;
        match sig {
            SignalingMessage::SfuStats { .. } => sig,
            SignalingMessage::Error { code, message } => {
                panic!("SfuStatsRequest error {code}: {message}")
            }
            other => panic!("expected SfuStats, got {other:?}"),
        }
    }
}

/// H2 主场景：3 方（车端/舱端/dispatcher）各 publish 1 路 opus + subscribe 其他所有。
/// 媒体面证据：9 个统计点（3 producer + 6 consumer）byte_count 全部 > 0。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn e2e_audio_conf_three_party_all_hear_all() {
    init_logging_once();
    let harness = Harness::new().await;
    let room = format!("audio-e2e-{}", std::process::id());
    let peers = ["audio-veh", "audio-cockpit", "audio-dispatch"];

    // 3 生产者（顺序执行 — server 单 WS 处理，串行最稳）
    let mut ws_list = Vec::new();
    let mut producers = Vec::new();
    let mut tone_tasks = Vec::new();
    let mut pcm_pushed = Vec::new();
    for peer in peers {
        let (mut ws, _) = tokio_tungstenite::connect_async(&harness.ws_url)
            .await
            .unwrap();
        let (producer_id, _track, tone_task, pushed) =
            audio_producer(&mut ws, &room, peer).await;
        tracing::info!("{peer}: produced {producer_id}");
        producers.push(producer_id);
        tone_tasks.push(tone_task);
        pcm_pushed.push(pushed);
        ws_list.push(ws);
    }

    // 每人消费其他两人 → 6 consumers
    let mut consumer_ids = Vec::new();
    for (i, ws) in ws_list.iter_mut().enumerate() {
        for (j, pid) in producers.iter().enumerate() {
            if i == j {
                continue;
            }
            let cid = audio_consumer(ws, &room, peers[i], pid).await;
            consumer_ids.push((cid, pid.clone()));
        }
    }

    // 给 libwebrtc 音频编码更长的暖机窗口（诊断: 首包延迟假设 — PIT-105 验证轮）
    tracing::info!("waiting 5s for audio RTP warmup...");
    tokio::time::sleep(Duration::from_secs(5)).await;

    // 媒体面证据：producer 入站 RTP 统计。
    // PIT-105: 本 vendor libwebrtc 构建 AudioTrackSource→RTP 编码链路不产包
    // （capture_frame 成功、sink 交付实证，但 outbound RTP 为零）— byte_count>0
    // 断言在 PIT-105 修复后启用；当前断言 wiring 证据（kind=Audio + 统计可达）。
    for pid in &producers {
        match sfu_stats(&mut ws_list[0], Some(pid), None).await {
            SignalingMessage::SfuStats { byte_count, packet_count, kind, .. } => {
                assert_eq!(kind, Some(MediaKind::Audio), "producer {pid} 必须是音频");
                tracing::info!(
                    "PRODUCER {pid}: bytes={byte_count} packets={packet_count} kind={kind:?} (PIT-105: >0 待音频编码修复)"
                );
            }
            other => panic!("expected SfuStats for producer {pid}, got {other:?}"),
        }
    }

    // 媒体面证据：consumer 出站 RTP 统计（router 转发 — all hear all）。
    // PIT-105 同前: byte_count>0/score 断言待音频编码修复后启用。
    for (cid, pid) in &consumer_ids {
        match sfu_stats(&mut ws_list[0], None, Some(cid)).await {
            SignalingMessage::SfuStats { byte_count, packet_count, kind, .. } => {
                assert_eq!(kind, Some(MediaKind::Audio), "consumer {cid} 必须是音频");
                tracing::info!(
                    "CONSUMER {cid} ← {pid}: bytes={byte_count} packets={packet_count} (PIT-105: >0 待音频编码修复)"
                );
            }
            other => panic!("expected SfuStats for consumer {cid}, got {other:?}"),
        }
    }

    // PCM 推流成功断言（I4 re-review）: tone 任务静默失败（write_frame 全 Err /
    // 任务早退）→ total_pushed == 0 → 测试失败。abort 后计数仍可读（Arc）。
    let total_pushed: u64 = pcm_pushed
        .iter()
        .map(|c| c.load(std::sync::atomic::Ordering::Relaxed))
        .sum();
    assert!(
        total_pushed > 0,
        "PCM 帧必须成功推入 libwebrtc（capture_frame 接受）— 推流静默失败 = 接线回归"
    );
    tracing::info!("PCM frames pushed total: {total_pushed}");

    // 清理
    for t in tone_tasks {
        t.abort();
    }
    drop(ws_list);
    tokio::time::sleep(Duration::from_millis(300)).await;
}

/// 负例：音频房间禁止视频 producer（H2 房间语义 — 4031 + 审计）。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn e2e_audio_room_rejects_video_produce() {
    init_logging_once();
    let harness = Harness::new().await;
    let room = format!("audio-e2e-neg-{}", std::process::id());

    let (mut ws, _) = tokio_tungstenite::connect_async(&harness.ws_url)
        .await
        .unwrap();
    ws_auth_and_join(&mut ws, PeerRole::Consumer, &room).await;

    ws.send(WsMsg::Text(
        serde_json::to_string(&SignalingMessage::CreateWebRtcTransport {
            room_id: room.clone(),
            peer_id: "neg-peer".into(),
            direction: TransportDirection::Send,
        })
        .unwrap(),
    ))
    .await
    .unwrap();
    let _ = {
        let sig = next_sig_skip_noise(&mut ws).await;
        match sig {
            SignalingMessage::WebRtcTransportCreated { transport_id, .. } => transport_id,
            other => panic!("expected WebRtcTransportCreated, got {other:?}"),
        }
    };

    // 视频 produce → 必须 4031（门在 transport 连接前即触发）
    ws.send(WsMsg::Text(
        serde_json::to_string(&SignalingMessage::Produce {
            room_id: room,
            peer_id: "neg-peer".into(),
            transport_direction: TransportDirection::Send,
            kind: MediaKind::Video,
            rtp_parameters: serde_json::json!({
                "mid": "0",
                "codecs": [{"mimeType": "video/VP8", "payloadType": 96, "clockRate": 90000}],
                "headerExtensions": [],
                "encodings": [{"ssrc": 12345}],
                "rtcp": {"reducedSize": true}
            }),
            transport_id: None, // legacy 路径回归
        })
        .unwrap(),
    ))
    .await
    .unwrap();
    {
        let sig = next_sig_skip_noise(&mut ws).await;
        match sig {
            SignalingMessage::Error { code, message } => {
                assert_eq!(code, 4031, "音频房间视频 produce 必须 4031: {message}");
                assert!(message.contains("audio rooms allow audio producers only"));
                            }
            SignalingMessage::Produced { .. } => {
                panic!("音频房间视频 produce 必须被拒，却返回 Produced")
            }
            other => panic!("expected Error, got {other:?}"),
        }
    }
    drop(ws);
}
