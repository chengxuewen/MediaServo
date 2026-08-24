//! Admin API module — rooms, peers, stats, events management.
//!
//! Protected by JWT Bearer token auth. All endpoints require admin role.

use crate::accounts::{self, AccountRegistry};
use crate::devices::{DeviceRegError, DeviceRegistry};
use crate::signaling::SignalingServer;
use axum::Router;
use axum::body::Body;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json};
use axum::routing::{delete, get};
use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use mediaservo_common::auth::{JwtAuth, JwtClaims};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::broadcast;

// ── State ───────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct AdminState {
    pub signaling: SignalingServer,
    pub event_tx: broadcast::Sender<String>,
    pub admin_jwt_secret: Option<String>,
    pub listen_host: String,
    pub listen_port: u16,
    pub rate_limit: u32,
    pub room_capacity: usize,
    pub consumer_limit_per_stream: usize,
    /// G3 舱端账号注册表（登录认证用; 空 = 无账号，登录一律 401）。
    pub accounts: Arc<AccountRegistry>,
    /// 设备注册表（unified-device-admin 管理端点热生效; signaling 与 admin 共享同一 Arc）。
    pub device_registry: std::sync::Arc<DeviceRegistry>,
    /// devices.yaml 绝对路径（管理写回用，main.rs 装配）。
    pub devices_path: String,
    #[cfg(feature = "sfu-mediasoup")]
    pub sfu_manager: Arc<crate::sfu::SfuManager>,
}

// ── Events ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AdminEvent {
    DeviceOnline { device_id: String, timestamp: String },
    DeviceOffline { device_id: String, timestamp: String },
    StreamCreate { device_id: String, stream_id: String, timestamp: String },
    StreamDestroy { device_id: String, stream_id: String, timestamp: String },
    ConsumerJoin { peer_id: String, device_id: String, stream_id: String, timestamp: String },
    ConsumerLeave { peer_id: String, device_id: String, stream_id: String, timestamp: String },
}

macro_rules! event_ts {
    () => {
        Utc::now().to_rfc3339()
    };
}

// ── Response types ──────────────────────────────────────────────────────────

#[derive(Serialize)]
struct StatsResponse {
    active_rooms: usize,
    total_peers: usize,
    active_connections: usize,
}

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
}

// ── Router ──────────────────────────────────────────────────────────────────

pub fn admin_router(state: AdminState) -> Router {
    let mut router = Router::new()
        .route("/api/admin/rooms", get(list_rooms))
        .route("/api/admin/rooms/{id}", get(get_room).delete(remove_room))
        .route("/api/admin/peers/{id}", delete(kick_peer))
        .route("/api/admin/stats", get(stats))
        .route("/api/admin/status", get(list_status))
        .route("/api/admin/config", get(server_config))
        .route("/api/admin/config/push", axum::routing::post(push_config))
        .route("/api/admin/devices", get(list_devices).post(register_device))
        .route("/api/admin/devices/:device_id", delete(revoke_device))
        .route(
            "/api/admin/devices/:device_id/reset-secret",
            axum::routing::post(reset_device_secret),
        )
        .route("/api/admin/events", get(ws_events));
    // H3: SFU 管理端点（仅 sfu-mediasoup 构建存在 — 原生构建无 SfuManager）。
    #[cfg(feature = "sfu-mediasoup")]
    {
        router = router
            .route("/api/admin/sfu/rooms", get(sfu_rooms))
            .route("/api/admin/sfu/stats", get(sfu_stats));
    }
    router
        .with_state(state.clone())
        // PIT-103 (G2 顺手修): admin API 此前完全无鉴权（check_auth 死代码）—
        // 客户端（www admin）REST 已带 Bearer、events WS 已带 ?token=，此处补服务端强制。
        .layer(axum::middleware::from_fn_with_state(state, auth_middleware))
}

// ── 登录（G3 账号体系）───────────────────────────────────────────────────────
// 独立 router: 登录端点本身不能被 auth middleware 拦（它就是发证入口）。

/// 登录限流（I3 review）: 2 请求/秒补桶 + 5 突发 — 暴力破解缓解。
const LOGIN_RATE_PER_SEC: u64 = 2;
const LOGIN_RATE_BURST: u32 = 5;

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Serialize)]
pub struct LoginResponse {
    pub token: String,
    pub username: String,
    pub role: String,
    pub expires_in_secs: u64,
}

/// POST /api/auth/login — 校验 username/password（accounts.yaml）→ 签发角色 JWT。
///
/// I3 review: 登录是暴力破解面 — 挂 per-bucket 速率限制（GlobalKeyExtractor:
/// 全服务单一桶, 简单且测试确定性; 多实例/代理场景按 X-Forwarded-For 分桶是后续细化）。
pub fn login_router(state: AdminState) -> Router {
    // ponytail: Box::leak — 进程生命周期的静态配置（tower_governor 0.3 Layer 持借用,
    // axum Router 要求 'static; 泄漏一个配置对象对单进程服务无碍）。
    let limiter: &'static _ = Box::leak(Box::new(
        tower_governor::governor::GovernorConfigBuilder::default()
            .key_extractor(tower_governor::key_extractor::GlobalKeyExtractor)
            .per_second(LOGIN_RATE_PER_SEC)
            .burst_size(LOGIN_RATE_BURST)
            .finish()
            .expect("governor config"),
    ));
    Router::new()
        .route("/api/auth/login", axum::routing::post(login))
        .layer(tower_governor::GovernorLayer { config: limiter })
        .with_state(state)
}

