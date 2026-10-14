//! host-streamer: push-stream process (Task C2) — FrameBus subscribe → WebRTC push.
//!
//! Usage: `host-streamer --stream <id> --config <host.yaml path> --token <token file path>`
//!
//! Flow: read host.yaml `streams` (source/codec default to id/h264,
//! [`mediaservo_host::translate::stream_config`]) → source config (fps) → FrameBus
//! subscribe `camera/<camera-id>` (FrameMeta + tight I420, C1 capturer wire format) →
//! field `PushSession` (connect → publish_video: SFU transport + answer negotiation +
//! Connect + Produce, reusing the field push chain) → `TrackSender::write_raw_i420_with_ts`
//! writes frames (timestamps from FrameMeta.ts_mono_ns, C17 passthrough). P2P mode: signaling connects
//! straight to the Server (before the Phase D gateway, MUST NOT introduce bus signaling).
//!
//! Signaling target = local gateway host-agent (D2): `--gateway <url>` (default
//! `ws://127.0.0.1:17980/ws`). The gateway's local side has no PSK challenge (trust boundary
//! 127.0.0.1); the whole-vehicle PSK lives on host-agent's remote connection. Room = `stream-<id>`
//! (the gateway intercepts RoomJoin and rewrites it to the vehicle room, folding multiple streams into one vehicle session).

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use mediaservo_common::protocol::{PeerRole, SignalingMessage};
use mediaservo_field::{PublishOptions, PushConfig, PushSession, SessionEvent};
use mediaservo_host::monitor::flow::StreamerStats;
use mediaservo_link::{
    FrameBus, FrameMeta, FrameRef, FrameStream, FrameTopic, SignalClient, SignalEvent,
    SignalSession, TokenFile,
};
use mediaservo_webrtc::data_channel::{RTCDataChannel, RTCDataChannelInit, RTCDataChannelState};
use mediaservo_webrtc::peer_connection::{RTCConfiguration, RTCIceCandidate};
use mediaservo_webrtc::sdp::{RTCSdpType, RTCSessionDescription};
use mediaservo_webrtc::stats::RTCStats;
use mediaservo_webrtc::traits::PeerConnectionApi;
use mediaservo_webrtc::{RTCPeerConnection, RTCPeerConnectionFactory};
use tokio::sync::{broadcast, mpsc};

/// FrameMeta pixel format: 1 = I420 (D243 enum, consistent with the C1 capturer).
const FORMAT_I420: u8 = 1;
/// no-frame watchdog: when the capturer has not started / has exited, exit after 10s without frames (C15,
/// visible failure rather than a hang; the deploy side restart_policy=always brings it up, aligned with the PIT-87 ICE-failed self-heal pattern).
const NO_FRAME_TIMEOUT: Duration = Duration::from_secs(10);
/// egress stats log interval (e2e evidence + observability).
const STATS_INTERVAL: Duration = Duration::from_secs(2);
/// vision DC label (D-H8 contract: the cockpit-side HMI overlay receives by this label).
const VISION_DC_LABEL: &str = "vision";
/// vision DC negotiation deadline: degrade if no cockpit-side answer (vision is an optional overlay; video is unaffected).
const VISION_NEGOTIATE_TIMEOUT: Duration = Duration::from_secs(10);

const USAGE: &str = "Usage: host-streamer --stream <id> --config <host.yaml> --token <token file>";

/// egress stats message sequence number (stats topic FrameMeta.seq, monotonic).
static STATS_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
struct Args {
    stream: String,
    config: PathBuf,
    token: PathBuf,
    /// local gateway WS address (D2); default `ws://127.0.0.1:17980/ws`.
    gateway: Option<String>,
    /// encoder backend (auto/software/hardware/nvenc/vaapi; default auto).
    encoder_backend: String,
    /// encode bitrate kbps (None = field default 2000).
    bitrate_kbps: Option<u32>,
    /// weak-network degradation preference (qos-framerate-priority; None = keep PushConfig default Balanced).
    degradation: Option<mediaservo_webrtc::rtp::RTCDegradationPreference>,
    /// content hint (None = default; only sent when translate expands Fluid).
    content_hint: Option<mediaservo_webrtc::rtp::RTCRtpContentHint>,
    /// bitrate floor kbps (None = no lower bound).
    min_bitrate_kbps: Option<u32>,
    /// keyframe interval seconds GOP (None = field default 2).
    keyframe_interval: Option<u32>,
}

