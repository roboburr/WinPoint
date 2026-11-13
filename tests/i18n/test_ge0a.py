"""UI i18n tooling and the Dutch/English UI baselines of Win11 voor GNOME (GE0a, notes/claude/round0/ge0a.md; ported
from H's tests/i18n/test_e0b.py, dropping its Python-host, bash, audit and transcript parts).

    python3 -m unittest tests/i18n/test_ge0a.py -v            (from the project root; a few seconds, no browser)
    W11_I18N_HEAVY=1 python3 -m unittest tests/i18n/test_ge0a.py -v
                                                              (also: the nl and en UI dumps in headless Chrome must
                                                               equal the baselines; about a minute, needs 2 GB free
                                                               and the heavy lock, see tests/i18n/run.sh)

* the UI ratchet (tools/i18n/ratchet.py): no new Dutch-only text in vendor/ui/js beyond tests/i18n/allow-ui.txt, no
  stale entry; the scanner's rules on a small fixture;
* the checker (tools/i18n/check.py): no errors;
* every host method the UI calls has a mock (a surface that calls an unknown method shows "Onbekende functie");
* the baselines are well-formed: baseline-nl.txt and baseline-en.txt are dumps of exactly surfaces.json (whose
  settings pages are the registry's and whose taskmgr.* ids are Taakbeheer's tabs), with no harness trouble;
  dutch-words.txt is a sorted list of lower-case words; BASELINE-CHANGES rows have their fields;
* the dump server serves only vendor/ui (as /ui/) and tests/i18n, read-only, with freeze.js first;
* the private session (tests/i18n/private.sh) refuses a work dir outside tests/ and hands its command a clean
  environment: no display, no live bus, private HOME and XDG dirs, keyfile GSettings, NO_AT_BRIDGE=1.

Temporary files go to tests/i18n/.run/ (never /tmp: notes/claude/plan.md).
"""

from __future__ import annotations

import contextlib
import io
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import unittest
import urllib.error
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
HERE = ROOT / "tests" / "i18n"
TOOLS = ROOT / "tools" / "i18n"
RUN = HERE / ".run"
sys.path.insert(0, str(TOOLS))

import check  # noqa: E402
import corpus  # noqa: E402
import paths  # noqa: E402
import ratchet  # noqa: E402

NODE = corpus.find_node()
HEAVY = os.environ.get("W11_I18N_HEAVY") == "1"
UI = paths.ui_js(ROOT)
# BASELINE-CHANGES step names (notes/claude/plan.md)
STEPS = re.compile(r"^(GE0[a-e]|GE1[a-c]|GE[2-9][a-z]?|R[0-9]+[a-z]?|-)$")


def tmpdir(prefix: str) -> tempfile.TemporaryDirectory:
    RUN.mkdir(exist_ok=True)
    return tempfile.TemporaryDirectory(prefix=prefix, dir=RUN)


def tearDownModule() -> None:
    with contextlib.suppress(OSError):
        RUN.rmdir()                  # only when empty


def mem_available_gb() -> float:
    for ln in Path("/proc/meminfo").read_text().split("\n"):
        if ln.startswith("MemAvailable:"):
            return int(ln.split()[1]) / 1024 / 1024
    return 0.0


class NodeAvailable(unittest.TestCase):
    def test_node_is_there(self):
        self.assertTrue(NODE, "node not found: the UI ratchet and the UI part of check.py did not run (set $NODE)")


