//! FloorState rules 1-9 (modules/01) under exhaustive test — plan-1 Task 4.
//!
//! Rule map (each test names its rule):
//! R1 exclusive cap-1 grant + queue           R2 release → auto-promote head
//! R3 priority ordering (desc, FIFO ties)     R4 duplicate Request idempotent
//! R5 preempt strictly-higher swap / deny     R6 MediaDown = immediate release
//! R7 timeout → idle(reason)                  R8 no-ops leave generation UNCHANGED
//! R9 mode switches (X→O drop, O→X keep-first, H cap-2) + mute + queue purge (D16)

use mediaservo_common::floor::{
    DenyReason, FloorEvent, FloorLimits, FloorMode, FloorSignal, FloorState,
};

type PeerId = String;

fn pid(s: &str) -> PeerId {
    s.to_string()
}

fn state(mode: FloorMode) -> FloorState {
    FloorState::initial_with_generation(mode, 100)
}

fn emitted(out: &[FloorSignal]) -> Vec<String> {
    out.iter()
        .map(|m| match m {
            FloorSignal::FloorGranted { grants, .. } => {
                let names: Vec<String> = grants.iter().map(|g| g.to_string()).collect();
                format!("granted({names:?})")
            }
            FloorSignal::FloorQueued { position, .. } => format!("queued({position})"),
            FloorSignal::FloorDenied { reason, .. } => format!("denied({reason:?})"),
            FloorSignal::FloorTaken { by, .. } => format!("taken({by})"),
            FloorSignal::FloorIdle { reason, .. } => format!("idle({reason:?})"),
        })
        .collect()
}

fn generation(s: &FloorState) -> u64 {
    s.generation()
}

// ── R1: exclusive cap 1 ─────────────────────────────────────────────────────

#[test]
fn r1_exclusive_first_request_grants_second_queues_at_zero() {
    let s = state(FloorMode::Exclusive);
    let (s, out) = s.apply(
        &FloorEvent::Request { peer: pid("a"), priority: 0, preempt: false },
        &FloorLimits::default(),
    );
    assert_eq!(emitted(&out), vec!["granted([\"a\"])"]);
    let (s, out) = s.apply(
        &FloorEvent::Request { peer: pid("b"), priority: 0, preempt: false },
        &FloorLimits::default(),
    );
    assert_eq!(emitted(&out), vec!["queued(0)"]);
    assert_eq!(s.queue().len(), 1);
}

// ── R3: priority ordering ───────────────────────────────────────────────────

#[test]
fn r3_higher_priority_jumps_ahead_fifo_within_tier() {
    let s = state(FloorMode::Exclusive);
    // a (priority 1) granted (capacity 1); b (priority 2) queues at 0;
    // c (priority 2, same tier) FIFOs in behind b at 1 — nobody shifts
    let (s, out_a) = s.apply(
        &FloorEvent::Request { peer: pid("a"), priority: 1, preempt: false },
        &FloorLimits::default(),
    );
    assert_eq!(emitted(&out_a), vec!["granted([\"a\"])"]);
    let (s, out_b) = s.apply(
        &FloorEvent::Request { peer: pid("b"), priority: 2, preempt: false },
        &FloorLimits::default(),
    );
    assert_eq!(emitted(&out_b), vec!["queued(0)"]);
    let (s, out_c) = s.apply(
        &FloorEvent::Request { peer: pid("c"), priority: 2, preempt: false },
        &FloorLimits::default(),
    );
    assert_eq!(emitted(&out_c), vec!["queued(1)"], "FIFO within tier, no shifts");

    // priority jump: d (priority 3) takes slot 0; b (was 0)→1, c (was 1)→2
    let (_, out_d) = s.apply(
        &FloorEvent::Request { peer: pid("d"), priority: 3, preempt: false },
        &FloorLimits::default(),
    );
    assert_eq!(emitted(&out_d), vec!["queued(0)", "queued(1)", "queued(2)"]);
}

// ── R4: duplicate Request idempotent ────────────────────────────────────────

