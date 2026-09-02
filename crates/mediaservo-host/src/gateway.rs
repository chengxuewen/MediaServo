//! host-agent 信令网关（Task D1，D-H6「一车一会话」核心）。
//!
//! 拓扑：
//! ```text
//! 子进程 (host-streamer 等) ──WS {src, msg}──▶ host-agent ◀──WS 纯 SignalingMessage── Server
//! ```
//!
//! 协议语义（Momus HIGH-1，实证于 signaling.rs）：
//! - (a) **RoomJoin 拦截**：各子进程的 RoomJoin 由 agent 拦截（server 只在建连阶段
//!   处理 RoomJoin，relay 循环内再收会被静默丢弃）；agent 以整车身份单次 join，
//!   子进程本地合成 `RoomJoined`（携带整车 peer_id）。
//! - (b) **响应路由**：SFU 消息（CreateWebRtcTransport/ConnectWebRtcTransport/
//!   Produce/Consume）每请求恰好一响应，server 单 WS 严格顺序处理（forward 循环
//!   await `handle_sfu_message` 后才读下一条）→ agent 维护**待决请求 FIFO**
//!   （conn id），响应到达即弹出匹配发起者。并发协商正确（与"按序复用"拒绝项不同
//!   ——不串行化协商，只按序配对在途请求）。FIFO 的三个边界不变量（D1 审查修复）：
//!   ① joined 检查先于 push（断线窗口 5001 不留陈旧槽）；② push 与远端 send 同一
//!   临界区（wire 顺序 ≡ pending 顺序）；③ 断连连接不位移队列（mark-dead 严格
//!   FIFO 消费——死槽被其自身响应弹出丢弃）。P2P relay 的 Sdp/RTCIceCandidate 无
//!   transport 标识 → 按**协商归属**追踪（最后一个上行 Sdp/ICE 的本地连接；
//!   Frame/EncoderStatus 等媒体/状态消息不更新归属），单协商串行语义。
//! - (c) 拒绝项均未实现："远端 from 前缀"（破坏 Server 零改动）/"按序复用"（并发
//!   协商必错乱）。
//!
//! 额外语义：
//! - **房间重写**：子进程消息 room_id → 整车房间上行；下行按目标子进程自己声明
//!   的房间改写。多 streamer（各自 `stream-<id>` 房间）聚合进同一整车会话。
//! - **回显去重**：server 将 relay 白名单消息（Sdp/ICE/Frame/EncoderStatus）广播给
//!   房间全员（含发送者自身）→ agent 按文本匹配丢弃自己转发的回显。
//! - **子进程 RoomLeave 拦截**：单进程 leave 若上行会让 server 断开整车会话。
//! - **断线**：远端断开 → 清空待决 FIFO/协商归属/回显缓存（server 已回收全部
//!   transport），本地连接保持，B1 重连后转发恢复。

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use mediaservo_common::protocol::{PeerRole, SignalingMessage};
use mediaservo_link::{RetryConfig, SignalClient, SignalEvent, SignalSession};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;

/// 本地协议信封：`{src, msg}` — 仅本地 wire；远端为纯 SignalingMessage（零改动）。
/// 类型定义在 mediaservo-link（D2 I1: 单一来源，防 wire 漂移——子进程经 field 复用同型）。
pub use mediaservo_link::signal::LocalEnvelope;

/// 网关配置。
#[derive(Debug, Clone)]
pub struct GatewayConfig {
    /// 本地监听端口（0 = 临时端口，测试用）。
    pub local_port: u16,
    /// 远端 Server WS 地址（SignalClient 直连）。
    pub remote_url: String,
    /// PSK（每连接重认证，B1 语义）。
    pub psk: String,
    /// G4 设备凭证（D-H11）：Some = 远端 Join 携带 device_id/device_secret（additive）；
    /// None = PSK 路径（G2 切换校验前保持）。host-agent 从 identity.json 加载。
    pub device: Option<mediaservo_link::DeviceCredential>,
    /// 整车房间（agent 单次 join 的房间）。
    pub room: String,
    /// 远端连接重试配置（断线重连复用）。
    pub retry: RetryConfig,
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            local_port: DEFAULT_LOCAL_PORT,
            remote_url: "ws://127.0.0.1:9800/ws".into(),
            psk: "mediaservo-dev".into(),
            device: None,
            // I1 review (D3 TODO 关闭): 整车房间由 host.yaml [signaling] room 配置 —
            // translate 转译为 agent --room；此处为 CLI 缺省（host-agent 内置默认）。
            room: "vehicle".into(),
            retry: RetryConfig::default(),
        }
    }
}

