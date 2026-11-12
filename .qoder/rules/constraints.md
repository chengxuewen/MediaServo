# Always-on core constraints

> Generated from `rules/common/constraints.md`; edit the true source.

1. Verify immediately after every change; on failure STOP, re-read, restore -- never stack a fix on an unverified failure.
2. Never weaken or bypass a gate to make a change pass.
3. Never hand-edit a generated layer; edit the true source, then re-sync.
4. No batch starts without the user's explicit authorization; silence or "continue" is not approval.
5. Every conclusion carries evidence (file:line or a reproducible command); unproven claims are labelled speculation.
