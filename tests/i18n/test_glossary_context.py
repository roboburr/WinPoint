"""The glossary lock-in (R3-UI, notes/claude/round3/r3-ui.md): the glossary rule of tools/i18n/check.py is an error,
and the context rows of tools/i18n/glossary-context.tsv work as documented there.

    python3 -B -m unittest tests/i18n/test_glossary_context.py -v        (from the project root; node for the last test)

The rule tests build a Glossary from small files in a temporary directory (deleted when the test ends).
"""

from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
sys.path.insert(0, str(ROOT / "tools" / "i18n"))

import check  # noqa: E402
import check_ui  # noqa: E402  (merges its DEFERRED into check.DEFERRED when the ui layer loads)
import corpus  # noqa: E402

UI = "vendor/ui/js/"
GLOSSARY = "Prullenbak\tRecycle Bin\nTerugzetten\tRestore\nReageert niet\tNot responding\nSchijfbeheer\tDisk Management\tname\n"


def build(context: str, glossary: str = GLOSSARY) -> check.Glossary:
    with tempfile.TemporaryDirectory() as d:
        g, c = Path(d) / "glossary.tsv", Path(d) / "glossary-context.tsv"
        g.write_text(glossary, encoding="utf-8")
        c.write_text(context, encoding="utf-8")
        return check.Glossary(g)


def rules(rep: check.Report) -> list[tuple[str, str, int, str]]:
    return [(i.level, i.rule, i.line, i.msg) for i in rep.issues]


class LockIn(unittest.TestCase):
    def test_the_glossary_rule_is_an_error(self):
        self.assertNotIn("glossary", check.DEFERRED)
        self.assertNotIn("glossary", check_ui.DEFERRED)
        rep = check.Report()
        rep.add("glossary", "x.js", 1, "nl “Prullenbak” → en should contain “Recycle Bin”")
        self.assertEqual(rep.issues[0].level, "error")
        src = (ROOT / "tools" / "i18n" / "check_pairs.py").read_text(encoding="utf-8")
        self.assertIn('report.error(where, f"glossary:', src)

    def test_the_real_context_file_loads_cleanly(self):
        g = check.Glossary(ROOT / "tools" / "i18n" / "glossary.tsv")
        self.assertEqual(g.context_problems, [])
        self.assertGreaterEqual(len(g.context), 10)
        retry = [c for c in g.context if c.nl == "Opnieuw proberen"]
        self.assertEqual(len(retry), 1, "D-G41: one Retry row for the Update page")
        self.assertTrue(retry[0].applies(UI + "apps/settings/pages/update.js", "retry"))
        self.assertFalse(retry[0].applies(UI + "apps/settings/common.js", "retry"))
        self.assertIn(("Opnieuw proberen", "Try again"), [(r[0], r[1]) for r in g.rows])


class ContextRows(unittest.TestCase):
    def test_a_context_row_replaces_the_rendering_at_its_place_only(self):
        g = build(f"{UI}a.js:revert,{UI}a.js:keep*\tTerugzetten\tRevert\t-\twhy\n")
        self.assertEqual(g.check("Terugzetten", "Revert", set(), where=(UI + "a.js", "revert")), [])
        self.assertEqual(g.check("Terugzetten over 3 s", "Reverting in 3 s", set(), where=(UI + "a.js", "keepBody")), [])
        # the general rendering is no longer enough there
        self.assertEqual(g.check("Terugzetten", "Restore", set(), where=(UI + "a.js", "revert")), [("Terugzetten", "Revert")])
        # elsewhere the glossary.tsv rendering holds
        self.assertEqual(g.check("Terugzetten", "Revert", set(), where=(UI + "b.js", "revert")), [("Terugzetten", "Restore")])
        self.assertEqual(g.check("Terugzetten", "Revert", set()), [("Terugzetten", "Restore")])
        self.assertEqual(g.context[0].hits, 3)

    def test_a_longer_context_phrase_masks_the_shorter_glossary_phrase(self):
        g = build("host/src/x.json:timeout\treageert niet op tijd\tanswer in time\t-\twhy\n")
        w = ("host/src/x.json", "timeout")
        self.assertEqual(g.check("UDisks reageert niet op tijd.", "UDisks didn't answer in time.", set(), where=w), [])
        self.assertEqual(g.check("UDisks reageert niet.", "UDisks didn't answer.", set(), where=w),
                         [("Reageert niet", "Not responding")])

    def test_name_rows_are_case_sensitive(self):
        g = build("host/src/x.json:unavailable\tSchijfbeheer\tDisk management\tname,known\twhy\n")
        w = ("host/src/x.json", "unavailable")
        self.assertEqual(g.check("Schijfbeheer werkt niet", "Disk management doesn't work", set(), where=w), [])
        self.assertEqual(g.check("Schijfbeheer werkt niet", "Disk Management doesn’t work", set(), where=w),
                         [("Schijfbeheer", "Disk management")])
        self.assertTrue(g.context[0].known)

    def test_stale_rows_places_outside_and_bad_rows_are_errors(self):
        g = build(f"{UI}a.js:k\tPrullenbak\tTrash\t-\tused\n"
                  f"{UI}a.js:gone\tPrullenbak\tBin\t-\tstale\n"
                  f"{UI}nofile.js:k\tPrullenbak\tBin\t-\tno such file\n"
                  "extension/x.js:k\tPrullenbak\tBin\t-\toutside\n"
                  f"{UI}a.js:k\tPrullenbak\n"
                  f"{UI}a.js:k\tPrullenbak\tBin\tweird\twhy\n")
        g.check("Prullenbak", "Trash", set(), where=(UI + "a.js", "k"))
        rep = check.Report()
        g.check_context(rep, {UI: True, "host/src/": True})
        got = rules(rep)
        self.assertTrue(all(lvl == "error" and rule == "glossary-context" for lvl, rule, _l, _m in got), got)
        lines = sorted(line for _lvl, _r, line, _m in got)
        # 2 stale, 3 no checked file + stale, 4 outside, 5 malformed, 6 unknown flag + stale
        self.assertEqual(lines, [2, 3, 3, 4, 5, 6, 6], got)

    def test_a_part_of_the_tree_that_was_not_checked_is_not_judged(self):
        g = build(f"{UI}a.js:k\tPrullenbak\tTrash\t-\twhy\n")
        rep = check.Report()
        g.check_context(rep, {UI: False, "host/src/": True})      # e.g. --no-js: the UI was not read
        self.assertEqual(rep.issues, [])


@unittest.skipUnless(corpus.find_node(), "node not found")
class Tree(unittest.TestCase):
    def test_no_glossary_findings_and_no_stale_context_row(self):
        rep = check.run(ROOT)
        bad = [i.text() for i in rep.issues if i.rule in ("glossary", "glossary-context")]
        self.assertEqual(bad, [])
        self.assertGreaterEqual(rep.stats.get("glossary_context_rows", 0), 10)


if __name__ == "__main__":
    unittest.main()