/// 默认本地端口（host.yaml [signaling] local_port 可覆盖，translate 传 --port）。
pub const DEFAULT_LOCAL_PORT: u16 = 17980;
/// 网关未连上远端 server 时对子进程的应答码（子进程可重试）。
const ERR_GATEWAY_DISCONNECTED: u16 = 5001;
/// 回显缓存容量（最近转发的 relay 消息文本，FIFO）。
const ECHO_CACHE_CAP: usize = 256;

/// 单个本地子进程连接。
#[derive(Clone)]
struct Conn {
    id: u64,
    /// 子进程 RoomJoin 声明（拦截时记录；下行响应房间改写目标）。
    room: String,
    /// 子进程标识（LocalEnvelope.src，最近一条上行消息的声明；E3 快照数据源）。
    src: String,
    /// 最近一条上行消息到达时刻（E3 快照数据源；None = 尚未收到消息）。
    last_msg: Option<Instant>,
    tx: mpsc::UnboundedSender<LocalEnvelope>,
}

/// 路由状态（两个后台任务共享，短临界区 std Mutex 足够）。
struct State {
    conns: HashMap<u64, Conn>,
    next_id: u64,
    /// 待决 SFU 请求 FIFO（conn id；响应按序弹出匹配）。
    pending: VecDeque<u64>,
    /// 最近转发的 relay 消息文本（回显去重）。
    echo_cache: VecDeque<String>,
    /// 整车 peer_id（真实 RoomJoined 取得，子进程合成应答使用）。
    vehicle_peer_id: String,
    /// 远端会话是否在途。
    joined: bool,
    /// 本次远端会话建立时刻（E3 快照数据源；reset_remote 清空）。
    remote_since: Option<Instant>,
    /// 整车房间（不可变）。
    vehicle_room: String,
    /// 待应用 ConfigPush（E4：server 云端配置下发；agent 轮询消费，最新覆盖旧值）。
    pending_config: Option<SignalingMessage>,
}

