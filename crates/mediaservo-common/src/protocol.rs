//! Signaling protocol message types.
//!
//! All signaling messages flow through the Server's WebSocket /ws endpoint
//! as JSON. Server relays messages between Host and Remote without modification
//! (except for room management messages).

use serde::{Deserialize, Serialize};

// ── S0: 协议版本协商（不变式 I5：能力门只看协商后的整数；包版本/协议整数/schema·ABI
// 三平面分离）。v1 = wire 无 protocol 字段（老形逐字节不变）；v2 = datachannel.control
// 域（F8 控制 DC 门）开放；v3 = 会话续期（S0.5：resume/session_nonce——整数单调性是
// 「方言精确」的载体：resume 必须占代际，否则同一方言号两套语义）。
/// 本仓端点支持的最高方言。
pub const SIGNALING_PROTOCOL_VERSION: u32 = 3;
/// 可接受连接的最低方言（不声明 protocol 的旧客户端 = v1；低于此值 → Error 4101）。
pub const SIGNALING_PROTOCOL_MIN_SUPPORTED: u32 = 1;
/// 开控制 DC 域（create_data_producer）所需最低方言。
pub const PROTOCOL_MIN_CONTROL_DC: u32 = 2;
/// 会话续期（resume 请求被受理）所需最低方言。
pub const PROTOCOL_MIN_RESUME: u32 = 3;

