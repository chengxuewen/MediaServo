//! Signaling protocol message types.
//!
//! All signaling messages flow through the Server's WebSocket /ws endpoint
//! as JSON. Server relays messages between Host and Remote without modification
//! (except for room management messages).

use serde::{Deserialize, Serialize};

// ── S0: protocol version negotiation (invariant I5: capability gates only look at the negotiated integer;
// package version / protocol integer / schema-ABI stay on three separate planes). v1 = no protocol field on the wire
// (legacy shape byte-identical); v2 = the datachannel.control domain (F8 control DC gate) opens;
// v3 = session resume (S0.5: resume/session_nonce - integer monotonicity is what makes dialect exact:
/// Highest dialect supported by the endpoints in this repo.
pub const SIGNALING_PROTOCOL_VERSION: u32 = 3;
/// Lowest dialect accepted for a connection (legacy clients that do not declare protocol = v1; below this -> Error 4101).
pub const SIGNALING_PROTOCOL_MIN_SUPPORTED: u32 = 1;
/// Lowest dialect needed to open the control DC domain (create_data_producer).
pub const PROTOCOL_MIN_CONTROL_DC: u32 = 2;
/// Lowest dialect needed for session resume (resume request accepted).
pub const PROTOCOL_MIN_RESUME: u32 = 3;

/// Negotiation result = min(client claim (None = v1), server max). The reject-low shape is handled up front by the
/// caller (claim < MIN_SUPPORTED -> 4101); this function only converges.
#[must_use]
pub fn negotiate_protocol(claim: Option<u32>) -> u32 {
    claim.unwrap_or(1).min(SIGNALING_PROTOCOL_VERSION)
}

