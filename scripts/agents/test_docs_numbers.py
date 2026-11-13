"""Numbering-gate fixture tests (r43): contiguity holes red unless VOID,
ghost cites red, year-like PIT tokens exempt (ruling 2-C)."""
import os
import tempfile
import unittest

import check_docs_numbers


class DocsNumbersTests(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.root = self._tmp.name
        os.makedirs(os.path.join(self.root, ".agents", "memorys"))
        os.makedirs(os.path.join(self.root, "docs"))
        with open(os.path.join(self.root, ".agents", "memorys", "pitfalls.md"),
                  "w", encoding="utf-8") as fh:
            fh.write("## PIT-1: x\n")

    def tearDown(self):
        self._tmp.cleanup()

    def _decisions(self, text):
        with open(os.path.join(self.root, ".agents", "memorys", "decisions.md"),
                  "w", encoding="utf-8") as fh:
            fh.write(text)

    def _corpus(self, text):
        with open(os.path.join(self.root, "docs", "a.md"), "w",
                  encoding="utf-8") as fh:
            fh.write(text)

    def test_contiguous_and_resolved_cite_green(self):
        self._decisions("## D196: a\n## D197: b\n")
        self._corpus("see D197 for the follow-up\n")
        fails, lines = check_docs_numbers.run(self.root)
        self.assertEqual(fails, 0)

    def test_contiguity_hole_red_until_void(self):
        self._decisions("## D196: a\n## D198: b\n")  # D197 missing, not VOID
        fails, lines = check_docs_numbers.run(self.root)
        self.assertEqual(fails, 1)
        self.assertIn("ghost number D197", lines[0])
        # registering the void (placeholder in the registry) turns it green
        self._decisions("## D196: a\n## D197: （占位）void\n## D198: b\n")
        fails, _ = check_docs_numbers.run(self.root)
        self.assertEqual(fails, 0)

    def test_ghost_cite_red(self):
        self._decisions("## D196: a\n")
        self._corpus("deploy does not build (D299)\n")
        fails, lines = check_docs_numbers.run(self.root)
        self.assertEqual(fails, 1)
        self.assertIn("ghost cite D299", lines[0])

    def test_archived_range_cite_exempt(self):
        self._decisions("## D196: a\n")
        self._corpus("historical: D45 was the first deploy fix\n")
        fails, lines = check_docs_numbers.run(self.root)
        self.assertEqual(fails, 0)

    def test_year_like_pit_token_exempt(self):
        self._corpus("the PIT-2026-10-09 pin: first tick emits zero\n")
        fails, lines = check_docs_numbers.run(self.root)
        self.assertEqual(fails, 0)


if __name__ == "__main__":
    unittest.main()