#!/usr/bin/env python3
"""MCP lifecycle: status / setup / sync / index.

`status` is read-only and always exits 0. `setup` installs npm servers at their
pins and the pixi environment; `index` builds the codegraph index and the serena
LSP cache; `sync` updates both incrementally where the tool supports it.

Usage:
    python3 scripts/agents/mcp_cli.py status
    python3 scripts/agents/mcp_cli.py setup|sync|index
"""

import json
import os
import shutil
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
REGISTRY = os.path.join(HERE, "mcp-servers.json")
PROJECT = os.environ.get("MEDIASERVO_AGENT_ROOT", ROOT)
NPM_CACHE = os.path.join(HERE, "node_modules", ".npm-cache")
PREFIX = "[mcp]"


def specs():
    with open(REGISTRY, encoding="utf-8") as handle:
        return json.load(handle)


def npm_bin():
    candidate = os.path.join(ROOT, ".pixi", "envs", "default", "bin", "npm")
    if os.path.isfile(candidate):
        return candidate
    return shutil.which("npm") or "npm"


def pixi_bin():
    found = shutil.which("pixi")
    if found:
        return found
    candidate = os.path.join(os.path.expanduser("~"), ".pixi", "bin", "pixi")
    return candidate if os.path.isfile(candidate) else "pixi"


def server_prefix(name):
    return os.path.join(HERE, "node_modules", ".mcp", name)


def npm_version(name, spec):
    parts = spec["pkg"].split("/") if spec["pkg"].startswith("@") else [spec["pkg"]]
    path = os.path.join(server_prefix(name), "node_modules", *parts, "package.json")
    if not os.path.isfile(path):
        return None
    try:
        with open(path, encoding="utf-8") as handle:
            return json.load(handle).get("version")
    except (OSError, ValueError):
        return None


def serena_bin(spec):
    return os.path.join(ROOT, ".pixi", "envs", spec["pixi_env"], "bin", "serena")


def codegraph_bin():
    return os.path.join(server_prefix("local-codegraph"), "node_modules", ".bin", "codegraph")


def run(command, cwd, timeout=None, env=None):
    return subprocess.run(
        command, cwd=cwd, stdin=subprocess.DEVNULL,
        capture_output=True, text=True, timeout=timeout, check=False, env=env)


def cmd_status() -> int:
    print(f"{PREFIX} servers declared in {REGISTRY}")
    for name, spec in specs().items():
        if spec.get("kind") == "npm":
            installed = npm_version(name, spec) or "missing"
            print(f"  {name:16s} npm  pin={spec['version']:10s} installed={installed}")
        elif spec.get("kind") == "pixi":
            binary = serena_bin(spec)
            version = "missing"
            if os.path.isfile(binary):
                try:
                    proc = run([binary, "--version"], ROOT, timeout=30)
                    version = (proc.stdout or proc.stderr).strip().splitlines()[0] if (proc.stdout or proc.stderr).strip() else "unknown"
                except (OSError, subprocess.TimeoutExpired):
                    version = "error"
            cache = os.path.join(PROJECT, ".serena", "cache")
            langs = ",".join(sorted(os.listdir(cache))) if os.path.isdir(cache) else "empty"
            print(f"  {name:16s} pixi pin={spec['version']:10s} reported={version:10s} env={spec['pixi_env']} cache=[{langs}]")
    binary = codegraph_bin()
    if os.path.isfile(binary):
        try:
            proc = run([binary, "status", PROJECT], PROJECT, timeout=60)
            text = (proc.stdout or proc.stderr).strip()
            if text:
                print(f"{PREFIX} codegraph status:")
                print(text)
        except (OSError, subprocess.TimeoutExpired):
            print(f"{PREFIX} codegraph status: error")
    else:
        print(f"{PREFIX} codegraph binary missing -- run: ./msrtc.sh mcp setup")
    print(f"{PREFIX} status is read-only")
    return 0


def install_npm(name, spec) -> int:
    prefix = server_prefix(name)
    os.makedirs(prefix, exist_ok=True)
    os.makedirs(NPM_CACHE, exist_ok=True)
    current = npm_version(name, spec)
    if current == spec["version"]:
        print(f"{PREFIX} {name:16s} npm {spec['version']} already installed")
        return 0
    print(f"{PREFIX} {name:16s} installing {spec['pkg']}@{spec['version']} ...")
    proc = subprocess.run(
        [npm_bin(), "install", "--no-save", "--no-package-lock",
         "--prefix", prefix, f"{spec['pkg']}@{spec['version']}"],
        cwd=HERE, env={**os.environ, "npm_config_cache": NPM_CACHE},
        stdin=subprocess.DEVNULL, check=False)
    return proc.returncode


