//! mediasoup SFU foundation.
//!
//! Provides `SfuManager` (global), `SfuRoom` (per-room Router + peers),
//! and `SfuPeer` (per-peer transports + producers/consumers).
//!
//! Only compiled when the `sfu-mediasoup` feature is enabled.

// ── Feature-gated imports ───────────────────────────────────────────────

#[cfg(feature = "sfu-mediasoup")]
use dashmap::DashMap;
use mediaservo_common::protocol;

/// H2: audio room predicate -- the room_id prefix convention `audio-<vehicle-id>`.
/// Audio room = a fully-interconnected opus conference (vehicle host-audio process + cockpit viewer+ / dispatcher);
/// coexists on the same machine with video/control rooms (RoomType-independent; SFU rooms are isolated by the room_id string).
pub fn is_audio_room(room_id: &str) -> bool {
    room_id.starts_with("audio-")
}

/// PIT-143 bare-metal predicate: whether the first announced IP belongs to a local interface IP.
/// In a container, announced = a host-reachable IP (not among the container's interfaces) -> false;
/// bare metal (announced auto-detected or env/yaml being a local IP) -> true.
/// true  -> all ListenInfos bind their own concrete IP (0.0.0.0 wildcard + concrete IP on the same port = EADDRINUSE, T3 evidence);
/// false -> 0.0.0.0 wildcard + first announced (container/injection scenarios).
pub fn use_bare_metal_listen(
    announced: &[String],
    local: &std::collections::HashSet<String>,
) -> bool {
    announced.first().is_some_and(|first| local.contains(first))
}

/// Detect the container's primary IP (zero-dependency UDP connect trick;
/// connect() on UDP only sets the default route target, no packet is sent).
/// Auto-collect all local IPv4s (equivalent to ifconfig -- advertises every NIC/VPN; loopback filtered).
/// Valid on bare metal only: a container sees the container NICs (172.x unreachable -- container scenarios must set env/yaml explicitly).
/// Advertising extra candidates is harmless (ICE multi-candidate; unreachable peers are skipped automatically).
/// One source, two consumers: the sfu runtime announced fallback + the doctor announced_ips rot inspection (T6b 4,
/// same-source criterion as if_addrs -- a second interface-enumeration implementation is forbidden, PLAN S8). if-addrs is an unconditional dependency,
/// usable in the stub posture too.
pub fn detect_all_ips() -> Vec<String> {
    let mut ips: Vec<String> = Vec::new();
    if let Ok(ifaces) = if_addrs::get_if_addrs() {
        for iface in ifaces {
            if let if_addrs::IfAddr::V4(v4) = iface.addr {
                let ip = v4.ip.to_string();
                if !ip.starts_with("127.") && !ips.contains(&ip) {
                    ips.push(ip);
                }
            }
        }
    }
    if ips.is_empty() {
        // fallback: outbound probing (the former detect_local_ip)
        if let Ok(socket) = std::net::UdpSocket::bind("0.0.0.0:0")
            && socket.connect("8.8.8.8:80").is_ok()
            && let Ok(addr) = socket.local_addr()
        {
            return vec![addr.ip().to_string()];
        }
    }
    ips
}

/// PIT-183 (option-A root fix, D274 leftover round): the two Router-teardown criteria --
/// consumers dropping to zero no longer drags a Router that still has producers into teardown; the moment producers are all gone,
/// `announce_producers_closed` performs the deferred teardown of registry/owners at its tail. Pure functions shared by both postures
/// with table-driven unit tests (the decision surface in one place, guarding against branch drift -- the F1/D272 discipline).
/// When a room's WS membership drops to zero, should the SFU/registry/owners be torn down with it.
/// A Router carrying producers must survive (the black-hole root cause, PIT-183).
pub fn should_teardown(room_removed: bool, has_producers: bool) -> bool {
    room_removed && !has_producers
}

/// After the all-producers-gone announcement, should deferred cleanup run (only when the room has no WS members and truly no producers).
pub fn should_deferred_cleanup(room_gone: bool, has_producers: bool) -> bool {
    room_gone && !has_producers
}

#[cfg(test)]
mod router_guard_tests {
    use super::{should_deferred_cleanup, should_teardown};

    #[test]
    fn teardown_truth_table() {
        // Truth table: teardown only for "WS empty AND no producers" (second column = has_producers)
        assert!(!should_teardown(false, false), "room not empty: no teardown");
        assert!(!should_teardown(false, true), "room not empty: no teardown (with producers)");
        assert!(should_teardown(true, false), "empty and no producers -> teardown");
        assert!(
            !should_teardown(true, true),
            "PIT-183 main case: empty but producers alive -> keep"
        );
    }

    #[test]
    fn deferred_cleanup_truth_table() {
        assert!(
            !should_deferred_cleanup(false, false),
            "K2: consumers remain -> leave it to the leave path"
        );
        assert!(!should_deferred_cleanup(false, true), "room alive with producers -> no-op");
        assert!(
            should_deferred_cleanup(true, false),
            "K1: room dispersed and producers gone -> deferred teardown"
        );
        assert!(
            !should_deferred_cleanup(true, true),
            "room dispersed but producers remain -> do not destroy"
        );
    }
}

#[cfg(test)]
mod bare_metal_tests {
    use super::use_bare_metal_listen;
    use std::collections::HashSet;

    fn local(ips: &[&str]) -> HashSet<String> {
        ips.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn bare_metal_all_announced_local() {
        let announced = ["192.168.2.127".to_string(), "10.144.0.3".to_string()];
        assert!(use_bare_metal_listen(&announced, &local(&["192.168.2.127", "10.144.0.3"])));
    }

    #[test]
    fn container_announced_not_local() {
        // container interfaces are 172.x, not including the host IP -> not bare metal
        let announced = ["10.144.0.3".to_string()];
        assert!(!use_bare_metal_listen(&announced, &local(&["172.18.0.2"])));
    }

    #[test]
    fn mixed_first_local_rest_not() {
        // first IP local = bare-metal semantics (later non-local addresses are skipped)
        let announced = ["192.168.2.127".to_string(), "10.144.0.3".to_string()];
        assert!(use_bare_metal_listen(&announced, &local(&["192.168.2.127"])));
    }

    #[test]
    fn empty_announced_falls_back() {
        assert!(
            !use_bare_metal_listen(&[], &local(&["192.168.2.127"])),
            "empty announced falls back to 0.0.0.0"
        );
    }
}

#[cfg(feature = "sfu-mediasoup")]
mod imp {
    use super::*;
    use mediasoup::prelude::*;
    use mediasoup::webrtc_server::{WebRtcServer, WebRtcServerListenInfos, WebRtcServerOptions};
    use mediasoup::worker_manager::WorkerManager;
    use std::net::{IpAddr, Ipv4Addr};
    use std::num::{NonZeroU8, NonZeroU32};
    use std::sync::Arc;

    /// PIT-58: announced_address resolution -- env MEDIASERVO_SFU_ANNOUNCED_IP,
    /// supports comma-separated multi-IP (multi-NIC hosts). fallback: in-container probing (172.18.0.2 is only reachable locally;
    /// deployers must inject the real host IP via CLI/external means).
    /// announced resolution (three-tier priority, PIT-138 revision):
    /// env `MEDIASERVO_SFU_ANNOUNCED_IP` (comma-separated multi-IP) > server.yaml `sfu.announced_ips`
    /// > auto-detect (outbound IP -- unreliable in containers, fallback only).
    fn announced_ips(config: Option<&mediaservo_common::config::ServerConfig>) -> Vec<String> {
        if let Ok(raw) = std::env::var("MEDIASERVO_SFU_ANNOUNCED_IP") {
            let ips: Vec<String> =
                raw.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
            if !ips.is_empty() {
                return ips;
            }
        }
        if let Some(cfg) = config
            && !cfg.sfu.announced_ips.is_empty()
        {
            return cfg.sfu.announced_ips.clone();
        }
        detect_all_ips()
    }

    /// Create RouterOptions with sensible default codecs (Opus + VP8 + H264).
    fn default_router_options() -> RouterOptions {
        RouterOptions::new(vec![
            RtpCodecCapability::Audio {
                mime_type: MimeTypeAudio::Opus,
                preferred_payload_type: Some(111), // Opus explicit (prevents collision with video)
                clock_rate: NonZeroU32::new(48000).unwrap(),
                channels: NonZeroU8::new(2).unwrap(),
                parameters: RtpCodecParametersParameters::default(),
                rtcp_feedback: vec![],
            },
            RtpCodecCapability::Video {
                mime_type: MimeTypeVideo::Vp8,
                preferred_payload_type: Some(96), // VP8 explicit (prevents conflict with H264 101)
                clock_rate: NonZeroU32::new(90000).unwrap(),
                parameters: RtpCodecParametersParameters::default(),
                rtcp_feedback: vec![],
            },
            RtpCodecCapability::Video {
                mime_type: MimeTypeVideo::H264,
                preferred_payload_type: Some(101), // PIT-51: must match the payloadType 101 the Host produces with (None=auto-assign != 101 -> produce fails)
                clock_rate: NonZeroU32::new(90000).unwrap(),
                parameters: RtpCodecParametersParameters::from([
                    ("level-asymmetry-allowed", 1_u32.into()),
                    ("packetization-mode", 1_u32.into()),
                    ("profile-level-id", "42e01f".into()), // v2: 4d0032->42e01f (browser decode compat, encoder-backend-codec-config T7 evidence)
                ]),
                rtcp_feedback: vec![],
            },
            // v2 (2026-08-11): user requested 264/265/vp8/vp9/av1 -- VP9/AV1 enabled;
            // H265 unavailable: mediasoup-rs 0.24 MimeTypeVideo has no H265 binding (worker unsupported; needs a newer/custom build)
            RtpCodecCapability::Video {
                mime_type: MimeTypeVideo::Vp9,
                preferred_payload_type: Some(99), // conflict avoidance: 96/101 in use, 100 is claimed by mediasoup auto-assignment
                clock_rate: NonZeroU32::new(90000).unwrap(),
                parameters: RtpCodecParametersParameters::default(),
                rtcp_feedback: vec![],
            },
            RtpCodecCapability::Video {
                mime_type: MimeTypeVideo::AV1,
                preferred_payload_type: Some(97), // dynamic PT at the pool tail (96/99/101 used; 100-102 pool head conflicts with mediasoup allocation, verified live)
                clock_rate: NonZeroU32::new(90000).unwrap(),
                parameters: RtpCodecParametersParameters::default(),
                rtcp_feedback: vec![],
            },
        ])
    }
    /// Result of a transport creation request.
    pub struct TransportCreated {
        pub transport_id: String,
        pub ice_parameters: protocol::IceParameters,
        pub dtls_parameters: protocol::DtlsParameters,
        pub ice_candidates: Vec<protocol::IceCandidate>,
        /// P1: transport-level SCTP parameters (mediasoup SctpParameters serialized passthrough; serialization failure = None).
        pub sctp_parameters: Option<serde_json::Value>,
    }

    /// Result of a producer creation request.
    pub struct ProduceResult {
        pub producer_id: String,
        pub kind: protocol::MediaKind,
    }

    /// Result of a consumer creation request.
    pub struct ConsumeResult {
        pub consumer_id: String,
        pub producer_id: String,
        pub kind: protocol::MediaKind,
        pub rtp_parameters_json: serde_json::Value,
    }

    /// Result of a data producer creation request.
    #[derive(Debug)]
    pub struct DataProduceResult {
        pub data_producer_id: String,
    }

