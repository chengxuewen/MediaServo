#!/usr/bin/env python3
"""Agent role lint (agent-subagent-layer B1).

Validates canonical role files in .agents/agents/*.md against the semantic
schema (plan section 3.3). The renderer (B2) consumes these; this gate keeps
the true source honest before any host projection.

Per role file:
  - frontmatter parses; name == file stem; description non-empty <= 1024
  - body <= 300 lines (a prompt, not a document)
  - tools subset of the semantic verb whitelist (lowercase)
  - read_only: true must not carry write/edit
  - skills: each referenced skill exists (.agents/skills/<name>/SKILL.md)
  - mode in {subagent, primary, all}
Cross-cutting:
  - name unique across the directory
Empty/missing directory is GREEN (a new gate must not open red).
"""
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
AGENTS = os.path.join(ROOT, ".agents", "agents")
SKILLS = os.path.join(ROOT, ".agents", "skills")
FM = re.compile(r"^---\n(.*?)\n---\n", re.S)
MAX_BODY_LINES = 300
MAX_DESC = 1024
TOOL_VERBS = {"read", "grep", "glob", "shell", "write", "edit",
              "task", "webfetch", "websearch", "skill"}
MODES = {"subagent", "primary", "all"}
WRITE_VERBS = {"write", "edit"}


def _parse_list(value):
    """Accept '[a, b]', 'a, b', or a single bare token; strip quotes."""
    value = value.strip()
    if value.startswith("[") and value.endswith("]"):
        value = value[1:-1]
    return [item.strip().strip("'\"") for item in value.split(",") if item.strip()]


def lint_agents(agents_dir, skills_dir):
    """Lint every role file; return (fail_count, role_count). Prints findings."""
    state = {"fail": 0}

    def fail(msg):
        print("AGENT-LINT ✗ %s" % msg)
        state["fail"] += 1

    if not os.path.isdir(agents_dir):
        print("AGENT-LINT ✓ 0 role(s) (no .agents/agents yet)")
        return 0, 0

    files = sorted(f for f in os.listdir(agents_dir) if f.endswith(".md"))
    seen = {}
    for filename in files:
        stem = filename[:-3]
        with open(os.path.join(agents_dir, filename), encoding="utf-8") as fh:
            text = fh.read()
        match = FM.match(text)
        if not match:
            fail("%s: frontmatter missing" % stem)
            continue
        block = match.group(1)
        body = text[match.end():]

        declared = re.search(r"^name:[ \t]*(\S+)", block, re.M)
        name = declared.group(1) if declared else stem
        if not declared or declared.group(1) != stem:
            fail("%s: name mismatch (%s)" % (stem, name))
        if name in seen:
            fail("duplicate role name: %s" % name)
        seen[name] = True

        desc = re.search(r"^description:[ \t]*(.+)$", block, re.M)
        if not desc or not desc.group(1).strip():
            fail("%s: description missing" % stem)
        elif len(desc.group(1)) > MAX_DESC:
            fail("%s: description %d chars > %d" % (stem, len(desc.group(1)), MAX_DESC))

        body_lines = len(body.splitlines())
        if body_lines > MAX_BODY_LINES:
            fail("%s: body %d lines > %d" % (stem, body_lines, MAX_BODY_LINES))

        mode = re.search(r"^mode:[ \t]*(\S+)", block, re.M)
        if mode and mode.group(1) not in MODES:
            fail("%s: mode %r not in %s" % (stem, mode.group(1), sorted(MODES)))

        tools_m = re.search(r"^tools:[ \t]*(.+)$", block, re.M)
        tools = _parse_list(tools_m.group(1)) if tools_m else []
        for verb in tools:
            if verb not in TOOL_VERBS:
                fail("%s: tool %r not a semantic verb" % (stem, verb))

        read_only = re.search(r"^read_only:[ \t]*(\S+)", block, re.M)
        if read_only and read_only.group(1).lower() == "true":
            bad = WRITE_VERBS.intersection(tools)
            if bad:
                fail("%s: read_only but carries %s" % (stem, ", ".join(sorted(bad))))

        skills_m = re.search(r"^skills:[ \t]*(.+)$", block, re.M)
        if skills_m:
            for skill in _parse_list(skills_m.group(1)):
                if not os.path.isfile(os.path.join(skills_dir, skill, "SKILL.md")):
                    fail("%s: skill %r not found in .agents/skills/" % (stem, skill))

    print("AGENT-LINT %s %d role(s)" % ("✓" if state["fail"] == 0 else "✗", len(files)))
    return state["fail"], len(files)


def main():
    fails, _ = lint_agents(AGENTS, SKILLS)
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())