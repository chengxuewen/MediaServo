#!/usr/bin/env python3
"""End-to-end acceptance for the local-playwright browser ladder.

Runs the nine acceptance items from the task in one command, including the
negative proofs -- with backups and restore, and WITHOUT destructive cache
moves (the "no managed build" case is simulated by pointing HOME at an empty
temp dir, which is exactly what the gate reads).

Usage: python3 scripts/agents/mcp_selftest.py
Exit: 0 when every item passes; 1 otherwise (per-item PASS/FAIL table printed).
"""

import json
import os
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
RECORD = os.path.join(ROOT, "out", "mcp-browser.json")
LAUNCH = os.path.join("scripts", "agents", "mcp-launch.sh")
GATE = os.path.join("scripts", "agents", "gate_mcp.py")
CLI = os.path.join("scripts", "agents", "mcp_cli.py")
PROBE = os.path.join("scripts", "agents", "mcp_probe.py")

results = []


def check(name, ok, detail=""):
    results.append((name, ok, detail))
    print(("PASS  " if ok else "FAIL  ") + name + (("  -- " + detail) if detail and not ok else ""))


def run(cmd, env=None, timeout=1800):
    e = dict(os.environ)
    e.update(env or {})
    try:
        proc = subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True,
                              env=e, timeout=timeout, check=False)
        return proc.returncode, (proc.stdout or "") + (proc.stderr or "")
    except subprocess.TimeoutExpired:
        return 124, "timeout"


def pinned_version():
    path = os.path.join(ROOT, "scripts", "agents", "node_modules", ".mcp",
                        "local-playwright", "node_modules", "playwright-core", "browsers.json")
    try:
        with open(path, encoding="utf-8") as handle:
            for entry in json.load(handle).get("browsers", []):
                if entry.get("name") == "chromium":
                    return entry.get("browserVersion")
    except (OSError, ValueError):
        pass
    return None


def main():
    print("== local-playwright acceptance ==")

    # 1. setup exits 0
    rc, out = run([sys.executable, CLI, "setup"])
    check("1 setup rc=0", rc == 0, "rc=%d" % rc)

    # 2. record shape: mode + exact pinned version
    try:
        with open(RECORD, encoding="utf-8") as handle:
            data = json.load(handle)
        entry = (data.get("resolutions") or {}).get("local-playwright") or {}
        want = pinned_version()
        check("2 record mode/playwright", entry.get("mode") == "playwright", str(entry))
        check("2 record version == pin", bool(want) and entry.get("version") == want,
              "record=%s pin=%s" % (entry.get("version"), want))
    except (OSError, ValueError) as err:
        check("2 record readable", False, str(err))

    # 3. status shows the resolution
    rc, out = run([sys.executable, CLI, "status"])
    check("3 status shows resolution", rc == 0 and "browser resolved: mode=playwright" in out)

    # 4. gate exits 0 with zero unresolved
    rc, out = run([sys.executable, GATE])
    check("4 gate rc=0 + unresolved 0", rc == 0 and "browser unresolved: 0" in out)

    # 5. the probe: a real browser_navigate snapshot
    print("   (probe launches a real browser; ~1-2 min)")
    rc, out = run([sys.executable, PROBE], timeout=300)
    check("5 probe snapshot", rc == 0 and "PROBE OK" in out and "snapshot" in out.lower())
    if rc == 0:
        print("   " + out.strip().splitlines()[-1][:160])

    # 6. hygiene: no .playwright-mcp at root, snapshots under out/<server>
    check("6 no .playwright-mcp at root", not os.path.exists(os.path.join(ROOT, ".playwright-mcp")))
    snap_dir = os.path.join(ROOT, "out", "local-playwright")
    snaps = [f for f in os.listdir(snap_dir)] if os.path.isdir(snap_dir) else []
    check("6 snapshots under out/", bool(snaps), "dir=%s" % snap_dir)

    # 7. strict mode stays green
    rc, out = run([sys.executable, CLI, "setup"], env={"MCP_BROWSER_STRICT": "1"})
    check("7 strict mode rc=0", rc == 0, "rc=%d" % rc)

    # 8. NP1: empty record, managed build present -> gate yellow w/ hint, launcher rc!=0
    backup = RECORD + ".selftest.bak"
    shutil.copyfile(RECORD, backup)
    try:
        with open(RECORD, "w", encoding="utf-8") as handle:
            json.dump({"record_version": 1, "resolutions": {}}, handle)
        rc, out = run([sys.executable, GATE])
        check("8n gate yellow + setup hint", "run: ./msrtc.sh mcp setup" in out)
        rc2, out2 = run(["bash", LAUNCH, "local-playwright"], timeout=30)
        check("8n launcher rc!=0 + hint", rc2 != 0 and "mcp setup" in out2, "rc=%d" % rc2)
    finally:
        shutil.move(backup, RECORD)

    # 9. NP2: empty record + no managed build (HOME override; nothing destroyed)
    empty_home = tempfile.mkdtemp()
    backup = RECORD + ".selftest.bak"
    shutil.copyfile(RECORD, backup)
    try:
        with open(RECORD, "w", encoding="utf-8") as handle:
            json.dump({"record_version": 1, "resolutions": {}}, handle)
        rc, out = run([sys.executable, GATE], env={"HOME": empty_home})
        check("9n unresolved counted", "browser unresolved: 1" in out)
        check("9n UNRESOLVED wording", "UNRESOLVED" in out and "managed build present" not in out)
    finally:
        shutil.move(backup, RECORD)
        shutil.rmtree(empty_home, ignore_errors=True)

    # restore check: everything green again
    rc, out = run([sys.executable, GATE])
    check("10 restored green", rc == 0 and "browser unresolved: 0" in out)

    failed = [r for r in results if not r[1]]
    print("\n== %d/%d passed ==" % (len(results) - len(failed), len(results)))
    return 0 if not failed else 1


if __name__ == "__main__":
    sys.exit(main())