# AGENTS.md — MediaServo Project Knowledge Base

**Generated:** 2026-07-23

MediaServo — Real-Time Media Servo Platform. A standalone video/media service platform covering surveillance camera ingest and recording/playback (NVR), video conferencing, remote desktop, teleoperation, live streaming push/pull, and more. Current state: Phase 3 complete, 7-crate workspace, webrtc triple-backend (webrtc-rs video pipeline fully aligned), mediaservo-codec three backends (stub+FFmpeg+GStreamer), 343 commits on main.

## STRUCTURE

```
MediaServo/
├── .opencode/          # OpenCode configuration (plugins, MCP, LSP, instructions)
│   ├── opencode.json   # Main config: models, plugins, instructions, MCP, LSP
│   ├── agent-model-tiers.md  # Model tier system
│   ├── oh-my-openagent.jsonc  # OMO Agent configuration
│   ├── init-lsp-wrap.mjs      # LSP wrapper initialization
│   ├── package.json    # OpenCode plugin dependencies
│   └── .gitignore
├── .agents/
│   ├── rules/          # Coding rule files (12 languages + common + web)
│   ├── skills/         # Skills (book-to-skill/doc-audit/lesson-review/openspec-*/review-hardcode/test-harness/think-before-act)
│   └── memorys/        # Project memory files (decisions.md, status.md)
├── scripts/agents/      # Agent toolchain (generator, MCP launcher, guards)
├── crates/              # Rust workspace (11 member crates)
│   ├── mediaservo-host/   # Host app (headless, capture+encode+push)
│   ├── mediaservo-client/ # Remote app (pull+decode+control)
│   ├── mediaservo-server/ # Server app (signaling+relay+monitoring)
│   ├── mediaservo-common/   # Shared foundation: config, error, metrics, protocol, auth (72 tests)
│   ├── mediaservo-media/  # Media pipeline: pipeline, broadcast, engine, transform (107 tests)
│   ├── mediaservo-webrtc/ # WebRTC abstraction layer (stub/webrtc-rs/webrtc-sys, video pipeline parity)
│   └── mediaservo-codec/  # Codec: stub + FFmpeg + GStreamer three backends
├── docs/               # Design docs (architecture.md + modules/ + reference/ + research/)
├── README.md           # Project overview
├── LICENSE             # Apache 2.0
├── package.json        # Root package.json (codegraph dev dependencies)
├── bootstrap.sh / bootstrap.bat  # Dev environment bootstrap scripts
├── .rustfmt.toml       # Rust formatting config
├── clippy.toml         # Clippy lint config
├── deny.toml           # cargo-deny audit config
├── rust-toolchain.toml # Rust toolchain version
└── .gitignore
```

## WHERE TO LOOK

| Task | Location | Notes |
|------|----------|-------|
| Project overview | `README.md` | Feature scope, architecture positioning, tech stack |
| Agent config | `.opencode/opencode.json` | instructions, MCP, LSP |
| Model tiers | `.opencode/agent-model-tiers.md` | Five-tier model mapping, provider selection |
| Language rules | `.agents/rules/{lang}/` | Per-language rules |
| Common rules | `.agents/rules/common/` | Security, coding style, testing, Git workflow |
| Architecture doc | `docs/architecture.md` | Overall architecture design |
| Module docs | `docs/modules/` | Detailed per-domain designs (27 docs) |
| Project memory | `.agents/memorys/` | Decision records (decisions.md), status tracking (status.md) |
| Rust source | `crates/` | Seven crates: mediaservo-host/mediaservo-client/mediaservo-server/mediaservo-common/mediaservo-webrtc/mediaservo-media/mediaservo-codec

## SKILL DIRECTORY

When facing these scenarios, use the `skill` tool to load the corresponding skill:

| Scenario | Skill | When to trigger |
|------|------|---------|
| **Developing new features** | `incremental-implementation` | Multi-file changes, cross-crate modifications |
| **Fixing bugs** | `systematic-debugging` (built-in) | Test failures, runtime errors |
| **Looking up external library usage** | `source-driven-development` | Using mediasoup/webrtc-rs/GStreamer and other external libs |
| **Writing tests** | `test-driven-development` (built-in) | Before any new feature or bug fix |
| **Testing browser UI** | `browser-testing` | Admin Dashboard changes, Playwright |
| **Security audit** | `security-hardening` | Secret management, auth code, mediasoup transport security |
| **Code review** | `code-review` (rules) | After completing any code modification |
| **Performance analysis** | `performance-optimization` | WebRTC latency, React rendering, cargo bench |
| **Architecture/API design** | `api-interface-design` | Rust trait design, WS protocol, REST contracts |
| **Context switching** | `context-engineering` | Switching tasks across Rust/TS/DevOps |
| **CI/CD changes** | `ci-cd-automation` | Dockerfile, pixi.toml, GitHub Actions |
| **Code simplification** | `code-simplification` | Refactoring, reducing complexity |
| **Optimizing agent ecosystem** | `ecosystem-scan` | Auditing .agents/, finding new skills |
| **Session end** | `lesson-review` | Summarize lessons learned |
| **Before acting** | `think-before-act` | Before any non-trivial action (auto-triggers) |
| **Uncertain which skill** | `skill-router` | Ambiguous intent or needs combined recommendations |
| **Doc audit** | `doc-audit` | Check consistency across docs/decisions/agent ecosystem |
| **Batch changes** | `openspec-propose` | Propose a standardized change |
| **Implementing changes** | `openspec-apply-change` | Implement step by step per proposal |
| **Archiving changes** | `openspec-archive-change` | Archive after completion |
| **Exploring options** | `openspec-explore` | Explore ideas during research phase |
| **Test tooling** | `test-harness` | Generate test skeletons |
| **Hardcode scan** | `review-hardcode` | Check for hardcoded secrets/ports/URLs |
| **Docs to skill** | `book-to-skill` | Convert documents/books into AI skills |
| **Sync specs** | `openspec-sync-specs` | Sync delta specs into main specs |

## CODE MAP

_Project has entered the code implementation phase. Current state:_

| Module | Status | Notes |
|------|------|------|
| mediaservo-host | 🟡 skeleton done | Host app: capture, encode, push, signaling, config |
| mediaservo-client | 🟡 skeleton done | Remote app: pull, decode, render, control |
| mediaservo-server | 🟡 skeleton done | Server app: signaling relay, monitoring, session management |
│ mediaservo-common | ✅ implemented | Shared foundation: config, error, metrics, protocol, auth (72 tests)
│ mediaservo-media | ✅ implemented | Media pipeline: pipeline, broadcast, engine, transform (107 tests)
│ mediaservo-webrtc | ✅ triple-backend | WebRTC abstraction layer (stub/webrtc-rs/webrtc-sys), 118+ tests, webrtc-rs video pipeline fully aligned
│ mediaservo-codec | ✅ three backends | Codec: stub + FFmpeg (static) + GStreamer (dynamic, pixi)
| Phase 2+ crates | 🔲 planned | See `docs/architecture.md`

## CONVENTIONS

### Rust
- Edition 2024, `cargo clippy -- -D warnings`
- `thiserror` for libraries, `anyhow` for applications
- `&str` over `String`, `&[T]` over `Vec<T>`
- Every `unsafe` block must have a `// SAFETY:` comment
- Business-critical enums use exhaustive match, no wildcard `_`

### TypeScript
- Explicit type annotations on public APIs
- `interface` over `type` (for object shapes)
- `unknown` > `any`
- Zod for boundary-layer schema validation
- No `as any` / `@ts-ignore` / `console.log`

### C++
- RAII everywhere — no bare `new`/`delete`, use smart pointers
- Forbidden: `malloc`/`free`, C-style arrays, `strcpy`/`strcat`/`sprintf`
- Always: `std::array`/`std::vector`, `std::string`, initialize variables

### General
- Immutability first (never mutate, always create new copies)
- Small files > large files (200-400 lines typical, 800 max)
- Explicit error handling, no silently swallowed exceptions
- Boolean prefixes `is`/`has`/`should`/`can`
- Official usage first — when using libraries/projects/tools, follow official docs, official repo source, official examples, and community-recommended usage; no invented speculative usage or minimal interfaces (see conventions.md C18)

## ANTI-PATTERNS

- **`as any` / `@ts-ignore`** — never use, zero exceptions
- **`console.log`** — forbidden in production code
- **Silently swallowed exceptions** — `catch(e) {}` absolutely not allowed
- **Object mutation** — always return new objects, never modify in place
- **Hardcoded secrets/ports/URLs** — use environment variables, config, or a secret manager; temporary values must be marked TODO
- **Unnecessary file writes** — create doc files only when the user explicitly asks
- **Rust `unwrap()` in production** — use `?` with `thiserror`/`anyhow`
- **Invented speculative usage / minimal interfaces** — external dependencies (webrtc/mediasoup/GStreamer etc.) must use official usage and examples; no trimming to minimal interfaces or invented semantics for convenience (PIT-65 lesson, C18)