fn parse_args() -> Result<Args, String> {
    let mut args = std::env::args().skip(1);
    let mut stream: Option<String> = None;
    let mut config: Option<PathBuf> = None;
    let mut token: Option<PathBuf> = None;
    let mut gateway: Option<String> = None;
    let mut encoder_backend: Option<String> = None;
    let mut bitrate_kbps: Option<u32> = None;
    let mut degradation: Option<mediaservo_webrtc::rtp::RTCDegradationPreference> = None;
    let mut content_hint: Option<mediaservo_webrtc::rtp::RTCRtpContentHint> = None;
    let mut min_bitrate_kbps: Option<u32> = None;
    let mut keyframe_interval: Option<u32> = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--stream" => stream = Some(args.next().ok_or("--stream missing value")?),
            "--config" => {
                config = Some(PathBuf::from(args.next().ok_or("--config missing value")?))
            }
            "--token" => token = Some(PathBuf::from(args.next().ok_or("--token missing value")?)),
            "--gateway" => gateway = Some(args.next().ok_or("--gateway missing value")?),
            "--encoder-backend" => {
                encoder_backend = Some(args.next().ok_or("--encoder-backend missing value")?)
            }
            "--bitrate-kbps" => {
                bitrate_kbps = Some(
                    args.next()
                        .ok_or("--bitrate-kbps missing value")?
                        .parse()
                        .map_err(|_| "--bitrate-kbps must be a number")?,
                )
            }
            // qos-framerate-priority defense-in-depth: translate already blocks illegal values; here FromStr Err →
            // parse_args Err → main exit(2) (never enters a session, preventing an oxmgr restart storm).
            "--degradation" => {
                degradation = Some(
                    args.next()
                        .ok_or("--degradation missing value")?
                        .parse()
                        .map_err(|e: String| e)?,
                )
            }
            "--content-hint" => {
                content_hint = Some(
                    args.next()
                        .ok_or("--content-hint missing value")?
                        .parse()
                        .map_err(|e: String| e)?,
                )
            }
            "--min-bitrate-kbps" => {
                min_bitrate_kbps = Some(
                    args.next()
                        .ok_or("--min-bitrate-kbps missing value")?
                        .parse()
                        .map_err(|_| "--min-bitrate-kbps must be a number")?,
                )
            }
            "--keyframe-interval" => {
                keyframe_interval = Some(
                    args.next()
                        .ok_or("--keyframe-interval missing value")?
                        .parse()
                        .map_err(|_| "--keyframe-interval must be a number")?,
                )
            }
            _ => return Err(format!("unknown argument: {arg}")),
        }
    }
    Ok(Args {
        stream: stream.ok_or("missing --stream")?,
        config: config.ok_or("missing --config")?,
        token: token.ok_or("missing --token")?,
        gateway,
        encoder_backend: encoder_backend.unwrap_or_else(|| "auto".into()),
        bitrate_kbps,
        degradation,
        content_hint,
        min_bitrate_kbps,
        keyframe_interval,
    })
}
/// gateway WS address (D2): `--gateway` argument > default local gateway.
fn gateway_url(gateway_arg: Option<&str>) -> String {
    gateway_arg.map(str::to_string).unwrap_or_else(|| "ws://127.0.0.1:17980/ws".to_string())
}
/// vision topic (bridge.rs B3 convention: `vision/<camera-id>` mirrors the camera id).
fn vision_topic(camera_id: &str) -> String {
    format!("vision/{camera_id}")
}

/// vision message-format gate: JSON payloads only (FrameMeta::FORMAT_JSON, same wire format as the E2 stats).
/// A pixel payload on the vision topic = publisher-side protocol violation; refuse to forward.
fn vision_meta_ok(meta: &FrameMeta) -> bool {
    meta.format == FrameMeta::FORMAT_JSON
}

/// transparent forwarding (D-H8 message-format decision): payload passed verbatim as DC text (streamer = pipe,
/// HMI parses, no re-encode — the frame-correlation ts_mono/seq are written into the payload JSON by the ROS
/// vision node, aligned with the camera frames it consumes). Non-UTF-8 payload = protocol violation, refuse to forward.
fn vision_payload_text(payload: &[u8]) -> Option<&str> {
    std::str::from_utf8(payload).ok()
}

/// vision transport B negotiation state (D-H8: separate PC, pure DC, no tracks, distinct from video transport A;
/// F1/F2 offerer mode — relayed to the cockpit side via the local gateway).
struct VisionNegotiation {
    signal: SignalSession,
    events: broadcast::Receiver<SignalEvent>,
    ice_rx: mpsc::UnboundedReceiver<RTCIceCandidate>,
    pc: RTCPeerConnection,
    dc: RTCDataChannel,
    /// remote candidates cached before the answer lands (libwebrtc add_ice_candidate is unreliable pre-negotiation).
    pending_ice: Vec<RTCIceCandidate>,
    remote_set: bool,
    /// negotiation deadline: degrade when the answer does not arrive (vision optional; video unaffected).
    deadline: tokio::time::Instant,
}

/// vision async events (aggregation of all transport B async sources).
enum VisionEvent {
    Signal(Result<SignalEvent, broadcast::error::RecvError>),
    Ice(Option<RTCIceCandidate>),
    Frame(Option<FrameRef>),
    /// negotiation deadline expired (no cockpit-side answer).
    Deadline,
}