def install_pixi(spec) -> int:
    print(f"{PREFIX} installing pixi env '{spec['pixi_env']}' ...")
    proc = subprocess.run(
        [pixi_bin(), "install", "--manifest-path", os.path.join(ROOT, "pixi.toml"),
         "-e", spec["pixi_env"]],
        cwd=ROOT, stdin=subprocess.DEVNULL, check=False)
    return proc.returncode


def cmd_setup() -> int:
    failures = 0
    for name, spec in specs().items():
        if spec.get("kind") == "npm":
            failures += 1 if install_npm(name, spec) else 0
        elif spec.get("kind") == "pixi":
            failures += 1 if install_pixi(spec) else 0
    if failures:
        print(f"{PREFIX} ✗ {failures} setup failure(s)")
        return 1
    print(f"{PREFIX} ✓ setup done")
    return 0


def serena_index(spec) -> int:
    binary = serena_bin(spec)
    if not os.path.isfile(binary):
        print(f"{PREFIX} {spec['pixi_env']} serena missing -- run: ./msrtc.sh mcp setup")
        return 1
    print(f"{PREFIX} rebuilding serena cache ({spec['pixi_env']}) ...")
    env_bin = os.path.join(ROOT, ".pixi", "envs", spec["pixi_env"], "bin")
    env = {**os.environ, "PATH": env_bin + os.pathsep + os.environ.get("PATH", "")}
    command = [binary, "project", "index", PROJECT,
               "--name", "mediaservo", "--ls", "rust", "--log-level", "INFO"]
    try:
        proc = run(command, PROJECT, timeout=900, env=env)
    except subprocess.TimeoutExpired:
        print(f"{PREFIX} serena index timeout after 900s")
        return 1
    if proc.returncode != 0:
        tail = (proc.stderr or proc.stdout).strip().splitlines()
        print(f"{PREFIX} serena index failed: {tail[-1] if tail else 'unknown'}")
        return 1
    return 0


def cmd_index() -> int:
    spec = specs()
    binary = codegraph_bin()
    if not os.path.isfile(binary):
        print(f"{PREFIX} codegraph missing -- run: ./msrtc.sh mcp setup")
        return 1
    if os.path.isdir(os.path.join(PROJECT, ".codegraph")):
        code = run([binary, "index", PROJECT], PROJECT).returncode
    else:
        code = run([binary, "init", "-y", PROJECT], PROJECT).returncode
    if code != 0:
        print(f"{PREFIX} codegraph index failed (exit {code})")
        return 1
    failures = 0
    for name, spec in spec.items():
        if spec.get("kind") == "pixi":
            failures += 1 if serena_index(spec) else 0
    if failures:
        print(f"{PREFIX} ✗ {failures} index failure(s)")
        return 1
    print(f"{PREFIX} ✓ index done")
    return 0


def cmd_sync() -> int:
    if not os.path.isdir(os.path.join(PROJECT, ".codegraph")):
        print(f"{PREFIX} no .codegraph yet -- run: ./msrtc.sh mcp index")
        return 1
    binary = codegraph_bin()
    if not os.path.isfile(binary):
        print(f"{PREFIX} codegraph missing -- run: ./msrtc.sh mcp setup")
        return 1
    code = run([binary, "sync", PROJECT], PROJECT).returncode
    if code != 0:
        print(f"{PREFIX} codegraph sync failed (exit {code})")
        return 1
    failures = 0
    for name, spec in specs().items():
        if spec.get("kind") == "pixi":
            failures += 1 if serena_index(spec) else 0
    if failures:
        print(f"{PREFIX} ✗ {failures} sync failure(s)")
        return 1
    print(f"{PREFIX} ✓ synced")
    return 0


def main() -> int:
    command = sys.argv[1] if len(sys.argv) > 1 else "status"
    if command == "status":
        return cmd_status()
    if command == "setup":
        return cmd_setup()
    if command == "sync":
        return cmd_sync()
    if command == "index":
        return cmd_index()
    print(f"{PREFIX} usage: status|setup|sync|index", file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main())
