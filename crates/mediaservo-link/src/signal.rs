//! 信令客户端（WS 连 server，复用 common `SignalingMessage`，PSK 认证）。
//!
//! 协议流程（与 host 一致）：
//! 1. 连接 `{url}/ws`
//! 2. 发送原始 PSK 作为首条文本消息
//! 3. 等待认证确认（`Error { code: 0 }`）
//! 4. 发送 `RoomJoin { room_id, peer_role, stream_id }`
//! 5. 等待 `RoomJoined { room_id, peer_id }`
//!
//! Phase B (B1)：`connect_with_retry` 指数退避重连（重试走完整 connect →
//! PSK 认证按连接重做）；`SignalSession::on_disconnect` 断线通知（供上层
//! 触发重连；主动 `close()` 不触发）。
//!
//! D2 网关模式（`SignalClient::new_gateway`）：本地 wire 包 `LocalEnvelope`
//! （无 PSK 挑战——网关本地侧不认证，整车 PSK 在 agent 的远端连接）。
use base64::Engine as _;
use ed25519_dalek::Signer as _;
use futures_util::{SinkExt, StreamExt};
use mediaservo_common::protocol::{
    negotiate_protocol, PeerRole, SignalingMessage, SIGNALING_PROTOCOL_VERSION,
};
use serde::{Deserialize, Serialize};
use tokio::net::TcpStream;
use tokio::sync::{broadcast, mpsc};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

use crate::error::LinkError;

type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// 本地网关信封（D2: 子进程 ↔ host-agent 本地 wire；下发方向 src 固定 "server"）。
/// 语义见 mediaservo-host::gateway（D1）：RoomJoin 拦截/响应路由/房间重写。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalEnvelope {
    /// 子进程标识（如 "host-streamer-cam0"）；下发方向固定为 "server"。
    pub src: String,
    pub msg: SignalingMessage,
}



/// 断线回调槽（会话与后台任务共享；注册后至多触发一次）。
type DisconnectSlot = std::sync::Arc<std::sync::Mutex<Option<Box<dyn Fn() + Send + Sync>>>>;

/// 信令事件。
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum SignalEvent {
    /// 已连接并加入房间。
    Connected { room_id: String },
    /// 收到一条信令消息。
    Message(SignalingMessage),
    /// 连接断开。
    Disconnected { reason: String },
    /// 错误（解析失败等）。
    Error(String),
}

/// 重连配置（coding-style retry_with_backoff 模式：base 100ms → max 30s，±25% jitter）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryConfig {
    /// 最大重试次数（初试之外；总尝试 = max_retries + 1）。
    pub max_retries: u32,
    /// 首次重试基础退避。
    pub base_delay: std::time::Duration,
    /// 退避上限（指数增长封顶）。
    pub max_delay: std::time::Duration,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            base_delay: std::time::Duration::from_millis(100),
            max_delay: std::time::Duration::from_secs(30),
        }
    }
}

/// 信令客户端（每节点一个；connect 建立会话）。
#[derive(Debug, Clone)]
pub struct SignalClient {
    url: String,
    psk: String,
    room_id: String,
    role: PeerRole,
    /// D2 本地网关模式：Some(src) = 信封 wire（无 PSK 挑战，信任边界 127.0.0.1）；
    /// None = 直连 server（PSK 认证）。
    gateway_src: Option<String>,
    /// G4 设备凭证（D-H11）：Some = RoomJoin 携带 device_id/device_secret（additive），
    /// G2 起 server 校验；None = PSK 认证路径（现状保持）。
    device: Option<DeviceCredential>,
    /// device-enroll T7：公钥指纹身份（Some = Join 带 device_pubkey 走验签链，
    /// 优先于 device）。gateway/host-agent 装配透传。
    identity: Option<DeviceIdentity>,
    /// a2（S0.5）：上次会话的一次性重挂票（server RoomJoined.session_nonce）。
    /// Some = 下一次 connect 的 RoomJoin 带 resume（仅 v3 被 server 受理）；
    /// 取出即焚（消费/失败都不复用——重放由 server 端 burn 保证）。
    resume_ticket: std::sync::Arc<std::sync::Mutex<Option<String>>>,
}

