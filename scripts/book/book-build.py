#!/usr/bin/env python3
"""Assemble build/book-src from docs/ + book/toc.toml and run mdbook (D290).

Unlisted docs are appended under "Appendix" with a WARN; `exclude` globs keep
research/history and the CJK ABI baseline out of the site. Output: build/book.
"""

import os
import shutil
import subprocess
import sys

import book_common as common


def main() -> int:
    toc = common.load_toc()
    patterns = toc.get("exclude", [])

    if os.path.isdir(common.SRC):
        shutil.rmtree(common.SRC)
    os.makedirs(common.SRC)
    shutil.copytree(os.path.join(common.ROOT, "docs"), os.path.join(common.SRC, "docs"))
    shutil.copy2(os.path.join(common.ROOT, "README.md"), os.path.join(common.SRC, "README.md"))
    web = os.path.join(common.BOOK, "web")
    if os.path.isdir(web):
        shutil.copytree(web, os.path.join(common.SRC, "web"))

    lines = ["# Summary", ""]
    covered = set()
    current_group = None

    def emit(source, title, group):
        nonlocal current_group
        if group != current_group:
            lines.append(f"# {group}" if group else "")
            current_group = group
        lines.append(f"- [{title}]({source})")
        covered.add(source)

    for entry in toc.get("chapter", []):
        source = entry["source"]
        if not os.path.isfile(os.path.join(common.ROOT, source)):
            print(f"BOOK ✗ missing chapter: {source}", file=sys.stderr)
            return 1
        title = entry.get("title") or common.first_heading(os.path.join(common.ROOT, source))
        emit(source, title, entry.get("group"))

    appendix = [rel for rel in common.doc_files()
                if rel not in covered and not common.excluded(rel, patterns)]
    if appendix:
        lines.append("# Appendix (unlisted in book/toc.toml)")
        for rel in appendix:
            print(f"BOOK WARN unlisted: {rel} -> appendix", file=sys.stderr)
            lines.append(f"- [{common.first_heading(os.path.join(common.ROOT, rel))}]({rel})")
        if len(appendix) > 60:
            print(f"BOOK WARN appendix has {len(appendix)} chapters (>60)", file=sys.stderr)

    with open(os.path.join(common.SRC, "SUMMARY.md"), "w", encoding="utf-8") as handle:
        handle.write("\n".join(lines) + "\n")

    if not shutil.which("mdbook"):
        print("BOOK ✗ mdbook not found (run: pixi install)", file=sys.stderr)
        return 1
    proc = subprocess.run(["mdbook", "build", common.BOOK, "-d", common.OUT],
                          cwd=common.ROOT, stdin=subprocess.DEVNULL)
    if proc.returncode != 0:
        print("BOOK ✗ mdbook build failed", file=sys.stderr)
        return proc.returncode
    print(f"BOOK ✓ built {len(covered)} chapter(s) -> {os.path.relpath(common.OUT, common.ROOT)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
