# Development Constraints

> This is a stub file. Platform and Docker constraints have been split out per D202.
>
> - [docker.md](docker.md) — Docker & Network constraints (volume mounts, UDP ports, ICE/STUN)
> - [platform.md](platform.md) — Platform constraints (macOS/Linux, mediasoup, CI)

## Git Commit Rules

### Cargo.lock Must Be Committed
**ALWAYS** commit `Cargo.lock` along with dependency changes. This file tracks exact dependency versions and must be in sync with `Cargo.toml`.

Common mistake: Forgetting to `git add Cargo.lock` after `Cargo.toml` changes. This causes build failures for other developers.

**Checklist before committing:**
- [ ] `Cargo.toml` changes committed
- [ ] `Cargo.lock` changes committed (if dependencies changed)
- [ ] `git status` shows clean working tree

<!-- BEGIN:always-on-core -->

## Always-On Core (bottom-line constraints; B10, O-29 ruled A)

The five constraints below are the always-on layer: rendered into every host's
resident context (CodeBuddy `alwaysApply`; opencode/Qoder via instructions/rules
faces). The generator reads THIS table -- edit here, never in a host directory.

| # | Constraint | Why always-on |
|---|------------|---------------|
| 1 | Verify immediately after every change; on failure STOP, re-read, restore -- never stack a fix on an unverified failure. | PIT-41: the largest measured token-waste source. |
| 2 | Never weaken or bypass a gate to make a change pass. | Gates are the constitution; weakening one hides the failure it was built to catch. |
| 3 | Never hand-edit a generated layer; edit the true source, then re-sync. | Hand edits are silently overwritten and corrupt the projection contract. |
| 4 | No batch starts without the user's explicit authorization; silence or "continue" is not approval. | Stage gates prevent scope creep and cross-stage mixing. |
| 5 | Every conclusion carries evidence (file:line or a reproducible command); unproven claims are labelled speculation. | The evidence standard every review and audit role depends on. |

<!-- END:always-on-core -->
