---
name: doc-audit
description: "MediaServo document & architecture audit. Parallel checks of self-consistency, completeness, gaps, and optimization opportunities across architecture docs, design docs, and decision records. Confirms every finding with the user one item at a time via interactive Q&A, listing details, options with pros/cons, sources, impact, and recommendation. Supports team mode (large audits) and background-agent mode (light checks)."
---

# Document & Architecture Audit

Full audit of the MediaServo documentation system: doc consistency, decision verification, agent infrastructure, reference benchmarking.

**Philosophy**: auditing isn't nitpicking — it's paying down debt. Documentation debt is as dangerous as code debt.

---

## Entry: Audit Types

### `/doc-audit` (no arguments)
Pops the audit-type menu:

```
[1] Full audit — all 5 dimensions (default)
[2] Decision verification — decisions.md follow-through
[3] Doc consistency — arch ↔ modules ↔ sub-docs cross-check
[4] Agent system — .agents/ rules/skills/memory self-consistency
[5] Gap optimization — security/reliability/ops scan
[6] Phase audit — Phase progress vs reality
```

### `/doc-audit full`
Starts the full audit directly, skipping the menu. Equivalent to `[1] Full audit`.

### `/doc-audit quick-fix`
Only checks LOW/MEDIUM issues and auto-fixes them, without the interactive review.

---

## Audit Dimensions

### 1. Decision Verification
Check how decisions.md D1-Dx are reflected in architecture docs, implementation code, and status records.

**Core questions**:
- Are decision conclusions correctly reflected in architecture.md and module docs?
- Any "decision says A, reality is B" contradictions?
- Do stale references inside decisions need updating?
- **Decision freshness check**: has the decision been superseded by a later one? Is the referenced tech-stack version outdated?

### 2. Doc Consistency
Check cross-consistency: architecture.md ↔ modules/ ↔ README ↔ AGENTS.md.

**Core questions**:
- Is the same concept described consistently? (crate count, Phase status, tech-stack versions)
- Do sub-docs duplicate the main doc?
- Are Phase terms ambiguous?
- Do crate names follow the C4 convention?

### 3. Agent System Audit (new)
Check self-consistency of rules, skills, and memory under .agents/.

**Core questions**:
- conventions.md numbering contiguous? C{n} cross-references valid?
- decisions.md numbering contiguous? D{n} order correct?
- pitfalls.md numbering contiguous? PIT-{n} cross-references complete?
- skill directory table (AGENTS.md) matches the actual skill list?
- task routing table (context-engineering) matches the actual skill list?
- all opencode.json instructions references exist?
- each skill's SKILL.md frontmatter complete?

### 4. Gap Optimization
Scan for missing key docs / design sections.

**Core questions**:
- security architecture: are PSK rotation, JWT expiry policy, mediasoup transport security documented?
- ops/observability: health checks, metrics export, log aggregation?
- error model: SFU connect failure, encode degradation, transport disconnect handling?
- hardware baseline: minimum CPU/RAM, Docker resource config?
- upgrade strategy: hot updates, crate version migration, config migration?

### 5. Phase Audit
Check Phase progress vs doc claims vs code reality.

**Core questions**:
- does status.md Phase status match git log / test count?
- are decisions.md Phase tags accurate?
- does the implementation match the completion state the docs claim?

---

## Audit Modes

### A. Team Mode (recommended — large audits)
3+ large documents → `team_create` with 4-6 members in parallel.

```
team_create(inline_spec={
  name: "doc-audit",
  members: [
    { name: "decision-validator", category: "deep", prompt: "<decision-verification core questions>" },
    { name: "consistency-checker", category: "deep", prompt: "<doc-consistency core questions>" },
    { name: "agent-auditor", category: "deep", prompt: "<agent-system core questions>" },
    { name: "gap-optimizer", category: "deep", prompt: "<gap-optimization core questions>" }
  ]
})
```

**Conductor spec** (dispatcher behavior):
- right after launch, report to the user: "started N parallel audit lanes, ETA 3-5 minutes"
- until all complete, do only "non-overlapping work"
- when all complete: **dedupe & merge** (same issue found by 2+ dimensions → merge into 1 item)
- sort by severity: CRITICAL → HIGH → MEDIUM → LOW
- timeout handling: any lane producing nothing past 10 minutes → mark "timed out"
- conflict handling: dimension A says X, dimension B says Y → flag for human review

### B. Background-Agent Mode (light audits)
Few docs → `task(category="deep", run_in_background=true)` × N in parallel.

### C. Single-Thread Mode
Tiny scope → check directly with Read/Grep, no subagents.

---

## Interactive Review: Finding Format

**Review item by item**, present each with the `question()` tool.

```markdown
## 🔴/🟠/🟡/🔵 [id]: [title]

### Details
| Source 1 | Location | Content |
|--------|------|------|
| doc A | line X | ... |
| doc B | line Y | ... |

### Options
| Option | Pros | Cons |
|------|------|------|
| A. [name] | ... | ... |
| B. [name] | ... | ... |

### Recommendation
[Option X]. [Reason]
```

Choices: adopt recommendation / pick another option / skip / custom
Progress: `[item N of M]`

---

## Workflow

### Phase 1: Launch
1. confirm audit scope and type
2. pick the mode (team / background / single-thread)
3. report: "starting N parallel audit lanes"

### Phase 2: Merge
1. dedupe: same issue from multiple sources → merge with annotations
2. sort: CRITICAL → HIGH → MEDIUM → LOW
3. cross-confirmation: issues agreed on by 2+ dimensions get priority bumped

### Phase 3: Interactive Review
Item-by-item review with question() confirmations.

### Phase 4: Fix
1. create a todo list
2. dependency order: fix decisions first → then docs → status last
3. verify after every edit

### Phase 5: Report
```
Audit complete — [date]
Audit type: [full/decisions/consistency/agent-system/gaps/phase]
Findings: N | fixed: M | skipped: K
Next suggestion: [issue-dense area]
```

---

## Severity Criteria

| Severity | Trigger | Blocking? |
|--------|---------|:---:|
| 🔴 CRITICAL | doc contradiction causing implementation error / decision overturned / core API missing | ✅ |
| 🟠 HIGH | stale references / Phase ambiguity / duplicated docs / numbering holes | ⚠️ |
| 🟡 MEDIUM | wording drift / conflicting examples / gap that doesn't block the current Phase | ❌ |
| 🔵 LOW | format inconsistency / missing reference / to-be-confirmed markers | ❌ |

## Audit Frequency

- after every D# decision change: `/doc-audit decisions`
- before Phase transitions: `/doc-audit full`
- weekly during active development: `/doc-audit full`
- after any major doc rewrite: `/doc-audit consistency`

---

## Community References

| Prior art | Pattern borrowed |
|------|-----------|
| [large-codebase-audit](https://github.com/MJWNA/large-codebase-audit-skill) | 9-surface AI-layer audit, aligned with Anthropic best practices |
| [claude-ecosystem](https://github.com/melodic-software/claude-code-plugins) | meta-skill architecture, 16 audit agents |
| [agent-self-audit](https://github.com/Xxt-XN/agent-self-audit) | two-tier self-audit design, auto-escalation |

---

## Division of Labor with ecosystem-scan

| Dimension | ecosystem-scan | doc-audit |
|------|:---:|:---:|
| agent infrastructure (skills/rules/MCP) | ✅ specialty | ✅ dimension 3 |
| external community comparison | ✅ core capability | ❌ |
| internal doc consistency | ❌ | ✅ specialty |
| decision verification | ❌ | ✅ specialty |
| security audit gate | ✅ Full mode | ❌ |
