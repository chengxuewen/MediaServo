//! Per-room floor actor — the single writer for one room's `FloorState`.
//!
//! The room owns its state; commands are serialized by the caller (an mpsc
//! mailbox in the signaling integration). This slice keeps the reducer
//! synchronous and testable; media reconciliation lands with the SFU wiring.

use mediaservo_common::floor::{FloorEvent, FloorLimits, FloorMode, FloorSignal, FloorState};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

/// One room's authoritative floor state.
pub struct FloorRoom {
    room_id: String,
    state: FloorState,
    limits: FloorLimits,
    /// Wall-clock epoch of the live grant window (set when grants become
    /// non-empty, cleared when empty) — the max_hold clock (§3.2).
    granted_at: Option<Instant>,
    /// §3.2 media watchdog: peers whose media the reconcile confirmed live
    /// (resume success). Cleared when the grant window empties.
    media_confirmed: HashSet<String>,
    /// Granted peers whose media was never confirmed, with when the clock armed
    /// — past `media_grace_secs` the supervisor fires `MediaDown{peer}`.
    media_missing: HashMap<String, Instant>,
}

impl FloorRoom {
    pub fn new(room_id: impl Into<String>, mode: FloorMode, limits: FloorLimits) -> Self {
        Self {
            room_id: room_id.into(),
            state: FloorState::initial_with_generation(mode, 1),
            limits,
            granted_at: None,
            media_confirmed: HashSet::new(),
            media_missing: HashMap::new(),
        }
    }

    pub fn room_id(&self) -> &str {
        &self.room_id
    }

    pub fn state(&self) -> &FloorState {
        &self.state
    }

    pub fn limits(&self) -> &FloorLimits {
        &self.limits
    }

    /// §3.2 max_hold clock input: when the current grant window opened.
    pub fn granted_at(&self) -> Option<Instant> {
        self.granted_at
    }

    /// §3.2 media watchdog: the reconcile confirmed live media for the peer
    /// (resume success) — clears the missing-media clock.
    pub fn media_confirm(&mut self, peer: &str) {
        self.media_missing.remove(peer);
        self.media_confirmed.insert(peer.to_string());
    }

    /// Granted peers missing confirmed media for >= `media_grace_secs` — the
    /// supervisor fires `MediaDown{peer}` for each (release + promotion).
    pub fn media_grace_due(&self, now: Instant) -> Vec<String> {
        self.media_missing
            .iter()
            .filter(|(_, t)| {
                now.duration_since(**t) >= Duration::from_secs(self.limits.media_grace_secs)
            })
            .map(|(peer, _)| peer.clone())
            .collect()
    }

    /// Apply one event and return the emitted signals. The state is replaced
    /// only by the reducer's result; no I/O happens here.
    pub fn apply(&mut self, event: FloorEvent) -> Vec<FloorSignal> {
        let (next, out) = self.state.apply(&event, &self.limits);
        self.state = next;
        // the max_hold clock runs while at least one grant is live
        if self.state.grants().is_empty() {
            self.granted_at = None;
            // an empty window resets the media bookkeeping wholesale
            self.media_confirmed.clear();
            self.media_missing.clear();
        } else {
            if self.granted_at.is_none() {
                self.granted_at = Some(Instant::now());
            }
            // media-grace bookkeeping: granted peers without confirmed media
            // arm the clock; confirmed or released peers leave it
            for g in self.state.grants() {
                if !self.media_confirmed.contains(g) {
                    self.media_missing.entry(g.clone()).or_insert_with(Instant::now);
                }
            }
            self.media_missing.retain(|peer, _| {
                self.state.is_granted(peer) && !self.media_confirmed.contains(peer)
            });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mediaservo_common::floor::{DenyReason, FloorSignal};

    fn room(mode: FloorMode) -> FloorRoom {
        FloorRoom::new("audio-p0", mode, FloorLimits::default())
    }

    fn req(peer: &str) -> FloorEvent {
        FloorEvent::Request { peer: peer.to_string(), priority: 0, preempt: false }
    }

    #[test]
    fn first_request_is_granted_and_second_is_queued() {
        let mut r = room(FloorMode::Exclusive);
        let out = r.apply(req("a"));
        assert!(matches!(out[0], FloorSignal::FloorGranted { .. }));
        let out = r.apply(req("b"));
        assert!(matches!(out[0], FloorSignal::FloorQueued { position: 0, .. }));
        assert!(r.state().is_granted(&"a".to_string()));
        assert_eq!(r.state().queue().len(), 1);
    }

    #[test]
    fn release_promotes_the_queue_head() {
        let mut r = room(FloorMode::Exclusive);
        let _ = r.apply(req("a"));
        let _ = r.apply(req("b"));
        let out = r.apply(FloorEvent::Release { peer: "a".to_string() });
        assert!(out.iter().any(|m| matches!(m, FloorSignal::FloorGranted { grants, .. } if grants == &vec!["b".to_string()])));
        assert!(r.state().is_granted(&"b".to_string()));
    }

    #[test]
    fn muted_request_is_denied_visibly() {
        let mut r = room(FloorMode::Exclusive);
        let _ = r.apply(FloorEvent::MuteSet {
            actor: "a".to_string(),
            peer: "b".to_string(),
            on: true,
        });
        let out = r.apply(req("b"));
        assert!(out.iter().any(|m| matches!(
            m,
            FloorSignal::FloorDenied { reason: DenyReason::Unauthorized, .. }
        )));
    }

    /// §3.2 media watchdog bookkeeping: a grant arms the missing-media clock;
    /// confirm clears it; release drops it; due only past the grace boundary.
    #[test]
    fn media_grace_bookkeeping_arms_confirms_and_expires() {
        let mut r = room(FloorMode::Exclusive);
        let _ = r.apply(req("a"));
        let t0 = Instant::now();
        assert!(r.media_grace_due(t0).is_empty(), "fresh grant: grace not elapsed");
        assert_eq!(
            r.media_grace_due(t0 + Duration::from_secs(3)),
            vec!["a".to_string()],
            "unconfirmed grantee is due past the grace"
        );

        r.media_confirm("a");
        assert!(
            r.media_grace_due(t0 + Duration::from_secs(3)).is_empty(),
            "confirmed media clears the clock"
        );

        let _ = r.apply(FloorEvent::Release { peer: "a".to_string() });
        assert!(
            r.media_grace_due(t0 + Duration::from_secs(3)).is_empty(),
            "released peer leaves the ledger"
        );
    }
}
