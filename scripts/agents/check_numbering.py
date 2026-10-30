#!/usr/bin/env python3
"""Numbering gate: memory high-water marks and D1-D<n> claims must agree."""
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
MEM = os.path.join(ROOT, ".agents", "memorys")


def numbers(path, pattern):
    if not os.path.isfile(path):
        return []
    text = open(path, encoding="utf-8").read()
    return [int(m.group(1)) for m in re.finditer(pattern, text, re.M)]


def main():
    decisions = numbers(os.path.join(MEM, "decisions.md"), r"^## D(\d+)")
    pitfalls = numbers(os.path.join(MEM, "pitfalls.md"), r"^## PIT-(\d+)")
    conventions = numbers(os.path.join(MEM, "conventions.md"), r"^## C(\d+)")
    max_d = max(decisions) if decisions else 0
    fail = 0
    if len(decisions) != len(set(decisions)):
        print("NUMBERING ✗ duplicate D headings")
        fail = 1
    # The hub carries the live high-water claim. status.md is an append-only
    # historical log (C45): its D1-D190 references describe an archived file and
    # must not be read as a current marker, so only explicit "high-water" lines
    # are checked there.
    claims = set()
    hub = os.path.join(ROOT, "AGENTS.md")
    if os.path.isfile(hub):
        for match in re.finditer(r"D1[-–]D(\d+)", open(hub, encoding="utf-8").read()):
            claims.add(int(match.group(1)))
    status = os.path.join(MEM, "status.md")
    if os.path.isfile(status):
        text = open(status, encoding="utf-8").read()
        for match in re.finditer(
                r"high[- ]water(?:\s+(?:today|now|current))?\s*(?:[:=]\s*)?D(\d+)",
                text, re.I):
            claims.add(int(match.group(1)))
    if claims and claims != {max_d}:
        print("NUMBERING ✗ D1-D<n> claims %s != max D%d" % (sorted(claims), max_d))
        fail = 1
    print("NUMBERING %s D%d / PIT-%d / C%d (claims %s)" % (
        "✓" if fail == 0 else "✗", max_d,
        max(pitfalls) if pitfalls else 0,
        max(conventions) if conventions else 0,
        sorted(claims) or "none"))
    return 1 if fail else 0


if __name__ == "__main__":
    sys.exit(main())
