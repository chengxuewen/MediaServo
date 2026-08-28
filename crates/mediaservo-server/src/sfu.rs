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

/// H2: 音频房间判定 — room_id 前缀约定 `audio-<vehicle-id>`。
/// 音频房间 = 全互连 opus 会议（车端 host-audio 进程 + 舱端 viewer+ + dispatcher）；
/// 与视频/控制房间（RoomType 无关，SFU room 由 room_id 字符串隔离）同机共存。
pub fn is_audio_room(room_id: &str) -> bool {
    room_id.starts_with("audio-")
}

/// PIT-143 裸机判定: 首 announced IP 是否属本机接口 IP 列表。
/// 容器内 announced = 宿主可达 IP（不在容器接口列表）→ false；
/// 裸机（announced 自动探测或 env/yaml 为本机 IP）→ true。
/// true  → 全部 ListenInfo bind 各自具体 IP（0.0.0.0 通配 + 具体 IP 同端口 = EADDRINUSE，T3 实证）；
/// false → 0.0.0.0 通配 + 首 announced（容器/注入场景）。
pub fn use_bare_metal_listen(
    announced: &[String],
    local: &std::collections::HashSet<String>,
) -> bool {
    announced.first().is_some_and(|first| local.contains(first))
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
        // 容器接口为 172.x，不含宿主 IP → 非裸机
        let announced = ["10.144.0.3".to_string()];
        assert!(!use_bare_metal_listen(&announced, &local(&["172.18.0.2"])));
    }

    #[test]
    fn mixed_first_local_rest_not() {
        // 首 IP 本机 = 裸机语义（后续非本机地址跳过）
        let announced = ["192.168.2.127".to_string(), "10.144.0.3".to_string()];
        assert!(use_bare_metal_listen(&announced, &local(&["192.168.2.127"])));
    }

    #[test]
    fn empty_announced_falls_back() {
        assert!(!use_bare_metal_listen(&[], &local(&["192.168.2.127"])), "空 announced 回退 0.0.0.0");
    }
}


#[cfg(feature = "sfu-mediasoup")]
mod imp {
    use super::*;
    use mediasoup::prelude::*;
    use mediasoup::worker_manager::WorkerManager;
    use mediasoup::webrtc_server::{WebRtcServer, WebRtcServerOptions, WebRtcServerListenInfos};
    use std::net::{IpAddr, Ipv4Addr};
    use std::sync::Arc;
    use std::num::{NonZeroU32, NonZeroU8};

