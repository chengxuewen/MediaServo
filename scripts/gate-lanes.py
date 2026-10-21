#!/usr/bin/env python3
"""Crate-lane reconciliation gate (S4, PLAN §4 S4).

THE problem this prevents: cmake/MediaServoCrates.cmake carries the lane table
(the single source the IDE surface is generated from). The workspace's member
list lives in Cargo.toml. If a crate is added/removed and the table is not kept
in step, the IDE silently loses (or invents) a lane — the same drift class as a
stale bridge copy.

Checks (hard failures, exit 1):
  1. Every `crates/*` workspace member appears in the table.
  2. Every table row points at an existing `crates/*` member (no ghosts).
  3. Each row's lib name is derived from the crate name (dash -> underscore;
     a deliberate mismatch must be justified, so it is treated as drift).
  4. The table is not empty.

Usage:
  python3 scripts/gate-lanes.py            # real check (run from the repo root)
  python3 scripts/gate-lanes.py --self-test # fixture: a table missing a member
                                            # MUST be rejected (detector proof)

The self-test is the fixture demanded by the S4 Done-when: "reconciliation
script FAILs on a planted missing row".
"""
from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CM = ROOT / "cmake" / "MediaServoCrates.cmake"
CARGO = ROOT / "Cargo.toml"


def table_from_text(text: str) -> list[tuple[str, str]]:
    """[(crate, lib)] from the MEDIASERVO_CRATE_LANES block."""
    m = re.search(r"set\(MEDIASERVO_CRATE_LANES(.*?)\)", text, re.S)
    if not m:
        raise SystemExit("FAIL: MEDIASERVO_CRATE_LANES block not found in cmake/MediaServoCrates.cmake")
    rows: list[tuple[str, str]] = []
    for row in re.findall(r'"([^"]+)"', m.group(1)):
        parts = row.split("@")
        if len(parts) != 3:
            raise SystemExit(f"FAIL: malformed lane row {row!r} (want crate@lib@folder)")
        rows.append((parts[0], parts[1]))
    return rows


def members() -> list[str]:
    text = CARGO.read_text(encoding="utf-8")
    ws = re.search(r"^\[workspace\](.*?)(^\[|\Z)", text, re.S | re.M)
    if not ws:
        raise SystemExit("FAIL: [workspace] section missing in Cargo.toml")
    block = ws.group(1)
    mem = re.search(r"members\s*=\s*\[(.*?)\]", block, re.S)
    if not mem:
        raise SystemExit("FAIL: workspace.members missing in Cargo.toml")
    rels = re.findall(r'"([^"]+)"', mem.group(1))
    return sorted(p.split("/", 1)[1].strip() for p in rels if p.startswith("crates/"))


def check(rows: list[tuple[str, str]], want: list[str]) -> list[str]:
    errs: list[str] = []
    if not rows:
        return ["lane table is EMPTY"]
    have = [c for c, _ in rows]
    for c in want:
        if c not in have:
            errs.append(f"member {c} has no lane row")
    for c in have:
        if c not in want:
            errs.append(f"lane row {c} is not a crates/* workspace member")
    seen = set()
    for c, lib in rows:
        if c in seen:
            errs.append(f"duplicate lane row {c}")
        seen.add(c)
        expect = c.replace("-", "_")
        if lib != expect:
            errs.append(f"lane row {c}: lib {lib!r} != crate-derived {expect!r} (justify or fix)")
    return errs


def main(argv: list[str]) -> int:
    rows = table_from_text(CM.read_text(encoding="utf-8"))
    want = members()
    if "--self-test" in argv:
        planted = [r for r in rows if r[0] != want[-1]]
        errs = check(planted, want)
        if not errs:
            print("FAIL(self-test): planted missing row was NOT detected")
            return 1
        print(f"OK(self-test): planted missing row detected ({errs[0]})")
        return 0
    errs = check(rows, want)
    if errs:
        for e in errs:
            print(f"FAIL: {e}")
        return 1
    print(f"OK: crate lanes reconciled ({len(rows)} lanes == {len(want)} crates/* members)")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
