#!/usr/bin/env python3
"""Rule-reference lint: D/PIT/C citations in .agents surfaces must resolve.

Scan roots (B4-3): .agents/rules/** plus .agents/skills/**/SKILL.md, AGENTS.md
and the root SKILL.md. Judgement is membership in the heading-ID set of
memorys/decisions.md, pitfalls.md, conventions.md -- a citation to an ID that
exists in another numbering space (parent repo, upstream project, worked
example text) goes through rule-ref-allowlist.txt, never silently.
"""
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
AGENTS = os.path.join(ROOT, ".agents")
ALLOW = os.path.join(os.path.dirname(os.path.abspath(__file__)), "rule-ref-allowlist.txt")


def numbers(path, pattern):
    if not os.path.isfile(path):
        return set()
    text = open(path, encoding="utf-8").read()
    return {int(m.group(1)) for m in re.finditer(pattern, text, re.M)}


def scan_targets():
    targets = []
    for base, _dirs, files in os.walk(os.path.join(AGENTS, "rules")):
        for name in sorted(files):
            if name.endswith(".md"):
                targets.append(os.path.join(base, name))
    for base, _dirs, files in os.walk(os.path.join(AGENTS, "skills")):
        for name in sorted(files):
            if name == "SKILL.md":
                targets.append(os.path.join(base, name))
    for extra in ("AGENTS.md", "SKILL.md"):
        path = os.path.join(ROOT, extra)
        if os.path.isfile(path):
            targets.append(path)
    return targets


def main():
    max_d = numbers(os.path.join(AGENTS, "memorys", "decisions.md"), r"^## D(\d+)")
    max_p = numbers(os.path.join(AGENTS, "memorys", "pitfalls.md"), r"^## PIT-(\d+)")
    max_c = numbers(os.path.join(AGENTS, "memorys", "conventions.md"), r"^## C(\d+)")
    allowed = set()
    if os.path.isfile(ALLOW):
        allowed = {line.split("#", 1)[0].strip() for line in open(ALLOW, encoding="utf-8")
                   if line.split("#", 1)[0].strip()}
    fail = 0
    for path in scan_targets():
        text = open(path, encoding="utf-8").read()
        for token, table in (("D", max_d), ("PIT-", max_p), ("C", max_c)):
            pattern = r"\b%s(\d{1,3})\b" % token if token != "C" else r"(?<![A-Za-z])C(\d{1,2})\b"
            for match in re.finditer(pattern, text):
                value = token + match.group(1)
                if int(match.group(1)) not in table and value not in allowed:
                    rel = os.path.relpath(path, ROOT)
                    print("RULE-REF ✗ %s: %s missing from memorys" % (rel, value))
                    fail = 1
    print("RULE-REF %s (D%d / PIT-%d / C%d)" % (
        "✓" if fail == 0 else "✗",
        max(max_d) if max_d else 0, max(max_p) if max_p else 0, max(max_c) if max_c else 0))
    return 1 if fail else 0


if __name__ == "__main__":
    sys.exit(main())