#[test]
fn r4_duplicate_request_same_position_no_dup() {
    let s = state(FloorMode::Exclusive);
    let (s, _) = s.apply(
        &FloorEvent::Request { peer: pid("a"), priority: 0, preempt: false },
        &FloorLimits::default(),
    );
    let (s, out_b1) = s.apply(
        &FloorEvent::Request { peer: pid("b"), priority: 0, preempt: false },
        &FloorLimits::default(),
    );
    assert_eq!(emitted(&out_b1), vec!["queued(0)"]);
    let gen_before = generation(&s);
    let (s, out2) = s.apply(
        &FloorEvent::Request { peer: pid("b"), priority: 0, preempt: false },
        &FloorLimits::default(),
    );
    assert_eq!(emitted(&out2), vec!["queued(0)"], "re-queue echoes same position");
    assert_eq!(s.queue().len(), 1, "no duplicate queue entry");
    assert_eq!(generation(&s), gen_before, "idempotent re-request: no gen bump");
}

// ── R2: release → auto-promote ──────────────────────────────────────────────

#[test]
fn r2_release_promotes_head_with_granted_broadcast_only() {
    let s = state(FloorMode::Exclusive);
    let (s, _) = s
        .apply(
            &FloorEvent::Request { peer: pid("a"), priority: 0, preempt: false },
            &FloorLimits::default(),
        )
        .0
        .apply(
            &FloorEvent::Request { peer: pid("b"), priority: 0, preempt: false },
            &FloorLimits::default(),
        );
    let gen_mid = generation(&s);
    let (s, out) = s.apply(&FloorEvent::Release { peer: pid("a") }, &FloorLimits::default());
    // modules/01: promote emits FloorGranted only; FloorIdle ONLY when queue empties
    assert_eq!(emitted(&out), vec!["granted([\"b\"])"]);
    assert!(s.queue().is_empty());
    assert_eq!(s.grants(), &[pid("b")]);
    assert!(generation(&s) > gen_mid, "real transition bumps generation");
}

#[test]
fn r2_release_to_empty_emits_floor_idle() {
    let s = state(FloorMode::Exclusive);
    let (s, _) = s.apply(
        &FloorEvent::Request { peer: pid("a"), priority: 0, preempt: false },
        &FloorLimits::default(),
    );
    let (s, out) = s.apply(&FloorEvent::Release { peer: pid("a") }, &FloorLimits::default());
    assert_eq!(emitted(&out), vec!["idle(None)"]);
    assert_eq!(s.grants().len(), 0);
}

// ── R6: MediaDown immediate ─────────────────────────────────────────────────

#[test]
fn r6_media_down_releases_immediately_grace_is_server_domain() {
    let s = state(FloorMode::Exclusive);
    let (s, _) = s.apply(
        &FloorEvent::Request { peer: pid("a"), priority: 0, preempt: false },
        &FloorLimits::default(),
    );
    let (s, out) = s.apply(&FloorEvent::MediaDown { peer: pid("a") }, &FloorLimits::default());
    // modules/05 E11: grace timer lives in server; core applies MediaDown as immediate release
    assert_eq!(emitted(&out), vec!["idle(None)"]);
    assert_eq!(s.grants().len(), 0);
}

// ── R7: timeout ─────────────────────────────────────────────────────────────

#[test]
fn r7_timeout_drops_grants_with_reason() {
    let s = state(FloorMode::Hybrid);
    let (s, _) = s
        .apply(
            &FloorEvent::Request { peer: pid("a"), priority: 0, preempt: false },
            &FloorLimits::default(),
        )
        .0
        .apply(
            &FloorEvent::Request { peer: pid("b"), priority: 0, preempt: false },
            &FloorLimits::default(),
        );
    let (s, out) = s.apply(&FloorEvent::Timeout, &FloorLimits::default());
    assert_eq!(emitted(&out), vec!["idle(Some(\"timeout\"))"]);
    assert_eq!(s.grants().len(), 0);
}

// ── R8: no-op generation stability ──────────────────────────────────────────

