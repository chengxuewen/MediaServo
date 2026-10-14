//! host-agent signaling gateway (Task D1, D-H6 "one vehicle one session" core).
//!
//! Topology:
//! ```text
//! child processes (host-streamer etc.) ──WS {src, msg}──▶ host-agent ◀──WS pure SignalingMessage── Server
//! ```
//!
//! Protocol semantics (Momus HIGH-1, verified against signaling.rs):
//! - (a) **RoomJoin interception**: each child process's RoomJoin is intercepted by the agent (the server only
//!   handles RoomJoin during connection setup; one received again inside the relay loop is silently dropped); the agent joins once
//!   with the whole-vehicle identity, and child processes get a locally synthesized `RoomJoined` (carrying the vehicle peer_id).
//! - (b) **Response routing**: SFU messages (CreateWebRtcTransport/ConnectWebRtcTransport/
//!   Produce/Consume) get exactly one response per request, and the single server WS is processed strictly in order (the forward loop
//!   awaits `handle_sfu_message` before reading the next) → the agent maintains a **pending-request FIFO**
//!   (conn ids); when a response arrives it pops the matching sender. Concurrent negotiation is correct (unlike the rejected "sequential reuse" option
//!   — negotiations are not serialized, only in-flight requests are paired in order). The three FIFO boundary invariants (D1 review fixes):
//!   1) the joined check precedes the push (the disconnect window's 5001 leaves no stale slot); 2) push and remote send share one
//!   critical section (wire order ≡ pending order); 3) a disconnected connection does not shift the queue (mark-dead strict
//!   FIFO consumption — a dead slot is popped and discarded by its own response). P2P relay's Sdp/RTCIceCandidate carry no
//!   transport id → tracked by **negotiation ownership** (the local connection of the last upstream Sdp/ICE;
//!   media/status messages like Frame/EncoderStatus do not update ownership), single-negotiation serial semantics.
//! - (c) Rejected options (none implemented): "remote from prefix" (breaks zero-change Server) / "sequential reuse" (concurrent
//!   negotiations must scramble).
//!
//! Additional semantics:
//! - **Room rewriting**: child-process message room_id → upstream vehicle room; downstream is rewritten to the room each target child process
//!   itself declared. Multiple streamers (each with a `stream-<id>` room) aggregate into one vehicle session.
//! - **Echo dedup**: the server broadcasts relay-whitelist messages (Sdp/ICE/Frame/EncoderStatus) to everyone in the
//!   room (including the sender itself) → the agent drops echoes of its own forwarded traffic by text match.
//! - **Child-process RoomLeave interception**: a single process's leave, forwarded upstream, would make the server drop the whole vehicle session.
//! - **Disconnect**: remote drop → clear the pending FIFO / negotiation ownership / echo cache (the server has reclaimed all
//!   transports); local connections stay up, and forwarding resumes after B1 reconnect.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use mediaservo_common::protocol::{PeerRole, SignalingMessage};
use mediaservo_link::{LinkError, RetryConfig, SignalClient, SignalEvent, SignalSession};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;

/// Local protocol envelope: `{src, msg}` — local wire only; the remote side is a pure SignalingMessage (zero change).
/// Type defined in mediaservo-link (D2 I1: single source, prevents wire drift — child processes reuse the same shape via field).
pub use mediaservo_link::signal::LocalEnvelope;

/// Gateway configuration.
#[derive(Debug, Clone)]
pub struct GatewayConfig {
    /// Local listen port (0 = ephemeral port, for tests).
    pub local_port: u16,
    /// Remote server WS address (SignalClient direct connect).
    pub remote_url: String,
    /// PSK (per-connection re-authentication, B1 semantics).
    pub psk: String,
    /// device-enroll device identity (public-key fingerprint): Some = the remote Join carries device_pubkey through the signature chain
    /// (not approved = EnrollPending → "awaiting admin approval" log + the existing backoff); None = PSK path.
    /// host-agent assembles it from identity.json + etc/link/signing.pem.
    pub device: Option<mediaservo_link::DeviceIdentity>,
    /// Vehicle room (the room the agent joins once).
    pub room: String,
    /// Remote connection retry config (reused for reconnect after disconnect).
    pub retry: RetryConfig,
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            local_port: DEFAULT_LOCAL_PORT,
            remote_url: "ws://127.0.0.1:9800/ws".into(),
            psk: "mediaservo-dev".into(),
            device: None,
            // I1 review (D3 TODO closed): the vehicle room is configured via host.yaml [signaling] room —
            // translate renders it into agent --room; this is the CLI default (host-agent builtin).
            room: "vehicle".into(),
            retry: RetryConfig::default(),
        }
    }
}

/// Default local port (overridable by host.yaml [signaling] local_port; translate passes --port).
pub const DEFAULT_LOCAL_PORT: u16 = 17980;
/// Response code toward child processes while the gateway has no remote server connection (child processes may retry).
const ERR_GATEWAY_DISCONNECTED: u16 = 5001;
/// Echo cache capacity (recently forwarded relay message texts, FIFO).
const ECHO_CACHE_CAP: usize = 256;

/// A single local child-process connection.
#[derive(Clone)]
#[allow(dead_code)] // id = diagnostics/audit field (reserved for the E3 snapshot extension)
struct Conn {
    id: u64,
    /// The child process's RoomJoin declaration (recorded at interception; target of downstream room rewriting).
    room: String,
    /// Child-process identifier (LocalEnvelope.src, the declaration on the most recent upstream message; E3 snapshot data source).
    src: String,
    /// Arrival time of the most recent upstream message (E3 snapshot data source; None = no message yet).
    last_msg: Option<Instant>,
    /// F1/T4: the real SFU host key (peer, room self-reported by the message) this downstream produced with — env.src drifts
    /// with the last message (e.g. a vision sub-connection src=xx-vision); cleanup must use produce's real key.
    producer_key: Option<(String, String)>,
    tx: mpsc::UnboundedSender<LocalEnvelope>,
}

