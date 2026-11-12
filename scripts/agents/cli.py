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
    target = os.path.abspath(args.target_root) if args.target_root else common.ROOT
    targets = os.path.abspath(args.targets) if args.targets else os.path.join(target, ".agents", "targets.json")
    if args.source:
        source = os.path.abspath(args.source)
    else:
        # O-20: cfg["source"] is the declared true source (was a dead field).
        # --source stays the explicit override; absent it, resolve cfg["source"]
        # relative to the target root. Never falls back across repos.
        cfg_source = None
        if os.path.isfile(targets):
            try:
                cfg_source = common.load_json(targets).get("source")
            except ValueError:
                cfg_source = None
        source = (os.path.abspath(os.path.join(target, cfg_source))
                  if cfg_source else os.path.join(common.ROOT, ".agents"))
    return args.command, source, target, targets


def repo_of(source):
    return os.path.dirname(source)


def load_registry(source):
    path = os.path.join(repo_of(source), "scripts", "agents", "mcp-servers.json")
    return common.load_json(path)


def canonical_mcp(source):
    path = os.path.join(source, "mcp.json")
    if not os.path.isfile(path):
        return {}
    return common.load_json(path).get("mcpServers", {})


def host_mcp(servers, launcher, accepts_types=("stdio",)):
    """Project the single-source servers onto a .mcp.json-style host.

    Entries whose type is not in accepts_types are skipped (the accepted set
    is declared per host in targets.json, B5-1 A9=C). This host format has no
    enabled field and no remote shape, so "stdio" is the default acceptance."""
    out = {}
    for name, entry in servers.items():
        if entry.get("type", "stdio") not in accepts_types:
            continue
        item = copy.deepcopy(entry)
        args = list(item.get("args", []))
        if args:
            args[0] = launcher
        else:
            args = [launcher, name]
        item["args"] = args
        out[name] = item
    return out


def opencode_mcp(servers, launcher, accepts_types=("stdio", "remote"),
                 filter_disabled=False):
    """Project the single-source servers onto an opencode-style host,
    branching per entry type (B5-1, A9=C). This removes O-11: the previous
    body hard-coded {"type": "local", ...} for every entry, so any remote
    entry admitted into the single source would have been rendered as a
    local server pointing at a task that does not exist."""
    out = {}
    for name, entry in servers.items():
        etype = entry.get("type", "stdio")
        if etype not in accepts_types:
            continue
        if filter_disabled and not entry.get("enabled", True):
            continue
        enabled = entry.get("enabled", True)
        if etype == "remote":
            out[name] = {"type": "remote", "url": entry["url"], "enabled": enabled}
        else:
            args = list(entry.get("args", []))
            if args:
                args[0] = launcher
            else:
                args = [name]
            out[name] = {"type": "local", "enabled": enabled,
                         "command": [entry.get("command", "bash")] + args}
    return out


def skill_rows(source):
    root = os.path.join(source, "skills")
    rows = []
    if not os.path.isdir(root):
        return rows
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
        # B4-4: surface the status gate in the registry view so no consumer
        # picks up an inoperative skill without the warning.
        text = open(skill, encoding="utf-8").read()
        if re.match(r"^---\n.*?^status:\s*inoperative\s*$", text, re.M | re.S):
            desc = "[inoperative] " + desc
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
    agents_dir = os.path.join(source, "agents")
    agent_count = len([f for f in os.listdir(agents_dir)
                       if f.endswith(".md")]) if os.path.isdir(agents_dir) else 0
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
        "- Never edit `3rdparty/MediaServo/.agents/**` from here; edit it in the submodule and bump the pointer.\n"
        "- `.refinfo/**` at the workspace top level is read-only reference: never edit or `git add` anything under it.\n\n"
        "## Entry points\n"
        "- `./msrtc.sh <task>` -- one-shot task (forwarded to the submodule)\n"
        "- `./msrtc.sh pixi <task>` -- pixi task passthrough\n"
        "- `./msrtc.sh agents check` -- generated layers vs source\n"
        "- `source bootstrap.sh` -- first-time setup\n\n"
        "## Memory\n"
        "- Shell facts: `.agents/memorys/` (active <=58 KiB; legacy `*-archived.md`).\n"
        "- Child memory index: `.agents/memorys/child-refs.md`; cite as `child:<id>`, never copy the body.\n\n"
        "## Skills\n"
        "- %d skill(s) come from the submodule; `.agents/skills` is a symlink to the submodule.\n\n"
        "## Agents\n"
        "- %d role(s) from the submodule, linked into each host's `agents/` dir (not copied); "
        "run `./msrtc.sh agents sync`.\n" % (count, agent_count)
    )


