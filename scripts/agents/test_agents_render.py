"""Tests for the agent role renderer (agent-subagent-layer B2, plan 3.4).

Pure parse/render over temporary fixtures -- no real .agents/agents needed.
"""
import os
import tempfile
import unittest

import cli

CANON = """---
name: reviewer
description: Read-only code review. Use after writing code.
mode: subagent
read_only: true
tools: [read, grep, glob]
skills: [browser-testing]
model: inherit
---
You are a reviewer.
"""

WRITABLE = """---
name: rust-core
description: Rust implementation.
read_only: false
tools: [read, grep, glob, shell, write, edit]
---
You implement Rust.
"""


def parse(text):
    with tempfile.TemporaryDirectory() as root:
        path = os.path.join(root, "role.md")
        with open(path, "w", encoding="utf-8") as fh:
            fh.write(text)
        return cli.parse_agent(path)


class ParseTests(unittest.TestCase):
    def test_parse_fields(self):
        agent = parse(CANON)
        self.assertEqual(agent["name"], "reviewer")
        self.assertEqual(agent["mode"], "subagent")
        self.assertTrue(agent["read_only"])
        self.assertEqual(agent["tools"], ["read", "grep", "glob"])
        self.assertEqual(agent["skills"], ["browser-testing"])
        self.assertEqual(agent["model"], "inherit")
        self.assertIn("You are a reviewer.", agent["body"])

    def test_parse_defaults(self):
        agent = parse("---\nname: x\ndescription: d\n---\nbody\n")
        self.assertEqual(agent["mode"], "subagent")
        self.assertFalse(agent["read_only"])
        self.assertEqual(agent["tools"], [])


class RenderTests(unittest.TestCase):
    def test_codebuddy_readonly_strips_write(self):
        agent = parse(CANON)
        out = cli.render_agent("codebuddy", agent)
        self.assertIn("name: reviewer", out)
        self.assertIn("tools: Read, Grep, Glob", out)
        self.assertNotIn("Write", out)
        self.assertIn("skills: browser-testing", out)
        self.assertIn("model: inherit", out)

    def test_codebuddy_writable_keeps_write(self):
        agent = parse(WRITABLE)
        out = cli.render_agent("codebuddy", agent)
        self.assertIn("Write", out)
        self.assertIn("Edit", out)
        self.assertIn("Bash", out)

    def test_qoder_readonly_emits_tools(self):
        agent = parse(CANON)
        out = cli.render_agent("qoder", agent)
        self.assertIn("tools: Read, Grep, Glob", out)
        self.assertNotIn("SearchReplace", out)

    def test_qoder_writable_omits_tools(self):
        # O-32 two-state: a tools field would force read-only, so writable
        # Qoder roles must NOT carry one.
        agent = parse(WRITABLE)
        out = cli.render_agent("qoder", agent)
        self.assertNotIn("tools:", out)
        self.assertIn("name: rust-core", out)

    def test_opencode_v1_readonly_boolean_deny(self):
        agent = parse(CANON)
        out = cli.render_agent("opencode", agent, dialect="v1")
        self.assertIn("write: false", out)
        self.assertIn("edit: false", out)
        self.assertIn("bash: false", out)
        self.assertIn("mode: subagent", out)
        self.assertNotIn("name:", out)  # id = file name

    def test_opencode_writable_enables_tools(self):
        agent = parse(WRITABLE)
        out = cli.render_agent("opencode", agent, dialect="v1")
        self.assertIn("write: true", out)
        self.assertIn("bash: true", out)

    def test_render_keeps_body(self):
        agent = parse(CANON)
        for kind in ("codebuddy", "qoder", "opencode"):
            self.assertIn("You are a reviewer.", cli.render_agent(kind, agent))


if __name__ == "__main__":
    unittest.main()