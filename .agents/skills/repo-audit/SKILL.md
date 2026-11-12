---
name: repo-audit
description: Whole-repo engineering audit across 11 domains (skills, rules, mcp, scripts, docs, book, cli, tests, structure, naming, agents/teams) + governance inventory (2D eight-asset, 3D increment table) + external scan: 2B adoptable skills/rules/MCP, 2C practice scan incl. P-5 product-form (pattern cards for workflows, doc systems, conventions, tooling, agent products).
---

# repo-audit — Whole-Repo Engineering Audit + Ecosystem Scan

> Two-tier scan (Quick daily / Full deep) across **11 engineering domains**, quality scoring, security-audit gate.
> Governing principle: **gates-first** -- this repo runs 32 gate sites in `scripts/all-gates.sh` (quick 25 + heavy 7);
> the scan never re-checks by hand what a gate proves. The scan (a) verifies every surface HAS an enforcement owner,
> (b) runs gap checks where no gate exists, (c) spends deep analysis only where judgment is needed.
> Reference precedents: [autoskills](https://github.com/B143KC47/autoskills), [agent-skill-discovery](https://github.com/ericgandrade/claude-superskills), [skill-update-team](https://github.com/franktsai2008-eng/skill-update-team), [agent-self-audit](https://github.com/Xxt-XN/agent-self-audit).

## Trigger Conditions

- The user says "优化 agents" / "审计 .agents" / "扫描生态" / "找新技能" / "全面体检" (optimize agents / audit .agents / scan ecosystem / find new skills / full checkup) <!-- c49:trigger -->
- `/repo-audit` -- pops up the mode-selection menu
- `/repo-audit quick` -- fast scan (default)
- `/repo-audit full` -- deep scan (auto-escalated on first run or after >5 Quick runs)
- `/repo-audit report` -- view the last scan report
- `/repo-audit <domain>` -- single-domain deep dive (skills|rules|mcp|scripts|docs|book|cli|tests|structure|naming|agents)

## The 11 Audit Domains

| # | Domain | Surface | Enforcement owner (gates) | The scan adds |
|---|--------|---------|---------------------------|---------------|
| 1 | **skills** | `.agents/skills/` | agent toolchain (skill_lint); agent single source (SKILLS-INDEX segment) | duplicate detection, community comparison, utilization |
| 2 | **rules** | `.agents/rules/` (84 files) | agent toolchain (rule_ref_lint), english only (C49), agent single source | semantic dedup, runnable-command check, quality |
| 3 | **mcp** | `.mcp.json`, host mirrors, `scripts/agents/mcp-*` | gate-mcp (`gate_mcp.py`: shape/lock/probe) | launcher portability (per-host cwd/env), pin drift, per-host parity |
| 4 | **scripts** | `scripts/` (28 sh/py) | shellcheck gates (partial), python retirement closure | syntax health sweep (`bash -n` / `ast.parse`), orphan detection (not referenced by pixi/gates/docs), CRLF/LF consistency, shebang + exec bits |
| 5 | **docs** | `docs/` (README, reference/, plans/) | docs links, doc promises, english only | cross-reference audit; numbering HOLES detected here, numbering SEMANTICS hand off to doc-audit dimension 3 |
| 6 | **book** | `book/` (book.toml, toc.toml, web/) | book structure | toc.toml<->chapter nesting consistency, vendored offline assets, fold structure, title-length budget |
| 7 | **cli** | `mediaservo.sh` | shellcheck mediaservo.sh, entry contract (E1-E3+golden) | USAGE<->pixi task sync, front-door discoverability; sh/bat parity **n/a** (no .bat front door; Windows runs the .sh via git-bash/WSL) |
| 8 | **tests** | crates tests, test-cxx, `scripts/*smoke*` | test --workspace (lib), test-cxx, delivery smoke | orphaned tests, registry<->test-file consistency, dual-mode (human/ctest) coverage |
| 9 | **structure** | repo tree | manual | AGENTS.md STRUCTURE tree vs `ls` drift, root pollution (unexpected top-level dirs), territory ownership (`build/` `target/` `.pixi/` `.cache/`) |
| 10 | **naming** | files, tasks, branches | agent toolchain (partial) | case-collision detection, task-name style consistency, kebab-case adherence, mixed line-ending detection |
| 11 | **agents/teams** | `.agents/agents/`, `.agents/teams/`, host mirrors | agent single source (projection); agent_lint (B1, pending) | role count ≤11 (true source only), per-team ≤4, team members are existing roles, read-only negative, three-host parity |

**Reading the table**: if the enforcement owner is green, that domain's mechanical health is proven -- do not re-audit it.
The scan's local value concentrates in domains 4, 9, 10 (thin/no gate coverage), domain 6's nesting consistency,
and the judgment checks of 1-3, 5-8. A domain whose enforcement owner is empty AND has no scan finding is a
**coverage hole**, not a pass. Domain 11 (agents/teams) is the governance surface added by B1b-2: its quota checks
are executable, its judgment checks (unique acceptance, merge candidates) feed the 3D increment table.

## Entry Menu

With no arguments, pop up:

```
[1] Quick Scan -- 11-domain fast audit (< 30s)
[2] Full Scan -- deep audit + community sync + security gate (3-5min)
[3] View Report -- view the last scan results
[4] Quick Fix -- auto-fix LOW/MEDIUM issues (non-interactive; NEVER touches
                 .agents/memorys/decisions.md or status.md history -- report only)
[5] Domain dive -- deep-scan one domain (see Trigger Conditions)
```

## Two-Tier Modes

| Feature | Quick | Full |
|------|:-----:|:----:|
| Local audit | 1 fast check per domain (11) | deep checks per domain (26 items) |
| External scan | 3-5 known high-star repos | websearch + GitHub search + practice scan (2C, 4 lanes) |
| Scoring | 3-dimension fast | 5-dimension full scoring |
| Security audit | none | 6 safety checks |
| Duration | < 30s | 3-5min |
| Trigger | `/repo-audit quick` | `/repo-audit full` or >=5 Quick runs |

---

## Phase 1: Quick Scan (default)

### 1A: Local fast audit (10 items, one per domain)

```
1. skills:    every SKILL.md has name + description frontmatter; directory table covers all
2. rules:     conventions/decisions/pitfalls numbering contiguous; no orphan rule files
3. mcp:       every launcher referenced by the host configs exists; pins present
4. scripts:   `bash -n` all *.sh, `ast.parse` all *.py -- zero syntax errors
5. docs:      read the LAST gate output for docs links/doc promises (do not hand-check what a gate proves)
6. book:      book structure gate green (no build artifacts = SKIP); toc.toml entry count == chapter count
7. cli:       USAGE-advertised names all exist in pixi.toml (entry contract gate)
8. tests:     every smoke/ctest entry has a live test file (spot-check 5)
9. structure: AGENTS.md STRUCTURE tree top-level dirs == actual `ls` (drift = finding)
10. naming:   no case-variant filename collisions in .agents/ and docs/; no mixed EOL in docs/ book/
11. agents:   role count <=11 (true-source .agents/agents/ only); each team <=4; team members exist (empty dir = SKIP, not fail)
```

### 1B: External fast scan

Search 3-5 known high-star repos:
- VoltAgent/awesome-agent-skills (index)
- addyosmani/agent-skills (engineering skills)
- ECC/affaan-m everything-claude-code (full-stack configs)

Fetch READMEs with `webfetch`; 3-dimension quick scoring: tech-stack fit / already-present-in-project / quality.

### 1C: Quick output

```
🟢 No issues -- N checks passed, no deep scan needed
🟡 Found N items worth attention (CRITICAL/HIGH/MEDIUM/LOW ladder, matching code review) -- recommend /repo-audit full
```

---

## Phase 2: Full Scan

### 2A: Local deep audit (24 items across 10 domains)

| Domain | Deep checks |
|--------|-------------|
| skills | duplicate detection; community comparison; utilization vs install count; frontmatter quality |
| rules | semantic dedup; runnable-command verification; cross-reference integrity; staleness |
| mcp | launcher portability matrix (per-host cwd/env); pin vs installed drift; single-source consistency |
| scripts | orphan candidates (referenced by nothing); duplicated logic; CRLF/LF mix; arg convention consistency; error-message convention (`GATE-* ✗` shapes) |
| docs | dead-link detection; §N/PIT/D numbering contiguity; glossary consistency; polish-scope coverage |
| book | toc.toml/chapter/web triple consistency; fold structure; vendor freshness + license files; mermaid/lazy-loader pairing |
| cli | full USAGE parity; task-advertising drift; exit-code convention |
| tests | registry<->test-file bidirectional completeness; dual-mode entry coverage; golden-file freshness |
| structure | territory ownership audit; gitignore gaps; generated-vs-canonical tree diff |
| naming | case-collision scan; naming-convention adherence per file type; branch/tag style |
| agents/teams | three-host directory parity; read-only negative (no Write/Edit rendered); role count ≤11 by true source (symlinks/inherited visibility excluded); per-team ≤4; `.agents/teams/*.md` member names are existing roles; unique-acceptance judgment |

### 2B: External deep scan

#### Search strategy (4 parallel lanes)

1. **Known repos**: anthropics/skills, addyosmani/agent-skills, VoltAgent/awesome-agent-skills, ECC/affaan-m, superpowers
2. **High-star discovery**: `websearch: "github opencode skills popular stars"`
3. **Tech-stack specialization**: `websearch: "best AI agent skills for Rust/C++/TypeScript/DevOps github"`
4. **MCP search**: GitHub MCP-server repos (Rust, cargo, git, docker)

#### Scoring system (5 dimensions, max 10)

| Dimension | Weight | Criteria |
|------|:---:|------|
| **Fit** | 0.30 | tech-stack match (Rust/C++ FFI/TS/DevOps/Web) |
| **Trust** | 0.20 | repo stars + owner reputation + LICENSE |
| **Track-record** | 0.20 | proven real-world use (not auto-generated) |
| **Freshness** | 0.15 | last update (deduct if >180 days stale) |
| **Specificity** | 0.15 | focused content vs generic talk |

**Sanity Gate**: any Trust < 2, or unreadable content -> discard outright.

#### Security-audit gate (Full mode, before install)

| Check | Severity | Content |
|--------|:---:|------|
| repo-trust | **block** | stars, owner reputation, LICENSE, not archived |
| code-review | **block** | no `curl \| sh`, no `eval()`, no unauthorized file access |
| permissions-scope | **block** | no global filesystem access, no sudo |
| dependency-audit | warn | dependency audit, CVE check |
| data-exfil | **block** | no unauthorized data transmission |
| freshness | warn | recent commits < 180 days |

- any **block** -> reject
- any **warn** -> warning + confirmation required
- all pass -> SAFE

### 2C: Practice scan -- study HOW other projects work (Full mode)

2B asks "what can we install?"; 2C asks "how do the good repos DO it?". Five
sub-lanes, each producing **pattern cards** (adoptable practices, not artifacts):

| Lane | Looks at | Example questions |
|------|----------|-------------------|
| P-1 workflow / ci-gate design | gate layering, ratchet floors, failure policy, release flow | how do mature repos split fast/slow gates? do they ratchet coverage floors? |
| P-2 documentation system | docs-as-code, audit loops, decision-record formats | how do others mark superseded decisions? do they run doc audits? |
| P-3 repo structure / naming | tree conventions, ownership territories, generated-dir policy | how do others fence generated output? who documents hidden dirs? |
| P-4 tooling choices | task runners, env/lock strategy, auxiliary tooling | what do Rust-heavy repos pair with pixi-style envs? |
| P-5 product form | shipped agent products: packaging, routing, team orchestration, permission dialect | how do agent products package experts? how do they route by category? how is read-only expressed? |

**P-5 observation targets** (mechanism only, never content):

| Target | Mechanism to collect |
|--------|----------------------|
| WorkBuddy | expert-pack structure (`plugin.json` + `agents/` + skills), lead decomposition + handoff, connector auth choices |
| oh-my-openagent | category routing (local-measured 8 categories) + `team_mode` + background concurrency |
| CodeBuddy / Qoder | project-level vs user-level dirs, frontmatter dialect, read-only permission expression |
| VoltAgent collections | the category taxonomy itself (do NOT port the 158 roles) |

**P-5 guardrails**: (1) CSR trap -- client-rendered doc sites (e.g. WorkBuddy) return an empty shell to plain fetch; render first, then extract. (2) Borrow mechanism, not content. (3) Fit -- a pattern conflicting with the constitution (English gate / append-only memory / gates-first) is `rewrite-to-fit` or discard. (4) Source priority -- locally installed plugin config (e.g. `.omo/omo.jsonc`) > official docs > third-party articles; local config already overturned an official "7 categories" claim (measured 8).

**Pattern card** (the unit of output):

| Field | Content |
|-------|---------|
| Pattern | one-line name |
| Source | repo + path (file:line where readable) |
| Here | this repo's current state: have it / lack it / do it better |
| Fit | compatibility with this repo's constitution (C49 English-only, the append-only memorys contract, gates-first, quota hard caps, user start gate) -- a pattern that violates them is rewrite-to-fit or discard |
| Cost | adoption effort (lines / gates / process change) |
| Verdict | adopt (P1) / consider (P2) / discard |

**Scoring**: reuse the 5-dimension table; Fit judges constitutional compatibility
instead of tech-stack match. **Security**: patterns introduce no code, so the
code/permission/exfil checks are n/a; repo-trust and freshness still apply before
a pattern is worth reading.

**Relay**: a P1 pattern that needs deep study is archived under `.refinfo/` and
distilled into a `docs/reference/` profile -- repo-audit discovers and triages;
the reference-library mechanism digests. **CSR guard**: client-rendered doc sites
(e.g. WorkBuddy) return an empty shell to plain fetch -- render first, then extract.

### 2D: Eight-Asset Governance Inventory (Full mode)

The 11 domains audit engineering health; 2D inventories the eight governance asset
classes. Each class splits a **mechanical column** (proven by an existing gate --
never re-implement) and a **judgment column** (give a conclusion). Mechanical cells
cite the gate; a class with no gate is a coverage hole fed to 3D.

| Asset | True source | Mechanical (gate/script) | Judgment (conclusion) |
|---|---|---|---|
| experts (roles) | `.agents/agents/*.md` | three-host dirs complete, name==file, read-only negative, ≤11 | unique acceptance? merge candidate? |
| expert teams | submodule `.agents/teams/*.md` | template exists, members are existing roles | optimal lineup? discipline cited? |
| skills | `.agents/skills/*` | skill_lint, TRIGGERS coverage, quarterly net ≤2 | duplicate? promote or retire? |
| rules | `.agents/rules/**` | C49 English, @ref resolution | semantic dup? stale? |
| constraints | `rules/common/constraints.md` + anti-cheat | gate list == all-gates.sh | bypassable? |
| config | targets.json / mcp-servers.json / opencode.json | single source + projection parity | over-configured? |
| scripts | `scripts/**` | bash -n / ast.parse / orphan detection | merge? missing gate? |
| MCP | mcp.json + pins | pin drift, per-host projection | worth installing? permission scope? |

**Substrate six-dimension lens** (judgment overlay): cross-map the eight assets onto
tool / memory / reasoning / policy / context / coordination. `memory` has no 2D home
-> report the hole as a 3D candidate; widening to a ninth asset class is a design
decision requiring user ruling -- do not self-expand the taxonomy.

**Enforcement coverage**: classify each constraint prose-only vs mechanically enforced
(gate / host permission); output the coverage ratio + the prose-only list (a local
reconstruction of OMO's "instruction != enforcement", not a hook port). The enforced
set is the B10 gate-list gate once it lands; before B10, use the current all-gates.sh.

**memorys bloat note** (report-only): size proxy (wc/bytes) + stale entries (superseded
but unarchived). Never prune (append-only iron rule). Ownership split with doc-audit
(N38): repo-audit owns size + staleness; doc-audit owns duplication + conflict coverage
+ growth rate; a finding is counted once with an owner tag. Numbering / cross-reference
self-consistency belongs to doc-audit.

---

## Phase 3: Synthesis & Recommendation

### 3A: Cross-comparison

- multiple sources recommending the same content -> +2 points
- tech-stack mismatch but portable pattern -> annotate "rewrite-to-fit"
- 2C pattern cards and 2B install candidates are ranked in one P1/P2 list, typed
  (install / pattern) so a P1 pattern and a P1 skill never compete for the same slot

### 3B: Output format

```markdown
## Ecosystem Scan Report -- {date}

### 🟢 Phase 1: Quick (N passed)
### 🔴 Phase 2: Full Local Audit (M issues found, by domain)
### 🟡 Phase 2: External Scan (K recommendations)
### ❌ Rejected (L items)

#### Per-domain verdict
| Domain | Enforcement | Finding | Action |
|--------|-------------|---------|--------|

#### P1: Strongly recommended
| # | Item | Source | Score | Security | Effort |
|---|------|------|:---:|:---:|:---:|

#### P2: Worth considering
| # | Item | Source | Score | Security | Effort |
|---|------|------|:---:|:---:|:---:|

#### Patterns (2C)
| # | Pattern | Source | Here | Fit | Cost | Verdict |
|---|---------|--------|------|:---:|:---:|:---:|

#### Governance increments (3D)
| Asset | Current | Gap | Candidate | Landing | Cost | Round | Quota | Gate |
|-------|---------|-----|-----------|---------|------|-------|-------|------|

#### Rejected
| Item | Reason | Security |
|------|------|:---:|
```

### 3D: Governance Increment Table (candidates, not a schedule)

Output format that feeds PLAN directly:

| Asset | Current | Gap | Candidate (source + pattern) | Landing (specific file) | Cost | Round | Quota verdict | Gate (new/reuse) |
|---|---|---|---|---|---|---|---|---|

**Hard rule**: missing Landing OR Gate -> the row demotes to backlog; writing "no gate"
must attach a reason. 3D gives suggestions; it never replaces PLAN scheduling.

**Evolution proposal card** (N35): a candidate row with ruling value upgrades to a
proposal card -- add "2-3 options + recommendation" and "source evidence" columns, plus
a strategy tag {harden, repair-only, innovate} (default harden; innovate must name the
new judgment path) and a pre-registered recheck criterion (gap anchor + baseline +
closure command + verification round; may only cite existing gates/audit rows, no
invented metrics). A row without options is a finding, not a proposal -- stays in
backlog. Proposal cards enter PLAN only after user ruling.

**Proposal-card meta audit** (self-check on 3D output): sample cards for completeness --
options / recommendation / evidence / landing+gate / quota ≤5 per quarter / strategy tag
/ recheck criterion. A card missing a required field demotes to backlog.

**Embedded-MCP wording** (reservation only): global MCP counts toward ≤5; a future
skill-embedded MCP form still counts capability + pin gate. The mechanism has no base
here -- do not implement; any quota-constant change requires user ruling.

### 3C: Persistence

Record scan results in `.agents/memorys/status.md` (a fixed "repo-audit" paragraph -- the memorys
surface is a fixed four-file set, and new files change the agent-links sync face; keep the text English, C49):
- each scan's findings, scores, decisions; the next scan first checks whether last time's issues were fixed
- the Quick-run counter lives in the same paragraph
- >=5 Quick runs -> automatically suggest Full
- **gap closure rate**: read the previous 3D gaps + evolution-batch acceptance, report closure rate against the pre-registered criteria (closed / previous gaps); unclosed -> flag for lesson-review, re-propose or drop next round. Full evolution-batch linkage lands B11+.

---

## Team-mode configuration (Full scan only; Quick runs in the main session)

Compressed to 4 members (this repo's per-team cap is 4; the synthesizer is the main session):

```
member 1: structure-analyst (deep) -- domains 9+10 + scripts syntax sweep
member 2: content-auditor (deep)   -- domains 1+2+5 (skills/rules/docs content)
member 3: platform-auditor (deep)  -- domains 3+6+7+8 (mcp/book/cli/tests)
member 4: ecosystem-auditor (deep) -- Phase 2B external 4 lanes + security audit
synthesizer: the main session      -- Phase 3 synthesis (waits for 1-4)
```

## Community references

This skill's composite design borrows from the following community precedents:

| Precedent | Pattern borrowed |
|------|-----------|
| [autoskills](https://github.com/B143KC47/autoskills) | 5-dimension scoring system, Sanity Gate, persistent memory |
| [agent-skill-discovery](https://github.com/ericgandrade/claude-superskills) | two-tier scope (installed / repo), platform detection |
| [skill-update-team](https://github.com/franktsai2008-eng/skill-update-team) | security-audit gate (6 checks), score weighting |
| [agent-self-audit](https://github.com/Xxt-XN/agent-self-audit) | Quick/Full two-tier design, auto-escalation |
| [claude-ecosystem](https://github.com/melodic-software/claude-code-plugins) | meta-skill architecture, audit agents, component health |
| [skill-optimizer](https://github.com/hqhq1025/skill-optimizer) | skill lifecycle (miner -> personalizer -> generalizer) |
| [large-codebase-audit](https://github.com/MJWNA/large-codebase-audit-skill) | multi-surface AI-layer audit, aligned with Anthropic best practices |

## Adapting to any project

Replace the variables:
```
- tech stack: Rust + C++ FFI + TypeScript/React + Docker + CMake -> {your tech stack}
- agent platform: CodeBuddy / Qoder / opencode -> {your platform}
- rules path: .agents/ -> {your rules path}
- package manager: pixi -> {your package manager}
- memory path: .agents/memorys/ -> {your memory path}
- gate list: scripts/all-gates.sh (quick 25 + heavy 7) -> {your gate list}
- entry docs: AGENTS.md + README.md -> {your entry docs}
```

## Boundary with doc-audit

- **repo-audit** (this skill): audits the **whole engineering surface** (11 domains above), inventories the
  eight governance assets (2D), and scans the **external** ecosystem for adoptable content (2B/2C).
- **doc-audit**: audits **project documentation** consistency (architecture/design/decision records) in depth;
  it already carries the "Division of labor with repo-audit" table.
- Rule of thumb: adopting/optimizing skills or rules, or a periodic whole-repo checkup -> this skill;
  docs/architecture consistency deep-dive -> `doc-audit`.
- **Governance boundary**: 3D only emits candidates, never schedules; role-count derivation belongs to the
  plan's derivation method (human + PLAN); porting execution belongs to `mechanism-porting`. repo-audit
  finds + judges + inventories + emits increments -- it does not modify files, schedule batches, or derive
  role counts. Memory ownership split with doc-audit per N38 (size+staleness here; dup+conflict+growth there).

## Rationalizations (anti-excuse)

| Excuse | Rebuttal |
|--------|----------|
| "more skills = better" | AI-slop skills dilute routing; adopt only high-fit, safe ones. Quota: net +2 per quarter max. |
| "skip the security gate, it's a popular repo" | Popularity is not safety; run the 6 checks. |
| "one quick scan is enough" | Quick misses duplicates/security; escalate to Full periodically. |
| "the gates are green, nothing to scan" | Gates prove mechanical health per surface; they do NOT prove coverage (a surface without a gate is invisible), nor naming/structure/duplication quality -- that is exactly the scan's gap analysis. |
| "audit .agents only, the rest is product code" | Scripts, CLI, tests and tree structure ARE the agent system's execution surface -- past launcher/naming/EOL pitfalls all lived outside .agents/. |

## Red Flags

- Installing a skill without the security gate.
- A skill whose frontmatter lacks name/description.
- Duplicated skills competing for the same trigger.
- No record of the scan result in memory.
- A domain in the matrix whose enforcement owner column is empty AND has no scan finding -- that is a coverage hole, not a pass.

## Verification (evidence)

- The scan report with per-domain verdict, per-item score + security verdict.
- `agent toolchain` (skill_lint) and the `agent single source` SKILLS-INDEX segment cover every skill (inventory consistent); the all-gates.sh chain green.
- 2C pattern cards cite source repo + path and pass the constitutional Fit check.
- Gap findings cite file:line and name the enforcement owner that should own them.
- The 11th domain gives correct conclusions in both empty (no `.agents/agents/`) and full states; no crash, no false positive.
- 3D sample of 5 rows: Landing + Gate columns non-empty; removing a column makes the report fail.
- A P-5 pattern card (e.g. OMO category routing) carries all six fields and its Fit cites this repo's constitution.