/// A signaling message exchanged via WebSocket.
///
/// # Flow
/// ```text
/// Host ──WS──▶ Server ──WS──▶ Remote
/// Remote ──WS──▶ Server ──WS──▶ Host
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SignalingMessage {
    /// Request to join a room. Sent by Host or Remote to Server.
    RoomJoin {
        room_id: String,
        peer_role: PeerRole,
        #[serde(skip_serializing_if = "Option::is_none")]
        stream_id: Option<String>,
        /// G4 device credential (additive, D-H11): when present the server uses device authentication (verified since G2);
        /// absent = PSK auth path. Old servers ignore unknown fields and old clients omit them - both directions compatible.
        #[serde(skip_serializing_if = "Option::is_none")]
        device_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        device_secret: Option<String>,
        /// device-enroll public-key admission (additive, D-E3 coexists with secret for one cycle): base64 32B Ed25519
        /// verifying key, present or absent together with device_id; when present it triggers the server signature challenge chain (design §5.2).
        /// absent = the legacy secret/PSK path, byte-identical; old servers ignore unknown fields = no degradation.
        #[serde(skip_serializing_if = "Option::is_none")]
        device_pubkey: Option<String>,
        /// S0 protocol negotiation (additive): absent = v1 (wire byte-identical before 2026-09-14;
        /// old server / old host are mutually unaware - no upgrade ordering constraint). Server joins the session with min(claim, max).
        #[serde(skip_serializing_if = "Option::is_none")]
        protocol: Option<u32>,
        /// Observation slot: the client own version string; takes part in no gate decision.
        #[serde(skip_serializing_if = "Option::is_none")]
        client_version: Option<String>,
        /// a2 session resume (additive, only consumed at negotiated>=3): carries the session_nonce
        /// issued last time as a reattach index - **not a credential**; the auth chain always reruns (D283 revocation takes effect immediately). Absent = full join.
        #[serde(skip_serializing_if = "Option::is_none")]
        resume: Option<String>,
    },

    /// Room join acknowledged by Server.
    RoomJoined {
        room_id: String,
        peer_id: String,
        /// S0: server echoes the agreed value (absent = old server, endpoints fall back to v1).
        #[serde(skip_serializing_if = "Option::is_none")]
        protocol: Option<u32>,
        /// Observation slot: server version string.
        #[serde(skip_serializing_if = "Option::is_none")]
        server_version: Option<String>,
        /// a2: one-time reattach ticket (>=32B CSPRNG base64; only issued at negotiated>=3). It only indexes the
        /// resume session snapshot - not bound to a source IP (changing IP is a designed scenario); use immediately, burn on use.
        #[serde(skip_serializing_if = "Option::is_none")]
        session_nonce: Option<String>,
    },

    /// A peer has left the room. Broadcast by Server.
    RoomLeave { room_id: String, peer_id: String },

    /// SDP offer/answer relayed through Server.
    Sdp { room_id: String, target: Option<String>, sdp: String },

    /// ICE candidate relayed through Server.
    /// PIT-106 (I2 review): `alias = "rtc_ice_candidate"` - the wire name browsers conventionally use
    /// (sent by sfu-client.ts); the canonical name r_t_c_ice_candidate is kept for backward compatibility (host/Rust clients).
    #[serde(alias = "rtc_ice_candidate")]
    RTCIceCandidate {
        room_id: String,
        target: Option<String>,
        candidate: String,
        sdp_mid: Option<String>,
        sdp_mline_index: Option<u16>,
    },

    // ── SFU transport negotiation (mediasoup) ────────────────────
    /// Request Server to create a WebRTC transport for this peer.
    /// The Server (SFU) creates the transport and returns parameters.
    CreateWebRtcTransport { room_id: String, peer_id: String, direction: TransportDirection },

    /// Server responds with transport parameters needed by the client.
    WebRtcTransportCreated {
        room_id: String,
        peer_id: String,
        transport_id: String,
        ice_parameters: IceParameters,
        dtls_parameters: DtlsParameters,
        /// ICE candidates for the transport (None for backward compat).
        #[serde(skip_serializing_if = "Option::is_none")]
        ice_candidates: Option<Vec<IceCandidate>>,
        /// P1 (client-dual-form): transport-level SCTP parameters (mediasoup SctpParameters serialization
        /// passed through as-is; None = backward compatible). Opaque Value following the rtp_parameters precedent - browser-side
        /// field mapping (OS/MIS/maxMessageSize) belongs to the single TS proto handler.
        #[serde(skip_serializing_if = "Option::is_none")]
        sctp_parameters: Option<serde_json::Value>,
    },

    /// Client sends back DTLS parameters to connect the transport.
    ConnectWebRtcTransport {
        room_id: String,
        peer_id: String,
        transport_id: String,
        dtls_parameters: DtlsParameters,
    },

    /// P1 (client-dual-form): the client asks for the room Router RTP capabilities -
    /// the input of mediasoup-client Device.load() (C18 official negotiation flow, first step).
    GetRouterRtpCapabilities { room_id: String },

    /// Server returns Router RTP capabilities (mediasoup RtpCapabilitiesFinalized serialization
    /// passed through as-is - opaque Value following the rtp_parameters precedent; hand-building is forbidden).
    RouterRtpCapabilities { room_id: String, capabilities: serde_json::Value },

    /// P1: consumer layer preference (simulcast/SVC layer selection - server-side entry for conference P4a bandwidth adaptation).
    /// ack follows the Error{code:0} precedent (same shape as transport_connected).
    SetPreferredLayers {
        room_id: String,
        peer_id: String,
        consumer_id: String,
        spatial_layer: u8,
        #[serde(skip_serializing_if = "Option::is_none")]
        temporal_layer: Option<u8>,
    },

    /// Error response from Server.
    Error { code: u16, message: String },

    /// Encoded media frame relayed through Server.
    /// data_base64 is encoded as base64 (JSON-safe).
    Frame { room_id: String, codec: String, sequence: u64, is_keyframe: bool, data_base64: String },

    // ── SFU produce/consume (mediasoup) ─────────────────────────
    /// Peer asks to produce media on its send transport.
    /// rtp_parameters is opaque JSON — server passes it through to mediasoup.
    Produce {
        room_id: String,
        peer_id: String,
        transport_direction: TransportDirection,
        kind: MediaKind,
        rtp_parameters: serde_json::Value,
        /// C1 (D-H14 order-independent): bind the target send transport (the
        /// transport_id returned by CreateWebRtcTransport). None = legacy single-slot fallback (most recently created send transport).
        #[serde(skip_serializing_if = "Option::is_none")]
        transport_id: Option<String>,
        /// producer-label T1 (device room + stream label, prerequisite for PIT-140 v2 retirement): stream business label (e.g. "fl"/"fm").
        /// additive: old servers ignore unknown fields, old clients omit them - both directions compatible (G4 device_id precedent).
        /// None = no label (old host shape) - the consumer side falls back to matching "the first stream in the room".
        #[serde(skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },

    /// Server confirms producer created.
    Produced { room_id: String, producer_id: String },

    /// Server broadcasts a new producer to all peers in the room.
    NewProducer {
        room_id: String,
        producer_id: String,
        peer_id: String,
        kind: MediaKind,
        /// producer-label T1: Produce.label passthrough (None = old host shape without a label).
        #[serde(skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },

    /// v2 (web-stream-stats T1): Host periodically reports encoder status (relayed by room broadcast to the browser).
    /// encoder_implementation: the actual libwebrtc outbound-rtp encoder name (software vs hardware encoder detection); None = unavailable.
    EncoderStatus {
        room_id: String,
        peer_id: String,
        codec: String,
        encoder_backend: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        encoder_implementation: Option<String>,
        frames_per_second: f64,
        frame_width: u32,
        frame_height: u32,
        /// v3 (encode-time-stats T1): average encode time per frame (ms/frame, incrementally computed on the Host) -
        /// Delta totalEncodeTime / Delta framesEncoded * 1000; None = unavailable (old host / first window).
        #[serde(skip_serializing_if = "Option::is_none")]
        avg_encode_ms: Option<f64>,
    },

    /// v4 (E3 host-multiprocess): host-agent whole-vehicle status report - topology + data flow + signaling
    /// three snapshots aggregated and periodically reported over the gateway remote WS (default 5s). The Server consumes and stores it directly
    /// (not a relay message, not broadcast in the room; an old Server failing to parse silently drops it = tolerable,
    /// periodic reporting self-heals in the next window).
    StatusReport {
        room_id: String,
        /// Data plane: data flow statistics per camera topic (E2 snapshot).
        topics: Vec<TopicFlowJson>,
        /// Data plane: push status per streamer (E2 snapshot).
        streams: Vec<StreamFlowJson>,
        /// Topology plane: union of expected and actual processes (E1 snapshot).
        processes: Vec<ProcessStateJson>,
        /// Signaling plane: connection state from the gateway perspective (E3 snapshot).
        signal: SignalStatusJson,
        /// Report timestamp (unix seconds).
        ts: u64,
        /// host.toml config version (tied to E4 ConfigPush; currently always 0).
        config_version: u64,
    },

    /// Peer asks to consume a producer on its recv transport.
    /// rtp_capabilities is opaque JSON — server passes it through to mediasoup.
    Consume {
        room_id: String,
        peer_id: String,
        producer_id: String,
        rtp_capabilities: serde_json::Value,
        /// C1: bind the target recv transport. None = legacy single-slot fallback (most recently created recv transport)
        /// - fixes recv_transport being overwritten across multiple connections sharing a peer_id -> consumer attached to the wrong transport, black screen.
        #[serde(skip_serializing_if = "Option::is_none")]
        transport_id: Option<String>,
    },

    /// Server confirms consumer created.
    Consumed {
        room_id: String,
        consumer_id: String,
        producer_id: String,
        kind: MediaKind,
        /// RTP parameters needed by the consumer to decode the stream.
        rtp_parameters: serde_json::Value,
    },
    /// v5 (E4 cloud config loop): Server -> host-agent whole-vehicle config delivery.
    /// config is the full host.toml text; target = whole-vehicle peer_id (other peers in the room ignore it);
    /// version ties to StatusReport.config_version (the agent reports it back after applying).
    ConfigPush { room_id: String, target: String, config: String, version: u64 },
    /// G3 emergency-stop command (cockpit -> server -> vehicle room): server-side strongly audited path (D-H11 estop strong audit).
    /// Chassis/gimbal regular control stays on the P2P DC (authorized by role during negotiation, invisible to the server); estop must
    /// leave a trace (who / when / which vehicle / which command) -> forwarded over signaling, then the host-agent hands it to the local controller.
    /// command semantics: vehicle-side convention (e.g. "e-stop"); the server only forwards and audits.
    EmergencyCommand { room_id: String, command: String },

    /// H1 (SFU data domain): the peer created a DataProducer on its send transport (SCTP DataChannel).
    /// sctp_stream_parameters is required (an SCTP producer needs a stream_id; Option only for wire tolerance,
    /// the server reports an explicit error when it is missing). label/protocol distinguish DataChannels such as control/vision.
    CreateDataProducer {
        room_id: String,
        peer_id: String,
        transport_direction: TransportDirection,
        /// DataChannel label (e.g. "control" / "vision").
        label: String,
        /// DataChannel sub-protocol name (e.g. "mediaservo.control").
        protocol: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        sctp_stream_parameters: Option<SctpStreamParameters>,
        /// C1: bind the target send transport. None = legacy single-slot fallback.
        #[serde(skip_serializing_if = "Option::is_none")]
        transport_id: Option<String>,
    },

    /// Server confirms data producer created.
    DataProducerCreated { room_id: String, data_producer_id: String },

    /// Server broadcasts a new data producer to all peers in the room (late-joiner sync).
    NewDataProducer {
        room_id: String,
        data_producer_id: String,
        peer_id: String,
        label: String,
        protocol: String,
    },

    /// Peer asks to consume a data producer on its recv transport.
    ConsumeData {
        room_id: String,
        peer_id: String,
        transport_direction: TransportDirection,
        data_producer_id: String,
        /// C1: bind the target recv transport. None = legacy single-slot fallback.
        #[serde(skip_serializing_if = "Option::is_none")]
        transport_id: Option<String>,
    },

    /// Server confirms data consumer created.
    /// S2d: official contract (mediasoup-client Chrome74.receiveDataChannel) = the consumer side must create the channel as a
    /// negotiated DC (id=streamId, negotiated out of band) to receive worker-forwarded messages - the worker never
    /// emits the in-band DCEP handshake. So the ack carries the consumer sctp parameters / label / protocol.
    /// Option+default keeps the old wire readable (missing fields = old server; a new client reports an explicit error instead of degrading silently).
    DataConsumed {
        room_id: String,
        data_consumer_id: String,
        data_producer_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        // S2d: tolerant of old servers omitting the field
        sctp_stream_parameters: Option<SctpStreamParameters>,
        #[serde(default)] // S2d: consumer DC label (used to build the channel)
        label: String,
        #[serde(default)] // S2d: consumer DC sub-protocol
        protocol: String,
    },

    /// S4/a4: emergency-stop WS audit copy (the cockpit sends it alongside the DC fast path, best-effort).
    /// Primary audit sink = the host actuator actuation log (the DC does not pass through the server, so the copy may be lost - PLAN §11.6
    /// slot-3 verdict); the server side only writes the audit ring plus a WARN trace, and neither forwards nor adjudicates execution.
    ControlAudit {
        room_id: String,
        seq: u64,
        cmd: String,
        /// Signature-present flag (the payload is not copied - audit size and payload privacy).
        sig_present: bool,
    },

    /// H2 (audio conference): query SFU producer/consumer RTP statistics (media-plane evidence + ops observability).
    /// Carries producer_id or consumer_id (either one); the server replies with SfuStats.
    SfuStatsRequest { producer_id: Option<String>, consumer_id: Option<String> },

    /// H2: response to SfuStatsRequest - RTP received bytes/packets (mediasoup get_stats).
    /// byte_count/packet_count: producer = inbound RTP received; consumer = outbound RTP routed and forwarded.
    SfuStats {
        producer_id: Option<String>,
        consumer_id: Option<String>,
        kind: Option<MediaKind>,
        byte_count: u64,
        packet_count: u64,
        score: u8,
    },
    /// H1 (session-recovery T1): producer death notice (room-level broadcast) - the server broadcasts when a peer WS closes.
    /// On receipt the web side stops the old track and waits for the new new_producer round to auto-resubscribe (self-healing without a page refresh).
    /// reason currently has a single source = peer_disconnected; mid-flight producer death detection depends on worker notifications (unreliable for H3, out of scope this round).
    ProducerClosed {
        room_id: String,
        peer_id: String,
        producer_id: String,
        kind: MediaKind,
        #[serde(skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    /// F1/T4: gateway reports a downstream sub-session dying (one streamer exiting/crashing - the whole-vehicle session does not;
    /// the reverse lookup on agent disconnect only covers whole-vehicle granularity, this message provides stream granularity). The server cleans up
    /// exactly by (room_id, peer_id) and broadcasts ProducerClosed. additive: an old server failing to parse silently drops it = same as if the message did not exist.
    DownstreamGone {
        /// Peer key self-reported by the sub-process (= the SFU-layer producer host key, literally "host" for video streams).
        peer_id: String,
        /// Stream sub-room from the sub-process RoomJoin.
        room_id: String,
    },

    /// device-enroll §2 (server -> host): signature challenge. nonce = base64 32B random (OsRng);
    /// at most once per connection, and only for RoomJoins carrying device_pubkey (never for the secret shape / PSK path).
    DeviceAuthChallenge { nonce: String },

    /// device-enroll §2 (host -> server): challenge response.
    /// Byte contract: sig = base64( Ed25519::sign( nonce_raw(32B) ‖ device_id ‖ room_id ) );
    /// pinned test in server devices.rs::sig_vector (the host side batch 3 cross-checks the same vector; D-E7 binds the room to prevent cross-use).
    DeviceAuthResponse { room_id: String, sig: String },

    /// device-enroll §2 (server -> host): manual mode, signature verified but not yet approved (moved to pending).
    /// The device keeps its existing retry backoff and reconnects while waiting for approval (log: "awaiting admin approval").
    DeviceAuthPending { device_id: String },
    // ponytail: add frame ack/retransmit when reliability matters
}

/// E3 status report: data flow statistics for a single topic (wire version, mirrors host monitor::flow::TopicFlow).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TopicFlowJson {
    pub topic: String,
    /// Frame rate within the window (<2 frames or a zero-length window -> 0).
    pub fps: f64,
    /// Byte rate within the window.
    pub bps: u64,
    /// Monotonic publisher timestamp of the most recent frame (ns; 0 = never received).
    pub last_ts_mono_ns: u64,
    /// Frames received within the window.
    pub frames: u64,
    /// Stalled (the last arrival is older than the threshold; never received also counts as stalled).
    pub stalled: bool,
}

/// E3 status report: push status for a single stream (wire version, mirrors host monitor::flow::StreamFlow).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StreamFlowJson {
    pub id: String,
    /// bytes_sent of the most recent stats (webrtc OutboundRtp, cumulative).
    pub bytes_sent: u64,
    /// frames_encoded of the most recent stats (libwebrtc u32, cumulative).
    pub frames_encoded: u32,
    pub frame_width: u32,
    pub frame_height: u32,
    /// Whether the most recent stats fall inside the freshness window.
    pub connected: bool,
}

/// E3 status report: topology status of a single process (union of expected and actual; running = oxmgr running).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProcessStateJson {
    pub name: String,
    pub running: bool,
    /// Process expected by host.toml (a discovered non-expected process = false).
    pub expected: bool,
}

