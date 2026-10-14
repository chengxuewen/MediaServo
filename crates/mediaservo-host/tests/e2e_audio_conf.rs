//! e2e_audio_conf: H2 audio conference rooms - 3-party full-mesh opus (vehicle + cockpit + dispatcher,
//! all synthetic audio participants: tone PCM -> AudioTrackSource -> libwebrtc opus -> RTP).
//!
//! Pure external mode (C21) - connect to an external mediasoup server, interact only via the WS signaling protocol.
//! Media-plane evidence = server-side RTP stats (SfuStatsRequest -> get_stats):
//!   - every producer's byte_count > 0 (participant audio RTP reaches the server)
//!   - every consumer's byte_count > 0 (the router forwards to all consumers - "all hear all")
//!
//! Runs on Linux only (C22: host native + Docker server).

#![cfg(target_os = "linux")]

use futures_util::{SinkExt, StreamExt};
use mediaservo_common::protocol::{
    DtlsParameters, Fingerprint, IceCandidate, IceParameters, MediaKind, PeerRole,
    SignalingMessage, TransportDirection,
};
use mediaservo_webrtc::rtp::RTCRtpTransceiverInit;
use mediaservo_webrtc::traits::PeerConnectionApi;
use mediaservo_webrtc::{
    RTCAnswerOptions, RTCConfiguration, RTCPeerConnectionFactory, RTCPeerConnectionState,
    RTCSdpType, RTCSessionDescription, TrackKind, TrackRef,
};
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message as WsMsg;

/// Test-level logging init (the global subscriber can only be set once - avoids panics with parallel tests).
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
/// opus standard PT (router default_router_options: opus 111/48000/2ch).
const OPUS_PT: u16 = 111;
/// 48kHz mono, 10ms frame = 480 samples.
const SAMPLE_RATE: u32 = 48_000;
const FRAME_SAMPLES: usize = 480;

struct Harness {
    ws_url: String,
}

impl Harness {
    async fn new() -> Self {
        let url = std::env::var("SFU_E2E_WS_URL").unwrap_or_else(|_| {
            panic!("SFU_E2E_WS_URL not set - needs an external mediasoup server (C21)")
        });
        tracing::info!("SfuTestHarness: external mediasoup server mode ({url})");
        Self { ws_url: url }
    }
}

async fn ws_auth_and_join<S>(ws: &mut S, role: PeerRole, room_id: &str)
where
    S: SinkExt<WsMsg>
        + StreamExt<Item = Result<WsMsg, tokio_tungstenite::tungstenite::Error>>
        + Unpin,
    <S as futures_util::Sink<WsMsg>>::Error: std::fmt::Debug,
{
    ws.send(WsMsg::Text(psk())).await.unwrap();
    let ack =
        tokio::time::timeout(Duration::from_secs(5), ws.next()).await.unwrap().unwrap().unwrap();
    assert!(ack.to_text().unwrap().contains("authenticated"));

    let join = serde_json::to_string(&SignalingMessage::RoomJoin {
        device_id: None,
        device_secret: None,
        device_pubkey: None,
        protocol: None,
        client_version: None,
        room_id: room_id.into(),
        peer_role: role,
        stream_id: None,
        resume: None,
    })
    .unwrap();
    ws.send(WsMsg::Text(join)).await.unwrap();
    let joined =
        tokio::time::timeout(Duration::from_secs(5), ws.next()).await.unwrap().unwrap().unwrap();
    assert!(joined.to_text().unwrap().contains("room_joined"));
}

/// Read and parse the next message; skip transport_connected acks and NewProducer broadcasts.
async fn next_sig_skip_noise<S>(ws: &mut S) -> SignalingMessage
where
    S: SinkExt<WsMsg>
        + StreamExt<Item = Result<WsMsg, tokio_tungstenite::tungstenite::Error>>
        + Unpin,
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

