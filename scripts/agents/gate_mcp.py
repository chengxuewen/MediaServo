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


def main() -> int:
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
