---
name: context-engineering
description: "Feed MediaServo agents the right context for a 7-crate polyglot workspace. Routes Rust rules to .rs tasks, C++/FFI rules to webrtc-sys/mediasoup boundaries, protocol rules to WS contract work. Prevents wrong-language lint violations and out-of-scope analysis. Use BEFORE any cross-crate or multi-language MediaServo task."
---

# context-engineering — Right Context, Right Language

> Route language-specific rules to the right crate. Don't apply C++ lints to Rust code. Don't audit Python scripts for Rust ownership semantics.

## When to Use

| Trigger | Reason |
|---------|--------|
| Cross-crate task touches 2+ languages | Avoids wrong-language rule pollution |
| New crate or module being created | Establishes correct context boundaries |
| FFI/SDK work (napi-rs, FlatBuffers) | Multi-language contract, not single-lang |
| Web/Admin Dashboard work | HTML/CSS/JS + Rust server, different rulesets |
| Code review spanning crates | Each crate needs its own rule set applied |

## MediaServo Crate → Language Mapping

| Crate / Layer | Primary | Ruleset | Verification |
|---------------|---------|---------|-------------|
| mediaservo-common | Rust | `rules/rust/` + `rules/common/` | `cargo clippy -- -D warnings` |
| mediaservo-media | Rust | `rules/rust/` + `rules/common/` | `cargo clippy -- -D warnings` |
| mediaservo-webrtc | Rust + C++ (FFI) | `rules/rust/` + `rules/common/` + C++ primer | `cargo clippy` for Rust; C++ reviewed manually |
| mediaservo-codec | Rust + C (GStreamer) | `rules/rust/` + FFI safety | `cargo clippy`; GStreamer pipeline tested via pixi |
| mediaservo-server | Rust + (HTML/JS admin) | `rules/rust/` + `rules/common/` | `cargo test -p mediaservo-server` |
| mediaservo-host | Rust (macOS native) | `rules/rust/` + `constraints.md` (macOS) | `cargo clippy -p mediaservo-host` |
| mediaservo-client | Rust (macOS native) | `rules/rust/` + `constraints.md` (macOS) | `cargo clippy -p mediaservo-client` |
| Admin Dashboard | HTML/CSS/JS | `rules/common/` (security, coding-style) | `npx tsc --noEmit` (if TS); manual JS review |

## Context Selection Protocol

### Phase 1: Identify the Scope

Before loading any rules, map the task to crates:

```
Task → crates affected → languages involved → applicable rulesets
```

**Example:**
```
Task: "Add new WS message type ProduceMedia"
→ mediaservo-common (protocol.rs) + mediaservo-server (handle_sfu_message)
→ Rust only → rules/rust/coding-style.md + rules/common/security.md
```

### Phase 2: Apply Rules Per Crate

**NEVER** batch-apply all rules to all crates. Each crate gets its relevant subset:

```
for each crate in scope:
  - Load rules/rust/* if crate uses Rust
  - Load rules/common/* (security, coding-style, testing) always
  - Skip rules/rust/hooks.md for non-Rust files
  - Apply constraints.md (macOS, Docker, platform limits) if relevant
```

### Phase 3: Verify Per Crate

| Scope | Verification |
|-------|-------------|
| Any .rs change | `cargo clippy -p <crate> -- -D warnings` |
| WebRTC FFI change | `cargo check -p mediaservo-webrtc --features backend-webrtc-rs` |
| SFU change | `cargo check -p mediaservo-server --features sfu-mediasoup` |
| Admin UI change | Visual QA via Playwright |
| Full workspace | `pixi run check` |

## Common Context Mistakes

| Anti-Pattern | Why Wrong | Fix |
|-------------|-----------|-----|
| Loading all 12 language rulesets for a Rust-only task | Wastes ~4K tokens, confuses agent | Route by crate (see table above) |
| Applying C++ ownership rules to Rust code | Wrong paradigm | Rust borrow checker is the authority |
| Skipping `constraints.md` for webrtc-sys work | Misses macOS -ObjC linker flag (PIT-01) | Always include constraints for FFI |
| Applying `rules/rust/hooks.md` to TypeScript files | Wrong hooks fire | Only load per-language hooks |
| Running `cargo test --workspace` for a single-crate change | Slow (~30s) | Use `cargo test -p <crate>` |
| Not loading `pitfalls.md` for SFU/mediasoup work | Misses PIT-06 through PIT-15 | Always load pitfalls for SFU tasks |

## Multi-Language Boundary Rules

### Rust → C/C++ (webrtc-sys, mediasoup-sys)
- Only `&[u8]` across FFI boundary (PIT-03, C5)
- `unsafe` blocks require `// SAFETY:` comment
- `cxx::SharedPtr` types need `impl_thread_safety!` macros

### Rust → Protocol (WebSocket JSON)
- All messages use `#[serde(tag = "type", rename_all = "snake_case")]`
- Browser clients must use snake_case NOT camelCase (PIT-06)
- Protocol changes → update `crates/mediaservo-common/src/protocol.rs`
- E2E tests must validate new message types

### Rust → Admin UI (HTML/JS)
- Embedded via `rust-embed`, compiled into server binary
- Feature-gated: `admin-dashboard`
- JS code goes in `crates/mediaservo-server/admin/`
- No shared type system — JSON envelope validated server-side

## Gate Checklist

Before submitting work touching multiple crates:

```
[ ] crate scope mapped correctly
[ ] only relevant rulesets loaded per crate
[ ] cargo check passes per crate (not just workspace)
[ ] clippy passes per crate with -D warnings
[ ] constraints.md checked for platform gotchas
[ ] pitfalls.md checked for known SFU/FFI patterns
[ ] protocol changes have backward-compat review
```

## Related Skills

| Skill | Relationship |
|-------|-------------|
| `think-before-act` | Context selection IS part of "check first, then act" |
| `api-interface-design` | Context-engineering informs protocol contracts |
| `test-harness` | Cross-crate testing needs correct per-crate context |
| `lesson-memory` (C9) | New multi-language pitfalls → write to `pitfalls.md` |

## Task → skill routing

The routing table lives in ONE place: `.agents/skills/TRIGGERS.md`
(and the skill-router intent classes). Do not maintain a copy here.

## Proactive suggestion mechanism

When the agent detects one of the keywords above but the matching skill is **not** loaded:
1. Brief prompt: "This task may need the `X` skill — want me to load it?"
2. **Not forced** — the user may skip
3. **No repeats** — suggest each skill at most once per session
4. Suggestion format:

```
💡 Suggestion: this task involves [scenario]; `skill(name="X")` can provide [value]. Need it?
```
