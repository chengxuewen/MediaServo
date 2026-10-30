#!/usr/bin/env python3
"""Static checks for the MediaServo docs site (D290).

--structure: SUMMARY entries resolve (SKIP when no book-src/SUMMARY.md exists).
default: build output + structure + CJK=0 on every SUMMARY entry.
Graph/browser checks from the donor are deliberately not ported.
"""

import argparse
import os
import sys

import book_common as common


def structure_failures():
    summary = os.path.join(common.SRC, "SUMMARY.md")
    entries = common.summary_entries(summary)
    failures = 0
    for rel in entries:
        if not os.path.isfile(os.path.join(common.SRC, rel)):
            print(f"BOOK-CHECK ✗ SUMMARY entry missing: {rel}")
            failures += 1
    for entry in common.load_toc().get("chapter", []):
        if not os.path.isfile(os.path.join(common.ROOT, entry["source"])):
            print(f"BOOK-CHECK ✗ toc source missing: {entry['source']}")
            failures += 1
    return entries, failures


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--structure", action="store_true", help="only check SUMMARY entries")
    args = parser.parse_args()

    summary = os.path.join(common.SRC, "SUMMARY.md")
    if not os.path.isfile(summary):
        if args.structure:
            print("BOOK-CHECK - no build/book-src/SUMMARY.md (SKIP)")
            return 0
        print("BOOK-CHECK ✗ no build/book-src (run: pixi run book-build)")
        return 1

    entries, failures = structure_failures()
    if args.structure:
        if failures:
            print(f"BOOK-CHECK ✗ {failures} structure problem(s)")
            return 1
        print(f"BOOK-CHECK ✓ structure ({len(entries)} entries)")
        return 0

    index = os.path.join(common.OUT, "index.html")
    if not os.path.isfile(index) or os.path.getsize(index) == 0:
        print("BOOK-CHECK ✗ build/book/index.html missing or empty (run: pixi run book-build)")
        failures += 1
    search = [name for name in os.listdir(common.OUT) if name.startswith("searchindex")]
    if not search:
        print("BOOK-CHECK ✗ search index missing")
        failures += 1

    for rel in entries:
        path = os.path.join(common.SRC, rel)
        if not os.path.isfile(path):
            continue
        with open(path, encoding="utf-8") as handle:
            for number, line in enumerate(handle, 1):
                if common.CJK.search(line) and "c49:trigger" not in line:
                    print(f"BOOK-CHECK ✗ CJK: {rel}:{number}")
                    failures += 1

    appendix = 0
    in_appendix = False
    with open(summary, encoding="utf-8") as handle:
        for line in handle:
            if line.startswith("# Appendix"):
                in_appendix = True
            elif in_appendix and common.SUMMARY_ENTRY.match(line.strip()):
                appendix += 1
    if appendix > 60:
        print(f"BOOK-CHECK WARN appendix has {appendix} chapters (>60)")

    if failures:
        print(f"BOOK-CHECK ✗ {failures} problem(s)")
        return 1
    print(f"BOOK-CHECK ✓ {len(entries)} entries, appendix {appendix}, CJK 0")
    return 0


if __name__ == "__main__":
    sys.exit(main())
