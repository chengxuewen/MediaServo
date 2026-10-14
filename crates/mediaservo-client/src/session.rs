//! Room session -- SFU consume + control egress on top of link signaling (the S2 core).
//!
//! The consume sequence locally mirrors `mediaservo-field::PullSession::subscribe`'s shape
//! (C21 dependency boundary: client must not depend on field); the control sequence mirrors `mediaservo-host::controller`'s
//! S1 egress shape (Send transport -> DC before answer -> Connect -> CreateDataProducer).
//! ICE-Lite: candidates arrive inline with WebRtcTransportCreated, no candidate exchange round;
//! `Error{code:0,"transport_connected"}` is the server's conventional ack, not a real error.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use mediaservo_common::protocol::{
    ControlAck, ControlEnvelope, DtlsParameters, Fingerprint, IceCandidate, IceParameters,
    MediaKind, SctpStreamParameters, SignalingMessage, TransportDirection,
};
use mediaservo_link::{SignalClient, SignalEvent};
use mediaservo_webrtc::rtp::{RTCRtpTransceiverDirection, RTCRtpTransceiverInit};
use mediaservo_webrtc::track::TrackKind;
use tokio::sync::{Mutex, broadcast, mpsc};

use crate::config::{ClientConfig, sfu_peer_key};
use crate::consumer::{Consumer, ConsumerSlot};
use crate::control::ControlChannel;
use crate::engine::{DcHandle, Engine, EngineDcEvent, PcHandle, SysEngine};
use crate::error::{ClientError, classify_link_error, from_wire_error};
use crate::sfu;
use crate::signal::{LinkSignal, Signal};
use crate::supervisor::{self, ConnectionState, SupervisorCtx};

/// The control-domain capability gate (I5's first user, S0 contract): the CreateDataProducer can_control gate
/// takes effect from dialect 2 -- below 2, pre-reject locally without sending a request (saves a 4012 round trip).
const CONTROL_MIN_PROTOCOL: u32 = 2;
/// CreateDataProducer sub-protocol (same value as host controller DATA_PROTOCOL).
const DATA_PROTOCOL: &str = "sctp";
/// Server response wait windows (transport/consumed/producer).
const RESPONSE_WAIT: Duration = Duration::from_secs(10);

/// Decode a video frame (I420, libwebrtc's pre-render format; C5 boundary semantics).
/// The single landing point of the inbound-rtp fold rule (a free function so tests can pin it without touching a real pc).
///
/// Multi-row semantics (PIT-2026-10-09 bitrate anomaly): libwebrtc can keep 2 inbound-rtp rows on one pc
/// (new and old SSRC reports coexisting, the old row frozen at its last timestamp). If cumulative fields (bytes/packets/frames)
/// `+=`, the frozen old row gets counted twice -> a misaligned differential baseline -> bitrate stuck at 0 / jumpy. **Dedup by ssrc,
/// keep each group's row with the newest timestamp**, then sum across groups (true multi-SSRC scenarios still accumulate correctly).
pub(crate) fn fold_inbound_stats(
    items: Vec<mediaservo_webrtc::stats::RTCStats>,
) -> VideoStreamStats {
    use mediaservo_webrtc::stats::RTCStats;
    // ssrc -> newest row (the bigger timestamp wins; an old ssrc=0 row is not lost -- id used as the grouping fallback)
    // diagnostics (debug level): distinguish "the stats table has no such inbound-rtp at all (demux registration failed)" from
    // "a row exists but all zeros (RTP never reached the transport)" -- the two-level predicate for zero-frame debugging (logged 09-24).
    tracing::debug!(
        rows = items.len(),
        kinds = ?items.iter().map(|x| match x {
            RTCStats::InboundRtp(_) => "inbound-rtp".to_string(),
            o => format!("{o:?}").chars().take(20).collect::<String>(),
        }).collect::<Vec<_>>(),
        "fold_inbound_stats raw rows"
    );
    let mut latest: std::collections::HashMap<
        (u32, String),
        mediaservo_webrtc::stats::RTCInboundRtpStreamStats,
    > = std::collections::HashMap::new();
    for st in &items {
        if let RTCStats::InboundRtp(r) = st {
            let key = (r.ssrc, r.id.clone());
            match latest.get_mut(&key) {
                Some(prev) if prev.timestamp >= r.timestamp => {}
                _ => {
                    latest.insert(key, r.clone());
                }
            }
        }
    }
    let mut out = VideoStreamStats::default();
    for (_, r) in latest {
        out.bytes_received += r.bytes_received;
        out.packets_received += r.packets_received;
        out.packets_lost += r.packets_lost;
        out.frames_decoded += u64::from(r.frames_decoded);
        out.frame_width = out.frame_width.max(r.frame_width);
        out.frame_height = out.frame_height.max(r.frame_height);
        out.frames_per_second = out.frames_per_second.max(r.frames_per_second);
        out.jitter = out.jitter.max(r.jitter);
        out.total_decode_time = out.total_decode_time.max(r.total_decode_time);
        out.frame_dropped += r.frame_dropped;
        out.nack_count += r.nack_count;
        out.pli_count += r.pli_count;
        out.fir_count += r.fir_count;
    }
    out
}

/// The flat result of [`RoomSession::video_stats_summary`] (serde key names = the C-surface JSON contract).
#[derive(Debug, Clone, Copy, Default, PartialEq, serde::Serialize)]
pub struct VideoStreamStats {
    pub bytes_received: u64,
    pub packets_received: u64,
    pub packets_lost: u64,
    pub frames_decoded: u64,
    pub frame_width: u32,
    pub frame_height: u32,
    pub frames_per_second: f64,
    pub jitter: f64,            // seconds (multi-path union takes max)
    pub total_decode_time: f64, // seconds cumulative (multi-path union takes max -- decode time is not additive)
    pub frame_dropped: u64,     // summed across paths
    pub nack_count: u64,
    pub pli_count: u64,
    pub fir_count: u64,
}

#[derive(Debug, Clone)]
pub struct VideoFrame {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
    /// Frame arrival has no clock source (webrtc-sys exposes no rtp ts) -- always 0; upper layers consume in arrival order.
    pub ts_us: i64,
}

