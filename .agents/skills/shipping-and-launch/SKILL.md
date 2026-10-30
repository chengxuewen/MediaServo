---
name: shipping-and-launch
description: "MediaServo release checklist: full gates, version/tag policy, changelog decision, artifact and submodule pin verification, staged rollout, and rollback. Use before a release, tag, or production handoff."
---

# Shipping and Launch

> Ship only what you can verify and roll back.

## Trigger Conditions

- Preparing a version, tag, or production handoff.
- A release candidate needs a final gate and a rollback plan.
- The changelog/version/artifact set must be frozen.
- 用户说“发布 / 上线 / 发版 / 打 tag / 回滚” <!-- c49:trigger -->

## Do Not Use

- A normal feature PR: use `code-review-and-quality`.
- Host/deployment topology changes: use the host-ops workflow.
- A hotfix under incident: use `debugging-and-error-recovery` first.

## Pre-Flight Checklist

| Check | Command / evidence |
|---|---|
| Focused gates | `pixi run check` / `pixi run test` |
| Full gate | `pixi run all-gates` |
| Agent layer | `pixi run agent-tools` |
| MCP | `pixi run gate-mcp` (3 servers, no orphans) |
| Docs/book | `pixi run book-check` |
| Protocol | `pixi run gate-openapi` / WS contract tests |
| Weak network | the agreed profile/scenario, if the release changes media behavior |
| Tree state | clean worktree; no untracked artifacts |
| Submodule pin | the pointer matches the intended commit |
| Changelog | entry added or an explicit "no user-facing change" |
| Version | follows the project policy; no tag from a dirty tree |

## Release Workflow

1. **Freeze the scope.** List the user-visible changes and the non-goals.
2. **Run the full gates.** Record the exact commands and results.
3. **Verify the artifact.** Build the release artifact; check names, checksums, and the embedded version.
4. **Verify the pin.** The workspace pointer and the artifact come from the same commit.
5. **Write the changelog.** User-facing changes, migrations, known issues.
6. **Tag.** Annotated tag on the verified commit; push only after the checks are green.
7. **Stage the rollout.** Canary or a small cohort first; define the success signal.
8. **Watch the signals.** Error rate, latency, disconnect/stall rate, and resource saturation.
9. **Rollback if needed.** Revert to the previous tag/artifact; do not debug forward on a broken release.

## Rollback Plan

- Previous known-good version/tag and artifact location.
- Data/schema compatibility: can the old version read the new state?
- Config compatibility: old keys still accepted?
- Owner and decision threshold (who calls the rollback, at what signal).
- Post-rollback verification and communication.

## Changelog Rules

- Group by user impact, not by commit.
- Call out breaking changes and migration steps.
- Link the decision/issue; do not paste the diff.
- "No user-facing change" is a valid entry when true.

## Anti-Patterns

| Smell | Fix |
|---|---|
| Tag from a dirty tree | Commit or stash; verify the tag points to the tested commit. |
| "Gates passed yesterday" | Run them on the release commit. |
| Changelog generated from commit titles | Write for users; group by impact. |
| No rollback path | Define it before the tag. |
| Canary with no success signal | Define the metric and threshold first. |
| Artifact and pin from different commits | Rebuild from the tagged commit. |

## Exit Criteria

- Full gates green on the release commit.
- Artifact, pin, and tag agree.
- Changelog and migration notes are present.
- Rollback is documented and tested where possible.
- Post-release signals and owner are named.

## Boundaries

- `ci-cd-automation`: pipeline mechanics; this skill owns the release decision.
- `security-hardening`: security sign-off before a public release.
- `documentation-and-adrs`: records the release decision and migration notes.