    /// Result of a data consumer creation request.
    #[derive(Debug)]
    pub struct DataConsumeResult {
        pub data_consumer_id: String,
        pub data_producer_id: String,
        /// S2d: worker-assigned consumer SCTP stream parameters -- per the official contract the consumer side must use these
        /// to build the negotiated DC (Chrome74.receiveDataChannel: id=streamId).
        pub sctp_stream_parameters: Option<protocol::SctpStreamParameters>,
        pub label: String,
        pub protocol: String,
    }

    /// Per-peer state: send/recv transports and active producers/consumers.
    pub struct SfuPeer {
        /// C1 (D-H14 order-independence): the send transport registry -- multiple host subprocesses share a peer_id
        /// (e.g. "host"); concurrent CreateWebRtcTransports must not overwrite each other (old single slot: the later one
        /// overwrote the earlier -> produce attached to the wrong transport -> silent stream loss). `.last()` = legacy single-slot fallback.
        pub send_transports: Vec<WebRtcTransport>,
        /// C1: the recv transport registry (same defect in multi-connection shared peer_id scenarios -- consumer
        /// attached to the wrong transport -> black screen, see the signaling.rs comment). `.last()` = legacy single-slot fallback.
        pub recv_transports: Vec<WebRtcTransport>,
        pub producers: Vec<Producer>,
        pub consumers: Vec<Consumer>,
        /// H1 (SFU data domain): SCTP DataChannel producers/consumers (the mediasoup-native data domain).
        pub data_producers: Vec<DataProducer>,
        pub data_consumers: Vec<DataConsumer>,
        /// C1: producer_id -> transport_id binding (recorded at produce; bind assertions/diagnostic accessor).
        pub producer_transports: std::collections::HashMap<String, String>,
        /// C1: data_producer_id -> transport_id binding.
        pub data_producer_transports: std::collections::HashMap<String, String>,
    }

    impl SfuPeer {
        /// Locate the send transport by transport_id; None = most recently created (legacy single-slot semantics;
        /// single-transport client behavior unchanged).
        fn send_transport(&self, transport_id: Option<&str>) -> Option<&WebRtcTransport> {
            match transport_id {
                Some(id) => self.send_transports.iter().find(|t| t.id().to_string() == id),
                None => self.send_transports.last(),
            }
        }

        /// Locate the recv transport by transport_id; None = most recently created (legacy single-slot semantics).
        fn recv_transport(&self, transport_id: Option<&str>) -> Option<&WebRtcTransport> {
            match transport_id {
                Some(id) => self.recv_transports.iter().find(|t| t.id().to_string() == id),
                None => self.recv_transports.last(),
            }
        }
    }
    /// Per-room SFU state: one Router, all connected peers.
    pub struct SfuRoom {
        pub router: Arc<Router>,
        pub peers: DashMap<String, SfuPeer>,
        /// producer-label T1.3: producer_id -> business label (Produce.label; the late-join replay source;
        /// empty map for compatibility with label-less old hosts)
        pub producer_labels: DashMap<String, String>,
    }

    /// H3: the admin panel room list summary (the audio-conference panel data source).
    #[derive(Debug, Clone, serde::Serialize)]
    #[serde(rename_all = "snake_case")]
    pub struct SfuRoomSummary {
        pub room_id: String,
        /// Number of connected participants (peers).
        pub participants: usize,
        /// Total producers in the room.
        pub producers: usize,
        /// Total consumers in the room.
        pub consumers: usize,
        /// H2 audio room (audio- prefix).
        pub audio: bool,
        /// Producer id list in the room (for SfuStats queries).
        pub producer_ids: Vec<String>,
        /// Consumer id list in the room (for SfuStats queries).
        pub consumer_ids: Vec<String>,
    }

    /// Global SFU manager — owns WorkerManager, maps room_id → SfuRoom.
    #[allow(dead_code)]
    pub struct SfuManager {
        worker_manager: WorkerManager,
        worker: Worker,
        webrtc_server: Arc<WebRtcServer>,
        rooms: DashMap<String, SfuRoom>,
    }

