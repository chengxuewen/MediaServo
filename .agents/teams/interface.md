# Interface delivery team (orchestration intent, host-neutral)

> A team is an orchestration intent, not a host file format. The main session is
> the lead; there is no separate lead role. Members below must be existing roles.

## Members
- ui-ux
- web-frontend
- code-reviewer

## Flow (derive -> list -> implement -> adjudicate -> merge)
1. Derive: ui-ux reads the design rules and the target surface, derives the interface requirements.
2. List: the lead turns the requirements into a concrete change list.
3. Implement: web-frontend implements the list within www/.
4. Adjudicate: ui-ux reviews interface quality (read-only); code-reviewer gives the five-axis verdict.
5. Merge: the lead merges only when both reviews pass and the gates are green.

## Discipline
- Read-only members never write; implementation belongs to web-frontend.
- No re-delegation: a member does not spawn its own sub-agents or hand its work further down; the assigned role does the work itself.
- Disagreements are surfaced to the lead, not voted away.
- This template is host-neutral: no concrete tool calls are encoded here.