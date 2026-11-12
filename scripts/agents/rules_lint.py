#!/usr/bin/env python3
"""Rules/annex lint (B10, plan 3.17 + r19 M-5).

Checks (true source only -- no generated layer needed):
  - the always-on core annex in rules/common/constraints.md parses and has
    1..5 rows with sequential indices (O-29 cap; CB best practice 3-5)
  - gate parity: GATE_RE must count every `gate` site in all-gates.sh,
    INCLUDING else-branch gates (the B10-A regression guard)
  - the layer-2 enumeration has no domain-prefix folder collisions and
    covers constraints.md itself (self-coverage)
  - the layer-2 resident description budget holds (deterministic char proxy,
    r19 audit: no invented metrics)
"""
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
sys.path.insert(0, HERE)

import cli  # noqa: E402  (GATE_RE / _parse_core_constraints / _rule_description)

STATE = {"fail": 0}


def fail(msg):
    print("RULES-LINT ✗ %s" % msg)
    STATE["fail"] = 1


def main():
    source = os.path.join(ROOT, ".agents")
    rules_root = os.path.join(source, "rules")

    # 1. Annex: parseable, 1..5 rows, sequential indices.
    core = cli._parse_core_constraints(source)
    if not core:
        fail("always-on core annex missing or unparseable in "
             "rules/common/constraints.md")
    elif len(core) > 5:
        fail("always-on core has %d rows (cap 5)" % len(core))
    indices = [r["index"] for r in core]
    if core and indices != list(range(1, len(core) + 1)):
        fail("annex indices not sequential: %s" % indices)

    # 2. Gate parity (B10-A regression guard): GATE_RE must see every gate
    #    site, including else-branch gates; SKIP lines are not gates.
    gates_path = os.path.join(ROOT, "scripts", "all-gates.sh")
    with open(gates_path, encoding="utf-8") as fh:
        gates_text = fh.read()
    counted = cli.GATE_RE.findall(gates_text)
    raw_sites = len(re.findall(r'(?m)^[ \t]*(?:else[ \t]+)?gate "', gates_text))
    if len(counted) != raw_sites:
        fail("gate parity broken: GATE_RE counted %d, all-gates.sh has %d sites"
             % (len(counted), raw_sites))

    # 3. Layer-2 enumeration: unique domain-prefixed folders; constraints.md
    #    itself must be covered (self-coverage).
    folders = {}
    total_desc = 0
    for dirpath, _dirnames, filenames in os.walk(rules_root):
        domain = os.path.basename(dirpath)
        for fn in sorted(filenames):
            if not fn.endswith(".md") or fn == "README.md":
                continue
            stem = fn[:-3]
            folder = "%s-%s" % (domain, stem)
            if folder in folders:
                fail("layer-2 folder collision: %s" % folder)
            folders[folder] = os.path.join(dirpath, fn)
            with open(os.path.join(dirpath, fn), encoding="utf-8") as fh:
                total_desc += len(cli._rule_description(fh.read(), stem))
    if "common-constraints" not in folders:
        fail("layer-2 does not cover constraints.md itself (self-coverage)")
    if not core and not STATE["fail"]:
        pass

    # 4. Resident description budget (deterministic char proxy; r19 audit).
    BUDGET = 12000
    if total_desc > BUDGET:
        fail("layer-2 description budget exceeded: %d > %d chars"
             % (total_desc, BUDGET))

    if STATE["fail"]:
        sys.exit(1)
    print("RULES-LINT ✓ annex %d/5 rows, gates %d sites, layer-2 %d folders, "
          "descriptions %d/%d chars" % (len(core), len(counted), len(folders),
                                        total_desc, BUDGET))
    sys.exit(0)


if __name__ == "__main__":
    main()