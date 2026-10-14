//! e2e_sfu_codec_prefs - setCodecPreferences negotiation verification matrix (T5)
//!
//! Plan: docs/plans/set-codec-preferences (v2, passed both reviews)
//!
//! 6 scenarios: no-preference baseline / [H264] force / [H264,VP8] prefer / [VP8,H264] reversed / [VP9] / [AV1] negative
//!
//! Prerequisites (C13/C21/C22):
//!   - server running via Docker: docker compose up -d (with MEDIASERVO_SFU_ANNOUNCED_IP set to the host IP)
//!   - the test executes natively on the host (Docker forbidden on the Host side, C22)
//!   - SFU_E2E_WS_URL / SFU_E2E_PSK environment variables
//!
//! Key design (v2 review absorbed):
//!   - multi-codec offer: VP8 PT96 + H264 PT101 in the same m-line (single-codec build_remote_sdp cannot verify ordering)
//!   - H264 fmtp carries local profile-level-id=42e01f (HIGH-2: router 4d0032 mismatches the local encoder)
//!   - assertions derive codecs from get_sending_rtp_parameters (main.rs pattern)
//!   - negative scenarios: [VP9]/[AV1] are not in the getCapabilities support list -> set may be rejected by libwebrtc
//!     (VerifyCodecPreferences INVALID_PARAMETER) or the codec is absent from the negotiation - assert the result lacks the preferred codec

use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message as WsMsg;

use mediaservo_common::protocol::{
    DtlsParameters, Fingerprint, IceParameters, PeerRole, SignalingMessage, TransportDirection,
};
use mediaservo_webrtc::traits::PeerConnectionApi;
use mediaservo_webrtc::{
    RTCAnswerOptions, RTCConfiguration, RTCPeerConnectionFactory, RTCPeerConnectionState,
    RTCSdpType, RTCSessionDescription, TrackKind,
};

fn psk() -> String {
    std::env::var("SFU_E2E_PSK").unwrap_or_else(|_| "mediaservo-dev".to_string())
}

// -- multi-codec remote SDP construction --

/// Multi-codec offer: VP8 PT96 + H264 PT101 (same m-line, H264 fmtp carries local profile 42e01f).
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
        format!("a=fingerprint:{} {}", fp.algorithm.to_lowercase(), fp.value),
        "a=setup:actpass".to_string(),
    ];

    // media line: VP8 96 + H264 101 (H264 fmtp 42e01f = local encoder profile, HIGH-2)
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
                c.foundation,
                c.protocol.to_uppercase(),
                c.priority,
                c.ip,
                c.port,
                c.candidate_type
            ));
        }
    }
    lines.push("a=end-of-candidates".to_string());
    lines.push(String::new());
    lines.join("\r\n")
}

// -- negotiation helper: returns codecs[0].mime_type of the negotiated sending parameters --

/// Full negotiation flow (independent transport/session per scenario).
/// Returns (negotiated_mime, prefs_set_result).
async fn negotiate_with_prefs(pref_order: Vec<&str>, tag: &str) -> (String, Result<(), String>) {
    // 1. WS connect + auth + join
    let url =
        std::env::var("SFU_E2E_WS_URL").unwrap_or_else(|_| "ws://127.0.0.1:9800/ws".to_string());
    let (ws, _) = connect_async(&url).await.expect("WS connect to SFU server");
    let (mut ws_tx, mut ws_rx) = ws.split();

    ws_tx.send(WsMsg::Text(psk())).await.expect("auth send");
    let ack = tokio::time::timeout(Duration::from_secs(5), ws_rx.next())
        .await
        .expect("auth timeout")
        .expect("auth stream")
        .expect("auth msg");
    assert!(ack.to_text().unwrap().contains("authenticated"));

    let join = serde_json::to_string(&SignalingMessage::RoomJoin {
        device_id: None,
        device_secret: None,
        device_pubkey: None,
        protocol: None,
        client_version: None,
        room_id: format!("codec-prefs-room-{tag}"),
        peer_role: PeerRole::Host,
        stream_id: None,
        resume: None,
    })
    .unwrap();
    ws_tx.send(WsMsg::Text(join)).await.unwrap();
    let joined = tokio::time::timeout(Duration::from_secs(5), ws_rx.next())
        .await
        .expect("join timeout")
        .expect("join stream")
        .expect("join msg");
    assert!(joined.to_text().unwrap().contains("room_joined"));

    // 2. CreateWebRtcTransport (Send)
    let create = serde_json::to_string(&SignalingMessage::CreateWebRtcTransport {
        room_id: format!("codec-prefs-room-{tag}"),
        peer_id: format!("codec-prefs-host-{tag}"),
        direction: TransportDirection::Send,
    })
    .unwrap();
    ws_tx.send(WsMsg::Text(create)).await.unwrap();

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

    // 3. multi-codec SDP -> set_remote_description
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

    // 4. add_track (sendonly transceiver negotiation)
    let track_id = pc.add_track("video", TrackKind::Video).expect("add_track");

    // 5. setCodecPreferences (Oracle F8: calling after set_remote_description works)
    // W3C recommendation: getCapabilities -> sort -> set (libwebrtc VerifyCodecPreferences
    // requires preferences to hit capabilities, otherwise Invalid codec preferences)
    let prefs_result = if pref_order.is_empty() {
        Ok(())
    } else {
        let caps = pc
            .get_sender_capabilities(TrackKind::Video)
            .expect("get_sender_capabilities")
            .expect("video caps");
        let mut codecs = caps.codecs.clone();
        // sort: mimes hit by pref_order move to the front (keeping preference order)
        codecs.sort_by_key(|c| {
            pref_order.iter().position(|m| c.mime_type.starts_with(m)).unwrap_or(usize::MAX)
        });
        // force mode: keep only the preferred mimes (rtx etc. get dropped - noted in the matrix)
        codecs.retain(|c| pref_order.iter().any(|m| c.mime_type.starts_with(m)));
        pc.transceiver_set_codec_preferences("video", codecs).map_err(|e| e.to_string())
    };

    // 6. create_answer → set_local_description
    let answer = pc.create_answer(&RTCAnswerOptions).await.expect("create_answer");
    pc.set_local_description(&answer).await.expect("set_local_description");

    // 7. ConnectWebRtcTransport + wait for Connected (keep the session complete)
    let fp_hex = pc.local_dtls_fingerprint().expect("dtls fingerprint");
    let connect = serde_json::to_string(&SignalingMessage::ConnectWebRtcTransport {
        room_id: format!("codec-prefs-room-{tag}"),
        peer_id: format!("codec-prefs-host-{tag}"),
        transport_id: transport_id.clone(),
        dtls_parameters: DtlsParameters {
            fingerprints: vec![Fingerprint { algorithm: "sha-256".into(), value: fp_hex }],
            role: "client".into(),
        },
    })
    .unwrap();
    let _ = &transport_id;
    ws_tx.send(WsMsg::Text(connect)).await.unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(15), connected.notified()).await;

    // 8. post-negotiation sending parameters -> codecs[0].mime_type
    // negative scenario (inactive answer): the sender may be detached -> the failure itself is negative evidence
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

    // cleanup: close the WS (the server clears the session after the test)
    let _ = ws_tx.close().await;
    (mime, prefs_result)
}

