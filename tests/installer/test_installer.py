#!/usr/bin/env python3
"""GE1c installer tests (scripts/install.py, uninstall.py and their helpers). Start them with tests/installer/run.sh.

Every install runs through sandbox.Case (bubblewrap, private HOME/XDG/prefix, PATH stubs, optional strace) in
tests/installer/run/<case>; Robin's HOME is not even visible there. Fast cases use a small fixture tree and
placeholder binaries; the REAL cases install a snapshot of this tree's vendor/ with the binaries of
tests/build/3b898e6-debug plus the native ones (install.NATIVE_BINS) of tests/build/native-release under strace, in
a path with spaces, % and $. The 3b898e6 binaries link WebKit: on a machine without the libraries they need, the REAL
cases are skipped and say which.
The byte-for-byte cases run scripts/*.py of 3b898e6 (git show) and the current ones in the same case dir. What
differs from 3b898e6 by design is left out of those comparisons (SHELL_SPLIT, same_as_baseline): D-G70 removed the
WebKit Start and desktop (win11-start, win11-desktop and their entries), the native shell, the Calendar app and the
native apps came with D-G63/D-G68 and 13-11 (win11-shell, win11-calendar, win11-gtk-<kind>, the Calendar entry and its
URI scheme default), D-G65 the payload's config/ folder, and since the HTML apps went (git tag last-webkit) the payload
has no host/ (vendor/host stays in the source tree, for 3b898e6's installer) and only ui/assets of ui/; and the done2
line of the closing text follows install.py.
Environment: W11_INSTALLER_REAL=0 skips the two REAL cases.
"""
from __future__ import annotations
import contextlib
import importlib
import io
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import unittest
from unittest import mock

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import sandbox  # noqa: E402
from sandbox import G, RUN, Case, settings_doc, write_json, desktop_entry  # noqa: E402

sys.path.insert(0, str(G/"scripts"))
install = importlib.import_module("install")

NASTY = 'ge1c sp %u $HOME `q` "dq"'
ENTRY_NAMES = json.loads((G/"host/src/desktop-entries.json").read_text())
DUTCH = {w.strip() for w in (G/"tests/i18n/dutch-words.txt").read_text().splitlines()
         if w.strip() and not w.startswith("#")}
SCRIPTS = G/"scripts"
NL_LOCALE = {"LANG": "nl_NL.UTF-8", "TZ": "Europe/Amsterdam"}
EN_LOCALE = {"LANG": "en_GB.UTF-8", "TZ": "Europe/London"}


def dutch_words(text: str) -> list[str]:
    return sorted({w for w in re.findall(r"[a-zà-ÿ]+", text.lower()) if w in DUTCH})


class Shared:
    built = False

    @classmethod
    def build(cls):
        if cls.built:
            return
        RUN.mkdir(parents=True, exist_ok=True)
        cls.stubs = sandbox.stub_build(RUN/"stub-build")
        cls.root = sandbox.fixture_root(RUN/"fixture-root")
        cls.old = sandbox.old_scripts(RUN/"old-scripts")
        cls.built = True


def entries_in(case: Case) -> dict[str, dict]:
    return {name: desktop_entry(case.data/"applications"/f"io.github.win11shell.{name}.desktop")
            for name in install.ENTRIES}


def entry_names(lang: str) -> dict[str, str]:
    """Name= of each entry install.py writes, in `lang` (host/src/desktop-entries.json)."""
    return {name: ENTRY_NAMES["entries"][name][lang] for name in install.ENTRIES}


# 3b898e6's own entries (its install.py wrote these ten; it had no ENTRIES tuple) and its closing line 2.
BASELINE_ENTRIES = ("Desktop", "Panel", "Start", "Settings", "DefaultApps", "Installer", "Search", "Disks",
                    "TaskManager", "Explorer")
BASELINE_DONE2_NL = "Start en Verkenner zijn los te openen via het applicatieoverzicht."


def shell_split() -> set[str]:
    """Paths of a case dir that differ from 3b898e6's install by design (module docstring): the binaries and entries
    of one installer only, and what goes with them."""
    bins = set(sandbox.baseline_bins()) ^ (set(install.BINS) | set(getattr(install, "NATIVE_BINS", ())))
    entries = set(BASELINE_ENTRIES) ^ (set(install.ENTRIES) | set(getattr(install, "NATIVE_ENTRIES", {})))
    out = {f"prefix/bin/{name}" for name in bins}
    out |= {f"data/applications/io.github.win11shell.{name}.desktop" for name in entries}
    if entries:
        out |= {"data/applications/mimeinfo.cache", "config/mimeapps.list"}
    return out


SHELL_SPLIT = shell_split()


PAYLOAD = "prefix/share/win11-gnome/"


def same_as_baseline(tree: dict[str, str]) -> dict[str, str]:
    """A case tree without SHELL_SPLIT, the payload's config/ folder (D-G65) and host/ (3b898e6 installed vendor/host,
    the Python reference; the current installer does not), and without what the payload's ui/ holds besides assets/
    (3b898e6 copied all of vendor/ui, the HTML apps included; the current installer copies vendor/ui/assets)."""
    def by_design(key: str) -> bool:
        if not key.startswith(PAYLOAD):
            return False
        top, _, rest = key[len(PAYLOAD):].partition("/")
        return top in ("config", "host") or (top == "ui" and rest != "" and rest.split("/", 1)[0] != "assets")
    return {k: v for k, v in tree.items() if k not in SHELL_SPLIT and not by_design(k)}


def as_now(old_stdout: str) -> str:
    """3b898e6's Dutch closing text with install.py's current line 2."""
    return old_stdout.replace(BASELINE_DONE2_NL + "\n", install.t("done2", "nl") + "\n")


def nl_done(prefix: Path) -> str:
    return (f"Geïnstalleerd in {prefix}/bin\n{install.t('done2', 'nl')}\n"
            "De taakbalk is geïnstalleerd, maar nog niet aangezet.\n"
            "Meld je aan bij Ubuntu/GNOME; na eerste installatie zo nodig af- en aanmelden.\n"
            f"Taakbalk aan: gnome-extensions enable {install.UUID}\n"
            "Je huidige Win11Shell, standaardbestandsbeheerder en desktopinstellingen zijn niet gewijzigd.\n")


