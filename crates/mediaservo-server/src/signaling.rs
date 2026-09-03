use crate::audit::{self, AuditEvent};
use crate::devices::{self, DeviceRegistry};
use crate::health::{HealthChecker, HealthStatus, ReadinessChecker};
use crate::roles::{AccountIdentity, CockpitRole, SessionIdentity};
use crate::room::RoomManager;
use crate::status::StatusRegistry;
use axum::Router;
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use axum::routing::get;
#[cfg(feature = "sfu-mediasoup")]
use futures_util::stream::SplitSink;
use futures_util::{SinkExt, StreamExt};
use mediaservo_common::auth::{JwtAuth, SimplePskAuth};
use mediaservo_common::error::CoreError;
use mediaservo_common::protocol::{PeerRole, SignalingMessage};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::broadcast;
use tokio::sync::watch;

struct RoomChannel {
    tx: broadcast::Sender<String>,
}

impl RoomChannel {
    fn new() -> Self {
        let (tx, _) = broadcast::channel::<String>(4096); // ponytail: 4096 frames ~= 4s at 1k fps
        Self { tx }
    }
}

#[derive(Clone)]
pub struct SignalingServer {
    channels: Arc<dashmap::DashMap<String, RoomChannel>>,
    pub room_manager: RoomManager,
    /// SFU manager for mediasoup transport negotiation.
    #[cfg(feature = "sfu-mediasoup")]
    pub sfu_manager: Arc<crate::sfu::SfuManager>,
    /// Shutdown signal — send `true` to request draining.
    shutdown_tx: watch::Sender<bool>,
    /// Number of currently active WebSocket connections.
    active_connections: Arc<AtomicUsize>,
    /// Admin 事件频道（列表秒级刷新触发；main.rs AdminState.event_tx 同源）。
    admin_events: tokio::sync::broadcast::Sender<String>,
    pub ws_max_message_size: usize,
    /// Pending messages cache — stores SDP offer + ICE candidates per room for late-joiner replay.
    pub pending_messages: Arc<dashmap::DashMap<String, Vec<String>>>,
    /// JWT authenticator (optional; PSK used as fallback).
    pub jwt_auth: Option<JwtAuth>,
    /// PSK 共享态（psk-admin-management）: main.rs 启动注入（config 优先 + env 兜底）;
    /// admin API 轮换写锁热更新。None = 未配置 — 保持既有「跳过 PSK 校验」语义。
    pub psk_state: std::sync::Arc<std::sync::RwLock<Option<String>>>,
    /// 整车状态上报注册表（E3: 每房间最新 StatusReport; H 阶段 admin API 读取）。
    pub status_registry: Arc<StatusRegistry>,
    /// G2 设备注册表（启动时从 devices.yaml 加载，只读；空 = PSK 路径）。
    pub device_registry: Arc<DeviceRegistry>,
    /// G2 连接级身份绑定（D-H11）: peer_id → device_id（设备认证成功时建立，断开时清除）。
    device_bindings: Arc<dashmap::DashMap<String, String>>,
    /// G3 房间主车登记（room_id → device_id; device 会话 join 成功时记录，
    /// 房间空时清除）— 舱端 RoomJoin/急停按主车做租户隔离 + 白名单授权。
    room_owners: Arc<dashmap::DashMap<String, String>>,
    /// G3 producer → 所属车 device_id（produce 时按会话设备绑定记录; consume 授权纵深防御）。
    producer_owners: Arc<dashmap::DashMap<String, String>>,
    /// F1/T4: 该设备本次会话已收到的 DownstreamGone 计数（agent 断开时用于区分
    /// "T4 已逐流清理"（正常）与"注册链断"（05:43 类悬案）——后者才 WARN）。
    t4_gone_seen: Arc<dashmap::DashMap<String, u32>>,
}

impl SignalingServer {
    #[cfg(feature = "sfu-mediasoup")]
    pub fn new(
        _sfu: Arc<crate::sfu::SfuManager>,
        ws_max_message_size: usize,
        jwt_auth: Option<JwtAuth>,
    ) -> Self {
        let (shutdown_tx, _) = watch::channel(false);
        Self {
            channels: Arc::new(dashmap::DashMap::new()),
            room_manager: RoomManager::new(),
            sfu_manager: _sfu,
            shutdown_tx,
            active_connections: Arc::new(AtomicUsize::new(0)),
            admin_events: { let (tx, _) = tokio::sync::broadcast::channel(256); tx },
            ws_max_message_size,
            pending_messages: Arc::new(dashmap::DashMap::new()),
            jwt_auth,
            psk_state: std::sync::Arc::new(std::sync::RwLock::new(None)),
            status_registry: Arc::new(StatusRegistry::default()),
            device_registry: Arc::new(DeviceRegistry::empty()),
            device_bindings: Arc::new(dashmap::DashMap::new()),
            room_owners: Arc::new(dashmap::DashMap::new()),
            producer_owners: Arc::new(dashmap::DashMap::new()),
            t4_gone_seen: Arc::new(dashmap::DashMap::new()),
        }
    }

    #[cfg(not(feature = "sfu-mediasoup"))]
    pub fn new(ws_max_message_size: usize, jwt_auth: Option<JwtAuth>) -> Self {
        let (shutdown_tx, _) = watch::channel(false);
        Self {
            channels: Arc::new(dashmap::DashMap::new()),
            room_manager: RoomManager::new(),
            shutdown_tx,
            active_connections: Arc::new(AtomicUsize::new(0)),
            admin_events: { let (tx, _) = tokio::sync::broadcast::channel(256); tx },
            ws_max_message_size,
            pending_messages: Arc::new(dashmap::DashMap::new()),
            jwt_auth,
            psk_state: std::sync::Arc::new(std::sync::RwLock::new(None)),
            status_registry: Arc::new(StatusRegistry::default()),
            device_registry: Arc::new(DeviceRegistry::empty()),
            device_bindings: Arc::new(dashmap::DashMap::new()),
            room_owners: Arc::new(dashmap::DashMap::new()),
            producer_owners: Arc::new(dashmap::DashMap::new()),
            t4_gone_seen: Arc::new(dashmap::DashMap::new()),
        }
    }

    /// G2 连接级身份（D-H11）: 返回该 peer_id 绑定到的 device_id（设备认证成功的会话）。
    /// G3 用此做角色授权；无绑定 = PSK/JWT 路径。
    pub fn device_id_of(&self, peer_id: &str) -> Option<String> {
        self.device_bindings.get(peer_id).map(|v| v.clone())
    }

    /// 当前设备绑定数（运维/测试用 — join 失败零残留的断言依据）。
    pub fn device_binding_count(&self) -> usize {
        self.device_bindings.len()
    }

    /// G3 房间主车（device_id）; 无主 = 车未上线或非车端房间。
    pub fn room_owner_of(&self, room_id: &str) -> Option<String> {
        self.room_owners.get(room_id).map(|v| v.clone())
    }

    /// G3 已登记主车的房间数（运维/测试用）。
    pub fn room_owner_count(&self) -> usize {
        self.room_owners.len()
    }

    /// Subscribe to the shutdown signal — cloned receivers are given to each
    /// WebSocket handler so they can detect when draining has been requested.
    pub fn subscribe_shutdown(&self) -> watch::Receiver<bool> {
        self.shutdown_tx.subscribe()
    }

    /// Request graceful shutdown: all active connections should close.
    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(true);
        tracing::info!(
            "Shutdown signal broadcast to {} active connections",
            self.active_connections.load(Ordering::Relaxed)
        );
    }

    /// Number of currently active WebSocket connections.
    pub fn active_connections(&self) -> usize {
        self.active_connections.load(Ordering::Relaxed)
    }

    /// Admin 事件频道（流上/下线事件 → 前端列表秒级刷新）。
    pub fn admin_events(&self) -> broadcast::Sender<String> {
        self.admin_events.clone()
    }

    pub(crate) fn get_or_create_channel(&self, room_id: &str) -> broadcast::Sender<String> {
        self.channels.entry(room_id.to_string()).or_insert_with(RoomChannel::new).tx.clone()
    }
    /// E4 云端配置下发: 向房间 Host（整车 host-agent 会话）推送 host.toml 全文。
    /// 经房间 broadcast 频道投递（target = host peer_id，接收端按 target 过滤）。
    /// 失败返回 Err —— 调用方必须打日志（C15）。
    pub fn push_config(&self, room_id: &str, config: &str, version: u64) -> Result<(), String> {
        let host_peer = self
            .room_manager
            .list_rooms()
            .iter()
            .find(|r| r.id == room_id)
            .and_then(|r| r.host.clone())
            .ok_or_else(|| format!("push_config: 房间 {room_id} 无 host"))?;
        let msg = SignalingMessage::ConfigPush {
            room_id: room_id.to_string(),
            target: host_peer.clone(),
            config: config.to_string(),
            version,
        };
        let text =
            serde_json::to_string(&msg).map_err(|e| format!("push_config: 序列化失败: {e}"))?;
        let tx = self.get_or_create_channel(room_id);
        tx.send(text).map_err(|_| format!("push_config: 房间 {room_id} 无接收者"))?;
        tracing::info!(
            room_id,
            peer = %host_peer,
            version,
            "ConfigPush 已下发（agent 应用后经 StatusReport.config_version 关联）"
        );
        Ok(())
    }
}

