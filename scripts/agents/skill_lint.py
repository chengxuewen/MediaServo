#!/usr/bin/env python3
"""Skill tree lint: frontmatter name/description, name == directory, no duplicates."""
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
SKILLS = os.path.join(ROOT, ".agents", "skills")
FM = re.compile(r"^---\n(.*?)\n---\n", re.S)


def main():
    seen = {}
    fail = 0
    names = sorted(name for name in os.listdir(SKILLS) if os.path.isdir(os.path.join(SKILLS, name)))
    for name in names:
        path = os.path.join(SKILLS, name, "SKILL.md")
        if not os.path.isfile(path):
            print("SKILL-LINT ✗ %s: SKILL.md missing" % name)
            fail = 1
            continue
        match = FM.match(open(path, encoding="utf-8").read())
        if not match:
            print("SKILL-LINT ✗ %s: frontmatter missing" % name)
            fail = 1
            continue
        block = match.group(1)
        declared = re.search(r"^name:\s*(\S+)", block, re.M)
        desc = re.search(r"^description:\s*(.+)$", block, re.M)
        if not declared or declared.group(1) != name:
            print("SKILL-LINT ✗ %s: name mismatch (%s)" % (name, declared.group(1) if declared else "none"))
            fail = 1
        if not desc or not desc.group(1).strip():
            print("SKILL-LINT ✗ %s: description missing" % name)
            fail = 1
        if name in seen:
            print("SKILL-LINT ✗ duplicate skill name: %s" % name)
            fail = 1
        seen[name] = True
    print("SKILL-LINT %s %d skill(s)" % ("✓" if fail == 0 else "✗", len(names)))
    return 1 if fail else 0


if __name__ == "__main__":
    sys.exit(main())
