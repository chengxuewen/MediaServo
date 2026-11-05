---
name: reuse-first
description: "Borrow before build: prove the existing supplier face cannot take it BEFORE writing any new mechanism (CMake target/function, script verb, wrapper, generator, config block). Four-step evidence chain — face inventory scan, NOTE reading, command-body verification, waiver declaration. Use when about to write a new X, or when a review finds a suspected duplicate. The charter (docs/governance/reuse-first-charter.md, ledger C60) is normative; this skill is its operational protocol."
---

# Reuse-First — Borrow Before Build

> The question is never "can I build it" — it is "can the existing face take it". Answer that in the open, with receipts, before the first line lands.

## Trigger Conditions

- About to write any NEW mechanism: a CMake target/function, a script verb, a wrapper, a generator, a config block
- A review or the user asks "doesn't this already exist?" / "这不已经有了吗"
- 用户说「复用优先 / 借先于造 / 别自造」 <!-- c49:trigger -->

## Why — this workspace's own scars (every rule has a receipt)

| Scar | What happened | The step that closes it |
|---|---|---|
| PIT-246 + C60 | claimed "no infrastructure exists" after one directory-scoped grep; the 82 `polyorch_*` function inventory was never listed | Step 1: existence-claims need the inventory attached |
| PIT-238 | one round, three traps: a display-only mount used as a build handle; a per-bin lane on a multi-bin crate (1 of 8 bins); hand-rolled cargo missing the face's PIT-14 env bridge | Steps 2+3: read the NOTE, verify the COMMAND body |
| C53 | the bindings callsite rode the shared AppTree choreography against the R6 NOTE → stillborn button (registered, never clickable) | Step 2 + first-click proof |
| deploy C+ round 2026-11-05 | a separate `-goods` helper target where one inline COMMAND sufficed (user: "ugly") — mechanism for mechanism's sake | Step 4: minimal form needs a waiver, not a new target |

## The Decision Ladder (charter §1.2)

1. **Supplier face** (PolyOrch faces / MediaServo cmake faces / mediaservo.sh verbs) — parameterize and reuse
2. **Consumer-repo common layer** — share inside the repo
3. **Minimal self-build + waiver declaration** — last resort

Two-branch rule (charter §1.1): a supplier face already exists → **MUST use it, zero elasticity**. No abstraction exists yet → the 2nd occurrence may stay duplicated but must be registered; the 3rd forces abstraction and folds 1-2 back in (Rule of Three).

## The Four-Step Evidence Chain

Every step leaves evidence **in the conversation** — reading a file is not judging it; finding a candidate is not proving it fits (the agent-utils-reuse lesson).

### Step 1 — Face inventory scan

```bash
grep -rh '^function(polyorch' 3rdparty/MediaServo/PolyOrch/cmake/*.cmake | wc -l   # 82 as of 2026-11-05
grep -rh '^function(_\?mediaservo' 3rdparty/MediaServo/cmake/*.cmake              # consumer faces
grep -n '^  [a-z:|]*build\|^  [a-z:|]*deploy' 3rdparty/MediaServo/mediaservo.sh | head   # CLI verbs
```

A failed search reports **"not found"** — never **"does not exist"**. The claim "no infrastructure exists" is admissible only with the inventory and the test-suite listing attached in the same message (PIT-246).

### Step 2 — NOTE reading

The selected shared face's header NOTE / design note must be read and quoted before riding it (C53 — the R6 NOTE said "bindings does NOT ride this shape" and the callsite missed it). A CLI verb with the same semantics → **mirror its invocation shape** (the C+ fresh-goods leg mirrors `build host` cargo-for-cargo — that parity is what made it reviewable).

### Step 3 — Command-body verification

Reusing a target/handle only after its COMMAND body is verified — names lie:

```bash
grep -c cargo <build-tree>/CMakeFiles/<handle>.dir/build.make   # 0 = display-only shell (PIT-238)
grep -c '\[\[bin\]\]' <crate>/Cargo.toml                        # multi-bin crate: one lane builds 1 of N
```

### Step 4 — Waiver declaration (self-build only)

When the ladder truly bottoms out, the reply carries one line: **why no existing face takes it** + the reclaim path ("when X lands, fold this back"). Going off the paved road is allowed — silently is not.

## Closure — registered ≠ working

A newly registered button/target/verb is exercised once before the task is called done: full run `[100%]` and landed artifacts non-empty (C53 — the bindings button was stillborn for weeks because nobody clicked it). Registration completes at first proof, not at first commit.

## Anti-Patterns

| Anti-pattern | Correct behavior |
|---|---|
| "I searched one directory, it doesn't exist" — then hand-writes a replacement | Inventory + test-suite listing attached, or say only "not found" |
| Picking a target by name shape (`-build` suffix) | Verify the COMMAND body in the generated build.make |
| Hand-rolling a cargo/env invocation next to a face that owns one | Mirror the face's own bridge (PIT-14) — copy from its generated build.make |
| New helper target where one inline COMMAND fits | Prefer the sequence-flat form; waiver + reclaim path if truly separate |
| Registering a face and moving on | First-click proof before "done" |

## Relations

- `think-before-act` — the approval gate (whether to act); this skill sits **before** it (build vs borrow)
- `source-driven-development` — external/upstream docs; this skill is the in-repo counterpart
- `code-simplification` / ponytail — post-hoc pruning; this skill is pre-hoc prevention
- **Normative source**: `docs/governance/reuse-first-charter.md` (ratified 2026-11-05) · ledger `C60` · consumer projection `docs/governance/reuse-first-consumer.md` (planned, PLAN B3) · CI lint = conditional (charter §1.6: graduates only after 2 more human-side misses)