/// Routing state (shared by two background tasks; a short-critical-section std Mutex suffices).
struct State {
    conns: HashMap<u64, Conn>,
    next_id: u64,
    /// Pending SFU request FIFO (conn ids; responses pop in order to match).
    pending: VecDeque<u64>,
    /// Recently forwarded relay message texts (echo dedup).
    echo_cache: VecDeque<String>,
    /// Vehicle peer_id (taken from the real RoomJoined; used in child-process synthetic replies).
    vehicle_peer_id: String,
    /// S0: the dialect version negotiated on the upstream (agent↔server) link (1 = old server). When synthesizing a child's
    /// RoomJoined it fills min(child claim, this value) — the child sees the end-to-end cap.
    upstream_negotiated: u32,
    /// a2 (S0.5): the one-shot resume ticket the upstream session may carry when it drops (issued by the server; cleared on take).
    pending_resume: Option<String>,
    /// Whether the remote session is live.
    joined: bool,
    /// Establishment time of the current remote session (E3 snapshot data source; cleared by reset_remote).
    remote_since: Option<Instant>,
    /// Vehicle room (immutable).
    vehicle_room: String,
    /// ConfigPush pending application (E4: server-side cloud config push; the agent polls to consume, newest overwrites older).
    pending_config: Option<SignalingMessage>,
}

impl State {
    /// Child-process upstream message → action (forward / reply locally / drop).
    fn upstream(&mut self, conn_id: u64, msg: SignalingMessage) -> UpstreamAction {
        // F1/T4: record this downstream's producer host key (produce messages self-report peer — video streams are often
        // the literal "host", a different source from env.src). Disconnect reporting sends DownstreamGone based on this.
        if let SignalingMessage::Produce { room_id, peer_id, .. } = &msg
            && let Some(c) = self.conns.get_mut(&conn_id)
        {
            c.producer_key = Some((peer_id.clone(), room_id.clone()));
        }
        match msg {
            SignalingMessage::RoomJoin { room_id, protocol: child_claim, .. } => {
                // (a) interception: the vehicle join is done by the agent; child processes get a locally synthesized RoomJoined
                if let Some(c) = self.conns.get_mut(&conn_id) {
                    c.room = room_id.clone();
                }
                if self.joined {
                    UpstreamAction::Reply(SignalingMessage::RoomJoined {
                        room_id,
                        peer_id: self.vehicle_peer_id.clone(),
                        // S0: the child sees the end-to-end cap = min(child claim (default v1), upstream negotiated).
                        protocol: Some(child_claim.unwrap_or(1).min(self.upstream_negotiated)),
                        server_version: None,
                        session_nonce: None,
                    })
                } else {
                    tracing::warn!(
                        conn_id,
                        "RoomJoin intercepted but the gateway has not connected to the server yet"
                    );
                    UpstreamAction::Reply(SignalingMessage::Error {
                        code: ERR_GATEWAY_DISCONNECTED,
                        message: "gateway not connected to server".into(),
                    })
                }
            }
            SignalingMessage::RoomLeave { .. } => {
                // forwarding a single-process upstream leave would make the server drop the vehicle session — intercept and discard
                tracing::warn!(
                    conn_id,
                    "child RoomLeave intercepted by the gateway (the vehicle session does not break on one process leaving)"
                );
                UpstreamAction::Drop
            }
            // Stream-diagnostic messages (web-stream-stats host-side completion): keep the stream sub-room declared by the streamer —
            // the server reaches browser consumers only when routing by the in-message room (relay_target_room); rewriting it into
            // the vehicle room would broadcast into a channel no browser is on. No pending slot / no echo-cache entry needed.
            m @ SignalingMessage::EncoderStatus { .. } => {
                if !self.joined {
                    return UpstreamAction::Reply(SignalingMessage::Error {
                        code: ERR_GATEWAY_DISCONNECTED,
                        message: "gateway not connected to server".into(),
                    });
                }
                UpstreamAction::Forward(m)
            }
            mut m => {
                rewrite_room(&mut m, &self.vehicle_room);
                // CRITICAL-1: the joined check precedes all state recording — requests inside the disconnect window
                // must not leave pending slots (otherwise after reconnect [A stale, C real] → C's response pops A
                // → crossed wires; single-client self-healing masked this defect)
                if !self.joined {
                    return UpstreamAction::Reply(SignalingMessage::Error {
                        code: ERR_GATEWAY_DISCONNECTED,
                        message: "gateway not connected to server".into(),
                    });
                }
                if is_sfu_request(&m) {
                    self.pending.push_back(conn_id);
                }
                // all-SFU (D decision 2026-08-25): no P2P negotiation ownership — downstream Sdp routes by room
                if is_relay_msg(&m) {
                    let text = serde_json::to_string(&m).unwrap_or_default();
                    self.echo_cache.push_back(text);
                    if self.echo_cache.len() > ECHO_CACHE_CAP {
                        self.echo_cache.pop_front();
                    }
                }
                UpstreamAction::Forward(m)
            }
        }
    }

