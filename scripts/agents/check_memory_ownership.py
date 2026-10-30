#!/usr/bin/env python3
"""Cross-repo memory gate: active budget, child index, citations, duplicates.

Runs from the submodule (H5) against a parent root:

    python3 scripts/agents/check_memory_ownership.py --parent-root <parent>

Checks:
  - parent active ledger (status/decisions/pitfalls/conventions/child-refs) <= 58 KiB;
  - child-refs.md exists and its D/PIT/C high-water matches the child memorys;
  - every `child:D..` / `child:PIT-..` / `child:C..` citation resolves;
  - archive files exist (nothing was silently deleted);
  - parent/child section titles do not duplicate (owner prefix + title normalized).
"""

import argparse
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
CHILD_MEM = os.path.join(ROOT, ".agents", "memorys")
ACTIVE = ("status.md", "decisions.md", "pitfalls.md", "conventions.md", "child-refs.md")
ARCHIVES = ("status-archived.md", "decisions-archived.md", "pitfalls-archived.md", "conventions-archived.md")
LIMIT = 58 * 1024
SECTION = re.compile(r"(?m)^## (D\d+|PIT-\d+|C\d+):\s*(.+)$")
CITATION = re.compile(r"child:(D\d+|PIT-\d+|C\d+)")
REF_ROW = re.compile(r"\|\s*(decisions|pitfalls|conventions)\.md\s*\|\s*(D\d+|PIT-\d+|C\d+)\s*\|")


def read(path):
    with open(path, encoding="utf-8") as handle:
        return handle.read()


def titles(path):
    text = read(path)
    return [(match.group(1), match.group(2)) for match in SECTION.finditer(text)]


def memory_sets(directory):
    sets = {"D": set(), "PIT": set(), "C": set()}
    for name in ("decisions.md", "pitfalls.md", "conventions.md",
                 "decisions-archived.md", "pitfalls-archived.md", "conventions-archived.md"):
        path = os.path.join(directory, name)
        if not os.path.isfile(path):
            continue
        for key, _title in titles(path):
            value = int(re.sub(r"\D", "", key))
            if key.startswith("D"):
                sets["D"].add(value)
            elif key.startswith("PIT"):
                sets["PIT"].add(value)
            else:
                sets["C"].add(value)
    return sets


def missing_citations(text, sets):
    missing = []
    for match in CITATION.finditer(text):
        token = match.group(1)
        value = int(re.sub(r"\D", "", token))
        kind = "D" if token.startswith("D") else "PIT" if token.startswith("PIT") else "C"
        if value not in sets[kind]:
            missing.append("child:" + token)
    return missing


def duplicate_titles(parent, child):
    return sorted(set(parent) & set(child))


def normalize(title):
    title = title.replace("`", "").replace("*", "")
    title = re.sub(r"^[DCP][A-Z]*-?\d+[:：]\s*", "", title)
    title = re.sub(r"[\s\u3000:：,，.。;；()（）—\-_/]+", "", title)
    return title.lower()


def self_test():
    sets = {"D": {297}, "PIT": {237}, "C": {52}}
    missing = missing_citations("child:D297 child:C52 child:D999", sets)
    if missing != ["child:D999"]:
        print("MEMORY-OWNERSHIP self-test ✗ citation miss not detected")
        return 1
    if normalize("D1: Foo bar") != normalize("D9: Foo-bar"):
        print("MEMORY-OWNERSHIP self-test ✗ title normalization")
        return 1
    if not duplicate_titles({"foobar": "a"}, {"foobar": "b"}):
        print("MEMORY-OWNERSHIP self-test ✗ duplicate miss")
        return 1
    print("MEMORY-OWNERSHIP self-test ✓ (citation, normalization, duplicates)")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--parent-root", required=False)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        return self_test()
    if not args.parent_root:
        print("usage: check_memory_ownership.py --parent-root <parent>", file=sys.stderr)
        return 2

    parent = os.path.abspath(args.parent_root)
    mem = os.path.join(parent, ".agents", "memorys")
    failures = 0

    total = 0
    for name in ACTIVE:
        path = os.path.join(mem, name)
        if not os.path.isfile(path):
            print(f"MEMORY-OWNERSHIP ✗ missing {name}")
            failures += 1
            continue
        size = os.path.getsize(path)
        total += size
        print(f"  {name:18s} {size:7d} B")
    if total > LIMIT:
        print(f"MEMORY-OWNERSHIP ✗ active {total} B > {LIMIT} B")
        failures += 1
    else:
        print(f"MEMORY-OWNERSHIP ✓ active {total} B <= {LIMIT} B")

    for name in ARCHIVES:
        if not os.path.isfile(os.path.join(mem, name)):
            print(f"MEMORY-OWNERSHIP ✗ missing archive {name}")
            failures += 1

    sets = memory_sets(CHILD_MEM)
    refs_path = os.path.join(mem, "child-refs.md")
    if os.path.isfile(refs_path):
        refs_text = read(refs_path)
        expected = {
            "decisions": "D%d" % (max(sets["D"]) if sets["D"] else 0),
            "pitfalls": "PIT-%d" % (max(sets["PIT"]) if sets["PIT"] else 0),
            "conventions": "C%d" % (max(sets["C"]) if sets["C"] else 0),
        }
        found = {}
        for match in REF_ROW.finditer(refs_text):
            found[match.group(1)] = match.group(2)
        for ledger, want in expected.items():
            got = found.get(ledger)
            if got != want:
                print(f"MEMORY-OWNERSHIP ✗ child-refs {ledger}: {got} != {want}")
                failures += 1
        missing = missing_citations(refs_text, sets)
        if missing:
            print("MEMORY-OWNERSHIP ✗ unresolved child-refs citations: " + ", ".join(missing))
            failures += 1

    for name in ACTIVE:
        path = os.path.join(mem, name)
        if not os.path.isfile(path):
            continue
        missing = missing_citations(read(path), sets)
        if missing:
            print(f"MEMORY-OWNERSHIP ✗ unresolved in {name}: " + ", ".join(sorted(set(missing))))
            failures += 1

    parent_titles = {}
    for name in ("decisions.md", "pitfalls.md", "conventions.md"):
        path = os.path.join(mem, name)
        if os.path.isfile(path):
            for _key, title in titles(path):
                value = normalize(title)
                if len(value) >= 4:
                    parent_titles[value] = title
    child_titles = {}
    for name in ("decisions.md", "pitfalls.md", "conventions.md"):
        path = os.path.join(CHILD_MEM, name)
        if os.path.isfile(path):
            for _key, title in titles(path):
                value = normalize(title)
                if len(value) >= 4:
                    child_titles[value] = title
    duplicates = duplicate_titles(parent_titles, child_titles)
    for value in duplicates:
        print(f"MEMORY-OWNERSHIP ✗ duplicate title: {parent_titles[value]} / {child_titles[value]}")
        failures += 1

    if failures:
        print(f"MEMORY-OWNERSHIP ✗ {failures} problem(s)")
        return 1
    print("MEMORY-OWNERSHIP ✓")
    return 0


if __name__ == "__main__":
    sys.exit(main())
