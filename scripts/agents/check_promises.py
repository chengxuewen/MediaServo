#!/usr/bin/env python3
"""Promise/overclaim gate for the user-facing summary docs.

Two checks:
  1. A cited repo path (docs/..., crates/..., scripts/..., book/..., www/...)
     must exist on disk.
  2. A delivered/available claim must not sit next to a future-tense marker
     (planned / in progress / coming soon / pending / roadmap).

Scope is the summary set (README, architecture, docs README), not plans or
research archives: those are allowed to reference future work.

Usage:
    python3 scripts/agents/check_promises.py [--report-only] [--self-test]
"""

import argparse
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
FILES = ("README.md", "docs/architecture.md", "docs/README.md")
PATH = re.compile(r"\b((?:docs|crates|scripts|book|www)/[A-Za-z0-9_./-]+\.(?:md|rs|toml|json|py|sh|ts|tsx|yml|yaml))\b")
CLAIM = re.compile(
    r"(?i)\b(?:delivered|shipped|available|supported|complete|implemented)\b"
    r"[^.\n]{0,20}\b(?:planned|in progress|coming soon|pending|roadmap)\b")


def allowlist():
    path = os.path.join(ROOT, "scripts", "promises-allow.txt")
    allowed = set()
    if not os.path.isfile(path):
        return allowed
    with open(path, encoding="utf-8") as handle:
        for line in handle:
            line = line.strip()
            if not line or line.startswith("#"):
                continue
            parts = line.split("|")
            if len(parts) >= 2:
                allowed.add((parts[0], parts[1]))
    return allowed


def check(path, allowed):
    with open(os.path.join(ROOT, path), encoding="utf-8") as handle:
        text = handle.read()
    findings = []
    for match in PATH.finditer(text):
        target = match.group(1)
        if (path, target) in allowed:
            continue
        if not os.path.exists(os.path.join(ROOT, target)):
            findings.append((path, "dangling", target))
    for line_no, line in enumerate(text.splitlines(), 1):
        for match in CLAIM.finditer(line):
            token = match.group(0)
            if any(p == path and t in line for p, t in allowed):
                continue
            findings.append((path, "overclaim", f"{line_no}: {token}"))
    return findings


def self_test() -> int:
    import tempfile
    with tempfile.TemporaryDirectory(dir=os.path.join(ROOT, ".cache")) as tmp:
        path = os.path.join(tmp, "README.md")
        with open(path, "w", encoding="utf-8") as handle:
            handle.write("Feature is delivered, but it is planned for next quarter.\n")
            handle.write("See docs/not-there.md.\n")
        old_root = globals()["ROOT"]
        globals()["ROOT"] = tmp
        try:
            findings = check("README.md", set())
        finally:
            globals()["ROOT"] = old_root
    kinds = {item[1] for item in findings}
    if kinds != {"dangling", "overclaim"}:
        print(f"PROMISES self-test ✗ findings={findings}")
        return 1
    print("PROMISES self-test ✓ (overclaim + dangling detected)")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--report-only", action="store_true")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        return self_test()

    allowed = allowlist()
    findings = []
    for path in FILES:
        if os.path.isfile(os.path.join(ROOT, path)):
            findings.extend(check(path, allowed))
    if findings:
        print(f"PROMISES {'report' if args.report_only else '✗'} — {len(findings)} finding(s):")
        for path, kind, detail in findings:
            print(f"  {path} [{kind}] {detail}")
        return 0 if args.report_only else 1
    print(f"PROMISES ✓ ({len(FILES)} summary file(s), 0 findings)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
