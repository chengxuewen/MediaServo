#!/usr/bin/env python3
"""Agent single-source generator: sync | check | status.

Single source: <repo>/.agents (rules, skills, memorys, mcp.json).
Targets: <target-root>/.agents/targets.json (host declarations).

Dual-context contract:
  - S mode (standalone): --source and --target-root default to this repo; self-links
    are skipped, repo artifacts (SKILL.md / CONSTRAINTS.md / AGENTS.md) are generated.
  - P mode (submodule): the parent passes absolute --source / --target-root /
    --targets; only the host layers declared there are materialized.
The generator never writes into the canonical tree (guard -> exit 2).

Exit codes: 0 clean, 1 findings (check) / actions applied (sync), 2 guard.
"""

import argparse
import copy
import os
import re
import sys

import common

SKILL_BEGIN = "<!-- BEGIN:skill-registry -->"
SKILL_END = "<!-- END:skill-registry -->"
SKILLS_BEGIN = "<!-- BEGIN:agents-skills -->"
SKILLS_END = "<!-- END:agents-skills -->"
MEM_BEGIN = "<!-- BEGIN:agents-memory -->"
MEM_END = "<!-- END:agents-memory -->"
GATES_BEGIN = "<!-- BEGIN:ci-gates -->"
GATES_END = "<!-- END:ci-gates -->"

GATE_RE = re.compile(r'^[ \t]*gate "([^"]+)"', re.MULTILINE)
WINDOW = {"decisions": 25, "pitfalls": 15, "conventions": 10, "status": 6}


def parse_args(argv):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["sync", "check", "status"])
    parser.add_argument("--source", default=None)
    parser.add_argument("--target-root", default=None)
    parser.add_argument("--targets", default=None)
    args = parser.parse_args(argv)
    source = os.path.abspath(args.source) if args.source else os.path.join(common.ROOT, ".agents")
    target = os.path.abspath(args.target_root) if args.target_root else common.ROOT
    targets = os.path.abspath(args.targets) if args.targets else os.path.join(target, ".agents", "targets.json")
    return args.command, source, target, targets


def repo_of(source):
    return os.path.dirname(source)


def load_registry(source):
    path = os.path.join(repo_of(source), "scripts", "agents", "mcp-servers.json")
    return common.load_json(path)


def canonical_mcp(source):
    return common.load_json(os.path.join(source, "mcp.json")).get("mcpServers", {})


def host_mcp(servers, launcher):
    out = {}
    for name, entry in servers.items():
        item = copy.deepcopy(entry)
        args = list(item.get("args", []))
        if args:
            args[0] = launcher
        else:
            args = [launcher, name]
        item["args"] = args
        out[name] = item
    return out


def opencode_mcp(servers, launcher):
    out = {}
    for name in servers:
        out[name] = {"type": "local", "enabled": True, "command": ["bash", launcher, name]}
    return out


def skill_rows(source):
    root = os.path.join(source, "skills")
    rows = []
    for name in sorted(os.listdir(root)):
        skill = os.path.join(root, name, "SKILL.md")
        if not os.path.isfile(skill):
            continue
        desc = common.skill_description(skill)
        # Generated hubs are English-only (C49): drop CJK trigger phrases from the
        # registry view; the skill file itself keeps them with c49:trigger.
        desc = re.sub(r"[\u4e00-\u9fff]+", "", desc)
        desc = re.sub(r"\s+", " ", desc).strip(" -/")
        desc = re.split(r"(?<=\.)\s", desc)[0].strip()[:140] or "(no description)"
        rows.append((name, desc))
    return rows


def skills_block(source):
    rows = skill_rows(source)
    lines = [
        "## SKILL DIRECTORY (generated -- edit .agents/skills/*, not this block)",
        "",
        "| Skill | Purpose |",
        "|-------|---------|",
    ]
    for name, desc in rows:
        lines.append("| `%s` | %s |" % (name, desc.replace("|", "\\|")))
    return "\n".join(lines)


def memory_block(source):
    lines = ["## MEMORY DIGEST (generated -- titles only; full text in .agents/memorys/)", ""]
    for name, limit in WINDOW.items():
        path = os.path.join(source, "memorys", name + ".md")
        if not os.path.isfile(path):
            continue
        titles = []
        for line in common.read_text(path).splitlines():
            if line.startswith("## ") or (name == "status" and line.startswith("### ")):
                titles.append(line.lstrip("#").strip())
        lines.append("### %s (last %d)" % (name, limit))
        for title in titles[-limit:]:
            # memorys are exempt from C49, but this hub is not: keep the digest
            # English-only so AGENTS.md stays inside the English gate.
            if re.search(r"[\u4e00-\u9fff]", title):
                continue
            lines.append("- " + title)
        lines.append("")
    return "\n".join(lines).rstrip()


