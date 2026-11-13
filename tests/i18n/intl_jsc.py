#!/usr/bin/env python3
"""Engine check: format.js and the i18n.js additions in JavaScriptCore, the engine the UI runs in (WebKitGTK).

    TZ=Europe/Amsterdam python3 -B tests/i18n/intl_jsc.py [--json]

Loads vendor/ui/js/core/i18n.js and vendor/ui/js/core/format.js (their import/export lines stripped) into a
JavaScriptCore context (``gi`` JavaScriptCore 4.1, WebKitGTK 2.52 with the system ICU) and compares the nl-NL / en-GB
output with the table of H's notes/english-option-design.md §2.8.1, plus list() and collator(), plural selection by
countOf(), and the CHECK report's module names from JSC's "fn@url:line:col" stacks. node is not the reference: it
bundles its own ICU.

Ported from H's tests/i18n/intl_jsc.py (E0a) in GE1a (notes/claude/round1/ge1a.md). Changes: the UI root is
vendor/ui; H's comparison of list() with its Python host's ``join()`` is gone (G's host is Rust; GE1b's i18n.rs has
its own join tests). Nothing but a JavaScriptCore context runs: no WebKit web process, no display, no D-Bus.

Exit 0 when everything matches, 1 on a mismatch (listed), 77 when the JavaScriptCore 4.1 typelib is missing (the
caller skips). The date rows assume TZ=Europe/Amsterdam (set when unset).
"""

from __future__ import annotations

import json
import os
import re
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
CORE = ROOT / "vendor" / "ui" / "js" / "core"

# Sunday 25 October 2026 14:35 in Amsterdam (CET, after the DST change of that night) = 13:35 UTC
WHEN_MS = 1792935300000

EXPECTED = {
    # name: (JS expression using `D` = WHEN_MS, nl-NL, en-GB)
    "date short": ("date(D, 'short')", "25-10-2026", "25/10/2026"),
    "date medium": ("date(D, 'medium')", "25 okt 2026", "25 Oct 2026"),
    "date day": ("date(D, 'day')", "zondag 25 oktober", "Sunday 25 October"),
    "date dayShort": ("date(D, 'dayShort')", "zo 25 okt", "Sun 25 Oct"),
    "date long": ("date(D, 'long')", "zondag 25 oktober 2026", "Sunday, 25 October 2026"),
    "date month": ("date(D, 'month')", "oktober 2026", "October 2026"),
    "date + time full": ("date(D, {dateStyle: 'full', timeStyle: 'short'})", "zondag 25 oktober 2026 om 14:35",
                         "Sunday, 25 October 2026 at 14:35"),
    "datetime": ("date(D, 'datetime')", "25-10-2026 14:35", "25/10/2026 14:35"),
    "time h24": ("time(D)", "14:35", "14:35"),
    "time 12-hour": ("time(D, {h24: false})", "2:35 p.m.", "2:35 pm"),
    "number": ("number(1234.56, 1)", "1.234,6", "1,234.6"),
    "number tie": ("number(2.25, 1)", "2,3", "2.3"),
    "bytes": ("bytes(1288490188)", "1,2 GB", "1.2 GB"),
    "bytes 0,98 GB": ("bytes(1053818880)", "0,98 GB", "0.98 GB"),
    "rate": ("rate(3565158)", "3,4 MB/s", "3.4 MB/s"),
    "percent": ("percent(45.2)", "45%", "45%"),
    "relative 5 min": ("relTime(D - 300000, D)", "5 minuten geleden", "5 minutes ago"),
    "relative yesterday": ("relTime(D - 86400000, D)", "gisteren", "yesterday"),
    "list 3": ("list(['A', 'B', 'C'])", "A, B en C", "A, B and C"),
    "list 2": ("list(['A', 'B'])", "A en B", "A and B"),
    "list 1": ("list(['A'])", "A", "A"),
    "list 0": ("list([])", "", ""),
    "list commas": ("list(['a, b', 'c'])", "a, b en c", "a, b and c"),
    "list or": ("list(['A', 'B', 'C'], 'disjunction')", "A, B of C", "A, B or C"),
    "collator numeric": ("['file10', 'b', 'file2', 'a', 'É', 'e'].sort(collator({numeric: true, sensitivity: 'base'}).compare).join(' ')",
                         "a b É e file2 file10", "a b É e file2 file10"),
    "collator default": ("['file10', 'b', 'file2', 'a', 'é', 'E'].sort(collator().compare).join(' ')",
                         "a b E é file10 file2", "a b E é file10 file2"),
    "collator locale": ("collator().resolvedOptions().locale", "nl", "en"),      # ICU: no region tailoring
    "plural formatted 1.000": ("PT('items', {n: number(1000), count: 1000})", "1.000 items", "1,000 items"),
    "plural formatted n only": ("PT('items', {n: number(1000)})", "1.000 items", "1,000 items"),
    "plural one": ("PT('items', {n: 1})", "1 item", "1 item"),
    "plural zero": ("PT('items', {n: 0})", "geen items", "no items"),
    "plural -1": ("PT('items', {n: -1})", "-1 items", "-1 items"),
    "PluralRules(-1) (why not Intl.PluralRules)": ("new Intl.PluralRules(locale()).select(-1)", "one", "one"),
}


