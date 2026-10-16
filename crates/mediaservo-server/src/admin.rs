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
use std::io::Write;
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
    /// G3 cockpit account registry (for login auth; empty = no accounts, login always 401).
    pub accounts: Arc<AccountRegistry>,
    /// Device registry (unified-device-admin hot effect on management endpoints; signaling and admin share the same Arc).
    pub device_registry: std::sync::Arc<DeviceRegistry>,
    /// devices.yaml absolute path (for management write-back, assembled in main.rs).
    pub devices_path: String,
    /// accounts.yaml absolute path (for account write-back, assembled in main.rs).
    pub accounts_path: String,
    /// API-key registry (accountless-client-auth T1: exchange issuance and /api/admin/apikeys
    /// management share the same Arc -- same hot-effect discipline as devices; empty = exchange always 401).
    pub api_registry: Arc<crate::apikeys::ApiKeyRegistry>,
    /// api_keys.yaml absolute path (for management write-back, assembled in main.rs).
    pub api_keys_path: String,
    /// exchange issue TTL in seconds (config api_token_ttl_secs, default 43200).
    pub api_token_ttl_secs: u64,
    /// PSK shared state (psk-admin-management): same Arc as signaling; rotation hot-updates through this.
    pub psk_state: std::sync::Arc<std::sync::RwLock<Option<String>>>,
    /// Config file absolute path (for PSK rotation write-back, assembled in main.rs).
    pub config_path: String,
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
pub(crate) struct ErrorResponse {
    pub(crate) error: String,
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
        // device-enroll S5.4: manual-mode pending queue (pending=GET list / approve=POST admit).
        // Role gate = the existing auth_middleware (admin full; dispatcher read-only -- POST denied).
        .route("/api/admin/devices/pending", get(list_pending_devices))
        .route("/api/admin/devices/approve", axum::routing::post(approve_pending_device))
        .route("/api/admin/accounts", get(list_accounts).post(create_account))
        .route(
            "/api/admin/accounts/:username",
            axum::routing::put(update_account).delete(delete_account),
        )
        // accountless-client-auth T1: API-key management plane (admin full; dispatcher read-only --
        // POST/DELETE denied by the existing auth_middleware, same discipline as devices).
        .route("/api/admin/apikeys", get(list_api_keys).post(register_api_key))
        .route("/api/admin/apikeys/:key_id", delete(revoke_api_key))
        .route("/api/admin/psk", get(get_psk).post(rotate_psk))
        .route("/api/admin/events", get(ws_events));
    // H3: SFU management endpoints (exist only in the sfu-mediasoup build -- the native build has no SfuManager).
    #[cfg(feature = "sfu-mediasoup")]
    {
        router = router
            .route("/api/admin/sfu/rooms", get(sfu_rooms))
            .route("/api/admin/sfu/stats", get(sfu_stats));
    }
    router
        .with_state(state.clone())
        // PIT-103 (G2 side fix): the admin API previously had no auth at all (check_auth dead code) --
        // clients (www admin) already sent Bearer on REST and ?token= on the events WS; this adds the server-side enforcement.
        .layer(axum::middleware::from_fn_with_state(state, auth_middleware))
}

// -- Login (G3 account system) ---------------------------------------------------------------
// Separate router: the login endpoint itself cannot be intercepted by the auth middleware (it is the credential issuer).

/// Login rate limiting (I3 review): 2 req/s refill bucket + 5 burst -- brute-force mitigation.
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

/// POST /api/auth/login -- validates username/password (accounts.yaml) -> issues a role JWT.
///
/// I3 review: login is a brute-force surface -- attaches a per-bucket rate limit (GlobalKeyExtractor:
/// a single bucket for the whole service, simple and deterministic in tests; bucketing by X-Forwarded-For for multi-instance/proxy setups is future refinement).
pub fn login_router(state: AdminState) -> Router {
    // ponytail: Box::leak -- process-lifetime static config (tower_governor 0.3 Layer holds a borrow,
    // axum Router requires 'static; leaking one config object is fine for a single-process service).
    let limiter: &'static _ = Box::leak(Box::new(
        tower_governor::governor::GovernorConfigBuilder::default()
            .key_extractor(tower_governor::key_extractor::GlobalKeyExtractor)
            .per_second(LOGIN_RATE_PER_SEC)
            .burst_size(LOGIN_RATE_BURST)
            .finish()
            .expect("governor config"),
    ));
    // exchange shares the brute-force surface -- its own bucket (login bursts do not eat the exchange quota, and vice versa).
    let ex_limiter: &'static _ = Box::leak(Box::new(
        tower_governor::governor::GovernorConfigBuilder::default()
            .key_extractor(tower_governor::key_extractor::GlobalKeyExtractor)
            .per_second(LOGIN_RATE_PER_SEC)
            .burst_size(LOGIN_RATE_BURST)
            .finish()
            .expect("governor config exchange"),
    ));
    Router::new()
        .route(
            "/api/auth/login",
            axum::routing::post(login).layer(tower_governor::GovernorLayer { config: limiter }),
        )
        .route(
            "/api/auth/exchange",
            axum::routing::post(exchange)
                .layer(tower_governor::GovernorLayer { config: ex_limiter }),
        )
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
    // Unknown-user and wrong-password are byte-identical (anti-enumeration, same as G2 devices).
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
    let ttl: u64 = 12 * 3600; // ponytail: 12h session; switch to config when shorter/renewal is needed
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

// -- API-key exchange (accountless-client-auth T1; same shape as LiveKit apikey/apiSecret) ------

#[derive(Debug, Deserialize)]
pub struct ExchangeRequest {
    pub key_id: String,
    pub secret: String,
}

