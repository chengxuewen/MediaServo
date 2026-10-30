---
name: debugging-and-error-recovery
description: "Systematic debugging for MediaServo: reproduce, capture evidence, localize the layer, narrow with bisect or a minimal input, fix minimally, add a regression guard, and roll back safely. Use when a test, build, gate, or runtime behavior is wrong."
---

# Debugging and Error Recovery

> Reproduce before you fix. A fix without a reproduction is a guess.

## Trigger Conditions

- A test, build, clippy, or `pixi run ci` / `all-gates` segment is red.
- Runtime behavior is wrong but nothing crashes (silent bug).
- An Admin Dashboard or SFU symptom appears without a clear stack trace.
- A weak-network scenario regresses.
- 用户说“调试 / 复现 / 定位根因 / 回归 / 先别改” <!-- c49:trigger -->

## Do Not Use

- Design or scope decisions: use `think-before-act` or `openspec-explore`.
- Pure performance tuning: use `performance-optimization`.
- A known style/lint failure with an obvious fix: fix it directly and run the gate.

## Workflow

### 1. Reproduce deterministically

```bash
pixi run check
pixi run test
cargo test -p <crate> <test_name> -- --nocapture
RUST_BACKTRACE=1 cargo test -p <crate> <test_name>
```

- Write the exact command and the exact failure text.
- If it is intermittent, record the rate and the conditions (profile, stream mode, browser, network).
- For weak-network or SFU symptoms, capture the profile and the scenario before changing code.

### 2. Capture evidence before editing

- Full error, not the last line.
- Logs and spans around the failure; include timestamps and IDs.
- `git log --oneline -10`, `git diff`, and the last known-good commit.
- The gate or test that should have caught it.

### 3. Localize the layer

| Layer | Typical evidence | First probe |
|---|---|---|
| Rust crate | panic, type error, failing unit test | `cargo test -p <crate>` |
| C ABI / FFI | crash at the boundary, ABI mismatch | `pixi run gate-abi` / `check-cxx` |
| SFU / mediasoup | transport, ICE, RTP, worker logs | server logs + `gate-mediasoup` |
| Admin Dashboard | console/network error, stale UI | browser devtools + `pixi run check` |
| Agent/tooling | generator drift, gate red | `pixi run agent-tools` |
| Weak network | latency/loss-dependent failure | `scripts/weaknet` profile + scenario |

### 4. Narrow

- Shrink the input: one stream, one codec, one frame, one route, one config key.
- Use `git bisect` when the regression window is known.
- Disable one variable at a time; never change two layers in one probe.

### 5. Fix minimally

- Change the smallest surface that removes the cause.
- Do not refactor while debugging; park cleanups for a separate change.
- If the fix is risky, prefer a reversible change or a feature flag over a rewrite.

### 6. Add a regression guard

- Rust: unit test for the exact failure; `proptest` for input-space bugs; `insta` for output shape.
- Protocol: a WS/REST contract test.
- UI: a deterministic browser check or a data-path assertion.
- Gate: a planted-failure self-test if the bug was in a guard.

The guard must fail without the fix and pass with it.

### 7. Verify and recover

```bash
pixi run check
pixi run test
pixi run ci
```

- Quote the before/after evidence.
- If the fix is not safe, document the rollback and stop; do not leave a half-fix.

## Exit Criteria

- Reproducible on demand, or nondeterminism quantified.
- Root cause stated in one sentence.
- Minimal fix + regression guard, both verified.
- Relevant gates green; no unrelated changes in the diff.

## Anti-Patterns

| Excuse | Reality |
|---|---|
| "I'll change this and see" | Without a reproduction you cannot tell whether it is fixed or moved. |
| "The test is flaky, ignore it" | Flakiness is a bug; capture the rate and root-cause it. |
| "Skip the test to get green" | Hiding the symptom breaks the gate's meaning. |
| "It only fails on my machine" | Compare the pixi environment and the exact profile. |
| "No time for a regression test" | The test is the proof; without it the bug returns. |

## Boundaries

- `think-before-act`: decides whether to act; this skill runs the debugging loop.
- `performance-optimization`: measures and tunes; this skill fixes correctness.
- `test-harness`: generates coverage skeletons; this skill proves one failure.
