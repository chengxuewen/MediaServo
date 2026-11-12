"""Tests for agent_lint (agent-subagent-layer B1): good green, bad red."""
import contextlib
import io
import os
import tempfile
import unittest

import agent_lint

GOOD = """---
name: reviewer
description: Read-only code review. Use after writing code.
mode: subagent
read_only: true
tools: [read, grep, glob]
---
You are a reviewer. Report findings with file:line evidence.
"""


def write_role(dirpath, name, text):
    with open(os.path.join(dirpath, name + ".md"), "w", encoding="utf-8") as fh:
        fh.write(text)


def run(agents_dir, skills_dir):
    buf = io.StringIO()
    with contextlib.redirect_stdout(buf):
        fails, count = agent_lint.lint_agents(agents_dir, skills_dir)
    return fails, count, buf.getvalue()


class AgentLintTests(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.root = self._tmp.name
        self.agents = os.path.join(self.root, "agents")
        self.skills = os.path.join(self.root, "skills")
        os.makedirs(self.agents)
        os.makedirs(os.path.join(self.skills, "browser-testing"))
        with open(os.path.join(self.skills, "browser-testing", "SKILL.md"),
                  "w", encoding="utf-8") as fh:
            fh.write("---\nname: browser-testing\ndescription: x\n---\n")

    def tearDown(self):
        self._tmp.cleanup()

    def test_empty_dir_green(self):
        fails, count, _ = run(self.agents, self.skills)
        self.assertEqual(fails, 0)
        self.assertEqual(count, 0)

    def test_missing_dir_green(self):
        fails, count, _ = run(os.path.join(self.root, "nope"), self.skills)
        self.assertEqual(fails, 0)
        self.assertEqual(count, 0)

    def test_good_role_green(self):
        write_role(self.agents, "reviewer", GOOD)
        fails, count, out = run(self.agents, self.skills)
        self.assertEqual(fails, 0)
        self.assertEqual(count, 1)
        self.assertIn("✓", out)

    def test_good_role_with_skill_green(self):
        write_role(self.agents, "reviewer",
                   GOOD.replace("tools: [read, grep, glob]",
                                "tools: [read, grep, glob]\nskills: [browser-testing]"))
        fails, _, _ = run(self.agents, self.skills)
        self.assertEqual(fails, 0)

    def test_name_mismatch_red(self):
        write_role(self.agents, "reviewer", GOOD.replace("name: reviewer", "name: other"))
        fails, _, out = run(self.agents, self.skills)
        self.assertGreater(fails, 0)
        self.assertIn("name mismatch", out)

    def test_description_missing_red(self):
        write_role(self.agents, "reviewer", GOOD.replace(
            "description: Read-only code review. Use after writing code.", "description:"))
        fails, _, out = run(self.agents, self.skills)
        self.assertGreater(fails, 0)
        self.assertIn("description missing", out)

    def test_description_too_long_red(self):
        write_role(self.agents, "reviewer", GOOD.replace(
            "description: Read-only code review. Use after writing code.",
            "description: " + "x" * 1025))
        fails, _, out = run(self.agents, self.skills)
        self.assertGreater(fails, 0)
        self.assertIn("chars >", out)

    def test_body_too_long_red(self):
        write_role(self.agents, "reviewer", GOOD + "\n".join("line %d" % i for i in range(301)))
        fails, _, out = run(self.agents, self.skills)
        self.assertGreater(fails, 0)
        self.assertIn("body", out)

    def test_invalid_tool_red(self):
        write_role(self.agents, "reviewer", GOOD.replace("tools: [read, grep, glob]",
                                                         "tools: [read, hack]"))
        fails, _, out = run(self.agents, self.skills)
        self.assertGreater(fails, 0)
        self.assertIn("not a semantic verb", out)

    def test_read_only_with_write_red(self):
        write_role(self.agents, "reviewer", GOOD.replace("tools: [read, grep, glob]",
                                                         "tools: [read, write]"))
        fails, _, out = run(self.agents, self.skills)
        self.assertGreater(fails, 0)
        self.assertIn("read_only but carries", out)

    def test_missing_skill_red(self):
        write_role(self.agents, "reviewer", GOOD.replace("tools: [read, grep, glob]",
                                                         "tools: [read]\nskills: [nope]"))
        fails, _, out = run(self.agents, self.skills)
        self.assertGreater(fails, 0)
        self.assertIn("not found", out)

    def test_invalid_mode_red(self):
        write_role(self.agents, "reviewer", GOOD.replace("mode: subagent", "mode: wizard"))
        fails, _, out = run(self.agents, self.skills)
        self.assertGreater(fails, 0)
        self.assertIn("mode", out)

    def test_duplicate_name_red(self):
        write_role(self.agents, "reviewer", GOOD)
        write_role(self.agents, "copy", GOOD)  # same declared name, different file
        fails, _, out = run(self.agents, self.skills)
        self.assertGreater(fails, 0)
        self.assertIn("duplicate role name", out)

    def test_frontmatter_missing_red(self):
        write_role(self.agents, "reviewer", "no frontmatter here\n")
        fails, _, out = run(self.agents, self.skills)
        self.assertGreater(fails, 0)
        self.assertIn("frontmatter missing", out)


if __name__ == "__main__":
    unittest.main()