#[test]
fn r8_no_ops_keep_generation_unchanged() {
    let s = state(FloorMode::Exclusive);
    let (s, _) = s.apply(
        &FloorEvent::Request { peer: pid("a"), priority: 0, preempt: false },
        &FloorLimits::default(),
    );
    let g = generation(&s);

    let (s, out) = s.apply(&FloorEvent::Release { peer: pid("zz") }, &FloorLimits::default());
    assert!(out.is_empty());
    assert_eq!(generation(&s), g, "release by non-granted = no-op");

    let (s, out) = s.apply(
        &FloorEvent::MuteSet { actor: pid("admin"), peer: pid("zz"), on: true },
        &FloorLimits::default(),
    );
    assert!(out.is_empty());
    assert_eq!(
        generation(&s),
        g,
        "mute of a silent id is a roster change, not a floor change (S7.5 A2/A3)",
    );
    assert!(s.is_muted(&pid("zz")), "the ban must still be recorded");

    let (s, out) = s.apply(&FloorEvent::Leave { peer: pid("zz") }, &FloorLimits::default());
    assert!(out.is_empty());
    assert_eq!(generation(&s), g, "leave of absent peer = no-op");
}

// ── R5: preempt ─────────────────────────────────────────────────────────────

#[test]
fn r5_preempt_strictly_higher_swaps_and_notifies_old_holder() {
    let s = state(FloorMode::Exclusive);
    let (s, _) = s.apply(
        &FloorEvent::Request { peer: pid("low"), priority: 1, preempt: false },
        &FloorLimits::default(),
    );
    let (s, out) = s.apply(
        &FloorEvent::Request { peer: pid("high"), priority: 5, preempt: true },
        &FloorLimits::default(),
    );
    assert_eq!(emitted(&out), vec!["taken(high)", "granted([\"high\"])"]);
    assert_eq!(s.grants(), &[pid("high")]);
}

#[test]
fn r5_preempt_equal_priority_denied() {
    let s = state(FloorMode::Exclusive);
    let (s, _) = s.apply(
        &FloorEvent::Request { peer: pid("p1"), priority: 3, preempt: false },
        &FloorLimits::default(),
    );
    let g = generation(&s);
    let (s, out) = s.apply(
        &FloorEvent::Request { peer: pid("p2"), priority: 3, preempt: true },
        &FloorLimits::default(),
    );
    assert_eq!(emitted(&out), vec!["denied(PreemptPriority)"]);
    assert_eq!(s.grants(), &[pid("p1")]);
    assert_eq!(generation(&s), g, "denial = no-op, no gen bump");
}

#[test]
fn r5_ceiling_violation_denied_busy() {
    let s = state(FloorMode::Exclusive);
    let (s, _) = s
        .apply(
            &FloorEvent::Request { peer: pid("q1"), priority: 0, preempt: false },
            &FloorLimits::default(),
        )
        .0
        .apply(
            &FloorEvent::Request { peer: pid("q2"), priority: 0, preempt: false },
            &FloorLimits::default(),
        );
    let (s, _) = s.apply(
        &FloorEvent::Request { peer: pid("q3"), priority: 0, preempt: false },
        &FloorLimits { max_peers: 1, max_queue: 2, ..FloorLimits::default() },
    );
    assert_eq!(s.queue().len(), 2, "q2,q3 fill the queue");
    let g = generation(&s);
    let (s, out) = s.apply(
        &FloorEvent::Request { peer: pid("q4"), priority: 0, preempt: false },
        &FloorLimits { max_peers: 1, max_queue: 2, ..FloorLimits::default() },
    );
    assert_eq!(emitted(&out), vec!["denied(Busy)"]);
    assert_eq!(s.queue().len(), 2, "queue stays at ceiling");
    assert_eq!(generation(&s), g, "denial = no-op");
}

// ── R9: mode switches ───────────────────────────────────────────────────────

#[test]
fn r9_exclusive_to_open_drops_queue_and_grants_everyone_idle() {
    let s = state(FloorMode::Exclusive);
    let (s, _) = s
        .apply(
            &FloorEvent::Request { peer: pid("a"), priority: 0, preempt: false },
            &FloorLimits::default(),
        )
        .0
        .apply(
            &FloorEvent::Request { peer: pid("b"), priority: 0, preempt: false },
            &FloorLimits::default(),
        );
    let (s, out) = s.apply(
        &FloorEvent::ModeChange { mode: FloorMode::Open, actor: pid("admin") },
        &FloorLimits::default(),
    );
    assert_eq!(s.mode(), FloorMode::Open);
    assert!(s.queue().is_empty(), "queue dropped on open");
    assert_eq!(emitted(&out), vec!["idle(None)"]);
}