@unittest.skipUnless(NODE, "node not found")
class Ratchet(unittest.TestCase):
    """No visible string without a key: a new Dutch-only text fails here, naming the file and the fix; so does an
    allowlist entry the tree no longer needs (prune it in the step that converted the text)."""

    def test_ui(self):
        r = ratchet.check("ui")
        self.assertTrue(r.ok, "\n" + ratchet.report(r))

    def test_allowlist_is_well_formed(self):
        rows = ratchet.read_allow(HERE / ratchet.ALLOW["ui"])
        self.assertTrue(rows)
        for (f, hx), e in rows.items():
            self.assertRegex(hx, r"^[0-9a-f]{12}$", f)
            self.assertGreaterEqual(e.count, 1, f"{f} {hx}")
            self.assertTrue(f.startswith(paths.UI_REL + "/"), f)
            self.assertTrue((ROOT / f).exists(), f"{f} does not exist (prune it)")
            self.assertRegex(e.status, r"^((keep|todo): \S.*)?$", f"{f} {hx}")

    def test_scanner_rules(self):
        words = {"bestand", "verwijderd", "niet", "gevonden", "map"}
        src = r"""
import { makeT } from '../core/i18n.js';
import x from './bestand-niet.js';
const t = makeT({ nl: { a: 'Bestand niet gevonden' }, en: { a: 'File not found' } });
const pair = { nl: 'Map verwijderd', en: 'Folder deleted' };
const TYPES_NL = { a: 'Bestand' }; const TYPES_EN = { a: 'File' };
console.warn('bestand niet gevonden');
el.title = 'Bestand niet gevonden';
const s = en ? 'Deleted' : 'Verwijderd';
const tpl = `Het bestand ${n} is niet gevonden`;
"""
        js = r"""
import { scanFile } from '%s';
const src = %s; const words = new Set(%s);
process.stdout.write(JSON.stringify(scanFile('apps/x.js', src, words)));
""" % ((TOOLS / "scan_ui.mjs").as_uri(), json.dumps(src), json.dumps(sorted(words)))
        r = subprocess.run([NODE, "--input-type=module", "-e", js], capture_output=True, text=True, timeout=60)
        self.assertEqual(r.returncode, 0, r.stderr)
        got = [(h["file"], h["kind"], h["text"]) for h in json.loads(r.stdout)]
        f = paths.UI_REL + "/apps/x.js"
        self.assertEqual(got, [(f, "string", "Bestand niet gevonden"), (f, "string", "Verwijderd"),
                               (f, "template", "Het bestand ${…} is niet gevonden")])

    def test_counts_and_stale(self):
        with tmpdir("ratchet-") as d:
            d = Path(d)
            f = paths.UI_REL + "/a.js"
            hits = [{"file": f, "line": 3, "kind": "string", "text": "Bestand  niet\ngevonden"}]
            ratchet.write_allow(d / ratchet.ALLOW["ui"], "ui", ratchet.entries(hits))
            same = [{"file": f, "line": 9, "kind": "string", "text": "Bestand niet gevonden"}]
            r = ratchet.check("ui", allow_dir=d, hits=same)
            self.assertEqual((r.new, r.stale), ([], []))           # moved and re-wrapped: the same entry
            two = same + [{"file": f, "line": 12, "kind": "string", "text": "Bestand niet gevonden"}]
            r = ratchet.check("ui", allow_dir=d, hits=two)
            self.assertEqual([h["line"] for h in r.new], [12])     # a second copy is new
            r = ratchet.check("ui", allow_dir=d, hits=[])
            self.assertEqual((r.new, len(r.stale), r.ok), ([], 1, False))   # converted: stale until pruned

    def test_allow_needs_a_reason_and_a_known_prefix(self):
        with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
            with self.assertRaises(SystemExit):
                ratchet.main(["--allow", paths.UI_REL + "/core/bridge.js:0123456789ab", "--keep"])
            with self.assertRaises(SystemExit):
                ratchet.main(["--allow", "host/src/lib.rs:0123456789ab", "--reason", "x"])


@unittest.skipUnless(NODE, "node not found")
class Checker(unittest.TestCase):
    def test_no_errors(self):
        rep = check.run(ROOT)
        self.assertEqual([i.text() for i in rep.errors], [])
        self.assertGreater(rep.stats.get("js_keys", 0), 2000)
        self.assertGreater(rep.stats.get("js_dicts", 0), 60)


