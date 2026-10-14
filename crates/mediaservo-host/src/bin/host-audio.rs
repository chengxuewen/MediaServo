//! host-audio: audio conference process (Task H2 real implementation).
//!
//! Usage: `host-audio --room audio-<vehicle> [--gateway ws://127.0.0.1:17980/ws | --server ws://host:9800/ws --psk <psk>] [--duration <secs>]`
//!
//! Flow (D-H12: one audio room per vehicle; each participant publishes 1 opus track + subscribes to all others):
//! 1. Signaling connect (default via local gateway host-agent; `--server` connects to the server directly)
//! 2. Send transport → standard answerer negotiation (audio SDP → add_track(audio) → answer) →
//!    Connect → Produce (opus, rtp_parameters derived from the negotiated result, C18)
//! 3. Tone source (synthetic 440Hz sine 10ms PCM → AudioTrackSource → libwebrtc opus encoding) —
//!    stub audio source; real ALSA/MMAPI microphones = Phase I+ (documented follow-up)
//! 4. Event loop: NewProducer (others joining) → Recv transport → Consume → sendonly SDP (ssrc injection)
//!    → answer → Connect (full-mesh subscription)
//! 5. Periodic SfuStats logging (media-plane evidence; PIT-105: libwebrtc audio encoding produces no
//!    packets — bytes>0 pending fix)
//! 6. SIGTERM / `--duration` → graceful exit 0 (C15: failures visible, deployment restart_policy relaunches)

use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use mediaservo_common::protocol::{
    DtlsParameters, Fingerprint, MediaKind, PeerRole, SignalingMessage, TransportDirection,
};
use mediaservo_host::audio;
use mediaservo_link::{SignalClient, SignalEvent, SignalSession};
use mediaservo_webrtc::rtp::RTCRtpTransceiverInit;
use mediaservo_webrtc::traits::PeerConnectionApi;
use mediaservo_webrtc::{
    RTCAnswerOptions, RTCConfiguration, RTCPeerConnectionFactory, RTCPeerConnectionState,
    RTCSdpType, RTCSessionDescription, TrackKind, TrackRef,
};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const STATS_INTERVAL: Duration = Duration::from_secs(5);

const USAGE: &str = "Usage: host-audio --room audio-<vehicle> [--gateway <ws> | --server <ws> --psk <psk>] [--duration <secs>] [--tone <hz>]";

struct Args {
    room: String,
    gateway: Option<String>,
    server: Option<String>,
    psk: Option<String>,
    duration_secs: Option<u64>,
    tone_hz: u16,
}

fn parse_args() -> Result<Args, String> {
    let mut args = std::env::args().skip(1);
    let mut room = None;
    let mut gateway = None;
    let mut server = None;
    let mut psk = None;
    let mut duration_secs = None;
    let mut tone_hz = 440u16;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--room" => room = args.next(),
            "--gateway" => gateway = args.next(),
            "--server" => server = args.next(),
            "--psk" => psk = args.next(),
            "--duration" => {
                duration_secs = args.next().and_then(|v| v.parse().ok());
            }
            "--tone" => {
                tone_hz = args.next().and_then(|v| v.parse().ok()).unwrap_or(440);
            }
            "--help" | "-h" => return Err(USAGE.to_string()),
            other => return Err(format!("unknown arg: {other}\n{USAGE}")),
        }
    }
    let room = room.ok_or_else(|| format!("missing --room\n{USAGE}"))?;
    if !room.starts_with("audio-") {
        return Err(format!(
            "--room must be of form audio-<vehicle> (H2 audio room convention), got {room}"
        ));
    }
    Ok(Args { room, gateway, server, psk, duration_secs, tone_hz })
}