/// 设备凭证（identity.json 格式 + RoomJoin wire 载体，G4/D-H13）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceCredential {
    /// 设备 ID（host init 生成，如 `ms-<12 hex>`）。
    pub device_id: String,
    /// 设备密钥（32 随机字节 hex）。
    pub device_secret: String,
}

/// device-enroll 验签链单跳超时（design §1：challenge 5s 未达/未答 = 断连错误）。
const DEVICE_AUTH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// 运行期设备身份（device-enroll T6/T7，公钥指纹路径）：gateway 持有；RoomJoin 携带
/// `pubkey_b64`，DeviceAuthChallenge 以 [`DeviceIdentity::sign_device_auth`] 应答。
/// 与 secret 形 [`DeviceCredential`] 共存一个周期（D-E3）。
#[derive(Clone)]
pub struct DeviceIdentity {
    /// 设备 ID（identity.json `device_id`，server 注册键）。
    pub device_id: String,
    /// Ed25519 私钥（`etc/link/signing.pem` PKCS#8 读出，D-E1 一钥两用）。
    pub signing: ed25519_dalek::SigningKey,
    /// 验签公钥 32B 的 base64 standard 形（无换行）= RoomJoin.device_pubkey wire 值。
    pub pubkey_b64: String,
}

impl DeviceIdentity {
    /// 由私钥构建并派生公钥指纹。
    pub fn new(device_id: impl Into<String>, signing: ed25519_dalek::SigningKey) -> Self {
        let pubkey_b64 = base64::engine::general_purpose::STANDARD
            .encode(signing.verifying_key().to_bytes());
        Self {
            device_id: device_id.into(),
            signing,
            pubkey_b64,
        }
    }

    /// 字节合同（design §3）：sig = base64( Ed25519::sign( nonce_raw(32B) ‖ device_id ‖ room_id ) )。
    /// 交叉复验锚 = server devices.rs::sig_vector（同常量必出同 sig）。
    pub fn sign_device_auth(&self, nonce_raw: &[u8], room_id: &str) -> String {
        let mut msg =
            Vec::with_capacity(nonce_raw.len() + self.device_id.len() + room_id.len());
        msg.extend_from_slice(nonce_raw);
        msg.extend_from_slice(self.device_id.as_bytes());
        msg.extend_from_slice(room_id.as_bytes());
        base64::engine::general_purpose::STANDARD
            .encode(self.signing.sign(&msg).to_bytes())
    }
}

impl std::fmt::Debug for DeviceIdentity {
    /// 私钥不落日志：signing 以占位呈现。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeviceIdentity")
            .field("device_id", &self.device_id)
            .field("pubkey_b64", &self.pubkey_b64)
            .field("signing", &"<redacted>")
            .finish()
    }
}

impl SignalClient {
    pub fn new(url: &str, psk: &str, room_id: &str, role: PeerRole) -> Self {
        Self {
            url: url.trim_end_matches('/').to_string(),
            psk: psk.to_string(),
            room_id: room_id.to_string(),
            role,
            gateway_src: None,
            device: None,
            identity: None,
            resume_ticket: std::sync::Arc::new(std::sync::Mutex::new(None)),
        }
    }

    /// 本地网关模式（D2）：WS 连 host-agent，无 PSK 挑战（网关本地侧不认证，
    /// 整车 PSK 在 agent 的远端连接）；全部消息包 LocalEnvelope {src, msg}。
    pub fn new_gateway(url: &str, src: &str, room_id: &str, role: PeerRole) -> Self {
        Self {
            url: url.trim_end_matches('/').to_string(),
            psk: String::new(),
            room_id: room_id.to_string(),
            role,
            gateway_src: Some(src.to_string()),
            device: None,
            identity: None,
            resume_ticket: std::sync::Arc::new(std::sync::Mutex::new(None)),
        }
    }

    /// 附加设备凭证（G4）：RoomJoin 携带 device_id/device_secret（additive，PSK 并存）。
    pub fn with_device_credentials(mut self, device: DeviceCredential) -> Self {
        self.device = Some(device);
        self
    }

