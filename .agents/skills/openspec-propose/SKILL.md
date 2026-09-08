---
name: openspec-propose
description: >-
  Propose a new change for MediaServo with structured artifacts (proposal, design,
  tasks). Generates docs/plans/<name>/proposal.md + design.md + tasks.md.
  Use when the user describes what they want to build and needs a complete proposal ready for implementation.
license: MIT
compatibility: Designed for Claude Code, GitHub Copilot, and similar agents.
disable-model-invocation: false
metadata:
  author: openspec
  version: "2.0"
  category: workflow
  project: MediaServo
---

# OpenSpec Propose — MediaServo

Create a structured change proposal for MediaServo. Produce three artifacts that together answer
"what are we building, how does it fit, and what's the plan?"

When ready to implement, follow with `/openspec-apply`.

---

**Input**: The user describes a feature, fix, or refactor. Do not start without a feature description.

---

## Steps

### 1. Confirm the change name

Ask: "What should we call this change? (kebab-case, e.g. `add-signal-monitoring`)"

**DO NOT auto-generate without asking.** Validate: lowercase letters, digits, hyphens only.

### 2. Gather context

Before writing any artifact, understand the existing surface area:

#### a. Read relevant specs

Search `openspec/specs/` for specs related to the change by module prefix:
`hal-type-system`, `hal-qos`, `hal-protocol`, `hal-config-barrier`.
Read every spec whose module overlaps. Note if no relevant spec exists.

#### b. Read project memory

- `.agents/memorys/status.md` — current phase, module status, known gaps
- `.agents/memorys/decisions.md` — D1-D50 architecture decisions
- `.agents/memorys/pitfalls.md` — known sharp edges (MODACS removal, DDS traps, CI gotchas)
- `.agents/memorys/conventions.md` — naming, immutability, TS/Rust/HAL conventions

#### c. Assess affected layers

| Layer | Location | When affected |
|-------|----------|---------------|
| **HAL Core** | `crates/mediaservo-common/` | New traits, types, primitives, error types |
| **mediaservo** | `crates/mediaservo/` | Transport/Discovery implementation changes |
| **FlatBuffers** | `crates/hal-flatbuffers/` + `.fbs` schemas | New/changed cross-language types |
| **Studio** | `apps/studio/` | Tauri+React+TypeScript frontend changes (D21) |

#### d. Assess affected transports

| Transport | Phase | Purpose |
|-----------|-------|---------|
| **mediaservo** | Phase 1 | In-process HAL Transport/Discovery (D11) |
| **amw_zenoh** | Phase 2 | Network transport via Zenoh (future) |

Most Phase 1 changes target `mediaservo` only.

### 3. Create the proposal directory

```bash
mkdir -p docs/plans/<change-name>
```

### 4. Write proposal.md

Create `docs/plans/<change-name>/proposal.md` with these sections:
- **What** — 2-4 sentences, specific
- **Why** — problem, use case, gap
- **Scope** — in scope / out of scope
- **Layers Affected** — checklist: HAL Core / mediaservo / FlatBuffers / Studio
- **Transports Affected** — mediaservo: yes/no/partial, amw_zenoh: yes/no/partial
- **Existing Specs** — list `openspec/specs/<name>.md` with one-line description each
- **New Specs Needed** — list or "None"
- **Risks** — 2-4 bullet points (thread safety, FFI, build, interop)
- **Success Criteria** — how we know it's done
- **References** — links to issues, design docs, external references

### 5. Write design.md

Create `docs/plans/<change-name>/design.md` with these sections:
- **Architecture** — ASCII diagram or text description showing modules, data flow, ownership
- **Files to Touch** — Create / Modify / Delete sub-tables with file paths and purpose
- **Data Flow** — critical path from entry to exit (Signal: write→store→callback; RPC: invoke→dispatch→result)
- **Integration Points** — HAL trait boundary, amw boundary, FlatBuffers boundary, Studio boundary
- **Rust/HAL Specifics** — new traits/structs, Signal/StreamChannel wiring (D10), thread safety (Send+Sync, Config Barrier D17), YAML→FlatBuffers config (D24)
- **Error Handling** — HAL 5-layer error model (D46): type/transport/resource/discovery/scheduling
- **Testing Strategy** — checklist: Rust unit, integration, FlatBuffers round-trip, mediaservo E2E, qa-fast gate (`./scripts/qa/qa-fast.sh`)
- **Dependencies** — new cargo deps, FlatBuffers schema changes (or "None")