/// A joined session: video consume + control egress share one link WS signaling surface.
///
/// S6/K1: signaling resilience -- `ctx` holds the supervisor context (reconnect chain/frame slots/state machine),
/// removal needs the id-based lookup; a re-push with the same id re-inserts over it).
pub struct RoomSession {
    /// S6/K1: the signaling resilience context (a single Arc instance; shared by forwarder/supervisor/replay).
    ctx: Arc<SupervisorCtx>,
    /// S4/T3.5: the emergency-stop HMAC key (used by emergency_stop signing; None = unsigned).
    hmac_key: Option<String>,
    /// Subscribed at connect time (broadcast has no history replay -- catches the server's join-time
    /// late-join NewProducer replay). Refreshed by the supervisor after reconnect.
    events: Mutex<broadcast::Receiver<SignalEvent>>,
    /// S2d: the dedicated second stream for open_control (subscribed at the same instant as connect = zero gap in the join replay;
    /// handed to the ack pump after take, once per session).
    pump_events: Mutex<Option<broadcast::Receiver<SignalEvent>>>,
    /// msrtc-stream-not-found: the room's full video label set (incrementally maintained in the wait loop --
    /// the events queue is monopolized by the wait, the loop is the only reliable aggregation point; reconnect replays enter through the same door).
    /// Old-host streams with label=None don't join the set. An empty set = unknown state (conservative blind wait, no not-found verdict).
    /// producer_labels: producer_id -> label reverse lookup (ProducerClosed carries no label field;
    /// removal needs the id-based lookup; a re-push with the same id re-inserts over it).
    known_video_labels: Mutex<std::collections::HashSet<String>>,
    producer_labels: Mutex<std::collections::HashMap<String, String>>,
}

impl RoomSession {
    /// S2c diagnostics surface: the video receiver's inbound-rtp stats (the decisive readout for the
    /// "packets never arrived" vs "arrived but not decoded" bisection; track_id = the "video" injected by the offer msid).
    #[must_use]
    pub fn video_receiver_stats(&self) -> Vec<mediaservo_webrtc::stats::RTCStats> {
        // the union across all paths (bit-identical to the old _pcs flat_map semantics; a single-path read goes through Consumer::stats).
        self.ctx
            .slots
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .flat_map(|slot| slot.receiver_stats())
            .collect()
    }

    /// Consumption-surface video stats summary (the W3 mini-stats data source): folds every inbound-rtp of this session.
    /// The shape is deliberately **flat + stable key names** (passed through C ABI JSON to the C++ mini-parse consumer,
    /// never exporting webrtc internal enum wires). Counters sum; fps takes the max (no sum semantics across tracks).
    pub fn video_stats_summary(&self) -> VideoStreamStats {
        fold_inbound_stats(self.video_receiver_stats())
    }

    /// S4/a4-T3.5: the emergency-stop dual path -- the DC fast path (with the HMAC sig, the vehicle act-then-audit primary record)
    /// accompanied by the WS `ControlAudit` audit copy (best-effort: a copy failure never affects the already-delivered DC command
    /// -- e-stops happen exactly during WS degraded windows; the audit's primary landing = the vehicle executor, PLAN §11.6 seat-3 ruling).
    pub async fn emergency_stop(
        &mut self,
        ctl: &mut ControlChannel,
        label: &str,
        seq: u64,
        payload: serde_json::Value,
    ) -> Result<(), ClientError> {
        let mut env = ControlEnvelope { seq, cmd: "estop".to_string(), payload, sig: None };
        if let Some(k) = &self.hmac_key {
            env.sig = Some(mediaservo_common::protocol::control_hmac_sign(k, &env));
        }
        ctl.send_envelope(label, &env).await?; // the DC fast path first (e-stop semantics = never wait for the audit copy)
        let _ = self
            .ctx
            .signal
            .send(SignalingMessage::ControlAudit {
                room_id: self.room_id().to_string(),
                seq,
                cmd: env.cmd.clone(),
                sig_present: env.sig.is_some(),
            })
            .await; // the WS copy best-effort (C15: silent failure = the primary record is on the vehicle)
        Ok(())
    }

    /// S4: whether the emergency-stop key is configured (observation surface; the key value never leaves).
    #[must_use]
    pub fn hmac_key_debug_present(&self) -> bool {
        self.hmac_key.is_some()
    }

    /// Signaling connect + room join (PSK or JWT, see [`ClientConfig`]; link carries the whole handshake).
    /// engine = the real webrtc-sys passthrough (the convenience wrapper of [`Self::connect_with_engine`]).
    pub async fn connect(cfg: &ClientConfig) -> Result<Self, ClientError> {
        Self::connect_with_engine(cfg, Arc::new(SysEngine::new())).await
    }

    /// Signaling connect + join + engine injection (S6 batch 0/K11: tests run deterministic drills over the full chain with FakeEngine).
    ///
    /// S6/K1: create the SupervisorCtx (holding Arc<SignalClient> for supervisor reconnects),
    /// subscribe the event stream then spawn the epoch-1 forwarder + supervisor.
    pub async fn connect_with_engine(
        cfg: &ClientConfig,
        engine: Arc<dyn Engine>,
    ) -> Result<Self, ClientError> {
        let client = SignalClient::new(
            &cfg.signaling_url,
            cfg.psk.as_deref().unwrap_or(""),
            &cfg.room_id,
            cfg.role.clone(),
        );
        let client = if let Some(jwt) = &cfg.jwt { client.with_jwt(jwt.clone()) } else { client };
        // T3: the device public-key identity (stackable with jwt -- server-side device authentication builds room membership first).
        // Load semantics documented at ClientConfig::identity_dir; the Err text is presented directly (a local config problem, retrying won't help).
        let client = match &cfg.identity_dir {
            Some(dir) => {
                match mediaservo_link::DeviceIdentity::load_from_instance_dir(dir)
                    .map_err(|e| ClientError::InvalidState(format!("device identity: {e}")))?
                {
                    Some(ident) => client.with_device_identity(ident),
                    None => client,
                }
            }
            None => client,
        };
        let session = client.connect().await.map_err(classify_link_error)?;
        // the second stream subscribed at the same instant as the main one (blocking_lock on LinkSignal::events in async context
        // would panic -- the subscription must complete at this sync point).
        let raw_events = session.events();
        let pump_events = session.events();
        let signal = Arc::new(LinkSignal::new(session, sfu_peer_key(&cfg.role).to_string()));
        let client = Arc::new(client);
        let ctx = SupervisorCtx::new(signal, client, engine, raw_events);
        let events = ctx.ev_tx.subscribe();
        supervisor::start(&ctx);
        Ok(Self {
            ctx,
            hmac_key: cfg.hmac_key.clone(),
            events: Mutex::new(events),
            pump_events: Mutex::new(Some(pump_events)),
            known_video_labels: Mutex::new(std::collections::HashSet::new()),
            producer_labels: Mutex::new(std::collections::HashMap::new()),
        })
    }