/// 协商结果 = min(客户端声明（None = v1），server 最高)。拒低形态由调用方先行
/// （claim < MIN_SUPPORTED → 4101），此处只收敛。
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
        /// G4 设备凭证（additive，D-H11）：携带时 server 走设备认证（G2 起校验）；
        /// 缺省 = PSK 认证路径。旧 server 忽略未知字段、旧 client 不带字段 —— 双向兼容。
        #[serde(skip_serializing_if = "Option::is_none")]
        device_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        device_secret: Option<String>,
        /// device-enroll 公钥准入（additive，D-E3 与 secret 共存一周期）：base64 32B Ed25519
        /// verifying key，与 device_id 同现同缺；携带时触发 server 验签挑战链（design §5.2）。
        /// 缺省 = 旧 secret/PSK 路径逐字节不变；旧 server 忽略未知字段 = 不劣化。
        #[serde(skip_serializing_if = "Option::is_none")]
        device_pubkey: Option<String>,
        /// S0 协议协商（additive）：缺省 = v1（2026-09-14 前 wire 逐字节不变，
        /// 旧 server/旧 host 双向无感——升级顺序无约束）。server 按 min(claim, max) 入会话。
        #[serde(skip_serializing_if = "Option::is_none")]
        protocol: Option<u32>,
        /// 观测位：客户端自身版本串，不参与任何门控判定。
        #[serde(skip_serializing_if = "Option::is_none")]
        client_version: Option<String>,
        /// a2 会话续期（additive，仅 negotiated≥3 被消费）：携带上次下发的 session_nonce
        /// 作重挂索引——**不是凭证**，认证链照常重跑（D283 吊销即刻生效）。缺省 = 全量 join。
        #[serde(skip_serializing_if = "Option::is_none")]
        resume: Option<String>,
    },

    /// Room join acknowledged by Server.
    RoomJoined {
        room_id: String,
        peer_id: String,
        /// S0：server 回显谈成值（缺省 = 旧 server，端按 v1）。
        #[serde(skip_serializing_if = "Option::is_none")]
        protocol: Option<u32>,
        /// 观测位：server 版本串。
        #[serde(skip_serializing_if = "Option::is_none")]
        server_version: Option<String>,
        /// a2：一次性重挂票（≥32B CSPRNG base64；仅 negotiated≥3 下发）。只索引重挂会话
        /// 快照——不绑源 IP（换 IP = 设计场景），即发即用、消费即焚。
        #[serde(skip_serializing_if = "Option::is_none")]
        session_nonce: Option<String>,
    },

    /// A peer has left the room. Broadcast by Server.
    RoomLeave {
        room_id: String,
        peer_id: String,
    },

    /// SDP offer/answer relayed through Server.
    Sdp {
        room_id: String,
        target: Option<String>,
        sdp: String,
    },

    /// ICE candidate relayed through Server.
    /// PIT-106 (I2 review): `alias = "rtc_ice_candidate"` — 浏览器 W3C 惯例 wire 名
    /// （sfu-client.ts 发送）; 规范名 r_t_c_ice_candidate 保持向后兼容（host/Rust 客户端）。
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
    CreateWebRtcTransport {
        room_id: String,
        peer_id: String,
        direction: TransportDirection,
    },

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
        /// P1 (client-dual-form): transport 级 SCTP 参数（mediasoup SctpParameters 序列化
        /// 原样透传；None = 向后兼容）。opaque Value 循 rtp_parameters 先例——浏览器侧
        /// 字段映射（OS/MIS/maxMessageSize）归 TS proto handler 单点。
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

    /// P1 (client-dual-form): 客户端请求房间 Router 的 RTP capabilities —
    /// mediasoup-client Device.load() 的输入（C18 官方协商流程第一步）。
    GetRouterRtpCapabilities {
        room_id: String,
    },

    /// Server 回 Router RTP capabilities（mediasoup RtpCapabilitiesFinalized 序列化
    /// 原样透传 — opaque Value 循 rtp_parameters 先例，禁手拼）。
    RouterRtpCapabilities {
        room_id: String,
        capabilities: serde_json::Value,
    },

    /// P1: consumer 层级偏好（simulcast/SVC 选层 — 会议 P4a 带宽自适应的服务端入口）。
    /// ack 循 Error{code:0} 先例（transport_connected 同型）。
    SetPreferredLayers {
        room_id: String,
        peer_id: String,
        consumer_id: String,
        spatial_layer: u8,
        #[serde(skip_serializing_if = "Option::is_none")]
        temporal_layer: Option<u8>,
    },

    /// Error response from Server.
    Error {
        code: u16,
        message: String,
    },

    /// Encoded media frame relayed through Server.
    /// data_base64 is encoded as base64 (JSON-safe).
    Frame {
        room_id: String,
        codec: String,
        sequence: u64,
        is_keyframe: bool,
        data_base64: String,
    },

    // ── SFU produce/consume (mediasoup) ─────────────────────────

    /// Peer asks to produce media on its send transport.
    /// rtp_parameters is opaque JSON — server passes it through to mediasoup.
    Produce {
        room_id: String,
        peer_id: String,
        transport_direction: TransportDirection,
        kind: MediaKind,
        rtp_parameters: serde_json::Value,
        /// C1 (D-H14 顺序无关): 绑定目标 send transport（CreateWebRtcTransport 返回的
        /// transport_id）。None = legacy 单槽回退（最近创建的 send transport）。
        #[serde(skip_serializing_if = "Option::is_none")]
        transport_id: Option<String>,
    },

    /// Server confirms producer created.
    Produced {
        room_id: String,
        producer_id: String,
    },

    /// Server broadcasts a new producer to all peers in the room.
    NewProducer {
        room_id: String,
        producer_id: String,
        peer_id: String,
        kind: MediaKind,
    },

    /// v2 (web-stream-stats T1): Host 周期上报编码状态（room 广播 relay 到浏览器）。
    /// encoder_implementation: libwebrtc outbound-rtp 实际编码器名（软编/硬编识别）; None = 不可用。
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
        /// v3 (encode-time-stats T1): 平均每帧编码耗时（ms/帧, Host 增量计算）—
        /// ΔtotalEncodeTime / ΔframesEncoded × 1000; None = 不可用（旧 host / 首周期）。
        #[serde(skip_serializing_if = "Option::is_none")]
        avg_encode_ms: Option<f64>,
    },


    /// v4 (E3 host-multiprocess): host-agent 整车状态上报 — 拓扑 + 数据流 + 信令
    /// 三快照聚合，经网关远端 WS 周期上报（默认 5s）。Server 直接消费存储
    /// （非 relay 消息，不广播房间；旧 Server 解析失败静默丢弃 = 可容忍，
    /// 周期性上报下一周期自愈）。
    StatusReport {
        room_id: String,
        /// 数据面: 各 camera topic 数据流统计（E2 快照）。
        topics: Vec<TopicFlowJson>,
        /// 数据面: 各 streamer 推流状态（E2 快照）。
        streams: Vec<StreamFlowJson>,
        /// 拓扑面: 期望 + 实际进程并集（E1 快照）。
        processes: Vec<ProcessStateJson>,
        /// 信令面: 网关视角连接状态（E3 快照）。
        signal: SignalStatusJson,
        /// 上报时刻（unix 秒）。
        ts: u64,
        /// host.toml 配置版本（E4 ConfigPush 关联；当前恒 0）。
        config_version: u64,
    },

    /// Peer asks to consume a producer on its recv transport.
    /// rtp_capabilities is opaque JSON — server passes it through to mediasoup.
    Consume {
        room_id: String,
        peer_id: String,
        producer_id: String,
        rtp_capabilities: serde_json::Value,
        /// C1: 绑定目标 recv transport。None = legacy 单槽回退（最近创建的 recv transport）
        /// ——修复多连接共享 peer_id 时 recv_transport 互相覆盖 → consumer 挂错 transport 黑屏。
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
    /// v5 (E4 云端配置闭环): Server → host-agent 整车配置下发。
    /// config 为 host.toml 全文；target = 整车 peer_id（房间内其他 peer 忽略）；
    /// version 与 StatusReport.config_version 关联（agent 应用成功后回报）。
    ConfigPush {
        room_id: String,
        target: String,
        config: String,
        version: u64,
    },
    /// G3 急停命令（舱端 → server → 车端房间）: 服务端强审计路径（D-H11 急停强审计）。
    /// 底盘/云台常规控制在 P2P DC（协商期已按角色授权，服务端不可见）; 急停必须
    /// 留痕（谁/何时/哪个车/什么命令）→ 经信令转发，host-agent 收下后转本地控制器。
    /// command 语义: 车端约定（如 "e-stop"），server 只透传 + 审计。
    EmergencyCommand {
        room_id: String,
        command: String,
    },

    /// H1 (SFU data 域): 对端在 send transport 上创建 DataProducer（SCTP DataChannel）。
    /// sctp_stream_parameters 为必填（SCTP producer 需要 stream_id; Option 仅为 wire 容错，
    /// 服务端缺失时明确报错）。label/protocol 用于区分 control/vision 等 DataChannel。
    CreateDataProducer {
        room_id: String,
        peer_id: String,
        transport_direction: TransportDirection,
        /// DataChannel label（如 "control" / "vision"）。
        label: String,
        /// DataChannel 子协议名（如 "mediaservo.control"）。
        protocol: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        sctp_stream_parameters: Option<SctpStreamParameters>,
        /// C1: 绑定目标 send transport。None = legacy 单槽回退。
        #[serde(skip_serializing_if = "Option::is_none")]
        transport_id: Option<String>,
    },

    /// Server confirms data producer created.
    DataProducerCreated {
        room_id: String,
        data_producer_id: String,
    },

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
        /// C1: 绑定目标 recv transport。None = legacy 单槽回退。
        #[serde(skip_serializing_if = "Option::is_none")]
        transport_id: Option<String>,
    },

    /// Server confirms data consumer created.
    /// S2d：官方契约（mediasoup-client Chrome74.receiveDataChannel）= consumer 侧必须以
    /// negotiated DC（id=streamId, 带外协商）建通道接收 worker 转发消息——DCEP 带内
    /// 握手 worker 从不代发。故回执携带 consumer 的 sctp 参数/label/protocol。
    /// Option+default 保旧 wire 可读（缺字段 = 老 server；新 client 明确报错不静默降级）。
    DataConsumed {
        room_id: String,
        data_consumer_id: String,
        data_producer_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")] // S2d: 老 server 缺省兼容
        sctp_stream_parameters: Option<SctpStreamParameters>,
        #[serde(default)] // S2d: consumer DC label（建通道用）
        label: String,
        #[serde(default)] // S2d: consumer DC sub-protocol
        protocol: String,
    },

    /// S4/a4：急停 WS 审计副本（舱端 send_estop 与 DC 快路径同发，best-effort）。
    /// 审计主落点 = host 执行器 actuation log（DC 不经 server，副本可丢——PLAN §11.6
    /// 席3 裁决）；server 侧仅落 audit 环 + WARN 留痕，不转发不裁决执行。
    ControlAudit {
        room_id: String,
        seq: u64,
        cmd: String,
        /// 签名存在位（载荷不复制——审计体积与 payload 隐私）。
        sig_present: bool,
    },

    /// H2 (audio conference): 查询 SFU producer/consumer RTP 统计（媒体面证据 + 运维观测）。
    /// 携带 producer_id 或 consumer_id（任一）；server 回复 SfuStats。
    SfuStatsRequest {
        producer_id: Option<String>,
        consumer_id: Option<String>,
    },

    /// H2: SfuStatsRequest 的响应 — RTP 接收字节/包计数（mediasoup get_stats）。
    /// byte_count/packet_count: producer = 收到的入站 RTP; consumer = 路由转发的出站 RTP。
    SfuStats {
        producer_id: Option<String>,
        consumer_id: Option<String>,
        kind: Option<MediaKind>,
        byte_count: u64,
        packet_count: u64,
        score: u8,
    },
    /// H1 (session-recovery T1): producer 死亡通知（房间级广播）——peer WS 关闭时 server 广播。
    /// web 收到后停旧 track 并等新轮 new_producer 自动重订（免刷新自愈）。
    /// reason 当前唯一来源 = peer_disconnected；producer 中途死检测依赖 worker 通知（H3 不可靠，本期不做）。
    ProducerClosed {
        room_id: String,
        peer_id: String,
        producer_id: String,
        kind: MediaKind,
        #[serde(skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    /// F1/T4: 网关上报下游子会话消亡（单 streamer 退出/crash——整车会话不断，
    /// agent 断开反查只覆盖整车粒度，此消息提供流粒度）。server 按 (room_id, peer_id)
    /// 精确清理并广播 ProducerClosed。additive：旧 server 解析失败静默丢弃 = 与无前行为一致。
    DownstreamGone {
        /// 子进程自报 peer 键（= SFU 层 producer 宿主键，视频流常为字面 "host"）。
        peer_id: String,
        /// 子进程 RoomJoin 的流子房间。
        room_id: String,
    },

    /// device-enroll §2 (server→host): 验签挑战。nonce = base64 32B 随机 (OsRng)；
    /// 每连接 0..1 次，仅对带 device_pubkey 的 RoomJoin 发出（secret 形/PSK 路径不发）。
    DeviceAuthChallenge {
        nonce: String,
    },

    /// device-enroll §2 (host→server): 挑战应答。
    /// 字节合同: sig = base64( Ed25519::sign( nonce_raw(32B) ‖ device_id ‖ room_id ) )，
    /// 钉死测试见 server devices.rs::sig_vector（host 侧批3 交叉复验同向量，D-E7 绑房防跨用）。
    DeviceAuthResponse {
        room_id: String,
        sig: String,
    },

    /// device-enroll §2 (server→host): 手动档验签过但未批准（入 pending）。
    /// 设备沿用既有 retry 退避重连等待批准（日志「待管理员批准」）。
    DeviceAuthPending {
        device_id: String,
    },

    // ponytail: add frame ack/retransmit when reliability matters
}


