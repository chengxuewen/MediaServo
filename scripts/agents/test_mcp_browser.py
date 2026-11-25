#!/usr/bin/env python3
"""Unit tests for the MCP browser resolution helpers (no network, no browser).

Covers the record contract (atomic write, shape, garbage tolerance) and the
pinned-version reader. The end-to-end acceptance lives in mcp_selftest.py.
"""

import importlib.util
import os
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))

spec = importlib.util.spec_from_file_location("mcp_cli", os.path.join(HERE, "mcp_cli.py"))
mcp_cli = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mcp_cli)


class RecordTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp()
        self._out = mcp_cli.OUT_DIR
        self._rec = mcp_cli.RECORD
        mcp_cli.OUT_DIR = self.tmp
        mcp_cli.RECORD = os.path.join(self.tmp, "mcp-browser.json")

    def tearDown(self):
        mcp_cli.OUT_DIR = self._out
        mcp_cli.RECORD = self._rec

    def test_roundtrip_shape(self):
        mcp_cli.record_resolution("srv", {"mode": "playwright", "browser": "chromium",
                                          "version": "1.2.3", "executable": None})
        data = mcp_cli.load_record()
        self.assertEqual(data["record_version"], 1)
        self.assertEqual(data["resolutions"]["srv"]["mode"], "playwright")
        self.assertEqual(data["resolutions"]["srv"]["version"], "1.2.3")

    def test_atomic_tmp_not_left_behind(self):
        mcp_cli.record_resolution("srv", {"mode": "playwright", "browser": "chromium",
                                          "version": "1", "executable": None})
        self.assertFalse(os.path.exists(mcp_cli.RECORD + ".tmp"))

    def test_second_write_merges(self):
        mcp_cli.record_resolution("a", {"mode": "playwright", "browser": "chromium",
                                        "version": "1", "executable": None})
        mcp_cli.record_resolution("b", {"mode": "system", "browser": "chromium",
                                        "version": "2", "executable": "/x"})
        data = mcp_cli.load_record()
        self.assertEqual(sorted(data["resolutions"]), ["a", "b"])

    def test_garbage_record_reads_none(self):
        with open(mcp_cli.RECORD, "w", encoding="utf-8") as handle:
            handle.write("{not json")
        self.assertIsNone(mcp_cli.load_record())

    def test_missing_record_reads_none(self):
        self.assertIsNone(mcp_cli.load_record())


class PinnedVersionTests(unittest.TestCase):
    def test_reads_installed_pin(self):
        prefix = mcp_cli.server_prefix("local-playwright")
        browsers = os.path.join(prefix, "node_modules", "playwright-core", "browsers.json")
        if not os.path.isfile(browsers):
            self.skipTest("local-playwright pin not installed")
        version, revision = mcp_cli.pinned_browser_version(prefix, "chromium")
        self.assertRegex(str(revision), r"^\d+$")
        self.assertRegex(str(version), r"^\d+\.\d+\.\d+")

    def test_unknown_browser_returns_none(self):
        prefix = mcp_cli.server_prefix("local-playwright")
        browsers = os.path.join(prefix, "node_modules", "playwright-core", "browsers.json")
        if not os.path.isfile(browsers):
            self.skipTest("local-playwright pin not installed")
        version, revision = mcp_cli.pinned_browser_version(prefix, "no-such-browser")
        self.assertIsNone(version)
        self.assertIsNone(revision)


class RegistryTests(unittest.TestCase):
    def test_playwright_pin_shape(self):
        specs = mcp_cli.specs()
        pw = specs["local-playwright"]
        # Flags outrank envs: browser/headless must NOT be baked into args.
        self.assertNotIn("--browser", pw["args"])
        self.assertNotIn("--headless", pw["args"])
        self.assertEqual(pw["browsers"], ["chromium"])
        self.assertRegex(pw["version"], r"^\d+\.\d+\.\d+$")  # pin must stay exact


if __name__ == "__main__":
    unittest.main()