//! Control-plane SFU-DC (client-dual-form S1) — host-controller's transport and command loop.
//!
//! Background (correction finalized 09-15): the old P2P form (controller=offerer, Sdp relay waiting for the cockpit answer)
//! became a dead end after the all-SFU decision (2026-08-25, server room.rs DeviceStream unification) —
//! Sdp/ICE of registered rooms are silently dropped by the frame filter. Real reference = field::session:
//! - Outbound ([`setup_send_side`]): the `PushSession::publish_video`
//!   CreateWebRtcTransport(Send)→TransportCreated→synthesized offer→DC→create_answer→
//!   ConnectWebRtcTransport(dtls)→CreateDataProducer sequence, DC-only (no add_track).
//! - Inbound ([`setup_recv_side`]): the Recv transport shape of `PullSession::subscribe`;
//!   S2d: consumer DC = an out-of-band negotiated channel created from the DataConsumed reply (official
//!   Chrome74.receiveDataChannel contract; the mediasoup proxy worker never sends DCEP on its behalf,
//!   in-band on_data_channel never fires in this domain).
//!   The cockpit control DataProducer arrives via `NewDataProducer` broadcast → `ConsumeData` →
//!   SCTP inbound DC on `pc.on_data_channel` (mediaservo-webrtc cross-backend in place).
//!
//! Command handling ([`handle_command`]): `parse_envelope` → [`Actuator::on_command`] →
//! `ControlAck{ack: seq, result}` written back to the "ack" DC (main path) + echo on the inbound
//! DC (same channel; both best-effort, C15 warn); side mirror FrameBus `control/cmd` / `control/ack`
//! (the bus is the mirror surface — attach/publish failures never block execution).
//!
//! Signaling-plane discipline: the synthesized remote SDP and ICE/DTLS message shapes mirror PushSession field by field (server
//! ICE-Lite: candidates are delivered inline with WebRtcTransportCreated, no RTCIceCandidate exchange round;
//! `transport_connected` (the Error{code:0} convention) is not a real error).

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use mediaservo_common::protocol::{
    ControlAck, ControlEnvelope, DtlsParameters, Fingerprint, IceCandidate, IceParameters,
    MediaKind, SctpStreamParameters, SignalingMessage, TransportDirection, control_hmac_verify,
    is_estop_cmd, parse_envelope,
};
use mediaservo_link::{FrameBus, FrameMeta, FrameTopic, SignalEvent, SignalSession};
use mediaservo_webrtc::data_channel::{
    RTCDataChannel, RTCDataChannelEvent, RTCDataChannelInit, RTCDataChannelState,
};
use mediaservo_webrtc::peer_connection::{
    RTCIceConnectionState, RTCIceServer, RTCIceTransportPolicy,
};
use mediaservo_webrtc::sdp::{RTCSdpType, RTCSessionDescription};
use mediaservo_webrtc::traits::PeerConnectionApi;
use mediaservo_webrtc::{
    RTCAnswerOptions, RTCConfiguration, RTCPeerConnection, RTCPeerConnectionFactory,
};
use tokio::sync::{broadcast, mpsc};

use crate::control::Actuator;

/// Command channel labels (D-H3: chassis/light reliable-ordered, gimbal partial-reliable).
pub const DEFAULT_LABELS: [&str; 3] = ["chassis", "gimbal", "light"];
/// Ack return DC label (controller builds its own DataProducer; the cockpit consumes it to receive ControlAck).
pub const ACK_LABEL: &str = "ack";
/// DataProducer sub-protocol name (the mediasoup protocol field, fixed "sctp" in v1).
const DATA_PROTOCOL: &str = "sctp";
/// SFU peer key (shares "host" with the streamer's publish — transport_id is bound explicitly against crossed wires, C1 convention).
const SFU_PEER_ID: &str = "host";
/// FrameBus mirror topics (command inbound / ack outbound; payload = envelope / ack raw JSON).
pub const TOPIC_CMD: &str = "control/cmd";
pub const TOPIC_ACK: &str = "control/ack";
/// Wait before exiting on ICE Failed self-heal (PIT-87: exit after state convergence; deploy-side restart_policy brings it back up).
const ICE_FAILED_WAIT: Duration = Duration::from_secs(1);

/// Channel reliability (D-H3): chassis/light reliable-ordered (estop/switch-class commands);
/// gimbal partial-reliable (continuous gimbal adjustment tolerates frame loss, latency first).
pub fn channel_init(label: &str) -> RTCDataChannelInit {
    match label {
        "gimbal" => {
            RTCDataChannelInit { ordered: false, max_retransmits: Some(5), ..Default::default() }
        }
        _ => RTCDataChannelInit::default(), // chassis / light / ack: reliable ordered
    }
}

