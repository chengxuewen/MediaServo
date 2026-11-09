//! FloorState property tests — deterministic randomized sequences.
//!
//! Covers the P1 gate properties: capacity, idempotence, generation monotonicity,
//! queue fairness (priority desc + FIFO ties) and "no silent denial".

use mediaservo_common::floor::{
    DenyReason, FloorEvent, FloorLimits, FloorMode, FloorSignal, FloorState,
};

fn peers() -> Vec<String> {
    ["a", "b", "c", "d"].iter().map(|s| s.to_string()).collect()
}

fn cap(mode: FloorMode) -> usize {
    match mode {
        FloorMode::Exclusive => 1,
        FloorMode::Hybrid => 2,
        FloorMode::Open => usize::MAX,
        _ => usize::MAX,
    }
}

fn qnames(s: &FloorState) -> Vec<&str> {
    s.queue().iter().map(|p| p.peer.as_str()).collect()
}

fn assert_invariants(s: &FloorState) {
    let c = cap(s.mode());
    if c != usize::MAX {
        assert!(s.grants().len() <= c, "grants exceed cap: {:?}", s.grants());
    }

    let mut grants = s.grants().to_vec();
    grants.sort();
    grants.dedup();
    assert_eq!(grants.len(), s.grants().len(), "duplicate grant");

    let mut queued: Vec<String> = s.queue().iter().map(|p| p.peer.clone()).collect();
    queued.sort();
    queued.dedup();
    assert_eq!(queued.len(), s.queue().len(), "duplicate queue entry");
    for p in s.queue() {
        assert!(!s.is_granted(&p.peer), "peer both granted and queued");
    }

    let mut muted = s.muted().to_vec();
    muted.sort();
    muted.dedup();
    assert_eq!(muted.len(), s.muted().len(), "duplicate muted entry");
}

struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0
    }

    fn pick(&mut self, n: usize) -> usize {
        (self.next() >> 33) as usize % n
    }

    fn flag(&mut self) -> bool {
        self.next() & 1 == 0
    }
}

fn random_event(rng: &mut Lcg, peers: &[String]) -> FloorEvent {
    let peer = peers[rng.pick(peers.len())].clone();
    let other = peers[rng.pick(peers.len())].clone();
    match rng.pick(100) {
        0..=39 => FloorEvent::Request { peer, priority: rng.pick(4) as u8, preempt: rng.flag() },
        40..=54 => FloorEvent::Release { peer },
        55..=64 => FloorEvent::Leave { peer },
        65..=74 => FloorEvent::MediaDown { peer },
        75..=84 => FloorEvent::Timeout,
        85..=94 => {
            let mode = match rng.pick(3) {
                0 => FloorMode::Exclusive,
                1 => FloorMode::Open,
                _ => FloorMode::Hybrid,
            };
            FloorEvent::ModeChange { mode, actor: peer }
        }
        _ => FloorEvent::MuteSet { actor: other, peer, on: rng.flag() },
    }
}

#[test]
fn random_sequences_keep_invariants() {
    let peers = peers();
    let limits = FloorLimits::default();
    for mode in [FloorMode::Exclusive, FloorMode::Open, FloorMode::Hybrid] {
        for seed in 1..=20u64 {
            let mut rng = Lcg(seed);
            let mut state = FloorState::initial_with_generation(mode, 1);
            for _ in 0..300 {
                let before = state.clone();
                let event = random_event(&mut rng, &peers);
                let (next, out) = state.apply(&event, &limits);

                assert!(
                    next.generation() >= before.generation(),
                    "generation decreased: {} -> {}",
                    before.generation(),
                    next.generation()
                );
                assert_invariants(&next);

                // No silent denial: a request that neither grants nor queues must say why.
                if let FloorEvent::Request { peer, .. } = &event {
                    let was_visible =
                        before.is_granted(peer) || before.queue_position(peer).is_some();
                    let is_visible = next.is_granted(peer) || next.queue_position(peer).is_some();
                    if !was_visible && !is_visible {
                        assert!(
                            out.iter().any(|m| matches!(m, FloorSignal::FloorDenied { .. })),
                            "request vanished silently: {event:?}"
                        );
                    }
                }
                state = next;
            }
        }
    }
}

#[test]
fn repeated_request_is_idempotent() {
    let limits = FloorLimits::default();
    for mode in [FloorMode::Exclusive, FloorMode::Open, FloorMode::Hybrid] {
        let mut state = FloorState::initial_with_generation(mode, 1);

        let req = |peer: &str| FloorEvent::Request {
            peer: peer.to_string(),
            priority: 1,
            preempt: false,
        };

        let (granted, _) = state.apply(&req("a"), &limits);
        let (again, _) = granted.apply(&req("a"), &limits);
        assert_eq!(again, granted, "granted peer request must be idempotent");

        let (queued, _) = granted.apply(&req("b"), &limits);
        if mode == FloorMode::Exclusive {
            let (again, _) = queued.apply(&req("b"), &limits);
            assert_eq!(again, queued, "queued peer request must be idempotent");
        }

        state = queued;
        assert_invariants(&state);
    }
}

#[test]
fn queue_fairness_priority_desc_and_fifo_ties() {
    let limits = FloorLimits::default();
    let mut state = FloorState::initial_with_generation(FloorMode::Exclusive, 1);
    let req = |peer: &str, priority: u8| FloorEvent::Request {
        peer: peer.to_string(),
        priority,
        preempt: false,
    };

    for peer in ["a", "b", "c", "d"] {
        state = state.apply(&req(peer, 1), &limits).0;
    }
    assert!(state.is_granted(&"a".to_string()));
    assert_eq!(qnames(&state), vec!["b", "c", "d"], "FIFO within one tier");

    state = state.apply(&req("e", 2), &limits).0;
    assert_eq!(qnames(&state), vec!["e", "b", "c", "d"], "higher priority jumps ahead");

    state = state.apply(&FloorEvent::Release { peer: "a".to_string() }, &limits).0;
    assert!(state.is_granted(&"e".to_string()), "queue head promoted");
    assert_eq!(qnames(&state), vec!["b", "c", "d"]);
}

#[test]
fn denied_requests_are_visible() {
    let limits = FloorLimits::default();
    let mut state = FloorState::initial_with_generation(FloorMode::Exclusive, 1);
    let req = |peer: &str, priority: u8, preempt: bool| FloorEvent::Request {
        peer: peer.to_string(),
        priority,
        preempt,
    };

    let (s, out) = state.apply(&req("a", 2, false), &limits);
    assert!(out.iter().any(|m| matches!(m, FloorSignal::FloorGranted { .. })));
    state = s;

    // Equal priority cannot pre-empt: the denial is explicit.
    let (_, out) = state.apply(&req("b", 2, true), &limits);
    assert!(out.iter().any(|m| matches!(
        m,
        FloorSignal::FloorDenied { reason: DenyReason::PreemptPriority, .. }
    )));

    // A muted peer's request is refused visibly, not silently.
    let (s, _) = state.apply(
        &FloorEvent::MuteSet { actor: "a".to_string(), peer: "b".to_string(), on: true },
        &limits,
    );
    let (_, out) = s.apply(&req("b", 1, false), &limits);
    assert!(
        out.iter().any(|m| matches!(
            m,
            FloorSignal::FloorDenied { reason: DenyReason::Unauthorized, .. }
        ))
    );
}