    /// Server downstream message → target child-process list (rooms already rewritten per target).
    fn downstream(&mut self, msg: SignalingMessage) -> Vec<(u64, SignalingMessage)> {
        // echo dedup: a relay message we forwarded upstream is broadcast back by the server to the whole vehicle room; drop on identical text
        if is_relay_msg(&msg) {
            let text = serde_json::to_string(&msg).unwrap_or_default();
            if self.echo_cache.contains(&text) {
                return Vec::new();
            }
        }
        match msg {
            // SFU response: match the sender via FIFO (the single server WS processes in order)
            SignalingMessage::WebRtcTransportCreated { .. }
            | SignalingMessage::Produced { .. }
            | SignalingMessage::Consumed { .. }
            | SignalingMessage::Error { .. }
            | SignalingMessage::SfuStats { .. } => {
                // H2: SfuStats responses use the same FIFO routing
                let Some(conn_id) = self.pending.pop_front() else {
                    tracing::warn!("SFU response has no matching pending request; dropping");
                    return Vec::new();
                };
                match self.conns.get(&conn_id) {
                    Some(conn) => {
                        let mut m = msg;
                        rewrite_room(&mut m, &conn.room);
                        vec![(conn_id, m)]
                    }
                    None => {
                        tracing::warn!("SFU response target connection already closed");
                        Vec::new()
                    }
                }
            }
            // Sdp/ICE downstream: all-SFU mode routes by room (all conns whose post-rewrite room matches)
            // — concurrent multi-stream negotiations are isolated per room; no match → drop (C15 logging).
            SignalingMessage::Sdp { .. } | SignalingMessage::RTCIceCandidate { .. } => {
                let mut m = msg;
                rewrite_room(&mut m, &self.vehicle_room);
                let target_room = msg_room_id(&m).unwrap_or_default().to_string();
                let room_matches: Vec<(u64, SignalingMessage)> = self
                    .conns
                    .iter()
                    .filter(|(_, c)| c.room == target_room)
                    .map(|(id, _)| (*id, m.clone()))
                    .collect();
                if room_matches.is_empty() {
                    tracing::warn!(room = %target_room, "Sdp/ICE downstream has no room-matched connection; dropping");
                }
                room_matches
            }
            // E4: cloud config push — a message exclusive to the vehicle agent (room + target peer match),
            // not routed to child processes; stored in the pending-apply slot (the agent polls take_config_push).
            SignalingMessage::ConfigPush { ref room_id, ref target, .. } => {
                if *room_id != self.vehicle_room || *target != self.vehicle_peer_id {
                    tracing::warn!(
                        room_id = %room_id,
                        target = %target,
                        vehicle_room = %self.vehicle_room,
                        vehicle_peer = %self.vehicle_peer_id,
                        "ConfigPush target mismatch; dropping"
                    );
                    return Vec::new();
                }
                tracing::info!(room_id = %room_id, peer = %target, "ConfigPush received (pending apply)");
                self.pending_config = Some(msg);
                Vec::new()
            }
            // room-level broadcasts (NewProducer/RoomLeave/RoomJoined etc.) → everyone
            other => {
                let targets: Vec<(u64, SignalingMessage)> = self
                    .conns
                    .iter()
                    .map(|(id, conn)| {
                        let mut m = other.clone();
                        rewrite_room(&mut m, &conn.room);
                        (*id, m)
                    })
                    .collect();
                targets
            }
        }
    }

    fn remove_conn(&mut self, conn_id: u64) {
        self.conns.remove(&conn_id);
        // IMPORTANT-4: do not remove from pending (mark-dead, strict FIFO consumption) — a disconnected
        // connection's request may still be in flight at the server, and its response must pop the dead slot in order; removing
        // the slot would shift the queue ([A,B] minus A → [B]), and A's response would pop B → crossed wires
    }

    /// Remote disconnect: clear everything bound to the remote session (the server has reclaimed all transports).
    fn reset_remote(&mut self) {
        self.joined = false;
        self.remote_since = None;
        self.pending.clear();
        self.echo_cache.clear();
        self.vehicle_peer_id.clear();
        self.upstream_negotiated = 1;
        self.pending_resume = None;
    }
}

enum UpstreamAction {
    /// Forward to the remote (the caller handles the joined check and sending).
    Forward(SignalingMessage),
    /// Reply directly to this child process.
    Reply(SignalingMessage),
    Drop,
}

fn is_sfu_request(msg: &SignalingMessage) -> bool {
    matches!(
        msg,
        SignalingMessage::CreateWebRtcTransport { .. }
            | SignalingMessage::ConnectWebRtcTransport { .. }
            | SignalingMessage::Produce { .. }
            | SignalingMessage::Consume { .. }
            | SignalingMessage::SfuStatsRequest { .. } // H2: SFU stats query (FIFO response routing)
    )
}

/// Server relay whitelist messages (verified in signaling.rs) — the scope of echo dedup and P2P ownership tracking.
fn is_relay_msg(msg: &SignalingMessage) -> bool {
    matches!(
        msg,
        SignalingMessage::Sdp { .. }
            | SignalingMessage::RTCIceCandidate { .. }
            | SignalingMessage::Frame { .. }
            | SignalingMessage::EncoderStatus { .. }
    )
}

/// The message's own room id (variants without a room field return None).
fn msg_room_id(msg: &SignalingMessage) -> Option<&str> {
    use SignalingMessage::*;
    match msg {
        RoomJoin { room_id, .. }
        | RoomJoined { room_id, .. }
        | RoomLeave { room_id, .. }
        | Sdp { room_id, .. }
        | RTCIceCandidate { room_id, .. }
        | CreateWebRtcTransport { room_id, .. }
        | WebRtcTransportCreated { room_id, .. }
        | ConnectWebRtcTransport { room_id, .. }
        | Frame { room_id, .. }
        | Produce { room_id, .. }
        | Produced { room_id, .. }
        | NewProducer { room_id, .. }
        | ProducerClosed { room_id, .. }
        | EncoderStatus { room_id, .. }
        | Consume { room_id, .. }
        | Consumed { room_id, .. }
        | CreateDataProducer { room_id, .. }
        | DataProducerCreated { room_id, .. }
        | NewDataProducer { room_id, .. }
        | ConsumeData { room_id, .. }
        | DataConsumed { room_id, .. }
        | GetRouterRtpCapabilities { room_id, .. }
        | RouterRtpCapabilities { room_id, .. }
        | SetPreferredLayers { room_id, .. }
        | EmergencyCommand { room_id, .. }
        | ControlAudit { room_id, .. } => Some(room_id),
        _ => None,
    }
}

