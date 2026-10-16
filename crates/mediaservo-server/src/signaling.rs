use crate::audit::{self, AuditEvent};
use crate::devices::{self, DeviceRegistry, PendingTable};
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
use base64::Engine as _;
use futures_util::{SinkExt, StreamExt}; // the handle_socket WS pump consumes it unconditionally (the stub posture needs it too --
// it was once wrongly gated behind the sfu cfg = the root cause of the --no-default-features lib compile break)
use mediaservo_common::auth::{JwtAuth, SimplePskAuth};
use mediaservo_common::error::CoreError;
use mediaservo_common::protocol::{PeerRole, SignalingMessage};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
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
    /// Admin events channel (triggers second-level list refresh; same source as main.rs AdminState.event_tx).
    admin_events: tokio::sync::broadcast::Sender<String>,
    pub ws_max_message_size: usize,
    /// a1 heartbeat (S0.5): the interval in seconds for server-initiated pings; 0 = off. env MEDIASERVO_WS_PING_SECS.
    pub ws_ping_secs: u64,
    /// a1: `miss` consecutive intervals without a pong -> judged dead (detection budget = interval x (miss+1), about 10-15s --
    /// not a "second-level" promise; RST/TCP errors are immediate). env MEDIASERVO_WS_PONG_MISS.
    pub ws_pong_miss: u32,
    /// a1 companion: the bare-wait cap after connecting but before PSK auth (default 10s; hardening of the previously bare await).
    pub ws_psk_wait_secs: u64,
    /// a1 companion: the wait cap when authenticated but not yet joined (default 30s).
    pub ws_join_wait_secs: u64,
    /// a2 (S0.5): the session-retention ticket table, key = (identity_key, room). Attached at the Joined-ack
    /// (Host+Device+n>=3); disconnect = delayed-cleanup credentials; a resume hit = take out and burn.
    pub resume_table:
        Arc<std::sync::Mutex<std::collections::HashMap<(String, String), HeldResume>>>,
    /// a2 monotonic ticket number (the delayed-cleanup identity credential + the cap eviction order).
    resume_seq: Arc<std::sync::atomic::AtomicU64>,
    /// a2 retention window seconds (env MEDIASERVO_WS_RESUME_HOLD_SECS, default 30).
    pub ws_resume_hold_secs: u64,
    /// Pending messages cache — stores SDP offer + ICE candidates per room for late-joiner replay.
    pub pending_messages: Arc<dashmap::DashMap<String, Vec<String>>>,
    /// JWT authenticator (optional; PSK used as fallback).
    pub jwt_auth: Option<JwtAuth>,
    /// PSK shared state (psk-admin-management): injected at main.rs startup (config first + env fallback);
    /// admin API rotation hot-updates through this lock. None = not configured -- keeps the existing "skip PSK verification" semantics.
    pub psk_state: std::sync::Arc<std::sync::RwLock<Option<String>>>,
    /// Vehicle status report registry (E3: the latest StatusReport per room; read by the phase-H admin API).
    pub status_registry: Arc<StatusRegistry>,
    /// G2 device registry (loaded from devices.yaml at startup, read-only; empty = the PSK path).
    pub device_registry: Arc<DeviceRegistry>,
    /// device-enroll: the manual-mode pending-approval table (in memory, not persisted, D-E5; shares the same Arc with admin pending/approve).
    pub pending_devices: Arc<PendingTable>,
    /// device-enroll S7: the ALLOW_DEV_ENROLL gate (main.rs reads the env at startup and injects it, mirroring the psk_state shared-state pattern).
    pub allow_dev_enroll: Arc<AtomicBool>,
    /// device-enroll: the enroll_auto persistence path (injected from the same source as AdminState.devices_path in main.rs).
    pub devices_path: Arc<str>,
    /// G2 connection-level identity binding (D-H11): peer_id -> device_id (established on successful device auth, cleared on disconnect).
    pub device_bindings: Arc<dashmap::DashMap<String, String>>,
    /// G3 room owner vehicle registry (room_id -> device_id; recorded when a device session joins successfully,
    /// cleared when the room empties) -- the cockpit's RoomJoin/emergency stop do tenant isolation + allowlist auth by the owner vehicle.
    room_owners: Arc<dashmap::DashMap<String, String>>,
    /// G3 producer -> owning vehicle device_id (recorded at produce from the session device binding; consume authorization defense-in-depth).
    producer_owners: Arc<dashmap::DashMap<String, String>>,
    /// F1/T4: the count of DownstreamGone messages this device has received in this session (used at agent disconnect to distinguish
    /// "T4 already cleared per-stream" (normal) from "the registration chain broke" (05:43-style unsolved cases) -- only the latter WARNs).
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
            admin_events: {
                let (tx, _) = tokio::sync::broadcast::channel(256);
                tx
            },
            ws_max_message_size,
            ws_ping_secs: hb_env("MEDIASERVO_WS_PING_SECS", 5),
            ws_pong_miss: (hb_env("MEDIASERVO_WS_PONG_MISS", 2) as u32).max(1),
            ws_psk_wait_secs: hb_env("MEDIASERVO_WS_PSK_WAIT_SECS", 10),
            ws_join_wait_secs: hb_env("MEDIASERVO_WS_JOIN_WAIT_SECS", 30),
            resume_table: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
            resume_seq: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            ws_resume_hold_secs: hb_env("MEDIASERVO_WS_RESUME_HOLD_SECS", 30),
            pending_messages: Arc::new(dashmap::DashMap::new()),
            jwt_auth,
            psk_state: std::sync::Arc::new(std::sync::RwLock::new(None)),
            status_registry: Arc::new(StatusRegistry::default()),
            device_registry: Arc::new(DeviceRegistry::empty()),
            pending_devices: Arc::new(PendingTable::default()),
            allow_dev_enroll: Arc::new(AtomicBool::new(false)),
            devices_path: Arc::from(""),
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
            admin_events: {
                let (tx, _) = tokio::sync::broadcast::channel(256);
                tx
            },
            ws_max_message_size,
            ws_ping_secs: hb_env("MEDIASERVO_WS_PING_SECS", 5),
            ws_pong_miss: (hb_env("MEDIASERVO_WS_PONG_MISS", 2) as u32).max(1),
            ws_psk_wait_secs: hb_env("MEDIASERVO_WS_PSK_WAIT_SECS", 10),
            ws_join_wait_secs: hb_env("MEDIASERVO_WS_JOIN_WAIT_SECS", 30),
            resume_table: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
            resume_seq: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            ws_resume_hold_secs: hb_env("MEDIASERVO_WS_RESUME_HOLD_SECS", 30),
            pending_messages: Arc::new(dashmap::DashMap::new()),
            jwt_auth,
            psk_state: std::sync::Arc::new(std::sync::RwLock::new(None)),
            status_registry: Arc::new(StatusRegistry::default()),
            device_registry: Arc::new(DeviceRegistry::empty()),
            pending_devices: Arc::new(PendingTable::default()),
            allow_dev_enroll: Arc::new(AtomicBool::new(false)),
            devices_path: Arc::from(""),
            device_bindings: Arc::new(dashmap::DashMap::new()),
            room_owners: Arc::new(dashmap::DashMap::new()),
            producer_owners: Arc::new(dashmap::DashMap::new()),
            t4_gone_seen: Arc::new(dashmap::DashMap::new()),
        }
    }

    /// G2 connection-level identity (D-H11): returns the device_id this peer_id is bound to (a session that passed device auth).
    /// G3 uses it for role authorization; no binding = the PSK/JWT path.
    pub fn device_id_of(&self, peer_id: &str) -> Option<String> {
        self.device_bindings.get(peer_id).map(|v| v.clone())
    }

    /// The current device binding count (ops/tests -- the assertion basis for zero residue on join failure).
    pub fn device_binding_count(&self) -> usize {
        self.device_bindings.len()
    }

    /// G3 room owner vehicle (device_id); none = the vehicle is offline or not a vehicle room.
    pub fn room_owner_of(&self, room_id: &str) -> Option<String> {
        self.room_owners.get(room_id).map(|v| v.clone())
    }

    /// test-only: register an owner for the room (the real path = the device RoomJoin branch, inserted at line 1105).
    #[cfg(test)]
    pub(crate) fn set_room_owner_for_test(&self, room_id: &str, device: &str) {
        self.room_owners.insert(room_id.to_string(), device.to_string());
    }

    /// The count of rooms with a registered G3 owner vehicle (ops/tests).
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

    /// Admin events channel (stream up/down events -> second-level frontend list refresh).
    pub fn admin_events(&self) -> broadcast::Sender<String> {
        self.admin_events.clone()
    }

    pub(crate) fn get_or_create_channel(&self, room_id: &str) -> broadcast::Sender<String> {
        self.channels.entry(room_id.to_string()).or_insert_with(RoomChannel::new).tx.clone()
    }
    /// E4 cloud config push: pushes the host.toml full text to the room's Host (the vehicle host-agent session).
    /// Delivered via the room broadcast channel (target = the host peer_id; the receiver filters by target).
    /// `actor` = the authenticated admin account behind the REST call (T1b6 k9; None = unknown origin).
    /// Returns Err on failure -- the caller must log it (C15).
    pub fn push_config(
        &self,
        room_id: &str,
        config: &str,
        version: u64,
        actor: Option<String>,
    ) -> Result<(), String> {
        let host_peer = self
            .room_manager
            .list_rooms()
            .iter()
            .find(|r| r.id == room_id)
            .and_then(|r| r.host.clone())
            .ok_or_else(|| format!("push_config: room {room_id} has no host"))?;
        let msg = SignalingMessage::ConfigPush {
            room_id: room_id.to_string(),
            target: host_peer.clone(),
            config: config.to_string(),
            version,
            actor: actor.clone(),
        };
        let text = serde_json::to_string(&msg)
            .map_err(|e| format!("push_config: serialization failed: {e}"))?;
        let tx = self.get_or_create_channel(room_id);
        tx.send(text).map_err(|_| format!("push_config: room {room_id} has no receiver"))?;
        tracing::info!(
            room_id,
            peer = %host_peer,
            version,
            actor = actor.as_deref().unwrap_or("-"),
            "ConfigPush delivered (the agent correlates it via StatusReport.config_version after applying)"
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
    /// readiness = alive + SFU worker alive. liveness != readiness: `/health` does not degrade when the worker dies
    /// (preventing external restart storms from killing signaling); `/ready` returns 503 for observation + a manual `msrtc-server restart`
    /// to recover (an automatic watchdog is a separate work item -- frontend-process-split T7/B3).
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
    // Extract JWT from the sec-websocket-protocol header -- supports both "Bearer <jwt>" and a bare <jwt>
    // PIT-49: browser subprotocols forbid spaces (RFC 6455 token), so only a bare JWT can be passed
    let jwt_token = headers
        .get("sec-websocket-protocol")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.trim().strip_prefix("Bearer ").unwrap_or(v.trim()))
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty());
    // PIT-49: the subprotocol must be echoed (browsers require the Sec-WebSocket-Protocol response confirmation),
    // otherwise browser negotiation fails and the connection is refused (admin JWT travels via the subprotocol)
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
/// a1 parameter env fallback (the env side of the C35 dual-channel discipline; invalid values revert to defaults).
fn hb_env(key: &str, default: u64) -> u64 {
    std::env::var(key).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

/// a1 detection budget pure function: silence > interval x (miss+1) means dead (the pin-test matrix; never recompute it in the loop).
#[must_use]
pub fn pong_budget_exceeded(
    silent: std::time::Duration,
    interval: std::time::Duration,
    miss: u32,
) -> bool {
    silent > interval * (miss + 1)
}

/// a2: the global retention-ticket cap (the UDP port pool 40000-40100 has only ~100 ports -- retention is a real resource;
/// the per-device cap of 1 is guaranteed naturally by the table key).
const RESUME_GLOBAL_CAP: usize = 32;

/// a2 retention record: the nonce is **not a credential** (authentication is fully re-run, SEC-1); peer_id = the key of the session being taken over.
pub struct HeldResume {
    pub peer_id: String,
    pub nonce: String,
    pub negotiated: u32,
    pub seq: u64,
}

/// a2: the stable session identity key (in v1 only Device participates in attaching; other keys always miss = full join).
fn identity_key(id: &SessionIdentity) -> String {
    match id {
        SessionIdentity::Device(d) => d.clone(),
        SessionIdentity::Account(a) => a.username.clone(),
        SessionIdentity::Legacy => "legacy".into(),
    }
}

/// a2 cap eviction: when over capacity globally, the oldest seq is cleared first (pure function pinned by unit tests).
fn prune_resume(table: &mut std::collections::HashMap<(String, String), HeldResume>, cap: usize) {
    while table.len() > cap {
        match table.iter().min_by_key(|(_, h)| h.seq).map(|(k, _)| k.clone()) {
            Some(k) => {
                table.remove(&k);
            }
            None => break,
        }
    }
}

/// a2 constant-time nonce comparison (even high-entropy tickets leave no timing oracle at the comparison layer).
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b.iter()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// a2: the one-time re-attach ticket (32B CSPRNG base64 -- the same spec as the device challenge nonce).
fn gen_session_nonce() -> String {
    let mut raw = [0u8; 32];
    use rand_core::RngCore as _;
    rand_core::OsRng.fill_bytes(&mut raw);
    base64::engine::general_purpose::STANDARD.encode(raw)
}

fn send_msg(msg: &SignalingMessage) -> Result<String, String> {
    serde_json::to_string(msg).map_err(|e| format!("serialize error: {e}"))
}

/// device-enroll S5.3: the in-flight verify-challenge state (per-connection; the nonce is one-shot per connection, burned on use).
struct PendingAuth {
    /// base64 32B random nonce (the answer verification compares the wire form directly).
    nonce_b64: String,
    device_id: String,
    room_id: String,
    peer_role: PeerRole,
    /// Some = unknown device (verification uses its reported vk; after passing, the policy gate applies); None = a registered public-key form (the registered vk wins).
    offered_vk: Option<String>,
    deadline: tokio::time::Instant,
}

/// design S5.2: "5s timeout without an answer = disconnect WARN".
const DEVICE_AUTH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// The deferred-queue cap for non-answer messages within the challenge window (prevents unauthenticated flooding; over the cap = disconnect WARN).
const MAX_DEFERRED_AUTH_MSGS: usize = 16;

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

    // PSK auth -- shared state (psk-admin-management: main.rs injects the config/env merge; admin rotation hot-updates)
    let psk = server.psk_state.read().unwrap_or_else(|e| e.into_inner()).clone();
    let psk_auth = psk.as_ref().map(|k| SimplePskAuth::new(k.as_bytes()));
    let mut authenticated = psk_auth.is_none();
    tracing::info!("Auth: psk_set={}, authenticated={}", psk.is_some(), authenticated);

    // G3 session identity: the parsed result of the JWT role claim (overwritten to Device after a successful RoomJoin device auth).
    let mut account_identity: Option<AccountIdentity> = None;

    // ── JWT auth (verified whenever a token is presented; PSK is fallback) ──
    // [!] do NOT gate with `if !authenticated` -- when psk is unset, authenticated is pre-set to true, which would skip the presented
    // token entirely: the account identity would never be established -> the session degrades to Legacy -> the produce/data authorization gates get bypassed
    // (the root cause of the double failure of e2e_sfu role_enforcement/data_domain on 2026-08-31, exposed by a fake-green test).
    if (!authenticated || jwt_token.is_some())
        && let (Some(jwt_auth), Some(token)) = (&server.jwt_auth, &jwt_token)
    {
        match jwt_auth.verify(token) {
            Ok(claims) => {
                // G3 role resolution (D-H11): a valid role -> account identity (cockpit); no role = a legacy
                // token (keep the original behavior); unknown role -> refuse the connection (4011).
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
                        // Account session: peer_id uses the username + a short random suffix (concurrent cockpits of the same account do not collide).
                        let role = CockpitRole::parse(role_str).expect("checked above");
                        account_identity = Some(AccountIdentity {
                            username: claims.sub.clone(),
                            role,
                            vehicles: claims.vehicles.clone().unwrap_or_default(),
                        });
                        peer_id =
                            format!("{}-{}", claims.sub, &uuid::Uuid::new_v4().to_string()[..8]);
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
                // T0 fail-closed (accountless-client-auth): a token presented but verification failed = refuse.
                // The old path fell through to PSK -- with PSK correct the identity = Legacy = the whole G3 matrix bypassed (an expired token actually gained privilege).
                // 4013 = the terminal code of the auth family (the Rust error family / TS classification / D273 doc three-sided sync is the completion criterion).
                tracing::warn!(
                    "JWT verification failed: {e} -> 4013 (no longer falls back to PSK)"
                );
                audit::log_event(AuditEvent::AuthFailure {
                    peer_id: peer_id.clone(),
                    reason: "jwt presented but verification failed".into(),
                });
                let error = SignalingMessage::Error {
                    code: 4013,
                    message: "JWT invalid or expired".into(),
                };
                let _ = ws_sender.lock().await.send(Message::Text(send_msg(&error).unwrap())).await;
                return;
            }
        }
    }

    // ── PSK auth (fallback) ───────────────────────────────────────────
    if !authenticated {
        tracing::info!("Auth: waiting for PSK...");
        // a1 companion: pre-auth bare-wait hardening (timeout = disconnect, preventing bare resource squatting -- the live site was previously an unbounded await).
        let psk_frame = match tokio::time::timeout(
            std::time::Duration::from_secs(server.ws_psk_wait_secs),
            receiver.next(),
        )
        .await
        {
            Ok(m) => m,
            Err(_) => {
                tracing::warn!(
                    "a1: pre-auth PSK wait timed out ({}s) -> disconnect",
                    server.ws_psk_wait_secs
                );
                return;
            }
        };
        match psk_frame {
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
    // device-enroll S5.2/S5.3: a Join carrying device_pubkey walks the "challenge -> answer -> verify" state machine;
    // the secret form and the credential-less path are byte-identical (regression isolation, D-E3 coexistence for one cycle).
    let mut pending_auth: Option<PendingAuth> = None;
    // S0: the client dialect claim (assigned only at RoomJoin parsing; None = v1).
    let mut client_protocol_claim: Option<u32> = None;
    // a2: the re-attach ticket claim (assigned at the same place; one-shot -- not recycled after being taken out and failing).
    let mut resume_claim: Option<String> = None;
    let mut deferred: Vec<SignalingMessage> = Vec::new();
    let (room_id, role, device_id, session_identity) = loop {
        // Check for shutdown during RoomJoin
        if *shutdown_rx.borrow() {
            tracing::info!("Shutdown requested during RoomJoin for peer {}", peer_id);
            return;
        }

        tracing::debug!("RoomJoin: waiting for message...");
        // challenge in flight -> wait for the answer with a deadline (timeout = a uniform 4010 + disconnect, the nonce dies with the connection).
        let msg = match pending_auth.as_ref() {
            Some(pa) => {
                let dur = pa.deadline.saturating_duration_since(tokio::time::Instant::now());
                match tokio::time::timeout(dur, receiver.next()).await {
                    Ok(Some(Ok(m))) => m,
                    // byte-aligned with the old `_ => continue`: Err frames continue; only stream end returns.
                    Ok(Some(Err(_))) => continue,
                    Ok(None) => return,
                    Err(_elapsed) => {
                        tracing::warn!(
                            "Peer {} device auth timeout (device={}, room={}) — closing",
                            peer_id,
                            pa.device_id,
                            pa.room_id
                        );
                        audit::log_event(AuditEvent::AuthFailure {
                            peer_id: peer_id.clone(),
                            reason: format!(
                                "device auth challenge timeout (device={})",
                                pa.device_id
                            ),
                        });
                        let error = SignalingMessage::Error {
                            code: 4010,
                            message: devices::DeviceAuthError::BadSecret.message().into(),
                        };
                        let _ = ws_sender
                            .lock()
                            .await
                            .send(Message::Text(send_msg(&error).unwrap()))
                            .await;
                        return;
                    }
                }
            }
            None => match tokio::time::timeout(
                std::time::Duration::from_secs(server.ws_join_wait_secs),
                receiver.next(),
            )
            .await
            {
                Ok(Some(Ok(m))) => m,
                Ok(Some(Err(_))) => continue,
                Ok(None) => return,
                // a1 companion: authenticated-but-not-joined timeout -> disconnect (prevents slot-squatting DoS).
                Err(_) => {
                    tracing::warn!(
                        "a1: authenticated-but-not-joined timed out ({}s) -> disconnect",
                        server.ws_join_wait_secs
                    );
                    return;
                }
            },
        };
        match msg {
            Message::Text(text) => {
                let text_str = text.to_string();

                // -- challenge in flight (S5.3): only DeviceAuthResponse accepted; the rest deferred, unparseable dropped --
                if let Some(pa) = pending_auth.take() {
                    let resp_sig = match serde_json::from_str::<SignalingMessage>(&text_str) {
                        Ok(SignalingMessage::DeviceAuthResponse { sig, .. }) => Some(sig),
                        Ok(other) => {
                            // non-answer message: restore the challenge state + the deferred queue (cap then disconnect, prevents unauthenticated flooding).
                            pending_auth = Some(pa);
                            if deferred.len() >= MAX_DEFERRED_AUTH_MSGS {
                                tracing::warn!(
                                    "Peer {} challenge-window deferred queue overflow (>{} msgs) — closing (anti-flood)",
                                    peer_id,
                                    MAX_DEFERRED_AUTH_MSGS
                                );
                                return;
                            }
                            deferred.push(other);
                            continue;
                        }
                        Err(_) => {
                            // unparseable garbage: dropped with the same semantics as the old path's `_ => continue`, not queued.
                            pending_auth = Some(pa);
                            continue;
                        }
                    };
                    let Some(sig) = resp_sig else { continue };
                    // the nonce is one-shot per connection: take() burns it; any later message never re-enters verification.
                    // -- S5.3 verification: the registered vk wins (a key change fails verification by itself); unknown uses the reported vk --
                    let registered = server.device_registry.entry_of(&pa.device_id);
                    let check_vk = match &registered {
                        Some(devices::Entry::PublicKey { vk, .. }) => Some(vk.clone()),
                        _ => pa.offered_vk.clone(),
                    };
                    let verified = match &check_vk {
                        Some(vk) => devices::verify_signature(
                            vk,
                            &pa.nonce_b64,
                            &pa.device_id,
                            &pa.room_id,
                            &sig,
                        )
                        .is_ok(),
                        None => false,
                    };
                    if !verified {
                        // S5.2 verification failure: not entered into pending (prevents garbage-slot DoS) + disconnect WARN.
                        // a uniform 4010 wire message (no "why" is sent -- the anti-enumeration discipline, same as review #1).
                        tracing::warn!(
                            "Peer {} device pubkey verify FAILED (device={}, room={}, registered={}) -- registered = a key change needs a web delete + re-enroll (S8); unknown = refuse to admit",
                            peer_id,
                            pa.device_id,
                            pa.room_id,
                            registered.is_some()
                        );
                        audit::log_event(AuditEvent::AuthFailure {
                            peer_id: peer_id.clone(),
                            reason: format!(
                                "device pubkey verify failed (device={})",
                                pa.device_id
                            ),
                        });
                        let error = SignalingMessage::Error {
                            code: 4010,
                            message: devices::DeviceAuthError::BadSecret.message().into(),
                        };
                        let _ = ws_sender
                            .lock()
                            .await
                            .send(Message::Text(send_msg(&error).unwrap()))
                            .await;
                        return;
                    }
                    audit::log_event(AuditEvent::AuthSuccess {
                        peer_id: peer_id.clone(),
                        device_id: Some(pa.device_id.clone()),
                    });
                    tracing::info!(
                        "Peer {} device pubkey-authenticated as {}",
                        peer_id,
                        pa.device_id
                    );
                    // unknown device (offered_vk present) -> the policy gate (S5.2); registered -> straight into the join funnel (D-E8).
                    if let Some(vk) = pa.offered_vk.as_deref() {
                        if server.allow_dev_enroll.load(Ordering::Relaxed) {
                            match server.device_registry.enroll_auto(&pa.device_id, vk, None) {
                                Ok(()) => {
                                    if let Err(e) =
                                        server.device_registry.save(server.devices_path.as_ref())
                                    {
                                        // recorded in memory (usable by this connection); the disk write-back failure = the device re-reports and
                                        // self-heals after a restart -- no backlash disconnect (aligned with reset-secret's no-rollback-logging semantics, C15).
                                        tracing::error!(
                                            "device {} auto-enroll write failed, memory/disk diverge: {e}",
                                            pa.device_id
                                        );
                                    }
                                    // mode-switch residue: clear any stale pending.
                                    server.pending_devices.remove(&pa.device_id);
                                    tracing::info!(
                                        "Peer {} auto-enrolled device {} (ALLOW_DEV_ENROLL=1)",
                                        peer_id,
                                        pa.device_id
                                    );
                                }
                                Err(devices::DeviceRegError::Duplicate) => {
                                    // race: already admitted while the challenge was in flight (admin approve / another connection won).
                                    // same key -> let it through; different key -> key-change rejection (verification used the reported vk,
                                    // while the registered one corresponds to a different private key).
                                    let same_vk = matches!(
                                        server.device_registry.entry_of(&pa.device_id),
                                        Some(devices::Entry::PublicKey { vk: reg, .. })
                                            if reg == vk
                                    );
                                    if !same_vk {
                                        tracing::warn!(
                                            "Peer {} device {} enroll race: registered vk differs from reported — rejected",
                                            peer_id,
                                            pa.device_id
                                        );
                                        let error = SignalingMessage::Error {
                                            code: 4010,
                                            message: devices::DeviceAuthError::BadSecret
                                                .message()
                                                .into(),
                                        };
                                        let _ = ws_sender
                                            .lock()
                                            .await
                                            .send(Message::Text(send_msg(&error).unwrap()))
                                            .await;
                                        return;
                                    }
                                }
                                Err(e) => {
                                    tracing::warn!(
                                        "Peer {} auto-enroll rejected device={}: {:?}",
                                        peer_id,
                                        pa.device_id,
                                        e
                                    );
                                    let error = SignalingMessage::Error {
                                        code: 4010,
                                        message: devices::DeviceAuthError::BadSecret
                                            .message()
                                            .into(),
                                    };
                                    let _ = ws_sender
                                        .lock()
                                        .await
                                        .send(Message::Text(send_msg(&error).unwrap()))
                                        .await;
                                    return;
                                }
                            }
                        } else {
                            // manual mode: verification passed -> pending{vk, verified=true} + DeviceAuthPending -> disconnect.
                            // the device keeps its existing retry backoff and waits for approval (batch-3 host wiring: this frame = the backoff signal).
                            server.pending_devices.insert(&pa.device_id, vk, true);
                            tracing::info!(
                                "Peer {} device {} verify passed, awaiting admin approval (pending, not enrolled)",
                                peer_id,
                                pa.device_id
                            );
                            let pending = SignalingMessage::DeviceAuthPending {
                                device_id: pa.device_id.clone(),
                            };
                            let _ = ws_sender
                                .lock()
                                .await
                                .send(Message::Text(send_msg(&pending).unwrap()))
                                .await;
                            return;
                        }
                    }
                    // let it into the join funnel (side effects reuse the original path's function body; deferred flushed before the relay loop).
                    break (
                        pa.room_id.clone(),
                        pa.peer_role.clone(),
                        Some(pa.device_id.clone()),
                        SessionIdentity::Device(pa.device_id.clone()),
                    );
                }
                if let Ok(SignalingMessage::RoomJoin {
                    room_id,
                    peer_role,
                    device_id,
                    device_secret,
                    device_pubkey,
                    protocol,
                    resume,
                    ..
                }) = serde_json::from_str(&text_str)
                {
                    // S0: record the claim; a too-old dialect -> 4101 (terminal family) disconnect -- below min is an
                    // explicit reject, not a silent downgrade (silence = an old endpoint with new expectations connects and its behavior drifts).
                    client_protocol_claim = protocol;
                    resume_claim = resume;
                    if protocol.unwrap_or(1)
                        < mediaservo_common::protocol::SIGNALING_PROTOCOL_MIN_SUPPORTED
                    {
                        let detail = format!(
                            "protocol_version_unsupported: claim {protocol:?} below min {}",
                            mediaservo_common::protocol::SIGNALING_PROTOCOL_MIN_SUPPORTED
                        );
                        tracing::warn!("RoomJoin denied: {detail} (peer={peer_id})");
                        let error = SignalingMessage::Error { code: 4101, message: detail };
                        let _ = ws_sender
                            .lock()
                            .await
                            .send(Message::Text(send_msg(&error).unwrap()))
                            .await;
                        return;
                    }
                    // -- device-enroll S5.2: pubkey form -> always send the challenge first (verify possession per connection, D-E8) --
                    if device_pubkey.is_some() {
                        if device_secret.is_some() {
                            // both forms present = protocol ambiguity -> a parameter-class rejection (4000).
                            tracing::warn!(
                                "Peer {} RoomJoin carries both device_secret and device_pubkey (device={:?}) — rejected",
                                peer_id,
                                device_id
                            );
                            let error = SignalingMessage::Error {
                                code: 4000,
                                message: "device credentials: exactly one of device_secret / device_pubkey".into(),
                            };
                            let _ = ws_sender
                                .lock()
                                .await
                                .send(Message::Text(send_msg(&error).unwrap()))
                                .await;
                            return;
                        }
                        let Some(did) = device_id.clone() else {
                            tracing::warn!(
                                "Peer {} RoomJoin device_pubkey without device_id — rejected",
                                peer_id
                            );
                            let error = SignalingMessage::Error {
                                code: 4000,
                                message: "device_id is required with device_pubkey".into(),
                            };
                            let _ = ws_sender
                                .lock()
                                .await
                                .send(Message::Text(send_msg(&error).unwrap()))
                                .await;
                            return;
                        };
                        match server.device_registry.entry_of(&did) {
                            // a legacy secret-form device connecting in pubkey form = migration not closed out (design S8):
                            // reject without a challenge (saves an RTT); the wire stays uniform 4010, actionable handling goes to the log only.
                            Some(devices::Entry::Secret(_)) => {
                                tracing::warn!(
                                    "Peer {peer_id} device {did} registered as secret-form, pubkey join rejected — migration: delete device in web, re-enroll with pubkey form (ALLOW_DEV_ENROLL auto-pass / manual one-time approve, design §8)"
                                );
                                audit::log_event(AuditEvent::AuthFailure {
                                    peer_id: peer_id.clone(),
                                    reason: format!(
                                        "pubkey join on secret-form device {did} (migrate via web re-enroll)"
                                    ),
                                });
                                let error = SignalingMessage::Error {
                                    code: 4010,
                                    message: devices::DeviceAuthError::BadSecret.message().into(),
                                };
                                let _ = ws_sender
                                    .lock()
                                    .await
                                    .send(Message::Text(send_msg(&error).unwrap()))
                                    .await;
                                return;
                            }
                            entry => {
                                let mut nonce = [0u8; 32];
                                {
                                    use rand_core::RngCore as _;
                                    rand_core::OsRng.fill_bytes(&mut nonce);
                                }
                                let nonce_b64 =
                                    base64::engine::general_purpose::STANDARD.encode(nonce);
                                let challenge = SignalingMessage::DeviceAuthChallenge {
                                    nonce: nonce_b64.clone(),
                                };
                                if ws_sender
                                    .lock()
                                    .await
                                    .send(Message::Text(send_msg(&challenge).unwrap()))
                                    .await
                                    .is_err()
                                {
                                    return;
                                }
                                pending_auth = Some(PendingAuth {
                                    nonce_b64,
                                    device_id: did,
                                    room_id,
                                    peer_role,
                                    // registered public-key form -> the registered vk wins (a mismatched offered fails by itself); unknown -> verify the reported vk.
                                    offered_vk: if matches!(
                                        entry,
                                        Some(devices::Entry::PublicKey { .. })
                                    ) {
                                        None
                                    } else {
                                        device_pubkey
                                    },
                                    deadline: tokio::time::Instant::now() + DEVICE_AUTH_TIMEOUT,
                                });
                                continue;
                            }
                        }
                    }
                    // -- G2 device auth (D-H11; a single 4010 error-code family guards against device enumeration) --
                    // both missing = the PSK path (keep the original flow); half present/failed = Error 4010 then disconnect.
                    match devices::authenticate(
                        &server.device_registry,
                        device_id.as_deref(),
                        device_secret.as_deref(),
                    ) {
                        None => {
                            // G3: no device credential -> the PSK path; identity = account (JWT) or legacy.
                            let identity = account_identity
                                .map(SessionIdentity::Account)
                                .unwrap_or(SessionIdentity::Legacy);
                            break (room_id, peer_role, None, identity);
                        }
                        Some(Err(auth_err)) => {
                            // review #1: the wire message is unified (anti-enumeration), but logs/audit keep the internal distinction
                            // (Unknown vs BadSecret -- ops can tell them apart, clients cannot probe it).
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
                            // auth passed: record the audit; connection-level identity binds after join succeeds.
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
            Message::Close(_) => return,
            _ => continue,
        }
    };

    // S0: negotiation = min(the client's claim (default v1), server max) -- the I5 capability gate reads only this.
    let negotiated = mediaservo_common::protocol::negotiate_protocol(client_protocol_claim);

    // a2: resume = the re-attach index -- **never a substitute for authentication** (the PSK/JWT/device challenge chain is fully re-run for this
    // connection in the join funnel; D283 revocation takes effect immediately, SEC-1). Protocol gate + constant-time ticket comparison + take-and-burn.
    let mut resumed_from: Option<HeldResume> = None;
    if let Some(n) = resume_claim.clone() {
        if negotiated >= mediaservo_common::protocol::PROTOCOL_MIN_RESUME {
            let key = (identity_key(&session_identity), room_id.clone());
            let hit = server.resume_table.lock().ok().and_then(|mut t| {
                let ok = t.get(&key).is_some_and(|h| {
                    h.negotiated == negotiated && ct_eq(n.as_bytes(), h.nonce.as_bytes())
                });
                ok.then(|| t.remove(&key)).flatten()
            });
            match hit {
                Some(h) => {
                    tracing::info!(
                        "a2: resume hit -> peer={} takes over, the delayed cleanup is voided (the ticket was rotated)",
                        h.peer_id
                    );
                    peer_id = h.peer_id.clone();
                    resumed_from = Some(h);
                }
                None => tracing::warn!(
                    "a2: resume ticket invalid/expired/dialect mismatch -> fall back to full join (room={room_id})"
                ),
            }
        } else {
            tracing::warn!(
                "a2: resume requested but negotiated={negotiated}<3 -> ignored (falls back to full join)"
            );
        }
    }
    // a2: the same (identity, room) has an unconsumed retention ticket yet a full join arrives = takeover -- clear the old peer immediately,
    // otherwise the old member is not removed -> join_room falsely reports RoomFull (the self-healing path of one-shot ticket loss).
    if resumed_from.is_none() {
        let key = (identity_key(&session_identity), room_id.clone());
        let stale =
            server.resume_table.lock().ok().and_then(|mut t| t.remove(&key)).map(|h| h.peer_id);
        if let Some(old_peer) = stale {
            tracing::info!(
                "a2: full join takeover -> clearing the old pending peer={old_peer} immediately"
            );
            let tx_now = server.get_or_create_channel(&room_id);
            disconnect_session(server.clone(), old_peer, room_id.clone(), tx_now).await;
        }
    }

    // -- G3 RoomJoin gate (the D-H11 matrix + tenant isolation) --
    // 1. account sessions cannot join with the Host role (Host = the vehicle slot; prevents an account hijacking the room and blocking the vehicle).
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
    // 2. Remote role = the P2P control-negotiation slot (I1 review): those without the control capability are barred --
    // viewer/dispatcher may pull streams (SFU) but must not take the control slot (also prevents DoS: occupying the single Remote slot
    // so a legitimate operator cannot negotiate control).
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
    // 3. matrix + allowlist checks by the room's owner vehicle (vehicle A cannot see vehicle B; accounts only for allowlisted vehicles).
    if let Some(owner) = server.room_owner_of(&room_id)
        && let Some(reason) = session_identity.join_vehicle_room(Some(&owner))
    {
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

    // Join the room (a2: a resume takeover = membership still within the retention window, not yet deleted, so skip join;
    // bindings/owners are naturally maintained by the old session's retained state)
    if resumed_from.is_some() {
        audit::log_event(AuditEvent::PeerJoin {
            peer_id: peer_id.clone(),
            room_id: room_id.clone(),
            role: format!("{:?}", role),
        });
    } else {
        match server.room_manager.join_room(&room_id, &peer_id, &role) {
            Ok(()) => {
                // G2: a session that passed device auth binds its connection-level identity here (peer_id -> device_id, D-H11).
                // review #2: the binding must happen after join succeeds -- failure paths (4001/4002) leave zero residue;
                // cleanup unbinds on disconnect (see the end of the relay loop).
                if let Some(device) = &device_id {
                    server.device_bindings.insert(peer_id.clone(), device.clone());
                    // G3: a successful vehicle join registers the room's owner vehicle (the deciding basis for tenant isolation/authorization).
                    // Note: what is stored is the device id (= vehicle id). For audio rooms room_id = audio-<vehicle>,
                    // the device id IS the vehicle -- join_vehicle_room compares its allowlist by it, naturally correct.
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
    }
    // Send RoomJoined ack
    // a2: vehicle device sessions (Host + Device identity + n>=3) issue a one-shot re-attach ticket and attach the retention record.
    // resume-takeover sessions also go through here = **ticket rotation refresh** (the old ticket was burned at adjudication; no dual-live window).
    let mut session_nonce: Option<String> = None;
    if role == PeerRole::Host
        && matches!(&session_identity, SessionIdentity::Device(_))
        && negotiated >= mediaservo_common::protocol::PROTOCOL_MIN_RESUME
    {
        let n = gen_session_nonce();
        let seq = server.resume_seq.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if let Ok(mut t) = server.resume_table.lock() {
            t.insert(
                (identity_key(&session_identity), room_id.clone()),
                HeldResume { peer_id: peer_id.clone(), nonce: n.clone(), negotiated, seq },
            );
            prune_resume(&mut t, RESUME_GLOBAL_CAP);
        }
        session_nonce = Some(n);
    }
    let ack = SignalingMessage::RoomJoined {
        room_id: room_id.clone(),
        peer_id: peer_id.clone(),
        protocol: Some(negotiated),
        server_version: Some(env!("CARGO_PKG_VERSION").into()),
        session_nonce,
    };
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

    // Clone ws_sender for direct responses (SFU + G3 authorization-denial replies) and relay
    let direct_sender = Arc::clone(&ws_sender);
    let hb_sender = Arc::clone(&ws_sender); // a1: used by the ping task (ws_sender is about to be moved by the pump)
    let relay_sender = ws_sender;

    tracing::info!("Relay spawned: peer={} room={}", relay_peer_id, relay_room);
    let relay_spawn_peer = relay_peer_id.clone();
    let relay_spawn_room = relay_room.clone();
    let relay_handle = tokio::spawn(async move {
        let reason = loop {
            match rx.recv().await {
                Ok(msg) => {
                    tracing::debug!(
                        "Relay: forwarding to peer {} ({} bytes)",
                        relay_spawn_peer,
                        msg.len()
                    );
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
        tracing::info!(
            "Relay ended: peer={} room={} reason={}",
            relay_spawn_peer,
            relay_spawn_room,
            reason
        );
    });

    // Forward: this peer's receiver → broadcast
    tracing::info!("Entering forward loop for peer {}", relay_peer_id);

    // Send existing producers to new consumer (late-joiner sync)
    #[cfg(feature = "sfu-mediasoup")]
    {
        tracing::info!("SFU: list_producers for room {}", relay_room);
        if let Some(created) = server.sfu_manager.list_producers(&relay_room) {
            tracing::info!("SFU: found {} producers in room {}", created.len(), relay_room);
            for (producer_id, kind, peer_id, label) in &created {
                // producer-label T1.3: the replay window carries the real label (the sfu producer_labels table)
                let msg = SignalingMessage::NewProducer {
                    room_id: relay_room.clone(),
                    producer_id: producer_id.clone(),
                    peer_id: peer_id.clone(),
                    kind: kind.clone(),
                    label: label.clone(),
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

    // a1 (S0.5): after the session is established the server heartbeats proactively (started after the ack -- zero control-frame noise in the handshake window).
    // pong/ping, either inbound liveness frame, refreshes the silence stamp; over budget -> the hb signal
    // -> the relay loop takes the same break as Close -> the existing cleanup pipeline (never build a second teardown path).
    let last_alive = Arc::new(std::sync::Mutex::new(std::time::Instant::now()));
    let (hb_tx, mut hb_rx) = tokio::sync::mpsc::channel::<()>(1);
    if server.ws_ping_secs > 0 {
        let (ptx, stamp, dead) = (Arc::clone(&hb_sender), Arc::clone(&last_alive), hb_tx.clone());
        let (interval, miss) =
            (std::time::Duration::from_secs(server.ws_ping_secs), server.ws_pong_miss.max(1));
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(interval);
            tick.tick().await; // swallow the t=0 ready tick (the first ping waits one interval after the ack)
            loop {
                tick.tick().await;
                let silent = match stamp.lock() {
                    Ok(g) => g.elapsed(),
                    Err(_) => return,
                };
                if pong_budget_exceeded(silent, interval, miss) {
                    tracing::info!(
                        "a1: inbound liveness silent {silent:?} over budget -> synthesizing a disconnect"
                    );
                    let _ = dead.try_send(());
                    return;
                }
                if ptx.lock().await.send(Message::Ping("msrtc-hb".into())).await.is_err() {
                    return; // the write side is broken: the main loop's read end finishes the cleanup first
                }
            }
        });
    }

    // device-enroll S5.3: messages deferred during the challenge window are flushed in order (after auth they enter the same pipeline as live messages).
    let mut deferred_iter = deferred.into_iter();
    loop {
        let msg = match deferred_iter.next() {
            Some(d) => Message::Text(send_msg(&d).unwrap_or_default()),
            None => tokio::select! {
                // a1: heartbeat death = the same pipeline as a peer Close (break -> the existing disconnect cleanup).
                _ = hb_rx.recv() => break,
                r = receiver.next() => match r {
                    Some(Ok(m)) => m,
                    _ => break,
                },
            },
        };
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

                // v4 (E3): the vehicle status report -- the Server consumes and stores it directly (not a relay message,
                // not broadcast to the room; an old Server silently dropping parse failures is tolerable -- the periodic report self-heals)
                // I3 review: identity gate -- only Device sessions or the Host role may report; others are refused + audited (C15).
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
                        // second-level list refresh: a change in the streams set -> push an admin event (the frontend only uses it as
                        // a refresh trigger; the 5s poll is kept as a fallback).
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

                // -- G3 emergency-stop command (strong-audit forwarding): operator/admin + access to that vehicle -> audit + forward to the vehicle --
                if let Ok(sig) = serde_json::from_str::<SignalingMessage>(&text_str) {
                    if let SignalingMessage::EmergencyCommand { command, .. } = &sig {
                        let owner = server.room_owner_of(&relay_room);
                        match session_identity.can_emergency(owner.as_deref()) {
                            Ok(()) => {
                                // strong audit: who/when (the tracing timestamp)/which vehicle/what command
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
                    // -- G3 ConfigPush: one-way server->client (admin REST); client-inbound is always refused --
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
                            negotiated,
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
                        // v2 (web-stream-stats double review HIGH): unparseable messages (e.g. unknown old variants) are silently dropped --
                        // C15 requires the error path to log; rate-limited to prevent flooding
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
                    // I1 review: P2P rooms (non-DeviceStream) SDP/ICE = the control-negotiation carrier --
                    // accounts without the control capability (viewer/dispatcher) are blocked from relaying even after joining as Consumer
                    // (Consumer only uses SFU media; negotiation is message-driven and needs no SDP relaying).
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
                            // v2: encoder_status passes through (encoding diagnostics, web-stream-stats T3 Oracle F3)
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

                    // Broadcast channel routing: EncoderStatus follows the sub-room declared inside the message (browser consumers are
                    // on the per-stream sub-room channels), everything else goes to the vehicle session channel (relay_room).
                    let out_tx = relay_target_room(&server, &text_str, &relay_room);
                    match out_tx.send(text_str.clone()) {
                        Ok(n) => {
                            tracing::debug!("Forward: broadcast to {} receivers", n);
                            // ── Cache SDP + ICE for late-joiner replay
                            if let Ok(sig_msg) = serde_json::from_str::<SignalingMessage>(&text_str)
                                && matches!(
                                    sig_msg,
                                    SignalingMessage::Sdp { .. }
                                        | SignalingMessage::RTCIceCandidate { .. }
                                )
                            {
                                let mut msgs =
                                    server.pending_messages.entry(relay_room.clone()).or_default();
                                msgs.push(text_str);
                                // ponytail: cap at 64 messages; real ring-buffer if this overflows
                                if msgs.len() > 64 {
                                    msgs.remove(0);
                                }
                            }
                        }
                        Err(tokio::sync::broadcast::error::SendError(_)) => {
                            tracing::debug!("Forward: no receivers, message dropped"); // W5: the stress-window flood (3321 lines/window) drowned real errors -- an empty channel is normal
                        }
                    }
                }
            }
            Message::Close(_) => break,
            // a1: pong/ping either = proof the peer is alive (the client's periodic ping also refreshes, freeing reliance on the
            // auto-pong flushing the split sink).
            Message::Pong(_) | Message::Ping(_) => {
                if let Ok(mut g) = last_alive.lock() {
                    *g = std::time::Instant::now();
                }
            }
            _ => {}
        }
    }

    relay_handle.abort();

    // a2 (S0.5): this (identity, room) still has an unconsumed retention ticket -> cleanup enters the retention window
    // (transport alive = resume can re-attach); a hit re-attach = the entry is burn-on-use, the delayed task idles.
    let hold_key = (identity_key(&session_identity), relay_room.clone());
    let held_seq = server
        .resume_table
        .lock()
        .ok()
        .and_then(|t| t.get(&hold_key).filter(|h| h.peer_id == relay_peer_id).map(|h| h.seq));
    match held_seq {
        Some(seq) => {
            tracing::info!(
                "a2: peer={} entered the retention window ({}s), waiting for a re-attach",
                relay_peer_id,
                server.ws_resume_hold_secs
            );
            let (srv, peer, room, tx, key) = (
                server.clone(),
                relay_peer_id.clone(),
                relay_room.clone(),
                tx.clone(),
                hold_key.clone(),
            );
            let hold = server.ws_resume_hold_secs;
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(hold)).await;
                let expired = srv
                    .resume_table
                    .lock()
                    .map(|mut t| {
                        if t.get(&key).is_some_and(|h| h.seq == seq) {
                            t.remove(&key);
                            true
                        } else {
                            false
                        }
                    })
                    .unwrap_or(false);
                if expired {
                    tracing::info!(
                        "a2: the retention window expired with no re-attach -> running the delayed cleanup peer={peer}"
                    );
                    disconnect_session(srv, peer, room, tx).await;
                } else {
                    tracing::debug!(
                        "a2: the retention ticket was taken over/replaced -- skipping the delayed cleanup peer={peer}"
                    );
                }
            });
        }
        None => disconnect_session(server, relay_peer_id, relay_room, tx).await,
    }
}

/// a2 teardown body: SFU cross-room cleanup / device unbind / leave / consumer disconnect / router deferred teardown / RoomLeave broadcast.
/// One shared copy for the three call sites (immediate / delayed / takeover) -- preventing multi-site drift.
async fn disconnect_session(
    server: SignalingServer,
    relay_peer_id: String,
    relay_room: String,
    tx: tokio::sync::broadcast::Sender<String>,
) {
    // Clean up SFU resources for the disconnecting peer
    #[cfg(feature = "sfu-mediasoup")]
    {
        // H1 fix: producers register under each stream room's sfu peers (relay_room = the vehicle room != their rooms;
        // a single-room remove_peer cannot reach them -- dead-producer leaks). Remove across rooms and broadcast ProducerClosed per room.
        let binding_dev = server.device_bindings.get(&relay_peer_id).map(|e| e.value().clone());
        let closed_rooms = server.sfu_manager.remove_peer_global(&relay_peer_id);
        // T4 companion: the global cleanup has already broadcast = the owned entities were already surfaced; an empty reverse lookup is no longer an anomaly (prevents constant false alarms).
        let global_announced: usize = closed_rooms.iter().map(|(_, c)| c.len()).sum();
        tracing::info!(
            "SFU: cleaned up peer {} ({} sfu rooms touched)",
            relay_peer_id,
            closed_rooms.len()
        );
        for (room_id, closed) in closed_rooms {
            // T4 refactor: broadcast/owners sync/list events on one unified chain (preventing multi-site drift).
            announce_producers_closed(
                &server,
                &room_id,
                &relay_peer_id,
                binding_dev.as_deref(),
                closed,
            );
        }

        // S4': reclaim the session's own resources (cockpit dps/producers registered under the session:<ws-id> key) --
        // SFU-layer self-reported keys ("consumer") are shared across sessions and cannot be deleted by key; id-targeted reclaim prevents hurting
        // other live cockpits; drag along the orphan consumers bound to dead dps (the sfu second pass).
        let session_key = format!("session:{relay_peer_id}");
        let mine: Vec<String> = server
            .producer_owners
            .iter()
            .filter(|e| e.value() == &session_key)
            .map(|e| e.key().clone())
            .collect();
        if !mine.is_empty() {
            let removed = server.sfu_manager.remove_producers_by_ids(&mine);
            for pid in &mine {
                server.producer_owners.remove(pid);
            }
            for (room_id, closed) in group_closed_by_room(removed) {
                announce_producers_closed(&server, &room_id, &relay_peer_id, None, closed);
            }
            tracing::info!("S4': session {relay_peer_id} reclaimed {} producers", mine.len());
        }
        let t4_seen = server.t4_gone_seen.remove(&relay_peer_id).map(|(_, n)| n).unwrap_or(0);
        // H1 fix (option A): the producers a device holds live under the stream rooms' self-reported peer keys,
        // which session-id cleanup misses -> reverse-lookup producer_owners for all of that device's producers and remove them,
        // broadcasting ProducerClosed per room (web re-subscribes without refresh).
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
                // T2: the owned-minus-removed set difference = ghost registrations (in the owners table but no SFU entity) -- WARN each.
                let removed_ids: std::collections::HashSet<String> =
                    removed.iter().map(|(_, p, _)| p.clone()).collect();
                for pid in owned.iter().filter(|p| !removed_ids.contains(*p)) {
                    tracing::warn!(
                        "remove_producers_by_ids: missed producer {pid} (device {device_id})"
                    );
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
                        &server,
                        &room,
                        &relay_peer_id,
                        Some(&device_id),
                        prods,
                    );
                }
            } else if global_announced == 0 && t4_seen == 0 {
                // T2: the device disconnected with zero registrations and the global cleanup gained nothing = the registration chain broke (05:43-style unsolved cases must not be silent).
                // The normal path where DownstreamGone cleared streams (global_announced>0 or T4 arrived first) -> debug.
                tracing::warn!(
                    "ProducerClosed cleanup: owned empty for device {device_id} (peer {relay_peer_id})"
                );
            } else {
                tracing::debug!(
                    "ProducerClosed cleanup: owned empty, {global_announced} global / {t4_seen} t4 announced (device {device_id})"
                );
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

    // G2: disconnect unbinds the connection-level identity (peer_id -> device_id valid only while the connection lives).
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

    // PIT-183 (the router dragged-along-teardown root fix): an empty RoomManager != a dead SFU room -- in the multi-stream topology
    // a stream room's WS members are consumers only (the producer's host hangs off the gateway session room).
    // Unconditionally tearing down a Router still carrying live producers => the host's stream goes into a black hole (bytes_sent
    // grows but it is never listed by list_producers again). The true destruction moment = all producers gone: the SFU's built-in
    // peers-empty self-teardown chain + the announce-tail deferred teardown (should_deferred_cleanup).
    #[cfg(feature = "sfu-mediasoup")]
    let sfu_alive = server.sfu_manager.room_has_producers(&relay_room);
    #[cfg(not(feature = "sfu-mediasoup"))]
    let sfu_alive = false; // stub has no SFU state -> the gate short-circuits; behaviorally equal to HEAD
    // If room became empty (and no producers left), also remove it from SFU
    #[cfg(feature = "sfu-mediasoup")]
    if crate::sfu::should_teardown(room_removed, sfu_alive) {
        server.sfu_manager.remove_room(&relay_room);
    }
    // E3: an empty room has no state to report -- clean the status registry (G3: the owner registry syncs; during the retention window
    // the owner staying present is what the device re-produce gate (:537) needs -- strictly better than the old behavior)
    if crate::sfu::should_teardown(room_removed, sfu_alive) {
        server.status_registry.remove(&relay_room);
        server.room_owners.remove(&relay_room);
    } else if room_removed {
        tracing::debug!(
            "PIT-183 guard: {relay_room} WS membership empty but producers alive -- router/registry kept"
        );
    }

    let leave_msg =
        SignalingMessage::RoomLeave { room_id: relay_room.clone(), peer_id: relay_peer_id.clone() };
    let _ = tx.send(serde_json::to_string(&leave_msg).unwrap());

    tracing::info!("Peer {} disconnected from room {}", relay_peer_id, relay_room);
}

/// I3 review: the StatusReport identity gate -- only Device sessions (vehicle side) or the Host role (a PSK vehicle)
/// may report vehicle status; accounts/other sessions are refused (cockpits cannot forge vehicle state). Returns the denial reason (None = allowed).
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

/// I3 review: the StatusReport gate + audit (C15) -- on refusal returns an Error 4031 response (sent by the caller),
/// and records AuthorizationDenied (who/action/details).
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
    // S0: the session's negotiated dialect (the capability-domain gates read this, never the packet version).
    negotiated: u32,
) -> Option<SignalingMessage> {
    let sfu = &server.sfu_manager;
    match msg {
        SignalingMessage::CreateWebRtcTransport { room_id, peer_id: msg_peer_id, direction } => {
            // PIT-65: use the message peer_id (each web page has a unique sfuPeerId), not the session relay_peer_id
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
                    sctp_parameters: created.sctp_parameters,
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
            // PIT-65: use the message peer_id -- consistent with create/consume
            let sfu_peer_id = msg_peer_id.as_str();
            match sfu
                .connect_transport(room_id, sfu_peer_id, transport_id, dtls_parameters.clone())
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
        // -- P1 (client-dual-form): the mediasoup-client standard negotiation face --
        // S4/a4: the emergency-stop WS audit copy -- trace only (execution and verify adjudication live in the vehicle actuator = the audit's main site,
        // PLAN S11.6 seat 3). Returns None = no response (copy semantics = no receipt needed, avoiding a reply storm).
        SignalingMessage::ControlAudit { room_id, seq, cmd, sig_present } => {
            audit::log_event(audit::AuditEvent::EstopAudit {
                room_id: room_id.clone(),
                seq: *seq,
                cmd: cmd.clone(),
                sig_present: *sig_present,
            });
            None
        }
        SignalingMessage::GetRouterRtpCapabilities { room_id } => {
            // the input to Device.load() (the C18 official flow); if the room does not exist the Router is lazily created (the same path as create transport).
            match tokio::time::timeout(
                std::time::Duration::from_secs(5),
                sfu.router_rtp_capabilities(room_id),
            )
            .await
            {
                Ok(Ok(capabilities)) => Some(SignalingMessage::RouterRtpCapabilities {
                    room_id: room_id.clone(),
                    capabilities,
                }),
                Ok(Err(e)) => {
                    tracing::error!("SFU: get router rtp capabilities failed: {e}");
                    Some(SignalingMessage::Error {
                        code: 5000,
                        message: format!("GetRouterRtpCapabilities failed: {e}"),
                    })
                }
                Err(_) => {
                    tracing::error!("SFU: get router rtp capabilities timed out after 5s");
                    Some(SignalingMessage::Error {
                        code: 5000,
                        message: "GetRouterRtpCapabilities timed out".into(),
                    })
                }
            }
        }
        SignalingMessage::SetPreferredLayers {
            room_id,
            peer_id: msg_peer_id,
            consumer_id,
            spatial_layer,
            temporal_layer,
        } => {
            // the PIT-65 discipline: use the message peer_id. The ack-on-Error{code:0} precedent (the transport_connected shape).
            let sfu_peer_id = msg_peer_id.as_str();
            match tokio::time::timeout(
                std::time::Duration::from_secs(5),
                sfu.set_preferred_layers(
                    room_id,
                    sfu_peer_id,
                    consumer_id,
                    *spatial_layer,
                    *temporal_layer,
                ),
            )
            .await
            {
                Ok(Ok(())) => {
                    tracing::info!(
                        "SFU: consumer {consumer_id} preferred layers {spatial_layer}/{temporal_layer:?} (peer {sfu_peer_id})"
                    );
                    Some(SignalingMessage::Error {
                        code: 0,
                        message: "preferred_layers_set".into(),
                    })
                }
                Ok(Err(e)) => {
                    tracing::warn!("SFU: set_preferred_layers failed: {e}");
                    Some(SignalingMessage::Error {
                        code: 5000,
                        message: format!("SetPreferredLayers failed: {e}"),
                    })
                }
                Err(_) => {
                    tracing::error!("SFU: set_preferred_layers timed out after 5s");
                    Some(SignalingMessage::Error {
                        code: 5000,
                        message: "SetPreferredLayers timed out".into(),
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
            label,
        } => {
            // PIT-65: use the message peer_id -- consistent with create/connect/consume
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
            // G3 gate: vehicles auto-allowed (their own streams); accounts denied produce (cockpits only consume); legacy allowed.
            if let Err(reason) = identity.can_produce(room_id) {
                let detail = format!("{reason} (peer={peer_id}, room={room_id})");
                tracing::warn!("Produce denied: {detail}");
                audit::log_event(AuditEvent::AuthorizationDenied {
                    action: "produce".into(),
                    peer_id: peer_id.to_string(),
                    detail: detail.clone(),
                });
                return Some(SignalingMessage::Error { code: 4031, message: detail });
            }
            // H2: audio rooms allow only audio producers (full-interconnect opus conference semantics) -- 4031 + audit (C15).
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
                    label.clone(),
                )
                .await
            {
                Ok(result) => {
                    // G3: a vehicle producer registers its owning device (consume authorization defense-in-depth).
                    // S4': same for data producers -- when device is absent fall to the session fallback key (reclaimed by id on disconnect).
                    let owner = server
                        .device_id_of(peer_id)
                        .unwrap_or_else(|| format!("session:{peer_id}"));
                    server.producer_owners.insert(result.producer_id.clone(), owner);
                    // Broadcast NewProducer to all peers in room
                    // producer-label T1.2: the label passes through from the Produce request (None = the old host form)
                    let broadcast = SignalingMessage::NewProducer {
                        room_id: room_id.clone(),
                        producer_id: result.producer_id.clone(),
                        peer_id: peer_id.to_string(),
                        kind: result.kind,
                        label: label.clone(),
                    };
                    // H1 root-cause fix: the broadcast must go to the produce target room's channel (not the calling session's own
                    // channel -- the agent session room = vehicle, web consumers subscribe to per-stream rooms -> on the old path
                    // web never received new producers -> after a host restart the black screen was only fixable by refreshing).
                    let room_tx = server.get_or_create_channel(room_id);
                    match room_tx.send(serde_json::to_string(&broadcast).unwrap()) {
                        Ok(n) => tracing::info!(
                            "NewProducer broadcast: {} channel receivers in room {}",
                            n,
                            room_id
                        ),
                        Err(_) => tracing::warn!(
                            "NewProducer broadcast: no receivers in room {}",
                            room_id
                        ),
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
            // PIT-65: use the message peer_id (each web page has a unique sfuPeerId), not the session relay_peer_id --
            // otherwise multiple pages sharing admin -> recv_transports overwrite each other -> the consumer attaches to the wrong transport -> black screen
            let sfu_peer_id = msg_peer_id.as_str();
            // G3 gate: accounts may only consume producers of vehicles they are authorized for (RoomJoin already filtered by room owner vehicle --
            // this is defense-in-depth per producer, guarding against other vehicles' streams mixed into the room).
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
            // PIT-65: use the message peer_id (each web page has a unique sfuPeerId), consistent with produce/consume
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
            // G3 gate: the same matrix as produce -- vehicles auto-allowed (their own DC); accounts denied by default; legacy allowed.
            // P1 fix (the true F8 landing point): after the all-SFU decision (D 2026-08-25, rooms uniformly DeviceStream) the
            // SDP-relay can_control gate is unreachable -- the control DC creation point = this message. Account control exemption:
            // operator/admin (can_control) may create the control DataProducer; viewer/dispatcher get an explicit
            // 4012 + audit (the D273 terminal red card, visible instead of silent).
            if let Err(reason) = identity.can_produce(room_id) {
                // S0 tightening: role present + dialect >= PROTOCOL_MIN_CONTROL_DC to pass (the I5 first user).
                if identity.can_control()
                    && negotiated >= mediaservo_common::protocol::PROTOCOL_MIN_CONTROL_DC
                {
                    tracing::info!(
                        "CreateDataProducer: account control DC passed the role gate (peer={peer_id}, room={room_id}, label={label})"
                    );
                } else {
                    let mut detail =
                        format!("control_denied: {reason} (peer={peer_id}, room={room_id})");
                    if identity.can_control() {
                        // role has permission, dialect below v2 -- distinct wording so troubleshooting old endpoints is unambiguous.
                        detail = format!(
                            "control_denied: requires_protocol>={} negotiated={negotiated} (peer={peer_id}, room={room_id})",
                            mediaservo_common::protocol::PROTOCOL_MIN_CONTROL_DC
                        );
                    }
                    tracing::warn!("CreateDataProducer denied: {detail}");
                    audit::log_event(AuditEvent::AuthorizationDenied {
                        action: "control_dc".into(),
                        peer_id: peer_id.to_string(),
                        detail: detail.clone(),
                    });
                    return Some(SignalingMessage::Error { code: 4012, message: detail });
                }
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
                    // G3: a vehicle data producer registers its owning device (consume_data authorization defense-in-depth,
                    // the same id space as media producers -- UUIDs are unique, no collision).
                    // S4': device absent (a cockpit account session) falls to the `session:<ws-id>` fallback key --
                    // disconnect cleanup reclaims in the same space (the old version only warned = cockpit dp disconnects were never reclaimed =
                    // the monotonic `found N data producers` leak and the `StreamId reserved` ceiling root cause).
                    let owner = server
                        .device_id_of(peer_id)
                        .unwrap_or_else(|| format!("session:{peer_id}"));
                    server.producer_owners.insert(result.data_producer_id.clone(), owner);
                    // Broadcast NewDataProducer to all peers in room (late-joiner sync)
                    let broadcast = SignalingMessage::NewDataProducer {
                        room_id: room_id.clone(),
                        data_producer_id: result.data_producer_id.clone(),
                        peer_id: peer_id.to_string(),
                        label: label.clone(),
                        protocol: protocol.clone(),
                    };
                    let _ = server
                        .get_or_create_channel(room_id)
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
            // G3 gate: the same matrix as consume -- accounts may only consume data producers of devices on vehicles they are authorized for.
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
                    // S2d: the official contract -- the consumer receives forwarded messages via a negotiated DC (id=stream_id).
                    sctp_stream_parameters: result.sctp_stream_parameters,
                    label: result.label,
                    protocol: result.protocol,
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
            // H2: G3 gate -- when an account queries producer stats, can_consume is checked against the owning device
            // (the same matrix as consume, defense-in-depth); consumer queries/ownerless producers pass (UUIDs are not enumerable).
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
                    match sfu.producer_stats(pid).await {
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
                    match sfu.consumer_stats(cid).await {
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
        // F1/T4: the gateway reports a downstream sub-session's death -- precise (room, peer) cleanup + ProducerClosed broadcast.
        // Identity gate: only device sessions (agents) may report; an old server dropping the unknown variant on parse = additive.
        SignalingMessage::DownstreamGone { peer_id: gone_peer, room_id } => {
            let Some(device) = server.device_id_of(peer_id) else {
                tracing::warn!(
                    "DownstreamGone rejected: session {peer_id} has no device binding (reported key={gone_peer})"
                );
                return None;
            };
            *server.t4_gone_seen.entry(device.clone()).or_insert(0) += 1;
            let closed = sfu.remove_peer_in_room(room_id, gone_peer);
            if closed.is_empty() {
                tracing::info!(
                    "DownstreamGone: {gone_peer} has no SFU entity in room {room_id} (never produced / key drift / already cleaned)"
                );
            } else {
                announce_producers_closed(server, room_id, gone_peer, Some(&device), closed);
            }
            None
        }
        _ => None,
    }
}

/// F1/T4: the unified ProducerClosed announcement chain -- shared by the close global cleanup / device reverse lookup / DownstreamGone
/// three call sites (preventing branch drift). Duties: broadcast each entry (Ok(n)/Err visible, C15) + owners-table sync
/// + the StreamDestroy list event (when device-owned).
///   S4': the remove_* triple manifest -> grouped by room (the unified broadcast chain's reuse shape).
fn group_closed_by_room(
    list: Vec<(String, String, mediaservo_common::protocol::MediaKind)>,
) -> Vec<(String, Vec<(String, mediaservo_common::protocol::MediaKind)>)> {
    let mut acc: std::collections::HashMap<
        String,
        Vec<(String, mediaservo_common::protocol::MediaKind)>,
    > = std::collections::HashMap::new();
    for (room, id, kind) in list {
        acc.entry(room).or_default().push((id, kind));
    }
    acc.into_iter().collect()
}

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
        // owners-table sync (prevents ghost registrations -> the next reverse lookup falsely reporting missed).
        if device_id.is_some() {
            server.producer_owners.remove(&producer_id);
        }
    }
    tracing::info!("SFU: broadcast ProducerClosed for peer {reporter_peer} in room {room_id}");
    if let Some(device) = device_id {
        // second-level list refresh: a stream-offline event (the frontend only uses it as a refresh trigger).
        let ev = crate::admin::AdminEvent::StreamDestroy {
            device_id: device.to_string(),
            stream_id: room_id.to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
        };
        let _ = server.admin_events.send(serde_json::to_string(&ev).unwrap_or_default());
    }
    // PIT-183 deferred teardown: the registry/owners kept by the gate while consumers hit zero are reclaimed at this
    // all-producers-gone announcement moment (the RoomManager view has already dispersed the room = when the delayed cleanup takes effect; if K2 timing
    // still has consumers in the room, skip and leave it to its leave path's should_teardown -- double insurance, no leak).
    #[cfg(feature = "sfu-mediasoup")]
    // the deferred teardown only makes sense in the SFU posture (the stub has no producer state,
    // cleanup proceeds normally through the leave-path guard above)
    if crate::sfu::should_deferred_cleanup(
        server.room_manager.get_room(room_id).is_none(),
        server.sfu_manager.room_has_producers(room_id),
    ) {
        server.status_registry.remove(room_id);
        server.room_owners.remove(room_id);
        tracing::info!("PIT-183 deferred cleanup done for room {room_id}");
    }
}

/// The broadcast channel for relay messages: `EncoderStatus` routes by the stream sub-room declared inside the message
/// (browser consumers are on `vehicle_test`-style sub-room channels, while device session channels are on the vehicle room),
/// other relay messages keep the session room.
fn relay_target_room(
    server: &SignalingServer,
    text: &str,
    fallback_room: &str,
) -> tokio::sync::broadcast::Sender<String> {
    match serde_json::from_str::<SignalingMessage>(text) {
        Ok(SignalingMessage::EncoderStatus { room_id, .. }) => {
            server.get_or_create_channel(&room_id)
        }
        _ => server.get_or_create_channel(fallback_room),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mediaservo_common::protocol::PeerRole;

    /// Test server constructor: under the sfu-mediasoup feature a live SfuManager is required (the G2 side fix of the
    /// `--tests --benches` compile break -- a legacy leftover where the two-arg constructor does not exist with the feature on).
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

    #[test]
    fn resume_prune_and_ct_eq() {
        use std::collections::HashMap;
        let mut t: HashMap<(String, String), HeldResume> = HashMap::new();
        for i in 0..35u64 {
            t.insert(
                (format!("dev{i}"), "r".into()),
                HeldResume { peer_id: format!("p{i}"), nonce: "n".into(), negotiated: 3, seq: i },
            );
            prune_resume(&mut t, RESUME_GLOBAL_CAP);
            assert!(t.len() <= RESUME_GLOBAL_CAP);
        }
        // the oldest 0..2 evicted, the newest must be present
        assert!(!t.contains_key(&("dev0".to_string(), "r".to_string())));
        assert!(t.contains_key(&("dev34".to_string(), "r".to_string())));
        assert!(ct_eq(b"abc123", b"abc123"));
        assert!(!ct_eq(b"abc123", b"abc124"));
        assert!(!ct_eq(b"abc", b"abc123"));
    }

    #[test]
    fn pong_budget_matrix() {
        use std::time::Duration;
        let i = Duration::from_secs(5);
        assert!(
            !pong_budget_exceeded(Duration::from_secs(10), i, 2),
            "a 2-silence window is within budget"
        );
        assert!(
            !pong_budget_exceeded(Duration::from_secs(15), i, 2),
            "=15s sits exactly on the budget line (strictly >)"
        );
        assert!(
            pong_budget_exceeded(Duration::from_secs(16), i, 2),
            "16s over budget = synthesize a disconnect"
        );
        assert!(
            pong_budget_exceeded(Duration::from_secs(3), Duration::from_secs(1), 1),
            "1s x 2 budget 3s <"
        );
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
        assert_eq!(
            sub_rx.try_recv().unwrap(),
            "es",
            "EncoderStatus should land on the sub-room channel declared in the message"
        );
        assert!(whole_rx.try_recv().is_err(), "it must not land on the vehicle channel");
        let sdp = serde_json::to_string(&SignalingMessage::Sdp {
            room_id: "whatever".into(),
            target: None,
            sdp: "s".into(),
        })
        .unwrap();
        relay_target_room(&server, &sdp, "vehicle").send("sdp".into()).unwrap();
        assert_eq!(
            whole_rx.try_recv().unwrap(),
            "sdp",
            "non-EncoderStatus keeps the fallback room"
        );
    }

    #[tokio::test]
    async fn push_config_delivers_to_room_host() {
        let server = new_test_server().await;
        server.room_manager.join_room("vehicle-1", "veh-peer", &PeerRole::Host).expect("join host");
        // room channel subscriber (the receiving end simulating a vehicle session's connection)
        let tx = server.get_or_create_channel("vehicle-1");
        let mut rx = tx.subscribe();

        server
            .push_config("vehicle-1", "[[cameras]]\nid = \"cam0\"\n", 7, Some("ops-admin".into()))
            .expect("push");
        let text = rx.try_recv().expect("the channel should hold a ConfigPush");
        match serde_json::from_str::<SignalingMessage>(&text).expect("parse") {
            SignalingMessage::ConfigPush { room_id, target, config, version, actor } => {
                assert_eq!(room_id, "vehicle-1");
                assert_eq!(target, "veh-peer", "target should be the room's host peer");
                assert!(config.contains("cam0"));
                assert_eq!(version, 7);
                assert_eq!(actor.as_deref(), Some("ops-admin"), "the admin actor rides the push (T1b6)");
            }
            other => panic!("expected ConfigPush, got {other:?}"),
        }
    }

    /// I3 review: the StatusReport gate -- Device sessions or the Host role pass; accounts/Legacy non-Host are refused + audited.
    #[test]
    fn status_report_gate_allows_device_and_host_only() {
        use crate::roles::{AccountIdentity, CockpitRole, SessionIdentity};
        let device = SessionIdentity::Device("ms-car1".into());
        let account = SessionIdentity::Account(AccountIdentity {
            username: "u".into(),
            role: CockpitRole::Viewer,
            vehicles: vec![],
        });
        // allowed: device sessions (vehicle side, any role); legacy + the Host role (a PSK vehicle)
        assert_eq!(status_report_denial_reason(&device, &PeerRole::Host), None);
        assert_eq!(status_report_denial_reason(&device, &PeerRole::Remote), None);
        assert_eq!(status_report_denial_reason(&SessionIdentity::Legacy, &PeerRole::Host), None);
        // refused: accounts (cockpits; the Host role is already blocked by the RoomJoin gate -- unreachable); legacy non-Host
        assert!(
            status_report_denial_reason(&account, &PeerRole::Remote).is_some(),
            "accounts are barred from reporting vehicle status (prevents forgery)"
        );
        assert!(status_report_denial_reason(&account, &PeerRole::Consumer).is_some());
        assert!(status_report_denial_reason(&SessionIdentity::Legacy, &PeerRole::Remote).is_some());
    }

    /// I3 review: the refusal path must return 4031 + audit AuthorizationDenied(status_report).
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
            .expect("refusal must return an Error response");
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
            "refusal must audit AuthorizationDenied(status_report): {after:?}"
        );
        // the allow path: no response, no audit
        let device = SessionIdentity::Device("ms-car1".into());
        assert!(status_report_denial(&device, &PeerRole::Remote, "p", "r").is_none());
    }

    #[tokio::test]
    #[cfg(feature = "sfu-mediasoup")] // handle_sfu_message exists only in the SFU posture (paying down the on-record stub --tests E0425 case)
    async fn push_config_errors_when_room_has_no_host() {
        let server = new_test_server().await;
        let err = server.push_config("empty-room", "cfg", 1, None).unwrap_err();
        assert!(err.contains("has no host"), "a host-less room must error: {err}");
    }

    /// F1/T4: the DownstreamGone identity gate -- a session with no device binding (browser/anonymous) reporting must be refused.
    #[tokio::test]
    #[cfg(feature = "sfu-mediasoup")] // handle_sfu_message exists only in the SFU posture (paying down the on-record stub --tests E0425 case)
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
            1,
        )
        .await;
        assert!(resp.is_none());
    }

    /// F1/T4: a device session reporting an unknown room = an idempotent no-op (no panic, no response) -- replay/race safe.
    #[tokio::test]
    #[cfg(feature = "sfu-mediasoup")] // handle_sfu_message exists only in the SFU posture (paying down the on-record stub --tests E0425 case)
    async fn downstream_gone_idempotent_on_unknown_room() {
        let server = new_test_server().await;
        server.device_bindings.insert("agent-1".into(), "dev-x".into());
        let (tx, _rx) = tokio::sync::broadcast::channel::<String>(4);
        for room in ["nope", "also_nope"] {
            let resp = handle_sfu_message(
                &SignalingMessage::DownstreamGone { peer_id: "host".into(), room_id: room.into() },
                &server,
                &tx,
                "agent-1",
                &SessionIdentity::Device("dev-x".into()),
                1,
            )
            .await;
            assert!(resp.is_none(), "unknown room {room} should be silently idempotent");
        }
    }
}
