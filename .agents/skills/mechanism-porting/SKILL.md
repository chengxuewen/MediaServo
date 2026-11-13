---
name: mechanism-porting
description: Execute a USER-RULED adoption of an external mechanism (agent-subagent-layer B11, N31). Takes a ruled pattern card or proposal, verifies the source, extracts the mechanism (never the content), checks constitutional Fit, and lands it through a gated batch. Use when a proposal names a mechanism to adopt/port and the user has ruled. NOT for discovery (repo-audit) or design decisions (think-before-act).
---

# mechanism-porting — ruled adoption, executed with gates

> This skill EXECUTES an adoption the user has already ruled. Discovery belongs to
> `repo-audit` (2B/2C pattern cards); the design decision belongs to
> `think-before-act` + the user's ruling; this skill is the porting executor
> (the "porting execution" named in repo-audit's governance boundary).

## Trigger Conditions

- A ruled proposal / pattern card names a mechanism to adopt or port.
- The user asks to "land the ruled adoption" or execute a proposal card.

## Method (input: a RULED card -- if unruled, stop and route back)

1. **Verify the source** (P-5 source priority): locally installed config >
   official docs > third-party articles. A mechanism observed only in an article
   needs a fresh source check before porting (CSR trap: render client sites first).
2. **Extract the mechanism, not the content**: state the mechanism in one sentence
   ("what problem it solves + how it works") with source evidence (repo + path).
   Porting a role list, category taxonomy, or prompt text verbatim is a violation.
3. **Fit check** against this repo's constitution: English gate (C49), append-only
   memory, gates-first, zero-copy, user start gates, quotas. A conflicting
   mechanism is rewritten-to-fit or discarded -- say which, with the reason.
4. **Minimal adaptation plan**: what lands, in which file, through which gate
   (landing + gate columns -- same hard rule as repo-audit 3D: missing either
   demotes back to backlog).
5. **Land through the ruled batch**: implement in the already-ruled batch, run the
   batch's acceptance gates, keep the diff small (mechanism ports are reviews
   like any other change).
6. **Ledger**: record provenance (source repo + path + what was borrowed vs
   rewritten) in `docs/reference/visiaengine.md`; record the outcome in
   `.agents/memorys/status.md`.

## Boundaries

- No ruling -> no port. "The community does it this way" is not a ruling.
- No discovery here: finding candidate mechanisms is `repo-audit`'s 2B/2C.
- No design authority: conflicting with the constitution is decided by the user
  (rewrite-to-fit vs discard), not by this skill.
- Content (role lists, taxonomies, prompt text) is never ported -- mechanism only.