/// Await one signaling event of the given kind (skipping unrelated broadcasts/acks).
async fn await_event<T, F>(
    rx: &mut tokio::sync::broadcast::Receiver<SignalEvent>,
    mut f: F,
) -> Result<T, String>
where
    F: FnMut(SignalingMessage) -> Result<Option<T>, String>,
{
    loop {
        match rx.recv().await {
            Ok(SignalEvent::Message(SignalingMessage::Error { message, .. }))
                if message == "transport_connected" =>
            {
                continue;
            }
            Ok(SignalEvent::Message(m)) => match f(m) {
                Ok(Some(v)) => return Ok(v),
                Ok(None) => continue,
                Err(e) => return Err(e),
            },
            Ok(SignalEvent::Disconnected { reason }) => {
                return Err(format!("signaling disconnected: {reason}"));
            }
            Ok(SignalEvent::Error(e)) => return Err(format!("signaling error: {e}")),
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                return Err("signaling channel closed".into());
            }
            _ => continue,
        }
    }
}

/// Publish one opus track: Send transport → answerer negotiation → Connect → Produce.
/// Returns (producer_id, an events receiver reusable by this room's members).
async fn publish_audio(
    signal: &SignalSession,
    room: &str,
    tone_hz: u16,
) -> Result<
    (
        String,
        Arc<mediaservo_webrtc::track::TrackSender>,
        std::sync::Arc<std::sync::atomic::AtomicU64>,
    ),
    String,
> {
    // 1. Send transport
    signal
        .send(SignalingMessage::CreateWebRtcTransport {
            room_id: room.into(),
            peer_id: signal.peer_id().into(),
            direction: TransportDirection::Send,
        })
        .await
        .map_err(|e| format!("create transport send: {e}"))?;
    let mut events = signal.events();
    let (send_tid, ice_params, dtls_params, ice_candidates) =
        await_event(&mut events, |m| match m {
            SignalingMessage::WebRtcTransportCreated {
                transport_id,
                ice_parameters,
                dtls_parameters,
                ice_candidates,
                ..
            } => Ok(Some((transport_id, ice_parameters, dtls_parameters, ice_candidates))),
            SignalingMessage::Error { code, message } => {
                Err(format!("CreateWebRtcTransport error {code}: {message}"))
            }
            _ => Ok(None),
        })
        .await?;

    // 2. PC + standard answerer negotiation (C18: remote SDP → add_track → create_answer)
    let factory = RTCPeerConnectionFactory::new();
    let pc = factory
        .create_peer_connection(RTCConfiguration::default())
        .await
        .map_err(|e| format!("create pc: {e}"))?;
    let connected = Arc::new(tokio::sync::Notify::new());
    let connected_clone = connected.clone();
    pc.on_peer_connection_state_change(move |state| {
        if state == RTCPeerConnectionState::Connected {
            connected_clone.notify_one();
        }
    });
    let remote_sdp = audio::build_remote_audio_sdp(
        &ice_params,
        &dtls_params,
        ice_candidates.as_ref(),
        false,
        None,
        "audio",
    );
    let remote_desc = RTCSessionDescription::new(RTCSdpType::Offer, remote_sdp);
    pc.set_remote_description(&remote_desc)
        .await
        .map_err(|e| format!("set remote description: {e}"))?;
    // track id must match libwebrtc's internal track label (create_audio_track builds "audio")
    let track_id =
        pc.add_track("audio", TrackKind::Audio).map_err(|e| format!("add audio track: {e}"))?;
    let answer =
        pc.create_answer(&RTCAnswerOptions {}).await.map_err(|e| format!("create answer: {e}"))?;
    pc.set_local_description(&answer).await.map_err(|e| format!("set local description: {e}"))?;

    // 3. ConnectWebRtcTransport (DTLS)
    let fp_hex = pc.local_dtls_fingerprint().ok_or("no DTLS fingerprint")?;
    signal
        .send(SignalingMessage::ConnectWebRtcTransport {
            room_id: room.into(),
            peer_id: signal.peer_id().into(),
            transport_id: send_tid.clone(),
            dtls_parameters: DtlsParameters {
                fingerprints: vec![Fingerprint { algorithm: "sha-256".to_string(), value: fp_hex }],
                role: "client".to_string(),
            },
        })
        .await
        .map_err(|e| format!("connect transport: {e}"))?;
    tokio::time::timeout(CONNECT_TIMEOUT, connected.notified())
        .await
        .map_err(|_| "producer PC connect timeout".to_string())?;
    if pc.connection_state() != RTCPeerConnectionState::Connected {
        return Err(format!("PC not connected: {:?}", pc.connection_state()));
    }

    // 4. Produce (rtp_parameters derived from the negotiated result — C18 official path)
    let rtp_params = pc
        .get_sending_rtp_parameters(&track_id)
        .map_err(|e| format!("get_sending_rtp_parameters: {e}"))?;
    signal
        .send(SignalingMessage::Produce {
            room_id: room.into(),
            peer_id: signal.peer_id().into(),
            transport_direction: TransportDirection::Send,
            kind: MediaKind::Audio,
            rtp_parameters: audio::build_audio_produce_rtp_parameters(&rtp_params),
            // C1: explicitly bind this session's send transport (host children share peer_id; prevent cross-talk)
            transport_id: Some(send_tid),
            label: None, // producer-label T3: audio carries no business label yet
        })
        .await
        .map_err(|e| format!("produce: {e}"))?;
    let producer_id = await_event(&mut events, |m| match m {
        SignalingMessage::Produced { producer_id, .. } => Ok(Some(producer_id)),
        SignalingMessage::Error { code, message } => {
            Err(format!("Produce error {code}: {message}"))
        }
        _ => Ok(None),
    })
    .await?;

    // 5. tone push task (stub audio source — real microphone in Phase I+)
    let sender = match pc.get_track(&track_id) {
        Some(TrackRef::Sender(s)) => s,
        _ => return Err("expected TrackSender".into()),
    };
    let tone_track = sender.clone();
    // PCM push success counter (I4 re-review): surfaced in periodic stats logs — silent
    // write_frame failures become observable (host_audio_e2e asserts pushed>0).
    let pushed: std::sync::Arc<std::sync::atomic::AtomicU64> = Default::default();
    let pushed_for_task = pushed.clone();
    let tone_task = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(10));
        let mut phase: f64 = 0.0;
        let freq = f64::from(tone_hz);
        loop {
            interval.tick().await;
            let frame = audio::tone_frame(&mut phase, freq);
            match tone_track.write_frame(&frame).await {
                Ok(()) => {
                    pushed_for_task.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
                Err(e) => {
                    tracing::warn!("host-audio: tone write_frame failed: {e}");
                    break;
                }
            }
        }
    });
    tracing::info!(
        "host-audio: published producer {producer_id} (tone={tone_hz}Hz, PIT-105: audio RTP pending libwebrtc fix)"
    );
    // PIT-105: keep the tone task alive for the full lifetime (avoid spurious reclamation) — ends with the process
    std::mem::forget(tone_task);
    Ok((producer_id, Arc::new(sender), pushed))
}