class MockCoverage(unittest.TestCase):
    """Every host method the UI calls (a literal method name outside mock/) has a mock: otherwise its surface shows
    "Onbekende functie: …" in mock mode (and run.sh --notes lists a "[mock] unknown method" warning)."""

    CALL = re.compile(r"\b(?:\w*[cC]all|run)\(\s*['\"`]([a-zA-Z]+\.[a-zA-Z.]+)['\"`]")
    DEF = re.compile(r"^\s*'([a-zA-Z]+\.[a-zA-Z.]+)'\s*:", re.M)

    def test_every_called_method_is_mocked(self):
        called: dict[str, str] = {}
        for f in sorted(UI.rglob("*.js")):
            rel = f.relative_to(UI).as_posix()
            if rel.startswith("mock/"):
                continue
            for m in self.CALL.finditer(f.read_text(encoding="utf-8")):
                called.setdefault(m.group(1), rel)
        mocked = set()
        for f in sorted((UI / "mock").glob("*.js")):
            mocked |= set(self.DEF.findall(f.read_text(encoding="utf-8")))
        self.assertGreater(len(called), 100)
        missing = sorted(f"{k} ({v})" for k, v in called.items() if k not in mocked)
        self.assertEqual(missing, [], "add a mock (vendor/ui/js/mock/*.js) and a tests/i18n/MOCK-CHANGES.md row")

    def test_every_mock_file_is_loaded(self):
        src = (UI / "mock" / "index.js").read_text(encoding="utf-8")
        files = set(re.findall(r"'([a-z_]+)'", re.search(r"const FILES = \[([^\]]*)\]", src).group(1)))
        have = {p.stem for p in (UI / "mock").glob("*.js")} - {"index"}
        self.assertEqual(sorted(have - files), [], "a mock file that mock/index.js does not load")


