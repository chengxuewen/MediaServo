---
name: lesson-review
description: "Batch session review: systematically extract lessons learned and write them to project memory. Use after long debugging sessions (>1h or >3 failed attempts), when user corrects multiple errors, after build fixes, on '总结经验'/'更新记忆'/'记录教训'/'回顾会话' (summarize experience / update memory / record lessons / review session), or /lesson-review command." # c49:trigger
---

# lesson-review — Batch Lesson Harvest

## Trigger Conditions

- User says "总结经验" (summarize experience) / "更新记忆" (update memory) / "记录教训" (record lessons) / "回顾会话" (review session) <!-- c49:trigger -->
- A long debugging session ends (>1h or >3 failed attempts)
- After the user points out multiple errors
- After solving a problem that took >30min
- The `/lesson-review` command

## Relationship with Rules and Skills

```
think-before-act  →  [action]  →  lesson-memory  →  doc-audit
   (consult)                      (instant capture)    (periodic audit)
                    lesson-review ← batch gap-fill
```

| | `lesson-memory` rule (C9) | `lesson-review` skill |
|---|---|---|
| Timing | instant (after every error, auto-triggered) | batch (session end / user-triggered) |
| Mode | reflex | interactive review |
| Granularity | single lesson | whole-session scan |
| Complementary | first line of defense (miss nothing) | second line of defense (no misjudgment + categorization) |

- `think-before-act` (consult): check existing lessons before acting, avoid repeating mistakes
- `lesson-memory` (C9 rule): writes **instantly and automatically** after every error, no user request needed
- `lesson-review` (this skill): **batch review** at session end — fill gaps, categorize, archive
- `doc-audit` (audit): periodically checks memory-file self-consistency

## Flow

### Step 1: Scan the Session

Review which moments in this session triggered the rules but may have been missed:

- compile/build failure
- >1 failed attempt before locating root cause
- user corrected your approach/preference
- unexpected discovery ("so that's why", "didn't expect that")
- a problem that took >30min
- post-edit syntax damage (unbalanced braces, duplicated lines, etc.)

### Step 2: Extract Item by Item (5-question checklist)

For each finding, complete every item:

1. **What went wrong?** (symptom)
2. **Why did it go wrong?** (root cause)
3. **Correct approach?** (solution)
4. **How to prevent it?** (check command / constraint)
5. **Where to store it?** (per the storage-target table)

### Step 3: Write

Write using the target file's template format.

- Important lesson → full template (symptom + root cause + solution, three elements)
- Trivial fix (1 line / no branch / no side effects) → 1-line capture, no full template needed

### Step 4: Verify

- [ ] every pitfall has a verify field (check command — how to confirm the fix)
- [ ] every convention is grep/lint-verifiable
- [ ] no duplicates with existing entries (grep the target file to confirm)
- [ ] decisions.md numbering is contiguous (no skipped numbers)
- [ ] cross-references are correct (e.g. pitfalls referencing conventions)
- [ ] high-frequency / high-cost lessons flagged for CI-gate upgrade if needed

## Output Format

```markdown
## Session Experience Summary — YYYY-MM-DD

### Recorded (N items)
1. [title] → pitfalls.md
2. [title] → conventions.md
...

### Not recorded (no need)
- [reason: one-off issue / already recorded / environment-specific]

### Upgrade suggestions
- [a lesson suggesting a CI gate, rationale: repeated 3+ times / cost >1h]
```

## Storage Targets (per project config)

Applies to any project — just change the paths:

```
- tech pitfalls  → {pitfall_log}      # MediaServo: .agents/memorys/pitfalls.md
- dev constraints → {conventions}      # MediaServo: .agents/memorys/conventions.md
- arch decisions → {decisions}         # MediaServo: .agents/memorys/decisions.md
- executable checks → {checks}         # MediaServo: .agents/rules/common/edit-safety.md
- test requirements → {test_rules}     # MediaServo: .agents/rules/common/testing.md
- security rules → {security_rules}    # MediaServo: .agents/rules/common/security.md
- rust style     → {rust_style}        # MediaServo: .agents/rules/rust/coding-style.md
- project status → {status}            # MediaServo: .agents/memorys/status.md
```