class Installer(unittest.TestCase):
    maxDiff = None

    @classmethod
    def setUpClass(cls):
        Shared.build()

    # ---- helpers -------------------------------------------------------------------------------------------
    def install(self, case: Case, args=(), locale=None, root=None, build=None, script=None, strace=False, extra=None):
        env = case.env(build or Shared.stubs, locale, extra)
        return case.run(script or SCRIPTS/"install.py", root or Shared.root, list(args), env, strace=strace)

    def assertOk(self, proc):
        self.assertEqual(proc.returncode, 0, f"exit {proc.returncode}\nstdout:\n{proc.stdout}\nstderr:\n{proc.stderr}")

    def assertNoOutsideWrites(self, case: Case, proc):
        count, outside, failed = case.writes_outside(proc.trace)
        summary = {"mutatingSyscallPaths": count, "writesOutside": outside, "failedWritesOutside": failed,
                   "bytes": proc.trace.stat().st_size}
        proc.trace.with_suffix(".summary.json").write_text(json.dumps(summary, indent=1) + "\n")
        if os.environ.get("W11_KEEP_STRACE") != "1":
            proc.trace.unlink()                      # a REAL install traces ~600 MB; the summary stays
        self.assertGreater(count, 50)
        self.assertEqual(outside, [], "writes outside the private case dir")
        self.assertEqual(failed, [], "attempted writes outside the private case dir")
        return count

    def compare_with_baseline(self, name, locale=None, settings=None, args=(), root=None, build=None,
                              strace=False, prepare=None):
        """Run the 3b898e6 installer and the current one in the same fresh case dir; return both results."""
        results = []
        for script in (Shared.old/"install.py", SCRIPTS/"install.py"):
            case = Case(name)
            if settings is not None:
                write_json(case.config/"win11-gnome/settings.json", settings)
                os.chmod(case.config/"win11-gnome", 0o700)
            if prepare:
                prepare(case)
            proc = self.install(case, args, locale, root, build, script=script, strace=strace and script.parent == SCRIPTS)
            results.append((case, proc, same_as_baseline(case.tree()), case.stub_calls()))
        return results

    # ---- byte-for-byte with 3b898e6 ------------------------------------------------------------------------------
    def test_nl_update_identical_to_baseline(self):
        """Robin's case: an existing nl settings.json. Output and every installed byte equal 3b898e6's."""
        (old_case, old, old_tree, _), (case, new, new_tree, calls) = self.compare_with_baseline(
            "nl-update", NL_LOCALE, settings_doc("nl"), strace=True)
        self.assertOk(old)
        self.assertOk(new)
        self.assertEqual(new.stdout, as_now(old.stdout))
        self.assertEqual(new.stderr, old.stderr)
        self.assertEqual(new.stdout, nl_done(case.prefix))
        self.assertEqual(new_tree, old_tree)
        self.assertEqual(calls, [])
        self.assertNoOutsideWrites(case, new)

    def test_nl_first_install_only_adds_banner_and_settings(self):
        (old_case, old, old_tree, _), (case, new, new_tree, _) = self.compare_with_baseline("nl-first", NL_LOCALE)
        self.assertOk(old)
        self.assertOk(new)
        banner = "Taal: Nederlands. In English: choose English in Instellingen › Tijd en taal › Taal en regio.\n"
        self.assertEqual(new.stdout, banner + as_now(old.stdout))
        self.assertEqual(new.stderr, old.stderr)
        extra = {k: v for k, v in new_tree.items() if k not in old_tree}
        self.assertEqual(sorted(extra), ["config/win11-gnome", "config/win11-gnome/.settings.lock",
                                         "config/win11-gnome/settings.json"])
        self.assertEqual({k: v for k, v in new_tree.items() if k in old_tree}, old_tree)
        settings = case.config/"win11-gnome/settings.json"
        self.assertEqual(json.loads(settings.read_text()), settings_doc("nl"))
        self.assertEqual(oct(settings.stat().st_mode & 0o777), "0o600")
        self.assertEqual(oct((case.config/"win11-gnome").stat().st_mode & 0o777), "0o700")
        self.assertEqual({n: e["Name"] for n, e in entries_in(case).items()}, entry_names("nl"))

    def test_update_without_settings_stays_nl_and_writes_nothing(self):
        """H D13: an existing install without a stored choice is nl, also on an English locale; nothing stored."""
        def first(case):
            proc = self.install(case, script=Shared.old/"install.py", locale=EN_LOCALE)
            self.assertOk(proc)
        (old_case, old, old_tree, _), (case, new, new_tree, _) = self.compare_with_baseline(
            "update-no-settings", EN_LOCALE, prepare=first)
        self.assertOk(new)
        self.assertEqual(new.stdout, as_now(old.stdout))
        self.assertEqual(new_tree, old_tree)
        self.assertFalse((case.config/"win11-gnome").exists())

    def test_missing_binary_message_as_baseline(self):
        broken = RUN/"build-missing-one"
        shutil.rmtree(broken, ignore_errors=True)
        shutil.copytree(Shared.stubs, broken)
        (broken/"win11-installer").unlink()
        (_, old, old_tree, _), (case, new, new_tree, _) = self.compare_with_baseline(
            "missing-one", NL_LOCALE, settings_doc("nl"), build=broken)
        self.assertEqual((old.returncode, new.returncode), (1, 1))
        self.assertEqual(new.stderr, old.stderr)
        self.assertEqual(new.stderr, f"Ontbreekt: {broken}/win11-installer. Bouw eerst met scripts/build.sh.\n")
        self.assertEqual(new_tree, old_tree)
        (broken/"win11-panel").unlink()
        (broken/"win11-launch").unlink()
        case = Case("missing-three")
        proc = self.install(case, locale=EN_LOCALE, build=broken, args=["--lang", "en"])
        self.assertEqual(proc.returncode, 1)
        self.assertEqual(proc.stderr.splitlines(), [f"Missing: {broken}/{n}. Build it first with scripts/build.sh."
                                                    for n in install.BINS
                                                    if n in ("win11-panel", "win11-launch", "win11-installer")])

    # ---- languages -----------------------------------------------------------------------------------------------
    def test_lang_en_first_install_is_english_everywhere(self):
        case = Case("en-first")
        proc = self.install(case, ["--lang", "en"], NL_LOCALE, strace=True)
        self.assertOk(proc)
        self.assertEqual(dutch_words(proc.stdout + proc.stderr), [])
        self.assertIn(f"Installed in {case.prefix}/bin\n", proc.stdout)
        self.assertEqual(len(proc.stdout.splitlines()), 6)
        entries = entries_in(case)
        self.assertEqual({n: e["Name"] for n, e in entries.items()}, entry_names("en"))
        self.assertEqual({e["Comment"] for e in entries.values()}, {ENTRY_NAMES["comment"]["en"]})
        self.assertEqual(json.loads((case.config/"win11-gnome/settings.json").read_text()), settings_doc("en"))
        self.assertNoOutsideWrites(case, proc)
        # An update without --lang keeps English: settings.json untouched, English names and messages, no banner.
        before = (case.config/"win11-gnome/settings.json").read_bytes()
        again = self.install(case, locale=NL_LOCALE)
        self.assertOk(again)
        self.assertEqual((case.config/"win11-gnome/settings.json").read_bytes(), before)
        self.assertEqual(again.stdout, proc.stdout)
        self.assertEqual({n: e["Name"] for n, e in entries_in(case).items()}, entry_names("en"))
        # An explicit --lang nl on that install speaks Dutch but keeps the stored English (C6), and says so.
        third = self.install(case, ["--lang", "nl"], NL_LOCALE)
        self.assertOk(third)
        self.assertEqual(third.stdout.splitlines()[0],
                         f"Let op: --lang nl geldt alleen zonder opgeslagen keuze. Win11 blijft Engels "
                         f"({case.config}/win11-gnome/settings.json); wijzig de taal in Settings › Time & language › Language & region.")
        self.assertEqual(third.stdout.splitlines()[1:], nl_done(case.prefix).splitlines())
        self.assertEqual((case.config/"win11-gnome/settings.json").read_bytes(), before)
        self.assertEqual(entries_in(case)["Explorer"]["Name"], "Win11 File Explorer")

    def test_switch_then_reinstall_keeps_english_names(self):
        """plan-critique F19: install (nl), switch to English in Settings, reinstall: the GNOME names stay English."""
        case = Case("switch-reinstall")
        self.assertOk(self.install(case, locale=NL_LOCALE))
        self.assertEqual(entries_in(case)["TaskManager"]["Name"], "Win11 Taakbeheer")
        write_json(case.config/"win11-gnome/settings.json", settings_doc("en"))
        proc = self.install(case, locale=NL_LOCALE)
        self.assertOk(proc)
        self.assertEqual(entries_in(case)["TaskManager"]["Name"], "Win11 Task Manager")
        self.assertEqual(dutch_words(proc.stdout + proc.stderr), [])

    def test_interrupted_first_install_keeps_the_language(self):
        """ge1c-review m1: a first install that fails after its payload is in place (here glib-compile-schemas
        fails) has already stored the guessed language, so the rerun stays English instead of taking the
        update-without-choice path (nl, H D13)."""
        case = Case("interrupted-first")
        failing = case.dir/"stubs/glib-compile-schemas"
        failing.write_text("#!/bin/sh\necho 'GE1c test: glib-compile-schemas fails on purpose' >&2\nexit 1\n")
        failing.chmod(0o755)
        first = self.install(case, locale=EN_LOCALE)
        self.assertEqual(first.returncode, 1, first.stdout + first.stderr)
        self.assertTrue((case.prefix/"share/win11-gnome"/install.MARKER).is_file())       # the payload is in place
        self.assertEqual(first.stdout.splitlines()[0],
                         "Language: English. In het Nederlands: kies Nederlands in Settings › Time & language › Language & region.")
        settings = case.config/"win11-gnome/settings.json"
        self.assertEqual(json.loads(settings.read_text()), settings_doc("en"))
        stored = settings.read_bytes()
        failing.unlink()
        again = self.install(case, locale=EN_LOCALE)
        self.assertOk(again)
        self.assertEqual(settings.read_bytes(), stored)
        self.assertEqual(again.stdout.splitlines()[0], f"Installed in {case.prefix}/bin")      # no second banner
        self.assertEqual(dutch_words(again.stdout + again.stderr), [])
        self.assertEqual({n: e["Name"] for n, e in entries_in(case).items()}, entry_names("en"))
        # A failed check before any change stores nothing: the conflict check refuses, no settings.json appears.
        blocked = Case("interrupted-preflight")
        (blocked.prefix/"bin").mkdir(parents=True)
        (blocked.prefix/"bin/win11-panel").write_text("someone else's\n")
        before = blocked.tree()
        refused = self.install(blocked, locale=EN_LOCALE)
        self.assertEqual(refused.returncode, 1)
        self.assertEqual(refused.stdout, "")
        self.assertEqual(blocked.tree(), before)

    def test_locale_guess_on_first_install(self):
        rows = [
            ({"LANG": "en_GB.UTF-8", "TZ": "Europe/London"}, "en",
             "Language: English. In het Nederlands: kies Nederlands in Settings › Time & language › Language & region."),
            ({"LANG": "en_US.UTF-8", "TZ": "Europe/Amsterdam"}, "nl",
             "Taal: Nederlands, vanwege de tijdzone Europe/Amsterdam. In English: choose English in Instellingen › Tijd en taal › Taal en regio."),
            ({"LANG": "en_US.UTF-8", "LC_TIME": "nl_NL.UTF-8", "TZ": "UTC"}, "nl",
             "Taal: Nederlands, vanwege de Nederlandse notatie in LC_TIME. In English: choose English in Instellingen › Tijd en taal › Taal en regio."),
            ({"LANG": "C.UTF-8"}, "nl", "Taal: Nederlands. In English: choose English in Instellingen › Tijd en taal › Taal en regio."),
            ({"LANG": "fr_FR.UTF-8", "TZ": "Europe/Paris"}, "en",
             "Language: English. In het Nederlands: kies Nederlands in Settings › Time & language › Language & region."),
        ]
        for index, (locale, lang, banner) in enumerate(rows):
            with self.subTest(locale=locale):
                case = Case(f"guess-{index}")
                proc = self.install(case, locale=locale)
                self.assertOk(proc)
                self.assertEqual(proc.stdout.splitlines()[0], banner)
                self.assertEqual(json.loads((case.config/"win11-gnome/settings.json").read_text())["language"], lang)
                self.assertEqual(entries_in(case)["Settings"]["Name"], ENTRY_NAMES["entries"]["Settings"][lang])
                body = "\n".join(proc.stdout.splitlines()[1:]) + proc.stderr
                if lang == "en":
                    self.assertEqual(dutch_words(body), [])
                else:
                    self.assertEqual(body + "\n", nl_done(case.prefix))

    def test_hyprland_language_moves_over_once(self):
        h_doc = {"language": "en", "theme": {"mode": "light"}, "taskbar": {"alignment": "left"}}
        case = Case("hyprland-en")
        write_json(case.config/"win11shell/settings.json", h_doc)
        h_before = (case.config/"win11shell/settings.json").read_bytes()
        proc = self.install(case, locale=NL_LOCALE)
        self.assertOk(proc)
        self.assertEqual(proc.stdout.splitlines()[0],
                         f"Language: English, taken over from Win11 Shell for Hyprland ({case.config}/win11shell/settings.json).")
        self.assertEqual(json.loads((case.config/"win11-gnome/settings.json").read_text()), settings_doc("en"))
        self.assertEqual((case.config/"win11shell/settings.json").read_bytes(), h_before)
        self.assertEqual(entries_in(case)["Panel"]["Name"], "Win11 Taskbar")
        # A second install no longer looks at H's file: the new settings.json decides.
        write_json(case.config/"win11shell/settings.json", {"language": "nl"})
        again = self.install(case, locale=NL_LOCALE)
        self.assertOk(again)
        self.assertEqual(json.loads((case.config/"win11-gnome/settings.json").read_text())["language"], "en")
        for name, prepare, banner in [
            ("hyprland-nolang", lambda c: write_json(c.config/"win11shell/settings.json", {"theme": {"mode": "dark"}}),
             "Taal: Nederlands, overgenomen van Win11 Shell voor Hyprland ({config}/win11shell/settings.json)."),
            ("hyprland-payload", lambda c: (c.data/"win11shell/ui").mkdir(parents=True),
             "Taal: Nederlands, overgenomen van Win11 Shell voor Hyprland ({data}/win11shell)."),
        ]:
            with self.subTest(name):
                case = Case(name)
                prepare(case)
                proc = self.install(case, locale=EN_LOCALE)
                self.assertOk(proc)
                self.assertEqual(proc.stdout.splitlines()[0], banner.format(config=case.config, data=case.data))
                self.assertEqual(json.loads((case.config/"win11-gnome/settings.json").read_text())["language"], "nl")
        # An explicit --lang beats H's choice on a first install.
        case = Case("hyprland-arg")
        write_json(case.config/"win11shell/settings.json", h_doc)
        proc = self.install(case, ["--lang=NL"], EN_LOCALE)
        self.assertOk(proc)
        self.assertEqual(proc.stdout, nl_done(case.prefix))
        self.assertEqual(json.loads((case.config/"win11-gnome/settings.json").read_text())["language"], "nl")

    def test_settings_language_values(self):
        """C1: only exactly "en" is English; the object form, "EN", other strings, broken JSON mean nl."""
        for index, (raw, lang) in enumerate([
            (json.dumps(settings_doc("en")), "en"), (json.dumps(settings_doc({"custom": "future"})), "nl"),
            (json.dumps(settings_doc("EN")), "nl"), (json.dumps(settings_doc("fr")), "nl"),
            (json.dumps({"version": 1}), "nl"), ("{broken", "nl"), ("[]", "nl"),
        ]):
            with self.subTest(raw=raw[:40]):
                case = Case(f"value-{index}")
                (case.config/"win11-gnome").mkdir(mode=0o700)
                (case.config/"win11-gnome/settings.json").write_text(raw)
                proc = self.install(case, locale=EN_LOCALE if lang == "nl" else NL_LOCALE)
                self.assertOk(proc)
                self.assertEqual((case.config/"win11-gnome/settings.json").read_text(), raw)
                self.assertEqual(entries_in(case)["Explorer"]["Name"], ENTRY_NAMES["entries"]["Explorer"][lang])

    def test_arguments(self):
        case = Case("arguments")
        before = case.tree()
        bad = self.install(case, ["--lang", "fr"])
        self.assertEqual(bad.returncode, 2)
        self.assertEqual(bad.stderr, "Onbekende taal ‘fr’: kies nl of en.\nUnknown language ‘fr’: choose nl or en.\n")
        self.assertEqual(self.install(case, ["--lang"]).returncode, 2)
        help_nl = self.install(case, ["--help"])
        self.assertEqual((help_nl.returncode, help_nl.stdout.splitlines()[0]), (0, "Gebruik: scripts/install.sh [--lang nl|en] [--no-session]"))
        help_en = self.install(case, ["--help", "--lang", "en"])
        self.assertEqual((help_en.returncode, help_en.stdout.splitlines()[0]), (0, "Usage: scripts/install.sh [--lang nl|en] [--no-session]"))
        self.assertEqual(dutch_words(help_en.stdout), [])
        unknown = self.install(case, ["--prefix", "/x"])
        self.assertEqual(unknown.returncode, 2)
        self.assertTrue(unknown.stderr.startswith("Onbekende optie: --prefix\nGebruik:"))
        self.assertEqual(case.tree(), before)

    def test_install_sh_passes_arguments(self):
        text = (SCRIPTS/"install.sh").read_text()
        self.assertIn('exec python3 "$ROOT/scripts/install.py" "$ROOT" "$@"', text)
        self.assertIn('"$ROOT/scripts/package.py" "$ROOT" "$@"', (SCRIPTS/"package.sh").read_text())

    def test_config_folder_must_be_own(self):
        case = Case("config-link")
        (case.dir/"elsewhere").mkdir()
        (case.config/"win11-gnome").symlink_to(case.dir/"elsewhere", target_is_directory=True)
        before = case.tree()
        proc = self.install(case, locale=NL_LOCALE)
        self.assertEqual(proc.returncode, 1)
        self.assertEqual(proc.stderr, f"Niet overschreven: {case.config}/win11-gnome moet een gewone map van deze gebruiker zijn.\n")
        self.assertEqual(case.tree(), before)

    # ---- files ---------------------------------------------------------------------------------------------------
    def test_extension_file_set_is_the_directory_listing(self):
        """C7 / plan-critique F18: a new extension module is installed; a removed one disappears on reinstall."""
        root = sandbox.fixture_root(RUN/"fixture-extra", {"ge1cExtra.js": "export const GE1C = 1;\n"})
        case = Case("extension-set")
        self.assertOk(self.install(case, root=root, locale=NL_LOCALE))
        target = case.data/"gnome-shell/extensions"/install.UUID
        source = {p.name for p in (root/"extension").iterdir() if p.is_file()}
        self.assertIn("ge1cExtra.js", source)
        self.assertEqual({p.name for p in target.iterdir() if p.is_file()}, source)
        self.assertEqual({p.name for p in target.iterdir() if p.is_dir()}, {"schemas"})
        for name in source:
            self.assertEqual((target/name).read_bytes(), (root/"extension"/name).read_bytes(), name)
        self.assertTrue((target/"schemas/gschemas.compiled").is_file())
        (root/"extension/ge1cExtra.js").unlink()
        (target/"ge1c notes.txt").write_text("not ours\n")              # a file nobody shipped (ge1c-review m3)
        again = self.install(case, root=root, locale=NL_LOCALE)
        self.assertOk(again)
        self.assertEqual({p.name for p in target.iterdir() if p.is_file()}, source - {"ge1cExtra.js"})
        # Each removed file is named, in Win11's language, before the closing lines (nothing goes without a trace).
        self.assertEqual(again.stdout, f"Verwijderd uit de extensiemap (geen bestand van deze versie): {target}/ge1c notes.txt\n"
                                       f"Verwijderd uit de extensiemap (geen bestand van deze versie): {target}/ge1cExtra.js\n"
                         + nl_done(case.prefix))
        write_json(case.config/"win11-gnome/settings.json", settings_doc("en"))
        (target/"old.js").write_text("// stale\n")
        english = self.install(case, root=root, locale=NL_LOCALE)
        self.assertOk(english)
        self.assertEqual(english.stdout.splitlines()[0],
                         f"Removed from the extension folder (not a file of this version): {target}/old.js")

    def test_entries_have_icons_the_stock_theme_finds(self):
        """O2: with the bundled Win11 icons in the payload every entry's Icon= is a file there (or Adwaita's name for
        the Panel); a bundle without those files keeps the theme names 3b898e6 wrote."""
        svg = "<svg xmlns='http://www.w3.org/2000/svg'/>\n"
        icons = {rel: svg for rel in [*install.BUNDLED_ICONS.values(), "apps/scalable/office-calendar.svg"]}
        root = sandbox.fixture_root(RUN/"fixture-entry-icons", icons=icons)
        case = Case("entry-icons")
        self.assertOk(self.install(case, root=root, locale=NL_LOCALE))
        check_entry_icons(self, case)
        entries = entries_in(case)
        for name, rel in install.BUNDLED_ICONS.items():
            self.assertEqual(entries[name]["Icon"], str(case.prefix/"share/win11-gnome/icons/Win11"/rel))
        self.assertEqual(entries["Calendar"]["Icon"], str(case.prefix/"share/win11-gnome/icons/Win11/apps/scalable/office-calendar.svg"))
        case = Case("entry-icons-fallback")
        self.assertOk(self.install(case, locale=NL_LOCALE))
        self.assertEqual({name: entries_in(case)[name]["Icon"] for name in install.BUNDLED_ICONS},
                         {name: Path(rel).stem for name, rel in install.BUNDLED_ICONS.items()})

    def test_build_directory_choice(self):
        """WIN11_BUILD_DIR unset: target/release wins over a stale bin/; a package (bin/ only) uses bin/; a directory
        that does not itself hold all 19 binaries is never the choice, WIN11_NATIVE_BUILD_DIR or not."""
        base = RUN/"build-choice"
        shutil.rmtree(base, ignore_errors=True)
        root = sandbox.fixture_root(base/"root")
        sandbox.stub_build(root/"target/release")
        (root/"bin").mkdir()
        for name in install.BINS[:6]:
            (root/"bin"/name).write_text("#!/bin/sh\n# stale\nexit 97\n")
            (root/"bin"/name).chmod(0o755)
        case = Case("build-choice-release")
        self.assertOk(self.install(case, root=root, locale=NL_LOCALE, extra={"WIN11_BUILD_DIR": None}))
        for name in install.BINS:
            self.assertEqual((case.prefix/"bin"/name).read_bytes(), (root/"target/release"/name).read_bytes())
        shutil.rmtree(root/"target")
        case = Case("build-choice-stale")
        proc = self.install(case, root=root, locale=NL_LOCALE, extra={"WIN11_BUILD_DIR": None})
        self.assertEqual(proc.returncode, 1)
        self.assertEqual(len(proc.stderr.splitlines()), len(install.BINS) - 6)
        self.assertTrue(all(line.startswith(f"Ontbreekt: {root}/bin/") for line in proc.stderr.splitlines()))
        # A bin/ with the host's binaries only, the native ones in WIN11_NATIVE_BUILD_DIR: refused, each native one
        # named as missing in bin/ (an old checkout's bin/ is never installed from).
        for name in install.BINS:
            if name not in install.NATIVE_BINS:
                (root/"bin"/name).write_text("#!/bin/sh\n# host only\nexit 97\n")
                (root/"bin"/name).chmod(0o755)
        native = sandbox.stub_build(base/"native")
        case = Case("build-choice-native-dir")
        proc = self.install(case, root=root, locale=NL_LOCALE,
                            extra={"WIN11_BUILD_DIR": None, "WIN11_NATIVE_BUILD_DIR": str(native)})
        self.assertEqual(proc.returncode, 1)
        self.assertEqual(proc.stderr.splitlines(), [f"Ontbreekt: {root}/bin/{name}. Bouw eerst met scripts/build.sh."
                                                    for name in install.BINS if name in install.NATIVE_BINS])
        self.assertEqual(sorted(p.name for p in case.prefix.iterdir()), [])
        sandbox.stub_build(root/"bin")
        case = Case("build-choice-package")
        self.assertOk(self.install(case, root=root, locale=NL_LOCALE, extra={"WIN11_BUILD_DIR": None}))

    def test_uninstall_removes_only_own_files_and_speaks_the_language(self):
        case = Case("uninstall")
        self.assertOk(self.install(case, ["--lang", "en"], NL_LOCALE))
        settings = (case.config/"win11-gnome/settings.json").read_bytes()
        env = case.env(Shared.stubs, NL_LOCALE)
        usage = case.run(SCRIPTS/"uninstall.py", None, ["--bogus"], env)
        self.assertEqual((usage.returncode, usage.stderr), (1, "Usage: scripts/uninstall.sh [--files-only] [--lang nl|en]\n"))
        extension = case.data/"gnome-shell/extensions"/install.UUID
        (extension/"ge1c-unknown.js").write_text("// nobody shipped this\n")    # ge1c-review m3: named, not silent
        proc = case.run(SCRIPTS/"uninstall.py", None, ["--files-only"], env, strace=True)
        self.assertOk(proc)
        self.assertEqual(proc.stdout, f"Removed from the extension folder (not a file of this version): {extension}/ge1c-unknown.js\n"
                                      "Win11 for GNOME removed. Your settings and the original Win11Shell were kept.\n")
        self.assertNoOutsideWrites(case, proc)
        self.assertEqual(case.stub_calls(), [])
        self.assertFalse((case.prefix/"share/win11-gnome").exists())
        self.assertFalse((case.data/"gnome-shell/extensions"/install.UUID).exists())
        self.assertEqual(sorted(p.name for p in (case.prefix/"bin").iterdir()), [])
        self.assertEqual(sorted(p.name for p in (case.data/"applications").glob("io.github.win11shell.*")), [])
        self.assertEqual((case.config/"win11-gnome/settings.json").read_bytes(), settings)
        again = case.run(SCRIPTS/"uninstall.py", None, ["--lang", "nl"], env)
        self.assertEqual(again.returncode, 1)
        self.assertEqual(again.stderr, f"Niets verwijderd: {case.prefix}/share/win11-gnome bevat geen installatiemarker van dit project.\n")

    def test_calendar_events_survive_an_update_and_an_uninstall(self):
        """13-11: with the default prefix the Calendar's old folder (win11-gnome/calendar) is the payload; an update and
        an uninstall move it to win11-gnome-calendar first (install.rescue_calendar)."""
        case = Case("calendar-data")
        share = {"XDG_DATA_HOME": str(case.prefix/"share")}
        events = '{"version": 1, "events": []}\n'
        old, new = case.prefix/"share/win11-gnome/calendar", case.prefix/"share/win11-gnome-calendar"
        self.assertOk(self.install(case, ["--lang", "en"], NL_LOCALE, extra=share))
        old.mkdir()
        (old/"events.json").write_text(events)
        self.assertOk(self.install(case, ["--lang", "en"], NL_LOCALE, extra=share))
        self.assertEqual((new/"events.json").read_text(), events)
        self.assertFalse(old.exists())
        shutil.rmtree(new)
        old.mkdir()
        (old/"events.json").write_text(events)
        proc = case.run(SCRIPTS/"uninstall.py", None, ["--files-only"], case.env(Shared.stubs, NL_LOCALE, share))
        self.assertOk(proc)
        self.assertFalse((case.prefix/"share/win11-gnome").exists())
        self.assertEqual((new/"events.json").read_text(), events)

    def test_payload_has_only_the_ui_assets(self):
        """The payload's ui/ is vendor/ui/assets alone, also while vendor/ui still holds the HTML apps' files, and it
        has no host/: vendor/host, the Python reference, stays in the source tree. Without vendor/ui/assets the
        installer refuses before any change."""
        root = sandbox.fixture_root(RUN/"fixture-html")
        (root/"vendor/ui/index.html").write_text("<!doctype html><title>GE1c fixture</title>\n")
        (root/"vendor/ui/js").mkdir()
        (root/"vendor/ui/js/main.js").write_text("// GE1c fixture\n")
        case = Case("payload-assets")
        self.assertOk(self.install(case, root=root, locale=NL_LOCALE))
        payload = case.prefix/"share/win11-gnome"
        self.assertEqual(sorted(p.name for p in (payload/"ui").iterdir()), ["assets"])
        self.assertEqual(sorted(p.relative_to(payload/"ui/assets") for p in (payload/"ui/assets").rglob("*")),
                         sorted(p.relative_to(root/"vendor/ui/assets") for p in (root/"vendor/ui/assets").rglob("*")))
        self.assertFalse((payload/"host").exists())
        self.assertTrue((root/"vendor/host").is_dir())
        shutil.rmtree(root/"vendor/ui/assets")
        case = Case("payload-no-assets")
        before = case.tree()
        proc = self.install(case, root=root, locale=NL_LOCALE)
        self.assertEqual((proc.returncode, proc.stdout), (1, ""))
        self.assertEqual(proc.stderr, f"Ontbrekende interfacebron: {root}/vendor/ui/assets\n")
        self.assertEqual(case.tree(), before)

    def test_webkit_state_goes_on_update_and_uninstall(self):
        """An update (install.py over its own payload) and uninstall.py, with and without --files-only, remove the
        WebKit apps' storage by install.remove_webkit_state's exact list and name each removed path. Everything else
        stays: Win11 Shell for Hyprland's win11shell* folders, the w11gpuprobe-* probes, webkit.py, a listed name that
        holds something else or is a link, the Calendar's folder, the rest of the cache, and the window-state files
        the native Explorer reads. A first install leaves them all."""
        gone = ["data/io.github.win11shell.Explorer", "data/io.github.win11shell.TaskManager",
                "cache/io.github.win11shell.Explorer", "cache/io.github.win11shell.Desktop",
                "data/org.freedesktop.impl.portal.desktop.win11shell",
                "cache/org.freedesktop.impl.portal.desktop.win11shell", "cache/win11-explorer",
                "cache/win11-gnome/WebKitCache", "cache/win11-gnome/CacheStorage",
                "tmp/win11-gnome/explorer-restart.lock"]
        kept = ["data/win11shell-explorer", "cache/win11shell-settings", "data/w11gpuprobe-b-desk",
                "cache/w11gpuprobe-L1-shell-shm-cpu", "data/webkit.py", "data/io.github.win11shell.Settings",
                "cache/io.github.win11shell.Settings", "elsewhere/WebKitCache", "data/io.github.win11shell.Calendar",
                "cache/win11-gnome/host.log", "cache/win11-gnome/index", "config/win11-gnome/window-state.json",
                "config/win11-gnome/.window-state.lock"]

        def seed(case: Case) -> None:
            webkit_data = {"hsts-storage.sqlite": "", "localstorage/http_127.0.0.1_1.localstorage": "",
                           "storage/salt": "", "mediakeys/v1/x": ""}
            webkit_cache = {"WebKitCache/Version 17/salt": "", "CacheStorage/salt": ""}
            layout = {"data/io.github.win11shell.Explorer": webkit_data, "data/io.github.win11shell.TaskManager": {},
                      "cache/io.github.win11shell.Explorer": webkit_cache,
                      "cache/io.github.win11shell.Desktop": {"WebKitCache/Version 17/salt": ""},
                      "data/org.freedesktop.impl.portal.desktop.win11shell": webkit_data,
                      "cache/org.freedesktop.impl.portal.desktop.win11shell": webkit_cache,
                      "cache/win11-explorer": webkit_cache, "cache/win11-gnome": {**webkit_cache, "host.log": "x\n",
                                                                                   "index/db": "x"},
                      "data/win11shell-explorer": webkit_data, "cache/win11shell-settings": webkit_cache,
                      "data/w11gpuprobe-b-desk": webkit_data, "cache/w11gpuprobe-L1-shell-shm-cpu": webkit_cache,
                      "data/webkit.py": webkit_data, "data/io.github.win11shell.Settings": {**webkit_data, "notes.txt": "x"},
                      "elsewhere": webkit_cache, "data/io.github.win11shell.Calendar": {"localstorage/x": ""},
                      "config/win11-gnome": {"window-state.json": "{}\n", ".window-state.lock": ""},
                      "tmp/win11-gnome": {"explorer-restart.lock": ""}}
            for folder, files in layout.items():
                (case.dir/folder).mkdir(parents=True, exist_ok=True)
                for rel, text in files.items():
                    (case.dir/folder/rel).parent.mkdir(parents=True, exist_ok=True)
                    (case.dir/folder/rel).write_text(text)
            if not (case.dir/"cache/io.github.win11shell.Settings").is_symlink():
                (case.dir/"cache/io.github.win11shell.Settings").symlink_to(case.dir/"elsewhere", target_is_directory=True)

        def removed_lines(case: Case) -> str:
            return "".join(install.t("webkitRemoved", "nl", path=case.dir/rel) + "\n" for rel in gone)

        def check(case: Case, what: str) -> None:
            self.assertEqual([rel for rel in gone if os.path.lexists(case.dir/rel)], [], what)
            self.assertEqual([rel for rel in kept if not os.path.lexists(case.dir/rel)], [], what)
            self.assertEqual(sorted(p.name for p in (case.dir/"data/webkit.py").iterdir()),
                             ["hsts-storage.sqlite", "localstorage", "mediakeys", "storage"], what)
            self.assertTrue((case.dir/"cache/io.github.win11shell.Settings").is_symlink(), what)

        case = Case("webkit-state")
        write_json(case.config/"win11-gnome/settings.json", settings_doc("nl"))
        seed(case)
        first = self.install(case, locale=NL_LOCALE)
        self.assertOk(first)
        self.assertEqual(first.stdout, nl_done(case.prefix))
        self.assertEqual([rel for rel in gone if not os.path.lexists(case.dir/rel)], [])
        update = self.install(case, locale=NL_LOCALE, strace=True)
        self.assertOk(update)
        self.assertEqual(update.stdout, removed_lines(case) + nl_done(case.prefix))
        check(case, "update")
        self.assertNoOutsideWrites(case, update)
        again = self.install(case, locale=NL_LOCALE)
        self.assertEqual(again.stdout, nl_done(case.prefix))
        for args in (["--files-only"], []):
            with self.subTest(args=args):
                self.assertOk(self.install(case, locale=NL_LOCALE))
                seed(case)
                proc = case.run(SCRIPTS/"uninstall.py", None, args, case.env(Shared.stubs, NL_LOCALE))
                self.assertOk(proc)
                self.assertEqual(proc.stdout, removed_lines(case)
                                 + "Win11 voor GNOME verwijderd. Je instellingen en oorspronkelijke Win11Shell zijn behouden.\n")
                check(case, f"uninstall {args}")

    def test_sandbox_negative_control(self):
        """The sandbox itself: the repo is read-only, the live config is invisible, and strace flags an outside write."""
        case = Case("sandbox-probe")
        probe = case.dir/"probe.py"
        probe.write_text(
            "import os, sys\n"
            f"live = {str(sandbox.REAL_HOME/'.config/win11-gnome')!r}\n"
            "print('live-config-visible', os.path.exists(live))\n"
            f"repo = {str(G/'scripts/.ge1c-probe')!r}\n"
            "try:\n    open(repo, 'w').write('x')\n    print('repo-write ok')\nexcept OSError as e:\n    print('repo-write', e.errno)\n"
            f"home = {str(sandbox.REAL_HOME/'.ge1c-probe')!r}\n"
            "open(home, 'w').write('x')\nprint('home-write tmpfs')\n")
        proc = case.run(probe, None, [], case.env(Shared.stubs), strace=True)
        self.assertOk(proc)
        self.assertEqual(proc.stdout.splitlines(), ["live-config-visible False", "repo-write 30", "home-write tmpfs"])
        _count, outside, failed = case.writes_outside(proc.trace)
        self.assertIn(f"openat {sandbox.REAL_HOME/'.ge1c-probe'}", outside)
        self.assertIn(f"openat {G/'scripts/.ge1c-probe'}", failed)
        self.assertFalse((sandbox.REAL_HOME/".ge1c-probe").exists())
        self.assertFalse((G/"scripts/.ge1c-probe").exists())

    def test_uninstall_nl_identical_to_baseline(self):
        """uninstall.py --files-only in nl: output and the remaining tree equal 3b898e6's uninstall.py."""
        results = []
        for script in (Shared.old/"uninstall.py", SCRIPTS/"uninstall.py"):
            case = Case("uninstall-nl")
            write_json(case.config/"win11-gnome/settings.json", settings_doc("nl"))
            self.assertOk(self.install(case, locale=NL_LOCALE))
            env = case.env(Shared.stubs, NL_LOCALE)
            proc = case.run(script, None, ["--files-only"], env)
            empty = Case("uninstall-nl-empty")
            nothing = empty.run(script, None, [], empty.env(Shared.stubs, NL_LOCALE))
            results.append((proc, same_as_baseline(case.tree()), nothing, case.stub_calls()))
        (old, old_tree, old_nothing, _), (new, new_tree, new_nothing, calls) = results
        self.assertOk(new)
        self.assertEqual((new.stdout, new.stderr), (old.stdout, old.stderr))
        self.assertEqual(new.stdout, "Win11 voor GNOME verwijderd. Je instellingen en oorspronkelijke Win11Shell zijn behouden.\n")
        self.assertEqual(new_tree, old_tree)
        self.assertEqual((new_nothing.returncode, new_nothing.stderr), (old_nothing.returncode, old_nothing.stderr))
        self.assertEqual(calls, [])

    # ---- the real tree -------------------------------------------------------------------------------------------
    @unittest.skipIf(os.environ.get("W11_INSTALLER_REAL") == "0", "W11_INSTALLER_REAL=0")
    def test_real_tree_nl_update_identical_and_contained(self):
        """The real vendor/ and the baseline binaries (BINS of 3b898e6), a path with spaces, % and $, strace on the
        new run."""
        root = real_snapshot()
        build = real_build(self)
        (old_case, old, old_tree, _), (case, new, new_tree, calls) = self.compare_with_baseline(
            f"real-nl {NASTY}", NL_LOCALE, settings_doc("nl"), root=root, build=build, strace=True)
        self.addCleanup(drop_payload, case)
        self.assertOk(old)
        self.assertOk(new)
        self.assertEqual((new.stdout, new.stderr), (as_now(old.stdout), old.stderr))
        self.assertEqual(without_moved_icons(new_tree), without_moved_icons(old_tree))
        check_moved_icons(self, case, old_tree, new_tree)
        check_entry_icons(self, case)
        self.assertEqual(calls, [])
        writes = self.assertNoOutsideWrites(case, new)
        print(f"\n  real nl update: {writes} mutating syscall paths, 0 outside the case dir", file=sys.stderr)
        for name in install.BINS:
            self.assertEqual((case.prefix/"bin"/name).read_bytes(), (build/name).read_bytes())
        check_exec_lines(self, case)

    @unittest.skipIf(os.environ.get("W11_INSTALLER_REAL") == "0", "W11_INSTALLER_REAL=0")
    def test_real_tree_en_first_install(self):
        root = real_snapshot()
        case = Case(f"real-en {NASTY}")
        self.addCleanup(drop_payload, case)
        proc = self.install(case, ["--lang", "en"], NL_LOCALE, root=root, build=real_build(self), strace=True)
        self.assertOk(proc)
        self.assertEqual(dutch_words(proc.stdout + proc.stderr), [])
        writes = self.assertNoOutsideWrites(case, proc)
        print(f"\n  real en first install: {writes} mutating syscall paths, 0 outside the case dir", file=sys.stderr)
        self.assertEqual({n: e["Name"] for n, e in entries_in(case).items()}, entry_names("en"))
        check_exec_lines(self, case)
        check_entry_icons(self, case)