    /// a2：设置重挂票（gateway 断线时回灌上次 nonce；None = 清票走全量 join）。
    pub fn set_resume_ticket(&self, nonce: Option<String>) {
        if let Ok(mut g) = self.resume_ticket.lock() {
            *g = nonce;
        }
    }

    /// a2：取出并清空（一次性）。
    fn take_resume_ticket(&self) -> Option<String> {
        self.resume_ticket.lock().ok().and_then(|mut g| g.take())
    }

    /// 附加设备身份（device-enroll T7）：RoomJoin 携带 device_pubkey，challenge 到达时
    /// 以私钥签 DeviceAuthResponse；未批准（手动档）→ `LinkError::EnrollPending`。
    pub fn with_device_identity(mut self, identity: DeviceIdentity) -> Self {
        self.identity = Some(identity);
        self
    }

    /// 连接 server、PSK 认证、加入房间，返回会话。
    pub async fn connect(&self) -> Result<SignalSession, LinkError> {
        let (ws_stream, _resp) = connect_async(&self.url)
            .await
            .map_err(|e| LinkError::Signal(format!("connect {}: {e}", self.url)))?;
        let (mut sender, mut receiver) = ws_stream.split();

        // Phase 1: PSK 认证 — 仅直连 server 模式；网关本地侧不认证（D2，
        // 信任边界 127.0.0.1，整车 PSK 在 agent 的远端连接）
        if self.gateway_src.is_none() {
            sender
                .send(Message::Text(self.psk.clone().into()))
                .await
                .map_err(|e| LinkError::Signal(format!("send auth: {e}")))?;
            let auth_msg = receiver
                .next()
                .await
                .ok_or_else(|| LinkError::Signal("connection closed during auth".into()))?
                .map_err(|e| LinkError::Signal(format!("auth read: {e}")))?;
            let auth_msg = match auth_msg {
                Message::Text(t) => serde_json::from_str::<SignalingMessage>(&t)
                    .map_err(|e| LinkError::Signal(format!("parse auth response: {e}")))?,
                Message::Close(_) => return Err(LinkError::Signal("closed during auth".into())),
                _ => return Err(LinkError::Signal("unexpected auth response".into())),
            };
            match auth_msg {
                SignalingMessage::Error { code, .. } if code == 0 => {}
                SignalingMessage::Error { code, message } => {
                    return Err(LinkError::Signal(format!("auth denied [{code}]: {message}")));
                }
                _ => return Err(LinkError::Signal("unexpected auth message".into())),
            }
        }

        // Phase 2: 加入房间（网关模式包 LocalEnvelope）
        let join = SignalingMessage::RoomJoin {
            room_id: self.room_id.clone(),
            peer_role: self.role.clone(),
            stream_id: None,
            device_id: self
                .identity
                .as_ref()
                .map(|i| i.device_id.clone())
                .or_else(|| self.device.as_ref().map(|d| d.device_id.clone())),
            device_secret: self.device.as_ref().map(|d| d.device_secret.clone()),
            device_pubkey: self.identity.as_ref().map(|i| i.pubkey_b64.clone()),
            // S0: 声明本端方言上限（server 取 min 回谈成值）。
            protocol: Some(SIGNALING_PROTOCOL_VERSION),
            client_version: None,
            // a2: 有一次性票则声明重挂（server 侧 negotiated≥3 + 认证重跑 + nonce 校验）。
            resume: self.take_resume_ticket(),
        };
        let (join_json, unwrap) = match &self.gateway_src {
            Some(src) => (
                serde_json::to_string(&LocalEnvelope { src: src.clone(), msg: join })
                    .map_err(|e| LinkError::Signal(format!("serialize RoomJoin envelope: {e}")))?,
                true,
            ),
            None => (
                serde_json::to_string(&join)
                    .map_err(|e| LinkError::Signal(format!("serialize RoomJoin: {e}")))?,
                false,
            ),
        };
        sender
            .send(Message::Text(join_json.into()))
            .await
            .map_err(|e| LinkError::Signal(format!("send RoomJoin: {e}")))?;
        let joined = if self.identity.is_some() {
            // device-enroll §1：pubkey 形 Join 后每一跳响应（challenge/终态）≤5s，静默 = 断连错误
            tokio::time::timeout(DEVICE_AUTH_TIMEOUT, read_join_response(&mut receiver, unwrap))
                .await
                .map_err(|_| {
                    LinkError::Signal("device auth: no server response within 5s after RoomJoin".into())
                })?
                ?
        } else {
            read_join_response(&mut receiver, unwrap).await?
        };
        let joined = match (&self.identity, &joined) {
            (Some(_), SignalingMessage::DeviceAuthChallenge { .. })
            | (Some(_), SignalingMessage::DeviceAuthPending { .. }) => {
                self.run_device_auth(joined, &mut sender, &mut receiver, unwrap)
                    .await?
            }
            _ => joined,
        };
        match joined {
            SignalingMessage::RoomJoined { room_id, peer_id, protocol, session_nonce, .. } => {
                let (events_tx, _) = broadcast::channel(64);
                // a3（S0.5）：双有界队列——hi（认证/控制/一切非白名单）背压不丢；
                // lo（封闭白名单 {StatusReport}）尽力而为，满=丢弃+计数。
                let (hi_tx, hi_rx) = mpsc::channel(64);
                let (lo_tx, lo_rx) = mpsc::channel(16);
                let dropped = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
                let on_disconnect = DisconnectSlot::default();
                let task = tokio::spawn(session_task(
                    receiver,
                    sender,
                    hi_rx,
                    lo_rx,
                    events_tx.clone(),
                    on_disconnect.clone(),
                    self.gateway_src.clone(),
                ));
                let _ = events_tx.send(SignalEvent::Connected { room_id: room_id.clone() });
                Ok(SignalSession {
                    room_id,
                    peer_id,
                    negotiated: negotiate_protocol(protocol),
                    session_nonce,
                    hi_tx,
                    lo_tx,
                    dropped,
                    events_tx,
                    task,
                    on_disconnect,
                })
            }
            SignalingMessage::Error { code, message } => {
                Err(LinkError::Signal(format!("room join failed [{code}]: {message}")))
            }
            _ => Err(LinkError::Signal("unexpected response to RoomJoin".into())),
        }
    }

