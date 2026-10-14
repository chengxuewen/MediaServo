---
name: code-simplification
description: "Reduce complexity in MediaServo Rust/TS code. Chesterton's Fence analysis, Rule of 500 enforcement, dead code elimination, borrow checker simplification patterns. Complements ponytail for Rust-specific over-engineering. Use after ponytail-audit, before PRs, or when diagnosing complexity smells."
---

# code-simplification — Complexity Reduction

> Chesterton's Fence + Rule of 500 + Rust borrow checker patterns.
> Delete only what should be deleted. Never delete what you don't understand.

## Trigger Conditions

- User says "too complex" / "simplify" / "refactor" / "dead code"
- After ponytail-audit finds removable items
- File exceeds 500 lines (`wc -l` > 500)
- `cargo clippy -- -W clippy::cognitive_complexity` reports high complexity
- Function exceeds 50 lines (`wc -L` > 50 per function)

## Chesterton's Fence Flow

```
find complexity → check why → reasonable reason? → YES: keep, add comment / NO: delete
```

### Tools for Checking the Reason

| Source | Command |
|------|------|
| git blame | `git blame -L <line>,<line> <file>` |
| commit messages | `git log --all -S "<code>" --oneline` |
| issues/PRs | `gh search issues "<keyword>" --repo $(git remote get-url origin)` |
| ADR decisions | `.agents/memorys/decisions.md` |
| community reference | `grep_app_searchGitHub` |

### Fence Comments

When keeping complexity that has a reason, mark it with a comment:

```rust
// chesterton: BufferPool uses unsafe zero-copy because the &[u8] output
// of the GStreamer appsink must be forwarded directly to the WebRTC
// TrackLocal to avoid memcpy latency.
// Tried a safe Vec<u8> copy → frame drops at 1080p60 went from 0% → 1.2%.
// git: a1b2c3d "zero-copy buffer pool for 60fps stability"
unsafe {
    pool.copy_to_track(track, data);
}
```

## Rule of 500

| Check | Command | Threshold | Action |
|--------|------|------|------|
| file lines | `wc -l <file>` | >500 | split into domain modules |
| function lines | `grep -n '^fn ' <file>`, estimate block | >50 | extract helper functions |
| parameter count | `grep -c ','` per fn sig | >5 | merge params into a struct |
| nesting depth | eyeball the indent | >4 | early return / `?` propagation |
| trait impl count | `grep -c 'impl.*for' <file>` | >3 | move to impl submodules |
| match arm count | `grep -c '=>'` per match block | >10 | `enum_dispatch` / strategy pattern |
| pub exposure count | `grep -c 'pub' <file>` | >20 | narrow visibility, finer-grained modules |

### Execution

```
# 1. scan files over the limit
find crates/ -name '*.rs' -exec wc -l {} + | sort -rn | head -20

# 2. apply Chesterton's Fence to each over-limit file
# 3. split, extract, delete
# 4. verify: cargo clippy -- -D warnings && cargo test -p <crate>
```

## MediaServo-Specific Patterns

### Borrow-Checker Simplification

```rust
// BEFORE: unnecessary Arc<Mutex<>> nesting
let data: Arc<Mutex<Vec<Arc<Mutex<Option<Box<dyn Trait>>>>>>> = ...;

// AFTER: single ownership, borrow on demand
let mut data: Vec<Box<dyn Trait>> = Vec::new();
// when sharing is needed: let data = Arc::new(RefCell::new(data));
// ponytail: RefCell suffices single-threaded; switch to RwLock for real multi-threading
```

### WebRTC Backend Abstraction Layer

```rust
// BEFORE: repeated glue code per backend
#[cfg(feature = "backend-webrtc-rs")]
fn create_pc() -> RTCPeerConnection { ... }
#[cfg(feature = "backend-webrtc-sys")]
fn create_pc() -> RTCPeerConnection { ... }
#[cfg(feature = "backend-stub")]
fn create_pc() -> RTCPeerConnection { ... }

// AFTER: extract the common part to a shared module; backends implement only the differences
// chesterton: the 3-backend cfg duplication is the architectural cost of decision D15 —
// it can't be eliminated, only minimized inside the backend/ submodule.
```

### Config Path Simplification

```rust
// BEFORE: deeply nested config
let port = config.server.webrtc.ice.transport.port.unwrap_or(9800);

// AFTER: flatten + sane default
let port = env::var("OMSP_ICE_PORT").ok()
    .and_then(|v| v.parse().ok())
    .unwrap_or(9800);
```

## Relationship with ponytail

| ponytail | code-simplification |
|----------|---------------------|
| global audit → ranked list | deep per-file/module pass |
| "can this be deleted?" | "why does this exist?" |
| deletion decisions | split + refactor decisions |
| one finding per line | structural refactoring |

**Workflow**: `/ponytail-audit` → rank → `/code-simplification` one by one

## Verification Gates

```bash
# 1. compiles
cargo check --workspace --all-features

# 2. clippy zero warnings
cargo clippy --workspace --all-features -- -D warnings

# 3. all tests pass
cargo test --workspace

# 4. file line counts went down
git diff --stat | grep -E '\+[0-9]+.*-' | tail -5

# 5. no new pub API (simplification must not enlarge the public surface)
# ponytail: only counts pub fn, not enforced — refactors may naturally add some
```

## Anti-Pattern Detection

| Anti-pattern | Detection command | Automated fix |
|--------|---------|-----------|
| uncommented `unwrap()` | `grep -r 'unwrap()' crates/ --include='*.rs' \| grep -v '//.*unwrap'` | replace with `?` or `.context()` |
| `clone()` to appease the borrow checker | `grep -r '\.clone()' crates/ --include='*.rs'` | analyze the borrow chain |
| `Box<dyn Trait>` where generics would do | clippy `boxed_local` lint | change to `impl Trait` |
| feature-gated dead code | `cargo deadlinks` / `cargo udeps` | remove the dead feature |
| duplicated impl blocks | eyeball | merge or extract a macro |
| `as` casts | `grep -r ' as ' crates/ --include='*.rs'` | prefer `.into()` / `.try_into()` |

## Output Format

```
## Simplification Report

File: crates/mediaservo-webrtc/src/backend/mod.rs
Before: 487 lines → After: 312 lines (-36%)

### Removed
- [lines 45-78] unused trait `LegacySdpParser` → deleted (git blame: introduced by D78, obsoleted by D112)
- [lines 203-206] dead code `#[cfg(all(nonexistent, feature = "..."))]`

### Split
- [mod.rs] → `backend/webrtc_rs.rs`, `backend/webrtc_sys.rs`, `backend/shared.rs`

### Kept (Chesterton's Fence)
- [lines 180-195] `#[cfg]` 3-backend duplication — architectural cost of D15, cannot be removed
- [line 300] `unsafe` buffer pool — performance constraint, see comment

### Verified
✅ cargo clippy -- -D warnings
✅ cargo test -p mediaservo-webrtc
```

## Forbidden

- deleting code you don't understand
- merging unrelated modules
- simplifying an `unsafe` block without understanding its memory semantics
- removing "looks unused" code without git blame evidence
- deleting feature-gated code without first checking the CI variants