#[test]
fn r9_open_to_exclusive_keeps_first_grant_rest_queued_in_order() {
    let s = state(FloorMode::Open);
    let (s, _) = s
        .apply(
            &FloorEvent::Request { peer: pid("a"), priority: 0, preempt: false },
            &FloorLimits::default(),
        )
        .0
        .apply(
            &FloorEvent::Request { peer: pid("b"), priority: 0, preempt: false },
            &FloorLimits::default(),
        );
    // open mode: both granted (open = everyone speaks)
    assert_eq!(s.grants().len(), 2);
    let (s, out) = s.apply(
        &FloorEvent::ModeChange { mode: FloorMode::Exclusive, actor: pid("admin") },
        &FloorLimits::default(),
    );
    assert_eq!(s.grants(), &[pid("a")], "first-granted kept");
    assert_eq!(s.queue().len(), 1);
    assert_eq!(s.queue()[0].peer, pid("b"), "rest queued in prior order");
    assert_eq!(emitted(&out), vec!["queued(0)"]); // to the demoted peer
}

#[test]
fn r9_hybrid_caps_two_grants() {
    let s = state(FloorMode::Hybrid);
    let (s, _) = s
        .apply(
            &FloorEvent::Request { peer: pid("a"), priority: 0, preempt: false },
            &FloorLimits::default(),
        )
        .0
        .apply(
            &FloorEvent::Request { peer: pid("b"), priority: 0, preempt: false },
            &FloorLimits::default(),
        );
    assert_eq!(s.grants().len(), 2, "hybrid cap 2 (default limits)");
    let (_, out) = s.apply(
        &FloorEvent::Request { peer: pid("c"), priority: 0, preempt: false },
        &FloorLimits::default(),
    );
    assert_eq!(emitted(&out), vec!["queued(0)"]);
}

// -- mute (Rule 2/8 re-scoped by the S7.5 walkthrough 2026-10-20) ------------

#[test]
fn mute_revokes_the_floor_and_unmute_restores_only_the_privilege() {
    let s = state(FloorMode::Open);
    let (s, _) = s.apply(
        &FloorEvent::Request { peer: pid("a"), priority: 0, preempt: false },
        &FloorLimits::default(),
    );
    assert!(s.is_granted(&pid("a")));

    // mute takes the mic away, so every mirror is told (PIT-21 shrink path)
    let (s, out) = s.apply(
        &FloorEvent::MuteSet { actor: pid("admin"), peer: pid("a"), on: true },
        &FloorLimits::default(),
    );
    assert!(s.is_muted(&pid("a")));
    assert!(!s.is_granted(&pid("a")), "mute must revoke the grant");
    assert!(
        matches!(out.first(), Some(FloorSignal::FloorIdle { reason: None, .. })),
        "the freed Open floor goes idle in front of everyone: {out:?}",
    );

    // un-mute clears the ban and NOTHING else (A3): no re-grant, no emit
    let (s, out) = s.apply(
        &FloorEvent::MuteSet { actor: pid("admin"), peer: pid("a"), on: false },
        &FloorLimits::default(),
    );
    assert!(!s.is_muted(&pid("a")));
    assert!(!s.is_granted(&pid("a")), "un-mute must not hand the floor back");
    assert!(out.is_empty(), "un-mute emits nothing - the peer presses again");

    // the next request carries its own priority, so it needs no remembered state
    let (s, _) = s.apply(
        &FloorEvent::Request { peer: pid("a"), priority: 0, preempt: false },
        &FloorLimits::default(),
    );
    assert!(s.is_granted(&pid("a")), "a freed member can hold again");
}