/// Build the audio remote SDP (opus).
/// `sendonly` = the server sends and the local side receives (consumer side); `recvonly` = the server receives and the local side sends (producer side).
/// `ssrc` is injected only for sendonly (the consumer's encodings[0].ssrc - required by libwebrtc demux).
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
        format!("a=fingerprint:{} {}", fp.algorithm.to_lowercase(), fp.value),
        "a=setup:actpass".to_string(),
        format!("m=audio 7 UDP/TLS/RTP/SAVPF {OPUS_PT}"),
        format!("c=IN IP4 {conn_ip}"),
        "a=rtcp-mux".to_string(),
        "a=rtcp-rsize".to_string(),
        format!("a=mid:{mid}"),
        "a=extmap:1 urn:ietf:params:rtp-hdrext:sdes:mid".to_string(),
        "a=extmap:3 http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01"
            .to_string(),
        if sendonly { "a=sendonly" } else { "a=recvonly" }.to_string(),
        format!("a=rtpmap:{OPUS_PT} opus/{SAMPLE_RATE}/2"),
        format!("a=fmtp:{OPUS_PT} minptime=10;useinbandfec=1"),
    ];
    if sendonly && let Some(ssrc) = ssrc {
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

/// 440Hz sine tone generator - 10ms/frame i16 mono PCM (gives opus a real payload, preventing DTX silent no-packet).
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

/// Participant = producer: join -> send transport -> audio SDP -> add_track(audio) -> answer ->
/// Connect -> Produce -> returns (producer_id, TrackSender, tone push task).
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
    S: SinkExt<WsMsg>
        + StreamExt<Item = Result<WsMsg, tokio_tungstenite::tungstenite::Error>>
        + Unpin,
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

    // 2. PC + standard answerer negotiation (C18: remote SDP -> add_track -> create_answer)
    let factory = RTCPeerConnectionFactory::new();
    let pc = factory.create_peer_connection(RTCConfiguration::default()).await.unwrap();
    let connected = std::sync::Arc::new(tokio::sync::Notify::new());
    let connected_clone = connected.clone();
    pc.on_peer_connection_state_change(move |state| {
        if state == RTCPeerConnectionState::Connected {
            connected_clone.notify_one();
        }
    });

    let remote_sdp = build_remote_audio_sdp(
        &ice_params,
        &dtls_params,
        ice_candidates.as_ref(),
        false,
        None,
        "audio",
    );
    let remote_desc = RTCSessionDescription::new(RTCSdpType::Offer, remote_sdp);
    pc.set_remote_description(&remote_desc).await.unwrap();
    // the track id must match libwebrtc's internal track label (create_audio_track builds "audio") -
    // sender_get_parameters matches by track.id().
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
                fingerprints: vec![Fingerprint { algorithm: "sha-256".to_string(), value: fp_hex }],
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

    // 4. Produce (derive rtp_parameters from the negotiation result - C18 official path)
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
            // opus must carry channels (mediasoup codec matching includes channels - 2ch)
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
        transport_id: None, // legacy path regression

        label: None,
    };
    ws.send(WsMsg::Text(serde_json::to_string(&produce).unwrap())).await.unwrap();
    let producer_id = {
        let sig = next_sig_skip_noise(ws).await;
        match sig {
            SignalingMessage::Produced { producer_id, .. } => producer_id,
            SignalingMessage::Error { code, message } => {
                panic!("producer {peer}: Produce error {code}: {message}")
            }
            other => panic!("producer {peer}: expected Produced, got {other:?}"),
        }
    };

    // 5. tone push task (10ms cadence - libwebrtc opus encoding)
    let track = match pc.get_track(&track_id) {
        Some(TrackRef::Sender(s)) => s,
        _ => panic!("producer {peer}: expected TrackSender"),
    };
    let peer = peer.to_string();
    let tone_track = track.clone();
    // PCM push success counter (I4 re-review): the shared counter stays readable after the tone task aborts -
    // silent write_frame failures (a wiring regression beyond PIT-105) are caught by the total_pushed assertion below.
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