/// SCTP stream params of outbound DCs: stream_id takes the DC id libwebrtc actually assigned (mediasoup Direct
/// transport does no remap, DataProducer.sctpStreamParameters.streamId == endpoint DC id);
/// ordered/retransmission share their source with [`channel_init`] (announce and real channel behavior never drift).
pub fn sctp_stream_params(label: &str, id: i32) -> Result<SctpStreamParameters, String> {
    let stream_id = u16::try_from(id).map_err(|_| {
        format!("DC {label} id={id} unusable as SCTP stream_id (DataChannel has no assigned id)")
    })?;
    let init = channel_init(label);
    Ok(SctpStreamParameters {
        stream_id,
        ordered: init.ordered,
        max_packet_life_time: init.max_retransmit_time.and_then(|v| u16::try_from(v).ok()),
        max_retransmits: init.max_retransmits.and_then(|v| u16::try_from(v).ok()),
    })
}

/// Synthesize a DC-only remote offer (application m-line) from mediasoup transport params.
/// Shape aligned field by field with field::sfu::build_remote_sdp (PIT-48: a=candidate must follow m=;
/// mDNS .local candidates skipped; a=ice-lite + actpass → local answer=DTLS client).
pub fn build_dc_remote_sdp(
    ice: &IceParameters,
    dtls: &DtlsParameters,
    candidates: Option<&Vec<IceCandidate>>,
) -> String {
    let Some(fp) = dtls.fingerprints.first() else {
        tracing::warn!(
            "dtls_parameters carries no fingerprint — using placeholder (DTLS will fail, logged)"
        );
        return String::new();
    };
    let conn_ip = candidates
        .and_then(|cs| cs.iter().find(|c| !c.ip.contains(".local")))
        .map(|c| c.ip.clone())
        .unwrap_or_else(|| "0.0.0.0".to_string());

    let mut lines = vec![
        "v=0".to_string(),
        "o=- 0 0 IN IP4 0.0.0.0".to_string(),
        "s=-".to_string(),
        "t=0 0".to_string(),
        "a=group:BUNDLE data".to_string(),
        "a=ice-lite".to_string(),
        format!("a=ice-ufrag:{}", ice.username_fragment),
        format!("a=ice-pwd:{}", ice.password),
        format!("a=fingerprint:{} {}", fp.algorithm.to_lowercase(), fp.value),
        "a=setup:actpass".to_string(),
        // libmediasoupclient data-section shape (application/UDP/DTLS/SCTP)
        "m=application 9 UDP/DTLS/SCTP webrtc-datachannel".to_string(),
        format!("c=IN IP4 {conn_ip}"),
        "a=mid:data".to_string(),
        "a=sctp-port:5000".to_string(),
        "a=max-message-size:262144".to_string(),
    ];

    if let Some(cands) = candidates {
        for c in cands {
            if c.ip.contains(".local") {
                continue; // skip mDNS
            }
            let ctype = match c.candidate_type.as_str() {
                "host" | "srflx" | "prflx" | "relay" => c.candidate_type.as_str(),
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

/// ControlAck sink (production = DC double-write; tests = in-memory collection).
#[async_trait]
pub trait AckSink: Send + Sync {
    async fn write(&self, text: &str) -> Result<(), String>;
}

/// Production impl: ack DC main path + echo on the inbound command DC (both best-effort, C15 warn).
pub struct DcAckSink {
    /// "ack" outbound DC slot (Arc: the routing task may be decoupled from ack setup — late binding is fine).
    pub ack: Arc<Mutex<Option<RTCDataChannel>>>,
    /// The inbound DC that triggered this ack (echo channel).
    pub inbound: RTCDataChannel,
}

#[async_trait]
impl AckSink for DcAckSink {
    async fn write(&self, text: &str) -> Result<(), String> {
        let ack_dc = self.ack.lock().unwrap_or_else(|p| p.into_inner()).clone();
        match ack_dc {
            Some(dc) => {
                if let Err(e) = dc.send_text(text).await {
                    tracing::warn!(label = %dc.label(), "ack DC write-back failed: {e}");
                }
            }
            None => {
                tracing::warn!("ack DC not established — main-path ack lost (echo still attempted)")
            }
        }
        if self.inbound.state() == RTCDataChannelState::Open
            && let Err(e) = self.inbound.send_text(text).await
        {
            tracing::warn!(label = %self.inbound.label(), "inbound DC echo failed: {e}");
        }
        Ok(())
    }
}

/// Test impl: collect written ack texts (for no-network env → ack roundtrip assertions).
#[derive(Clone, Default)]
pub struct CollectAckSink {
    pub items: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl AckSink for CollectAckSink {
    async fn write(&self, text: &str) -> Result<(), String> {
        self.items.lock().unwrap_or_else(|p| p.into_inner()).push(text.to_string());
        Ok(())
    }
}

/// FrameBus mirror publish (side surface: any failure only warns, never blocks execution, C15).
pub fn publish_bus(bus: Option<&FrameBus>, topic: &str, payload: &[u8]) {
    let Some(bus) = bus else { return };
    if let Err(e) = bus.publish(&FrameTopic::new(topic), payload, &FrameMeta::default()) {
        tracing::warn!(topic, "FrameBus mirror publish failed (execution unaffected): {e}");
    }
}

/// Single command handling: parse_envelope → Actuator::on_command → ControlAck write-back + bus mirror.
/// S4/a4 T3.5: command execution policy (estop signature gate + actuation audit landing point).
/// Rules (no self-lock migration form): hmac_key unset = verification disabled (estop passes + WARN trail);
/// once configured, estop must verify; a missing sig passes only when `allow_unsigned_estop=true`.
#[derive(Debug, Clone, Default)]
pub struct CommandPolicy {
    pub hmac_key: Option<String>,
    pub allow_unsigned_estop: bool,
    /// act-then-audit jsonl sink (None = tracing line only, never blocks execution).
    pub actuation_log: Option<std::path::PathBuf>,
}

impl CommandPolicy {
    /// env-driven construction (single deploy-side point):
    /// `MEDIASERVO_CONTROL_HMAC_KEY_FILE` (**recommended**, G13 file form 0600, same discipline and single source as cockpit client-c
    /// `hmac_key_file`) takes precedence over `MEDIASERVO_CONTROL_HMAC_KEY` (plaintext form,
    /// migration compat); all unset = verification disabled. A file that is **configured but unreadable = panic and die early** (fail-fast:
    /// silently degrading a security feature to the unset form is unacceptable — the opposite direction from the cockpit's "bad path does not block the session": the vehicle-side
    /// key IS the verification gate itself, silent pass = installing a door and leaving it unlocked). `CONTROL_ALLOW_UNSIGNED_ESTOP=1` escape hatch;
    /// `CONTROL_ACTUATION_LOG` unset = tracing line only.
    #[must_use]
    pub fn from_env() -> Self {
        let key_file =
            std::env::var("MEDIASERVO_CONTROL_HMAC_KEY_FILE").ok().filter(|v| !v.is_empty());
        Self {
            hmac_key: match key_file {
                Some(p) => Some(
                    mediaservo_common::protocol::control_hmac_key_from_file(&p)
                        .unwrap_or_else(|e| panic!("controller estop key unavailable: {e}")),
                ),
                None => std::env::var("MEDIASERVO_CONTROL_HMAC_KEY").ok().filter(|v| !v.is_empty()),
            },
            allow_unsigned_estop: std::env::var("CONTROL_ALLOW_UNSIGNED_ESTOP").as_deref()
                == Ok("1"),
            actuation_log: std::env::var("CONTROL_ACTUATION_LOG")
                .ok()
                .filter(|v| !v.is_empty())
                .map(std::path::PathBuf::from),
        }
    }
}

/// estop verification verdict matrix (pure function, testable).
#[must_use]
pub fn estop_verdict(policy: &CommandPolicy, env: &ControlEnvelope) -> (&'static str, bool) {
    // (sig_state, whether execution is allowed)
    if !is_estop_cmd(&env.cmd) {
        return ("n/a", true);
    }
    match (policy.hmac_key.as_deref(), env.sig.as_deref()) {
        (None, Some(_)) => ("signed-unconfigured", true),
        (None, None) => ("unsigned-unconfigured", true),
        (Some(_), Some(sig)) => {
            if control_hmac_verify(policy.hmac_key.as_deref().unwrap_or_default(), env, sig) {
                ("ok", true)
            } else {
                ("invalid", false)
            }
        }
        (Some(_), None) => {
            if policy.allow_unsigned_estop {
                ("absent-allowed", true)
            } else {
                ("absent", false)
            }
        }
    }
}

pub async fn handle_command(
    label: &str,
    data: &[u8],
    actuator: &dyn Actuator,
    bus: Option<&FrameBus>,
    sink: &dyn AckSink,
    policy: &CommandPolicy,
) {
    let env = match parse_envelope(data) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!(label, len = data.len(), "envelope parse failed: {e}");
            return;
        }
    };
    // S4/a4: e-stop signature gate (refusal before execution = semantics; refusals also ack err + write an audit line).
    let (sig_state, allow) = estop_verdict(policy, &env);
    if !allow {
        tracing::warn!(
            channel = %label,
            cmd = %env.cmd,
            seq = env.seq,
            "S4: e-stop signature verification failed, execution refused (sig={sig_state})"
        );
        actuation_audit(policy, label, &env, sig_state, "rejected");
        let json = serde_json::to_string(&ControlAck::err(env.seq, "estop_signature_rejected"))
            .unwrap_or_default();
        let _ = sink.write(&json).await;
        return;
    }
    if sig_state == "unsigned-unconfigured" && is_estop_cmd(&env.cmd) {
        tracing::warn!(
            "S4: estop arrived but hmac_key is unset — passing (deployment gap, see actuation audit)"
        );
    }
    publish_bus(bus, TOPIC_CMD, data);
    let mut exec_state = "ok";
    let ack = match actuator.on_command(label, &env) {
        Ok(result) => ControlAck::ok(env.seq, result),
        Err(e) => {
            exec_state = "error";
            // C15: error branches must log (the error reply still goes to the peer; local observability cannot be lost)
            tracing::warn!(
                channel = %label,
                cmd = %env.cmd,
                seq = env.seq,
                error = %e,
                "actuator command failed"
            );
            ControlAck::err(env.seq, e)
        }
    };
    actuation_audit(policy, label, &env, sig_state, exec_state);
    let json = match serde_json::to_string(&ack) {
        Ok(j) => j,
        Err(e) => {
            tracing::warn!(channel = %label, seq = env.seq, "ControlAck serialization failed: {e}");
            return;
        }
    };
    publish_bus(bus, TOPIC_ACK, json.as_bytes());
    if let Err(e) = sink.write(&json).await {
        tracing::warn!(channel = %label, seq = env.seq, "ack write-back failed: {e}");
    }
}

/// act-then-audit: jsonl (file written only when the env path is explicit) + an always-emitted tracing line; IO errors are tolerated
/// (the audit never blocks execution — seat-3 ruling).
fn actuation_audit(
    policy: &CommandPolicy,
    label: &str,
    env: &ControlEnvelope,
    sig: &str,
    exec: &str,
) {
    let ts_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    tracing::info!(
        channel = %label,
        cmd = %env.cmd,
        seq = env.seq,
        sig,
        exec,
        ts_ms,
        "actuation"
    );
    let Some(path) = &policy.actuation_log else { return };
    let line = serde_json::json!({
        "ts_ms": ts_ms, "channel": label, "cmd": env.cmd, "seq": env.seq,
        "sig": sig, "exec": exec,
    });
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{line}"); // audit write failure tolerated (execution already happened)
    }
}

/// Single-channel routing: spool recv → [`handle_command`] → ack (ack DC main path + same-channel echo).
pub async fn route_dc(
    mut dc: RTCDataChannel,
    actuator: Arc<dyn Actuator>,
    bus: Option<Arc<FrameBus>>,
    ack: Arc<Mutex<Option<RTCDataChannel>>>,
    mut purge: mpsc::UnboundedReceiver<()>,
    policy: Arc<CommandPolicy>,
) {
    let label = dc.label().to_string();
    let mut rx = dc.spool().await;
    tracing::info!(label, "DC routing started");
    loop {
        tokio::select! {
            ev = rx.recv() => match ev {
                Some(RTCDataChannelEvent::Open) => tracing::info!(label, "DC open"),
                Some(RTCDataChannelEvent::Closed) => {
                    tracing::info!(label, "DC closed");
                    break;
                }
                Some(RTCDataChannelEvent::Error(e)) => tracing::warn!(label, "DC error: {e}"),
                Some(RTCDataChannelEvent::Message(m)) => {
                    let sink = DcAckSink {
                        ack: ack.clone(),
                        inbound: dc.clone(),
                    };
                    handle_command(&label, &m.data, actuator.as_ref(), bus.as_deref(), &sink, &policy)
                        .await;
                }
                None => break,
            },
            // S4': the upstream DataProducer died (directed by the main loop) → close locally to release the libwebrtc
            // stream id occupation (otherwise zombie DC = the `StreamId reserved` consumption ceiling).
            _ = purge.recv() => {
                tracing::info!(label, "S4': upstream DataProducer dead — consumer DC self-teardown");
                dc.close().await;
                break;
            }
        }
    }
}

/// Transport establishment result (the WebRtcTransportCreated payload quadruple).
struct TransportCreated {
    transport_id: String,
    ice: IceParameters,
    dtls: DtlsParameters,
    candidates: Option<Vec<IceCandidate>>,
}

/// A cockpit data producer awaiting consumption (cache of broadcasts smuggled in during setup, guarding against no-replay races).
#[derive(Clone)]
struct PendingProducer {
    data_producer_id: String,
    label: String,
}

/// Consume signaling until WebRtcTransportCreated; NewDataProducer arrivals go into pending (never lost).
/// `events` must be subscribed before any request is sent (broadcast has no history replay — same as field session.rs:137).
async fn await_transport_created(
    events: &mut broadcast::Receiver<SignalEvent>,
    pending: &mut Vec<PendingProducer>,
) -> Result<TransportCreated, String> {
    loop {
        match events.recv().await {
            Ok(SignalEvent::Message(SignalingMessage::WebRtcTransportCreated {
                transport_id,
                ice_parameters,
                dtls_parameters,
                ice_candidates,
                ..
            })) => {
                return Ok(TransportCreated {
                    transport_id,
                    ice: ice_parameters,
                    dtls: dtls_parameters,
                    candidates: ice_candidates,
                });
            }
            Ok(SignalEvent::Message(SignalingMessage::NewDataProducer {
                data_producer_id,
                label,
                ..
            })) => {
                pending.push(PendingProducer { data_producer_id, label });
            }
            // transport_connected is the Connect confirmation (server convention Error{code:0}, not a real error)
            Ok(SignalEvent::Message(SignalingMessage::Error { message, .. }))
                if message == "transport_connected" => {}
            Ok(SignalEvent::Message(SignalingMessage::Error { code, message })) => {
                return Err(format!("SFU error [{code}]: {message}"));
            }
            Ok(SignalEvent::Disconnected { reason }) => {
                return Err(format!("signaling dropped during transport setup: {reason}"));
            }
            Ok(SignalEvent::Connected { .. }) => {}
            Ok(SignalEvent::Error(e)) => return Err(format!("signaling error: {e}")),
            Ok(_) => {} // other messages ignored (NewProducer/RoomJoined etc., same field convention)
            Err(_) => return Err("signaling event stream closed during transport setup".into()),
        }
    }
}

/// Consume signaling until DataProducerCreated (returns the producer id); cache/exemption rules same as
/// [`await_transport_created`]。
async fn await_data_producer_created(
    events: &mut broadcast::Receiver<SignalEvent>,
    pending: &mut Vec<PendingProducer>,
) -> Result<String, String> {
    loop {
        match events.recv().await {
            Ok(SignalEvent::Message(SignalingMessage::DataProducerCreated {
                data_producer_id,
                ..
            })) => return Ok(data_producer_id),
            Ok(SignalEvent::Message(SignalingMessage::NewDataProducer {
                data_producer_id,
                label,
                ..
            })) => {
                pending.push(PendingProducer { data_producer_id, label });
            }
            Ok(SignalEvent::Message(SignalingMessage::Error { message, .. }))
                if message == "transport_connected" => {}
            Ok(SignalEvent::Message(SignalingMessage::Error { code, message })) => {
                return Err(format!("DataProducer creation rejected [{code}]: {message}"));
            }
            Ok(SignalEvent::Disconnected { reason }) => {
                return Err(format!("signaling dropped during DataProducer setup: {reason}"));
            }
            Ok(SignalEvent::Connected { .. }) => {}
            Ok(SignalEvent::Error(e)) => return Err(format!("signaling error: {e}")),
            Ok(_) => {}
            Err(_) => return Err("signaling event stream closed during DataProducer setup".into()),
        }
    }
}

/// CreateWebRtcTransport(direction) + PC setup + set_remote (synthesized offer).
/// The answer is not sent here — the outbound side must first create_data_channel and the inbound side must first
/// register on_data_channel (late registration loses the first event, a PullSession lesson); hence split into [`connect_answer`].
async fn open_transport(
    signal: &SignalSession,
    events: &mut broadcast::Receiver<SignalEvent>,
    pending: &mut Vec<PendingProducer>,
    direction: TransportDirection,
) -> Result<(RTCPeerConnection, TransportCreated), String> {
    let dir_tag = format!("{direction:?}");
    signal
        .send(SignalingMessage::CreateWebRtcTransport {
            room_id: signal.room_id().to_string(),
            peer_id: SFU_PEER_ID.into(),
            direction,
        })
        .await
        .map_err(|e| format!("CreateWebRtcTransport({dir_tag}): {e}"))?;
    let t = await_transport_created(events, pending).await?;

    let factory = RTCPeerConnectionFactory::new();
    let pc = factory
        .create_peer_connection(RTCConfiguration {
            ice_servers: Vec::<RTCIceServer>::new(), // SFU: mediasoup ICE-Lite, no STUN needed
            ice_transport_type: RTCIceTransportPolicy::All,
        })
        .await
        .map_err(|e| format!("create_peer_connection: {e}"))?;

    let remote_sdp = build_dc_remote_sdp(&t.ice, &t.dtls, t.candidates.as_ref());
    pc.set_remote_description(&RTCSessionDescription::new(RTCSdpType::Offer, remote_sdp))
        .await
        .map_err(|e| format!("set_remote_description: {e}"))?;
    Ok((pc, t))
}

/// Negotiation closure: create_answer → set_local → ConnectWebRtcTransport(local DTLS fingerprint,
/// role=client). Same as PushSession step 6; mediasoup direct transports carry no upstream SDP,
/// the answer only feeds the local state machine (the server consumes only the connect fingerprint + CreateDataProducer params).
async fn connect_answer(
    signal: &SignalSession,
    pc: &RTCPeerConnection,
    t: &TransportCreated,
) -> Result<(), String> {
    let answer =
        pc.create_answer(&RTCAnswerOptions).await.map_err(|e| format!("create_answer: {e}"))?;
    tracing::debug!("controller answer SDP:\n{}", answer.sdp);
    pc.set_local_description(&answer).await.map_err(|e| format!("set_local_description: {e}"))?;
    let fp_hex =
        pc.local_dtls_fingerprint().ok_or_else(|| "no local DTLS fingerprint".to_string())?;
    signal
        .send(SignalingMessage::ConnectWebRtcTransport {
            room_id: signal.room_id().to_string(),
            peer_id: SFU_PEER_ID.into(),
            transport_id: t.transport_id.clone(),
            dtls_parameters: DtlsParameters {
                fingerprints: vec![Fingerprint { algorithm: "sha-256".to_string(), value: fp_hex }],
                role: "client".to_string(),
            },
        })
        .await
        .map_err(|e| format!("ConnectWebRtcTransport: {e}"))?;
    Ok(())
}

/// Outbound setup: Send transport → per label create_data_channel (incl. "ack", before the answer) →
/// answer/connect → per-DC CreateDataProducer announce. Returns (pc, set of self-built producer ids).
#[allow(clippy::too_many_arguments)] // SFU-DC connection parameter group (S1 shape); bundling = a separate refactor
async fn setup_send_side(
    signal: &SignalSession,
    events: &mut broadcast::Receiver<SignalEvent>,
    pending: &mut Vec<PendingProducer>,
    labels: &[String],
    ack: Arc<Mutex<Option<RTCDataChannel>>>,
    actuator: &Arc<dyn Actuator>,
    bus: &Option<Arc<FrameBus>>,
    policy: &Arc<CommandPolicy>,
) -> Result<(RTCPeerConnection, HashSet<String>), String> {
    let (pc, t) = open_transport(signal, events, pending, TransportDirection::Send).await?;
    let mut dcs = Vec::with_capacity(labels.len());
    for label in labels {
        let dc = pc
            .create_data_channel(label, channel_init(label))
            .await
            .map_err(|e| format!("create_data_channel {label}: {e}"))?;
        if label == ACK_LABEL {
            *ack.lock().unwrap_or_else(|p| p.into_inner()) = Some(dc.clone());
        }
        // outbound channels also get routing attached (defensive: if the peer writes back through this channel it still runs the unified command chain)
        // self-built outbound DCs need no directed teardown: tx is deliberately forgotten = the channel lives forever and never sends
        // (the select purge arm stays parked; the 4-byte leak is an explicit contract).
        let (purge_tx, purge_rx) = mpsc::unbounded_channel();
        std::mem::forget(purge_tx);
        tokio::spawn(route_dc(
            dc.clone(),
            actuator.clone(),
            bus.clone(),
            ack.clone(),
            purge_rx,
            policy.clone(),
        ));
        dcs.push((label.clone(), dc));
    }
    connect_answer(signal, &pc, &t).await?;
    let mut own = HashSet::new();
    for (label, dc) in &dcs {
        signal
            .send(SignalingMessage::CreateDataProducer {
                room_id: signal.room_id().to_string(),
                peer_id: SFU_PEER_ID.into(),
                transport_direction: TransportDirection::Send,
                label: label.clone(),
                protocol: DATA_PROTOCOL.into(),
                sctp_stream_parameters: Some(sctp_stream_params(dc.label(), dc.id())?),
                transport_id: Some(t.transport_id.clone()),
            })
            .await
            .map_err(|e| format!("CreateDataProducer {label}: {e}"))?;
        let dp_id = await_data_producer_created(events, pending).await?;
        tracing::info!(label = %label, data_producer_id = %dp_id, "controller DataProducer established");
        own.insert(dp_id);
    }
    Ok((pc, own))
}

/// Inbound setup: Recv transport → answer/connect. Since S2d on_data_channel is no longer registered
/// (in-band DCEP never arrives in the mediasoup proxy domain); consumer channels are created in negotiated form
/// by the main loop from the server's reply params upon DataConsumed (request-side closure of [`consume_data`]).
async fn setup_recv_side(
    signal: &SignalSession,
    events: &mut broadcast::Receiver<SignalEvent>,
    pending: &mut Vec<PendingProducer>,
) -> Result<(RTCPeerConnection, String), String> {
    let (pc, t) = open_transport(signal, events, pending, TransportDirection::Recv).await?;
    connect_answer(signal, &pc, &t).await?;
    Ok((pc, t.transport_id))
}

/// Consume one cockpit data producer (ConsumeData; the DataConsumed reply lets the main loop build a negotiated DC, S2d).
async fn consume_data(signal: &SignalSession, recv_transport_id: &str, dp_id: &str) {
    if let Err(e) = signal
        .send(SignalingMessage::ConsumeData {
            room_id: signal.room_id().to_string(),
            peer_id: SFU_PEER_ID.into(),
            transport_direction: TransportDirection::Recv,
            data_producer_id: dp_id.to_string(),
            transport_id: Some(recv_transport_id.to_string()),
        })
        .await
    {
        tracing::warn!(data_producer_id = %dp_id, "ConsumeData send failed: {e}");
    }
}

/// Controller configuration (v1: default three command channels + ack; the label set is overridable for fixtures/evolution).
#[derive(Debug, Clone)]
pub struct ControllerConfig {
    pub labels: Vec<String>,
}

impl Default for ControllerConfig {
    fn default() -> Self {
        let mut labels: Vec<String> = DEFAULT_LABELS.iter().map(|s| s.to_string()).collect();
        labels.push(ACK_LABEL.to_string());
        Self { labels }
    }
}

/// Main loop: dual-side transport setup → drain the setup-time cache → event loop (NewDataProducer consume /
/// DataConsumed·DataProducerCreated logging / signaling drop·ICE Failed self-heal exit).
/// Returns the process exit code (0 = graceful exit, 1 = failure exit awaiting bring-up).
pub async fn control_loop(
    signal: SignalSession,
    cfg: ControllerConfig,
    actuator: Arc<dyn Actuator>,
    bus: Option<Arc<FrameBus>>,
    policy: CommandPolicy,
) -> u8 {
    let policy = Arc::new(policy);
    // subscribe to events before sending any request (broadcast has no history replay)
    let mut events = signal.events();
    let mut pending: Vec<PendingProducer> = Vec::new();
    let ack: Arc<Mutex<Option<RTCDataChannel>>> = Arc::new(Mutex::new(None));

    let (send_pc, own) = match setup_send_side(
        &signal,
        &mut events,
        &mut pending,
        &cfg.labels,
        ack.clone(),
        &actuator,
        &bus,
        &policy,
    )
    .await
    {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(
                "controller: outbound (Send transport + DataProducer) setup failed: {e}"
            );
            return 1;
        }
    };
    let (recv_pc, recv_tid) = match setup_recv_side(&signal, &mut events, &mut pending).await {
        Ok(v) => v,
        Err(e) => {
            tracing::error!("controller: inbound (Recv transport) setup failed: {e}");
            send_pc.close().await;
            return 1;
        }
    };
    // own echo dedup: the server broadcasts NewDataProducer to the whole room (including itself) — skip self-built ids
    let mut consumed: HashSet<String> = own.clone();
    // S4': dp_id → directed teardown channel for the consumer DC (send when ProducerClosed(Data) arrives).
    let mut consumer_purge: HashMap<String, mpsc::UnboundedSender<()>> = HashMap::new();

    // NewDataProducers smuggled in during setup: drain and consume
    for np in pending.drain(..) {
        if consumed.insert(np.data_producer_id.clone()) {
            tracing::info!(
                label = %np.label,
                data_producer_id = %np.data_producer_id,
                "controller consuming cockpit DataProducer (setup-time cache)"
            );
            consume_data(&signal, &recv_tid, &np.data_producer_id).await;
        }
    }

    // ICE Failed self-heal (PIT-87: either side Failed → exit after state convergence, awaiting bring-up)
    let (fail_tx, mut fail_rx) = mpsc::unbounded_channel::<()>();
    for pc in [&send_pc, &recv_pc] {
        let tx = fail_tx.clone();
        pc.on_ice_connection_state_change(move |state| {
            if state == RTCIceConnectionState::Failed {
                tracing::error!("ICE Failed — exiting for restart (PIT-87 self-heal)");
                let _ = tx.send(());
            }
        });
    }

    println!("controller ready: room={} labels={}", signal.room_id(), cfg.labels.join(","));

    let mut exit_code: u8 = 0;
    'run: loop {
        tokio::select! {
            _ = wait_shutdown() => break 'run,
            _ = fail_rx.recv() => {
                tokio::time::sleep(ICE_FAILED_WAIT).await;
                exit_code = 1;
                break 'run;
            }
            ev = events.recv() => match ev {
                Ok(SignalEvent::Message(SignalingMessage::NewDataProducer {
                    data_producer_id,
                    label,
                    ..
                })) => {
                    if consumed.insert(data_producer_id.clone()) {
                        tracing::info!(
                            label = %label,
                            data_producer_id = %data_producer_id,
                            "controller consuming cockpit DataProducer"
                        );
                        consume_data(&signal, &recv_tid, &data_producer_id).await;
                    }
                }
                Ok(SignalEvent::Message(SignalingMessage::DataConsumed {
                    data_consumer_id,
                    data_producer_id,
                    sctp_stream_parameters,
                    label,
                    protocol,
                    ..
                })) => {
                    // S2d official contract: the consumer receives forwarded messages via an out-of-band negotiated DC
                    // (id = the stream_id assigned by the worker). Missing params = old server; fail loudly (C15).
                    let Some(sp) = sctp_stream_parameters else {
                        tracing::error!(
                            data_consumer_id,
                            "DataConsumed lacks sctp params (server too old?) — cannot create negotiated DC"
                        );
                        continue;
                    };
                    let init = RTCDataChannelInit {
                        negotiated: true,
                        id: sp.stream_id as i32,
                        ordered: sp.ordered,
                        max_retransmit_time: sp.max_packet_life_time.map(i32::from),
                        max_retransmits: sp.max_retransmits.map(i32::from),
                        protocol,
                    };
                    match recv_pc.create_data_channel(&label, init).await {
                        Ok(dc) => {
                            tracing::info!(
                                data_consumer_id,
                                data_producer_id,
                                label,
                                stream_id = sp.stream_id,
                                "DataConsumed: negotiated consumer DC created, commands now flowing"
                            );
                            let (a, b, k) = (actuator.clone(), bus.clone(), ack.clone());
                            let (purge_tx, purge_rx) = mpsc::unbounded_channel();
                            consumer_purge.insert(data_producer_id.clone(), purge_tx);
                            tokio::spawn(route_dc(dc, a, b, k, purge_rx, policy.clone()));
                        }
                        Err(e) => {
                            tracing::error!(label, "negotiated consumer DC creation failed: {e}");
                        }
                    }
                }
                Ok(SignalEvent::Message(SignalingMessage::ProducerClosed {
                    producer_id,
                    kind: MediaKind::Data,
                    ..
                })) => {
                    // S4': the upstream data producer died (server directed broadcast) → the local consumer
                    // DC self-tears down + ledger cleanup (allows re-consuming after the peer re-subscribes).
                    if let Some(tx) = consumer_purge.remove(&producer_id) {
                        let _ = tx.send(());
                        consumed.remove(&producer_id);
                        tracing::info!(producer_id, "S4': directed teardown dispatched");
                    }
                }
                Ok(SignalEvent::Message(SignalingMessage::DataProducerCreated { .. })) => {
                    // announce confirmations were consumed during setup; gateway broadcast echoes are ignored here (C16 chain established)
                }
                Ok(SignalEvent::Message(SignalingMessage::Error { code, message }))
                    if code != 0 =>
                {
                    // data-domain requests do not enter the gateway FIFO queue, so unmatched Errors may broadcast here —
                    // log and continue; signaling-level faults are closed out by Disconnected.
                    tracing::warn!("control-domain signaling error [{code}]: {message}");
                }
                Ok(SignalEvent::Message(_)) => {} // the rest passes through (RoomJoined/media plane etc.)
                Ok(SignalEvent::Connected { .. }) => {}
                Ok(SignalEvent::Error(e)) => {
                    tracing::error!("signaling error: {e}");
                    exit_code = 1;
                    break 'run;
                }
                Ok(SignalEvent::Disconnected { reason }) => {
                    tracing::error!("signaling dropped: {reason} — exiting for restart");
                    exit_code = 1;
                    break 'run;
                }
                Ok(_) => {} // SignalEvent non_exhaustive catch-all
                Err(_) => {
                    tracing::error!("signaling event stream closed");
                    exit_code = 1;
                    break 'run;
                }
            },
        }
    }

    if let Err(e) = signal.close().await {
        tracing::warn!("close: {e}");
    }
    send_pc.close().await;
    recv_pc.close().await;
    tracing::info!("controller stopped (exit={exit_code})");
    exit_code
}

/// Wait for SIGINT/SIGTERM (unix main path; other platforms use ctrl_c only).
pub async fn wait_shutdown() {
    #[cfg(unix)]
    {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sigterm) => {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    _ = sigterm.recv() => {}
                }
            }
            Err(e) => {
                tracing::warn!("SIGTERM listener failed (ctrl_c only): {e}");
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
