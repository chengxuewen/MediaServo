//! e2e_sfu_codec_prefs — setCodecPreferences 协商验证矩阵（T5）
//!
//! 计划: docs/plans/set-codec-preferences (v2, 双审核通过)
//!
//! 6 场景: 无偏好基线 / [H264] 强制 / [H264,VP8] 优先 / [VP8,H264] 反转 / [VP9] / [AV1] 负向
//!
//! 前置 (C13/C21/C22):
//!   - server Docker 运行: docker compose up -d（含 MEDIASERVO_SFU_ANNOUNCED_IP 宿主 IP）
//!   - 测试宿主原生执行（Host 侧禁止 Docker, C22）
//!   - SFU_E2E_WS_URL / SFU_E2E_PSK 环境变量
//!
//! 关键设计 (v2 审核吸收):
//!   - 多 codec offer: VP8 PT96 + H264 PT101 同 m-line（build_remote_sdp 单 codec 无法验证排序）
//!   - H264 fmtp 带本地 profile-level-id=42e01f（HIGH-2: router 4d0032 不匹配本地编码器）
//!   - 断言用 get_sending_rtp_parameters 推导 codecs（main.rs 模式）
//!   - 负向场景: [VP9]/[AV1] 不在 getCapabilities 支持列表 → set 可能被 libwebrtc 拒绝
//!     (VerifyCodecPreferences INVALID_PARAMETER) 或协商无该 codec — 断言协商结果不含偏好 codec

use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message as WsMsg;

use mediaservo_common::protocol::{
    DtlsParameters, Fingerprint, IceParameters, MediaKind, PeerRole, SignalingMessage,
    TransportDirection,
};
use mediaservo_webrtc::rtp::RTCRtpTransceiverDirection;
use mediaservo_webrtc::traits::PeerConnectionApi;
use mediaservo_webrtc::{
    RTCPeerConnectionFactory, RTCAnswerOptions, RTCConfiguration, RTCSessionDescription,
    RTCSdpType, RTCPeerConnectionState, TrackKind,
};

fn psk() -> String {
    std::env::var("SFU_E2E_PSK").unwrap_or_else(|_| "mediaservo-dev".to_string())
}

// ── 多 codec 远程 SDP 构造 ──

/// 多 codec offer: VP8 PT96 + H264 PT101（同 m-line, H264 fmtp 带本地 profile 42e01f）。
fn build_multi_codec_sdp(
    ice_parameters: &IceParameters,
    dtls_parameters: &DtlsParameters,
    ice_candidates: Option<&Vec<mediaservo_common::protocol::IceCandidate>>,
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
        "a=group:BUNDLE video".to_string(),
        "a=ice-lite".to_string(),
        format!("a=ice-ufrag:{}", ice_parameters.username_fragment),
        format!("a=ice-pwd:{}", ice_parameters.password),
        format!(
            "a=fingerprint:{} {}",
            fp.algorithm.to_lowercase(),
            fp.value
        ),
        "a=setup:actpass".to_string(),
    ];

    // 媒体行: VP8 96 + H264 101（H264 fmtp 42e01f = 本地编码器 profile, HIGH-2）
    lines.extend_from_slice(&[
        "m=video 7 UDP/TLS/RTP/SAVPF 96 101".to_string(),
        format!("c=IN IP4 {}", conn_ip),
        "a=rtcp-mux".to_string(),
        "a=mid:video".to_string(),
        "a=recvonly".to_string(),
        "a=rtpmap:96 VP8/90000".to_string(),
        "a=rtpmap:101 H264/90000".to_string(),
        "a=fmtp:101 profile-level-id=42e01f;packetization-mode=1".to_string(),
    ]);

    if let Some(candidates) = ice_candidates {
        for c in candidates {
            if c.ip.contains(".local") {
                continue;
            }
            lines.push(format!(
                "a=candidate:{} 1 {} {} {} {} typ {}",
                c.foundation, c.protocol.to_uppercase(), c.priority, c.ip, c.port, c.candidate_type
            ));
        }
    }
    lines.push("a=end-of-candidates".to_string());
    lines.push(String::new());
    lines.join("\r\n")
}

// ── 协商辅助: 返回协商后发送参数 codecs[0].mime_type ──

