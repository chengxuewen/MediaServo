# Reuse-First: Consumer Rules (MediaServo projection)

> Projection of the reuse-first charter (workspace root: `docs/governance/reuse-first-charter.md`; ledger: parent C60).
> Self-contained: readable in a standalone MediaServo clone. PolyOrch paths below are relative to this repo's root (submodule `PolyOrch/`).

## The rule

When the same capability shows up a second time, adjudicate "where does this live" before writing it twice. Default precedence:

**PolyOrch supplier face > MediaServo common layer > local copy**

If a PolyOrch face already provides it, hand-written / ad-hoc / improvised equivalents in this repo are forbidden (user ruling 2026-11-05: no self-building, no hand-writing, no improvising in consumer repos).

## What "a supplier face exists" looks like

- CMake helpers: 82 public `polyorch_*` functions under `PolyOrch/cmake/` (count as of 2026-11-05).
- VSCode rows (launch/tasks): generated inside `__POLYORCH_GENERATED__` managed regions — one writer per region, user-owned rows live above the BEGIN marker and are never rewritten.
- Typical mappings: c/cpp executables → `polyorch_native_debug`; python scripts → `polyorch_python_run` (SCRIPT-based, no executable target needed); ts/js → node faces; rust → rust faces.

## Pre-flight (mandatory before claiming "the infrastructure does not exist")

From this repo's root:

```bash
grep -rh '^function(polyorch' PolyOrch/cmake/*.cmake | wc -l
ls PolyOrch/tests/
```

A failed search reports **"not found"** — never **"does not exist"**. (PIT-246: that claim was made twice without ever listing the inventory; both plans then reserved hand-written slots the generator family already covered.)

## Post-hoc audit

Config rows must not land outside managed-marker systems:

```bash
grep -c 'POLYORCH_MANAGED' .vscode/*.json    # legacy hand-written marker family; measured 2026-11-05: launch=2, tasks=2; expect 0 after migration
```

Match loosely: the real markers are `__POLYORCH_MANAGED_BEGIN__` / `__POLYORCH_MANAGED_END__`; an exact grep for `__POLYORCH_MANAGED__` never matches and silently disables the audit (PIT-247).

## Exceptions

- One-shot probes / verification scripts (lifetime under one session).
- No supplier face yet: the 2nd occurrence may stay duplicated if registered in the plan ("known 2nd occurrence"); the 3rd occurrence forces abstraction and folds occurrences 1-2 back in (Rule of Three — premature unification is its own bug class).

## Escalation

Same-class failure twice → the rule graduates one layer: memory note → ledger entry with a check command → CI lint (managed-marker gate). Every recurrence names the layer that failed to catch it.