    /// The negotiated dialect version (S0; an old server = 1).
    #[must_use]
    pub fn negotiated(&self) -> u32 {
        self.ctx.signal.negotiated()
    }

    #[must_use]
    pub fn room_id(&self) -> &str {
        self.ctx.signal.room_id()
    }

    /// Wait for a video producer in the room (the server replays NewProducer for existing producers at join).
    pub async fn wait_video_producer(&mut self, wait: Duration) -> Result<String, ClientError> {
        self.wait_video_producer_labeled(wait, None).await
    }

    /// producer-label T2.1: label-targeted waiting. label=None = no filter (the first-path fallback semantics);
    /// Some(label) matches only producers carrying the same label -- unlabeled streams (old hosts) never match, keep waiting.
    pub async fn wait_video_producer_labeled(
        &mut self,
        wait: Duration,
        label: Option<&str>,
    ) -> Result<String, ClientError> {
        // the msrtc-stream-not-found fast-path precondition quantity: existence is only judged for label-targeted waits.
        let mut ev = self.events.lock().await;
        // replay aggregation window: a timing race exists between the join ack and the NewProducer replay -- judging the set empty immediately
        // would wrongly enter the conservative blind wait. First non-blocking drain of the current queue (replay events, if any, are in the buffer),
        // collect the known labels into the set, then make the existence judgment.
        {
            let mut known = self.known_video_labels.lock().await;
            let mut plabels = self.producer_labels.lock().await;
            while let Ok(sig) = ev.try_recv() {
                if let SignalEvent::Message(SignalingMessage::NewProducer {
                    producer_id,
                    kind: MediaKind::Video,
                    label: Some(l),
                    ..
                }) = sig
                {
                    known.insert(l.clone());
                    plabels.insert(producer_id, l);
                }
            }
        }
        let set_snapshot = self.known_video_labels.lock().await.clone();
        // existence verdict: the set still empty = the room confirmed to have no labeled video stream at all (the replay window drained).
        // the target label absent from the full set -> fallback window 3s (covering the "consumer first, push later" race),
        // only after the window expires without a hit is StreamNotFound judged (distinct from the blind network Timeout).
        const ABSENT_GRACE: Duration = Duration::from_secs(3);
        const REPLAY_SETTLE: Duration = Duration::from_millis(500);
        let grace = match label {
            Some(want) if !set_snapshot.contains(want) => Some(ABSENT_GRACE),
            _ => None,
        };
        let effective_wait = grace.unwrap_or(wait);
        let _ = REPLAY_SETTLE; // aggregation already done by the try_recv drain (keeping the constant's documented semantics)
        tokio::time::timeout(effective_wait, async {
            loop {
                match ev.recv().await {
                    Ok(SignalEvent::Message(SignalingMessage::NewProducer {
                        producer_id,
                        kind: MediaKind::Video,
                        label: pl,
                        ..
                    })) => {
                        // full-set incremental maintenance (None labels never join) + the id -> label reverse-lookup record
                        if let Some(l) = &pl {
                            self.known_video_labels.lock().await.insert(l.clone());
                            self.producer_labels
                                .lock()
                                .await
                                .insert(producer_id.clone(), l.clone());
                        }
                        let matched = match label {
                            None => true,
                            Some(want) => pl.as_deref() == Some(want),
                        };
                        if matched {
                            return Ok(producer_id);
                        }
                        // label mismatch -- keep waiting for the next path
                    }
                    Ok(SignalEvent::Message(SignalingMessage::ProducerClosed {
                        producer_id: ref pid,
                        kind: MediaKind::Video,
                        ..
                    })) => {
                        // stream offline -> reverse-lookup the label by id and remove it from the full set (a re-push back into the room re-delivers NewProducer;
                        // the ProducerClosed protocol surface carries no label field -- producer_labels reverse lookup)
                        if let Some(l) = self.producer_labels.lock().await.remove(pid) {
                            self.known_video_labels.lock().await.remove(&l);
                        }
                    }
                    Ok(SignalEvent::Disconnected { reason }) => {
                        return Err(ClientError::InvalidState(format!(
                            "signaling disconnected while waiting for a producer: {reason}"
                        )));
                    }
                    Ok(_) => {}
                    Err(_) => {
                        return Err(ClientError::InvalidState(
                            "the signaling event stream closed".into(),
                        ));
                    }
                }
            }
        })
        .await
        .map_err(|_| match grace {
            // fallback-window timeout = the label confirmed absent from the full set (also no new producer within 3s)
            Some(_) => ClientError::StreamNotFound { label: label.unwrap_or("").to_string() },
            None => ClientError::Timeout { what: "video NewProducer" },
        })?
    }

    /// Snapshot of the room's full known video label set (diagnostics/testing surface; incrementally maintained in the wait loop).
    #[must_use]
    pub async fn known_video_labels(&self) -> Vec<String> {
        let known = self.known_video_labels.lock().await;
        known.iter().cloned().collect()
    }

