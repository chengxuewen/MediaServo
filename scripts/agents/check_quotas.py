#!/usr/bin/env python3
"""Quota gate (B11, plan 3.8; r19 audit MINOR -- the quota constants finally
exist in a parsed, enforced form).

Parses the fixed quota table from .agents/rules/common/agents.md and enforces
the mechanical caps:
  - role files (true source, both repos): 11
  - MCP servers: 5
  - members per team: 4
Skills net-per-quarter is REPORT-ONLY -- it needs the quarter ledger; there is
no mechanical baseline to enforce against (noted, not silently skipped).

S/P awareness (py-retirement form, all-gates.sh:76-80 precedent): running from
the submodule nested in the parent counts BOTH repos' role files (10+1=11);
standalone counts own roles only (10) and says so. A parent is recognized by
containing this repo at its registered submodule path (never by a bare ../
guess).

Usage: check_quotas.py [context_root]  (default: this repo root)
"""
import glob
import json
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
DEFAULT_ROOT = os.path.dirname(os.path.dirname(HERE))


def parse_caps(agents_md_path):
    """Parse the fixed quota table: asset name (lower) -> hard cap int.
    Non-numeric caps (skills 'net +2 per quarter') are skipped -- report-only."""
    if not os.path.isfile(agents_md_path):
        return {}
    with open(agents_md_path, encoding="utf-8") as fh:
        text = fh.read()
    section = re.search(r"## Role Roster and Quotas.*?(?=\n## |\Z)", text, re.S)
    if not section:
        return {}
    caps = {}
    for line in section.group(0).splitlines():
        m = re.match(r"^\|\s*([^|]+?)\s*\|\s*(\d+)\s*\|", line)
        if m:
            caps[m.group(1).strip().lower()] = int(m.group(2))
    return caps


def count_role_files(context_root):
    """True-source role files. Parent context = submodule + parent shell role."""
    sub = len(glob.glob(os.path.join(context_root, ".agents", "agents", "*.md")))
    parent_root = os.path.dirname(os.path.dirname(context_root))
    sub_at = os.path.join(parent_root, "3rdparty", "MediaServo")
    parent_mode = os.path.isdir(sub_at) and os.path.samefile(sub_at, context_root)
    parent_roles = 0
    if parent_mode:
        parent_roles = len(glob.glob(os.path.join(parent_root, ".agents", "agents", "*.md")))
    return sub, parent_roles, parent_mode


def count_mcp(context_root):
    path = os.path.join(context_root, ".agents", "mcp.json")
    if not os.path.isfile(path):
        return 0
    with open(path, encoding="utf-8") as fh:
        return len(json.load(fh).get("mcpServers", {}))


def count_teams(context_root):
    """Per-team member counts from .agents/teams/*.md (## Members bullets)."""
    teams = {}
    teams_dir = os.path.join(context_root, ".agents", "teams")
    if not os.path.isdir(teams_dir):
        return teams
    for fn in sorted(os.listdir(teams_dir)):
        if not fn.endswith(".md") or fn == "routing.md":
            continue
        with open(os.path.join(teams_dir, fn), encoding="utf-8") as fh:
            text = fh.read()
        m = re.search(r"## Members[^\n]*\n(.*?)(?=\n## |\Z)", text, re.S)
        members = re.findall(r"^- (\S+)", m.group(1), re.M) if m else []
        teams[fn] = len(members)
    return teams


def count_skills(context_root):
    root = os.path.join(context_root, ".agents", "skills")
    if not os.path.isdir(root):
        return 0
    return len([n for n in os.listdir(root)
                if os.path.isfile(os.path.join(root, n, "SKILL.md"))])


def run(root):
    fails = []
    caps = parse_caps(os.path.join(root, ".agents", "rules", "common", "agents.md"))
    if not caps:
        return 1, ["quota table missing or unparseable in rules/common/agents.md"]

    sub, parent_roles, parent_mode = count_role_files(root)
    total_roles = sub + parent_roles
    cap_roles = next((v for k, v in caps.items() if "role" in k), None)
    if cap_roles is not None and total_roles > cap_roles:
        fails.append("role files %d > cap %d" % (total_roles, cap_roles))

    mcp = count_mcp(root)
    cap_mcp = next((v for k, v in caps.items() if "mcp" in k), None)
    # MCP count cap REMOVED by user ruling (11-13): report-only now. The gate
    # still checks pins/routing/same-name via gate_mcp.
    if cap_mcp is not None and mcp > cap_mcp:
        fails.append("MCP servers %d > cap %d" % (mcp, cap_mcp))

    teams = count_teams(root)
    cap_team = next((v for k, v in caps.items() if "team" in k), None)
    for name, members in sorted(teams.items()):
        if cap_team is not None and members > cap_team:
            fails.append("team %s has %d members > cap %d" % (name, members, cap_team))

    skills = count_skills(root)
    notes = []
    if parent_mode:
        notes.append("parent context: %d submodule + %d parent roles" % (sub, parent_roles))
    else:
        notes.append("standalone: own %d roles counted only" % sub)
    notes.append("skills %d -- net +2/quarter is ledger-accounted (report-only)" % skills)

    mcp_fmt = "%d" % mcp if cap_mcp is None else "%d/%d" % (mcp, cap_mcp)
    summary = ("QUOTAS %s roles %d/%s, mcp %s (no count cap), teams %s, skills %d (report-only) | %s"
               % ("✓" if not fails else "✗",
                  total_roles, cap_roles, mcp_fmt,
                  {k: v for k, v in teams.items()} if teams else "{}",
                  skills, "; ".join(notes)))
    return len(fails), fails + [summary]


def main():
    root = os.path.abspath(sys.argv[1]) if len(sys.argv) > 1 else DEFAULT_ROOT
    fails, lines = run(root)
    summary = lines[-1]
    for line in lines[:-1]:
        print("CHECK-QUOTAS ✗ " + line)
    print(("CHECK-QUOTAS ✓ " if not fails else "CHECK-QUOTAS ✗ ") + summary)
    sys.exit(1 if fails else 0)


if __name__ == "__main__":
    main()