/// Establish transport B (signaling + PC + DC "vision" + offer). Any step failing → None
/// (degrade to video-only; C15 logs every branch).
async fn setup_vision_dc(gateway: &str, src: &str, room: &str) -> Option<VisionNegotiation> {
    let signal = match SignalClient::new_gateway(gateway, src, room, PeerRole::Host).connect().await
    {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("vision signaling connect failed (vision forwarding disabled): {e}");
            return None;
        }
    };
    let events = signal.events();
    let factory = RTCPeerConnectionFactory::new();
    let pc = match factory.create_peer_connection(RTCConfiguration::default()).await {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("vision create_peer_connection failed: {e}");
            return None;
        }
    };
    let (ice_tx, ice_rx) = mpsc::unbounded_channel::<RTCIceCandidate>();
    pc.on_ice_candidate(move |candidate| {
        let _ = ice_tx.send(candidate);
    });
    let dc = match pc.create_data_channel(VISION_DC_LABEL, RTCDataChannelInit::default()).await {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!("vision create_data_channel failed: {e}");
            return None;
        }
    };
    let offer = match pc.create_offer(&Default::default()).await {
        Ok(o) => o,
        Err(e) => {
            tracing::warn!("vision create_offer failed: {e}");
            return None;
        }
    };
    if let Err(e) = pc.set_local_description(&offer).await {
        tracing::warn!("vision set_local_description failed: {e}");
        return None;
    }
    let offer_json = match serde_json::to_string(&offer) {
        Ok(j) => j,
        Err(e) => {
            tracing::warn!("vision offer serialization failed: {e}");
            return None;
        }
    };
    if let Err(e) = signal
        .send(SignalingMessage::Sdp { room_id: room.into(), target: None, sdp: offer_json })
        .await
    {
        tracing::warn!("vision failed to send offer: {e}");
        return None;
    }
    tracing::info!("vision offer sent (transport B, waiting for the cockpit-side answer)");
    Some(VisionNegotiation {
        signal,
        events,
        ice_rx,
        pc,
        dc,
        pending_ice: Vec::new(),
        remote_set: false,
        deadline: tokio::time::Instant::now() + VISION_NEGOTIATE_TIMEOUT,
    })
}

/// Aggregate transport B async sources (vision=None/vision_frames=None → hang forever, i.e. vision disabled).
async fn next_vision_event(
    vision: &mut Option<VisionNegotiation>,
    vision_frames: &Option<FrameStream>,
) -> VisionEvent {
    match (vision.as_mut(), vision_frames.as_ref()) {
        (Some(v), Some(frames)) => tokio::select! {
            ev = v.events.recv() => VisionEvent::Signal(ev),
            c = v.ice_rx.recv() => VisionEvent::Ice(c),
            f = frames.recv() => VisionEvent::Frame(f),
            _ = tokio::time::sleep_until(v.deadline) => VisionEvent::Deadline,
        },
        _ => std::future::pending::<VisionEvent>().await,
    }
}

/// Handle vision signaling events (answer/ICE/disconnect). true = continue, false = degrade and disable.
async fn handle_vision_signal(
    v: &mut VisionNegotiation,
    ev: Result<SignalEvent, broadcast::error::RecvError>,
) -> bool {
    match ev {
        Ok(SignalEvent::Message(SignalingMessage::Sdp { sdp, .. })) => {
            let desc = match serde_json::from_str::<RTCSessionDescription>(&sdp) {
                Ok(d) => d,
                Err(e) => {
                    tracing::warn!("vision answer parse failed: {e}");
                    return false;
                }
            };
            if desc.sdp_type != RTCSdpType::Answer {
                tracing::warn!(sdp_type = %desc.sdp_type, "vision got a non-answer Sdp, degrading");
                return false;
            }
            match v.pc.set_remote_description(&desc).await {
                Ok(()) => {
                    tracing::info!("vision answer set — negotiation complete");
                    v.remote_set = true;
                    // negotiation done: cancel the deadline (slow ICE / slow first frame no longer triggers degradation)
                    v.deadline = tokio::time::Instant::now() + Duration::from_secs(3600);
                    for c in v.pending_ice.drain(..) {
                        if let Err(e) = v.pc.add_ice_candidate(&c).await {
                            tracing::warn!("vision add_ice_candidate: {e}");
                        }
                    }
                    true
                }
                Err(e) => {
                    tracing::warn!("vision set_remote_description failed: {e}");
                    false
                }
            }
        }
        Ok(SignalEvent::Message(SignalingMessage::RTCIceCandidate {
            candidate,
            sdp_mid,
            sdp_mline_index,
            ..
        })) => {
            let c = RTCIceCandidate { candidate, sdp_mid, sdp_mline_index };
            if v.remote_set {
                if let Err(e) = v.pc.add_ice_candidate(&c).await {
                    tracing::warn!("vision add_ice_candidate: {e}");
                }
            } else {
                v.pending_ice.push(c);
            }
            true
        }
        Ok(SignalEvent::Message(_)) => true, // RoomJoined/other messages pass through and are ignored
        Ok(SignalEvent::Error(e)) => {
            tracing::warn!("vision signaling error: {e} — degrading to disabled");
            false
        }
        Ok(SignalEvent::Disconnected { reason }) => {
            tracing::warn!("vision signaling closed: {reason} — degrading to disabled");
            false
        }
        Ok(SignalEvent::Connected { .. }) => true,
        Ok(_) => true, // SignalEvent non_exhaustive
        Err(broadcast::error::RecvError::Lagged(n)) => {
            tracing::warn!("vision signaling events lagged by {n}");
            true
        }
        Err(broadcast::error::RecvError::Closed) => {
            tracing::warn!("vision signaling event stream closed — degrading to disabled");
            false
        }
    }
}

