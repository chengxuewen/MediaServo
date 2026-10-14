---
name: think-before-act
description: "Meta-constraint: check before acting, list options for user approval, no brute-forcing. Use BEFORE any non-trivial action (debug/test/implement/refactor/fix/config/upgrade). Especially when facing silent failures, runtime errors not caught by compiler, or behavior that contradicts documentation."
---

# think-before-act — research → list options → user approval → (team review) → execute

> No brute-forcing.

## Trigger Conditions

Any **non-trivial** operation: debugging, testing, implementation, refactoring, fixes, config changes, upgrades.
Trigger especially when the failure mode is unusual: silent failures, runtime errors the compiler cannot catch, behavior that contradicts the docs.

---

## Decision Gates (self-ask before every operation)

1. **"What does the official recommendation say?"** → if you don't know, look it up — don't guess.
2. **"Has this project hit this before?"** → check the project knowledge base (.agents/memorys/ / git log).
3. **"Has the community solved it?"** → search issues / forums / StackOverflow (search the error message verbatim).
4. **"Does the user approve this approach?"** → list options and wait for approval — don't just edit.
5. **"Why did the last attempt fail?"** → if you can't explain it, don't try the next one.

---

## Tiers

| Tier | Criteria | Flow |
|------|------|------|
| **Trivial** | no interface/build/dependency change, reversible, single file, no side effects | just do it + one-line note |
| **Standard** | touches interface/build/deps/multi-file/irreversible/framework-level | Phase 1→2→3 |
| **Urgent** | user explicitly declares "urgent / fix first report later" (production down, data loss) | do it first + backfill the report |

---

## Phase 1: Research (Standard tier must not skip)

### 5-Layer Knowledge Sources (by priority)

1. **Project history**: .agents/memorys/decisions.md / pitfalls.md / git log
2. **Language/toolchain native docs**: cargo doc / rustdoc / --help / man
3. **Framework/tool official docs**: find the recommended practice, not just any search hit
4. **Community experience**: GitHub issues / StackOverflow / forums (search the error message verbatim)
5. **Similar projects**: how open-source implementations on the same stack do it

### Extra Checklist for Test Scenarios

- [ ] project test conventions (framework? AAA? naming? coverage?)
- [ ] spec/contract docs → extract test scenarios
- [ ] existing test patterns (how is the same module tested? fixtures?)

### Output

Research summary: what was found, what is officially recommended, what the project conventions are.

---

## Phase 2: List Options (Standard tier; auto-execution forbidden)

### Format

```
Option A: [description] — pros / cons / blast radius
Option B: [description] — pros / cons / blast radius
Recommendation: [X], reason: [...]
```

### Code changes must list

- which files will be modified
- change summary (not the full diff)
- potential risks
- rollback method

### Approval

Use the `question` tool and wait for the user's confirmation.
- silence / timeout / "continue" ≠ approval
- requires the user's explicit affirmative response
- partial approval ("do A not B") → execute only the approved part

### Complex changes: team review

For complex changes (build/architecture changes, framework-level issues, impact on >5 files),
after user approval and before execution, add a team-review round:

```
1. team_create — create the review team (3 perspectives)
2. team_task_create — assign review tasks
3. team_send_message — send the plan draft
4. collect review feedback → revise the plan
5. user confirms the revision → enter Phase 3
```

CRITICAL items found in review must be fixed. LOW items may be logged as tech debt.

---

## Phase 3: Execute (user-approved)

- execute the option the user chose
- verify each step (never accumulate unverified changes)
- on failure: roll back + report, **do not auto-try option B** (return to Phase 1 for more research)

---

## Core Iron Rules

1. **No new information = no new attempt.** After a failure, retrying with tweaked parameters without new diagnostic info is brute-forcing.
2. **Test the right level.** Unit (logic) / integration (boundaries) / E2E (user flows) / acceptance (business) — each level catches different failures. If unit tests pass but the feature is broken, you tested the wrong level.
3. **Trivial code needs no tests.** One line, no branches, no side effects → YAGNI applies to tests too.

---

## Test Level Quick Reference

| Level | What it catches | Rust tooling |
|------|----------|-----------|
| unit | logic errors | `cargo test` |
| integration | interface mismatches | `cargo test --test '*'` |
| E2E | broken user paths | Playwright / Python WS scripts |
| acceptance | unmet requirements | manual verification / pixi run test-sfu |

---

## Forbidden List (verify Y before doing X)

| Want to do | Verify first |
|------|--------|
| change config/build | checked the official recommended config method |
| retry after a failure | you have new diagnostic information (not parameter-tweaking luck) |
| edit generated files/build artifacts | confirmed the change won't be overwritten at next build |
| delete what you don't understand | confirmed nothing else depends on it |
| edit a file directly | the user has approved the plan |
| change deps/pixi.toml in production | verified locally first |

---

## Relationship with Existing Skills

| Skill | Relationship |
|------|------|
| `lesson-memory` (C9) | think-before-act consults past lessons; lesson-memory records new ones |
| `lesson-review` | think-before-act is prevention; lesson-review is retrospection |
| `systematic-debugging` | think-before-act governs "check before fixing"; debugging governs "how to fix" |
| `verification-before-completion` | think-before-act governs "before acting"; verification governs "after finishing" |

## Multi-Ecosystem Examples

| Ecosystem | Typical "check before acting" scenario |
|------|----------------------|
| Rust | lifetime error → read the matching rustbook chapter first, don't spray `.clone()` |
| Python | dependency conflict → check pip/uv resolution rules first, don't `pip install` repeatedly |
| Go | interface not satisfied → read the godoc interface contract first, don't blindly change signatures |
| JS/TS | duplicate modules → check the bundler's official recommendation first, don't fiddle with aliases |
| Java | version conflict → run `mvn dependency:tree` first, don't exclude by hand |
| K8s | Pod crash → check official troubleshooting + `kubectl describe` first, don't edit yaml at random |

---

## Karpathy Four-Principles Mapping

This think-before-act skill aligns naturally with Andrej Karpathy's four engineering-philosophy principles:

| Karpathy principle | This skill's counterpart | Notes |
|--------------|-----------|------|
| **Think Before Coding** | Phase 1: research + 5 knowledge layers | check project history, official docs, community experience first — never act on guesses |
| **Simplicity First** | iron rule #3 + tiers (trivial/standard) | simple solutions first, YAGNI, no over-design |
| **Surgical Changes** | Phase 2: option listing + forbidden list | precise edits, state the blast radius, don't touch unrelated code |
| **Goal-Driven Execution** | Phase 3: execution + decision gates | execute the approved plan, verify each step, don't drift |

> **Community reference**: [multica-ai/andrej-karpathy-skills](https://github.com/multica-ai/andrej-karpathy-skills) — systematizes Karpathy's engineering philosophy into AI-executable skills; consistent with this skill's design philosophy.
