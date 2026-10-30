#!/usr/bin/env python3
"""Relative-link gate for user-facing docs.

Scans README/AGENTS/SKILL plus architecture/module/reference/sdd docs, strips
fenced code blocks, and checks that every relative markdown link target exists.
Plans, superpowers, research archives, build/ and target/ are out of scope
(forward-looking or generated).

Usage:
    python3 scripts/agents/check_docs_links.py [--report-only] [--self-test]

Exit: 0 clean (or report-only), 1 broken links, 2 setup error.
"""

import argparse
import os
import re
import subprocess
import sys
import tempfile
import urllib.parse

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
EXCLUDE_PREFIXES = (
    "docs/plans/",
    "docs/superpowers/",
    "docs/reference/research/",
    "docs/handoff-",
)
LINK = re.compile(r"!?\[[^\]]*\]\(([^)]+)\)")
FENCE = re.compile(r"(?ms)^```.*?^```")
SKIP = re.compile(r"^(https?:|mailto:|#|<|build/|target/)")


def files():
    proc = subprocess.run(
        ["git", "ls-files", "README.md", "AGENTS.md", "SKILL.md",
         "docs/**/*.md"],
        cwd=ROOT, capture_output=True, text=True, check=False)
    found = []
    for name in proc.stdout.splitlines():
        if any(name.startswith(prefix) for prefix in EXCLUDE_PREFIXES):
            continue
        found.append(name)
    return sorted(set(found))


def allowlist():
    path = os.path.join(ROOT, "scripts", "docs-links-allow.txt")
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


def broken_links(path, allowed):
    with open(os.path.join(ROOT, path), encoding="utf-8") as handle:
        text = FENCE.sub("", handle.read())
    broken = []
    for match in LINK.finditer(text):
        dest = match.group(1).strip()
        if not dest or SKIP.match(dest):
            continue
        target = dest.split("#", 1)[0].split("?", 1)[0]
        target = urllib.parse.unquote(target).strip()
        if not target or " " in target:
            continue
        if (path, dest) in allowed:
            continue
        resolved = os.path.normpath(os.path.join(os.path.dirname(path), target))
        if not os.path.exists(os.path.join(ROOT, resolved)):
            broken.append((path, dest, resolved))
    return broken


def self_test() -> int:
    with tempfile.TemporaryDirectory(dir=os.path.join(ROOT, ".cache")) as tmp:
        good = os.path.join(tmp, "good.md")
        bad = os.path.join(tmp, "bad.md")
        open(os.path.join(tmp, "target.md"), "w").close()
        with open(good, "w", encoding="utf-8") as handle:
            handle.write("[ok](target.md)\n")
        with open(bad, "w", encoding="utf-8") as handle:
            handle.write("[missing](nope.md)\n")
        old_root = globals()["ROOT"]
        globals()["ROOT"] = tmp
        try:
            good_hits = broken_links("good.md", set())
            bad_hits = broken_links("bad.md", set())
        finally:
            globals()["ROOT"] = old_root
    if good_hits or not bad_hits:
        print(f"DOC-LINKS self-test ✗ good={good_hits} bad={bad_hits}")
        return 1
    print("DOC-LINKS self-test ✓ (good green, missing red)")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--report-only", action="store_true")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        return self_test()

    allowed = allowlist()
    scanned = files()
    broken = []
    for path in scanned:
        broken.extend(broken_links(path, allowed))
    if broken:
        print(f"DOC-LINKS {'report' if args.report_only else '✗'} — {len(broken)} broken link(s) / {len(scanned)} file(s):")
        for path, dest, resolved in broken:
            print(f"  {path} -> {dest} (resolved {resolved})")
        return 0 if args.report_only else 1
    print(f"DOC-LINKS ✓ ({len(scanned)} file(s), 0 broken links)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
