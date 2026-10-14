# Lesson Memory & Self-Correction Rule

> When the AI encounters a mistake, a new problem, or a new experience, it **proactively** writes the lesson into project memory and rule files.

## Trigger conditions (any one triggers)

| Trigger scenario | Action |
|----------|------|
| Compile/build fails after editing code | Record in `pitfalls.md` + update `edit-safety.md` |
| Same class of error appears a 2nd time | **Immediately** add a rule to `edit-safety.md` or create a new rule file |
| A community solution is found | Record in `pitfalls.md` (with community link) + update `decisions.md` |
| Tests pass but the feature doesn't work | Record in `pitfalls.md` + update `testing.md` to add E2E requirements |
| Dependency version conflicts | Record in `pitfalls.md` + update `edit-safety.md` |
| Patches/scripts break build artifacts | Record in `pitfalls.md` + update `edit-safety.md` |
| User points out an issue the AI missed | Record in `pitfalls.md` + analyze root cause + add a prevention rule |
| Root cause found only after >1 failed attempt | Record in `pitfalls.md` (including failed attempts + correct path) |
| User corrects an approach/preference | Record in `conventions.md` or `decisions.md` |

## Write targets

| File | Content written | Format |
|------|---------|------|
| `.agents/memorys/pitfalls.md` | Problem + cause + solution + verification + prohibition | Five-section form (PIT-{n}) |
| `.agents/memorys/decisions.md` | Architecture/technical decisions + rationale + references | `## D{N}: title` |
| `.agents/memorys/conventions.md` | Development constraints/user preferences | C{n}: "constraint" + check command |
| `.agents/rules/common/edit-safety.md` | Executable check rules | `### N. rule name` + check command + blocking condition |
| `.agents/rules/common/testing.md` | Test coverage requirements | Test types + execution commands + pass criteria |

## When to write

1. **Immediate write**: record the lesson at the moment the problem is found, don't wait for session end
2. **Verify after fixing**: after writing, verify the rule is executable (has concrete commands, not empty talk)
3. **Dedup check**: `grep` the target file before writing to confirm no duplicate

## Rule quality standard

| Standard | Description |
|------|------|
| **Executable** | Contains concrete shell commands or check steps, not "be careful about XXX" |
| **Verifiable** | Has explicit pass/fail criteria |
| **Blocking** | Clearly states what to do when it doesn't pass |
| **Has precedent** | References errors that actually occurred |

## Anti-patterns (forbidden)

| Anti-pattern | Correct practice |
|--------|---------|
| "We'll watch out next time" | Write a concrete check command into the rule file |
| Record without preventing | Every pitfall must map to one executable rule |
| Rule says "edit carefully" | Write `grep -c '{' file && grep -c '}' file` |
| Waiting for the user to remind before recording | Record on discovery (C9), don't wait for the user to point it out |

## Capture checklist (self-ask on every trigger)

1. **What went wrong?** (symptom)
2. **Why did it go wrong?** (root cause)
3. **What is the correct approach?** (solution)
4. **How to prevent it?** (check command / constraint)
5. **Where does it get stored?** (per the write-targets table)

## Trivial-fix escape hatch

1-line fix / no branches / no side effects → 1-line capture, no full template needed.

## Pitfall template (required content when writing to pitfalls.md)

```markdown
## PIT-{n}: Title (date)
- **Symptom**: [description of what was observed]
- **Root cause**: [root-cause analysis]
- **Solution**: [the correct approach]
- **Verification**: [check command]
```

## Prevention escalation (high-cost / high-frequency lessons)

- Repeated ≥2 times or >3 rounds spent locating → add a CI gate or pre-commit hook
- Low frequency / low cost → record in pitfalls.md only