    /// Convert mediasoup DtlsParameters → protocol DtlsParameters via serde.
    fn convert_dtls_parameters(
        dtls: &mediasoup::prelude::DtlsParameters,
    ) -> protocol::DtlsParameters {
        // DtlsParameters derives Serialize; DtlsFingerprint has a custom Serialize
        // that produces {"algorithm": "sha-256", "value": "AA:BB:..."}.
        // Serialize to JSON, then deserialize into our protocol types.
        // ponytail: serde round-trip for type conversion; hand-write converters if perf matters.
        let json = serde_json::to_value(dtls).unwrap_or_default();
        protocol::DtlsParameters {
            fingerprints: json
                .get("fingerprints")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .map(|f| protocol::Fingerprint {
                            algorithm: f["algorithm"].as_str().unwrap_or("unknown").to_string(),
                            value: f["value"].as_str().unwrap_or("").to_string(),
                        })
                        .collect()
                })
                .unwrap_or_default(),
            role: json["role"].as_str().unwrap_or("auto").to_string(),
        }
    }

    fn convert_ice_parameters(ice: &IceParameters) -> protocol::IceParameters {
        protocol::IceParameters {
            username_fragment: ice.username_fragment.clone(),
            password: ice.password.clone(),
        }
    }

    fn convert_ice_candidates(candidates: &[IceCandidate]) -> Vec<protocol::IceCandidate> {
        candidates
            .iter()
            .map(|c| protocol::IceCandidate {
                ip: c.address.clone(),
                port: c.port,
                protocol: format!("{:?}", c.protocol).to_lowercase(),
                foundation: c.foundation.clone(),
                priority: c.priority,
                candidate_type: format!("{:?}", c.r#type).to_lowercase(),
            })
            .collect()
    }

    /// SFU listen port resolution (the D-B 10-12 ruling): yaml `sfu.port` > env `MEDIASERVO_SFU_PORT` >
    /// `DEFAULT_SFU_PORT`. Same semantics as lifecycle/templates.rs `resolve_port` -- that function lives in the
    /// main.rs binary-side module, unreachable from the lib crate; this is the function family's second instance (priority-order changes must sync both).
    fn resolve_sfu_port(yaml: Option<u16>, env: Option<u16>) -> u16 {
        yaml.or(env).unwrap_or(mediaservo_common::config::DEFAULT_SFU_PORT)
    }

    /// list_producers returns a tuple shape (a 4th element added in the producer-label round; the 1.97 toolchain type_complexity cleanup).
    type ProducerListing = Vec<(String, protocol::MediaKind, String, Option<String>)>;

    impl SfuManager {
        /// Create a new SfuManager with a single mediasoup Worker and WebRtcServer.
        /// SFU listen port = resolve_sfu_port(cfg.sfu.port, env) (D-B: yaml > env > 20000) --
        /// tests use `new_with_port` with a random port.
        pub async fn new(
            config: Option<&mediaservo_common::config::ServerConfig>,
        ) -> Result<Self, String> {
            let sfu_port = resolve_sfu_port(
                config.and_then(|c| c.sfu.port),
                std::env::var("MEDIASERVO_SFU_PORT").ok().and_then(|p| p.parse().ok()),
            );
            Self::new_with_port_config(sfu_port, config).await
        }

        /// Specify the WebRtcServer listen port (test isolation: parallel tests each bind a distinct port).
        pub async fn new_with_port(sfu_port: u16) -> Result<Self, String> {
            Self::new_with_port_config(sfu_port, None).await
        }

        /// Internal implementation: port + config (the announced three-tier resolution entry).
        async fn new_with_port_config(
            sfu_port: u16,
            config: Option<&mediaservo_common::config::ServerConfig>,
        ) -> Result<Self, String> {
            let worker_manager = WorkerManager::new();
            let worker = worker_manager
                .create_worker(WorkerSettings::default())
                .await
                .map_err(|e| format!("Failed to create mediasoup worker: {e}"))?;
            tracing::info!("mediasoup Worker created (id: {:?})", worker.id());

            // Create WebRtcServer with single port (port 20000)
            // PIT-44: listen 0.0.0.0 must set announced_address (a mediasoup official requirement);
            // otherwise candidate=0.0.0.0 is unusable by peers over ICE; in-container probing detects the container IP.
            // PIT-58: in-container probing = 172.18.0.2 (an internal address unreachable by other hosts -> Signal Lost);
            // a host-reachable IP is required -- configure via env MEDIASERVO_SFU_ANNOUNCED_IP (the host NIC IP).
            // Multi-announced-IP (PIT-143 revision): only **bare metal** supports multiple ListenInfos -- each
            // ListenInfo **listens on its own concrete IP** (re-binding 0.0.0.0 on the same port = conflict);
            // in a container (the announced host IP not among the container's interfaces) -> a single ListenInfo
            // (0.0.0.0 + first announced -- containers cannot advertise multiple addresses).
            let announced_ips = announced_ips(config);
            let local_ips: std::collections::HashSet<String> = if_addrs::get_if_addrs()
                .map(|ifaces| {
                    ifaces
                        .iter()
                        .filter_map(|i| match &i.addr {
                            if_addrs::IfAddr::V4(v4) => Some(v4.ip.to_string()),
                            _ => None,
                        })
                        .collect()
                })
                .unwrap_or_default();
            // First ListenInfo selection (PIT-143 revision + T3 S3.2 evidence):
            // bare metal (first announced in local interfaces) -> bind the concrete IP: the 0.0.0.0 wildcard grabs the port first,
            // then binding the concrete IP on the same port = Linux EADDRINUSE (uv_udp_bind ... address already in use);
            // container/injection (the announced host IP is not a local interface) -> 0.0.0.0 + first announced (mediasoup's requirement).
            let first_listen_ip = if use_bare_metal_listen(&announced_ips, &local_ips) {
                match announced_ips[0].parse::<IpAddr>() {
                    Ok(ip) => ip,
                    Err(_) => {
                        tracing::warn!(
                            "bare-metal announced IP invalid ({}), falling back to the 0.0.0.0 wildcard",
                            announced_ips[0]
                        );
                        IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0))
                    }
                }
            } else {
                IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0))
            };
            let make_info = |listen_ip: IpAddr, announced: Option<String>| ListenInfo {
                protocol: Protocol::Udp,
                ip: listen_ip,
                announced_address: announced,
                expose_internal_ip: false,
                port: Some(sfu_port), // Fixed ICE port
                port_range: None,
                flags: None,
                send_buffer_size: None,
                recv_buffer_size: None,
            };
            let mut listen_infos = WebRtcServerListenInfos::new(make_info(
                first_listen_ip,
                announced_ips.first().cloned(),
            ));
            // Remaining announced IPs: local interfaces -> one ListenInfo each (listening on the concrete IP);
            // non-local (host IPs injected for containers etc.) -> skipped.
            for ip in announced_ips.iter().skip(1) {
                if !local_ips.contains(ip) {
                    continue; // a host IP injected for a container -- no such interface inside the container, skip
                }
                let Ok(listen_ip) = ip.parse::<IpAddr>() else { continue };
                listen_infos = listen_infos.insert(make_info(listen_ip, Some(ip.clone())));
            }
            tracing::info!(
                "WebRtcServer created on port {sfu_port} (announced: {announced_ips:?})"
            );
            let webrtc_server = worker
                .create_webrtc_server(WebRtcServerOptions::new(listen_infos))
                .await
                .map_err(|e| format!("Failed to create WebRtcServer: {e}"))?;

            Ok(Self {
                worker_manager,
                worker,
                webrtc_server: Arc::new(webrtc_server),
                rooms: DashMap::new(),
            })
        }

        /// P1: lazy per-room Router creation (shared by create_webrtc_transport / router_rtp_capabilities --
        /// the mediasoup-client negotiation flow wants caps before transport; both must take the same path).
        async fn ensure_router(&self, room_id: &str) -> Result<Arc<Router>, String> {
            if let Some(room) = self.rooms.get(room_id) {
                return Ok(Arc::clone(&room.router));
            }
            // No room yet — create one
            let router =
                self.worker.create_router(default_router_options()).await.map_err(|e| {
                    // v2 diagnostics: print the codec list to locate PT conflicts
                    tracing::error!(
                        "Router create failed; media_codecs={:?}",
                        default_router_options()
                    );
                    format!("Failed to create router: {e}")
                })?;
            let router = Arc::new(router);
            tracing::info!("Router created for room {}", room_id);

            self.rooms.insert(
                room_id.to_string(),
                SfuRoom {
                    router: Arc::clone(&router),
                    peers: DashMap::new(),
                    producer_labels: DashMap::new(),
                },
            );
            Ok(router)
        }

        /// Create a WebRTC transport for a peer in a room.
        pub async fn create_webrtc_transport(
            &self,
            room_id: &str,
            peer_id: &str,
            direction: &str,
        ) -> Result<TransportCreated, String> {
            // Get or create room (P1: extracted ensure_router -- the caps request lazily creates on the same path)
            let router = self.ensure_router(room_id).await?;

            // Create transport using shared WebRtcServer (single port)
            // H1 (SFU data domain): enable_sctp = true -- required for SCTP/DataChannel negotiation (mediasoup
            // official: WebRtcTransportOptions.enableSctp defaults to false). The SCTP association is only established when the
            // peer's SDP contains m=application -- pure media streams unaffected (additive).
            let mut options =
                WebRtcTransportOptions::new_with_server(self.webrtc_server.as_ref().clone());
            options.enable_sctp = true;
            let transport = router
                .create_webrtc_transport(options)
                .await
                .map_err(|e| format!("Failed to create transport: {e}"))?;

            let transport_id = transport.id().to_string();
            let ice = transport.ice_parameters().clone();
            let dtls = transport.dtls_parameters();
            let ice_candidates = convert_ice_candidates(transport.ice_candidates());
            // P1: transport-level SCTP parameter passthrough (enable_sctp always on -> normally Some; failure = None + warn, C15).
            let sctp_parameters = transport.sctp_parameters().and_then(|p| {
                serde_json::to_value(p)
                    .map_err(|e| tracing::warn!("sctp_parameters serialization failed: {e}"))
                    .ok()
            });

            // Store transport on peer
            if let Some(room) = self.rooms.get_mut(room_id) {
                let mut peer = room.peers.entry(peer_id.to_string()).or_insert_with(|| SfuPeer {
                    send_transports: Vec::new(),
                    recv_transports: Vec::new(),
                    producers: Vec::new(),
                    consumers: Vec::new(),
                    data_producers: Vec::new(),
                    data_consumers: Vec::new(),
                    producer_transports: std::collections::HashMap::new(),
                    data_producer_transports: std::collections::HashMap::new(),
                });

                match direction {
                    "send" => {
                        // C1: append to the registry (no overwrite -- concurrent subprocesses keep their own transports)
                        peer.send_transports.push(transport);
                    }
                    "recv" => {
                        peer.recv_transports.push(transport);
                    }
                    _ => return Err(format!("Invalid direction: {direction}")),
                }
            }

            Ok(TransportCreated {
                transport_id,
                ice_parameters: convert_ice_parameters(&ice),
                dtls_parameters: convert_dtls_parameters(&dtls),
                ice_candidates,
                sctp_parameters,
            })
        }

        /// P1 (client-dual-form): the room Router's RTP capabilities -- the input to mediasoup-client
        /// Device.load() (C18). If the room does not exist the Router is lazily created (negotiation before transport).
        pub async fn router_rtp_capabilities(
            &self,
            room_id: &str,
        ) -> Result<serde_json::Value, String> {
            let router = self.ensure_router(room_id).await?;
            let caps = router.rtp_capabilities();
            serde_json::to_value(&caps)
                .map_err(|e| format!("rtp_capabilities serialization failed: {e}"))
        }

        /// P1: consumer layer preference (simulcast/SVC layer selection). Unknown room/peer/consumer -> Err
        /// (C15: the caller warns + sends Error back). The Consumer handle is cloned and the DashMap guard released before awaiting.
        pub async fn set_preferred_layers(
            &self,
            room_id: &str,
            peer_id: &str,
            consumer_id: &str,
            spatial_layer: u8,
            temporal_layer: Option<u8>,
        ) -> Result<(), String> {
            let consumer = {
                let room =
                    self.rooms.get(room_id).ok_or_else(|| format!("Room {room_id} not found"))?;
                let peer = room
                    .peers
                    .get(peer_id)
                    .ok_or_else(|| format!("Peer {peer_id} not found in room {room_id}"))?;
                peer.consumers
                    .iter()
                    .find(|c| c.id().to_string() == consumer_id)
                    .cloned()
                    .ok_or_else(|| format!("Consumer {consumer_id} not found for peer {peer_id}"))?
            };
            consumer
                .set_preferred_layers(ConsumerLayers { spatial_layer, temporal_layer })
                .await
                .map_err(|e| format!("set_preferred_layers failed: {e}"))
        }

        /// Remove a peer from a room, cleaning up transports, producers, and consumers.
        /// Returns true if the peer was found and removed.
        /// If the room becomes empty after removal, the Router is destroyed.
        pub fn remove_peer(&self, room_id: &str, peer_id: &str) -> bool {
            if let Some(room) = self.rooms.get_mut(room_id) {
                let removed = room.peers.remove(peer_id).is_some();
                if removed {
                    tracing::info!("Peer {} removed from SFU room {}", peer_id, room_id);
                    if room.peers.is_empty() {
                        drop(room);
                        self.remove_room(room_id);
                    }
                }
                removed
            } else {
                false
            }
        }

        /// H1 fix: clear across rooms by peer_id (producers register per-stream under their stream rooms' sfu peers,
        /// while the signaling session's relay_room = the vehicle room -- the old single-room remove_peer could not reach stream-room
        /// producers, causing dead-producer leaks after reconnect with no ProducerClosed broadcast).
        /// Returns (room_id, [(producer_id, kind)]) for per-room broadcasts. An emptied room destroys its Router.
        pub fn remove_peer_global(
            &self,
            peer_id: &str,
        ) -> Vec<(String, Vec<(String, protocol::MediaKind)>)> {
            let mut out = Vec::new();
            let mut empty_rooms = Vec::new();
            for mut entry in self.rooms.iter_mut() {
                let rid = entry.key().clone();
                if let Some((_pid, peer)) = entry.value_mut().peers.remove(peer_id) {
                    let closed: Vec<(String, protocol::MediaKind)> = peer
                        .producers
                        .iter()
                        .map(|p| {
                            let kind = match p.kind() {
                                MediaKind::Audio => protocol::MediaKind::Audio,
                                MediaKind::Video => protocol::MediaKind::Video,
                            };
                            (p.id().to_string(), kind)
                        })
                        .collect();
                    tracing::info!(
                        "Peer {} removed from SFU room {} ({} producers)",
                        peer_id,
                        rid,
                        closed.len()
                    );
                    if entry.value().peers.is_empty() {
                        empty_rooms.push(rid.clone());
                    }
                    out.push((rid, closed));
                }
            }
            // (iter_mut guards released when the for loop ends -- the deletion below is safe)
            for rid in empty_rooms {
                self.remove_room(&rid);
            }
            out
        }

        /// F1/T4: precise single-room removal (video producer host keys are the self-reported peer_id, usually the same literal
        /// across streams -- a global key remove would wrongly kill other live streams; must be (room, peer) granularity).
        /// Returns the closed producers for the ProducerClosed broadcast; an emptied room destroys its Router.
        pub fn remove_peer_in_room(
            &self,
            room_id: &str,
            peer_id: &str,
        ) -> Vec<(String, protocol::MediaKind)> {
            let Some(room) = self.rooms.get_mut(room_id) else {
                return Vec::new();
            };
            let Some((_pid, peer)) = room.peers.remove(peer_id) else {
                return Vec::new();
            };
            let closed: Vec<(String, protocol::MediaKind)> = peer
                .producers
                .iter()
                .map(|p| {
                    let kind = match p.kind() {
                        MediaKind::Audio => protocol::MediaKind::Audio,
                        MediaKind::Video => protocol::MediaKind::Video,
                    };
                    (p.id().to_string(), kind)
                })
                .collect();
            if room.peers.is_empty() {
                drop(room);
                self.remove_room(room_id);
            }
            closed
        }

        /// H1 fix: clear across rooms by producer_id list (each stream room's sfu peer key is the self-reported
        /// peer_id; session-id cleanup missed them -- the leak). Returns (room, producer_id, kind) for broadcasting.
        pub fn remove_producers_by_ids(
            &self,
            producer_ids: &[String],
        ) -> Vec<(String, String, protocol::MediaKind)> {
            let mut out = Vec::new();
            // First pass: close producers by id (media + data symmetric). facade Drop = close
            // (mediasoup-rs ownership semantics); the manifest is returned for the ProducerClosed broadcast.
            let mut gone_dps: std::collections::HashSet<String> = std::collections::HashSet::new();
            for mut entry in self.rooms.iter_mut() {
                let rid = entry.key().clone();
                for mut peer_ref in entry.value_mut().peers.iter_mut() {
                    let peer = peer_ref.value_mut();
                    let mut hits: Vec<(String, protocol::MediaKind)> = Vec::new();
                    peer.producers.retain(|p| {
                        let pid = p.id().to_string();
                        if producer_ids.contains(&pid) {
                            let kind = match p.kind() {
                                MediaKind::Audio => protocol::MediaKind::Audio,
                                MediaKind::Video => protocol::MediaKind::Video,
                            };
                            hits.push((pid, kind));
                            false
                        } else {
                            true
                        }
                    });
                    // S4': DataProducers under the device name get the same cleanup (the old version only collected media = dp escaped).
                    peer.data_producers.retain(|p| {
                        let pid = p.id().to_string();
                        if producer_ids.contains(&pid) {
                            gone_dps.insert(pid.clone());
                            hits.push((pid, protocol::MediaKind::Data));
                            false
                        } else {
                            true
                        }
                    });
                    for (pid, kind) in &hits {
                        peer.producer_transports.remove(pid);
                        peer.data_producer_transports.remove(pid);
                        tracing::info!(
                            "Producer {pid} ({kind:?}) removed from SFU room {rid} (owned cleanup)"
                        );
                    }
                    out.extend(hits.into_iter().map(|(pid, kind)| (rid.clone(), pid, kind)));
                }
            }
            // Second pass (the rooms borrow is over): drag along the data consumers bound to dead dps --
            // otherwise the peer consumer occupies a slot and the worker's used_sctp_stream_ids never frees
            // = the root cause of the `StreamId reserved` consumption ceiling (confirmed live 2026-09-15).
            if !gone_dps.is_empty() {
                for mut entry in self.rooms.iter_mut() {
                    let room_label = entry.key().clone();
                    for mut peer_ref in entry.value_mut().peers.iter_mut() {
                        let peer = peer_ref.value_mut();
                        let before = peer.data_consumers.len();
                        peer.data_consumers
                            .retain(|c| !gone_dps.contains(&c.data_producer_id().to_string()));
                        let reaped = before - peer.data_consumers.len();
                        if reaped > 0 {
                            tracing::info!(
                                "S4′: closed {reaped} orphan data consumers in room {room_label}"
                            );
                        }
                    }
                }
            }
            out
        }

        /// Remove a room and its Router (stops forwarding for all peers).
        pub fn remove_room(&self, room_id: &str) -> bool {
            let existed = self.rooms.remove(room_id).is_some();
            if existed {
                tracing::info!("SFU room {} destroyed", room_id);
            }
            existed
        }
        /// Create a producer for a peer on its send transport.
        /// C1: bind the transport named by transport_id; None = legacy single-slot fallback (most recently created).
        pub async fn create_producer(
            &self,
            room_id: &str,
            peer_id: &str,
            kind: &protocol::MediaKind,
            rtp_parameters_json: serde_json::Value,
            transport_id: Option<&str>,
            label: Option<String>,
        ) -> Result<ProduceResult, String> {
            // Convert JSON RTP parameters to mediasoup type
            let rtp_parameters: RtpParameters = serde_json::from_value(rtp_parameters_json)
                .map_err(|e| format!("Invalid RTP parameters: {e}"))?;

            let ms_kind = match kind {
                protocol::MediaKind::Audio => MediaKind::Audio,
                protocol::MediaKind::Video => MediaKind::Video,
                // S4' broadcast kind value domain; never appears on the produce wire (rejected by the media plane).
                protocol::MediaKind::Data => {
                    return Err("MediaKind::Data is broadcast-only, not producible".to_string());
                }
            };

            let room = self
                .rooms
                .get_mut(room_id)
                .ok_or_else(|| format!("Room {} not found for produce", room_id))?;
            let mut peer = room
                .peers
                .get_mut(peer_id)
                .ok_or_else(|| format!("Peer {} not found in room {}", peer_id, room_id))?;

            let transport = peer.send_transport(transport_id).ok_or_else(|| {
                format!("No send transport for peer {} (transport_id={transport_id:?})", peer_id)
            })?;

            // ponytail: construct ProducerOptions; let compiler validate the exact constructor
            let producer_options = ProducerOptions::new(ms_kind, rtp_parameters);
            let producer = transport
                .produce(producer_options)
                .await
                .map_err(|e| format!("Failed to create producer: {e}"))?;

            let producer_id = producer.id().to_string();
            tracing::info!(
                "Producer {} ({:?}) created for peer {} in room {}",
                producer_id,
                kind,
                peer_id,
                room_id
            );

            // C1: record the producer->transport binding (bind assertions/diagnostics) -- take the id first to end the transport borrow.
            let bound_transport_id = transport.id().to_string();
            peer.producers.push(producer);
            peer.producer_transports.insert(producer_id.clone(), bound_transport_id);
            // producer-label T1.3: record the business label (the late-join replay source; None not recorded).
            // Deadlock lesson (PIT): must reuse the already-held room guard (inside the get_mut write lock) -- re-entering
            // self.rooms.get() on the same shard's read lock = same-task self-deadlock (DashMap is not re-entrant).
            if let Some(l) = label {
                room.producer_labels.insert(producer_id.clone(), l);
            }
            Ok(ProduceResult { producer_id, kind: kind.clone() })
        }

        /// Create a consumer for a peer on its recv transport,
        /// subscribing to an existing producer in the room.
        /// C1: bind the recv transport named by transport_id; None = legacy single-slot fallback.
        pub async fn create_consumer(
            &self,
            room_id: &str,
            peer_id: &str,
            producer_id: &str,
            rtp_capabilities_json: serde_json::Value,
            transport_id: Option<&str>,
        ) -> Result<ConsumeResult, String> {
            // Convert JSON RTP capabilities to mediasoup type
            let rtp_capabilities: RtpCapabilities =
                serde_json::from_value(rtp_capabilities_json)
                    .map_err(|e| format!("Invalid RTP capabilities: {e}"))?;

            // Find the producer and extract its id + kind
            // ponytail: read-lock first to get producer info, then write-lock for consumer insert
            let (producer_id_ms, producer_kind) = {
                let room = self
                    .rooms
                    .get(room_id)
                    .ok_or_else(|| format!("Room {} not found for consume", room_id))?;
                room.peers
                    .iter()
                    .find_map(|entry| {
                        entry
                            .producers
                            .iter()
                            .find(|p| p.id().to_string() == producer_id)
                            .map(|p| (p.id(), p.kind()))
                    })
                    .ok_or_else(|| {
                        format!("Producer {} not found in room {}", producer_id, room_id)
                    })?
            };

            // Now get the consumer peer's recv transport
            let room =
                self.rooms.get_mut(room_id).ok_or_else(|| format!("Room {} not found", room_id))?;
            let mut peer = room
                .peers
                .get_mut(peer_id)
                .ok_or_else(|| format!("Peer {} not found in room {}", peer_id, room_id))?;
            let transport = peer.recv_transport(transport_id).ok_or_else(|| {
                format!("No recv transport for peer {} (transport_id={transport_id:?})", peer_id)
            })?;

            let consumer_options = ConsumerOptions::new(producer_id_ms, rtp_capabilities);
            let consumer = transport
                .consume(consumer_options)
                .await
                .map_err(|e| format!("Failed to create consumer: {e}"))?;

            let consumer_id = consumer.id().to_string();
            let protocol_kind = match producer_kind {
                MediaKind::Audio => protocol::MediaKind::Audio,
                MediaKind::Video => protocol::MediaKind::Video,
            };
            let rtp_parameters_json =
                serde_json::to_value(consumer.rtp_parameters()).unwrap_or_default();

            tracing::info!(
                "Consumer {} created for peer {} (producer: {}, kind: {:?})",
                consumer_id,
                peer_id,
                producer_id,
                protocol_kind
            );

            // PIT-65 observation: Consumer RTP trace (per-packet handling/drop reasons)
            {
                let cid = consumer_id.clone();
                let _ = consumer.on_trace(
                    move |trace: &mediasoup::consumer::ConsumerTraceEventData| {
                        tracing::info!("CONS-TRACE {}: {:?}", cid, trace);
                    },
                );
                let _ = consumer
                    .enable_trace_event(vec![mediasoup::consumer::ConsumerTraceEventType::Rtp])
                    .await;
            }

            // PIT-65 observation: Consumer dump (rtp_streams score -- the IsActive criterion)
            {
                let cid = consumer_id.clone();
                if let Ok(dump) = consumer.dump().await {
                    let scores: Vec<u8> = dump.rtp_streams.iter().map(|s| s.score).collect();
                    tracing::info!("CONS-DUMP {}: paused={} scores={:?}", cid, dump.paused, scores);
                }
            }

            // PIT-76: request a keyframe right after consume -- bypasses the libwebrtc 99s GOP (the x-google-
            // max-keyframe-interval injection has no effect on the software VP8 encoder; still 99s in practice).
            // mediasoup request_key_frame -> the producer side sends a keyframe request -> the encoder emits an IDR immediately
            if protocol_kind == protocol::MediaKind::Video {
                match consumer.request_key_frame().await {
                    Ok(()) => tracing::info!(
                        "Consumer {consumer_id}: requested key frame (instant first-frame)"
                    ),
                    Err(e) => {
                        tracing::warn!("Consumer {consumer_id}: request_key_frame failed: {e}")
                    }
                }
            }

            peer.consumers.push(consumer);

            Ok(ConsumeResult {
                consumer_id,
                producer_id: producer_id.to_string(),
                kind: protocol_kind,
                rtp_parameters_json,
            })
        }

        /// Create a data producer (SCTP DataChannel) for a peer on its send transport.
        /// H1 (SFU data domain): the mediasoup-native data domain -- DataProducerOptions::new_sctp
        /// (official DataProducerOptions.sctpStreamParameters + label + protocol).
        /// Message flow happens inside the mediasoup worker (endpoint SCTP -> worker -> DataConsumers);
        /// the server only wires produce/consume (official docs transport.produceData).
        pub async fn create_data_producer(
            &self,
            room_id: &str,
            peer_id: &str,
            label: &str,
            protocol: &str,
            sctp_stream_parameters: Option<protocol::SctpStreamParameters>,
            transport_id: Option<&str>,
        ) -> Result<DataProduceResult, String> {
            let sctp_params = sctp_stream_parameters.ok_or_else(|| {
                "sctp_stream_parameters required for SCTP data producer".to_string()
            })?;
            let ms_sctp = SctpStreamParameters {
                stream_id: sctp_params.stream_id,
                ordered: sctp_params.ordered,
                max_packet_life_time: sctp_params.max_packet_life_time,
                max_retransmits: sctp_params.max_retransmits,
            };

            let room = self
                .rooms
                .get_mut(room_id)
                .ok_or_else(|| format!("Room {} not found for produce_data", room_id))?;
            let mut peer = room
                .peers
                .get_mut(peer_id)
                .ok_or_else(|| format!("Peer {} not found in room {}", peer_id, room_id))?;
            let transport = peer.send_transport(transport_id).ok_or_else(|| {
                format!("No send transport for peer {} (transport_id={transport_id:?})", peer_id)
            })?;

            let mut options = DataProducerOptions::new_sctp(ms_sctp);
            options.label = label.to_string();
            options.protocol = protocol.to_string();
            let producer = transport
                .produce_data(options)
                .await
                .map_err(|e| format!("Failed to create data producer: {e}"))?;

            let data_producer_id = producer.id().to_string();
            tracing::info!(
                "DataProducer {} (label={}) created for peer {} in room {}",
                data_producer_id,
                label,
                peer_id,
                room_id
            );
            // C1: record the data producer->transport binding (bind assertions/diagnostics) -- take the id first to end the transport borrow.
            let bound_transport_id = transport.id().to_string();
            peer.data_producers.push(producer);
            peer.data_producer_transports.insert(data_producer_id.clone(), bound_transport_id);
            Ok(DataProduceResult { data_producer_id })
        }

        /// Create a data consumer (SCTP DataChannel) for a peer on its recv transport,
        /// subscribing to an existing data producer in the room.
        /// DataConsumerOptions::new_sctp inherits the producer's ordered/reliability parameters (official API).
        pub async fn create_data_consumer(
            &self,
            room_id: &str,
            peer_id: &str,
            data_producer_id: &str,
            transport_id: Option<&str>,
        ) -> Result<DataConsumeResult, String> {
            // Find the data producer in the room (read-lock first, then write-lock insert)
            // ponytail: read-lock first to get producer info, then write-lock for consumer insert
            let producer_id_ms = {
                let room = self
                    .rooms
                    .get(room_id)
                    .ok_or_else(|| format!("Room {} not found for consume_data", room_id))?;
                room.peers
                    .iter()
                    .find_map(|entry| {
                        entry
                            .data_producers
                            .iter()
                            .find(|p| p.id().to_string() == data_producer_id)
                            .map(|p| p.id())
                    })
                    .ok_or_else(|| {
                        format!("DataProducer {} not found in room {}", data_producer_id, room_id)
                    })?
            };

            let room =
                self.rooms.get_mut(room_id).ok_or_else(|| format!("Room {} not found", room_id))?;
            let mut peer = room
                .peers
                .get_mut(peer_id)
                .ok_or_else(|| format!("Peer {} not found in room {}", peer_id, room_id))?;
            let transport = peer.recv_transport(transport_id).ok_or_else(|| {
                format!("No recv transport for peer {} (transport_id={transport_id:?})", peer_id)
            })?;

            let consumer_options = DataConsumerOptions::new_sctp(producer_id_ms);
            let consumer = transport
                .consume_data(consumer_options)
                .await
                .map_err(|e| format!("Failed to create data consumer: {e}"))?;

            let data_consumer_id = consumer.id().to_string();
            tracing::info!(
                "DataConsumer {} created for peer {} (data_producer: {}, sctp={:?}) in room {}",
                data_consumer_id,
                peer_id,
                data_producer_id,
                consumer.sctp_stream_parameters(),
                room_id
            );

            // H1 observation: the consuming-side on_message -- the SCTP DataConsumer message event (received over the
            // endpoint's SCTP association; Direct-type messages fire on the Rust side; an official dataConsumer event).
            let cid = data_consumer_id.clone();
            let _ = consumer.on_message(move |msg: &WebRtcMessage<'_>| {
                tracing::debug!("DATA-CONS {}: {:?}", cid, msg);
            });

            // S2d: take the worker's actual parameters before push (a mediasoup->wire isomorphic mapping, mirroring the produce side).
            let (ms_sctp, c_label, c_protocol) = (
                consumer.sctp_stream_parameters(),
                consumer.label().clone(),
                consumer.protocol().clone(),
            );
            peer.data_consumers.push(consumer);

            Ok(DataConsumeResult {
                data_consumer_id,
                data_producer_id: data_producer_id.to_string(),
                sctp_stream_parameters: ms_sctp.map(|p| protocol::SctpStreamParameters {
                    stream_id: p.stream_id,
                    ordered: p.ordered,
                    max_packet_life_time: p.max_packet_life_time,
                    max_retransmits: p.max_retransmits,
                }),
                label: c_label,
                protocol: c_protocol,
            })
        }

        /// List all data producers in a room. Returns (data_producer_id, label, peer_id) tuples.
        /// Used for late-joiner sync (mirror list_producers).
        pub fn list_data_producers(&self, room_id: &str) -> Option<Vec<(String, String, String)>> {
            let room = self.rooms.get(room_id)?;
            let mut result = Vec::new();
            for entry in room.peers.iter() {
                let peer_id = entry.key().clone();
                for dp in &entry.data_producers {
                    result.push((dp.id().to_string(), dp.label().clone(), peer_id.clone()));
                }
            }
            Some(result)
        }

        /// Connect a WebRTC transport with DTLS parameters from the client.
        pub async fn connect_transport(
            &self,
            room_id: &str,
            peer_id: &str,
            transport_id: &str,
            dtls_parameters: protocol::DtlsParameters,
        ) -> Result<(), String> {
            // Convert protocol::DtlsParameters → mediasoup DtlsParameters via serde round-trip
            // ponytail: serde round-trip for type conversion; hand-write converters if perf matters.
            let ms_dtls: mediasoup::prelude::DtlsParameters = {
                let json = serde_json::to_value(&dtls_parameters)
                    .map_err(|e| format!("serialize DtlsParameters: {e}"))?;
                serde_json::from_value(json)
                    .map_err(|e| format!("deserialize DtlsParameters: {e}"))?
            };

            let room = self
                .rooms
                .get_mut(room_id)
                .ok_or_else(|| format!("Room {room_id} not found for connect"))?;
            let peer = room
                .peers
                .get_mut(peer_id)
                .ok_or_else(|| format!("Peer {peer_id} not found in room {room_id}"))?;

            // C1: look up by id in the send/recv registries (old single slot: later overwrote earlier -> the earlier connect failed)
            let transport = peer
                .send_transports
                .iter()
                .chain(peer.recv_transports.iter())
                .find(|t| t.id().to_string() == transport_id)
                .ok_or_else(|| format!("Transport {transport_id} not found for peer {peer_id}"))?;

            transport
                .connect(mediasoup::prelude::WebRtcTransportRemoteParameters {
                    dtls_parameters: ms_dtls,
                })
                .await
                .map_err(|e| format!("Failed to connect transport: {e}"))?;

            // PIT observation: dump the transport's selected tuple (the client address the server learned -- the RTP send target)
            if let Ok(dump) = transport.dump().await {
                tracing::info!(
                    "SFU: transport {transport_id} dump: ice={:?} dtls={:?} selected_tuple={:?}",
                    dump.ice_state,
                    dump.dtls_state,
                    dump.ice_selected_tuple
                );
            }

            tracing::info!(
                "SFU: transport {transport_id} connected for peer {peer_id} in room {room_id}"
            );
            Ok(())
        }

        /// C1: the producer -> transport binding accessor (bind assertions/diagnostics; recorded at produce).
        pub fn producer_transport_id(&self, producer_id: &str) -> Option<String> {
            self.rooms.iter().find_map(|room| {
                room.peers
                    .iter()
                    .find_map(|peer| peer.producer_transports.get(producer_id).cloned())
            })
        }

        /// C1: the data producer -> transport binding accessor (bind assertions/diagnostics).
        pub fn data_producer_transport_id(&self, data_producer_id: &str) -> Option<String> {
            self.rooms.iter().find_map(|room| {
                room.peers
                    .iter()
                    .find_map(|peer| peer.data_producer_transports.get(data_producer_id).cloned())
            })
        }

        /// List all producers in a room. Returns (producer_id, kind, peer_id) tuples.
        /// Used for late-joiner sync to send existing producers to new consumers.
        pub fn list_producers(&self, room_id: &str) -> Option<ProducerListing> {
            let room = self.rooms.get(room_id)?;
            let mut result = Vec::new();
            for entry in room.peers.iter() {
                let peer_id = entry.key().clone();
                for producer in &entry.producers {
                    let kind = match producer.kind() {
                        MediaKind::Audio => protocol::MediaKind::Audio,
                        MediaKind::Video => protocol::MediaKind::Video,
                    };
                    result.push((producer.id().to_string(), kind, peer_id.clone()));
                }
            }
            // producer-label T1.3: collect first, then look up labels (avoids a double-get borrow conflict on DashMap)
            let labeled: Vec<_> = result
                .into_iter()
                .map(|(pid, kind, peer)| {
                    let label = room.producer_labels.get(pid.as_str()).map(|l| l.value().clone());
                    (pid, kind, peer, label)
                })
                .collect();
            Some(labeled)
        }

        /// PIT-183: whether the room still has live producers (guards against dragged-along teardown).
        /// Same traversal shape as `list_producers` (DashMap iter -> SfuPeer.producers).
        pub fn room_has_producers(&self, room_id: &str) -> bool {
            self.rooms.get(room_id).is_some_and(|r| r.peers.iter().any(|p| !p.producers.is_empty()))
        }

        /// Send raw RTP data through the first video producer in the room.
        /// Used for WS→SFU frame relay (avoids Host-side ICE/DTLS).
        ///
        /// Note: Requires a DirectProducer. Regular producers (WebRtcTransport-based)
        /// receive RTP from the client-side peer connection, not server-side injection.
        pub fn send_frame(&self, _room_id: &str, _rtp_data: &[u8]) -> Result<(), String> {
            Err("send_frame requires DirectProducer; WebRtcTransport producers receive RTP from client-side ICE/DTLS".into())
        }

        /// Number of active rooms.
        /// Worker subprocess liveness. The mediasoup-rs reaper flips closed on **any exit** (crash or normal)
        /// (worker.rs:445 swap(true)) -> `!closed()` is a safe synchronous liveness probe
        /// (frontend-process-split T7, feeds /ready).
        pub fn worker_alive(&self) -> bool {
            !self.worker.closed()
        }

        pub fn room_count(&self) -> usize {
            self.rooms.len()
        }

        /// H3: all-room summaries (the admin panel room list) -- including producer/consumer ids for SfuStats queries.
        pub fn list_rooms(&self) -> Vec<SfuRoomSummary> {
            self.rooms
                .iter()
                .map(|room| {
                    let mut producer_ids = Vec::new();
                    let mut consumer_ids = Vec::new();
                    let mut producers = 0;
                    let mut consumers = 0;
                    for peer in room.peers.iter() {
                        producers += peer.producers.len();
                        consumers += peer.consumers.len();
                        producer_ids.extend(peer.producers.iter().map(|p| p.id().to_string()));
                        consumer_ids.extend(peer.consumers.iter().map(|c| c.id().to_string()));
                    }
                    SfuRoomSummary {
                        room_id: room.key().clone(),
                        participants: room.peers.len(),
                        producers,
                        consumers,
                        audio: super::is_audio_room(room.key()),
                        producer_ids,
                        consumer_ids,
                    }
                })
                .collect()
        }

        /// H2: query a producer's inbound RTP stats (get_stats) -- media-plane evidence (audio room e2e).
        /// Returns (kind, byte_count, packet_count, score).
        pub async fn producer_stats(
            &self,
            producer_id: &str,
        ) -> Result<(protocol::MediaKind, u64, u64, u8), String> {
            let producer = self
                .rooms
                .iter()
                .find_map(|room| {
                    room.peers.iter().find_map(|peer| {
                        peer.producers.iter().find(|p| p.id().to_string() == producer_id).cloned()
                    })
                })
                .ok_or_else(|| format!("Producer {producer_id} not found"))?;
            let kind = match producer.kind() {
                MediaKind::Audio => protocol::MediaKind::Audio,
                MediaKind::Video => protocol::MediaKind::Video,
            };
            let stats = producer.get_stats().await.map_err(|e| format!("Producer stats: {e}"))?;
            // with no RTP arriving, mediasoup does not create an RtpStream -> empty vec (legal zero-value semantics).
            let (bytes, packets, score) = match stats.first() {
                Some(s) => (s.byte_count, s.packet_count, s.score),
                None => (0, 0, 0),
            };
            Ok((kind, bytes, packets, score))
        }

        /// H2: query a consumer's outbound RTP stats (get_stats) -- forwarding evidence (audio room e2e).
        /// Returns (kind, byte_count, packet_count, score).
        pub async fn consumer_stats(
            &self,
            consumer_id: &str,
        ) -> Result<(protocol::MediaKind, u64, u64, u8), String> {
            let consumer = self
                .rooms
                .iter()
                .find_map(|room| {
                    room.peers.iter().find_map(|peer| {
                        peer.consumers.iter().find(|c| c.id().to_string() == consumer_id).cloned()
                    })
                })
                .ok_or_else(|| format!("Consumer {consumer_id} not found"))?;
            let kind = match consumer.kind() {
                MediaKind::Audio => protocol::MediaKind::Audio,
                MediaKind::Video => protocol::MediaKind::Video,
            };
            let stats = consumer.get_stats().await.map_err(|e| format!("Consumer stats: {e}"))?;
            let s = stats.consumer_stats();
            Ok((kind, s.byte_count, s.packet_count, s.score))
        }

        /// weaknet-harness T1 (cut A): a transport-level observation list for all streams (filterable by room).
        /// producer = upstream (publisher transport), consumer = downstream (consumer transport).
        /// **remote_port is a stream-distinguishable key** -- this project's WebRtcServer is fixed to a single port (all local_port values equal).
        /// transport stats failure/not found -> the field is null, the whole row is not dropped (C15 warn trail).
        /// Cut C (weaknet T5/D276): `owner_of` = the room -> registered device id reverse lookup (signaling.room_owners
        /// injected by the caller; this module holds no handle -- ownerless room owner=null (the wire key is always present, see stream_stat_row)).
        /// Generic-closure shape (not &dyn Fn): preserves the Send derivation of the handler future (a hard axum 0.7 Handler requirement).
        pub async fn list_stream_stats(
            &self,
            room: Option<&str>,
            owner_of: impl Fn(&str) -> Option<String>,
        ) -> Vec<serde_json::Value> {
            let mut rows = Vec::new();
            for room_entry in self.rooms.iter() {
                let (room_id, sfu_room) = room_entry.pair();
                if room.is_some_and(|r| r != room_id.as_str()) {
                    continue;
                }
                let owner = owner_of(room_id.as_str());
                for peer_entry in sfu_room.peers.iter() {
                    let (peer_id, peer) = peer_entry.pair();
                    for producer in peer.producers.iter() {
                        // Fast path: an already-closed handle skips directly (the T2 main predicate lives in transport_stat_fields
                        // stats reachability -- in the disconnect-leak scenario closed() never turns true, see PIT-185).
                        if producer.transport().closed() {
                            continue;
                        }
                        let pid = producer.id().to_string();
                        let tid = producer.transport().id().to_string();
                        let Some(tfields) = Self::transport_stat_fields(peer, &tid).await else {
                            continue; // dead transport (a disconnect-leak row) -- see PIT-185; root cause is a separate case
                        };
                        let (bytes, packets, score) = match producer.get_stats().await {
                            Ok(s) => s
                                .first()
                                .map(|v| (v.byte_count, v.packet_count, v.score))
                                .unwrap_or((0, 0, 0)),
                            Err(e) => {
                                tracing::warn!("sfu list_stream_stats producer {pid} stats: {e}");
                                (0, 0, 0)
                            }
                        };
                        let kind = match producer.kind() {
                            MediaKind::Audio => "audio",
                            MediaKind::Video => "video",
                        };
                        rows.push(Self::stream_stat_row(
                            room_id,
                            peer_id,
                            "producer",
                            &pid,
                            kind,
                            bytes,
                            packets,
                            score,
                            owner.as_deref(),
                            &tfields,
                        ));
                    }
                    for consumer in peer.consumers.iter() {
                        if consumer.transport().closed() {
                            continue;
                        }
                        let cid = consumer.id().to_string();
                        let tid = consumer.transport().id().to_string();
                        let Some(tfields) = Self::transport_stat_fields(peer, &tid).await else {
                            continue;
                        };
                        let (bytes, packets, score) = match consumer.get_stats().await {
                            Ok(s) => {
                                let v = s.consumer_stats();
                                (v.byte_count, v.packet_count, v.score)
                            }
                            Err(e) => {
                                tracing::warn!("sfu list_stream_stats consumer {cid} stats: {e}");
                                (0, 0, 0)
                            }
                        };
                        let kind = match consumer.kind() {
                            MediaKind::Audio => "audio",
                            MediaKind::Video => "video",
                        };
                        rows.push(Self::stream_stat_row(
                            room_id,
                            peer_id,
                            "consumer",
                            &cid,
                            kind,
                            bytes,
                            packets,
                            score,
                            owner.as_deref(),
                            &tfields,
                        ));
                    }
                }
            }
            rows
        }

        /// transport stats (the WebRtcTransportStat single struct with tuple/fractionLost/BWE) -> the observation field set.
        /// **None = a non-live row** (not in the registry / stats RPC failed / empty vec) -- list mode skips rows on this,
        /// guaranteeing "in the table = alive". The T2 live-verified predicate revision: closed() is untrustworthy in the
        /// consumer-key-drift leak scenario (the handle was never closed); stats reachability is the liveness ground truth.
        async fn transport_stat_fields(
            peer: &SfuPeer,
            transport_id: &str,
        ) -> Option<serde_json::Map<String, serde_json::Value>> {
            let t = peer
                .send_transports
                .iter()
                .chain(peer.recv_transports.iter())
                .find(|t| t.id().to_string() == transport_id)?;
            match t.get_stats().await {
                Ok(stats) => match stats.into_iter().next() {
                    Some(s) => {
                        let tp = s.ice_selected_tuple.as_ref();
                        // Liveness predicate (the T1 "in the table = alive" contract, verified over 35min in the G/E rounds): an unselected tuple
                        // = not connected or aged out after death (~10s) -> not counted as live. Note the transport
                        // object itself never self-closes (UDP ICE never judges death, IceState comment), stats reachability
                        // is not liveness either (G2 +150s fake-live), and closed() is equally untrustworthy -- the tuple is the only
                        // read-side signal strictly equivalent to "can be targeted by weaknet". PIT-185 root cause is a separate case.
                        tp?;
                        Some(Self::transport_stat_fields_json(
                            transport_id,
                            tp.map(|v| v.local_port()),
                            tp.and_then(|v| v.remote_ip()).map(|ip| ip.to_string()),
                            tp.and_then(|v| v.remote_port()),
                            (s.rtp_packet_loss_sent, s.rtp_packet_loss_received),
                            (
                                s.available_outgoing_bitrate.map(u64::from),
                                s.available_incoming_bitrate.map(u64::from),
                            ),
                        ))
                    }
                    None => None,
                },
                Err(e) => {
                    tracing::debug!(
                        "sfu transport {transport_id} stats unavailable (dead/race): {e}"
                    );
                    None
                }
            }
        }

        /// Pure function (pinned by wire-shape unit tests): transport observation field construction.
        /// loss = **the fractionLost 0-1 ratio (f64, Momus M1: integer truncation forbidden -- 8% truncating to 0 would fake a pass)**.
        fn transport_stat_fields_json(
            transport_id: &str,
            local_port: Option<u16>,
            remote_ip: Option<String>,
            remote_port: Option<u16>,
            loss: (Option<f64>, Option<f64>),
            bwe: (Option<u64>, Option<u64>),
        ) -> serde_json::Map<String, serde_json::Value> {
            serde_json::json!({
                "transport_id": transport_id,
                "local_port": local_port,
                "remote_ip": remote_ip,
                "remote_port": remote_port,
                "rtp_packet_loss_sent": loss.0,
                "rtp_packet_loss_received": loss.1,
                "available_outgoing_bitrate": bwe.0,
                "available_incoming_bitrate": bwe.1,
            })
            .as_object()
            .cloned()
            .unwrap_or_default()
        }

        /// Pure function (pinned by wire-shape unit tests): producer/consumer base fields union transport observation fields.
        /// `owner` = the room's registered device id (cut C): unknown/ownerless = **null with the key always present** -- the same wire
        /// contract as this module's transport observation fields ("wire key must always exist / unmeasured = null",
        /// pinned by transport_fields_absent_values_stay_present_as_null); a single consumer-side criterion, no forking.
        #[allow(clippy::too_many_arguments)]
        fn stream_stat_row(
            room: &str,
            peer_id: &str,
            role: &str,
            id: &str,
            kind: &str,
            byte_count: u64,
            packet_count: u64,
            score: u8,
            owner: Option<&str>,
            tfields: &serde_json::Map<String, serde_json::Value>,
        ) -> serde_json::Value {
            let mut row = serde_json::json!({
                "room": room,
                "owner": owner,
                "peer_id": peer_id,
                "role": role,
                "id": id,
                "kind": kind,
                "byte_count": byte_count,
                "packet_count": packet_count,
                "score": score,
            })
            .as_object()
            .cloned()
            .unwrap_or_default();
            row.extend(tfields.clone());
            serde_json::Value::Object(row)
        }
    }

    #[cfg(all(test, feature = "sfu-mediasoup"))]
    mod weaknet_stat_tests {
        use super::*;

        #[test]
        fn stream_stat_row_wire_shape_complete() {
            let fields = SfuManager::transport_stat_fields_json(
                "t-1",
                Some(20000),
                Some("127.0.0.1".into()),
                Some(54321),
                (Some(0.08), None),
                (Some(1_500_000), None),
            );
            let row = SfuManager::stream_stat_row(
                "vehicle_test1",
                "peer-a",
                "producer",
                "p-1",
                "video",
                1000,
                10,
                10,
                Some("dev-a"),
                &fields,
            );
            assert_eq!(row["room"], "vehicle_test1");
            assert_eq!(row["role"], "producer");
            assert_eq!(row["owner"], "dev-a");
            assert_eq!(row["local_port"], 20000);
            assert_eq!(row["remote_port"], 54321);
            // fractionLost stays a floating ratio (0.08 must not be integer-truncated); the missing side is null.
            assert_eq!(row["rtp_packet_loss_sent"].as_f64(), Some(0.08));
            assert!(row["rtp_packet_loss_received"].is_null());
        }

        /// Cut C pin test: ownerless room owner = null with the key always present (the same wire shape as owned rooms;
        /// consumers judge with the single `row["owner"].as_str()` check).
        #[test]
        fn stream_stat_row_owner_key_always_present_null_when_unowned() {
            let fields = SfuManager::transport_stat_fields_json(
                "t-3",
                Some(20000),
                Some("127.0.0.1".into()),
                Some(40001),
                (None, None),
                (None, None),
            );
            let row = SfuManager::stream_stat_row(
                "unowned-room",
                "peer-b",
                "consumer",
                "c-1",
                "audio",
                7,
                1,
                10,
                None,
                &fields,
            );
            assert!(
                row.get("owner").is_some(),
                "the owner key must always be present (null form, not omitted)"
            );
            assert!(row["owner"].is_null(), "ownerless room owner=null");
            let owned = SfuManager::stream_stat_row(
                "vehicle_test1",
                "peer-b",
                "consumer",
                "c-1",
                "audio",
                7,
                1,
                10,
                Some("dev-a"),
                &fields,
            );
            assert_eq!(owned["owner"], "dev-a");
        }

        #[test]
        fn transport_fields_absent_values_stay_present_as_null() {
            let fields = SfuManager::transport_stat_fields_json(
                "t-2",
                None,
                None,
                None,
                (None, None),
                (None, None),
            );
            for k in [
                "transport_id",
                "local_port",
                "remote_ip",
                "remote_port",
                "rtp_packet_loss_sent",
                "rtp_packet_loss_received",
                "available_outgoing_bitrate",
                "available_incoming_bitrate",
            ] {
                assert!(fields.contains_key(k), "wire key {k} must always exist");
                if k != "transport_id" {
                    assert!(fields[k].is_null(), "unmeasured {k} must be null, not absent");
                }
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// T3 mine-removal three-state pin: the SFU listen port resolution order = yaml sfu.port > env MEDIASERVO_SFU_PORT > 20000 (D-B,
        /// the 10-12 ruling reversing the old env-only behavior; the same family shape as lifecycle/templates.rs resolve_port).
        #[test]
        fn sfu_port_resolution_yaml_env_default_order() {
            use mediaservo_common::config::DEFAULT_SFU_PORT;
            // (a) yaml set + env unset -> yaml (the old code's 20000 here = hard evidence of a fake key)
            assert_eq!(resolve_sfu_port(Some(20005), None), 20005);
            // (b) yaml set + env set -> yaml wins (the D-B order)
            assert_eq!(resolve_sfu_port(Some(20005), Some(20006)), 20005);
            // (c) both unset -> the default constant, single-sourced
            assert_eq!(resolve_sfu_port(None, None), DEFAULT_SFU_PORT);
            assert_eq!(DEFAULT_SFU_PORT, 20000);
            // Extra shape: yaml unset + env set -> the env fallback survives
            assert_eq!(resolve_sfu_port(None, Some(20006)), 20006);
        }

        #[test]
        fn is_audio_room_recognizes_prefix() {
            assert!(is_audio_room("audio-ms-car1"), "the audio- prefix = audio room");
            assert!(!is_audio_room("ms-car1"), "an ordinary vehicle room is not an audio room");
            assert!(!is_audio_room("room-1"), "a prefix-less room is not an audio room");
        }

        /// PIT-58: announced_address must prefer the env variable (a host-reachable IP) --
        /// in-container probing (172.18.0.2) is only locally reachable; other hosts' ICE cannot reach it -> Signal Lost.
        #[test]
        fn detect_all_ips_collects_interfaces_no_loopback() {
            let ips = detect_all_ips();
            assert!(!ips.is_empty(), "bare metal has at least one non-loopback IPv4");
            assert!(
                ips.iter().all(|ip| !ip.starts_with("127.")),
                "must not contain loopback: {ips:?}"
            );
            // dedupe
            let mut sorted = ips.clone();
            sorted.sort();
            sorted.dedup();
            assert_eq!(ips.len(), sorted.len(), "the IP list is deduped");
        }

        fn announced_ips_prefers_env_and_falls_back() {
            // env priority (the host IP scenario)
            // SAFETY: set/restored serially inside the test; no concurrent readers
            unsafe {
                std::env::set_var("MEDIASERVO_SFU_ANNOUNCED_IP", "192.168.2.127");
            }
            assert_eq!(announced_ips(None), vec!["192.168.2.127".to_string()]);

            // multi-IP comma-separated
            // SAFETY: same as above
            unsafe {
                std::env::set_var("MEDIASERVO_SFU_ANNOUNCED_IP", "192.168.2.127,10.0.0.5");
            }
            assert_eq!(
                announced_ips(None),
                vec!["192.168.2.127".to_string(), "10.0.0.5".to_string()]
            );

            // comma + whitespace tolerance
            // SAFETY: same as above
            unsafe {
                std::env::set_var("MEDIASERVO_SFU_ANNOUNCED_IP", " 192.168.2.127 , 10.0.0.5 ");
            }
            assert_eq!(announced_ips(None).len(), 2);

            // fallback probing (the unconfigured scenario)
            // SAFETY: same as above
            unsafe {
                std::env::remove_var("MEDIASERVO_SFU_ANNOUNCED_IP");
            }
            let fallback = announced_ips(None);

            // server.yaml sfu.announced_ips (in effect when env is unset)
            // SAFETY: same as above
            unsafe {
                std::env::remove_var("MEDIASERVO_SFU_ANNOUNCED_IP");
            }
            let cfg: mediaservo_common::config::ServerConfig = serde_yaml::from_str(
            "version: 1\nlisten:\n  host: 0.0.0.0\n  port: 9800\nsfu:\n  announced_ips:\n    - 10.144.0.3\n",
        )
        .unwrap();
            assert_eq!(announced_ips(Some(&cfg)), vec!["10.144.0.3".to_string()]);
            assert!(!fallback.is_empty(), "the fallback probe should return a non-empty IP list");

            // restore the environment to avoid polluting other tests
            // SAFETY: same as above
            unsafe {
                std::env::remove_var("MEDIASERVO_SFU_ANNOUNCED_IP");
            }
        }

        /// H1: WebRtcTransport must be created with SCTP (enable_sctp) -- the precondition of DataChannel negotiation.
        /// Asserts the transport dump's sctp_parameters is non-empty (mediasoup official: enableSctp defaults to false;
        /// without it the dump has no sctp section).
        #[tokio::test]
        async fn transport_sctp_enabled() {
            let sfu = SfuManager::new_with_port(random_udp_port()).await.expect("sfu");
            sfu.create_webrtc_transport("room-sctp", "peer-a", "send").await.expect("transport");
            let room = sfu.rooms.get("room-sctp").expect("room");
            let peer = room.peers.get("peer-a").expect("peer");
            let dump =
                peer.send_transports.last().expect("send transport").dump().await.expect("dump");
            assert!(
                dump.sctp_parameters.is_some(),
                "WebRtcTransport must enable SCTP (enable_sctp): sctp_parameters={:?}",
                dump.sctp_parameters
            );
        }

        /// H1: data-domain entity creation + worker-side delivery metric (the upper bound of proof of message receipt) --
        /// DataProducer/DataConsumer wiring succeeds + after DataProducer.send() the consumer stats
        /// messages_sent>0 (the worker routes internally to the DataConsumer, official Router::OnTransportDataProducerMessageReceived).
        /// Note: the worker->app notification channels (DataConsumer.on_message / on_data_producer_close) are entirely dead in
        /// this deployment (an isomorphic reproduction of the official mediasoup-rs test fails the same way -- see data_message_roundtrip_direct
        ///'s #[ignore] doc); for proof of message receipt see that test.
        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn data_domain_entity_creation() {
            use mediasoup::prelude::*;
            use mediasoup::worker_manager::WorkerManager;

            let worker_manager = WorkerManager::new();
            let worker =
                worker_manager.create_worker(WorkerSettings::default()).await.expect("worker");
            let router = worker.create_router(default_router_options()).await.expect("router");
            let t1 = router
                .create_direct_transport(DirectTransportOptions::default())
                .await
                .expect("direct transport 1");
            let t2 = router
                .create_direct_transport(DirectTransportOptions::default())
                .await
                .expect("direct transport 2");

            // producer (Direct) + consumer (Direct) wiring
            let producer =
                t1.produce_data(DataProducerOptions::new_direct()).await.expect("produce_data");
            let consumer = t2
                .consume_data(DataConsumerOptions::new_direct(producer.id(), None))
                .await
                .expect("consume_data");
            assert!(matches!(producer, DataProducer::Direct(_)));
            assert!(matches!(consumer, DataConsumer::Direct(_)));
            assert!(!consumer.closed());

            // DirectDataProducer.send() -> worker routing (proven by consumer stats)
            let data_producer = match producer {
                DataProducer::Direct(d) => d,
                _ => unreachable!(),
            };
            data_producer
                .send(
                    WebRtcMessage::String(std::borrow::Cow::Borrowed(b"sfu-data-echo")),
                    None,
                    None,
                )
                .expect("send");

            let stats = consumer
                .get_stats()
                .await
                .expect("consumer stats")
                .into_iter()
                .next()
                .expect("one stat");
            assert_eq!(
                stats.messages_sent, 1,
                "the worker must have routed the message to the DataConsumer (messages_sent=1)"
            );
            assert_eq!(stats.bytes_sent, 13);
        }

        /// S4': `remove_producers_by_ids` must reap DataProducers (kind=Data reported) and
        /// drag along the other-peer data consumers bound to dead dps -- the root-fix pin of the
        /// monotonic `found N data producers` leak + the `StreamId reserved` consumption ceiling (confirmed live 2026-09-15).
        #[tokio::test]
        async fn remove_producers_by_ids_reaps_data_plane() {
            let sfu = SfuManager::new_with_port(random_udp_port()).await.expect("sfu");
            // cockpit outbound (the producer side's send transport must connect before produce -- reusing
            // the produce_data_on_unconnected_transport_graceful path that only creates without asserting connect;
            // create_data_producer still registers successfully on an unconnected transport = the existing tolerant behavior).
            sfu.create_webrtc_transport("room-reap", "cockpit", "send").await.expect("t-send");
            let dp = sfu
                .create_data_producer(
                    "room-reap",
                    "cockpit",
                    "chassis",
                    "sctp",
                    Some(protocol::SctpStreamParameters {
                        stream_id: 0,
                        ordered: true,
                        max_packet_life_time: None,
                        max_retransmits: None,
                    }),
                    None,
                )
                .await
                .expect("data producer")
                .data_producer_id;
            // vehicle-side consumer
            sfu.create_webrtc_transport("room-reap", "host", "recv").await.expect("t-recv");
            let dc = sfu
                .create_data_consumer("room-reap", "host", &dp, None)
                .await
                .expect("data consumer")
                .data_consumer_id;

            let closed = sfu.remove_producers_by_ids(&[dp.clone()]);
            assert!(
                closed.iter().any(|(_, id, kind)| id == &dp && *kind == protocol::MediaKind::Data),
                "the dp must be reaped by id and kind=Data (the old version only iterated media = the cockpit-leak root cause)"
            );
            let room = sfu.rooms.get("room-reap").expect("room");
            assert!(
                room.peers.get("cockpit").expect("cp").data_producers.is_empty(),
                "the dp ledger must be emptied"
            );
            let host = room.peers.get("host").expect("hp");
            assert!(
                host.data_consumers.iter().all(|c| c.id().to_string() != dc),
                "orphan consumers bound to dead dps must be reaped (the precondition for freeing used sids)"
            );
        }

        /// H1: DataProducer.send() -> DataConsumer.on_message() end-to-end proof of message receipt.
        /// #[ignore]: blocked by a mediasoup-rs 0.24.1 deployment-level bug -- the worker->app notification channels are dead
        /// (on_message / on_data_producer_close / worker_close all silently lost; an isomorphic reproduction of the official mediasoup-rs
        /// data_consumer::tests::data_producer_close_event fails in this deployment too).
        /// Proven: request/response work (dump/get_stats), worker-side routing works (messages_sent=1);
        /// the loss point = mediasoup-rs channel notification dispatch (buffer/subscription lifecycle, suspected buffer-guard race).
        /// Ownership: mediasoup-rs upstream (the H1 report PIT); the host-side SFU-DC wiring waits for the fix to verify.
        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        #[ignore = "mediasoup-rs notification channel bug in this deployment (PIT H1)"]
        async fn data_message_roundtrip_direct() {
            use mediasoup::prelude::*;
            use mediasoup::worker_manager::WorkerManager;
            use std::time::Duration;

            let worker_manager = WorkerManager::new();
            let worker =
                worker_manager.create_worker(WorkerSettings::default()).await.expect("worker");
            let router = worker.create_router(default_router_options()).await.expect("router");
            let t1 = router
                .create_direct_transport(DirectTransportOptions::default())
                .await
                .expect("direct transport 1");
            let t2 = router
                .create_direct_transport(DirectTransportOptions::default())
                .await
                .expect("direct transport 2");

            let producer =
                t1.produce_data(DataProducerOptions::new_direct()).await.expect("produce_data");
            let consumer = t2
                .consume_data(DataConsumerOptions::new_direct(producer.id(), None))
                .await
                .expect("consume_data");

            // on_message: worker -> Rust callback (proof of message delivery)
            let (tx, rx) = std::sync::mpsc::sync_channel::<Vec<u8>>(4);
            consumer.on_message(move |msg: &WebRtcMessage<'_>| {
                if let WebRtcMessage::String(payload) = msg {
                    let _ = tx.send(payload.to_vec());
                }
            });

            let data_producer = match producer {
                DataProducer::Direct(d) => d,
                _ => unreachable!(),
            };
            data_producer
                .send(
                    WebRtcMessage::String(std::borrow::Cow::Borrowed(b"sfu-data-echo")),
                    None,
                    None,
                )
                .expect("send");

            // worker-side delivery proof (holds even when on_message is dead)
            let stats = consumer
                .get_stats()
                .await
                .expect("consumer stats")
                .into_iter()
                .next()
                .expect("one stat");
            assert_eq!(
                stats.messages_sent, 1,
                "the worker must have routed the message to the DataConsumer"
            );

            let got = std::thread::spawn(move || {
                rx.recv_timeout(Duration::from_secs(5))
                    .expect("the on_message callback must receive the message (the mediasoup worker->app notification channel)")
            })
            .join()
            .expect("waiter thread");
            assert_eq!(got, b"sfu-data-echo".to_vec());
        }

        /// H1: SfuManager-level data producer creation -- the result of produce_data on a WebRtcTransport
        /// whose DTLS is not yet complete (worker behavior probe: mediasoup's SCTP association is created only after
        /// DTLS connects). Assert: returns Err (graceful, no panic/hang) or Ok (worker allows).
        #[tokio::test]
        async fn produce_data_on_unconnected_transport_graceful() {
            let sfu = SfuManager::new_with_port(random_udp_port()).await.expect("sfu");
            sfu.create_webrtc_transport("room-dp", "peer-a", "send").await.expect("transport");
            let result = sfu
                .create_data_producer(
                    "room-dp",
                    "peer-a",
                    "control",
                    "mediaservo.control",
                    Some(protocol::SctpStreamParameters {
                        stream_id: 1,
                        ordered: true,
                        max_packet_life_time: None,
                        max_retransmits: None,
                    }),
                    None,
                )
                .await;
            match result {
                Ok(r) => {
                    assert!(!r.data_producer_id.is_empty());
                    tracing::info!(
                        "produce_data on unconnected transport OK: {}",
                        r.data_producer_id
                    );
                }
                Err(e) => {
                    tracing::info!("produce_data on unconnected transport graceful error: {e}");
                    assert!(e.contains("data"), "the error should state the data/SCTP reason: {e}");
                }
            }
        }

        /// H1: missing sctp_stream_parameters must error explicitly (required for an SCTP producer, per official docs).
        #[tokio::test]
        async fn produce_data_requires_sctp_params() {
            let sfu = SfuManager::new_with_port(random_udp_port()).await.expect("sfu");
            sfu.create_webrtc_transport("room-dp2", "peer-a", "send").await.expect("transport");
            let err = sfu
                .create_data_producer(
                    "room-dp2",
                    "peer-a",
                    "control",
                    "mediaservo.control",
                    None,
                    None,
                )
                .await
                .expect_err("missing sctp_stream_parameters must error");
            assert!(err.contains("sctp_stream_parameters"), "error message: {err}");
        }

        // -- C1: per-peer transport registry (D-H14 order-independence -- host subprocesses create transports concurrently) --

        /// C1 key regression: after concurrent CreateWebRtcTransport(send) (boot storm / parallel crash-recovery),
        /// Produce must bind to the transport named by transport_id, not "most recently created" (the old single-slot
        /// overwrite -> producer attached to the wrong transport -> silent stream loss).
        #[tokio::test]
        async fn transport_registry_binds_produce_to_named_transport() {
            let sfu = SfuManager::new_with_port(random_udp_port()).await.expect("sfu");
            let t1 = sfu
                .create_webrtc_transport("room-c1", "host", "send")
                .await
                .expect("t1")
                .transport_id;
            let t2 = sfu
                .create_webrtc_transport("room-c1", "host", "send")
                .await
                .expect("t2")
                .transport_id;
            let t3 = sfu
                .create_webrtc_transport("room-c1", "host", "send")
                .await
                .expect("t3")
                .transport_id;
            // three send transports coexist (old impl: t3 overwrote the t1/t2 single slot)
            let rtp = serde_json::json!({"mid": "0", "codecs": [{"mimeType": "video/VP8", "payloadType": 100, "clockRate": 90000}], "headerExtensions": [], "encodings": [{"ssrc": 12345}], "rtcp": {"reducedSize": true}});
            let p1 = sfu
                .create_producer(
                    "room-c1",
                    "host",
                    &protocol::MediaKind::Video,
                    rtp.clone(),
                    Some(&t1),
                    None,
                )
                .await
                .expect("p1");
            let p2 = sfu
                .create_producer(
                    "room-c1",
                    "host",
                    &protocol::MediaKind::Video,
                    rtp.clone(),
                    Some(&t2),
                    None,
                )
                .await
                .expect("p2");
            assert_eq!(
                sfu.producer_transport_id(&p1.producer_id).as_deref(),
                Some(t1.as_str()),
                "P1 must bind to t1 (not the most recently created t3)"
            );
            assert_eq!(
                sfu.producer_transport_id(&p2.producer_id).as_deref(),
                Some(t2.as_str()),
                "P2 must bind to t2 (not the most recently created t3)"
            );
            // legacy client fallback: no transport_id -> most recently created (single-slot semantics preserved)
            let p3 = sfu
                .create_producer("room-c1", "host", &protocol::MediaKind::Video, rtp, None, None)
                .await
                .expect("p3");
            assert_eq!(
                sfu.producer_transport_id(&p3.producer_id).as_deref(),
                Some(t3.as_str()),
                "legacy (no transport_id) must fall back to the most recently created transport"
            );
        }

        /// C1: the data producer has the same semantics -- binds to the send transport named by transport_id;
        /// connect_transport looks up by id in the registry (old single slot: the first transport was overwritten and became unconnectable).
        #[tokio::test]
        async fn transport_registry_binds_data_producer_and_connects_by_id() {
            let sfu = SfuManager::new_with_port(random_udp_port()).await.expect("sfu");
            let t1 = sfu
                .create_webrtc_transport("room-c1d", "host", "send")
                .await
                .expect("t1")
                .transport_id;
            sfu.create_webrtc_transport("room-c1d", "host", "send").await.expect("t2");
            let dp = sfu
                .create_data_producer(
                    "room-c1d",
                    "host",
                    "control",
                    "mediaservo.control",
                    Some(protocol::SctpStreamParameters {
                        stream_id: 1,
                        ordered: true,
                        max_packet_life_time: None,
                        max_retransmits: None,
                    }),
                    Some(&t1),
                )
                .await
                .expect("dp");
            assert_eq!(
                sfu.data_producer_transport_id(&dp.data_producer_id).as_deref(),
                Some(t1.as_str()),
                "the DataProducer must bind to t1 (not the most recently created t2)"
            );
            // connect: t1 remains findable by id even after t2 was created (old single slot: t1 overwritten -> not found)
            let dtls = protocol::DtlsParameters {
            fingerprints: vec![protocol::Fingerprint {
                algorithm: "sha-256".into(),
                value: "AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99".into(),
            }],
            role: "client".into(),
        };
            // hit the registry (no not found); mediasoup-side success/failure both prove the lookup path is correct
            match sfu.connect_transport("room-c1d", "host", &t1, dtls).await {
                Ok(()) => {}
                Err(e) => {
                    assert!(!e.contains("not found"), "t1 must be found in the registry: {e}")
                }
            }
            // unknown id -> an explicit error (the reverse assertion of the registry lookup path)
            let err = sfu
                .connect_transport(
                    "room-c1d",
                    "host",
                    "ghost-transport",
                    protocol::DtlsParameters { fingerprints: vec![], role: "client".into() },
                )
                .await
                .expect_err("an unknown transport_id must error");
            assert!(err.contains("not found"), "error message: {err}");
        }
    }
}