/// room_id rewriting (vehicle room ↔ child-process room; Error has no room field).
fn rewrite_room(msg: &mut SignalingMessage, room: &str) {
    use SignalingMessage::*;
    // H2: audio-room (audio-<vehicle>) messages are NOT rewritten — judge the message's **own** room_id
    // (in the upstream direction the room parameter = the vehicle room, not the message room; using the wrong value
    // folded audio-room requests into the vehicle video room — the I3 review gateway probe proved 4031 never fired).
    if msg_room_id(msg).is_some_and(|r| r.starts_with("audio-")) {
        return;
    }
    // multi-stream P3: per-stream rooms (<vehicle>_<stream>) pass through — publishers isolate a room per stream
    // (PIT-140 v2); the gateway does not fold them into the vehicle room (same principle as audio-: the message's own room_id semantics wins).
    if msg_room_id(msg).is_some_and(|r| r.starts_with(&format!("{room}_"))) {
        return;
    }
    match msg {
        RoomJoin { room_id, .. }
        | RoomJoined { room_id, .. }
        | RoomLeave { room_id, .. }
        | Sdp { room_id, .. }
        | RTCIceCandidate { room_id, .. }
        | CreateWebRtcTransport { room_id, .. }
        | WebRtcTransportCreated { room_id, .. }
        | ConnectWebRtcTransport { room_id, .. }
        | Frame { room_id, .. }
        | Produce { room_id, .. }
        | Produced { room_id, .. }
        | NewProducer { room_id, .. }
        | ProducerClosed { room_id, .. }
        | EncoderStatus { room_id, .. }
        | Consume { room_id, .. }
        | Consumed { room_id, .. }
        | CreateDataProducer { room_id, .. }
        | DataProducerCreated { room_id, .. }
        | NewDataProducer { room_id, .. }
        | ConsumeData { room_id, .. }
        | DataConsumed { room_id, .. }
        // P1 (client-dual-form): mediasoup-client negotiation-surface messages — rewritten in the same group as CreateWebRtcTransport
        | GetRouterRtpCapabilities { room_id, .. }
        | RouterRtpCapabilities { room_id, .. }
        | SetPreferredLayers { room_id, .. } => *room_id = room.to_string(),
        Error { .. } => {}
        StatusReport { .. } => {}
        // H2: SFU stats messages without a room field — no rewrite needed (downstream routes by FIFO)
        SfuStatsRequest { .. } => {}
        SfuStats { .. } => {}
        ConfigPush { .. } => {} // E4: agent-exclusive, never routed to child processes, no room rewrite needed
        DownstreamGone { .. } => {} // F1/T4: the gateway's own upstream report, never downstream-routed, no room rewrite needed
        // device-enroll: the three verification states are digested by the gateway itself, not routed to child processes (the real reply chain lands in batch 3/T7)
        DeviceAuthChallenge { .. } | DeviceAuthResponse { .. } | DeviceAuthPending { .. } => {}
        EmergencyCommand { room_id, .. } => *room_id = room.to_string(), // G3: estop room-level broadcast
        // S4/a4: estop WS audit copy (if it passes through the gateway = rewritten in the same group as EmergencyCommand; cockpit direct connections bypass it).
        ControlAudit { room_id, .. } => *room_id = room.to_string(),
    }
}

/// Signaling-plane snapshot (E3 monitoring data source; the sole holder of connection state = the gateway).
#[derive(Debug, Clone, Default)]
pub struct GatewayStatus {
    /// Local child-process WS connections (sorted by src, deterministic).
    pub children: Vec<ChildStatus>,
    /// Remote server WS connection state.
    pub remote: RemoteStatus,
}

/// Status of a single local child-process connection.
#[derive(Debug, Clone)]
pub struct ChildStatus {
    /// Child-process identifier (LocalEnvelope.src; never sent a message = empty string).
    pub src: String,
    /// Connecting (the snapshot only contains live connections, always true).
    pub connected: bool,
    /// Seconds since the most recent upstream message (0 = just received; u64::MAX = never sent).
    pub last_msg_secs: u64,
}

/// Remote server WS connection state.
#[derive(Debug, Clone, Default)]
pub struct RemoteStatus {
    /// Connected and joined.
    pub connected: bool,
    /// Seconds since the current remote session was established (not connected = None).
    pub since_secs: Option<u64>,
    /// Vehicle peer_id (not connected = empty string).
    pub peer_id: String,
}

/// a3 (S0.5): dual upstream queues. hi = unbounded (auth/control/terminal — small volume with bounded growth, never dropped);
/// lo = bounded best-effort (closed whitelist {StatusReport}, full = drop + count — dropping at the **entry** is what preserves freshness;
/// backlog in gateway memory goes stale just the same). The real drop point under network-congestion backpressure is the link session's lo (same shape).
#[derive(Clone)]
struct UpstreamQueue {
    hi: mpsc::UnboundedSender<SignalingMessage>,
    lo: mpsc::Sender<SignalingMessage>,
    dropped: Arc<std::sync::atomic::AtomicU64>,
}