async fn login(
    State(state): State<AdminState>,
    axum::Json(req): axum::Json<LoginRequest>,
) -> Result<Json<LoginResponse>, (StatusCode, Json<ErrorResponse>)> {
    let secret = state.admin_jwt_secret.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse { error: "admin jwt secret not configured".into() }),
        )
    })?;
    if state.accounts.is_empty() {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(ErrorResponse {
                error: "account authentication failed: invalid credentials".into(),
            }),
        ));
    }
    // 未知用户与错误密码逐字一致（防枚举，同 G2 devices）。
    let identity = state.accounts.authenticate(&req.username, &req.password).map_err(|_| {
        tracing::warn!("login failed for user {}", req.username);
        crate::audit::log_event(crate::audit::AuditEvent::AuthFailure {
            peer_id: req.username.clone(),
            reason: "account login failed".into(),
        });
        (
            StatusCode::UNAUTHORIZED,
            Json(ErrorResponse {
                error: "account authentication failed: invalid credentials".into(),
            }),
        )
    })?;
    let ttl: u64 = 12 * 3600; // ponytail: 12h 会话; 需要更短/续签时改为配置项
    let token = accounts::issue_account_token(secret, &identity, ttl).map_err(|e| {
        tracing::error!("login token issuance failed: {e}");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse { error: "token issuance failed".into() }),
        )
    })?;
    crate::audit::log_event(crate::audit::AuditEvent::AuthSuccess {
        peer_id: identity.username.clone(),
        device_id: None,
    });
    Ok(Json(LoginResponse {
        token,
        username: identity.username,
        role: identity.role.as_str().to_string(),
        expires_in_secs: ttl,
    }))
}

/// POST /api/admin/config/push — admin 专属整车配置下发（E4 push_config 的 HTTP 入口）。
#[derive(Debug, Deserialize)]
pub struct ConfigPushRequest {
    pub room_id: String,
    pub config: String,
    #[serde(default)]
    pub version: u64,
}

async fn push_config(
    State(state): State<AdminState>,
    axum::Json(req): axum::Json<ConfigPushRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
    state
        .signaling
        .push_config(&req.room_id, &req.config, req.version)
        .map(|_| Json(serde_json::json!({"pushed": req.room_id, "version": req.version})))
        .map_err(|e| {
            tracing::error!("config push failed: {e}");
            (StatusCode::NOT_FOUND, Json(ErrorResponse { error: e }))
        })
}

// ── Devices admin (unified-device-admin) ──────────────────────────────────
// 设备注册表管理（G2 devices.yaml 热生效; dispatcher 只读 GET 由 auth_middleware 保证）。

/// device_id 轻校验（1-64 字符，[A-Za-z0-9-_]；现有设备名 ms-<hex> 兼容）。
fn validate_device_id(id: &str) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    let ok = !id.is_empty()
        && id.len() <= 64
        && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if ok {
        Ok(())
    } else {
        Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("invalid device_id {id:?}: want 1-64 chars of [A-Za-z0-9-_]"),
            }),
        ))
    }
}

#[derive(Debug, Deserialize)]
pub struct RegisterDeviceRequest {
    pub device_id: String,
}

#[derive(Serialize)]
struct DeviceView {
    device_id: String,
}

/// GET /api/admin/devices — 注册表清单（已授权设备; 在线态由前端交叉 rooms 视图）。
async fn list_devices(State(state): State<AdminState>) -> Json<serde_json::Value> {
    let mut devices: Vec<DeviceView> = state
        .device_registry
        .device_ids()
        .into_iter()
        .map(|device_id| DeviceView { device_id })
        .collect();
    devices.sort_by(|a, b| a.device_id.cmp(&b.device_id));
    Json(serde_json::json!({ "devices": devices, "count": devices.len() }))
}

/// POST /api/admin/devices — 注册设备，返回 secret（唯一一次明文展示）。
async fn register_device(
    State(state): State<AdminState>,
    Extension(claims): Extension<JwtClaims>,
    axum::Json(req): axum::Json<RegisterDeviceRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, Json<ErrorResponse>)> {
    validate_device_id(&req.device_id)?;
    let (secret_hash, secret) =
        state.device_registry.register(&req.device_id).map_err(|e| match e {
            DeviceRegError::Duplicate => (
                StatusCode::CONFLICT,
                Json(ErrorResponse {
                    error: format!("device {} already registered", req.device_id),
                }),
            ),
            DeviceRegError::Unknown => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse { error: "device registry internal error".into() }),
            ),
        })?;
    if let Err(e) = state.device_registry.save(&state.devices_path) {
        // 落盘失败 → 回滚内存（单一事实源=磁盘，保持内存不变语义）
        tracing::error!("device {} register: write failed, rolling back: {e}", req.device_id);
        let _ = state.device_registry.revoke(&req.device_id);
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse { error: format!("device registry write failed: {e}") }),
        ));
    }
    crate::audit::log_event(crate::audit::AuditEvent::DeviceRegistered {
        device_id: req.device_id.clone(),
        actor: claims.sub.clone(),
    });
    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({
            "device_id": req.device_id,
            "secret": secret,
            "secret_hash": secret_hash,
            "note": "secret 仅此一次明文展示",
        })),
    ))
}

