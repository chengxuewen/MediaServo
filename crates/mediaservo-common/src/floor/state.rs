//! FloorState — the pure floor-arbitration reducer (modules/01 rules 1-9, D12 v2 model).
//!
//! Total function design: `apply()` takes `(state, event, limits)` and returns
//! `(new_state, emitted_wire_messages)`. No I/O, no clocks, no randomness inside
//! the reducer — `generation` advances as `prev + 1` (fresh entropy enters only
//! at `initial()` via the process monotonic counter (O29: stays < 2^53 so TS
//! clients compare it with plain Number); tests pin determinism via
//! `initial_with_generation`).

use super::PeerId;
use super::wire::{DenyReason, FloorSignal};
use std::sync::atomic::{AtomicU64, Ordering};

/// Simultaneous-speaker and queue caps, passed per-call so the core stays
/// config-free (values come from server config; modules/06).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FloorLimits {
    /// Max simultaneous granted speakers (Exclusive=1, Hybrid=2 default shape).
    pub max_peers: usize,
    /// Max queued requests before `Denied{Busy}`.
    pub max_queue: usize,
    /// §3.2: max seconds a grant window may run before the server emits `Timeout`
    /// (dispatch profile 45 s). Enforcement lives in the server actor layer — the
    /// core stays wall-clock-free.
    pub max_hold_secs: u64,
    /// §3.2: seconds a grantee has to produce media before revocation (2 s).
    /// The field ships now so the config surface is stable; arming requires the
    /// media plane and lands with the O23 reconcile rewrite.
    pub media_grace_secs: u64,
    /// §3.2: per-connection floor-request guard (requests per rolling minute).
    pub rate_limit_per_min: u32,
    /// §3.2: room-level request-priority cap; the effective ceiling is the lower
    /// of this and the sender's role ceiling.
    pub priority_ceiling: u8,
}

impl Default for FloorLimits {
    fn default() -> Self {
        // modules/01 defaults: exclusive=1/hybrid=2 grants; queue depth 16 at PoC;
        // dispatch profile hold 45 s / media grace 2 s (§3.2)
        Self {
            max_peers: 1,
            max_queue: 16,
            max_hold_secs: 45,
            media_grace_secs: 2,
            rate_limit_per_min: 30,
            priority_ceiling: 100,
        }
    }
}

/// One waiting floor request (dispatch-scope only, D12).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    pub peer: PeerId,
    pub priority: u8,
}

/// Immutable floor state — every `apply()` returns a fresh value (no mutation).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FloorState {
    mode: super::wire::FloorMode,
    grants: Vec<PeerId>,
    /// Request-priority of each grant, index-aligned with `grants`
    /// (modules/01 §3: preempt compares against the sitting holder's
    /// request priority — the state must carry it).
    grant_priority: Vec<u8>,
    muted: Vec<PeerId>,
    queue: Vec<Pending>,
    generation: u64,
}

pub use super::wire::FloorMode;

/// Events the reducer consumes (server-derived; modules/01 §4).
/// `actor` fields mark who drove the change (modules/01 normative signature,
/// D16 audit surface) — the core does not police authorization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FloorEvent {
    Request {
        peer: PeerId,
        priority: u8,
        preempt: bool,
    },
    Release {
        peer: PeerId,
    },
    /// P4-1 admin REST (`force-release`): an admin drops a sitting holder.
    /// Reducer semantics are IDENTICAL to `Release` (R2 auto-promote, R8
    /// no-op) — the distinct variant exists for the audit line only: a reused
    /// `Release` would misattribute the mutation to the holder (§3.5: one
    /// mutation = one honest audit line; the actor is the admin).
    ForceRelease {
        actor: PeerId,
        peer: PeerId,
    },
    Leave {
        peer: PeerId,
    },
    MediaDown {
        peer: PeerId,
    },
    Timeout,
    ModeChange {
        mode: FloorMode,
        actor: PeerId,
    },
    MuteSet {
        actor: PeerId,
        peer: PeerId,
        on: bool,
    },
    /// O18: dispatcher adjusts a queued peer's request priority. The peer must
    /// currently be queued; the queue reorders priority-first (stable). The
    /// reducer emits nothing — the server follows up with private position
    /// updates / a dispatcher snapshot (role-scoped, D12).
    QueuePrioritySet {
        actor: PeerId,
        peer: PeerId,
        priority: u8,
    },
}

