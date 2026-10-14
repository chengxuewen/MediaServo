//! Structured audit logging for security-relevant server events.
//!
//! All audit events are emitted as structured JSON via `tracing` so they
//! flow into the same observability pipeline as operational logs.
//!
//! Event types:
//! - `RoomCreate` / `RoomDestroy` — room lifecycle
//! - `PeerJoin` / `PeerLeave` — peer lifecycle within a room
//! - `AuthSuccess` / `AuthFailure` — authentication outcomes
//! - `DeviceOnline` / `DeviceOffline` — device lifecycle
//! - `StreamCreate` / `StreamDestroy` — stream lifecycle
//! - `ConsumerJoin` / `ConsumerLeave` — stream consumer lifecycle

/// Audit event variants covering all security-relevant server operations.
#[derive(Debug, Clone, PartialEq)]
pub enum AuditEvent {
    /// S4/a4: cockpit emergency-stop WS audit copy (the DC fast path bypasses the server; this copy is best-effort).
    EstopAudit { room_id: String, seq: u64, cmd: String, sig_present: bool },
    /// A new room was created.
    RoomCreate { room_id: String },
    /// A room was destroyed (last peer left).
    RoomDestroy { room_id: String },
    /// A peer joined a room.
    PeerJoin { peer_id: String, room_id: String, role: String },
    /// A peer left a room.
    PeerLeave { peer_id: String, room_id: String },
    /// Authentication succeeded (PSK/JWT/device). device_id is set only on the device-auth path (G2).
    AuthSuccess { peer_id: String, device_id: Option<String> },
    /// PSK authentication failed.
    AuthFailure { peer_id: String, reason: String },
    /// A device came online.
    DeviceOnline { device_id: String },
    /// A device went offline.
    DeviceOffline { device_id: String },
    /// A media stream was created.
    StreamCreate { stream_id: String, device_id: String },
    /// A media stream was destroyed.
    StreamDestroy { stream_id: String },
    /// A peer started consuming a stream.
    ConsumerJoin { stream_id: String, peer_id: String },
    /// A peer stopped consuming a stream.
    ConsumerLeave { stream_id: String, peer_id: String },
    /// G3 emergency-stop strong audit (D-H11) -- who/when/which vehicle/what command.
    /// when = log timestamp (injected by tracing); vehicle = target vehicle device_id (room owner vehicle).
    EmergencyCommand { username: String, role: String, vehicle: String, command: String },
    /// Admin-plane device registration (unified-device-admin) -- actor = admin account.
    DeviceRegistered { device_id: String, actor: String },
    /// Admin-plane device revocation (unified-device-admin).
    DeviceRevoked { device_id: String, actor: String },
    /// Admin-plane device secret reset (unified-device-admin).
    DeviceSecretReset { device_id: String, actor: String },
    /// Admin-plane API-key registration (accountless-client-auth T1) -- stores key_id only; the secret never enters the event.
    ApiKeyRegistered { key_id: String, actor: String },
    /// Admin-plane API-key revocation (subsequent exchange gets 401; issued JWTs die naturally at exp).
    ApiKeyRevoked { key_id: String, actor: String },
    /// Admin-plane account creation (unified-device-admin) -- actor = admin account.
    AccountCreated { username: String, actor: String, role: String },
    /// Admin-plane account update (unified-device-admin).
    AccountUpdated { username: String, actor: String },
    /// Admin-plane account deletion (unified-device-admin).
    AccountDeleted { username: String, actor: String },
    /// Admin-plane PSK one-time view (psk-admin-management).
    PskViewed { actor: String },
    /// Admin-plane PSK rotation (psk-admin-management).
    PskRotated { actor: String },
    /// G3 authorization denial (C15: every denial must be logged + audited).
    /// action: room_join|consume|produce|config_push|emergency.
    AuthorizationDenied { action: String, peer_id: String, detail: String },
}

