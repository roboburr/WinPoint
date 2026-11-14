"""The native layer of the i18n checker (tools/i18n/check_native.py) and the Start menu's copy of the settings
registry.

    python3 -B -m unittest tests/i18n/test_native.py -v          (from the project root; a few seconds)

* the layer reads every idiom it documents from a small fixture tree (calls by their definitions, match arms, row
  tables with a column legend, NL/EN arrays, ifs on the language, language matches, closures, struct fields, the
  page! macro, JSON objects), skips test code, and reports its rules (empty halves, placeholders, NL/EN tables,
  Dutch in English, apostrophes, the glossary, articles, allow-list rows, Dutch outside every pair);
* native/shell/src/start/settings_pages.json (what Start's search offers) is native/apps/settings/src/registry.rs
  (the Settings app's own registry) without its Home page: the same pages in the same order, with the same parent,
  icon, titles, descriptions and keywords. Change both, or Start finds a page by a title Settings no longer has;
* every app kind has its name (the window title, kit AppSpec) in both languages, as the WebKit host's C4 title test
  had it (`git show last-webkit:host/src/window.rs`, title_tests).

The fixture lives in tests/i18n/.run/ while a test runs (never /tmp: notes/claude/plan.md).
"""

from __future__ import annotations

import contextlib
import json
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
HERE = ROOT / "tests" / "i18n"
RUN = HERE / ".run"
sys.path.insert(0, str(ROOT / "tools" / "i18n"))

import check  # noqa: E402
import check_native  # noqa: E402

KIT = r'''
pub fn en() -> bool { false }
pub fn t<'a>(nl: &'a str, en_text: &'a str) -> &'a str { if en() { en_text } else { nl } }
pub fn label(nl: &'static str, en_text: &'static str) -> gtk::Label { gtk::Label::new(Some(t(nl, en_text))) }
pub struct AppSpec { pub kind: &'static str, pub title: (&'static str, &'static str) }
'''
CONTROLS = r'''
pub fn label(text: &str, class: &str) -> gtk::Label { gtk::Label::new(Some(text)) }
'''
DEMO = r'''
use kit::t;

fn tr(key: &str) -> &'static str {
    let (nl, en) = match key {
        "open" => ("Openen", "Open"),
        "empty" => ("", "Empty"),
        _ => ("", ""),
    };
    kit::t(nl, en)
}

/// (key, nl, en)
const TEXTS: &[(&str, &str, &str)] = &[
    ("trash", "Prullenbak", "Trash"),
    ("count", "{n} items", "{count} items"),
];

const DAYS_NL: [&str; 2] = ["maandag", "dinsdag"];
const DAYS_EN: [&str; 1] = ["Monday"];

const SPEC: kit::AppSpec = kit::AppSpec { kind: "demo", title: ("Voorbeeld", "Example") };

fn view(en: bool, key: &str, n: usize) {
    let a = kit::t("Bestand openen", "Open bestand");
    let b = t("Map", "Map");
    let c = if en { "Don't save" } else { "Niet opslaan" };
    let d = controls::label("Instellingen wijzigen", "w11-title");
    let e = kit::i18n::label("Nieuwe {type}", "A {type}");
    let f = t(&format!("{n} mappen"), &format!("{n} folders"));
    let with = |nl: &str, en_text: &str| if en { en_text.to_owned() } else { nl.to_owned() };
    let g = with("Tekst", "Text");
    let h = match (key, en) {
        ("yes", false) => "Ja",
        ("yes", true) => "Yes",
        _ => "",
    };
    for (id, nl, en) in [("x", "Knippen", "Cut")] {}
}

macro_rules! page {
    ($id:expr, $nl:expr, $en:expr, [$($knl:expr),*], [$($ken:expr),*]) => {
        Page { id: $id, title: ($nl, $en), keywords: (&[$($knl),*], &[$($ken),*]) }
    };
}

static PAGES: &[Page] = &[page!("home", "Startpagina", "Home", ["begin"], ["start"])];

#[cfg(test)]
mod tests {
    fn only_a_test() {
        kit::t("Alleen in een test", "Only in a test");
    }
}
'''
DATA = [{"id": "system", "title": {"nl": "Systeem", "en": "System"}, "keywords": {"nl": ["pc"], "en": ["computer"]}},
        {"id": "sound", "keywords": {"nl": ["geluid"], "en": []}}]