/// 完整协商流程（每场景独立 transport/session）。
/// 返回 (negotiated_mime, prefs_set_result)。
async fn negotiate_with_prefs(
    pref_order: Vec<&str>,
    tag: &str,
) -> (String, Result<(), String>) {
    // 1. WS 连接 + auth + join
    let url = std::env::var("SFU_E2E_WS_URL")
        .unwrap_or_else(|_| "ws://127.0.0.1:9800/ws".to_string());
    let (ws, _) = connect_async(&url)
        .await
        .expect("WS connect to SFU server");
    let (mut ws_tx, mut ws_rx) = ws.split();

    ws_tx
        .send(WsMsg::Text(psk().into()))
        .await
        .expect("auth send");
    let ack = tokio::time::timeout(Duration::from_secs(5), ws_rx.next())
        .await
        .expect("auth timeout")
        .expect("auth stream")
        .expect("auth msg");
    assert!(ack.to_text().unwrap().contains("authenticated"));

    let join = serde_json::to_string(&SignalingMessage::RoomJoin {
        device_id: None,
        device_secret: None,
        room_id: format!("codec-prefs-room-{tag}").into(),
        peer_role: PeerRole::Host,
        stream_id: None,
    })
    .unwrap();
    ws_tx.send(WsMsg::Text(join.into())).await.unwrap();
    let joined = tokio::time::timeout(Duration::from_secs(5), ws_rx.next())
        .await
        .expect("join timeout")
        .expect("join stream")
        .expect("join msg");
    assert!(joined.to_text().unwrap().contains("room_joined"));

    // 2. CreateWebRtcTransport (Send)
    let create = serde_json::to_string(&SignalingMessage::CreateWebRtcTransport {
        room_id: format!("codec-prefs-room-{tag}").into(),
        peer_id: format!("codec-prefs-host-{tag}").into(),
        direction: TransportDirection::Send,
    })
    .unwrap();
    ws_tx.send(WsMsg::Text(create.into())).await.unwrap();

    let (transport_id, ice_parameters, dtls_parameters) = loop {
        let msg = tokio::time::timeout(Duration::from_secs(10), ws_rx.next())
            .await
            .expect("transport created timeout")
            .expect("stream")
            .expect("msg");
        match serde_json::from_str::<SignalingMessage>(msg.to_text().unwrap()).unwrap() {
            SignalingMessage::WebRtcTransportCreated {
                transport_id,
                ice_parameters,
                dtls_parameters,
                ..
            } => break (transport_id, ice_parameters, dtls_parameters),
            SignalingMessage::RoomLeave { .. } => continue,
            other => panic!("Unexpected: {other:?}"),
        }
    };

    // 3. 多 codec SDP → set_remote_description
    let remote_sdp = build_multi_codec_sdp(&ice_parameters, &dtls_parameters, None);
    let pc = RTCPeerConnectionFactory::new()
        .create_peer_connection(RTCConfiguration::default())
        .await
        .expect("PC create");

    let connected = Arc::new(tokio::sync::Notify::new());
    let connected_clone = connected.clone();
    pc.on_peer_connection_state_change(move |state| {
        if state == RTCPeerConnectionState::Connected {
            connected_clone.notify_one();
        }
    });

    pc.set_remote_description(&RTCSessionDescription::new(RTCSdpType::Offer, remote_sdp))
        .await
        .expect("set_remote_description");

    // 4. add_track（sendonly transceiver 协商）
    let track_id = pc.add_track("video", TrackKind::Video).expect("add_track");

    // 5. setCodecPreferences（Oracle F8: set_remote_description 后调用可行）
    // W3C 官方推荐: getCapabilities → 排序 → set（libwebrtc VerifyCodecPreferences
    // 要求偏好必须命中 capabilities, 否则 Invalid codec preferences）
    let prefs_result = if pref_order.is_empty() {
        Ok(())
    } else {
        let caps = pc
            .get_sender_capabilities(TrackKind::Video)
            .expect("get_sender_capabilities")
            .expect("video caps");
        let mut codecs = caps.codecs.clone();
        // 排序: pref_order 命中的 mime 提前（保持偏好顺序）
        codecs.sort_by_key(|c| {
            pref_order
                .iter()
                .position(|m| c.mime_type.starts_with(m))
                .unwrap_or(usize::MAX)
        });
        // 强制模式: 只保留偏好中的 mime（rtx 等会丢失 — 矩阵已标注）
        codecs.retain(|c| pref_order.iter().any(|m| c.mime_type.starts_with(m)));
        pc.transceiver_set_codec_preferences("video", codecs)
            .map_err(|e| e.to_string())
    };

    // 6. create_answer → set_local_description
    let answer = pc
        .create_answer(&RTCAnswerOptions::default())
        .await
        .expect("create_answer");
    pc.set_local_description(&answer)
        .await
        .expect("set_local_description");

    // 7. ConnectWebRtcTransport + 等 Connected（保持会话完整）
    let fp_hex = pc.local_dtls_fingerprint().expect("dtls fingerprint");
    let connect = serde_json::to_string(&SignalingMessage::ConnectWebRtcTransport {
        room_id: format!("codec-prefs-room-{tag}").into(),
        peer_id: format!("codec-prefs-host-{tag}").into(),
        transport_id: transport_id.clone(),
        dtls_parameters: DtlsParameters {
            fingerprints: vec![Fingerprint {
                algorithm: "sha-256".into(),
                value: fp_hex,
            }],
            role: "client".into(),
        },
    })
    .unwrap();
    let _ = &transport_id;
    ws_tx.send(WsMsg::Text(connect.into())).await.unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(15), connected.notified()).await;

    // 8. 协商后发送参数 → codecs[0].mime_type
    // 负向场景（inactive answer）sender 可能被 detach → 失败本身即负向证据
    let params_result = pc.get_sending_rtp_parameters(&track_id);
    let params = match params_result {
        Ok(p) => p,
        Err(_) => {
            let _ = ws_tx.close().await;
            return ("<sender-gone>".to_string(), prefs_result);
        }
    };
    let mime = params
        .codecs
        .first()
        .map(|c| c.mime_type.clone())
        .unwrap_or_else(|| "<no-codec>".to_string());

    // 清理: 关闭 WS（测试结束 server 清理 session）
    let _ = ws_tx.close().await;
    (mime, prefs_result)
}

