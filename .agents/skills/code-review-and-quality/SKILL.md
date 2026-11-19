---
name: code-review-and-quality
description: "Review MediaServo changes on five axes (correctness, security, maintainability, tests, docs/ops), label findings Nit/Optional/FYI/Blocking, keep diffs near 100 lines, and run an adversarial second pass. Use before merge or when asked to review a diff."
---

# Code Review and Quality

> Review for the failure the author did not imagine.

## Trigger Conditions

- A PR or local diff is ready for review.
- A change touches a public contract, a gate, or a cross-crate boundary.
- You need a merge verdict, not a style opinion.
- 用户说“代码评审 / 审查这个改动 / 质量把关 / 帮我看 diff” <!-- c49:trigger -->

## Do Not Use

- Security-only audit: use `security-hardening` for the deep pass.
- Complexity cleanup: use `code-simplification`.
- A one-line typo: inspect and merge; no ceremony.

## Change-type -> lens routing

Route by WHAT the change touches; the five axes stay the core, lens skills add
depth. Mixed or unclear changes run EVERY matching lens (the fallback row is
mandatory, not optional).

| Change touches | Lens (depth) |
|---|---|
| `docs/**` prose/docs | doc-audit (semantic consistency); links/numbering are gates |
| `.agents/**` skills/rules | repo-audit (mechanical) + doc-audit (semantic) |
| `www/**` frontend/UI | ui-ux-review (token changes mount design-system) |
| Security-sensitive (auth/PSK/JWT/crypto/security rules) | security-hardening |
| Performance-sensitive (media pipeline/latency) | performance-optimization |
| Network/transport (webrtc/signaling/ws) | weaknet-testing (when measuring) |
| **Mixed / unclear** | **All lenses, one pass each (fallback)** |

## Five Axes

1. **Correctness**: does it do what it claims, including error paths and concurrency?
2. **Security**: untrusted input, auth boundaries, secrets, injection, unsafe/FFI.
3. **Maintainability**: naming, layering, duplication, dead code, rule-of-500.
4. **Tests**: behavior pinned, red-green evidence, deterministic, no weakened assertions.
5. **Docs/ops**: contract docs, migration notes, observability, rollback.

## Severity Labels

| Label | Meaning | Merge effect |
|---|---|---|
| **Blocking** | Bug, security issue, data loss, contract break | Must fix |
| **Optional** | Real improvement, not required for correctness | Author decides |
| **Nit** | Style or wording; linter should own it | Non-blocking |
| **FYI** | Context or future risk | No action required |

Do not inflate nits into blockers; do not bury a blocker among nits.

## Review Process

1. **Read the claim.** What does the change say it does? What is out of scope?
2. **Read the diff, then the surrounding code.** A diff without context hides regressions.
3. **Check the contract.** Public API, WS/REST shape, config, FFI, error codes.
4. **Run the gates if available.**
   ```bash
   pixi run check
   pixi run test
   pixi run ci
   ```
5. **Adversarial second pass.** Assume the change is wrong: find the input, timing, or lifecycle that breaks it.
6. **Check the tests.** Would they fail on the parent commit? Are they deterministic?
7. **Write findings with evidence.** File/line, scenario, impact, suggested direction.
8. **Give a verdict.** Approve, approve with optional fixes, or request changes.

## Diff Discipline

- Around 100 lines is a reviewable unit; larger changes need a reason and a split.
- Generated files are reviewed at their source, not line by line.
- A refactor mixed with a behavior change is two changes; ask for a split.
- No drive-by formatting in a functional diff.

## Output Template

```text
Summary: <one line: what was reviewed and the scope>
Critical (Blocking):
| # | Axis | File:line | Scenario | Impact | Fix direction |
Suggestions:
| # | Axis | Severity (Optional/Nit/FYI) | File:line | Suggestion |
What Looks Good:
- <at least one concrete strength; a report with zero positives is not a verdict>
Uncovered:
- <what was NOT reviewed and why> (write "none" only if truly nothing)
Verdict: Approve | Request Changes | Needs Discussion
Tests/gates: <what ran, what did not>
Residual risk: <one sentence>
```

Verdict semantics: **Approve** (mergeable; optional notes are advisory),
**Request Changes** (blocking findings present), **Needs Discussion**
(direction genuinely uncertain -- name the open question in one line instead
of forcing a side). User-template variant: when the user explicitly asks for
a report template, follow theirs; keep the Verdict and evidence fields.

## Anti-Patterns

| Smell | Fix |
|---|---|
| "LGTM" with no evidence | State what you read and ran. |
| Style nits as blockers | Let the linter own style. |
| Reviewing only the happy path | Check errors, cancellation, teardown. |
| Ignoring generated drift | Review the generator/source. |
| Approving without tests | Ask for the red/green evidence. |
| One giant diff | Split by contract or layer. |

## Exit Criteria

- Every finding has a severity and evidence.
- Tests/gates status is explicit.
- Verdict matches the findings.
- No unresolved blocker is hidden in prose.

## Boundaries

- `security-hardening`: deep security audit; this skill covers the security axis at review depth.
- `code-simplification`: complexity reduction; this skill flags it.
- `test-driven-development`: proves one behavior; this skill checks the proof.
