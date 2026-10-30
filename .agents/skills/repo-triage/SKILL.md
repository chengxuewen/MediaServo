---
name: repo-triage
description: "Triage an ambiguous MediaServo task into the right layer and files before editing: Rust crates, C ABI/FFI, mediasoup, Admin Dashboard, agent tooling, book/docs, or weaknet. Use when the ask is broad or the first file is unclear."
---

# Repository Triage

> Find the layer before the file. A wrong first edit costs more than a slow map.

## Trigger Conditions

- The ask is broad: "fix the stream", "make the dashboard work", "improve the server".
- You cannot name the owning crate, package, or script.
- A change may cross Rust, C++, TypeScript, and docs boundaries.
- 用户说“这该放哪层 / 影响哪些 crate / 先看哪里 / 帮我拆一下” <!-- c49:trigger -->

## Do Not Use

- The user asks which *skill* to use: use `skill-router`.
- The task is a pure design exploration: use `openspec-explore`.
- The owning file is already known and the change is small: edit it.

## Layer Map

| Layer | Roots | Typical work |
|---|---|---|
| Rust crates | `crates/` | signaling, media pipeline, config, admin API |
| C ABI / FFI | `bindings/`, `webrtc-sys` | ABI surface, ownership, build flags |
| SFU / mediasoup | server + `3rdparty` worker | transports, producers/consumers, RTP/ICE |
| Admin Dashboard | `www/` | React UI, WS client, playback, metrics |
| Agent tooling | `scripts/agents/`, `.agents/` | generators, gates, skills, MCP |
| Docs / book | `docs/`, `book/` | architecture, plans, book chapters |
| Weak network | `scripts/weaknet*` | profiles, scenarios, impairment tests |

## Triage Steps

1. **Restate the ask as an observable outcome.** One sentence, with the surface (API, UI, stream, gate).
2. **Name the suspected layer and one alternative.** If you cannot, read the entry points first.
3. **Locate the owning files.**
   ```bash
   rg "<symbol or phrase>" crates www scripts docs
   ```
   Prefer codegraph/serena for symbol impact, `rg` for text.
4. **Check the contract boundary.** Is the change internal, across a crate boundary, across FFI, or on the wire?
5. **List affected tests and gates.** A change without a gate is not triaged.
6. **Pick the skill for the work.** The layer map does not choose the workflow; name it explicitly.
7. **Propose the smallest slice.** One crate/package, one contract, one verification.

## Output Format

```text
Ask: <observable outcome>
Layer: <primary> / <alternative>
Files: <owning files>
Contract: <internal | crate | FFI | wire | UI>
Tests/gates: <names>
Skill: <skill>
First slice: <one change + verification>
Open question: <only if it blocks the slice>
```

## Heuristics

- A wire change is not a UI change; update the protocol contract first.
- An FFI change needs an ABI check and a C++ build, not only `cargo test`.
- A dashboard symptom may be a server contract bug; check the WS payload before the React state.
- An agent/gate failure may be generated drift; run the generator check before editing generated files.
- A docs task may still need a code anchor; verify the path exists.

## Anti-Patterns

| Smell | Fix |
|---|---|
| Editing the first file that mentions the symptom | Map the data path first. |
| Treating `www/` as the whole product | The server and SFU are separate layers. |
| Cross-layer change in one slice | Split by contract; verify each layer. |
| "I'll find the tests later" | List the gate before editing. |

## Boundaries

- `skill-router`: chooses a skill; this skill chooses the layer and files.
- `context-engineering`: routes rules by language; this skill routes the task by layer.
- `openspec-explore`: explores requirements; this skill scopes an implementation.