/// tight I420 payload validation (wire-format assumption: tight strides Y + U + V).
fn valid_i420(meta: &FrameMeta, payload_len: usize) -> bool {
    meta.format == FORMAT_I420
        && meta.width.is_multiple_of(2)
        && meta.height.is_multiple_of(2)
        && payload_len == (meta.width * meta.height * 3 / 2) as usize
}

/// Wait for SIGINT/SIGTERM (unix main path; other platforms only ctrl_c).
async fn shutdown_signal() -> std::io::Result<()> {
    #[cfg(unix)]
    {
        let mut sigterm =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = sigterm.recv() => {}
        }
    }
    #[cfg(not(unix))]
    tokio::signal::ctrl_c().await?;
    Ok(())
}

/// egress stats: logging (e2e evidence: bytes_sent/frames_encoded > 0, aligned with field pattern D4)
/// + FrameBus publish [`StreamerStats`] JSON to `stats/stream-<id>` (E2 data-plane monitoring,
///   additive; only monitoring subscribers consume; with no consumers the publish short-circuits at zero cost).
///   encode-time delta baseline (ΔtotalEncodeTime/ΔframesEncoded — the web stats panel's avg_encode_ms)
static LAST_ENCODE: std::sync::Mutex<Option<(f64, u64)>> = std::sync::Mutex::new(None);

/// codec id → W3C mimeType (the web panel's `codec.replace('video/','')` display convention).
fn codec_mime(codec: &str) -> String {
    format!("video/{}", codec.to_uppercase())
}

/// Build EncoderStatus (v2 protocol message — the server relay / web merge landed long ago; the host
/// sender was lost in the E3 multi-process split migration; restored here. Wire shape is pinned by unit tests).
#[allow(clippy::too_many_arguments)]
fn build_encoder_status(
    room_id: &str,
    peer_id: &str,
    codec: &str,
    backend: &str,
    encoder_implementation: Option<String>,
    frames_per_second: f64,
    frame_width: u32,
    frame_height: u32,
    avg_encode_ms: Option<f64>,
) -> SignalingMessage {
    SignalingMessage::EncoderStatus {
        room_id: room_id.into(),
        peer_id: peer_id.into(),
        codec: codec_mime(codec),
        encoder_backend: backend.into(),
        encoder_implementation,
        frames_per_second,
        frame_width,
        frame_height,
        avg_encode_ms,
    }
}

/// Consecutive signaling-send failure counter (≥SIGNAL_FAIL_MAX marks a zombie session → exit and rebuild;
/// a fallback channel independent of the H6 5001 notification — the notification's target-table entry may
/// already have been cleaned when the upstream dropped). Single-stream single-process form: a file-level static, same pattern as LAST_ENCODE.
static SIGNAL_FAIL_STREAK: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
const SIGNAL_FAIL_MAX: u32 = 3;
/// Upstream-unreachable signatures (gateway not connected to a server: room join rejected with 5001 / not-connected notice).
fn upstream_unavailable(err: &str) -> bool {
    err.contains("[5001]") || err.contains("gateway not connected")
}

