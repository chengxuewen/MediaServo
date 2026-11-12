"""Tests for the skills surface (agent-subagent-layer B7).

Only CodeBuddy needs projection (symlinks to true-source skill dirs); Qoder /
opencode read .agents/skills natively (O-31 / N34). All fixtures temporary.
"""
import contextlib
import io
import json
import os
import shutil
import tempfile
import unittest

import cli


def write(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as fh:
        fh.write(text)


def capture(*args):
    buf = io.StringIO()
    with contextlib.redirect_stdout(buf):
        rc = cli.run(*args)
    return rc, buf.getvalue()


def cb_skills_host():
    return {"label": "codebuddy", "kind": "codebuddy", "surfaces": ["skills"],
            "skills_source": ".agents/skills", "skills_target": ".codebuddy/skills"}


class SkillsSurfaceTests(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.base = self._tmp.name
        self.child = os.path.join(self.base, "child")
        self.parent = os.path.join(self.base, "parent")
        self.child_source = os.path.join(self.child, ".agents")
        # two skills + a non-skill file (must be excluded)
        write(os.path.join(self.child_source, "skills", "alpha", "SKILL.md"),
              "---\nname: alpha\ndescription: a\n---\nalpha body\n")
        write(os.path.join(self.child_source, "skills", "beta", "SKILL.md"),
              "---\nname: beta\ndescription: b\n---\nbeta body\n")
        write(os.path.join(self.child_source, "skills", "TRIGGERS.md"), "registry\n")
        self.child_targets = os.path.join(self.child_source, "targets.json")
        write(self.child_targets, json.dumps({
            "source": ".agents", "repo_artifacts": False,
            "hosts": [cb_skills_host()]}))

    def tearDown(self):
        self._tmp.cleanup()

    def parent_targets(self):
        path = os.path.join(self.parent, ".agents", "targets.json")
        write(path, json.dumps({
            "source": self.child_source, "repo_artifacts": False,
            "hosts": [cb_skills_host()]}))
        return path

    def test_s_mode_projects_symlinks(self):
        rc, _ = capture("sync", self.child_source, self.child, self.child_targets)
        self.assertEqual(rc, 0)
        proj = os.path.join(self.child, ".codebuddy", "skills")
        self.assertEqual(sorted(os.listdir(proj)), ["alpha", "beta"])  # no TRIGGERS
        self.assertTrue(os.path.islink(os.path.join(proj, "alpha")))
        # resolves to the true-source SKILL.md
        with open(os.path.join(proj, "alpha", "SKILL.md"), encoding="utf-8") as fh:
            self.assertIn("alpha body", fh.read())

    def test_s_mode_idempotent(self):
        capture("sync", self.child_source, self.child, self.child_targets)
        rc, out = capture("sync", self.child_source, self.child, self.child_targets)
        self.assertEqual(rc, 0)
        self.assertIn("already in sync (0 writes)", out)
        rc, _ = capture("check", self.child_source, self.child, self.child_targets)
        self.assertEqual(rc, 0)

    def test_p_mode_inherits_symlinks(self):
        targets = self.parent_targets()
        rc, _ = capture("sync", self.child_source, self.parent, targets)
        self.assertEqual(rc, 0)
        proj = os.path.join(self.parent, ".codebuddy", "skills")
        self.assertEqual(sorted(os.listdir(proj)), ["alpha", "beta"])
        link = os.path.join(proj, "alpha")
        self.assertTrue(os.path.islink(link))
        self.assertFalse(os.readlink(link).startswith("/"))  # relative
        with open(os.path.join(link, "SKILL.md"), encoding="utf-8") as fh:
            self.assertIn("alpha body", fh.read())

    def test_equality_and_prune_stale(self):
        targets = self.parent_targets()
        capture("sync", self.child_source, self.parent, targets)
        proj = os.path.join(self.parent, ".codebuddy", "skills")
        # remove a source skill -> the projection must prune the stale link
        shutil.rmtree(os.path.join(self.child_source, "skills", "beta"))
        rc, out = capture("check", self.child_source, self.parent, targets)
        self.assertEqual(rc, 1)
        self.assertIn("stale skill link", out)
        rc, _ = capture("sync", self.child_source, self.parent, targets)
        self.assertEqual(rc, 0)
        self.assertEqual(os.listdir(proj), ["alpha"])

    def test_write_through_negative(self):
        targets = self.parent_targets()
        capture("sync", self.child_source, self.parent, targets)
        src = os.path.join(self.child_source, "skills", "alpha", "SKILL.md")
        with open(src, encoding="utf-8") as fh:
            before = fh.read()
        capture("sync", self.child_source, self.parent, targets)
        with open(src, encoding="utf-8") as fh:
            self.assertEqual(fh.read(), before)  # parent never writes the source

    def test_submodule_absent_degrades(self):
        targets = self.parent_targets()
        capture("sync", self.child_source, self.parent, targets)
        proj = os.path.join(self.parent, ".codebuddy", "skills")
        # child skills dir disappears -> links preserved, check degrades
        shutil.rmtree(os.path.join(self.child_source, "skills"))
        rc, out = capture("check", self.child_source, self.parent, targets)
        self.assertEqual(rc, 0)
        self.assertEqual(sorted(os.listdir(proj)), ["alpha", "beta"])


if __name__ == "__main__":
    unittest.main()