class Autostart(unittest.TestCase):
    """GW7 (design §6.6): install.py writes $XDG_CONFIG_HOME/autostart/io.github.win11shell.PostUpgradeCheck.desktop
    (Name in Win11's language, NoDisplay, OnlyShowIn=GNOME;, no X-GNOME-Autostart-Delay) and uninstall.py removes it;
    against 3b898e6's installer those are the only new paths."""
    maxDiff = None

    @classmethod
    def setUpClass(cls):
        Shared.build()

    def install(self, case: Case, args=(), locale=None, script=None, extra=None):
        return case.run(script or SCRIPTS/"install.py", Shared.root, list(args), case.env(Shared.stubs, locale, extra))

    def entry(self, case: Case) -> Path:
        return case.dir/sandbox.AUTOSTART

    def expected(self, case: Case, name: str) -> str:
        return ("[Desktop Entry]\nType=Application\n"
                f"Name={name}\n"
                f"Exec=/usr/bin/env -- {install.desktop_exec(case.prefix/'bin/win11-launch')} --post-upgrade-check\n"
                "OnlyShowIn=GNOME;\nNoDisplay=true\n")

    def assertOk(self, proc):
        self.assertEqual(proc.returncode, 0, f"exit {proc.returncode}\nstdout:\n{proc.stdout}\nstderr:\n{proc.stderr}")

    def test_first_install_writes_it_in_win11s_language_and_nothing_else_changes(self):
        # nl: against 3b898e6's installer in the same fresh case dir, the only new paths besides the stored settings
        trees = []
        for script in (Shared.old/"install.py", SCRIPTS/"install.py"):
            case = Case("autostart-nl")
            proc = self.install(case, (), NL_LOCALE, script=script)
            self.assertOk(proc)
            trees.append((case, proc, same_as_baseline(case.tree(autostart=True))))
        (_, old, old_tree), (case, new, new_tree) = trees
        self.assertEqual(self.entry(case).read_text(), self.expected(case, "Win11-controle na een Ubuntu-upgrade"))
        extra = sorted(k for k in new_tree if k not in old_tree and not k.startswith("config/win11-gnome"))
        self.assertEqual(extra, [sandbox.AUTOSTART_DIR, sandbox.AUTOSTART])
        self.assertEqual({k: v for k, v in new_tree.items() if k in old_tree}, old_tree)
        self.assertEqual(case.stub_calls(), [])
        # en (--lang en): the English name; the entry has no autostart delay
        case = Case("autostart-en")
        self.assertOk(self.install(case, ("--lang", "en"), NL_LOCALE))
        self.assertEqual(self.entry(case).read_text(), self.expected(case, "Win11 check after an Ubuntu upgrade"))
        self.assertNotIn("X-GNOME-Autostart-Delay", self.entry(case).read_text())
        self.assertEqual(case.stub_calls(), [])

    def test_update_keeps_the_stored_language_and_a_nasty_prefix_round_trips(self):
        case = Case(f"autostart-update {NASTY}")
        write_json(case.config/"win11-gnome/settings.json", settings_doc("en"))
        os.chmod(case.config/"win11-gnome", 0o700)
        self.assertOk(self.install(case, (), NL_LOCALE))
        self.assertEqual(self.entry(case).read_text(), self.expected(case, "Win11 check after an Ubuntu upgrade"))
        # a reinstall in nl messages keeps the stored English (C6) and rewrites the same bytes
        before = self.entry(case).read_bytes()
        self.assertOk(self.install(case, ("--lang", "nl"), NL_LOCALE))
        self.assertEqual(self.entry(case).read_bytes(), before)
        # Exec= gives back the exact program path and the flag (desktop-entry unescaping, then GLib's argv rules)
        import gi
        gi.require_version("GLib", "2.0")
        from gi.repository import GLib
        keyfile = GLib.KeyFile()
        keyfile.load_from_file(str(self.entry(case)), GLib.KeyFileFlags.NONE)
        ok, argv = GLib.shell_parse_argv(keyfile.get_string("Desktop Entry", "Exec"))
        self.assertTrue(ok)
        self.assertEqual([argv[0], argv[1], argv[2].replace("%%", "%"), argv[3]],
                         ["/usr/bin/env", "--", str(case.prefix/"bin/win11-launch"), "--post-upgrade-check"])
        self.assertEqual(keyfile.get_string("Desktop Entry", "OnlyShowIn"), "GNOME;")
        self.assertTrue(keyfile.get_boolean("Desktop Entry", "NoDisplay"))
        self.assertEqual(keyfile.get_string("Desktop Entry", "Type"), "Application")

    def test_uninstall_removes_only_this_entry(self):
        for args in (["--files-only"], []):
            case = Case("autostart-uninstall")
            self.assertOk(self.install(case, ("--lang", "en"), NL_LOCALE))
            other = case.config/"autostart/someone-else.desktop"
            other.write_text("[Desktop Entry]\nType=Application\nName=Other\nExec=true\n")
            self.assertTrue(self.entry(case).is_file())
            proc = case.run(SCRIPTS/"uninstall.py", None, args, case.env(Shared.stubs, NL_LOCALE))
            self.assertOk(proc)
            self.assertFalse(self.entry(case).exists(), args)
            self.assertTrue(other.is_file(), "another autostart entry stays")
            # gnome-extensions is a stub here: without --files-only it is called (and fails) as before
            self.assertEqual(case.stub_calls()[:1] if args == [] else case.stub_calls(),
                             [f"gnome-extensions\tdisable {install.UUID}"] if args == [] else [])
        # 3b898e6's uninstall.py leaves it (the old installer never wrote it): only the new one removes it
        case = Case("autostart-uninstall-old")
        self.assertOk(self.install(case, (), NL_LOCALE))
        proc = case.run(Shared.old/"uninstall.py", None, ["--files-only"], case.env(Shared.stubs, NL_LOCALE))
        self.assertOk(proc)
        self.assertTrue(self.entry(case).is_file())

    def test_a_foreign_entry_blocks_a_first_install(self):
        case = Case("autostart-conflict")
        (case.config/"autostart").mkdir(parents=True)
        self.entry(case).write_text("[Desktop Entry]\nName=not ours\n")
        proc = self.install(case, ("--lang", "en"), NL_LOCALE)
        self.assertEqual(proc.returncode, 1)
        self.assertIn(str(self.entry(case)), proc.stderr)
        self.assertEqual(self.entry(case).read_text(), "[Desktop Entry]\nName=not ours\n")