WORDS = "# test words\nbestand\nmap\nmappen\nopenen\ninstellingen\nwijzigen\nprullenbak\nalleen\n"
GLOSSARY = "Prullenbak\tRecycle Bin\nKnippen\tCut\n"


@contextlib.contextmanager
def fixture(allow: str = ""):
    RUN.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="native-", dir=RUN) as d:
        root = Path(d)
        files = {
            "native/kit/src/i18n.rs": KIT,
            "native/kit/src/controls/mod.rs": CONTROLS,
            "native/apps/demo/src/main.rs": DEMO,
            "native/apps/demo/src/data.json": json.dumps(DATA),
            "native/kit/examples/gallery.rs": 'fn main() { kit::t("Galerij", "Gallerij"); }\n',
            "tests/i18n/dutch-words.txt": WORDS,
            "tools/i18n/glossary.tsv": GLOSSARY,
        }
        if allow:
            files[check_native.ALLOW] = allow
        for rel, text in files.items():
            (root / rel).parent.mkdir(parents=True, exist_ok=True)
            (root / rel).write_text(text, encoding="utf-8")
        yield root


def layer(root: Path) -> check.Report:
    rep = check.Report()
    check_native.check_layer(root, rep, check.Glossary(root / "tools/i18n/glossary.tsv"))
    return rep


class Idioms(unittest.TestCase):
    def test_every_documented_idiom_is_read(self):
        with fixture() as root:
            pairs, _ = check_native.read_pairs(root)
        got = {(p.kind, p.key, str(p.nl), str(p.en)) for p in pairs}
        want = {
            ("arm", "open", "Openen", "Open"),
            ("arm", "empty", "", "Empty"),
            ("row", "trash", "Prullenbak", "Trash"),
            ("row", "count", "{n} items", "{count} items"),
            ("table", "DAYS[0]", "maandag", "Monday"),
            ("row", "AppSpec(demo).title", "Voorbeeld", "Example"),
            ("call", "view/Bestand openen", "Bestand openen", "Open bestand"),
            ("call", "view/Map", "Map", "Map"),
            ("if", "view/Niet opslaan", "Niet opslaan", "Don't save"),
            ("call", "view/Nieuwe {type}", "Nieuwe {type}", "A {type}"),
            ("call", "view/{n} mappen", "{n} mappen", "{n} folders"),
            ("call", "view/Tekst", "Tekst", "Text"),
            ("arm", "yes", "Ja", "Yes"),
            ("row", "x", "Knippen", "Cut"),
            ("macro", "page(home).title", "Startpagina", "Home"),
            ("macro", "page(home).keywords", "['begin']", "['start']"),
            ("json", "system.title", "Systeem", "System"),
            ("json", "system.keywords", "['pc']", "['computer']"),
            ("json", "sound.keywords", "['geluid']", "[]"),
        }
        self.assertEqual(sorted(got - want), [], "read but not expected")
        self.assertEqual(sorted(want - got), [], "expected but not read")

    def test_test_code_the_gallery_and_other_functions_of_the_same_name_are_no_pairs(self):
        with fixture() as root:
            pairs, _ = check_native.read_pairs(root)
        texts = {str(p.nl) for p in pairs}
        self.assertNotIn("Alleen in een test", texts)                  # #[cfg(test)]
        self.assertNotIn("Galerij", texts)                             # native/kit/examples
        self.assertNotIn("Instellingen wijzigen", texts)               # controls::label(text, class)
        self.assertNotIn("Ja", {str(p.en) for p in pairs})             # the language bool picked en, not nl


