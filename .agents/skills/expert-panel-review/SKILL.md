---
name: expert-panel-review
description: "Multi-role expert panel reviewing ONE artifact (a plan, a conclusion, a decision, a design): independence first, isolation discipline, a mandatory adversary, disagreements preserved, panel size gated by complexity. Triggers when the user says the panel keywords (see Trigger Conditions), names team mode, or asks for a red-team of a specific plan/verdict. Host-adaptive: without team_create the default is one agent doing isolated sequential role passes; with team_create parallel is opt-in. Not for auditing a whole doc corpus (doc-audit), not for the act/no-act gate (think-before-act), and disagreements go to adjudication-walkthrough for one-by-one user verdicts."
---

# expert-panel-review — Expert Panel Engine (plan / verdict / decision)

Convene a group of experts with **genuinely different lenses** to adversarially
review **one artifact** (a plan, a conclusion, a decision, a design), producing
deduplicated, severity-ranked findings and handing disagreements to the user
verbatim.

**Philosophy**: the value of a panel is not headcount, it is **independence**.
N roles reaching the same conclusion does not mean N independent verifications
-- it may be one model, one corpus, one bias copied N times ("consensus
illusion"). Every design choice below exists to **create and protect lens
diversity**.

## Trigger Conditions

- The user says the panel keywords: "专家团" / "团队审核" / "多角色评审" /
  "审这份计划" / "审这个结论" / "red-team 一下" <!-- c49:trigger -->
- The user names team mode explicitly.
- A plan/decision needs multi-angle stress testing before execution (commonly
  invoked from the complex-change branch of `think-before-act`).

### Explicit non-triggers (routing fence, do not steal these words)

| Need | Go to | Difference |
|------|-------|------------|
| Audit the **whole doc corpus** for self-consistency | `doc-audit` | Object is a document set; method is cross-checking |
| Decide **whether to act / whether to convene** | `think-before-act` | That is the decision gate; this engine is the reviewer it calls |
| Present **disagreements one by one** for user verdicts | `adjudication-walkthrough` | That is the presentation protocol; conflicts land there |

## Position: the four-layer split

```
think-before-act         decides "act or not / convene or not" (decision gate)
      |  calls this on complex changes
      v
expert-panel-review      how to run one multi-role review (this engine)
      |  produces conflict items
      v
adjudication-walkthrough presents disagreements one by one (presentation protocol)

doc-audit                audits the whole doc corpus (parallel; its Team Mode
                         section references this engine's execution model)
```

This engine is a **reusable executor**: when `think-before-act`, `doc-audit`,
or another skill needs "team / multi-role", they reference this execution model
instead of inlining their own `team_create` recipe.

## Execution engine (dual-track, host-adaptive)

Probe host capability before convening, then pick a track.

### Track 1 — Sequential (default)

**Applies**: hosts without `team_create` (the only parallel-capable subagent is
CodeReview, which fits diffs only, not plans/docs); and dependency-heavy,
single-artifact reasoning reviews (community consensus: for these, one agent
often beats a pile of subagents).

One agent performs **multiple structured role passes**. The isolation
discipline is **hard-coded -- violating it creates an echo chamber**:

1. Each pass, the role receives ONLY: `the artifact under review` + `this
   role's brief (lens + adversarial instruction)` + `shared acceptance
   criteria`.
2. **Forbidden**: feeding conclusions or drafts from earlier roles into the
   current pass -- independent first, merge later.
3. At the end of each pass, **lock** that role's `verdict + evidence (file:line)
   + confidence + uncovered areas` before starting the next pass.
4. Only after all roles are locked does the lead merge pass begin.

**Honest limitation**: sequential same-context role-play isolates **more
weakly** than true subagent contexts (the model can still see its own earlier
passes). Compensation: forced adversarial briefs, restating "judge only the
artifact against the criteria, do not anchor on earlier conclusions" each pass,
and file:line evidence requirements.

**Optional boost**: when the artifact under review **is a code diff**, one
independent-context correctness/security verification can be delegated to the
host's `CodeReview` subagent (real isolation); all other roles stay on the
sequential track. Do not use it for plans/docs (type mismatch).

### Track 2 — Parallel (opt-in)

**Applies**: hosts providing `team_create` / `team_task_create` (opencode /
omo), and reviews that parallelize with low inter-member dependency.

Keeps the original team-mode semantics: 3-5 **read-only** members, **no git
writes**, each member in a true independent context (strongest isolation), lead
merges, dedupes, publishes the ranked report, and **closes the team
immediately**. Member briefs and output formats are identical to Track 1.

> The parallel track is kept, not cut (do not trim declared capability); it is
> simply not assumed to exist.

## Panel composition (2 constant cores + scenario mounts)

### Constant core (always present, 2 roles)

| Role | Brief | Adversarial instruction |
|------|-------|------------------------|
| **Correctness officer** | Does the conclusion hold? Can the plan reach its goal? Logic holes, hidden assumptions, circular reasoning? | Assume the conclusion is wrong; try to falsify it |
| **Evidence officer (devil's advocate)** | Does every claim have a file:line / checkable source? Are acceptance criteria testable? | Hunt unevidenced claims; mark each one pass/fail |

> The evidence officer is the DEBATE / CourtEval "devil's advocate /
> prosecutor" -- **mandatory**; without it the panel degrades into mutual
> agreement.

### Scenario mounts (1-3 roles by artifact type, reusing existing skill lenses -- never invent new personas)

| Artifact type | Mounted roles | Reused lens |
|---------------|---------------|-------------|
| Code / build change | Concurrency-reliability officer, architecture-boundary officer | `code-review-and-quality` (five axes), `api-interface-design` |
| Security-relevant | Security officer | `security-hardening` |
| Docs / decisions | Consistency-and-freshness officer | `doc-audit` (decision verification + freshness) |
| Contract / API / protocol | Contract officer | `api-interface-design` |
| Release / deploy | Ops-observability officer | `doc-audit` gap dimension (health checks / metrics / rollback) |
| Performance / resources | Performance-cost officer | `performance-optimization` |
| Weak-network / transport | Weak-network measurement officer | `weaknet-testing` |

**Diversity hard requirement** (ChatEval): roles must truly differ in
**lens / input focus / output format**; if all roles share one persona or one
view, the payoff is zero -- do not convene at all.

**Declare the lineup when convening**: Phase 1 announces the roles to the user,
who may add or remove on the spot ("add a security officer", "drop perf").

## Complexity gating (do not imitate 64-agent runs)

Panel size follows the artifact's risk, not enthusiasm:

| Tier | Criteria | Lineup |
|------|----------|--------|
| **No panel** | Trivial / reversible / single-point / no cross-domain constraints | Core 2, one pass each, or the trivial branch of `think-before-act` |
| **Standard** | Interface change / multi-file / trade-offs required | Core 2 + 1 scenario mount = 3 roles |
| **Deep** | Architecture-level / irreversible / >5 files / high-security surface | Core 2 + 2-3 scenario mounts = 4-5 roles |

> Community evidence: Anthropic multi-agent runs cost ~15x tokens;
> dependency-heavy tasks do not parallelize well; the "45% threshold" -- the
> higher single-agent accuracy already is, the smaller the multi-agent payoff.
> Before adding a role ask: **does it add a genuinely new judgment path, or
> just repeat an existing answer?**

## Output format

### Per role (locked independently)

```
### <role> — independent verdict
- Verdict: <pass / pass-with-conditions / fail> + one-sentence judgment
- Evidence: <file:line or checkable source> (unproven judgments are labeled "speculation")
- Confidence: high / medium / low
- Uncovered: <what this role's lens cannot see; name the role that should>
```

### Lead merge pass (single writing pass, never parallel)

1. **Dedupe**: the same issue found by multiple roles merges into 1 item,
   annotated "hit jointly by X/Y" (raises priority).
2. **Rank**: CRITICAL -> HIGH -> MEDIUM -> LOW (aligned with the `doc-audit`
   severity table).
3. **Preserve disagreement**: role A says X, role B says Y -> keep **both, as a
   conflict item**; never smooth it over with majority vote. Conflict items go
   to `adjudication-walkthrough` for item-by-item verdicts.
4. **Report**: findings list (each with file:line + severity + hitting roles) +
   conflict list + suggested next step.

## Workflow

### Phase 1: Convene
1. Confirm the artifact under review and the shared acceptance criteria ("what
   counts as passing", 3-5 hard indicators, fixed before running).
2. Set the panel size via complexity gating and **declare the lineup**; the
   user may adjust roles.
3. Probe host capability, pick sequential / parallel, announce: "starting an
   N-role panel (sequential/parallel)".

### Phase 2: Independent review
Each role produces its verdict per the isolation discipline (sequential passes
or parallel members), locking `verdict + evidence + confidence + uncovered`.

### Phase 3: Merge, dedupe, rank
The lead merges in one pass: dedupe -> rank -> annotate hitting roles.

### Phase 4: Conflicts upstream
Conflict items go to `adjudication-walkthrough` for item-by-item verdicts;
CRITICAL items must be resolved before execution proceeds.

### Phase 5: Report

```
Panel review complete — [date]
Artifact: [name] | Track: [sequential/parallel] | Lineup: [N roles]
Findings: M total (CRITICAL a / HIGH b / MEDIUM c / LOW d) | Conflicts: K (handed to adjudication)
Next: [fix CRITICAL / user adjudicates conflicts / proceed]
```

## Anti-excuses

| Excuse | Rebuttal |
|--------|----------|
| "More roles is safer" | Headcount is not independence; same-view roles just copy the bias. Name the new judgment path first. |
| "Sharing context between roles is more efficient" | Shared drafts = shared blind spots = echo chamber. Lock independently first, merge later. |
| "Everyone agrees, so it passes" | Consensus can be an illusion. The point is complementary evidence, not nodding in unison; disagreements must survive. |
| "The evidence officer is too picky, it slows things down" | Unevidenced claims are exactly where incidents come from; picking is the job, not an obstacle. |
| "A panel for THIS?" | That is what the gate is for; trivial artifacts get the core 2 once, or no panel. |

## Red flags

- A role has no **independent** verdict (copied another role) -- echo chamber.
- A finding has no file:line / checkable source.
- Disagreements were smoothed over with "majority agrees"; conflicts never
  reached `adjudication-walkthrough`.
- A trivial artifact convened 4-5 roles (no gating).
- All roles share one persona / one lens (diversity is zero).
- On the sequential track, an earlier role's conclusions were fed into a later
  pass (isolation broken).

## Verification (evidence)

- Every role's verdict traces to file:line or is explicitly labeled "speculation".
- The merged report carries severity + hitting roles + a conflict list.
- Conflicts actually reached `adjudication-walkthrough`, not silently absorbed.
- The lineup was declared in Phase 1 and the user had a chance to adjust it.

## Community references

| Prior art | Pattern borrowed |
|-----------|------------------|
| ChatEval (Chan et al. 2024) | Multi-role judge panel; **role diversity is the lifeline**, same persona = zero payoff |
| DEBATE (Kim et al., ACL 2024) | Scorer + Critic (devil's advocate) + Commander; adversarial fault-finding |
| CourtEval (Kumar et al., ACL 2025) | Judge + Prosecutor + Defender; **keep both sides** before re-judging |
| Du et al. 2023, Multi-Agent Debate (arXiv:2305.14325) | Independent answers -> cross-review -> multi-round convergence |
| OpenAI 64-agent retrospective | "Consensus illusion"; deliberately induce disagreement, isolate early information |
| Anthropic multi-agent research system (2025-06) | ~15x tokens; dependency-heavy tasks parallelize poorly; reports are **never written in parallel** |
| Anthropic "When to fold subagents into your main agent loop" (2026-10) | Prefer one agent; start from the simplest pattern; skills not subagents |
| Google/DeepMind/MIT "45% threshold" (2026-07) | Higher single-agent accuracy shrinks multi-agent payoff |

## Relations to other skills

| Skill | Relation |
|-------|----------|
| `think-before-act` | Owns "act or not / convene or not" (the gate); calls this engine on complex changes |
| `adjudication-walkthrough` | Receives this engine's conflict items for item-by-item verdicts |
| `doc-audit` | Parallel; its Team Mode section references this execution model, its 5 dimensions stay its own |
| Lens skills | `code-review-and-quality` / `security-hardening` / `api-interface-design` / `performance-optimization` / `weaknet-testing` provide ready-made scenario views |
