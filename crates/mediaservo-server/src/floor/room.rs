//! Per-room floor actor — the single writer for one room's `FloorState`.
//!
//! The room owns its state; commands are serialized by the caller (an mpsc
//! mailbox in the signaling integration). This slice keeps the reducer
//! synchronous and testable; media reconciliation lands with the SFU wiring.

use mediaservo_common::floor::{FloorEvent, FloorLimits, FloorMode, FloorSignal, FloorState};

/// One room's authoritative floor state.
pub struct FloorRoom {
    room_id: String,
    state: FloorState,
    limits: FloorLimits,
}

impl FloorRoom {
    pub fn new(room_id: impl Into<String>, mode: FloorMode, limits: FloorLimits) -> Self {
        Self {
            room_id: room_id.into(),
            state: FloorState::initial_with_generation(mode, 1),
            limits,
        }
    }

    pub fn room_id(&self) -> &str {
        &self.room_id
    }

    pub fn state(&self) -> &FloorState {
        &self.state
    }

    /// Apply one event and return the emitted signals. The state is replaced
    /// only by the reducer's result; no I/O happens here.
    pub fn apply(&mut self, event: FloorEvent) -> Vec<FloorSignal> {
        let (next, out) = self.state.apply(&event, &self.limits);
        self.state = next;
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
}