// ── Stub when sfu-mediasoup is not enabled ──────────────────────────────

#[cfg(not(feature = "sfu-mediasoup"))]
mod imp {
    use super::protocol;

    /// Stub SfuManager — SFU not available.
    pub struct SfuManager;

    impl SfuManager {
        /// Returns an error in non-SFU builds.
        pub async fn new() -> Result<Self, String> {
            Err("sfu-mediasoup feature not enabled".into())
        }

        /// Stub — returns error in non-SFU builds.
        pub async fn create_webrtc_transport(
            &self,
            _room_id: &str,
            _peer_id: &str,
            _direction: &str,
        ) -> Result<TransportCreated, String> {
            Err("sfu-mediasoup feature not enabled".into())
        }

        /// Stub — returns error in non-SFU builds.
        pub async fn create_producer(
            &self,
            _room_id: &str,
            _peer_id: &str,
            _kind: &protocol::MediaKind,
            _rtp_parameters_json: serde_json::Value,
            _transport_id: Option<&str>,
            _label: Option<String>,
        ) -> Result<ProduceResult, String> {
            Err("sfu-mediasoup feature not enabled".into())
        }

        /// Stub — returns error in non-SFU builds.
        pub async fn create_consumer(
            &self,
            _room_id: &str,
            _peer_id: &str,
            _producer_id: &str,
            _rtp_capabilities_json: serde_json::Value,
            _transport_id: Option<&str>,
        ) -> Result<ConsumeResult, String> {
            Err("sfu-mediasoup feature not enabled".into())
        }