class Remnants(unittest.TestCase):
    """tools/h-remnants.sh: lists H's leftovers in a fake HOME and changes nothing at all."""
    maxDiff = None

    @classmethod
    def setUpClass(cls):
        Shared.build()

    def layout(self, case: Case) -> Path:
        home, config, data = case.dir/"home", case.config, case.data
        (data/"win11shell/ui/js").mkdir(parents=True)
        (data/"win11shell/ui/js/main.js").write_text("// H\n")
        (home/".local/bin").mkdir(parents=True)
        for name in ("win11shell", "win11shell-ctl", "win11shell-session"):
            (home/".local/bin"/name).symlink_to(data/"win11shell/bin"/name)
        write_json(config/"win11shell/settings.json", {"language": "en"})
        (config/"win11shell/installer").mkdir()
        (config/"win11shell/installer/themes").write_text("Win11\tx abc icons\nFluent-Dark\tx def themes\n")
        (data/"icons/Win11").mkdir(parents=True)
        (home/".themes/Fluent-Dark").mkdir(parents=True)
        (data/"applications").mkdir(parents=True)
        for name in ("explorer", "settings"):
            (data/"applications"/f"win11shell-{name}.desktop").write_text(f"[Desktop Entry]\nName=Win11 {name}\n")
        (config/"mimeapps.list").write_text("[Default Applications]\ninode/directory=win11shell-explorer.desktop\n")
        (config/"systemd/user/hypridle.service.d").mkdir(parents=True)
        (config/"systemd/user/hypridle.service.d/win11shell-not-in-gnome.conf").write_text("# Win11 Shell (install.sh)\n")
        (config/"systemd/user/win11shell-session.target").symlink_to(data/"win11shell/config/systemd/win11shell-session.target")
        (config/"gtk-4.0").mkdir()
        (config/"gtk-4.0/gtk.css").write_text("/* Win11 Shell: Fluent */\n@import 'x';\n/* end Win11 Shell */\n")
        sysroot = case.dir/"sysroot"
        (sysroot/"usr/share/wayland-sessions").mkdir(parents=True)
        (sysroot/"usr/share/wayland-sessions/win11shell.desktop").write_text("[Desktop Entry]\n")
        return sysroot

    def run_tool(self, case: Case, args=(), sysroot=None, extra=None):
        env = case.env(Shared.stubs, {"LANG": "en_US.UTF-8"},
                       {"W11_REMNANTS_SYSROOT": str(sysroot or case.dir/"sysroot"), **(extra or {})})
        return case.run(G/"tools/h-remnants.sh", None, list(args), env, strace=True)

    def assertReadOnly(self, case: Case, proc):
        mutated = []
        for line in proc.trace.read_text(errors="replace").splitlines():
            parsed = sandbox.strace_paths(line)
            if parsed:
                mutated += [p for p in parsed[1] if not p.startswith(("/dev/null", "/dev/tty"))]
        self.assertEqual(mutated, [])

    def test_lists_everything_and_changes_nothing(self):
        case = Case("remnants")
        sysroot = self.layout(case)
        before = case.tree()
        nl = self.run_tool(case)
        self.assertEqual(nl.returncode, 1, nl.stderr)
        self.assertReadOnly(case, nl)
        self.assertEqual(case.tree(), before)
        text = nl.stdout
        for needle in ("~/.local/bin/win11shell → ", "~/.local/bin/win11shell-ctl → ", "~/.local/bin/win11shell-session → ",
                       "data/win11shell, 1 bestand, ", "met een eigen interface",
                       "settings.json (taal: en)", "installer: 1 bestand\n", "win11shell-explorer.desktop (Name=Win11 explorer)",
                       "mimeapps.list: 2:inode/directory=win11shell-explorer.desktop", "  • /usr/share/wayland-sessions/win11shell.desktop\n",
                       "hypridle.service.d/win11shell-not-in-gnome.conf", "win11shell-session.target → ",
                       "data/icons/Win11\n", "~/.themes/Fluent-Dark", "gtk-4.0/gtk.css: 2 regels",
                       "Gevonden: 16 onderdelen van H."):
            self.assertIn(needle, text)
        en = self.run_tool(case, ["--lang", "en"])
        self.assertEqual(en.returncode, 1)
        self.assertReadOnly(case, en)
        prose = re.sub(r"[~/][^\s,()]*", "", en.stdout)
        self.assertEqual(dutch_words(prose), [])
        self.assertIn("Found: 16 parts of H. Nothing was changed", en.stdout)
        # Win11's own language decides without --lang.
        write_json(case.config/"win11-gnome/settings.json", settings_doc("en"))
        self.assertIn("Found: 16 parts", self.run_tool(case).stdout)

    def test_nothing_found(self):
        case = Case("remnants-empty")
        proc = self.run_tool(case)
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertReadOnly(case, proc)
        self.assertTrue(proc.stdout.rstrip().endswith("Niets van H gevonden."))
        self.assertEqual(proc.stdout.count("(niets)"), 10)
        bad = self.run_tool(case, ["--lang", "fr"])
        self.assertEqual((bad.returncode, bad.stderr), (2, "Onbekende taal: fr (kies nl of en).\n"))