/// Subscribe one producer: Recv transport → Consume (take the ssrc first) → sendonly SDP → answer → Connect.
async fn subscribe_producer(
    signal: &SignalSession,
    room: &str,
    producer_id: &str,
    consumer_ids: Arc<std::sync::Mutex<Vec<String>>>,
) {
    let peer = signal.peer_id().to_string();
    let result: Result<(), String> = async {
        // 1. Recv transport
        signal
            .send(SignalingMessage::CreateWebRtcTransport {
                room_id: room.into(),
                peer_id: signal.peer_id().into(),
                direction: TransportDirection::Recv,
            })
            .await
            .map_err(|e| format!("create transport recv: {e}"))?;
        let mut events = signal.events();
        let (recv_tid, ice_params, dtls_params, ice_candidates) = await_event(
            &mut events,
            |m| match m {
                SignalingMessage::WebRtcTransportCreated {
                    transport_id,
                    ice_parameters,
                    dtls_parameters,
                    ice_candidates,
                    ..
                } => Ok(Some((transport_id, ice_parameters, dtls_parameters, ice_candidates))),
                SignalingMessage::Error { code, message } => {
                    Err(format!("CreateWebRtcTransport(recv) error {code}: {message}"))
                }
                _ => Ok(None),
            },
        )
        .await?;

        // 2. Consume before SDP (inject the consumer ssrc into the remote SDP — PullSession order)
        signal
            .send(SignalingMessage::Consume {
                room_id: room.into(),
                peer_id: signal.peer_id().into(),
                producer_id: producer_id.to_string(),
                rtp_capabilities: serde_json::json!({
                    "codecs": [{"mimeType": "audio/opus", "clockRate": 48000, "kind": "audio", "channels": 2}],
                    "headerExtensions": [
                        {"uri": "urn:ietf:params:rtp-hdrext:sdes:mid", "preferredId": 1, "kind": "audio", "preferredEncrypt": false, "direction": "sendrecv"},
                        {"uri": "http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01", "preferredId": 3, "kind": "audio", "preferredEncrypt": false, "direction": "sendrecv"},
                    ],
                }),
                // C1: explicitly bind this session's recv transport (prevent cross-talk from connections sharing peer_id)
                transport_id: Some(recv_tid.clone()),
            })
            .await
            .map_err(|e| format!("consume: {e}"))?;
        let (consumer_id, consumer_rtp) = await_event(&mut events, |m| match m {
            SignalingMessage::Consumed { consumer_id, rtp_parameters, .. } => {
                Ok(Some((consumer_id, rtp_parameters)))
            }
            SignalingMessage::Error { code, message } => {
                Err(format!("Consume error {code}: {message}"))
            }
            _ => Ok(None),
        })
        .await?;
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

        // 3. PC + recvonly audio transceiver → sendonly SDP (with ssrc) → answer
        let factory = RTCPeerConnectionFactory::new();
        let pc = factory
            .create_peer_connection(RTCConfiguration::default())
            .await
            .map_err(|e| format!("create pc: {e}"))?;
        let connected = Arc::new(tokio::sync::Notify::new());
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
        .map_err(|e| format!("add_transceiver: {e}"))?;
        let remote_sdp = audio::build_remote_audio_sdp(
            &ice_params,
            &dtls_params,
            ice_candidates.as_ref(),
            true,
            ssrc,
            &mid,
        );
        let remote_desc = RTCSessionDescription::new(RTCSdpType::Offer, remote_sdp);
        pc.set_remote_description(&remote_desc)
            .await
            .map_err(|e| format!("set remote description: {e}"))?;
        let answer = pc
            .create_answer(&RTCAnswerOptions {})
            .await
            .map_err(|e| format!("create answer: {e}"))?;
        pc.set_local_description(&answer)
            .await
            .map_err(|e| format!("set local description: {e}"))?;

        // 4. ConnectWebRtcTransport (DTLS — prerequisite for mediasoup forwarding)
        let fp_hex = pc.local_dtls_fingerprint().ok_or("no DTLS fingerprint")?;
        signal
            .send(SignalingMessage::ConnectWebRtcTransport {
                room_id: room.into(),
                peer_id: signal.peer_id().into(),
                transport_id: recv_tid,
                dtls_parameters: DtlsParameters {
                    fingerprints: vec![Fingerprint {
                        algorithm: "sha-256".to_string(),
                        value: fp_hex,
                    }],
                    role: "client".to_string(),
                },
            })
            .await
            .map_err(|e| format!("connect recv transport: {e}"))?;
        tokio::time::timeout(CONNECT_TIMEOUT, connected.notified())
            .await
            .map_err(|_| "consumer PC connect timeout".to_string())?;

        tracing::info!("host-audio: subscribed {producer_id} → consumer {consumer_id} (ssrc={ssrc:?})");
        consumer_ids.lock().unwrap().push(consumer_id);
        Ok(())
    }
    .await;
    if let Err(e) = result {
        tracing::warn!("host-audio({peer}): subscribe {producer_id} failed: {e}");
    }
}

