//! E2E integration tests for Host→mediasoup SFU pipeline.
//!
//! Tests the integration between Host-side mediaservo-webrtc
//! (backend-webrtc-sys) and Server-side mediasoup SFU.
//!
//! Runs on Linux only — connects to an external mediasoup server (C21).
//! mediasoup 仅限 mediaservo-server；本测试仅通过 WS 信令协议交互。
//!
//! Tests:
//! - D1: Host creates RTCPeerConnection, negotiates with SFU, connects
//! - D1n: Negative test — retry loop when server is not started
//! - D2: Host produce → Consumer consume via SFU
//! - D3: Full pipeline with actual video frames

#![cfg(target_os = "linux")]

use futures_util::{SinkExt, StreamExt};
use mediaservo_common::protocol::{
    DtlsParameters, Fingerprint, IceCandidate, IceParameters, MediaKind, PeerRole,
    SignalingMessage, TransportDirection,
};
use mediaservo_webrtc::{
    RTCAnswerOptions, RTCConfiguration,
    RTCPeerConnectionFactory, RTCPeerConnectionState, RTCSdpType, RTCSessionDescription,
    TrackKind, TrackRef,
};
use mediaservo_webrtc::traits::PeerConnectionApi;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message as WsMsg;

// v2: PSK 可配置 — 外部 Docker server 模式用 SFU_E2E_PSK env 覆盖（server 用 MEDIASERVO_PSK=mediaservo-dev）
fn psk() -> String {
    std::env::var("SFU_E2E_PSK").unwrap_or_else(|_| "mediaservo-dev".to_string()) // 对齐 dev server (MEDIASERVO_PSK)
}
const ROOM: &str = "host-sfu-test-room";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

// ═══════════════════════════════════════════════════════════════════════════
// Helper: build remote SDP from mediasoup transport parameters (ICE-Lite answer)
// ═══════════════════════════════════════════════════════════════════════════