/// POST /api/auth/exchange -- API-key to short JWT. After verifying the pair, signs with `admin_jwt_secret`
/// `{sub:"apikey:<id>", role, vehicles, exp}`; from then on `/ws` and REST go through the existing account gate (T0 three-sided sync).
/// Anti-enumeration: unknown key and wrong secret are byte-identical (same discipline as devices); a separate rate-limit bucket (login_router).
async fn exchange(
    State(state): State<AdminState>,
    axum::Json(req): axum::Json<ExchangeRequest>,
) -> Result<Json<LoginResponse>, (StatusCode, Json<ErrorResponse>)> {
    let signing = state.admin_jwt_secret.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse { error: "admin jwt secret not configured".into() }),
        )
    })?;
    let entry = state.api_registry.verify(&req.key_id, &req.secret).map_err(|e| {
        tracing::warn!("exchange failed for key_id={}", req.key_id);
        crate::audit::log_event(crate::audit::AuditEvent::AuthFailure {
            peer_id: format!("apikey:{}", req.key_id),
            reason: "api key exchange failed".into(),
        });
        (StatusCode::UNAUTHORIZED, Json(ErrorResponse { error: e.message().into() }))
    })?;
    let ttl = state.api_token_ttl_secs;
    let token =
        crate::apikeys::issue_api_token(signing, &req.key_id, &entry, ttl).map_err(|e| {
            tracing::error!("exchange token issuance failed: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse { error: "token issuance failed".into() }),
            )
        })?;
    crate::audit::log_event(crate::audit::AuditEvent::AuthSuccess {
        peer_id: format!("apikey:{}", req.key_id),
        device_id: None,
    });
    Ok(Json(LoginResponse {
        token,
        username: req.key_id,
        role: entry.role.as_str().to_string(),
        expires_in_secs: ttl,
    }))
}

#[derive(Debug, Deserialize)]
pub struct RegisterApiKeyRequest {
    pub key_id: String,
    pub role: String,
    #[serde(default)]
    pub vehicles: Vec<String>,
    #[serde(default)]
    pub label: Option<String>,
}

/// GET /api/admin/apikeys -- list (secret_hash never leaves memory, same semantics as devices/C33).
async fn list_api_keys(State(state): State<AdminState>) -> Json<serde_json::Value> {
    let items: Vec<_> = state
        .api_registry
        .list()
        .into_iter()
        .map(|(key_id, role, vehicles, label)| {
            serde_json::json!({"key_id": key_id, "role": role, "vehicles": vehicles, "label": label})
        })
        .collect();
    Json(serde_json::json!({"api_keys": items}))
}

/// POST /api/admin/apikeys -- register (server generates the secret, **plaintext shown once in the response**, C33 discipline).
/// Disk write failure -> in-memory rollback (register_device same shape).
async fn register_api_key(
    State(state): State<AdminState>,
    Extension(claims): Extension<JwtClaims>,
    axum::Json(req): axum::Json<RegisterApiKeyRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, Json<ErrorResponse>)> {
    let secret = state
        .api_registry
        .register(&req.key_id, &req.role, &req.vehicles, req.label.as_deref())
        .map_err(|e| match e {
            crate::apikeys::ApiKeyRegError::Duplicate => (
                StatusCode::CONFLICT,
                Json(ErrorResponse { error: "api key already registered".into() }),
            ),
            crate::apikeys::ApiKeyRegError::Invalid(msg) => {
                (StatusCode::BAD_REQUEST, Json(ErrorResponse { error: msg }))
            }
            crate::apikeys::ApiKeyRegError::Unknown => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse { error: "api key registry internal error".into() }),
            ),
        })?;
    if let Err(e) = state.api_registry.save(&state.api_keys_path) {
        let _ = state.api_registry.revoke(&req.key_id);
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse { error: format!("api keys file write failed: {e}") }),
        ));
    }
    crate::audit::log_event(crate::audit::AuditEvent::ApiKeyRegistered {
        key_id: req.key_id.clone(),
        actor: claims.sub.clone(),
    });
    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({
            "key_id": req.key_id,
            "secret": secret,
            "note": "secret shown this once only; lost = revoke and re-register (no recovery path). Revocation only blocks future exchanges; issued JWTs expire naturally at TTL."
        })),
    ))
}

/// DELETE /api/admin/apikeys/:key_id -- revoke (in-memory immediate + write-back; issued JWTs die naturally at exp = F10).
async fn revoke_api_key(
    State(state): State<AdminState>,
    Path(key_id): Path<String>,
    Extension(claims): Extension<JwtClaims>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
    state.api_registry.revoke(&key_id).map_err(|_| {
        (StatusCode::NOT_FOUND, Json(ErrorResponse { error: "api key not found".into() }))
    })?;
    if let Err(e) = state.api_registry.save(&state.api_keys_path) {
        tracing::error!("api key revoked in memory but save failed: {e}");
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse { error: format!("api keys file write failed: {e}") }),
        ));
    }
    crate::audit::log_event(crate::audit::AuditEvent::ApiKeyRevoked {
        key_id: key_id.clone(),
        actor: claims.sub.clone(),
    });
    Ok(Json(serde_json::json!({"revoked": key_id})))
}

/// POST /api/admin/config/push -- admin-only vehicle config push (the HTTP entry for E4 push_config).
#[derive(Debug, Deserialize)]
pub struct ConfigPushRequest {
    pub room_id: String,
    pub config: String,
    #[serde(default)]
    pub version: u64,
}

async fn push_config(
    State(state): State<AdminState>,
    Extension(claims): Extension<JwtClaims>,
    axum::Json(req): axum::Json<ConfigPushRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
    state
        .signaling
        .push_config(&req.room_id, &req.config, req.version, Some(claims.sub))
        .map(|_| Json(serde_json::json!({"pushed": req.room_id, "version": req.version})))
        .map_err(|e| {
            tracing::error!("config push failed: {e}");
            (StatusCode::NOT_FOUND, Json(ErrorResponse { error: e }))
        })
}