class Baselines(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.surfaces = json.loads((HERE / "surfaces.json").read_text(encoding="utf-8"))["surfaces"]

    def test_surfaces(self):
        ids = [s["id"] for s in self.surfaces]
        self.assertEqual(len(ids), len(set(ids)), "duplicate surface ids")
        for s in self.surfaces:
            # D-G70: the HTML shell and desktop surfaces are gone (the native GTK 4 shell replaced them)
            self.assertRegex(s["q"], r"^surface=app\b", s["id"])
            self.assertNotRegex(s["q"], r"(^|&)(lang|mock|mockreset|mockStatic|i18ncheck|i18nstop|token)=", s["id"])
            for step in s.get("do", []):
                self.assertTrue(set(step) <= {"click", "contextmenu", "nth", "wait"}, s["id"])
                self.assertEqual(len({"click", "contextmenu"} & set(step)), 1, s["id"])
            # R3-UI fix round: dump.html's checks of a state the text does not show
            self.assertTrue(set(s) <= {"id", "q", "wait", "do", "seed", "probe", "size"}, s["id"])
            for step in s.get("probe", []):
                self.assertTrue(set(step) <= {"disabled", "inview", "nth"}, s["id"])
                self.assertEqual(len({"disabled", "inview"} & set(step)), 1, s["id"])

    def test_probes_hold(self):
        """The settings-check.* probes record the expected state in both baselines (R3-UI fix round)."""
        want = {
            "settings-check.restartall-dry": "# probe disabled .st-notice .fl-infobar__action .fl-btn: false",
            "settings-check.restartall-sent": "# probe disabled .st-notice .fl-infobar__action .fl-btn: true",
            "settings-check.recovery-link": "# probe inview .st-upd__backup: true",
            "settings-check.recovery-none": "# probe inview .st-upd__backup: false",
        }
        self.assertEqual(sorted(want), sorted(s["id"] for s in self.surfaces if "probe" in s))
        for lang in ("nl", "en"):
            text = (HERE / f"baseline-{lang}.txt").read_text(encoding="utf-8")
            for sid, line in want.items():
                block = re.search(rf"^== {re.escape(sid)} .*?(?=^== )", text, re.M | re.S)
                self.assertIsNotNone(block, f"{lang} {sid}")
                self.assertIn(line + "\n", block.group(0), f"{lang} {sid}")
            # a hand-edited "EN" shows the option C1 resolves it to (cross.md §7 item 6), not the placeholder
            block = re.search(r"^== settings-check\.language-EN .*?(?=^== )", text, re.M | re.S).group(0)
            self.assertIn("\nNederlands\n", block, lang)

    def test_every_settings_page_and_taskmgr_tab(self):
        reg = (UI / "apps/settings/registry.js").read_text(encoding="utf-8")
        pages = re.findall(r"^  P\('([a-z0-9-]+)'", reg, re.M)
        got = [s["id"][len("settings."):] for s in self.surfaces
               if s["id"].startswith("settings.") and "do" not in s]
        self.assertEqual(sorted(set(got) - {"search"}), sorted(pages))
        tm = (UI / "apps/taskmgr/main.js").read_text(encoding="utf-8")
        tabs = re.findall(r"'([a-z]+)'", re.search(r"const PAGE_IDS = \[([^\]]*)\]", tm).group(1))
        got = [s["id"][len("taskmgr."):] for s in self.surfaces if s["id"].startswith("taskmgr.")]
        self.assertEqual(sorted(got), sorted(tabs))

    def test_every_app_has_a_surface(self):
        main = (UI / "main.js").read_text(encoding="utf-8")
        apps = re.findall(r"'([a-z]+)'", re.search(r"const APPS = \[([^\]]*)\]", main).group(1))
        have = {m.group(1) for s in self.surfaces for m in [re.search(r"(?:^|&)app=([a-z]+)", s["q"])] if m}
        self.assertEqual(sorted(set(apps) - have), [])

    def _baseline(self, lang: str) -> None:
        text = (HERE / f"baseline-{lang}.txt").read_text(encoding="utf-8")
        head = text.split("\n", 1)[0]
        self.assertRegex(head, rf"^# UI text dump · lang={lang} · .+ · \d+ surfaces · tests/i18n/run\.sh$")
        blocks = re.findall(r"^== (\S+) /ui/index\.html\?(\S*) ==$", text, re.M)
        self.assertEqual(blocks, [(s["id"], s["q"]) for s in self.surfaces],
                         f"baseline-{lang}.txt is not a dump of surfaces.json (re-record: tests/i18n/run.sh --record "
                         f"--lang {lang}, with --accept for changed blocks)")
        self.assertRegex(text, r"\n== END \d+ surfaces ==\n$")
        for bad in ("# NOT READY", "# NOT IDLE", "# HARNESS ERROR", "# DO FAILED", "# NO i18n REPORT",
                    "# i18n missing:", "# i18n unfilled:", "Onbekende functie", "Unknown function"):
            self.assertNotIn(bad, text)

    def test_baseline_nl(self):
        self._baseline("nl")

    def test_baseline_en(self):
        self._baseline("en")

    def test_dutch_words(self):
        raw = (HERE / "dutch-words.txt").read_text(encoding="utf-8").split("\n")
        words = [w for w in raw if w and not w.startswith("#")]
        self.assertGreater(len(words), 1000)
        self.assertEqual(words, sorted(set(words)), "sorted and unique")
        for w in words:
            self.assertEqual(w, w.lower(), w)
            self.assertRegex(w, r"^[a-zà-öø-ÿ]+('[a-zë]+)?$", w)

    def test_baseline_changes_rows(self):
        for ln in (HERE / "BASELINE-CHANGES").read_text(encoding="utf-8").split("\n"):
            if ln and not ln.startswith("#"):
                parts = ln.split("\t")
                self.assertGreaterEqual(len(parts), 3, ln)
                self.assertRegex(parts[0], STEPS, ln)


class DumpServer(unittest.TestCase):
    def test_mounts_freeze_first_and_read_only(self):
        with tmpdir("serve-") as d:
            root = Path(d)
            (root / "vendor" / "ui").mkdir(parents=True)
            (root / "tests" / "i18n").mkdir(parents=True)
            (root / "vendor" / "ui" / "index.html").write_text("<!doctype html>\n<html>\n<head>\n<title>x</title>\n</head></html>\n")
            (root / "vendor" / "ui" / "a.js").write_text("export const a = 1;\n")
            (root / "tests" / "i18n" / "freeze.js").write_text("/* x */\n")
            (root / "secret.txt").write_text("not served\n")
            p = subprocess.Popen([sys.executable, "-I", str(HERE / "serve.py"), str(root)], stdout=subprocess.PIPE,
                                 text=True)
            try:
                port = int(p.stdout.readline())
                get = lambda u: urllib.request.urlopen(f"http://127.0.0.1:{port}{u}", timeout=10)  # noqa: E731
                for url in ("/ui/index.html?x=1", "/ui/"):
                    with get(url) as r:
                        body = r.read().decode()
                    self.assertTrue(body.startswith('<!doctype html>\n<html>\n<head>\n<script src="/tests/i18n/'
                                                    'freeze.js"></script>\n'), body)
                with get("/ui/a.js") as r:
                    self.assertEqual(r.headers.get("Cache-Control"), "no-store")
                with get("/tests/i18n/freeze.js") as r:
                    self.assertEqual(r.read(), b"/* x */\n")
                for url in ("/", "/secret.txt", "/vendor/ui/a.js", "/ui/%2e%2e/%2e%2e/secret.txt", "/tests/"):
                    with self.assertRaises(urllib.error.HTTPError, msg=url) as cm:
                        get(url)
                    self.assertEqual(cm.exception.code, 404, url)
                    cm.exception.close()
                req = urllib.request.Request(f"http://127.0.0.1:{port}/ui/a.js", data=b"x", method="POST")
                with self.assertRaises(urllib.error.HTTPError) as cm:
                    urllib.request.urlopen(req, timeout=10)
                self.assertEqual(cm.exception.code, 405)
                cm.exception.close()
            finally:
                p.terminate()
                p.wait(10)
                p.stdout.close()


@unittest.skipUnless(shutil.which("dbus-run-session"), "no dbus-run-session")
class PrivateSession(unittest.TestCase):
    def test_refuses_a_workdir_outside_tests(self):
        for bad in (ROOT / "notes" / "ge0a-must-not-exist", ROOT / "tests", ROOT / "tests" / ".." / "notes" / "x"):
            r = subprocess.run(["bash", str(HERE / "private.sh"), str(bad), "true"], capture_output=True,
                               text=True, timeout=60)
            self.assertEqual(r.returncode, 70, f"{bad}: {r.stderr}")
        self.assertFalse((ROOT / "notes" / "ge0a-must-not-exist").exists(), "a refused work dir was created")

    def test_clean_environment(self):
        with tmpdir("private-") as d:
            r = subprocess.run(["bash", str(HERE / "private.sh"), d, "env"], capture_output=True, text=True,
                               timeout=60, env={**os.environ, "DISPLAY": ":0", "WAYLAND_DISPLAY": "wayland-0"})
            self.assertEqual(r.returncode, 0, r.stderr)
            env = dict(ln.split("=", 1) for ln in r.stdout.splitlines() if "=" in ln)
            for k in ("DISPLAY", "WAYLAND_DISPLAY", "XAUTHORITY", "SESSION_MANAGER", "SSH_AUTH_SOCK"):
                self.assertNotIn(k, env)
            self.assertTrue(env["DBUS_SESSION_BUS_ADDRESS"].startswith("unix:abstract=w11-i18n-"))
            for k in ("HOME", "XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_CACHE_HOME", "XDG_RUNTIME_DIR", "TMPDIR"):
                self.assertTrue(env[k].startswith(str(Path(d).resolve()) + "/"), k)
            self.assertEqual((env["GSETTINGS_BACKEND"], env["NO_AT_BRIDGE"], env["GTK_A11Y"]), ("keyfile", "1", "none"))


@unittest.skipUnless(HEAVY, "W11_I18N_HEAVY=1 runs the UI dumps (headless Chrome)")
class Reproduce(unittest.TestCase):
    def _run(self, *args: str) -> None:
        if mem_available_gb() < 2:
            self.skipTest("MemAvailable under 2 GB: no headless Chrome now")
        r = subprocess.run(["bash", str(HERE / "run.sh"), *args], capture_output=True, text=True, timeout=3 * 3600)
        self.assertEqual(r.returncode, 0, r.stdout[-4000:] + r.stderr[-2000:])

    def test_nl_dump_equals_the_baseline(self):
        self._run()

    def test_en_dump_equals_the_baseline(self):
        self._run("--lang", "en")


if __name__ == "__main__":
    unittest.main()