impl UpstreamQueue {
    fn new() -> (Self, mpsc::UnboundedReceiver<SignalingMessage>, mpsc::Receiver<SignalingMessage>)
    {
        let (hi, hi_rx) = mpsc::unbounded_channel();
        let (lo, lo_rx) = mpsc::channel(16);
        (Self { hi, lo, dropped: Arc::new(std::sync::atomic::AtomicU64::new(0)) }, hi_rx, lo_rx)
    }

    /// Route into a queue: whitelist → lo try_send (full = drop + count); everything else → hi (unbounded, only Err on a closed channel).
    fn push(&self, msg: SignalingMessage) -> Result<(), String> {
        if matches!(msg, SignalingMessage::StatusReport { .. }) {
            if self.lo.try_send(msg).is_err() {
                let n = self.dropped.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                if n == 1 || n.is_multiple_of(64) {
                    tracing::warn!(
                        "a3: gateway low-priority queue full, StatusReport dropped (cumulative {n})"
                    );
                }
            }
            return Ok(());
        }
        self.hi.send(msg).map_err(|_| "upstream closed".into())
    }
}

/// Gateway runtime handle (E3) — monitoring snapshot + remote reporting channel.
#[derive(Clone)]
pub struct GatewayHandle {
    state: Arc<Mutex<State>>,
    upstream: UpstreamQueue,
}

impl GatewayHandle {
    /// Signaling-plane snapshot (E3 monitoring data source).
    pub fn snapshot(&self) -> GatewayStatus {
        let st = lock_state(&self.state);
        let now = Instant::now();
        let mut children: Vec<ChildStatus> = st
            .conns
            .values()
            .map(|c| ChildStatus {
                src: c.src.clone(),
                connected: true,
                last_msg_secs: c
                    .last_msg
                    .map_or(u64::MAX, |t| now.saturating_duration_since(t).as_secs()),
            })
            .collect();
        children.sort_by(|a, b| a.src.cmp(&b.src));
        GatewayStatus {
            children,
            remote: RemoteStatus {
                connected: st.joined,
                since_secs: st.remote_since.map(|t| now.saturating_duration_since(t).as_secs()),
                peer_id: st.vehicle_peer_id.clone(),
            },
        }
    }

    /// Send via the gateway's remote WS (joined check + Err on failure — the caller must log, C15).
    /// When not joined, refuse entry to the channel: messages in the disconnect window would be silently drained away by
    /// remote_loop, so they are intercepted at the source.
    pub fn send_remote(&self, msg: SignalingMessage) -> Result<(), String> {
        let st = lock_state(&self.state);
        if !st.joined {
            return Err("gateway not connected to server".into());
        }
        self.upstream.push(msg).map_err(|_| "remote session closed".into())
    }

    /// Take the pending ConfigPush (E4; None = nothing pending; newest overwrites older).
    pub fn take_config_push(&self) -> Option<SignalingMessage> {
        lock_state(&self.state).pending_config.take()
    }

    /// Vehicle room (the data source for reported room_id).
    pub fn vehicle_room(&self) -> String {
        lock_state(&self.state).vehicle_room.clone()
    }
}

fn lock_state(state: &Arc<Mutex<State>>) -> MutexGuard<'_, State> {
    state.lock().unwrap_or_else(|e| e.into_inner())
}

/// Envelope JSON serialization (theoretically infallible: all SignalingMessage fields are serializable; on failure drop and warn).
fn env_json(env: &LocalEnvelope) -> Option<String> {
    match serde_json::to_string(env) {
        Ok(t) => Some(t),
        Err(e) => {
            tracing::error!("envelope serialization failed: {e}");
            None
        }
    }
}

fn gateway_disconnected_error() -> SignalingMessage {
    SignalingMessage::Error {
        code: ERR_GATEWAY_DISCONNECTED,
        message: "gateway not connected to server".into(),
    }
}

/// Start the gateway: bind the local port + resident background tasks (local accept / remote connect + reconnect).
///
/// Returns the actually bound port (`local_port = 0` → ephemeral). The caller holds the runtime
/// (bins wait for signals; tests simply end with the runtime).
/// Returns (actually bound port, runtime handle). The caller holds the runtime
/// (bins wait for signals; tests simply end with the runtime).
pub async fn run_gateway(config: GatewayConfig) -> Result<(u16, GatewayHandle), String> {
    let listener = TcpListener::bind(("127.0.0.1", config.local_port))
        .await
        .map_err(|e| format!("bind local gateway :{}: {e}", config.local_port))?;
    let port = listener.local_addr().map_err(|e| format!("local_addr: {e}"))?.port();

    let state = Arc::new(Mutex::new(State {
        conns: HashMap::new(),
        next_id: 1,
        pending: VecDeque::new(),
        echo_cache: VecDeque::new(),
        vehicle_peer_id: String::new(),
        upstream_negotiated: 1,
        pending_resume: None,
        joined: false,
        remote_since: None,
        vehicle_room: config.room.clone(),
        pending_config: None,
    }));
    let (upstream, remote_hi_rx, remote_lo_rx) = UpstreamQueue::new();
    let handle = GatewayHandle { state: Arc::clone(&state), upstream: upstream.clone() };

    // local accept loop
    let accept_state = Arc::clone(&state);
    let accept_tx = upstream.clone();
    tokio::spawn(async move {
        loop {
            let (stream, peer) = match listener.accept().await {
                Ok(v) => v,
                Err(e) => {
                    tracing::error!("local accept failed: {e}");
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    continue;
                }
            };
            let ws = match tokio_tungstenite::accept_async(stream).await {
                Ok(w) => w,
                Err(e) => {
                    tracing::warn!("local WS handshake failed {peer}: {e}");
                    continue;
                }
            };
            let (conn_id, out_rx) = {
                let mut st = lock_state(&accept_state);
                let (tx, rx) = mpsc::unbounded_channel();
                let id = st.next_id;
                st.next_id += 1;
                st.conns.insert(
                    id,
                    Conn {
                        id,
                        room: String::new(),
                        src: String::new(),
                        last_msg: None,
                        producer_key: None,
                        tx,
                    },
                );
                (id, rx)
            };
            tracing::info!(conn_id, peer = %peer, "local child process connected");
            let st = Arc::clone(&accept_state);
            let rtx = accept_tx.clone();
            tokio::spawn(conn_task(conn_id, ws, out_rx, st, rtx));
        }
    });

    // remote connect + reconnect loop
    tokio::spawn(remote_loop(config, state, remote_hi_rx, remote_lo_rx));

    Ok((port, handle))
}