// ── Devices admin (unified-device-admin) ──────────────────────────────────
// Device registry management (G2 devices.yaml hot effect; dispatcher read-only GET enforced by auth_middleware).

/// device_id light validation (1-64 chars, [A-Za-z0-9-_]; compatible with existing ms-<hex> device names).
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
    /// Optional: admin-supplied secret (option A -- configure host first then register; the secret is never lost); default server-generated.
    #[serde(default)]
    pub secret: Option<String>,
}

#[derive(Serialize)]
struct DeviceView {
    device_id: String,
}

/// GET /api/admin/devices -- registry listing (authorized devices; online state is cross-checked by the frontend rooms view).
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

/// POST /api/admin/devices -- register a device, returns the secret (plaintext shown exactly once).
async fn register_device(
    State(state): State<AdminState>,
    Extension(claims): Extension<JwtClaims>,
    axum::Json(req): axum::Json<RegisterDeviceRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, Json<ErrorResponse>)> {
    validate_device_id(&req.device_id)?;
    let (secret_hash, secret) = state
        .device_registry
        .register_with_secret(&req.device_id, req.secret.as_deref())
        .map_err(|e| match e {
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
            DeviceRegError::InvalidSecret(msg) => {
                (StatusCode::BAD_REQUEST, Json(ErrorResponse { error: msg }))
            }
            DeviceRegError::InvalidPublicKey(msg) => {
                (StatusCode::BAD_REQUEST, Json(ErrorResponse { error: msg }))
            }
        })?;
    if let Err(e) = state.device_registry.save(&state.devices_path) {
        // Disk write failure -> roll back memory (single source of truth = disk; memory-unchanged semantics preserved)
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
            "note": "secret plaintext shown this once only",
        })),
    ))
}

/// DELETE /api/admin/devices/{device_id} -- revoke a device (next access gets 4010).
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

/// POST /api/admin/devices/{device_id}/reset-secret -- reset the key (old key invalidated immediately).
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
        // No rollback (old hash not snapshotted) -- memory new / disk old: the error message notes the divergence,
        // any later save (register/revoke) fixes it; a restart reverts to the old value (C15 already logged).
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
            "note": "secret plaintext shown this once only",
        })),
    ))
}

// -- device-enroll S5.4: pending queue (manual mode) ------------------------------------------

/// GET /api/admin/devices/pending -- pending devices (verified but not admitted; in-memory table D-E5, cleared on restart).
async fn list_pending_devices(State(state): State<AdminState>) -> Json<serde_json::Value> {
    let pending: Vec<serde_json::Value> = state
        .signaling
        .pending_devices
        .list()
        .into_iter()
        .map(|(device_id, e)| {
            serde_json::json!({
                "device_id": device_id,
                "public_key": format!("ed25519:{}", e.vk),
                "first_seen_ms": e.first_seen_ms,
                "verified": e.verified,
            })
        })
        .collect();
    Json(serde_json::json!({ "pending": pending, "count": pending.len() }))
}

#[derive(Debug, Deserialize)]
pub struct ApproveDeviceRequest {
    pub device_id: String,
    /// Optional display name (enters the registry public_key-form entry; default null).
    #[serde(default)]
    pub name: Option<String>,
}

/// POST /api/admin/devices/approve -- approve a pending device:
/// hit -> enroll_auto (public_key form) + save + kick from pending; miss -> 404; already registered by a race -> 409.
/// approve = the policy gate (D-E8): the device's next connection still walks the verify challenge; possession proof is not waived.
async fn approve_pending_device(
    State(state): State<AdminState>,
    Extension(claims): Extension<JwtClaims>,
    axum::Json(req): axum::Json<ApproveDeviceRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, Json<ErrorResponse>)> {
    validate_device_id(&req.device_id)?;
    let entry = state.signaling.pending_devices.remove(&req.device_id).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!(
                    "no pending device {} (never reported or already handled)",
                    req.device_id
                ),
            }),
        )
    })?;
    state.device_registry.enroll_auto(&req.device_id, &entry.vk, req.name.as_deref()).map_err(
        |e| {
            // Admission failure -> restore pending (do not swallow the device re-report window).
            state.signaling.pending_devices.insert(&req.device_id, &entry.vk, entry.verified);
            match e {
                DeviceRegError::Duplicate => (
                    StatusCode::CONFLICT,
                    Json(ErrorResponse {
                        error: format!("device {} already registered", req.device_id),
                    }),
                ),
                other => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse { error: format!("enroll failed: {other:?}") }),
                ),
            }
        },
    )?;
    if let Err(e) = state.device_registry.save(&state.devices_path) {
        // Disk write failure -> roll back memory + restore pending (single source of truth = disk, same rollback chain as register).
        tracing::error!("device {} approve: write failed, rolling back: {e}", req.device_id);
        let _ = state.device_registry.revoke(&req.device_id);
        state.signaling.pending_devices.insert(&req.device_id, &entry.vk, entry.verified);
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse { error: format!("device registry write failed: {e}") }),
        ));
    }
    crate::audit::log_event(crate::audit::AuditEvent::DeviceRegistered {
        device_id: req.device_id.clone(),
        actor: claims.sub.clone(),
    });
    tracing::info!("admin {} approved pending device {}", claims.sub, req.device_id);
    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({
            "device_id": req.device_id,
            "public_key": format!("ed25519:{}", entry.vk),
            "name": req.name,
            "note": "admitted; on reconnect (with backoff) the device passes via public-key verification",
        })),
    ))
}

// ── Accounts admin (unified-device-admin) ────────────────────────────────
// Account registry management (G3 accounts.yaml hot effect; dispatcher read-only GET enforced by auth_middleware).

