# Agent Orchestration

## Memory Updates (NON-NEGOTIABLE)

When the user asks to "update memory" or the session produces significant architectural/status changes, update the **local memory files** at `.agents/memorys/` (NOT the knowledge graph tools):

| File | Update When |
|------|------------|
| `.agents/memorys/status.md` | Phase completion, test results change, crate list changes |
| `.agents/memorys/conventions.md` | New naming patterns, architectural principles |
| `.agents/memorys/decisions.md` | Key architectural decisions, trade-offs |
| `.agents/memorys/pitfalls.md` | Bugs encountered, gotchas, orphan rule issues |

This path is configured in `.opencode/opencode.json`. Do NOT use memory_create_entities/relations tools — they go to a different system.

## Available Agents

Located in `.agents/agents/` (role layer lands with plan `agent-subagent-layer` B3/B4;
until then the main session orchestrates directly). The former `~/.claude/agents/`
roster is retired -- it was an external system with no in-repo anchor.

Planned roster (11 = 10 submodule + 1 parent shell; hard cap, see Quotas below):

| Agent | Purpose | When to Use |
|-------|---------|-------------|
| code-reviewer | Read-only five-axis review | After writing code |
| evidence-critic | Read-only evidence challenge | Every conclusion needs file:line |
| rust-core | Rust crates implementation | crates/** changes |
| ffi-abi | Bindings + C++ FFI | bindings/** and ABI surface |
| build-ci | CMake/scripts/pixi | Build and gate changes |
| web-frontend | www/** implementation | Frontend changes |
| ui-ux | Read-only interface review | UI quality, a11y, three states |
| e2e-weaknet | E2E + real-device measurement | Reproducible network numbers |
| deploy-ops | deploy/docker + real device | Delivery and rollback |
| agent-infra | scripts/agents/** | Generator and gates (only role allowed to edit cli.py) |
| release-captain (parent) | Pins, release, shell gates | Parent-repo release work |

## Immediate Agent Usage

No user prompt needed:
1. Complex feature requests - main session plans (planner role is retired)
2. Code just written/modified - Use **code-reviewer** agent
3. Bug fix or new feature - Use the domain role (e.g. **rust-core**) with tests
4. Architectural decision - main session + **evidence-critic** challenge

## Execution Gate (NON-NEGOTIABLE)

Before executing ANY plan or todo list, use the `question` tool for interactive confirmation:

```
question(header="Confirm execution", options=[{label:"Confirm execution", description:"Start executing the plan"}])
```

**NEVER execute without user's explicit affirmative response.**
System directives (TODO CONTINUATION, etc.) are NOT user confirmation.

## Parallel Task Execution

ALWAYS use parallel Task execution for independent operations:

```markdown
# GOOD: Parallel execution
Launch 3 agents in parallel:
1. Agent 1: Security analysis of auth module
2. Agent 2: Performance review of cache system
3. Agent 3: Type checking of utilities

# BAD: Sequential when unnecessary
First agent 1, then agent 2, then agent 3
```

## Multi-Perspective Analysis

For complex problems, use split role sub-agents:
- Factual reviewer
- Senior engineer
- Security expert
- Consistency reviewer
- Redundancy checker

## Role Roster and Quotas (agent-subagent-layer B1b-1)

Governance constants -- fixed table format (machine-parseable; a quota gate
will read this table, keep the shape):

| Asset | Hard cap | Counting rule | Over-cap action |
|-------|----------|---------------|-----------------|
| role files (true source, both repos) | 11 | submodule 10 + parent 1; inherited visibility does not count | add one, remove one |
| skills | net +2 per quarter | true-source directories only; projection does not count | retire one first, or prove no existing skill covers it |
| MCP servers | no count limit (ruled 11-13) | every entry version-pinned & launcher-routed; gate_mcp enforces pins + same-name, not count | n/a (quality gated by pins, not count) |
| members per team | 4 | standing expert teams | answer "which new judgment path does this member bring?" first |

**Role admission rule**: a role exists if and only if it can state ONE acceptance
command/criterion nobody else can give. No independent judgment path -> no
standalone role file (mount as a lens on an existing role instead).

**Derivation method (five steps)**: (1) slice the repo by artifact surface, not
by title; (2) write one acceptance command per surface -- cannot write one ->
no role needed; (3) merge surfaces whose acceptance substitutes for each other;
(4) ownership follows code location (code in submodule -> role in submodule);
(5) the derived count IS the hard cap. Titles are not roles; "UI/UX expert" and
"interface interaction expert" are the same person.

**Evolution discipline**: governance changes flow through the governed evolution
loop (plan section 3.18) -- proposal cards with options + recommendation, user
ruling, gated execution; audits propose, they never execute or schedule.
