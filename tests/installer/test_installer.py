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
URI scheme default), D-G65 the payload's config/ folder, and since the HTML apps went (commit 6ebf169f) the payload
has no host/ (vendor/host left the tree with D-G73; the test trees give 3b898e6's installer a placeholder,
sandbox.old_installer_host) and only ui/assets of ui/; the done2 line of the closing text follows install.py; and since
0.9.1 the payload carries scripts/ and the prefix bin/winpoint-uninstall (UNINSTALLER_PATHS), and uninstall.py's
closing line names WinPoint (REMOVED_NL).
The Updater cases (D-G74) run the bundle's own installer as WinPoint's updater does (UPDATER, frozen_env): --check,
the exit codes and the status and log files, a failure after each step and a SIGTERM (everything back, byte for
byte, the binaries' inodes too), a SIGKILL and a rerun (roll forward), a second installer (busy), and the GNOME and
ldd checks. The Terminal cases (0.9.1) run the terminal install on a pty: its one password (the pkexec stub runs the
root script into the case's system root), the AccountsService choice (the busctl stub), the two extension lists,
the refusals, a failure after the root step, and winpoint-uninstall (also after 0.9.0's uninstall); and that the
updater's path is still 0.9.0's (v0.9.0's own scripts, git show).
Every case without a tty runs with WIN11_NO_SESSION=1 (sandbox.Case.env): 0.9.0's path, which they were written for.
Environment: W11_INSTALLER_REAL=0 skips the two REAL cases.
"""
from __future__ import annotations
import configparser
import contextlib
import hashlib
import importlib
import importlib.util
import io
import json
import os
from pathlib import Path
import pwd
import re
import shlex
import shutil
import signal
import stat
import struct
import subprocess
import sys
import time
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
# uninstall.py's closing line: 3b898e6's (up to 0.9.0), and since 0.9.1 with WinPoint's name
BASELINE_REMOVED_NL = "Win11 voor GNOME verwijderd. Je instellingen en oorspronkelijke Win11Shell zijn behouden.\n"
REMOVED_NL = "WinPoint is verwijderd. Je instellingen en oorspronkelijke Win11Shell zijn behouden.\n"
REMOVED_EN = "WinPoint removed. Your settings and the original Win11Shell were kept.\n"
# 0.9.1: the uninstaller in the prefix (install.UNINSTALLER); the payload's scripts/ is left out with its top folder
UNINSTALLER_PATHS = {f"prefix/bin/{install.UNINSTALLER}"}


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
# Extension modules that 3b898e6 did not have: the settings-parity bridges (notes/claude/settings-parity.md §6.6). The
# installer copies every file of extension/ (contracts C7), so they are new in the case tree by design.
NEW_EXTENSION_FILES = {"launchBridge.js", "displayBridge.js", "inputBridge.js"}


def same_as_baseline(tree: dict[str, str]) -> dict[str, str]:
    """A case tree without SHELL_SPLIT, the payload's config/ folder (D-G65) and host/ (3b898e6 installed vendor/host,
    the Python reference; the current installer does not), without what the payload's ui/ holds besides assets/
    (3b898e6 copied all of vendor/ui, the HTML apps included; the current installer copies vendor/ui/assets), and
    without 0.9.1's payload scripts/ and UNINSTALLER_PATHS."""
    def by_design(key: str) -> bool:
        if key in UNINSTALLER_PATHS:
            return True
        if key.startswith("data/gnome-shell/extensions/") and key.rsplit("/", 1)[-1] in NEW_EXTENSION_FILES:
            return True
        if not key.startswith(PAYLOAD):
            return False
        top, _, rest = key[len(PAYLOAD):].partition("/")
        return top in ("config", "host", "scripts") or (top == "ui" and rest != "" and rest.split("/", 1)[0] != "assets")
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
        update-without-choice path (nl, H D13). Since D-G74 the failure puts everything else back (exit 3): the
        stored language is the one change that stays."""
        case = Case("interrupted-first")
        failing = case.dir/"stubs/glib-compile-schemas"
        failing.write_text("#!/bin/sh\necho 'GE1c test: glib-compile-schemas fails on purpose' >&2\nexit 1\n")
        failing.chmod(0o755)
        before = case.tree()
        first = self.install(case, locale=EN_LOCALE)
        self.assertEqual(first.returncode, 3, first.stdout + first.stderr)
        self.assertFalse((case.prefix/"share/win11-gnome").exists())                      # rolled back
        self.assertEqual(first.stderr.splitlines()[-1], "The installation failed; everything was put back as it was.")
        after = case.tree()
        # besides the stored language, only the sandbox's keyfile GSettings folder, which GLib makes (empty) when the
        # checks read the icon theme of a first install (icon_bundle.preflight_aliases)
        self.assertEqual(sorted(k for k in after if k not in before),
                         ["config/glib-2.0", "config/glib-2.0/settings", "config/win11-gnome",
                          "config/win11-gnome/.settings.lock", "config/win11-gnome/settings.json"])
        self.assertEqual({k: v for k, v in after.items() if k in before}, before)
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
        # 0.9.1: --no-session is no advice any more (the updater's flag, still accepted)
        help_nl = self.install(case, ["--help"])
        self.assertEqual((help_nl.returncode, help_nl.stdout.splitlines()[0]), (0, "Gebruik: scripts/install.sh [--lang nl|en]"))
        help_en = self.install(case, ["--help", "--lang", "en", "--no-session"])
        self.assertEqual((help_en.returncode, help_en.stdout.splitlines()[0]), (0, "Usage: scripts/install.sh [--lang nl|en]"))
        self.assertNotIn("--no-session", help_nl.stdout + help_en.stdout)
        self.assertEqual(dutch_words(help_en.stdout), [])
        unknown = self.install(case, ["--prefix", "/x"])
        self.assertEqual(unknown.returncode, 2)
        self.assertTrue(unknown.stderr.startswith("Onbekende optie: --prefix\nGebruik:"))
        self.assertEqual(case.tree(), before)

    def test_install_sh_passes_arguments(self):
        """0.9.1: install.sh and uninstall.sh run Ubuntu's own python3 as the updater does (-E -s -B), not PATH's, and
        their comments give no --no-session or --lang advice."""
        text = (SCRIPTS/"install.sh").read_text()
        self.assertIn('exec /usr/bin/python3 -E -s -B "$ROOT/scripts/install.py" "$ROOT" "$@"', text)
        uninstall = (SCRIPTS/"uninstall.sh").read_text()
        self.assertIn('exec /usr/bin/python3 -E -s -B "$ROOT/scripts/uninstall.py" "$@"', uninstall)
        for script in (text, uninstall):
            self.assertNotIn("--no-session", script)
            self.assertNotIn("--lang", script)
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
        # 0.9.1: --keep-session is new, and the usage names winpoint-uninstall first
        self.assertEqual((usage.returncode, usage.stderr),
                         (1, "Usage: winpoint-uninstall or scripts/uninstall.sh [--files-only] [--keep-session] [--lang nl|en]\n"))
        extension = case.data/"gnome-shell/extensions"/install.UUID
        (extension/"ge1c-unknown.js").write_text("// nobody shipped this\n")    # ge1c-review m3: named, not silent
        proc = case.run(SCRIPTS/"uninstall.py", None, ["--files-only"], env, strace=True)
        self.assertOk(proc)
        self.assertEqual(proc.stdout, f"Removed from the extension folder (not a file of this version): {extension}/ge1c-unknown.js\n"
                         + REMOVED_EN)
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
        has no host/, also while the source tree has a vendor/host (the fixture's placeholder for 3b898e6's
        installer). Without vendor/ui/assets the installer refuses before any change."""
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
                self.assertEqual(proc.stdout, removed_lines(case) + REMOVED_NL)
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
        # 3b898e6's uninstaller removes only the files it shipped, so the settings-parity bridges (NEW_EXTENSION_FILES,
        # filtered out above) keep its extension folder; the current one removes every file of it
        ext_dir = f"data/gnome-shell/extensions/{install.UUID}"
        if ext_dir in old_tree and ext_dir not in new_tree and not any(k.startswith(ext_dir + "/") for k in old_tree):
            del old_tree[ext_dir]
        self.assertOk(new)
        self.assertEqual((new.stdout, new.stderr), (old.stdout.replace(BASELINE_REMOVED_NL, REMOVED_NL), old.stderr))
        self.assertEqual(new.stdout, REMOVED_NL)
        self.assertEqual(new_tree, old_tree)
        self.assertEqual((new_nothing.returncode, new_nothing.stderr), (old_nothing.returncode, old_nothing.stderr))
        self.assertEqual(calls, [])

    def test_payload_version_and_the_update_state_on_uninstall(self):
        """D-G74a: a tree with a root Cargo.toml gets share/win11-gnome/VERSION, its [workspace.package] version and a
        newline (what WinPoint's update check compares with); a tree without one, as every other test's, or with a
        version that is no X.Y.Z[-pre], gets none, so those trees stay 3b898e6's. uninstall.py removes
        $XDG_CACHE_HOME/win11-gnome/winpoint, a real folder of this user only, and nothing else of the cache."""
        import tomllib
        workspace = tomllib.loads((G/"Cargo.toml").read_text())["workspace"]["package"]["version"]
        self.assertEqual(install.bundle_version(G), workspace)
        root = sandbox.fixture_root(RUN/"fixture-version")
        (root/"Cargo.toml").write_text('[workspace]\nmembers = ["host"]\n\n[workspace.package]\nversion = "0.9.0"\n')
        case = Case("payload-version")
        self.assertOk(self.install(case, root=root, locale=NL_LOCALE))
        self.assertEqual((case.prefix/"share/win11-gnome/VERSION").read_text(), "0.9.0\n")
        for text in ('[workspace.package]\nversion = "0.9"\n', '[package]\nversion = "0.9.0"\n', "not toml ["):
            (root/"Cargo.toml").write_text(text)
            other = Case("payload-version-none")
            self.assertOk(self.install(other, root=root, locale=NL_LOCALE))
            self.assertFalse((other.prefix/"share/win11-gnome/VERSION").exists(), text)
        self.assertFalse((Shared.root/"Cargo.toml").exists())
        cache = case.dir/"cache/win11-gnome"
        (cache/"winpoint").mkdir(parents=True, mode=0o700)
        (cache/"winpoint/state.json").write_text("{}\n")
        (cache/"update").mkdir(mode=0o700)
        (cache/"update/snapshot.json").write_text("{}\n")
        proc = case.run(SCRIPTS/"uninstall.py", None, ["--files-only"], case.env(Shared.stubs, NL_LOCALE))
        self.assertOk(proc)
        self.assertEqual(proc.stdout, REMOVED_NL)
        self.assertFalse((cache/"winpoint").exists())
        self.assertTrue((cache/"update/snapshot.json").is_file(), "only the update check's folder")
        # a link in its place is left alone, and so is what it points at
        linked = Case("payload-version-link")
        self.assertOk(self.install(linked, root=root, locale=NL_LOCALE))
        elsewhere = linked.dir/"home/elsewhere"
        elsewhere.mkdir()
        (elsewhere/"keep").write_text("x\n")
        (linked.dir/"cache/win11-gnome").mkdir(parents=True)
        (linked.dir/"cache/win11-gnome/winpoint").symlink_to(elsewhere)
        self.assertOk(linked.run(SCRIPTS/"uninstall.py", None, ["--files-only"], linked.env(Shared.stubs, NL_LOCALE)))
        self.assertTrue((linked.dir/"cache/win11-gnome/winpoint").is_symlink())
        self.assertEqual((elsewhere/"keep").read_text(), "x\n")

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


# ---- D-G74: WinPoint's updater runs the bundle's own installer ----------------------------------------------------------

STEPS = ("payload", "binaries", "aliases", "portal", "extension", "entries", "autostart")
UPDATER = ["/usr/bin/python3", "-E", "-s", "-B"]                 # never -I: it drops scripts/ from sys.path


def make_bundle(name: str, version: str, extension_extra: dict[str, str] | None = None,
                shell: list[str] | None = None, scripts_at: str | None = None) -> Path:
    """A small release bundle as scripts/package.py lays one out: the fixture tree with a root Cargo.toml of `version`
    (and that version-name in metadata.json, `shell` as its shell-version), a settings icon of its own, host/data's
    four system files, the current scripts/ (the updater runs the bundle's own installer; scripts_at: those of that
    git revision instead) and bin/ with placeholder binaries of this version."""
    root = sandbox.fixture_root(RUN/name, extension_extra)
    (root/"host/data").mkdir()
    for rel in install.SYSTEM_FILES:
        shutil.copy2(G/"host/data"/Path(rel).name, root/"host/data"/Path(rel).name)
    (root/"Cargo.toml").write_text(f'[workspace]\nmembers = ["host"]\n\n[workspace.package]\nversion = "{version}"\n')
    metadata = json.loads((root/"extension/metadata.json").read_text())
    metadata["version-name"] = version
    if shell is not None:
        metadata["shell-version"] = shell
    (root/"extension/metadata.json").write_text(json.dumps(metadata, indent=2, ensure_ascii=False) + "\n")
    (root/"vendor/ui/assets/icons/settings.svg").write_text(f"<svg xmlns='http://www.w3.org/2000/svg'><!-- {version} --></svg>\n")
    (root/"scripts").mkdir()
    if scripts_at:
        sandbox.old_scripts(root/"scripts", scripts_at, ("install.py", "install-session.py", "install-system.py",
                                                         "uninstall.py", "icon_bundle.py", "file_portal.py"))
    else:
        for script in [*SCRIPTS.glob("*.py"), *SCRIPTS.glob("*.sh")]:
            shutil.copy2(script, root/"scripts"/script.name)
    (root/"bin").mkdir()
    for binary in install.BINS:
        (root/"bin"/binary).write_text(f"#!/bin/sh\n# GE1c placeholder for {binary} {version}; installer tests never run it\nexit 97\n")
        (root/"bin"/binary).chmod(0o755)
    return root


def script_at(revision: str, name: str, directory: Path):
    """scripts/<name> of a git revision (git show), loaded as a module from `directory` (its main() does not run)."""
    directory.mkdir(parents=True, exist_ok=True)
    sandbox.old_scripts(directory, revision, (name,))
    spec = importlib.util.spec_from_file_location(f"{name.replace('-', '_')[:-3]}_{revision.replace('.', '_')}",
                                                  directory/name)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def frozen_env(case: Case, bundle: Path) -> dict[str, str]:
    """Exactly the updater's environment (scripts/install.py docstring, D-G74), with the case's prefix."""
    user = pwd.getpwuid(os.getuid()).pw_name
    return {"HOME": str(case.dir/"home"), "USER": user, "LOGNAME": user, "PATH": "/usr/bin:/bin", "LANG": "C.UTF-8",
            "XDG_DATA_HOME": str(case.data), "XDG_CONFIG_HOME": str(case.config),
            "XDG_CACHE_HOME": str(case.dir/"cache"), "XDG_RUNTIME_DIR": str(case.dir/"tmp"),
            "WIN11_BUILD_DIR": str(bundle/"bin"), "WIN11_NO_SESSION": "1", "WIN11_INSTALL_PREFIX": str(case.prefix)}


def mutations(trace: Path) -> list[str]:
    """Every path a traced run wrote, created, renamed or removed, /dev/ aside."""
    found = []
    with open(trace, encoding="utf-8", errors="replace") as stream:
        for line in stream:
            parsed = sandbox.strace_paths(line.rstrip("\n"))
            if parsed:
                found += [f"{parsed[0]} {path}" for path in parsed[1] if not path.startswith("/dev/")]
    return found


def stop(proc: subprocess.Popen) -> None:
    if proc.poll() is None:
        proc.kill()
        proc.wait(timeout=30)


class Updater(unittest.TestCase):
    """D-G74: scripts/install.py as WinPoint's updater starts it (module docstring of install.py): prepare and apply,
    --check, the status and log files, the Transaction, the repair of a killed install, the lock and the two new
    checks. 0.9.0 is installed from a terminal first; 0.9.1 (another settings icon and VERSION, binaries of its own and
    one more extension file) comes through the updater."""
    maxDiff = None

    @classmethod
    def setUpClass(cls):
        Shared.build()
        cls.old = make_bundle("bundle-0.9.0", "0.9.0")
        cls.new = make_bundle("bundle-0.9.1", "0.9.1", {"ge1cNew.js": "export const GE1C_NEW = 1;\n"})

    # ---- helpers -------------------------------------------------------------------------------------------
    def base(self, name: str, first: bool = False) -> Case:
        """A case with Win11's language stored (nl) and, unless first, 0.9.0 installed from a terminal."""
        case = Case(name)
        write_json(case.config/"win11-gnome/settings.json", settings_doc("nl"))
        os.chmod(case.config/"win11-gnome", 0o700)
        if not first:
            self.assertOk(self.terminal(case, self.old))
        return case

    def terminal(self, case: Case, bundle: Path, args=(), extra=None):
        return case.run(bundle/"scripts/install.py", bundle, list(args), case.env(bundle/"bin", NL_LOCALE, extra))

    def files(self, case: Case, name: str = "install") -> list[str]:
        return ["--status", str(case.dir/f"logs/{name}.status"), "--log", str(case.dir/f"logs/{name}.log")]

    def updater(self, case: Case, bundle: Path, mode: list[str], lang: str = "nl", extra=None, strace=False):
        args = ["--lang", lang, "--no-session", *mode]
        return case.run(bundle/"scripts/install.py", bundle, args, {**frozen_env(case, bundle), **(extra or {})},
                        strace=strace, interpreter=UPDATER)

    def start(self, case: Case, bundle: Path, fail: str) -> subprocess.Popen:
        proc = case.start(bundle/"scripts/install.py", bundle, ["--lang", "nl", "--no-session", *self.files(case)],
                          {**frozen_env(case, bundle), "WIN11_INSTALL_TEST_FAIL": fail}, interpreter=UPDATER)
        self.addCleanup(stop, proc)
        deadline = time.monotonic() + 60
        while fail not in self.log(case):
            self.assertIsNone(proc.poll(), self.log(case))
            self.assertLess(time.monotonic(), deadline, "the installer never reached its pause")
            time.sleep(0.05)
        return proc

    def status(self, case: Case, name: str = "install") -> str | None:
        path = case.dir/f"logs/{name}.status"
        return path.read_text() if path.exists() else None

    def log(self, case: Case, name: str = "install") -> str:
        path = case.dir/f"logs/{name}.log"
        return path.read_text() if path.exists() else ""

    def inodes(self, case: Case) -> dict[str, int]:
        return {name: (case.prefix/"bin"/name).stat().st_ino for name in install.BINS}

    def answer(self, proc) -> dict:
        self.assertEqual(proc.stdout.count("\n"), 1, proc.stdout + proc.stderr)
        self.assertTrue(proc.stdout.isascii())
        return json.loads(proc.stdout)

    def assertOk(self, proc):
        self.assertEqual(proc.returncode, 0, f"exit {proc.returncode}\nstdout:\n{proc.stdout}\nstderr:\n{proc.stderr}")

    def leftovers(self, case: Case) -> list[str]:
        share = case.prefix/"share"
        found = [p.name for p in share.iterdir() if p.name.startswith(".")] if share.is_dir() else []
        found += [p.name for p in (case.prefix/"bin").glob(".*.new")] if (case.prefix/"bin").is_dir() else []
        return sorted(found)

    # ---- --check -------------------------------------------------------------------------------------------
    def test_check_reports_and_writes_nothing(self):
        case = self.base("check")
        before = case.tree()
        proc = self.updater(case, self.new, ["--check"], strace=True)
        self.assertEqual((proc.returncode, proc.stderr), (0, ""))
        self.assertEqual(self.answer(proc), {"ok": True, "version": "0.9.1", "extensionChanged": True, "systemFiles": []})
        self.assertEqual(case.tree(), before)
        self.assertEqual(mutations(proc.trace), [], "--check wrote")
        # the installed version itself: the same extension; a tree without a version (the fixture's) says null
        same = self.updater(case, self.old, ["--check"])
        self.assertEqual(self.answer(same), {"ok": True, "version": "0.9.0", "extensionChanged": False, "systemFiles": []})
        plain = case.run(SCRIPTS/"install.py", Shared.root, ["--lang", "nl", "--no-session", "--check"],
                         {**frozen_env(case, Shared.root), "WIN11_BUILD_DIR": str(Shared.stubs)}, interpreter=UPDATER)
        self.assertEqual(self.answer(plain)["version"], None)
        # a first install: nothing installed, so the extension is new
        fresh = self.base("check-first", first=True)
        before = fresh.tree()
        self.assertEqual(self.answer(self.updater(fresh, self.new, ["--check"])),
                         {"ok": True, "version": "0.9.1", "extensionChanged": True, "systemFiles": []})
        self.assertEqual(fresh.tree(), before)
        # a refusal: its key, params and text in --lang; exit 1, still nothing written
        (fresh.prefix/"bin").mkdir()
        (fresh.prefix/"bin/win11-panel").write_text("someone else's\n")
        before = fresh.tree()
        refused = self.updater(fresh, self.new, ["--check"], lang="en")
        self.assertEqual(refused.returncode, 1)
        panel = str(fresh.prefix/"bin/win11-panel")
        self.assertEqual(self.answer(refused), {"ok": False, "code": "conflicts", "params": {"paths": [panel]},
                                                "message": install.t("conflicts", "en") + "\n" + panel})
        self.assertEqual(fresh.tree(), before)

    # ---- the frozen contract -------------------------------------------------------------------------------
    def test_frozen_argv_environment_and_exit_codes(self):
        """Exactly the updater's argv and environment; each exit code with its status file and log."""
        reference = self.base("contract")
        self.assertOk(self.terminal(reference, self.new))
        want = reference.tree()
        case = self.base("contract")
        before = case.tree()
        # 1: refused before any change (here a previous payload that is not ours)
        foreign = case.prefix/"share"/install.PREVIOUS
        foreign.mkdir()
        (foreign/"note").write_text("not ours\n")
        refused_tree = case.tree()
        proc = self.updater(case, self.new, self.files(case))
        self.assertEqual((proc.returncode, proc.stdout, proc.stderr, self.status(case)), (1, "", "", "1\n"))
        self.assertEqual(self.log(case), install.t("previousCopy", "nl", backup=foreign) + "\n")
        self.assertEqual(case.tree(), refused_tree)
        self.assertEqual(self.answer(self.updater(case, self.new, ["--check"])),
                         {"ok": False, "code": "previousCopy", "params": {"backup": str(foreign)},
                          "message": install.t("previousCopy", "nl", backup=foreign)})
        shutil.rmtree(foreign)
        # 3: failed, everything put back
        proc = self.updater(case, self.new, self.files(case), extra={"WIN11_INSTALL_TEST_FAIL": "entries"})
        self.assertEqual((proc.returncode, self.status(case)), (3, "3\n"))
        self.assertTrue(self.log(case).endswith(install.t("rolledBack", "nl") + "\n"), self.log(case))
        self.assertEqual(case.tree(), before)
        # 4: failed, and putting it back was incomplete (the last change stays); the rerun rolls forward
        proc = self.updater(case, self.new, self.files(case), extra={"WIN11_INSTALL_TEST_FAIL": "entries:rollback"})
        self.assertEqual((proc.returncode, self.status(case)), (4, "4\n"))
        calendar = case.data/"applications/io.github.win11shell.Calendar.desktop"
        self.assertIn(install.t("notRestored", "nl", path=calendar,
                                error=install.t("testFail", "nl", step="rollback")), self.log(case))
        self.assertTrue(self.log(case).endswith(install.t("rollbackIncomplete", "nl") + "\n"), self.log(case))
        self.assertNotEqual(case.tree(), before)
        # 0: installed; every line in the log (Win11's language), nothing on stdout or stderr; byte for byte the tree a
        # terminal install of 0.9.1 leaves
        proc = self.updater(case, self.new, self.files(case))
        self.assertEqual((proc.returncode, proc.stdout, proc.stderr, self.status(case)), (0, "", "", "0\n"))
        self.assertEqual(self.log(case), nl_done(case.prefix))
        self.assertEqual(case.tree(), want)
        self.assertEqual((case.prefix/"share/win11-gnome/VERSION").read_text(), "0.9.1\n")
        self.assertEqual(self.leftovers(case), [])
        self.assertFalse(list(self.new.rglob("__pycache__")), "-B: no bytecode in the bundle")

    # ---- the Transaction -----------------------------------------------------------------------------------
    def test_a_failure_after_each_step_puts_every_byte_back(self):
        """An update and a first install: WIN11_INSTALL_TEST_FAIL after each step gives exit 3, status 3 and the tree
        of before, byte for byte (the folders a first install made are gone again, the binaries keep their inodes)."""
        for first in (False, True):
            case = self.base("each-step-first" if first else "each-step", first=first)
            before = case.tree()
            inodes = None if first else self.inodes(case)
            for step in STEPS:
                with self.subTest(first=first, step=step):
                    proc = self.updater(case, self.new, self.files(case), extra={"WIN11_INSTALL_TEST_FAIL": step})
                    self.assertEqual((proc.returncode, self.status(case)), (3, "3\n"), self.log(case))
                    self.assertIn(install.t("testFail", "nl", step=step), self.log(case))
                    self.assertTrue(self.log(case).endswith(install.t("rolledBack", "nl") + "\n"), self.log(case))
                    self.assertEqual(case.tree(), before)
                    if inodes:
                        self.assertEqual(self.inodes(case), inodes)
            self.assertOk(self.updater(case, self.new, self.files(case)))
            self.assertEqual(self.leftovers(case), [])

    def test_sigterm_during_the_binaries_puts_everything_back(self):
        case = self.base("sigterm")
        before, inodes = case.tree(), self.inodes(case)
        proc = self.start(case, self.new, "binary:wait")
        # halfway: the first binary is 0.9.1's, the others still 0.9.0's, the old payload waits next to the new one
        bins = case.prefix/"bin"
        self.assertIn("0.9.1", (bins/install.BINS[0]).read_text())
        self.assertIn("0.9.0", (bins/install.BINS[-1]).read_text())
        self.assertTrue((case.prefix/"share"/install.PREVIOUS).is_dir())
        # meanwhile a second installer, its --check and uninstall.py are refused: busy, and nothing changes
        busy = install.t("busy", "nl", path=case.prefix/"share")
        second = self.updater(case, self.new, self.files(case, "second"))
        self.assertEqual((second.returncode, self.status(case, "second"), self.log(case, "second")), (1, "1\n", busy + "\n"))
        self.assertEqual(self.answer(self.updater(case, self.new, ["--check"])),
                         {"ok": False, "code": "busy", "params": {"path": str(case.prefix/"share")}, "message": busy})
        removed = case.run(SCRIPTS/"uninstall.py", None, ["--files-only"], case.env(Shared.stubs, NL_LOCALE))
        self.assertEqual((removed.returncode, removed.stderr), (1, busy + "\n"))
        # SIGTERM twice: the second one cannot interrupt the rollback
        pid = sandbox.descendant(proc.pid)
        self.assertIsNotNone(pid)
        os.kill(pid, signal.SIGTERM)
        os.kill(pid, signal.SIGTERM)
        self.assertEqual(proc.wait(timeout=60), 3, self.log(case))
        self.assertEqual(self.status(case), "3\n")
        self.assertIn(install.t("interrupted", "nl") + "\n", self.log(case))
        self.assertTrue(self.log(case).endswith(install.t("rolledBack", "nl") + "\n"), self.log(case))
        self.assertEqual(case.tree(), before)
        self.assertEqual(self.inodes(case), inodes, "the original inodes are back")

    def test_a_signal_started_ignored_stays_ignored(self):
        """nohup (SIGHUP) and a background job (SIGINT) start an install with that signal ignored: it stays ignored,
        so closing the terminal does not roll the install back (SIGTERM, not ignored, still does: the case above)."""
        reference = self.base("nohup")
        self.assertOk(self.updater(reference, self.new, self.files(reference)))
        want = reference.tree()
        case = self.base("nohup")
        old = signal.signal(signal.SIGHUP, signal.SIG_IGN)
        try:
            proc = self.start(case, self.new, "binary:pause")
        finally:
            signal.signal(signal.SIGHUP, old)
        pid = sandbox.descendant(proc.pid)
        self.assertIsNotNone(pid)
        os.kill(pid, signal.SIGHUP)
        self.assertEqual(proc.wait(timeout=60), 0, self.log(case))
        self.assertEqual(self.status(case), "0\n")
        self.assertEqual(case.tree(), want)

    def test_a_rollback_never_leaves_a_half_removed_payload_in_place(self):
        """The rollback puts the new payload aside with one rename before the old one comes back, and repair() renames
        an old copy aside before it removes it (uninstall.py: the interrupted uninstall case): a kill at any moment
        leaves a Transaction folder (which the next run removes) and never a payload without its marker."""
        case = self.base("aside")
        payload = case.prefix/"share/win11-gnome"
        before = case.tree()
        tx = install.Transaction(payload.parent)
        tx.begin()
        stage = payload.with_name(install.STAGE + "ge1c")
        shutil.copytree(payload, stage)
        (stage/"VERSION").write_text("0.9.1\n")
        tx.swap(stage, payload)
        self.assertEqual((payload/"VERSION").read_text(), "0.9.1\n")
        real_rmtree = shutil.rmtree
        kept = (payload, payload.with_name(install.PREVIOUS))

        def rmtree(path, *args, **kwargs):
            if Path(path) in kept:
                self.fail(f"rmtree of {path} where it lies")
            return real_rmtree(path, *args, **kwargs)
        with mock.patch("shutil.rmtree", side_effect=rmtree):
            self.assertTrue(tx.rollback())
        self.assertEqual((payload/"VERSION").read_text(), "0.9.0\n")
        self.assertEqual(case.tree(), before)
        self.assertEqual(self.leftovers(case), [])
        # repair: an own old copy next to an own payload goes aside first, then away
        shutil.copytree(payload, payload.with_name(install.PREVIOUS))
        with mock.patch("shutil.rmtree", side_effect=rmtree):
            self.assertTrue(install.repair(payload, case.prefix))
        self.assertTrue(payload.is_dir())
        self.assertEqual(self.leftovers(case), [])
        # a repair killed after that rename leaves only a Transaction folder, which the next run removes unasked
        aside = payload.with_name(install.TX + "discard-1-2")
        shutil.copytree(payload, aside)
        (aside/install.MARKER).unlink()
        self.assertTrue(install.repair(payload, case.prefix))
        self.assertEqual(self.leftovers(case), [])
        self.assertEqual(case.tree(), before)

    def test_sigkill_then_the_same_bundle_rolls_forward(self):
        reference = self.base("sigkill")
        self.assertOk(self.updater(reference, self.new, self.files(reference)))
        want = reference.tree()
        case = self.base("sigkill")
        proc = self.start(case, self.new, "binary:wait")
        os.kill(sandbox.descendant(proc.pid), signal.SIGKILL)
        proc.wait(timeout=60)
        self.assertIsNone(self.status(case), "no status: the updater calls that interrupted")
        self.assertIn(install.PREVIOUS, self.leftovers(case))
        self.assertTrue(any(name.startswith(install.TX) for name in self.leftovers(case)))
        # --check repairs first (the old payload goes, the new one stays) and cannot tell which extension runs
        self.assertEqual(self.answer(self.updater(case, self.new, ["--check"])),
                         {"ok": True, "version": "0.9.1", "extensionChanged": True, "systemFiles": []})
        self.assertEqual(self.leftovers(case), [])
        self.assertOk(self.updater(case, self.new, self.files(case)))
        self.assertEqual(case.tree(), want)

    def test_repair_puts_a_lone_previous_payload_back_and_uninstall_takes_the_leftovers(self):
        """Killed between the payload's two renames: only .win11-gnome-previous is there, and the repair renames it
        back. uninstall.py removes what a killed install left (an own previous payload, stage and Transaction folders)
        and nothing else."""
        case = self.base("repair")
        before = case.tree()
        payload = case.prefix/"share/win11-gnome"
        payload.rename(payload.with_name(install.PREVIOUS))
        (case.prefix/"share/.win11-gnome-tx-ge1c/bin").mkdir(parents=True)
        (case.prefix/"bin/.win11-panel.new").write_text("half a copy\n")
        self.assertEqual(self.answer(self.updater(case, self.old, ["--check"]))["ok"], True)
        self.assertEqual(case.tree(), before)
        (case.prefix/"share/.win11-gnome-tx-ge1c").mkdir()
        (case.prefix/"share/.win11-gnome-install-ge1c").mkdir()
        shutil.copytree(payload, payload.with_name(install.PREVIOUS))
        (case.prefix/"share/someone-elses").mkdir()
        proc = case.run(SCRIPTS/"uninstall.py", None, ["--files-only"], case.env(Shared.stubs, NL_LOCALE))
        self.assertOk(proc)
        self.assertEqual(sorted(p.name for p in (case.prefix/"share").iterdir()), ["someone-elses"])

    # The stand-in for an uninstall interrupted inside the removal of the payload or of the old copy next to it, loaded
    # by uninstall.py's python3 (PYTHONPATH): shutil.rmtree of the Nth marked folder does what Python's own does
    # first (every top-level file goes before the subfolders, the marker among them), then Ctrl+C.
    INTERRUPT = """import os, shutil
real, seen = shutil.rmtree, [0]
def rmtree(path, *args, **kwargs):
    if os.path.isfile(os.path.join(path, ".win11-gnome-owned")):
        seen[0] += 1
        if seen[0] == int(os.environ["GE1C_RMTREE_STOP"]):
            for entry in os.scandir(path):
                if not entry.is_dir(follow_symlinks=False):
                    os.unlink(entry.path)
            raise KeyboardInterrupt
    return real(path, *args, **kwargs)
shutil.rmtree = rmtree
"""

    def test_an_interrupted_uninstall_never_leaves_a_payload_without_its_marker(self):
        """uninstall.py renames the payload and an own old copy aside before it removes them (install.discard): a
        Ctrl+C or a closed terminal halfway leaves each one whole with its marker or gone, never in its place without
        the marker (which every installer and uninstall.py itself refuse to touch). The next install takes the
        Transaction folders it left, and so does uninstall.sh run again (its repair puts a lone own old copy back
        first, which it then removes like any payload)."""
        removed = REMOVED_NL
        for stop in (1, 2):
            for then in ("install", "uninstall"):
                case = self.base(f"uninstall-cut-{stop}-{then}")
                payload = case.prefix/"share/win11-gnome"
                previous = payload.with_name(install.PREVIOUS)
                shutil.copytree(payload, previous)
                shim = case.dir/"shim"
                shim.mkdir()
                (shim/"sitecustomize.py").write_text(self.INTERRUPT)
                env = case.env(Shared.stubs, NL_LOCALE, {"PYTHONPATH": str(shim), "GE1C_RMTREE_STOP": str(stop)})
                proc = case.run(SCRIPTS/"uninstall.py", None, ["--files-only"], env)
                self.assertNotEqual(proc.returncode, 0, proc.stdout + proc.stderr)
                self.assertIn("KeyboardInterrupt", proc.stderr)
                for folder in (payload, previous):
                    self.assertTrue(not os.path.lexists(folder) or install.owned(folder), f"stop {stop}: {folder}")
                self.assertFalse(os.path.lexists(payload), f"stop {stop}: the payload went first")
                self.assertEqual(previous.is_dir(), stop == 1, f"stop {stop}")
                self.assertTrue(any(name.startswith(install.TX + "discard-") for name in self.leftovers(case)),
                                self.leftovers(case))
                if then == "install":
                    # an install afterwards is no notOwned: its repair takes the Transaction folders (and puts a lone
                    # own old copy back first)
                    self.assertOk(self.terminal(case, self.old))
                    self.assertTrue(install.owned(payload))
                else:
                    # no "Nothing removed" with the folders left in place: the old copy (stop 1) is removed as the
                    # payload; with nothing put back (stop 2) the rerun only takes the Transaction folder
                    again = case.run(SCRIPTS/"uninstall.py", None, ["--files-only"], case.env(Shared.stubs, NL_LOCALE))
                    if stop == 1:
                        self.assertOk(again)
                        self.assertEqual(again.stdout, removed)
                    else:
                        self.assertEqual((again.returncode, again.stderr),
                                         (1, f"Niets verwijderd: {payload} bevat geen installatiemarker van dit project.\n"))
                    self.assertFalse(os.path.lexists(payload), f"stop {stop}")
                    self.assertFalse(os.path.lexists(previous), f"stop {stop}")
                    self.assertEqual(sorted(p.name for p in (case.prefix/"share").iterdir()), [], f"stop {stop}")
                self.assertEqual(self.leftovers(case), [], f"stop {stop}, then {then}")

    # ---- the new checks ------------------------------------------------------------------------------------
    def running_major(self, case: Case) -> str:
        probe = case.dir/"major.py"
        probe.write_text(f"import sys\nsys.path.insert(0, {str(SCRIPTS)!r})\nimport install\nprint(install.gnome_major() or '')\n")
        major = case.run(probe, None, [], case.env(Shared.stubs)).stdout.strip()
        if not major:
            self.skipTest("no /usr/bin/gnome-shell that says its version")
        return major

    def test_a_gnome_shell_other_than_the_extensions_is_refused(self):
        case = self.base("gnome")
        running = self.running_major(case)
        other = str(int(running) - 1)
        bundle = make_bundle("bundle-gnome", "0.9.1", shell=[other])
        before = case.tree()
        refused = self.updater(case, bundle, ["--check"])
        self.assertEqual(refused.returncode, 1)
        self.assertEqual(self.answer(refused), {"ok": False, "code": "gnomeMismatch",
                                                "params": {"needs": other, "running": running},
                                                "message": install.t("gnomeMismatch", "nl", needs=other, running=running)})
        proc = self.terminal(case, bundle, ["--lang", "en"])
        self.assertEqual((proc.returncode, proc.stdout), (1, ""))
        self.assertEqual(proc.stderr, install.t("gnomeMismatch", "en", needs=other, running=running) + "\n")
        self.assertEqual(case.tree(), before)
        # GNOME's own rule: "50.2" matches a running 50
        minor = make_bundle("bundle-gnome-minor", "0.9.1", shell=["40", f"{running}.2"])
        self.assertTrue(self.answer(self.updater(case, minor, ["--check"]))["ok"])

    def test_a_binary_ldd_cannot_load_is_refused(self):
        """A real ldd -r failure: /usr/bin/true with one symbol version renamed (libc has no GLIBC_9.x), as a build
        made on a newer Ubuntu would need one this system lacks."""
        program = Path("/usr/bin/true").read_bytes()
        match = re.search(rb"GLIBC_2\.[0-9.]+\0", program)
        if program[:4] != b"\x7fELF" or not match:
            self.skipTest("no dynamic /usr/bin/true with a GLIBC_2 version")
        bundle = make_bundle("bundle-abi", "0.9.1")
        broken = bundle/"bin/win11-launch"
        broken.write_bytes(program[:match.start()] + b"GLIBC_9" + program[match.start() + 7:])
        broken.chmod(0o755)
        case = self.base("abi")
        before = case.tree()
        refused = self.updater(case, bundle, ["--check"])
        answer = self.answer(refused)
        self.assertEqual((refused.returncode, answer["ok"], answer["code"], answer["params"]["binary"]),
                         (1, False, "abi", "win11-launch"))
        self.assertRegex(answer["params"]["detail"], r"version `GLIBC_9[0-9.]*' not found")
        self.assertEqual(answer["message"], install.t("abi", "nl", **answer["params"]))
        proc = self.terminal(case, bundle)
        self.assertEqual((proc.returncode, proc.stderr), (1, answer["message"] + "\n"))
        self.assertEqual(case.tree(), before)

    # ---- 0.9.1: the updater's path is 0.9.0's -------------------------------------------------------------
    def test_the_updaters_path_is_still_090s(self):
        """0.9.1 changes nothing on the updater's path (WIN11_NO_SESSION=1, --no-session, no tty) but the payload's
        additions: v0.9.0's own installer (git show v0.9.0) and this one, each over an install of v0.9.0's, give the
        same --check answer, exit code, status and log, and the same tree but UNINSTALLER_PATHS and the payload's
        scripts/; neither runs pkexec or busctl. With the root files a 0.9.0 terminal install left (v0.9.0's
        contents(), host/data), systemFiles stays empty and the session counts, still without either."""
        extra = {"ge1cNew.js": "export const GE1C_NEW = 1;\n"}
        base = make_bundle("bundle-0.9.0-as-released", "0.9.0", scripts_at="v0.9.0")
        old = make_bundle("bundle-0.9.1-v0.9.0-scripts", "0.9.1", extra, scripts_at="v0.9.0")

        def without_additions(tree: dict[str, str]) -> dict[str, str]:
            return {k: v for k, v in tree.items() if k not in UNINSTALLER_PATHS and not k.startswith(PAYLOAD + "scripts")}
        results = []
        for bundle in (old, self.new):
            case = self.base("updater-090", first=True)
            self.assertOk(self.terminal(case, base))
            answer = self.answer(self.updater(case, bundle, ["--check"]))
            proc = self.updater(case, bundle, self.files(case))
            results.append((answer, proc.returncode, self.status(case), self.log(case),
                            without_additions(case.tree()), case.stub_calls()))
        self.assertEqual(results[1], results[0])
        answer, code, status, log, _tree, calls = results[1]
        self.assertEqual((answer, code, status, log, calls),
                         ({"ok": True, "version": "0.9.1", "extensionChanged": True, "systemFiles": []}, 0, "0\n",
                          nl_done(case.prefix), []))
        self.assertTrue(install.own_uninstaller(case.prefix/"bin"/install.UNINSTALLER))
        # the root files of a 0.9.0 install, on the case's system root (WIN11_SYSTEM_ROOT, a test seam: never in the
        # updater's environment)
        session = script_at("v0.9.0", "install-session.py", RUN/"v0.9.0-scripts")
        system = case.system
        (system/"usr/share/gnome-shell/modes").mkdir(parents=True)
        (system/"usr/share/gnome-shell/modes/ubuntu.json").write_text('{"stylesheetName": "Yaru/gnome-shell.css"}\n')
        contents = session.contents(system)
        for name, directory in session.FILES:
            (system/directory.lstrip("/")).mkdir(parents=True, exist_ok=True)
            (system/directory.lstrip("/")/name).write_bytes(contents[name])
        for rel in install.SYSTEM_FILES:
            (system/rel).parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(G/"host/data"/Path(rel).name, system/rel)
        seam = {"WIN11_SYSTEM_ROOT": str(system)}
        self.assertEqual(self.answer(self.updater(case, self.new, ["--check"], extra=seam)),
                         {"ok": True, "version": "0.9.1", "extensionChanged": False, "systemFiles": []})
        proc = self.updater(case, self.new, self.files(case), extra=seam)
        self.assertEqual((proc.returncode, self.status(case)), (0, "0\n"), self.log(case))
        self.assertEqual(self.log(case), session_done(case.prefix, preselected=False))
        self.assertTrue((case.config/"autostart"/install.GNOME_LOOK).is_file())
        self.assertEqual(case.stub_calls(), [])
        # a stale helper is still "system", as in 0.9.0: WinPoint's updater sends the user to the terminal
        (system/install.SYSTEM_FILES[0]).write_text("an older helper\n")
        self.assertEqual(self.answer(self.updater(case, self.new, ["--check"], extra=seam))["systemFiles"], ["system"])


# ---- 0.9.1: the terminal install's one password, the login choice, winpoint-uninstall ---------------------------------

TERMINAL = ["/usr/bin/python3", "-E", "-s", "-B"]               # what scripts/install.sh runs
SESSION = install.session_script()
MODE = "/usr/share/gnome-shell/modes/winpoint.json"
USER_PATH = f"/org/freedesktop/Accounts/User{os.getuid()}"
# every busctl call: the system bus, never a second password dialog, its own short timeout (install.BUSCTL_OPTIONS)
BUSCTL = "busctl\t--system --allow-interactive-authorization=no --timeout=5"
FIND_USER = (f"{BUSCTL} call org.freedesktop.Accounts /org/freedesktop/Accounts org.freedesktop.Accounts "
             f"FindUserById x {os.getuid()}")
GET_SESSION = f"{BUSCTL} get-property org.freedesktop.Accounts {USER_PATH} org.freedesktop.Accounts.User Session"


def accounts_call(method: str, value: str) -> str:
    return f"{BUSCTL} call org.freedesktop.Accounts {USER_PATH} org.freedesktop.Accounts.User {method} s {value}"


def install_order(rels) -> list[str]:
    """`rels` (installed paths) in the order the root script installs them: GDM's list entry winpoint.desktop last of
    the session's four, install-system.py's after them."""
    rank = {"winpoint.session": 0, "winpoint.session.conf": 0, "winpoint.json": 0, "winpoint.desktop": 1}
    return sorted(rels, key=lambda rel: rank.get(rel.rsplit("/", 1)[-1], 2))


def root_files(system: Path) -> dict[str, bytes]:
    """The eight root files as a terminal install of this tree leaves them on `system` (install-session.py's
    contents() for that system, then host/data), by installed path, in the root script's order."""
    contents = SESSION.contents(system)
    files = {f"{directory}/{name}": contents[name] for name, directory in SESSION.FILES}
    files.update({f"{directory}/{name}": (G/"host/data"/name).read_bytes() for name, directory, _mode in SESSION.SYSTEM_FILES})
    return files


def seed_system(case: Case, files: dict[str, bytes] | None = None) -> None:
    for rel, data in (root_files(case.system) if files is None else files).items():
        (case.system/rel.lstrip("/")).parent.mkdir(parents=True, exist_ok=True)
        (case.system/rel.lstrip("/")).write_bytes(data)


def present(case: Case) -> list[str]:
    return [rel for rel in root_files(case.system) if os.path.lexists(case.system/rel.lstrip("/"))]


def session_done(prefix: Path, lang: str = "nl", preselected: bool = True, moved: bool = False) -> str:
    """The closing lines of an install with the login session (install.py apply); moved: the UUID was in
    enabled-extensions (sessionMoved)."""
    return "".join(line + "\n" for line in (
        install.t("done1", lang, prefix=prefix), install.t("done2", lang), install.t("session1", lang),
        *([install.t("sessionMoved", lang)] if moved else []),
        install.t("sessionNext" if preselected else "session2", lang), install.t("done6", lang)))


def session_text(key: str, lang: str = "nl", **params) -> str:
    return SESSION.T[key][lang].format_map(params)


def uninstall_texts() -> dict:
    """scripts/uninstall.py's T, read with ast (the script does its work as it is imported)."""
    import ast
    tree = ast.parse((SCRIPTS/"uninstall.py").read_text())
    node = next(node for node in tree.body if isinstance(node, ast.Assign)
                and any(isinstance(target, ast.Name) and target.id == "T" for target in node.targets))
    return ast.literal_eval(node.value)


UNINSTALL_TEXTS = uninstall_texts()


def uninstall_text(key: str, lang: str = "nl", **params) -> str:
    return UNINSTALL_TEXTS[key][lang].format_map(params)


def keyfile(case: Case) -> dict[str, dict[str, str]]:
    """The sandbox's keyfile GSettings, by group and key (the values as GVariant text)."""
    parser = configparser.ConfigParser(interpolation=None, delimiters=("=",))
    parser.optionxform = str
    parser.read(case.config/"glib-2.0/settings/keyfile")
    return {group: dict(parser[group]) for group in parser.sections()}


def write_keyfile(case: Case, groups: dict[str, dict[str, str]]) -> None:
    (case.config/"glib-2.0/settings").mkdir(parents=True, exist_ok=True)
    (case.config/"glib-2.0/settings/keyfile").write_text(
        "".join(f"[{group}]\n" + "".join(f"{key}={value}\n" for key, value in keys.items()) + "\n"
                for group, keys in groups.items()))


SHELL_KEYS = {"org/gnome/shell": {"disabled-extensions": f"['other@ge1c', '{install.UUID}']",
                                  "enabled-extensions": "['ubuntu-dock@ubuntu.com']",
                                  "favorite-apps": "['firefox_firefox.desktop']"},
              "org/gnome/desktop/interface": {"color-scheme": "'prefer-dark'"}}


class Terminal(unittest.TestCase):
    """0.9.1 (Robin, 16-11): the terminal install (scripts/install.py's module docstring) on a pty, as scripts/install.sh
    runs it (TERMINAL): one pkexec for the eight root files (the stub runs the real root script into the case's system
    root), WinPoint preselected in AccountsService (the busctl stub), WinPoint's UUID out of the two extension lists
    only; its refusals (no tty, a cancelled password, root, another architecture) change nothing, a failure after the
    root step says its files stay; and winpoint-uninstall removes it all with one password."""
    maxDiff = None

    @classmethod
    def setUpClass(cls):
        Shared.build()
        cls.bundle = make_bundle("bundle-terminal", "0.9.1")
        # the one program uninstall.py runs (win11-app --gnome-look): it notes each run with enabled-extensions as
        # GSettings has it then (app_runs)
        (cls.bundle/"bin/win11-app").write_text(
            "#!/bin/sh\n# GE1c stand-in for win11-app 0.9.1: logs its arguments, changes nothing\n"
            "printf '%s %s\\n' \"$*\" \"$(grep '^enabled-extensions=' \"$XDG_CONFIG_HOME/glib-2.0/settings/keyfile\")\" "
            ">> \"$XDG_CONFIG_HOME/../logs/win11-app.log\"\n")

    def assertOk(self, proc):
        self.assertEqual(proc.returncode, 0, f"exit {proc.returncode}\nstdout:\n{proc.stdout}\nstderr:\n{proc.stderr}")

    def case(self, name: str, pkexec: str | None = "ok", busctl: str | None = "ok") -> Case:
        case = Case(name)
        case.pkexec(pkexec)
        case.busctl(busctl)
        # the sandbox's keyfile GSettings folder, which GLib makes when the checks read the icon theme of a first
        # install (icon_bundle.preflight_aliases); here before the "nothing changed" comparisons
        (case.config/"glib-2.0/settings").mkdir(parents=True)
        return case

    def terminal(self, case: Case, args=(), locale=NL_LOCALE, extra=None, bundle=None, tty=True, root_uid=False):
        bundle = bundle or self.bundle
        env = case.env(bundle/"bin", locale, {"WIN11_NO_SESSION": None, **(extra or {})})
        return case.run(bundle/"scripts/install.py", bundle, list(args), env, tty=tty, interpreter=TERMINAL,
                        root_uid=root_uid)

    def check(self, case: Case, bundle=None) -> dict:
        """The updater's --check on the case's system root (frozen_env and WIN11_SYSTEM_ROOT)."""
        bundle = bundle or self.bundle
        proc = case.run(bundle/"scripts/install.py", bundle, ["--lang", "nl", "--no-session", "--check"],
                        {**frozen_env(case, bundle), "WIN11_SYSTEM_ROOT": str(case.system)}, interpreter=UPDATER)
        self.assertEqual(proc.stdout.count("\n"), 1, proc.stdout + proc.stderr)
        return json.loads(proc.stdout)

    def uninstall(self, case: Case, args=(), extra=None, tty=False):
        """<prefix>/bin/winpoint-uninstall of the case, as a person runs it: without WIN11_INSTALL_PREFIX (the case's
        prefix is not ~/.local: the wrapper's payload argument names it)."""
        return case.run(case.prefix/"bin"/install.UNINSTALLER, None, list(args),
                        case.env(Shared.stubs, NL_LOCALE, {"WIN11_INSTALL_PREFIX": None, **(extra or {})}),
                        interpreter=["/bin/sh"], tty=tty)

    @staticmethod
    def app_runs(case: Case) -> list[str]:
        """The runs of the bundle's win11-app stand-in: its arguments and enabled-extensions at that moment."""
        log = case.dir/"logs/win11-app.log"
        return log.read_text().splitlines() if log.exists() else []

    # ---- the install ---------------------------------------------------------------------------------------
    def test_a_first_terminal_install_asks_one_password_for_eight_files_and_preselects(self):
        case = self.case("terminal-first")
        proc = self.terminal(case)
        self.assertOk(proc)
        self.assertEqual(proc.stderr, "")
        wanted = root_files(case.system)
        calls = case.stub_calls()
        script = hashlib.sha256(SESSION.INSTALL_SCRIPT.encode()).hexdigest()[:16]
        sums = " ".join(hashlib.sha256(data).hexdigest() for data in wanted.values())
        self.assertEqual(len(calls), 4, calls)
        self.assertRegex(calls[0], rf"^pkexec\t/bin/sh -c <script {script}> win11-gnome-session-install "
                                   rf"{re.escape(str(case.dir/'tmp'))}/win11-gnome-session-[^/ ]+ {sums}$")
        self.assertEqual(calls[1:], [FIND_USER, accounts_call("SetSession", "winpoint"),
                                     accounts_call("SetSessionType", "wayland")])
        self.assertEqual((case.accounts(), case.accounts("SessionType")), ("winpoint", "wayland"))
        for rel, data in wanted.items():
            path = case.system/rel.lstrip("/")
            self.assertEqual(path.read_bytes(), data, rel)
            self.assertEqual(stat.S_IMODE(path.stat().st_mode), 0o755 if rel.startswith("/usr/libexec/") else 0o644, rel)
        self.assertEqual([p.name for p in (case.dir/"tmp").iterdir() if p.name.startswith("win11-gnome-session-")], [])
        # what it does first (install-session.py's lines only: each statement once, the /bin/sh and administrator
        # sentence in its password line), what the root step did (GDM's list entry last of the session's four), then
        # the language and the closing lines (no systemHint)
        lines = proc.stdout.splitlines()
        self.assertEqual(lines[0], session_text("intro"))
        self.assertEqual([line for line in lines if line.startswith("Geïnstalleerd: ")],
                         [session_text("installed", path=f"{case.system}{rel}") for rel in install_order(wanted)])
        self.assertEqual(lines.count(session_text("password")), 1)
        self.assertIn("/bin/sh", session_text("password"))
        self.assertEqual([line for line in lines if "/bin/sh" in line or "één keer" in line], [session_text("password")])
        self.assertEqual([line for line in lines if "naast Ubuntu" in line], [session_text("intro")])
        self.assertTrue(proc.stdout.endswith(install.t("langChosen", "nl") + "\n" + session_done(case.prefix)), proc.stdout)
        # the user side of the session: GNOME's look back elsewhere, the portal choice in winpoint-portals.conf
        self.assertTrue((case.config/"autostart"/install.GNOME_LOOK).is_file())
        self.assertTrue((case.config/"xdg-desktop-portal/winpoint-portals.conf").is_file())
        self.assertFalse((case.config/"xdg-desktop-portal/gnome-portals.conf").exists())
        # again: nothing differs, so no password; WinPoint is preselected again
        again = self.terminal(case)
        self.assertOk(again)
        self.assertEqual((again.stdout, again.stderr), (session_done(case.prefix), ""))
        self.assertEqual(case.stub_calls()[4:], [FIND_USER, accounts_call("SetSession", "winpoint"),
                                                 accounts_call("SetSessionType", "wayland")])

    def test_a_cancelled_or_wrong_password_refuses_and_changes_nothing(self):
        for mode, key in (("cancel", "noPassword"), ("wrong", "wrongPassword")):
            with self.subTest(mode=mode):
                case = self.case(f"terminal-{mode}", pkexec=mode)
                before = case.tree()
                proc = self.terminal(case)
                self.assertEqual(proc.returncode, 1, proc.stdout + proc.stderr)
                self.assertEqual(proc.stderr, session_text(key) + "\n" + install.t("sessionNeeded", "nl") + "\n")
                self.assertEqual(case.tree(), before)
                self.assertEqual([call.split("\t")[0] for call in case.stub_calls()], ["pkexec"])
                self.assertIsNone(case.accounts())
                self.assertNotIn(install.t("done1", "nl", prefix=case.prefix), proc.stdout)
        # the password given after all: the same case installs, from scratch
        case.pkexec("ok")
        self.assertOk(self.terminal(case))
        self.assertEqual(present(case), list(root_files(case.system)))

    def test_without_a_terminal_it_refuses_and_changes_nothing(self):
        case = self.case("terminal-none")
        before = case.tree()
        proc = self.terminal(case, tty=False)
        self.assertEqual((proc.returncode, proc.stdout, proc.stderr), (1, "", install.t("sessionTerminal", "nl") + "\n"))
        self.assertEqual(case.tree(), before)
        self.assertEqual(case.stub_calls(), [])
        en = self.terminal(case, ["--lang", "en"], tty=False)
        self.assertEqual((en.returncode, en.stderr), (1, install.t("sessionTerminal", "en") + "\n"))
        # --check never takes the terminal path: it answers
        checked = self.terminal(case, ["--check"], tty=False)
        self.assertEqual((checked.returncode, json.loads(checked.stdout)["ok"]), (0, True), checked.stderr)
        self.assertEqual(case.tree(), before)
        # --no-session is 0.9.0's path, on a terminal too: no password, nothing preselected, the old hint line
        proc = self.terminal(case, ["--no-session"])
        self.assertOk(proc)
        hint = install.t("systemHint", "nl", command="python3 " + shlex.quote(str(self.bundle/"scripts/install-system.py")))
        self.assertEqual(proc.stdout, install.t("langChosen", "nl") + "\n" + nl_done(case.prefix) + hint + "\n")
        self.assertEqual(case.stub_calls(), [])

    def test_lang_en_speaks_english_also_in_the_root_step(self):
        """A first install has no settings.json yet: install.py passes its language to install-session.py."""
        case = self.case("terminal-en")
        proc = self.terminal(case, ["--lang", "en"])
        self.assertOk(proc)
        self.assertEqual(dutch_words(proc.stdout + proc.stderr), [])
        self.assertIn(session_text("password", "en") + "\n", proc.stdout)
        self.assertIn(session_text("installed", "en", path=f"{case.system}{MODE}") + "\n", proc.stdout)
        self.assertTrue(proc.stdout.endswith(session_done(case.prefix, "en")), proc.stdout)

    def test_winpoint_json_differing_only_in_ubuntus_keys_is_no_difference(self):
        """install-session.py stale(): winpoint.json by meaning, without the keys it copies from ubuntu.json; the other
        files byte for byte. Neither --check (systemFiles) nor the terminal install sees such a difference."""
        case = self.case("terminal-meaning")
        modes = case.system/MODE.lstrip("/")
        modes.parent.mkdir(parents=True)
        (modes.parent/"ubuntu.json").write_text(json.dumps({
            "parentMode": "user", "stylesheetName": "Yaru/gnome-shell.css",
            "themeResourceName": "theme/Yaru/gnome-shell-theme.gresource",
            "iconsResourceName": "theme/Yaru/gnome-shell-icons.gresource", "enabledExtensions": ["ubuntu-dock@ubuntu.com"]}))
        files = root_files(case.system)
        self.assertIn(b"Yaru/gnome-shell.css", files[MODE])
        older = json.loads(files[MODE])
        older["stylesheetName"] = "Yaru-2025/gnome-shell.css"
        del older["iconsResourceName"]
        files[MODE] = (json.dumps(older, indent=4) + "\n").encode()
        seed_system(case, files)
        self.assertEqual(self.check(case)["systemFiles"], [])
        proc = self.terminal(case)
        self.assertOk(proc)
        self.assertEqual([call for call in case.stub_calls() if call.startswith("pkexec")], [])
        self.assertEqual(modes.read_bytes(), files[MODE])
        # a difference in meaning (another extension) is one: --check reports it, the terminal install replaces it
        older["enabledExtensions"].append("other@ge1c")
        modes.write_text(json.dumps(older, indent=4) + "\n")
        self.assertEqual(self.check(case)["systemFiles"], ["session"])
        self.assertOk(self.terminal(case))
        self.assertEqual(len([call for call in case.stub_calls() if call.startswith("pkexec")]), 1)
        self.assertEqual(modes.read_bytes(), root_files(case.system)[MODE])

    def test_the_two_lists_lose_only_winpoints_uuid(self):
        """GNOME's mode skips an extension listed in disabled-extensions, and one in enabled-extensions (an in-GNOME
        install of 0.9.0) also runs in the normal session: the terminal install takes WinPoint's UUID out of both
        values, enabled-extensions first, and changes no other key; one closing line says the taskbar left the normal
        session when it was in enabled-extensions."""
        for both in (False, True):
            with self.subTest(both=both):
                case = self.case(f"terminal-disabled-{int(both)}")
                groups = json.loads(json.dumps(SHELL_KEYS))
                if both:
                    groups["org/gnome/shell"]["enabled-extensions"] = f"['ubuntu-dock@ubuntu.com', '{install.UUID}']"
                write_keyfile(case, groups)
                seed_system(case)
                proc = self.terminal(case)
                self.assertOk(proc)
                self.assertEqual([call for call in case.stub_calls() if call.startswith("pkexec")], [])
                groups["org/gnome/shell"].update({"enabled-extensions": "['ubuntu-dock@ubuntu.com']",
                                                  "disabled-extensions": "['other@ge1c']"})
                self.assertEqual(keyfile(case), groups)
                self.assertTrue(proc.stdout.endswith(install.t("langChosen", "nl") + "\n"
                                                     + session_done(case.prefix, moved=both)), proc.stdout)
                # the next install: nothing left to move, so no such line
                again = self.terminal(case)
                self.assertEqual((again.stdout, keyfile(case)), (session_done(case.prefix), groups))
        # --no-session (and the updater's path) leave GSettings alone
        case = self.case("terminal-disabled-no-session")
        write_keyfile(case, SHELL_KEYS)
        self.assertOk(self.terminal(case, ["--no-session"]))
        self.assertEqual(keyfile(case), SHELL_KEYS)

    def test_only_the_system_files_failing_is_one_warning(self):
        case = self.case("terminal-system-fails")
        (case.system/"usr/libexec").mkdir(parents=True)
        (case.system/"usr/libexec/win11-gnome").write_text("not a folder\n")
        proc = self.terminal(case)
        self.assertOk(proc)
        # install's own complaint (the root script's stderr goes to the terminal), then the two lines
        self.assertEqual(proc.stderr.splitlines()[-2:], [session_text("failedPartly", code="mkdir"), install.t("rootPartly", "nl")])
        self.assertEqual(present(case), list(root_files(case.system))[:4])
        self.assertTrue(proc.stdout.endswith(session_done(case.prefix)), proc.stdout)
        self.assertEqual(case.accounts(), "winpoint")

    def test_the_gear_instruction_when_accountsservice_says_no(self):
        case = self.case("terminal-no-accounts", busctl="fail")
        proc = self.terminal(case, ["--lang", "en"])
        self.assertOk(proc)
        self.assertTrue(proc.stdout.endswith(session_done(case.prefix, "en", preselected=False)), proc.stdout)
        self.assertEqual(case.stub_calls()[1:], [FIND_USER])

    def test_root_is_refused_before_anything(self):
        case = self.case("terminal-root")
        before = case.tree()
        for args, tty in (([], True), (["--no-session"], False)):
            with self.subTest(args=args):
                proc = self.terminal(case, args, tty=tty, root_uid=True)
                self.assertEqual((proc.returncode, proc.stderr), (1, install.t("noRoot", "nl") + "\n"))
        # README's line under sudo on an English system: the locale guess speaks, as prepare() would (no --lang)
        proc = self.terminal(case, locale=EN_LOCALE, root_uid=True)
        self.assertEqual((proc.returncode, proc.stderr), (1, install.t("noRoot", "en") + "\n"))
        checked = self.terminal(case, ["--check", "--lang", "en"], tty=False, root_uid=True)
        self.assertEqual(checked.returncode, 1)
        self.assertEqual(json.loads(checked.stdout), {"ok": False, "code": "noRoot", "params": {},
                                                      "message": install.t("noRoot", "en")})
        self.assertEqual(case.tree(), before)
        self.assertEqual(case.stub_calls(), [])

    def test_a_bundle_of_another_architecture_is_refused(self):
        """The ELF e_machine of each binary against this python3's (an aarch64 build here): abi, as ldd's refusals."""
        program = Path("/usr/bin/true").read_bytes()
        own = install.elf_machine(Path(os.path.realpath(sys.executable)))
        if program[:4] != b"\x7fELF" or own is None:
            self.skipTest("no ELF /usr/bin/true or python3")
        other = 183 if own != 183 else 62
        bundle = make_bundle("bundle-arch", "0.9.1")
        (bundle/"bin/win11-launch").write_bytes(program[:18] + struct.pack("<H" if program[5] == 1 else ">H", other) + program[20:])
        case = self.case("terminal-arch")
        before = case.tree()
        answer = self.check(case, bundle)
        detail = install.t("abiMachine", "nl", needs=install.MACHINES[other],
                           running=install.MACHINES.get(own, f"e_machine {own}"))
        self.assertEqual(dutch_words(install.t("abiMachine", "en", needs="x", running="y")), [])
        self.assertEqual(answer, {"ok": False, "code": "abi", "params": {"binary": "win11-launch", "detail": detail},
                                  "message": install.t("abi", "nl", binary="win11-launch", detail=detail)})
        proc = self.terminal(case, bundle=bundle)
        self.assertEqual((proc.returncode, proc.stderr), (1, answer["message"] + "\n"))
        self.assertEqual(case.tree(), before)
        self.assertEqual(case.stub_calls(), [])

    def test_a_failure_after_the_root_step_says_its_files_stay(self):
        """apply()'s Transaction cannot take back what root_step installed: a first install that fails after it (here
        right after the binaries and the wrapper) puts the home folder back (exit 3) and ends with rolledBackRoot,
        naming install-session.py --remove, never rolledBack. uninstall.sh then finds no install of this prefix
        (nothingRemoved) and, on a terminal, names that removal too; the next install finishes the first."""
        case = self.case("terminal-rollback")
        before = case.tree()
        proc = self.terminal(case, extra={"WIN11_INSTALL_TEST_FAIL": "binaries"})
        self.assertEqual(proc.returncode, 3, proc.stdout + proc.stderr)
        command = "python3 " + shlex.quote(str(self.bundle/"scripts/install-session.py")) + " --remove"
        self.assertEqual(proc.stderr.splitlines()[-1], install.t("rolledBackRoot", "nl", command=command))
        self.assertNotIn(install.t("rolledBack", "nl"), proc.stderr)
        self.assertEqual(present(case), list(root_files(case.system)))
        after = {rel: value for rel, value in case.tree().items() if rel.split(os.sep)[0] != "system"}
        self.assertEqual(sorted(rel for rel in after if rel not in before),
                         ["config/win11-gnome", "config/win11-gnome/.settings.lock", "config/win11-gnome/settings.json"])
        self.assertEqual({rel: after[rel] for rel in before}, before)
        self.assertFalse((case.prefix/"bin"/install.UNINSTALLER).exists())
        # in English (--lang en) the same line in English
        english = self.case("terminal-rollback-en")
        proc = self.terminal(english, ["--lang", "en"], extra={"WIN11_INSTALL_TEST_FAIL": "binaries"})
        self.assertEqual(proc.returncode, 3, proc.stdout + proc.stderr)
        self.assertEqual(proc.stderr.splitlines()[-1], install.t("rolledBackRoot", "en", command=command))
        # the updater's path (no root step) keeps rolledBack: test_a_failure_after_each_step_puts_every_byte_back
        removal = case.run(self.bundle/"scripts/uninstall.sh", None, [], case.env(Shared.stubs, NL_LOCALE), tty=True)
        payload = case.prefix/"share/win11-gnome"
        self.assertEqual((removal.returncode, removal.stderr), (1, uninstall_text("nothingRemoved", payload=payload) + "\n"))
        self.assertEqual(removal.stdout, uninstall_text(
            "sessionHint", command=f"python3 {self.bundle}/scripts/install-session.py --remove") + "\n")
        self.assertEqual(present(case), list(root_files(case.system)))
        calls = len(case.stub_calls())
        again = self.terminal(case)
        self.assertOk(again)
        self.assertEqual((again.stdout, again.stderr), (session_done(case.prefix), ""))
        self.assertEqual([call for call in case.stub_calls()[calls:] if call.startswith("pkexec")], [])

    def test_after_0_9_0s_uninstall_the_wrapper_names_a_release_whose_uninstall_takes_the_rest(self):
        """0.9.0's uninstall.sh (an old release folder, or its README) removes the payload and leaves what it does not
        know: winpoint-uninstall, the eight root files and Session=winpoint. The wrapper then says so in nl and en
        (exit 1, nothing changed, no password); a release's uninstall.sh takes that rest with one password, without a
        payload; after that there is nothing left (nothingRemoved)."""
        case = self.case("terminal-after-090")
        self.assertOk(self.terminal(case))
        old = sandbox.old_scripts(case.dir/"v0.9.0", "v0.9.0", ("install.py", "uninstall.py", "icon_bundle.py",
                                                               "file_portal.py", "install-session.py"))
        self.assertOk(case.run(old/"uninstall.py", None, [], case.env(Shared.stubs, NL_LOCALE)))
        wrapper, payload = case.prefix/"bin"/install.UNINSTALLER, case.prefix/"share/win11-gnome"
        self.assertFalse(payload.exists())
        self.assertTrue(install.own_uninstaller(wrapper))
        self.assertEqual(present(case), list(root_files(case.system)))
        self.assertEqual(case.accounts(), "winpoint")
        before, calls = case.tree(), len(case.stub_calls())
        gone = self.uninstall(case)
        self.assertEqual((gone.returncode, gone.stdout, gone.stderr),
                         (1, "", install.t("uninstallerGone", "nl", script=payload/"scripts/uninstall.py") + "\n"
                          + install.t("uninstallerGone", "en", script=payload/"scripts/uninstall.py") + "\n"))
        self.assertEqual(case.tree(), before)
        self.assertEqual(case.stub_calls()[calls:], [])
        proc = case.run(self.bundle/"scripts/uninstall.sh", None, [], case.env(Shared.stubs, NL_LOCALE))
        self.assertOk(proc)
        script = hashlib.sha256(SESSION.REMOVE_SCRIPT.encode()).hexdigest()[:16]
        self.assertEqual(case.stub_calls()[calls:],
                         [f"pkexec\t/bin/sh -c <script {script}> win11-gnome-session-remove", FIND_USER, GET_SESSION,
                          accounts_call("SetSession", "")])
        self.assertEqual(present(case), [])
        self.assertEqual(case.accounts(), "")
        self.assertFalse(wrapper.exists())
        self.assertEqual(proc.stdout.splitlines()[-1] + "\n", REMOVED_NL)
        self.assertEqual(proc.stderr, "")
        again = case.run(self.bundle/"scripts/uninstall.sh", None, [], case.env(Shared.stubs, NL_LOCALE), tty=True)
        self.assertEqual((again.returncode, again.stdout, again.stderr),
                         (1, "", uninstall_text("nothingRemoved", payload=payload) + "\n"))

    # ---- the root scripts ----------------------------------------------------------------------------------
    PROBE = """import hashlib, importlib.util, json, os, shutil, subprocess, sys
from pathlib import Path
spec = importlib.util.spec_from_file_location("install_session", sys.argv[1])
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)
here = Path(sys.argv[2])
names = [name for name, _d in m.FILES] + [name for name, _d, _m in m.SYSTEM_FILES]
data = {**m.contents(here/"none"), **{name: (m.DATA/name).read_bytes() for name, _d, _m in m.SYSTEM_FILES}}
def stage(name, files):
    folder = here/name
    folder.mkdir()
    for file in files:
        (folder/file).write_bytes(data[file])
    return folder
def run(script, name, root, *args):
    proc = subprocess.run(["/bin/sh", "-c", script, name, *map(str, args)], capture_output=True, text=True,
                          env={"PATH": "/usr/bin:/bin", "WIN11_SYSTEM_ROOT": str(here/root)})
    return [proc.returncode, proc.stdout.replace(str(here/root), "R").splitlines()]
def sums(files):
    return [hashlib.sha256(data[file]).hexdigest() for file in files]
def listing(root):
    base = here/root
    return sorted(f"{oct(p.stat().st_mode & 0o777)} {p.relative_to(base)}" for p in base.rglob("*") if p.is_file())
out = {}
eight = stage("eight", names)
out["eight"] = run(m.INSTALL_SCRIPT, m.INSTALL_NAME, "r1", eight, *sums(names))
out["eightFiles"] = listing("r1")
out["again"] = run(m.INSTALL_SCRIPT, m.INSTALL_NAME, "r1", eight, *sums(names))
out["four"] = run(m.INSTALL_SCRIPT, m.INSTALL_NAME, "r2", stage("four", names[:4]), *sums(names[:4]))
out["fourFiles"] = listing("r2")
out["systemWithoutSums"] = run(m.INSTALL_SCRIPT, m.INSTALL_NAME, "r3", eight, *sums(names[:4]))
extra = stage("extra", names)
(extra/"evil.sh").write_text("x")
out["extra"] = run(m.INSTALL_SCRIPT, m.INSTALL_NAME, "r3", extra, *sums(names))
linked = stage("linked", names[:3])
(linked/names[3]).symlink_to(eight/names[3])
out["link"] = run(m.INSTALL_SCRIPT, m.INSTALL_NAME, "r3", linked, *sums(names[:4]))
(here/"stage-link").symlink_to(eight)
out["stageLink"] = run(m.INSTALL_SCRIPT, m.INSTALL_NAME, "r3", here/"stage-link", *sums(names))
bad = sums(names)
bad[6] = "0" * 64
out["checksum"] = run(m.INSTALL_SCRIPT, m.INSTALL_NAME, "r3", eight, *bad)
out["relative"] = run(m.INSTALL_SCRIPT, m.INSTALL_NAME, "r3", "eight", *sums(names))
out["count"] = run(m.INSTALL_SCRIPT, m.INSTALL_NAME, "r3", eight, *sums(names)[:6])
out["r3"] = listing("r3") if (here/"r3").exists() else []
out["remove"] = run(m.REMOVE_SCRIPT, m.REMOVE_NAME, "r1")
out["removed"] = sorted(str(p.relative_to(here/"r1")) for p in (here/"r1").rglob("*"))
out["removeAgain"] = run(m.REMOVE_SCRIPT, m.REMOVE_NAME, "r1")
(here/"r2/usr/libexec/win11-gnome").mkdir(parents=True)
(here/"r2/usr/libexec/win11-gnome/not-ours").write_text("x")
out["removeFour"] = run(m.REMOVE_SCRIPT, m.REMOVE_NAME, "r2")
out["removeArgs"] = run(m.REMOVE_SCRIPT, m.REMOVE_NAME, "r2", "x")
# a run that stops at the mode file (a folder it may not write): GDM's list entry is not there yet
modes = here/"r4/usr/share/gnome-shell/modes"
modes.mkdir(parents=True)
modes.chmod(0o555)
out["modeFails"] = run(m.INSTALL_SCRIPT, m.INSTALL_NAME, "r4", eight, *sums(names))
modes.chmod(0o755)
out["modeFailsFiles"] = listing("r4")
print(json.dumps(out))
"""

    def test_the_root_scripts_take_only_their_own_files(self):
        """install-session.py's root scripts as the user on temporary roots (WIN11_SYSTEM_ROOT, the scripts' own rule):
        the four session files alone (5 arguments) or all eight (9), GDM's list entry last of the four, so a run that
        stops halfway lists no WinPoint without its mode; only the known names, plain files in a real stage, each
        checksum; removal of exactly the eight that are there and the two folders once empty."""
        case = Case("root-scripts")
        probe = case.dir/"probe.py"
        probe.write_text(self.PROBE)
        (case.dir/"probe").mkdir()
        proc = case.run(probe, None, [str(SCRIPTS/"install-session.py"), str(case.dir/"probe")], case.env(Shared.stubs))
        self.assertOk(proc)
        out = json.loads(proc.stdout)
        paths = [f"R{rel}" for rel in root_files(Path("/none"))]
        self.assertEqual(out["eight"], [0, [f"@@ok {path}" for path in install_order(paths)] + ["@@done"]])
        self.assertEqual(out["eightFiles"], sorted(f"{'0o755' if '/libexec/' in rel else '0o644'} {rel[1:]}"
                                                   for rel in root_files(Path("/none"))))
        self.assertEqual(out["again"], out["eight"])
        self.assertEqual(out["four"], [0, [f"@@ok {path}" for path in install_order(paths[:4])] + ["@@done"]])
        self.assertEqual(len(out["fourFiles"]), 4)
        for name, code in (("systemWithoutSums", "unknown"), ("extra", "unknown"), ("link", "missing"),
                           ("stageLink", "stage"), ("checksum", "checksum"), ("relative", "stage"), ("count", "usage")):
            self.assertEqual(out[name], [2, [f"@@fail {code}"]], name)
        self.assertEqual(out["r3"], [], "a refused run installs nothing")
        self.assertEqual(out["remove"], [0, [f"@@ok {path}" for path in paths]
                                         + ["@@ok R/usr/lib/systemd/user/gnome-session@winpoint.target.d",
                                            "@@ok R/usr/libexec/win11-gnome", "@@done"]])
        self.assertNotIn("usr/libexec/win11-gnome", out["removed"])
        self.assertIn("usr/share/polkit-1/actions", out["removed"])
        self.assertEqual(out["removeAgain"], [0, ["@@done"]])
        self.assertEqual(out["removeFour"], [0, [f"@@ok {path}" for path in paths[:4]]
                                             + ["@@ok R/usr/lib/systemd/user/gnome-session@winpoint.target.d", "@@done"]])
        self.assertEqual(out["removeArgs"], [2, ["@@fail usage"]])
        # stopped at winpoint.json: the .session and the drop-in are there, the login screen lists nothing yet
        self.assertEqual(out["modeFails"], [1, [f"@@ok {path}" for path in paths[1:3]] + ["@@fail install"]])
        self.assertEqual(out["modeFailsFiles"], sorted(f"0o644 {rel[2:]}" for rel in paths[1:3]))

    def test_install_session_dry_run_and_its_own_options(self):
        case = Case("session-dry-run")
        env = case.env(Shared.stubs)
        script = SCRIPTS/"install-session.py"
        proc = case.run(script, None, ["--dry-run", "--system", "--lang", "en"], env)
        self.assertOk(proc)
        lines = proc.stdout.splitlines()
        wanted = root_files(case.system)
        self.assertEqual(lines[0], session_text("intro", "en"))
        self.assertEqual([line.strip() for line in lines if line.startswith("  /")], list(wanted))
        self.assertIn(session_text("introSystem", "en"), lines)
        self.assertEqual(lines[-9:], [session_text("checksums", "en")]
                         + [f"{hashlib.sha256(data).hexdigest()}  {rel}" for rel, data in wanted.items()])
        self.assertRegex(lines[-10], r"^/\S+/pkexec /bin/sh -c '<script>' win11-gnome-session-install \S+( [0-9a-f]{64}){8}$")
        self.assertEqual(case.stub_calls(), [])
        # --remove with nothing there: no password at all
        nothing = case.run(script, None, ["--remove", "--lang=en"], env)
        self.assertEqual((nothing.returncode, nothing.stdout), (0, session_text("nothingRemoved", "en") + "\n"))
        self.assertEqual(case.stub_calls(), [])
        for bad in (["--lang", "fr"], ["--system", "--system"], ["--bogus"]):
            refused = case.run(script, None, bad, env)
            self.assertEqual((refused.returncode, refused.stderr), (2, session_text("usage") + "\n"), bad)

    # ---- winpoint-uninstall ----------------------------------------------------------------------------------
    def test_winpoint_uninstall_removes_everything_with_one_password(self):
        case = self.case("terminal-uninstall")
        self.assertOk(self.terminal(case, ["--lang", "en"]))
        wrapper = case.prefix/"bin"/install.UNINSTALLER
        payload = case.prefix/"share/win11-gnome"
        self.assertEqual(wrapper.read_text(), install.uninstaller_text(payload))
        self.assertEqual(stat.S_IMODE(wrapper.stat().st_mode), 0o755)
        self.assertEqual(sorted(p.name for p in (payload/"scripts").iterdir()), sorted(install.UNINSTALL_SCRIPTS))
        groups = json.loads(json.dumps(SHELL_KEYS))
        groups["org/gnome/shell"]["enabled-extensions"] = f"['ubuntu-dock@ubuntu.com', '{install.UUID}']"
        write_keyfile(case, groups)
        calls = len(case.stub_calls())
        # in the WinPoint session itself: refused, nothing changed, no password asked
        before = case.tree()
        inside = self.uninstall(case, extra={"XDG_CURRENT_DESKTOP": "WinPoint:GNOME"})
        self.assertEqual((inside.returncode, inside.stdout, inside.stderr),
                         (1, "", "You’re signed in to WinPoint now. Sign out, choose Ubuntu at the login screen (the gear "
                                 "at the bottom right, after your name) and remove WinPoint there. Nothing was removed.\n"))
        self.assertEqual(case.tree(), before)
        # a cancelled password: exit 1 and everything stays
        case.pkexec("cancel")
        cancelled = self.uninstall(case)
        self.assertEqual(cancelled.returncode, 1, cancelled.stdout + cancelled.stderr)
        self.assertEqual(cancelled.stderr, session_text("noPassword", "en") + "\n"
                         + "WinPoint was not removed. Run the uninstall again and enter the password, or keep "
                           "WinPoint’s system files with --keep-session.\n")
        self.assertEqual(case.tree(), before)
        self.assertEqual(case.accounts(), "winpoint")
        self.assertEqual([call.split("\t")[0] for call in case.stub_calls()[calls:]], ["pkexec"])
        # the password: one pkexec for the eight files, AccountsService back to GDM's default, the UUID out of both
        # lists, every file gone, the wrapper and the payload's scripts last; one closing line, nothing named stale
        case.pkexec("ok")
        calls = len(case.stub_calls())
        proc = self.uninstall(case)
        self.assertOk(proc)
        script = hashlib.sha256(SESSION.REMOVE_SCRIPT.encode()).hexdigest()[:16]
        self.assertEqual(case.stub_calls()[calls:],
                         [f"pkexec\t/bin/sh -c <script {script}> win11-gnome-session-remove", FIND_USER, GET_SESSION,
                          accounts_call("SetSession", "")])
        self.assertEqual(present(case), [])
        self.assertFalse((case.system/"usr/libexec/win11-gnome").exists())
        self.assertFalse((case.system/"usr/lib/systemd/user/gnome-session@winpoint.target.d").exists())
        self.assertEqual(case.accounts(), "")
        groups["org/gnome/shell"].update({"enabled-extensions": "['ubuntu-dock@ubuntu.com']",
                                          "disabled-extensions": "['other@ge1c']"})
        self.assertEqual(keyfile(case), groups)
        self.assertFalse(payload.exists())
        self.assertFalse(wrapper.exists())
        self.assertEqual(sorted(p.name for p in (case.prefix/"bin").iterdir()), [])
        self.assertFalse((case.data/"gnome-shell/extensions"/install.UUID).exists())
        self.assertEqual(proc.stdout.splitlines()[-1] + "\n", REMOVED_EN)
        self.assertNotIn("Removed from the extension folder", proc.stdout)
        self.assertEqual(proc.stdout.splitlines()[0], session_text("introRemove", "en"))
        self.assertIn(session_text("passwordRemove", "en") + "\n" , proc.stdout)
        self.assertIn(session_text("doneRemove", "en") + "\n", proc.stdout)
        self.assertEqual(proc.stderr, "")
        # GNOME's own look back, once the extension was out of enabled-extensions
        self.assertEqual(self.app_runs(case), ["--gnome-look enabled-extensions=['ubuntu-dock@ubuntu.com']"])

    def test_winpoint_uninstall_keeps_the_session_on_request_and_old_ways_still_work(self):
        """--keep-session leaves the eight root files (no password); --files-only also works inside WinPoint; the
        uninstall.sh of a release folder or checkout (README's old instructions) still works on 0.9.1's install, and
        a 0.9.0 uninstaller's leftover wrapper is no conflict for the next install."""
        case = self.case("terminal-keep")
        self.assertOk(self.terminal(case))
        calls = len(case.stub_calls())
        proc = self.uninstall(case, ["--keep-session"])
        self.assertOk(proc)
        self.assertEqual(proc.stdout, REMOVED_NL)
        self.assertEqual(present(case), list(root_files(case.system)))
        self.assertEqual([call.split("\t")[0] for call in case.stub_calls()[calls:]], ["busctl"] * 3)
        self.assertEqual(case.accounts(), "")
        # on a terminal one hint line first: the payload's install-session.py is gone, so it names a release's
        self.assertOk(self.terminal(case))
        proc = self.uninstall(case, ["--keep-session"], tty=True)
        self.assertOk(proc)
        release = uninstall_text("releaseCommand")
        self.assertEqual(proc.stdout, uninstall_text("sessionHint", command=release) + "\n" + REMOVED_NL)
        self.assertEqual(present(case), list(root_files(case.system)))
        # inside WinPoint with --files-only: only the user's files, no AccountsService, no pkexec
        self.assertOk(self.terminal(case))
        calls = len(case.stub_calls())
        proc = self.uninstall(case, ["--files-only"], extra={"XDG_CURRENT_DESKTOP": "WinPoint:GNOME"})
        self.assertOk(proc)
        self.assertEqual(case.stub_calls()[calls:], [])
        self.assertFalse((case.prefix/"share/win11-gnome").exists())
        # uninstall.sh of the tree (here the bundle's), as README told: the same removal with one password
        self.assertOk(self.terminal(case))
        proc = case.run(self.bundle/"scripts/uninstall.sh", None, [], case.env(Shared.stubs, NL_LOCALE))
        self.assertOk(proc)
        self.assertEqual(present(case), [])
        self.assertFalse((case.prefix/"bin"/install.UNINSTALLER).exists())
        # 0.9.0's uninstall.py leaves the wrapper (it does not know it): the next first install takes it over; another
        # file at its path blocks a first install and is left alone by an update
        self.assertOk(self.terminal(case))
        old = sandbox.old_scripts(case.dir/"v0.9.0", "v0.9.0", ("install.py", "uninstall.py", "icon_bundle.py",
                                                               "file_portal.py", "install-session.py"))
        self.assertOk(case.run(old/"uninstall.py", None, ["--files-only"], case.env(Shared.stubs, NL_LOCALE)))
        self.assertTrue(install.own_uninstaller(case.prefix/"bin"/install.UNINSTALLER))
        self.assertOk(self.terminal(case))
        foreign = case.prefix/"bin"/install.UNINSTALLER
        foreign.write_text("#!/bin/sh\necho mine\n")
        update = self.terminal(case)
        self.assertOk(update)
        self.assertEqual(foreign.read_text(), "#!/bin/sh\necho mine\n")
        fresh = self.case("terminal-foreign-wrapper")
        (fresh.prefix/"bin").mkdir()
        (fresh.prefix/"bin"/install.UNINSTALLER).write_text("#!/bin/sh\necho mine\n")
        before = fresh.tree()
        refused = self.terminal(fresh)
        self.assertEqual((refused.returncode, refused.stderr),
                         (1, install.t("conflicts", "nl") + "\n" + str(fresh.prefix/"bin"/install.UNINSTALLER) + "\n"))
        self.assertEqual(fresh.tree(), before)


    def test_the_wrapper_of_another_prefix_finds_its_own_install(self):
        """winpoint-uninstall passes only its payload: uninstall.py takes the prefix from it, so the wrapper of an
        install in another prefix (by hand, or the updater's, which passes WIN11_INSTALL_PREFIX only for such a prefix)
        works in a shell without that variable; uninstall.py of a tree without it still means ~/.local."""
        case = self.case("terminal-prefix")
        self.assertOk(self.terminal(case))
        self.assertNotEqual(case.prefix, case.dir/"home/.local")
        plain = case.run(self.bundle/"scripts/uninstall.sh", None, [],
                         case.env(Shared.stubs, NL_LOCALE, {"WIN11_INSTALL_PREFIX": None}))
        self.assertEqual((plain.returncode, plain.stderr),
                         (1, f"Niets verwijderd: {case.dir}/home/.local/share/win11-gnome bevat geen installatiemarker "
                             "van dit project.\n"))
        self.assertEqual(present(case), list(root_files(case.system)))
        proc = self.uninstall(case)
        self.assertOk(proc)
        self.assertEqual(present(case), [])
        self.assertFalse((case.prefix/"share/win11-gnome").exists())
        self.assertEqual(sorted(p.name for p in (case.prefix/"bin").iterdir()), [])

    def test_a_half_written_wrapper_is_rewritten_and_never_blocks(self):
        """apply() writes winpoint-uninstall as the binaries (a temporary copy, then one rename; repair() removes a
        leftover copy), and an empty file of this user at its path (what a power cut can leave of a new one) counts as
        its own: the updater's rerun of the same bundle rewrites it, uninstall.py removes it, a first install takes it
        over instead of refusing it."""
        case = self.case("terminal-wrapper-empty")
        self.assertOk(self.terminal(case))
        wrapper, temporary = case.prefix/"bin"/install.UNINSTALLER, case.prefix/"bin"/install.UNINSTALLER_NEW
        payload = case.prefix/"share/win11-gnome"
        wrapper.write_text("")
        temporary.write_text("#!/bin/sh\n")
        update = case.run(self.bundle/"scripts/install.py", self.bundle, ["--lang", "nl", "--no-session"],
                          {**frozen_env(case, self.bundle), "WIN11_SYSTEM_ROOT": str(case.system)}, interpreter=UPDATER)
        self.assertOk(update)
        self.assertEqual(wrapper.read_text(), install.uninstaller_text(payload))
        self.assertEqual(stat.S_IMODE(wrapper.stat().st_mode), 0o755)
        self.assertFalse(temporary.exists())
        wrapper.write_text("")
        self.assertOk(case.run(self.bundle/"scripts/uninstall.sh", None, ["--files-only"], case.env(Shared.stubs, NL_LOCALE)))
        self.assertFalse(wrapper.exists())
        wrapper.write_text("")
        self.assertOk(self.terminal(case))
        self.assertEqual(wrapper.read_text(), install.uninstaller_text(payload))

    def test_an_install_inside_gnome_gets_gnomes_look_back_and_its_system_files_named_as_such(self):
        """An install inside the normal GNOME session (--no-session, 0.9.0's mode, which 0.9.1 still updates) has no
        GnomeLook entry: winpoint-uninstall runs win11-app --gnome-look all the same, after the UUID left
        enabled-extensions, and never with --files-only. With install-system.py's files only (no login session),
        its removal and its hint speak of system files, not of the login screen."""
        case = self.case("terminal-inside-gnome")
        self.assertOk(self.terminal(case, ["--no-session"]))
        self.assertFalse((case.config/"autostart"/install.GNOME_LOOK).exists())
        groups = json.loads(json.dumps(SHELL_KEYS))
        groups["org/gnome/shell"]["enabled-extensions"] = f"['ubuntu-dock@ubuntu.com', '{install.UUID}']"
        write_keyfile(case, groups)
        system = {rel: data for rel, data in root_files(case.system).items() if "/win11-gnome/" in rel or "/polkit-1/" in rel}
        self.assertEqual(len(system), 4)
        seed_system(case, system)
        # from a release folder with --keep-session, on a terminal: the hint names that folder's install-session.py
        proc = case.run(self.bundle/"scripts/uninstall.sh", None, ["--keep-session"], case.env(Shared.stubs, NL_LOCALE),
                        tty=True)
        self.assertOk(proc)
        command = f"python3 {self.bundle}/scripts/install-session.py --remove"
        self.assertEqual(proc.stdout, uninstall_text("systemHint", command=command) + "\n" + REMOVED_NL)
        self.assertEqual(self.app_runs(case), ["--gnome-look enabled-extensions=['ubuntu-dock@ubuntu.com']"])
        # again with the payload's wrapper: one password for the four, texts without the login screen
        self.assertOk(self.terminal(case, ["--no-session"]))
        write_keyfile(case, groups)
        proc = self.uninstall(case)
        self.assertOk(proc)
        lines = proc.stdout.splitlines()
        self.assertEqual(lines[:5], [session_text("introRemoveSystem")] + [f"  {rel}" for rel in system])
        self.assertIn(session_text("doneRemoveSystem"), lines)
        self.assertNotIn("inloggen", proc.stdout)
        self.assertEqual(present(case), [])
        self.assertEqual(self.app_runs(case)[1:], ["--gnome-look enabled-extensions=['ubuntu-dock@ubuntu.com']"])
        # --files-only never runs it
        self.assertOk(self.terminal(case, ["--no-session"]))
        self.assertOk(self.uninstall(case, ["--files-only"]))
        self.assertEqual(len(self.app_runs(case)), 2)


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
            # 0.9.1: no gnome-extensions any more (GSettings); without --files-only only AccountsService is asked (a
            # blocked stub here, so nothing is reset) and, with no root file in the case's system, no pkexec
            self.assertEqual(case.stub_calls(), [] if args else [
                "busctl\t--system --allow-interactive-authorization=no --timeout=5 call org.freedesktop.Accounts "
                f"/org/freedesktop/Accounts org.freedesktop.Accounts FindUserById x {os.getuid()}"])
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
    content, vendor/ui and vendor/config by path, size and mtime (the icon tree is stable)."""
    import hashlib
    digest = hashlib.sha256()
    files = [G/"host/src/desktop-entries.json", G/"host/src/settings-defaults.json",
             *sorted(p for p in (G/"extension").rglob("*") if p.is_file()),
             *(G/"vendor"/n for n in ("icon-bundle.json", "ui-sha256.json", "ui-original-sha256.json", "ui-patches.json"))]
    for path in files:
        if path.is_file():
            digest.update(str(path.relative_to(G)).encode() + b"\0" + path.read_bytes())
    for name in ("ui", "config"):
        for path in sorted((G/"vendor"/name).rglob("*")):
            if path.is_file() and "__pycache__" not in path.parts:
                st = path.stat()
                digest.update(f"{path.relative_to(G)}\0{st.st_size}\0{st.st_mtime_ns}\n".encode())
    return digest.hexdigest()


def real_snapshot() -> Path:
    """A frozen copy of this tree's install inputs (other steps edit vendor/ meanwhile); made again when those
    inputs changed since the copy (snapshot_inputs). Its vendor/host is the placeholder 3b898e6's installer
    requires (sandbox.old_installer_host): the tree has none since D-G73."""
    root = RUN/"real-root"
    stamp = f"GE1c snapshot {snapshot_inputs()}\n"
    if (root/".complete").is_file() and (root/".complete").read_text() == stamp:
        return root
    shutil.rmtree(root, ignore_errors=True)
    shutil.copytree(G/"extension", root/"extension")
    (root/"host/src").mkdir(parents=True)
    for name in ("desktop-entries.json", "settings-defaults.json"):
        shutil.copy2(G/"host/src"/name, root/"host/src"/name)
    for name in ("ui", "config", "icons", "icon-source"):
        if not (G/"vendor"/name).is_dir():
            continue
        shutil.copytree(G/"vendor"/name, root/"vendor"/name, symlinks=True,
                        ignore=shutil.ignore_patterns("__pycache__", "*.pyc"))
    for name in ("icon-bundle.json", "ICON_PROVENANCE.md", "ui-sha256.json", "ui-original-sha256.json", "ui-patches.json"):
        if (G/"vendor"/name).is_file():
            shutil.copy2(G/"vendor"/name, root/"vendor"/name)
    sandbox.old_installer_host(root/"vendor")
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

    def test_session_files_are_byte_for_byte_090s(self):
        """D-G74's frozen bytes: install-session.py's contents() for a system equals v0.9.0's (git show v0.9.0), with
        and without Ubuntu's ubuntu.json (also one whose keys are not all text); stale() finds nothing in the files a
        0.9.0 install left, so --check reports no "session" for them."""
        here = RUN/"units/session-090"
        shutil.rmtree(here, ignore_errors=True)
        old = script_at("v0.9.0", "install-session.py", here/"scripts")
        new = install.session_script()
        self.assertEqual(new.FILES, old.FILES)
        live = Path("/usr/share/gnome-shell/modes/ubuntu.json")
        ubuntu = live.read_text() if live.is_file() else json.dumps({
            "parentMode": "user", "stylesheetName": "Yaru/gnome-shell.css",
            "themeResourceName": "theme/Yaru/gnome-shell-theme.gresource",
            "iconsResourceName": "theme/Yaru/gnome-shell-icons.gresource", "enabledExtensions": ["ubuntu-dock@ubuntu.com"]})
        for name, text in (("without", None), ("ubuntu", ubuntu), ("odd", '{"stylesheetName": 3, "themeResourceName": "x"}'),
                           ("broken", "{")):
            with self.subTest(ubuntu=name):
                root = here/name
                if text is not None:
                    (root/"usr/share/gnome-shell/modes").mkdir(parents=True)
                    (root/"usr/share/gnome-shell/modes/ubuntu.json").write_text(text)
                root.mkdir(parents=True, exist_ok=True)
                self.assertEqual(new.contents(root), old.contents(root))
                self.assertEqual(new.stale(root), [file for file, _directory in new.FILES])
                for file, directory in old.FILES:
                    (root/directory.lstrip("/")).mkdir(parents=True, exist_ok=True)
                    (root/directory.lstrip("/")/file).write_bytes(old.contents(root)[file])
                self.assertEqual(new.stale(root), [])
                self.assertTrue(new.installed(root))
                self.assertEqual(install.system_files(G, root), [])

    def test_the_system_file_tables_agree(self):
        """install-session.py's SYSTEM_FILES are install-system.py's FILES and install.py's SYSTEM_FILES, from the same
        host/data; UNINSTALL_SCRIPTS holds every module of scripts/ that uninstall.py imports or runs."""
        spec = importlib.util.spec_from_file_location("install_system", SCRIPTS/"install-system.py")
        system = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(system)
        self.assertEqual(SESSION.SYSTEM_FILES, system.FILES)
        self.assertEqual(SESSION.DATA, system.DATA)
        self.assertEqual([f"{directory}/{name}"[1:] for name, directory, _mode in SESSION.SYSTEM_FILES],
                         list(install.SYSTEM_FILES))
        self.assertTrue(SESSION.system_sources())
        import ast
        used = set()
        for name in ("uninstall.py", "install.py", "install-session.py"):
            tree = ast.parse((SCRIPTS/name).read_text())
            used |= {f"{alias.name}.py" for node in ast.walk(tree) if isinstance(node, ast.Import) for alias in node.names}
            used |= {f"{node.module}.py" for node in ast.walk(tree) if isinstance(node, ast.ImportFrom) and node.module}
        used = {name for name in used if (SCRIPTS/name).is_file()} | {"install-session.py"}
        self.assertLessEqual(used, set(install.UNINSTALL_SCRIPTS))
        self.assertTrue(all((SCRIPTS/name).is_file() for name in install.UNINSTALL_SCRIPTS))

    def test_abi_problems_and_elf_machines(self):
        """check_abi: ldd's three problem lines (a foreign or static binary gives "not a dynamic executable"); the
        e_machine of an ELF file, None for anything else."""
        for line in ("\tlibfoo.so.1 => not found", "undefined symbol: g_x\t(./win11-panel)", "\tnot a dynamic executable"):
            self.assertRegex(line, install.ABI_PROBLEM)
        self.assertNotRegex("\tlibc.so.6 => /lib/x86_64-linux-gnu/libc.so.6 (0x0000)", install.ABI_PROBLEM)
        self.assertIsNone(install.elf_machine(SCRIPTS/"install.sh"))
        self.assertIsNone(install.elf_machine(RUN/"units/absent"))
        self.assertIn(install.elf_machine(Path(os.path.realpath(sys.executable))), install.MACHINES)

    def test_the_terminal_path_and_its_refusals(self):
        """terminal_install(): --check, --no-session and WIN11_NO_SESSION=1 are 0.9.0's path (False); then no tty is
        sessionTerminal, a missing gnome-session or pkexec missingParts with its package."""
        class Tty(io.StringIO):
            def isatty(self):
                return True
        is_file, which = Path.is_file, shutil.which
        without_session = lambda path: str(path) != "/usr/bin/gnome-session" and is_file(path)     # noqa: E731
        without_pkexec = lambda name, *args, **kwargs: None if name == "pkexec" else which(name, *args, **kwargs)  # noqa: E731
        with mock.patch.dict(os.environ, {"WIN11_NO_SESSION": ""}), mock.patch.object(install, "NO_SESSION", False):
            with mock.patch("sys.stdin", Tty()):
                self.assertFalse(install.terminal_install(check=True))
                with mock.patch.object(install, "NO_SESSION", True):
                    self.assertFalse(install.terminal_install())
                with mock.patch.dict(os.environ, {"WIN11_NO_SESSION": "1"}):
                    self.assertFalse(install.terminal_install())
                with mock.patch("shutil.which", lambda name, *a, **k: f"/usr/bin/{name}"):
                    self.assertTrue(install.terminal_install())
                for patch, missing, packages in ((mock.patch.object(Path, "is_file", without_session), "gnome-session",
                                                  "gnome-session-bin"),
                                                 (mock.patch("shutil.which", without_pkexec), "pkexec", "pkexec")):
                    with self.subTest(missing=missing), patch, self.assertRaises(install.Refusal) as raised:
                        install.terminal_install()
                    self.assertEqual((raised.exception.key, raised.exception.params),
                                     ("missingParts", {"missing": missing, "packages": packages}))
                    self.assertEqual(raised.exception.code, install.t("missingParts", missing=missing) + "\n"
                                     + install.t("installParts", packages=packages))
            with mock.patch("sys.stdin", io.StringIO()), self.assertRaises(install.Refusal) as raised:
                install.terminal_install()
            self.assertEqual(raised.exception.key, "sessionTerminal")

    def test_messages_before_prepare_speak_the_language_it_picks(self):
        """Without --lang, locate() sets the message language prepare() will pick, read only, so busy and the path
        checks follow it: settings.json, an update without one (nl), the locale guess; as root the guess (noRoot),
        never a home folder. --lang leaves it alone."""
        here = RUN/"units/early-language"
        shutil.rmtree(here, ignore_errors=True)
        (here/"config/win11-gnome").mkdir(parents=True)
        env = {"HOME": str(here/"home"), "XDG_CONFIG_HOME": str(here/"config"), "XDG_DATA_HOME": str(here/"data"),
               "WIN11_INSTALL_PREFIX": str(here/"prefix"), "LANG": "en_GB.UTF-8", "TZ": "Europe/London"}
        self.addCleanup(install.set_language, "nl")
        with mock.patch.dict(os.environ, env, clear=True):
            install.set_language("nl")
            install.locate(here, install.Options())
            self.assertEqual(install.LANGUAGE, "en")
            (here/"config/win11-gnome/settings.json").write_text('{"language": "nl"}\n')
            install.locate(here, install.Options())
            self.assertEqual(install.LANGUAGE, "nl")
            (here/"config/win11-gnome/settings.json").unlink()
            (here/"prefix/share/win11-gnome").mkdir(parents=True)
            install.set_language("en")
            install.locate(here, install.Options())
            self.assertEqual(install.LANGUAGE, "nl")
            (here/"config/win11-gnome/settings.json").write_text('{"language": "nl"}\n')
            with mock.patch("os.geteuid", return_value=0), self.assertRaises(install.Refusal) as raised:
                install.locate(here, install.Options())
            self.assertEqual((raised.exception.key, raised.exception.code), ("noRoot", install.t("noRoot", "en")))
            install.set_language("en")                      # what parse_arguments set for --lang en
            install.locate(here, install.Options(explicit="en"))
            self.assertEqual(install.LANGUAGE, "en")
        self.assertEqual(sorted(str(p.relative_to(here)) for p in here.rglob("*")),
                         ["config", "config/win11-gnome", "config/win11-gnome/settings.json", "prefix", "prefix/share",
                          "prefix/share/win11-gnome"], "nothing written")

    def test_the_uninstaller_wrapper_quotes_its_paths(self):
        """<prefix>/bin/winpoint-uninstall runs the payload's uninstall.py with the payload, whatever the prefix holds;
        own_uninstaller() knows it by its second line."""
        here = RUN/f"units/wrapper {NASTY}"
        shutil.rmtree(here, ignore_errors=True)
        (here/"bin").mkdir(parents=True)
        payload = here/"share/win11-gnome"
        text = install.uninstaller_text(payload)
        self.assertEqual(shlex.split(text.splitlines()[3]),
                         ["exec", "/usr/bin/python3", "-E", "-s", "-B", str(payload/"scripts/uninstall.py"), str(payload), "$@"])
        wrapper = here/"bin"/install.UNINSTALLER
        wrapper.write_text(text, encoding="utf-8")
        self.assertTrue(install.own_uninstaller(wrapper))
        # without the payload's uninstall.py (0.9.0's uninstall.sh): both languages, exit 1, before any exec
        case = Case("units-wrapper")
        gone = case.run(wrapper, None, [], case.env(case.dir/"stubs"), interpreter=["/bin/sh"])
        self.assertEqual((gone.returncode, gone.stdout, gone.stderr),
                         (1, "", install.t("uninstallerGone", "nl", script=payload/"scripts/uninstall.py") + "\n"
                          + install.t("uninstallerGone", "en", script=payload/"scripts/uninstall.py") + "\n"))
        wrapper.write_text("#!/bin/sh\nexec true\n")
        self.assertFalse(install.own_uninstaller(wrapper))
        wrapper.unlink()
        (here/"elsewhere").write_text(text)
        wrapper.symlink_to(here/"elsewhere")
        self.assertFalse(install.own_uninstaller(wrapper))

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
        PACKAGES names no WebKit, JavaScriptCore, libsoup or GTK 3 library: no binary links one since commit
        6ebf169f and the host's GTK 3 (14-11). A build that still does is older than this tree; it is skipped, by name,
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

    def test_system_files_report_stale_root_files(self):
        """D-G74, --check's systemFiles: "session" when install-session.py's four files are there and one differs from
        what this tree's would write, "system" when one of install-system.py's is there and differs from host/data;
        files that are not installed are no report."""
        here = RUN/"units/system-files"
        shutil.rmtree(here, ignore_errors=True)
        root, system = here/"tree", here/"system"
        (root/"host/data").mkdir(parents=True)
        for rel in install.SYSTEM_FILES:
            (root/"host/data"/Path(rel).name).write_text(f"GE1c {rel}\n")
        system.mkdir()
        self.assertEqual(install.system_files(root, system), [])
        session = install.session_script()
        wanted = session.contents(system)
        for name, directory in session.FILES:
            (system/directory.lstrip("/")).mkdir(parents=True, exist_ok=True)
            (system/directory.lstrip("/")/name).write_bytes(wanted[name])
        self.assertEqual(install.system_files(root, system), [])
        stale = system/session.FILES[1][1].lstrip("/")/session.FILES[1][0]
        stale.write_bytes(wanted[session.FILES[1][0]] + b"# older\n")
        self.assertEqual(install.system_files(root, system), ["session"])
        stale.write_bytes(wanted[session.FILES[1][0]])
        helper = system/install.SYSTEM_FILES[0]
        helper.parent.mkdir(parents=True)
        helper.write_text(f"GE1c {install.SYSTEM_FILES[0]}\n")
        self.assertEqual(install.system_files(root, system), [])
        helper.write_text("GE1c an older helper\n")
        self.assertEqual(install.system_files(root, system), ["system"])
        stale.write_bytes(b"older\n")
        self.assertEqual(install.system_files(root, system), ["session", "system"])

    def test_extension_changed_compares_files_and_schemas(self):
        here = RUN/"units/extension-changed"
        shutil.rmtree(here, ignore_errors=True)
        root, installed = here/"tree", here/"installed"
        (root/"extension/schemas").mkdir(parents=True)
        (root/"extension/extension.js").write_text("// 1\n")
        (root/"extension/schemas/x.gschema.xml").write_text("<schemalist/>\n")
        self.assertTrue(install.extension_changed(root, installed))             # nothing installed
        shutil.copytree(root/"extension", installed)
        (installed/"schemas/gschemas.compiled").write_bytes(b"GVariant")       # made by the install itself
        self.assertFalse(install.extension_changed(root, installed))
        for change in (lambda: (installed/"extension.js").write_text("// 0\n"),
                       lambda: (installed/"old.js").write_text("// gone in this version\n"),
                       lambda: (installed/"schemas/x.gschema.xml").write_text("<schemalist><!-- 0 --></schemalist>\n")):
            shutil.rmtree(installed)
            shutil.copytree(root/"extension", installed)
            change()
            self.assertTrue(install.extension_changed(root, installed))

    def test_portal_selector_switch_writes_nothing_until_register(self):
        """D-G74: a portal record that names the other selector (the login session came or went since) is undone by
        file_portal.register now, not by preflight, which only reads (install.py --check); the result is byte for
        byte the one of the earlier file_portal.py (d30430b0^), whose preflight undid it at once."""
        spec = importlib.util.spec_from_loader("file_portal_before", loader=None)
        before = importlib.util.module_from_spec(spec)
        exec(subprocess.run(["git", "-C", str(G), "show", "d30430b0^:scripts/file_portal.py"], capture_output=True,
                            text=True, check=True).stdout, before.__dict__)
        portal = importlib.import_module("file_portal")
        here = RUN/"units/portal-switch"
        payload, prefix, data, config = here/"prefix/share/win11-gnome", here/"prefix", here/"data", here/"config"

        def listing() -> dict[str, bytes]:
            return {str(p.relative_to(here)): p.read_bytes() for p in sorted(here.rglob("*")) if p.is_file()}

        def installed(session: bool) -> None:
            shutil.rmtree(here, ignore_errors=True)
            payload.mkdir(parents=True)
            (config/"xdg-desktop-portal").mkdir(parents=True)
            (config/"xdg-desktop-portal/gnome-portals.conf").write_text("[preferred]\ndefault=gnome;gtk;\n# mine\n")
            portal.register(payload, portal.preflight(payload, prefix, data, config, session))

        with mock.patch.dict(os.environ, {"XDG_CURRENT_DESKTOP": "ubuntu:GNOME"}):
            for session in (True, False):
                with self.subTest(session=session):
                    installed(not session)
                    seen = listing()
                    plan = before.preflight(payload, prefix, data, config, session)
                    before.register(payload, plan)
                    want = listing()
                    self.assertNotEqual(want, seen)
                    installed(not session)
                    plan = portal.preflight(payload, prefix, data, config, session)
                    self.assertIn("undo", plan)
                    self.assertEqual(listing(), seen, "preflight wrote")
                    portal.register(payload, plan)
                    self.assertEqual(listing(), want)
                    self.assertNotIn("undo", json.loads((payload/portal.MANIFEST).read_text()))

    def test_uninstall_keeps_the_update_folder_while_its_installer_runs(self):
        """D-G74 (design 5.5): $XDG_CACHE_HOME/win11-gnome/winpoint stays only while job.json says installing and
        its installer {pid, start} (the host's process_start) still runs."""
        here = RUN/"units/winpoint-cache"
        live = {"pid": os.getpid(), "start": install.process_start(os.getpid())}
        self.assertTrue(live["start"] and live["start"].isdigit())
        for job, kept in [({"state": "installing", "installer": live}, True),
                          ({"state": "installing", "installer": {**live, "start": "1"}}, False),
                          ({"state": "installing", "installer": {**live, "pid": True}}, False),
                          ({"state": "downloading", "installer": live}, False),
                          ({"state": "installing", "installer": None}, False), (None, False), ("{broken", False)]:
            with self.subTest(job=job):
                shutil.rmtree(here, ignore_errors=True)
                folder = here/"cache/win11-gnome/winpoint"
                folder.mkdir(parents=True)
                if job is not None:
                    (folder/"job.json").write_text(job if isinstance(job, str) else json.dumps(job))
                install.remove_winpoint_cache(here/"cache")
                self.assertEqual(folder.is_dir(), kept)

    def test_the_release_build_loads_cleanly(self):
        """D-G74's gate: ldd -r finds nothing missing in the 19 binaries of the reference release build, so its own
        install passes check_abi."""
        build = COMPLETE_BUILD
        if not all((build/name).is_file() for name in install.BINS):
            self.skipTest(f"no complete build in {build}")
        self.assertTrue(all(install.elf_needed(build/name) is not None for name in install.BINS))
        install.check_abi(build)

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