def drop_payload(case: Case) -> None:
    """A REAL case leaves ~400 MB (binaries, icons); keep only its logs."""
    for sub in ("prefix", "data"):
        shutil.rmtree(case.dir/sub, ignore_errors=True)


def real_build(test: unittest.TestCase) -> Path:
    """The REAL cases' binaries: hard links (else copies) of the 3b898e6 debug build plus the binaries 3b898e6 did not
    have (the native ones) of tests/build/native-release, so 3b898e6's installer and the current one each find theirs.
    The 3b898e6 binaries link WebKit: without a library they need, the current installer's preflight refuses, so the
    cases are skipped and name it."""
    base, native = G/"tests/build/3b898e6-debug", G/"tests/build/native-release"
    wanted = {**{n: base/n for n in sandbox.baseline_bins()}, **{n: base/n for n in install.BINS}}
    for name in set(install.BINS) - set(sandbox.baseline_bins()):
        wanted[name] = native/name
    missing = [str(path) for path in wanted.values() if not path.is_file()]
    if missing:
        test.skipTest(f"REAL build incomplete: {', '.join(missing)}")
    needed = {library for path in wanted.values() for library in install.elf_needed(path) or []}
    absent = sorted(needed - install.known_libraries())
    if absent:
        test.skipTest(f"REAL build needs {', '.join(absent)}, which this machine does not have")
    build = RUN/"real-build"
    build.mkdir(parents=True, exist_ok=True)
    for name, source in wanted.items():
        target = build/name
        if target.exists() and os.path.samefile(target, source):
            continue
        target.unlink(missing_ok=True)
        try:
            os.link(source, target)
        except OSError:
            shutil.copy2(source, target)
    return build