        /// Stub — returns error in non-SFU builds.
        pub async fn create_data_producer(
            &self,
            _room_id: &str,
            _peer_id: &str,
            _label: &str,
            _protocol: &str,
            _sctp_stream_parameters: Option<protocol::SctpStreamParameters>,
            _transport_id: Option<&str>,
        ) -> Result<DataProduceResult, String> {
            Err("sfu-mediasoup feature not enabled".into())
        }

        /// Stub — returns error in non-SFU builds.
        pub async fn create_data_consumer(
            &self,
            _room_id: &str,
            _peer_id: &str,
            _data_producer_id: &str,
            _transport_id: Option<&str>,
        ) -> Result<DataConsumeResult, String> {
            Err("sfu-mediasoup feature not enabled".into())
        }

        /// Stub — returns None in non-SFU builds.
        pub fn list_data_producers(&self, _room_id: &str) -> Option<Vec<(String, String, String)>> {
            None
        }

        /// Stub — returns error in non-SFU builds.
        pub async fn connect_transport(
            &self,
            _room_id: &str,
            _peer_id: &str,
            _transport_id: &str,
            _dtls_params: protocol::DtlsParameters,
        ) -> Result<(), String> {
            Err("sfu-mediasoup feature not enabled".into())
        }

        /// Stub — returns error in non-SFU builds (P1).
        pub async fn router_rtp_capabilities(
            &self,
            _room_id: &str,
        ) -> Result<serde_json::Value, String> {
            Err("sfu-mediasoup feature not enabled".into())
        }

