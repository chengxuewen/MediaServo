import contextlib
import io
import json
import os
import tempfile
import unittest

import cli

HERE = os.path.dirname(os.path.abspath(__file__))
SOURCE = os.path.abspath(os.path.join(HERE, "..", "..", ".agents"))


class DualContextTests(unittest.TestCase):
    def test_mcp_layer_is_path_independent(self):
        cfg = {
            "source": ".agents",
            "repo_artifacts": False,
            "hosts": [
                {
                    "label": "mcp",
                    "kind": "mcp",
                    "file": ".mcp.json",
                    "surfaces": ["mcp"],
                    "mcp_launcher": "scripts/agents/mcp-launch.sh",
                }
            ],
        }
        with tempfile.TemporaryDirectory() as first, tempfile.TemporaryDirectory() as second:
            for root in (first, second):
                os.makedirs(os.path.join(root, ".agents"), exist_ok=True)
                targets = os.path.join(root, ".agents", "targets.json")
                with open(targets, "w", encoding="utf-8") as handle:
                    json.dump(cfg, handle)
                with contextlib.redirect_stdout(io.StringIO()):
                    self.assertEqual(cli.run("sync", SOURCE, root, targets), 0)

            with open(os.path.join(first, ".mcp.json"), encoding="utf-8") as handle:
                left = handle.read()
            with open(os.path.join(second, ".mcp.json"), encoding="utf-8") as handle:
                right = handle.read()

            self.assertEqual(left, right)
            self.assertIn("scripts/agents/mcp-launch.sh", left)
            self.assertNotIn(first, left)
            self.assertNotIn(second, left)


if __name__ == "__main__":
    unittest.main()
