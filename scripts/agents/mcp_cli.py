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


def node_bin():
    candidate = os.path.join(ROOT, ".pixi", "envs", "default", "bin", "node")
    if os.path.isfile(candidate):
        return candidate
    return shutil.which("node") or "node"


OUT_DIR = os.path.join(ROOT, "out")
RECORD = os.path.join(OUT_DIR, "mcp-browser.json")


def load_record():
    try:
        with open(RECORD, encoding="utf-8") as handle:
            data = json.load(handle)
        return data if isinstance(data, dict) else None
    except (OSError, ValueError):
        return None


def record_resolution(name, entry):
    """Atomic write (tmp + os.replace): a torn record must never mislead the launcher."""
    os.makedirs(OUT_DIR, exist_ok=True)
    data = load_record() or {"record_version": 1, "resolutions": {}}
    data.setdefault("resolutions", {})[name] = entry
    tmp = RECORD + ".tmp"
    with open(tmp, "w", encoding="utf-8") as handle:
        json.dump(data, handle, indent=2)
        handle.write("\n")
    os.replace(tmp, RECORD)


def pinned_browser_version(prefix, browser):
    """Read the pin's expected browser version from its own browsers.json -- never guess."""
    path = os.path.join(prefix, "node_modules", "playwright-core", "browsers.json")
    try:
        with open(path, encoding="utf-8") as handle:
            for entry in json.load(handle).get("browsers", []):
                if entry.get("name") == browser:
                    return entry.get("browserVersion"), entry.get("revision")
    except (OSError, ValueError):
        pass
    return None, None


def browser_ladder(name, spec):
    """Three rungs: pinned installer (host override) -> live system browser -> WARN.

    Rung 1 uses the pin's own installer and CDN (version-exact, trusted artifact,
    bounded cache) with PLAYWRIGHT_HOST_PLATFORM_OVERRIDE so a dropped host still
    receives the build the pin wants. Rung 2 proves a system browser with
    --version and records its real path. Strict env turns rung 3 red.
    """
    prefix = server_prefix(name)
    browser = spec["browsers"][0]
    version, revision = pinned_browser_version(prefix, browser)

    cli = os.path.join(prefix, "node_modules", "playwright-core", "cli.js")
    if os.path.isfile(cli):
        env = node_env()
        if sys.platform.startswith("linux") and not env.get("PLAYWRIGHT_HOST_PLATFORM_OVERRIDE"):
            env["PLAYWRIGHT_HOST_PLATFORM_OVERRIDE"] = os.environ.get(
                "MCP_PLAYWRIGHT_HOST_OVERRIDE", "ubuntu22.04-x64")
        print(f"{PREFIX} {name:16s} rung 1: installer {browser} (pin build {revision or '?'}) ...")
        try:
            proc = subprocess.run([node_bin(), cli, "install", browser],
                                  cwd=HERE, env=env, stdin=subprocess.DEVNULL,
                                  check=False, timeout=1800)
            rc = proc.returncode
        except subprocess.TimeoutExpired:
            rc = 1
        if rc == 0:
            record_resolution(name, {"mode": "playwright", "browser": browser,
                                     "version": version or "unknown", "executable": None})
            print(f"{PREFIX} {name:16s} resolved: playwright {browser} {version or ''}")
            return 0
        print(f"{PREFIX} {name:16s} rung 1 refused (rc={rc})")

    for cand in ("google-chrome", "google-chrome-stable", "chromium", "chromium-browser", "chrome"):
        found = shutil.which(cand)
        if not found:
            continue
        real = os.path.realpath(found)
        try:
            proc = subprocess.run([real, "--version"], capture_output=True, text=True,
                                  check=False, timeout=30)
        except (OSError, subprocess.TimeoutExpired):
            continue
        if proc.returncode == 0 and proc.stdout.strip():
            ver = proc.stdout.strip().split()[-1]
            record_resolution(name, {"mode": "system", "browser": cand,
                                     "version": ver, "executable": real})
            print(f"{PREFIX} {name:16s} resolved: system {cand} {ver} "
                  f"(NOTE: system browsers auto-update; only playwright builds are pinned)")
            return 0

    print(f"{PREFIX} {name:16s} WARN: no usable browser (installer refused + no system browser)")
    if os.environ.get("MCP_BROWSER_STRICT"):
        print(f"{PREFIX} {name:16s} MCP_BROWSER_STRICT set -> failing")
        return 1
    return 0


