---
name: deprecation-and-migration
description: "Deprecate and migrate MediaServo surfaces safely: mark the old surface, publish a migration path, keep an alias until consumers move, then remove it with a guard. Use for Rust APIs, WS messages, REST endpoints, config keys, and UI contracts."
---

# Deprecation and Migration

> Mark, migrate, then remove. Never delete a public surface silently.

## Trigger Conditions

- Removing or renaming a Rust public API, trait, or config key.
- Changing a WS `SignalingMessage` variant or a REST payload.
- Replacing an Admin Dashboard contract or a CLI verb.
- A consumer-facing behavior needs a compatibility window.
- 用户说“弃用 / 迁移 / 移除旧接口 / 兼容窗口” <!-- c49:trigger -->

## Do Not Use

- An internal refactor with no external consumer: simplify directly.
- A bug fix that keeps the contract: use `debugging-and-error-recovery`.
- A new API design from scratch: use `api-interface-design` first.

## Migration Contract

Every deprecation needs five parts:

1. **Old surface**: exact symbol, message, field, or route.
2. **New surface**: exact replacement.
3. **Detection**: how a consumer knows it is on the old path (warning, log, metric, compile error).
4. **Window**: when removal is allowed and who owns the migration.
5. **Proof**: a test that the old path works during the window and fails after removal.

## Workflow

### 1. Inventory consumers

```bash
rg "<old_symbol_or_field>" crates www scripts docs
```

- Separate in-repo consumers from external ones.
- For WS/REST, check the protocol docs and the Admin Dashboard client.
- For config keys, check sample configs and deployment docs.

### 2. Choose the mechanism

| Surface | Mechanism |
|---|---|
| Rust API | `#[deprecated(note = "...")]`, re-export alias, or a shim |
| WS message | accept both shapes; emit the new shape; log the old one |
| REST endpoint | keep the route, add `Deprecation`/`Sunset` headers, forward |
| Config key | accept old + new; warn once; document precedence |
| CLI verb | alias to the new verb; print a one-line warning |

### 3. Publish the migration path

- Add a short migration note with before/after examples.
- Record the decision in `.agents/memorys/decisions.md` (D-series) if it changes a public contract.
- Update the architecture/protocol doc and the changelog.
- Name the removal condition; "later" is not a condition.

### 4. Implement the compatibility layer

- One source of truth for the new behavior; the old surface delegates to it.
- Do not fork the logic; a shim that drifts is worse than no shim.
- Emit a warning once per process/session, not per message.

### 5. Test both sides

- Old surface: still accepted during the window.
- New surface: the canonical path.
- Removal guard: a test or gate that fails when the old surface is reintroduced after the removal date.

### 6. Remove

- Delete the shim and the old docs in one change.
- Run the full gates; search for residuals.
- Record the removal in the same D-entry or a follow-up.

## Anti-Patterns

| Smell | Fix |
|---|---|
| Silent removal | Mark + window + migration note. |
| Forever alias | Set a removal condition and a guard. |
| Two implementations | Old delegates to new; one source of truth. |
| "Deprecated" with no replacement | Publish the new surface first. |
| Only in-repo search | Check external consumers and protocol docs. |

## Exit Criteria

- Old and new surfaces both tested.
- Migration note + decision record exist.
- Warning/detection is observable.
- Removal condition and owner are explicit.
- Gates green; no residual references without a reason.

## Boundaries

- `api-interface-design`: decides the new contract; this skill migrates consumers.
- `incremental-implementation`: slices the rollout; this skill defines compatibility.
- `documentation-and-adrs`: records the decision; this skill executes the migration.
