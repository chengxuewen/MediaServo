# Task routing map (orchestration intent, host-neutral)

> This is a routing map, not a team and not a skill. It classifies a task into one
> of four categories and routes it to the right team / role / discipline. Borrowed
> mechanism from category-based routing (OMO): classify first, then pick the lineup;
> the categories are this repo's own, not a port of anyone's role list.
> All names below are existing roles / teams / skills. The main session is the lead.

## Classify first

Ask: what is the artifact surface and the blast radius? Then route:

| Category | Signal | Route | Discipline / gates |
|---|---|---|---|
| Visual delivery | a page / component / UI feature in `www/**` | interface team; add design-system team if tokens change | `web-frontend` implements, `ui-ux` reviews; `tsc --noEmit` + Playwright + ui-ux verdict |
| Deep refactor | multi-file / framework-level / interface or ABI change | domain role + delivery-review team | `think-before-act` (options + approval) then `expert-panel-review`; full relevant gates |
| Small quick fix | single file, reversible, no interface/build/dep change | the domain role directly, no team | `think-before-act` trivial tier; targeted test/lint only (YAGNI on ceremony) |
| Architecture adjudication | a design decision, trade-off, or contested verdict | `expert-panel-review` + `adjudication-walkthrough`, `evidence-critic` | no implementation until ruled; record via `documentation-and-adrs` |

## Category notes

- **Visual delivery**: derive -> list -> implement -> adjudicate -> merge (see
  `interface.md`). If the change touches the token layer, run the design-system team
  flow too (`design-system.md`); styling consumes `@mediaservo/ui`, not new raw hex.
- **Deep refactor**: the domain role (`rust-core` / `ffi-abi` / `build-ci` /
  `web-frontend` / `deploy-ops` / `e2e-weaknet`) owns implementation; the
  delivery-review team (`delivery-review.md`) mounts the matching scenario lens.
  Never skip the options-and-approval gate for a refactor.
- **Small quick fix**: do not convene a team for a one-line reversible change. The
  domain role acts and leaves a one-line note. Escalate to deep-refactor routing if
  the blast radius grows mid-fix.
- **Architecture adjudication**: this is a decision, not an implementation. Produce a
  severity-tagged, evidence-backed verdict; the user rules; only then does a batch
  land. Audits propose, they never schedule or execute.

## Discipline (all categories)

- Read-only reviewers never write; implementation belongs to the writable domain role.
- No re-delegation: a member does not spawn its own sub-agents or hand its work further down; the assigned role does the work itself.
- Independent conclusions before cross-feed; disagreements surfaced, not voted away.
- Host-neutral: no concrete tool calls are encoded here.