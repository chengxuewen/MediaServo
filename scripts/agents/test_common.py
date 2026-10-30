import os
import tempfile
import unittest

import common


class RegionTests(unittest.TestCase):
    def test_replace_region(self):
        text = "a\n<!-- B -->\nold\n<!-- E -->\nb\n"
        out = common.replace_region(text, "<!-- B -->", "<!-- E -->", "new")
        self.assertIn("new", out)
        self.assertNotIn("old", out)

    def test_place_region_appends(self):
        out = common.place_region("head\n", "<!-- B -->", "<!-- E -->", "x")
        self.assertTrue(out.endswith("<!-- E -->\n"))

    def test_guard_rejects_inside_source(self):
        with tempfile.TemporaryDirectory() as base:
            source = os.path.join(base, "src")
            os.makedirs(source)
            self.assertIsNotNone(
                common.guard_target(os.path.join(source, "x"), os.path.realpath(source)))
            self.assertIsNone(
                common.guard_target(os.path.join(base, "x"), os.path.realpath(source)))

    def test_json_text_has_trailing_newline(self):
        self.assertTrue(common.json_text({"a": 1}).endswith("\n"))


if __name__ == "__main__":
    unittest.main()
