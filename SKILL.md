# MediaServo Skills Registry

## Superpowers

General skills loaded via the `superpowers` plugin; applicable to all projects.

## Project Skills

MediaServo project-specific skills, located in `.agents/skills/`:

| Skill | Type | Description |
|-------|------|------|
| `book-to-skill` | Tooling | Convert books/documents into skill files |
| `doc-audit` | Tooling | Documentation audit and consistency checks |
| `openspec-propose` | Spec workflow | Create OpenSpec change proposals |
| `openspec-apply-change` | Spec workflow | Apply OpenSpec changes |
| `openspec-explore` | Spec workflow | Explore existing specs and changes |
| `openspec-archive-change` | Spec workflow | Archive completed changes |
| `openspec-sync-specs` | Spec workflow | Sync specs and archive changes |
| `test-harness` | Testing | Multi-language test framework (Rust/TS/Python/C++/C) |

## Usage

Skills are auto-activated by AI agents based on task context. No manual invocation needed.

<!-- BEGIN:skill-registry -->
## SKILL DIRECTORY (generated -- edit .agents/skills/*, not this block)

| Skill | Purpose |
|-------|---------|
| `adjudication-walkthrough` | one-at-a-time adjudication protocol. |
| `api-interface-design` | Contract-first design for MediaServo: Rust traits (Component/Plugin), WebSocket signaling protocol (SignalingMessage enum), and REST API bou |
| `book-to-skill` | Converts books and documents (PDF, EPUB, DOCX, HTML, Markdown, plain text, RTF, MOBI/AZW with Calibre) into structured agent skills, extract |
| `browser-testing` | Admin Dashboard testing via Playwright/DevTools MCP. |
| `ci-cd-automation` | MediaServo CI/CD pipeline management: Docker compose (server + SFU), GitHub Actions (fmt/check/clippy/test/mediasoup/benchmark), pixi tasks  |
| `code-review-and-quality` | Review MediaServo changes on five axes (correctness, security, maintainability, tests, docs/ops), label findings Nit/Optional/FYI/Blocking,  |
| `code-simplification` | Reduce complexity in MediaServo Rust/TS code. |
| `context-engineering` | Feed MediaServo agents the right context for a 7-crate polyglot workspace. |
| `debugging-and-error-recovery` | Systematic debugging for MediaServo: reproduce, capture evidence, localize the layer, narrow with bisect or a minimal input, fix minimally,  |
| `deprecation-and-migration` | Deprecate and migrate MediaServo surfaces safely: mark the old surface, publish a migration path, keep an alias until consumers move, then r |
| `doc-audit` | MediaServo document & architecture audit. |
| `documentation-and-adrs` | Record MediaServo decisions and keep docs consistent: D-series entries, architecture and protocol updates, book chapters, changelog, and lin |
| `ecosystem-scan` | Audit the .agents/ system and scan the community ecosystem for skills/rules/MCP worth adopting. |
| `incremental-implementation` | Thin vertical slices across Rust+TS. |
| `lesson-review` | Batch session review: systematically extract lessons learned and write them to project memory. |
| `observability-and-instrumentation` | Instrument MediaServo with useful logs, metrics, and traces: structured fields, RED metrics, spans across signaling/media, symptom-based ale |
| `openspec-apply-change` | Implement tasks from an OpenSpec change. |
| `openspec-archive-change` | Archive a completed change in the experimental workflow. |
| `openspec-explore` | > |
| `openspec-propose` | > |
| `openspec-sync-specs` | Sync delta specs from a change to main specs. |
| `performance-optimization` | MediaServo performance profiling and optimization. |
| `repo-triage` | Triage an ambiguous MediaServo task into the right layer and files before editing: Rust crates, C ABI/FFI, mediasoup, Admin Dashboard, agent |
| `review-hardcode` | Hardcoded secrets/ports/URLs scan. |
| `security-hardening` | MediaServo security audit and hardening. |
| `shipping-and-launch` | MediaServo release checklist: full gates, version/tag policy, changelog decision, artifact and submodule pin verification, staged rollout, a |
| `skill-router` | Analyze user intent and output a recommended skill list. |
| `source-driven-development` | Ground every external dependency decision in official docs. |
| `test-driven-development` | Red-green-refactor for MediaServo Rust and protocol work: write the failing test first, make it pass minimally, then refactor under the gate |
| `test-harness` | MediaServo multi-language automated test harness. |
| `think-before-act` | Meta-constraint: check before acting, list options for user approval, no brute-forcing. |
| `weaknet-testing` | Weak-network reproduction and measurement discipline for MediaServo (tc netem + the mediaservo-weaknet agent, optional web panel). |
<!-- END:skill-registry -->
