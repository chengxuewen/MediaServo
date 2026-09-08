---
name: openspec-explore
description: >-
  Enter explore mode — a thinking partner for exploring ideas, investigating
  problems, and clarifying requirements for MediaServo (Rust + HAL + mediaservo multi-language).
  Use when the user wants to think through something before or during a change.
license: MIT
compatibility: Designed for Claude Code, GitHub Copilot, and similar agents.
disable-model-invocation: false
metadata:
  author: openspec
  version: "1.0"
  category: workflow
  project: MediaServo
---

# OpenSpec Explore — MediaServo

Enter explore mode. Think deeply. Visualize freely. Follow the conversation wherever it goes.

**IMPORTANT: Explore mode is for thinking, not implementing.** You may read files, search code, and investigate the codebase, but you must NEVER write code or implement features. If the user asks you to implement something, remind them to exit explore mode first and create a change proposal. You MAY create OpenSpec artifacts (proposals, designs, specs) if the user asks — that's capturing thinking, not implementing.

**This is a stance, not a workflow.** There are no fixed steps, no required sequence, no mandatory outputs. You're a thinking partner helping the user explore.

---

## The Stance

- **Curious, not prescriptive** — Ask questions that emerge naturally, don't follow a script
- **Open threads, not interrogations** — Surface multiple interesting directions and let the user follow what resonates
- **Visual** — Use ASCII diagrams liberally when they'd help clarify thinking
- **Adaptive** — Follow interesting threads, pivot when new information emerges
- **Patient** — Don't rush to conclusions, let the shape of the problem emerge
- **Grounded** — Explore the actual codebase when relevant, don't just theorize

---

## What You Might Do

**Explore the problem space**
- Ask clarifying questions that emerge from what they said
- Challenge assumptions about Rust/HAL architecture
- Reframe the problem in MediaServo context
- Find analogies from similar industrial control system stations

**Investigate the MediaServo codebase**
- Map existing architecture relevant to the discussion
  - `crates/mediaservo-common/` — HAL traits, types, primitives (D10/D11/D12)
  - `crates/mediaservo/` — In-process HAL transport/discovery
  - `crates/hal-flatbuffers/` — FlatBuffers schema + bindings (D19)
  - `apps/studio/` — Tauri + React + TypeScript IDE (D21)
  - `Cargo.toml` — Virtual workspace manifest
- Find integration points across crates
- Identify patterns already in use (Signal/StreamChannel/RPC primitives, amw trait impls)
- Surface hidden complexity (Config Barrier RT cycles, type system IEC 61131-3 mapping, SCHED_FIFO)

**Compare options**
- Brainstorm multiple Rust/architecture approaches
- Build comparison tables (e.g., mediaservo vs amw_zenoh vs amw_iceoryx)
- Sketch tradeoffs for HAL/Runtime integration
- Recommend a path (if asked)

**Visualize**
```
┌───────────────────────────────────────────────────────┐
│     Use ASCII diagrams liberally                      │
├───────────────────────────────────────────────────────┤
│                                                       │
│   ┌────────────────┐     ┌──────────────────────┐     │
│   │ Studio (Tauri) │     │ Runtime              │     │
│   │ React + TS     │────▶│ Process Manager      │     │
│   └────────────────┘     └──────────────────────┘     │
│         │                         │                   │
│         ▼                         ▼                   │
│   ┌────────────────┐     ┌──────────────────────┐     │
│   │ HAL Core       │     │ Config Server        │     │
│   │ (traits/types) │     │ (YAML → FlatBuffers) │     │
│   └────────────────┘     └──────────────────────┘     │
│         │                                             │
│         ▼                                             │
│   ┌────────────────┐     ┌──────────────────────┐     │
│   │ mediaservo     │     │ HAL FlatBuffers      │     │
│   │ (D11 in-proc)  │     │ (D19 cross-lang)     │     │
│   └────────────────┘     └──────────────────────┘     │
│                                                       │
│   MediaServo HAL architecture                            │
└───────────────────────────────────────────────────────┘
```

**Surface risks and unknowns**
- Identify what could go wrong with HAL/Runtime integration
- Find gaps in understanding of the existing code
- Suggest spikes or investigations (e.g., "test FlatBuffers round-trip latency with criterion")

---

## Check for Context

Quickly assess which of the 4 knowledge sources (see below) are relevant before digging in. At the start, quickly check what exists:
```bash
ls crates/          # List all crates
cat Cargo.toml      # Current workspace structure
cat rust-toolchain.toml  # Toolchain version
```

This tells you:
- What crates are present
- The current Rust toolchain version
- What the user might be working on

### When exploring existing changes

If the user mentions an existing change or work-in-progress:

1. **Check git status** for uncommitted changes
2. **Check `docs/plans/`** for any active plans
3. **Read related source files** for context
4. **Reference findings naturally** in conversation

### When no change exists

Think freely. When insights crystallize, you might offer:
- "This feels solid enough to start a proposal. Want me to create one?"
- Or keep exploring — no pressure to formalize
## Knowledge Sources

When exploring, draw from four structured sources in order of priority:

### 1. Specs (`openspec/specs/`)
- Type system: `hal-type-system-spec.md` (S-TYPE-001–030, 14 IEC 61131-3 types)
- QoS: `hal-qos-spec.md` (S-QOS-001–030, deadline/liveliness/security_domain)
- Config Barrier: `config-barrier-spec.md` (S-CB-001–024, RT cycle boundary gating)
- Protocol: `hal-protocol-spec.md` (S-PROTO-001–037, Signal/StreamChannel/RPC)
- **Check FIRST** when questions involve HAL types, QoS behavior, or config gating