#[derive(Debug, Deserialize)]
pub struct CreateAccountRequest {
    pub username: String,
    pub password: String,
    pub role: String,
    #[serde(default)]
    pub vehicles: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateAccountRequest {
    pub role: Option<String>,
    #[serde(default)]
    pub vehicles: Option<Vec<String>>,
    #[serde(default)]
    pub new_password: Option<String>,
}

/// GET /api/admin/accounts -- account listing (no password hashes).
async fn list_accounts(State(state): State<AdminState>) -> Json<serde_json::Value> {
    let accounts = state.accounts.list_accounts();
    let count = accounts.len();
    Json(serde_json::json!({ "accounts": accounts, "count": count }))
}

/// POST /api/admin/accounts -- create an account (admin only).
async fn create_account(
    State(state): State<AdminState>,
    Extension(claims): Extension<JwtClaims>,
    axum::Json(req): axum::Json<CreateAccountRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, Json<ErrorResponse>)> {
    if req.username.is_empty() || req.username.len() > 64 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse { error: "invalid username: 1-64 chars".into() }),
        ));
    }
    state
        .accounts
        .create_account(&req.username, &req.password, &req.role, &req.vehicles)
        .map_err(map_account_reg_error)?;
    if let Err(e) = state.accounts.save(&state.accounts_path) {
        tracing::error!("account {} create: write failed, rolling back: {e}", req.username);
        let _ = state.accounts.delete_account(&req.username);
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse { error: format!("accounts file write failed: {e}") }),
        ));
    }
    crate::audit::log_event(crate::audit::AuditEvent::AccountCreated {
        username: req.username.clone(),
        actor: claims.sub.clone(),
        role: req.role.clone(),
    });
    Ok((StatusCode::CREATED, Json(serde_json::json!({ "created": req.username }))))
}

/// PUT /api/admin/accounts/{username} -- update role/vehicle allowlist/password (admin only).
async fn update_account(
    State(state): State<AdminState>,
    Extension(claims): Extension<JwtClaims>,
    Path(username): Path<String>,
    axum::Json(req): axum::Json<UpdateAccountRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
    state
        .accounts
        .update_account(
            &username,
            req.role.as_deref(),
            req.vehicles.as_deref(),
            req.new_password.as_deref(),
        )
        .map_err(map_account_reg_error)?;
    if let Err(e) = state.accounts.save(&state.accounts_path) {
        // No snapshot rollback -- memory new / disk old: the error message notes the divergence (same as devices reset-secret).
        tracing::error!("account {username} update: write failed, memory/disk diverge: {e}");
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("accounts file write failed (registry may diverge): {e}"),
            }),
        ));
    }
    crate::audit::log_event(crate::audit::AuditEvent::AccountUpdated {
        username: username.clone(),
        actor: claims.sub,
    });
    Ok(Json(serde_json::json!({ "updated": username })))
}

/// DELETE /api/admin/accounts/{username} -- delete an account (admin only).
async fn delete_account(
    State(state): State<AdminState>,
    Extension(claims): Extension<JwtClaims>,
    Path(username): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
    if username == claims.sub {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse { error: "cannot delete the account you are logged in as".into() }),
        ));
    }
    state.accounts.delete_account(&username).map_err(map_account_reg_error)?;
    if let Err(e) = state.accounts.save(&state.accounts_path) {
        tracing::error!("account {username} delete: write failed: {e}");
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse { error: format!("accounts file write failed: {e}") }),
        ));
    }
    crate::audit::log_event(crate::audit::AuditEvent::AccountDeleted {
        username: username.clone(),
        actor: claims.sub,
    });
    Ok(Json(serde_json::json!({ "deleted": username })))
}

/// AccountRegError -> HTTP status mapping (C15: a readable error per branch).
fn map_account_reg_error(e: crate::accounts::AccountRegError) -> (StatusCode, Json<ErrorResponse>) {
    match e {
        crate::accounts::AccountRegError::Duplicate => {
            (StatusCode::CONFLICT, Json(ErrorResponse { error: "account already exists".into() }))
        }
        crate::accounts::AccountRegError::Unknown => {
            (StatusCode::NOT_FOUND, Json(ErrorResponse { error: "account not found".into() }))
        }
        crate::accounts::AccountRegError::InvalidRole(role) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("invalid role {role:?}: want viewer|operator|admin|dispatcher"),
            }),
        ),
        crate::accounts::AccountRegError::InvalidVehicles(msg) => {
            (StatusCode::BAD_REQUEST, Json(ErrorResponse { error: msg }))
        }
    }
}

// -- PSK management helpers (psk-admin-management) --------------------------------------------

/// psk mask (for hint display): first 2 chars + `..`; empty -> empty; <=2 chars -> as-is (no leak).
pub fn mask_psk(psk: &str) -> String {
    if psk.is_empty() {
        return String::new();
    }
    let chars: Vec<char> = psk.chars().collect();
    if chars.len() <= 2 {
        return psk.to_string();
    }
    format!("{}··", chars[..2].iter().collect::<String>())
}

