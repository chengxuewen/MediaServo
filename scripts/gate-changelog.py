#!/usr/bin/env python3
"""F11-X: CHANGELOG Unreleased entries must carry an allowed-vocabulary [scope] tag (synced with ci.yml)."""
import re, sys
ALLOWED = {"host", "server", "sdk-client", "sdk-field", "protocol", "deploy"}
text = open("CHANGELOG.md", encoding="utf-8").read()
m = re.search(r"## Unreleased\n(.*?)(?=\n## |\Z)", text, re.S)
bad = []
if m:
    for n, line in enumerate(m.group(1).splitlines(), 1):
        if line.startswith("- "):
            head = line[: line.find(": ") if ": " in line else 80]
            tags = re.findall(r"\[([a-z0-9-]+)\]", head)
            if not tags: bad.append(f"L{n}: missing [scope]")
            bad += [f"L{n}: [{t}] outside allowed vocabulary" for t in tags if t not in ALLOWED]
sys.exit("\n".join(bad) or print("changelog markers ok"))