    /// S6/K4: subscribe one video producer, returning a [`Consumer`] handle (multi-path registration).
    ///
    /// Sequence = the old consume_video body verbatim (moved into [`consume_sequence`], zero copies); the difference =
    // the pc is hung on a registration slot: the frame channel frame_tx lives with the slot, the K1 reconnect replay swaps the pc not the slot, so the
    // app's receiver keeps streaming across reconnects (imperceptibly). Serialized with replay via consume_lock (K1).
    pub async fn consume(&self, producer_id: &str) -> Result<Consumer, ClientError> {
        let _seq = self.ctx.consume_lock.lock().await;
        let mut ev = self.events.lock().await;
        let (frame_tx, frame_rx) = mpsc::channel::<VideoFrame>(3);
        let pc = consume_sequence(
            producer_id,
            &self.ctx.engine,
            &self.ctx.signal,
            &mut ev,
            frame_tx.clone(),
        )
        .await?;
        let slot = Arc::new(ConsumerSlot::new(producer_id, frame_tx));
        slot.set_pc(pc);
        self.ctx.slots.lock().unwrap_or_else(|e| e.into_inner()).push(slot.clone());
        Ok(Consumer::new(producer_id.to_string(), slot, frame_rx))
    }

    /// Legacy bridge (R3 behavior remap): equivalent to `consume(p).await?.into_receiver()`,
    /// returning the bare frame-stream receiver, byte-identical in behavior to the old consume_video.
    pub async fn consume_video(
        &mut self,
        producer_id: &str,
    ) -> Result<mpsc::Receiver<VideoFrame>, ClientError> {
        self.consume(producer_id).await.map(Consumer::into_receiver)
    }

    /// Open the egress control channel set (host S1 mirror). Precondition I5 gate: dialect >= 2, else a local pre-rejection.
    pub async fn open_control(&mut self, labels: &[&str]) -> Result<ControlChannel, ClientError> {
        let got = self.ctx.signal.negotiated();
        if got < CONTROL_MIN_PROTOCOL {
            return Err(ClientError::ProtocolTooLow { need: CONTROL_MIN_PROTOCOL, got });
        }
        let mut ev = self.events.lock().await;
        let room = self.ctx.signal.room_id().to_string();
        let peer = self.ctx.signal.sfu_peer_id().to_string();
        // S2d: take the second stream pre-subscribed at connect time (zero gap for the vehicle ack announce during establishment).
        let pump_ev = self.pump_events.lock().await.take().ok_or_else(|| {
            ClientError::InvalidState(
                "the control plane is already open (the ack pump runs once per session)".into(),
            )
        })?;

        // 1. Send transport
        self.ctx
            .signal
            .send(SignalingMessage::CreateWebRtcTransport {
                room_id: room.clone(),
                peer_id: peer.clone(),
                direction: TransportDirection::Send,
            })
            .await?;
        let (transport_id, ice, dtls, candidates) =
            tokio::time::timeout(RESPONSE_WAIT, await_transport_created(&mut ev))
                .await
                .map_err(|_| ClientError::Timeout { what: "WebRtcTransportCreated(send)" })??;

        // 2. PC + synthetic offer set_remote; the DC must be created before the answer (the S1 ordering contract)
        let pc = self.ctx.engine.create_pc().await?;
        let remote_sdp = sfu::build_dc_remote_sdp(&ice, &dtls, candidates.as_ref());
        pc.set_remote_offer(remote_sdp).await?;

        // 3. one egress DC per label (the mediasoup unidirectional-pair model: a producer DC has no ingress; the return
        //    path = step 6's ack consumption chain -- live-proven 2026-09-15 that consumer reverse is not relayed).
        let (ack_tx, ack_rx) = mpsc::channel::<ControlAck>(32);
        let mut dcs: HashMap<String, Arc<dyn DcHandle>> = HashMap::new();
        let mut order = Vec::with_capacity(labels.len());
        for label in labels {
            let dc = pc.create_data_channel(label, sfu::channel_init(label)).await?;
            dcs.insert(label.to_string(), dc);
            order.push(label.to_string());
        }

        // 4. answer/connect wrap-up
        let answer = pc.create_answer().await?;
        pc.set_local_answer(&answer).await?;
        connect_transport(&*self.ctx.signal, &room, &peer, &transport_id, &pc).await?;

        // 5. CreateDataProducer announce per DC (4012 -> ControlDenied terminal)
        let mut producer_ids = Vec::with_capacity(order.len());
        for label in &order {
            let dc = &dcs[label];
            self.ctx
                .signal
                .send(SignalingMessage::CreateDataProducer {
                    room_id: room.clone(),
                    peer_id: peer.clone(),
                    transport_direction: TransportDirection::Send,
                    label: label.clone(),
                    protocol: DATA_PROTOCOL.into(),
                    sctp_stream_parameters: Some(
                        sfu::sctp_stream_params(dc.label(), dc.id())
                            .map_err(ClientError::WebRtc)?,
                    ),
                    transport_id: Some(transport_id.clone()),
                })
                .await?;
            let dp = tokio::time::timeout(RESPONSE_WAIT, await_data_producer_created(&mut ev))
                .await
                .map_err(|_| ClientError::Timeout { what: "DataProducerCreated" })??;
            tracing::info!(label = %label, data_producer_id = %dp, "client DataProducer established");
            producer_ids.push(dp);
        }

        // 6. the S2d official unidirectional-pair model: a background task consumes the vehicle's label=ack DataProducer, routing the negotiated
        //    consumer DC's ControlAck into ack_rx (the official supply source of recv_ack).
        tokio::spawn(ack_consumer_pump(
            pump_ev,
            self.ctx.signal.clone(),
            self.ctx.engine.clone(),
            room,
            peer,
            ack_tx,
        ));

        Ok(ControlChannel::new(dcs, order, ack_rx, producer_ids, pc))
    }

    /// Close the session (shutdown supervisor + release WS; frame/ack streams converge on drop).
    pub async fn close(self) -> Result<(), ClientError> {
        self.ctx.shutdown();
        self.ctx.signal.close().await
    }

    /// S6/K1: signaling connection-state observation (the single watch truth source; reconnect/terminal states readable live).
    #[must_use]
    pub fn connection_state(&self) -> ConnectionState {
        self.ctx.state()
    }

    /// S6/K1: the signaling connection-state change stream (subscriber semantics; reconnect/terminal pushed automatically).
    #[must_use]
    pub fn subscribe_connection_state(&self) -> tokio::sync::watch::Receiver<ConnectionState> {
        self.ctx.subscribe_state()
    }