#[tokio::main]
async fn main() -> Result<ExitCode, Box<dyn std::error::Error>> {
    mediaservo_host::init_logging("audio");
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            return Ok(ExitCode::from(2));
        }
    };

    // Signaling connect (gateway mode = vehicle session child process; --server = direct)
    let signal = match (&args.gateway, &args.server) {
        (Some(gw), _) => {
            SignalClient::new_gateway(gw, "audio", &args.room, PeerRole::Consumer).connect().await
        }
        (_, Some(srv)) => {
            let psk = args.psk.clone().ok_or("direct mode requires --psk")?;
            SignalClient::new(srv, &psk, &args.room, PeerRole::Consumer).connect().await
        }
        _ => {
            // default: local gateway (D2 vehicle architecture)
            SignalClient::new_gateway(
                "ws://127.0.0.1:17980/ws",
                "audio",
                &args.room,
                PeerRole::Consumer,
            )
            .connect()
            .await
        }
    };
    let signal = match signal {
        Ok(s) => Arc::new(s),
        Err(e) => {
            tracing::error!("host-audio: signaling connect failed: {e}");
            return Ok(ExitCode::from(1));
        }
    };
    tracing::info!("host-audio: joined audio room {}", args.room);

    // Publish one opus track
    let (producer_id, _sender, pushed) =
        match publish_audio(&signal, &args.room, args.tone_hz).await {
            Ok(v) => v,
            Err(e) => {
                tracing::error!("host-audio: publish failed: {e}");
                if let Ok(s) = Arc::try_unwrap(signal) {
                    let _ = s.close().await;
                }
                return Ok(ExitCode::from(1));
            }
        };

    // Event loop + periodic stats + shutdown signal
    let consumer_ids: Arc<std::sync::Mutex<Vec<String>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));
    let consumer_ids_for_events = consumer_ids.clone();
    let room = args.room.clone();
    let signal_for_events = signal;
    let pid = producer_id.clone();
    let consumer_ids_for_stats = consumer_ids.clone();

    let stats_task = tokio::spawn(async move {
        let mut interval = tokio::time::interval(STATS_INTERVAL);
        loop {
            interval.tick().await;
            let own_consumers = consumer_ids_for_stats.lock().unwrap().len();
            let pushed_n = pushed.load(std::sync::atomic::Ordering::Relaxed);
            tracing::info!(
                "host-audio: producer={pid} consumers={own_consumers} pushed_pcm={pushed_n} (PIT-105: RTP byte stats take effect after the libwebrtc audio-encoding fix)"
            );
        }
    });

    let mut events = signal_for_events.events();
    let mut subscribed = std::collections::HashSet::new();
    let mut shutdown = false;

    // SIGTERM/SIGINT
    #[cfg(unix)]
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    #[cfg(unix)]
    let mut sigint = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
    let mut duration_wait =
        args.duration_secs.map(|d| Box::pin(tokio::time::sleep(Duration::from_secs(d))));

    loop {
        tokio::select! {
            ev = events.recv() => {
                match ev {
                    Ok(SignalEvent::Message(SignalingMessage::NewProducer { producer_id: np, .. })) => {
                        if np == producer_id || subscribed.contains(&np) {
                            continue;
                        }
                        subscribed.insert(np.clone());
                        let signal = signal_for_events.clone();
                        let room = room.clone();
                        let cids = consumer_ids_for_events.clone();
                        tokio::spawn(async move {
                            subscribe_producer(&signal, &room, &np, cids).await;
                        });
                    }
                    Ok(SignalEvent::Disconnected { reason }) => {
                        tracing::warn!("host-audio: signaling disconnected: {reason}");
                        break;
                    }
                    // H6: upstream switch (gateway reconnect notice / passive 5001 during outage) — the
                    // session is untrusted; exit and restart to rebuild (same channel as Disconnected;
                    // succeeds on first try once the server is ready).
                    Ok(SignalEvent::Message(SignalingMessage::Error { code: 5001, .. })) => {
                        tracing::warn!("host-audio: upstream switch (gateway 5001) — exiting to rebuild the session");
                        break;
                    }
                    Ok(SignalEvent::Message(SignalingMessage::SfuStats { .. })) => {}
                    _ => {}
                }
            },
            _ = sigterm.recv() => { tracing::info!("host-audio: SIGTERM received, graceful exit"); shutdown = true; },
            _ = sigint.recv() => { tracing::info!("host-audio: SIGINT received, graceful exit"); shutdown = true; },
            _ = async {
                match &mut duration_wait {
                    Some(sleep) => sleep.as_mut().await,
                    None => std::future::pending::<()>().await,
                }
            } => {
                tracing::info!("host-audio: --duration elapsed, exiting");
                shutdown = true;
            },
        }
        if shutdown {
            break;
        }
    }

    stats_task.abort();
    Arc::try_unwrap(signal_for_events).map_err(|_| "session still shared")?.close().await?;
    tracing::info!("host-audio: exited (code 0)");
    Ok(ExitCode::SUCCESS)
}