/// E3 status report: signaling plane (gateway perspective).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SignalStatusJson {
    /// Whether the remote server WS is connected and has joined the room.
    pub remote_connected: bool,
    /// Seconds since the current remote session was established (None = not connected).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_since_secs: Option<u64>,
    /// Whole-vehicle peer_id (empty string = not connected).
    pub remote_peer_id: String,
    /// List of local sub-process WS connections.
    pub children: Vec<ChildSignalJson>,
    /// Seconds since host-agent startup.
    pub agent_uptime_secs: u64,
}

/// E3 status report: one sub-process WS connection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChildSignalJson {
    /// Sub-process identifier (LocalEnvelope.src).
    pub src: String,
    /// Connected (the snapshot only holds in-flight connections, always true; the field is kept for the H-phase renderer).
    pub connected: bool,
    /// Seconds since the most recent upstream message (0 = just received; u64::MAX = never sent).
    pub last_msg_secs: u64,
}

/// Direction of a WebRTC transport (send-only or recv-only).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TransportDirection {
    Send,
    Recv,
}

/// ICE parameters returned after WebRTC transport creation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IceParameters {
    pub username_fragment: String,
    pub password: String,
}

/// DTLS parameters for transport connection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DtlsParameters {
    pub fingerprints: Vec<Fingerprint>,
    /// "auto" | "client" | "server"
    pub role: String,
}

/// A DTLS fingerprint.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Fingerprint {
    /// e.g. "sha-256"
    pub algorithm: String,
    /// hex-encoded fingerprint value
    pub value: String,
}

/// An ICE candidate for WebRTC transport connection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct IceCandidate {
    /// IP address of the candidate.
    pub ip: String,
    /// Port of the candidate.
    pub port: u16,
    /// Transport protocol ("udp" or "tcp").
    pub protocol: String,
    /// Unique identifier for the candidate.
    pub foundation: String,
    /// Assigned priority of the candidate.
    pub priority: u32,
    /// Type of candidate ("host", "srflx", "prflx", "relay").
    pub candidate_type: String,
}

/// Role of a peer in a room.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PeerRole {
    Host,
    Remote,
    Consumer,
}

/// Media kind for produce/consume.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    Audio,
    Video,
    /// S4′: SCTP DataProducer (used by ProducerClosed broadcasts; never appears on the produce wire face).
    Data,
}

/// H1 (SFU data domain): SCTP stream parameters for a DataChannel (wire version,
/// mirrors mediasoup SctpStreamParameters - official docs sctp-parameters section).
/// camelCase serialization (JS/mediasoup convention: streamId/ordered/maxPacketLifeTime/maxRetransmits).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SctpStreamParameters {
    /// SCTP stream id (the negotiated id of the endpoint DataChannel).
    pub stream_id: u16,
    /// Ordered reliable transport (true = ordered; false may add maxPacketLifeTime/maxRetransmits).
    pub ordered: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_packet_life_time: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_retransmits: Option<u16>,
}

/// Control request envelope (extracted from mediaservo-host by client-dual-form T1.3 - four-way single source of truth:
/// host-controller / host-emergency / TS mirror / fixtures all share the live `{seq, cmd, payload}` shape).
/// Channel boundary = DC label (chassis/gimbal/light); the envelope does not repeat the channel name;
/// `seq` increments monotonically on the sender, and the reply `ack` echoes it back unchanged for pairing (D-H3 semantics).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlEnvelope {
    /// Monotonic sender sequence number (used to pair with the reply).
    pub seq: u64,
    /// Actuator command (e.g. "steer"/"pan"/"on"; semantics defined by the actuator implementation).
    pub cmd: String,
    /// Command arguments (absent = empty object).
    #[serde(default = "default_control_payload")]
    pub payload: serde_json::Value,
    /// S4/T3.5 (R-B HMAC scheme): estop-class commands must carry a signature -
    /// `hex(HMAC-SHA256(key, canonical_envelope_json))`; key = the deployment pre-shared
    /// (env `MEDIASERVO_CONTROL_HMAC_KEY`, configured identically on the vehicle and cockpit sides).
    /// additive: old endpoints ignore it via serde (their estop will be rejected by a new vehicle = a non-breaking version wall).
    /// Threat model note: an HMAC shared key gives no non-repudiation (interim shape);
    /// the Ed25519/PKI signature face is decided by the P3′ threat model (PLAN §2 P3′ row).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sig: Option<String>,
}