    /// S6/K1: dynamically toggle auto-reconnect (D273 red-card semantics: Failed = the auth family terminal, not governed by this switch).
    ///
    /// **DEVIATION**: the design first considered a ClientConfig field, but client-c lib.rs:238's
    /// ClientConfig is an exhaustive literal; a new field = a C ABI break (forbidden this batch),
    /// so it became a runtime method (K5 also uses the runtime-setter shape).
    pub fn set_auto_reconnect(&mut self, on: bool) {
        self.ctx.set_auto_reconnect(on);
    }
}

impl Drop for RoomSession {
    fn drop(&mut self) {
        self.ctx.shutdown();
    }
}

/// K4/K1: the full establishment sequence of one video receive path (Recv transport -> router caps -> Consume ->
/// answerer negotiation -> Connect). Extracted verbatim from the old consume_video = shared with the K1 replay, zero copies.
/// The frame outlet is injected via `frame_tx` (the slot-held long-lived sender; on_track clones one per sink).
pub(crate) async fn consume_sequence(
    producer_id: &str,
    engine: &Arc<dyn Engine>,
    signal: &LinkSignal,
    ev: &mut broadcast::Receiver<SignalEvent>,
    frame_tx: mpsc::Sender<VideoFrame>,
) -> Result<Arc<dyn PcHandle>, ClientError> {
    let room = signal.room_id().to_string();
    let peer = signal.sfu_peer_id().to_string();

    // 1. Recv transport
    signal
        .send(SignalingMessage::CreateWebRtcTransport {
            room_id: room.clone(),
            peer_id: peer.clone(),
            direction: TransportDirection::Recv,
        })
        .await?;
    let (transport_id, ice, dtls, candidates) =
        tokio::time::timeout(RESPONSE_WAIT, await_transport_created(ev))
            .await
            .map_err(|_| ClientError::Timeout { what: "WebRtcTransportCreated(recv)" })??;

    // 2. PC (mediasoup ICE-Lite, no STUN) + on_track before set_remote (late registration loses the first track)
    let pc = engine.create_pc().await?;
    pc.on_track(Box::new(move |track| {
        // S2c diagnostics (permanent log): on_track arriving = RTP already demuxed into a track;
        // the watershed predicate for "ICE/DTLS pass but zero frames".
        tracing::info!(kind = ?track.kind, track_id = ?track.track_id, "consume on_track arrived");
        track.set_frame_sink(Box::new(FrameChanSink { tx: frame_tx.clone() }));
    }));

    // 3a. Router capability query passthrough (the same official flow as mediasoup-client Device.load, C18;
    //     hand-built caps are always rejected under an H264 producer -- the S2b live lesson; PullSession shares the sin, another case).
    signal.send(SignalingMessage::GetRouterRtpCapabilities { room_id: room.clone() }).await?;
    let router_caps = tokio::time::timeout(RESPONSE_WAIT, await_router_caps(ev))
        .await
        .map_err(|_| ClientError::Timeout { what: "RouterRtpCapabilities" })??;

    // 3b. Consume (C1 explicitly binds transport_id; caps = the router's verbatim reply)
    signal
        .send(SignalingMessage::Consume {
            room_id: room.clone(),
            peer_id: peer.clone(),
            producer_id: producer_id.to_string(),
            rtp_capabilities: router_caps,
            transport_id: Some(transport_id.clone()),
        })
        .await?;
    let consumer_rtp = tokio::time::timeout(RESPONSE_WAIT, await_consumed(ev))
        .await
        .map_err(|_| ClientError::Timeout { what: "Consumed" })??;

    // 4. remote SDP: codec taken from the consumer rtp_parameters, ssrc injected for demux
    let (pt, name, clock, fmtp) = sfu::codec_from_consumer(&consumer_rtp).ok_or_else(|| {
        ClientError::MalformedResponse("Consumed rtp_parameters has no video codec".into())
    })?;
    let remote_sdp = sfu::build_recv_video_sdp(
        &ice,
        &dtls,
        candidates.as_ref(),
        pt,
        &name,
        clock,
        fmtp.as_deref(),
    );
    let remote_sdp = sfu::inject_remote_ssrc(&remote_sdp, &consumer_rtp);

    // 5. answerer wrap-up: add_transceiver(recvonly) -> set_remote(offer) -> answer
    pc.add_transceiver(
        TrackKind::Video,
        RTCRtpTransceiverInit {
            direction: RTCRtpTransceiverDirection::Recvonly,
            ..Default::default()
        },
    )?;
    tracing::debug!(remote_sdp = %remote_sdp, "consume remote offer (after SSRC injection)");
    pc.set_remote_offer(remote_sdp).await?;
    let answer = pc.create_answer().await?;
    pc.set_local_answer(&answer).await?;

    // 6. Connect (the local DTLS fingerprint, role=client)
    connect_transport(signal, &room, &peer, &transport_id, &pc).await?;

    tracing::info!(producer_id, transport_id = %transport_id, "client consume_video established");
    Ok(pc)
}

async fn connect_transport(
    signal: &dyn Signal,
    room: &str,
    peer: &str,
    transport_id: &str,
    pc: &Arc<dyn PcHandle>,
) -> Result<(), ClientError> {
    let fp_hex = pc
        .local_dtls_fingerprint()
        .ok_or_else(|| ClientError::WebRtc("no local DTLS fingerprint".into()))?;
    signal
        .send(SignalingMessage::ConnectWebRtcTransport {
            room_id: room.to_string(),
            peer_id: peer.to_string(),
            transport_id: transport_id.to_string(),
            dtls_parameters: DtlsParameters {
                fingerprints: vec![Fingerprint { algorithm: "sha-256".to_string(), value: fp_hex }],
                role: "client".to_string(),
            },
        })
        .await
}

// -- event waiting (broadcast single-stream sequential consumption; the transport_connected ack exemption) --