// ── HealthChecker impl ────────────────────────────────────────────────────

impl HealthChecker for SignalingServer {
    fn name(&self) -> &'static str {
        "signaling"
    }

    fn check_health(&self) -> HealthStatus {
        let connections = self.active_connections.load(Ordering::Relaxed);
        let rooms = self.room_manager.active_rooms();
        tracing::debug!("Health: {connections} connections, {rooms} rooms");
        // ponytail: always healthy while alive; add degraded thresholds if needed
        HealthStatus::Healthy
    }
}

impl ReadinessChecker for SignalingServer {
    /// readiness = 存活 + SFU worker 存活。liveness≠readiness：`/health` 不因 worker 死亡
    /// 降级（防外部重启风暴误杀信令）；`/ready` 503 供观测 + 人工 `msrtc-server restart`
    /// 恢复（自动看门狗另立项——frontend-process-split T7/B3）。
    fn check_readiness(&self) -> HealthStatus {
        #[cfg(feature = "sfu-mediasoup")]
        {
            if !self.sfu_manager.worker_alive() {
                return HealthStatus::unhealthy(
                    "mediasoup worker process has exited — SFU unusable until this server restarts",
                );
            }
        }
        self.check_health()
    }
}

pub fn signaling_router(server: SignalingServer) -> Router {
    Router::new().route("/ws", get(ws_handler)).with_state(server)
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    headers: HeaderMap,
    State(server): State<SignalingServer>,
) -> impl IntoResponse {
    // Extract JWT from sec-websocket-protocol header — 兼容 "Bearer <jwt>" 与纯 <jwt>
    // PIT-49: 浏览器子协议禁止空格（RFC 6455 token），只能传纯 JWT
    let jwt_token = headers
        .get("sec-websocket-protocol")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.trim().strip_prefix("Bearer ").unwrap_or(v.trim()))
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty());
    // PIT-49: 必须回显子协议（浏览器要求 Sec-WebSocket-Protocol 响应确认），
    // 否则浏览器协商失败连接被拒（admin JWT 经子协议传递）
    let client_protocols: Vec<String> = headers
        .get("sec-websocket-protocol")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.split(',').map(|p| p.trim().to_string()).collect())
        .unwrap_or_default();
    let max_size = server.ws_max_message_size;
    ws.max_message_size(max_size)
        .protocols(client_protocols)
        .on_upgrade(move |socket| handle_socket(socket, server, jwt_token))
}

/// Send a signaling message to this peer directly (not broadcast).
fn send_msg(msg: &SignalingMessage) -> Result<String, String> {
    serde_json::to_string(msg).map_err(|e| format!("serialize error: {e}"))
}