def snapshot_inputs() -> str:
    """A fingerprint of the install inputs real_snapshot() copies: the two host tables, extension/ and the manifests by
    content, vendor/ui, vendor/host and vendor/config by path, size and mtime (the icon tree is stable). vendor/host
    is an input of 3b898e6's installer only, which requires and installs it."""
    import hashlib
    digest = hashlib.sha256()
    files = [G/"host/src/desktop-entries.json", G/"host/src/settings-defaults.json",
             *sorted(p for p in (G/"extension").rglob("*") if p.is_file()),
             *(G/"vendor"/n for n in ("icon-bundle.json", "ui-sha256.json", "ui-original-sha256.json", "ui-patches.json"))]
    for path in files:
        if path.is_file():
            digest.update(str(path.relative_to(G)).encode() + b"\0" + path.read_bytes())
    for name in ("ui", "host", "config"):
        for path in sorted((G/"vendor"/name).rglob("*")):
            if path.is_file() and "__pycache__" not in path.parts:
                st = path.stat()
                digest.update(f"{path.relative_to(G)}\0{st.st_size}\0{st.st_mtime_ns}\n".encode())
    return digest.hexdigest()


def real_snapshot() -> Path:
    """A frozen copy of this tree's install inputs (other steps edit vendor/ meanwhile); made again when those
    inputs changed since the copy (snapshot_inputs)."""
    root = RUN/"real-root"
    stamp = f"GE1c snapshot {snapshot_inputs()}\n"
    if (root/".complete").is_file() and (root/".complete").read_text() == stamp:
        return root
    shutil.rmtree(root, ignore_errors=True)
    shutil.copytree(G/"extension", root/"extension")
    (root/"host/src").mkdir(parents=True)
    for name in ("desktop-entries.json", "settings-defaults.json"):
        shutil.copy2(G/"host/src"/name, root/"host/src"/name)
    for name in ("ui", "host", "config", "icons", "icon-source"):
        if not (G/"vendor"/name).is_dir():
            continue
        shutil.copytree(G/"vendor"/name, root/"vendor"/name, symlinks=True,
                        ignore=shutil.ignore_patterns("__pycache__", "*.pyc"))
    for name in ("icon-bundle.json", "ICON_PROVENANCE.md", "ui-sha256.json", "ui-original-sha256.json", "ui-patches.json"):
        if (G/"vendor"/name).is_file():
            shutil.copy2(G/"vendor"/name, root/"vendor"/name)
    (root/".complete").write_text(stamp)
    return root