async fn await_transport_created(
    ev: &mut broadcast::Receiver<SignalEvent>,
) -> Result<(String, IceParameters, DtlsParameters, Option<Vec<IceCandidate>>), ClientError> {
    loop {
        match next_msg(ev).await? {
            SignalingMessage::WebRtcTransportCreated {
                transport_id,
                ice_parameters,
                dtls_parameters,
                ice_candidates,
                ..
            } => return Ok((transport_id, ice_parameters, dtls_parameters, ice_candidates)),
            SignalingMessage::Error { message, .. } if message == "transport_connected" => {}
            // concurrent-event tolerance (the vehicle-room norm: another party's producer broadcast and the transport reply share a window);
            // only Error is terminal (C15 with context).
            err @ SignalingMessage::Error { .. } => {
                return Err(on_unexpected(err, "WebRtcTransportCreated"));
            }
            other => {
                tracing::debug!(msg = ?other, "await(transport) ignoring an unrelated event, keep waiting");
                continue;
            }
        }
    }
}

async fn await_consumed(
    ev: &mut broadcast::Receiver<SignalEvent>,
) -> Result<serde_json::Value, ClientError> {
    loop {
        match next_msg(ev).await? {
            SignalingMessage::Consumed { rtp_parameters, .. } => return Ok(rtp_parameters),
            SignalingMessage::Error { message, .. } if message == "transport_connected" => {}
            err @ SignalingMessage::Error { .. } => return Err(on_unexpected(err, "Consumed")),
            other => {
                tracing::debug!(msg = ?other, "await(consumed) ignoring an unrelated event, keep waiting");
                continue;
            }
        }
    }
}

async fn await_data_producer_created(
    ev: &mut broadcast::Receiver<SignalEvent>,
) -> Result<String, ClientError> {
    loop {
        match next_msg(ev).await? {
            SignalingMessage::DataProducerCreated { data_producer_id, .. } => {
                return Ok(data_producer_id);
            }
            SignalingMessage::Error { message, .. } if message == "transport_connected" => {}
            err @ SignalingMessage::Error { .. } => {
                return Err(on_unexpected(err, "DataProducerCreated"));
            }
            other => {
                tracing::debug!(msg = ?other, "await(producer) ignoring an unrelated event, keep waiting");
                continue;
            }
        }
    }
}

/// S2b: query the router's real caps before consume -- a hand-built "VP8 only" declaration is directly rejected by an H264 producer's
/// can_consume (5000 No compatible media codecs, live-proven 09-15).
async fn await_router_caps(
    ev: &mut broadcast::Receiver<SignalEvent>,
) -> Result<serde_json::Value, ClientError> {
    match next_msg(ev).await? {
        SignalingMessage::RouterRtpCapabilities { capabilities, .. } => Ok(capabilities),
        other => Err(on_unexpected(other, "RouterRtpCapabilities")),
    }
}

// -- the S2d ack consumption chain (cockpit = the consumer of the vehicle's ack producer) --

/// Wait for the vehicle ack producer's announce (non-ack labels / own echo skipped; `None` = the event stream ended).
/// S4': en route, `ProducerClosed(Data)` tears down the corresponding local consumer DC (freeing the libwebrtc
/// stream id it held = the cockpit-side answer to the `StreamId reserved` consumption ceiling).
async fn await_ack_producer(
    ev: &mut broadcast::Receiver<SignalEvent>,
    self_peer: &str,
    consumers: &mut HashMap<String, mpsc::UnboundedSender<()>>,
) -> Option<String> {
    loop {
        match ev.recv().await {
            Ok(SignalEvent::Message(SignalingMessage::NewDataProducer {
                data_producer_id,
                label,
                peer_id,
                ..
            })) => {
                if label == sfu::ACK_LABEL && peer_id != self_peer {
                    return Some(data_producer_id);
                }
            }
            Ok(SignalEvent::Message(SignalingMessage::ProducerClosed {
                producer_id,
                kind: mediaservo_common::protocol::MediaKind::Data,
                ..
            })) => {
                if let Some(tx) = consumers.remove(&producer_id) {
                    let _ = tx.send(());
                    tracing::info!(
                        producer_id,
                        "ack pump: the vehicle DataProducer died -- targeted teardown of the local DC"
                    );
                }
            }
            Ok(SignalEvent::Message(_)) | Ok(SignalEvent::Connected { .. }) => {}
            Ok(SignalEvent::Error(e)) => {
                tracing::warn!("ack pump: signaling event error (ignored, keep waiting): {e}")
            }
            Ok(_) => {}
            Err(broadcast::error::RecvError::Lagged(n)) => {
                tracing::warn!("ack pump: queue overflow dropped {n} events, continuing");
            }
            Err(_) => return None, // Closed = session ended
        }
    }
}

/// Wait for DataConsumed's S2d parameter triple (Error is terminal; other unrelated events are ignored, keep waiting).
async fn await_data_consumed(
    ev: &mut broadcast::Receiver<SignalEvent>,
) -> Result<(SctpStreamParameters, String, String), ClientError> {
    loop {
        match next_msg(ev).await? {
            // the connect ack (the code=0 exemption shape, same as await_transport_created -- the pump happens to span the connect window).
            SignalingMessage::Error { ref message, .. } if message == "transport_connected" => {}
            SignalingMessage::DataConsumed { sctp_stream_parameters, label, protocol, .. } => {
                let sp = sctp_stream_parameters.ok_or_else(|| {
                    ClientError::MalformedResponse(
                        "DataConsumed missing sctp params (server too old, no negotiated basis)"
                            .into(),
                    )
                })?;
                return Ok((sp, label, protocol));
            }
            err @ SignalingMessage::Error { .. } => {
                return Err(on_unexpected(err, "DataConsumed"));
            }
            other => {
                tracing::debug!(msg = ?other, "ignoring an unrelated event while the ack pump awaits");
                continue;
            }
        }
    }
}

