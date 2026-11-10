//! Per-connection delivery registry (R13).
//!
//! Floor messages are private: they go to one `conn_id`, never to the room
//! broadcast. Eligibility is `is_account && negotiated >= PROTOCOL_MIN_FLOOR`.

use dashmap::DashMap;
use mediaservo_common::protocol::{SignalingMessage, PROTOCOL_MIN_FLOOR};
use tokio::sync::mpsc;

/// One registered connection. `tx` carries serialized signaling frames.
#[derive(Debug, Clone)]
pub struct ConnEntry {
    pub conn_id: String,
    pub room_id: String,
    pub peer_id: String,
    pub negotiated: u32,
    pub is_account: bool,
    pub tx: mpsc::Sender<String>,
}

impl ConnEntry {
    /// Floor delivery requires dialect 4 and an account session (R5/R13).
    pub fn floor_eligible(&self) -> bool {
        self.is_account && self.negotiated >= PROTOCOL_MIN_FLOOR
    }
}

/// Conn-id keyed registry. A `peer_id` may be shared by several connections.
#[derive(Default)]
pub struct ConnRegistry {
    inner: DashMap<String, ConnEntry>,
}

impl ConnRegistry {
    pub fn register(&self, entry: ConnEntry) -> Option<ConnEntry> {
        self.inner.insert(entry.conn_id.clone(), entry)
    }

    pub fn unregister(&self, conn_id: &str) -> Option<ConnEntry> {
        self.inner.remove(conn_id).map(|(_, v)| v)
    }

    /// Deliver one floor message to a specific connection. Returns false when
    /// the conn is absent, ineligible, or its channel is full/closed.
    pub fn deliver(&self, conn_id: &str, msg: &SignalingMessage) -> bool {
        let Some(entry) = self.inner.get(conn_id) else {
            return false;
        };
        if !entry.floor_eligible() {
            return false;
        }
        let Ok(text) = serde_json::to_string(msg) else {
            return false;
        };
        entry.tx.try_send(text).is_ok()
    }

    /// Public floor state (grants/mode/generation) to every eligible conn in
    /// the room. Returns the number of successful deliveries.
    pub fn broadcast_room(&self, room_id: &str, msg: &SignalingMessage) -> usize {
        let Ok(text) = serde_json::to_string(msg) else {
            return 0;
        };
        let mut sent = 0;
        for entry in self.inner.iter() {
            if entry.room_id == room_id
                && entry.floor_eligible()
                && entry.tx.try_send(text.clone()).is_ok()
            {
                sent += 1;
            }
        }
        sent
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Number of registered floor connections for one room (idle-TTL eviction input).
    pub fn room_conn_count(&self, room_id: &str) -> usize {
        self.inner.iter().filter(|e| e.room_id == room_id).count()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mediaservo_common::protocol::SignalingMessage;

    fn conn(
        id: &str,
        room: &str,
        peer: &str,
        negotiated: u32,
        is_account: bool,
    ) -> (ConnEntry, mpsc::Receiver<String>) {
        let (tx, rx) = mpsc::channel(4);
        (
            ConnEntry {
                conn_id: id.to_string(),
                room_id: room.to_string(),
                peer_id: peer.to_string(),
                negotiated,
                is_account,
                tx,
            },
            rx,
        )
    }

    fn idle(room: &str) -> SignalingMessage {
        SignalingMessage::FloorIdle { room_id: room.into(), reason: None, generation: 1 }
    }

    #[test]
    fn only_v4_account_conns_receive_floor_messages() {
        let reg = ConnRegistry::default();
        let (v4, mut rx_v4) = conn("c1", "r", "p1", 4, true);
        let (v3, mut rx_v3) = conn("c2", "r", "p1", 3, true);
        let (device, mut rx_device) = conn("c3", "r", "p1", 4, false);
        reg.register(v4);
        reg.register(v3);
        reg.register(device);

        assert!(reg.deliver("c1", &idle("r")));
        assert!(!reg.deliver("c2", &idle("r")));
        assert!(!reg.deliver("c3", &idle("r")));
        assert!(rx_v4.try_recv().is_ok());
        assert!(rx_v3.try_recv().is_err());
        assert!(rx_device.try_recv().is_err());
    }

    #[test]
    fn broadcast_targets_only_eligible_conns_in_the_room() {
        let reg = ConnRegistry::default();
        let (a, mut rx_a) = conn("a", "room-a", "p1", 4, true);
        let (b, mut rx_b) = conn("b", "room-a", "p2", 3, true);
        let (c, mut rx_c) = conn("c", "room-b", "p3", 4, true);
        reg.register(a);
        reg.register(b);
        reg.register(c);

        assert_eq!(reg.broadcast_room("room-a", &idle("room-a")), 1);
        assert!(rx_a.try_recv().is_ok());
        assert!(rx_b.try_recv().is_err());
        assert!(rx_c.try_recv().is_err());
    }

    #[test]
    fn unregister_removes_the_connection() {
        let reg = ConnRegistry::default();
        let (a, _rx) = conn("a", "room-a", "p1", 4, true);
        reg.register(a);
        assert_eq!(reg.len(), 1);
        assert!(reg.unregister("a").is_some());
        assert!(reg.is_empty());
        assert!(!reg.deliver("a", &idle("room-a")));
    }
}
