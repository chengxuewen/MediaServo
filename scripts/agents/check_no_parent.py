#\!/usr/bin/env python3
"""Forbid parent-workspace literals in the submodule's agent layer (C15)."""
import os
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
HERE = os.path.dirname(os.path.abspath(__file__))
ALLOW = os.path.join(HERE, "parent-literal-allow.txt")
PATTERNS = ("MSRTC_", "3rdparty/MediaServo", "ms_rtc")
ROOTS = (os.path.join(ROOT, "scripts", "agents"), os.path.join(ROOT, ".agents"))
SKIP = {".pyc", ".png", ".zip", ".gz"}


def allowed_files():
    if not os.path.isfile(ALLOW):
        return set()
    return {line.strip() for line in open(ALLOW, encoding="utf-8")
            if line.strip() and not line.startswith("#")}


def main():
    allow = allowed_files()
    fail = 0
    for root in ROOTS:
        for base, dirs, files in os.walk(root):
            dirs[:] = [d for d in dirs if d not in {"__pycache__", "node_modules", "memorys"}]
            for name in files:
                if os.path.splitext(name)[1] in SKIP:
                    continue
                path = os.path.join(base, name)
                if os.path.abspath(path) == os.path.abspath(__file__):
                    continue
                rel = os.path.relpath(path, ROOT)
                if rel in allow:
                    continue
                try:
                    text = open(path, encoding="utf-8").read()
                except (OSError, UnicodeDecodeError):
                    continue
                for line_no, line in enumerate(text.splitlines(), 1):
                    if any(p in line for p in PATTERNS):
                        print("NO-PARENT ✗ %s:%d %s" % (rel, line_no, line.strip()[:100]))
                        fail = 1
    print("NO-PARENT %s" % ("✓" if fail == 0 else "✗"))
    return 1 if fail else 0


if __name__ == "__main__":
    sys.exit(main())