def expected_agents(source):
    path = os.path.join(repo_of(source), "AGENTS.md")
    if not os.path.isfile(path):
        return None
    text = common.read_text(path)
    text = common.place_region(text, SKILLS_BEGIN, SKILLS_END, skills_block(source), before=MEM_BEGIN)
    text = common.place_region(text, MEM_BEGIN, MEM_END, memory_block(source))
    text = common.place_region(text, RULES_INDEX_BEGIN, RULES_INDEX_END,
                               rules_index_block(source))
    return text


# --- agent role layer (agent-subagent-layer B2) ---------------------------
# Semantic verb -> host tool dialect. Canonical roles speak lowercase semantic
# verbs (plan 3.3); the renderer translates per host. Qoder is a read-only
# TOGGLE, not a strict whitelist (O-32): a tools field forces read-only mode,
# so writable Qoder roles must OMIT tools entirely.
CB_TOOLS = {"read": "Read", "grep": "Grep", "glob": "Glob", "shell": "Bash",
            "write": "Write", "edit": "Edit", "task": "Task",
            "webfetch": "WebFetch", "websearch": "WebSearch", "skill": "Skill"}
QODER_TOOLS = dict(CB_TOOLS, edit="SearchReplace", task="Agent")
OC_TOOLS = {"read": "read", "grep": "grep", "glob": "glob", "shell": "bash",
            "write": "write", "edit": "edit", "task": "task",
            "webfetch": "webfetch", "websearch": "websearch", "skill": "skill"}
WRITE_VERBS = ("write", "edit")
RULES_INDEX_BEGIN = "<!-- BEGIN:rules-index -->"
RULES_INDEX_END = "<!-- END:rules-index -->"


def parse_agent(path):
    """Parse a canonical role file into a dict (semantic schema, plan 3.3)."""
    text = common.read_text(path)
    match = re.match(r"^---\n(.*?)\n---\n", text, re.S)
    block = match.group(1) if match else ""
    body = text[match.end():] if match else text

    def field(key):
        m = re.search(r"^%s:[ \t]*(.+)$" % key, block, re.M)
        return m.group(1).strip() if m else None

    def lst(key):
        value = field(key)
        if not value:
            return []
        if value.startswith("[") and value.endswith("]"):
            value = value[1:-1]
        return [x.strip().strip("'\"") for x in value.split(",") if x.strip()]

    return {
        "name": field("name") or os.path.basename(path)[:-3],
        "description": field("description") or "",
        "mode": field("mode") or "subagent",
        "read_only": (field("read_only") or "").lower() == "true",
        "tools": lst("tools"),
        "skills": lst("skills"),
        "model": field("model"),
        "body": body,
    }


def render_agent(kind, agent, dialect="v1"):
    """Render a canonical role into a host dialect (plan 3.4 rendering matrix)."""
    tools = agent["tools"]
    ro = agent["read_only"]
    if kind == "opencode":
        # V1 (1.19.3): tools boolean map; name omitted (id = file name), mode kept.
        lines = ["---", "description: %s" % agent["description"],
                 "mode: %s" % agent["mode"]]
        tmap = {}
        for verb in tools:
            if verb in OC_TOOLS:
                tmap[OC_TOOLS[verb]] = True
        if ro:
            tmap["write"] = False
            tmap["edit"] = False
            tmap["bash"] = False
        if tmap:
            lines.append("tools:")
            for key in sorted(tmap):
                lines.append("  %s: %s" % (key, "true" if tmap[key] else "false"))
        lines.append("---")
        return "\n".join(lines) + "\n" + agent["body"]

    lines = ["---", "name: %s" % agent["name"],
             "description: %s" % agent["description"]]
    if kind == "codebuddy":
        eff = [v for v in tools if not (ro and v in WRITE_VERBS)]
        if eff:
            lines.append("tools: %s" % ", ".join(
                CB_TOOLS[v] for v in eff if v in CB_TOOLS))
    elif kind == "qoder":
        # Two-state (O-32): read-only -> emit a tools field (forces read-only
        # mode); writable -> omit tools (full mode). Never a strict whitelist.
        if ro:
            eff = [v for v in tools if v not in WRITE_VERBS] or ["read", "grep", "glob"]
            lines.append("tools: %s" % ", ".join(
                QODER_TOOLS[v] for v in eff if v in QODER_TOOLS))
    if agent["skills"] and kind in ("codebuddy", "qoder"):
        lines.append("skills: %s" % ", ".join(agent["skills"]))
    if agent["model"] and kind in ("codebuddy", "qoder"):
        lines.append("model: %s" % agent["model"])
    lines.append("---")
    return "\n".join(lines) + "\n" + agent["body"]


