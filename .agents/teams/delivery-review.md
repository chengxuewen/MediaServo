# Delivery-review team (orchestration intent, host-neutral)

> A team is an orchestration intent, not a host file format. The main session is
> the lead; there is no separate lead role. Members below must be existing roles.
> Discipline follows `expert-panel-review` (lineup gated by complexity, independent
> conclusions, keep disagreements, lead merges but does not review).

## Members (constant core)
- code-reviewer
- evidence-critic

## Scenario mounts (1-2, chosen from existing domain roles per the change)
Pick the mount(s) that match the delivery surface; they are lenses on the review,
not new roles. Examples: `rust-core` (wire/ABI contract), `ffi-abi` (cross-language
boundary), `build-ci` (gates not weakened), `deploy-ops` (rollback / single-instance),
`e2e-weaknet` (reproducible numbers), `web-frontend` (frontend contract). Security and
performance stay lenses on code-reviewer (no standalone roles, per the roster rule).

## Product
A severity-tagged adjudication report (ready / not-ready) with reproducible evidence.

## Flow (scope -> independent review -> challenge -> adjudicate)
1. Scope: the lead announces the lineup (core + chosen mounts) and the change under review.
2. Independent review: each member reviews independently and gives a five-axis / domain verdict; no member sees another's conclusion first.
3. Challenge: evidence-critic demands file:line + a reproducible command for every finding; unproven claims are labelled speculation.
4. Adjudicate: the lead merges the verdicts into a severity-tagged report; disagreements are preserved, not voted away; CRITICAL findings block the merge.

## Discipline
- Read-only throughout: this team reviews, it does not implement.
- No re-delegation: a member does not spawn its own sub-agents or hand its review further down; the assigned role does the review itself.
- The lead merges conclusions but does not add its own review verdict.
- Host-neutral: no concrete tool calls are encoded here.