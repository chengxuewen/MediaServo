#!/usr/bin/env python3
"""End-to-end MCP probe: drive the real launcher through a JSON-RPC handshake.

initialize -> notifications/initialized -> tools/call browser_navigate(about:blank),
hold stdin open, then assert the response carries a page snapshot. This is the
only check that proves a browser WORKS (a binary answering --version may still
fail to launch).

Usage: python3 scripts/agents/mcp_probe.py [server]   (default: local-playwright)
Exit: 0 on a captured snapshot; 1 otherwise.
"""

import json
import os
import select
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))


def send(proc, payload):
    line = json.dumps(payload) + "\n"
    proc.stdin.write(line.encode())
    proc.stdin.flush()


def read_response(proc, want_id, deadline):
    """Read JSON-RPC lines until the response with want_id arrives (or deadline)."""
    buf = b""
    while time.time() < deadline:
        ready, _, _ = select.select([proc.stdout], [], [], 0.5)
        if not ready:
            continue
        chunk = proc.stdout.readline()
        if not chunk:
            return None
        buf += chunk
        text = buf.decode(errors="replace").strip()
        try:
            msg = json.loads(text)
        except ValueError:
            buf = b""
            continue
        buf = b""
        if msg.get("id") == want_id:
            return msg
    return None


def main():
    server = sys.argv[1] if len(sys.argv) > 1 else "local-playwright"
    proc = subprocess.Popen(
        ["bash", os.path.join("scripts", "agents", "mcp-launch.sh"), server],
        cwd=ROOT, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    try:
        send(proc, {"jsonrpc": "2.0", "id": 1, "method": "initialize",
                    "params": {"protocolVersion": "2025-06-18", "capabilities": {},
                               "clientInfo": {"name": "mcp-probe", "version": "1"}}})
        init = read_response(proc, 1, time.time() + 60)
        if not init:
            print("PROBE FAIL: no initialize response")
            print((proc.stderr.read(2000) or b"").decode(errors="replace"))
            return 1
        send(proc, {"jsonrpc": "2.0", "method": "notifications/initialized"})
        send(proc, {"jsonrpc": "2.0", "id": 2, "method": "tools/call",
                    "params": {"name": "browser_navigate",
                               "arguments": {"url": "about:blank"}}})
        # Browser launch can be slow on a cold cache; the real action may take a while.
        resp = read_response(proc, 2, time.time() + 120)
        if not resp:
            print("PROBE FAIL: no browser_navigate response")
            print((proc.stderr.read(2000) or b"").decode(errors="replace"))
            return 1
        result = json.dumps(resp.get("result", {}))
        if resp.get("error"):
            print("PROBE FAIL: " + json.dumps(resp["error"])[:400])
            return 1
        # Hold stdin open for a moment so the server can flush trailing traffic.
        time.sleep(1)
        snapshot = "snapshot" in result.lower() or "page" in result.lower()
        if snapshot:
            print("PROBE OK: captured snapshot (" + str(len(result)) + " bytes)")
            print(result[:300])
            return 0
        print("PROBE FAIL: response has no page snapshot: " + result[:300])
        return 1
    finally:
        try:
            proc.stdin.close()
        except Exception:
            pass
        try:
            proc.terminate()
            proc.wait(timeout=5)
        except Exception:
            proc.kill()


if __name__ == "__main__":
    sys.exit(main())