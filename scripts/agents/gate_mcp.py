#!/usr/bin/env python3
"""gate_mcp (B12, plan matrix MCP row): pins, quota, and user/project same-name.

- every mcp-servers.json pin carries a version (no floating npm)
- every .mcp.json entry routes via mcp-launch.sh or is an explicit remote url
- quota <= 5 (the cap comes from the SAME quota table check_quotas parses)
- same-name between project .mcp.json and the user-level ~/.codebuddy/mcp.json:
  user file missing -> SKIP + note (exit 0, VISIBLY noted -- CI must never read
  a skip as a silent green); overlap -> red.
  Fixture: set CHECK_MCP_USER_FILE to simulate a user file without touching $HOME.
"""
import json
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
DEFAULT_ROOT = os.path.dirname(os.path.dirname(HERE))
sys.path.insert(0, HERE)

import check_quotas  # noqa: E402  (single-source quota table parser)


def user_level_path():
    return os.environ.get("CHECK_MCP_USER_FILE",
                          os.path.expanduser(os.path.join("~", ".codebuddy", "mcp.json")))


def load(path):
    if not os.path.isfile(path):
        return None
    with open(path, encoding="utf-8") as fh:
        return json.load(fh)


def run(root):
    fails = []
    notes = []

    # 1. Pins: every registry entry carries a version.
    pins_path = os.path.join(root, "scripts", "agents", "mcp-servers.json")
    pins = load(pins_path) or {}
    if not pins:
        fails.append("pin registry missing/empty: scripts/agents/mcp-servers.json")
    for name, pin in sorted(pins.items()):
        if not isinstance(pin, dict) or not pin.get("version"):
            fails.append("pin %s floating (no version)" % name)

    # 2. Project face: launcher-routed or explicit remote; quota <= 5.
    project = load(os.path.join(root, ".agents", "mcp.json")) or {}
    servers = project.get("mcpServers", {})
    for name, entry in sorted(servers.items()):
        args = entry.get("args", []) or []
        routed = (entry.get("command") in ("bash", "sh")
                  and any(str(a).endswith("mcp-launch.sh") for a in args))
        remote = entry.get("type") == "remote" and bool(entry.get("url"))
        if not (routed or remote):
            fails.append("entry %s neither launcher-routed nor explicit remote" % name)
    caps = check_quotas.parse_caps(
        os.path.join(root, ".agents", "rules", "common", "agents.md"))
    cap_mcp = next((v for k, v in caps.items() if "mcp" in k), None)
    # Count cap REMOVED by user ruling (11-13): quality is gated by pins +
    # same-name, not by count. cap_mcp stays None for a numeric-cap table.
    if cap_mcp is not None and len(servers) > cap_mcp:
        fails.append("MCP servers %d > cap %d" % (len(servers), cap_mcp))

    # 3. Same-name vs user level (skip VISIBLE when the user file is absent).
    user_path = user_level_path()
    user = load(user_path)
    if user is None:
        notes.append("SKIP: user-level file absent (%s) -- same-name check not "
                     "applicable; CI environments must see this note, not a "
                     "silent green" % user_path)
    else:
        user_keys = set((user or {}).get("mcpServers", {}).keys())
        overlap = sorted(user_keys & set(servers.keys()))
        if overlap:
            fails.append("same-name MCP in user and project level: %s "
                         "(project pin should win; disable one)" % ", ".join(overlap))
        else:
            notes.append("user-level %d server(s), no same-name overlap" % len(user_keys))

    proj_fmt = ("%d > cap %d" % (len(servers), cap_mcp)) if cap_mcp is not None \
        else "%d (no count cap)" % len(servers)
    summary = ("GATE-MCP %s pins %d, project %s, same-name %s | %s"
               % ("✓" if not fails else "✗", len(pins), proj_fmt,
                  "checked" if user is not None else "SKIPPED (user file absent)",
                  "; ".join(notes) if notes else "no notes"))
    return len(fails), fails + [summary]


def main():
    root = os.path.abspath(sys.argv[1]) if len(sys.argv) > 1 else DEFAULT_ROOT
    fails, lines = run(root)
    summary = lines[-1]
    for line in lines[:-1]:
        print("GATE-MCP ✗ " + line)
    print(("GATE-MCP ✓ " if not fails else "GATE-MCP ✗ ") + summary)
    sys.exit(1 if fails else 0)


if __name__ == "__main__":
    main()