/// Text-level write-back of the top-level `psk: "<value>"` in server.yaml (comments/indent/other fields preserved).
/// psk line not found -> append a top-level psk line (YAML top-level semantics -- config psk sits at the top level, a sibling of listen).
/// Failure: error cleanup (no temp residue) and return Err (C15 logging is the caller's job).
fn write_back_psk(path: &std::path::Path, psk: &str) -> Result<(), String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("server config {}: read failed: {e}", path.display()))?;
    let (new_text, replaced_count) = {
        // Only replace a zero-indent top-level psk line (comments/indent/other fields preserved); indented lines (# psk: inside comments) untouched
        let mut count = 0usize;
        let replaced = text
            .lines()
            .map(|line| {
                if line.starts_with("psk:") && !line.starts_with(char::is_whitespace) {
                    count += 1;
                    // Preserve the inline comment (`"..."   # comment` shape -- take the trailing # after the value's closing quote)
                    let base = format!("psk: {psk:?}");
                    return match line.rfind('"') {
                        Some(q) => {
                            let tail = &line[q + 1..];
                            match tail.split_once('#').map(|x| x.1) {
                                Some(c) if !c.trim().is_empty() => format!("{base}  #{c}"),
                                _ => base,
                            }
                        }
                        None => base, // unquoted value (non-standard shape) -- replace the whole thing
                    };
                }
                line.to_string()
            })
            .collect::<Vec<_>>()
            .join("\n");
        (replaced, count)
    };
    let final_text = if replaced_count > 0 { new_text } else { format!("{text}\npsk: {psk:?}\n") };
    let tmp = path.with_extension("yaml.tmp");
    let res = (|| -> std::io::Result<()> {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(final_text.as_bytes())?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, path)?;
        Ok(())
    })();
    match res {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(format!("server config {}: write failed: {e}", path.display()))
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct RotatePskRequest {
    /// Optional: specify the new psk (8-128, no whitespace); default server random 32B hex.
    #[serde(default)]
    pub password: Option<String>,
}

#[derive(Serialize)]
struct PskResponse {
    /// psk plaintext -- **this once only** (C33 discipline; the frontend shows it and drops it).
    psk: String,
    hint: String,
}

fn validate_psk_value(v: &str) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    if v.is_empty()
        || v.len() < 8
        || v.len() > 128
        || v.contains('"')
        || v.chars().any(|c| c.is_whitespace())
    {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse { error: "invalid psk: 8-128 chars, no whitespace".into() }),
        ));
    }
    Ok(())
}

/// GET /api/admin/psk -- one-time view (admin-only; dispatcher blocked by the role gate).
async fn get_psk(
    State(state): State<AdminState>,
    Extension(claims): Extension<JwtClaims>,
) -> Result<Json<PskResponse>, (StatusCode, Json<ErrorResponse>)> {
    let psk = state.psk_state.read().unwrap_or_else(|e| e.into_inner()).clone();
    let psk = psk.ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "psk not configured (no server.yaml psk / MEDIASERVO_PSK)".into(),
            }),
        )
    })?;
    crate::audit::log_event(crate::audit::AuditEvent::PskViewed { actor: claims.sub });
    let hint = mask_psk(&psk);
    Ok(Json(PskResponse { psk, hint }))
}

/// POST /api/admin/psk -- rotate (admin-only): hot effect + write back to server.yaml + audit.
async fn rotate_psk(
    State(state): State<AdminState>,
    Extension(claims): Extension<JwtClaims>,
    axum::Json(req): axum::Json<RotatePskRequest>,
) -> Result<Json<PskResponse>, (StatusCode, Json<ErrorResponse>)> {
    let new_psk = match req.password {
        Some(p) => {
            validate_psk_value(&p)?;
            p
        }
        None => {
            // Random 32B hex (uuid v4 simple, 36 chars < 128 -- legal range; same path as devices new_secret)
            uuid::Uuid::new_v4().to_string().replace('-', "")
        }
    };
    // Write back the file (disk first, effect second -- a failed write does not switch memory)
    if let Err(e) = write_back_psk(std::path::Path::new(&state.config_path), &new_psk) {
        tracing::error!(
            "psk rotate: write-back to {} failed (memory not updated): {e}",
            state.config_path
        );
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse { error: format!("server config write failed: {e}") }),
        ));
    }
    *state.psk_state.write().unwrap_or_else(|e| e.into_inner()) = Some(new_psk.clone());
    crate::audit::log_event(crate::audit::AuditEvent::PskRotated { actor: claims.sub.clone() });
    tracing::warn!("PSK rotated (actor={}) -- all hosts must sync the new psk", claims.sub);
    let hint = mask_psk(&new_psk);
    Ok(Json(PskResponse { psk: new_psk, hint }))
}

// ── Auth helper ─────────────────────────────────────────────────────────────

/// Take the token from the Authorization: Bearer <jwt> header or the `?token=<jwt>` query parameter.
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