    /// 验签应答链（device-enroll §4 修订）：challenge→签发 DeviceAuthResponse→（再）读，
    /// RoomJoined/Error 等终态放行给调用方 match；DeviceAuthPending → typed EnrollPending。
    /// 调用方保证 identity = Some；每跳 5s 超时 = 断连错误（§1）。
    async fn run_device_auth(
        &self,
        first: SignalingMessage,
        sender: &mut futures_util::stream::SplitSink<WsStream, Message>,
        receiver: &mut futures_util::stream::SplitStream<WsStream>,
        unwrap: bool,
    ) -> Result<SignalingMessage, LinkError> {
        let identity = self
            .identity
            .as_ref()
            .ok_or_else(|| LinkError::Signal("device auth without identity (internal)".into()))?;
        let mut msg = first;
        loop {
            msg = match msg {
                SignalingMessage::DeviceAuthChallenge { nonce } => {
                    let nonce_raw = base64::engine::general_purpose::STANDARD
                        .decode(&nonce)
                        .map_err(|e| LinkError::Signal(format!("challenge nonce base64: {e}")))?;
                    if nonce_raw.len() != 32 {
                        return Err(LinkError::Signal(format!(
                            "challenge nonce must be 32 bytes, got {}",
                            nonce_raw.len()
                        )));
                    }
                    let resp = SignalingMessage::DeviceAuthResponse {
                        room_id: self.room_id.clone(),
                        sig: identity.sign_device_auth(&nonce_raw, &self.room_id),
                    };
                    let json = match &self.gateway_src {
                        Some(src) => serde_json::to_string(&LocalEnvelope {
                            src: src.clone(),
                            msg: resp,
                        }),
                        None => serde_json::to_string(&resp),
                    }
                    .map_err(|e| {
                        LinkError::Signal(format!("serialize DeviceAuthResponse: {e}"))
                    })?;
                    sender
                        .send(Message::Text(json))
                        .await
                        .map_err(|e| LinkError::Signal(format!("send DeviceAuthResponse: {e}")))?;
                    tokio::time::timeout(
                        DEVICE_AUTH_TIMEOUT,
                        read_join_response(receiver, unwrap),
                    )
                    .await
                    .map_err(|_| {
                        LinkError::Signal(
                            "device auth: no server response within 5s after DeviceAuthResponse"
                                .into(),
                        )
                    })??
                }
                SignalingMessage::DeviceAuthPending { device_id } => {
                    return Err(LinkError::EnrollPending { device_id });
                }
                other => return Ok(other),
            };
        }
    }