/// Build a DC-only recv transport and connect (the same shape as host S1 setup_recv_side; a negotiated
/// DC's out-of-band negotiation is exempt from the "DC before answer" ordering contract).
async fn open_dc_recv_transport(
    ev: &mut broadcast::Receiver<SignalEvent>,
    signal: &LinkSignal,
    engine: &Arc<dyn Engine>,
    room: &str,
    peer: &str,
) -> Result<(Arc<dyn PcHandle>, String), ClientError> {
    signal
        .send(SignalingMessage::CreateWebRtcTransport {
            room_id: room.to_string(),
            peer_id: peer.to_string(),
            direction: TransportDirection::Recv,
        })
        .await?;
    let (transport_id, ice, dtls, candidates) =
        tokio::time::timeout(RESPONSE_WAIT, await_transport_created(ev))
            .await
            .map_err(|_| ClientError::Timeout { what: "WebRtcTransportCreated(recv-ack)" })??;
    let pc = engine.create_pc().await?;
    let remote_sdp = sfu::build_dc_remote_sdp(&ice, &dtls, candidates.as_ref());
    // error-string normalization (the old "(recv-ack)" local prefix folded into the engine's unified mapping -- a text-only diff, type/path unchanged).
    pc.set_remote_offer(remote_sdp).await?;
    let answer = pc.create_answer().await?;
    pc.set_local_answer(&answer).await?;
    connect_transport(signal, room, peer, &transport_id, &pc).await?;
    Ok((pc, transport_id))
}

/// Background pump: consumes the vehicle ack producer -> negotiated DC -> routes ControlAck into `ack_tx`.
/// Loops waiting for later announces (adapts to vehicle restarts/multi-instance); the transport's first build is reused.
/// Exits when all `ack_tx` receivers drop (ControlChannel released) or the event stream closes.
async fn ack_consumer_pump(
    mut ev: broadcast::Receiver<SignalEvent>,
    signal: Arc<LinkSignal>,
    engine: Arc<dyn Engine>,
    room: String,
    peer: String,
    ack_tx: mpsc::Sender<ControlAck>,
) {
    let mut transport: Option<(Arc<dyn PcHandle>, String)> = None;
    // S4': dp_id -> the teardown channel of the local ack consumer DC.
    let mut consumers: HashMap<String, mpsc::UnboundedSender<()>> = HashMap::new();
    // the pre-subscribed stream and the main stream each see the full broadcast = the opening backlog mixes the step1 send transport's
    // Created/connect acks (the pump awaiting directly would grab a stale reply -> a second connect on the send slot
    // = worker "connect() already called", live-proven 2026-09-15). Drain the backlog:
    // ack announces go into the seed, the rest discarded; afterwards the queue only holds this pump round's replies.
    let mut seed: Vec<String> = Vec::new();
    'drain: loop {
        match ev.try_recv() {
            Ok(SignalEvent::Message(SignalingMessage::NewDataProducer {
                data_producer_id,
                label,
                peer_id,
                ..
            })) => {
                if label == sfu::ACK_LABEL && peer_id != peer {
                    seed.push(data_producer_id);
                }
            }
            Ok(_) => {}
            Err(broadcast::error::TryRecvError::Lagged(_)) => continue 'drain,
            Err(broadcast::error::TryRecvError::Empty) => break 'drain,
            Err(broadcast::error::TryRecvError::Closed) => return,
        }
    }
    while !ack_tx.is_closed() {
        let dp_id = match seed.pop() {
            Some(dp) => dp,
            None => {
                let Some(dp) = await_ack_producer(&mut ev, &peer, &mut consumers).await else {
                    return;
                };
                dp
            }
        };
        tracing::info!(data_producer_id = %dp_id, "ack pump: vehicle DataProducer found, consuming");
        let (pc, tid) = match transport.clone() {
            Some(t) => t,
            None => match open_dc_recv_transport(&mut ev, &signal, &engine, &room, &peer).await {
                Ok(t) => {
                    transport = Some(t.clone());
                    t
                }
                Err(e) => {
                    tracing::error!("ack pump: recv transport establishment failed: {e}");
                    continue;
                }
            },
        };
        if let Err(e) = signal
            .send(SignalingMessage::ConsumeData {
                room_id: room.clone(),
                peer_id: peer.clone(),
                transport_direction: TransportDirection::Recv,
                data_producer_id: dp_id.clone(),
                transport_id: Some(tid),
            })
            .await
        {
            tracing::error!("ack pump: ConsumeData send failed: {e}");
            continue;
        }
        let sp = match tokio::time::timeout(RESPONSE_WAIT, await_data_consumed(&mut ev)).await {
            Ok(Ok(v)) => v,
            Ok(Err(e)) => {
                tracing::error!("ack pump: DataConsumed error: {e}");
                continue;
            }
            Err(_) => {
                tracing::error!("ack pump: DataConsumed wait timed out");
                continue;
            }
        };
        let (sp, label, protocol) = sp;
        match pc.create_data_channel(&label, sfu::negotiated_init(&sp, &protocol)).await {
            Ok(dc) => {
                tracing::info!(
                    label,
                    stream_id = sp.stream_id,
                    "ack pump: the negotiated consumer DC is up, receipts wired into recv_ack"
                );
                let tx = ack_tx.clone();
                let mut rx = dc.events().await;
                let (purge_tx, mut purge_rx) = mpsc::unbounded_channel();
                consumers.insert(dp_id.clone(), purge_tx);
                tokio::spawn(async move {
                    let _dc = dc; // the channel lifetime anchor (Arc drop closes it)
                    loop {
                        tokio::select! {
                                                    item = rx.recv() => match item {
                                                        Some(EngineDcEvent::Message(data)) => {
                                                            match serde_json::from_slice::<ControlAck>(&data) {
                                                                Ok(ack) => {
                                                                    if tx.send(ack).await.is_err() {
                                                                        break;
                                                                    }
                                                                }
                                                                Err(e) => {
                                                                    tracing::warn!("dropping a non-ControlAck payload on the ack DC: {e}")
                                                                }
                                                            }
                                                        }
                                                        Some(EngineDcEvent::Closed) => break,
                                                        Some(_) => {}
                                                        None => break,
                                                    },
                        // S4': the vehicle ack dp died (pump-targeted) -> close locally to free the sid.
                                                    _ = purge_rx.recv() => {
                                                        tracing::info!("ack pump: the local consumer DC self-tears (the vehicle DataProducer died)");
                                                        _dc.close().await;
                                                        break;
                                                    }
                                                }
                    }
                });
            }
            Err(e) => tracing::error!("ack pump: negotiated DC creation failed: {e}"),
        }
    }
}