def node_env():
    # npm and the installed npm bins (codegraph, playwright-mcp) use
    # `#!/usr/bin/env node` shebangs: resolving the npm path alone is not
    # enough — the pixi env's bin must be on PATH so `env node` resolves
    # without an activated shell (same policy as mcp-launch.sh).
    env_bin = os.path.join(ROOT, ".pixi", "envs", "default", "bin")
    if os.path.isdir(env_bin):
        return {**os.environ, "PATH": env_bin + os.pathsep + os.environ.get("PATH", "")}
    return dict(os.environ)


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
            proc = run([binary, "status", PROJECT], PROJECT, timeout=60, env=node_env())
            text = (proc.stdout or proc.stderr).strip()
            if text:
                print(f"{PREFIX} codegraph status:")
                print(text)
        except (OSError, subprocess.TimeoutExpired):
            print(f"{PREFIX} codegraph status: error")
    else:
        print(f"{PREFIX} codegraph binary missing -- run: ./msrtc.sh mcp setup")
    record = (load_record() or {}).get("resolutions") or {}
    for name, spec in specs().items():
        if spec.get("kind") == "npm" and spec.get("browsers"):
            entry = record.get(name) or {}
            if entry.get("mode"):
                print(f"{PREFIX} {name:16s} browser resolved: mode={entry['mode']} "
                      f"browser={entry.get('browser')} version={entry.get('version')}")
            else:
                print(f"{PREFIX} {name:16s} browser UNRESOLVED -- run: ./msrtc.sh mcp setup")
    print(f"{PREFIX} status is read-only")
    return 0


def cmd_latest() -> int:
    """Report-only: show newer upstream versions. Never installs, never touches pins."""
    for name, spec in specs().items():
        if spec.get("kind") != "npm":
            continue
        latest = "unknown"
        try:
            proc = subprocess.run([npm_bin(), "view", spec["pkg"], "version", "--json"],
                                  capture_output=True, text=True, check=False,
                                  env=node_env(), timeout=120)
            latest = proc.stdout.strip().strip('"') or "unknown"
        except (OSError, subprocess.TimeoutExpired):
            pass
        drift = "" if latest == spec["version"] else f"  (pin {spec['version']})"
        print(f"{PREFIX} {name:16s} latest={latest}{drift}")
    print(f"{PREFIX} report-only: pins are NOT changed")
    return 0


def install_npm(name, spec) -> int:
    prefix = server_prefix(name)
    os.makedirs(prefix, exist_ok=True)
    os.makedirs(NPM_CACHE, exist_ok=True)
    current = npm_version(name, spec)
    if current == spec["version"]:
        print(f"{PREFIX} {name:16s} npm {spec['version']} already installed")
        rc = 0
    else:
        print(f"{PREFIX} {name:16s} installing {spec['pkg']}@{spec['version']} ...")
        proc = subprocess.run(
            [npm_bin(), "install", "--no-save", "--no-package-lock",
             "--prefix", prefix, f"{spec['pkg']}@{spec['version']}"],
            cwd=HERE, env={**node_env(), "npm_config_cache": NPM_CACHE},
            stdin=subprocess.DEVNULL, check=False)
        rc = proc.returncode
    if rc:
        return rc
    # Pinned browsers (registry `browsers` list, e.g. playwright's chromium):
    # `playwright install` is idempotent and fetches the build matching the
    # pinned package -- replacing the old hardcoded --executable-path, which
    # broke on cache clears and new machines. No --with-deps: OS-level apt
    # deps are outside the gate (ubuntu20.04 is unsupported; this box already
    # has a system chromium's shared libs the bundled one can run against).
    if spec.get("browsers"):
        return browser_ladder(name, spec)
    return 0


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
    print(f"{PREFIX} rebuilding codegraph index ...")
    if os.path.isdir(os.path.join(PROJECT, ".codegraph")):
        code = run([binary, "index", PROJECT], PROJECT, env=node_env()).returncode
    else:
        code = run([binary, "init", "-y", PROJECT], PROJECT, env=node_env()).returncode
        if code != 0 and os.path.isdir(os.path.join(PROJECT, ".codegraph")):
            # init can exit non-zero leaving a truncated index (interrupted
            # auto-index); the db exists, so recover with one full index pass.
            print(f"{PREFIX} codegraph init exited {code} -- recovering with a full index")
            code = run([binary, "index", PROJECT], PROJECT, env=node_env()).returncode
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
    print(f"{PREFIX} syncing codegraph index ...")
    code = run([binary, "sync", PROJECT], PROJECT, env=node_env()).returncode
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
    if command == "latest":
        return cmd_latest()
    print(f"{PREFIX} usage: status|setup|sync|index|latest", file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main())