/// E3 状态上报: 单 topic 数据流统计（wire 版，镜像 host monitor::flow::TopicFlow）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TopicFlowJson {
    pub topic: String,
    /// 窗口内帧率（<2 帧或窗口为零 → 0）。
    pub fps: f64,
    /// 窗口内字节率。
    pub bps: u64,
    /// 最近一帧发布端单调时间戳（ns；从未收到 → 0）。
    pub last_ts_mono_ns: u64,
    /// 窗口内收到帧数。
    pub frames: u64,
    /// 停滞（距最近到达超阈值；从未收到帧也视为停滞）。
    pub stalled: bool,
}

/// E3 状态上报: 单流推流状态（wire 版，镜像 host monitor::flow::StreamFlow）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StreamFlowJson {
    pub id: String,
    /// 最近一次 stats 的 bytes_sent（webrtc OutboundRtp，累计）。
    pub bytes_sent: u64,
    /// 最近一次 stats 的 frames_encoded（libwebrtc u32，累计）。
    pub frames_encoded: u32,
    pub frame_width: u32,
    pub frame_height: u32,
    /// 最近 stats 是否在新鲜窗口内。
    pub connected: bool,
}

/// E3 状态上报: 单进程拓扑状态（期望 + 实际并集；running = oxmgr running）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProcessStateJson {
    pub name: String,
    pub running: bool,
    /// host.toml 期望进程（实际发现的非期望进程 = false）。
    pub expected: bool,
}