/// DELETE /api/admin/devices/{device_id} — 吊销设备（下次接入 4010）。
async fn revoke_device(
    State(state): State<AdminState>,
    Extension(claims): Extension<JwtClaims>,
    Path(device_id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
    state.device_registry.revoke(&device_id).map_err(|_| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse { error: format!("device {device_id} not registered") }),
        )
    })?;
    if let Err(e) = state.device_registry.save(&state.devices_path) {
        tracing::error!("device {device_id} revoke: write failed: {e}");
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse { error: format!("device registry write failed: {e}") }),
        ));
    }
    crate::audit::log_event(crate::audit::AuditEvent::DeviceRevoked {
        device_id: device_id.clone(),
        actor: claims.sub,
    });
    Ok(Json(serde_json::json!({ "revoked": device_id })))
}

/// POST /api/admin/devices/{device_id}/reset-secret — 重置密钥（旧密钥立即失效）。
async fn reset_device_secret(
    State(state): State<AdminState>,
    Extension(claims): Extension<JwtClaims>,
    Path(device_id): Path<String>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, Json<ErrorResponse>)> {
    let (_, secret) = state.device_registry.reset_secret(&device_id).map_err(|_| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse { error: format!("device {device_id} not registered") }),
        )
    })?;
    if let Err(e) = state.device_registry.save(&state.devices_path) {
        // 无回滚（旧 hash 未快照）— 内存新值/磁盘旧值: 错误消息注明偏差，
        // 后续任意 save（register/revoke）会修正; 重启则回旧值（C15 已留痕）。
        tracing::error!("device {device_id} reset-secret: write failed, memory/disk diverge: {e}");
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("device registry write failed (registry may diverge): {e}"),
            }),
        ));
    }
    crate::audit::log_event(crate::audit::AuditEvent::DeviceSecretReset {
        device_id: device_id.clone(),
        actor: claims.sub,
    });
    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({
            "device_id": device_id,
            "secret": secret,
            "note": "secret 仅此一次明文展示",
        })),
    ))
}

// ── Auth helper ─────────────────────────────────────────────────────────────

/// 从 Authorization: Bearer <jwt> 头或 `?token=<jwt>` 查询参数取 token。
fn extract_token(req: &axum::http::Request<Body>) -> Option<String> {
    req.headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|s| s.to_string())
        .or_else(|| {
            req.uri().query().and_then(|q| {
                q.split('&').find_map(|kv| kv.strip_prefix("token=")).map(|v| v.to_string())
            })
        })
        .filter(|s| !s.is_empty())
}

fn check_auth(
    req: &axum::http::Request<Body>,
    state: &AdminState,
) -> Result<JwtClaims, (StatusCode, Json<ErrorResponse>)> {
    let secret = state.admin_jwt_secret.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse { error: "admin jwt secret not configured".into() }),
        )
    })?;
    let token = extract_token(req).ok_or_else(|| {
        (
            StatusCode::UNAUTHORIZED,
            Json(ErrorResponse { error: "missing authorization token".into() }),
        )
    })?;
    JwtAuth::new(secret).verify(&token).map_err(|_| {
        (StatusCode::UNAUTHORIZED, Json(ErrorResponse { error: "invalid token".into() }))
    })
}

/// admin API 全路由鉴权中间件（axum 0.7 from_fn 标准形态）。
/// H3 角色感知: admin = 全部端点; dispatcher = 只读 GET（/api/admin/config 除外 — 配置属 admin 专属;
/// 写操作 POST/DELETE 一律拒绝）。viewer/operator/未知角色 → 401（现有语义不变）。
async fn auth_middleware(
    State(state): State<AdminState>,
    mut req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Result<axum::response::Response, (StatusCode, Json<ErrorResponse>)> {
    let claims = check_auth(&req, &state)?;
    let role = claims.role.as_deref().unwrap_or("");
    let is_admin = role == "admin";
    if !is_admin && role != "dispatcher" {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(ErrorResponse { error: "admin or dispatcher role required".into() }),
        ));
    }
    if !is_admin {
        // H3 dispatcher: 只读视图（音频房间/状态/视频监控），无 config/control。
        let read_only = matches!(*req.method(), axum::http::Method::GET)
            && !req.uri().path().starts_with("/api/admin/config");
        if !read_only {
            let detail = "dispatcher role is read-only";
            tracing::warn!("admin API denied: {detail} (path={})", req.uri().path());
            crate::audit::log_event(crate::audit::AuditEvent::AuthorizationDenied {
                action: "admin_api".into(),
                peer_id: claims.sub.clone(),
                detail: format!("{detail} (path={})", req.uri().path()),
            });
            return Err((StatusCode::UNAUTHORIZED, Json(ErrorResponse { error: detail.into() })));
        }
    }
    req.extensions_mut().insert(claims);
    Ok(next.run(req).await)
}

// ── Handlers ────────────────────────────────────────────────────────────────

async fn list_rooms(State(state): State<AdminState>) -> Json<serde_json::Value> {
    let devices = state.signaling.room_manager.list_devices();
    let rooms = state.signaling.room_manager.list_rooms();
    Json(serde_json::json!({
        "devices": devices,
        "rooms": rooms,
    }))
}

async fn get_room(
    State(state): State<AdminState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
    match state.signaling.room_manager.get_room(&id) {
        Some(room) => Ok(Json(serde_json::json!(room))),
        None => Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse { error: format!("room {} not found", id) }),
        )),
    }
}