class Rules(unittest.TestCase):
    def issues(self, rep: check.Report) -> set[tuple[str, str, str]]:
        return {(i.level, i.rule, i.msg.split(":")[0]) for i in rep.issues}

    def test_the_rules_fire(self):
        with fixture() as root:
            rep = layer(root)
        got = self.issues(rep)
        for want in [
            ("error", "native-pair", "empty"),
            ("error", "native-pair", "sound.keywords"),
            ("error", "native-placeholders", "count"),
            ("error", "native-table", "DAYS_NL has 2 entries, DAYS_EN 1"),
            ("error", "glossary", "trash"),
            ("error", "native-dutch", "view/Bestand openen"),
            ("error", "native-dutch", "view/Map"),
            ("error", "native-apostrophe", "view/Niet opslaan"),
            ("warning", "article", "view/Nieuwe {type}"),
            ("warning", "native-dutch-only", "Dutch outside an nl/en pair"),
        ]:
            self.assertIn(want, got)
        self.assertNotIn(("error", "glossary", "x"), got, "Knippen → Cut has its glossary phrase")
        self.assertNotIn(("error", "native-placeholders", "view/{n} mappen"), got, "format! names match")
        self.assertEqual(rep.stats["native_uncovered"], 1)              # "Instellingen wijzigen"
        self.assertEqual(rep.stats["native_pairs"], 19)

    def test_allow_rows_accept_a_dutch_name_and_go_stale(self):
        allow = ("# fixture\n"
                 "native/apps/demo/src/*.rs\ten:bestand\ta Dutch word on purpose (fixture)\n"
                 "native/apps/demo/src/*.rs\ten:Alles\tnever used (stale)\n"
                 "native/apps/demo/src/*.rs\tAlles\tmalformed\n")
        with fixture(allow) as root:
            got = self.issues(layer(root))
        self.assertNotIn(("error", "native-dutch", "view/Bestand openen"), got)
        self.assertIn(("error", "native-dutch", "view/Map"), got)
        self.assertIn(("error", "native-allow", "native/apps/demo/src/*.rs en"), got)
        self.assertIn(("error", "native-allow", "expected <file glob><TAB>en"), got)

    def test_the_real_allow_list_is_well_formed(self):
        rep = check.Report()
        check_native.load_allow(ROOT, rep)
        self.assertEqual([i.text() for i in rep.issues], [])


# ------------------------------------------------------------------------------------------------------------------
# settings_pages.json against registry.rs
# ------------------------------------------------------------------------------------------------------------------

REGISTRY = "native/apps/settings/src/registry.rs"
START_PAGES = "native/shell/src/start/settings_pages.json"
# Pages of the registry that Start's search does not offer: Settings' own Home page (the JSON never had it,
# f728b33b; registry.js's search in the HTML shell did).
NOT_IN_START = {"home"}


def registry_pages(root: Path = ROOT) -> list[dict]:
    """The page!(id, parent, icon, title nl, en, description nl, en, [keywords nl], [keywords en]) calls of
    registry.rs, read with the native layer's lexer, as settings_pages.json writes a page."""
    sources, _ = check_native.load(root)
    src = next(s for s in sources if s.rel == REGISTRY)
    toks, out = src.toks, []

    def value(a: int, b: int):
        k = check_native.lit_at(src, a, b)
        if k is not None:
            return toks[k].text
        texts = [t.text for t in toks[a:b]]
        if texts == ["None"]:
            return None
        if len(texts) == 4 and texts[0] == "Some" and toks[a + 2].kind == "str":
            return toks[a + 2].text
        arr = check_native.array_at(src, a)
        if arr is not None:
            return [toks[check_native.lit_at(src, x, y)].text for x, y in check_native.split_top(toks, *arr)]
        raise ValueError(f"{REGISTRY}:{toks[a].line}: unexpected page! argument {' '.join(texts)}")

    for i in range(len(toks) - 2):
        if toks[i].text == "page" and toks[i + 1].text == "!" and toks[i + 2].text == "(" and toks[i - 1].text != "macro_rules":
            v = [value(a, b) for a, b in check_native.split_top(toks, i + 3, src.close[i + 2])]
            if len(v) != 9:
                raise ValueError(f"{REGISTRY}:{toks[i].line}: page! with {len(v)} arguments")
            out.append({"id": v[0], "parent": v[1], "icon": v[2], "title": {"nl": v[3], "en": v[4]},
                        "description": {"nl": v[5], "en": v[6]}, "keywords": {"nl": v[7], "en": v[8]}})
    return out