def gates_block(source):
    path = os.path.join(repo_of(source), "scripts", "all-gates.sh")
    names = GATE_RE.findall(common.read_text(path)) if os.path.isfile(path) else []
    lines = [
        "## Gate list (generated from `scripts/all-gates.sh`; %d sites)" % len(names),
        "",
    ]
    if not names:
        lines.append("(no `gate \"` sites found)")
    for index, name in enumerate(names, 1):
        lines.append("%d. `%s`" % (index, name))
    return "\n".join(lines)


def skill_header():
    return (
        "# MediaServo Skills Registry\n\n"
        "> Generated by `pixi run agents sync` (source: `.agents/skills/`). "
        "Do not hand-edit the registry block.\n\n"
    )


def constraints_header():
    return (
        "# CONSTRAINTS -- what \"done\" means here\n\n"
        "Two parts: hand-written anti-gaming rules above, generated gate list below.\n\n"
        "## Anti-gaming rules\n"
        "- Never commit with `--no-verify`.\n"
        "- Never silence a warning with `#[allow]` without a named invariant.\n"
        "- Never delete or weaken an assertion to make a red gate green.\n"
        "- Fix the cause, not the gate.\n"
        "- Never hand-edit a generated layer.\n\n"
    )


def expected_markdown(path, begin, end, block, header):
    if os.path.isfile(path):
        text = common.read_text(path)
        replaced = common.replace_region(text, begin, end, block)
        if replaced is not None:
            return replaced
        return common.place_region(text, begin, end, block)
    return header + begin + "\n" + block.rstrip("\n") + "\n" + end + "\n"


def hub_text(source):
    count = len(skill_rows(source))
    return (
        "# msrtc workspace -- agent hub\n\n"
        "> Generated by `./msrtc.sh agents sync` (plan agent-infra-thin-shell). Do not hand-edit.\n\n"
        "## Command form (entry translation)\n"
        "MediaServo docs are written for its own repo root. In this workspace:\n"
        "- `pixi run <task>` -> `./msrtc.sh pixi <task>`\n"
        "- `pixi run -e <env> <task>` -> `./msrtc.sh -e <env> <task>`\n"
        "- MediaServo CLI verbs (`build`, `deploy`, `package`, `test`, `config`, ...) -> `./msrtc.sh <verb>`\n\n"
        "## Boundaries\n"
        "- MediaServo (`3rdparty/MediaServo`) owns skills, rules, memory, MCP pins, pixi and gates.\n"
        "- This shell owns 3rdparty pins, brand, release, shell docs and three gates.\n"
        "- Never edit `3rdparty/MediaServo/.agents/**` from here; edit it in the submodule and bump the pointer.\n\n"
        "## Entry points\n"
        "- `./msrtc.sh <task>` -- one-shot task (forwarded to the submodule)\n"
        "- `./msrtc.sh pixi <task>` -- pixi task passthrough\n"
        "- `./msrtc.sh agents check` -- generated layers vs source\n"
        "- `source bootstrap.sh` -- first-time setup\n\n"
        "## Memory\n"
        "- Shell facts: `.agents/memorys/` (active <=58 KiB; legacy `*-archived.md`).\n"
        "- Child memory index: `.agents/memorys/child-refs.md`; cite as `child:<id>`, never copy the body.\n\n"
        "## Skills\n"
        "- %d skill(s) come from the submodule; `.agents/skills` is a symlink to the submodule.\n" % count
    )


def expected_agents(source):
    path = os.path.join(repo_of(source), "AGENTS.md")
    if not os.path.isfile(path):
        return None
    text = common.read_text(path)
    text = common.place_region(text, SKILLS_BEGIN, SKILLS_END, skills_block(source), before=MEM_BEGIN)
    text = common.place_region(text, MEM_BEGIN, MEM_END, memory_block(source))
    return text


