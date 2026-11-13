"""gate_mcp fixture tests (B12): user-level absent -> visible SKIP (exit 0);
same-name overlap -> red; floating pin -> red; quota over -> red."""
import json
import os
import tempfile
import unittest

import gate_mcp

PIN = {"figma": {"kind": "npm", "pkg": "figma-developer-mcp",
                 "version": "0.13.2", "bin": "figma-developer-mcp", "args": ["--stdio"]}}


def make_fixture(root, extra_project=None, pins=None):
    os.makedirs(os.path.join(root, "scripts", "agents"))
    os.makedirs(os.path.join(root, ".agents", "rules", "common"))
    pins = pins if pins is not None else dict(PIN)
    with open(os.path.join(root, "scripts", "agents", "mcp-servers.json"),
              "w", encoding="utf-8") as fh:
        json.dump(pins, fh)
    servers = {
        "local-codegraph": {"type": "stdio", "command": "bash",
                            "args": ["scripts/agents/mcp-launch.sh", "local-codegraph"]},
        "figma": {"type": "stdio", "command": "bash",
                  "args": ["scripts/agents/mcp-launch.sh", "figma"]},
    }
    servers.update(extra_project or {})
    with open(os.path.join(root, ".agents", "mcp.json"), "w", encoding="utf-8") as fh:
        json.dump({"mcpServers": servers}, fh)
    with open(os.path.join(root, ".agents", "rules", "common", "agents.md"),
              "w", encoding="utf-8") as fh:
        fh.write("| MCP servers | 5 | pinned registry entries | disable one |\n")


class GateMcpTests(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.root = self._tmp.name
        self._old_env = os.environ.pop("CHECK_MCP_USER_FILE", None)

    def tearDown(self):
        if self._old_env is not None:
            os.environ["CHECK_MCP_USER_FILE"] = self._old_env
        else:
            os.environ.pop("CHECK_MCP_USER_FILE", None)
        self._tmp.cleanup()

    def test_user_absent_visible_skip(self):
        make_fixture(self.root)
        # deterministic absence (the real ~/.codebuddy/mcp.json may exist empty)
        os.environ["CHECK_MCP_USER_FILE"] = os.path.join(self._tmp.name,
                                                         "no-such-user.json")
        fails, lines = gate_mcp.run(self.root)
        self.assertEqual(fails, 0)
        self.assertIn("SKIP", lines[-1])
        self.assertIn("user-level file absent", lines[-1])

    def test_same_name_overlap_red(self):
        make_fixture(self.root)
        user_file = os.path.join(self._tmp.name, "user-mcp.json")
        with open(user_file, "w", encoding="utf-8") as fh:
            json.dump({"mcpServers": {"figma": {"command": "other"}}}, fh)
        os.environ["CHECK_MCP_USER_FILE"] = user_file
        fails, lines = gate_mcp.run(self.root)
        self.assertEqual(fails, 1)
        self.assertIn("same-name", lines[0])
        self.assertIn("figma", lines[0])

    def test_floating_pin_red(self):
        make_fixture(self.root, pins={"figma": {"kind": "npm",
                                                "pkg": "figma-developer-mcp"}})
        fails, lines = gate_mcp.run(self.root)
        self.assertEqual(fails, 1)
        self.assertIn("floating", lines[0])

    def test_no_count_cap_reported(self):
        # B12 ruling: MCP has no count cap -- many entries stay green, count
        # is reported in the summary.
        make_fixture(self.root, extra_project={
            "x1": {"type": "remote", "url": "https://x1"},
            "x2": {"type": "remote", "url": "https://x2"},
            "x3": {"type": "remote", "url": "https://x3"},
            "x4": {"type": "remote", "url": "https://x4"},
        })
        fails, lines = gate_mcp.run(self.root)
        self.assertEqual(fails, 0)
        self.assertIn("project 6 (no count cap)", lines[-1])


if __name__ == "__main__":
    unittest.main()