    /// 连接并自动重试（指数退避 + ±25% jitter）。
    ///
    /// 每次重试都走完整 `connect()`（WS 连接 → PSK 认证 → 入房），
    /// 故重连后认证自动重做（PSK 挑战按连接计，无需额外状态）。
    pub async fn connect_with_retry(&self, cfg: RetryConfig) -> Result<SignalSession, LinkError> {
        let mut attempt = 0u32;
        loop {
            match self.connect().await {
                Ok(session) => return Ok(session),
                Err(e) if attempt < cfg.max_retries => {
                    let backoff = cfg
                        .base_delay
                        .saturating_mul(2u32.saturating_pow(attempt))
                        .min(cfg.max_delay);
                    let sleep = jittered(backoff);
                    tracing::warn!(
                        "signal connect failed (attempt {}) to {}, retry in {:?}: {e}",
                        attempt + 1,
                        self.url,
                        sleep
                    );
                    tokio::time::sleep(sleep).await;
                    attempt += 1;
                }
                Err(e @ LinkError::EnrollPending { .. }) => return Err(e),
                Err(e) => {
                    return Err(LinkError::Signal(format!(
                        "connect after {} retries: {e}",
                        cfg.max_retries
                    )));
                }
            }
        }
    }
}

/// 信令会话：send 发送消息，events 接收事件。
pub struct SignalSession {
    room_id: String,
    /// RoomJoined 返回的 peer_id（D1 网关合成子进程应答使用）。
    peer_id: String,
    /// S0：协商谈成的方言版本（server RoomJoined.protocol；缺省 = v1）。
    negotiated: u32,
    /// a2：一次性重挂票（negotiated≥3 时 server 下发；消费即焚由 server 端保证）。
    session_nonce: Option<String>,
    /// a3：高优有界队列（认证/控制/一切非白名单）。
    hi_tx: mpsc::Sender<SignalingMessage>,
    /// a3：低优尽力队列（封闭白名单 {StatusReport}）——满即丢，dropped 计数。
    lo_tx: mpsc::Sender<SignalingMessage>,
    dropped: std::sync::Arc<std::sync::atomic::AtomicU64>,
    events_tx: broadcast::Sender<SignalEvent>,
    task: tokio::task::JoinHandle<()>,
    on_disconnect: DisconnectSlot,
}

impl std::fmt::Debug for SignalSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SignalSession")
            .field("room_id", &self.room_id)
            .field("peer_id", &self.peer_id)
            .finish()
    }
}

impl SignalSession {
    /// 订阅信令事件（返回新接收器）。
    pub fn events(&self) -> broadcast::Receiver<SignalEvent> {
        self.events_tx.subscribe()
    }

    /// 注册断线回调：WS 断开（对端 Close/网络错误）时触发一次，供上层触发重连。
    /// 注意：`close()` 主动关闭不触发。
    pub fn on_disconnect(&self, cb: Box<dyn Fn() + Send + Sync>) {
        if let Ok(mut slot) = self.on_disconnect.lock() {
            *slot = Some(cb);
        }
    }

    /// 发送一条信令消息（JSON 序列化后经 WS 发出）。
    /// a3 路由：白名单（StatusReport）→ lo try_send（满 = 丢弃+计数，拥塞期保新鲜度）；
    /// 其余 → hi await 背压（认证/控制/终态永不丢）。
    pub async fn send(&self, msg: SignalingMessage) -> Result<(), LinkError> {
        if is_low_priority(&msg) {
            if self.lo_tx.try_send(msg).is_err() {
                let n = self
                    .dropped
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                    + 1;
                if n == 1 || n % 64 == 0 {
                    tracing::warn!("a3: 低优队列满，StatusReport 丢弃（累计 {n}）");
                }
            }
            return Ok(());
        }
        self.hi_tx
            .send(msg)
            .await
            .map_err(|_| LinkError::Signal("session closed".into()))
    }

