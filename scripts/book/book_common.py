"""Shared paths and helpers for the MediaServo docs site (H6)."""

import fnmatch
import os
import re
import tomllib

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
BOOK = os.path.join(ROOT, "book")
SRC = os.path.join(ROOT, "build", "book-src")
OUT = os.path.join(ROOT, "build", "book")

SUMMARY_ENTRY = re.compile(r"^- \[[^\]]+\]\(([^)]+)\)")
CJK = re.compile(r"[\u4e00-\u9fff]")


def load_toc():
    with open(os.path.join(BOOK, "toc.toml"), "rb") as handle:
        return tomllib.load(handle)


def doc_files():
    found = []
    docs = os.path.join(ROOT, "docs")
    for base, _dirs, names in os.walk(docs):
        for name in sorted(names):
            if name.endswith(".md"):
                found.append(os.path.relpath(os.path.join(base, name), ROOT))
    return sorted(found)


def excluded(rel, patterns):
    return any(fnmatch.fnmatch(rel, pattern) for pattern in patterns)


def first_heading(path):
    try:
        with open(path, encoding="utf-8") as handle:
            for line in handle:
                if line.startswith("# "):
                    return line[2:].strip()
    except OSError:
        pass
    return os.path.splitext(os.path.basename(path))[0]


def summary_entries(summary_path):
    entries = []
    if not os.path.isfile(summary_path):
        return entries
    with open(summary_path, encoding="utf-8") as handle:
        for line in handle:
            match = SUMMARY_ENTRY.match(line.strip())
            if match:
                entries.append(match.group(1))
    return entries
