import contextlib
import io
import json
import os
import tempfile
import unittest

import cli

HERE = os.path.dirname(os.path.abspath(__file__))
SOURCE = os.path.abspath(os.path.join(HERE, "..", "..", ".agents"))
CFG = {
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


def quiet(*args):
    with contextlib.redirect_stdout(io.StringIO()):
        return cli.run(*args)


class CliBehaviorTests(unittest.TestCase):
    def test_check_sync_drift_cycle(self):
        with tempfile.TemporaryDirectory() as root:
            targets = os.path.join(root, ".agents", "targets.json")
            os.makedirs(os.path.dirname(targets))
            with open(targets, "w", encoding="utf-8") as handle:
                json.dump(CFG, handle)

            self.assertEqual(quiet("check", SOURCE, root, targets), 1)  # missing
            self.assertEqual(quiet("sync", SOURCE, root, targets), 0)
            self.assertEqual(quiet("check", SOURCE, root, targets), 0)

            with open(os.path.join(root, ".mcp.json"), "a", encoding="utf-8") as handle:
                handle.write("x")
            self.assertEqual(quiet("check", SOURCE, root, targets), 1)  # drift
            self.assertEqual(quiet("sync", SOURCE, root, targets), 0)   # repaired
            self.assertEqual(quiet("check", SOURCE, root, targets), 0)


if __name__ == "__main__":
    unittest.main()
