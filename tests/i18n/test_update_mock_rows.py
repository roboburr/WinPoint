"""Windows Update's i18n lists (GW3, notes/claude/windows-update-gnome.md §8 and §10; notes/claude/wu/gw3.md).

    python3 -B -m unittest tests/i18n/test_update_mock_rows.py -v        (from the project root; a few seconds, no browser)

* The host rows that vendor/ui/js/mock/update.js renders its error and warning messages from (the block between
  ``/* host-rows:begin */`` and ``/* host-rows:end */``, ``'key': ['nl', 'en'],``) equal host/i18n/update.tsv, the
  catalogue the Rust service renders them from (GW2). While update.tsv has no ``update.*`` rows yet (GW2 not merged),
  they are checked against the design's §3.7 table instead, the source both are written from.
* tests/i18n/BASELINE-ADDITIONS.ui lists exactly the new Dutch rows of the dictionary snapshot
  (tools/i18n/check_ui.py, the rows the dict diff against tests/i18n/dict-nl-3b898e6.tsv reports as added) in the
  Windows Update files: apps/settings/pages/update*.js and the registry's P(update-*) pages; each line's text is the
  snapshot's text (needs node; skipped without it).
* The mock's update.status scenarios: every settings-update.<id> surface of tests/i18n/surfaces.json names a scenario
  of mock/update.js and every scenario has a surface.
"""

from __future__ import annotations

import json
import re
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
HERE = ROOT / "tests" / "i18n"
TOOLS = ROOT / "tools" / "i18n"
MOCK = ROOT / "vendor" / "ui" / "js" / "mock" / "update.js"
TSV = ROOT / "host" / "i18n" / "update.tsv"
DESIGN = ROOT / "notes" / "claude" / "windows-update-gnome.md"
ADDITIONS = HERE / "BASELINE-ADDITIONS.ui"
sys.path.insert(0, str(TOOLS))

import check_ui  # noqa: E402
import corpus  # noqa: E402
import paths  # noqa: E402

NODE = corpus.find_node()
STEP = re.compile(r"^(GW[1-9][a-z]?|GX[1-9]|GE1a|R3-UI|WU-FIX)$")   # GE1a: update.js exportName (a moved literal)
# the vendor/ui/js files whose new keys BASELINE-ADDITIONS.ui lists (Windows Update), and the registry pages
WU_FILES = re.compile(r"^apps/settings/pages/update(-[a-z]+)?\.js$")
WU_PAGES = re.compile(r"^P\(update-[a-z]+\)\.")


def js_string(s: str, i: int) -> tuple[str, int]:
    """The single-quoted JS string literal starting at s[i] == "'" → (value, index after it)."""
    assert s[i] == "'"
    out, i = [], i + 1
    while s[i] != "'":
        if s[i] == "\\":
            nxt = s[i + 1]
            out.append({"n": "\n", "t": "\t", "\\": "\\", "'": "'"}.get(nxt, nxt))
            i += 2
            continue
        out.append(s[i])
        i += 1
    return "".join(out), i + 1


def mock_rows() -> dict[str, tuple[str, str]]:
    src = MOCK.read_text(encoding="utf-8")
    m = re.search(r"/\* host-rows:begin \*/(.*?)/\* host-rows:end \*/", src, re.S)
    if not m:
        raise AssertionError("mock/update.js has no host-rows block")
    rows: dict[str, tuple[str, str]] = {}
    for ln in m.group(1).split("\n"):
        ln = ln.strip()
        if not ln.startswith("'"):
            continue
        key, i = js_string(ln, 0)
        rest = ln[i:].lstrip()
        assert rest.startswith(": ["), ln
        j = ln.index("[", i) + 1
        nl, j = js_string(ln, ln.index("'", j))
        en, j = js_string(ln, ln.index("'", j))
        if key in rows:
            raise AssertionError(f"mock/update.js: {key} twice")
        rows[key] = (nl, en)
    return rows


def tsv_unescape(s: str) -> str:
    return re.sub(r"\\(.)", lambda m: {"n": "\n", "t": "\t", "\\": "\\"}.get(m.group(1), m.group(0)), s)


def tsv_rows() -> dict[str, tuple[str, str]]:
    rows: dict[str, tuple[str, str]] = {}
    for ln in TSV.read_text(encoding="utf-8").split("\n"):
        if not ln or ln.startswith("#"):
            continue
        parts = ln.split("\t")
        if len(parts) == 3:
            rows[parts[0]] = (tsv_unescape(parts[1]), tsv_unescape(parts[2]))
    return rows