/// estop command-family check (prefix contract: `estop` / `estop_*` / `estop-*`).
#[must_use]
pub fn is_estop_cmd(cmd: &str) -> bool {
    cmd == "estop" || cmd.starts_with("estop_") || cmd.starts_with("estop-")
}

/// Canonical message for the S4/T3.5 HMAC signature = the envelope JSON without sig
/// (the same function builds it on the send and verify sides = byte-stable; sig=None serializes as if the field were absent).
fn canonical_envelope_bytes(env: &ControlEnvelope) -> Vec<u8> {
    let base = ControlEnvelope {
        seq: env.seq,
        cmd: env.cmd.clone(),
        payload: env.payload.clone(),
        sig: None,
    };
    serde_json::to_vec(&base).expect("ControlEnvelope is infallible to serialize")
}

/// `hex(HMAC-SHA256(key, canonical))` (constructor shared by vehicle and cockpit).
/// Emergency-stop HMAC key file read (G13 file channel **shared source of truth on both sides**: cockpit client-c
/// `hmac_key_file` and vehicle `MEDIASERVO_CONTROL_HMAC_KEY_FILE` follow the same discipline).
/// Permission gate 0600 (group/other read is rejected - a weak file mode is a key-leak surface), trailing newline stripped, non-empty, UTF-8.
pub fn control_hmac_key_from_file(path: &str) -> Result<String, String> {
    use std::os::unix::fs::PermissionsExt;
    let meta = std::fs::metadata(path).map_err(|e| format!("hmac key file {path}: {e}"))?;
    let mode = meta.permissions().mode() & 0o777;
    if mode & 0o077 != 0 {
        return Err(format!(
            "hmac key file {path}: mode {mode:04o} too permissive (must be 0600 or stricter, G13)"
        ));
    }
    let mut bytes = std::fs::read(path).map_err(|e| format!("hmac key file {path}: {e}"))?;
    while matches!(bytes.last(), Some(b'\n') | Some(b'\r')) {
        bytes.pop();
    }
    if bytes.is_empty() {
        return Err(format!("hmac key file {path}: empty key"));
    }
    String::from_utf8(bytes).map_err(|_| format!("hmac key file {path}: key is not UTF-8"))
}

pub fn control_hmac_sign(key: &str, env: &ControlEnvelope) -> String {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    let mut mac =
        Hmac::<Sha256>::new_from_slice(key.as_bytes()).expect("hmac accepts any key length");
    mac.update(&canonical_envelope_bytes(env));
    // Full 32B -> 64 hex (enough for remote-control semantics; friendly to the DC frame budget).
    mac.finalize().into_bytes().iter().map(|b| format!("{b:02x}")).collect()
}

/// Constant-time verification (hmac `verify_slice` = internally constant-time; hex decoding hand-written to avoid a new dependency).
#[must_use]
pub fn control_hmac_verify(key: &str, env: &ControlEnvelope, sig_hex: &str) -> bool {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    fn from_hex(h: &str) -> Option<Vec<u8>> {
        let b = h.as_bytes();
        (b.len().is_multiple_of(2))
            .then(|| {
                b.chunks(2)
                    .map(|p| u8::from_str_radix(std::str::from_utf8(p).ok()?, 16).ok())
                    .collect::<Option<Vec<u8>>>()
            })
            .flatten()
    }
    let Some(given) = from_hex(sig_hex) else {
        return false;
    };
    let mut mac =
        Hmac::<Sha256>::new_from_slice(key.as_bytes()).expect("hmac accepts any key length");
    mac.update(&canonical_envelope_bytes(env));
    mac.verify_slice(&given).is_ok()
}

/// Control reply (sent back on the same channel as the request).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlAck {
    /// Sequence of the request this reply answers.
    pub ack: u64,
    /// Actuator result; on failure `{"error": "<reason>"}`.
    pub result: serde_json::Value,
}

impl ControlAck {
    pub fn ok(seq: u64, result: serde_json::Value) -> Self {
        Self { ack: seq, result }
    }

    pub fn err(seq: u64, message: impl Into<String>) -> Self {
        Self { ack: seq, result: serde_json::json!({ "error": message.into() }) }
    }
}

/// Parse a request envelope from DC bytes.
pub fn parse_envelope(data: &[u8]) -> Result<ControlEnvelope, serde_json::Error> {
    serde_json::from_slice(data)
}