def differences(registry: list[dict], start: list[dict]) -> list[str]:
    want = [p for p in registry if p["id"] not in NOT_IN_START]
    out = []
    if [p["id"] for p in start] != [p["id"] for p in want]:
        out.append(f"pages: Start has {[p['id'] for p in start]}, the registry {[p['id'] for p in want]}")
    by_id = {p["id"]: p for p in want}
    for p in start:
        r = by_id.get(p["id"])
        if r is None:
            continue
        for k in sorted(set(r) | set(p)):
            if p.get(k) != r.get(k):
                out.append(f"{p['id']}.{k}: Start has {p.get(k)!r}, the registry {r.get(k)!r}")
    return out


class SettingsPages(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.registry = registry_pages()
        cls.start = json.loads((ROOT / START_PAGES).read_text(encoding="utf-8"))

    def test_start_offers_the_registry_s_pages(self):
        self.assertGreater(len(self.registry), 30)
        self.assertEqual(NOT_IN_START - {p["id"] for p in self.registry}, set(), "NOT_IN_START names no page")
        self.assertEqual(differences(self.registry, self.start), [],
                         f"{START_PAGES} and {REGISTRY} disagree: change both")

    def test_a_drift_is_found(self):
        start = json.loads(json.dumps(self.start))
        start[0]["title"]["en"] += " (old)"
        start[1]["keywords"]["nl"] = start[1]["keywords"]["nl"][:-1]
        del start[-1]
        got = differences(self.registry, start)
        self.assertEqual(len(got), 3, got)


# ------------------------------------------------------------------------------------------------------------------
# the apps' names
# ------------------------------------------------------------------------------------------------------------------

# kind: (GApplication id, Dutch, English) of each app's kit::AppSpec::new(kind, id, nl, en) in native/apps/*/src
APP_NAMES = {
    "settings": ("io.github.win11shell.Settings", "Instellingen", "Settings"),
    "taskmgr": ("io.github.win11shell.TaskManager", "Taakbeheer", "Task Manager"),
    "alles": ("io.github.win11shell.Search", "Alles", "Alles"),
    "disks": ("io.github.win11shell.Disks", "Schijven", "Disks"),
    "defaultapps": ("io.github.win11shell.DefaultApps", "Standaard-apps", "Default apps"),
    "installer": ("io.github.win11shell.Installer", "App-installatieprogramma", "App Installer"),
    "explorer": ("io.github.win11shell.Explorer", "Verkenner", "File Explorer"),
}


def app_specs(root: Path = ROOT) -> dict[str, tuple[str, ...]]:
    """kind: (id, nl, en) of every AppSpec::new call with four literals in native/apps, read with the layer's lexer."""
    sources, _ = check_native.load(root)
    out = {}
    for src in sources:
        if not src.rel.startswith("native/apps/"):
            continue
        toks = src.toks
        for i in range(len(toks) - 3):
            if [t.text for t in toks[i:i + 4]] == ["AppSpec", "::", "new", "("]:
                spans = check_native.split_top(toks, i + 4, src.close[i + 3])
                args = [check_native.lit_at(src, a, b) for a, b in spans]
                if len(args) != 4 or None in args:
                    raise ValueError(f"{src.rel}:{toks[i].line}: AppSpec::new without four literals")
                kind, *rest = (toks[k].text for k in args)
                if kind in out:
                    raise ValueError(f"{src.rel}:{toks[i].line}: a second AppSpec for {kind}")
                out[kind] = tuple(rest)
    return out


class AppNames(unittest.TestCase):
    def test_every_kind_has_its_name_in_both_languages(self):
        self.assertEqual(app_specs(), APP_NAMES)


def tearDownModule() -> None:
    with contextlib.suppress(OSError):
        RUN.rmdir()                  # only when empty


if __name__ == "__main__":
    unittest.main()