async fn log_stats(
    session: &PushSession,
    bus: &FrameBus,
    topic: &FrameTopic,
    started: Instant,
    codec: &str,
    backend: &str,
) {
    let Some(pc) = session.peer_connection() else {
        return;
    };
    let stats = pc.sender_get_stats("video");
    let Some(o) = stats.iter().find_map(|s| match s {
        RTCStats::OutboundRtp(o) => Some(o),
        _ => None,
    }) else {
        return;
    };
    // encode-time delta (same as the old host's EncoderStatus — ΔtotalEncodeTime/ΔframesEncoded)
    let avg_encode_ms = {
        let mut guard = LAST_ENCODE.lock().unwrap_or_else(|e| e.into_inner());
        let now = (o.total_encode_time.unwrap_or(0.0), o.frames_encoded as u64);
        let avg = match *guard {
            Some((t0, f0)) if now.1 > f0 && now.0 >= t0 => {
                Some((now.0 - t0) * 1000.0 / (now.1 - f0) as f64)
            }
            _ => None,
        };
        *guard = Some(now);
        avg
    };
    tracing::info!(
        "streamer stats: bytes_sent={} frames_encoded={} frame={}x{} codec={} avg_encode_ms={:?} enc={:?}",
        o.bytes_sent,
        o.frames_encoded,
        o.frame_width,
        o.frame_height,
        codec,
        avg_encode_ms,
        o.encoder_implementation
    );
    // E2 additive: publish push state (FrameMeta::FORMAT_JSON marker; ts_mono = process-start
    // monotonic clock (C17 anchoring semantics; the monitoring side does not consume stats ts_mono, it is only a monotonic marker))
    let meta = FrameMeta {
        seq: STATS_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        format: FrameMeta::FORMAT_JSON,
        ts_mono_ns: started.elapsed().as_nanos() as u64,
        ..Default::default()
    };
    let payload = match serde_json::to_vec(&StreamerStats {
        bytes_sent: o.bytes_sent,
        frames_encoded: o.frames_encoded,
        frame_width: o.frame_width,
        frame_height: o.frame_height,
        codec: codec.to_string(),
        avg_encode_ms,
        encoder_implementation: o.encoder_implementation.clone(),
    }) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("stats serialization failed: {e}");
            return;
        }
    };
    if let Err(e) = bus.publish(topic, &payload, &meta) {
        tracing::warn!(topic = %topic.as_str(), "stats publish failed: {e}");
    }
    // EncoderStatus report (room broadcast, relayed to browser consumers in the same room; C15 logs failures).
    let msg = build_encoder_status(
        session.signal().room_id(),
        session.signal().peer_id(),
        codec,
        backend,
        o.encoder_implementation.clone(),
        o.frames_per_second,
        o.frame_width,
        o.frame_height,
        avg_encode_ms,
    );
    if let Err(e) = session.signal().send(msg).await {
        let streak = SIGNAL_FAIL_STREAK.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        tracing::warn!(streak, "encoder_status send failed: {e}");
    } else {
        SIGNAL_FAIL_STREAK.store(0, std::sync::atomic::Ordering::Relaxed);
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    mediaservo_host::init_logging("streamer");
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}\n{USAGE}");
            return ExitCode::from(2);
        }
    };

    // stream config (camera/codec default to id/h264) + camera config (fps)
    let cfg_text = match std::fs::read_to_string(&args.config) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("streamer: failed to read config {}: {e}", args.config.display());
            return ExitCode::from(1);
        }
    };
    let stream = match mediaservo_host::translate::stream_config(&cfg_text, &args.stream) {
        Ok(Some(s)) => s,
        Ok(None) => {
            eprintln!("streamer: no stream {} in config", args.stream);
            return ExitCode::from(1);
        }
        Err(e) => {
            eprintln!("streamer: {e}");
            return ExitCode::from(1);
        }
    };
    let cam = match mediaservo_host::translate::camera_config(&cfg_text, &stream.source) {
        Ok(Some(c)) => c,
        Ok(None) => {
            eprintln!(
                "streamer: source {} referenced by stream {} does not exist",
                stream.source, stream.id
            );
            return ExitCode::from(1);
        }
        Err(e) => {
            eprintln!("streamer: {e}");
            return ExitCode::from(1);
        }
    };
    // room convention (producer-label T3, §12-1 ruling: device room + stream label): push into the vehicle/device room;
    // stream identity is expressed via Produce.label = stream id (echoed back by NewProducer) — consumers pick streams by label.
    // PIT-140 v2 derived rooms (<vehicle>_<stream id>) are deprecated by this change (retired after one transition version, ruling §12-2).
    let vehicle_room = mediaservo_host::translate::signaling_room(&cfg_text)
        .ok()
        .flatten()
        .unwrap_or_else(|| "vehicle".to_string());
    let room = vehicle_room;

    // token → FrameBus attach → subscribe camera/<camera-id>
    let token_bytes = match std::fs::read(&args.token) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("streamer: failed to read token {}: {e}", args.token.display());
            return ExitCode::from(1);
        }
    };
    let (verifying_key, token) = match TokenFile::decode(&token_bytes) {
        Ok(kv) => kv,
        Err(e) => {
            eprintln!("streamer: token {} invalid: {e}", args.token.display());
            return ExitCode::from(1);
        }
    };
    let bus = match FrameBus::attach("", &token, &verifying_key) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("streamer: FrameBus attach failed: {e}");
            return ExitCode::from(1);
        }
    };
    let topic = FrameTopic::new(format!("camera/{}", cam.id));
    // D-H14 order-agnostic + SystemInFlux residue tolerance: subscription failure (capturer started
    // later / SHM-residue transient) retries every 3s up to 10 times (30s) — a single failure exiting would get oxmgr restarts driven to errored (PIT)
    let frames = loop {
        match bus.subscribe(&topic) {
            Ok(f) => break f,
            Err(e) => {
                eprintln!(
                    "streamer: subscribe {} failed (retry in 3s — capturer-late/SHM-residue tolerance): {e}",
                    topic.as_str()
                );
                std::thread::sleep(std::time::Duration::from_secs(3));
            }
        }
    };
    tracing::info!(topic = %topic.as_str(), "FrameBus subscribed");

    // F3 (D-H7/D-H8): subscribe to vision results vision/<camera-id> (published by the ROS vision node;
    // external node attach uses the Perception role + capability token, see acl.rs). Failure degrades (vision optional).
    let vision_topic = FrameTopic::new(vision_topic(&cam.id));
    let mut vision_frames: Option<FrameStream> = match bus.subscribe(&vision_topic) {
        Ok(f) => {
            tracing::info!(topic = %vision_topic.as_str(), "vision FrameBus subscribed");
            Some(f)
        }
        Err(e) => {
            tracing::warn!(
                "subscribe {} failed (vision forwarding disabled): {e}",
                vision_topic.as_str()
            );
            None
        }
    };

    // push session (reuses the field PushSession; D2: via the local gateway, no PSK)
    // self-reported src brand-derived (D269/T2 — display-only; gateway snapshot/StatusReport display string with no matching consumer;
    // default brand app_prefix="host-" → byte-identical to the old literal, zero behavior change)
    let app_prefix = mediaservo_common::brand::media_brand().app_prefix;
    let mut cfg = PushConfig::via_gateway(
        gateway_url(args.gateway.as_deref()),
        format!("{app_prefix}streamer-{}", stream.id),
        room.clone(),
    )
    // multi-stream same-room in the device room: peer_id must carry the stream suffix ("host-generator3"), else 6
    // streamers sharing peer_id="host" shadow each other → produce "Peer host not found"
    // (same shape as PIT-65, root cause of the 2026-10-09 generator3/5 black screens). Restarting the same stream =
    // shadowing one's own old session (takeover semantics, correct).
    .with_peer_suffix(&stream.id);
    cfg.framerate = cam.fps;
    // encode-parameter passthrough (streams config surface; None = field default 2000kbps/2s GOP)
    if let Some(k) = args.bitrate_kbps {
        cfg.bitrate_kbps = k;
    }
    // qos-framerate-priority: wire up the already-expanded primitives (no policy logic — the ruling lives in translate).
    if let Some(d) = args.degradation {
        cfg.degradation = d;
    }
    if let Some(h) = args.content_hint {
        cfg.content_hint = h;
    }
    if let Some(k) = args.min_bitrate_kbps {
        cfg.min_bitrate_kbps = Some(k);
    }
    if let Some(g) = args.keyframe_interval {
        cfg.keyframe_interval = g as u64;
    }
    // C fix: during an outage window a new connect straight eats 5001 → exiting immediately would cycle at
    // 1-2s per round and trip the oxmgr fuse (3/5min) → nobody re-lifts even after the server recovers. In-process
    // 10s backoff retries (up to ~6min), the same convention as gateway remote_loop connect_with_retry.
    let (mut session, mut events) = 'connect: {
        let mut last_err = String::from("never attempted");
        for attempt in 1..=36u32 {
            match PushSession::connect(cfg.clone()).await {
                Ok(se) => break 'connect se,
                Err(e) if upstream_unavailable(&e.to_string()) => {
                    tracing::warn!(
                        attempt,
                        "upstream not ready (gateway 5001), retrying connect in 10s: {e}"
                    );
                    last_err = e.to_string();
                    tokio::time::sleep(Duration::from_secs(10)).await;
                }
                Err(e) => {
                    eprintln!("streamer: PushSession connect failed: {e}");
                    return ExitCode::from(1);
                }
            }
        }
        eprintln!(
            "streamer: upstream not ready for ~6min, exiting for oxmgr to re-lift: {last_err}"
        );
        return ExitCode::from(1);
    };

    // F3 (D-H8): transport B — separate pure-DC PC (label "vision"), relayed to the cockpit side through
    // the same local gateway (F1/F2 offerer mode; mediasoup send/recv separation + SCTP/RTP scheduling on
    // one PC drags each other down → separate from video transport A). Failure degrades to video-only (vision is an optional overlay).
    let mut vision: Option<VisionNegotiation> = setup_vision_dc(
        &gateway_url(args.gateway.as_deref()),
        &format!("{app_prefix}streamer-{}-vision", stream.id),
        &room,
    )
    .await;
    if vision.is_none() {
        tracing::warn!("transport B unavailable — degrading to video-only push");
    }

    // first frame decides resolution (capturer fixed 1280x720; adapting per meta is steadier) → publish
    let first = match tokio::time::timeout(NO_FRAME_TIMEOUT, frames.recv()).await {
        Ok(Some(f)) => f,
        Ok(None) => {
            eprintln!("streamer: frame stream closed (capturer not running?)");
            return ExitCode::from(1);
        }
        Err(_) => {
            eprintln!(
                "streamer: no frames for {NO_FRAME_TIMEOUT:?} — capturer not started or exited"
            );
            return ExitCode::from(1);
        }
    };
    if !valid_i420(first.meta(), first.payload().len()) {
        eprintln!(
            "streamer: first frame invalid (format={} {}x{} payload={})",
            first.meta().format,
            first.meta().width,
            first.meta().height,
            first.payload().len()
        );
        return ExitCode::from(1);
    }
    cfg.width = first.meta().width;
    cfg.height = first.meta().height;
    let opts = PublishOptions {
        codec: stream.codec.clone(),
        encoder_backend: args.encoder_backend.clone(),
        label: Some(stream.id.clone()), // producer-label T3: the stream id is the label
    };
    if let Err(e) = session.publish_video(&cfg, &opts).await {
        eprintln!("streamer: publish_video failed: {e}");
        return ExitCode::from(1);
    }
    let sender = match session.video_sender() {
        Some(s) => s,
        None => {
            eprintln!("streamer: no video sender after publish");
            return ExitCode::from(1);
        }
    };
    println!(
        "streamer ready: stream={} topic={} {}x{}@{} codec={} room={} vision={}",
        stream.id,
        topic.as_str(),
        cfg.width,
        cfg.height,
        cam.fps,
        stream.codec,
        cfg.room,
        if vision.is_some() && vision_frames.is_some() { "on (transport B)" } else { "off" }
    );

    // E2 additive: push-state topic + monotonic clock origin (for stats publishing)
    let stats_topic = FrameTopic::new(format!("stats/stream-{}", stream.id));
    let started = Instant::now();
    let mut exit_code: u8 = 0;
    let mut last_stats = Instant::now();
    'run: loop {
        tokio::select! {
            sig = shutdown_signal() => match sig {
                Ok(()) => break 'run,
                Err(e) => {
                    eprintln!("streamer: signal handling failed: {e}");
                    exit_code = 1;
                    break 'run;
                }
            },
            ev = events.recv() => match ev {
                Some(SessionEvent::Error(e)) => {
                    // H6: upstream switch (gateway 5001) — this session's transport/producer are fully dead;
                    // exit for oxmgr to restart → a brand-new produce (same self-heal channel as Disconnected).
                    if e.to_string().contains("[5001]") {
                        // H6: passive 5001 during the server outage window — exiting immediately restarts every 1-2s
                        // and trips the oxmgr fuse; back off 15s to give the server restart time, then exit and rebuild.
                        tracing::error!("upstream switch (gateway 5001): {e} — exiting after a 15s backoff to re-produce");
                        tokio::time::sleep(std::time::Duration::from_secs(15)).await;
                        break 'run;
                    }
                    tracing::warn!("session error: {e}");
                }
                Some(SessionEvent::Disconnected { reason }) => {
                    tracing::error!("signal disconnected: {reason}");
                    exit_code = 1;
                    break 'run;
                }
                None => {
                    tracing::error!("session event stream closed");
                    exit_code = 1;
                    break 'run;
                }
                _ => {} // Message/Connected/TrackPublished ignored
            },
            frame = tokio::time::timeout(NO_FRAME_TIMEOUT, frames.recv()) => match frame {
                Ok(Some(f)) => {
                    let meta = f.meta();
                    if !valid_i420(meta, f.payload().len()) {
                        tracing::warn!(
                            seq = meta.seq,
                            "invalid frame (format={} {}x{} payload={})",
                            meta.format, meta.width, meta.height, f.payload().len()
                        );
                        continue;
                    }
                    // C17: timestamp passthrough — the capturer already anchors a monotonic clock (ts_mono_ns)
                    let ts_us = (meta.ts_mono_ns / 1000) as i64;
                    if let Err(e) = sender
                        .write_raw_i420_with_ts(
                            f.payload(),
                            meta.width,
                            meta.height,
                            Some(ts_us),
                        )
                        .await
                    {
                        tracing::warn!(seq = meta.seq, "write frame: {e}");
                    }
                    if last_stats.elapsed() >= STATS_INTERVAL {
                        log_stats(&session, &bus, &stats_topic, started, &stream.codec, &args.encoder_backend).await;
                        last_stats = Instant::now();
                        // zombie fallback: when the signaling WS is half-dead, events deliver nothing at all;
                        // only the send-failure streak is visible — exit and re-produce after 3 consecutive (≈6s).
                        if SIGNAL_FAIL_STREAK.load(std::sync::atomic::Ordering::Relaxed) >= SIGNAL_FAIL_MAX {
                            tracing::error!("signaling sends failed {} times in a row — zombie session, exiting to re-produce", SIGNAL_FAIL_MAX);
                            exit_code = 1;
                            break 'run;
                        }
                    }
                }
                Ok(None) => {
                    tracing::error!("frame stream closed (capturer exited?)");
                    exit_code = 1;
                    break 'run;
                }
                Err(_) => {
                    tracing::error!("no frames for {NO_FRAME_TIMEOUT:?} — capturer stopped, exiting for restart");
                    exit_code = 1;
                    break 'run;
                }
            },
            // F3: transport B events (vision DC; hangs harmlessly when disabled, not disturbing video)
            vis = next_vision_event(&mut vision, &vision_frames) => match vis {
                VisionEvent::Signal(ev) => {
                    let keep = match vision.as_mut() {
                        Some(v) => handle_vision_signal(v, ev).await,
                        None => true,
                    };
                    if !keep {
                        tracing::warn!("vision forwarding degraded to disabled (video unaffected)");
                        vision = None;
                        vision_frames = None;
                    }
                }
                VisionEvent::Ice(Some(c)) => {
                    let Some(v) = vision.as_mut() else { continue };
                    let msg = SignalingMessage::RTCIceCandidate {
                        room_id: format!("stream-{}", stream.id),
                        target: None,
                        candidate: c.candidate,
                        sdp_mid: c.sdp_mid,
                        sdp_mline_index: c.sdp_mline_index,
                    };
                    if let Err(e) = v.signal.send(msg).await {
                        tracing::warn!("vision ICE candidate upstream send failed: {e}");
                    }
                }
                VisionEvent::Ice(None) => {
                    tracing::warn!("vision ICE channel closed — degrading to disabled");
                    vision = None;
                    vision_frames = None;
                }
                VisionEvent::Frame(Some(f)) => {
                    let meta = f.meta();
                    if !vision_meta_ok(meta) {
                        tracing::warn!(seq = meta.seq, "vision payload not in JSON format (publisher protocol violation), dropped");
                        continue;
                    }
                    let Some(text) = vision_payload_text(f.payload()) else {
                        tracing::warn!(seq = meta.seq, "vision payload not UTF-8, dropped");
                        continue;
                    };
                    let Some(v) = vision.as_mut() else { continue };
                    if v.dc.state() != RTCDataChannelState::Open {
                        continue; // DC not Open (negotiating) — skip silently, no log spam
                    }
                    if let Err(e) = v.dc.send_text(text).await {
                        tracing::warn!(seq = meta.seq, "vision DC send failed: {e}");
                    }
                }
                VisionEvent::Frame(None) => {
                    tracing::warn!("vision frame stream closed — degrading to disabled");
                    vision = None;
                    vision_frames = None;
                }
                VisionEvent::Deadline => {
                    tracing::warn!(
                        "no cockpit-side answer within {VISION_NEGOTIATE_TIMEOUT:?} — vision forwarding disabled (video unaffected)"
                    );
                    vision = None;
                    vision_frames = None;
                }
            },
        }
    }

    // F3: close transport B (signal + PC; independent of video transport A)
    if let Some(v) = vision {
        if let Err(e) = v.signal.close().await {
            tracing::warn!("vision signal close: {e}");
        }
        v.pc.close().await;
    }

    if let Err(e) = session.close().await {
        tracing::warn!("close: {e}");
    }
    tracing::info!("streamer stopped (exit={exit_code})");
    ExitCode::from(exit_code)
}