pub(crate) fn check_auth(
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

/// Admin API all-route auth middleware (standard axum 0.7 from_fn shape).
/// H3 role awareness: admin = every endpoint; dispatcher = read-only GET (except /api/admin/config -- config is admin-only;
/// write ops POST/DELETE always denied). viewer/operator/unknown roles -> 401 (existing semantics unchanged).
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
        // H3 dispatcher: read-only views (audio rooms/status/video monitoring), no config/control.
        let read_only = matches!(*req.method(), axum::http::Method::GET)
            && !req.uri().path().starts_with("/api/admin/config")
            && !req.uri().path().starts_with("/api/admin/psk");
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
    let devices = state.signaling.room_manager.list_devices(&state.signaling.status_registry);
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

/// H3: multi-vehicle monitoring view data source -- full StatusRegistry snapshot (latest StatusReport per room).
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

/// H3: audio conference panel data source -- SFU room list summary (participants/producers/consumers + ids).
#[cfg(feature = "sfu-mediasoup")]
async fn sfu_rooms(State(state): State<AdminState>) -> Json<serde_json::Value> {
    let rooms = state.sfu_manager.list_rooms();
    Json(serde_json::json!({ "rooms": rooms }))
}

/// H3: SfuStats REST query (mirrors the WS signaling SfuStatsRequest -- the management-plane path of the H2 protocol).
/// Query params: ?producer_id=X or ?consumer_id=X (either).
#[cfg(feature = "sfu-mediasoup")]
/// H2: SfuStats REST query (mirrors the WS signaling SfuStatsRequest -- the management-plane path of the H2 protocol).
/// Query params: `?producer_id=X` or `?consumer_id=X` = single entry (with transport observation enrichment);
/// **both absent = list mode** (optional `?room=<id>` filter) -- weaknet-harness T1 cut A/B:
/// the old "no query -> 400" semantics was re-judged (pinned test re-judged with it); 400 is kept only for dual-id ambiguity.
#[cfg(feature = "sfu-mediasoup")]
async fn sfu_stats(
    State(state): State<AdminState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
    let producer_id = params.get("producer_id").cloned();
    let consumer_id = params.get("consumer_id").cloned();
    if producer_id.is_some() && consumer_id.is_some() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "producer_id and consumer_id are mutually exclusive".into(),
            }),
        ));
    }
    let base = if let Some(pid) = producer_id {
        let (kind, bytes, packets, score) =
            state.sfu_manager.producer_stats(&pid).await.map_err(|e| {
                tracing::error!("admin sfu_stats producer failed: {e}");
                (StatusCode::NOT_FOUND, Json(ErrorResponse { error: e }))
            })?;
        (
            pid.clone(),
            serde_json::json!({
                "producer_id": pid, "consumer_id": None::<String>,
                "kind": kind, "byte_count": bytes, "packet_count": packets, "score": score,
            }),
        )
    } else if let Some(cid) = consumer_id {
        let (kind, bytes, packets, score) =
            state.sfu_manager.consumer_stats(&cid).await.map_err(|e| {
                tracing::error!("admin sfu_stats consumer failed: {e}");
                (StatusCode::NOT_FOUND, Json(ErrorResponse { error: e }))
            })?;
        (
            cid.clone(),
            serde_json::json!({
                "producer_id": None::<String>, "consumer_id": cid,
                "kind": kind, "byte_count": bytes, "packet_count": packets, "score": score,
            }),
        )
    } else {
        // List mode (cut A): transport-level observation rows for all streams, optional room filter.
        let room = params.get("room").cloned();
        let streams = state
            .sfu_manager
            .list_stream_stats(room.as_deref(), &|r: &str| state.signaling.room_owner_of(r))
            .await;
        tracing::info!("admin sfu_stats: list mode room={room:?} → {} entries", streams.len());
        return Ok(Json(serde_json::json!({ "streams": streams })));
    };
    let (qid, mut result) = base;
    // Cut B: per-ID mode goes through the list observation surface and gains the transport fields (not found -> keep the base fields, no downgrade).
    const T_FIELDS: [&str; 8] = [
        "transport_id",
        "local_port",
        "remote_ip",
        "remote_port",
        "rtp_packet_loss_sent",
        "rtp_packet_loss_received",
        "available_outgoing_bitrate",
        "available_incoming_bitrate",
    ];
    let rows = state
        .sfu_manager
        .list_stream_stats(None, &|r: &str| state.signaling.room_owner_of(r))
        .await;
    match rows.iter().find(|v| v.get("id").and_then(|i| i.as_str()) == Some(qid.as_str())) {
        Some(row) => {
            if let Some(obj) = result.as_object_mut() {
                for k in T_FIELDS {
                    if let Some(val) = row.get(k) {
                        obj.insert(k.to_string(), val.clone());
                    }
                }
            }
        }
        None => {
            tracing::debug!("admin sfu_stats: {qid} has no transport observation row (race/closed)")
        }
    }
    // C15: response-path logging (successful queries also leave a trace, visible to ops).
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
    /// psk mask (first 2 chars + ..) -- irreversible; visible to dispatcher read-only.
    psk_hint: String,
}

