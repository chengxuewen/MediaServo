---
name: test-driven-development
description: "Red-green-refactor for MediaServo Rust and protocol work: write the failing test first, make it pass minimally, then refactor under the gates. Use for new behavior, bug fixes with a guard, and contract changes."
---

# Test-Driven Development

> Red first, green minimal, refactor under the gates.

## Trigger Conditions

- New behavior with an observable contract.
- A bug fix that needs a regression guard.
- A protocol or API change where the wire contract must be pinned.
- 用户说“测试先行 / 红绿重构 / TDD / 先写失败测试” <!-- c49:trigger -->

## Do Not Use

- Exploratory spikes with no stable contract: timebox and delete the spike first.
- Pure documentation or formatting changes.
- Coverage skeleton generation from specs: use `test-harness`.

## The Loop

1. **Name the behavior in one sentence.** If you cannot, the test is not ready.
2. **Write the smallest failing test.**
   - Rust unit/integration test in the owning crate.
   - Protocol test for WS/REST shape and error paths.
   - Browser assertion only when the behavior is UI-level.
3. **Run it and prove red.**
   ```bash
   cargo test -p <crate> <test_name> -- --nocapture
   ```
   A test that passes before the change proves nothing.
4. **Make it pass minimally.** No drive-by refactor, no new abstraction.
5. **Run the focused test, then the crate suite.**
6. **Refactor with the test green.** Keep the public contract fixed.
7. **Run the gates.**
   ```bash
   pixi run check
   pixi run test
   pixi run ci
   ```

## Test Type Selection

| Behavior | Test |
|---|---|
| Pure Rust logic | unit test in the crate |
| Input-space edge cases | `proptest` |
| Stable output shape | `insta` snapshot |
| WS/REST contract | protocol test with serde round-trip |
| Cross-crate integration | integration test with a minimal fixture |
| SFU/media path | focused integration test; keep it deterministic |
| UI data path | browser or data assertion, not a screenshot-only check |

## Rules

- One behavior per test; one reason to fail.
- No network, wall-clock sleep, or shared mutable global state unless the test owns it.
- Pin the error path, not only the happy path.
- A regression test must fail on the parent commit and pass on the fix.
- Do not weaken an existing assertion to make a change pass.
- Keep test names as behavior statements, not implementation names.

## Anti-Patterns

| Smell | Fix |
|---|---|
| Test written after the implementation, never seen red | Revert the implementation, prove red, reapply. |
| Snapshot updated blindly | Inspect the diff; update only when the change is intended. |
| Mocking the unit under test | Test the real unit; mock only the boundary. |
| Sleep-based synchronization | Use a deterministic signal or a bounded poll. |
| One giant integration test | Split by contract; keep failures local. |

## Exit Criteria

- Red was observed and recorded.
- Green is minimal and the focused test passes.
- Refactor left the contract unchanged.
- Gates green; the new test is part of the change.

## Boundaries

- `test-harness`: generates skeletons and coverage maps; this skill drives one behavior to green.
- `debugging-and-error-recovery`: investigates an unexplained failure; this skill starts from a known behavior gap.
- `api-interface-design`: decides the contract; this skill pins it with a test.
