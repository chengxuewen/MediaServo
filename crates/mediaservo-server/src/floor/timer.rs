//! Floor timers (§3.2): a per-server supervisor scans the designated floor
//! rooms once a second and drives the wall-clock events the pure core expects
//! the server domain to decide — max_hold timeout and idle-TTL eviction.
//!
//! Media-grace arming needs the media plane (resume-success signals) and lands
//! with the O23 reconcile rewrite — deliberately not here. Rate limiting is a
//! command-entry guard (signaling.rs), not a timer.

use crate::signaling::{SignalingServer, apply_floor_event};
use mediaservo_common::floor::FloorEvent;
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

pub const TICK_SECS: u64 = 1;
/// §3.2 floor_idle_ttl: how long an empty room keeps its floor state before the
/// supervisor evicts it (config knob rides with the config-precedence pass).
pub const IDLE_TTL_SECS: u64 = 300;

/// Pure deadline check (unit-tested): a grant window older than max_hold is due.
fn hold_timed_out(granted_at: Option<Instant>, max_hold_secs: u64, now: Instant) -> bool {
    match granted_at {
        Some(t) => now.duration_since(t) >= Duration::from_secs(max_hold_secs),
        None => false,
    }
}

/// Pure eviction check: idle longer than the TTL.
fn idle_past_ttl(empty_since: Option<Instant>, ttl_secs: u64, now: Instant) -> bool {
    match empty_since {
        Some(t) => now.duration_since(t) >= Duration::from_secs(ttl_secs),
        None => false,
    }
}

/// A room carries nothing worth preserving: no grants, no queue, no muted set,
/// and no registered floor connections.
fn state_preservable(
    room: &crate::floor::FloorRoom,
    server: &SignalingServer,
    room_id: &str,
) -> bool {
    let st = room.state();
    st.grants().is_empty()
        && st.queue().is_empty()
        && st.muted().is_empty()
        && server.floor_registry.room_conn_count(room_id) == 0
}

/// One supervisor pass. The DashMap iteration guard is dropped before any await
/// (the documented sfu.rs:851-869 cross-guard deadlock lesson).
async fn tick(server: &SignalingServer, empty_since: &mut HashMap<String, Instant>) {
    let now = Instant::now();

    // Pass 1 (sync): snapshot Arc handles out of the map.
    let rooms: Vec<(String, std::sync::Arc<tokio::sync::Mutex<crate::floor::FloorRoom>>)> = server
        .floor_rooms
        .iter()
        .map(|e| (e.key().clone(), std::sync::Arc::clone(e.value())))
        .collect();

    // Pass 2: classify rooms (locks are per-room; no map guard held).
    let mut due_timeout: Vec<String> = Vec::new();
    let mut empty_now: HashSet<String> = HashSet::new();
    for (room_id, room) in &rooms {
        let g = room.lock().await;
        if hold_timed_out(g.granted_at(), g.limits().max_hold_secs, now) {
            due_timeout.push(room_id.clone());
        }
        if state_preservable(&g, server, room_id) {
            empty_now.insert(room_id.clone());
        }
    }

    // Pass 3: fire due timeouts through the standard pipeline (audit + delivery).
    for room_id in &due_timeout {
        apply_floor_event(server, room_id, "timer", FloorEvent::Timeout).await;
    }

    // Pass 4: idle-TTL bookkeeping. Rooms gone from the map or no longer empty
    // leave the idle ledger; rooms empty past the TTL are evicted.
    empty_since.retain(|room_id, _| {
        server.floor_rooms.contains_key(room_id) && empty_now.contains(room_id)
    });
    for room_id in rooms.iter().map(|(id, _)| id) {
        if empty_now.contains(room_id) {
            empty_since.entry(room_id.clone()).or_insert(now);
        }
    }
    let evict: Vec<String> = empty_since
        .iter()
        .filter(|(_, t)| idle_past_ttl(Some(**t), IDLE_TTL_SECS, now))
        .map(|(room_id, _)| room_id.clone())
        .collect();
    for room_id in evict {
        // re-verify emptiness under the room lock before removing; the DashMap
        // entry guard is released (Arc cloned out) before the await
        let room = match server.floor_rooms.get(&room_id) {
            Some(e) => std::sync::Arc::clone(e.value()),
            None => {
                empty_since.remove(&room_id);
                continue;
            }
        };
        let g = room.lock().await;
        if state_preservable(&g, server, &room_id) {
            drop(g);
            server.floor_rooms.remove(&room_id);
            empty_since.remove(&room_id);
            tracing::info!(room_id, "floor room evicted (idle TTL)");
        }
    }
}

/// Spawn the supervisor. Called once from server startup; it stops with the
/// runtime (floor state is in-memory and rebuilds lazily, §3.2).
pub fn spawn(server: SignalingServer) {
    tokio::spawn(async move {
        let mut empty_since: HashMap<String, Instant> = HashMap::new();
        loop {
            tokio::time::sleep(Duration::from_secs(TICK_SECS)).await;
            tick(&server, &mut empty_since).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hold_timeout_only_when_window_exceeds_max_hold() {
        let now = Instant::now();
        assert!(!hold_timed_out(None, 45, now), "no grant -> never timed out");
        assert!(!hold_timed_out(Some(now), 45, now), "fresh grant");
        let old = now - Duration::from_secs(46);
        assert!(hold_timed_out(Some(old), 45, now), "46 s hold with 45 s cap");
        let edge = now - Duration::from_secs(45);
        assert!(hold_timed_out(Some(edge), 45, now), "exactly at the cap is due");
    }

    #[test]
    fn idle_eviction_needs_the_full_ttl() {
        let now = Instant::now();
        assert!(!idle_past_ttl(None, 300, now));
        assert!(!idle_past_ttl(Some(now), 300, now));
        assert!(idle_past_ttl(Some(now - Duration::from_secs(301)), 300, now));
    }
}