    /// Detect the container's primary IP (zero-dependency UDP connect trick;
    /// connect() on UDP only sets the default route target, no packet is sent).
    /// 自动收集本机全部 IPv4（等价 ifconfig——多网卡/VPN 全公告；过滤 loopback）。
    /// 仅裸机有效：容器内看到的是容器网卡（172.x 不可达——容器场景必须 env/yaml 显式）。
    /// 公告多余候选无害（ICE 多候选，对端连不通自动跳过）。
    fn detect_all_ips() -> Vec<String> {
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
            // 兜底: 出网探测（原 detect_local_ip）
            if let Ok(socket) = std::net::UdpSocket::bind("0.0.0.0:0") {
                if socket.connect("8.8.8.8:80").is_ok() {
                    if let Ok(addr) = socket.local_addr() {
                        return vec![addr.ip().to_string()];
                    }
                }
            }
        }
        ips
    }

    /// PIT-58: announced_address 解析 — 环境变量 MEDIASERVO_SFU_ANNOUNCED_IP，
    /// 支持逗号分隔多 IP（宿主多网卡）。fallback: 容器内探测（172.18.0.2 仅本机可用，
    /// 部署方应经 CLI/外部注入真实宿主 IP）。
    /// announced 解析（三层优先级，PIT-138 修正）:
    /// env `MEDIASERVO_SFU_ANNOUNCED_IP`（逗号分隔多 IP）> server.yaml `sfu.announced_ips`
    /// > 自动探测（出网 IP——容器内不可靠，仅兜底）。
    fn announced_ips(config: Option<&mediaservo_common::config::ServerConfig>) -> Vec<String> {
        if let Ok(raw) = std::env::var("MEDIASERVO_SFU_ANNOUNCED_IP") {
            let ips: Vec<String> = raw
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
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
                preferred_payload_type: Some(111),  // Opus 显式（防与视频冲突）
                clock_rate: NonZeroU32::new(48000).unwrap(),
                channels: NonZeroU8::new(2).unwrap(),
                parameters: RtpCodecParametersParameters::default(),
                rtcp_feedback: vec![],
            },
            RtpCodecCapability::Video {
                mime_type: MimeTypeVideo::Vp8,
                preferred_payload_type: Some(96),  // VP8 显式（防与 H264 101 冲突）
                clock_rate: NonZeroU32::new(90000).unwrap(),
                parameters: RtpCodecParametersParameters::default(),
                rtcp_feedback: vec![],
            },
            RtpCodecCapability::Video {
                mime_type: MimeTypeVideo::H264,
                preferred_payload_type: Some(101),  // PIT-51: 与 Host produce 的 payloadType 101 匹配（None=自动分配≠101 → produce 失败）
                clock_rate: NonZeroU32::new(90000).unwrap(),
                parameters: RtpCodecParametersParameters::from([
                    ("level-asymmetry-allowed", 1_u32.into()),
                    ("packetization-mode", 1_u32.into()),
                    ("profile-level-id", "42e01f".into()), // v2: 4d0032→42e01f（浏览器解码兼容性, encoder-backend-codec-config T7 实证）
                ]),
                rtcp_feedback: vec![],
            },
            // v2 (2026-08-11): 用户要求 264/265/vp8/vp9/av1 — VP9/AV1 启用;
            // H265 不可用: mediasoup-rs 0.24 MimeTypeVideo 无 H265 绑定（worker 不支持, 需新版/自定义）
            RtpCodecCapability::Video {
                mime_type: MimeTypeVideo::Vp9,
                preferred_payload_type: Some(99),  // 防冲突: 96/101 已用, 100 被 mediasoup 自动分配占用
                clock_rate: NonZeroU32::new(90000).unwrap(),
                parameters: RtpCodecParametersParameters::default(),
                rtcp_feedback: vec![],
            },
            RtpCodecCapability::Video {
                mime_type: MimeTypeVideo::AV1,
                preferred_payload_type: Some(97),  // 池尾动态 PT（96/99/101 已用; 100-102 池首与 mediasoup 分配冲突, 实测）
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
    }

    /// Per-peer state: send/recv transports and active producers/consumers.
    pub struct SfuPeer {
        /// C1 (D-H14 顺序无关): send transport 注册表 — 多 host 子进程共享 peer_id
        /// （如 "host"），并发 CreateWebRtcTransport 不得互相覆盖（旧单槽: 后建覆盖
        /// 先建 → produce 挂错 transport → 静默流丢失）。`.last()` = legacy 单槽回退。
        pub send_transports: Vec<WebRtcTransport>,
        /// C1: recv transport 注册表（多连接共享 peer_id 场景同缺陷 — consumer 挂错
        /// transport → 黑屏，见 signaling.rs 注释）。`.last()` = legacy 单槽回退。
        pub recv_transports: Vec<WebRtcTransport>,
        pub producers: Vec<Producer>,
        pub consumers: Vec<Consumer>,
        /// H1 (SFU data 域): SCTP DataChannel producers/consumers（mediasoup 原生 data 域）。
        pub data_producers: Vec<DataProducer>,
        pub data_consumers: Vec<DataConsumer>,
        /// C1: producer_id → transport_id 绑定（produce 时记录；bind 断言/诊断访问器）。
        pub producer_transports: std::collections::HashMap<String, String>,
        /// C1: data_producer_id → transport_id 绑定。
        pub data_producer_transports: std::collections::HashMap<String, String>,
    }

    impl SfuPeer {
        /// 按 transport_id 定位 send transport；None = 最近创建（legacy 单槽语义，
        /// 单 transport 客户端行为不变）。
        fn send_transport(&self, transport_id: Option<&str>) -> Option<&WebRtcTransport> {
            match transport_id {
                Some(id) => self.send_transports.iter().find(|t| t.id().to_string() == id),
                None => self.send_transports.last(),
            }
        }

        /// 按 transport_id 定位 recv transport；None = 最近创建（legacy 单槽语义）。
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
    }

    /// H3: 管理面板房间列表摘要（音频会议面板数据源）。
    #[derive(Debug, Clone, serde::Serialize)]
    #[serde(rename_all = "snake_case")]
    pub struct SfuRoomSummary {
        pub room_id: String,
        /// 已连接参与者（peer）数。
        pub participants: usize,
        /// 房间内 producer 总数。
        pub producers: usize,
        /// 房间内 consumer 总数。
        pub consumers: usize,
        /// H2 音频房间（audio- 前缀）。
        pub audio: bool,
        /// 房间内 producer id 列表（SfuStats 查询用）。
        pub producer_ids: Vec<String>,
        /// 房间内 consumer id 列表（SfuStats 查询用）。
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
    fn convert_dtls_parameters(dtls: &mediasoup::prelude::DtlsParameters) -> protocol::DtlsParameters {
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

    impl SfuManager {
        /// Create a new SfuManager with a single mediasoup Worker and WebRtcServer.
        /// SFU 端口取自 `MEDIASERVO_SFU_PORT`（缺省 20000）— 测试用 `new_with_port` 传随机端口。
        pub async fn new(config: Option<&mediaservo_common::config::ServerConfig>) -> Result<Self, String> {
            let sfu_port = std::env::var("MEDIASERVO_SFU_PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(20000);
            Self::new_with_port_config(sfu_port, config).await
        }

        /// 指定 WebRtcServer 监听端口（测试隔离: 多测试并行各绑独立端口）。
        pub async fn new_with_port(sfu_port: u16) -> Result<Self, String> {
            Self::new_with_port_config(sfu_port, None).await
        }

        /// 内部实现: 端口 + 配置（announced 三层解析入口）。
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
            // PIT-44: listen 0.0.0.0 必须设 announced_address（mediasoup 官方要求），
            // 否则 candidate=0.0.0.0 对端无法 ICE；容器内探测本机 IP。
            // PIT-58: 容器内探测 = 172.18.0.2 (内网地址, 其他主机不可达 → Signal Lost);
            // 必须用宿主可达 IP — 环境变量 MEDIASERVO_SFU_ANNOUNCED_IP 配置 (宿主机网卡 IP)
            // 多 announced IP（PIT-143 修正）: 仅**裸机**支持多 ListenInfo——每个
            // ListenInfo **listen 各自具体 IP**（0.0.0.0 重复 bind 同端口 = 冲突）；
            // 容器内（announced 宿主 IP 不在容器接口列表）→ 单 ListenInfo
            // （0.0.0.0 + 首 announced——容器无法公告多地址）。
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
            // 首 ListenInfo 选择（PIT-143 修正 + T3 §3.2 实证）:
            // 裸机（首 announced ∈ 本机接口）→ bind 具体 IP：0.0.0.0 通配先占端口,
            // 之后具体 IP 同端口 bind = Linux EADDRINUSE（uv_udp_bind ... address already in use）;
            // 容器/注入（announced 宿主 IP 非本机接口）→ 0.0.0.0 + 首 announced（mediasoup 要求）。
            let first_listen_ip = if use_bare_metal_listen(&announced_ips, &local_ips) {
                match announced_ips[0].parse::<IpAddr>() {
                    Ok(ip) => ip,
                    Err(_) => {
                        tracing::warn!(
                            "bare-metal announced IP 非法 ({}), 回退 0.0.0.0 通配",
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
                port: Some(sfu_port),  // Fixed ICE port
                port_range: None,
                flags: None,
                send_buffer_size: None,
                recv_buffer_size: None,
            };
            let mut listen_infos = WebRtcServerListenInfos::new(make_info(
                first_listen_ip,
                announced_ips.first().cloned(),
            ));
            // 其余 announced: 本机接口 → 各建 ListenInfo（listen 具体 IP）;
            // 非本机（容器注入的宿主 IP 等）→ 跳过。
            for ip in announced_ips.iter().skip(1) {
                if !local_ips.contains(ip) {
                    continue; // 容器注入的宿主 IP——容器内无该接口，跳过
                }
                let Ok(listen_ip) = ip.parse::<IpAddr>() else { continue };
                listen_infos = listen_infos.insert(make_info(listen_ip, Some(ip.clone())));
            }
            tracing::info!("WebRtcServer created on port {sfu_port} (announced: {announced_ips:?})");
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

        /// Create a WebRTC transport for a peer in a room.
        pub async fn create_webrtc_transport(
            &self,
            room_id: &str,
            peer_id: &str,
            direction: &str,
        ) -> Result<TransportCreated, String> {
            // Get or create room
            let router = {
                if let Some(room) = self.rooms.get(room_id) {
                    Arc::clone(&room.router)
                } else {
                    // No room yet — create one
                    let router = self
                        .worker
                        .create_router(default_router_options())
                        .await
                        .map_err(|e| {
                            // v2 诊断: 打印 codec 列表定位 PT 冲突
                            tracing::error!("Router create failed; media_codecs={:?}", default_router_options());
                            format!("Failed to create router: {e}")
                        })?;
                    let router = Arc::new(router);
                    tracing::info!("Router created for room {}", room_id);

                    self.rooms.insert(
                        room_id.to_string(),
                        SfuRoom {
                            router: Arc::clone(&router),
                            peers: DashMap::new(),
                        },
                    );
                    router
                }
            };

            // Create transport using shared WebRtcServer (single port)
            // H1 (SFU data 域): enable_sctp = true — SCTP/DataChannel 协商必需（mediasoup
            // 官方: WebRtcTransportOptions.enableSctp 默认 false）。仅当对端 SDP 含
            // m=application 时才建 SCTP association — 纯媒体流不受影响（additive）。
            let mut options = WebRtcTransportOptions::new_with_server(self.webrtc_server.as_ref().clone());
            options.enable_sctp = true;
            let transport = router
                .create_webrtc_transport(options)
                .await
                .map_err(|e| format!("Failed to create transport: {e}"))?;

            let transport_id = transport.id().to_string();
            let ice = transport.ice_parameters().clone();
            let dtls = transport.dtls_parameters();
            let ice_candidates = convert_ice_candidates(transport.ice_candidates());

            // Store transport on peer
            if let Some(room) = self.rooms.get_mut(room_id) {
                let mut peer = room.peers.entry(peer_id.to_string()).or_insert_with(|| {
                    SfuPeer {
                        send_transports: Vec::new(),
                        recv_transports: Vec::new(),
                        producers: Vec::new(),
                        consumers: Vec::new(),
                        data_producers: Vec::new(),
                        data_consumers: Vec::new(),
                        producer_transports: std::collections::HashMap::new(),
                        data_producer_transports: std::collections::HashMap::new(),
                    }
                });

                match direction {
                    "send" => {
                        // C1: 追加进注册表（不覆盖 — 并发子进程各自 transport 并存）
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
            })
        }

        /// Remove a peer from a room, cleaning up transports, producers, and consumers.
        /// Returns true if the peer was found and removed.
        /// If the room becomes empty after removal, the Router is destroyed.
        pub fn remove_peer(&self, room_id: &str, peer_id: &str) -> bool {
            if let Some(mut room) = self.rooms.get_mut(room_id) {
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

        /// H1 修正: 跨房间按 peer_id 清除（producer 按流注册在各 stream 房间的 sfu peers，
        /// 而 signaling 会话的 relay_room=整车房间——旧单房间版 remove_peer 清不到 stream 房间的
        /// producer，造成重连后死 producer 泄漏 + 无 ProducerClosed 广播）。
        /// 返回 (room_id, [(producer_id, kind)]) 供逐房间广播。房间清空则销毁 Router。
        pub fn remove_peer_global(&self, peer_id: &str) -> Vec<(String, Vec<(String, protocol::MediaKind)>)> {
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
                    tracing::info!("Peer {} removed from SFU room {} ({} producers)", peer_id, rid, closed.len());
                    if entry.value().peers.is_empty() {
                        empty_rooms.push(rid.clone());
                    }
                    out.push((rid, closed));
                }
            }
            // (iter_mut 守卫随 for 循环结束已释放——下方删除安全)
            for rid in empty_rooms {
                self.remove_room(&rid);
            }
            out
        }

        /// H1 修正: 按 producer_id 列表跨房间清理（各 stream 房间的 sfu peer 键为
        /// 自报 peer_id，会话 id 清理漏删——泄漏）。返回 (room, producer_id, kind) 供广播。
        pub fn remove_producers_by_ids(&self, producer_ids: &[String]) -> Vec<(String, String, protocol::MediaKind)> {
            let mut out = Vec::new();
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
                    for (pid, kind) in &hits {
                        peer.producer_transports.remove(pid);
                        tracing::info!("Producer {pid} removed from SFU room {rid} (device-owned cleanup)");
                    }
                    out.extend(hits.into_iter().map(|(pid, kind)| (rid.clone(), pid, kind)));
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
        /// C1: transport_id 指名绑定 transport；None = legacy 单槽回退（最近创建）。
        pub async fn create_producer(
            &self,
            room_id: &str,
            peer_id: &str,
            kind: &protocol::MediaKind,
            rtp_parameters_json: serde_json::Value,
            transport_id: Option<&str>,
        ) -> Result<ProduceResult, String> {
            // Convert JSON RTP parameters to mediasoup type
            let rtp_parameters: RtpParameters = serde_json::from_value(rtp_parameters_json)
                .map_err(|e| format!("Invalid RTP parameters: {e}"))?;

            let ms_kind = match kind {
                protocol::MediaKind::Audio => MediaKind::Audio,
                protocol::MediaKind::Video => MediaKind::Video,
            };

            let room = self.rooms.get_mut(room_id)
                .ok_or_else(|| format!("Room {} not found for produce", room_id))?;
            let mut peer = room.peers.get_mut(peer_id)
                .ok_or_else(|| format!("Peer {} not found in room {}", peer_id, room_id))?;

            let transport = peer.send_transport(transport_id)
                .ok_or_else(|| format!("No send transport for peer {} (transport_id={transport_id:?})", peer_id))?;

            // ponytail: construct ProducerOptions; let compiler validate the exact constructor
            let producer_options = ProducerOptions::new(ms_kind, rtp_parameters);
            let producer = transport.produce(producer_options).await
                .map_err(|e| format!("Failed to create producer: {e}"))?;

            let producer_id = producer.id().to_string();
            tracing::info!(
                "Producer {} ({:?}) created for peer {} in room {}",
                producer_id, kind, peer_id, room_id
            );


            // C1: 记录 producer→transport 绑定（bind 断言/诊断）— 先取 id 结束 transport 借用。
            let bound_transport_id = transport.id().to_string();
            peer.producers.push(producer);
            peer.producer_transports.insert(producer_id.clone(), bound_transport_id);
            Ok(ProduceResult {
                producer_id,
                kind: kind.clone(),
            })
        }

        /// Create a consumer for a peer on its recv transport,
        /// subscribing to an existing producer in the room.
        /// C1: transport_id 指名绑定 recv transport；None = legacy 单槽回退。
        pub async fn create_consumer(
            &self,
            room_id: &str,
            peer_id: &str,
            producer_id: &str,
            rtp_capabilities_json: serde_json::Value,
            transport_id: Option<&str>,
        ) -> Result<ConsumeResult, String> {
            // Convert JSON RTP capabilities to mediasoup type
            let rtp_capabilities: RtpCapabilities = serde_json::from_value(rtp_capabilities_json)
                .map_err(|e| format!("Invalid RTP capabilities: {e}"))?;

            // Find the producer and extract its id + kind
            // ponytail: read-lock first to get producer info, then write-lock for consumer insert
            let (producer_id_ms, producer_kind) = {
                let room = self.rooms.get(room_id)
                    .ok_or_else(|| format!("Room {} not found for consume", room_id))?;
                room.peers.iter()
                    .find_map(|entry| {
                        entry.producers.iter()
                            .find(|p| p.id().to_string() == producer_id)
                            .map(|p| (p.id(), p.kind()))
                    })
                    .ok_or_else(|| {
                        format!("Producer {} not found in room {}", producer_id, room_id)
                    })?
            };

            // Now get the consumer peer's recv transport
            let room = self.rooms.get_mut(room_id)
                .ok_or_else(|| format!("Room {} not found", room_id))?;
            let mut peer = room.peers.get_mut(peer_id)
                .ok_or_else(|| format!("Peer {} not found in room {}", peer_id, room_id))?;
            let transport = peer.recv_transport(transport_id)
                .ok_or_else(|| format!("No recv transport for peer {} (transport_id={transport_id:?})", peer_id))?;

            let consumer_options = ConsumerOptions::new(producer_id_ms, rtp_capabilities);
            let consumer = transport.consume(consumer_options).await
                .map_err(|e| format!("Failed to create consumer: {e}"))?;

            let consumer_id = consumer.id().to_string();
            let protocol_kind = match producer_kind {
                MediaKind::Audio => protocol::MediaKind::Audio,
                MediaKind::Video => protocol::MediaKind::Video,
            };
            let rtp_parameters_json = serde_json::to_value(consumer.rtp_parameters())
                .unwrap_or_default();

            tracing::info!(
                "Consumer {} created for peer {} (producer: {}, kind: {:?})",
                consumer_id, peer_id, producer_id, protocol_kind
            );

            // PIT-65 观测: Consumer RTP trace (每包处理/丢弃原因)
            {
                let cid = consumer_id.clone();
                consumer.on_trace(move |trace: &mediasoup::consumer::ConsumerTraceEventData| {
                    tracing::info!("CONS-TRACE {}: {:?}", cid, trace);
                });
                let _ = consumer
                    .enable_trace_event(vec![mediasoup::consumer::ConsumerTraceEventType::Rtp])
                    .await;
            }

            // PIT-65 观测: Consumer dump (rtp_streams score — IsActive 判定依据)
            {
                let cid = consumer_id.clone();
                if let Ok(dump) = consumer.dump().await {
                    let scores: Vec<u8> = dump.rtp_streams.iter().map(|s| s.score).collect();
                    tracing::info!("CONS-DUMP {}: paused={} scores={:?}", cid, dump.paused, scores);
                }
            }


            // PIT-76: consume 后立即请求关键帧 — 绕过 libwebrtc 99s GOP（x-google-
            // max-keyframe-interval 注入对软件 VP8 编码器不生效，实测仍 99s）。
            // mediasoup request_key_frame → producer 侧发送关键帧请求 → 编码器立即出 IDR
            if protocol_kind == protocol::MediaKind::Video {
                match consumer.request_key_frame().await {
                    Ok(()) => tracing::info!("Consumer {consumer_id}: requested key frame (instant first-frame)"),
                    Err(e) => tracing::warn!("Consumer {consumer_id}: request_key_frame failed: {e}"),
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
        /// H1 (SFU data 域): mediasoup 原生 data 域 — DataProducerOptions::new_sctp
        /// (官方 DataProducerOptions.sctpStreamParameters + label + protocol)。
        /// 消息流转是 mediasoup worker 内部的（端点 SCTP → worker → DataConsumers），
        /// server 只负责 produce/consume 接线（官方文档 transport.produceData）。
        pub async fn create_data_producer(
            &self,
            room_id: &str,
            peer_id: &str,
            label: &str,
            protocol: &str,
            sctp_stream_parameters: Option<protocol::SctpStreamParameters>,
            transport_id: Option<&str>,
        ) -> Result<DataProduceResult, String> {
            let sctp_params = sctp_stream_parameters
                .ok_or_else(|| "sctp_stream_parameters required for SCTP data producer".to_string())?;
            let ms_sctp = SctpStreamParameters {
                stream_id: sctp_params.stream_id,
                ordered: sctp_params.ordered,
                max_packet_life_time: sctp_params.max_packet_life_time,
                max_retransmits: sctp_params.max_retransmits,
            };

            let room = self.rooms.get_mut(room_id)
                .ok_or_else(|| format!("Room {} not found for produce_data", room_id))?;
            let mut peer = room.peers.get_mut(peer_id)
                .ok_or_else(|| format!("Peer {} not found in room {}", peer_id, room_id))?;
            let transport = peer.send_transport(transport_id)
                .ok_or_else(|| format!("No send transport for peer {} (transport_id={transport_id:?})", peer_id))?;

            let mut options = DataProducerOptions::new_sctp(ms_sctp);
            options.label = label.to_string();
            options.protocol = protocol.to_string();
            let producer = transport.produce_data(options).await
                .map_err(|e| format!("Failed to create data producer: {e}"))?;

            let data_producer_id = producer.id().to_string();
            tracing::info!(
                "DataProducer {} (label={}) created for peer {} in room {}",
                data_producer_id, label, peer_id, room_id
            );
            // C1: 记录 data producer→transport 绑定（bind 断言/诊断）— 先取 id 结束 transport 借用。
            let bound_transport_id = transport.id().to_string();
            peer.data_producers.push(producer);
            peer.data_producer_transports.insert(data_producer_id.clone(), bound_transport_id);
            Ok(DataProduceResult { data_producer_id })
        }

        /// Create a data consumer (SCTP DataChannel) for a peer on its recv transport,
        /// subscribing to an existing data producer in the room.
        /// DataConsumerOptions::new_sctp 继承 producer 的 ordered/可靠性参数（官方 API）。
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
                let room = self.rooms.get(room_id)
                    .ok_or_else(|| format!("Room {} not found for consume_data", room_id))?;
                room.peers.iter()
                    .find_map(|entry| {
                        entry.data_producers.iter()
                            .find(|p| p.id().to_string() == data_producer_id)
                            .map(|p| p.id())
                    })
                    .ok_or_else(|| {
                        format!("DataProducer {} not found in room {}", data_producer_id, room_id)
                    })?
            };

            let room = self.rooms.get_mut(room_id)
                .ok_or_else(|| format!("Room {} not found", room_id))?;
            let mut peer = room.peers.get_mut(peer_id)
                .ok_or_else(|| format!("Peer {} not found in room {}", peer_id, room_id))?;
            let transport = peer.recv_transport(transport_id)
                .ok_or_else(|| format!("No recv transport for peer {} (transport_id={transport_id:?})", peer_id))?;

            let consumer_options = DataConsumerOptions::new_sctp(producer_id_ms);
            let consumer = transport.consume_data(consumer_options).await
                .map_err(|e| format!("Failed to create data consumer: {e}"))?;

            let data_consumer_id = consumer.id().to_string();
            tracing::info!(
                "DataConsumer {} created for peer {} (data_producer: {}) in room {}",
                data_consumer_id, peer_id, data_producer_id, room_id
            );

            // H1 观测: 消费端 on_message — SCTP DataConsumer 的消息事件（端点经 SCTP
            // association 接收; Direct 型消息在 Rust 侧触发; 官方 dataConsumer 事件）。
            let cid = data_consumer_id.clone();
            consumer.on_message(move |msg: &WebRtcMessage<'_>| {
                tracing::debug!("DATA-CONS {}: {:?}", cid, msg);
            });

            peer.data_consumers.push(consumer);

            Ok(DataConsumeResult {
                data_consumer_id,
                data_producer_id: data_producer_id.to_string(),
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

            let room = self.rooms.get_mut(room_id)
                .ok_or_else(|| format!("Room {room_id} not found for connect"))?;
            let peer = room.peers.get_mut(peer_id)
                .ok_or_else(|| format!("Peer {peer_id} not found in room {room_id}"))?;

            // C1: 在 send/recv 注册表中按 id 查找（旧单槽: 后建覆盖先建 → 先建 connect 失败）
            let transport = peer
                .send_transports
                .iter()
                .chain(peer.recv_transports.iter())
                .find(|t| t.id().to_string() == transport_id)
                .ok_or_else(|| {
                    format!("Transport {transport_id} not found for peer {peer_id}")
                })?;

            transport.connect(mediasoup::prelude::WebRtcTransportRemoteParameters { dtls_parameters: ms_dtls }).await
                .map_err(|e| format!("Failed to connect transport: {e}"))?;

            // PIT 观测: dump transport selected tuple（server 学到的客户端地址 — RTP 发送目标）
            if let Ok(dump) = transport.dump().await {
                tracing::info!(
                    "SFU: transport {transport_id} dump: ice={:?} dtls={:?} selected_tuple={:?}",
                    dump.ice_state, dump.dtls_state, dump.ice_selected_tuple
                );
            }

            tracing::info!(
                "SFU: transport {transport_id} connected for peer {peer_id} in room {room_id}"
            );
            Ok(())
        }

        /// C1: producer → transport 绑定访问器（bind 断言/诊断；produce 时记录）。
        pub fn producer_transport_id(&self, producer_id: &str) -> Option<String> {
            self.rooms.iter().find_map(|room| {
                room.peers.iter().find_map(|peer| {
                    peer.producer_transports.get(producer_id).cloned()
                })
            })
        }

        /// C1: data producer → transport 绑定访问器（bind 断言/诊断）。
        pub fn data_producer_transport_id(&self, data_producer_id: &str) -> Option<String> {
            self.rooms.iter().find_map(|room| {
                room.peers.iter().find_map(|peer| {
                    peer.data_producer_transports.get(data_producer_id).cloned()
                })
            })
        }

        /// List all producers in a room. Returns (producer_id, kind, peer_id) tuples.
        /// Used for late-joiner sync to send existing producers to new consumers.
        pub fn list_producers(&self, room_id: &str) -> Option<Vec<(String, protocol::MediaKind, String)>> {
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
            Some(result)
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
        pub fn room_count(&self) -> usize {
            self.rooms.len()
        }

        /// H3: 全部房间摘要（管理面板房间列表）— 含 producer/consumer id 供 SfuStats 查询。
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

        /// H2: 查询 producer 的入站 RTP 统计（get_stats）— 媒体面证据（音频房间 e2e）。
        /// 返回 (kind, byte_count, packet_count, score)。
        pub async fn producer_stats(&self, producer_id: &str) -> Result<(protocol::MediaKind, u64, u64, u8), String> {
            let producer = self
                .rooms
                .iter()
                .find_map(|room| {
                    room.peers.iter().find_map(|peer| {
                        peer.producers
                            .iter()
                            .find(|p| p.id().to_string() == producer_id)
                            .cloned()
                    })
                })
                .ok_or_else(|| format!("Producer {producer_id} not found"))?;
            let kind = match producer.kind() {
                MediaKind::Audio => protocol::MediaKind::Audio,
                MediaKind::Video => protocol::MediaKind::Video,
            };
            let stats = producer.get_stats().await.map_err(|e| format!("Producer stats: {e}"))?;
            // 无 RTP 到达时 mediasoup 不建 RtpStream → 空 vec（合法零值语义）。
            let (bytes, packets, score) = match stats.first() {
                Some(s) => (s.byte_count, s.packet_count, s.score),
                None => (0, 0, 0),
            };
            Ok((kind, bytes, packets, score))
        }

        /// H2: 查询 consumer 的出站 RTP 统计（get_stats）— 路由转发证据（音频房间 e2e）。
        /// 返回 (kind, byte_count, packet_count, score)。
        pub async fn consumer_stats(&self, consumer_id: &str) -> Result<(protocol::MediaKind, u64, u64, u8), String> {
            let consumer = self
                .rooms
                .iter()
                .find_map(|room| {
                    room.peers.iter().find_map(|peer| {
                        peer.consumers
                            .iter()
                            .find(|c| c.id().to_string() == consumer_id)
                            .cloned()
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
    }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_audio_room_recognizes_prefix() {
        assert!(is_audio_room("audio-ms-car1"), "audio- 前缀 = 音频房间");
        assert!(!is_audio_room("ms-car1"), "普通整车房间不是音频房间");
        assert!(!is_audio_room("room-1"), "无前缀房间不是音频房间");
    }


    /// PIT-58: announced_address 必须优先环境变量 (宿主可达 IP) —
    /// 容器内探测 (172.18.0.2) 仅本机可用, 其他主机 ICE 不可达 → Signal Lost。
    #[test]
    fn detect_all_ips_collects_interfaces_no_loopback() {
        let ips = detect_all_ips();
        assert!(!ips.is_empty(), "裸机至少一个非 loopback IPv4");
        assert!(
            ips.iter().all(|ip| !ip.starts_with("127.")),
            "不得含 loopback: {ips:?}"
        );
        // 去重
        let mut sorted = ips.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(ips.len(), sorted.len(), "IP 列表去重");
    }

    fn announced_ips_prefers_env_and_falls_back() {
        // env 优先 (宿主 IP 场景)
        // SAFETY: 测试内串行设置/恢复, 无并发读
        unsafe { std::env::set_var("MEDIASERVO_SFU_ANNOUNCED_IP", "192.168.2.127"); }
        assert_eq!(announced_ips(None), vec!["192.168.2.127".to_string()]);

        // 多 IP 逗号分隔
        // SAFETY: 同上
        unsafe { std::env::set_var("MEDIASERVO_SFU_ANNOUNCED_IP", "192.168.2.127,10.0.0.5"); }
        assert_eq!(
            announced_ips(None),
            vec!["192.168.2.127".to_string(), "10.0.0.5".to_string()]
        );

        // 逗号 + 空格容错
        // SAFETY: 同上
        unsafe { std::env::set_var("MEDIASERVO_SFU_ANNOUNCED_IP", " 192.168.2.127 , 10.0.0.5 "); }
        assert_eq!(announced_ips(None).len(), 2);

        // fallback 探测 (未配置场景)
        // SAFETY: 同上
        unsafe { std::env::remove_var("MEDIASERVO_SFU_ANNOUNCED_IP"); }
        let fallback = announced_ips(None);

        // server.yaml sfu.announced_ips（env 未设时生效）
        // SAFETY: 同上
        unsafe { std::env::remove_var("MEDIASERVO_SFU_ANNOUNCED_IP"); }
        let cfg: mediaservo_common::config::ServerConfig = serde_yaml::from_str(
            "version: 1\nlisten:\n  host: 0.0.0.0\n  port: 9800\nsfu:\n  announced_ips:\n    - 10.144.0.3\n",
        )
        .unwrap();
        assert_eq!(announced_ips(Some(&cfg)), vec!["10.144.0.3".to_string()]);
        assert!(!fallback.is_empty(), "fallback 探测应返回非空 IP");

        // 恢复环境, 避免污染其他测试
        // SAFETY: 同上
        unsafe { std::env::remove_var("MEDIASERVO_SFU_ANNOUNCED_IP"); }
    }

    /// H1: WebRtcTransport 必须带 SCTP 创建（enable_sctp）— DataChannel 协商的前置条件。
    /// 断言 transport dump 的 sctp_parameters 非空（mediasoup 官方: enableSctp 默认 false，
    /// 未启用时 dump 无 sctp 段）。
    #[tokio::test]
    async fn transport_sctp_enabled() {
        let sfu = SfuManager::new_with_port(random_udp_port()).await.expect("sfu");
        sfu.create_webrtc_transport("room-sctp", "peer-a", "send")
            .await
            .expect("transport");
        let room = sfu.rooms.get("room-sctp").expect("room");
        let peer = room.peers.get("peer-a").expect("peer");
        let dump = peer
            .send_transports
            .last()
            .expect("send transport")
            .dump()
            .await
            .expect("dump");
        assert!(
            dump.sctp_parameters.is_some(),
            "WebRtcTransport 必须启用 SCTP（enable_sctp）: sctp_parameters={:?}",
            dump.sctp_parameters
        );
    }

    /// H1: data 域实体创建 + worker 侧投递指标（消息接收证明的上限）—
    /// DataProducer/DataConsumer 接线成功 + DataProducer.send() 后 consumer stats
    /// messages_sent>0（worker 内部路由到 DataConsumer，官方 Router::OnTransportDataProducerMessageReceived）。
    /// 注: worker→app 通知通道（DataConsumer.on_message / on_data_producer_close）在本部署
    /// 整体失效（官方 mediasoup-rs 测试同构复刻亦失败 — 见 data_message_roundtrip_direct
    /// 的 #[ignore] 文档），消息接收证明见该测试。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn data_domain_entity_creation() {
        use mediasoup::prelude::*;
        use mediasoup::worker_manager::WorkerManager;

        let worker_manager = WorkerManager::new();
        let worker = worker_manager
            .create_worker(WorkerSettings::default())
            .await
            .expect("worker");
        let router = worker
            .create_router(default_router_options())
            .await
            .expect("router");
        let t1 = router
            .create_direct_transport(DirectTransportOptions::default())
            .await
            .expect("direct transport 1");
        let t2 = router
            .create_direct_transport(DirectTransportOptions::default())
            .await
            .expect("direct transport 2");

        // producer (Direct) + consumer (Direct) 接线
        let producer = t1
            .produce_data(DataProducerOptions::new_direct())
            .await
            .expect("produce_data");
        let consumer = t2
            .consume_data(DataConsumerOptions::new_direct(producer.id(), None))
            .await
            .expect("consume_data");
        assert!(matches!(producer, DataProducer::Direct(_)));
        assert!(matches!(consumer, DataConsumer::Direct(_)));
        assert!(!consumer.closed());

        // DirectDataProducer.send() → worker 路由（consumer stats 证明）
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
            "worker 必须已把消息路由到 DataConsumer (messages_sent=1)"
        );
        assert_eq!(stats.bytes_sent, 13);
    }

    /// H1: DataProducer.send() → DataConsumer.on_message() 端到端消息接收证明。
    /// #[ignore]: 被 mediasoup-rs 0.24.1 部署级 bug 阻塞 — worker→app 通知通道整体失效
    /// （on_message / on_data_producer_close / worker_close 全部静默丢失; 官方 mediasoup-rs
    /// data_consumer::tests::data_producer_close_event 同构复刻在本部署同样失败）。
    /// 已证实: 请求/响应正常（dump/get_stats），worker 侧路由正常（messages_sent=1），
    /// 丢失点 = mediasoup-rs channel 通知分发（缓冲/订阅生命周期，疑似 buffer guard 竞态）。
    /// 归属: mediasoup-rs upstream（H1 报告 PIT）；host 侧 SFU-DC 接线依赖修复后验证。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "mediasoup-rs notification channel bug in this deployment (PIT H1)"]
    async fn data_message_roundtrip_direct() {
        use mediasoup::prelude::*;
        use mediasoup::worker_manager::WorkerManager;
        use std::time::Duration;

        let worker_manager = WorkerManager::new();
        let worker = worker_manager
            .create_worker(WorkerSettings::default())
            .await
            .expect("worker");
        let router = worker
            .create_router(default_router_options())
            .await
            .expect("router");
        let t1 = router
            .create_direct_transport(DirectTransportOptions::default())
            .await
            .expect("direct transport 1");
        let t2 = router
            .create_direct_transport(DirectTransportOptions::default())
            .await
            .expect("direct transport 2");

        let producer = t1
            .produce_data(DataProducerOptions::new_direct())
            .await
            .expect("produce_data");
        let consumer = t2
            .consume_data(DataConsumerOptions::new_direct(producer.id(), None))
            .await
            .expect("consume_data");

        // on_message: worker → Rust 回调（消息投递证明）
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

        // worker 侧投递证明（即使 on_message 失效也成立）
        let stats = consumer
            .get_stats()
            .await
            .expect("consumer stats")
            .into_iter()
            .next()
            .expect("one stat");
        assert_eq!(stats.messages_sent, 1, "worker 必须已路由消息到 DataConsumer");

        let got = std::thread::spawn(move || {
            rx.recv_timeout(Duration::from_secs(5))
                .expect("on_message 回调必须收到消息（mediasoup worker→app 通知通道）")
        })
        .join()
        .expect("waiter thread");
        assert_eq!(got, b"sfu-data-echo".to_vec());
    }

    /// H1: SfuManager 级 data producer 创建 — 未完成 DTLS 的 WebRtcTransport 上
    /// produce_data 的结果（worker 行为探测: mediasoup SCTP association 需 DTLS
    /// 连接后创建）。断言: 返回 Err（优雅失败, 无 panic/挂起）或 Ok（worker 允许）。
    #[tokio::test]
    async fn produce_data_on_unconnected_transport_graceful() {
        let sfu = SfuManager::new_with_port(random_udp_port()).await.expect("sfu");
        sfu.create_webrtc_transport("room-dp", "peer-a", "send")
            .await
            .expect("transport");
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
                tracing::info!("produce_data on unconnected transport OK: {}", r.data_producer_id);
            }
            Err(e) => {
                tracing::info!("produce_data on unconnected transport graceful error: {e}");
                assert!(e.contains("data"), "错误应说明 data/SCTP 原因: {e}");
            }
        }
    }

    /// H1: 缺 sctp_stream_parameters 必须明确报错（SCTP producer 必需, 官方文档）。
    #[tokio::test]
    async fn produce_data_requires_sctp_params() {
        let sfu = SfuManager::new_with_port(random_udp_port()).await.expect("sfu");
        sfu.create_webrtc_transport("room-dp2", "peer-a", "send")
            .await
            .expect("transport");
        let err = sfu
            .create_data_producer("room-dp2", "peer-a", "control", "mediaservo.control", None, None)
            .await
            .expect_err("缺 sctp_stream_parameters 必须报错");
        assert!(err.contains("sctp_stream_parameters"), "错误信息: {err}");
    }

    // ── C1: per-peer transport 注册表（D-H14 顺序无关 — host 子进程并发建 transport）──

    /// C1 关键回归: 并发 CreateWebRtcTransport(send)（boot storm / 并行 crash-recovery）
    /// 后，Produce 必须绑定到 transport_id 命名的 transport，而非"最近创建"（旧单槽
    /// 覆盖 → producer 挂错 transport → 静默流丢失）。
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
        // 三个 send transport 并存（旧实现: t3 覆盖 t1/t2 单槽）
        let rtp = serde_json::json!({"mid": "0", "codecs": [{"mimeType": "video/VP8", "payloadType": 100, "clockRate": 90000}], "headerExtensions": [], "encodings": [{"ssrc": 12345}], "rtcp": {"reducedSize": true}});
        let p1 = sfu
            .create_producer("room-c1", "host", &protocol::MediaKind::Video, rtp.clone(), Some(&t1))
            .await
            .expect("p1");
        let p2 = sfu
            .create_producer("room-c1", "host", &protocol::MediaKind::Video, rtp.clone(), Some(&t2))
            .await
            .expect("p2");
        assert_eq!(
            sfu.producer_transport_id(&p1.producer_id).as_deref(),
            Some(t1.as_str()),
            "P1 必须绑定到 t1（非最近创建的 t3）"
        );
        assert_eq!(
            sfu.producer_transport_id(&p2.producer_id).as_deref(),
            Some(t2.as_str()),
            "P2 必须绑定到 t2（非最近创建的 t3）"
        );
        // legacy 客户端回退: 无 transport_id → 最近创建（单槽语义保持）
        let p3 = sfu
            .create_producer("room-c1", "host", &protocol::MediaKind::Video, rtp, None)
            .await
            .expect("p3");
        assert_eq!(
            sfu.producer_transport_id(&p3.producer_id).as_deref(),
            Some(t3.as_str()),
            "legacy（无 transport_id）必须回退最近创建 transport"
        );
    }

    /// C1: data producer 同语义 — 绑定 transport_id 命名的 send transport；
    /// connect_transport 在注册表中按 id 查找（旧单槽: 先建 transport 被覆盖后无法连接）。
    #[tokio::test]
    async fn transport_registry_binds_data_producer_and_connects_by_id() {
        let sfu = SfuManager::new_with_port(random_udp_port()).await.expect("sfu");
        let t1 = sfu
            .create_webrtc_transport("room-c1d", "host", "send")
            .await
            .expect("t1")
            .transport_id;
        sfu.create_webrtc_transport("room-c1d", "host", "send")
            .await
            .expect("t2");
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
            "DataProducer 必须绑定到 t1（非最近创建的 t2）"
        );
        // connect: t1 在 t2 创建后仍可按 id 找到（旧单槽: t1 被覆盖 → not found）
        let dtls = protocol::DtlsParameters {
            fingerprints: vec![protocol::Fingerprint {
                algorithm: "sha-256".into(),
                value: "AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99".into(),
            }],
            role: "client".into(),
        };
        // 命中注册表（不报 not found）；mediasoup 侧成功/失败均证明查找路径正确
        match sfu.connect_transport("room-c1d", "host", &t1, dtls).await {
            Ok(()) => {}
            Err(e) => assert!(!e.contains("not found"), "t1 必须在注册表中找到: {e}"),
        }
        // 未知 id → 明确报错（注册表查找路径的反向断言）
        let err = sfu
            .connect_transport(
                "room-c1d",
                "host",
                "ghost-transport",
                protocol::DtlsParameters { fingerprints: vec![], role: "client".into() },
            )
            .await
            .expect_err("未知 transport_id 必须报错");
        assert!(err.contains("not found"), "错误信息: {err}");
    }
}}

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
        pub async fn connect_transport(&self, _room_id: &str, _peer_id: &str, _transport_id: &str, _dtls_params: protocol::DtlsParameters) -> Result<(), String> {
            Err("sfu-mediasoup feature not enabled".into())
        }

        /// Stub — no-op in non-SFU builds.
        pub fn remove_peer(&self, _room_id: &str, _peer_id: &str) -> bool {
            false
        }

        /// Stub — no-op in non-SFU builds.
        pub fn remove_room(&self, _room_id: &str) -> bool {
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

pub use imp::{SfuManager, SfuPeer, SfuRoom, TransportCreated, ProduceResult, ConsumeResult, DataProduceResult, DataConsumeResult};

/// 测试用: 进程内唯一的 SFU 测试端口（原子计数器，每次调用 +1）。
/// 曾用 bind :0 探空闲端口 — 并行测试 TOCTOU 竞态会拿到同一端口（PIT-103 实证），
/// 计数器保证单进程内绝不重复；不同测试二进制=不同进程，cargo 串行跑 → 无冲突。
pub fn random_udp_port() -> u16 {
    static NEXT: std::sync::atomic::AtomicU16 = std::sync::atomic::AtomicU16::new(31000);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}