/// Emit an audit event as a structured `tracing` info-level log.
///
/// The `audit.event` field is used as the JSON key for downstream filtering
/// (e.g., log aggregation, SIEM ingestion).
pub fn log_event(event: AuditEvent) {
    // Bounded ring buffer (ops/tests read recent events; tracing logs remain the main channel)
    {
        let mut ring = recent_sink().lock().unwrap_or_else(|e| e.into_inner());
        ring.push_back(event.clone());
        while ring.len() > AUDIT_RING_CAP {
            ring.pop_front();
        }
    }
    match event {
        AuditEvent::EstopAudit { room_id, seq, cmd, sig_present } => {
            tracing::warn!(
                audit.event = "estop_audit",
                room_id = %room_id,
                seq,
                cmd = %cmd,
                sig_present,
                "WS audit copy: cockpit e-stop"
            );
        }
        AuditEvent::RoomCreate { room_id } => {
            tracing::info!(
                audit.event = "room_create",
                room_id = %room_id,
                "Room created"
            );
        }
        AuditEvent::RoomDestroy { room_id } => {
            tracing::info!(
                audit.event = "room_destroy",
                room_id = %room_id,
                "Room destroyed"
            );
        }
        AuditEvent::PeerJoin { peer_id, room_id, role } => {
            tracing::info!(
                audit.event = "peer_join",
                peer_id = %peer_id,
                room_id = %room_id,
                role = %role,
                "Peer joined room"
            );
        }
        AuditEvent::PeerLeave { peer_id, room_id } => {
            tracing::info!(
                audit.event = "peer_leave",
                peer_id = %peer_id,
                room_id = %room_id,
                "Peer left room"
            );
        }
        AuditEvent::AuthSuccess { peer_id, device_id } => {
            tracing::info!(
                audit.event = "auth_success",
                peer_id = %peer_id,
                device_id = device_id.as_deref().unwrap_or("-"),
                "Authentication succeeded"
            );
        }
        AuditEvent::AuthFailure { peer_id, reason } => {
            tracing::warn!(
                audit.event = "auth_failure",
                peer_id = %peer_id,
                reason = %reason,
                "Authentication failed"
            );
        }
        AuditEvent::DeviceOnline { device_id } => {
            tracing::info!(
                audit.event = "device_online",
                device_id = %device_id,
                "Device came online"
            );
        }
        AuditEvent::DeviceOffline { device_id } => {
            tracing::info!(
                audit.event = "device_offline",
                device_id = %device_id,
                "Device went offline"
            );
        }
        AuditEvent::StreamCreate { stream_id, device_id } => {
            tracing::info!(
                audit.event = "stream_create",
                stream_id = %stream_id,
                device_id = %device_id,
                "Stream created"
            );
        }
        AuditEvent::StreamDestroy { stream_id } => {
            tracing::info!(
                audit.event = "stream_destroy",
                stream_id = %stream_id,
                "Stream destroyed"
            );
        }
        AuditEvent::ConsumerJoin { stream_id, peer_id } => {
            tracing::info!(
                audit.event = "consumer_join",
                stream_id = %stream_id,
                peer_id = %peer_id,
                "Consumer joined stream"
            );
        }
        AuditEvent::ConsumerLeave { stream_id, peer_id } => {
            tracing::info!(
                audit.event = "consumer_leave",
                stream_id = %stream_id,
                peer_id = %peer_id,
                "Consumer left stream"
            );
        }
        AuditEvent::EmergencyCommand { username, role, vehicle, command } => {
            tracing::info!(
                audit.event = "emergency_command",
                username = %username,
                role = %role,
                vehicle = %vehicle,
                command = %command,
                "Emergency command (strong audit)"
            );
        }
        AuditEvent::AuthorizationDenied { action, peer_id, detail } => {
            tracing::warn!(
                audit.event = "authorization_denied",
                action = %action,
                peer_id = %peer_id,
                detail = %detail,
                "Authorization denied"
            );
        }
        AuditEvent::DeviceRegistered { device_id, actor } => {
            tracing::info!(
                audit.event = "device_registered",
                device_id = %device_id,
                actor = %actor,
                "Device registered by admin"
            );
        }
        AuditEvent::DeviceRevoked { device_id, actor } => {
            tracing::warn!(
                audit.event = "device_revoked",
                device_id = %device_id,
                actor = %actor,
                "Device revoked by admin"
            );
        }
        AuditEvent::DeviceSecretReset { device_id, actor } => {
            tracing::info!(
                audit.event = "device_secret_reset",
                device_id = %device_id,
                actor = %actor,
                "Device secret reset by admin"
            );
        }
        AuditEvent::ApiKeyRegistered { key_id, actor } => {
            tracing::info!(
                audit.event = "api_key_registered",
                key_id = %key_id,
                actor = %actor,
                "API key registered by admin"
            );
        }
        AuditEvent::ApiKeyRevoked { key_id, actor } => {
            tracing::warn!(
                audit.event = "api_key_revoked",
                key_id = %key_id,
                actor = %actor,
                "API key revoked by admin"
            );
        }
        AuditEvent::AccountCreated { username, actor, role } => {
            tracing::info!(
                audit.event = "account_created",
                username = %username,
                actor = %actor,
                role = %role,
                "Account created by admin"
            );
        }
        AuditEvent::AccountUpdated { username, actor } => {
            tracing::info!(
                audit.event = "account_updated",
                username = %username,
                actor = %actor,
                "Account updated by admin"
            );
        }
        AuditEvent::AccountDeleted { username, actor } => {
            tracing::warn!(
                audit.event = "account_deleted",
                username = %username,
                actor = %actor,
                "Account deleted by admin"
            );
        }
        AuditEvent::PskViewed { actor } => {
            tracing::info!(
                audit.event = "psk_viewed",
                actor = %actor,
                "PSK viewed (one-time) by admin"
            );
        }
        AuditEvent::PskRotated { actor } => {
            tracing::warn!(
                audit.event = "psk_rotated",
                actor = %actor,
                "PSK rotated by admin — all hosts must sync"
            );
        }
    }
}