#[test]
fn muted_peer_request_is_refused_visibly_and_a_queue_slot_is_voided_too() {
    // Exclusive, cap 1: a holds, b queues, then both get muted.
    let s = state(FloorMode::Exclusive);
    let (s, _) = s.apply(
        &FloorEvent::Request { peer: pid("a"), priority: 0, preempt: false },
        &FloorLimits::default(),
    );
    let (s, _) = s.apply(
        &FloorEvent::Request { peer: pid("b"), priority: 0, preempt: false },
        &FloorLimits::default(),
    );
    assert_eq!(s.queue().len(), 1, "b queued behind a");

    let (s, out) = s.apply(
        &FloorEvent::MuteSet { actor: pid("admin"), peer: pid("a"), on: true },
        &FloorLimits::default(),
    );
    assert_eq!(
        s.grants().to_vec(),
        vec![pid("b")],
        "muting the holder promotes the queue head, not silence",
    );
    assert!(
        matches!(out.first(), Some(FloorSignal::FloorGranted { .. })),
        "the promotion must be broadcast",
    );

    // b now holds; muting b empties the room
    let (_, out) = s.apply(
        &FloorEvent::MuteSet { actor: pid("admin"), peer: pid("b"), on: true },
        &FloorLimits::default(),
    );
    assert!(
        matches!(out.first(), Some(FloorSignal::FloorIdle { reason: None, .. })),
        "muting the last holder idles the room: {out:?}",
    );

    // a muted member who presses again is refused WITH A REASON (silent drops are
    // forbidden, modules/01 sec 1), reusing the existing Unauthorized variant so
    // pre-S7.5 clients keep parsing (D16).
    let (_, out) = s.apply(
        &FloorEvent::Request { peer: pid("a"), priority: 9, preempt: true },
        &FloorLimits::default(),
    );
    assert!(
        matches!(
            out.first(),
            Some(FloorSignal::FloorDenied { reason: DenyReason::Unauthorized, .. })
        ),
        "muted request refused visibly with the existing reason: {out:?}",
    );
}

#[test]
fn mute_voids_a_queued_reservation_too() {
    let s = state(FloorMode::Exclusive);
    let (s, _) = s.apply(
        &FloorEvent::Request { peer: pid("a"), priority: 0, preempt: false },
        &FloorLimits::default(),
    );
    let (s, _) = s.apply(
        &FloorEvent::Request { peer: pid("b"), priority: 5, preempt: false },
        &FloorLimits::default(),
    );
    let (s, out) = s.apply(
        &FloorEvent::MuteSet { actor: pid("admin"), peer: pid("b"), on: true },
        &FloorLimits::default(),
    );
    assert_eq!(s.queue().len(), 0, "mute voids the queue slot (S7.5 A3)");
    assert_eq!(
        s.grants().to_vec(),
        vec![pid("a")],
        "the sitting holder is untouched by muting a queued peer",
    );
    assert!(out.is_empty(), "no floor change, so no rebroadcast - the roster rides the snapshot",);
}

// ── D16 queue purge on Leave ────────────────────────────────────────────────

#[test]
fn d16_leave_while_queued_purges_and_rebroadcasts_positions() {
    let s = state(FloorMode::Exclusive);
    let (s, _) = s.apply(
        &FloorEvent::Request { peer: pid("a"), priority: 0, preempt: false },
        &FloorLimits::default(),
    );
    let (s, _) = s
        .apply(
            &FloorEvent::Request { peer: pid("b"), priority: 0, preempt: false },
            &FloorLimits::default(),
        )
        .0
        .apply(
            &FloorEvent::Request { peer: pid("c"), priority: 0, preempt: false },
            &FloorLimits::default(),
        );
    assert_eq!(s.queue().len(), 2); // b(0), c(1)
    let g = generation(&s);
    let (s, out) = s.apply(&FloorEvent::Leave { peer: pid("b") }, &FloorLimits::default());
    assert_eq!(s.queue().len(), 1);
    assert_eq!(s.queue()[0].peer, pid("c"));
    // c was position 1 → now 0: rebroadcast. b left: gets nothing.
    assert_eq!(emitted(&out), vec!["queued(0)"]);
    assert!(generation(&s) > g, "queue mutation is a real transition");
}

#[test]
fn d16_leave_of_granted_peer_promotes_like_release() {
    let s = state(FloorMode::Exclusive);
    let (s, _) = s
        .apply(
            &FloorEvent::Request { peer: pid("a"), priority: 0, preempt: false },
            &FloorLimits::default(),
        )
        .0
        .apply(
            &FloorEvent::Request { peer: pid("b"), priority: 0, preempt: false },
            &FloorLimits::default(),
        );
    let (s, out) = s.apply(&FloorEvent::Leave { peer: pid("a") }, &FloorLimits::default());
    assert_eq!(emitted(&out), vec!["granted([\"b\"])"]);
    assert_eq!(s.grants(), &[pid("b")]);
}