async fn server_config(State(state): State<AdminState>) -> Json<ServerConfigResponse> {
    Json(ServerConfigResponse {
        listen_host: state.listen_host.clone(),
        listen_port: state.listen_port,
        rate_limit: state.rate_limit,
        room_capacity: state.room_capacity,
        consumer_limit_per_stream: state.consumer_limit_per_stream,
        psk_hint: state
            .psk_state
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .as_deref()
            .map(mask_psk)
            .unwrap_or_default(),
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
                        if ws_sender.send(Message::Text(msg)).await.is_err() {
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
                        sctp_parameters: created.sctp_parameters,
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
                .connect_transport(room_id, peer_id, transport_id, dtls_parameters.clone())
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
            // PIT-65: use the message peer_id (unique per connection), not a hardcoded admin -- multi-connection isolation
            match sfu
                .create_consumer(
                    room_id,
                    peer_id,
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
pub(crate) mod tests {
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
            accounts_path: "/tmp/mediaservo-test-accounts.yaml".into(),
            psk_state: std::sync::Arc::new(std::sync::RwLock::new(None)),
            config_path: "/tmp/mediaservo-test-server.yaml".into(),
            device_registry: Arc::new(DeviceRegistry::empty()),
            devices_path: "/tmp/mediaservo-test-devices.yaml".into(),
            api_registry: Arc::new(crate::apikeys::ApiKeyRegistry::empty()),
            api_keys_path: format!("/tmp/ms-apikeys-{}.yaml", uuid::Uuid::new_v4()),
            api_token_ttl_secs: 3600,
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
            accounts_path: "/tmp/mediaservo-test-accounts.yaml".into(),
            psk_state: std::sync::Arc::new(std::sync::RwLock::new(None)),
            config_path: "/tmp/mediaservo-test-server.yaml".into(),
            device_registry: Arc::new(DeviceRegistry::empty()),
            devices_path: "/tmp/mediaservo-test-devices.yaml".into(),
            api_registry: Arc::new(crate::apikeys::ApiKeyRegistry::empty()),
            api_keys_path: format!("/tmp/ms-apikeys-{}.yaml", uuid::Uuid::new_v4()),
            api_token_ttl_secs: 3600,
        }
    }

    /// Shared by rooms and other cross-module integration tests: a valid JWT with any sub/role.
    #[cfg(test)]
    pub(crate) fn token_of_role(state: &AdminState, user: &str, role: Option<&str>) -> String {
        let now =
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()
                as usize;
        let claims = JwtClaims {
            sub: user.into(),
            iat: now,
            exp: now + 3600,
            role: role.map(str::to_string),
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

    pub(crate) fn admin_token(state: &AdminState) -> String {
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

// -- G3 tests: account login + role-based admin auth + config-push gate ------------------------

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
        assert_eq!(
            wb, ub,
            "unknown-user and wrong-password responses must be byte-identical (anti-enumeration)"
        );
        assert!(String::from_utf8_lossy(&wb).contains("invalid credentials"));
    }

    #[tokio::test]
    async fn login_is_rate_limited() {
        // I3 review: burst beyond the burst limit -> 429 (GlobalKeyExtractor single bucket, test determinism)
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
        assert!(
            limited >= 2,
            "after burst={LOGIN_RATE_BURST} must be rate-limited (429), got: {statuses:?}"
        );
        // The first requests must not be 429 (the limit layer exists but does not over-block normal logins)
        assert!(
            statuses[..LOGIN_RATE_BURST as usize]
                .iter()
                .all(|s| *s != StatusCode::TOO_MANY_REQUESTS),
            "requests within the burst must not be rate-limited: {statuses:?}"
        );
    }

    #[tokio::test]
    async fn login_without_secret_503_and_without_accounts_401() {
        let mut state = super::tests::make_state().await; // no accounts
        let app = login_router(state.clone());
        let resp = app
            .oneshot(login_request(r#"{"username":"carol","password":"s3cret"}"#))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED, "empty registry always 401");

        state.accounts = Arc::new(AccountRegistry::from_yaml(&accounts_yaml()).unwrap());
        state.admin_jwt_secret = None;
        let app = login_router(state);
        let resp = app
            .oneshot(login_request(r#"{"username":"carol","password":"s3cret"}"#))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE, "no JWT secret -> 503");
    }

    #[tokio::test]
    async fn config_push_admin_ok_operator_denied() {
        let state = make_state_with_accounts().await;
        state
            .signaling
            .room_manager
            .join_room("vehicle-1", "veh-peer", &PeerRole::Host)
            .expect("join host");
        // Room channel subscriber (the receiving end of an equivalent real vehicle WS connection -- push needs a receiver).
        let _rx = state.signaling.get_or_create_channel("vehicle-1").subscribe();
        let app = admin_router(state);
        let secret = "test-admin-secret-32-byte-min";

        // admin role (account adm) -> 200
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
        assert_eq!(resp.status(), StatusCode::OK, "admin config push must pass");

        // operator role -> 401 (middleware decides by role)
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
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED, "operator config push must be denied");
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

    // -- H3: dispatcher read-only + new endpoints ---------------------------------------------

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
        assert_eq!(resp.status(), StatusCode::OK, "dispatcher can read the status view");
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
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "dispatcher is denied write operations"
        );
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
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED, "config is admin-only");
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
            "admin is not blocked by read-only (room missing -> 404, not 401)"
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
            config_running: None,
            config_file: None,
            config_last_push: None,
            config_flags: None,
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
        assert_eq!(resp.status(), StatusCode::OK, "dispatcher can read the SFU room list");
        let body = axum::body::to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["rooms"], serde_json::json!([]));
    }

    #[cfg(feature = "sfu-mediasoup")]
    #[tokio::test]
    async fn sfu_stats_no_query_returns_list() {
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
        // weaknet-harness T1 re-judgment (Momus N1): no query = list mode 200;
        // the old 400 semantics moved to "dual-id ambiguity" (see sfu_stats_both_ids_ambiguous).
        assert_eq!(resp.status(), StatusCode::OK, "no query -> list mode 200");
        let body = axum::body::to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            json["streams"],
            serde_json::json!([]),
            "list mode on an empty manager = empty streams array"
        );
    }

    #[cfg(feature = "sfu-mediasoup")]
    #[tokio::test]
    async fn sfu_stats_both_ids_ambiguous() {
        let state = super::tests::make_state().await;
        let secret = state.admin_jwt_secret.clone().unwrap();
        let app = admin_router(state);
        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/admin/sfu/stats?producer_id=a&consumer_id=b")
                    .header("Authorization", format!("Bearer {}", dispatcher_token(&secret)))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::BAD_REQUEST,
            "both ids given = ambiguous -> 400 (after the list-mode re-judgment, 400 keeps only this meaning)"
        );
    }

    // -- psk-admin-management T2: helper functions -------------------------------------------

    #[test]
    fn mask_psk_bounds() {
        assert_eq!(mask_psk(""), "");
        assert_eq!(mask_psk("a"), "a");
        assert_eq!(mask_psk("ab"), "ab");
        assert_eq!(mask_psk("abc"), "ab··");
        assert_eq!(mask_psk("secret-12345678"), "se··");
        assert!(
            !mask_psk("secret-12345678").contains("cret"),
            "mask must not leak the middle part"
        );
    }

    #[test]
    fn write_back_psk_replaces_top_level_preserving_comments_and_indent() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("ms-server-psk-{}.yaml", uuid::Uuid::new_v4()));
        // With inline comment and indented fields (mimicking the server.docker.yaml shape)
        std::fs::write(&path,
            "listen:\n  host: \"0.0.0.0\"\n  port: 9800\npsk: \"old-secret\"   # PIT-49\njwt_secret: \"x\"\n").unwrap();
        write_back_psk(&path, "new-secret-123").unwrap();
        let out = std::fs::read_to_string(&path).unwrap();
        assert!(
            out.contains("psk: \"new-secret-123\"  # PIT-49"),
            "replacement preserves the inline comment: {out}"
        );
        assert!(out.contains("jwt_secret:"), "other fields preserved: {out}");
        assert!(!out.contains("old-secret"), "old value cleared: {out}");
        assert!(!path.with_extension("yaml.tmp").exists(), "temp must be cleaned up");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn write_back_psk_appends_when_missing() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("ms-server-psk-noval-{}.yaml", uuid::Uuid::new_v4()));
        std::fs::write(&path, "listen:\n  host: \"0.0.0.0\"\n").unwrap();
        write_back_psk(&path, "fresh-secret-9").unwrap();
        let out = std::fs::read_to_string(&path).unwrap();
        assert!(out.contains("psk: \"fresh-secret-9\""), "appended when no psk line: {out}");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn write_back_psk_does_not_touch_indented_comment_psk() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("ms-server-psk-comment-{}.yaml", uuid::Uuid::new_v4()));
        std::fs::write(
            &path,
            "listen:\n  host: \"x\"\n# psk: \"commented\" -- a non-config line inside a comment\n",
        )
        .unwrap();
        write_back_psk(&path, "real-secret-77").unwrap();
        let out = std::fs::read_to_string(&path).unwrap();
        // Comment line untouched + top-level append
        assert!(out.contains("# psk: \"commented\""), "commented psk line not replaced: {out}");
        assert!(out.contains("psk: \"real-secret-77\""), "real psk appended: {out}");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn write_back_psk_unwritable_path_errors() {
        let path = std::path::Path::new("/nonexistent-dir-xyz/psk.yaml");
        assert!(write_back_psk(path, "secret-1").is_err());
    }
}