impl State {
    /// 子进程上行消息 → 动作（转发 / 本地应答 / 丢弃）。
    fn upstream(&mut self, conn_id: u64, msg: SignalingMessage) -> UpstreamAction {
        match msg {
            SignalingMessage::RoomJoin { room_id, .. } => {
                // (a) 拦截：整车 join 由 agent 完成；子进程本地合成 RoomJoined
                if let Some(c) = self.conns.get_mut(&conn_id) {
                    c.room = room_id.clone();
                }
                if self.joined {
                    UpstreamAction::Reply(SignalingMessage::RoomJoined {
                        room_id,
                        peer_id: self.vehicle_peer_id.clone(),
                    })
                } else {
                    tracing::warn!(conn_id, "RoomJoin 拦截时网关尚未连上 server");
                    UpstreamAction::Reply(SignalingMessage::Error {
                        code: ERR_GATEWAY_DISCONNECTED,
                        message: "gateway not connected to server".into(),
                    })
                }
            }
            SignalingMessage::RoomLeave { .. } => {
                // 单进程 leave 上行会让 server 断开整车会话 — 拦截丢弃
                tracing::warn!(conn_id, "子进程 RoomLeave 被网关拦截（整车会话不因单进程离开而断）");
                UpstreamAction::Drop
            }
            // 流诊断消息（web-stream-stats host 侧补齐）：保留 streamer 声明的流子房间——
            // server 按消息内 room 路由（relay_target_room）才能到达浏览器消费者；若改写成
            // 整车房间则广播进浏览器不在的频道。无 pending 槽/无需 echo 缓存。
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
                // CRITICAL-1: joined 检查先于一切状态记录 — 断线窗口内的请求
                // 不得留下 pending 槽（否则重连后 [A陈旧, C真实] → C 的响应弹 A
                // → 串线；单客户端自愈掩盖了该缺陷）
                if !self.joined {
                    return UpstreamAction::Reply(SignalingMessage::Error {
                        code: ERR_GATEWAY_DISCONNECTED,
                        message: "gateway not connected to server".into(),
                    });
                }
                if is_sfu_request(&m) {
                    self.pending.push_back(conn_id);
                }
                // 全 SFU（D 决策 2026-08-25）: 无 P2P 协商归属——Sdp 下行按房间路由
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

    /// server 下行消息 → 目标子进程列表（房间已按各目标改写）。
    fn downstream(&mut self, msg: SignalingMessage) -> Vec<(u64, SignalingMessage)> {
        // 回显去重：自己上行转发的 relay 消息被 server 房间广播回整车，文本一致即丢弃
        if is_relay_msg(&msg) {
            let text = serde_json::to_string(&msg).unwrap_or_default();
            if self.echo_cache.contains(&text) {
                return Vec::new();
            }
        }
        match msg {
            // SFU 响应：按 FIFO 匹配发起者（server 单 WS 顺序处理）
            SignalingMessage::WebRtcTransportCreated { .. }
            | SignalingMessage::Produced { .. }
            | SignalingMessage::Consumed { .. }
            | SignalingMessage::Error { .. }
            | SignalingMessage::SfuStats { .. } => { // H2: SfuStats 响应同 FIFO 路由
                let Some(conn_id) = self.pending.pop_front() else {
                    tracing::warn!("SFU 响应无对应待决请求，丢弃");
                    return Vec::new();
                };
                match self.conns.get(&conn_id) {
                    Some(conn) => {
                        let mut m = msg;
                        rewrite_room(&mut m, &conn.room);
                        vec![(conn_id, m)]
                    }
                    None => {
                        tracing::warn!("SFU 响应目标连接已断开");
                        Vec::new()
                    }
                }
            }
            // Sdp/ICE 下行：全 SFU 模式按房间路由（rewrite 后 room 一致的所有 conn）
            // — 多流并发协商各自房间互不干扰；无匹配丢弃（C15 日志）。
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
                    tracing::warn!(room = %target_room, "Sdp/ICE 下行无房间匹配连接，丢弃");
                }
                room_matches
            }
            // E4: 云端配置下发 — 整车 agent 专属消息（房间 + 目标 peer 匹配），
            // 不入子进程路由；存入待应用槽（agent 轮询 take_config_push）。
            SignalingMessage::ConfigPush { ref room_id, ref target, .. } => {
                if *room_id != self.vehicle_room || *target != self.vehicle_peer_id {
                    tracing::warn!(
                        room_id = %room_id,
                        target = %target,
                        vehicle_room = %self.vehicle_room,
                        vehicle_peer = %self.vehicle_peer_id,
                        "ConfigPush 目标不匹配，丢弃"
                    );
                    return Vec::new();
                }
                tracing::info!(room_id = %room_id, peer = %target, "ConfigPush 接收（待应用）");
                self.pending_config = Some(msg);
                Vec::new()
            }
            // 房间级广播（NewProducer/RoomLeave/RoomJoined 等）→ 全员
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
        // IMPORTANT-4: 不在 pending 中移除（mark-dead，严格 FIFO 消费）— 断连
        // 连接的请求可能仍在 server 在途，其响应必须按序弹出死槽丢弃；若移除
        // 槽位造成位移（[A,B] 移 A → [B]），A 的响应会弹走 B → 串线
    }

    /// 远端断线：清空一切与远端会话绑定的状态（server 已回收全部 transport）。
    fn reset_remote(&mut self) {
        self.joined = false;
        self.remote_since = None;
        self.pending.clear();
        self.echo_cache.clear();
        self.vehicle_peer_id.clear();
    }
}

enum UpstreamAction {
    /// 转发到远端（调用方负责 joined 检查与发送）。
    Forward(SignalingMessage),
    /// 直接回给该子进程。
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
            | SignalingMessage::SfuStatsRequest { .. } // H2: SFU 统计查询（FIFO 响应路由）
    )
}

/// server relay 白名单消息（signaling.rs 实证）——回显去重与 P2P 归属跟踪范围。
fn is_relay_msg(msg: &SignalingMessage) -> bool {
    matches!(
        msg,
        SignalingMessage::Sdp { .. }
            | SignalingMessage::RTCIceCandidate { .. }
            | SignalingMessage::Frame { .. }
            | SignalingMessage::EncoderStatus { .. }
    )
}

/// 消息自身的房间 id（无房间字段的变体返回 None）。
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
        | EmergencyCommand { room_id, .. } => Some(room_id),
        _ => None,
    }
}

