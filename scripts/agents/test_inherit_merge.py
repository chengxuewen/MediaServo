"""Integration tests for the agents surface + inheritance (B2, plan 3.14).

All fixtures are temporary -- no real .agents/agents is required (B3 lands the
actual roles). Covers: S-mode render idempotence, P-mode symlink distribution,
name collision, write-through / orphan-link / submodule-absent negatives,
relative links, and expected_hosts.
"""
import contextlib
import io
import json
import os
import shutil
import tempfile
import unittest

import cli
import common

REVIEWER = ("---\nname: reviewer\ndescription: Read-only review.\n"
            "read_only: true\ntools: [read, grep, glob]\n---\nReviewer body.\n")
SECOND = ("---\nname: second\ndescription: Second role.\n"
          "read_only: true\ntools: [read]\n---\nSecond body.\n")
CAPTAIN = ("---\nname: release-captain\ndescription: Release owner.\n"
           "tools: [read, grep, glob, shell, write]\n---\nCaptain body.\n")


def write(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as fh:
        fh.write(text)


def cb_host():
    return {"label": "codebuddy", "kind": "codebuddy", "surfaces": ["agents"],
            "agents_source": ".agents/agents", "agents_target": ".codebuddy/agents"}


def put_targets(root, cfg):
    path = os.path.join(root, ".agents", "targets.json")
    write(path, json.dumps(cfg))
    return path


def capture(*args):
    buf = io.StringIO()
    with contextlib.redirect_stdout(buf):
        rc = cli.run(*args)
    return rc, buf.getvalue()


class InheritMergeTests(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.base = self._tmp.name
        self.child = os.path.join(self.base, "child")
        self.parent = os.path.join(self.base, "parent")
        self.child_source = os.path.join(self.child, ".agents")
        # child: one canonical role + a codebuddy agents host
        write(os.path.join(self.child_source, "agents", "reviewer.md"), REVIEWER)
        self.child_targets = put_targets(self.child, {
            "source": ".agents", "repo_artifacts": False, "hosts": [cb_host()]})
        # parent: one self-owned role
        write(os.path.join(self.parent, ".agents", "agents", "release-captain.md"), CAPTAIN)

    def tearDown(self):
        self._tmp.cleanup()

    def parent_targets(self, **extra):
        cfg = {"source": self.child_source, "repo_artifacts": False,
               "hosts": [cb_host()]}
        cfg.update(extra)
        return put_targets(self.parent, cfg)

    def sync_child(self):
        return capture("sync", self.child_source, self.child, self.child_targets)

    def sync_parent(self, targets):
        return capture("sync", self.child_source, self.parent, targets)

    def test_s_mode_render_idempotent(self):
        rc, out = self.sync_child()
        self.assertEqual(rc, 0)
        rendered = os.path.join(self.child, ".codebuddy", "agents", "reviewer.md")
        self.assertTrue(os.path.isfile(rendered))
        self.assertFalse(os.path.islink(rendered))
        rc, out = self.sync_child()
        self.assertEqual(rc, 0)
        self.assertIn("already in sync (0 writes)", out)
        rc, _ = capture("check", self.child_source, self.child, self.child_targets)
        self.assertEqual(rc, 0)

    def test_inherit_creates_symlinks(self):
        self.sync_child()
        targets = self.parent_targets(inherit=self.child_source)
        rc, out = self.sync_parent(targets)
        self.assertEqual(rc, 0)
        host_dir = os.path.join(self.parent, ".codebuddy", "agents")
        entries = sorted(os.listdir(host_dir))
        self.assertEqual(entries, ["release-captain.md", "reviewer.md"])
        link = os.path.join(host_dir, "reviewer.md")
        self.assertTrue(os.path.islink(link))
        self.assertFalse(os.path.islink(os.path.join(host_dir, "release-captain.md")))
        # inherited content is zero-copy: the link resolves to the child render
        with open(link, encoding="utf-8") as fh:
            self.assertIn("Reviewer body.", fh.read())

    def test_relative_link(self):
        self.sync_child()
        targets = self.parent_targets(inherit=self.child_source)
        self.sync_parent(targets)
        link = os.path.join(self.parent, ".codebuddy", "agents", "reviewer.md")
        self.assertFalse(os.readlink(link).startswith("/"))

    def test_name_collision_guard(self):
        # parent owns a role with the same name as an inherited one -> exit 2
        write(os.path.join(self.parent, ".agents", "agents", "reviewer.md"), REVIEWER)
        self.sync_child()
        targets = self.parent_targets(inherit=self.child_source)
        rc, out = self.sync_parent(targets)
        self.assertEqual(rc, cli.common.EXIT_GUARD)
        self.assertIn("collision", out)

    def test_no_inherit_no_links(self):
        self.sync_child()
        targets = self.parent_targets()  # no inherit key
        rc, _ = self.sync_parent(targets)
        self.assertEqual(rc, 0)
        host_dir = os.path.join(self.parent, ".codebuddy", "agents")
        self.assertEqual(os.listdir(host_dir), ["release-captain.md"])

    def test_inherited_not_written_into_agents(self):
        self.sync_child()
        targets = self.parent_targets(inherit=self.child_source)
        self.sync_parent(targets)
        own = os.listdir(os.path.join(self.parent, ".agents", "agents"))
        self.assertEqual(own, ["release-captain.md"])

    def test_write_through_negative(self):
        self.sync_child()
        targets = self.parent_targets(inherit=self.child_source)
        self.sync_parent(targets)
        child_render = os.path.join(self.child, ".codebuddy", "agents", "reviewer.md")
        with open(child_render, encoding="utf-8") as fh:
            before = fh.read()
        # re-running the parent must never write through the link into the child
        self.sync_parent(targets)
        with open(child_render, encoding="utf-8") as fh:
            self.assertEqual(fh.read(), before)

    def test_orphan_link_pruned(self):
        write(os.path.join(self.child_source, "agents", "second.md"), SECOND)
        self.sync_child()
        targets = self.parent_targets(inherit=self.child_source)
        self.sync_parent(targets)
        host_dir = os.path.join(self.parent, ".codebuddy", "agents")
        self.assertEqual(len(os.listdir(host_dir)), 3)
        # child removes the role and re-syncs; the parent link must be unlinked
        os.remove(os.path.join(self.child_source, "agents", "second.md"))
        self.sync_child()
        rc, out = self.sync_parent(targets)
        self.assertEqual(rc, 0)
        self.assertNotIn("second.md", os.listdir(host_dir))
        self.assertIn("PRUNE", out)

    def test_submodule_absent_warns_not_blocks(self):
        # child has NOT rendered agents -> parent warns but still succeeds
        targets = self.parent_targets(inherit=self.child_source)
        rc, out = self.sync_parent(targets)
        self.assertEqual(rc, 0)
        self.assertIn("WARN", out)
        self.assertTrue(os.path.isfile(
            os.path.join(self.parent, ".codebuddy", "agents", "release-captain.md")))

    def test_expected_hosts_missing_red(self):
        self.sync_child()
        targets = self.parent_targets(inherit=self.child_source,
                                      expected_hosts=["codebuddy", "qoder"])
        self.sync_parent(targets)  # materialize the agents first
        rc, out = capture("check", self.child_source, self.parent, targets)
        self.assertEqual(rc, 1)
        self.assertIn("qoder", out)
        rc, out = capture("status", self.child_source, self.parent, targets)
        self.assertIn("missing: qoder", out)
        # removing the promise turns it green again (files already synced)
        targets = self.parent_targets(inherit=self.child_source)
        rc, _ = capture("check", self.child_source, self.parent, targets)
        self.assertEqual(rc, 0)

    def test_child_host_dir_absent_preserves_links(self):
        # M-6: child not rendered -> inherited links preserved, check degrades
        self.sync_child()
        targets = self.parent_targets(inherit=self.child_source)
        self.sync_parent(targets)
        host_dir = os.path.join(self.parent, ".codebuddy", "agents")
        self.assertEqual(len(os.listdir(host_dir)), 2)
        shutil.rmtree(os.path.join(self.child, ".codebuddy", "agents"))
        rc, out = capture("check", self.child_source, self.parent, targets)
        self.assertEqual(rc, 0)
        self.assertIn("WARN", out)
        self.assertEqual(len(os.listdir(host_dir)), 2)  # links not pruned

    def test_source_absent_p_mode_degrades(self):
        # M-6: uninitialized submodule (source missing) never blocks the parent
        absent = os.path.join(self.base, "no-such-source")
        rc, out = capture("check", absent, self.parent,
                          os.path.join(self.parent, "targets.json"))
        self.assertEqual(rc, 0)
        self.assertIn("degraded", out)

    def test_source_absent_s_mode_still_guard(self):
        # S mode (target == this repo) keeps the hard guard on a missing source
        rc, _ = capture("check", os.path.join(self.base, "nope"),
                        common.ROOT, os.path.join(self.base, "targets.json"))
        self.assertEqual(rc, common.EXIT_GUARD)


if __name__ == "__main__":
    unittest.main()