def _strip_module(src: str) -> str:
    src = re.sub(r"^import\s[^;]*;\s*$", "", src, flags=re.M)
    return re.sub(r"^export\s+(?=(?:async\s+)?function|const|let|class)", "", src, flags=re.M)


def main() -> int:
    as_json = "--json" in sys.argv
    os.environ.setdefault("TZ", "Europe/Amsterdam")
    time.tzset()
    try:
        import gi
        gi.require_version("JavaScriptCore", "4.1")
        from gi.repository import JavaScriptCore as JSC
    except (ImportError, ValueError) as e:
        print(f"skip: JavaScriptCore 4.1 not available ({e})")
        return 77
    engine = f"{JSC.get_major_version()}.{JSC.get_minor_version()}.{JSC.get_micro_version()}"
    ctx = JSC.Context.new()

    def run(code: str, uri: str = "file:///intl_jsc.js"):
        v = ctx.evaluate_with_source_uri(code, -1, uri, 1)
        exc = ctx.get_exception()
        if exc is not None:
            ctx.clear_exception()
            raise RuntimeError(f"{uri}: {exc.get_message()}")
        return v

    run("var location = {search: '?i18ncheck=1'}; var window = globalThis; var document = {documentElement: {}};"
        "var console = {warn() {}, error() {}, log() {}};")
    run(_strip_module((CORE / "i18n.js").read_text(encoding="utf-8")), (CORE / "i18n.js").as_uri())
    run(_strip_module((CORE / "format.js").read_text(encoding="utf-8")), (CORE / "format.js").as_uri())
    demo = (ROOT / "vendor" / "ui" / "js" / "apps" / "intl-jsc-demo.js").as_uri()   # not a real file: the stack's URL
    run("var PT = makeT({nl: {items: {zero: 'geen items', one: '1 item', other: '{n} items'}, a: 'x {y}'},"
        " en: {items: {zero: 'no items', one: '1 item', other: '{n} items'}}});", demo)

    mismatches: list[dict] = []
    values: dict[str, dict] = {}
    for lg, loc in (("nl", "nl-NL"), ("en", "en-GB")):
        run(f"setLang('{lg}');")
        for name, (expr, want_nl, want_en) in EXPECTED.items():
            want = want_nl if lg == "nl" else want_en
            got = run(f"var D = {WHEN_MS}; String({expr})").to_string()
            values.setdefault(name, {})[loc] = got
            if got != want:
                mismatches.append({"what": name, "lang": loc, "got": got, "want": want})
        run("PT('a'); PT('nokey');")
    rep = json.loads(run("JSON.stringify(i18nReport)").to_string())
    want_rep = {"missing": ["apps/intl-jsc-demo.js:nl:nokey", "apps/intl-jsc-demo.js:en:a",
                            "apps/intl-jsc-demo.js:en:nokey"],
                "unfilled": ["apps/intl-jsc-demo.js:a:{y}"],
                "plural": ["apps/intl-jsc-demo.js:items"]}
    for k, want in want_rep.items():
        if sorted(rep.get(k, [])) != sorted(want):
            mismatches.append({"what": f"CHECK report {k} (JSC stack format)", "lang": "-", "got": rep.get(k),
                               "want": want})
    out = {"ok": not mismatches, "engine": engine, "tz": os.environ.get("TZ"), "mismatches": mismatches,
           "values": values}
    if as_json:
        print(json.dumps(out, ensure_ascii=False, indent=1))
    else:
        print(f"JavaScriptCore {engine}: {len(EXPECTED) * 2} values, {len(mismatches)} mismatch(es)")
        for m in mismatches:
            print(f"  {m['what']} [{m['lang']}]: got {m['got']!r}, want {m['want']!r}")
    return 0 if not mismatches else 1


if __name__ == "__main__":
    sys.exit(main())