/// One local connection task: read envelope → route → forward/reply; write downstream envelopes.
async fn conn_task(
    conn_id: u64,
    ws: WebSocketStream<TcpStream>,
    mut out_rx: mpsc::UnboundedReceiver<LocalEnvelope>,
    state: Arc<Mutex<State>>,
    remote_tx: UpstreamQueue,
) {
    let (mut ws_tx, mut ws_rx) = ws.split();
    loop {
        tokio::select! {
            incoming = ws_rx.next() => {
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        let env: LocalEnvelope = match serde_json::from_str(&text) {
                            Ok(e) => e,
                            Err(e) => {
                                tracing::warn!(conn_id, "local envelope parse failed: {e}");
                                continue;
                            }
                        };
                        // CRITICAL-2: push (inside upstream) and send (here) must share one
                        // critical section — two conn_tasks interleaving (A push, B push, B send, A
                        // send) makes wire order ≠ pending order → response mismatch. The Unbounded
                        // Sender sends synchronously with no await, safe inside the lock.
                        let mut reply: Option<SignalingMessage> = None;
                        {
                            let mut st = lock_state(&state);
                            // E3: snapshot data source — src declaration + last-message time (same critical section)
                            if let Some(c) = st.conns.get_mut(&conn_id) {
                                c.src = env.src.clone();
                                c.last_msg = Some(Instant::now());
                            }
                            match st.upstream(conn_id, env.msg) {
                                UpstreamAction::Forward(msg) => {
                                    if remote_tx.push(msg).is_err() {
                                        // the remote task has exited (gateway closing): roll back this request's
                                        // pending slot (the only push inside the lock, pop_back is safe)
                                        st.pending.pop_back();
                                        tracing::warn!(conn_id, "remote task exited");
                                        break;
                                    }
                                }
                                UpstreamAction::Reply(m) => reply = Some(m),
                                UpstreamAction::Drop => {}
                            }
                        }
                        if let Some(msg) = reply
                            && let Some(t) = env_json(&LocalEnvelope { src: "server".into(), msg })
                                && ws_tx.send(Message::Text(t)).await.is_err() {
                                    break;
                                }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {}
                    Some(Err(e)) => {
                        tracing::warn!(conn_id, "local WS read error: {e}");
                        break;
                    }
                }
            }
            outgoing = out_rx.recv() => {
                match outgoing {
                    Some(env) => {
                        let Some(text) = env_json(&env) else { continue };
                        if ws_tx.send(Message::Text(text)).await.is_err() {
                            break;
                        }
                    }
                    None => break,
                }
            }
        }
    }
    // F1/T4: a downstream child process died → report to the server for stream-granular cleanup (producer host keys span
    // rooms; only the (src, room) pairing pinpoints one; whole-vehicle-granularity disconnect lookups are backstopped by the existing chain, no interference).
    let gone = {
        let st = lock_state(&state);
        st.conns.get(&conn_id).and_then(|c| c.producer_key.clone())
    };
    if let Some((peer, room)) = gone {
        tracing::info!(conn_id, "DownstreamGone reported: peer={peer} room={room}");
        if remote_tx
            .push(SignalingMessage::DownstreamGone { peer_id: peer, room_id: room })
            .is_err()
        {
            tracing::debug!(
                conn_id,
                "DownstreamGone dropped (upstream down; the vehicle-wide reverse lookup will backstop)"
            );
        }
    }
    lock_state(&state).remove_conn(conn_id);
    tracing::info!(conn_id, "local child process disconnected");
}