        /// Stub — returns error in non-SFU builds (P1).
        pub async fn set_preferred_layers(
            &self,
            _room_id: &str,
            _peer_id: &str,
            _consumer_id: &str,
            _spatial_layer: u8,
            _temporal_layer: Option<u8>,
        ) -> Result<(), String> {
            Err("sfu-mediasoup feature not enabled".into())
        }

        /// Stub — no-op in non-SFU builds.
        pub fn remove_peer(&self, _room_id: &str, _peer_id: &str) -> bool {
            false
        }

        /// Stub -- no-op in non-SFU builds (F1/T4 single-room removal).
        pub fn remove_peer_in_room(
            &self,
            _room_id: &str,
            _peer_id: &str,
        ) -> Vec<(String, protocol::MediaKind)> {
            Vec::new()
        }

        /// Stub — no-op in non-SFU builds.
        pub fn remove_room(&self, _room_id: &str) -> bool {
            false
        }

        /// Stub -- the PIT-183 guard is always false in the no-SFU posture (behaviorally equal to the current direct teardown).
        pub fn room_has_producers(&self, _room_id: &str) -> bool {
            false
        }

        /// Stub — returns 0.
        pub fn room_count(&self) -> usize {
            0
        }
    }

    /// Stub TransportCreated — SFU not available.
    pub struct TransportCreated;

    /// Stub SfuRoom — SFU not available.
    pub struct SfuRoom;

    /// Stub SfuPeer — SFU not available.
    pub struct SfuPeer;

    /// Stub ProduceResult — SFU not available.
    pub struct ProduceResult;

    /// Stub ConsumeResult — SFU not available.
    pub struct ConsumeResult;

    /// Stub DataProduceResult — SFU not available.
    #[derive(Debug)]
    pub struct DataProduceResult;

    /// Stub DataConsumeResult — SFU not available.
    #[derive(Debug)]
    pub struct DataConsumeResult;
}

pub use imp::{
    ConsumeResult, DataConsumeResult, DataProduceResult, ProduceResult, SfuManager, SfuPeer,
    SfuRoom, TransportCreated,
};

/// For tests: the process-unique SFU test port (an atomic counter, incremented per call).
/// Once used bind :0 to probe free ports -- parallel-test TOCTOU races could get the same port (PIT-103 evidence);
/// the counter guarantees no repeats within one process; different test binaries = different processes, cargo runs them serially -> no conflict.
pub fn random_udp_port() -> u16 {
    static NEXT: std::sync::atomic::AtomicU16 = std::sync::atomic::AtomicU16::new(31000);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}