/// E3 状态上报: 信令平面（网关视角）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SignalStatusJson {
    /// 远端 server WS 是否已连接并入房。
    pub remote_connected: bool,
    /// 本次远端会话建立至今秒数（未连接 = None）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_since_secs: Option<u64>,
    /// 整车 peer_id（未连接 = 空串）。
    pub remote_peer_id: String,
    /// 本地子进程 WS 连接列表。
    pub children: Vec<ChildSignalJson>,
    /// host-agent 启动至今秒数。
    pub agent_uptime_secs: u64,
}

/// E3 状态上报: 单子进程 WS 连接。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChildSignalJson {
    /// 子进程标识（LocalEnvelope.src）。
    pub src: String,
    /// 连接中（快照仅含在途连接，恒 true；字段保留供 H 阶段渲染）。
    pub connected: bool,
    /// 距最近一条上行消息的秒数（0 = 刚收到；u64::MAX = 未发过消息）。
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
    /// S4′: SCTP DataProducer（ProducerClosed 广播用；produce wire 面不出现此值）。
    Data,
}

/// H1 (SFU data 域): SCTP stream parameters for a DataChannel (wire 版，
/// 镜像 mediasoup SctpStreamParameters — 官方文档 sctp-parameters 节)。
/// camelCase 序列化（JS/mediasoup 惯例: streamId/ordered/maxPacketLifeTime/maxRetransmits）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SctpStreamParameters {
    /// SCTP stream id（端点 DataChannel 的 negotiated id）。
    pub stream_id: u16,
    /// 有序可靠传输（true = ordered; false 时可选 maxPacketLifeTime/maxRetransmits）。
    pub ordered: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_packet_life_time: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_retransmits: Option<u16>,
}