def build_artifacts(source, target_root, cfg):
    servers = canonical_mcp(source)
    arts = []
    for host in cfg.get("hosts", []):
        if not host.get("enabled", True):
            continue
        surfaces = host.get("surfaces", [])
        path = os.path.join(target_root, host["file"]) if host.get("file") else None
        if host.get("kind") == "mcp" and "mcp" in surfaces:
            arts.append(("json", path, {"mcpServers": host_mcp(servers, host["mcp_launcher"])}))
        elif host.get("kind") == "opencode":
            disk = common.load_json(path) if os.path.isfile(path) else {}
            if "instructions" in surfaces:
                disk["instructions"] = host.get("instructions", [])
            if "mcp" in surfaces:
                current = dict(disk.get("mcp", {}))
                for name, entry in opencode_mcp(servers, host["mcp_launcher"]).items():
                    current[name] = entry
                for name in host.get("drop_mcp", []):
                    current.pop(name, None)
                disk["mcp"] = current
            arts.append(("json", path, disk))
        elif host.get("kind") == "qoder":
            if "rules" in surfaces:
                source_dir = os.path.join(target_root, host["rules_source"])
                target_dir = os.path.join(target_root, host["rules_target"])
                for name in sorted(os.listdir(source_dir)):
                    if not name.endswith(".md"):
                        continue
                    body = "<!-- generated from %s/%s; edit the overlay source -->\n\n" % (
                        host["rules_source"], name)
                    body += common.read_text(os.path.join(source_dir, name))
                    arts.append(("text", os.path.join(target_dir, name), body))
    if cfg.get("hub"):
        arts.append(("text", os.path.join(target_root, "AGENTS.md"), hub_text(source)))
    if cfg.get("repo_artifacts"):
        repo = repo_of(source)
        arts.append(("region", os.path.join(repo, "SKILL.md"), SKILL_BEGIN, SKILL_END,
                     skills_block(source), skill_header()))
        arts.append(("region", os.path.join(repo, "CONSTRAINTS.md"), GATES_BEGIN, GATES_END,
                     gates_block(source), constraints_header()))
        agents = expected_agents(source)
        if agents is not None:
            arts.append(("text", os.path.join(repo, "AGENTS.md"), agents))
    return arts


def expected_for(art):
    kind = art[0]
    if kind == "json":
        return common.json_text(art[2])
    if kind == "text":
        return art[2]
    if kind == "region":
        return expected_markdown(art[1], art[2], art[3], art[4], art[5])
    raise ValueError(kind)


def run(command, source, target_root, targets_path):
    if not os.path.isdir(source):
        print("GATE-AGENTS ✗ source missing: %s" % source)
        return common.EXIT_GUARD
    if not os.path.isfile(targets_path):
        print("GATE-AGENTS ✗ targets missing: %s" % targets_path)
        return common.EXIT_GUARD
    cfg = common.load_json(targets_path)
    source_real = os.path.realpath(source)
    arts = build_artifacts(source, target_root, cfg)

    if command == "status":
        print("AGENTS status")
        print("  source: %s" % source)
        print("  target: %s" % target_root)
        print("  targets: %s" % targets_path)
        print("  skills: %d" % len(skill_rows(source)))
        print("  hosts: %s" % ", ".join(
            "%s=%s" % (h.get("label"), "on" if h.get("enabled", True) else "off")
            for h in cfg.get("hosts", [])))
        for art in arts:
            print("  %-10s %s" % ("present" if os.path.exists(art[1]) else "missing", art[1]))
        return common.EXIT_OK

    worst = common.EXIT_OK
    wrote = 0
    for art in arts:
        path = art[1]
        reason = common.guard_target(path, source_real)
        if reason:
            print("GATE-AGENTS ✗ guard %s" % reason)
            return common.EXIT_GUARD
        expected = expected_for(art)
        if command == "check":
            if not os.path.isfile(path):
                print(" [ FIX ] %s (missing)" % path)
                worst = max(worst, common.EXIT_FINDINGS)
                continue
            current = common.read_text(path)
            if art[0] == "json":
                try:
                    same = common.load_json(path) == art[2]
                except ValueError:
                    same = False
            else:
                same = current == expected
            if not same:
                print(" [ FIX ] %s (drift)" % path)
                worst = max(worst, common.EXIT_FINDINGS)
            continue
        if os.path.isfile(path) and common.read_text(path) == expected:
            continue
        common.write_text(path, expected)
        print(" [ WRITE] %s" % path)
        wrote += 1

    if command == "sync":
        if wrote:
            print("GATE-AGENTS ✓ applied %d change(s)" % wrote)
        else:
            print("GATE-AGENTS ✓ already in sync (0 writes)")
    if command == "check":
        print("GATE-AGENTS %s" % ("✓ all layers clean" if worst == common.EXIT_OK else "✗ findings -- run: agents sync"))
    return worst


def main(argv):
    command, source, target_root, targets_path = parse_args(argv)
    return run(command, source, target_root, targets_path)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