## NOTES

- **Phase 3 complete** — Docker/CI/DevContainer in place, SFU connect_transport implemented, 343 commits on main
- **Skeleton code created** — `crates/mediaservo-{host,client,server}` three crates with module skeletons
- **Ecosystem shared dependency** — third-party platforms consume this repo via statically linked Rust crates or napi bindings

<!-- BEGIN:agents-skills -->
## SKILL DIRECTORY (generated -- edit .agents/skills/*, not this block)

| Skill | Purpose |
|-------|---------|
| `adjudication-walkthrough` | one-at-a-time adjudication protocol. |
| `api-interface-design` | Contract-first design for MediaServo: Rust traits (Component/Plugin), WebSocket signaling protocol (SignalingMessage enum), and REST API bou |
| `book-to-skill` | Converts books and documents (PDF, EPUB, DOCX, HTML, Markdown, plain text, RTF, MOBI/AZW with Calibre) into structured agent skills, extract |
| `browser-testing` | Admin Dashboard testing via Playwright/DevTools MCP. |
| `ci-cd-automation` | MediaServo CI/CD pipeline management: Docker compose (server + SFU), GitHub Actions (fmt/check/clippy/test/mediasoup/benchmark), pixi tasks  |
| `code-simplification` | Reduce complexity in MediaServo Rust/TS code. |
| `context-engineering` | Feed MediaServo agents the right context for a 7-crate polyglot workspace. |
| `doc-audit` | MediaServo document & architecture audit. |
| `ecosystem-scan` | Audit the .agents/ system and scan the community ecosystem for skills/rules/MCP worth adopting. |
| `incremental-implementation` | Thin vertical slices across Rust+TS. |
| `lesson-review` | Batch session review: systematically extract lessons learned and write them to project memory. |
| `openspec-apply-change` | Implement tasks from an OpenSpec change. |
| `openspec-archive-change` | Archive a completed change in the experimental workflow. |
| `openspec-explore` | > |
| `openspec-propose` | > |
| `openspec-sync-specs` | Sync delta specs from a change to main specs. |
| `performance-optimization` | MediaServo performance profiling and optimization. |
| `review-hardcode` | Hardcoded secrets/ports/URLs scan. |
| `security-hardening` | MediaServo security audit and hardening. |
| `skill-router` | Analyze user intent and output a recommended skill list. |
| `source-driven-development` | Ground every external dependency decision in official docs. |
| `test-harness` | MediaServo multi-language automated test harness. |
| `think-before-act` | Meta-constraint: check before acting, list options for user approval, no brute-forcing. |
| `weaknet-testing` | Weak-network reproduction and measurement discipline for MediaServo (tc netem + the mediaservo-weaknet agent, optional web panel). |
<!-- END:agents-skills -->

<!-- BEGIN:agents-memory -->
## MEMORY DIGEST (generated -- titles only; full text in .agents/memorys/)

### decisions (last 25)
- D292: wire-compat-governance — four-plane compatibility governance (2026-10-14, user-ratified; codified as C50)
- D293: SDK artifact ABI compatibility baseline (doc 25; 2026-10-15, rulings #1-#11 user-ratified)
- D294: ABI recipe + decode-sink contract shipped in-tree (2026-10-15, consumer-side session; commits 721f47c..9c6745a)

### pitfalls (last 15)
- PIT-223: Raw-line text judges false-red on comment-stripping diffs (2026-10-14, G3 gate design)
- PIT-224: i18n round dropped a CSS comment opener — parser ate the whole grid rule (2026-10-14)
- PIT-225: K3 v1 explicit-dir gate deadbolt — monit/ps permanently unusable (2026-10-14)
- PIT-226: grep -P Unicode class silently matches NOTHING under non-UTF-8 locale (2026-10-15)
- PIT-227: conda-forge gcc's `-static-libstdc++` is a SILENT NO-OP for cdylib links (2026-10-15)
- PIT-228: stop self-proof brand-needle vacuity — legacy name list matches NOTHING on branded trees (2026-10-16)

### conventions (last 10)
- C50: Cross-end signaling & media-negotiation compatibility governance — four-plane model (2026-10-14, user-ratified)
- C51: commit message shape — subject <=50 chars, one fact per body line (2026-10-19 user ruling; mirror of main-repo C55, with this repo's 200-char-subject history as the negative exhibit)

### status (last 6)
- 2026-10-28: K3 closed — container face 13/13 ALL-GATES PASS
- 2026-10-30: P1-8/P1-9/P1-10 provenance + guards + self-tests
<!-- END:agents-memory -->