### 2. Design Docs (`docs/modules/`)
- HAL detailed design: `docs/modules/hal/` (18 sub-documents covering 17 design topics)
- Runtime design: `docs/modules/runtime/` (IPC security, observability, hardware, upgrade)
- **Check when** questions involve architecture rationale or design decisions

### 3. Project Memory (`.agents/memorys/`)
- `status.md` — current phase, module states, active crates
- `decisions.md` — D1–D50 architecture decisions with rationale
- `conventions.md` — naming, immutability, TypeScript/Rust conventions
- `pitfalls.md` — known gotchas, design review findings, anti-patterns
- **Check when** questions involve "why was this done this way?" or project history

### 4. Codebase (`crates/`, `apps/`)
- `crates/mediaservo-common/` — HAL traits, types, primitives (D10/D11/D12)
- `crates/mediaservo/` — in-process transport/discovery (D11, Phase 1)
- `crates/hal-flatbuffers/` — FlatBuffers schema + bindings (D19, cross-language)
- `apps/studio/` — Tauri + React + TypeScript IDE (D21, Phase 1)
- **Check when** questions involve actual implementation details

---

## MediaServo Project Awareness

When exploring, keep these MediaServo specifics in mind:

| Concern | Details |
|---------|---------|
| Build system | cargo workspace → cargo build / cargo test |
| Rust standard | Rust stable, ownership, borrowing, traits |
| HAL primitives | Signal / StreamChannel / RPC (D10) |
| Middleware | mediaservo (Phase 1), amw_zenoh (Phase 2) (D11) |
| Type system | IEC 61131-3 mapped — 14 types (D12) |
| Multi-language | FlatBuffers schemas + codegen (D19) |
| RT config | HAL Config Barrier at cycle boundary (D17) |
| Studio | Tauri window management (D21) |
| Config format | YAML dev → FlatBuffers runtime (D24) |

---

## What You Don't Have To Do

- Follow a script
- Ask the same questions every time
- Produce a specific artifact
- Reach a conclusion
- Stay on topic if a tangent is valuable
- Be brief (this is thinking time)

---

## Handling Different Entry Points

**User brings a vague idea:**
```
User: I'm thinking about adding a signal monitoring panel

You: Signal monitoring could go in several places. Let me think about this...

      SIGNAL MONITORING OPTIONS IN MediaServo
      ═══════════════════════════════════════════

      ┌──────────────┐  ┌──────────────┐  ┌──────────────┐
      │ Studio Panel │  │ Runtime       │  │ Config       │
      │ (Tauri)      │  │ Dashboard     │  │ Web UI       │
      ├──────────────┤  ├──────────────┤  ├──────────────┤
      │ Integrated   │  │ Separate      │  │ Browser-     │
      │ in IDE       │  │ monitoring    │  │ based        │
      │ React + TS   │  │ process       │  │ PWA (D21)    │
      └──────────────┘  └──────────────┘  └──────────────┘

      Where's your head at?
```

**User brings a specific problem:**
```
User: The HAL Signal write is not propagating

You: [reads hal-core signal module and related files]

     Let me trace what happens when a Signal write occurs...

     write_signal() → Config Barrier check (D17) → mediaservo publish
         │                                               │
         │                                               ▼
         │                                    subscriber callback
         │                                    latest-value overwrite
         │                                               │
         ▼                                               ▼
     Signal::new_value → notify subscribers (latest-value semantics)

     Where's the failure point?
```

**User wants to compare options:**
```
User: Should we use mediaservo or amw_zenoh for Phase 1?

You: Let's look at what MediaServo already does...

     mediaservo (current)  vs   amw_zenoh (Phase 2+)
     ─────────────────────────────────────────────────
     Mature in codebase     │  New, needs setup
     Zero-copy in-process   │  Network transport
     No external deps       │  Zenoh protocol dep
     Phase 0/1 ready        │  Phase 2+ target

     Unless you need network transport right now,
     mediaservo is the lower-risk path for Phase 1.
```

---

## Ending Discovery

There's no required ending. Discovery might:
- **Flow into a proposal**: "Ready to start? I can create a change proposal."
- **Result in artifact updates**: "Updated design notes with these decisions"
- **Just provide clarity**: User has what they need, moves on
- **Continue later**: "We can pick this up anytime"

When it feels like things are crystallizing, you might summarize:
```
## What We Figured Out

**The problem**: [crystallized understanding]

**The approach**: [if one emerged]

**Open questions**: [if any remain]

**Next steps** (if ready):
- Create a change proposal
- Keep exploring: just keep talking
```

---

## Guardrails

- **Don't implement** — Never write code or implement features. Creating artifacts is fine, writing application code is not.
- **Don't fake understanding** — If something is unclear (e.g., Config Barrier RT cycles, SCHED_FIFO), dig deeper
- **Don't rush** — Discovery is thinking time, not task time
- **Don't force structure** — Let patterns emerge naturally
- **Don't auto-capture** — Offer to save insights, don't just do it
- **Do visualize** — A good diagram is worth many paragraphs
- **Do explore the codebase** — Ground discussions in MediaServo reality
- **Do question assumptions** — Including the user's and your own