#[cfg(test)]
mod tests {
    use super::{
        build_encoder_status, gateway_url, upstream_unavailable, vision_meta_ok,
        vision_payload_text, vision_topic,
    };
    use mediaservo_link::FrameMeta;

    #[test]
    fn upstream_unavailable_matches_gateway_5001_only() {
        // observed error string (verbatim from the test6 log)
        assert!(upstream_unavailable(
            "link: signal error: room join failed [5001]: gateway not connected to server",
        ));
        assert!(!upstream_unavailable("link: signal error: connect refused"));
        assert!(!upstream_unavailable("auth failed [4001]"));
    }

    #[test]
    fn gateway_url_defaults_to_local_gateway() {
        assert_eq!(gateway_url(None), "ws://127.0.0.1:17980/ws");
    }

    #[test]
    fn gateway_url_override_wins() {
        assert_eq!(gateway_url(Some("ws://127.0.0.1:18888/ws")), "ws://127.0.0.1:18888/ws");
    }

    #[test]
    fn vision_topic_mirrors_camera_id() {
        // bridge.rs B3 convention: vision/<camera-id> mirrors the camera id (single source for ROS bridge config)
        assert_eq!(vision_topic("cam0"), "vision/cam0");
        assert_eq!(vision_topic("front/raw"), "vision/front/raw");
    }