async fn remove_room(
    State(state): State<AdminState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
    let removed = state.signaling.room_manager.remove_room(&id);
    if removed {
        let _ = state.event_tx.send(
            serde_json::to_string(&AdminEvent::DeviceOffline {
                device_id: id.clone(),
                timestamp: event_ts!(),
            })
            .unwrap_or_default(),
        );
        Ok(Json(serde_json::json!({"removed": id})))
    } else {
        Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse { error: format!("room {} not found", id) }),
        ))
    }
}

async fn kick_peer(
    State(state): State<AdminState>,
    Path(peer_id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
    // Search all rooms for the peer and kick them
    let rooms = state.signaling.room_manager.list_rooms();
    let mut found = false;
    let mut room_ids = Vec::new();

    for room in &rooms {
        if room.host.as_deref() == Some(&peer_id)
            || room.remote.as_deref() == Some(&peer_id)
            || room.consumers.iter().any(|c| c.peer_id == peer_id)
        {
            room_ids.push(room.id.clone());
            found = true;
        }
    }

    if found {
        for rid in &room_ids {
            state.signaling.room_manager.leave_room(rid, &peer_id);
        }
        let _ = state.event_tx.send(
            serde_json::to_string(&AdminEvent::ConsumerLeave {
                peer_id: peer_id.clone(),
                device_id: String::new(),
                stream_id: String::new(),
                timestamp: event_ts!(),
            })
            .unwrap_or_default(),
        );
        Ok(Json(serde_json::json!({"kicked": peer_id, "from_rooms": room_ids})))
    } else {
        Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse { error: format!("peer {} not found in any room", peer_id) }),
        ))
    }
}

async fn stats(State(state): State<AdminState>) -> Json<StatsResponse> {
    let active_rooms = state.signaling.room_manager.active_rooms();
    let total_peers = state.signaling.room_manager.get_peer_count();
    let active_connections = state.signaling.active_connections();

    Json(StatsResponse { active_rooms, total_peers, active_connections })
}

/// H3: 多车监控视图数据源 — StatusRegistry 全量快照（每房间最新 StatusReport）。
async fn list_status(State(state): State<AdminState>) -> Json<serde_json::Value> {
    let vehicles = state
        .signaling
        .status_registry
        .list()
        .into_iter()
        .map(|(room_id, report)| serde_json::json!({ "room_id": room_id, "report": report }))
        .collect::<Vec<_>>();
    Json(serde_json::json!({ "vehicles": vehicles }))
}

/// H3: 音频会议面板数据源 — SFU 房间列表摘要（participants/producers/consumers + ids）。
#[cfg(feature = "sfu-mediasoup")]
async fn sfu_rooms(State(state): State<AdminState>) -> Json<serde_json::Value> {
    let rooms = state.sfu_manager.list_rooms();
    Json(serde_json::json!({ "rooms": rooms }))
}

/// H3: SfuStats REST 查询（镜像 WS 信令 SfuStatsRequest — H2 协议的管理面路径）。
/// 查询参数: ?producer_id=X 或 ?consumer_id=X（任一）。
#[cfg(feature = "sfu-mediasoup")]
async fn sfu_stats(
    State(state): State<AdminState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
    let producer_id = params.get("producer_id").cloned();
    let consumer_id = params.get("consumer_id").cloned();
    let qid = producer_id.clone().or_else(|| consumer_id.clone());
    let Some(qid) = qid else {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse { error: "producer_id or consumer_id required".into() }),
        ));
    };
    let result = if let Some(pid) = producer_id {
        let (kind, bytes, packets, score) =
            state.sfu_manager.producer_stats(&pid).await.map_err(|e| {
                tracing::error!("admin sfu_stats producer failed: {e}");
                (StatusCode::NOT_FOUND, Json(ErrorResponse { error: e }))
            })?;
        serde_json::json!({
            "producer_id": pid, "consumer_id": None::<String>,
            "kind": kind, "byte_count": bytes, "packet_count": packets, "score": score,
        })
    } else if let Some(cid) = consumer_id {
        let (kind, bytes, packets, score) =
            state.sfu_manager.consumer_stats(&cid).await.map_err(|e| {
                tracing::error!("admin sfu_stats consumer failed: {e}");
                (StatusCode::NOT_FOUND, Json(ErrorResponse { error: e }))
            })?;
        serde_json::json!({
            "producer_id": None::<String>, "consumer_id": cid,
            "kind": kind, "byte_count": bytes, "packet_count": packets, "score": score,
        })
    } else {
        unreachable!("query_id guard above");
    };
    // C15: 响应路径日志（查询成功侧也留痕，运维可见）。
    tracing::info!(
        "admin sfu_stats: {qid} → {} bytes / {} packets",
        result["byte_count"],
        result["packet_count"]
    );
    Ok(Json(result))
}

#[derive(Serialize)]
struct ServerConfigResponse {
    listen_host: String,
    listen_port: u16,
    rate_limit: u32,
    room_capacity: usize,
    consumer_limit_per_stream: usize,
}

async fn server_config(State(state): State<AdminState>) -> Json<ServerConfigResponse> {
    Json(ServerConfigResponse {
        listen_host: state.listen_host.clone(),
        listen_port: state.listen_port,
        rate_limit: state.rate_limit,
        room_capacity: state.room_capacity,
        consumer_limit_per_stream: state.consumer_limit_per_stream,
    })
}

async fn ws_events(ws: WebSocketUpgrade, State(state): State<AdminState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws_events(socket, state))
}

#[cfg(feature = "sfu-mediasoup")]
use mediaservo_common::protocol::{SignalingMessage, TransportDirection};