def rules_index_block(source):
    """Index of product rules (path + one-line applicability), English (C49).

    Zero-copy inheritance (plan 3.14): the parent reads product rules through
    this index, never a full projection. Paths are relative to the submodule
    AGENTS.md directory; the parent hub carries the submodule prefix.
    """
    root = os.path.join(source, "rules")
    lines = [
        "## Rule index (generated -- read the rule file when its condition applies)",
        "",
        "Paths are relative to this file's directory.",
        "",
        "| Rule | When to read |",
        "|------|--------------|",
    ]
    if os.path.isdir(root):
        for dirpath, _dirnames, filenames in os.walk(root):
            for fn in sorted(filenames):
                if not fn.endswith(".md") or fn == "README.md":
                    continue
                rel = common.rel(os.path.join(dirpath, fn), repo_of(source))
                domain = os.path.basename(dirpath)
                lines.append("| `%s` | %s work |" % (rel, domain))
    return "\n".join(lines)


def _agents_arts(source, target_root, cfg, host):
    """Build the agents-face artifacts for one host (B2).

    Self-owned roles are rendered into the host dir. Inherited roles (strict
    opt-in via cfg["inherit"], parent only) are SYMLINKS to the child's
    already-rendered artifacts -- zero content copy (plan 3.14 / N26).
    """
    kind = host.get("kind")
    agents_target = host.get("agents_target")
    dialect = cfg.get("opencode_dialect", "v1")
    target_dir = os.path.join(target_root, agents_target)
    arts = []
    keep = set()
    # Self-owned roles: S mode target_root == repo_of(source) -> source/agents;
    # P mode -> the parent's own .agents/agents (e.g. release-captain).
    self_dir = os.path.join(target_root, ".agents", "agents")
    if os.path.isdir(self_dir):
        for fn in sorted(os.listdir(self_dir)):
            if not fn.endswith(".md"):
                continue
            agent = parse_agent(os.path.join(self_dir, fn))
            arts.append(("text", os.path.join(target_dir, fn),
                         render_agent(kind, agent, dialect)))
            keep.add(fn)
    # Inherited roles: never fall back to --source; only when explicitly opted
    # in AND this is the parent (target_root != the source's repo).
    inherit = cfg.get("inherit")
    is_parent = os.path.abspath(target_root) != os.path.abspath(repo_of(source))
    if inherit and is_parent:
        child_dir = os.path.join(repo_of(source), agents_target)
        if os.path.isdir(child_dir):
            for fn in sorted(os.listdir(child_dir)):
                if not fn.endswith(".md"):
                    continue
                if fn in keep:
                    raise ValueError(
                        "agent name collision (self vs inherited): %s" % fn)
                arts.append(("link", os.path.join(target_dir, fn),
                             os.path.join(child_dir, fn)))
                keep.add(fn)
    arts.append(("prune", target_dir, keep))
    return arts


def _rule_description(rule_body, stem):
    for line in rule_body.splitlines():
        if line.startswith("# "):
            return line[2:].strip()[:120]
    return stem.replace("-", " ")


