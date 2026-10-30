#\!/usr/bin/env python3
"""Run the agent guard family: numbering, skills, rule refs, parent literals."""
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
CHECKS = [
    ("numbering", [sys.executable, os.path.join(HERE, "check_numbering.py")]),
    ("skill-lint", [sys.executable, os.path.join(HERE, "skill_lint.py")]),
    ("rule-ref", [sys.executable, os.path.join(HERE, "rule_ref_lint.py")]),
    ("no-parent", [sys.executable, os.path.join(HERE, "check_no_parent.py")]),
]


def main():
    worst = 0
    for label, command in CHECKS:
        print("== %s ==" % label)
        rc = subprocess.run(command, check=False).returncode
        worst = max(worst, rc)
    print("GATE-GUARDS %s" % ("✓" if worst == 0 else "✗"))
    return 1 if worst else 0


if __name__ == "__main__":
    sys.exit(main())