/// 控制请求信封（client-dual-form T1.3 自 mediaservo-host 提仓——四方单一真源：
/// host-controller/host-emergency/TS 镜像/夹具共用 `{seq, cmd, payload}` 现网活形）。
/// 通道边界 = DC label（chassis/gimbal/light），信封内不重复携带通道名；
/// `seq` 发送方单调递增，回执 `ack` 原样回传配对（D-H3 语义）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlEnvelope {
    /// 发送方单调递增序号（回执配对用）。
    pub seq: u64,
    /// 执行器命令（如 "steer"/"pan"/"on"；语义由执行器实现定义）。
    pub cmd: String,
    /// 命令参数（缺省 = 空对象）。
    #[serde(default = "default_control_payload")]
    pub payload: serde_json::Value,
    /// S4/T3.5(R-B HMAC 方案)：e-stop 类命令必带签名——
    /// `hex(HMAC-SHA256(key, canonical_envelope_json))`；key=部署预共享
    /// （env `MEDIASERVO_CONTROL_HMAC_KEY`，车舱双侧配置一致）。
    /// additive：旧端 serde 忽略（其 estop 会被新车端拒 = 版本墙非破坏）。
    /// 威胁模型注记：HMAC 共享密钥 = 无不可否认性（中间形态），
    /// Ed25519/PKI 签名面归 P3′ 威胁模型裁决（PLAN §2 P3′ 行）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sig: Option<String>,
}

/// e-stop 命令族判定（前缀合同：`estop` / `estop_*` / `estop-*`）。
#[must_use]
pub fn is_estop_cmd(cmd: &str) -> bool {
    cmd == "estop" || cmd.starts_with("estop_") || cmd.starts_with("estop-")
}

/// S4/T3.5 HMAC 签名的 canonical 消息 = 不含 sig 的信封 JSON
/// （发送/校验两侧同一函数构造 = 字节稳定；sig=None 序列化即无该字段）。
fn canonical_envelope_bytes(env: &ControlEnvelope) -> Vec<u8> {
    let base = ControlEnvelope {
        seq: env.seq,
        cmd: env.cmd.clone(),
        payload: env.payload.clone(),
        sig: None,
    };
    serde_json::to_vec(&base).expect("ControlEnvelope is infallible to serialize")
}

