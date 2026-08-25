use crate::audit::{self, AuditEvent};
use dashmap::DashMap;
use mediaservo_common::error::CoreError;
use mediaservo_common::protocol::PeerRole;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum RoomType {
    P2P,
    DeviceStream,
}

#[derive(Debug, Clone, Serialize)]
pub struct Consumer {
    pub peer_id: String,
    /// ISO 时间字符串 (API 展示) — 原 Instant 被 serde(skip) 排除导致前端 undefined (PIT-66)
    pub connected_since: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RoomSnapshot {
    pub id: String,
    pub room_type: RoomType,
    pub device_id: Option<String>,
    pub stream_id: Option<String>,
    pub host: Option<String>,
    pub remote: Option<String>,
    pub consumers: Vec<Consumer>,
    #[serde(skip)]
    pub created_at: Instant,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeviceSnapshot {
    pub device_id: String,
    #[serde(skip)]
    pub online_since: Instant,
    pub streams: Vec<StreamSnapshot>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StreamSnapshot {
    pub stream_id: String,
    pub consumers: Vec<Consumer>,
    /// 推流在线（multi-stream P2: host-agent StatusReport.StreamFlowJson.connected）。
    pub online: bool,
}

/// A room with at most one Host. P2P rooms have exactly one Remote;
/// DeviceStream rooms support N consumers.
#[derive(Debug)]
pub struct Room {
    pub id: String,
    pub room_type: RoomType,
    pub device_id: Option<String>,
    pub stream_id: Option<String>,
    pub host: Option<String>,
    pub remote: Option<String>, // KEPT for backward compat (P2P rooms)
    pub consumers: Vec<Consumer>,
    pub created_at: Instant,
}

/// In-memory room state managed by the signaling server.
#[derive(Debug, Clone)]
pub struct RoomManager {
    rooms: Arc<DashMap<String, Room>>,
}

impl RoomManager {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            rooms: Arc::new(DashMap::new()),
        }
    }

    /// Join a room as Host, Remote, or Consumer.
    ///
    /// - `Host`: only 1 host slot (RoomFull if taken).
    /// - `Remote` in a P2P room: 1 remote slot (RoomFull if taken).
    /// - `Remote` in a DeviceStream room: pushes to consumers (N allowed).
    /// - `Consumer`: pushes to consumers (N allowed).
    /// - New room creation: Host → P2P, Remote/Consumer → DeviceStream.
    pub fn join_room(&self, room_id: &str, peer_id: &str, role: &PeerRole) -> Result<(), CoreError> {
        let id = room_id.to_string();
        let pid = peer_id.to_string();

        if let Some(mut room) = self.rooms.get_mut(&id) {
            match role {
                PeerRole::Host => {
                    if room.host.is_some() {
                        return Err(CoreError::RoomFull);
                    }
                    room.host = Some(pid);
                }
                PeerRole::Remote => match room.room_type {
                    RoomType::P2P => {
                        if room.remote.is_some() {
                            return Err(CoreError::RoomFull);
                        }
                        room.remote = Some(pid);
                    }
                    RoomType::DeviceStream => {
                        room.consumers.push(Consumer {
                            peer_id: pid,
                            connected_since: chrono::Utc::now().to_rfc3339(),
                        });
                    }
                },
                PeerRole::Consumer => {
                    room.consumers.push(Consumer {
                        peer_id: pid,
                        connected_since: chrono::Utc::now().to_rfc3339(),
                    });
                }
            }
            tracing::info!("Peer {} joined room {} as {:?}", peer_id, room_id, role);
        } else {
            let room_type = match role {
                PeerRole::Host => RoomType::P2P,
                PeerRole::Remote | PeerRole::Consumer => RoomType::DeviceStream,
            };
            let (h, r, consumers) = match role {
                PeerRole::Host => (Some(pid), None, vec![]),
                PeerRole::Remote => (None, Some(pid), vec![]),
                PeerRole::Consumer => (
                    None,
                    None,
                    vec![Consumer {
                        peer_id: pid.clone(),
                        connected_since: chrono::Utc::now().to_rfc3339(),
                    }],
                ),
            };
            self.rooms.insert(
                id.clone(),
                Room {
                    id,
                    room_type,
                    device_id: None,
                    stream_id: None,
                    host: h,
                    remote: r,
                    consumers,
                    created_at: Instant::now(),
                },
            );
            tracing::info!("Room {} created by {:?} {}", room_id, role, peer_id);
            audit::log_event(AuditEvent::RoomCreate {
                room_id: room_id.to_string(),
            });
        }

        Ok(())
    }

    /// Leave a room. Removes the peer from host, remote, or consumers.
    /// Returns true if the room was removed (became empty).
    pub fn leave_room(&self, room_id: &str, peer_id: &str) -> bool {
        let id = room_id.to_string();
        if let Some(mut room) = self.rooms.get_mut(&id) {
            if room.host.as_deref() == Some(peer_id) {
                room.host = None;
            }
            if room.remote.as_deref() == Some(peer_id) {
                room.remote = None;
            }
            room.consumers.retain(|c| c.peer_id != peer_id);
            tracing::info!("Peer {} left room {}", peer_id, room_id);
        }
        // Retain rooms that still have any peers
        self.rooms
            .retain(|_, r| r.host.is_some() || r.remote.is_some() || !r.consumers.is_empty());
        let room_gone = !self.rooms.contains_key(&id);
        if room_gone {
            tracing::info!("Room {} destroyed (last peer left)", room_id);
            audit::log_event(AuditEvent::RoomDestroy {
                room_id: room_id.to_string(),
            });
        }
        room_gone
    }

    #[allow(clippy::unnecessary_lazy_evaluations)]
    pub fn get_other_peer(&self, room_id: &str, _peer_id: &str) -> Option<String> {
        // ponytail: simple single-room pair relay; extend for multi-remote later
        // For now: if sender is host, return remote; if sender is remote, return host
        self.rooms.get(room_id).and_then(|_room| {
            // We don't know exactly which is the sender without ws state,
            // so in practice this is called from signaling with ws peer context.
            // Return whichever peer is not the sender.
            None // stub — relay routing done in signaling handler directly
        })
    }

    pub fn active_rooms(&self) -> usize {
        self.rooms.len()
    }

    /// Get a room snapshot by ID.
    pub fn get_room(&self, room_id: &str) -> Option<RoomSnapshot> {
        self.rooms.get(room_id).map(|r| RoomSnapshot {
            id: r.id.clone(),
            room_type: r.room_type.clone(),
            device_id: r.device_id.clone(),
            stream_id: r.stream_id.clone(),
            host: r.host.clone(),
            remote: r.remote.clone(),
            consumers: r.consumers.clone(),
            created_at: r.created_at,
        })
    }

    /// Remove a room by ID. Returns true if a room was removed.
    pub fn remove_room(&self, room_id: &str) -> bool {
        self.rooms.remove(room_id).is_some()
    }

    /// Check whether a room is a DeviceStream room.
    pub fn is_device_stream(&self, room_id: &str) -> bool {
        self.rooms
            .get(room_id)
            .map(|r| r.room_type == RoomType::DeviceStream)
            .unwrap_or(false)
    }

    pub fn connected_peers(&self) -> usize {
        self.rooms
            .iter()
            .map(|r| (r.host.is_some() as usize) + (r.remote.is_some() as usize) + r.consumers.len())
            .sum()
    }

    pub fn get_peer_count(&self) -> usize {
        self.connected_peers()
    }


    /// Snapshot of all active rooms.

    /// Snapshot of all active rooms.
    pub fn list_rooms(&self) -> Vec<RoomSnapshot> {
        self.rooms
            .iter()
            .map(|r| RoomSnapshot {
                id: r.id.clone(),
                room_type: r.room_type.clone(),
                device_id: r.device_id.clone(),
                stream_id: r.stream_id.clone(),
                host: r.host.clone(),
                remote: r.remote.clone(),
                consumers: r.consumers.clone(),
                created_at: r.created_at,
            })
            .collect()
    }

    /// Aggregate rooms by device_id into DeviceSnapshot list.
    /// 流列表从 host-agent StatusReport 构造（multi-stream P2）——Room.device_id/
    /// stream_id 恒 None（join_room 不设），真正的流标识与在线状态在
    /// StatusRegistry（agent 每 5s 整车聚合上报）。无报告 → streams 空。
    pub fn list_devices(&self, status: &crate::status::StatusRegistry) -> Vec<DeviceSnapshot> {
        let mut device_map: HashMap<String, DeviceSnapshot> = HashMap::new();
        for r in self.rooms.iter() {
            let device_id = r.device_id.clone().unwrap_or_else(|| r.id.clone());
            let entry = device_map
                .entry(device_id.clone())
                .or_insert_with(|| DeviceSnapshot {
                    device_id: device_id.clone(),
                    online_since: r.created_at,
                    streams: Vec::new(),
                });
            if r.created_at < entry.online_since {
                entry.online_since = r.created_at;
            }
            // 该房间的 StatusReport → 每流状态（id/connected）；无报告/无流 → 空
            let report_streams: Vec<StreamSnapshot> = status
                .get(&r.id)
                .and_then(|m| match m {
                    mediaservo_common::protocol::SignalingMessage::StatusReport { streams, .. } => Some(streams.clone()),
                    _ => None,
                })
                .map(|sfs| {
                    sfs.iter()
                        .map(|sf| StreamSnapshot {
                            stream_id: sf.id.clone(),
                            consumers: r.consumers.clone(),
                            online: sf.connected,
                        })
                        .collect()
                })
                .unwrap_or_default();
            if !report_streams.is_empty() {
                entry.streams = report_streams;
            }
        }
        device_map.into_values().collect()
    }

    /// Clear all consumers from a room and return their peer_ids.
    /// Removes the room if it becomes empty after clearing.
    pub fn disconnect_consumers(&self, room_id: &str) -> Vec<String> {
        let id = room_id.to_string();
        let mut peer_ids = Vec::new();
        if let Some(mut room) = self.rooms.get_mut(&id) {
            peer_ids = room.consumers.iter().map(|c| c.peer_id.clone()).collect();
            room.consumers.clear();
        }
        // Clean up if room is now empty
        self.rooms
            .retain(|_, r| r.host.is_some() || r.remote.is_some() || !r.consumers.is_empty());
        peer_ids
    }

    /// Find and remove a room by device_id+stream_id. Returns the old room_id if found.
    pub fn replace_device_room(&self, device_id: &str, stream_id: &str) -> Option<String> {
        let old_room_id = self
            .rooms
            .iter()
            .find(|r| {
                r.device_id.as_deref() == Some(device_id)
                    && r.stream_id.as_deref() == Some(stream_id)
            })
            .map(|r| r.id.clone());
        if let Some(ref id) = old_room_id {
            self.rooms.remove(id);
        }
        old_room_id
    }

    /// Clean rooms whose last peer left more than `timeout_secs` ago.
    pub fn cleanup_stale(&self, _timeout_secs: u64) {
        // ponytail: lazy cleanup via leave_room retention; add timer-based GC if stale rooms build up
        // self.rooms.retain(|_, r| r.created_at.elapsed().as_secs() < timeout_secs || r.host.is_some() || r.remote.is_some());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- existing tests (updated for new Room fields) ----

    #[test]
    fn join_room_creates_new_room() {
        let mgr = RoomManager::new();
        let result = mgr.join_room("room-1", "peer-a", &PeerRole::Host);
        assert!(result.is_ok());
        assert_eq!(mgr.active_rooms(), 1);
        assert_eq!(mgr.get_peer_count(), 1);
    }

    #[test]
    fn two_peers_join_same_room() {
        let mgr = RoomManager::new();
        mgr.join_room("room-1", "host-1", &PeerRole::Host).unwrap();
        mgr.join_room("room-1", "remote-1", &PeerRole::Remote).unwrap();
        assert_eq!(mgr.active_rooms(), 1);
        assert_eq!(mgr.get_peer_count(), 2);
    }

    #[test]
    fn join_full_host_slot_errors() {
        let mgr = RoomManager::new();
        mgr.join_room("room-1", "host-1", &PeerRole::Host).unwrap();
        let result = mgr.join_room("room-1", "host-2", &PeerRole::Host);
        assert!(result.is_err());
    }

    #[test]
    fn join_full_remote_slot_errors() {
        // P2P room: Host+Remote fills the single remote slot
        let mgr = RoomManager::new();
        mgr.join_room("room-1", "host-1", &PeerRole::Host).unwrap();
        mgr.join_room("room-1", "remote-1", &PeerRole::Remote).unwrap();
        let result = mgr.join_room("room-1", "remote-2", &PeerRole::Remote);
        assert!(result.is_err());
    }

    #[test]
    fn leave_removes_peer() {
        let mgr = RoomManager::new();
        mgr.join_room("room-1", "host-1", &PeerRole::Host).unwrap();
        mgr.join_room("room-1", "remote-1", &PeerRole::Remote).unwrap();
        assert_eq!(mgr.get_peer_count(), 2);

        mgr.leave_room("room-1", "host-1");
        assert_eq!(mgr.get_peer_count(), 1);
    }

    #[test]
    fn leave_last_peer_removes_room() {
        let mgr = RoomManager::new();
        mgr.join_room("room-1", "host-1", &PeerRole::Host).unwrap();
        mgr.leave_room("room-1", "host-1");
        assert_eq!(mgr.active_rooms(), 0);
    }

    #[test]
    fn room_not_found_returns_none() {
        let mgr = RoomManager::new();
        let result = mgr.get_other_peer("nonexistent", "peer-a");
        assert!(result.is_none());
    }

    #[test]
    fn get_other_peer_stub() {
        let mgr = RoomManager::new();
        mgr.join_room("room-1", "host-1", &PeerRole::Host).unwrap();
        let result = mgr.get_other_peer("room-1", "host-1");
        assert!(result.is_none());
    }

    // ---- new tests ----

    #[test]
    fn join_device_stream_consumer() {
        let mgr = RoomManager::new();
        // Consumer creates a DeviceStream room
        mgr.join_room("stream-1", "consumer-1", &PeerRole::Consumer)
            .unwrap();
        let rooms = mgr.list_rooms();
        assert_eq!(rooms.len(), 1);
        assert_eq!(rooms[0].room_type, RoomType::DeviceStream);
        assert_eq!(rooms[0].host, None);
        assert_eq!(rooms[0].consumers.len(), 1);
        assert_eq!(rooms[0].consumers[0].peer_id, "consumer-1");
    }

    #[test]
    fn multiple_consumers() {
        let mgr = RoomManager::new();
        // First consumer creates the room
        mgr.join_room("stream-1", "consumer-1", &PeerRole::Consumer)
            .unwrap();
        // Second consumer joins (Remote in DeviceStream room)
        mgr.join_room("stream-1", "consumer-2", &PeerRole::Remote)
            .unwrap();
        // Third consumer joins
        mgr.join_room("stream-1", "consumer-3", &PeerRole::Consumer)
            .unwrap();
        assert_eq!(mgr.get_peer_count(), 3);
        let rooms = mgr.list_rooms();
        assert_eq!(rooms.len(), 1);
        assert_eq!(rooms[0].consumers.len(), 3);
    }

    #[test]
    fn consumer_leave() {
        let mgr = RoomManager::new();
        mgr.join_room("stream-1", "consumer-1", &PeerRole::Consumer)
            .unwrap();
        mgr.join_room("stream-1", "consumer-2", &PeerRole::Consumer)
            .unwrap();
        assert_eq!(mgr.get_peer_count(), 2);

        mgr.leave_room("stream-1", "consumer-1");
        assert_eq!(mgr.get_peer_count(), 1);

        let rooms = mgr.list_rooms();
        assert_eq!(rooms[0].consumers.len(), 1);
        assert_eq!(rooms[0].consumers[0].peer_id, "consumer-2");

        // Last consumer leaves → room removed
        mgr.leave_room("stream-1", "consumer-2");
        assert_eq!(mgr.active_rooms(), 0);
    }

    fn test_report(room: &str, streams: Vec<(String, bool)>) -> mediaservo_common::protocol::SignalingMessage {
        mediaservo_common::protocol::SignalingMessage::StatusReport {
            room_id: room.into(),
            topics: vec![],
            streams: streams
                .into_iter()
                .map(|(id, connected)| mediaservo_common::protocol::StreamFlowJson {
                    id,
                    bytes_sent: 0,
                    frames_encoded: 0,
                    frame_width: 0,
                    frame_height: 0,
                    connected,
                })
                .collect(),
            processes: vec![],
            signal: mediaservo_common::protocol::SignalStatusJson {
                remote_connected: true,
                remote_since_secs: Some(1),
                remote_peer_id: "p".into(),
                children: vec![],
                agent_uptime_secs: 1,
            },
            ts: 1,
            config_version: 0,
        }
    }

    #[test]
    fn list_devices_aggregates_rooms() {
        let mgr = RoomManager::new();
        let status = crate::status::StatusRegistry::default();

        // P2P room（device 分组用 r.device_id 或伪 device=room.id）
        mgr.join_room("room-1", "host-1", &PeerRole::Host).unwrap();
        mgr.join_room("room-2", "consumer-1", &PeerRole::Consumer)
            .unwrap();
        mgr.join_room("room-3", "host-2", &PeerRole::Host).unwrap();

        let devices = mgr.list_devices(&status);
        assert_eq!(devices.len(), 3); // 无报告 → 每房间一个 pseudo device，streams 空

        // 有报告 → streams 来自 StatusReport（id/connected），consumers 保留
        status.store("room-1", test_report("room-1", vec![("test-30fps".into(), true)]));
        status.store("room-3", test_report("room-3", vec![("test-15fps".into(), false)]));
        let devices = mgr.list_devices(&status);
        assert_eq!(devices.len(), 3);
        let d1 = devices.iter().find(|d| d.device_id == "room-1").unwrap();
        assert_eq!(d1.streams.len(), 1);
        assert_eq!(d1.streams[0].stream_id, "test-30fps");
        assert!(d1.streams[0].online);
        let d3 = devices.iter().find(|d| d.device_id == "room-3").unwrap();
        assert_eq!(d3.streams[0].stream_id, "test-15fps");
        assert!(!d3.streams[0].online);
    }

    #[test]
    fn list_devices_keeps_consumers_with_report_streams() {
        let mgr = RoomManager::new();
        let status = crate::status::StatusRegistry::default();
        mgr.join_room("room-1", "host-1", &PeerRole::Host).unwrap();
        mgr.join_room("room-1", "consumer-1", &PeerRole::Consumer).unwrap();
        status.store("room-1", test_report("room-1", vec![("test-30fps".into(), true)]));

        let devices = mgr.list_devices(&status);
        let d = devices.iter().find(|d| d.device_id == "room-1").unwrap();
        assert_eq!(d.streams.len(), 1);
        assert_eq!(d.streams[0].consumers.len(), 1, "consumers 保留（向后兼容）");
        assert_eq!(d.streams[0].consumers[0].peer_id, "consumer-1");
    }

    #[test]
    fn disconnect_consumers_clears_all() {
        let mgr = RoomManager::new();
        mgr.join_room("stream-1", "consumer-1", &PeerRole::Consumer)
            .unwrap();
        mgr.join_room("stream-1", "consumer-2", &PeerRole::Consumer)
            .unwrap();
        mgr.join_room("stream-1", "consumer-3", &PeerRole::Consumer)
            .unwrap();

        let removed = mgr.disconnect_consumers("stream-1");
        assert_eq!(removed.len(), 3);
        assert!(removed.contains(&"consumer-1".into()));
        assert!(removed.contains(&"consumer-2".into()));
        assert!(removed.contains(&"consumer-3".into()));

        // Room should be empty now (only consumers were there)
        assert_eq!(mgr.active_rooms(), 0);
    }

    #[test]
    fn disconnect_consumers_nonexistent_room() {
        let mgr = RoomManager::new();
        let removed = mgr.disconnect_consumers("nonexistent");
        assert!(removed.is_empty());
    }

    #[test]
    fn replace_device_room_removes_old() {
        let mgr = RoomManager::new();
        mgr.join_room("room-old", "host-1", &PeerRole::Host).unwrap();
        {
            let mut room = mgr.rooms.get_mut("room-old").unwrap();
            room.device_id = Some("device-x".into());
            room.stream_id = Some("stream-1".into());
        }

        let old_id = mgr.replace_device_room("device-x", "stream-1");
        assert_eq!(old_id, Some("room-old".into()));
        assert_eq!(mgr.active_rooms(), 0);
    }

    #[test]
    fn replace_device_room_no_match() {
        let mgr = RoomManager::new();
        let old_id = mgr.replace_device_room("device-x", "stream-1");
        assert_eq!(old_id, None);
    }

    #[test]
    fn get_peer_count_includes_consumers() {
        let mgr = RoomManager::new();
        mgr.join_room("room-1", "host-1", &PeerRole::Host).unwrap();
        mgr.join_room("room-1", "consumer-1", &PeerRole::Consumer)
            .unwrap();
        mgr.join_room("room-1", "consumer-2", &PeerRole::Consumer)
            .unwrap();
        assert_eq!(mgr.get_peer_count(), 3); // host + 2 consumers
    }

    #[test]
    fn remote_in_device_stream_room_goes_to_consumers() {
        // Remote creating a room → DeviceStream
        let mgr = RoomManager::new();
        mgr.join_room("room-1", "remote-1", &PeerRole::Remote)
            .unwrap();
        assert_eq!(mgr.get_peer_count(), 1);

        // Second Remote joins → consumers (N allowed in DeviceStream)
        let result = mgr.join_room("room-1", "remote-2", &PeerRole::Remote);
        assert!(result.is_ok());
        assert_eq!(mgr.get_peer_count(), 2);
    }
}