async fn handle_ws_events(socket: WebSocket, state: AdminState) {
    let (mut ws_sender, mut ws_receiver) = socket.split();
    let mut rx = state.event_tx.subscribe();

    // SFU routing for admin WS (create transports, consume)
    #[cfg(feature = "sfu-mediasoup")]
    let sfu = std::sync::Arc::clone(&state.sfu_manager);

    loop {
        tokio::select! {
            event = rx.recv() => {
                match event {
                    Ok(msg) => {
                        if ws_sender.send(Message::Text(msg.into())).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!("Admin WS: lagged behind by {} events", n);
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
            incoming = ws_receiver.next() => {
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        // Try to parse and route SFU messages
                        #[cfg(feature = "sfu-mediasoup")]
                        {
                            if let Ok(sig) = serde_json::from_str::<SignalingMessage>(&text) {
                                handle_admin_sfu(&sig, &sfu, &mut ws_sender).await;
                                continue;
                            }
                        }
                        // Non-SFU message on admin WS — ignore
                        let _ = text;
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    _ => {}
                }
            }
        }
    }
}

/// Handle SFU messages from admin WebSocket — call SfuManager directly.
#[cfg(feature = "sfu-mediasoup")]
async fn handle_admin_sfu(
    msg: &SignalingMessage,
    sfu: &crate::sfu::SfuManager,
    ws_sender: &mut futures_util::stream::SplitSink<WebSocket, Message>,
) -> bool {
    match msg {
        SignalingMessage::CreateWebRtcTransport { room_id, peer_id, direction } => {
            let dir_str = match direction {
                TransportDirection::Send => "send",
                TransportDirection::Recv => "recv",
            };
            tracing::info!(
                "Admin SFU: creating {} transport for peer {} in room {}",
                dir_str,
                peer_id,
                room_id
            );
            match sfu.create_webrtc_transport(room_id, peer_id, dir_str).await {
                Ok(created) => {
                    let response = SignalingMessage::WebRtcTransportCreated {
                        room_id: room_id.clone(),
                        peer_id: peer_id.clone(),
                        transport_id: created.transport_id,
                        ice_parameters: created.ice_parameters,
                        dtls_parameters: created.dtls_parameters,
                        ice_candidates: Some(created.ice_candidates),
                    };
                    let _ = ws_sender
                        .send(Message::Text(serde_json::to_string(&response).unwrap()))
                        .await;
                }
                Err(e) => {
                    let error = SignalingMessage::Error {
                        code: 5000,
                        message: format!("Transport creation failed: {e}"),
                    };
                    let _ =
                        ws_sender.send(Message::Text(serde_json::to_string(&error).unwrap())).await;
                }
            }
            true
        }
        SignalingMessage::ConnectWebRtcTransport {
            room_id,
            peer_id,
            transport_id,
            dtls_parameters,
        } => {
            match sfu
                .connect_transport(&room_id, &peer_id, &transport_id, dtls_parameters.clone())
                .await
            {
                Ok(()) => {
                    tracing::info!(
                        "Admin SFU: transport {transport_id} connected for peer {peer_id}"
                    );
                    let response =
                        SignalingMessage::Error { code: 0, message: "transport_connected".into() };
                    let _ = ws_sender
                        .send(Message::Text(serde_json::to_string(&response).unwrap()))
                        .await;
                }
                Err(e) => {
                    tracing::error!("Admin SFU: connect transport failed: {e}");
                    let response = SignalingMessage::Error {
                        code: 5000,
                        message: format!("Connect failed: {e}"),
                    };
                    let _ = ws_sender
                        .send(Message::Text(serde_json::to_string(&response).unwrap()))
                        .await;
                }
            }
            true
        }
        SignalingMessage::Consume {
            room_id,
            peer_id,
            producer_id,
            rtp_capabilities,
            transport_id,
        } => {
            // PIT-65: 用消息 peer_id (每连接唯一), 非硬编码 admin — 多连接隔离
            match sfu
                .create_consumer(
                    room_id,
                    &peer_id,
                    producer_id,
                    rtp_capabilities.clone(),
                    transport_id.as_deref(),
                )
                .await
            {
                Ok(result) => {
                    let response = SignalingMessage::Consumed {
                        room_id: room_id.clone(),
                        consumer_id: result.consumer_id,
                        producer_id: result.producer_id,
                        kind: result.kind,
                        rtp_parameters: result.rtp_parameters_json,
                    };
                    let _ = ws_sender
                        .send(Message::Text(serde_json::to_string(&response).unwrap()))
                        .await;
                }
                Err(e) => {
                    let error = SignalingMessage::Error {
                        code: 5000,
                        message: format!("Consumer creation failed: {e}"),
                    };
                    let _ =
                        ws_sender.send(Message::Text(serde_json::to_string(&error).unwrap())).await;
                }
            }
            true
        }
        _ => false,
    }
}
// ── Bootstrap ───────────────────────────────────────────────────────────────

/// Print a long-lived admin JWT token for initial setup (valid 1 year).
pub fn print_setup_token(secret: &str) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_secs() as usize;
    let claims = JwtClaims {
        sub: "admin".into(),
        iat: now,
        exp: now + 365 * 86400, // ponytail: 1 year; rotate with shorter TTL if needed
        role: Some("admin".into()),
        vehicles: None,
    };
    let token = jsonwebtoken::encode(
        &jsonwebtoken::Header::default(),
        &claims,
        &jsonwebtoken::EncodingKey::from_secret(secret.as_bytes()),
    )
    .expect("JWT encode");
    println!("Admin bootstrap token (valid 1 year):\n  {token}");
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use http::{Method, Request, StatusCode};
    use tower::util::ServiceExt;

    #[cfg(feature = "sfu-mediasoup")]
    pub(crate) async fn make_state() -> AdminState {
        let sfu = Arc::new(
            crate::sfu::SfuManager::new_with_port(crate::sfu::random_udp_port()).await.unwrap(),
        );
        let signaling = crate::signaling::SignalingServer::new(sfu.clone(), 65536, None);
        let (event_tx, _) = broadcast::channel(256);
        AdminState {
            signaling,
            event_tx,
            admin_jwt_secret: Some("test-admin-secret-32-byte-min".into()),
            listen_host: "0.0.0.0".into(),
            listen_port: 9800,
            rate_limit: 100,
            room_capacity: 10,
            consumer_limit_per_stream: 50,
            accounts: Arc::new(AccountRegistry::empty()),
            device_registry: Arc::new(DeviceRegistry::empty()),
            devices_path: "/tmp/mediaservo-test-devices.yaml".into(),
            sfu_manager: sfu,
        }
    }
    #[cfg(not(feature = "sfu-mediasoup"))]
    pub(crate) async fn make_state() -> AdminState {
        let signaling = crate::signaling::SignalingServer::new(65536, None);
        let (event_tx, _) = broadcast::channel(256);
        AdminState {
            signaling,
            event_tx,
            admin_jwt_secret: Some("test-admin-secret-32-byte-min".into()),
            listen_host: "0.0.0.0".into(),
            listen_port: 9800,
            rate_limit: 100,
            room_capacity: 10,
            consumer_limit_per_stream: 50,
            accounts: Arc::new(AccountRegistry::empty()),
            device_registry: Arc::new(DeviceRegistry::empty()),
            devices_path: "/tmp/mediaservo-test-devices.yaml".into(),
        }
    }

    fn admin_token(state: &AdminState) -> String {
        let _jwt = JwtAuth::new(state.admin_jwt_secret.as_deref().unwrap());
        // ponytail: manually encode with role since sign() doesn't accept role
        let now =
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()
                as usize;
        let claims = JwtClaims {
            sub: "admin".into(),
            iat: now,
            exp: now + 3600,
            role: Some("admin".into()),
            vehicles: None,
        };
        jsonwebtoken::encode(
            &jsonwebtoken::Header::default(),
            &claims,
            &jsonwebtoken::EncodingKey::from_secret(
                state.admin_jwt_secret.as_deref().unwrap().as_bytes(),
            ),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn stats_returns_200() {
        let state = super::tests::make_state().await;
        let token = admin_token(&state);
        let app = admin_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/admin/stats")
                    .header("Authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    #[ignore = "auth temporarily disabled"]
    async fn stats_returns_401_without_token() {
        let state = super::tests::make_state().await;
        let app = admin_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/admin/stats")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    #[ignore = "auth temporarily disabled"]
    async fn stats_returns_401_with_invalid_token() {
        let state = super::tests::make_state().await;
        let app = admin_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/admin/stats")
                    .header("Authorization", "Bearer invalid-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    #[ignore = "auth temporarily disabled"]
    async fn stats_returns_503_without_secret() {
        let state = super::tests::make_state().await;
        let app = admin_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/admin/stats")
                    .header("Authorization", "Bearer whatever")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn list_rooms_returns_devices_and_rooms() {
        let state = super::tests::make_state().await;
        let token = admin_token(&state);
        let app = admin_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/admin/rooms")
                    .header("Authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn get_room_returns_404_for_missing() {
        let state = super::tests::make_state().await;
        let token = admin_token(&state);
        let app = admin_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/admin/rooms/nonexistent")
                    .header("Authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn remove_room_returns_404_for_missing() {
        let state = super::tests::make_state().await;
        let token = admin_token(&state);
        let app = admin_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::DELETE)
                    .uri("/api/admin/rooms/nonexistent")
                    .header("Authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn kick_peer_returns_404_for_missing() {
        let state = super::tests::make_state().await;
        let token = admin_token(&state);
        let app = admin_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::DELETE)
                    .uri("/api/admin/peers/nonexistent")
                    .header("Authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn admin_event_serialization() {
        let ev = AdminEvent::DeviceOnline {
            device_id: "dev-1".into(),
            timestamp: "2024-01-01T00:00:00Z".into(),
        };
        let json = serde_json::to_string(&ev).unwrap();
        assert!(json.contains(r#""type":"device_online""#));
        assert!(json.contains("dev-1"));
    }

    #[test]
    fn print_setup_token_works() {
        // Just ensure it doesn't panic
        print_setup_token("test-secret-with-at-least-32-bytes-here");
    }
}

// ── G3 tests: 账号登录 + 角色化 admin 鉴权 + 配置下发门 ───────────────────────

#[cfg(test)]
mod g3_tests {
    use super::*;
    use crate::accounts::hash_password;
    use axum::body::Body;
    use http::{Method, Request, StatusCode};
    use mediaservo_common::protocol::PeerRole;
    use tower::util::ServiceExt;

    fn accounts_yaml() -> String {
        let hash = hash_password("carol", "s3cret");
        format!(
            "accounts:\n  carol:\n    password_hash: \"{hash}\"\n    role: operator\n    vehicles: [\"ms-car1\"]\n  adm:\n    password_hash: \"{}\"\n    role: admin\n",
            hash_password("adm", "adm-secret")
        )
    }

    async fn make_state_with_accounts() -> AdminState {
        let mut s = super::tests::make_state().await;
        s.accounts = Arc::new(AccountRegistry::from_yaml(&accounts_yaml()).unwrap());
        s
    }

    fn operator_token(secret: &str) -> String {
        role_token(secret, "operator", Some(vec!["ms-car1".into()]))
    }

    fn dispatcher_token(secret: &str) -> String {
        role_token(secret, "dispatcher", None)
    }

    fn role_token(secret: &str, role: &str, vehicles: Option<Vec<String>>) -> String {
        let now =
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()
                as usize;
        let claims = mediaservo_common::auth::JwtClaims {
            sub: "u".into(),
            iat: now,
            exp: now + 3600,
            role: Some(role.into()),
            vehicles,
        };
        jsonwebtoken::encode(
            &jsonwebtoken::Header::default(),
            &claims,
            &jsonwebtoken::EncodingKey::from_secret(secret.as_bytes()),
        )
        .unwrap()
    }

    fn login_request(body: &str) -> Request<Body> {
        Request::builder()
            .method(Method::POST)
            .uri("/api/auth/login")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    #[tokio::test]
    async fn login_success_issues_role_token_with_vehicles() {
        let state = make_state_with_accounts().await;
        let secret = state.admin_jwt_secret.clone().unwrap();
        let app = login_router(state);

        let resp = app
            .oneshot(login_request(r#"{"username":"carol","password":"s3cret"}"#))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["username"], "carol");
        assert_eq!(json["role"], "operator");
        let token = json["token"].as_str().unwrap();
        let claims = JwtAuth::new(secret).verify(token).unwrap();
        assert_eq!(claims.sub, "carol");
        assert_eq!(claims.role.as_deref(), Some("operator"));
        assert_eq!(claims.vehicles.as_deref(), Some(&["ms-car1".to_string()][..]));
    }

    #[tokio::test]
    async fn login_wrong_password_and_unknown_user_identical_401() {
        let state = make_state_with_accounts().await;
        let app = login_router(state);

        let wrong = app
            .clone()
            .oneshot(login_request(r#"{"username":"carol","password":"wrong"}"#))
            .await
            .unwrap();
        let unknown = app
            .clone()
            .oneshot(login_request(r#"{"username":"nobody","password":"s3cret"}"#))
            .await
            .unwrap();
        assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(unknown.status(), StatusCode::UNAUTHORIZED);
        let wb = axum::body::to_bytes(wrong.into_body(), 1 << 20).await.unwrap();
        let ub = axum::body::to_bytes(unknown.into_body(), 1 << 20).await.unwrap();
        assert_eq!(wb, ub, "未知用户与错误密码响应必须逐字一致（防枚举）");
        assert!(String::from_utf8_lossy(&wb).contains("invalid credentials"));
    }

    #[tokio::test]
    async fn login_is_rate_limited() {
        // I3 review: 突发超过 burst → 429（GlobalKeyExtractor 单桶, 测试确定性）
        let state = make_state_with_accounts().await;
        let app = login_router(state);
        let mut statuses = Vec::new();
        for _ in 0..(LOGIN_RATE_BURST + 2) {
            let resp = app
                .clone()
                .oneshot(login_request(r#"{"username":"carol","password":"s3cret"}"#))
                .await
                .unwrap();
            statuses.push(resp.status());
        }
        let limited = statuses.iter().filter(|s| **s == StatusCode::TOO_MANY_REQUESTS).count();
        assert!(limited >= 2, "burst={LOGIN_RATE_BURST} 之后必须被限流(429), got: {statuses:?}");
        // 前若干请求必须不是 429（限流层存在但不过度拦截正常登录）
        assert!(
            statuses[..LOGIN_RATE_BURST as usize]
                .iter()
                .all(|s| *s != StatusCode::TOO_MANY_REQUESTS),
            "burst 内的请求不应被限流: {statuses:?}"
        );
    }

    #[tokio::test]
    async fn login_without_secret_503_and_without_accounts_401() {
        let mut state = super::tests::make_state().await; // 空账号
        let app = login_router(state.clone());
        let resp = app
            .oneshot(login_request(r#"{"username":"carol","password":"s3cret"}"#))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED, "空注册表一律 401");

        state.accounts = Arc::new(AccountRegistry::from_yaml(&accounts_yaml()).unwrap());
        state.admin_jwt_secret = None;
        let app = login_router(state);
        let resp = app
            .oneshot(login_request(r#"{"username":"carol","password":"s3cret"}"#))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE, "无 JWT secret 503");
    }

    #[tokio::test]
    async fn config_push_admin_ok_operator_denied() {
        let state = make_state_with_accounts().await;
        state
            .signaling
            .room_manager
            .join_room("vehicle-1", "veh-peer", &PeerRole::Host)
            .expect("join host");
        // 房间频道订阅者（等价真实车端 WS 连接的接收端 — push 需有接收者）。
        let _rx = state.signaling.get_or_create_channel("vehicle-1").subscribe();
        let app = admin_router(state);
        let secret = "test-admin-secret-32-byte-min";

        // admin 角色（账号 adm）→ 200
        let admin_claims = mediaservo_common::auth::JwtClaims {
            sub: "adm".into(),
            iat: 1,
            exp: 9999999999,
            role: Some("admin".into()),
            vehicles: None,
        };
        let admin_tok = jsonwebtoken::encode(
            &jsonwebtoken::Header::default(),
            &admin_claims,
            &jsonwebtoken::EncodingKey::from_secret(secret.as_bytes()),
        )
        .unwrap();
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/admin/config/push")
                    .header("content-type", "application/json")
                    .header("Authorization", format!("Bearer {admin_tok}"))
                    .body(Body::from(
                        r#"{"room_id":"vehicle-1","config":"[[cameras]]\nid=\"cam0\"\n","version":3}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "admin config push 必须放行");

        // operator 角色 → 401（中间件按 role 判定）
        let op_tok = operator_token(secret);
        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/admin/config/push")
                    .header("content-type", "application/json")
                    .header("Authorization", format!("Bearer {op_tok}"))
                    .body(Body::from(r#"{"room_id":"vehicle-1","config":"evil","version":4}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED, "operator config push 必须拒绝");
    }

    #[tokio::test]
    async fn config_push_admin_no_host_404() {
        let state = make_state_with_accounts().await;
        let app = admin_router(state);
        let claims = mediaservo_common::auth::JwtClaims {
            sub: "adm".into(),
            iat: 1,
            exp: 9999999999,
            role: Some("admin".into()),
            vehicles: None,
        };
        let admin_tok = jsonwebtoken::encode(
            &jsonwebtoken::Header::default(),
            &claims,
            &jsonwebtoken::EncodingKey::from_secret("test-admin-secret-32-byte-min".as_bytes()),
        )
        .unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/admin/config/push")
                    .header("content-type", "application/json")
                    .header("Authorization", format!("Bearer {admin_tok}"))
                    .body(Body::from(r#"{"room_id":"nope","config":"x"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    // ── H3: dispatcher 角色只读 + 新端点 ─────────────────────────────────────

    #[tokio::test]
    async fn dispatcher_can_read_status() {
        let state = super::tests::make_state().await;
        let secret = state.admin_jwt_secret.clone().unwrap();
        let app = admin_router(state);
        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/admin/status")
                    .header("Authorization", format!("Bearer {}", dispatcher_token(&secret)))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "dispatcher 可读状态视图");
    }

    #[tokio::test]
    async fn dispatcher_readonly_write_denied() {
        let state = super::tests::make_state().await;
        let secret = state.admin_jwt_secret.clone().unwrap();
        let app = admin_router(state);
        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::DELETE)
                    .uri("/api/admin/rooms/ms-car1")
                    .header("Authorization", format!("Bearer {}", dispatcher_token(&secret)))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED, "dispatcher 禁止写操作");
    }

    #[tokio::test]
    async fn dispatcher_config_denied() {
        let state = super::tests::make_state().await;
        let secret = state.admin_jwt_secret.clone().unwrap();
        let app = admin_router(state);
        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/admin/config")
                    .header("Authorization", format!("Bearer {}", dispatcher_token(&secret)))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED, "config 属 admin 专属");
    }

    #[tokio::test]
    async fn admin_can_delete_room() {
        let state = super::tests::make_state().await;
        let secret = state.admin_jwt_secret.clone().unwrap();
        let app = admin_router(state);
        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::DELETE)
                    .uri("/api/admin/rooms/ms-car1")
                    .header(
                        "Authorization",
                        format!("Bearer {}", role_token(&secret, "admin", None)),
                    )
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::NOT_FOUND,
            "admin 不被只读拦截（房间不存在 → 404 而非 401）"
        );
    }

    #[tokio::test]
    async fn status_endpoint_returns_stored_reports() {
        let state = super::tests::make_state().await;
        use mediaservo_common::protocol::SignalStatusJson;
        let report = mediaservo_common::protocol::SignalingMessage::StatusReport {
            room_id: "ms-car1".into(),
            topics: vec![],
            streams: vec![],
            processes: vec![],
            signal: SignalStatusJson {
                remote_connected: true,
                remote_since_secs: Some(42),
                remote_peer_id: "p".into(),
                children: vec![],
                agent_uptime_secs: 7,
            },
            ts: 1000,
            config_version: 0,
        };
        state.signaling.status_registry.store("ms-car1", report);
        let secret = state.admin_jwt_secret.clone().unwrap();
        let app = admin_router(state);
        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/admin/status")
                    .header(
                        "Authorization",
                        format!("Bearer {}", role_token(&secret, "admin", None)),
                    )
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["vehicles"][0]["room_id"], "ms-car1");
        assert_eq!(json["vehicles"][0]["report"]["ts"], 1000);
    }

    #[cfg(feature = "sfu-mediasoup")]
    #[tokio::test]
    async fn sfu_rooms_returns_empty_list() {
        let state = super::tests::make_state().await;
        let secret = state.admin_jwt_secret.clone().unwrap();
        let app = admin_router(state);
        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/admin/sfu/rooms")
                    .header("Authorization", format!("Bearer {}", dispatcher_token(&secret)))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "dispatcher 可读 SFU 房间列表");
        let body = axum::body::to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["rooms"], serde_json::json!([]));
    }

    #[cfg(feature = "sfu-mediasoup")]
    #[tokio::test]
    async fn sfu_stats_requires_query_id() {
        let state = super::tests::make_state().await;
        let secret = state.admin_jwt_secret.clone().unwrap();
        let app = admin_router(state);
        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/admin/sfu/stats")
                    .header("Authorization", format!("Bearer {}", dispatcher_token(&secret)))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "缺少 producer/consumer id → 400");
    }
}
