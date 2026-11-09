#!/usr/bin/env python3
"""MCP start probe — four states: green / skip-credential / skip-env / red.

Registry: scripts/agents/mcp-servers.json (the single pin source).
Each npm server is started through mcp-launch.sh with empty stdin: a clean
start-then-EOF is green, a credential error is skip-credential (not a failure),
a non-npm kind is skip-env (counted separately, never folded into green), and
anything else is red.

Exit: 0 if no red, 1 otherwise.
"""

import json
import os
import re
import signal
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
REGISTRY = os.path.join(HERE, "mcp-servers.json")
LAUNCHER = os.path.join(HERE, "mcp-launch.sh")
TIMEOUT = int(os.environ.get("MCP_PROBE_TIMEOUT", "300"))
TOKEN_HINT = re.compile(r"token|unauthorized|401|credential", re.IGNORECASE)
ROUTING = os.path.join(ROOT, ".agents", "rules", "common", "tool-routing.md")
LOCKED_SERVERS = ("local-codegraph", "local-serena", "local-playwright")


def load(path, label):
    try:
        with open(path, encoding="utf-8") as handle:
            data = json.load(handle)
    except (OSError, json.JSONDecodeError) as exc:
        print(f" [SHAPE ] {label:16s} {'':16s} unreadable: {exc}")
        return None
    if not isinstance(data, dict):
        print(f" [SHAPE ] {label:16s} {'':16s} top level must be an object")
        return None
    return data


def shape_check() -> int:
    """B4-5 static form gate -- the complement of the start-probe below.

    Checks file shapes only: single-source entry forms, host-file command
    being a STRING (array is the opencode form), launcher targets existing,
    env values being strings. Service-name-set equality vs the single source
    stays with `agents check` (the whole-file rewrite is the strong gate
    there) -- this gate never duplicates that ledger.
    """
    red = 0

    def bad(surface, name, why):
        nonlocal red
        print(f" [SHAPE ] {surface:16s} {name:16s} {why}")
        red += 1

    source = load(os.path.join(ROOT, ".agents", "mcp.json"), "source")
    if source is None:
        return 1
    servers = source.get("mcpServers")
    if not isinstance(servers, dict):
        print(" [SHAPE ] source           missing mcpServers object")
        return 1
    for name, entry in servers.items():
        etype = entry.get("type", "stdio")
        if etype not in ("stdio", "remote"):
            bad("source", name, f"bad type {etype!r}")
            continue
        if etype == "remote" and not isinstance(entry.get("url"), str):
            bad("source", name, "remote entry needs a string url")
        if etype == "stdio" and not isinstance(entry.get("command"), str):
            bad("source", name, "stdio entry needs a string command")
        args = entry.get("args")
        if args is not None and (not isinstance(args, list)
                                 or not all(isinstance(x, str) for x in args)):
            bad("source", name, "args must be a string array")
        env = entry.get("env")
        if env is not None and (not isinstance(env, dict)
                                or not all(isinstance(v, str) for v in env.values())):
            bad("source", name, "env must map to string values (${VAR} form preferred)")

    host = load(os.path.join(ROOT, ".mcp.json"), ".mcp.json")
    if host is not None:
        for name, entry in host.get("mcpServers", {}).items():
            cmd = entry.get("command")
            if not isinstance(cmd, str):
                bad(".mcp.json", name, "command must be a STRING (array is the opencode form)")
            elif "/" in cmd and not cmd.startswith("$"):
                target = cmd if os.path.isabs(cmd) else os.path.join(ROOT, cmd)
                if not os.path.isfile(target):
                    bad(".mcp.json", name, f"command target missing: {cmd}")

    oc = load(os.path.join(ROOT, ".opencode", "opencode.json"), "opencode.json")
    if oc is not None:
        for name, entry in oc.get("mcp", {}).items():
            etype = entry.get("type")
            if etype not in ("local", "remote"):
                bad("opencode.json", name, f"bad type {etype!r}")
            elif etype == "remote":
                if not isinstance(entry.get("url"), str):
                    bad("opencode.json", name, "remote entry needs a string url")
            else:
                command = entry.get("command")
                if not isinstance(command, list) or not command:
                    bad("opencode.json", name, "local entry needs a non-empty command array")
                elif len(command) >= 2 and command[0] == "bash" \
                        and not os.path.isfile(os.path.join(ROOT, command[1])):
                    bad("opencode.json", name, f"launcher missing: {command[1]}")

    if red:
        print(f"GATE-MCP shape ✗ ({red} issue(s)) -- fix forms before probing")
    return 1 if red else 0


