//! Vehicle status report registry (Task E3) -- stores the latest StatusReport per room.
//!
//! Consumers: phase-H admin API (currently storage only; no other consumption path).
//! Semantics: overwrite-style (only the most recent report per room is kept); cleaned up when a room is removed (empty room),
//! so stale data never lingers.

use std::collections::HashMap;
use std::sync::Mutex;

use mediaservo_common::protocol::SignalingMessage;

/// Latest status report per room (room_id -> most recent StatusReport message).
#[derive(Default)]
pub struct StatusRegistry {
    latest: Mutex<HashMap<String, SignalingMessage>>,
}

impl StatusRegistry {
    /// Store the latest report (overwrites the old value for the same room).
    pub fn store(&self, room_id: &str, msg: SignalingMessage) {
        self.latest.lock().unwrap_or_else(|e| e.into_inner()).insert(room_id.to_string(), msg);
    }

    /// Most recent report (read by the phase-H admin API).
    pub fn get(&self, room_id: &str) -> Option<SignalingMessage> {
        self.latest.lock().unwrap_or_else(|e| e.into_inner()).get(room_id).cloned()
    }

    /// Clean up when a room is removed (empty room = no state left to report).
    pub fn remove(&self, room_id: &str) {
        self.latest.lock().unwrap_or_else(|e| e.into_inner()).remove(room_id);
    }

    /// Full snapshot (room_id -> latest report) -- data source for the H3 multi-vehicle monitoring view.
    pub fn list(&self) -> Vec<(String, SignalingMessage)> {
        self.latest
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mediaservo_common::protocol::SignalStatusJson;

    fn report(room: &str, ts: u64) -> SignalingMessage {
        SignalingMessage::StatusReport {
            room_id: room.into(),
            topics: vec![],
            streams: vec![],
            processes: vec![],
            signal: SignalStatusJson {
                remote_connected: true,
                remote_since_secs: None,
                remote_peer_id: "p".into(),
                children: vec![],
                agent_uptime_secs: 1,
            },
            ts,
            config_version: 0,
        }
    }

    #[test]
    fn store_get_roundtrip() {
        let reg = StatusRegistry::default();
        assert!(reg.get("room-a").is_none());
        reg.store("room-a", report("room-a", 100));
        match reg.get("room-a") {
            Some(SignalingMessage::StatusReport { ts, .. }) => assert_eq!(ts, 100),
            other => panic!("expected StatusReport, got {other:?}"),
        }
    }

    #[test]
    fn store_overwrites_latest_per_room() {
        let reg = StatusRegistry::default();
        reg.store("room-a", report("room-a", 100));
        reg.store("room-a", report("room-a", 200));
        reg.store("room-b", report("room-b", 300));
        match reg.get("room-a") {
            Some(SignalingMessage::StatusReport { ts, .. }) => {
                assert_eq!(ts, 200, "same room should overwrite the old value")
            }
            other => panic!("expected StatusReport, got {other:?}"),
        }
        match reg.get("room-b") {
            Some(SignalingMessage::StatusReport { ts, .. }) => {
                assert_eq!(ts, 300, "rooms should be isolated")
            }
            other => panic!("expected StatusReport, got {other:?}"),
        }
    }

    #[test]
    fn remove_clears_room() {
        let reg = StatusRegistry::default();
        reg.store("room-a", report("room-a", 100));
        reg.remove("room-a");
        assert!(reg.get("room-a").is_none());
    }

    #[test]
    fn list_returns_all_rooms() {
        let reg = StatusRegistry::default();
        reg.store("room-a", report("room-a", 100));
        reg.store("room-b", report("room-b", 200));
        let all = reg.list();
        assert_eq!(all.len(), 2, "should return all rooms");
        assert!(all.iter().any(|(k, _)| k == "room-a"));
        assert!(all.iter().any(|(k, _)| k == "room-b"));
    }
}