// -- accountless-client-auth T1: API-key management plane + exchange end-to-end pin -------------

#[cfg(test)]
mod apikey_tests {
    use super::tests::{admin_token, make_state};
    use super::*;
    use crate::apikeys;
    use axum::body::Body;
    use http::{Method, Request, StatusCode};
    use tower::util::ServiceExt;

    async fn json_body(resp: http::Response<Body>) -> serde_json::Value {
        let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn apikey_crud_exchange_and_revoke_flow() {
        let state = make_state().await;

        // 1) Register on the management plane -> 201 + one-time plaintext secret
        let admin = admin_token(&state);
        let app = admin_router(state.clone());
        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/admin/apikeys")
                    .header("Authorization", format!("Bearer {admin}"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "key_id": "ci-1", "role": "viewer", "vehicles": ["vehicle_a"]
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let body = json_body(resp).await;
        let secret = body["secret"].as_str().expect("secret one-time plaintext").to_string();

        // 2) List contains no hash/secret (C33 semantics)
        let app = admin_router(state.clone());
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/admin/apikeys")
                    .header("Authorization", format!("Bearer {admin}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let list = json_body(resp).await;
        let text = list.to_string();
        assert!(
            !text.contains("secret_hash") && !text.contains(&secret),
            "list leaks secrets: {text}"
        );

        // 3) exchange success -> token claims shape (F5 sub prefix / role / vehicles / TTL)
        let app = login_router(state.clone());
        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/auth/exchange")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(
                            &serde_json::json!({"key_id": "ci-1", "secret": secret}),
                        )
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let ex = json_body(resp).await;
        assert_eq!(ex["role"], "viewer");
        assert_eq!(ex["expires_in_secs"], 3600);
        let claims: mediaservo_common::auth::JwtClaims = jsonwebtoken::decode(
            ex["token"].as_str().unwrap(),
            &jsonwebtoken::DecodingKey::from_secret(b"test-admin-secret-32-byte-min"),
            &jsonwebtoken::Validation::default(),
        )
        .unwrap()
        .claims;
        assert_eq!(claims.sub, "apikey:ci-1");
        assert_eq!(claims.vehicles.as_deref().unwrap(), ["vehicle_a"]);

        // 4) Anti-enumeration: wrong secret and unknown key are byte-identical
        let mut msgs: Vec<String> = Vec::new();
        for (kid, sec) in [("ci-1", "typo"), ("nope", &secret)] {
            let app = login_router(state.clone());
            let resp = app
                .oneshot(
                    Request::builder()
                        .method(Method::POST)
                        .uri("/api/auth/exchange")
                        .header("content-type", "application/json")
                        .body(Body::from(
                            serde_json::to_vec(&serde_json::json!({"key_id": kid, "secret": sec}))
                                .unwrap(),
                        ))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::UNAUTHORIZED, "{kid}");
            let b = json_body(resp).await;
            msgs.push(b["error"].as_str().unwrap().to_string());
        }
        assert_eq!(msgs[0], msgs[1], "exchange anti-enumeration broken");

        // 5) Revoke -> subsequent exchange immediately 401 (F10: blocks new issues; issued JWTs die naturally at exp)
        let app = admin_router(state.clone());
        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::DELETE)
                    .uri("/api/admin/apikeys/ci-1")
                    .header("Authorization", format!("Bearer {admin}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let app = login_router(state);
        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/auth/exchange")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(
                            &serde_json::json!({"key_id": "ci-1", "secret": secret}),
                        )
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "exchange after revocation must be denied"
        );
    }

    #[tokio::test]
    async fn register_rejects_illegal_role_with_400() {
        let state = make_state().await;
        let admin = admin_token(&state);
        let app = admin_router(state);
        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/admin/apikeys")
                    .header("Authorization", format!("Bearer {admin}"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(
                            &serde_json::json!({"key_id": "k", "role": "superuser"}),
                        )
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::BAD_REQUEST,
            "out-of-matrix role caught at deploy time"
        );
    }

    #[tokio::test]
    async fn exchange_issue_path_uses_apikeys_module_directly() {
        // Pure-function chain: register -> verify -> issue_api_token sub-prefix pin (a second layer of insurance decoupled from the handler)
        let reg = apikeys::ApiKeyRegistry::empty();
        let secret = reg.register("ci-2", "operator", &[], None).unwrap();
        let entry = reg.verify("ci-2", &secret).unwrap();
        let tok =
            apikeys::issue_api_token("s3cret-min-32-bytes-for-test!!", "ci-2", &entry, 60).unwrap();
        let claims: mediaservo_common::auth::JwtClaims = jsonwebtoken::decode(
            &tok,
            &jsonwebtoken::DecodingKey::from_secret(b"s3cret-min-32-bytes-for-test!!"),
            &jsonwebtoken::Validation::default(),
        )
        .unwrap()
        .claims;
        assert_eq!(claims.sub, "apikey:ci-2");
        assert_eq!(claims.role.as_deref(), Some("operator"));
    }
}