/// room_id 改写（整车房间 ↔ 子进程房间；Error 无房间字段）。
fn rewrite_room(msg: &mut SignalingMessage, room: &str) {
    use SignalingMessage::*;
    // H2: 音频房间（audio-<vehicle>）消息不改写 — 判定**消息自身**的 room_id
    // （upstream 方向 room 参数 = 整车房间而非消息房间，用错值会把音频房间请求
    // 并入整车视频房间 — I3 review 网关探针实证 4031 未触发）。
    if msg_room_id(msg).is_some_and(|r| r.starts_with("audio-")) {
        return;
    }
    // multi-stream P3: per-stream 房间（<整车>_<stream>）直通——推流端按流隔离房间
    // （PIT-140 v2），网关不并入整车（与 audio- 同原则：消息自身 room_id 语义优先）。
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
        | DataConsumed { room_id, .. } => *room_id = room.to_string(),
        Error { .. } => {}
        StatusReport { .. } => {}
        // H2: 无房间字段的 SFU 统计消息 — 无需改写（下游按 FIFO 路由）
        SfuStatsRequest { .. } => {}
        SfuStats { .. } => {}
        ConfigPush { .. } => {} // E4: agent 专属，不入子进程路由，无需房间改写
        EmergencyCommand { room_id, .. } => *room_id = room.to_string(), // G3: 急停房间级广播
    }
}

/// 信令平面快照（E3 监控数据源；连接状态唯一持有者 = 网关）。
#[derive(Debug, Clone, Default)]
pub struct GatewayStatus {
    /// 本地子进程 WS 连接（按 src 排序，确定性）。
    pub children: Vec<ChildStatus>,
    /// 远端 server WS 连接状态。
    pub remote: RemoteStatus,
}

/// 单个本地子进程连接状态。
#[derive(Debug, Clone)]
pub struct ChildStatus {
    /// 子进程标识（LocalEnvelope.src；未发过消息 = 空串）。
    pub src: String,
    /// 连接中（快照仅含在途连接，恒 true）。
    pub connected: bool,
    /// 距最近一条上行消息的秒数（0 = 刚收到；u64::MAX = 未发过消息）。
    pub last_msg_secs: u64,
}

/// 远端 server WS 连接状态。
#[derive(Debug, Clone, Default)]
pub struct RemoteStatus {
    /// 已连接并入房。
    pub connected: bool,
    /// 本次远端会话建立至今秒数（未连接 = None）。
    pub since_secs: Option<u64>,
    /// 整车 peer_id（未连接 = 空串）。
    pub peer_id: String,
}

/// 网关运行期句柄（E3）— 监控快照 + 远端上报通道。
#[derive(Clone)]
pub struct GatewayHandle {
    state: Arc<Mutex<State>>,
    remote_tx: mpsc::UnboundedSender<SignalingMessage>,
}