def check_exec_lines(test: unittest.TestCase, case: Case) -> None:
    """Exec= must give back the exact binary path (desktop-entry unescaping, then GLib's argv rules, then %%)."""
    import gi
    gi.require_version("GLib", "2.0")
    from gi.repository import GLib
    for name in install.ENTRIES:
        keyfile = GLib.KeyFile()
        keyfile.load_from_file(str(case.data/"applications"/f"io.github.win11shell.{name}.desktop"), GLib.KeyFileFlags.NONE)
        ok, argv = GLib.shell_parse_argv(keyfile.get_string("Desktop Entry", "Exec"))
        test.assertTrue(ok)
        test.assertEqual(argv[:2], ["/usr/bin/env", "--"])
        test.assertEqual(argv[2].replace("%%", "%"), str(case.prefix/"bin"/re.search(
            r"win11-[a-z-]+", argv[2].rsplit("/", 1)[-1]).group(0)))
    service = case.data/"dbus-1/services/org.freedesktop.impl.portal.desktop.win11shell.service"
    import shlex
    line = next(l for l in service.read_text().splitlines() if l.startswith("Exec="))
    test.assertEqual(shlex.split(line[5:].replace("\\\\", "\\")), [str(case.prefix/"bin/win11-file-portal")])


# O2 (13-11): the entries whose Icon= is a file of the bundled Win11 icons when the payload has it (install.py
# BUNDLED_ICONS); 3b898e6 wrote the theme name (the file's stem). The Panel keeps a name the stock Adwaita has.
MOVED_ICONS = {f"data/applications/io.github.win11shell.{name}.desktop": name for name in install.BUNDLED_ICONS}
ADWAITA_ICONS = {"Panel": "view-app-grid-symbolic"}


def without_moved_icons(tree: dict[str, str]) -> dict[str, str]:
    return {k: v for k, v in tree.items() if k not in MOVED_ICONS}


def check_moved_icons(test: unittest.TestCase, case: Case, old_tree: dict[str, str], new_tree: dict[str, str]) -> None:
    """Each of MOVED_ICONS is 3b898e6's entry byte for byte but for its Icon= line, the bundled file."""
    import hashlib
    for rel, name in MOVED_ICONS.items():
        text = (case.dir/rel).read_text()
        icon = case.prefix/"share/win11-gnome/icons/Win11"/install.BUNDLED_ICONS[name]
        test.assertIn(f"\nIcon={icon}\n", text, name)
        as_baseline = text.replace(f"\nIcon={icon}\n", f"\nIcon={icon.stem}\n")
        mode = new_tree[rel].split()[1]
        test.assertEqual(old_tree[rel], f"file {mode} {hashlib.sha256(as_baseline.encode()).hexdigest()}", name)


def check_entry_icons(test: unittest.TestCase, case: Case) -> None:
    """O2: every installed io.github.win11shell.* entry has an icon GNOME's own UI (Alt+Tab, the overview) finds
    under the stock Adwaita: a file of the payload (the bundled Win11 icons, ui/assets), or a name Adwaita has."""
    payload = case.prefix/"share/win11-gnome"
    for name, entry in entries_in(case).items():
        icon = entry["Icon"]
        if name in ADWAITA_ICONS:
            test.assertEqual(icon, ADWAITA_ICONS[name])
            continue
        test.assertTrue(icon.startswith(f"{payload}/"), (name, icon))
        test.assertTrue(Path(icon).is_file(), (name, icon))
    adwaita = Path("/usr/share/icons/Adwaita")
    if adwaita.is_dir():
        for name in ADWAITA_ICONS.values():
            test.assertTrue(list(adwaita.glob(f"*/*/{name}.*")), name)


# The build the preflight checks read, with all 19 binaries: WIN11_NATIVE_BUILD_DIR, else the reference release build.
COMPLETE_BUILD = Path(os.environ.get("WIN11_NATIVE_BUILD_DIR") or G/"tests/build/native-release")
WEBKIT_LIBS = re.compile(r"lib(webkit2gtk|javascriptcoregtk|soup)-")
# GTK 3 left the host after that (plan group L, 14-11): no binary of this tree links it either
GTK3_LIBS = re.compile(r"lib(gtk|gdk)-3\.")


