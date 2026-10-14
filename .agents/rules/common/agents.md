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

Located in `~/.claude/agents/`:

| Agent | Purpose | When to Use |
|-------|---------|-------------|
| planner | Implementation planning | Complex features, refactoring |
| architect | System design | Architectural decisions |
| tdd-guide | Test-driven development | New features, bug fixes |
| code-reviewer | Code review | After writing code |
| security-reviewer | Security analysis | Before commits |
| build-error-resolver | Fix build errors | When build fails |
| e2e-runner | E2E testing | Critical user flows |
| refactor-cleaner | Dead code cleanup | Code maintenance |
| doc-updater | Documentation | Updating docs |
| rust-reviewer | Rust code review | Rust projects |

## Immediate Agent Usage

No user prompt needed:
1. Complex feature requests - Use **planner** agent
2. Code just written/modified - Use **code-reviewer** agent
3. Bug fix or new feature - Use **tdd-guide** agent
4. Architectural decision - Use **architect** agent

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
