# Development Workflow

The Feature Implementation Workflow describes the development pipeline: research, planning, TDD, code review, and then committing to git.

## Feature Implementation Workflow

0. **Research & Reuse** _(mandatory before any new implementation)_
   - **GitHub code search first:** Run `gh search repos` and `gh search code` to find existing implementations, templates, and patterns before writing anything new.
   - **Library docs second:** Use Context7 or primary vendor docs to confirm API behavior, package usage, and version-specific details before implementing.
   - **Exa only when the first two are insufficient:** Use Exa for broader web research or discovery after GitHub search and primary docs.
   - **Check package registries:** Search npm, PyPI, crates.io, and other registries before writing utility code. Prefer battle-tested libraries over hand-rolled solutions.
   - **Search for adaptable implementations:** Look for open-source projects that solve 80%+ of the problem and can be forked, ported, or wrapped.
   - Prefer adopting or porting a proven approach over writing net-new code when it meets the requirement.

1. **Plan First**
   - Use **planner** agent to create implementation plan
   - Generate planning docs before coding: PRD, architecture, system_design, tech_doc, task_list
   - Identify dependencies and risks
   - Break down into phases
   - Present the plan to the user

1.5 **Execution Confirmation Gate** _(mandatory — NEVER skip)_
   - **MUST get explicit user confirmation** before executing ANY plan or todo list:
     - Use the `question` tool with a "Proceed with execution?" prompt (interactive confirm)
     - Wait for user's affirmative response ("\u786e\u8ba4", "\u6267\u884c", "yes", "proceed")
   - **What requires confirmation**:
     - Creating or deleting crates/workspace members
     - Moving modules between crates
     - Changing directory structures
     - Altering Cargo.toml features (adding/removing features, deps)
     - Modifying CMake build infrastructure
     - Any multi-step todo list execution
   - **What does NOT count as confirmation**:
     - System directives (TODO CONTINUATION, SYSTEM REMINDER)
     - The word "\u7ee7\u7eed" (continue) \u2014 means "continue discussing" or "continue with trivial/safe work"
     - Silence or timeout
   - **Process**: Present plan → display confirmation prompt → wait for user → execute only after explicit affirmative

2. **TDD Approach**
   - Use **tdd-guide** agent
   - Write tests first (RED)
   - Implement to pass tests (GREEN)
   - Refactor (IMPROVE)
   - Verify 80%+ coverage

3. **Code Review**
   - Use **code-reviewer** agent immediately after writing code
   - Address CRITICAL and HIGH issues
   - Fix MEDIUM issues when possible

4. **Commit & Push**
   - Detailed commit messages following conventional commits format (see below)
   - Follow the Pull Request Workflow (see below)

5. **Pre-Review Checks**
   - Verify all automated checks (CI/CD) are passing
   - Resolve any merge conflicts
   - Ensure branch is up to date with target branch
   - Only request review after these checks pass

## Commit Message Format
```
<type>(<scope>): <summary>

- <one fact per bullet, wrapped at 72 chars>
- <continuation lines indented 2 spaces>

<optional trailers, e.g. Change-Id>
```

Hard rules for the first line:
- **50 chars advised / 72 hard limit**.
- Imperative, present tense ("add", not "added"/"adds").
- No trailing period.
- One thing per commit: no chain symbols (`+`, ` + `, and-chains) stitching
  multiple changes into one line -- split into separate commits instead.

Hard rules for the body (machine-checked by the shell `commit-lint`:
hook layer for the commit being born + gate layer for anchor..HEAD):
- Blank line between subject and body.
- **List, not prose**: a body longer than one short line (<= 72 chars)
  must be a bullet list -- every line starts with `- ` (or `* ` / `1. `)
  or is a 2-space-indented continuation of the preceding bullet.
  One fact per bullet; no dense multi-fact paragraphs.
- **72-char hard wrap** on every body line. Trailer lines
  (`Change-Id:`, `Bug:`, ...) are exempt and go last.
- Merge / fixup! / squash! subjects skip the lint (rebase plumbing).

Types: feat, fix, refactor, docs, test, chore, perf, ci

Note: Attribution disabled globally via ~/.claude/settings.json.

## Pull Request Workflow

When creating PRs:
1. Analyze full commit history (not just latest commit)
2. Use `git diff [base-branch]...HEAD` to see all changes
3. Draft comprehensive PR summary
4. Include test plan with TODOs
5. Push with `-u` flag if new branch

## Governed Self-Evolution Protocol

Governance changes follow the governed self-evolution loop (plan section 3.18,
B11): evolution has PROPOSAL rights, never execution rights.

**Quarterly cadence (the quarterly rotation)**: one cycle per quarter -- external
radar (community scan) -> internal inventory (repo-audit 2D) -> proposal cards
(capped at 5 per quarter) -> user ruling -> adversarial review
(`expert-panel-review`) -> land one batch of at most 2 skills.

**Event triggers** (any one opens a proposal card immediately, off-cycle):
a gate going red; a quota overage; a host dialect change; a donor shipping a new
mechanism; the same lesson recurring 2+ times.

1. **Proposal card**: 2-3 options + a recommendation + source evidence + a
   strategy tag {harden, repair-only, innovate} (default harden; innovate must
   name the new judgment path) + a pre-registered recheck criterion that only
   cites existing gates/audit rows -- inventing metrics is a violation.
2. **User ruling**: the card is decided via the plan's N-table; silence or
   "continue" is not approval; no ruling -> no port.
3. **Citation requirement**: an evolution batch's start gate must cite the ruled
   N-number. A batch that cannot cite its ruling is refused at start -- running
   a proposal without a recorded verdict bypasses the loop and is treated as a
   gate bypass.
4. **Gated execution**: the ruled card lands as a batch with landing + gate
   columns; evolution batches pass the SAME gate chain as regular batches.
5. **Solidify two-phase**: experiment drafts live in `evidence/` (verify, then
   delete); production consolidation requires the ruled batch + gate + rollback
   anchor. An "evolution" label never skips the draft phase.
6. **Worktree merge-back** (N39): an evolution batch worked in a worktree
   rechecks its touch surface and bumps the pointer in the SAME commit as the
   merge; the start gate is never waived for worktree work.
7. **Recheck**: the pre-registered criterion decides closure; unclosed gaps go
   back to `lesson-review` or the next quarter's proposal queue (gap closure
   rate, repo-audit 3C). Quota: net +2 skills per quarter; a replacement (retire
   one, add one) does not consume the net quota.