/// Build a remote SDP (ICE-Lite server offer) from mediasoup transport parameters.
/// Mirrors the SDP construction in `main.rs` — the Host side sees the SFU as
/// an ICE-Lite responder that provides a=ice-lite + a=setup:actpass.
fn build_remote_sdp(
    ice_parameters: &IceParameters,
    dtls_parameters: &DtlsParameters,
    ice_candidates: Option<&Vec<IceCandidate>>,
    payload_type: u16,
    codec_name: &str,
    clock_rate: u32,
    fmtp: Option<&str>,
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
        "a=setup:actpass".to_string(), // ICE-Lite responder expects client to initiate
    ];

    // PIT-48: a=candidate 行必须位于 m= 行之后（media section 内）——
    // 会话级 candidate 被 libwebrtc 忽略 → remote candidate 丢失 → ICE 不发起 STUN
    lines.extend_from_slice(&[
        format!("m=video 7 UDP/TLS/RTP/SAVPF {}", payload_type),
        format!("c=IN IP4 {}", conn_ip),
        "a=rtcp-mux".to_string(),
        "a=mid:video".to_string(),
        "a=recvonly".to_string(),
        format!("a=rtpmap:{} {}/{}", payload_type, codec_name, clock_rate),
    ]);

    if let Some(fmtp_val) = fmtp {
        lines.push(format!("a=fmtp:{} {}", payload_type, fmtp_val));
    }

    if let Some(candidates) = ice_candidates {
        for c in candidates {
            if c.ip.contains(".local") {
                continue;
            } // skip mDNS
            let ctype = match c.candidate_type.as_str() {
                "host" => "host",
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

// ═══════════════════════════════════════════════════════════════════════════
// Test harness: 纯外部模式 — 连外部 mediasoup server（C21）
// ═══════════════════════════════════════════════════════════════════════════

/// SfuTestHarness — 纯外部模式：连外部 mediasoup server（C21）。
/// mediasoup 仅限 mediaservo-server；测试仅通过 WS 信令协议交互，不 import server 类型。
struct SfuTestHarness {
    ws_url: String,
}

impl SfuTestHarness {
    async fn new() -> Self {
        // 缺省 dev server（docker compose 9800）——仍连外部 server（C21）; 生产/CI 用 env 覆盖
        let url = std::env::var("SFU_E2E_WS_URL").unwrap_or_else(|_| {
            "ws://127.0.0.1:9800/ws".to_string()
        });
        tracing::info!("SfuTestHarness: 外部 mediasoup server 模式 ({url})");
        Self { ws_url: url }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Helper: auth + RoomJoin on a WebSocket
// ═══════════════════════════════════════════════════════════════════════════

async fn ws_auth_and_join<S>(
    ws: &mut S,
    role: PeerRole,
    room_id: &str,
) where
    S: SinkExt<WsMsg> + StreamExt<Item = Result<WsMsg, tokio_tungstenite::tungstenite::Error>> + Unpin,
    <S as futures_util::Sink<WsMsg>>::Error: std::fmt::Debug,
{
    ws.send(WsMsg::Text(psk().into())).await.unwrap();
    let ack = tokio::time::timeout(Duration::from_secs(5), ws.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(ack.to_text().unwrap().contains("authenticated"));

    let join =
        serde_json::to_string(&SignalingMessage::RoomJoin {
            device_id: None,
            device_secret: None,
            device_pubkey: None,
            room_id: room_id.into(),
            peer_role: role,
            stream_id: None,
        })
        .unwrap();
    ws.send(WsMsg::Text(join.into())).await.unwrap();
    let joined = tokio::time::timeout(Duration::from_secs(5), ws.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(joined.to_text().unwrap().contains("room_joined"));
}

// ═══════════════════════════════════════════════════════════════════════════
// D1: Host creates RTCPeerConnection, negotiates with SFU, connects
// ═══════════════════════════════════════════════════════════════════════════

/// Full Host→SFU WebRTC connect flow:
/// 1. Host WS auth + RoomJoin
/// 2. CreateWebRtcTransport (Send) → get ICE/DTLS params
/// 3. Build remote SDP from params
/// 4. Create RTCPeerConnection via mediaservo-webrtc
/// 5. set_remote_description → add_track → create_answer → set_local_description
/// 6. Send ConnectWebRtcTransport with DTLS fingerprint
/// 7. Wait for PC state = Connected
/// 8. Assert transport connected
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn e2e_sfu_host_webrtc_connect() {
    let harness = SfuTestHarness::new().await;

    let host_url = harness.ws_url.clone();
    let host_handle = tokio::spawn(async move {
        let (mut ws, _) = tokio_tungstenite::connect_async(&host_url)
            .await
            .unwrap();

        ws_auth_and_join(&mut ws, PeerRole::Host, ROOM).await;

        // Create send WebRTC transport
        let create_transport =
            serde_json::to_string(&SignalingMessage::CreateWebRtcTransport {
                room_id: ROOM.into(),
                peer_id: "host".to_string(),
                direction: TransportDirection::Send,
            })
            .unwrap();
        ws.send(WsMsg::Text(create_transport.into()))
            .await
            .unwrap();

        // Wait for WebRtcTransportCreated
        let (transport_id, ice_parameters, dtls_parameters, ice_candidates) = loop {
            let resp = tokio::time::timeout(Duration::from_secs(5), ws.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            let text = resp.to_text().unwrap();
            let sig: SignalingMessage = serde_json::from_str(text).unwrap();
            match sig {
                SignalingMessage::WebRtcTransportCreated {
                    transport_id,
                    ice_parameters,
                    dtls_parameters,
                    ice_candidates,
                    ..
                } => break (transport_id, ice_parameters, dtls_parameters, ice_candidates),
                SignalingMessage::RoomLeave { .. } => continue,
                other => panic!("Unexpected message: {other:?}"),
            }
        };
        assert!(!transport_id.is_empty());
        assert!(!ice_parameters.username_fragment.is_empty());

        // Build remote SDP (VP8, matching SFU default codec list)
        let remote_sdp = build_remote_sdp(
            &ice_parameters,
            &dtls_parameters,
            ice_candidates.as_ref(),
            100, // VP8 PT from default_router_options
            "VP8",
            90000,
            None,
        );

        // Create PeerConnection via mediaservo-webrtc
        let factory = RTCPeerConnectionFactory::new();
        let config = RTCConfiguration::default();
        let pc = factory
            .create_peer_connection(config)
            .await
            .expect("Failed to create PC");

        // Set up connection state watcher
        let connected = Arc::new(tokio::sync::Notify::new());
        let connected_clone = connected.clone();
        pc.on_peer_connection_state_change(move |state| {
            if state == RTCPeerConnectionState::Connected {
                connected_clone.notify_one();
            }
        });

        // Negotiate: set remote → add track → create answer → set local
        let remote_desc = RTCSessionDescription::new(RTCSdpType::Offer, remote_sdp);
        pc.set_remote_description(&remote_desc)
            .await
            .expect("set_remote_description");

        let track_id = pc
            .add_track("video", TrackKind::Video)
            .expect("add_track");

        let answer = pc
            .create_answer(&RTCAnswerOptions::default())
            .await
            .expect("create_answer");
        pc.set_local_description(&answer)
            .await
            .expect("set_local_description");
        let _ = remote_sdp;

        // Extract DTLS fingerprint → send ConnectWebRtcTransport
        let fp_hex = pc
            .local_dtls_fingerprint()
            .expect("local_dtls_fingerprint");
        let connect = SignalingMessage::ConnectWebRtcTransport {
            room_id: ROOM.into(),
            peer_id: "host".to_string(),
            transport_id: transport_id.clone(),
            dtls_parameters: DtlsParameters {
                fingerprints: vec![Fingerprint {
                    algorithm: "sha-256".to_string(),
                    value: fp_hex,
                }],
                role: "client".to_string(),
            },
        };
        let json = serde_json::to_string(&connect).unwrap();
        ws.send(WsMsg::Text(json.into())).await.unwrap();

        // Wait for PC connected (ICE + DTLS negotiation)
        match tokio::time::timeout(CONNECT_TIMEOUT, connected.notified()).await {
            Ok(()) => {
                let state = pc.connection_state();
                assert!(
                    state == RTCPeerConnectionState::Connected,
                    "Expected Connected, got {state:?}"
                );
                (ws, transport_id, track_id)
            }
            Err(_) => panic!("ICE/DTLS connection timeout after {:?}", CONNECT_TIMEOUT),
        }
    });

    let (host_ws, transport_id, track_id) = host_handle.await.unwrap();
    assert!(!transport_id.is_empty());
    assert!(!track_id.is_empty());


    // Cleanup
    drop(host_ws);
    tokio::time::sleep(Duration::from_millis(200)).await;
}

// ═══════════════════════════════════════════════════════════════════════════
// D1n: Negative test — server not started, reconnect with exponential backoff
// ═══════════════════════════════════════════════════════════════════════════

/// Host tries to connect to a server that isn't running.
/// Retries up to 5 times with exponential backoff, then fails.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn e2e_sfu_host_reconnect_failure() {
    // Bind a port to know it's free, then immediately drop the listener
    // so the port is available but nothing is listening on it
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener); // port is now free — no server listening

    let ws_url = format!("ws://{}/ws", addr);
    const MAX_RETRIES: u32 = 5;
    let mut last_err = None;

    for attempt in 1..=MAX_RETRIES {
        match tokio_tungstenite::connect_async(&ws_url).await {
            Ok((mut ws, _)) => {
                // Unexpectedly connected — close gracefully and break
                let _ = ws.close(None).await;
                panic!(
                    "Unexpectedly connected to {} on attempt {}",
                    ws_url, attempt
                );
            }
            Err(e) => {
                last_err = Some(format!("{e}"));
                if attempt < MAX_RETRIES {
                    let delay = Duration::from_secs(1) * 2u32.pow(attempt - 1);
                    tokio::time::sleep(delay).await;
                }
            }
        }
    }

    assert!(
        last_err.is_some(),
        "Expected connection failure after {MAX_RETRIES} attempts to {ws_url}"
    );
    let err_msg = last_err.unwrap();
    assert!(
        err_msg.contains("Connection refused")
            || err_msg.contains("refused")
            || err_msg.contains("connect"),
        "Expected connection error, got: {err_msg}"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// D2: Host produce → Consumer consume via SFU
// ═══════════════════════════════════════════════════════════════════════════

/// Full produce→consume cycle:
/// Host: connect + produce video
/// Consumer: recv transport → NewProducer → consume → Consumed
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn e2e_sfu_host_produce() {
    let harness = SfuTestHarness::new().await;
    let produce_room = "sfu-host-produce-room";

    // ── Host: connect transport + produce ──
    let host_url = harness.ws_url.clone();
    let host_handle = tokio::spawn(async move {
        let (mut ws, _) = tokio_tungstenite::connect_async(&host_url)
            .await
            .unwrap();

        ws_auth_and_join(&mut ws, PeerRole::Host, produce_room).await;

        // Create send transport
        let create = SignalingMessage::CreateWebRtcTransport {
            room_id: produce_room.into(),
            peer_id: "host".into(),
            direction: TransportDirection::Send,
        };
        ws.send(WsMsg::Text(serde_json::to_string(&create).unwrap().into()))
            .await
            .unwrap();

        let (send_tid, send_dtls) = loop {
            let resp = tokio::time::timeout(Duration::from_secs(5), ws.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            let sig: SignalingMessage =
                serde_json::from_str(resp.to_text().unwrap()).unwrap();
            match sig {
                SignalingMessage::WebRtcTransportCreated {
                    transport_id,
                    dtls_parameters,
                    ..
                } => break (transport_id, dtls_parameters),
                SignalingMessage::RoomLeave { .. } => continue,
                other => panic!("Unexpected: {other:?}"),
            }
        };

        // Connect transport
        let connect = SignalingMessage::ConnectWebRtcTransport {
            room_id: produce_room.into(),
            peer_id: "host".into(),
            transport_id: send_tid.clone(),
            dtls_parameters: send_dtls,
        };
        ws.send(WsMsg::Text(serde_json::to_string(&connect).unwrap().into()))
            .await
            .unwrap();
        let conn_resp = tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(conn_resp.to_text().unwrap().contains("transport_connected"));

        // Produce video (VP8)
        let produce = SignalingMessage::Produce {
            room_id: produce_room.into(),
            peer_id: "host".into(),
            transport_direction: TransportDirection::Send,
            kind: MediaKind::Video,
            rtp_parameters: serde_json::json!({
                "mid": "0",
                "codecs": [{"mimeType": "video/VP8", "payloadType": 100, "clockRate": 90000}],
                "headerExtensions": [],
                "encodings": [{"ssrc": 12345}],
                "rtcp": {"reducedSize": true}
            }),
            transport_id: None, // legacy 路径回归
        };
        ws.send(WsMsg::Text(serde_json::to_string(&produce).unwrap().into()))
            .await
            .unwrap();
        let produced: SignalingMessage = loop {
            let resp = tokio::time::timeout(Duration::from_secs(5), ws.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            let sig: SignalingMessage =
                serde_json::from_str(resp.to_text().unwrap()).unwrap();
            match sig {
                SignalingMessage::NewProducer { .. } => continue, // broadcast, skip
                other => break other,
            }
        };
        let producer_id = match produced {
            SignalingMessage::Produced { producer_id, .. } => producer_id,
            other => panic!("Expected Produced, got: {other:?}"),
        };

        // Signal done with producer_id
        ws.send(WsMsg::Text(format!("host-ready:{}", producer_id)))
            .await
            .unwrap();
        (ws, producer_id)
    });

    let (host_ws, producer_id) = host_handle.await.unwrap();

    // ── Consumer: recv transport → consume ──
    let consumer_url = harness.ws_url.clone();
    let pid = producer_id.clone();
    let consumer_handle = tokio::spawn(async move {
        let (mut ws, _) = tokio_tungstenite::connect_async(&consumer_url)
            .await
            .unwrap();

        ws_auth_and_join(&mut ws, PeerRole::Remote, produce_room).await;

        // Create recv transport
        let create = SignalingMessage::CreateWebRtcTransport {
            room_id: produce_room.into(),
            peer_id: "consumer".into(),
            direction: TransportDirection::Recv,
        };
        ws.send(WsMsg::Text(serde_json::to_string(&create).unwrap().into()))
            .await
            .unwrap();
        let mut saw_new_producer = false;
        let (recv_tid, recv_dtls) = loop {
            let resp = tokio::time::timeout(Duration::from_secs(5), ws.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            let sig: SignalingMessage =
                serde_json::from_str(resp.to_text().unwrap()).unwrap();
            match sig {
                SignalingMessage::WebRtcTransportCreated {
                    transport_id,
                    dtls_parameters,
                    ..
                } => break (transport_id, dtls_parameters),
                SignalingMessage::NewProducer { .. } => {
                    saw_new_producer = true;
                    continue;
                }
                SignalingMessage::RoomLeave { .. } => continue,
                other => panic!("Unexpected: {other:?}"),
            }
        };

        // Connect recv transport
        let connect = SignalingMessage::ConnectWebRtcTransport {
            room_id: produce_room.into(),
            peer_id: "consumer".into(),
            transport_id: recv_tid,
            dtls_parameters: recv_dtls,
        };
        ws.send(WsMsg::Text(serde_json::to_string(&connect).unwrap().into()))
            .await
            .unwrap();
        let conn_resp = tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(conn_resp.to_text().unwrap().contains("transport_connected"));

        // Drain any remaining NewProducer messages (late-joiner sync / broadcast).
        // Timeout tolerated — the broadcast may already have been consumed
        // by the transport-creation loop above.
        loop {
            let msg = match tokio::time::timeout(Duration::from_secs(2), ws.next()).await {
                Ok(Some(Ok(msg))) => msg,
                _ => break, // no more messages pending
            };
            let text = msg.to_text().unwrap();
            if let Ok(sig) = serde_json::from_str::<SignalingMessage>(text) {
                if matches!(sig, SignalingMessage::NewProducer { .. }) {
                    saw_new_producer = true;
                } else {
                    break;
                }
            } else {
                break;
            }
        }

        // Now send Consume for the host's producer
        let consume = SignalingMessage::Consume {
            room_id: produce_room.into(),
            peer_id: "consumer".into(),
            producer_id: pid.clone(),
            rtp_capabilities: serde_json::json!({
                "codecs": [{"mimeType": "video/VP8", "clockRate": 90000, "kind": "video"}],
                "headerExtensions": []
            }),
            transport_id: None, // legacy 路径回归
        };
        ws.send(WsMsg::Text(serde_json::to_string(&consume).unwrap().into()))
            .await
            .unwrap();

        let consumed_resp =
            tokio::time::timeout(Duration::from_secs(5), ws.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
        let consumed: SignalingMessage =
            serde_json::from_str(consumed_resp.to_text().unwrap()).unwrap();
        let (consumer_id, consumed_producer_id, consumed_kind) = match consumed {
            SignalingMessage::Consumed {
                consumer_id,
                producer_id,
                kind,
                ..
            } => (consumer_id, producer_id, kind),
            other => panic!("Expected Consumed, got: {other:?}"),
        };

        assert!(!consumer_id.is_empty());
        assert_eq!(consumed_producer_id, pid);
        assert_eq!(consumed_kind, MediaKind::Video);
        (ws, saw_new_producer)
    });

    let (consumer_ws, saw_new_producer) = consumer_handle.await.unwrap();
    assert!(
        saw_new_producer,
        "Consumer should have received NewProducer (broadcast or late-joiner sync)"
    );


    // Cleanup
    drop(host_ws);
    drop(consumer_ws);
    tokio::time::sleep(Duration::from_millis(300)).await;
}

// ═══════════════════════════════════════════════════════════════════════════
// D3: Full pipeline — Host produce → Consumer consume with actual video frames
// ═══════════════════════════════════════════════════════════════════════════

/// Full pipeline with real video frames sent via mediaservo-webrtc.
/// Host creates PC, negotiates with SFU, produces video, sends I420 frames.
/// Consumer receives NewProducer, consumes, verifies pipeline end-to-end.
///
/// NOTE: TransportStats (bytesReceived, packetCount) not exposed via SfuManager
/// public API. The test verifies full pipeline setup + frame delivery indirectly
/// through Consumer creation and Producer state.
/// TODO: Add get_transport_stats() to SfuManager for stat assertions.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn e2e_sfu_full_pipeline() {
    let harness = SfuTestHarness::new().await;
    let pipeline_room = "sfu-pipeline-room";

    // ── Host: full WebRTC negotiation + produce + frame send ──
    let host_url = harness.ws_url.clone();
    let host_handle = tokio::spawn(async move {
        let (mut ws, _) = tokio_tungstenite::connect_async(&host_url)
            .await
            .unwrap();

        ws_auth_and_join(&mut ws, PeerRole::Host, pipeline_room).await;

        // Create send transport
        let create = SignalingMessage::CreateWebRtcTransport {
            room_id: pipeline_room.into(),
            peer_id: "host".into(),
            direction: TransportDirection::Send,
        };
        ws.send(WsMsg::Text(serde_json::to_string(&create).unwrap().into()))
            .await
            .unwrap();

        let (send_tid, ice_params, dtls_params, ice_candidates) = loop {
            let resp = tokio::time::timeout(Duration::from_secs(5), ws.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            let sig: SignalingMessage =
                serde_json::from_str(resp.to_text().unwrap()).unwrap();
            match sig {
                SignalingMessage::WebRtcTransportCreated {
                    transport_id,
                    ice_parameters,
                    dtls_parameters,
                    ice_candidates,
                    ..
                } => break (transport_id, ice_parameters, dtls_parameters, ice_candidates),
                SignalingMessage::RoomLeave { .. } => continue,
                other => panic!("Unexpected: {other:?}"),
            }
        };

        // Create PeerConnection with mediaservo-webrtc
        let factory = RTCPeerConnectionFactory::new();
        let pc = factory
            .create_peer_connection(RTCConfiguration::default())
            .await
            .unwrap();

        let connected = Arc::new(tokio::sync::Notify::new());
        let connected_clone = connected.clone();
        pc.on_peer_connection_state_change(move |state| {
            if state == RTCPeerConnectionState::Connected {
                connected_clone.notify_one();
            }
        });

        // Build remote SDP from SFU params + negotiate
        let remote_sdp = build_remote_sdp(
            &ice_params,
            &dtls_params,
            ice_candidates.as_ref(),
            100,
            "VP8",
            90000,
            None,
        );
        let remote_desc = RTCSessionDescription::new(RTCSdpType::Offer, remote_sdp);
        pc.set_remote_description(&remote_desc).await.unwrap();

        let track_id = pc.add_track("video", TrackKind::Video).unwrap();
        let answer = pc.create_answer(&RTCAnswerOptions::default()).await.unwrap();
        pc.set_local_description(&answer).await.unwrap();

        // Connect transport
        let fp_hex = pc.local_dtls_fingerprint().unwrap();
        let connect = SignalingMessage::ConnectWebRtcTransport {
            room_id: pipeline_room.into(),
            peer_id: "host".into(),
            transport_id: send_tid.clone(),
            dtls_parameters: DtlsParameters {
                fingerprints: vec![Fingerprint {
                    algorithm: "sha-256".to_string(),
                    value: fp_hex,
                }],
                role: "client".to_string(),
            },
        };
        ws.send(WsMsg::Text(serde_json::to_string(&connect).unwrap().into()))
            .await
            .unwrap();

        // Wait for PC connected
        tokio::time::timeout(CONNECT_TIMEOUT, connected.notified())
            .await
            .expect("PC connection timeout");
        assert_eq!(pc.connection_state(), RTCPeerConnectionState::Connected);

        // Drain transport_connected ack + any NewProducer broadcasts before Produce
        loop {
            let msg = match tokio::time::timeout(Duration::from_secs(2), ws.next()).await {
                Ok(Some(Ok(msg))) => msg,
                _ => break,
            };
            let text = msg.to_text().unwrap();
            if let Ok(sig) = serde_json::from_str::<SignalingMessage>(text) {
                match sig {
                    SignalingMessage::Error { message, .. } if message == "transport_connected" => continue,
                    SignalingMessage::NewProducer { .. } => continue,
                    _ => break,
                }
            } else {
                break;
            }
        }

        // Produce video
        let produce = SignalingMessage::Produce {
            room_id: pipeline_room.into(),
            peer_id: "host".into(),
            transport_direction: TransportDirection::Send,
            kind: MediaKind::Video,
            rtp_parameters: serde_json::json!({
                "mid": "0",
                "codecs": [{"mimeType": "video/VP8", "payloadType": 100, "clockRate": 90000}],
                "headerExtensions": [],
                "encodings": [{"ssrc": 12345}],
                "rtcp": {"reducedSize": true}
            }),
            transport_id: None, // legacy 路径回归
        };
        ws.send(WsMsg::Text(serde_json::to_string(&produce).unwrap().into()))
            .await
            .unwrap();
        let produced: SignalingMessage = loop {
            let resp = tokio::time::timeout(Duration::from_secs(5), ws.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            let sig: SignalingMessage =
                serde_json::from_str(resp.to_text().unwrap()).unwrap();
            match sig {
                SignalingMessage::NewProducer { .. } => continue, // broadcast, skip
                SignalingMessage::Error { message, .. } if message == "transport_connected" => continue,
                other => break other,
            }
        };
        let producer_id = match produced {
            SignalingMessage::Produced { producer_id, .. } => producer_id,
            other => panic!("Expected Produced, got: {other:?}"),
        };

        // Send a few I420 video frames to exercise the RTP pipeline
        let track = match pc.get_track(&track_id) {
            Some(TrackRef::Sender(s)) => s,
            _ => panic!("Expected TrackSender"),
        };

        let width: u32 = 320;
        let height: u32 = 240;
        let y_size = (width * height) as usize;
        let uv_size = (width * height / 4) as usize;
        let frame_size = y_size + 2 * uv_size;
        let mut frame = vec![0u8; frame_size];
        frame[y_size..y_size + uv_size].fill(128);
        frame[y_size + uv_size..].fill(128);

        let mut frames_sent = 0u32;
        for i in 0..10 {
            // Fill Y plane with a simple pattern
            let y_plane = &mut frame[..y_size];
            let offset = ((i * 20) % 200) as u8;
            for y in 0..height {
                for x in 0..width {
                    let idx = (y * width + x) as usize;
                    y_plane[idx] = if ((x / 20 + y / 20 + offset as u32 / 10) & 1) == 0
                    {
                        40
                    } else {
                        200
                    };
                }
            }
            match track.write_raw_i420(&frame, width, height).await {
                Ok(()) => frames_sent += 1,
                Err(e) => {
                    eprintln!("Frame {i} write error: {e}");
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(
            frames_sent > 0,
            "Should have sent at least one video frame"
        );

        // Signal completion
        ws.send(WsMsg::Text(format!("host-done:{}", producer_id)))
            .await
            .unwrap();
        (ws, producer_id, frames_sent)
    });

    let (host_ws, producer_id, frames_sent) = host_handle.await.unwrap();
    assert!(frames_sent > 0, "No frames were sent");

    // ── Consumer: recv transport → wait for NewProducer → consume ──
    let consumer_url = harness.ws_url.clone();
    let pid = producer_id.clone();
    let consumer_handle = tokio::spawn(async move {
        let (mut ws, _) = tokio_tungstenite::connect_async(&consumer_url)
            .await
            .unwrap();

        ws_auth_and_join(&mut ws, PeerRole::Remote, pipeline_room).await;

        // Create + connect recv transport
        let create = SignalingMessage::CreateWebRtcTransport {
            room_id: pipeline_room.into(),
            peer_id: "consumer".into(),
            direction: TransportDirection::Recv,
        };
        ws.send(WsMsg::Text(serde_json::to_string(&create).unwrap().into()))
            .await
            .unwrap();
        let mut got_producer = false;
        let (recv_tid, recv_dtls) = loop {
            let resp = tokio::time::timeout(Duration::from_secs(5), ws.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            let sig: SignalingMessage =
                serde_json::from_str(resp.to_text().unwrap()).unwrap();
            match sig {
                SignalingMessage::WebRtcTransportCreated {
                    transport_id,
                    dtls_parameters,
                    ..
                } => break (transport_id, dtls_parameters),
                SignalingMessage::NewProducer { producer_id: np_id, .. } => {
                    if np_id == pid {
                        got_producer = true;
                    }
                    continue;
                }
                SignalingMessage::RoomLeave { .. } => continue,
                other => panic!("Unexpected: {other:?}"),
            }
        };
        let connect = SignalingMessage::ConnectWebRtcTransport {
            room_id: pipeline_room.into(),
            peer_id: "consumer".into(),
            transport_id: recv_tid,
            dtls_parameters: recv_dtls,
        };
        ws.send(WsMsg::Text(serde_json::to_string(&connect).unwrap().into()))
            .await
            .unwrap();
        let conn_resp = tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(conn_resp.to_text().unwrap().contains("transport_connected"));

        // Wait for NewProducer (broadcast or late-joiner sync) — may already
        // have been consumed by the transport-creation loop above.
        if !got_producer {
            for _ in 0..10 {
                let msg = tokio::time::timeout(Duration::from_secs(3), ws.next())
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap();
                let text = msg.to_text().unwrap();
                if let Ok(SignalingMessage::NewProducer {
                    producer_id: np_id, ..
                }) = serde_json::from_str(text)
                {
                    if np_id == pid {
                        got_producer = true;
                        break;
                    }
                }
            }
        }
        assert!(
            got_producer,
            "Consumer should receive NewProducer for producer {pid}"
        );
        // Consume
        let consume = SignalingMessage::Consume {
            room_id: pipeline_room.into(),
            peer_id: "consumer".into(),
            producer_id: pid.clone(),
            rtp_capabilities: serde_json::json!({
                "codecs": [{"mimeType": "video/VP8", "clockRate": 90000, "kind": "video"}],
                "headerExtensions": []
            }),
            transport_id: None, // legacy 路径回归
        };
        ws.send(WsMsg::Text(serde_json::to_string(&consume).unwrap().into()))
            .await
            .unwrap();
        let consumed_resp =
            tokio::time::timeout(Duration::from_secs(5), ws.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
        let consumed: SignalingMessage =
            serde_json::from_str(consumed_resp.to_text().unwrap()).unwrap();
        match consumed {
            SignalingMessage::Consumed {
                consumer_id,
                kind,
                rtp_parameters,
                ..
            } => {
                assert!(!consumer_id.is_empty(), "consumer_id must not be empty");
                assert_eq!(kind, MediaKind::Video);
                assert!(
                    !rtp_parameters.is_null(),
                    "rtp_parameters must not be null"
                );
            }
            other => panic!("Expected Consumed, got: {other:?}"),
        }

        ws
    });

    let consumer_ws = consumer_handle.await.unwrap();


    // Cleanup
    drop(host_ws);
    drop(consumer_ws);
    tokio::time::sleep(Duration::from_millis(300)).await;
}
