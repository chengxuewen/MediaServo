#!/usr/bin/env python3
"""Registry-integrity gate (r43, audit H1/H2): decision/pitfall numbering.

Checks two things over the memorys registries (.agents/memorys):
  1. Continuity -- inside each registry's active range, every number must
     exist as an entry or be a REGISTERED VOID (a placeholder line in the
     registry itself, tagged 有意空缺/占位). Ghost numbers = fail.
  2. Ghost cites -- any D<n> / PIT-<n> token in the reference corpus
     (docs, scripts, crates src/tests, www src, cmake, mediaservo.sh,
     README, .agents rules/skills/teams) must resolve to an existing entry.
     Running code citing a non-existent registry entry is real breakage
     (audit r42: cmake cited D266 which had no entry).

Scope notes (documented limits, not oversights):
  - decisions D1-D190 are ARCHIVED (decisions.md:3; recoverable via
    `git cat-file -p 16ce9bb1664d`) -- cites into that range are exempt.
  - conventions.md C-numbers are inline prose, not `## C<n>:` headings; the
    C36-C42 gap is tracked as audit M14 (manual watch), not gated here.
  - VOID sets below are the gate's allowlist; each void ALSO exists as a
    placeholder line in its registry (append-only: never renumber).
"""
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
DEFAULT_ROOT = os.path.dirname(os.path.dirname(HERE))

# Registered intentional gaps (each has a placeholder line in its registry).
VOID_D = {191, 192, 193, 194, 195}
VOID_PIT = {42}
ARCHIVED_D = range(1, 191)  # decisions.md:3 -- D1-D190 archived (cite-exempt)

CORPUS_GLOBS = [
    "docs/**/*.md", "scripts/**/*.sh", "scripts/**/*.py", "scripts/**/*.mjs",
    "cmake/**/*.cmake", "cmake/*.cmake", "mediaservo.sh", "README.md",
    ".agents/rules/**/*.md", ".agents/skills/**/SKILL.md", ".agents/teams/*.md",
    "PolyOrch/SKILL.md",
    "crates/*/src/**/*.rs", "crates/*/tests/**/*.rs",
    "www/packages/*/src/**/*.ts", "www/packages/*/src/**/*.tsx",
    "www/apps/*/src/**/*.ts", "www/apps/*/src/**/*.tsx",
]
EXCLUDE_PARTS = {"node_modules", "vendor", "dist", "target", "build", ".pixi",
                 "book", ".ruff_cache", "__pycache__"}
# check_memory_ownership.py runs a self-test with FAKE registry numbers
# (a sentinel id in its test strings) -- excluded by design, not an oversight.
# This gate excludes ITSELF too: a lint is not a cite corpus.
EXCLUDE_FILES = {"scripts/agents/check_memory_ownership.py",
                 "scripts/agents/check_docs_numbers.py",
                 "scripts/agents/test_docs_numbers.py"}


def registry_numbers(path, pattern):
    if not os.path.isfile(path):
        return set()
    numbers = set()
    with open(path, encoding="utf-8") as fh:
        for line in fh:
            m = re.match(pattern, line)
            if m:
                numbers.add(int(m.group(1)))
    return numbers


def corpus_files(root):
    import glob
    files = []
    for g in CORPUS_GLOBS:
        for p in glob.glob(os.path.join(root, g), recursive=True):
            parts = set(os.path.normpath(p).split(os.sep))
            if parts & EXCLUDE_PARTS:
                continue
            rel = os.path.relpath(p, root).replace(os.sep, "/")
            if rel in EXCLUDE_FILES:
                continue
            if os.path.isfile(p):
                files.append(p)
    return sorted(set(files))


def parent_registries(root):
    """Cross-repo cites (e.g. reuse-first receipts) may target the parent
    workspace's registries. Resolved as a fallback, never for continuity."""
    pr = os.path.dirname(os.path.dirname(root))
    sub = os.path.join(pr, "3rdparty", "MediaServo")
    if not (os.path.isdir(sub) and os.path.samefile(sub, root)):
        return set(), set()
    mem = os.path.join(pr, ".agents", "memorys")
    return (registry_numbers(os.path.join(mem, "decisions.md"), r"^## D(\d+):"),
            registry_numbers(os.path.join(mem, "pitfalls.md"), r"^## PIT-(\d+):"))


def run(root):
    mem = os.path.join(root, ".agents", "memorys")
    d_nums = registry_numbers(os.path.join(mem, "decisions.md"), r"^## D(\d+):")
    pit_nums = registry_numbers(os.path.join(mem, "pitfalls.md"), r"^## PIT-(\d+):")
    parent_d, parent_pit = parent_registries(root)
    fails = []

    # 1. Continuity over each active range (min..max must exist or be VOID).
    if d_nums:
        for n in range(min(d_nums), max(d_nums) + 1):
            if n not in d_nums and n not in VOID_D and n in ARCHIVED_D:
                continue  # archived range: exempt from continuity
            if n not in d_nums and n not in VOID_D and n not in ARCHIVED_D:
                fails.append("decisions.md ghost number D%d (in active range, not VOID)" % n)
    if pit_nums:
        for n in range(min(pit_nums), max(pit_nums) + 1):
            if n not in pit_nums and n not in VOID_PIT:
                fails.append("pitfalls.md ghost number PIT-%d (not VOID)" % n)

    # 2. Ghost cites across the corpus.
    cite_re = re.compile(r"\bD(\d{2,3})\b|\bPIT-(\d+)\b")
    seen = set()
    for path in corpus_files(root):
        with open(path, encoding="utf-8", errors="replace") as fh:
            for i, line in enumerate(fh, 1):
                for m in cite_re.finditer(line):
                    d, pit = m.group(1), m.group(2)
                    if d is not None:
                        n = int(d)
                        if n in ARCHIVED_D:
                            continue
                        if (n not in d_nums and n not in parent_d and n not in VOID_D):
                            key = ("D", n)
                            if key not in seen:
                                seen.add(key)
                                fails.append("ghost cite D%d at %s:%d (no registry entry, not VOID)"
                                             % (n, os.path.relpath(path, root), i))
                    else:
                        n = int(pit)
                        if 2000 <= n <= 2099:
                            continue  # year-like prose ("PIT-2026-10-09 pin"), not a cite
                        if (n not in pit_nums and n not in parent_pit and n not in VOID_PIT):
                            key = ("PIT", n)
                            if key not in seen:
                                seen.add(key)
                                fails.append("ghost cite PIT-%d at %s:%d (no registry entry, not VOID)"
                                             % (n, os.path.relpath(path, root), i))

    summary = ("DOCS-NUMBERS %s decisions %d entries (active D%d+), pitfalls %d "
               "entries, ghost cites %d" % ("✓" if not fails else "✗", len(d_nums),
                                            min(d_nums) if d_nums else 0,
                                            len(pit_nums), len(seen)))
    return len(fails), fails + [summary]


def main():
    root = os.path.abspath(sys.argv[1]) if len(sys.argv) > 1 else DEFAULT_ROOT
    fails, lines = run(root)
    summary = lines[-1]
    for line in lines[:-1]:
        print("DOCS-NUMBERS ✗ " + line)
    print(("DOCS-NUMBERS ✓ " if not fails else "DOCS-NUMBERS ✗ ") + summary)
    sys.exit(1 if fails else 0)


if __name__ == "__main__":
    main()