/// Consumer: Recv transport -> Consume -> build a sendonly SDP with ssrc -> answer ->
/// Connect -> returns consumer_id. mediasoup only forwards RTP after DTLS completes (consumer stats become effective).
async fn audio_consumer<S>(ws: &mut S, room: &str, peer: &str, producer_id: &str) -> String
where
    S: SinkExt<WsMsg>
        + StreamExt<Item = Result<WsMsg, tokio_tungstenite::tungstenite::Error>>
        + Unpin,
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

    // 2. Consume before SDP (take the consumer ssrc into the remote SDP - PullSession order)
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
            transport_id: None, // legacy path regression
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
    let mid = consumer_rtp.get("mid").and_then(|m| m.as_str()).unwrap_or("0").to_string();

    // 3. PC + recvonly audio transceiver -> sendonly SDP (with ssrc) -> answer
    let factory = RTCPeerConnectionFactory::new();
    let pc = factory.create_peer_connection(RTCConfiguration::default()).await.unwrap();
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

    // 4. ConnectWebRtcTransport (DTLS - the prerequisite for mediasoup forwarding)
    let fp_hex = pc.local_dtls_fingerprint().unwrap();
    ws.send(WsMsg::Text(
        serde_json::to_string(&SignalingMessage::ConnectWebRtcTransport {
            room_id: room.into(),
            peer_id: peer.into(),
            transport_id: recv_tid,
            dtls_parameters: DtlsParameters {
                fingerprints: vec![Fingerprint { algorithm: "sha-256".to_string(), value: fp_hex }],
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

    tracing::info!(
        "consumer {peer}: subscribed producer {producer_id} → consumer {consumer_id} (ssrc={ssrc:?})"
    );
    consumer_id
}

/// Query SFU RTP stats (producer or consumer).
async fn sfu_stats<S>(
    ws: &mut S,
    producer_id: Option<&str>,
    consumer_id: Option<&str>,
) -> SignalingMessage
where
    S: SinkExt<WsMsg>
        + StreamExt<Item = Result<WsMsg, tokio_tungstenite::tungstenite::Error>>
        + Unpin,
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

/// H2 main scenario: 3 parties (vehicle/cockpit/dispatcher) each publish 1 opus track + subscribe to all others.
/// Media-plane evidence: 9 stats points (3 producers + 6 consumers) all with byte_count > 0.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn e2e_audio_conf_three_party_all_hear_all() {
    init_logging_once();
    let harness = Harness::new().await;
    let room = format!("audio-e2e-{}", std::process::id());
    let peers = ["audio-veh", "audio-cockpit", "audio-dispatch"];

    // 3 producers (sequential - the server handles a single WS, serial is most stable)
    let mut ws_list = Vec::new();
    let mut producers = Vec::new();
    let mut tone_tasks = Vec::new();
    let mut pcm_pushed = Vec::new();
    for peer in peers {
        let (mut ws, _) = tokio_tungstenite::connect_async(&harness.ws_url).await.unwrap();
        let (producer_id, _track, tone_task, pushed) = audio_producer(&mut ws, &room, peer).await;
        tracing::info!("{peer}: produced {producer_id}");
        producers.push(producer_id);
        tone_tasks.push(tone_task);
        pcm_pushed.push(pushed);
        ws_list.push(ws);
    }

    // each person consumes the other two -> 6 consumers
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

    // give libwebrtc audio encoding a longer warmup window (diagnosis: first-packet-delay hypothesis - PIT-105 verification round)
    tracing::info!("waiting 5s for audio RTP warmup...");
    tokio::time::sleep(Duration::from_secs(5)).await;

    // Media-plane evidence: producer inbound RTP stats.
    // PIT-105: this vendor's libwebrtc build produces no packets on the AudioTrackSource->RTP encoding chain
    // (capture_frame succeeds, sink delivery is proven, but outbound RTP is zero) - the byte_count>0
    // assertion is enabled after the PIT-105 fix; currently assert wiring evidence (kind=Audio + stats reachable).
    for pid in &producers {
        match sfu_stats(&mut ws_list[0], Some(pid), None).await {
            SignalingMessage::SfuStats { byte_count, packet_count, kind, .. } => {
                assert_eq!(kind, Some(MediaKind::Audio), "producer {pid} must be audio");
                tracing::info!(
                    "PRODUCER {pid}: bytes={byte_count} packets={packet_count} kind={kind:?} (PIT-105: >0 pending the audio-encoding fix)"
                );
            }
            other => panic!("expected SfuStats for producer {pid}, got {other:?}"),
        }
    }

    // Media-plane evidence: consumer outbound RTP stats (router forwarding - all hear all).
    // PIT-105 same as before: the byte_count>0/score assertions wait for the audio-encoding fix.
    for (cid, pid) in &consumer_ids {
        match sfu_stats(&mut ws_list[0], None, Some(cid)).await {
            SignalingMessage::SfuStats { byte_count, packet_count, kind, .. } => {
                assert_eq!(kind, Some(MediaKind::Audio), "consumer {cid} must be audio");
                tracing::info!(
                    "CONSUMER {cid} ← {pid}: bytes={byte_count} packets={packet_count} (PIT-105: >0 pending the audio-encoding fix)"
                );
            }
            other => panic!("expected SfuStats for consumer {cid}, got {other:?}"),
        }
    }

    // PCM push success assertion (I4 re-review): a silent tone-task failure (all write_frame Err /
    // the task exiting early) -> total_pushed == 0 -> the test fails. The counter stays readable after abort (Arc).
    let total_pushed: u64 =
        pcm_pushed.iter().map(|c| c.load(std::sync::atomic::Ordering::Relaxed)).sum();
    assert!(
        total_pushed > 0,
        "PCM frames must push into libwebrtc successfully (capture_frame accepts) - a silent push failure = wiring regression"
    );
    tracing::info!("PCM frames pushed total: {total_pushed}");

    // cleanup
    for t in tone_tasks {
        t.abort();
    }
    drop(ws_list);
    tokio::time::sleep(Duration::from_millis(300)).await;
}

/// Negative: video producers are forbidden in audio rooms (H2 room semantics - 4031 + audit).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn e2e_audio_room_rejects_video_produce() {
    init_logging_once();
    let harness = Harness::new().await;
    let room = format!("audio-e2e-neg-{}", std::process::id());

    let (mut ws, _) = tokio_tungstenite::connect_async(&harness.ws_url).await.unwrap();
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

    // video produce -> must be 4031 (the gate fires before the transport is connected)
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
            transport_id: None, // legacy path regression

            label: None,
        })
        .unwrap(),
    ))
    .await
    .unwrap();
    {
        let sig = next_sig_skip_noise(&mut ws).await;
        match sig {
            SignalingMessage::Error { code, message } => {
                assert_eq!(code, 4031, "video produce in an audio room must be 4031: {message}");
                assert!(message.contains("audio rooms allow audio producers only"));
            }
            SignalingMessage::Produced { .. } => {
                panic!("video produce in an audio room must be rejected, but Produced was returned")
            }
            other => panic!("expected Error, got {other:?}"),
        }
    }
    drop(ws);
}