async fn handle_socket(socket: WebSocket, server: SignalingServer, jwt_token: Option<String>) {
    // Track active connection count
    server.active_connections.fetch_add(1, Ordering::Relaxed);

    // Decrement on every exit path
    struct Guard(Arc<AtomicUsize>);
    impl Drop for Guard {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::Relaxed);
        }
    }
    let _guard = Guard(Arc::clone(&server.active_connections));

    // Subscribe to shutdown signal
    let shutdown_rx = server.subscribe_shutdown();

    let (ws_sender, mut receiver) = socket.split();
    let ws_sender = Arc::new(tokio::sync::Mutex::new(ws_sender));

    let mut peer_id = uuid::Uuid::new_v4().to_string();
    tracing::info!("New connection: peer={}", peer_id);

    // PSK auth — 共享态（psk-admin-management: main.rs 注入 config/env 合并; admin 轮换热更新）
    let psk = server.psk_state.read().unwrap_or_else(|e| e.into_inner()).clone();
    let psk_auth = psk.as_ref().map(|k| SimplePskAuth::new(k.as_bytes()));
    let mut authenticated = psk_auth.is_none();
    tracing::info!("Auth: psk_set={}, authenticated={}", psk.is_some(), authenticated);

    // G3 会话身份: JWT 角色 claim 解析结果（RoomJoin 设备认证成功后覆盖为 Device）。
    let mut account_identity: Option<AccountIdentity> = None;

    // ── JWT auth (verified whenever a token is presented; PSK is fallback) ──
    // ⚠ 不能用 `if !authenticated` 拦——psk 未配置时 authenticated 预置 true，会把提交的
    // token 整个跳过：账号身份永不建立 → 会话退化为 Legacy → produce/data 域授权门被旁路
    // （2026-08-31 e2e_sfu role_enforcement/data_domain 双失败根因，测试假绿暴露）。
    if !authenticated || jwt_token.is_some() {
        if let (Some(jwt_auth), Some(token)) = (&server.jwt_auth, &jwt_token) {
            match jwt_auth.verify(token) {
                Ok(claims) => {
                    // G3 角色解析（D-H11）: 合法 role → 账号身份（舱端）; 无 role = legacy
                    // token（保持原行为）; 未知 role → 拒绝连接（4011）。
                    match claims.role.as_deref() {
                        Some(role_str) if CockpitRole::parse(role_str).is_none() => {
                            let reason = format!("token role {role_str:?} not recognized");
                            tracing::warn!("JWT auth rejected: {reason}");
                            audit::log_event(AuditEvent::AuthFailure {
                                peer_id: peer_id.clone(),
                                reason: reason.clone(),
                            });
                            let error = SignalingMessage::Error { code: 4011, message: reason };
                            let _ = ws_sender
                                .lock()
                                .await
                                .send(Message::Text(send_msg(&error).unwrap()))
                                .await;
                            return;
                        }
                        Some(role_str) => {
                            // 账号会话: peer_id 用 username + 短随机后缀（同账号多舱端并发不冲突）。
                            let role = CockpitRole::parse(role_str).expect("checked above");
                            account_identity = Some(AccountIdentity {
                                username: claims.sub.clone(),
                                role,
                                vehicles: claims.vehicles.clone().unwrap_or_default(),
                            });
                            peer_id = format!(
                                "{}-{}",
                                claims.sub,
                                &uuid::Uuid::new_v4().to_string()[..8]
                            );
                            authenticated = true;
                            tracing::info!(
                                "JWT account authenticated: user={} role={}",
                                claims.sub,
                                role_str
                            );
                            audit::log_event(AuditEvent::AuthSuccess {
                                peer_id: peer_id.clone(),
                                device_id: None,
                            });
                        }
                        None => {
                            peer_id = claims.sub.clone();
                            authenticated = true;
                            tracing::info!("JWT authenticated: peer={}", peer_id);
                            audit::log_event(AuditEvent::AuthSuccess {
                                peer_id: peer_id.clone(),
                                device_id: None,
                            });
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("JWT verification failed: {}, falling back to PSK", e);
                }
            }
        }
    }

    // ── PSK auth (fallback) ───────────────────────────────────────────
    if !authenticated {
        tracing::info!("Auth: waiting for PSK...");
        match receiver.next().await {
            Some(Ok(Message::Text(text))) => {
                if let Some(ref a) = psk_auth
                    && (a.sign(peer_id.as_bytes()) == a.sign(text.as_bytes())
                        || text == psk.as_deref().unwrap_or(""))
                {
                    authenticated = true;
                    tracing::info!("Peer {} authenticated via PSK", peer_id);
                    audit::log_event(AuditEvent::AuthSuccess {
                        peer_id: peer_id.clone(),
                        device_id: None,
                    });
                }
                if !authenticated {
                    let error = SignalingMessage::Error {
                        code: 4003,
                        message: "PSK authentication failed".into(),
                    };
                    let _ =
                        ws_sender.lock().await.send(Message::Text(send_msg(&error).unwrap())).await;
                    audit::log_event(AuditEvent::AuthFailure {
                        peer_id: peer_id.clone(),
                        reason: "PSK authentication failed".into(),
                    });
                    return;
                }
            }
            _ => {
                let error = SignalingMessage::Error {
                    code: 4003,
                    message: "Authentication required".into(),
                };
                let _ = ws_sender.lock().await.send(Message::Text(send_msg(&error).unwrap())).await;
                audit::log_event(AuditEvent::AuthFailure {
                    peer_id: peer_id.clone(),
                    reason: "Authentication required".into(),
                });
                return;
            }
        }
    }

    // Always send auth ack (or skip if no auth required)
    let ack = SignalingMessage::Error { code: 0, message: "authenticated".into() };
    let _ = ws_sender.lock().await.send(Message::Text(send_msg(&ack).unwrap())).await;
    tracing::info!("Auth ack sent, entering RoomJoin phase");

    // Phase 2: RoomJoin
    let (room_id, role, device_id, session_identity) = loop {
        // Check for shutdown during RoomJoin
        if *shutdown_rx.borrow() {
            tracing::info!("Shutdown requested during RoomJoin for peer {}", peer_id);
            return;
        }

        tracing::debug!("RoomJoin: waiting for message...");
        match receiver.next().await {
            Some(Ok(Message::Text(text))) => {
                let text_str = text.to_string();
                if let Ok(SignalingMessage::RoomJoin {
                    room_id,
                    peer_role,
                    device_id,
                    device_secret,
                    ..
                }) = serde_json::from_str(&text_str)
                {
                    // ── G2 设备认证（D-H11; 错误码 4010 单一家族防设备枚举）──────
                    // 双缺 = PSK 路径（保持原流程）; 半带/失败 = Error 4010 后断开。
                    match devices::authenticate(
                        &server.device_registry,
                        device_id.as_deref(),
                        device_secret.as_deref(),
                    ) {
                        None => {
                            // G3: 无设备凭证 → PSK 路径; 身份 = 账号（JWT）或 legacy。
                            let identity = account_identity
                                .map(SessionIdentity::Account)
                                .unwrap_or(SessionIdentity::Legacy);
                            break (room_id, peer_role, None, identity);
                        }
                        Some(Err(auth_err)) => {
                            // review #1: wire 消息统一（防枚举），但日志/审计保留内部区分
                            // （Unknown vs BadSecret — 运维可辨别，客户端不可探测）。
                            tracing::warn!(
                                "Peer {} device auth failed: {:?} (wire: {})",
                                peer_id,
                                auth_err,
                                auth_err.message()
                            );
                            audit::log_event(AuditEvent::AuthFailure {
                                peer_id: peer_id.clone(),
                                reason: format!(
                                    "device auth failed (device={:?}): {:?}",
                                    device_id, auth_err
                                ),
                            });
                            let error = SignalingMessage::Error {
                                code: 4010,
                                message: auth_err.message().into(),
                            };
                            let _ = ws_sender
                                .lock()
                                .await
                                .send(Message::Text(send_msg(&error).unwrap()))
                                .await;
                            return;
                        }
                        Some(Ok(())) => {
                            // 认证通过: 记审计，连接级身份在 join 成功后绑定。
                            let device = device_id.unwrap_or_default();
                            audit::log_event(AuditEvent::AuthSuccess {
                                peer_id: peer_id.clone(),
                                device_id: Some(device.clone()),
                            });
                            tracing::info!("Peer {} device-authenticated as {}", peer_id, device);
                            break (
                                room_id,
                                peer_role,
                                Some(device.clone()),
                                SessionIdentity::Device(device),
                            );
                        }
                    }
                }
            }
            Some(Ok(Message::Close(_))) | None => return,
            _ => continue,
        }
    };

    // ── G3 RoomJoin 门（D-H11 矩阵 + 租户隔离）──────────────────────────
    // ① 账号会话禁止以 Host 角色入房（Host = 车端位，防账号抢占房间使车无法上线）。
    if role == PeerRole::Host
        && let Some(reason) = session_identity.host_join_denied()
    {
        let detail = format!("{reason} (peer={peer_id})");
        tracing::warn!("RoomJoin denied: {detail}");
        audit::log_event(AuditEvent::AuthorizationDenied {
            action: "room_join".into(),
            peer_id: peer_id.clone(),
            detail: detail.clone(),
        });
        let error = SignalingMessage::Error { code: 4031, message: detail };
        let _ = ws_sender.lock().await.send(Message::Text(send_msg(&error).unwrap())).await;
        return;
    }
    // ② Remote 角色 = P2P 控制协商位（I1 review）: 无控制能力者禁止 —
    // viewer/dispatcher 可拉流（SFU）但不得占控制位（同时防 DoS: 占单 Remote 槽
    // 使合法 operator 无法协商控制）。
    if role == PeerRole::Remote && !session_identity.can_control() {
        let detail = format!(
            "role lacks control capability for remote/P2P negotiation (peer={peer_id}, room={room_id})"
        );
        tracing::warn!("RoomJoin denied: {detail}");
        audit::log_event(AuditEvent::AuthorizationDenied {
            action: "room_join".into(),
            peer_id: peer_id.clone(),
            detail: detail.clone(),
        });
        let error = SignalingMessage::Error { code: 4031, message: detail };
        let _ = ws_sender.lock().await.send(Message::Text(send_msg(&error).unwrap())).await;
        return;
    }
    // ③ 按房间主车做矩阵 + 白名单校验（车 A 不可见车 B; 账号仅授权车）。
    if let Some(owner) = server.room_owner_of(&room_id) {
        if let Some(reason) = session_identity.join_vehicle_room(Some(&owner)) {
            let detail = format!("{reason} (peer={peer_id}, room={room_id})");
            tracing::warn!("RoomJoin denied: {detail}");
            audit::log_event(AuditEvent::AuthorizationDenied {
                action: "room_join".into(),
                peer_id: peer_id.clone(),
                detail: detail.clone(),
            });
            let error = SignalingMessage::Error { code: 4031, message: detail };
            let _ = ws_sender.lock().await.send(Message::Text(send_msg(&error).unwrap())).await;
            return;
        }
    }

    // Join the room
    match server.room_manager.join_room(&room_id, &peer_id, &role) {
        Ok(()) => {
            // G2: 设备认证成功的会话在此绑定连接级身份（peer_id → device_id, D-H11）。
            // review #2: 绑定必须发生在 join 成功之后 — 失败路径（4001/4002）零残留;
            // 断开时 cleanup 解除（见 relay 循环结束处）。
            if let Some(device) = &device_id {
                server.device_bindings.insert(peer_id.clone(), device.clone());
                // G3: 车端 join 成功即登记房间主车（租户隔离/授权的裁决依据）。
                // 注: 存的是设备 ID（= 车辆 ID）。音频房间 room_id=audio-<vehicle>，
                // 设备 ID 即 vehicle — join_vehicle_room 按它比对 allowlist，天然正确。
                server.room_owners.insert(room_id.clone(), device.clone());
            }
            audit::log_event(AuditEvent::PeerJoin {
                peer_id: peer_id.clone(),
                room_id: room_id.clone(),
                role: format!("{:?}", role),
            });
        }
        Err(CoreError::RoomFull) => {
            let error = SignalingMessage::Error { code: 4002, message: "Room is full".into() };
            let _ = ws_sender.lock().await.send(Message::Text(send_msg(&error).unwrap())).await;
            return;
        }
        Err(e) => {
            tracing::error!("Room join error: {}", e);
            let error = SignalingMessage::Error {
                code: 4001,
                message: format!("Failed to join room: {}", e),
            };
            let _ = ws_sender.lock().await.send(Message::Text(send_msg(&error).unwrap())).await;
            return;
        }
    }

    // Send RoomJoined ack
    let ack = SignalingMessage::RoomJoined { room_id: room_id.clone(), peer_id: peer_id.clone() };
    let _ = ws_sender.lock().await.send(Message::Text(send_msg(&ack).unwrap())).await;

    // ── Replay cached SDP offer + ICE candidates for late joiners ────────
    if let Some(cached) = server.pending_messages.get(&room_id) {
        let sender = Arc::clone(&ws_sender);
        let count = cached.len();
        for msg in cached.iter() {
            let _ = sender.lock().await.send(Message::Text(msg.clone())).await;
        }
        tracing::info!("Replayed {} cached messages for room {}", count, room_id);
    }

    let tx = server.get_or_create_channel(&room_id);
    let mut rx = tx.subscribe();

    // Phase 3: Message relay
    let relay_peer_id = peer_id.clone();
    let relay_room = room_id.clone();

    // Clone ws_sender for direct responses (SFU + G3 授权拒绝回复) and relay
    let direct_sender = Arc::clone(&ws_sender);
    let relay_sender = ws_sender;

    tracing::info!("Relay spawned: peer={} room={}", relay_peer_id, relay_room);
    let relay_spawn_peer = relay_peer_id.clone();
    let relay_spawn_room = relay_room.clone();
    let relay_handle = tokio::spawn(async move {
        let reason = loop {
            match rx.recv().await {
                Ok(msg) => {
                    tracing::debug!("Relay: forwarding to peer {} ({} bytes)", relay_spawn_peer, msg.len());
                    if relay_sender.lock().await.send(Message::Text(msg)).await.is_err() {
                        break "send-failed";
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!("Relay {}: lagged {} messages", relay_spawn_peer, n);
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    break "channel-closed";
                }
            }
        };
        tracing::info!("Relay ended: peer={} room={} reason={}", relay_spawn_peer, relay_spawn_room, reason);
    });

    // Forward: this peer's receiver → broadcast
    tracing::info!("Entering forward loop for peer {}", relay_peer_id);

    // Send existing producers to new consumer (late-joiner sync)
    #[cfg(feature = "sfu-mediasoup")]
    {
        tracing::info!("SFU: list_producers for room {}", relay_room);
        if let Some(created) = server.sfu_manager.list_producers(&relay_room) {
            tracing::info!("SFU: found {} producers in room {}", created.len(), relay_room);
            for (producer_id, kind, peer_id) in &created {
                let msg = SignalingMessage::NewProducer {
                    room_id: relay_room.clone(),
                    producer_id: producer_id.clone(),
                    peer_id: peer_id.clone(),
                    kind: kind.clone(),
                };
                let _ =
                    direct_sender.lock().await.send(Message::Text(send_msg(&msg).unwrap())).await;
            }
        }
    }

    // H1: Send existing data producers to new peer (late-joiner sync, mirror above)
    #[cfg(feature = "sfu-mediasoup")]
    {
        if let Some(created) = server.sfu_manager.list_data_producers(&relay_room) {
            tracing::info!("SFU: found {} data producers in room {}", created.len(), relay_room);
            for (data_producer_id, label, peer_id) in &created {
                let msg = SignalingMessage::NewDataProducer {
                    room_id: relay_room.clone(),
                    data_producer_id: data_producer_id.clone(),
                    peer_id: peer_id.clone(),
                    label: label.clone(),
                    protocol: String::new(),
                };
                let _ =
                    direct_sender.lock().await.send(Message::Text(send_msg(&msg).unwrap())).await;
            }
        }
    }

    while let Some(Ok(msg)) = receiver.next().await {
        // Check shutdown signal before processing each message
        if *shutdown_rx.borrow() {
            tracing::info!("Shutdown requested, disconnecting peer {}", relay_peer_id);
            // Notify room peers
            let leave_msg = SignalingMessage::RoomLeave {
                room_id: relay_room.clone(),
                peer_id: relay_peer_id.clone(),
            };
            let _ = tx.send(serde_json::to_string(&leave_msg).unwrap());
            break;
        }

        match msg {
            Message::Text(text) => {
                let text_str = text.to_string();

                // Handle RoomLeave — relay to peers then disconnect (cleanup in disconnect path)
                if let Ok(sig) = serde_json::from_str::<SignalingMessage>(&text_str)
                    && matches!(sig, SignalingMessage::RoomLeave { .. })
                {
                    let _ = tx.send(text_str);
                    break;
                }

                // v4 (E3): 整车状态上报 — Server 直接消费存储（非 relay 消息，
                // 不广播房间；旧 Server 解析失败静默丢弃 = 可容忍，周期性上报自愈）
                // I3 review: 身份门 — 仅 Device 会话或 Host 角色可上报；其他拒绝 + 审计（C15）。
                if let Ok(sig) = serde_json::from_str::<SignalingMessage>(&text_str) {
                    let is_report = matches!(&sig, SignalingMessage::StatusReport { .. });
                    if is_report {
                        if let Some(error) = status_report_denial(
                            &session_identity,
                            &role,
                            &relay_peer_id,
                            &relay_room,
                        ) {
                            let _ = direct_sender
                                .lock()
                                .await
                                .send(Message::Text(send_msg(&error).unwrap()))
                                .await;
                            continue;
                        }
                        // 列表秒级刷新: streams 集合变化 → 推 admin 事件（前端仅作
                        // 刷新触发；5s 轮询保留兜底）。
                        if let SignalingMessage::StatusReport { streams, .. } = &sig {
                            let new_ids: std::collections::BTreeSet<&str> =
                                streams.iter().map(|s| s.id.as_str()).collect();
                            let changed = match server.status_registry.get(&relay_room) {
                                Some(SignalingMessage::StatusReport { streams: old_s, .. }) => {
                                    let old_ids: std::collections::BTreeSet<&str> =
                                        old_s.iter().map(|s| s.id.as_str()).collect();
                                    old_ids != new_ids
                                }
                                _ => !new_ids.is_empty(),
                            };
                            if changed {
                                for s in streams {
                                    let ev = crate::admin::AdminEvent::StreamCreate {
                                        device_id: relay_room.clone(),
                                        stream_id: s.id.clone(),
                                        timestamp: chrono::Utc::now().to_rfc3339(),
                                    };
                                    let _ = server
                                        .admin_events
                                        .send(serde_json::to_string(&ev).unwrap_or_default());
                                }
                                tracing::info!(
                                    "StatusReport streams changed in room {} — admin event pushed",
                                    relay_room
                                );
                            }
                        }
                        server.status_registry.store(&relay_room, sig);
                        tracing::info!("StatusReport stored for room {relay_room}");
                        continue;
                    }
                }

                // ── G3 急停命令（强审计转发）: operator/admin + 该车访问权 → 审计 + 转发车端 ──
                if let Ok(sig) = serde_json::from_str::<SignalingMessage>(&text_str) {
                    if let SignalingMessage::EmergencyCommand { command, .. } = &sig {
                        let owner = server.room_owner_of(&relay_room);
                        match session_identity.can_emergency(owner.as_deref()) {
                            Ok(()) => {
                                // 强审计: 谁/何时(tracing 时间戳)/哪个车/什么命令
                                let (username, role) = match &session_identity {
                                    SessionIdentity::Account(a) => {
                                        (a.username.clone(), a.role.as_str().to_string())
                                    }
                                    _ => (relay_peer_id.clone(), "legacy".into()),
                                };
                                audit::log_event(AuditEvent::EmergencyCommand {
                                    username,
                                    role,
                                    vehicle: owner.clone().unwrap_or_default(),
                                    command: command.clone(),
                                });
                                tracing::info!(
                                    "G3: emergency command relayed to room {relay_room}"
                                );
                                let _ = tx.send(text_str.clone());
                            }
                            Err(reason) => {
                                let detail =
                                    format!("{reason} (peer={relay_peer_id}, room={relay_room})");
                                tracing::warn!("Emergency denied: {detail}");
                                audit::log_event(AuditEvent::AuthorizationDenied {
                                    action: "emergency".into(),
                                    peer_id: relay_peer_id.clone(),
                                    detail: detail.clone(),
                                });
                                let error = SignalingMessage::Error { code: 4031, message: detail };
                                let _ = direct_sender
                                    .lock()
                                    .await
                                    .send(Message::Text(send_msg(&error).unwrap()))
                                    .await;
                            }
                        }
                        continue;
                    }
                    // ── G3 ConfigPush: server 单向下发（admin REST）; 客户端入站一律拒绝 ──
                    if let SignalingMessage::ConfigPush { .. } = &sig {
                        let detail =
                            format!("config push is server-initiated only (peer={relay_peer_id})");
                        tracing::warn!("ConfigPush inbound rejected: {detail}");
                        audit::log_event(AuditEvent::AuthorizationDenied {
                            action: "config_push".into(),
                            peer_id: relay_peer_id.clone(),
                            detail,
                        });
                        let error = SignalingMessage::Error {
                            code: 4031,
                            message: "config push is server-initiated only".into(),
                        };
                        let _ = direct_sender
                            .lock()
                            .await
                            .send(Message::Text(send_msg(&error).unwrap()))
                            .await;
                        continue;
                    }
                }

                // Check for SFU transport messages (server-side handling)
                #[cfg(feature = "sfu-mediasoup")]
                {
                    tracing::debug!(
                        "SFU check: parsing message: {}",
                        &text_str[..text_str.len().min(200)]
                    );
                    if let Ok(sig_msg) = serde_json::from_str::<SignalingMessage>(&text_str) {
                        tracing::debug!("SFU check: parsed OK, calling handle_sfu_message");
                        if let Some(response) = handle_sfu_message(
                            &sig_msg,
                            &server,
                            &tx,
                            &relay_peer_id,
                            &session_identity,
                        )
                        .await
                        {
                            // Send SFU response directly via WS sender
                            let _ = direct_sender
                                .lock()
                                .await
                                .send(Message::Text(send_msg(&response).unwrap()))
                                .await;
                            tracing::info!("SFU: response sent to peer {}", relay_peer_id);
                            continue; // Handled by SFU, don't relay
                        }
                    } else {
                        tracing::debug!(
                            "SFU check: parse FAILED for: {}",
                            &text_str[..text_str.len().min(100)]
                        );
                    }
                }

                // Try SignalingMessage first, then raw JSON for Frame
                let should_relay = match serde_json::from_str::<SignalingMessage>(&text_str) {
                    Ok(sig_msg) => matches!(
                        sig_msg,
                        SignalingMessage::Sdp { .. }
                            | SignalingMessage::RTCIceCandidate { .. }
                            | SignalingMessage::Frame { .. }
                            | SignalingMessage::EncoderStatus { .. } // v2: web-stream-stats T3
                    ),
                    Err(e) => {
                        // v2 (web-stream-stats 双审 HIGH): 解析失败消息（如旧版未知变体）静默丢弃 —
                        // C15 要求错误路径打日志; 限频防刷屏
                        if let Ok(raw) = serde_json::from_str::<serde_json::Value>(&text_str) {
                            let is_frame =
                                raw.get("type").and_then(|v| v.as_str()) == Some("frame");
                            if !is_frame {
                                tracing::warn!(
                                    "signaling: unparsable message dropped (len={}): {e}",
                                    text_str.len()
                                );
                            }
                            is_frame
                        } else {
                            tracing::warn!(
                                "signaling: non-JSON message dropped (len={})",
                                text_str.len()
                            );
                            false
                        }
                    }
                };
                if should_relay {
                    // ── DeviceStream filter: only relay Frame, skip SDP/ICE from consumers ──
                    // I1 review: P2P 房间（非 DeviceStream）的 SDP/ICE = 控制协商载体 —
                    // 无控制能力账号（viewer/dispatcher）即使以 Consumer 入房也拦截其中继
                    // （Consumer 只走 SFU 媒体，协商是消息驱动，不需要 SDP 中继）。
                    let is_device_room = server.room_manager.is_device_stream(&relay_room);
                    if !is_device_room && !session_identity.can_control() {
                        let is_negotiation = serde_json::from_str::<SignalingMessage>(&text_str)
                            .map(|m| {
                                matches!(
                                    m,
                                    SignalingMessage::Sdp { .. }
                                        | SignalingMessage::RTCIceCandidate { .. }
                                )
                            })
                            .unwrap_or(false);
                        if is_negotiation {
                            let detail = format!(
                                "control negotiation (SDP/ICE) requires control capability (peer={relay_peer_id}, room={relay_room})"
                            );
                            tracing::warn!("SDP relay denied: {detail}");
                            audit::log_event(AuditEvent::AuthorizationDenied {
                                action: "control_negotiation".into(),
                                peer_id: relay_peer_id.clone(),
                                detail,
                            });
                            continue;
                        }
                    }
                    if is_device_room {
                        let is_frame = if let Ok(sig_msg) =
                            serde_json::from_str::<SignalingMessage>(&text_str)
                        {
                            // v2: encoder_status 放行（编码诊断, web-stream-stats T3 Oracle F3）
                            matches!(
                                sig_msg,
                                SignalingMessage::Frame { .. }
                                    | SignalingMessage::EncoderStatus { .. }
                            )
                        } else if let Ok(raw) = serde_json::from_str::<serde_json::Value>(&text_str)
                        {
                            raw.get("type").and_then(|v| v.as_str()) == Some("frame")
                        } else {
                            false
                        };
                        if !is_frame {
                            tracing::debug!("DeviceStream: dropping non-Frame message");
                            continue;
                        }
                    }

                    // 广播频道路由：EncoderStatus 按 streamer 声明的子房间（浏览器消费者在
                    // 每流子房间频道），其余走整车会话频道（relay_room）。
                    let out_tx = relay_target_room(&server, &text_str, &relay_room);
                    match out_tx.send(text_str.clone()) {
                        Ok(n) => {
                            tracing::debug!("Forward: broadcast to {} receivers", n);
                            // ── Cache SDP + ICE for late-joiner replay
                            if let Ok(sig_msg) = serde_json::from_str::<SignalingMessage>(&text_str)
                            {
                                if matches!(
                                    sig_msg,
                                    SignalingMessage::Sdp { .. }
                                        | SignalingMessage::RTCIceCandidate { .. }
                                ) {
                                    let mut msgs = server
                                        .pending_messages
                                        .entry(relay_room.clone())
                                        .or_default();
                                    msgs.push(text_str);
                                    // ponytail: cap at 64 messages; real ring-buffer if this overflows
                                    if msgs.len() > 64 {
                                        msgs.remove(0);
                                    }
                                }
                            }
                        }
                        Err(tokio::sync::broadcast::error::SendError(_)) => {
                            tracing::warn!("Forward: no receivers, message dropped");
                        }
                    }
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }

    relay_handle.abort();

    // Clean up SFU resources for the disconnecting peer
    #[cfg(feature = "sfu-mediasoup")]
    {
        // H1 修正: producer 注册在各 stream 房间的 sfu peers（relay_room=整车房间 ≠ 其房间，
        // 单房间 remove_peer 清不到——死 producer 泄漏）。跨房间移除并逐房间广播 ProducerClosed。
        let binding_dev = server.device_bindings.get(&relay_peer_id).map(|e| e.value().clone());
        let closed_rooms = server.sfu_manager.remove_peer_global(&relay_peer_id);
        // T4 配套：全局清理已广播过 = 名下实体已见光，反查空手不再当异常（防常态误报）。
        let global_announced: usize = closed_rooms.iter().map(|(_, c)| c.len()).sum();
        tracing::info!(
            "SFU: cleaned up peer {} ({} sfu rooms touched)",
            relay_peer_id, closed_rooms.len()
        );
        for (room_id, closed) in closed_rooms {
            // T4 重构：广播/owners 同步/列表事件统一链（防多站点漂移）。
            announce_producers_closed(
                &server, &room_id, &relay_peer_id, binding_dev.as_deref(), closed,
            );
        }

        let t4_seen = server.t4_gone_seen.remove(&relay_peer_id).map(|(_, n)| n).unwrap_or(0);
        // H1 修正（方案 A）: device 持有的 producer 存于各 stream 房间自报 peer 键，
        // 会话 id 清理漏删 → producer_owners 反查该设备全部 producer 移除，
        // 逐房间广播 ProducerClosed（web 免刷新重订阅）。
        if let Some(device) = server.device_bindings.get(&relay_peer_id) {
            let device_id = device.value().clone();
            drop(device);
            let owned: Vec<String> = server
                .producer_owners
                .iter()
                .filter(|e| *e.value() == device_id)
                .map(|e| e.key().clone())
                .collect();
            if !owned.is_empty() {
                let removed = server.sfu_manager.remove_producers_by_ids(&owned);
                // T2: owned−removed 差集 = 幽灵登记（owners 表有、SFU 房间无实体）——逐条 WARN。
                let removed_ids: std::collections::HashSet<String> =
                    removed.iter().map(|(_, p, _)| p.clone()).collect();
                for pid in owned.iter().filter(|p| !removed_ids.contains(*p)) {
                    tracing::warn!("remove_producers_by_ids: missed producer {pid} (device {device_id})");
                }
                for pid in &owned {
                    server.producer_owners.remove(pid);
                }
                let mut by_room: std::collections::HashMap<
                    String,
                    Vec<(String, mediaservo_common::protocol::MediaKind)>,
                > = std::collections::HashMap::new();
                for (room, pid, kind) in removed {
                    by_room.entry(room).or_default().push((pid, kind));
                }
                for (room, prods) in by_room {
                    announce_producers_closed(
                        &server, &room, &relay_peer_id, Some(&device_id), prods,
                    );
                }
            } else if global_announced == 0 && t4_seen == 0 {
                // T2: 设备断开但名下零登记且全局清理无收获 = 注册链断（05:43 类悬案禁止静默）。
                // DownstreamGone 已逐流清理的正常路径（global_announced>0 或 T4 逐流先到）→ debug。
                tracing::warn!("ProducerClosed cleanup: owned empty for device {device_id} (peer {relay_peer_id})");
            } else {
                tracing::debug!("ProducerClosed cleanup: owned empty, {global_announced} global / {t4_seen} t4 announced (device {device_id})");
            }
        }
    }

    // ── Check if leaving peer is a DeviceStream host (before leave_room removes it)
    let is_device_stream_host = server.room_manager.is_device_stream(&relay_room)
        && server
            .room_manager
            .list_rooms()
            .iter()
            .find(|r| r.id == relay_room)
            .and_then(|r| r.host.as_deref())
            == Some(&relay_peer_id);

    #[allow(unused_variables)]
    let room_removed = server.room_manager.leave_room(&relay_room, &relay_peer_id);

    // G2: 断开即解除连接级身份绑定（peer_id → device_id 仅连接存活期间有效）。
    if let Some(device) = server.device_bindings.remove(&relay_peer_id) {
        tracing::info!("Peer {} disconnected, released device binding {}", relay_peer_id, device.1);
    }
    audit::log_event(AuditEvent::PeerLeave {
        peer_id: relay_peer_id.clone(),
        room_id: relay_room.clone(),
    });

    // ── DeviceStream host disconnect: drop all consumers
    if is_device_stream_host {
        tracing::info!("DeviceStream host {} disconnected, disconnecting consumers", relay_peer_id);
        let consumers = server.room_manager.disconnect_consumers(&relay_room);
        for consumer_id in &consumers {
            audit::log_event(AuditEvent::PeerLeave {
                peer_id: consumer_id.clone(),
                room_id: relay_room.clone(),
            });
            tracing::info!("DeviceStream consumer {} removed (host left)", consumer_id);
        }
        if !consumers.is_empty() {
            tracing::info!("Disconnected {} consumers from room {}", consumers.len(), relay_room);
        }
    }

    // If room became empty, also remove it from SFU
    #[cfg(feature = "sfu-mediasoup")]
    if room_removed {
        server.sfu_manager.remove_room(&relay_room);
    }
    // E3: 空房无状态可报 — 清理状态注册表，避免陈旧数据悬挂
    if room_removed {
        server.status_registry.remove(&relay_room);
        // G3: 房间空 = 主车登记失效（车端离开且无消费方）。
        server.room_owners.remove(&relay_room);
    }

    let leave_msg =
        SignalingMessage::RoomLeave { room_id: relay_room.clone(), peer_id: relay_peer_id.clone() };
    let _ = tx.send(serde_json::to_string(&leave_msg).unwrap());

    tracing::info!("Peer {} disconnected from room {}", relay_peer_id, relay_room);
}

/// I3 review: StatusReport 身份门判定 — 仅 Device 会话（车端）或 Host 角色（PSK 车端）
/// 可上报整车状态；账号/其他会话拒绝（舱端不可伪造车端状态）。返回拒绝原因（None = 允许）。
fn status_report_denial_reason(
    identity: &SessionIdentity,
    role: &PeerRole,
) -> Option<&'static str> {
    if matches!(identity, SessionIdentity::Device(_)) || *role == PeerRole::Host {
        None
    } else {
        Some("status reports require a device session or host role")
    }
}

/// I3 review: StatusReport 门 + 审计（C15）— 拒绝时返回 Error 4031 响应（调用方发送），
/// 并记录 AuthorizationDenied（谁/动作/详情）。
fn status_report_denial(
    identity: &SessionIdentity,
    role: &PeerRole,
    peer_id: &str,
    room: &str,
) -> Option<SignalingMessage> {
    let reason = status_report_denial_reason(identity, role)?;
    let detail = format!("{reason} (peer={peer_id}, room={room})");
    tracing::warn!("StatusReport denied: {detail}");
    audit::log_event(AuditEvent::AuthorizationDenied {
        action: "status_report".into(),
        peer_id: peer_id.to_string(),
        detail: detail.clone(),
    });
    Some(SignalingMessage::Error { code: 4031, message: detail })
}

/// Handle SFU transport negotiation and produce/consume messages.
/// Returns the response message to send, or None if not handled.
/// Caller is responsible for sending the response (avoids sender lock contention with relay loop).
#[cfg(feature = "sfu-mediasoup")]
pub(crate) async fn handle_sfu_message(
    msg: &SignalingMessage,
    server: &SignalingServer,
    _broadcast_tx: &tokio::sync::broadcast::Sender<String>,
    peer_id: &str,
    identity: &SessionIdentity,
) -> Option<SignalingMessage> {
    let sfu = &server.sfu_manager;
    match msg {
        SignalingMessage::CreateWebRtcTransport { room_id, peer_id: msg_peer_id, direction } => {
            // PIT-65: 用消息 peer_id (每网页唯一 sfuPeerId), 非 session relay_peer_id
            let sfu_peer_id = msg_peer_id.as_str();
            tracing::info!(
                "SFU: creating {} transport for peer {} in room {}",
                serde_json::to_string(direction).unwrap_or_default(),
                sfu_peer_id,
                room_id
            );
            let dir_str = match direction {
                mediaservo_common::protocol::TransportDirection::Send => "send",
                mediaservo_common::protocol::TransportDirection::Recv => "recv",
            };
            match tokio::time::timeout(
                std::time::Duration::from_secs(5),
                sfu.create_webrtc_transport(room_id, sfu_peer_id, dir_str),
            )
            .await
            {
                Ok(Ok(created)) => Some(SignalingMessage::WebRtcTransportCreated {
                    room_id: room_id.clone(),
                    peer_id: peer_id.to_string(),
                    transport_id: created.transport_id,
                    ice_parameters: created.ice_parameters,
                    dtls_parameters: created.dtls_parameters,
                    ice_candidates: Some(created.ice_candidates),
                }),
                Ok(Err(e)) => {
                    tracing::error!("SFU: create transport failed: {e}");
                    Some(SignalingMessage::Error {
                        code: 5000,
                        message: format!("Transport creation failed: {e}"),
                    })
                }
                Err(_) => {
                    tracing::error!("SFU: create transport timed out after 5s");
                    Some(SignalingMessage::Error {
                        code: 5000,
                        message: "Transport creation timed out".into(),
                    })
                }
            }
        }
        SignalingMessage::ConnectWebRtcTransport {
            room_id,
            peer_id: msg_peer_id,
            transport_id,
            dtls_parameters,
        } => {
            // PIT-65: 用消息 peer_id — 与 create/consume 一致
            let sfu_peer_id = msg_peer_id.as_str();
            match sfu
                .connect_transport(&room_id, sfu_peer_id, &transport_id, dtls_parameters.clone())
                .await
            {
                Ok(()) => {
                    tracing::info!(
                        "SFU: transport {transport_id} connected for peer {sfu_peer_id}"
                    );
                    Some(SignalingMessage::Error { code: 0, message: "transport_connected".into() })
                }
                Err(e) => {
                    tracing::error!("SFU: connect transport failed: {e}");
                    Some(SignalingMessage::Error {
                        code: 5000,
                        message: format!("Connect failed: {e}"),
                    })
                }
            }
        }
        SignalingMessage::Produce {
            room_id,
            peer_id: msg_peer_id,
            transport_direction,
            kind,
            rtp_parameters,
            transport_id,
        } => {
            // PIT-65: 用消息 peer_id — 与 create/connect/consume 一致
            let sfu_peer_id = msg_peer_id.as_str();
            tracing::info!(
                "SFU: Produce received room={} kind={:?} dir={:?} rtp={}",
                room_id,
                kind,
                transport_direction,
                rtp_parameters
            );
            if !matches!(transport_direction, mediaservo_common::protocol::TransportDirection::Send)
            {
                return Some(SignalingMessage::Error {
                    code: 4000,
                    message: "Produce requires send transport".into(),
                });
            }
            // G3 门: 车端自动允许（自己的流）; 账号禁止 produce（舱端只消费）; legacy 放行。
            if let Err(reason) = identity.can_produce() {
                let detail = format!("{reason} (peer={peer_id}, room={room_id})");
                tracing::warn!("Produce denied: {detail}");
                audit::log_event(AuditEvent::AuthorizationDenied {
                    action: "produce".into(),
                    peer_id: peer_id.to_string(),
                    detail: detail.clone(),
                });
                return Some(SignalingMessage::Error { code: 4031, message: detail });
            }
            // H2: 音频房间只允许 audio producer（全互连 opus 会议语义）— 4031 + 审计（C15）。
            if crate::sfu::is_audio_room(room_id)
                && *kind != mediaservo_common::protocol::MediaKind::Audio
            {
                let detail = format!(
                    "audio rooms allow audio producers only (peer={peer_id}, room={room_id}, kind={kind:?})"
                );
                tracing::warn!("Produce denied: {detail}");
                audit::log_event(AuditEvent::AuthorizationDenied {
                    action: "produce".into(),
                    peer_id: peer_id.to_string(),
                    detail: detail.clone(),
                });
                return Some(SignalingMessage::Error { code: 4031, message: detail });
            }
            match sfu
                .create_producer(
                    room_id,
                    sfu_peer_id,
                    kind,
                    rtp_parameters.clone(),
                    transport_id.as_deref(),
                )
                .await
            {
                Ok(result) => {
                    // G3: 车端 producer 登记所属设备（consume 授权纵深防御）。
                    if let Some(device) = server.device_id_of(peer_id) {
                        server.producer_owners.insert(result.producer_id.clone(), device);
                    } else {
                        // T2: produce 会话缺设备绑定 → owners 断链、agent 断开反查必漏（响亮告警）。
                        tracing::warn!("producer_owners: device binding absent on produce (session={peer_id}, room={room_id})");
                    }
                    // Broadcast NewProducer to all peers in room
                    let broadcast = SignalingMessage::NewProducer {
                        room_id: room_id.clone(),
                        producer_id: result.producer_id.clone(),
                        peer_id: peer_id.to_string(),
                        kind: result.kind,
                    };
                    // H1 根因修复: 广播必须发到 produce 目标房间的频道（而非调用会话自己的
                    // 频道——agent 会话房间=vehicle，web 消费者订阅的是各流房间 → 旧路径
                    // web 永远收不到新 producer → host 重启后黑屏只能刷新恢复）。
                    let room_tx = server.get_or_create_channel(&room_id);
                    match room_tx.send(serde_json::to_string(&broadcast).unwrap()) {
                        Ok(n) => tracing::info!("NewProducer broadcast: {} channel receivers in room {}", n, room_id),
                        Err(_) => tracing::warn!("NewProducer broadcast: no receivers in room {}", room_id),
                    }
                    tracing::info!(
                        "SFU: broadcast NewProducer for peer {} in room {}",
                        peer_id,
                        room_id
                    );
                    Some(SignalingMessage::Produced {
                        room_id: room_id.clone(),
                        producer_id: result.producer_id,
                    })
                }
                Err(e) => {
                    tracing::error!("SFU: Producer creation failed: {e}");
                    Some(SignalingMessage::Error {
                        code: 5000,
                        message: format!("Producer creation failed: {e}"),
                    })
                }
            }
        }
        SignalingMessage::Consume {
            room_id,
            peer_id: msg_peer_id,
            producer_id,
            rtp_capabilities,
            transport_id,
        } => {
            // PIT-65: 用消息 peer_id (每网页唯一 sfuPeerId), 非 session relay_peer_id —
            // 否则多网页共享 admin → recv_transport 互相覆盖 → consumer 挂错 transport → 黑屏
            let sfu_peer_id = msg_peer_id.as_str();
            // G3 门: 账号只能 consume 有权车的 producer（RoomJoin 已按房间主车过滤 —
            // 此处对具体 producer 纵深防御，防房间内混入他车流）。
            let producer_owner = server.producer_owners.get(producer_id).map(|v| v.clone());
            if let Err(reason) = identity.can_consume(producer_owner.as_deref()) {
                let detail = format!("{reason} (peer={peer_id}, room={room_id})");
                tracing::warn!("Consume denied: {detail}");
                audit::log_event(AuditEvent::AuthorizationDenied {
                    action: "consume".into(),
                    peer_id: peer_id.to_string(),
                    detail: detail.clone(),
                });
                return Some(SignalingMessage::Error { code: 4031, message: detail });
            }
            match sfu
                .create_consumer(
                    room_id,
                    sfu_peer_id,
                    producer_id,
                    rtp_capabilities.clone(),
                    transport_id.as_deref(),
                )
                .await
            {
                Ok(result) => Some(SignalingMessage::Consumed {
                    room_id: room_id.clone(),
                    consumer_id: result.consumer_id,
                    producer_id: result.producer_id,
                    kind: result.kind,
                    rtp_parameters: result.rtp_parameters_json,
                }),
                Err(e) => Some(SignalingMessage::Error {
                    code: 5000,
                    message: format!("Consumer creation failed: {e}"),
                }),
            }
        }
        SignalingMessage::CreateDataProducer {
            room_id,
            peer_id: msg_peer_id,
            transport_direction,
            label,
            protocol,
            sctp_stream_parameters,
            transport_id,
        } => {
            // PIT-65: 用消息 peer_id (每网页唯一 sfuPeerId), 与 produce/consume 一致
            let sfu_peer_id = msg_peer_id.as_str();
            tracing::info!(
                "SFU: CreateDataProducer room={} label={} dir={:?}",
                room_id,
                label,
                transport_direction
            );
            if !matches!(transport_direction, mediaservo_common::protocol::TransportDirection::Send)
            {
                return Some(SignalingMessage::Error {
                    code: 4000,
                    message: "CreateDataProducer requires send transport".into(),
                });
            }
            // G3 门: 与 produce 同矩阵 — 车端自动允许（自己的 DC）; 账号禁止; legacy 放行。
            if let Err(reason) = identity.can_produce() {
                let detail = format!("{reason} (peer={peer_id}, room={room_id})");
                tracing::warn!("CreateDataProducer denied: {detail}");
                audit::log_event(AuditEvent::AuthorizationDenied {
                    action: "produce_data".into(),
                    peer_id: peer_id.to_string(),
                    detail: detail.clone(),
                });
                return Some(SignalingMessage::Error { code: 4031, message: detail });
            }
            match sfu
                .create_data_producer(
                    room_id,
                    sfu_peer_id,
                    label,
                    protocol,
                    sctp_stream_parameters.clone(),
                    transport_id.as_deref(),
                )
                .await
            {
                Ok(result) => {
                    // G3: 车端 data producer 登记所属设备（consume_data 授权纵深防御,
                    // 与媒体 producer 同一 id 空间 — UUID 唯一不冲突）。
                    if let Some(device) = server.device_id_of(peer_id) {
                        server.producer_owners.insert(result.data_producer_id.clone(), device);
                    } else {
                        // T2: 同媒体 producer——data 登记断链同样导致反查漏网。
                        tracing::warn!("producer_owners: device binding absent on data produce (session={peer_id}, room={room_id})");
                    }
                    // Broadcast NewDataProducer to all peers in room (late-joiner sync)
                    let broadcast = SignalingMessage::NewDataProducer {
                        room_id: room_id.clone(),
                        data_producer_id: result.data_producer_id.clone(),
                        peer_id: peer_id.to_string(),
                        label: label.clone(),
                        protocol: protocol.clone(),
                    };
                    let _ = server
                        .get_or_create_channel(&room_id)
                        .send(serde_json::to_string(&broadcast).unwrap());
                    tracing::info!(
                        "SFU: broadcast NewDataProducer (label={}) for peer {} in room {}",
                        label,
                        peer_id,
                        room_id
                    );
                    Some(SignalingMessage::DataProducerCreated {
                        room_id: room_id.clone(),
                        data_producer_id: result.data_producer_id,
                    })
                }
                Err(e) => {
                    tracing::error!("SFU: DataProducer creation failed: {e}");
                    Some(SignalingMessage::Error {
                        code: 5000,
                        message: format!("DataProducer creation failed: {e}"),
                    })
                }
            }
        }
        SignalingMessage::ConsumeData {
            room_id,
            peer_id: msg_peer_id,
            transport_direction,
            data_producer_id,
            transport_id,
        } => {
            let sfu_peer_id = msg_peer_id.as_str();
            tracing::info!(
                "SFU: ConsumeData room={} data_producer={} dir={:?}",
                room_id,
                data_producer_id,
                transport_direction
            );
            if !matches!(transport_direction, mediaservo_common::protocol::TransportDirection::Recv)
            {
                return Some(SignalingMessage::Error {
                    code: 4000,
                    message: "ConsumeData requires recv transport".into(),
                });
            }
            // G3 门: 与 consume 同矩阵 — 账号只能 consume 有权车设备的 data producer。
            let producer_owner = server.producer_owners.get(data_producer_id).map(|v| v.clone());
            if let Err(reason) = identity.can_consume(producer_owner.as_deref()) {
                let detail = format!("{reason} (peer={peer_id}, room={room_id})");
                tracing::warn!("ConsumeData denied: {detail}");
                audit::log_event(AuditEvent::AuthorizationDenied {
                    action: "consume_data".into(),
                    peer_id: peer_id.to_string(),
                    detail: detail.clone(),
                });
                return Some(SignalingMessage::Error { code: 4031, message: detail });
            }
            match sfu
                .create_data_consumer(
                    room_id,
                    sfu_peer_id,
                    data_producer_id,
                    transport_id.as_deref(),
                )
                .await
            {
                Ok(result) => Some(SignalingMessage::DataConsumed {
                    room_id: room_id.clone(),
                    data_consumer_id: result.data_consumer_id,
                    data_producer_id: result.data_producer_id,
                }),
                Err(e) => {
                    tracing::error!("SFU: DataConsumer creation failed: {e}");
                    Some(SignalingMessage::Error {
                        code: 5000,
                        message: format!("DataConsumer creation failed: {e}"),
                    })
                }
            }
        }
        SignalingMessage::SfuStatsRequest { producer_id, consumer_id } => {
            // H2: G3 门 — 账号查询 producer 统计时按其所属设备做 can_consume 校验
            // （与 consume 同矩阵纵深防御）; consumer 查询/无主 producer 放行（UUID 不可枚举）。
            let query_id = producer_id.as_ref().or(consumer_id.as_ref());
            let Some(qid) = query_id else {
                return Some(SignalingMessage::Error {
                    code: 4000,
                    message: "SfuStatsRequest requires producer_id or consumer_id".into(),
                });
            };
            let owner = server.producer_owners.get(qid).map(|v| v.clone());
            if let Err(reason) = identity.can_consume(owner.as_deref()) {
                let detail = format!("{reason} (peer={peer_id})");
                tracing::warn!("SfuStatsRequest denied: {detail}");
                audit::log_event(AuditEvent::AuthorizationDenied {
                    action: "sfu_stats".into(),
                    peer_id: peer_id.to_string(),
                    detail: detail.clone(),
                });
                return Some(SignalingMessage::Error { code: 4031, message: detail });
            }
            #[cfg(feature = "sfu-mediasoup")]
            {
                if let Some(pid) = producer_id {
                    match sfu.producer_stats(&pid).await {
                        Ok((kind, bytes, packets, score)) => Some(SignalingMessage::SfuStats {
                            producer_id: Some(pid.to_string()),
                            consumer_id: None,
                            kind: Some(kind),
                            byte_count: bytes,
                            packet_count: packets,
                            score,
                        }),
                        Err(e) => Some(SignalingMessage::Error { code: 5000, message: e }),
                    }
                } else if let Some(cid) = consumer_id {
                    match sfu.consumer_stats(&cid).await {
                        Ok((kind, bytes, packets, score)) => Some(SignalingMessage::SfuStats {
                            producer_id: None,
                            consumer_id: Some(cid.to_string()),
                            kind: Some(kind),
                            byte_count: bytes,
                            packet_count: packets,
                            score,
                        }),
                        Err(e) => Some(SignalingMessage::Error { code: 5000, message: e }),
                    }
                } else {
                    None
                }
            }
            #[cfg(not(feature = "sfu-mediasoup"))]
            {
                let _ = sfu;
                Some(SignalingMessage::Error {
                    code: 5000,
                    message: "sfu-mediasoup not enabled".into(),
                })
            }
        }
        // F1/T4: 网关上报下游子会话消亡——按 (room, peer) 精确清理并广播 ProducerClosed。
        // 身份门：仅设备会话（agent）可报；旧 server 收到未知变体解析失败丢弃 = additive。
        SignalingMessage::DownstreamGone { peer_id: gone_peer, room_id } => {
            let Some(device) = server.device_id_of(peer_id) else {
                tracing::warn!("DownstreamGone rejected: session {peer_id} 无设备绑定（报键={gone_peer}）");
                return None;
            };
            *server.t4_gone_seen.entry(device.clone()).or_insert(0) += 1;
            let closed = sfu.remove_peer_in_room(room_id, gone_peer);
            if closed.is_empty() {
                tracing::info!("DownstreamGone: {gone_peer} 在房间 {room_id} 无 SFU 实体（未 produce/键漂移/已被清理）");
            } else {
                announce_producers_closed(server, room_id, gone_peer, Some(&device), closed);
            }
            None
        }
        _ => None,
    }
}

/// F1/T4: ProducerClosed 通告统一链——close 全局清理 / device 反查 / DownstreamGone
/// 三处共用（防分支漂移）。职责：逐条广播（Ok(n)/Err 可见，C15）+ owners 表同步
/// + StreamDestroy 列表事件（设备归属时）。
fn announce_producers_closed(
    server: &SignalingServer,
    room_id: &str,
    reporter_peer: &str,
    device_id: Option<&str>,
    closed: Vec<(String, mediaservo_common::protocol::MediaKind)>,
) {
    if closed.is_empty() {
        return;
    }
    let tx = server.get_or_create_channel(room_id);
    for (producer_id, kind) in closed {
        let msg = SignalingMessage::ProducerClosed {
            room_id: room_id.to_string(),
            peer_id: reporter_peer.to_string(),
            producer_id: producer_id.clone(),
            kind,
            reason: Some("peer_disconnected".into()),
        };
        match serde_json::to_string(&msg)
            .map_err(|e| format!("serialize: {e}"))
            .and_then(|t| tx.send(t).map_err(|e| format!("send: {e}")))
        {
            Ok(n) => tracing::info!("ProducerClosed broadcast: {n} channel receivers"),
            Err(e) => tracing::warn!("ProducerClosed broadcast failed (room {room_id}): {e}"),
        }
        // owners 表同步（防幽灵登记→下次反查 missed 误报）。
        if device_id.is_some() {
            server.producer_owners.remove(&producer_id);
        }
    }
    tracing::info!("SFU: broadcast ProducerClosed for peer {reporter_peer} in room {room_id}");
    if let Some(device) = device_id {
        // 列表秒级刷新: 流下线事件（前端仅作刷新触发）。
        let ev = crate::admin::AdminEvent::StreamDestroy {
            device_id: device.to_string(),
            stream_id: room_id.to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
        };
        let _ = server.admin_events.send(serde_json::to_string(&ev).unwrap_or_default());
    }
}

/// relay 消息的广播频道：`EncoderStatus` 按消息内声明的流子房间路由
/// （浏览器消费者在 `vehicle_test` 类子房间频道，而设备会话频道在整车房间），
/// 其余 relay 消息沿用会话房间。
fn relay_target_room(
    server: &SignalingServer,
    text: &str,
    fallback_room: &str,
) -> tokio::sync::broadcast::Sender<String> {
    match serde_json::from_str::<SignalingMessage>(text) {
        Ok(SignalingMessage::EncoderStatus { room_id, .. }) => server.get_or_create_channel(&room_id),
        _ => server.get_or_create_channel(fallback_room),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mediaservo_common::protocol::PeerRole;

    /// 测试用 server 构造：sfu-mediasoup 特征下需活体 SfuManager（G2 顺手修
    /// `--tests --benches` 编译挂 — 两参构造在特征开启时不存在的历史遗留）。
    async fn new_test_server() -> SignalingServer {
        #[cfg(feature = "sfu-mediasoup")]
        {
            let sfu = std::sync::Arc::new(
                crate::sfu::SfuManager::new_with_port(crate::sfu::random_udp_port()).await.unwrap(),
            );
            SignalingServer::new(sfu, 1 << 20, None)
        }
        #[cfg(not(feature = "sfu-mediasoup"))]
        {
            SignalingServer::new(1 << 20, None)
        }
    }

    #[tokio::test]
    async fn relay_target_room_routes_encoder_status_to_subroom() {
        let server = new_test_server().await;
        let msg = serde_json::to_string(&SignalingMessage::EncoderStatus {
            room_id: "vehicle_test".into(),
            peer_id: "host-1".into(),
            codec: "video/H264".into(),
            encoder_backend: "software".into(),
            encoder_implementation: Some("OpenH264".into()),
            frames_per_second: 30.0,
            frame_width: 1280,
            frame_height: 720,
            avg_encode_ms: Some(3.0),
        })
        .unwrap();
        let mut sub_rx = server.get_or_create_channel("vehicle_test").subscribe();
        let mut whole_rx = server.get_or_create_channel("vehicle").subscribe();
        relay_target_room(&server, &msg, "vehicle").send("es".into()).unwrap();
        assert_eq!(sub_rx.try_recv().unwrap(), "es", "EncoderStatus 应落消息声明的子房间频道");
        assert!(whole_rx.try_recv().is_err(), "不应落整车频道");
        let sdp = serde_json::to_string(&SignalingMessage::Sdp {
            room_id: "whatever".into(),
            target: None,
            sdp: "s".into(),
        })
        .unwrap();
        relay_target_room(&server, &sdp, "vehicle").send("sdp".into()).unwrap();
        assert_eq!(whole_rx.try_recv().unwrap(), "sdp", "非 EncoderStatus 沿用 fallback 房间");
    }

    #[tokio::test]
    async fn push_config_delivers_to_room_host() {
        let server = new_test_server().await;
        server.room_manager.join_room("vehicle-1", "veh-peer", &PeerRole::Host).expect("join host");
        // 房间频道订阅者（模拟整车会话的连接接收端）
        let tx = server.get_or_create_channel("vehicle-1");
        let mut rx = tx.subscribe();

        server.push_config("vehicle-1", "[[cameras]]\nid = \"cam0\"\n", 7).expect("push");
        let text = rx.try_recv().expect("channel 应有 ConfigPush");
        match serde_json::from_str::<SignalingMessage>(&text).expect("parse") {
            SignalingMessage::ConfigPush { room_id, target, config, version } => {
                assert_eq!(room_id, "vehicle-1");
                assert_eq!(target, "veh-peer", "target 应为房间 host peer");
                assert!(config.contains("cam0"));
                assert_eq!(version, 7);
            }
            other => panic!("expected ConfigPush, got {other:?}"),
        }
    }

    /// I3 review: StatusReport 门 — Device 会话或 Host 角色放行；账号/Legacy 非 Host 拒绝 + 审计。
    #[test]
    fn status_report_gate_allows_device_and_host_only() {
        use crate::roles::{AccountIdentity, CockpitRole, SessionIdentity};
        let device = SessionIdentity::Device("ms-car1".into());
        let account = SessionIdentity::Account(AccountIdentity {
            username: "u".into(),
            role: CockpitRole::Viewer,
            vehicles: vec![],
        });
        // 放行: 设备会话（车端，角色不限）; legacy + Host 角色（PSK 车端）
        assert_eq!(status_report_denial_reason(&device, &PeerRole::Host), None);
        assert_eq!(status_report_denial_reason(&device, &PeerRole::Remote), None);
        assert_eq!(status_report_denial_reason(&SessionIdentity::Legacy, &PeerRole::Host), None);
        // 拒绝: 账号（舱端，Host 角色已被 RoomJoin 门拦截 — 不可达）; legacy 非 Host
        assert!(
            status_report_denial_reason(&account, &PeerRole::Remote).is_some(),
            "账号禁止上报整车状态（防伪造车端）"
        );
        assert!(status_report_denial_reason(&account, &PeerRole::Consumer).is_some());
        assert!(status_report_denial_reason(&SessionIdentity::Legacy, &PeerRole::Remote).is_some());
    }

    /// I3 review: 拒绝路径必须返回 4031 + 审计 AuthorizationDenied(status_report)。
    #[test]
    fn status_report_denial_audits_and_returns_4031() {
        use crate::roles::{AccountIdentity, CockpitRole, SessionIdentity};
        let account = SessionIdentity::Account(AccountIdentity {
            username: "carol".into(),
            role: CockpitRole::Viewer,
            vehicles: vec![],
        });
        let before = audit::recent().len();
        let resp = status_report_denial(&account, &PeerRole::Remote, "peer-x", "vehicle-1")
            .expect("拒绝必须返回 Error 响应");
        match resp {
            SignalingMessage::Error { code, message } => {
                assert_eq!(code, 4031);
                assert!(message.contains("status reports require"), "{message}");
            }
            other => panic!("expected Error, got {other:?}"),
        }
        let after = audit::recent();
        assert!(
            after.iter().skip(before).any(|e| matches!(
                e,
                AuditEvent::AuthorizationDenied { action, peer_id, .. }
                    if action == "status_report" && peer_id == "peer-x"
            )),
            "拒绝必须审计 AuthorizationDenied(status_report): {after:?}"
        );
        // 放行路径: 无响应无审计
        let device = SessionIdentity::Device("ms-car1".into());
        assert!(status_report_denial(&device, &PeerRole::Remote, "p", "r").is_none());
    }

    #[tokio::test]
    async fn push_config_errors_when_room_has_no_host() {
        let server = new_test_server().await;
        let err = server.push_config("empty-room", "cfg", 1).unwrap_err();
        assert!(err.contains("无 host"), "无 host 房间必须报错: {err}");
    }

    /// F1/T4: DownstreamGone 身份门——无设备绑定的会话（浏览器/匿名）上报必须拒绝。
    #[tokio::test]
    async fn downstream_gone_rejected_without_device_binding() {
        let server = new_test_server().await;
        let (tx, _rx) = tokio::sync::broadcast::channel::<String>(4);
        let resp = handle_sfu_message(
            &SignalingMessage::DownstreamGone {
                peer_id: "host".into(),
                room_id: "vehicle_test1".into(),
            },
            &server,
            &tx,
            "browser-session-1",
            &SessionIdentity::Legacy,
        )
        .await;
        assert!(resp.is_none());
    }

    /// F1/T4: 设备会话上报未知房间 = 幂等 no-op（不 panic、无响应）——重放/竞态安全。
    #[tokio::test]
    async fn downstream_gone_idempotent_on_unknown_room() {
        let server = new_test_server().await;
        server
            .device_bindings
            .insert("agent-1".into(), "dev-x".into());
        let (tx, _rx) = tokio::sync::broadcast::channel::<String>(4);
        for room in ["nope", "also_nope"] {
            let resp = handle_sfu_message(
                &SignalingMessage::DownstreamGone {
                    peer_id: "host".into(),
                    room_id: room.into(),
                },
                &server,
                &tx,
                "agent-1",
                &SessionIdentity::Device("dev-x".into()),
            )
            .await;
            assert!(resp.is_none(), "未知房间 {room} 应静默幂等");
        }
    }
}