/// Single remote WS loop: connect (B1 reconnect) → bidirectional forwarding → disconnect cleanup → reconnect.
async fn remote_loop(
    config: GatewayConfig,
    state: Arc<Mutex<State>>,
    mut remote_hi_rx: mpsc::UnboundedReceiver<SignalingMessage>,
    mut remote_lo_rx: mpsc::Receiver<SignalingMessage>,
) {
    let mut client =
        SignalClient::new(&config.remote_url, &config.psk, &config.room, PeerRole::Host);
    // device-enroll T7: the device identity rides on Join via device_pubkey (None = PSK path)
    if let Some(ident) = config.device.clone() {
        client = client.with_device_identity(ident);
    }
    // H6 (timing fix): the upstream "down→recovered" reconnect flag — the notification is emitted only after a successful reconnect.
    // Notifying at the moment of disconnection makes streamers restart every 1-2s during the server-outage window → the oxmgr
    // crash-loop circuit breaker (3 times/5min) trips → never recovers. Notifying after a successful reconnect = server ready, one shot hits.
    let mut reconnected = false;
    loop {
        let session = match client.connect_with_retry(config.retry).await {
            Ok(s) => s,
            Err(LinkError::EnrollPending { device_id }) => {
                tracing::warn!(
                    "device awaiting admin approval device_id={device_id} (public key already pending), reconnecting in 10s"
                );
                tokio::time::sleep(Duration::from_secs(10)).await;
                continue;
            }
            Err(e) => {
                tracing::error!("remote signaling connect failed: {e}, retrying in 10s");
                tokio::time::sleep(Duration::from_secs(10)).await;
                continue;
            }
        };
        tracing::info!(room = %config.room, "agent joined the vehicle room");
        {
            let mut st = lock_state(&state);
            st.joined = true;
            st.remote_since = Some(Instant::now());
            st.vehicle_peer_id = session.peer_id().to_string();
            st.upstream_negotiated = session.negotiated_protocol();
            st.pending_resume = session.session_nonce().map(str::to_string);
        }
        if reconnected {
            let notify: Vec<_> = {
                let st = lock_state(&state);
                st.conns.values().map(|c| (c.tx.clone(), c.src.clone())).collect()
            };
            for (tx, src) in &notify {
                let env = LocalEnvelope { src: "server".into(), msg: gateway_disconnected_error() };
                if tx.send(env).is_err() {
                    tracing::warn!(src = %src, "upstream-recovery notify target connection already closed");
                }
            }
            tracing::info!(
                children = notify.len(),
                "H6: upstream recovered, asking downstream to rebuild SFU sessions"
            );
            let _ = reconnected; // keep the reset-bit semantics; the current branch has no later read (clippy unused_assignments)
        }
        // a2: this session ending = disconnection. Hand the server-issued resume ticket to the client (the next
        // connect_with_retry carries resume; a miss naturally falls back to a full join — the ticket is one-shot).
        client.set_resume_ticket(lock_state(&state).pending_resume.take());
        run_session(session, &state, &mut remote_hi_rx, &mut remote_lo_rx).await;
        // disconnect: clear state bound to the remote session + drop in-flight upstreams (handled as a new session after reconnect)
        lock_state(&state).reset_remote();
        while remote_hi_rx.try_recv().is_ok() {}
        while remote_lo_rx.try_recv().is_ok() {}
        reconnected = true;
        tracing::info!("remote disconnected, reconnecting...");
    }
}