// ── generation boot offset ──────────────────────────────────────────────────

#[test]
fn initial_is_monotonic_nonzero_and_ts_number_safe() {
    // O29: the in-process counter stays far below 2^53 (JS Number precision)
    // and strictly increases within a process lifetime. Restart reset is safe
    // under R12: floor state is in-memory and clients resync on reconnect.
    let a = FloorState::initial(FloorMode::Exclusive);
    let b = FloorState::initial(FloorMode::Exclusive);
    assert_ne!(a.generation(), 0);
    assert_ne!(b.generation(), 0);
    assert!(b.generation() > a.generation(), "counter strictly increases");
    assert!(b.generation() < (1u64 << 53), "TS-safe: plain Number compares exactly");
}

#[test]
fn initial_with_generation_zero_allowed_for_tests() {
    let s = FloorState::initial_with_generation(FloorMode::Exclusive, 0);
    assert_eq!(s.generation(), 0);
}

// ── R9 supplementary coverage: remaining mode-switch directions ────────────

#[test]
fn r9_hybrid_to_exclusive_demotes_tail_with_priority() {
    let s = state(FloorMode::Hybrid);
    let (s, _) = s
        .apply(
            &FloorEvent::Request { peer: pid("a"), priority: 4, preempt: false },
            &FloorLimits::default(),
        )
        .0
        .apply(
            &FloorEvent::Request { peer: pid("b"), priority: 2, preempt: false },
            &FloorLimits::default(),
        );
    assert_eq!(s.grants().len(), 2);
    let (s, out) = s.apply(
        &FloorEvent::ModeChange { mode: FloorMode::Exclusive, actor: pid("admin") },
        &FloorLimits::default(),
    );
    assert_eq!(s.grants(), &[pid("a")], "head grant kept");
    assert_eq!(s.queue().len(), 1);
    assert_eq!(s.queue()[0].peer, pid("b"));
    assert_eq!(s.queue()[0].priority, 2, "demoted grant keeps its priority");
    assert_eq!(emitted(&out), vec!["queued(0)"]);
}

#[test]
fn r9_hybrid_to_open_drops_queue_and_re_mits_idle() {
    let s = state(FloorMode::Hybrid);
    // hybrid cap 2: a and b grant, q1 is the one that queues
    let (s, _) = s
        .apply(
            &FloorEvent::Request { peer: pid("a"), priority: 0, preempt: false },
            &FloorLimits::default(),
        )
        .0
        .apply(
            &FloorEvent::Request { peer: pid("b"), priority: 0, preempt: false },
            &FloorLimits::default(),
        )
        .0
        .apply(
            &FloorEvent::Request { peer: pid("q1"), priority: 0, preempt: false },
            &FloorLimits::default(),
        );
    assert_eq!(s.queue().len(), 1);
    let (s, out) = s.apply(
        &FloorEvent::ModeChange { mode: FloorMode::Open, actor: pid("admin") },
        &FloorLimits::default(),
    );
    assert!(s.queue().is_empty(), "queue dropped");
    assert_eq!(s.mode(), FloorMode::Open);
    assert_eq!(emitted(&out), vec!["idle(None)"]);
}

#[test]
fn r9_noop_mode_change_is_silent() {
    let s = state(FloorMode::Exclusive);
    let g = generation(&s);
    let (s, out) = s.apply(
        &FloorEvent::ModeChange { mode: FloorMode::Exclusive, actor: pid("admin") },
        &FloorLimits::default(),
    );
    assert!(out.is_empty());
    assert_eq!(generation(&s), g, "same-mode change = no-op");
}

#[test]
fn r6_media_down_of_ungranted_peer_is_noop() {
    let s = state(FloorMode::Exclusive);
    let g = generation(&s);
    let (s, out) = s.apply(&FloorEvent::MediaDown { peer: pid("ghost") }, &FloorLimits::default());
    assert!(out.is_empty());
    assert_eq!(generation(&s), g);
}

