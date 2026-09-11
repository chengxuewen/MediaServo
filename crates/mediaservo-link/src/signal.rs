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
use futures_util::{SinkExt, StreamExt};
use mediaservo_common::protocol::{PeerRole, SignalingMessage};
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
}

/// 设备凭证（identity.json 格式 + RoomJoin wire 载体，G4/D-H13）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceCredential {
    /// 设备 ID（host init 生成，如 `ms-<12 hex>`）。
    pub device_id: String,
    /// 设备密钥（32 随机字节 hex）。
    pub device_secret: String,
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
        }
    }

    /// 附加设备凭证（G4）：RoomJoin 携带 device_id/device_secret（additive，PSK 并存）。
    pub fn with_device_credentials(mut self, device: DeviceCredential) -> Self {
        self.device = Some(device);
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
            device_id: self.device.as_ref().map(|d| d.device_id.clone()),
            device_secret: self.device.as_ref().map(|d| d.device_secret.clone()),
            device_pubkey: None,
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
        let joined = receiver
            .next()
            .await
            .ok_or_else(|| LinkError::Signal("connection closed during room join".into()))?
            .map_err(|e| LinkError::Signal(format!("RoomJoin read: {e}")))?;
        let joined = match joined {
            Message::Text(t) => {
                if unwrap {
                    let env: LocalEnvelope = serde_json::from_str(&t)
                        .map_err(|e| LinkError::Signal(format!("parse envelope response: {e}")))?;
                    env.msg
                } else {
                    serde_json::from_str::<SignalingMessage>(&t)
                        .map_err(|e| LinkError::Signal(format!("parse RoomJoined: {e}")))? 
                }
            }
            Message::Close(_) => return Err(LinkError::Signal("closed during room join".into())),
            _ => return Err(LinkError::Signal("unexpected RoomJoined response".into())),
        };
        match joined {
            SignalingMessage::RoomJoined { room_id, peer_id } => {
                let (events_tx, _) = broadcast::channel(64);
                let (send_tx, send_rx) = mpsc::unbounded_channel();
                let on_disconnect = DisconnectSlot::default();
                let task = tokio::spawn(session_task(
                    receiver,
                    sender,
                    send_rx,
                    events_tx.clone(),
                    on_disconnect.clone(),
                    self.gateway_src.clone(),
                ));
                let _ = events_tx.send(SignalEvent::Connected { room_id: room_id.clone() });
                Ok(SignalSession {
                    room_id,
                    peer_id,
                    send_tx,
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
    send_tx: mpsc::UnboundedSender<SignalingMessage>,
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
    pub async fn send(&self, msg: SignalingMessage) -> Result<(), LinkError> {
        self.send_tx
            .send(msg)
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

    /// 关闭会话：停止发送通道并等待后台任务退出。
    pub async fn close(self) -> Result<(), LinkError> {
        drop(self.send_tx); // 触发后台任务退出
        let _ = self.task.await;
        Ok(())
    }
}

/// 后台任务：WS 读 → events；send 通道 → WS 写。
/// D2 网关模式：gateway_src = Some(src) 时收发均包 LocalEnvelope 信封。
#[allow(clippy::too_many_arguments)]
async fn session_task(
    mut ws_rx: futures_util::stream::SplitStream<WsStream>,
    mut ws_tx: futures_util::stream::SplitSink<WsStream, Message>,
    mut send_rx: mpsc::UnboundedReceiver<SignalingMessage>,
    events_tx: broadcast::Sender<SignalEvent>,
    on_disconnect: DisconnectSlot,
    gateway_src: Option<String>,
) {
    loop {
        tokio::select! {
            msg = ws_rx.next() => {
                match msg {
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
                    Some(Ok(_)) => {} // 忽略非文本
                    Some(Err(e)) => {
                        let _ = events_tx.send(SignalEvent::Error(e.to_string()));
                        fire_disconnect(&on_disconnect);
                        break;
                    }
                }
            }
            msg = send_rx.recv() => {
                match msg {
                    Some(m) => {
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
                    None => break, // 会话主动关闭，不触发 on_disconnect
                }
            }
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