def build_artifacts(source, target_root, cfg):
    if cfg.get("hub") and cfg.get("repo_artifacts"):
        raise ValueError("hub and repo_artifacts are mutually exclusive "
                         "(both write the same AGENTS.md)")
    servers = canonical_mcp(source)
    arts = []
    for host in cfg.get("hosts", []):
        if not host.get("enabled", True):
            continue
        surfaces = host.get("surfaces", [])
        path = os.path.join(target_root, host["file"]) if host.get("file") else None
        # Independent agents branch (B2): MUST NOT be a trailing elif -- the
        # opencode/qoder kinds are already matched below, so an elif here would
        # never run for them. Runs for any host kind declaring the agents face.
        if "agents" in surfaces:
            arts.extend(_agents_arts(source, target_root, cfg, host))
        if host.get("kind") == "mcp" and "mcp" in surfaces:
            arts.append(("json", path, {"mcpServers": host_mcp(
                servers, host["mcp_launcher"], host.get("accepts_types", ("stdio",)))}))
        elif host.get("kind") == "opencode":
            disk = common.load_json(path) if os.path.isfile(path) else {}
            if "instructions" in surfaces:
                disk["instructions"] = host.get("instructions", [])
            if "mcp" in surfaces:
                # B4-1: whole-section equal rewrite. want is computed from the
                # single source alone; anything on disk outside want (orphans)
                # disappears instead of surviving the old merge + drop_mcp dance
                # (whose blind spot: a source-side removal nobody mirrored into
                # drop_mcp stayed on disk forever, invisible to check).
                disk["mcp"] = opencode_mcp(
                    servers, host["mcp_launcher"], host.get("accepts_types", ("stdio", "remote")),
                    host.get("filter_disabled", False))
            arts.append(("json", path, disk))
        elif host.get("kind") == "qoder":
            if "rules" in surfaces:
                source_dir = os.path.join(target_root, host["rules_source"])
                target_dir = os.path.join(target_root, host["rules_target"])
                for name in sorted(os.listdir(source_dir)):
                    if not name.endswith(".md"):
                        continue
                    body = "<!-- generated from %s/%s; edit %s, not this copy -->\n\n" % (
                        host["rules_source"], name, host["rules_source"])
                    body += common.read_text(os.path.join(source_dir, name))
                    arts.append(("text", os.path.join(target_dir, name), body))
                # B4-1 (A10=B): the rules dir is judged two-way now -- a file
                # left in the target dir that no longer exists in the overlay
                # is stale and gets pruned. Guardrails: .md only, and
                # `agents check` is the dry-run (reports, deletes nothing).
                keep = {name for name in os.listdir(source_dir) if name.endswith(".md")}
                arts.append(("prune", target_dir, keep))
        elif host.get("kind") == "codebuddy":
            # N20=A: CodeBuddy rules are <name>/RULE.mdc folders + frontmatter
            # (flat .md is NOT injected -- B0-3 v1 proven). Reuse qoder's prune
            # semantics but render the folder dialect; prune is folder-level.
            if "rules" in surfaces:
                source_dir = os.path.join(target_root, host["rules_source"])
                target_dir = os.path.join(target_root, host["rules_target"])
                keep = set()
                if os.path.isdir(source_dir):
                    for name in sorted(os.listdir(source_dir)):
                        if not name.endswith(".md"):
                            continue
                        stem = name[:-3]
                        rule_body = common.read_text(os.path.join(source_dir, name))
                        mdc = ("---\ndescription: %s\nalwaysApply: true\n---\n\n"
                               % _rule_description(rule_body, stem)) + rule_body
                        arts.append(("text", os.path.join(target_dir, stem, "RULE.mdc"), mdc))
                        keep.add(stem)
                arts.append(("prune_tree", target_dir, keep))
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
    if kind in ("prune", "prune_tree", "link"):
        return None  # handled specially in run()
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
    try:
        arts = build_artifacts(source, target_root, cfg)
    except ValueError as exc:
        print("GATE-AGENTS ✗ %s" % exc)
        return common.EXIT_GUARD

    # expected_hosts is a voluntary promise (B2): a declared-but-missing host
    # is a finding, so "silent missing host" can never pass again.
    expected_hosts = cfg.get("expected_hosts", [])
    declared = {h.get("label") for h in cfg.get("hosts", []) if h.get("enabled", True)}
    missing_hosts = [h for h in expected_hosts if h not in declared]

    if command == "status":
        print("AGENTS status")
        print("  source: %s" % source)
        print("  target: %s" % target_root)
        print("  targets: %s" % targets_path)
        print("  skills: %d" % len(skill_rows(source)))
        print("  hosts: %s" % ", ".join(
            "%s=%s" % (h.get("label"), "on" if h.get("enabled", True) else "off")
            for h in cfg.get("hosts", [])))
        if expected_hosts:
            print("  expected: %s | declared: %s | missing: %s" % (
                ",".join(expected_hosts), ",".join(sorted(declared)),
                ",".join(missing_hosts) if missing_hosts else "(none)"))
        seen_paths = set()
        for art in arts:
            if art[1] in seen_paths:
                continue  # de-dupe (a dir and its files were listed twice)
            seen_paths.add(art[1])
            print("  %-10s %s" % ("present" if os.path.exists(art[1]) else "missing", art[1]))
        return common.EXIT_OK

    worst = common.EXIT_OK
    if command == "check" and missing_hosts:
        for host in missing_hosts:
            print(" [ FIX ] host %s: promised in expected_hosts but not declared" % host)
        worst = common.EXIT_FINDINGS

    wrote = 0
    own_agents_real = os.path.realpath(os.path.join(target_root, ".agents"))
    # Inherit opted in but the child has not rendered its agents (or the
    # submodule is absent): surface a WARN, never block the parent (B2 / M-6).
    if cfg.get("inherit") and os.path.abspath(target_root) != os.path.abspath(repo_of(source)):
        for host in cfg.get("hosts", []):
            if "agents" in host.get("surfaces", []) and host.get("agents_target"):
                child_dir = os.path.join(repo_of(source), host["agents_target"])
                if not os.path.isdir(child_dir):
                    print(" [ WARN] inherit: child agents not rendered (%s); "
                          "run sync in submodule" % child_dir)
    for art in arts:
        path = art[1]
        reason = common.guard_target(path, source_real)
        if reason:
            print("GATE-AGENTS ✗ guard %s" % reason)
            return common.EXIT_GUARD
        # Self-source guard: the target's own .agents is outside source_real in
        # P mode, so guard_target alone would not protect it. Never write there.
        path_real = os.path.realpath(path)
        if path_real == own_agents_real or path_real.startswith(own_agents_real + os.sep):
            print("GATE-AGENTS ✗ guard target inside own .agents: %s" % path)
            return common.EXIT_GUARD
        if art[0] == "link":
            link_to = art[2]
            rel_target = os.path.relpath(link_to, os.path.dirname(path)).replace(os.sep, "/")
            child_rendered = os.path.isfile(link_to)
            if command == "check":
                if not os.path.islink(path):
                    if child_rendered:
                        print(" [ FIX ] %s (missing link)" % path)
                        worst = max(worst, common.EXIT_FINDINGS)
                    else:
                        print(" [ WARN] %s (child agents not rendered)" % path)
                elif os.readlink(path) != rel_target:
                    print(" [ FIX ] %s (link drift)" % path)
                    worst = max(worst, common.EXIT_FINDINGS)
                elif not os.path.isfile(path):
                    print(" [ FIX ] %s (dangling link)" % path)
                    worst = max(worst, common.EXIT_FINDINGS)
                continue
            if os.path.islink(path) and os.readlink(path) == rel_target and os.path.isfile(path):
                continue
            if not child_rendered:
                # Submodule absent / child not synced: downgrade to warn so a
                # fresh clone never blocks the parent (B2 / M-6). Never write.
                print(" [ WARN] %s (run sync in submodule first)" % path)
                continue
            os.makedirs(os.path.dirname(path), exist_ok=True)
            if os.path.islink(path) or os.path.exists(path):
                os.remove(path)
            os.symlink(rel_target, path)  # link-only: never write_text (no write-through)
            print(" [ LINK] %s -> %s" % (path, rel_target))
            wrote += 1
            continue
        if art[0] == "prune_tree":
            target_dir, keep = art[1], art[2]
            stale = sorted(
                name for name in os.listdir(target_dir)
                if os.path.isdir(os.path.join(target_dir, name)) and name not in keep
            ) if os.path.isdir(target_dir) else []
            if command == "check":
                for name in stale:
                    print(" [ FIX ] %s (stale rule folder)" % os.path.join(target_dir, name))
                    worst = max(worst, common.EXIT_FINDINGS)
                continue
            for name in stale:
                folder = os.path.join(target_dir, name)
                mdc = os.path.join(folder, "RULE.mdc")
                if os.path.isfile(mdc):
                    os.remove(mdc)
                if os.path.isdir(folder) and not os.listdir(folder):
                    os.rmdir(folder)
                print(" [ PRUNE] %s" % folder)
                wrote += 1
            continue
        if art[0] == "prune":
            target_dir, keep = art[1], art[2]
            stale = sorted(
                name for name in os.listdir(target_dir)
                if name.endswith(".md") and name not in keep
            ) if os.path.isdir(target_dir) else []
            if command == "check":
                for name in stale:
                    print(" [ FIX ] %s (stale, not in source overlay)" % os.path.join(target_dir, name))
                    worst = max(worst, common.EXIT_FINDINGS)
                continue
            victims = [os.path.join(target_dir, name) for name in stale]
            for victim in victims:
                # Symlink-aware (B2 / NC1): os.remove unlinks a symlink without
                # touching its target, so stale inherited links can be cleaned.
                # Refuse only real directories.
                if os.path.isdir(victim) and not os.path.islink(victim):
                    print("GATE-AGENTS ✗ prune guard refused %s" % victim)
                    return common.EXIT_GUARD
            for victim in victims:
                os.remove(victim)
                print(" [ PRUNE] %s" % victim)
                wrote += 1
            continue
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