impl FloorState {
    /// Production constructor: generation from the in-process monotonic counter
    /// (nonzero, strictly increasing; O29 -- stays far below 2^53 so TS clients
    /// compare it safely with plain Number). Reset on restart is safe: R12
    /// declares floor state in-memory and clients resync (snapshot covers the
    /// local generation, no cross-restart comparison ever happens).
    pub fn initial(mode: FloorMode) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let generation = NEXT.fetch_add(1, Ordering::Relaxed);
        Self::initial_with_generation(mode, generation.max(1))
    }

    /// Deterministic constructor for tests and replay: any generation is legal
    /// (P1-M review: makes the coverage gate deterministic; ordering assumptions
    /// on generation values are forbidden — comparison only ever checks equality).
    pub fn initial_with_generation(mode: FloorMode, generation: u64) -> Self {
        Self {
            mode,
            grants: Vec::new(),
            grant_priority: Vec::new(),
            muted: Vec::new(),
            queue: Vec::new(),
            generation,
        }
    }

    // ── accessors ───────────────────────────────────────────────────────
    pub fn mode(&self) -> FloorMode {
        self.mode
    }
    pub fn grants(&self) -> &[PeerId] {
        &self.grants
    }
    /// Operationally muted peers (MuteSet; survives snapshots — S0.1).
    pub fn muted(&self) -> &[PeerId] {
        &self.muted
    }
    pub fn queue(&self) -> &[Pending] {
        &self.queue
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn is_granted(&self, peer: &PeerId) -> bool {
        self.grants.contains(peer)
    }
    pub fn is_muted(&self, peer: &PeerId) -> bool {
        self.muted.contains(peer)
    }
    pub fn queue_position(&self, peer: &PeerId) -> Option<usize> {
        self.queue.iter().position(|p| &p.peer == peer)
    }

    fn capacity(&self) -> usize {
        match self.mode {
            FloorMode::Exclusive => 1,
            FloorMode::Hybrid => 2,
            FloorMode::Open => usize::MAX,
        }
    }

    /// The reducer (modules/01 rules 1-9). Pure: same inputs → same outputs.
    pub fn apply(&self, ev: &FloorEvent, limits: &FloorLimits) -> (FloorState, Vec<FloorSignal>) {
        let mut next = self.clone();
        let mut out = Vec::new();

        match ev {
            // ── R1/R3/R4/R5: Request ────────────────────────────────────
            FloorEvent::Request { peer, priority, preempt } => {
                if self.is_granted(peer) || self.queue_position(peer).is_some() {
                    // R4 idempotent: echo current position, no state change
                    if let Some(pos) = self.queue_position(peer) {
                        out.push(FloorSignal::FloorQueued {
                            position: pos as u32,
                            generation: self.generation,
                        });
                    }
                    return (next, out);
                }

                // Rule 2 (S7.5 ruling 2026-10-20): a muted member holds no speaking
                // privilege, so the request is refused VISIBLY - silent drops are
                // forbidden (modules/01 sec 1). `unauthorized` is REUSED instead of a new
                // wire variant so pre-S7.5 clients keep parsing (D16 compat); the client
                // tells this denial apart from the S3 command denial by which message it
                // answers, and by its own name sitting in the local `muted` roster.
                if self.is_muted(peer) {
                    out.push(FloorSignal::FloorDenied {
                        reason: DenyReason::Unauthorized,
                        generation: self.generation,
                    });
                    return (next, out);
                }

                // R5 preempt: strictly-higher priority may displace the sole
                // exclusive holder (modules/01: preempt is an Exclusive-mode
                // instrument; other modes ignore the flag — space path below)
                if *preempt && self.mode == FloorMode::Exclusive && !self.grants.is_empty() {
                    let holder_priority = self.grant_priority[0];
                    if *priority > holder_priority {
                        next.grants = vec![peer.clone()];
                        next.grant_priority = vec![*priority];
                        next.generation += 1;
                        // `by` = the new holder; this message routes to the
                        // displaced peer ("your floor was taken by …")
                        out.push(FloorSignal::FloorTaken {
                            by: peer.clone(),
                            generation: next.generation,
                        });
                        out.push(FloorSignal::FloorGranted {
                            grants: next.grants.clone(),
                            generation: next.generation,
                        });
                        return (next, out);
                    }
                    // equal/lower preempt → denied, no-op (R5)
                    out.push(FloorSignal::FloorDenied {
                        reason: DenyReason::PreemptPriority,
                        generation: self.generation,
                    });
                    return (next, out);
                }

                // capacity check
                if self.grants.len() < self.capacity() {
                    next.grants.push(peer.clone());
                    next.grant_priority.push(*priority);
                    next.generation += 1;
                    out.push(FloorSignal::FloorGranted {
                        grants: next.grants.clone(),
                        generation: next.generation,
                    });
                    return (next, out);
                }

                // queue path (R1/R3): insert priority-desc, FIFO within tier
                let entry = Pending { peer: peer.clone(), priority: *priority };
                match next.queue.len().cmp(&limits.max_queue) {
                    std::cmp::Ordering::Greater => {
                        // over ceiling: deny Busy (R5-adjacent; D16 caps)
                        return (
                            next,
                            vec![FloorSignal::FloorDenied {
                                reason: DenyReason::Busy,
                                generation: self.generation,
                            }],
                        );
                    }
                    std::cmp::Ordering::Equal => {
                        return (
                            next,
                            vec![FloorSignal::FloorDenied {
                                reason: DenyReason::Busy,
                                generation: self.generation,
                            }],
                        );
                    }
                    std::cmp::Ordering::Less => {}
                }
                let insert_at = next
                    .queue
                    .iter()
                    .position(|p| p.priority < entry.priority)
                    .unwrap_or(next.queue.len());
                next.queue.insert(insert_at, entry);
                next.generation += 1;
                out.push(FloorSignal::FloorQueued {
                    position: insert_at as u32,
                    generation: next.generation,
                });
                // R3: every PRE-EXISTING entry from insert_at on was shifted
                // down by one — rebroadcast their new positions (the newcomer
                // at insert_at already got its own message above)
                for (pos, p) in next.queue.iter().enumerate().skip(insert_at + 1) {
                    out.push(FloorSignal::FloorQueued {
                        position: pos as u32,
                        generation: next.generation,
                    });
                    let _ = p;
                }
                (next, out)
            }

            // ── R2: Release → auto-promote (ForceRelease shares the body:
            // identical reducer semantics; only the audit line differs) ──
            FloorEvent::Release { peer } | FloorEvent::ForceRelease { peer, .. } => {
                if !self.is_granted(peer) {
                    return (next, out); // R8 no-op
                }
                let idx = self.grants.iter().position(|p| p == peer).unwrap();
                next.grants.remove(idx);
                next.grant_priority.remove(idx);
                next.generation += 1;
                Self::promote_or_idle(&mut next, &mut out);
                (next, out)
            }

            // ── Leave: release-like + D16 queue purge ───────────────────
            FloorEvent::Leave { peer } => {
                let was_granted = self.is_granted(peer);
                let was_queued_at = self.queue_position(peer);
                if !was_granted && was_queued_at.is_none() {
                    return (next, out); // R8 no-op
                }
                if let Some(idx) = self.grants.iter().position(|p| p == peer) {
                    next.grants.remove(idx);
                    next.grant_priority.remove(idx);
                }
                if let Some(pos) = was_queued_at {
                    next.queue.remove(pos);
                    next.generation += 1;
                    // D16: rebroadcast shifted positions; the leaver gets nothing
                    for (i, p) in next.queue.iter().enumerate().skip(pos) {
                        out.push(FloorSignal::FloorQueued {
                            position: i as u32,
                            generation: next.generation,
                        });
                        let _ = p;
                    }
                    return (next, out);
                }
                next.generation += 1;
                Self::promote_or_idle(&mut next, &mut out);
                (next, out)
            }

            // ── R6: MediaDown = immediate release (grace is server-domain) ──
            FloorEvent::MediaDown { peer } => {
                let idx = match self.grants.iter().position(|p| p == peer) {
                    Some(i) => i,
                    None => return (next, out),
                };
                next.grants.remove(idx);
                next.grant_priority.remove(idx);
                next.generation += 1;
                Self::promote_or_idle(&mut next, &mut out);
                (next, out)
            }

            // ── R7: Timeout ─────────────────────────────────────────────
            FloorEvent::Timeout => {
                if self.grants.is_empty() {
                    return (next, out);
                }
                next.grants.clear();
                next.grant_priority.clear();
                next.generation += 1;
                out.push(FloorSignal::FloorIdle {
                    generation: next.generation,
                    reason: Some("timeout".into()),
                });
                // timed-out holder freed the floor — promote like release;
                // the timeout idle IS the idle emission (no second one)
                if !next.queue.is_empty() {
                    let head = next.queue.remove(0);
                    next.grants.push(head.peer);
                    next.grant_priority.push(head.priority);
                    next.generation += 1;
                    out.push(FloorSignal::FloorGranted {
                        grants: next.grants.clone(),
                        generation: next.generation,
                    });
                    for (i, _) in next.queue.iter().enumerate() {
                        out.push(FloorSignal::FloorQueued {
                            position: i as u32,
                            generation: next.generation,
                        });
                    }
                }
                (next, out)
            }

            // ── R9: ModeChange ──────────────────────────────────────────
            FloorEvent::ModeChange { mode, actor } => {
                if *mode == self.mode {
                    return (next, out);
                }
                next.mode = *mode;
                next.generation += 1;
                let _ = actor;
                match (*mode, self.mode) {
                    // anything → Open: drop queue, everyone may speak
                    (FloorMode::Open, _) => {
                        next.queue.clear();
                        if !self.grants.is_empty() {
                            out.push(FloorSignal::FloorIdle {
                                generation: next.generation,
                                reason: None,
                            });
                        }
                    }
                    // Open → tighter: keep first-cap grants, rest queue in order
                    (FloorMode::Exclusive, FloorMode::Open)
                    | (FloorMode::Hybrid, FloorMode::Open) => {
                        let cap = next.capacity();
                        if self.grants.len() > cap {
                            let demoted: Vec<PeerId> = self.grants[cap..].to_vec();
                            let demoted_prio: Vec<u8> = self.grant_priority[cap..].to_vec();
                            next.grants = self.grants[..cap].to_vec();
                            next.grant_priority = self.grant_priority[..cap].to_vec();
                            for (p, prio) in demoted.into_iter().zip(demoted_prio) {
                                next.queue.push(Pending { peer: p, priority: prio });
                            }
                            out.push(FloorSignal::FloorQueued {
                                position: 0,
                                generation: next.generation,
                            });
                        }
                    }
                    (FloorMode::Exclusive, FloorMode::Hybrid) => {
                        // cap drops 2 → 1: demote the tail
                        let cap = next.capacity();
                        if self.grants.len() > cap {
                            let demoted: Vec<PeerId> = self.grants[cap..].to_vec();
                            next.grants = self.grants[..cap].to_vec();
                            next.grant_priority = self.grant_priority[..cap].to_vec();
                            for (i, p) in demoted.into_iter().enumerate() {
                                next.queue.push(Pending {
                                    peer: p,
                                    priority: self.grant_priority[cap + i],
                                });
                            }
                            out.push(FloorSignal::FloorQueued {
                                position: 0,
                                generation: next.generation,
                            });
                        }
                    }
                    (FloorMode::Hybrid, FloorMode::Exclusive) => {
                        let cap = next.capacity();
                        if self.grants.len() > cap {
                            let demoted: Vec<PeerId> = self.grants[cap..].to_vec();
                            next.grants = self.grants[..cap].to_vec();
                            next.grant_priority = self.grant_priority[..cap].to_vec();
                            for (i, p) in demoted.into_iter().enumerate() {
                                next.queue.push(Pending {
                                    peer: p,
                                    priority: self.grant_priority[cap + i],
                                });
                            }
                            out.push(FloorSignal::FloorQueued {
                                position: 0,
                                generation: next.generation,
                            });
                        }
                    }
                    _ => {}
                }
                (next, out)
            }

            // -- mute governance (Rule 2/8, re-scoped by the S7.5 walkthrough
            // 2026-10-20: mute is a standing privilege on a room member) ----------
            FloorEvent::MuteSet { actor, peer, on } => {
                let _ = actor;
                // The core holds no roster, so "is this peer actually in the room" is the
                // server's check (room.rs) - here any id may be muted, and muting takes
                // away everything the floor grants: the grant AND the queue slot (a
                // queued position is a reservation of the very privilege being revoked).
                if *on {
                    if !next.muted.contains(peer) {
                        next.muted.push(peer.clone());
                    }
                    // A peer is never both granted and queued (Request returns early on
                    // either), so the two removals are exclusive cases - each mirrors the
                    // shape the Release/Leave branches already use for the same transition.
                    let was_granted = next.grants.iter().position(|g| g == peer);
                    if let Some(i) = was_granted {
                        next.grants.remove(i);
                        next.grant_priority.remove(i);
                        next.generation += 1;
                        // freed capacity promotes the queue head; promote_or_idle carries
                        // the PIT-21 shrink rebroadcast so every mirror converges.
                        Self::promote_or_idle(&mut next, &mut out);
                    } else if let Some(pos) = next.queue.iter().position(|p| p.peer == *peer) {
                        next.queue.remove(pos);
                        next.generation += 1;
                        // queue-only voiding: shifted positions rebroadcast (Leave
                        // precedent), the sitting holder is NOT re-announced.
                        for i in pos..next.queue.len() {
                            out.push(FloorSignal::FloorQueued {
                                position: i as u32,
                                generation: next.generation,
                            });
                        }
                    }
                    // Roster-only mute (a silent member): no generation bump, no floor
                    // event. The name reaches clients through the role-scoped snapshot the
                    // server pushes for MuteSet (room.rs) - that is how the ws_flow
                    // "MuteSet mutates silently" gap closes without a wire variant.
                } else {
                    // Un-mute restores the PRIVILEGE only, never the floor (A3): the
                    // vacated slot may already belong to a promoted head, and re-adding
                    // would either break the cap or pre-empt without priority (Rule 4).
                    // The peer presses again and that request carries its own priority,
                    // so there is no priority state to remember or replay.
                    next.muted.retain(|p| p != peer);
                }
                (next, out)
            }
            FloorEvent::QueuePrioritySet { actor, peer, priority } => {
                let _ = actor;
                // O18: only a queued peer can be re-prioritized (grants/members are
                // outside the queue's authority). Priority-first stable reorder:
                // the queue is a service order, so equal priorities keep arrival order.
                if let Some(slot) = next.queue.iter_mut().find(|p| p.peer == *peer) {
                    slot.priority = *priority;
                } else {
                    return (next, out);
                }
                next.queue.sort_by_key(|p| std::cmp::Reverse(p.priority));
                next.generation += 1;
                (next, out)
            }
        }
    }

    /// R2: after grants shrink — auto-promote queue head, or FloorIdle when
    /// everything is empty (promote emits FloorGranted ONLY per modules/01).
    fn promote_or_idle(next: &mut FloorState, out: &mut Vec<FloorSignal>) {
        if next.grants.len() < next.capacity() && !next.queue.is_empty() {
            let head = next.queue.remove(0);
            next.grants.push(head.peer);
            next.grant_priority.push(head.priority);
            next.generation += 1;
            out.push(FloorSignal::FloorGranted {
                grants: next.grants.clone(),
                generation: next.generation,
            });
            // shifted positions rebroadcast
            for (i, _) in next.queue.iter().enumerate() {
                out.push(FloorSignal::FloorQueued {
                    position: i as u32,
                    generation: next.generation,
                });
            }
            return;
        }
        if next.grants.is_empty() && next.queue.is_empty() {
            out.push(FloorSignal::FloorIdle { generation: next.generation, reason: None });
        } else if !next.grants.is_empty() {
            // D12 grant-set gap: the set shrank (release/leave/media-down) with
            // no head to promote but holders remaining — Exclusive-era callers only
            // ever saw "promote" or "idle", so a multi-holder (Open/Hybrid) room
            // left every mirror stale. Re-broadcast so all peers converge.
            out.push(FloorSignal::FloorGranted {
                grants: next.grants.clone(),
                generation: next.generation,
            });
        }
    }
}