/// `hex(HMAC-SHA256(key, canonical))`（车舱共用构造函数）。
/// 急停 HMAC 密钥文件读取（G13 文件通道的**双端共用真源**：舱端 client-c
/// `hmac_key_file` 与车端 `MEDIASERVO_CONTROL_HMAC_KEY_FILE` 同纪律）。
/// 权限门 0600（组/他可读即拒——弱文件权限=密钥泄露面）、尾换行剥离、非空、UTF-8。
pub fn control_hmac_key_from_file(path: &str) -> Result<String, String> {
    use std::os::unix::fs::PermissionsExt;
    let meta = std::fs::metadata(path).map_err(|e| format!("hmac key file {path}: {e}"))?;
    let mode = meta.permissions().mode() & 0o777;
    if mode & 0o077 != 0 {
        return Err(format!(
            "hmac key file {path}: mode {mode:04o} 过宽（须 0600 或更严，G13）"
        ));
    }
    let mut bytes = std::fs::read(path).map_err(|e| format!("hmac key file {path}: {e}"))?;
    while matches!(bytes.last(), Some(b'\n') | Some(b'\r')) {
        bytes.pop();
    }
    if bytes.is_empty() {
        return Err(format!("hmac key file {path}: 空密钥"));
    }
    String::from_utf8(bytes).map_err(|_| format!("hmac key file {path}: 非 UTF-8 密钥"))
}

pub fn control_hmac_sign(key: &str, env: &ControlEnvelope) -> String {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    let mut mac = Hmac::<Sha256>::new_from_slice(key.as_bytes())
        .expect("hmac accepts any key length");
    mac.update(&canonical_envelope_bytes(env));
    // 全 32B → 64 hex（遥控语义足够；DC 帧预算友好）。
    mac
        .finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// 恒时校验（hmac `verify_slice` = 内部 constant-time；hex 解码手写免增依赖）。
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
    let mut mac = Hmac::<Sha256>::new_from_slice(key.as_bytes())
        .expect("hmac accepts any key length");
    mac.update(&canonical_envelope_bytes(env));
    mac.verify_slice(&given).is_ok()
}

/// 控制回执（与请求同通道发回）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlAck {
    /// 回执对应的请求 seq。
    pub ack: u64,
    /// 执行器结果；失败时 `{"error": "<原因>"}`。
    pub result: serde_json::Value,
}

impl ControlAck {
    pub fn ok(seq: u64, result: serde_json::Value) -> Self {
        Self { ack: seq, result }
    }

    pub fn err(seq: u64, message: impl Into<String>) -> Self {
        Self {
            ack: seq,
            result: serde_json::json!({ "error": message.into() }),
        }
    }
}

/// 从 DC 字节解析请求信封。
pub fn parse_envelope(data: &[u8]) -> Result<ControlEnvelope, serde_json::Error> {
    serde_json::from_slice(data)
}