/// Fetch the next signaling message (stream break/disconnect/non-message events error; Error frames are classified by the caller).
async fn next_msg(
    ev: &mut broadcast::Receiver<SignalEvent>,
) -> Result<SignalingMessage, ClientError> {
    match ev.recv().await {
        Ok(SignalEvent::Message(msg)) => Ok(msg),
        Ok(SignalEvent::Disconnected { reason }) => Err(ClientError::InvalidState(format!(
            "signaling disconnected while waiting: {reason}"
        ))),
        Ok(SignalEvent::Error(e)) => {
            Err(ClientError::Signal(mediaservo_link::LinkError::Signal(e)))
        }
        Ok(SignalEvent::Connected { .. }) => Err(ClientError::InvalidState(
            "a connection event while waiting (resume re-mount unsupported)".into(),
        )),
        Ok(_) => Err(ClientError::InvalidState("unknown signaling event".into())),
        Err(_) => Err(ClientError::InvalidState("the signaling event stream closed".into())),
    }
}

/// Wait for events other than the target message: Error{code} -> typed terminal, the rest = a protocol misalignment.
fn on_unexpected(msg: SignalingMessage, want: &str) -> ClientError {
    match msg {
        SignalingMessage::Error { code, message } => from_wire_error(code, &message),
        other => ClientError::MalformedResponse(format!("expected {want}, got {other:?}")),
    }
}

/// FrameSink -> bounded mpsc (full = drop, the latest-wins semantics, same as field PullFrameSink).
struct FrameChanSink {
    tx: mpsc::Sender<VideoFrame>,
}

impl mediaservo_webrtc::track::FrameSink for FrameChanSink {
    fn on_frame(&self, data: &[u8], width: u32, height: u32) {
        let _ = self.tx.try_send(VideoFrame { width, height, data: data.to_vec(), ts_us: 0 });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mediaservo_webrtc::stats::{RTCInboundRtpStreamStats, RTCStats};

    fn mk(bytes: u64, w: u32, fps: f64) -> RTCStats {
        mk_keyed("x", 1, 0.0, bytes, w, fps)
    }

    fn mk_keyed(id: &str, ssrc: u32, ts: f64, bytes: u64, w: u32, fps: f64) -> RTCStats {
        RTCStats::InboundRtp(RTCInboundRtpStreamStats {
            id: id.into(),
            timestamp: ts,
            ssrc,
            kind: "video".into(),
            packets_received: bytes,
            packets_lost: bytes,
            bytes_received: bytes,
            frames_decoded: 10,
            frame_width: w,
            frame_height: 720,
            frames_per_second: fps,
            jitter: 0.0,
            total_decode_time: 0.0,
            frame_dropped: 0,
            nack_count: 0,
            pli_count: 0,
            fir_count: 0,
        })
    }

    #[test]
    fn fold_inbound_unions_new_w3c_fields() {
        // the 09-24 widened-semantics pin: jitter takes the max across paths, dropped/nack/pli/fir sum across SSRCs
        // (the PIT-212 revision: fold dedups by (ssrc,id) -- fixtures must differ in ssrc to count as "multi-path")
        let mut mk2 = mk_keyed("a", 1, 1.0, 100, 1280, 30.0);
        if let RTCStats::InboundRtp(r) = &mut mk2 {
            r.jitter = 0.004;
            r.frame_dropped = 7;
            r.nack_count = 5;
            r.pli_count = 2;
            r.fir_count = 1;
        }
        let mut mk3 = mk_keyed("b", 2, 1.0, 100, 1280, 30.0);
        if let RTCStats::InboundRtp(r) = &mut mk3 {
            r.jitter = 0.009;
            r.frame_dropped = 3;
            r.nack_count = 4;
        }
        let out = fold_inbound_stats(vec![mk2, mk3]);
        assert!((out.jitter - 0.009).abs() < 1e-9, "jitter=max");
        assert_eq!(out.frame_dropped, 10);
        assert_eq!(out.nack_count, 9);
        assert_eq!(out.pli_count, 2);
        assert_eq!(out.fir_count, 1);
        assert_eq!(out.bytes_received, 200, "distinct ssrcs sum across groups");
    }

    #[test]
    fn fold_inbound_dedup_frozen_row_takes_latest() {
        // the PIT-212 pin (the bitrate-anomaly root cause): two rows on one pc = a frozen old row + a live row (same ssrc, same id).
        // the frozen row's bytes stop at the old value -- `+=` would count it twice -> a misaligned differential baseline.
        // the new semantics: for the same key keep the row with the newest timestamp (the live row), drop the old.
        let frozen = mk_keyed("x", 1, 10.0, 100, 1280, 24.0); // the old row: older ts, smaller bytes
        let mut live = mk_keyed("x", 1, 20.0, 900, 1920, 30.0); // the live row: newer ts
        if let RTCStats::InboundRtp(r) = &mut live {
            r.frame_dropped = 5;
            r.nack_count = 3;
        }
        let out = fold_inbound_stats(vec![frozen, live]);
        assert_eq!(out.bytes_received, 900, "same key keeps the newest-timestamp row (no summing)");
        assert_eq!(out.frame_width, 1920);
        assert_eq!(out.frames_per_second, 30.0);
        assert_eq!(out.frame_dropped, 5, "the frozen row's dropped must not add in");
        assert_eq!(out.nack_count, 3);
    }

    #[test]
    fn fold_inbound_sums_counts_maxes_size_and_fps() {
        // distinct ssrcs (true multi-path) sum across groups
        let out = fold_inbound_stats(vec![
            mk_keyed("x", 1, 1.0, 100, 1280, 24.0),
            mk_keyed("y", 2, 1.0, 50, 1920, 30.0),
        ]);
        assert_eq!(out.bytes_received, 150);
        assert_eq!(out.packets_lost, 150);
        assert_eq!(out.frames_decoded, 20);
        assert_eq!(out.frame_width, 1920);
        assert_eq!(out.frames_per_second, 30.0);
        assert_eq!(fold_inbound_stats(vec![]), VideoStreamStats::default());
    }
}
