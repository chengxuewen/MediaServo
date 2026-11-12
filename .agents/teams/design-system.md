# Design-system team (orchestration intent, host-neutral)

> A team is an orchestration intent, not a host file format. The main session is
> the lead; there is no separate lead role. Members below must be existing roles.
> Discipline follows `expert-panel-review` (announce lineup, independent conclusions
> first, keep disagreements, lead merges but does not review).

## Members
- ui-ux
- web-frontend
- code-reviewer
- evidence-critic

## Product
Design tokens + component contracts + a migration list (the `@mediaservo/ui` layer).

## Flow (derive -> token -> consume -> contract -> evidence)
1. Derive: ui-ux reads the design rules and the target surfaces, derives the token needs.
2. Token: the lead turns needs into token additions/changes in `www/packages/ui/tokens.css` (via the `design-system` method; a token needs a real consumer).
3. Consume: web-frontend validates the tokens from the consumer side (admin/voice actually use them, no new raw hex).
4. Contract: code-reviewer checks the token/component contract stays backward-compatible (no silent rename/break of a consumed token).
5. Evidence: evidence-critic requires file:line / screenshot proof for each claim; the lead merges only when the gates are green.

## Discipline
- Read-only members (ui-ux, evidence-critic) never write; token edits go through web-frontend / the lead.
- No re-delegation: a member does not spawn its own sub-agents or hand its work further down; the assigned role does the work itself.
- Each member gives an independent conclusion before seeing the others'.
- Disagreements are surfaced to the lead, not voted away.
- Host-neutral: no concrete tool calls are encoded here.