### 6. Write tasks.md

Create `docs/plans/<change-name>/tasks.md`. Tasks must be **atomic, ordered, independently testable** — each produces one verifiable result. Structure in phases:

```markdown
# Tasks: <Change-Name>

## Phase 1: Foundation

- [ ] **Add `<trait/struct>` to HAL Core**
  - File: `crates/mediaservo-common/src/<path>/<file>.rs`
  - Verify: `cargo check -p mediaservo-common`

- [ ] **Implement for mediaservo**
  - File: `crates/mediaservo/src/<file>.rs`
  - Verify: `cargo check -p mediaservo`

## Phase 2: Transport & Bindings

- [ ] **Update FlatBuffers schema** (if needed)
  - File: `crates/hal-flatbuffers/<file>.fbs`
  - Verify: `cargo build -p hal-flatbuffers`

## Phase 3: Tests

- [ ] **Add Rust unit tests** (AAA pattern, D33)
  - File: same as implementation
  - Verify: `cargo test -p mediaservo-common`

- [ ] **Add integration tests**
  - File: `tests/<name>_test.rs`
  - Verify: `cargo test --test <name>_test`

- [ ] **Run qa-fast gate**
  - Verify: `./scripts/qa/qa-fast.sh` (5 gates: test/clippy/fmt/deny/unwrap)

## Phase 4: Documentation & Cleanup

- [ ] **Write/update spec file**
  - File: `openspec/specs/<name>.md` (SDD format: ID→precondition→operation→expected→edge cases)

- [ ] **Update project memory** (after implementation)
  - `.agents/memorys/status.md`, `decisions.md`, `pitfalls.md` as applicable
```

Adjust phases to fit the change: single-file fix → 3 tasks; multi-module feature → 15+ tasks across 5 phases.

### 7. Present and iterate

Display summary — change name, artifact list, line counts. Let user request changes, iterate until approved.

---

## File Path Conventions

| Purpose | Path |
|---------|------|
| HAL Core | `crates/mediaservo-common/src/` |
| mediaservo | `crates/mediaservo/src/` |
| FlatBuffers schemas | `crates/hal-flatbuffers/*.fbs` |
| Studio | `apps/studio/src/` |
| Specs | `openspec/specs/` |
| Plans | `docs/plans/<change-name>/` |
| Integration tests | `tests/` |

---

## MediaServo-Specific Guidelines

### Crate references

| Crate | Path | Type |
|---------|------|------|
| HAL Core | `crates/mediaservo-common/` | Rust (traits, types, primitives) |
| amw-inproc | `crates/mediaservo/` | Rust (HAL Transport/Discovery in-process) |
| HAL FlatBuffers | `crates/hal-flatbuffers/` | Rust + .fbs schemas |
| Studio | `apps/studio/` | Tauri + React + TypeScript (D21) |

### Build commands

```bash
cargo build                                    # Full build
cargo build --package mediaservo-common --package mediaservo  # HAL-only
cargo test                                     # Debug build + tests
./scripts/qa/qa-fast.sh                        # QA fast gate (5 checks)
```

### Rust conventions

- Rust stable toolchain, ownership, borrowing, traits
- HAL traits + FlatBuffers for cross-language interop (D19)
- mediaservo for Phase 1 transport (D11)
- Multi-language via FlatBuffers schema (D19)
- Thread safety: Config Barrier (D17) for RT config changes
- Config via YAML → FlatBuffers (D24)

---

## Guardrails

- **Always ask for the change name** — do not generate one without user confirmation
- **Read specs before proposing** — ignoring existing SDD contracts is waste
- **Layer assessment must be explicit** — "maybe affects FlatBuffers" is not acceptable; decide and document
- **Transport assessment must be explicit** — mediaservo-only? amw_zenoh? Both? Document the split
- **Tasks must be atomic** — each task produces one verifiable result (compiling code, passing tests)
- Always reference actual MediaServo file paths and crate names
- If context is critically unclear, ask — but prefer reasonable decisions to keep momentum
- If a proposal with that name already exists, ask to continue or create new
- Do NOT propose changes to `version.txt` — versioning is user-managed
- Do NOT propose changes to external dependencies — separate repositories
- Verify each artifact file exists after writing before proceeding
- Do NOT reference MODACS — fully de-MODACS-ized project (D3)