class Units(unittest.TestCase):
    """In-process checks of install.py's helpers (nothing is installed)."""

    def test_bundled_entry_icons_are_files_of_the_win11_theme(self):
        """O2: each BUNDLED_ICONS path is an SVG of vendor/icons/Win11 (a link inside the theme is fine), named
        after the theme name it replaces."""
        base = G/"vendor/icons/Win11"
        self.assertEqual(sorted(install.BUNDLED_ICONS), sorted(set(install.BUNDLED_ICONS) & set(install.ENTRIES)))
        for name, rel in install.BUNDLED_ICONS.items():
            path = base/rel
            self.assertTrue(path.is_file(), rel)
            self.assertTrue(path.resolve().is_relative_to(base.resolve()), rel)
            self.assertIn(b"<svg", path.read_bytes()[:4096], rel)
        self.assertEqual({Path(rel).stem for rel in install.BUNDLED_ICONS.values()},
                         {"system-software-install", "system-search", "drive-harddisk", "utilities-system-monitor",
                          "system-file-manager"})

    def test_guess_language(self):
        here = RUN/"units"
        here.mkdir(parents=True, exist_ok=True)
        tzfile = here/"timezone"
        tzfile.write_text("Europe/Amsterdam\n")
        link = here/"localtime"
        if link.is_symlink():
            link.unlink()
        link.symlink_to("/usr/share/zoneinfo/posix/America/Kralendijk")
        none = str(here/"absent")
        rows = [
            ({"LANG": "C.UTF-8"}, none, none, ("nl", "default", None)),
            ({}, none, none, ("nl", "default", None)),
            ({"LANG": "nl_NL.UTF-8"}, none, none, ("nl", "locale", None)),
            ({"LANG": "nl_BE.UTF-8"}, none, none, ("nl", "locale", None)),
            ({"LANG": "fy_NL.UTF-8"}, none, none, ("nl", "locale", None)),
            ({"LANG": "en_US.UTF-8"}, none, none, ("en", "locale", None)),
            ({"LANG": "en_US.UTF-8", "TZ": "Europe/Amsterdam"}, none, none, ("nl", "locale", "tz:Europe/Amsterdam")),
            ({"LANG": "en_US.UTF-8", "TZ": ":Europe/Amsterdam"}, none, none, ("nl", "locale", "tz:Europe/Amsterdam")),
            ({"LANG": "en_US.UTF-8", "TZ": "/usr/share/zoneinfo/America/Curacao"}, none, none, ("nl", "locale", "tz:America/Curacao")),
            ({"LANG": "en_US.UTF-8", "TZ": "Europe/Brussels"}, none, none, ("en", "locale", None)),
            ({"LANG": "en_US.UTF-8", "TZ": "CET-1CEST"}, str(tzfile), none, ("en", "locale", None)),
            ({"LANG": "en_US.UTF-8"}, str(tzfile), none, ("nl", "locale", "tz:Europe/Amsterdam")),
            ({"LANG": "en_US.UTF-8"}, none, str(link), ("nl", "locale", "tz:America/Kralendijk")),
            ({"LANG": "en_US.UTF-8", "LC_TIME": "nl_NL.UTF-8"}, none, none, ("nl", "locale", "lc:LC_TIME")),
            ({"LANG": "en_US.UTF-8", "LC_MEASUREMENT": "nl_BE.UTF-8"}, none, none, ("nl", "locale", "lc:LC_MEASUREMENT")),
            ({"LC_ALL": "en_GB.UTF-8", "LC_TIME": "nl_NL.UTF-8"}, none, none, ("en", "locale", None)),
            ({"LANG": "nl_NL.UTF-8", "LANGUAGE": "en_GB:nl"}, none, none, ("nl", "locale", "lc:LC_TIME")),
            ({"LANG": "C", "LANGUAGE": "en_GB"}, none, none, ("nl", "default", None)),
            ({"LANG": "de_DE.UTF-8", "TZ": "Europe/Amsterdam"}, none, none, ("en", "locale", None)),
        ]
        for env, timezone, localtime, want in rows:
            with self.subTest(env=env, timezone=timezone, localtime=localtime):
                self.assertEqual(install.guess_language(env, timezone, localtime), want)

    def test_webkit_state_needs_own_real_folders(self):
        """install.remove_webkit_state removes nothing of another user (pretended here through the uid it compares
        with), and never a file or a link under a listed folder name; only what it removed is named."""
        here = RUN/"units/webkit-owner"
        shutil.rmtree(here, ignore_errors=True)
        data, cache, runtime = here/"data", here/"cache", here/"run"
        for folder in (data/"io.github.win11shell.Explorer/localstorage", cache/"win11-gnome/WebKitCache",
                       runtime/"win11-gnome"):
            folder.mkdir(parents=True)
        (runtime/"win11-gnome/explorer-restart.lock").write_text("")
        (cache/"io.github.win11shell.Explorer").write_text("a file\n")
        (data/"io.github.win11shell.Search").symlink_to(data/"io.github.win11shell.Explorer", target_is_directory=True)

        def listing() -> list[str]:
            return sorted(str(p.relative_to(here)) for p in here.rglob("*"))
        before = listing()
        out = io.StringIO()
        with mock.patch("os.geteuid", return_value=os.geteuid() + 1), contextlib.redirect_stdout(out):
            install.remove_webkit_state(data, cache, runtime)
        self.assertEqual((out.getvalue(), listing()), ("", before))
        try:
            install.set_language("en")
            with contextlib.redirect_stdout(out):
                install.remove_webkit_state(data, cache, runtime)
        finally:
            install.set_language("nl")
        self.assertEqual(out.getvalue().splitlines(),
                         [f"Removed (data of an earlier version’s WebKit apps): {path}" for path in
                          (data/"io.github.win11shell.Explorer", cache/"win11-gnome/WebKitCache",
                           runtime/"win11-gnome/explorer-restart.lock")])
        self.assertEqual(listing(), ["cache", "cache/io.github.win11shell.Explorer", "cache/win11-gnome", "data",
                                     "data/io.github.win11shell.Search", "run", "run/win11-gnome"])

    def test_cache_home_ignores_an_empty_or_relative_value(self):
        """install.cache_home (install.py and uninstall.py): an unset, empty or relative XDG_CACHE_HOME is ~/.cache,
        as the XDG spec has it, so such a value never stops an install or an uninstall; an absolute one is used."""
        home = Path.home()/".cache"
        for value, want in [(None, home), ("", home), ("cache", home), ("~/cache", home), ("./x", home),
                            ("/var/tmp/w11-cache", Path("/var/tmp/w11-cache"))]:
            env = {key: v for key, v in os.environ.items() if key != "XDG_CACHE_HOME"}
            if value is not None:
                env["XDG_CACHE_HOME"] = value
            with self.subTest(value=value), mock.patch.dict(os.environ, env, clear=True):
                self.assertEqual(install.cache_home(), want)

    def test_language_of_file(self):
        here = RUN/"units"
        here.mkdir(parents=True, exist_ok=True)
        for raw, want in [('{"language":"en"}', "en"), ('{"language":"nl"}', "nl"), ('{"language":"EN"}', "nl"),
                          ('{"language":{"custom":"x"}}', "nl"), ("{}", "nl"), ("[]", "nl"), ("{", "nl"), ("", "nl")]:
            (here/"s.json").write_text(raw)
            self.assertEqual(install.language_of_file(here/"s.json"), want, raw)
        self.assertEqual(install.language_of_file(here/"absent.json"), "nl")

    def test_elf_needed_matches_readelf(self):
        binary = G/"tests/build/3b898e6-debug/win11-panel"
        if not binary.is_file():
            self.skipTest("no baseline build")
        out = subprocess.run(["readelf", "-d", str(binary)], capture_output=True, text=True, check=True).stdout
        self.assertEqual(install.elf_needed(binary), re.findall(r"\(NEEDED\)\s+Shared library: \[([^\]]+)\]", out))
        self.assertIsNone(install.elf_needed(G/"scripts/install.sh"))

    def test_preflight_names_missing_parts(self):
        """On a build with all 19 binaries: a missing libgtk-4.so.1 (the native shell, Calendar and apps) names
        libgtk-4-1, the dlopened libarchive.so.13 (DLOPENED) libarchive13t64, in Win11's language."""
        build = COMPLETE_BUILD
        if not all((build/name).is_file() for name in install.BINS):
            self.skipTest(f"no complete build in {build}")
        install.preflight_runtime(build)                       # this machine has everything
        real = install.known_libraries
        try:
            install.known_libraries = lambda: real() - {"libarchive.so.13", "libgtk-4.so.1"}
            for lang, want in [("nl", "Ontbrekende systeemonderdelen: libgtk-4.so.1, libarchive.so.13.\n"
                                      "Installeer ze op Ubuntu met: sudo apt install libgtk-4-1 libarchive13t64"),
                               ("en", "Missing system components: libgtk-4.so.1, libarchive.so.13.\n"
                                      "On Ubuntu, install them with: sudo apt install libgtk-4-1 libarchive13t64")]:
                install.set_language(lang)
                with self.assertRaises(SystemExit) as raised:
                    install.preflight_runtime(build)
                self.assertEqual(str(raised.exception.code), want)
        finally:
            install.known_libraries = real
            install.set_language("nl")

    def test_every_needed_library_has_its_package(self):
        """Every library the 19 binaries need (DT_NEEDED) has its Ubuntu package in PACKAGES, except libc6's own, and
        PACKAGES names no WebKit, JavaScriptCore, libsoup or GTK 3 library: no binary links one since the last-webkit
        tag and the host's GTK 3 (14-11). A build that still does is older than this tree; it is skipped, by name,
        once the other libraries are checked."""
        old = lambda lib: WEBKIT_LIBS.match(lib) or GTK3_LIBS.match(lib)     # noqa: E731
        self.assertEqual([lib for lib in install.PACKAGES if old(lib)], [])
        build = COMPLETE_BUILD
        if not all((build/name).is_file() for name in install.BINS):
            self.skipTest(f"no complete build in {build}")
        needed = {library for name in install.BINS for library in install.elf_needed(build/name) or []}
        self.assertTrue({"libgtk-4.so.1", "libgraphene-1.0.so.0", "libpango-1.0.so.0"} <= needed, needed)
        older = sorted(lib for lib in needed if old(lib))
        libc6 = re.compile(r"lib(c|m|pthread|dl|rt|resolv)\.so\.\d+|ld-linux-[\w-]+\.so\.\d+")
        self.assertEqual(sorted(lib for lib in needed - set(older)
                                if lib not in install.PACKAGES and not libc6.fullmatch(lib)), [])
        if older:
            self.skipTest(f"{build} still links {', '.join(older)}: a build older than this tree, rebuild it from this tree")

    def test_desktop_entry_table_matches_baseline_dutch(self):
        """nl of host/src/desktop-entries.json == the literals of install.py at 3b898e6 (C6)."""
        old = subprocess.run(["git", "-C", str(G), "show", "3b898e6:scripts/install.py"], capture_output=True,
                             text=True, check=True).stdout
        names = dict(re.findall(r'\("(\w+)", "win11-[a-z-]+", "([^"]+)",', old))
        table = {n: v["nl"] for n, v in ENTRY_NAMES["entries"].items()}
        self.assertEqual({n: v for n, v in table.items() if n in names}, {n: v for n, v in names.items() if n in table})
        self.assertEqual(sorted(set(names) - set(table)), ["Desktop", "Start"])     # D-G70: the WebKit Start and desktop
        self.assertEqual(sorted(set(table) - set(names)), ["Calendar"])             # D-G68: the Calendar app
        self.assertIn(f'Comment={ENTRY_NAMES["comment"]["nl"]}\\n', old)

    def test_developer_scripts_speak_both_languages(self):
        """build.sh (without cargo on PATH, so nothing is built) and verify-ui.py: nl as 3b898e6, en on request."""
        if any(Path(d, "cargo").exists() for d in ("/usr/bin", "/bin")):
            self.skipTest("cargo on /usr/bin would really build")
        here = RUN/"units/dev"
        shutil.rmtree(here, ignore_errors=True)
        (here/"scripts").mkdir(parents=True)
        (here/"scripts/build.sh").write_text(subprocess.run(["git", "-C", str(G), "show", "3b898e6:scripts/build.sh"],
                                                            capture_output=True, text=True, check=True).stdout)
        env = {"PATH": "/usr/bin:/bin", "HOME": str(here), "LANG": "C.UTF-8"}
        old = subprocess.run(["/bin/bash", str(here/"scripts/build.sh")], env=env, capture_output=True, text=True)
        new = subprocess.run(["/bin/bash", str(SCRIPTS/"build.sh")], env=env, capture_output=True, text=True)
        self.assertEqual((new.returncode, new.stdout, new.stderr), (old.returncode, old.stdout, old.stderr))
        self.assertEqual(new.stderr, "Rust/Cargo en pkg-config zijn vereist.\n")
        en = subprocess.run(["/bin/bash", str(SCRIPTS/"build.sh"), "--lang", "en"], env=env, capture_output=True, text=True)
        self.assertEqual((en.returncode, en.stderr), (1, "Rust/Cargo and pkg-config are required.\n"))
        en = subprocess.run(["/bin/bash", str(SCRIPTS/"build.sh")], env={**env, "WIN11_LANG": "en"}, capture_output=True, text=True)
        self.assertEqual(en.stderr, "Rust/Cargo and pkg-config are required.\n")
        # contracts C8 / plan-critique F34 (ge1c-review m7): offline, locked, two jobs unless WIN11_BUILD_JOBS says so.
        cargo = [line for line in (SCRIPTS/"build.sh").read_text().splitlines() if line.startswith("cargo ")]
        # D-G70: a second build for the native workspace (win11-shell, win11-calendar) into the same target dir
        self.assertEqual(cargo, ['cargo build --release --offline --locked -j "${WIN11_BUILD_JOBS:-2}"',
                                 'cargo build --release --offline --locked -j "${WIN11_BUILD_JOBS:-2}" '
                                 '--manifest-path native/Cargo.toml \\'])

    def test_pairs_checker_is_clean_and_catches_errors(self):
        proc = subprocess.run([sys.executable, "-B", str(G/"tools/i18n/check_pairs.py")], capture_output=True, text=True)
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        self.assertIn(" 0 errors", proc.stdout)
        sys.path.insert(0, str(G/"tools/i18n"))
        check_pairs = importlib.import_module("check_pairs")
        here = RUN/"units/pairs"
        shutil.rmtree(here, ignore_errors=True)
        (here/"extension").mkdir(parents=True)
        (here/"scripts").mkdir()
        (here/"extension/bad.js").write_text(
            "// i18n: pairs\nconst TEXT = {\n  a: {nl: 'Hallo {name}', en: 'Hello {who}'},\n  b: {nl: 'Alleen'},\n"
            "  c: {nl: 'De taakbalk kan niet worden geminimaliseerd.', en: 'The native panel cannot be minimized.'},\n};\n"
            "const x = {nl: 'los'};\nfunction f() { failure('bad_args', 'Literal text.'); Main.notify('T', 'B'); this._t('zz'); }\n")
        (here/"scripts/bad.py").write_text(
            "T = {'a': {'nl': 'Hallo {n}', 'en': 'Hello'}, 'b': {'nl': 'x', 'en': ''}}\n"
            "def t(k, **p): return T[k]['nl'].format(**p)\nprint('Letterlijke tekst')\nt('missing')\n")
        report = check_pairs.Report()
        old_root = check_pairs.ROOT
        try:
            check_pairs.ROOT = here
            check_pairs.check_js(report, here/"extension/bad.js")
            tables = {}
            tree = check_pairs.check_py(report, here/"scripts/bad.py", tables)
            check_pairs.check_py_uses(report, here/"scripts/bad.py", tree, tables)
        finally:
            check_pairs.ROOT = old_root
        text = "\n".join(report.errors)
        for needle in ("placeholders differ", "languages ['nl']", "only one of nl:/en:", "failure() with literal text",
                       "Main.notify() with literal text", "'zz' is used but not", "an empty or non-text value",
                       "print() with literal text", "unknown text key 'missing'"):
            self.assertIn(needle, text)
        # ge1c-review m2: the glossary rule is an error since the round-3 lock-in (R3-UI); en-GB spelling warns.
        self.assertIn("glossary: TEXT.c: nl “Taakbalk” → en should contain “Taskbar”", text)
        warnings = "\n".join(report.warnings)
        self.assertIn("en-GB: TEXT.c: US spelling “minimized”", warnings)
        clean = subprocess.run([sys.executable, "-B", str(G/"tools/i18n/check_pairs.py"), "--warnings"],
                               capture_output=True, text=True)
        self.assertNotIn("glossary:", clean.stdout)
        self.assertNotIn("en-GB:", clean.stdout)


if __name__ == "__main__":
    unittest.main(verbosity=2)