#[test]
fn r2_promote_rebroadcasts_shifted_positions() {
    let s = state(FloorMode::Exclusive);
    let (s, _) = s
        .apply(
            &FloorEvent::Request { peer: pid("h"), priority: 0, preempt: false },
            &FloorLimits::default(),
        )
        .0
        .apply(
            &FloorEvent::Request { peer: pid("q1"), priority: 0, preempt: false },
            &FloorLimits::default(),
        )
        .0
        .apply(
            &FloorEvent::Request { peer: pid("q2"), priority: 0, preempt: false },
            &FloorLimits::default(),
        )
        .0
        .apply(
            &FloorEvent::Request { peer: pid("q3"), priority: 0, preempt: false },
            &FloorLimits::default(),
        );
    assert_eq!(s.queue().len(), 3);
    let (s, out) = s.apply(&FloorEvent::Release { peer: pid("h") }, &FloorLimits::default());
    // q1 promoted (granted); q2,q3 shift → rebroadcast 0,1
    assert_eq!(emitted(&out), vec!["granted([\"q1\"])", "queued(0)", "queued(1)"]);
    assert_eq!(s.queue().len(), 2);
}

// ── P0 multi-web concurrent talk (Open mode) ───────────────────────────────

#[test]
fn open_concurrent_requests_all_granted_release_drops_only_that_peer() {
    let req = |p: &str| FloorEvent::Request { peer: pid(p), priority: 0, preempt: false };
    let s = state(FloorMode::Open);
    let (s, _) = s.apply(&req("a"), &FloorLimits::default());
    let (s, _) = s.apply(&req("b"), &FloorLimits::default());
    let (s, _) = s.apply(&req("c"), &FloorLimits::default());
    assert_eq!(s.grants().len(), 3, "Open grants every concurrent request");

    let (s, out) = s.apply(&FloorEvent::Release { peer: pid("b") }, &FloorLimits::default());
    let names: Vec<String> = s.grants().iter().map(|g| g.to_string()).collect();
    assert_eq!(names, vec!["a".to_string(), "c".to_string()], "release drops only b");
    assert_eq!(emitted(&out), vec!["granted([\"a\", \"c\"])"]);
}

// ── O18: QueuePrioritySet (dispatcher re-prioritization) ────────────────────

#[test]
fn queue_priority_set_reorders_priority_first_stable() {
    use mediaservo_common::floor::{FloorEvent, FloorLimits};

    let mut s =
        FloorState::initial_with_generation(mediaservo_common::floor::FloorMode::Exclusive, 10);
    // queue three peers at priority 1: a, b, c (arrival order)
    let limits = FloorLimits::default();
    for p in ["a", "b", "c"] {
        let ev = FloorEvent::Request { peer: p.to_string(), priority: 1, preempt: false };
        let (ns, _) = s.apply(&ev, &limits);
        s = ns;
    }
    // Exclusive capacity=1: the first requester (a) is GRANTED; b, c queue at priority 1.
    assert_eq!(s.grants(), ["a"]);
    assert_eq!(s.queue().iter().map(|p| p.peer.as_str()).collect::<Vec<_>>(), ["b", "c"]);

    // dispatcher boosts c to 9 -> [c(9), b(1)] (equal priorities keep arrival order)
    let ev = FloorEvent::QueuePrioritySet {
        actor: "dispatch".to_string(),
        peer: "c".to_string(),
        priority: 9,
    };
    let (ns, out) = s.apply(&ev, &limits);
    assert_eq!(ns.queue().iter().map(|p| p.peer.as_str()).collect::<Vec<_>>(), ["c", "b"]);
    assert_eq!(ns.queue()[0].priority, 9);
    // reducer emits nothing — the server follows up privately (role-scoped, D12)
    assert!(out.is_empty());
    assert!(ns.generation() > s.generation(), "mutation bumps generation");
}

#[test]
fn queue_priority_set_ignores_non_queued_peer() {
    use mediaservo_common::floor::{FloorEvent, FloorLimits};

    let s = FloorState::initial_with_generation(mediaservo_common::floor::FloorMode::Exclusive, 1);
    let limits = FloorLimits::default();
    let ev = FloorEvent::QueuePrioritySet {
        actor: "dispatch".to_string(),
        peer: "ghost".to_string(),
        priority: 9,
    };
    let (ns, out) = s.apply(&ev, &limits);
    assert_eq!(ns, s, "non-queued peer: no state change, no generation bump");
    assert!(out.is_empty());
}