/// 缺省 payload = 空对象（`Value::default()` 是 Null，语义不符）。
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
        // stream_id/device 字段 None 时不出现在 JSON（additive 契约：旧 server 无感知）
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
            SignalingMessage::RoomJoin { room_id, peer_role, stream_id, device_id, device_secret, .. } => {
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
            // seed=bytes(0..32) 派生的真 vk，与 devices.rs sig_vector 测试同值（跨 crate 锚）
            device_pubkey: Some("A6EHv/POEL4dcN0Y50vAmWfk1jCbpQ1fHdyGZBJVMbg=".into()),
            protocol: None,
            client_version: None,
            resume: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""device_pubkey":"A6EHv/POEL4dcN0Y50vAmWfk1jCbpQ1fHdyGZBJVMbg=""#));
        // pubkey 形与 secret 互斥：secret None 不出 wire
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
        // additive 钉①：None → JSON 不含 device_pubkey 键（旧 server 无感知）
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

        // additive 钉②：旧 wire（缺键 JSON）照 parse = 新 server 兼容旧 host（升级序先 server 后 host）
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
                SignalingMessage::DeviceAuthPending {
                    device_id: "ms-0a1b2c3d4e5f".into(),
                },
                r#""type":"device_auth_pending""#,
            ),
        ];
        for (msg, wire_tag) in cases {
            let json = serde_json::to_string(&msg).unwrap();
            assert!(json.contains(wire_tag), "{json}");
            let reparsed = serde_json::to_string(&serde_json::from_str::<SignalingMessage>(&json).unwrap()).unwrap();
            assert_eq!(reparsed, json);
        }
    }

    #[test]
    fn serialize_error() {
        let msg = SignalingMessage::Error {
            code: 4003,
            message: "PSK authentication failed".into(),
        };
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
        // S0 硬门①：v1 wire（无 protocol/client_version）序列化前后逐字节不变——
        // 旧端点行为等价是 additive 承诺的全部含义。
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
        // S0 硬门②：协商纯函数矩阵（缺省=1 / 同代=2 / 超宣钳 server max）。
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
        let msg = SignalingMessage::RoomLeave {
            room_id: "room-99".into(),
            peer_id: "peer-3".into(),
        };
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
        // F1/T4 additive 契约钉住：snake_case type 标签 + 两字段
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
        let msg = SignalingMessage::Sdp {
            room_id: "room-x".into(),
            target: None,
            sdp: "v=0".into(),
        };
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
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"produce""#));
        assert!(json.contains("transport-9"), "transport_id 必须上 wire: {json}");
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
        let msg = SignalingMessage::Produced {
            room_id: "room-1".into(),
            producer_id: "prod-1".into(),
        };
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
        assert!(matches!(
            parsed,
            SignalingMessage::ProducerClosed { reason: Some(_), .. }
        ));
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

    /// C1: transport_id 缺省（legacy 客户端 wire 无该字段）→ 解析为 None（向后兼容）；
    /// None 不序列化该字段（旧 server 也可解析）。
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
        // 反向: None 不序列化该字段（legacy server 也可解析）
        let out = serde_json::to_string(&parsed).unwrap();
        assert!(!out.contains("transport_id"), "None transport_id 不得上 wire: {out}");
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
                ProcessStateJson { name: "host-capturer-cam0".into(), running: false, expected: true },
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

    // ── H1 SFU data 域 ────────────────────────────────────────────────

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
        let parsed2: SctpStreamParameters = serde_json::from_str(&serde_json::to_string(&sp2).unwrap()).unwrap();
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
        assert!(json.contains("transport-9"), "transport_id 必须上 wire: {json}");
        let parsed: SignalingMessage = serde_json::from_str(&json).unwrap();
        match parsed {
            SignalingMessage::CreateDataProducer { room_id, label, protocol, sctp_stream_parameters, transport_id, .. } => {
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
            SignalingMessage::CreateDataProducer { room_id, label, protocol, sctp_stream_parameters, .. } => {
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
        assert!(!json.contains("transport_id"), "None transport_id 不得上 wire: {json}");
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
        // 旧线形（无 sig 字段）必须可读；新发送带 sig 也回读对称。
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
        assert!(control_hmac_key_from_file(p.to_str().unwrap()).unwrap_err().contains("过宽"));
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

    /// S4′: ProducerClosed 的 data kind wire 值钉（TS 媒体面过滤依赖 "data" 字面）。
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
        assert!(matches!(
            back,
            SignalingMessage::ProducerClosed {
                kind: MediaKind::Data,
                ..
            }
        ));
    }

    /// S2d 兼容钉：老 server 的 wire（无 sctpStreamParameters/label/protocol）必须可解析。
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

    /// PIT-106 (I2 review): 浏览器 W3C 惯例发 `rtc_ice_candidate`（sfu-client.ts:236），
    /// 规范 wire 名是 `r_t_c_ice_candidate`（serde snake_case）→ 服务端 alias 兼容。
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

    // ── P1 (client-dual-form): mediasoup-client 标准协商面 ──────────────────

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
        // temporal_layer None → 不上 wire（additive 容错）
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
        // None → 字段不上 wire；缺字段 wire → 解析 None（双向兼容钉）
        let json = serde_json::to_string(&make(None)).unwrap();
        assert!(!json.contains("sctp_parameters"));
        let parsed: SignalingMessage = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, SignalingMessage::WebRtcTransportCreated { sctp_parameters: None, .. }));
        // Some → 透传
        let json_with = serde_json::to_string(&make(Some(serde_json::json!({ "port": 5000 })))).unwrap();
        assert!(json_with.contains(r#""sctp_parameters":{"port":5000}"#));
    }
}