def design_rows() -> dict[str, tuple[str, str]]:
    """§3.7's table rows with one key: | `key` | when | nl | en |"""
    text = DESIGN.read_text(encoding="utf-8")
    sec = text[text.index("### 3.7 Host texts"):text.index("### 3.8 ")]
    rows: dict[str, tuple[str, str]] = {}
    for ln in sec.split("\n"):
        m = re.match(r"^\| `([a-zA-Z0-9_.]+)` \| [^|]* \| ([^|]+) \| ([^|]+) \|$", ln)
        if m:
            rows[m.group(1)] = (m.group(2).strip(), m.group(3).strip())
    return rows


class MockHostRows(unittest.TestCase):
    def setUp(self):
        self.mock = mock_rows()

    def test_block_is_well_formed(self):
        self.assertIn("update.product", self.mock)
        for k, (nl, en) in self.mock.items():
            self.assertRegex(k, r"^update\.[a-zA-Z0-9_.]+$")
            self.assertTrue(nl and en, k)
            self.assertEqual(set(re.findall(r"\{(\w+)\}", nl)), set(re.findall(r"\{(\w+)\}", en)), k)
            self.assertNotIn("'", en, f"{k}: en uses a straight apostrophe (D-G23: ’)")

    def test_rows_equal_the_catalogue(self):
        tsv = {k: v for k, v in tsv_rows().items() if k.startswith("update.")}
        if not tsv:
            self.skipTest("host/i18n/update.tsv has no update.* rows yet (GW2); see test_rows_equal_the_design")
        for k, v in self.mock.items():
            self.assertIn(k, tsv, f"{k}: not in host/i18n/update.tsv")
            self.assertEqual(v, tsv[k], f"{k}: mock/update.js and host/i18n/update.tsv differ")

    def test_rows_equal_the_design(self):
        if any(k.startswith("update.") for k in tsv_rows()):
            self.skipTest("host/i18n/update.tsv has rows: test_rows_equal_the_catalogue decides")
        design = design_rows()
        self.assertGreater(len(design), 40)
        for k, v in self.mock.items():
            self.assertIn(k, design, f"{k}: not a one-key row of the design's §3.7 table")
            self.assertEqual(v, design[k], f"{k}: mock/update.js and the design's §3.7 differ")


class Scenarios(unittest.TestCase):
    def test_every_scenario_has_a_surface(self):
        src = MOCK.read_text(encoding="utf-8")
        body = src[src.index("const SCENARIOS = {"):src.index("\n};", src.index("const SCENARIOS = {"))]
        scen = set(re.findall(r"^  ([HR]\d\d[a-z]?):", body, re.M))
        surf = json.loads((HERE / "surfaces.json").read_text(encoding="utf-8"))["surfaces"]
        got = {s["id"].split(".", 1)[1] for s in surf if s["id"].startswith("settings-update.")}
        self.assertEqual(sorted(got), sorted(scen))
        for s in surf:
            if s["id"].startswith("settings-update."):
                self.assertIn(f"updateScenario={s['id'].split('.', 1)[1]}", s["q"], s["id"])


@unittest.skipUnless(NODE, "node not found")
class BaselineAdditions(unittest.TestCase):
    def test_lists_exactly_the_new_rows(self):
        old = check_ui.read_snapshot(HERE / "dict-nl-3b898e6.tsv")
        new = check_ui.snapshot_rows(check_ui.snapshot_data(NODE, paths.ui_js(ROOT)))
        added = {f"{r[0]}:{r[2]}": r[4] for r in check_ui.dict_diff(old, new)["added"]
                 if r[1] != "pair" and (WU_FILES.match(r[0]) or (r[0] == "apps/settings/registry.js" and WU_PAGES.match(r[2])))}
        listed: dict[str, str] = {}
        for no, ln in enumerate(ADDITIONS.read_text(encoding="utf-8").split("\n"), 1):
            if not ln or ln.startswith("#"):
                continue
            parts = ln.split("\t")
            self.assertEqual(len(parts), 3, f"line {no}: STEP<TAB>key:<file>:<key><TAB><nl>")
            self.assertRegex(parts[0], STEP, f"line {no}")
            self.assertTrue(parts[1].startswith("key:"), f"line {no}")
            k = parts[1][4:]
            self.assertNotIn(k, listed, f"line {no}: {k} twice")
            listed[k] = check_ui.unesc(parts[2])
        self.assertEqual(sorted(set(added) - set(listed)), [], "new keys missing from BASELINE-ADDITIONS.ui")
        self.assertEqual(sorted(set(listed) - set(added)), [], "BASELINE-ADDITIONS.ui lines that are no new key")
        for k, v in listed.items():
            self.assertEqual(v, added[k], f"{k}: BASELINE-ADDITIONS.ui has another text")


if __name__ == "__main__":
    unittest.main()