def lock_check() -> int:
    """B5-4 rule-lock: a locked server named in any host face <=> its row in
    tool-routing.md. Kills both "MCP removed but rules still teach it" and
    the reverse."""
    red = 0
    text = ""
    if os.path.isfile(ROUTING):
        text = open(ROUTING, encoding="utf-8").read()
    named = set()
    for label, path, getter in ((".mcp.json", os.path.join(ROOT, ".mcp.json"),
                                 lambda d: list(d.get("mcpServers", {}))),
                                ("opencode.json", os.path.join(ROOT, ".opencode", "opencode.json"),
                                 lambda d: list(d.get("mcp", {})))):
        data = load(path, label)
        if data:
            named.update(n for n in getter(data) if n in LOCKED_SERVERS)
    for server in LOCKED_SERVERS:
        # Row-level match: the name must appear in a matrix table row, not
        # merely anywhere in the file (the lock-in prose mentions all three).
        row_present = any(line.lstrip().startswith("|") and server in line
                          for line in text.splitlines())
        if server in named and not row_present:
            print(f" [LOCK  ] {server:16s} named in host face but tool-routing.md row missing")
            red += 1
        if server not in named and row_present:
            print(f" [LOCK  ] {server:16s} routing row exists but server absent from every host face")
            red += 1
    if red:
        print(f"GATE-MCP lock ✗ ({red} issue(s))")
    return 1 if red else 0


def main() -> int:
    shape_rc = shape_check()
    if shape_rc:
        return shape_rc
    if lock_check():
        return 1

    with open(REGISTRY, encoding="utf-8") as handle:
        servers = json.load(handle)

    green = yellow = skip_env = red = 0
    for name, spec in servers.items():
        if spec.get("kind") == "pixi":
            env_dir = os.path.join(ROOT, ".pixi", "envs", spec["pixi_env"])
            if not os.path.isdir(env_dir):
                print(f" [SKIPENV] {name:16s} pixi env '{spec['pixi_env']}' missing")
                skip_env += 1
                continue
        elif spec.get("kind") != "npm":
            print(f" [SKIPENV] {name:16s} kind={spec.get('kind')}")
            skip_env += 1
            continue

        label = spec.get("pkg") or spec.get("task") or spec.get("kind")
        print(f" [PROBE]  {name:16s} {label}@{spec['version']}")
        probe = spec.get("probe")
        if probe:
            # B5-3: cheap version probe -- no server start, no timeout burn.
            if spec.get("kind") == "pixi":
                argv = [os.path.join(ROOT, ".pixi", "envs", spec["pixi_env"], "bin", probe[0])]
            else:
                argv = [os.path.join(HERE, "node_modules", ".mcp", name,
                                     "node_modules", ".bin", probe[0])]
            try:
                proc = subprocess.run(argv + list(probe[1:]), cwd=ROOT,
                                      capture_output=True, text=True, timeout=60)
            except subprocess.TimeoutExpired:
                print(f" [RED   ] {name:16s} version probe timeout")
                red += 1
                continue
            if proc.returncode == 0:
                first = (proc.stdout or proc.stderr).strip().splitlines()
                print(f" [VER   ] {name:16s} {first[-1][:60] if first else 'version ok'}")
                green += 1
            else:
                print(f" [RED   ] {name:16s} version probe failed (exit {proc.returncode})")
                red += 1
            continue
        proc = subprocess.Popen(
            ["bash", LAUNCHER, name],
            cwd=ROOT,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            start_new_session=True,
        )
        timed_out = False
        try:
            stdout, stderr = proc.communicate(timeout=TIMEOUT)
        except subprocess.TimeoutExpired:
            timed_out = True
            stdout, stderr = "", ""
            os.killpg(proc.pid, signal.SIGTERM)
            proc.communicate()
        finally:
            # The launcher may exit before its pixi/node child; kill the whole group.
            try:
                os.killpg(proc.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass

        if timed_out:
            print(f" [RED   ] {name:16s} probe timeout after {TIMEOUT}s")
            red += 1
            continue

        output = (stdout or "") + (stderr or "")
        if proc.returncode == 0:
            print(f" [OK    ] {name:16s} starts")
            green += 1
            continue
        if TOKEN_HINT.search(output):
            print(f" [SKIP  ] {name:16s} missing credential (expected in CI)")
            yellow += 1
            continue
        tail = output.strip().splitlines()
        print(f" [RED   ] {name:16s} exit={proc.returncode}")
        if tail:
            print(f"          {tail[-1][:160]}")
        red += 1

    mark = "✓" if red == 0 else "✗"
    print(f"GATE-MCP {mark} ({green} green, {yellow} skipped-credential, {skip_env} skipped-env)")
    return 1 if red else 0


if __name__ == "__main__":
    sys.exit(main())