impl GatewayHandle {
    /// 信令平面快照（E3 监控数据源）。
    pub fn snapshot(&self) -> GatewayStatus {
        let st = lock_state(&self.state);
        let now = Instant::now();
        let mut children: Vec<ChildStatus> = st
            .conns
            .values()
            .map(|c| ChildStatus {
                src: c.src.clone(),
                connected: true,
                last_msg_secs: c.last_msg.map_or(u64::MAX, |t| now.saturating_duration_since(t).as_secs()),
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

    /// 经网关远端 WS 发送（joined 检查 + 失败返回 Err —— 调用方必须打日志, C15）。
    /// 未 joined 时拒绝入通道：断线窗口的消息会被 remote_loop 的 drain 静默丢弃，
    /// 故在源头拦截。
    pub fn send_remote(&self, msg: SignalingMessage) -> Result<(), String> {
        let st = lock_state(&self.state);
        if !st.joined {
            return Err("gateway not connected to server".into());
        }
        self.remote_tx
            .send(msg)
            .map_err(|_| "remote session closed".into())
    }

    /// 取走待应用 ConfigPush（E4；None = 无待应用；最新覆盖旧值）。
    pub fn take_config_push(&self) -> Option<SignalingMessage> {
        lock_state(&self.state).pending_config.take()
    }

    /// 整车房间（上报 room_id 数据源）。
    pub fn vehicle_room(&self) -> String {
        lock_state(&self.state).vehicle_room.clone()
    }
}

fn lock_state(state: &Arc<Mutex<State>>) -> MutexGuard<'_, State> {
    state.lock().unwrap_or_else(|e| e.into_inner())
}

/// 信封 JSON 序列化（理论不可失败：SignalingMessage 全部字段可序列化；失败即丢弃并告警）。
fn env_json(env: &LocalEnvelope) -> Option<String> {
    match serde_json::to_string(env) {
        Ok(t) => Some(t),
        Err(e) => {
            tracing::error!("信封序列化失败: {e}");
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

/// 启动网关：绑定本地端口 + 常驻后台任务（本地 accept / 远端连接+重连）。
///
/// 返回实际绑定端口（`local_port = 0` 时为临时端口）。调用方持有运行期
/// （bin 等待信号；测试直接随 runtime 结束）。
/// 返回 (实际绑定端口, 运行期句柄)。调用方持有运行期
/// （bin 等待信号；测试直接随 runtime 结束）。
pub async fn run_gateway(config: GatewayConfig) -> Result<(u16, GatewayHandle), String> {
    let listener = TcpListener::bind(("127.0.0.1", config.local_port))
        .await
        .map_err(|e| format!("bind local gateway :{}: {e}", config.local_port))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("local_addr: {e}"))?
        .port();

    let state = Arc::new(Mutex::new(State {
        conns: HashMap::new(),
        next_id: 1,
        pending: VecDeque::new(),
        echo_cache: VecDeque::new(),
        vehicle_peer_id: String::new(),
        joined: false,
        remote_since: None,
        vehicle_room: config.room.clone(),
        pending_config: None,
    }));
    let (remote_tx, remote_rx) = mpsc::unbounded_channel::<SignalingMessage>();
    let handle = GatewayHandle { state: Arc::clone(&state), remote_tx: remote_tx.clone() };

    // 本地 accept 循环
    let accept_state = Arc::clone(&state);
    let accept_tx = remote_tx.clone();
    tokio::spawn(async move {
        loop {
            let (stream, peer) = match listener.accept().await {
                Ok(v) => v,
                Err(e) => {
                    tracing::error!("本地 accept 失败: {e}");
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    continue;
                }
            };
            let ws = match tokio_tungstenite::accept_async(stream).await {
                Ok(w) => w,
                Err(e) => {
                    tracing::warn!("本地 WS 握手失败 {peer}: {e}");
                    continue;
                }
            };
            let (conn_id, out_rx) = {
                let mut st = lock_state(&accept_state);
                let (tx, rx) = mpsc::unbounded_channel();
                let id = st.next_id;
                st.next_id += 1;
                st.conns.insert(id, Conn {
                    id,
                    room: String::new(),
                    src: String::new(),
                    last_msg: None,
                    tx,
                });
                (id, rx)
            };
            tracing::info!(conn_id, peer = %peer, "本地子进程接入");
            let st = Arc::clone(&accept_state);
            let rtx = accept_tx.clone();
            tokio::spawn(conn_task(conn_id, ws, out_rx, st, rtx));
        }
    });

    // 远端连接 + 重连循环
    tokio::spawn(remote_loop(config, state, remote_rx));

    Ok((port, handle))
}

/// 单本地连接任务：读信封 → 路由 → 转发/应答；写下发信封。
async fn conn_task(
    conn_id: u64,
    ws: WebSocketStream<TcpStream>,
    mut out_rx: mpsc::UnboundedReceiver<LocalEnvelope>,
    state: Arc<Mutex<State>>,
    remote_tx: mpsc::UnboundedSender<SignalingMessage>,
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
                                tracing::warn!(conn_id, "本地信封解析失败: {e}");
                                continue;
                            }
                        };
                        // CRITICAL-2: push（upstream 内）与 send（此处）必须在同一
                        // 临界区 — 两个 conn_task 若交错（A push, B push, B send, A
                        // send）则 wire 顺序 ≠ pending 顺序 → 响应错配。Unbounded
                        // Sender 同步发送、无 await，锁内安全。
                        let mut reply: Option<SignalingMessage> = None;
                        {
                            let mut st = lock_state(&state);
                            // E3: 快照数据源 — src 声明 + 最近消息时刻（同一临界区）
                            if let Some(c) = st.conns.get_mut(&conn_id) {
                                c.src = env.src.clone();
                                c.last_msg = Some(Instant::now());
                            }
                            match st.upstream(conn_id, env.msg) {
                                UpstreamAction::Forward(msg) => {
                                    if remote_tx.send(msg).is_err() {
                                        // 远端任务已退出（网关关闭）：回滚本请求的
                                        // pending 槽（锁内唯一 push，pop_back 安全）
                                        st.pending.pop_back();
                                        tracing::warn!(conn_id, "远端任务已退出");
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
                        tracing::warn!(conn_id, "本地 WS 读错误: {e}");
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
    lock_state(&state).remove_conn(conn_id);
    tracing::info!(conn_id, "本地子进程断开");
}

/// 远端单 WS 循环：connect（B1 重连）→ 双向转发 → 断线清理 → 重连。
async fn remote_loop(
    config: GatewayConfig,
    state: Arc<Mutex<State>>,
    mut remote_rx: mpsc::UnboundedReceiver<SignalingMessage>,
) {
    let mut client = SignalClient::new(&config.remote_url, &config.psk, &config.room, PeerRole::Host);
    // G4: 设备凭证随 Join 携带（additive；None = PSK 路径）
    if let Some(cred) = config.device.clone() {
        client = client.with_device_credentials(cred);
    }
    // H6（时机修正）: 上游"断开→恢复"的重连标记——通知只在重连成功后下发。
    // 断线瞬间通知会让 streamer 在 server 宕机窗口 1-2s 一轮重启 → 触发 oxmgr
    // crash-loop 熔断（3次/5min）→ 永不恢复。重连成功后通知 = server 就绪，一发即中。
    let mut reconnected = false;
    loop {
        let session = match client.connect_with_retry(config.retry).await {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("远端信令连接失败: {e}，10s 后重试");
                tokio::time::sleep(Duration::from_secs(10)).await;
                continue;
            }
        };
        tracing::info!(room = %config.room, "agent 已加入整车房间");
        {
            let mut st = lock_state(&state);
            st.joined = true;
            st.remote_since = Some(Instant::now());
            st.vehicle_peer_id = session.peer_id().to_string();
        }
        if reconnected {
            let notify: Vec<_> = {
                let st = lock_state(&state);
                st.conns.values().map(|c| (c.tx.clone(), c.src.clone())).collect()
            };
            for (tx, src) in &notify {
                let env = LocalEnvelope { src: "server".into(), msg: gateway_disconnected_error() };
                if tx.send(env).is_err() {
                    tracing::warn!(src = %src, "上游恢复通知目标连接已断开");
                }
            }
            tracing::info!(children = notify.len(), "H6: 上游已恢复，要求下游重建 SFU 会话");
            reconnected = false;
        }
        run_session(session, &state, &mut remote_rx).await;
        // 断线：清空与远端会话绑定的状态 + 丢弃在途上行（重连后按新会话处理）
        lock_state(&state).reset_remote();
        while remote_rx.try_recv().is_ok() {}
        reconnected = true;
        tracing::info!("远端断开，重新连接…");
    }
}

/// 单次远端会话：events → 下行路由；上行通道 → session.send。
async fn run_session(
    session: SignalSession,
    state: &Arc<Mutex<State>>,
    remote_rx: &mut mpsc::UnboundedReceiver<SignalingMessage>,
) {
    let mut events = session.events();
    loop {
        tokio::select! {
            ev = events.recv() => match ev {
                Ok(SignalEvent::Message(msg)) => {
                    let targets = lock_state(state).downstream(msg);
                    for (conn_id, m) in targets {
                        let tx = lock_state(state).conns.get(&conn_id).map(|c| c.tx.clone());
                        if let Some(tx) = tx {
                            let env = LocalEnvelope { src: "server".into(), msg: m };
                            if tx.send(env).is_err() {
                                tracing::warn!(conn_id, "下发目标连接已断开");
                            }
                        }
                    }
                }
                Ok(SignalEvent::Disconnected { reason }) => {
                    tracing::warn!("远端断开: {reason}");
                    break;
                }
                Ok(SignalEvent::Error(e)) => tracing::warn!("远端信令错误: {e}"),
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!("远端事件滞后 {n} 条");
                }
                Err(_) => break, // 会话通道关闭 = 断开
            },
            msg = remote_rx.recv() => match msg {
                Some(m) => {
                    if let Err(e) = session.send(m).await {
                        tracing::warn!("远端发送失败: {e}");
                        break;
                    }
                }
                None => return, // 网关关闭
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// H2 (I3 review): 音频房间消息不改写 — 判定消息自身 room_id（upstream
    /// 方向 room 参数 = 整车房间，早期实现误用它导致音频房间被并入整车房间）。
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
                assert_eq!(room_id, "audio-ms-car1", "音频房间必须原样（不改写）")
            }
            other => panic!("意外变体: {other:?}"),
        }

        // 非音频房间照常改写
        let mut msg2 = SignalingMessage::CreateWebRtcTransport {
            room_id: "stream-cam0".into(),
            peer_id: "s".into(),
            direction: TransportDirection::Send,
        };
        rewrite_room(&mut msg2, "vehicle");
        match &msg2 {
            SignalingMessage::CreateWebRtcTransport { room_id, .. } => {
                assert_eq!(room_id, "vehicle", "非音频房间必须改写为整车房间")
            }
            other => panic!("意外变体: {other:?}"),
        }

        // multi-stream P3: per-stream 房间（<整车>_<stream>）直通
        let mut msg3 = SignalingMessage::CreateWebRtcTransport {
            room_id: "vehicle_test-30fps".into(),
            peer_id: "s".into(),
            direction: TransportDirection::Send,
        };
        rewrite_room(&mut msg3, "vehicle");
        match &msg3 {
            SignalingMessage::CreateWebRtcTransport { room_id, .. } => {
                assert_eq!(room_id, "vehicle_test-30fps", "per-stream 房间必须直通")
            }
            other => panic!("意外变体: {other:?}"),
        }
    }

    fn test_state() -> Arc<Mutex<State>> {
        Arc::new(Mutex::new(State {
            conns: HashMap::new(),
            next_id: 1,
            pending: VecDeque::new(),
            echo_cache: VecDeque::new(),
            vehicle_peer_id: String::new(),
            joined: false,
            remote_since: None,
            vehicle_room: "vehicle-1".into(),
            pending_config: None,
        }))
    }


    fn handle_for(state: Arc<Mutex<State>>) -> (GatewayHandle, mpsc::UnboundedReceiver<SignalingMessage>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (GatewayHandle { state, remote_tx: tx }, rx)
    }

    #[test]
    fn snapshot_reports_children_and_remote() {
        let state = test_state();
        {
            let mut st = lock_state(&state);
            st.joined = true;
            st.vehicle_peer_id = "veh-peer".into();
            st.remote_since = Some(Instant::now());
            st.conns.insert(1, Conn {
                id: 1,
                room: "room-a".into(),
                src: "host-streamer".into(),
                last_msg: Some(Instant::now()),
                tx: mpsc::unbounded_channel().0,
            });
            st.conns.insert(2, Conn {
                id: 2,
                room: "room-b".into(),
                src: "host-capturer".into(),
                last_msg: None,
                tx: mpsc::unbounded_channel().0,
            });
        }
        let (handle, _rx) = handle_for(state);
        let snap = handle.snapshot();
        // children 按 src 排序（确定性）
        assert_eq!(snap.children.len(), 2);
        assert_eq!(snap.children[0].src, "host-capturer");
        assert!(snap.children[0].connected);
        assert_eq!(snap.children[0].last_msg_secs, u64::MAX, "未发消息 = u64::MAX");
        assert_eq!(snap.children[1].src, "host-streamer");
        assert_eq!(snap.children[1].last_msg_secs, 0, "刚收到消息 = 0");
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
            .send_remote(SignalingMessage::Sdp { room_id: "r".into(), target: None, sdp: "v=0".into() })
            .unwrap_err();
        assert!(err.contains("not connected"), "未 joined 必须拒绝: {err}");
        assert!(rx.try_recv().is_err(), "拒绝的消息不得进入远端通道");
    }

    #[test]
    fn send_remote_forwards_when_joined() {
        let state = test_state();
        lock_state(&state).joined = true;
        let (handle, mut rx) = handle_for(state);
        let msg = SignalingMessage::Sdp { room_id: "r".into(), target: None, sdp: "v=0".into() };
        handle.send_remote(msg.clone()).expect("joined 时应发送成功");
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
        assert!(targets.is_empty(), "ConfigPush 不入子进程路由: {targets:?}");
        let handle = handle_for(state).0;
        match handle.take_config_push() {
            Some(SignalingMessage::ConfigPush { version, .. }) => assert_eq!(version, 3),
            other => panic!("期望 ConfigPush, got {other:?}"),
        }
        assert!(handle.take_config_push().is_none(), "take 后应清空");
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
        assert!(handle.take_config_push().is_none(), "不匹配的 push 不得入待应用槽");
    }
}