    #[test]
    fn vision_meta_ok_accepts_only_json_format() {
        let json = FrameMeta { format: FrameMeta::FORMAT_JSON, ..Default::default() };
        assert!(vision_meta_ok(&json), "a JSON payload must pass the gate");
        let i420 = FrameMeta { format: super::FORMAT_I420, ..Default::default() };
        assert!(!vision_meta_ok(&i420), "a pixel payload must not be forwarded as vision JSON");
    }

    #[test]
    fn vision_payload_text_is_transparent() {
        // D-H8 decision: transparent forwarding (streamer = pipe, HMI parses, no re-encode)
        let payload = br#"{"frame":{"seq":1,"ts_mono_ns":2},"objects":[]}"#;
        assert_eq!(
            vision_payload_text(payload),
            Some(r#"{"frame":{"seq":1,"ts_mono_ns":2},"objects":[]}"#)
        );
        assert_eq!(
            vision_payload_text(&[0xff, 0xfe]),
            None,
            "non-UTF-8 payload rejected (protocol violation)"
        );
    }

    #[test]
    fn encoder_status_wire_matches_web_contract() {
        // field names aligned verbatim with the sfu-client.ts msg.* consumption surface (the web-stream-stats break was precisely that nobody sent).
        let v = serde_json::to_value(build_encoder_status(
            "vehicle_test",
            "host-1",
            "h264",
            "software",
            Some("OpenH264".into()),
            30.0,
            1280,
            720,
            Some(3.1),
        ))
        .unwrap();
        assert_eq!(v["type"], "encoder_status");
        assert_eq!(v["room_id"], "vehicle_test");
        assert_eq!(v["codec"], "video/H264");
        assert_eq!(v["encoder_backend"], "software");
        assert_eq!(v["encoder_implementation"], "OpenH264");
        assert_eq!(v["frames_per_second"], 30.0);
        assert_eq!(v["frame_width"], 1280);
        assert_eq!(v["avg_encode_ms"], 3.1);
        let v2 = serde_json::to_value(build_encoder_status(
            "r", "p", "vp8", "auto", None, 0.0, 0, 0, None,
        ))
        .unwrap();
        assert!(v2.get("encoder_implementation").is_none() && v2.get("avg_encode_ms").is_none());
        assert_eq!(v2["codec"], "video/VP8");
    }
}