    /// 当前房间 ID。
    pub fn room_id(&self) -> &str {
        &self.room_id
    }

    /// 当前会话的 peer_id（RoomJoined 时 server 分配；D1 网关合成子进程应答）。
    pub fn peer_id(&self) -> &str {
        &self.peer_id
    }

    /// S0 方言协商结果（min(client claim, server max)；1 = 旧对端/旧 server）。
    #[must_use]
    pub fn negotiated_protocol(&self) -> u32 {
        self.negotiated
    }

    /// a2: 上次会话下发的重挂票（None = 未谈成 v3；使用方 B3 resume 链接线）。
    #[must_use]
    pub fn session_nonce(&self) -> Option<&str> {
        self.session_nonce.as_deref()
    }

    /// a3: 低优累计丢弃数（SLA 测量面；StatusReport 透传位届时接）。
    #[must_use]
    pub fn dropped_low(&self) -> u64 {
        self.dropped.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// 关闭会话：停止发送通道并等待后台任务退出。
    pub async fn close(self) -> Result<(), LinkError> {
        drop((self.hi_tx, self.lo_tx)); // 双通道齐关 → writer 退出；单端 None 不误判
        let _ = self.task.await;
        Ok(())
    }
}

/// RoomJoin 首响应：一帧 WS 读 +（信封）JSON 解析（逻辑自 connect 原路径原样迁入）。
async fn read_join_response(
    receiver: &mut futures_util::stream::SplitStream<WsStream>,
    unwrap: bool,
) -> Result<SignalingMessage, LinkError> {
    let joined = receiver
        .next()
        .await
        .ok_or_else(|| LinkError::Signal("connection closed during room join".into()))?
        .map_err(|e| LinkError::Signal(format!("RoomJoin read: {e}")))?;
    match joined {
        Message::Text(t) => {
            if unwrap {
                let env: LocalEnvelope = serde_json::from_str(&t)
                    .map_err(|e| LinkError::Signal(format!("parse envelope response: {e}")))?;
                Ok(env.msg)
            } else {
                serde_json::from_str::<SignalingMessage>(&t)
                    .map_err(|e| LinkError::Signal(format!("parse RoomJoined: {e}")))
            }
        }
        Message::Close(_) => Err(LinkError::Signal("closed during room join".into())),
        _ => Err(LinkError::Signal("unexpected RoomJoined response".into())),
    }
}

/// a3（S0.5）：封闭丢弃白名单——v1 仅 {StatusReport}（高频、丢一条无硬害、下一条覆盖）。
/// 新条目必须显式列入并复核「可丢」论证。
#[must_use]
fn is_low_priority(msg: &SignalingMessage) -> bool {
    matches!(msg, SignalingMessage::StatusReport { .. })
}

/// 后台任务：WS 读 → events；双上行队列（hi 优先出队 / lo 尽力）→ WS 写。
/// D2 网关模式：gateway_src = Some(src) 时收发均包 LocalEnvelope 信封。
async fn session_task(
    mut ws_rx: futures_util::stream::SplitStream<WsStream>,
    mut ws_tx: futures_util::stream::SplitSink<WsStream, Message>,
    mut hi_rx: mpsc::Receiver<SignalingMessage>,
    mut lo_rx: mpsc::Receiver<SignalingMessage>,
    events_tx: broadcast::Sender<SignalEvent>,
    on_disconnect: DisconnectSlot,
    gateway_src: Option<String>,
) {
    // a1 client 侧：定期 ping（split-sink 下自动 pong 须有写 poll 驱动冲刷——双向 ping
    // 是免误杀的诚实解）+ 入站静默 15s 判死（预算与 server 侧 interval×(miss+1) 对齐；
    // 措辞纪律：非「秒级」承诺）。网关本地环回不做心跳（断开即时可见）。
    let mut hb = tokio::time::interval(std::time::Duration::from_secs(5));
    hb.tick().await; // 吞掉 interval 的 t=0 就绪拍（刚建连即 ping = 无谓帧）
    let mut last_rx = std::time::Instant::now();
    loop {
        // biased：读最优先（饿读 = 活性误判 + pong 不冲刷）；hi 先于 lo（应用层 HoL 解药，
        // 边界见 PLAN §11.2 a3——kernel/TCP 归急停双路，死链归本心跳）。
        let outbound = tokio::select! {
            biased;
            ws = ws_rx.next() => {
                last_rx = std::time::Instant::now();
                match ws {
                    Some(Ok(Message::Text(text))) => {
                        let parsed = match &gateway_src {
                            Some(_) => serde_json::from_str::<LocalEnvelope>(&text)
                                .map(|env| env.msg)
                                .map_err(|e| format!("parse envelope: {e}")),
                            None => serde_json::from_str::<SignalingMessage>(&text)
                                .map_err(|e| format!("parse message: {e}")),
                        };
                        match parsed {
                            Ok(m) => { let _ = events_tx.send(SignalEvent::Message(m)); }
                            Err(e) => { let _ = events_tx.send(SignalEvent::Error(e)); }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        let _ = events_tx.send(SignalEvent::Disconnected { reason: "connection closed".into() });
                        fire_disconnect(&on_disconnect);
                        break;
                    }
                    Some(Ok(_)) => {} // 忽略非文本（含入站 pong——tokio-tungstenite 已自动应答）
                    Some(Err(e)) => {
                        let _ = events_tx.send(SignalEvent::Error(e.to_string()));
                        fire_disconnect(&on_disconnect);
                        break;
                    }
                }
                continue;
            }
            m = hi_rx.recv() => m,
            m = lo_rx.recv() => m,
            _ = hb.tick() => {
                if gateway_src.is_some() {
                    continue; // 本地环回模式：不心跳
                }
                if last_rx.elapsed() > std::time::Duration::from_secs(15) {
                    tracing::warn!("a1: 入站静默 15s → 判死，主动断开走既有重连");
                    let _ = events_tx.send(SignalEvent::Disconnected {
                        reason: "heartbeat silent >15s".into(),
                    });
                    fire_disconnect(&on_disconnect);
                    break;
                }
                if ws_tx.send(Message::Ping("msrtc-hb".into())).await.is_err() {
                    fire_disconnect(&on_disconnect);
                    break;
                }
                continue;
            }
        };
        let Some(m) = outbound else { break }; // 双 sender 同坠 = 会话主动关闭，不触发断线回调
        let json = match &gateway_src {
            Some(src) => serde_json::to_string(&LocalEnvelope { src: src.clone(), msg: m })
                .map_err(|e| format!("serialize envelope: {e}")),
            None => serde_json::to_string(&m).map_err(|e| format!("serialize: {e}")),
        };
        let json = match json {
            Ok(j) => j,
            Err(e) => {
                let _ = events_tx.send(SignalEvent::Error(e));
                continue;
            }
        };
        if ws_tx.send(Message::Text(json.into())).await.is_err() {
            fire_disconnect(&on_disconnect);
            break;
        }
    }
}

/// 触发一次断线回调（take 后调用，保证只触发一次；锁外执行用户代码防毒化）。
fn fire_disconnect(slot: &DisconnectSlot) {
    let cb = match slot.lock() {
        Ok(mut g) => g.take(),
        Err(poisoned) => poisoned.into_inner().take(),
    };
    if let Some(cb) = cb {
        cb();
    }
}

/// 指数退避 × ±25% jitter（xorshift32 种子取自时钟，避免引入 RNG 依赖）。
fn jittered(backoff: std::time::Duration) -> std::time::Duration {
    let mut seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u32)
        .unwrap_or(0x9e37_79b9);
    seed ^= seed << 13;
    seed ^= seed >> 17;
    seed ^= seed << 5;
    let jitter = (seed as f64 / u32::MAX as f64) * 0.5 - 0.25;
    std::time::Duration::from_secs_f64(backoff.as_secs_f64() * (1.0 + jitter))
}