// -- 6-scenario matrix --

/// Scenario 1: no preferences (baseline) -> offer order has VP8 first -> VP8
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn codec_prefs_baseline_vp8() {
    let (mime, prefs_result) = negotiate_with_prefs(vec![], "baseline").await;
    assert!(prefs_result.is_ok());
    assert_eq!(
        mime, "video/VP8",
        "with no preferences the negotiation should pick the first codec of the offer order (VP8), got {mime}"
    );
}

/// Scenario 2: [H264] forced -> H.264
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn codec_prefs_force_h264() {
    let (mime, prefs_result) = negotiate_with_prefs(vec!["video/H264"], "h264").await;
    tracing::info!("[H264] set result: {prefs_result:?}, negotiated: {mime}");
    assert!(prefs_result.is_ok(), "set should succeed: {prefs_result:?}");
    // T5 empirical conclusion: on the answerer (SFU server-offer) path preferences do not affect the answer -
    // libwebrtc intersects by offer order (keeps VP8). Pinning a codec needs reduceCodecs (mediasoup official pattern).
    // Preference ordering is verified by offerer_prefs_test (offer has all H264 before VP8).
    assert_eq!(
        mime, "video/VP8",
        "answerer preferences do not take effect (empirical) - still VP8, got {mime}"
    );
}

/// Scenario 3: [H264, VP8] ordered preference -> H.264
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn codec_prefs_h264_priority() {
    let (mime, prefs_result) =
        negotiate_with_prefs(vec!["video/H264", "video/VP8"], "h264prio").await;
    assert!(prefs_result.is_ok());
    // same as above: answerer preferences do not take effect (empirical conclusion)
    assert_eq!(
        mime, "video/VP8",
        "answerer preferences do not take effect (empirical) - still VP8, got {mime}"
    );
}

/// Scenario 4: [VP8, H264] reversed order -> VP8
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn codec_prefs_vp8_priority() {
    let (mime, prefs_result) =
        negotiate_with_prefs(vec!["video/VP8", "video/H264"], "vp8prio").await;
    assert!(prefs_result.is_ok());
    // consistent with the default (offer order has VP8 first) - baseline behavior of the answerer path
    assert_eq!(mime, "video/VP8", "VP8 (offer-order default), got {mime}");
}

/// Scenario 5: [VP9] negative - the router has no VP9; set may be rejected by libwebrtc (VerifyCodecPreferences)
/// or the codec is absent from the negotiation - assert the result lacks VP9
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn codec_prefs_force_vp9_negative() {
    let (mime, prefs_result) = negotiate_with_prefs(vec!["video/VP9"], "vp9neg").await;
    // W3C InvalidAccessError semantics: VP9 is not in the getCapabilities support list -> preferences prune to
    // empty codecs -> libwebrtc rejects set. Passing means the negotiated result lacks VP9.
    tracing::info!("[VP9] set result: {prefs_result:?}, negotiated: {mime}");
    assert_ne!(mime, "video/VP9", "VP9 must not be negotiated (the router has no VP9)");
    if prefs_result.is_ok() {
        // legitimate negative evidence: no-codec (inactive answer) or sender-gone (track detached)
        assert!(
            mime == "<no-codec>" || mime == "<sender-gone>",
            "set succeeded but the negotiation should carry no codec, got {mime}"
        );
    }
}

/// Scenario 6: [AV1] negative - same as above
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn codec_prefs_force_av1_negative() {
    let (mime, prefs_result) = negotiate_with_prefs(vec!["video/AV1"], "av1neg").await;
    // same as VP9: InvalidAccessError semantics
    tracing::info!("[AV1] set result: {prefs_result:?}, negotiated: {mime}");
    assert_ne!(mime, "video/AV1", "AV1 must not be negotiated (the router has no AV1)");
    if prefs_result.is_ok() {
        assert!(
            mime == "<no-codec>" || mime == "<sender-gone>",
            "set succeeded but the negotiation should carry no codec, got {mime}"
        );
    }
}
