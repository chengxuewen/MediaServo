# Agent-infrastructure team (orchestration intent, host-neutral)

> A team is an orchestration intent, not a host file format. The main session is
> the lead; there is no separate lead role. Members below must be existing roles.
> Discipline follows `expert-panel-review`.

## Members
- agent-infra
- code-reviewer
- evidence-critic

## Product
`agents sync` / `agents check` green in both contexts + negative-case evidence.

## Flow (change -> idempotence -> dual-context -> negatives -> merge)
1. Change: agent-infra implements the generator/gate change in `scripts/agents/**` (the only role allowed to edit cli.py).
2. Idempotence: code-reviewer checks the change is idempotent (second sync = 0 writes) and does not hand-edit a generated layer.
3. Dual-context: verify S (standalone) and P (submodule) outputs stay consistent; inherited links are never written through.
4. Negatives: evidence-critic confirms each negative fixture actually goes red (a guard that cannot fail is not a guard).
5. Merge: the lead merges only when sync/check are green and the negatives are proven red.

## Discipline
- Read-only members (code-reviewer, evidence-critic) never write; implementation belongs to agent-infra.
- Independent conclusions first; disagreements surfaced to the lead, not voted away.
- Host-neutral: no concrete tool calls are encoded here.