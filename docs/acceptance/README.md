# Acceptance matrix — P6 port of the archived talkservo-port §5.2

Opened 2026-11-11 (plan `docs/plans/acceptance-release/` in the shell repo).
Status legend: **✅ covered** (evidence linked) · **🔶 partial** (unit covered,
listed e2e remains) · **⏳ gap** (P6-2 worklist) · **🔵 deferred** (P5 spike,
R7 non-gating).

A15a/b (WebEngine / C-ABI native evidence) belong to the P5 spike and are NOT
a milestone gate here (R7; CF2: media paths never merge into one OR row).

## Matrix

| # | Criterion (condensed) | Status | Evidence |
|---|---|---|---|
| A1 | Exclusive: A holds, B/C denied+queued, A unpaused / B,C paused | ✅ | O26 matrix B1/B2 (3 contexts, fake media); `floor_rules` R1; `sfu.rs` pause_producer/resume_producer (P2 media-first) |
| A2 | Release → Idle or auto-promote head; promoted peer resumes without re-produce | ✅ | O26 B4/B5; `floor_rules` R2 (promote + idle); reconcile resume chain (O23) |
| A3 | Preempt only strictly-higher priority; equal/lower → `preempt_priority` | ✅ | `floor_rules` R5 (strict swap / deny); `state.rs` unit matrix |
| A4 | Mute revokes grant+queue; unmute does not restore; muted request → `unauthorized` | ✅ | O26 B6 (revoke → denied → unmute → fresh request grants); `floor_rules` mute arms |
| A5 | Mode transitions Exclusive/Hybrid/Open preserve caps + requeue order (rules 8-9) | ✅ | `floor_rules` R9 (X→O drop, O→X keep-first, H cap-2); O26 B7 (mode_set round-trip) |
| A6 | Reconnect/resync: snapshot restores, no double grant, stale generation discarded | ✅ | O26 B3/B8/B9 (reload resync); `floor_rules` stale-generation filter; O29 counter semantics |
| A7 | Holder WS drop → release; upload death (grace) → MediaDown → release | 🔶 | `floor_rules` Leave/MediaDown arms + reconcile MediaDown (O23) covered at unit level; e2e kill-socket listed in P6-2 |
| A8 | Non-holder producer paused at SFU; consumer-side stats show no growth | 🔶 | unit: producer pause/resume (P2) + media-first reconcile; e2e: p0-audio harness byte-growth probes exist (pub/sub counters) — a paused-growth assertion is listed in P6-2 (optional) |
| A9 | Unauthorized mode/mute denied AND audited; operator/viewer cannot govern | ✅ | server lib floor_command role-gate tests (4032); `floor_command_allowed`; audit FloorAction emission in `apply_floor_event` |
| A10 | Every mutation writes one audit line (actor/target/reason) | 🔶 | code path covered (audit line in `apply_floor_event` + REST mutations share it); a dedicated audit-JSONL assertion test is listed in P6-2 (optional) |
| A11 | Admin action repaints another tab < 1 s; socket-down → degraded + poll fallback | 🔶 | server: FloorChanged push < 1 s asserted (floor-admin-ops P4-2 test); UI: Audio.tsx event-driven + 5 s poll fallback; a two-tab Playwright e2e is listed in P6-2 (optional) |
| A12 | PTT latency: LAN p50 ≤ 350 ms, p95 ≤ 500 ms (new); public p50 ≤ 600 ms | ✅ LAN / ⏳ public | `evidence/a12-ptt-latency.json`: p50 47 ms, p95 60 ms (25 presses, oscillator + AnalyserNode edge); public budget → P6-3 (weaknet profile, ADJ-1 open) |
| A13 | Non-talk rooms (incl. legacy `audio-*`) send ZERO `floor_*` frames; producers never floor-gated | ⏳ | command-side 4032 negative exists (`floor_rules` + role gates); FRAME-side zero-leak e2e negative → **P6-2** |
| A14 | Crash recovery: kill -9 + respawn rebuilds, tokens verify, late joiner gets snapshot (Idle per R12) | ⏳ | → **P6-2** (resume table state after respawn included) |
| A15a | WebEngine audible-path evidence | 🔵 | P5 spike (R7: failure recorded, not a red milestone) |
| A15b | C-ABI native evidence (RTP bytes > 0, ABI symbol/soname/header) | 🔵 | P5 spike (O25 probe-first; not a milestone gate) |

## P6-2 gap worklist (ordered)

1. **A13** frame-side zero-leak negative (e2e): join a non-talk room, assert no
   `floor_*` frame ever arrives and producers are never paused by floor.
2. **A14** crash-recovery e2e: `kill -9` + respawn → rooms rebuilt, token
   verifies, late joiner receives `floor_state` (Idle), resume table sane.
3. *(optional)* **A7** e2e kill-socket → immediate release.
4. *(optional)* **A10** audit-JSONL per-mutation assertion test.
5. *(optional)* **A8** paused-producer consumer-stats non-growth assertion.
6. *(optional)* **A11** two-tab repaint Playwright e2e (push < 1 s already
   asserted server-side).

Items 3-6 are optional because their substance is covered at unit/test level
(linked above); they graduate to real gaps only if a review finds the unit
evidence insufficient.

## Relationship to other evidence

- O26 floor matrix: `docs/plans/voice-web-app/evidence/o26-matrix.json` (11
  scenarios, shell repo) — run via `www/tests/o26-floor-matrix`.
- A12 latency: `docs/plans/voice-web-app/evidence/a12-ptt-latency.json`
  (shell repo) — run via `www/tests/a12-ptt-latency`.
- Wire/negative units: `crates/mediaservo-common/tests/floor_rules.rs` (R1-R9
  + O26-surfaced voice-domain + managed-table tests).