/// A single remote session: events → downstream routing; upstream channels → session.send.
async fn run_session(
    session: SignalSession,
    state: &Arc<Mutex<State>>,
    remote_hi_rx: &mut mpsc::UnboundedReceiver<SignalingMessage>,
    remote_lo_rx: &mut mpsc::Receiver<SignalingMessage>,
) {
    let mut events = session.events();
    loop {
        // fair poll (do NOT biased — an events flood would starve upstream; the hi>lo ordering lives at the link network egress).
        tokio::select! {
            ev = events.recv() => match ev {
                Ok(SignalEvent::Message(msg)) => {
                    let targets = lock_state(state).downstream(msg);
                    for (conn_id, m) in targets {
                        let tx = lock_state(state).conns.get(&conn_id).map(|c| c.tx.clone());
                        if let Some(tx) = tx {
                            let env = LocalEnvelope { src: "server".into(), msg: m };
                            if tx.send(env).is_err() {
                                tracing::warn!(conn_id, "downstream target connection already closed");
                            }
                        }
                    }
                }
                Ok(SignalEvent::Disconnected { reason }) => {
                    tracing::warn!("remote disconnected: {reason}");
                    break;
                }
                Ok(SignalEvent::Error(e)) => tracing::warn!("remote signaling error: {e}"),
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!("remote event stream lagged by {n}");
                }
                Err(_) => break, // session channel closed = disconnected
            },
            // a3: hi drains before lo (same order contract as the link layer); lo closing does not end the session.
            msg = remote_hi_rx.recv(), if !remote_hi_rx.is_closed() => match msg {
                Some(m) => {
                    if let Err(e) = session.send(m).await {
                        tracing::warn!("remote send failed: {e}");
                        break;
                    }
                }
                None => return, // gateway closing
            },
            msg = remote_lo_rx.recv(), if !remote_lo_rx.is_closed() => {
                if let Some(m) = msg
                    && let Err(e) = session.send(m).await {
                        tracing::warn!("remote send failed: {e}");
                        break;
                    }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// H2 (I3 review): audio-room messages are not rewritten — the check uses the message's own room_id (in the upstream
    /// direction the room parameter = the vehicle room; an early implementation misused it and folded audio rooms into the vehicle room).
    #[test]
    fn rewrite_room_passthrough_audio_rooms() {
        use mediaservo_common::protocol::TransportDirection;
        let mut msg = SignalingMessage::CreateWebRtcTransport {
            room_id: "audio-ms-car1".into(),
            peer_id: "audio".into(),
            direction: TransportDirection::Send,
        };
        rewrite_room(&mut msg, "vehicle");
        match &msg {
            SignalingMessage::CreateWebRtcTransport { room_id, .. } => {
                assert_eq!(room_id, "audio-ms-car1", "the audio room must stay as-is (no rewrite)")
            }
            other => panic!("unexpected variant: {other:?}"),
        }

        // non-audio rooms are rewritten as usual
        let mut msg2 = SignalingMessage::CreateWebRtcTransport {
            room_id: "stream-cam0".into(),
            peer_id: "s".into(),
            direction: TransportDirection::Send,
        };
        rewrite_room(&mut msg2, "vehicle");
        match &msg2 {
            SignalingMessage::CreateWebRtcTransport { room_id, .. } => {
                assert_eq!(
                    room_id, "vehicle",
                    "non-audio rooms must be rewritten to the vehicle room"
                )
            }
            other => panic!("unexpected variant: {other:?}"),
        }

        // multi-stream P3: per-stream rooms (<vehicle>_<stream>) pass through
        let mut msg3 = SignalingMessage::CreateWebRtcTransport {
            room_id: "vehicle_test-30fps".into(),
            peer_id: "s".into(),
            direction: TransportDirection::Send,
        };
        rewrite_room(&mut msg3, "vehicle");
        match &msg3 {
            SignalingMessage::CreateWebRtcTransport { room_id, .. } => {
                assert_eq!(room_id, "vehicle_test-30fps", "per-stream rooms must pass through")
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    fn test_state() -> Arc<Mutex<State>> {
        Arc::new(Mutex::new(State {
            conns: HashMap::new(),
            next_id: 1,
            pending: VecDeque::new(),
            echo_cache: VecDeque::new(),
            vehicle_peer_id: String::new(),
            upstream_negotiated: 1,
            pending_resume: None,
            joined: false,
            remote_since: None,
            vehicle_room: "vehicle-1".into(),
            pending_config: None,
        }))
    }

    fn handle_for(
        state: Arc<Mutex<State>>,
    ) -> (GatewayHandle, mpsc::UnboundedReceiver<SignalingMessage>) {
        let (upstream, hi_rx, lo_rx) = UpstreamQueue::new();
        // synchronous tests without a runtime merge the dual queues: this group only feeds hi (Sdp/terminal families);
        // the lo receiver is kept alive to avoid a spurious try_send Closed count.
        std::mem::forget(lo_rx);
        (GatewayHandle { state, upstream }, hi_rx)
    }

    #[test]
    fn snapshot_reports_children_and_remote() {
        let state = test_state();
        {
            let mut st = lock_state(&state);
            st.joined = true;
            st.vehicle_peer_id = "veh-peer".into();
            st.remote_since = Some(Instant::now());
            st.conns.insert(
                1,
                Conn {
                    id: 1,
                    room: "room-a".into(),
                    src: "host-streamer".into(),
                    last_msg: Some(Instant::now()),
                    producer_key: None,
                    tx: mpsc::unbounded_channel().0,
                },
            );
            st.conns.insert(
                2,
                Conn {
                    id: 2,
                    room: "room-b".into(),
                    src: "host-capturer".into(),
                    last_msg: None,
                    producer_key: None,
                    tx: mpsc::unbounded_channel().0,
                },
            );
        }
        let (handle, _rx) = handle_for(state);
        let snap = handle.snapshot();
        // children sorted by src (deterministic)
        assert_eq!(snap.children.len(), 2);
        assert_eq!(snap.children[0].src, "host-capturer");
        assert!(snap.children[0].connected);
        assert_eq!(snap.children[0].last_msg_secs, u64::MAX, "never sent = u64::MAX");
        assert_eq!(snap.children[1].src, "host-streamer");
        assert_eq!(snap.children[1].last_msg_secs, 0, "just received = 0");
        assert!(snap.remote.connected);
        assert_eq!(snap.remote.since_secs, Some(0));
        assert_eq!(snap.remote.peer_id, "veh-peer");
        assert_eq!(handle.vehicle_room(), "vehicle-1");
    }

    #[test]
    fn send_remote_rejects_when_not_joined() {
        let state = test_state();
        let (handle, mut rx) = handle_for(state);
        let err = handle
            .send_remote(SignalingMessage::Sdp {
                room_id: "r".into(),
                target: None,
                sdp: "v=0".into(),
            })
            .unwrap_err();
        assert!(err.contains("not connected"), "must refuse when not joined: {err}");
        assert!(rx.try_recv().is_err(), "a refused message must not reach the remote channel");
    }

    #[test]
    fn send_remote_forwards_when_joined() {
        let state = test_state();
        lock_state(&state).joined = true;
        let (handle, mut rx) = handle_for(state);
        let msg = SignalingMessage::Sdp { room_id: "r".into(), target: None, sdp: "v=0".into() };
        handle.send_remote(msg.clone()).expect("should send when joined");
        assert!(matches!(rx.try_recv(), Ok(m) if matches!(m, SignalingMessage::Sdp { .. })));
    }
    #[test]
    fn config_push_stored_when_targeted_at_vehicle() {
        let state = test_state();
        lock_state(&state).vehicle_peer_id = "veh-peer".into();
        let push = SignalingMessage::ConfigPush {
            room_id: "vehicle-1".into(),
            target: "veh-peer".into(),
            config: "sources:\n".into(),
            version: 3,
        };
        let targets = lock_state(&state).downstream(push.clone());
        assert!(targets.is_empty(), "ConfigPush must not enter child-process routing: {targets:?}");
        let handle = handle_for(state).0;
        match handle.take_config_push() {
            Some(SignalingMessage::ConfigPush { version, .. }) => assert_eq!(version, 3),
            other => panic!("expected ConfigPush, got {other:?}"),
        }
        assert!(handle.take_config_push().is_none(), "take should clear");
    }

    #[test]
    fn config_push_ignored_for_other_target_or_room() {
        let state = test_state();
        lock_state(&state).vehicle_peer_id = "veh-peer".into();
        let wrong_peer = SignalingMessage::ConfigPush {
            room_id: "vehicle-1".into(),
            target: "other-peer".into(),
            config: "cfg".into(),
            version: 1,
        };
        assert!(lock_state(&state).downstream(wrong_peer).is_empty());
        let wrong_room = SignalingMessage::ConfigPush {
            room_id: "other-room".into(),
            target: "veh-peer".into(),
            config: "cfg".into(),
            version: 1,
        };
        assert!(lock_state(&state).downstream(wrong_room).is_empty());
        let handle = handle_for(state).0;
        assert!(
            handle.take_config_push().is_none(),
            "a mismatched push must not enter the pending slot"
        );
    }
}