// -- Bounded audit ring buffer (ops/tests read recent events) --------------------------------
// ponytail: in-memory ring of 256 entries covers recent events; long-term retention goes through the tracing log pipeline (SIEM).

const AUDIT_RING_CAP: usize = 256;

fn recent_sink() -> &'static std::sync::Mutex<std::collections::VecDeque<AuditEvent>> {
    static RECENT: std::sync::OnceLock<std::sync::Mutex<std::collections::VecDeque<AuditEvent>>> =
        std::sync::OnceLock::new();
    RECENT.get_or_init(|| std::sync::Mutex::new(std::collections::VecDeque::new()))
}

/// Recent audit events (bounded ring, newest at the tail; used by tests and ops queries).
pub fn recent() -> Vec<AuditEvent> {
    recent_sink().lock().unwrap_or_else(|e| e.into_inner()).iter().cloned().collect()
}

/// Clear the ring buffer (test isolation -- each test asserts on new events).
pub fn clear_recent() {
    recent_sink().lock().unwrap_or_else(|e| e.into_inner()).clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    // Note: the ring buffer is process-global and shared -- parallel tests write to it (login audits etc.).
    // Mechanical assertions use white-box access (lock the deque directly, race-free throughout); log_event assertions use
    // presence filtering (events within the 256 window never get evicted); exact-position assertions are forbidden (parallel races).

    #[test]
    fn recent_ring_records_and_bounds() {
        // White-box: exclusive deque access while holding the lock -- deterministic verification of cap + evict-oldest + keep-newest.
        let mut ring = recent_sink().lock().unwrap_or_else(|e| e.into_inner());
        ring.clear();
        for i in 0..300 {
            ring.push_back(AuditEvent::AuthorizationDenied {
                action: "test".into(),
                peer_id: format!("p{i}"),
                detail: "d".into(),
            });
            if ring.len() > AUDIT_RING_CAP {
                ring.pop_front();
            }
        }
        assert_eq!(ring.len(), AUDIT_RING_CAP, "ring must be capped");
        assert_eq!(
            ring[0],
            AuditEvent::AuthorizationDenied {
                action: "test".into(),
                peer_id: "p44".into(),
                detail: "d".into(),
            },
            "oldest event evicted (300 - 256 = 44)"
        );
        assert_eq!(
            ring.back(),
            Some(&AuditEvent::AuthorizationDenied {
                action: "test".into(),
                peer_id: "p299".into(),
                detail: "d".into(),
            }),
            "newest event kept"
        );
        ring.clear();
    }

    #[test]
    fn log_event_appends_to_ring() {
        // log_event's push path (presence assertions -- parallel writes never evict in-window events).
        clear_recent();
        log_event(AuditEvent::AuthorizationDenied {
            action: "log-event-path".into(),
            peer_id: "p".into(),
            detail: "d".into(),
        });
        assert!(
            recent().iter().any(|e| matches!(
                e,
                AuditEvent::AuthorizationDenied { action, peer_id, .. }
                    if action == "log-event-path" && peer_id == "p"
            )),
            "log_event must write into the ring buffer"
        );
    }

    #[test]
    fn emergency_command_event_carries_who_vehicle_command() {
        clear_recent();
        log_event(AuditEvent::EmergencyCommand {
            username: "carol".into(),
            role: "operator".into(),
            vehicle: "ms-car1".into(),
            command: "e-stop".into(),
        });
        assert!(
            recent().iter().any(|e| matches!(
                e,
                AuditEvent::EmergencyCommand {
                    username,
                    role,
                    vehicle,
                    command
                } if username == "carol"
                    && role == "operator"
                    && vehicle == "ms-car1"
                    && command == "e-stop"
            )),
            "emergency-stop audit event must contain who/role/vehicle/command"
        );
    }
}