/// Absent payload = empty object (`Value::default()` is Null, which does not fit the semantics).
fn default_control_payload() -> serde_json::Value {
    serde_json::json!({})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialize_room_join() {
        let msg = SignalingMessage::RoomJoin {
            room_id: "room-1".into(),
            peer_role: PeerRole::Host,
            stream_id: None,
            device_id: None,
            device_secret: None,
            device_pubkey: None,
            protocol: None,
            client_version: None,
            resume: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"room_join""#));
        assert!(json.contains(r#""room_id":"room-1""#));
        assert!(json.contains(r#""peer_role":"host""#));
        // stream_id/device fields are absent from the JSON when None (additive contract: old servers are unaware)
        assert!(!json.contains("stream_id"));
        assert!(!json.contains("device_"));
    }

    #[test]
    fn roundtrip_room_join_with_stream_id() {
        let msg = SignalingMessage::RoomJoin {
            room_id: "room-1".into(),
            peer_role: PeerRole::Consumer,
            stream_id: Some("stream-42".into()),
            device_id: Some("ms-001122334455".into()),
            device_secret: Some("s3cr3t".into()),
            device_pubkey: None,
            protocol: None,
            client_version: None,
            resume: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"room_join""#));
        assert!(json.contains(r#""peer_role":"consumer""#));
        assert!(json.contains(r#""stream_id":"stream-42""#));
        assert!(json.contains(r#""device_id":"ms-001122334455""#));
        assert!(json.contains(r#""device_secret":"s3cr3t""#));

        let parsed: SignalingMessage = serde_json::from_str(&json).unwrap();
        match parsed {
            SignalingMessage::RoomJoin {
                room_id,
                peer_role,
                stream_id,
                device_id,
                device_secret,
                ..
            } => {
                assert_eq!(room_id, "room-1");
                assert_eq!(peer_role, PeerRole::Consumer);
                assert_eq!(stream_id.as_deref(), Some("stream-42"));
                assert_eq!(device_id.as_deref(), Some("ms-001122334455"));
                assert_eq!(device_secret.as_deref(), Some("s3cr3t"));
            }
            _ => panic!("expected RoomJoin"),
        }
    }

    #[test]
    fn roundtrip_room_join_with_device_pubkey() {
        let msg = SignalingMessage::RoomJoin {
            room_id: "room-1".into(),
            peer_role: PeerRole::Host,
            stream_id: None,
            device_id: Some("ms-0a1b2c3d4e5f".into()),
            device_secret: None,
            // A real vk derived from seed=bytes(0..32), the same value as the devices.rs sig_vector test (cross-crate anchor)
            device_pubkey: Some("A6EHv/POEL4dcN0Y50vAmWfk1jCbpQ1fHdyGZBJVMbg=".into()),
            protocol: None,
            client_version: None,
            resume: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""device_pubkey":"A6EHv/POEL4dcN0Y50vAmWfk1jCbpQ1fHdyGZBJVMbg=""#));
        // The pubkey shape and secret are mutually exclusive: secret None does not go on the wire
        assert!(!json.contains("device_secret"));

        match serde_json::from_str::<SignalingMessage>(&json).unwrap() {
            SignalingMessage::RoomJoin { device_id, device_pubkey, .. } => {
                assert_eq!(device_id.as_deref(), Some("ms-0a1b2c3d4e5f"));
                assert_eq!(
                    device_pubkey.as_deref(),
                    Some("A6EHv/POEL4dcN0Y50vAmWfk1jCbpQ1fHdyGZBJVMbg=")
                );
            }
            _ => panic!("expected RoomJoin"),
        }
    }

    #[test]
    fn room_join_without_pubkey_omits_field_on_wire() {
        // additive pin 1: None -> the JSON has no device_pubkey key (old servers are unaware)
        let msg = SignalingMessage::RoomJoin {
            room_id: "room-1".into(),
            peer_role: PeerRole::Host,
            stream_id: None,
            device_id: Some("ms-0a1b2c3d4e5f".into()),
            device_secret: Some("s3cr3t".into()),
            device_pubkey: None,
            protocol: None,
            client_version: None,
            resume: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(!json.contains("device_pubkey"));

        // additive pin 2: an old wire (JSON missing the key) still parses = a new server is compatible with an old host (upgrade server first, then host)
        let old_wire = r#"{"type":"room_join","room_id":"room-1","peer_role":"host","device_id":"ms-0a1b2c3d4e5f","device_secret":"s3cr3t"}"#;
        match serde_json::from_str::<SignalingMessage>(old_wire).unwrap() {
            SignalingMessage::RoomJoin { device_pubkey, device_secret, .. } => {
                assert_eq!(device_pubkey, None);
                assert_eq!(device_secret.as_deref(), Some("s3cr3t"));
            }
            _ => panic!("expected RoomJoin"),
        }
    }

    #[test]
    fn roundtrip_device_auth_variants() {
        let cases = [
            (
                SignalingMessage::DeviceAuthChallenge {
                    nonce: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".into(),
                },
                r#""type":"device_auth_challenge""#,
            ),
            (
                SignalingMessage::DeviceAuthResponse {
                    room_id: "vehicle_cam0".into(),
                    sig: "c2ln".into(),
                },
                r#""type":"device_auth_response""#,
            ),
            (
                SignalingMessage::DeviceAuthPending { device_id: "ms-0a1b2c3d4e5f".into() },
                r#""type":"device_auth_pending""#,
            ),
        ];
        for (msg, wire_tag) in cases {
            let json = serde_json::to_string(&msg).unwrap();
            assert!(json.contains(wire_tag), "{json}");
            let reparsed =
                serde_json::to_string(&serde_json::from_str::<SignalingMessage>(&json).unwrap())
                    .unwrap();
            assert_eq!(reparsed, json);
        }
    }

    #[test]
    fn serialize_error() {
        let msg =
            SignalingMessage::Error { code: 4003, message: "PSK authentication failed".into() };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"error""#));
        assert!(json.contains("4003"));
    }

    #[test]
    fn roundtrip_ice_candidate() {
        let msg = SignalingMessage::RTCIceCandidate {
            room_id: "r1".into(),
            target: None,
            candidate: "candidate:1 1 UDP 2130706431 10.0.0.1 8000 typ host".into(),
            sdp_mid: Some("0".into()),
            sdp_mline_index: Some(0),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: SignalingMessage = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, SignalingMessage::RTCIceCandidate { .. }));
    }

    #[test]
    fn room_join_protocol_absent_is_v1_byte_stable() {
        // S0 hard gate 1: v1 wire (no protocol/client_version) is byte-identical before and after serialization -
        // behavior equivalence for old endpoints is the entire meaning of the additive promise.
        let old = r#"{"type":"room_join","room_id":"vehicle_test","peer_role":"host"}"#;
        let parsed: SignalingMessage = serde_json::from_str(old).unwrap();
        assert_eq!(serde_json::to_string(&parsed).unwrap(), old);
        match parsed {
            SignalingMessage::RoomJoin { protocol, .. } => assert_eq!(protocol, None),
            other => panic!("expected RoomJoin, got {other:?}"),
        }
        let old_joined = r#"{"type":"room_joined","room_id":"vehicle_test","peer_id":"p1"}"#;
        let parsed: SignalingMessage = serde_json::from_str(old_joined).unwrap();
        assert_eq!(serde_json::to_string(&parsed).unwrap(), old_joined);
    }

    #[test]
    fn negotiate_protocol_matrix() {
        // S0 hard gate 2: negotiation pure-function matrix (absent=1 / same generation=2 / overclaim clamped to the server max).
        assert_eq!(negotiate_protocol(None), 1);
        assert_eq!(negotiate_protocol(Some(1)), 1);
        assert_eq!(negotiate_protocol(Some(2)), 2);
        assert_eq!(negotiate_protocol(Some(99)), SIGNALING_PROTOCOL_VERSION);
    }

    #[test]
    fn roundtrip_room_joined() {
        let msg = SignalingMessage::RoomJoined {
            room_id: "room-42".into(),
            peer_id: "peer-7".into(),
            protocol: None,
            server_version: None,
            session_nonce: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"room_joined""#));
        let parsed: SignalingMessage = serde_json::from_str(&json).unwrap();
        match parsed {
            SignalingMessage::RoomJoined { room_id, peer_id, .. } => {
                assert_eq!(room_id, "room-42");
                assert_eq!(peer_id, "peer-7");
            }
            _ => panic!("expected RoomJoined"),
        }
    }

    #[test]
    fn roundtrip_room_leave() {
        let msg =
            SignalingMessage::RoomLeave { room_id: "room-99".into(), peer_id: "peer-3".into() };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"room_leave""#));
        let parsed: SignalingMessage = serde_json::from_str(&json).unwrap();
        match parsed {
            SignalingMessage::RoomLeave { room_id, peer_id } => {
                assert_eq!(room_id, "room-99");
                assert_eq!(peer_id, "peer-3");
            }
            _ => panic!("expected RoomLeave"),
        }
    }

    #[test]
    fn roundtrip_downstream_gone() {
        // F1/T4 additive contract pinned: snake_case type tag + the two fields
        let msg = SignalingMessage::DownstreamGone {
            peer_id: "host".into(),
            room_id: "vehicle_test2".into(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"downstream_gone""#));
        let parsed: SignalingMessage = serde_json::from_str(&json).unwrap();
        match parsed {
            SignalingMessage::DownstreamGone { peer_id, room_id } => {
                assert_eq!(peer_id, "host");
                assert_eq!(room_id, "vehicle_test2");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn roundtrip_sdp() {
        let msg = SignalingMessage::Sdp {
            room_id: "room-1".into(),
            target: Some("peer-a".into()),
            sdp: "v=0\r\no=- 1 2 IN IP4 127.0.0.1\r\ns=-".into(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"sdp""#));
        let parsed: SignalingMessage = serde_json::from_str(&json).unwrap();
        match parsed {
            SignalingMessage::Sdp { room_id, target, sdp } => {
                assert_eq!(room_id, "room-1");
                assert_eq!(target.as_deref(), Some("peer-a"));
                assert!(sdp.starts_with("v=0"));
            }
            _ => panic!("expected Sdp"),
        }
    }

    #[test]
    fn roundtrip_sdp_without_target() {
        let msg =
            SignalingMessage::Sdp { room_id: "room-x".into(), target: None, sdp: "v=0".into() };
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: SignalingMessage = serde_json::from_str(&json).unwrap();
        match parsed {
            SignalingMessage::Sdp { target, .. } => {
                assert!(target.is_none());
            }
            _ => panic!("expected Sdp"),
        }
    }

    #[test]
    fn peer_role_host_serde() {
        let json = serde_json::to_string(&PeerRole::Host).unwrap();
        assert_eq!(json, r#""host""#);
        let parsed: PeerRole = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, PeerRole::Host);
    }

    #[test]
    fn peer_role_remote_serde() {
        let json = serde_json::to_string(&PeerRole::Remote).unwrap();
        assert_eq!(json, r#""remote""#);
        let parsed: PeerRole = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, PeerRole::Remote);
    }

    #[test]
    fn peer_role_consumer_serde() {
        let json = serde_json::to_string(&PeerRole::Consumer).unwrap();
        assert_eq!(json, r#""consumer""#);
        let parsed: PeerRole = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, PeerRole::Consumer);
    }

    #[test]
    fn deserialize_unknown_type() {
        let json = r#"{"type":"unknown_kind","room_id":"x"}"#;
        let result: Result<SignalingMessage, _> = serde_json::from_str(json);
        assert!(result.is_err(), "unknown type should fail deserialization");
    }

    #[test]
    fn deserialize_missing_required_field() {
        let json = r#"{"type":"error","message":"oops"}"#;
        // Error variant requires both code and message
        let result: Result<SignalingMessage, _> = serde_json::from_str(json);
        assert!(result.is_err(), "missing 'code' field should fail");
    }

    #[test]
    fn deserialize_bad_peer_role() {
        let json = r#""invalid_role""#;
        let result: Result<PeerRole, _> = serde_json::from_str(json);
        assert!(result.is_err(), "invalid role should fail deserialization");
    }

    #[test]
    fn roundtrip_create_webrtc_transport() {
        let msg = SignalingMessage::CreateWebRtcTransport {
            room_id: "room-1".into(),
            peer_id: "peer-a".into(),
            direction: TransportDirection::Send,
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"create_web_rtc_transport""#));
        let parsed: SignalingMessage = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, SignalingMessage::CreateWebRtcTransport { .. }));
    }

    #[test]
    fn roundtrip_webrtc_transport_created() {
        let msg = SignalingMessage::WebRtcTransportCreated {
            room_id: "room-1".into(),
            peer_id: "peer-a".into(),
            transport_id: "transport-1".into(),
            ice_parameters: IceParameters {
                username_fragment: "ufrag".into(),
                password: "pwd".into(),
            },
            dtls_parameters: DtlsParameters {
                fingerprints: vec![Fingerprint {
                    algorithm: "sha-256".into(),
                    value: "AA:BB:CC".into(),
                }],
                role: "auto".into(),
            },
            ice_candidates: None,
            sctp_parameters: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"web_rtc_transport_created""#));
        let parsed: SignalingMessage = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, SignalingMessage::WebRtcTransportCreated { .. }));
    }

    #[test]
    fn roundtrip_connect_webrtc_transport() {
        let msg = SignalingMessage::ConnectWebRtcTransport {
            room_id: "room-1".into(),
            peer_id: "peer-a".into(),
            transport_id: "transport-1".into(),
            dtls_parameters: DtlsParameters {
                fingerprints: vec![Fingerprint {
                    algorithm: "sha-256".into(),
                    value: "DD:EE:FF".into(),
                }],
                role: "client".into(),
            },
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"connect_web_rtc_transport""#));
        let parsed: SignalingMessage = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, SignalingMessage::ConnectWebRtcTransport { .. }));
    }

    #[test]
    fn roundtrip_media_kind() {
        assert_eq!(serde_json::to_string(&MediaKind::Audio).unwrap(), r#""audio""#);
        assert_eq!(serde_json::to_string(&MediaKind::Video).unwrap(), r#""video""#);
        let kind: MediaKind = serde_json::from_str(r#""audio""#).unwrap();
        assert_eq!(kind, MediaKind::Audio);
    }

    #[test]
    fn roundtrip_produce() {
        let msg = SignalingMessage::Produce {
            room_id: "room-1".into(),
            peer_id: "peer-1".into(),
            transport_direction: TransportDirection::Send,
            kind: MediaKind::Video,
            rtp_parameters: serde_json::json!({"codecs": [{"mimeType": "video/VP8"}]}),
            transport_id: Some("transport-9".into()),
            label: Some("fl".into()),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"produce""#));
        assert!(json.contains("transport-9"), "transport_id must go on the wire: {json}");
        let parsed: SignalingMessage = serde_json::from_str(&json).unwrap();
        match parsed {
            SignalingMessage::Produce { room_id, kind, transport_id, .. } => {
                assert_eq!(room_id, "room-1");
                assert_eq!(kind, MediaKind::Video);
                assert_eq!(transport_id.as_deref(), Some("transport-9"));
            }
            _ => panic!("expected Produce"),
        }
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"produce""#));
        let parsed: SignalingMessage = serde_json::from_str(&json).unwrap();
        match parsed {
            SignalingMessage::Produce { room_id, kind, .. } => {
                assert_eq!(room_id, "room-1");
                assert_eq!(kind, MediaKind::Video);
            }
            _ => panic!("expected Produce"),
        }
    }

    #[test]
    fn roundtrip_produced() {
        let msg =
            SignalingMessage::Produced { room_id: "room-1".into(), producer_id: "prod-1".into() };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"produced""#));
        let parsed: SignalingMessage = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, SignalingMessage::Produced { .. }));
    }

    #[test]
    fn roundtrip_new_producer() {
        let msg = SignalingMessage::NewProducer {
            room_id: "room-1".into(),
            producer_id: "prod-1".into(),
            peer_id: "peer-a".into(),
            kind: MediaKind::Audio,
            label: Some("fl".into()),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"new_producer""#));
        let parsed: SignalingMessage = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, SignalingMessage::NewProducer { .. }));
    }

    #[test]
    fn roundtrip_producer_closed() {
        let msg = SignalingMessage::ProducerClosed {
            room_id: "room-1".into(),
            peer_id: "peer-a".into(),
            producer_id: "prod-1".into(),
            kind: MediaKind::Video,
            reason: Some("peer_disconnected".into()),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"producer_closed""#));
        let parsed: SignalingMessage = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, SignalingMessage::ProducerClosed { reason: Some(_), .. }));
    }

    #[test]
    fn roundtrip_consume() {
        let msg = SignalingMessage::Consume {
            room_id: "room-1".into(),
            peer_id: "peer-1".into(),
            producer_id: "prod-1".into(),
            rtp_capabilities: serde_json::json!({"codecs": [{"mimeType": "video/VP8"}]}),
            transport_id: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"consume""#));
        let parsed: SignalingMessage = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, SignalingMessage::Consume { .. }));
    }

    /// C1: transport_id absent (legacy client wire has no such field) -> parses as None (backward compatible);
    /// None is not serialized (old servers can parse it too).
    #[test]
    fn roundtrip_consume_legacy_without_transport_id() {
        let json = serde_json::json!({
            "type": "consume",
            "room_id": "room-1",
            "peer_id": "peer-1",
            "producer_id": "prod-1",
            "rtp_capabilities": {"codecs": []},
        });
        let parsed: SignalingMessage = serde_json::from_str(&json.to_string()).unwrap();
        match parsed {
            SignalingMessage::Consume { ref transport_id, .. } => assert_eq!(transport_id, &None),
            other => panic!("expected Consume, got {other:?}"),
        }
        // Reverse: None is not serialized (a legacy server can parse it too)
        let out = serde_json::to_string(&parsed).unwrap();
        assert!(!out.contains("transport_id"), "None transport_id must not go on the wire: {out}");
    }

    #[test]
    fn roundtrip_consumed() {
        let msg = SignalingMessage::Consumed {
            room_id: "room-1".into(),
            consumer_id: "cons-1".into(),
            producer_id: "prod-1".into(),
            kind: MediaKind::Video,
            rtp_parameters: serde_json::json!({"codecs": [{"mimeType": "video/VP8"}]}),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"consumed""#));
        let parsed: SignalingMessage = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, SignalingMessage::Consumed { .. }));
    }

    #[test]
    fn roundtrip_config_push() {
        let msg = SignalingMessage::ConfigPush {
            room_id: "vehicle-1".into(),
            target: "veh-peer".into(),
            config: "[[cameras]]\nid = \"cam0\"\n".into(),
            version: 7,
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"config_push""#));
        assert!(json.contains(r#""version":7"#));
        let parsed: SignalingMessage = serde_json::from_str(&json).unwrap();
        match parsed {
            SignalingMessage::ConfigPush { room_id, target, config, version } => {
                assert_eq!(room_id, "vehicle-1");
                assert_eq!(target, "veh-peer");
                assert!(config.contains("cam0"));
                assert_eq!(version, 7);
            }
            other => panic!("expected ConfigPush, got {other:?}"),
        }
    }

    #[test]
    fn roundtrip_emergency_command() {
        let msg = SignalingMessage::EmergencyCommand {
            room_id: "vehicle-1".into(),
            command: "e-stop".into(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"emergency_command""#));
        assert!(json.contains("e-stop"));
        let parsed: SignalingMessage = serde_json::from_str(&json).unwrap();
        match parsed {
            SignalingMessage::EmergencyCommand { room_id, command } => {
                assert_eq!(room_id, "vehicle-1");
                assert_eq!(command, "e-stop");
            }
            other => panic!("expected EmergencyCommand, got {other:?}"),
        }
    }

    #[test]
    fn roundtrip_status_report() {
        let msg = SignalingMessage::StatusReport {
            room_id: "vehicle-1".into(),
            topics: vec![TopicFlowJson {
                topic: "camera/cam0".into(),
                fps: 29.7,
                bps: 800_000,
                last_ts_mono_ns: 1_234_567_890,
                frames: 148,
                stalled: false,
            }],
            streams: vec![StreamFlowJson {
                id: "cam0".into(),
                bytes_sent: 42_000_000,
                frames_encoded: 21_000,
                frame_width: 1280,
                frame_height: 720,
                connected: true,
            }],
            processes: vec![
                ProcessStateJson { name: "host-agent".into(), running: true, expected: true },
                ProcessStateJson {
                    name: "host-capturer-cam0".into(),
                    running: false,
                    expected: true,
                },
            ],
            signal: SignalStatusJson {
                remote_connected: true,
                remote_since_secs: Some(120),
                remote_peer_id: "veh-peer".into(),
                children: vec![ChildSignalJson {
                    src: "host-streamer".into(),
                    connected: true,
                    last_msg_secs: 1,
                }],
                agent_uptime_secs: 3600,
            },
            ts: 1_700_000_000,
            config_version: 0,
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"status_report""#));
        let parsed: SignalingMessage = serde_json::from_str(&json).unwrap();
        match parsed {
            SignalingMessage::StatusReport {
                room_id,
                topics,
                streams,
                processes,
                signal,
                ts,
                config_version,
            } => {
                assert_eq!(room_id, "vehicle-1");
                assert_eq!(topics[0].topic, "camera/cam0");
                assert_eq!(topics[0].frames, 148);
                assert_eq!(streams[0].frames_encoded, 21_000);
                assert!(!processes[1].running);
                assert!(processes[1].expected);
                assert_eq!(signal.remote_peer_id, "veh-peer");
                assert_eq!(signal.children[0].src, "host-streamer");
                assert_eq!(ts, 1_700_000_000);
                assert_eq!(config_version, 0);
            }
            _ => panic!("expected StatusReport"),
        }
    }

    // ── H1 SFU data domain ────────────────────────────────────────────────

    #[test]
    fn roundtrip_sctp_stream_parameters() {
        let sp = SctpStreamParameters {
            stream_id: 7,
            ordered: true,
            max_packet_life_time: None,
            max_retransmits: None,
        };
        let json = serde_json::to_string(&sp).unwrap();
        assert!(json.contains(r#"streamId":7"#), "camelCase wire: {json}");
        let parsed: SctpStreamParameters = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.stream_id, 7);
        assert!(parsed.ordered);

        let sp2 = SctpStreamParameters {
            stream_id: 3,
            ordered: false,
            max_packet_life_time: Some(100),
            max_retransmits: Some(5),
        };
        let parsed2: SctpStreamParameters =
            serde_json::from_str(&serde_json::to_string(&sp2).unwrap()).unwrap();
        assert_eq!(parsed2.max_packet_life_time, Some(100));
        assert_eq!(parsed2.max_retransmits, Some(5));
    }

    #[test]
    fn roundtrip_create_data_producer() {
        let msg = SignalingMessage::CreateDataProducer {
            room_id: "room-1".into(),
            peer_id: "peer-1".into(),
            transport_direction: TransportDirection::Send,
            label: "control".into(),
            protocol: "mediaservo.control".into(),
            sctp_stream_parameters: Some(SctpStreamParameters {
                stream_id: 1,
                ordered: true,
                max_packet_life_time: None,
                max_retransmits: None,
            }),
            transport_id: Some("transport-9".into()),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#"type":"create_data_producer"#));
        assert!(json.contains(r#"label":"control"#));
        assert!(json.contains("transport-9"), "transport_id must go on the wire: {json}");
        let parsed: SignalingMessage = serde_json::from_str(&json).unwrap();
        match parsed {
            SignalingMessage::CreateDataProducer {
                room_id,
                label,
                protocol,
                sctp_stream_parameters,
                transport_id,
                ..
            } => {
                assert_eq!(room_id, "room-1");
                assert_eq!(label, "control");
                assert_eq!(protocol, "mediaservo.control");
                assert_eq!(sctp_stream_parameters.unwrap().stream_id, 1);
                assert_eq!(transport_id.as_deref(), Some("transport-9"));
            }
            other => panic!("expected CreateDataProducer, got {other:?}"),
        }
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#"type":"create_data_producer"#));
        assert!(json.contains(r#"label":"control"#));
        let parsed: SignalingMessage = serde_json::from_str(&json).unwrap();
        match parsed {
            SignalingMessage::CreateDataProducer {
                room_id,
                label,
                protocol,
                sctp_stream_parameters,
                ..
            } => {
                assert_eq!(room_id, "room-1");
                assert_eq!(label, "control");
                assert_eq!(protocol, "mediaservo.control");
                assert_eq!(sctp_stream_parameters.unwrap().stream_id, 1);
            }
            other => panic!("expected CreateDataProducer, got {other:?}"),
        }
    }

    #[test]
    fn roundtrip_data_producer_created() {
        let msg = SignalingMessage::DataProducerCreated {
            room_id: "room-1".into(),
            data_producer_id: "dp-1".into(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#"type":"data_producer_created"#));
        let parsed: SignalingMessage = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, SignalingMessage::DataProducerCreated { .. }));
    }

    #[test]
    fn roundtrip_new_data_producer() {
        let msg = SignalingMessage::NewDataProducer {
            room_id: "room-1".into(),
            data_producer_id: "dp-1".into(),
            peer_id: "peer-a".into(),
            label: "vision".into(),
            protocol: "mediaservo.vision".into(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#"type":"new_data_producer"#));
        let parsed: SignalingMessage = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, SignalingMessage::NewDataProducer { .. }));
    }

    #[test]
    fn roundtrip_consume_data() {
        let msg = SignalingMessage::ConsumeData {
            room_id: "room-1".into(),
            peer_id: "peer-1".into(),
            transport_direction: TransportDirection::Recv,
            data_producer_id: "dp-1".into(),
            transport_id: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#"type":"consume_data"#));
        assert!(
            !json.contains("transport_id"),
            "None transport_id must not go on the wire: {json}"
        );
        let parsed: SignalingMessage = serde_json::from_str(&json).unwrap();
        match parsed {
            SignalingMessage::ConsumeData { transport_id, .. } => assert_eq!(transport_id, None),
            other => panic!("expected ConsumeData, got {other:?}"),
        }
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#"type":"consume_data"#));
        let parsed: SignalingMessage = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, SignalingMessage::ConsumeData { .. }));
    }

    #[test]
    fn s4_envelope_wire_backcompat_no_sig() {
        // An old wire shape (no sig field) must remain readable; a new sender with sig also round-trips symmetrically.
        let old_wire = r#"{"seq":7,"cmd":"steer","payload":{"deg":1.0}}"#;
        let env: ControlEnvelope = serde_json::from_str(old_wire).unwrap();
        assert_eq!(env.sig, None);
        let signed = control_hmac_sign("k1", &env);
        let env2 = ControlEnvelope {
            seq: 7,
            cmd: "steer".into(),
            payload: env.payload.clone(),
            sig: Some(signed.clone()),
        };
        assert!(control_hmac_verify("k1", &env2, &signed));
        assert!(!control_hmac_verify("wrong", &env2, &signed));
        assert!(!control_hmac_verify("k1", &env2, "deadbeef"));
    }

    #[test]
    fn control_hmac_key_from_file_gates() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("mskey-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("k");
        std::fs::write(&p, b"secret\n").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(control_hmac_key_from_file(p.to_str().unwrap()).unwrap(), "secret");
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o640)).unwrap();
        assert!(
            control_hmac_key_from_file(p.to_str().unwrap()).unwrap_err().contains("too permissive")
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn s4_estop_cmd_family() {
        assert!(is_estop_cmd("estop"));
        assert!(is_estop_cmd("estop_all"));
        assert!(is_estop_cmd("estop-steer"));
        assert!(!is_estop_cmd("steer"));
        assert!(!is_estop_cmd("restore_estop"));
    }

    #[test]
    fn roundtrip_data_consumed() {
        let msg = SignalingMessage::DataConsumed {
            room_id: "room-1".into(),
            data_consumer_id: "dc-1".into(),
            data_producer_id: "dp-1".into(),
            sctp_stream_parameters: Some(SctpStreamParameters {
                stream_id: 7,
                ordered: true,
                max_packet_life_time: None,
                max_retransmits: None,
            }),
            label: "chassis".into(),
            protocol: "mediaservo.control".into(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#"type":"data_consumed"#));
        let parsed: SignalingMessage = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, SignalingMessage::DataConsumed { .. }));
    }

    /// S4′: pin the data kind wire value of ProducerClosed (the TS media-plane filter depends on the literal "data").
    #[test]
    fn producer_closed_serializes_data_kind() {
        let msg = SignalingMessage::ProducerClosed {
            room_id: "r".into(),
            peer_id: "p".into(),
            producer_id: "d1".into(),
            kind: MediaKind::Data,
            reason: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""kind":"data""#), "wire snake_case: {json}");
        let back: SignalingMessage = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, SignalingMessage::ProducerClosed { kind: MediaKind::Data, .. }));
    }

    /// S2d compatibility pin: wire from an old server (no sctpStreamParameters/label/protocol) must still parse.
    #[test]
    fn data_consumed_parses_legacy_wire_without_sctp_fields() {
        let legacy = r#"{"type":"data_consumed","room_id":"r","data_consumer_id":"c","data_producer_id":"p"}"#;
        let parsed: SignalingMessage = serde_json::from_str(legacy).unwrap();
        assert!(matches!(
            parsed,
            SignalingMessage::DataConsumed { sctp_stream_parameters: None, label, .. } if label.is_empty()
        ));
    }

    #[test]
    fn roundtrip_sfu_stats_request_and_response() {
        let req = SignalingMessage::SfuStatsRequest {
            producer_id: Some("p-1".into()),
            consumer_id: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains(r#"type":"sfu_stats_request"#));
        let parsed: SignalingMessage = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, SignalingMessage::SfuStatsRequest { .. }));

        let resp = SignalingMessage::SfuStats {
            producer_id: Some("p-1".into()),
            consumer_id: None,
            kind: Some(MediaKind::Audio),
            byte_count: 4096,
            packet_count: 64,
            score: 10,
        };
        let json2 = serde_json::to_string(&resp).unwrap();
        assert!(json2.contains(r#"type":"sfu_stats"#));
        let parsed2: SignalingMessage = serde_json::from_str(&json2).unwrap();
        assert!(matches!(parsed2, SignalingMessage::SfuStats { .. }));
    }

    /// PIT-106 (I2 review): browsers send the W3C conventional `rtc_ice_candidate` (sfu-client.ts:236),
    /// the canonical wire name is `r_t_c_ice_candidate` (serde snake_case) -> the server aliases for compatibility.
    #[test]
    fn ice_candidate_alias_parses_browser_wire_name() {
        let json = r#"{"type":"rtc_ice_candidate","room_id":"r1","target":null,"candidate":"candidate:1 1 UDP 2130706431 10.0.0.1 8000 typ host","sdp_mid":"0","sdp_mline_index":0}"#;
        let parsed: SignalingMessage = serde_json::from_str(json).unwrap();
        match parsed {
            SignalingMessage::RTCIceCandidate { room_id, candidate, sdp_mline_index, .. } => {
                assert_eq!(room_id, "r1");
                assert_eq!(candidate, "candidate:1 1 UDP 2130706431 10.0.0.1 8000 typ host");
                assert_eq!(sdp_mline_index, Some(0));
            }
            other => panic!("expected RTCIceCandidate, got {other:?}"),
        }
    }

    // ── P1 (client-dual-form): mediasoup-client standard negotiation face ──────────────────

    #[test]
    fn roundtrip_get_router_rtp_capabilities() {
        let msg = SignalingMessage::GetRouterRtpCapabilities { room_id: "r1".into() };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"get_router_rtp_capabilities""#));
        let parsed: SignalingMessage = serde_json::from_str(&json).unwrap();
        assert!(
            matches!(parsed, SignalingMessage::GetRouterRtpCapabilities { room_id } if room_id == "r1")
        );
    }

    #[test]
    fn roundtrip_router_rtp_capabilities_opaque() {
        let msg = SignalingMessage::RouterRtpCapabilities {
            room_id: "r1".into(),
            capabilities: serde_json::json!({ "codecs": [{ "mimeType": "video/H264" }] }),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"router_rtp_capabilities""#));
        let parsed: SignalingMessage = serde_json::from_str(&json).unwrap();
        match parsed {
            SignalingMessage::RouterRtpCapabilities { capabilities, .. } => {
                assert_eq!(capabilities["codecs"][0]["mimeType"], "video/H264");
            }
            other => panic!("expected RouterRtpCapabilities, got {other:?}"),
        }
    }

    #[test]
    fn roundtrip_set_preferred_layers() {
        let msg = SignalingMessage::SetPreferredLayers {
            room_id: "r1".into(),
            peer_id: "p1".into(),
            consumer_id: "c1".into(),
            spatial_layer: 2,
            temporal_layer: Some(1),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"set_preferred_layers""#));
        let parsed: SignalingMessage = serde_json::from_str(&json).unwrap();
        assert!(matches!(
            parsed,
            SignalingMessage::SetPreferredLayers { spatial_layer: 2, temporal_layer: Some(1), .. }
        ));
        // temporal_layer None -> not on the wire (additive tolerance)
        let msg2 = SignalingMessage::SetPreferredLayers {
            room_id: "r1".into(),
            peer_id: "p1".into(),
            consumer_id: "c1".into(),
            spatial_layer: 0,
            temporal_layer: None,
        };
        assert!(!serde_json::to_string(&msg2).unwrap().contains("temporal_layer"));
    }

    #[test]
    fn transport_created_sctp_parameters_optional_on_wire() {
        let make = |sctp: Option<serde_json::Value>| SignalingMessage::WebRtcTransportCreated {
            room_id: "r1".into(),
            peer_id: "p1".into(),
            transport_id: "t1".into(),
            ice_parameters: IceParameters { username_fragment: "u".into(), password: "p".into() },
            dtls_parameters: DtlsParameters { fingerprints: vec![], role: "auto".into() },
            ice_candidates: None,
            sctp_parameters: sctp,
        };
        // None -> the field does not go on the wire; a wire missing the field -> parses as None (two-way compatibility pin)
        let json = serde_json::to_string(&make(None)).unwrap();
        assert!(!json.contains("sctp_parameters"));
        let parsed: SignalingMessage = serde_json::from_str(&json).unwrap();
        assert!(matches!(
            parsed,
            SignalingMessage::WebRtcTransportCreated { sctp_parameters: None, .. }
        ));
        // Some -> passthrough
        let json_with =
            serde_json::to_string(&make(Some(serde_json::json!({ "port": 5000 })))).unwrap();
        assert!(json_with.contains(r#""sctp_parameters":{"port":5000}"#));
    }
}
