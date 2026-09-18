#!/usr/bin/env python3
"""C43③: bindings 手抄三处 == mediaservo-field 交付域版本（与 ci.yml 同步形）。"""
import json, subprocess, sys
out = subprocess.run(["cargo","metadata","--no-deps","--format-version","1"],
                     capture_output=True, text=True).stdout
v = next(p["version"] for p in json.loads(out)["packages"] if p["name"]=="mediaservo-field")
checks = [("bindings/python/mediaservo/pyproject.toml", f'version = "{v}"'),
          ("bindings/python/mediaservo/mediaservo/__init__.py", f'__version__ = "{v}"'),
          ("bindings/node/package.json", f'"version": "{v}"')]
bad = [f"{p} != {v}" for p, pat in checks if pat not in open(p, encoding="utf-8").read()]
sys.exit("\n".join(bad) or print(f"bindings parity ok ({v})"))
