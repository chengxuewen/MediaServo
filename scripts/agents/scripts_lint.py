#!/usr/bin/env python3
"""Scripts-domain audit gate (B14, matrix scripts row: bash -n + orphan check).

- bash -n on every scripts/**/*.sh -- a broken script is a HARD failure.
- ast.parse on every scripts/**/*.py (excluding __pycache__) -- same.
- Orphan detection: a script whose basename appears NOWHERE outside its own
  file (gates, launcher, package manifests, docs, memory, agent mounts) is
  reported as an orphan. Orphans are WARN-only: deletion is a judgment call
  and audits propose, they never execute (N15).
"""
import ast
import glob
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
DEFAULT_ROOT = os.path.dirname(os.path.dirname(HERE))

fails = []
orphans = []


def sh_files(root):
    return [p for p in glob.glob(os.path.join(root, "scripts", "**", "*.sh"), recursive=True)
            if "node_modules" not in p]


def py_files(root):
    return [p for p in glob.glob(os.path.join(root, "scripts", "**", "*.py"), recursive=True)
            if "__pycache__" not in p]


def reference_corpus(root, own):
    """Text of every plausible reference site, minus the script's own file."""
    corpus = []
    sites = ([os.path.join(root, "scripts", "all-gates.sh"), os.path.join(root, "pixi.toml"),
              os.path.join(root, "AGENTS.md"), os.path.join(root, "README.md"),
              os.path.join(HERE, "check_guards.py"), os.path.join(HERE, "mcp-serve.mjs")]
             + sh_files(root) + py_files(root) + glob.glob(os.path.join(root, "docs", "**", "*.md"), recursive=True)
             + glob.glob(os.path.join(root, ".agents", "**", "*.md"), recursive=True)
             + glob.glob(os.path.join(root, "book", "**", "*.md"), recursive=True))
    for site in sites:
        if os.path.isfile(site) and os.path.abspath(site) != os.path.abspath(own):
            try:
                with open(site, encoding="utf-8", errors="replace") as fh:
                    corpus.append(fh.read())
            except OSError:
                pass
    return "\n".join(corpus)


def main():
    root = os.path.abspath(sys.argv[1]) if len(sys.argv) > 1 else DEFAULT_ROOT

    for path in sh_files(root):
        proc = subprocess.run(["bash", "-n", path], capture_output=True)
        if proc.returncode != 0:
            fails.append("bash -n failed: %s" % path)
    for path in py_files(root):
        try:
            with open(path, encoding="utf-8") as fh:
                ast.parse(fh.read(), filename=path)
        except SyntaxError as exc:
            fails.append("ast.parse failed: %s (%s)" % (path, exc))

    for path in sh_files(root) + py_files(root):
        base = os.path.basename(path)
        if base in ("cli.py", "mcp-serve.mjs"):  # entry points: invoked by name/directly
            continue
        corpus = reference_corpus(root, path)
        if base not in corpus:
            orphans.append(base)

    for line in fails:
        print("SCRIPTS-LINT ✗ " + line)
    for name in orphans:
        print("SCRIPTS-LINT [WARN] orphan script (referenced nowhere outside itself): %s "
              "-- deletion is a judgment call for repo-audit 2D" % name)
    summary = ("sh %d + py %d syntax-checked, orphans %d (warn-only)"
               % (len(sh_files(root)), len(py_files(root)), len(orphans)))
    print(("SCRIPTS-LINT ✓ " if not fails else "SCRIPTS-LINT ✗ ") + summary)
    sys.exit(1 if fails else 0)


if __name__ == "__main__":
    main()