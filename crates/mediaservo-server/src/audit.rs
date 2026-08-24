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
    /// A new room was created.
    RoomCreate { room_id: String },
    /// A room was destroyed (last peer left).
    RoomDestroy { room_id: String },
    /// A peer joined a room.
    PeerJoin { peer_id: String, room_id: String, role: String },
    /// A peer left a room.
    PeerLeave { peer_id: String, room_id: String },
    /// Authentication succeeded (PSK/JWT/device). device_id 仅设备认证路径有值（G2）。
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
    /// G3 急停强审计（D-H11）— 谁/何时/哪个车/什么命令。
    /// when = 日志时间戳（tracing 注入）；vehicle = 目标车 device_id（房间主车）。
    EmergencyCommand { username: String, role: String, vehicle: String, command: String },
    /// 管理面设备注册（unified-device-admin）— actor = 管理员账号。
    DeviceRegistered { device_id: String, actor: String },
    /// 管理面设备吊销（unified-device-admin）。
    DeviceRevoked { device_id: String, actor: String },
    /// 管理面设备密钥重置（unified-device-admin）。
    DeviceSecretReset { device_id: String, actor: String },
    /// 管理面账号创建（unified-device-admin）— actor = 管理员账号。
    AccountCreated { username: String, actor: String, role: String },
    /// 管理面账号更新（unified-device-admin）。
    AccountUpdated { username: String, actor: String },
    /// 管理面账号删除（unified-device-admin）。
    AccountDeleted { username: String, actor: String },
    /// G3 授权拒绝（C15: 所有 denial 必须打日志 + 审计）。
    /// action: room_join|consume|produce|config_push|emergency。
    AuthorizationDenied { action: String, peer_id: String, detail: String },
}

/// Emit an audit event as a structured `tracing` info-level log.
///
/// The `audit.event` field is used as the JSON key for downstream filtering
/// (e.g., log aggregation, SIEM ingestion).
pub fn log_event(event: AuditEvent) {
    // 有界环形缓冲（运维/测试读最近事件; tracing 日志仍是主通道）
    {
        let mut ring = recent_sink().lock().unwrap_or_else(|e| e.into_inner());
        ring.push_back(event.clone());
        while ring.len() > AUDIT_RING_CAP {
            ring.pop_front();
        }
    }
    match event {
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
                "Emergency command (强审计)"
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
    }
}

// ── 有界审计环形缓冲（运维/测试读最近事件）──────────────────────────────────
// ponytail: 内存环形 256 条覆盖最近事件; 长期留存靠 tracing 日志管道（SIEM）。

const AUDIT_RING_CAP: usize = 256;

fn recent_sink() -> &'static std::sync::Mutex<std::collections::VecDeque<AuditEvent>> {
    static RECENT: std::sync::OnceLock<std::sync::Mutex<std::collections::VecDeque<AuditEvent>>> =
        std::sync::OnceLock::new();
    RECENT.get_or_init(|| std::sync::Mutex::new(std::collections::VecDeque::new()))
}

/// 最近审计事件（有界环形，新事件在尾部；测试与运维查询用）。
pub fn recent() -> Vec<AuditEvent> {
    recent_sink().lock().unwrap_or_else(|e| e.into_inner()).iter().cloned().collect()
}

/// 清空环形缓冲（测试隔离用 — 各测试按新事件断言）。
pub fn clear_recent() {
    recent_sink().lock().unwrap_or_else(|e| e.into_inner()).clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    // 注意: 环形缓冲是进程全局共享 — 并行测试会写入（login 审计等）。
    // 机械性断言用白盒（直接持锁操作 deque，全程无竞争）; log_event 断言用
    // presence 过滤（窗口 256 内不会被挤掉），禁止精确位置断言（并行竞态）。

    #[test]
    fn recent_ring_records_and_bounds() {
        // 白盒: 持锁期间独占 deque — 确定性验证 封顶 + 淘汰最旧 + 保留最新。
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
        assert_eq!(ring.len(), AUDIT_RING_CAP, "环形必须封顶");
        assert_eq!(
            ring[0],
            AuditEvent::AuthorizationDenied {
                action: "test".into(),
                peer_id: "p44".into(),
                detail: "d".into(),
            },
            "最旧事件被淘汰（300 - 256 = 44）"
        );
        assert_eq!(
            ring.back(),
            Some(&AuditEvent::AuthorizationDenied {
                action: "test".into(),
                peer_id: "p299".into(),
                detail: "d".into(),
            }),
            "最新事件保留"
        );
        ring.clear();
    }

    #[test]
    fn log_event_appends_to_ring() {
        // log_event 的 push 路径（presence 断言 — 并行写入不挤掉窗口内事件）。
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
            "log_event 必须写入环形缓冲"
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
            "急停审计事件必须含 谁/角色/车/命令"
        );
    }
}