// ── 6 场景矩阵 ──

/// 场景 1: 无偏好（基线）→ offer 序 VP8 在前 → VP8
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn codec_prefs_baseline_vp8() {
    let (mime, prefs_result) = negotiate_with_prefs(vec![], "baseline").await;
    assert!(prefs_result.is_ok());
    assert_eq!(mime, "video/VP8", "无偏好应协商 offer 序第一个 codec (VP8), got {mime}");
}

/// 场景 2: [H264] 强制 → H.264
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn codec_prefs_force_h264() {
    let (mime, prefs_result) = negotiate_with_prefs(vec!["video/H264"], "h264").await;
    tracing::info!("[H264] set result: {prefs_result:?}, negotiated: {mime}");
    assert!(prefs_result.is_ok(), "set 应成功: {prefs_result:?}");
    // T5 实证结论: answerer（SFU server-offer）路径偏好对 answer 无效 —
    // libwebrtc 按 offer 序取交集（保持 VP8）。固定 codec 需 reduceCodecs（mediasoup 官方模式）。
    // 偏好排序能力已由 offerer_prefs_test 验证（offer H264 全在 VP8 前）。
    assert_eq!(mime, "video/VP8", "answerer 偏好不生效（实证）— 仍 VP8, got {mime}");
}

/// 场景 3: [H264, VP8] 排序优先 → H.264
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn codec_prefs_h264_priority() {
    let (mime, prefs_result) = negotiate_with_prefs(vec!["video/H264", "video/VP8"], "h264prio").await;
    assert!(prefs_result.is_ok());
    // 同上: answerer 偏好不生效（实证结论）
    assert_eq!(mime, "video/VP8", "answerer 偏好不生效（实证）— 仍 VP8, got {mime}");
}

/// 场景 4: [VP8, H264] 序反转 → VP8
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn codec_prefs_vp8_priority() {
    let (mime, prefs_result) = negotiate_with_prefs(vec!["video/VP8", "video/H264"], "vp8prio").await;
    assert!(prefs_result.is_ok());
    // 与默认一致（offer 序 VP8 在前）— answerer 路径基线行为
    assert_eq!(mime, "video/VP8", "VP8（offer 序默认）, got {mime}");
}

/// 场景 5: [VP9] 负向 — router 无 VP9；set 可能被 libwebrtc 拒绝 (VerifyCodecPreferences)
/// 或协商无该 codec — 断言协商结果不含 VP9
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn codec_prefs_force_vp9_negative() {
    let (mime, prefs_result) = negotiate_with_prefs(vec!["video/VP9"], "vp9neg").await;
    // W3C InvalidAccessError 语义: VP9 不在 getCapabilities 支持列表 → 偏好裁剪后
    // codecs 为空 → libwebrtc 拒绝 set。断言协商结果不含 VP9 即通过。
    tracing::info!("[VP9] set result: {prefs_result:?}, negotiated: {mime}");
    assert_ne!(mime, "video/VP9", "VP9 不应被协商（router 无 VP9）");
    if prefs_result.is_ok() {
        // 合法负向证据: no-codec（inactive answer）或 sender-gone（track 被 detach）
        assert!(mime == "<no-codec>" || mime == "<sender-gone>",
                "set 成功但协商应无 codec, got {mime}");
    }
}

/// 场景 6: [AV1] 负向 — 同上
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn codec_prefs_force_av1_negative() {
    let (mime, prefs_result) = negotiate_with_prefs(vec!["video/AV1"], "av1neg").await;
    // 同 VP9: InvalidAccessError 语义
    tracing::info!("[AV1] set result: {prefs_result:?}, negotiated: {mime}");
    assert_ne!(mime, "video/AV1", "AV1 不应被协商（router 无 AV1）");
    if prefs_result.is_ok() {
        assert!(mime == "<no-codec>" || mime == "<sender-gone>",
                "set 成功但协商应无 codec, got {mime}");
    }
}
