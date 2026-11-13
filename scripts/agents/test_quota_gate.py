"""Quota gate fixture tests (B11): over-limit goes red; standalone does not
false-positive (plan acceptance for check_quotas)."""
import os
import tempfile
import unittest

import check_quotas

QUOTA_MD = """# rules

## Role Roster and Quotas (agent-subagent-layer B1b-1)

| Asset | Hard cap | Counting rule | Over-cap action |
|-------|----------|---------------|-----------------|
| role files (true source, both repos) | 11 | submodule 10 + parent 1; inherited visibility does not count | add one, remove one |
| skills | net +2 per quarter | true-source directories only; projection does not count | retire one first, or prove no existing skill covers it |
| MCP servers | 5 | pinned registry entries | disable one before adding |
| members per team | 4 | standing expert teams | answer "which new judgment path does this member bring?" first |

## Other section
"""

ROLE = """---
name: role-{n}
description: Fixture role {n}.
mode: subagent
read_only: true
tools: [read, grep, glob]
---
Fixture body.
"""

TEAM = """# Fixture team

## Members
- code-reviewer
- evidence-critic
- rust-core
"""


def make_fixture(root, role_count=10, team_size=3):
    os.makedirs(os.path.join(root, ".agents", "rules", "common"))
    os.makedirs(os.path.join(root, ".agents", "agents"))
    os.makedirs(os.path.join(root, ".agents", "teams"))
    with open(os.path.join(root, ".agents", "rules", "common", "agents.md"),
              "w", encoding="utf-8") as fh:
        fh.write(QUOTA_MD)
    for n in range(role_count):
        with open(os.path.join(root, ".agents", "agents", "role-%d.md" % n),
                  "w", encoding="utf-8") as fh:
            fh.write(ROLE.format(n=n))
    with open(os.path.join(root, ".agents", "mcp.json"), "w", encoding="utf-8") as fh:
        fh.write('{"mcpServers": {"a": {}, "b": {}, "c": {}, "d": {}}}')
    members = ["code-reviewer", "evidence-critic", "rust-core"][:team_size]
    members += ["role-%d" % i for i in range(team_size - 3)]
    body = "# Fixture team\n\n## Members\n" + "".join("- %s\n" % m for m in members)
    with open(os.path.join(root, ".agents", "teams", "interface.md"),
              "w", encoding="utf-8") as fh:
        fh.write(body)


class QuotaGateTests(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.root = self._tmp.name

    def tearDown(self):
        self._tmp.cleanup()

    def test_standalone_within_caps_green_with_note(self):
        make_fixture(self.root, role_count=10)
        fails, lines = check_quotas.run(self.root)
        self.assertEqual(fails, 0)
        self.assertIn("standalone", lines[-1])
        self.assertIn("roles 10/11", lines[-1])
        self.assertIn("report-only", lines[-1])

    def test_role_over_limit_red(self):
        make_fixture(self.root, role_count=13)
        fails, lines = check_quotas.run(self.root)
        self.assertEqual(fails, 1)
        self.assertIn("role files 13 > cap 11", lines[0])

    def test_team_over_limit_red(self):
        make_fixture(self.root, role_count=10, team_size=5)
        fails, lines = check_quotas.run(self.root)
        self.assertEqual(fails, 1)
        self.assertIn("members > cap 4", lines[0])

    def test_missing_quota_table_red(self):
        os.makedirs(os.path.join(self.root, ".agents", "agents"))
        fails, lines = check_quotas.run(self.root)
        self.assertEqual(fails, 1)
        self.assertIn("quota table missing", lines[0])


if __name__ == "__main__":
    unittest.main()