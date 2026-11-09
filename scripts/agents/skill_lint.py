#!/usr/bin/env python3
"""Skill tree lint (B4-2 deepened; B4-4 status gate).

Per skill directory:
  - SKILL.md exists, frontmatter parses
  - name == directory, description non-empty (existing checks)
  - SKILL.md <= 800 lines
  - description <= 1024 chars
  - `@...md` path references resolve (form: @word/....md; GitHub-action
    pins like @v4 never match -- zero instances today, the gate guards the
    form if it is ever introduced)
  - status: frontmatter, when present, must be exactly `inoperative`
    (the `deprecated` value is parked as O-14 until a real deprecation case)
Cross-cutting:
  - every skill directory is mentioned in TRIGGERS.md (the registry keeps
    pace with the tree -- doc-audit dimension 3 as a gate, not an audit item)
"""
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
SKILLS = os.path.join(ROOT, ".agents", "skills")
TRIGGERS = os.path.join(SKILLS, "TRIGGERS.md")
FM = re.compile(r"^---\n(.*?)\n---\n", re.S)
MAX_LINES = 800
MAX_DESC = 1024
ALLOWED_STATUS = {"inoperative"}


def fail(msg, state):
    print("SKILL-LINT ✗ %s" % msg)
    state["fail"] = 1


def main():
    state = {"fail": 0}
    triggers_text = open(TRIGGERS, encoding="utf-8").read() if os.path.isfile(TRIGGERS) else ""
    if not triggers_text:
        fail("TRIGGERS.md missing or empty", state)
    seen = {}
    inoperative = []
    names = sorted(name for name in os.listdir(SKILLS) if os.path.isdir(os.path.join(SKILLS, name)))
    for name in names:
        path = os.path.join(SKILLS, name, "SKILL.md")
        if not os.path.isfile(path):
            fail("%s: SKILL.md missing" % name, state)
            continue
        text = open(path, encoding="utf-8").read()
        match = FM.match(text)
        if not match:
            fail("%s: frontmatter missing" % name, state)
            continue
        block = match.group(1)
        declared = re.search(r"^name:\s*(\S+)", block, re.M)
        desc = re.search(r"^description:\s*(.+)$", block, re.M)
        status = re.search(r"^status:\s*(\S+)", block, re.M)
        if not declared or declared.group(1) != name:
            fail("%s: name mismatch (%s)" % (name, declared.group(1) if declared else "none"), state)
        if not desc or not desc.group(1).strip():
            fail("%s: description missing" % name, state)
        elif len(desc.group(1)) > MAX_DESC:
            fail("%s: description %d chars > %d" % (name, len(desc.group(1)), MAX_DESC), state)
        lines = text.count("\n") + (0 if text.endswith("\n") else 1)
        if lines > MAX_LINES:
            fail("%s: %d lines > %d" % (name, lines, MAX_LINES), state)
        if status and status.group(1) not in ALLOWED_STATUS:
            fail("%s: status %r not in %s (deprecated is parked as O-14)"
                 % (name, status.group(1), sorted(ALLOWED_STATUS)), state)
        if status and status.group(1) == "inoperative":
            inoperative.append(name)
        # `@path.md` references must resolve: relative to the skill dir first,
        # then the repo root. (Zero instances today -- keeps the form honest.)
        for ref in re.findall(r"@(\w[\w./-]*\.md)\b", text):
            for base in (os.path.join(SKILLS, name), ROOT):
                if os.path.isfile(os.path.join(base, ref)):
                    break
            else:
                fail("%s: @%s does not resolve" % (name, ref), state)
        if name in seen:
            fail("duplicate skill name: %s" % name, state)
        seen[name] = True
        if triggers_text and name not in triggers_text:
            fail("%s: not mentioned in TRIGGERS.md (registry must cover every skill)" % name, state)
    # An inoperative skill must say so in the registry too, so no agent picks
    # it up from TRIGGERS without the warning.
    for name in inoperative:
        entry_line = next((line for line in triggers_text.splitlines()
                           if "`%s`" % name in line and "| " in line), "")
        if entry_line and "INOPERATIVE" not in entry_line:
            fail("%s: status inoperative but its TRIGGERS.md row lacks the INOPERATIVE mark" % name, state)
    print("SKILL-LINT %s %d skill(s)%s" % (
        "✓" if state["fail"] == 0 else "✗", len(names),
        " (inoperative: %s)" % ", ".join(inoperative) if inoperative else ""))
    return 1 if state["fail"] else 0


if __name__ == "__main__":
    sys.exit(main())
