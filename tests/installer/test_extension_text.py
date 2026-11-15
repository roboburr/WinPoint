#!/usr/bin/env python3
"""GE1c extension text tests: tests/installer/ext/dump.mjs runs extension/*.js in a fake GNOME Shell under Node
and prints every text the extension shows or returns. Compared here:
* nl of the current extension against nl of 3b898e6's (git show): only the keys listed in
  tests/i18n/BASELINE-CHANGES.ext may differ, with exactly the listed Dutch;
* en: no Dutch word (tests/i18n/dutch-words.txt) outside the intended exceptions;
* a live switch (settings.json replaced by rename, or rewritten in place, or its folder created later) gives
  exactly what a fresh start in that language gives; disable() leaves no monitor or timer behind;
* C1's language values (object form, "EN", unknown, broken JSON, H's file only) mean nl.
"""
from __future__ import annotations
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import unittest

HERE = Path(__file__).resolve().parent
G = HERE.parents[1]
RUN = HERE/"run/ext"
NODE = os.environ.get("NODE") or shutil.which("node") or "/home/robin/.local/node/bin/node"
FILES = ("extension.js", "panelBridge.js", "shortcutBridge.js", "desktopBridge.js", "notificationsBridge.js",
         "frostedBridge.js")
DUTCH = {w.strip() for w in (G/"tests/i18n/dutch-words.txt").read_text().splitlines()
         if w.strip() and not w.startswith("#")}
# en values that may hold Dutch on purpose: the host asked for the nl test notification (args.language "nl").
EN_DUTCH_OK = re.compile(r"^(after\.)?notificationTest\.nl$")
# Rows of tests/i18n/BASELINE-CHANGES.ext whose texts went with their methods: windows.hostState and
# windows.hostRestore, whose only caller was the WebKit apps' saved window state in the host (commit 6ebf169f).
# The dump no longer calls them; their GE1c rows stay in that file as history.
REMOVED_KEYS = frozenset({"failure.sessionLocked", "failure.badHost", "failure.hostUnmapped", "failure.hostUnpainted",
                          "failure.badGeometry", "failure.noDisplay", "failure.waitFrame"})


def baseline_extension() -> Path:
    target = RUN/"ext-3b898e6"
    target.mkdir(parents=True, exist_ok=True)
    for name in FILES:
        text = subprocess.run(["git", "-C", str(G), "show", f"3b898e6:extension/{name}"], capture_output=True,
                              text=True, check=True).stdout
        (target/name).write_text(text)
    return target


def config_dir(name: str, settings=None, raw: str | None = None, hyprland=None) -> Path:
    base = RUN/name
    shutil.rmtree(base, ignore_errors=True)
    config = base/"config"
    config.mkdir(parents=True)
    (base/"home").mkdir()
    if settings is not None or raw is not None:
        (config/"win11-gnome").mkdir(mode=0o700)
        (config/"win11-gnome/settings.json").write_text(raw if raw is not None else json.dumps(settings, indent=2) + "\n")
    if hyprland is not None:
        (base/"home/.config/win11shell").mkdir(parents=True)
        (base/"home/.config/win11shell/settings.json").write_text(json.dumps(hyprland))
    return config


def doc(language) -> dict:
    return {**json.loads((G/"host/src/settings-defaults.json").read_text()), "language": language}


def dump(extension: Path, config: Path, *args: str) -> dict[str, str]:
    proc = subprocess.run([NODE, "--import", "./register.mjs", "dump.mjs", str(extension), str(config), *args],
                          cwd=HERE/"ext", capture_output=True, text=True, timeout=60)
    if proc.returncode:
        raise AssertionError(f"dump failed ({proc.returncode}):\n{proc.stderr}")
    out = {}
    for line in proc.stdout.splitlines():
        key, _, value = line.partition("\t")
        out[key] = value
    return out


def message(value: str) -> str:
    """The shown text of a dump value ("code<TAB>message" for failures)."""
    return value.split("\t", 1)[1] if "\t" in value else value


def baseline_changes() -> dict[str, str]:
    rows = {}
    for line in (G/"tests/i18n/BASELINE-CHANGES.ext").read_text().splitlines():
        if not line or line.startswith("#"):
            continue
        fields = line.split("\t")
        if len(fields) >= 4 and fields[1].startswith("key:ext."):
            rows[fields[1][len("key:ext."):]] = fields[2].replace("\\t", "\t").replace("\\n", "\n")
    return {key: text for key, text in rows.items() if key not in REMOVED_KEYS}


class ExtensionText(unittest.TestCase):
    maxDiff = None

    @classmethod
    def setUpClass(cls):
        cls.old = baseline_extension()
        cls.new = G/"extension"
        cls.nl_old = dump(cls.old, config_dir("nl-old", doc("nl")))
        cls.nl = dump(cls.new, config_dir("nl", doc("nl")))
        cls.en = dump(cls.new, config_dir("en", doc("en")))

    def visible(self, data: dict) -> dict:
        return {k: v for k, v in data.items() if not k.startswith("meta.")}

    def test_dutch_differs_only_in_listed_keys(self):
        old, new = self.visible(self.nl_old), self.visible(self.nl)
        self.assertEqual(sorted(old), sorted(new))
        self.assertEqual(sorted(REMOVED_KEYS & set(new)), [])
        changed = {k: message(new[k]) for k in new if new[k] != old[k]}
        self.assertEqual(changed, baseline_changes())
        self.assertGreaterEqual(len(new), 56 - len(REMOVED_KEYS))
        # The failure codes never change, only their text.
        for key in new:
            if key.startswith("failure."):
                self.assertEqual(new[key].split("\t")[0], old[key].split("\t")[0], key)

    def test_english_has_no_dutch(self):
        hits = {k: sorted({w for w in re.findall(r"[a-zà-ÿ]+", message(v).lower()) if w in DUTCH})
                for k, v in self.visible(self.en).items() if not EN_DUTCH_OK.match(k)}
        self.assertEqual({k: v for k, v in hits.items() if v}, {})
        self.assertEqual(self.en["button.1"], "Search")
        self.assertEqual(self.en["clockFormat"], "%H:%M\\n%d/%m/%Y")
        self.assertEqual(self.en["clock"], "14:13\\n05/11/2026")
        self.assertEqual(self.en["ctrlAltTab"], "Win11 Taskbar")
        self.assertEqual(self.en["appInfo.alles"], "Alles")
        self.assertEqual(self.nl["appInfo.alles"], "Alles")
        self.assertEqual(self.en["notificationTest.none"], self.en["notificationTest.en"])
        self.assertEqual(self.nl["notificationTest.none"], self.nl["notificationTest.nl"])

    def test_every_key_differs_between_languages_where_expected(self):
        same = {k for k in self.visible(self.nl) if self.nl[k] == self.en[k]}
        self.assertEqual(sorted(same), sorted([
            "button.0", "appInfo.alles", "notificationTest.nl", "notificationTest.en", "notificationTest.nl.appName",
            "notificationTest.en.appName", "notificationTest.none.appName", "notificationUntitled.appName"]))

    def test_live_switch_equals_a_fresh_start(self):
        for name, start, target, args in [("switch-rename", "nl", "en", ()), ("switch-inplace", "nl", "en", ("--rewrite-in-place",)),
                                          ("switch-back", "en", "nl", ())]:
            with self.subTest(name):
                out = dump(self.new, config_dir(name, doc(start)), "--switch-to", target, *args)
                fresh = self.en if target == "en" else self.nl
                after = {k[len("after."):]: v for k, v in out.items() if k.startswith("after.")}
                # notificationOffline disables the bridge, so the dump takes it once, at its very end.
                self.assertEqual(after, {k: v for k, v in self.visible(fresh).items() if k != "failure.notificationOffline"})
                self.assertEqual(out["failure.notificationOffline"], fresh["failure.notificationOffline"])
                self.assertEqual(out["meta.monitorsAfterDisable"], "0")
                self.assertEqual(out["meta.timersAfterDisable"], "0")
                self.assertEqual(out["meta.monitors"], "win11-gnome:8")

    def test_slow_writer_is_read_once_after_its_last_event(self):
        """ge1c-review m4: a debounce, not a throttle. A writer that rewrites settings.json in place in three steps
        (half-written JSON first) restarts the 100 ms timer on each event; the one read then sees the whole file,
        so the bar goes straight to en (no flash of nl from broken JSON, no second re-label)."""
        out = dump(self.new, config_dir("burst", doc("nl")), "--switch-to", "en", "--burst")
        pending = [ids.split(",") for ids in out["meta.burstPending"].split(" ")]
        self.assertEqual([len(ids) for ids in pending], [1, 1, 1])             # always exactly one timer
        self.assertEqual(len({ids[0] for ids in pending}), 3)                  # and a new one for every event
        self.assertEqual(out["meta.burstReadsDuring"], "0")
        after = {k[len("after."):]: v for k, v in out.items() if k.startswith("after.")}
        self.assertEqual(after, {k: v for k, v in self.visible(self.en).items() if k != "failure.notificationOffline"})
        self.assertEqual(out["meta.timersAfterDisable"], "0")

    def test_folder_created_after_enable(self):
        out = dump(self.new, config_dir("create-later"), "--create-later", "en")
        self.assertEqual(out["button.1"], "Zoeken")
        self.assertEqual(out["after.button.1"], "Search")
        self.assertEqual(out["after.ctrlAltTab"], "Win11 Taskbar")
        self.assertEqual(out["after.clockFormat"], "%H:%M\\n%d/%m/%Y")
        self.assertEqual(out["meta.monitors"], "win11-gnome:8")        # the parent watch ended with the folder
        self.assertEqual(out["meta.monitorsAfterDisable"], "0")

    def test_language_values_and_sources(self):
        cases = [("object", doc({"custom": "future"}), None, None), ("upper", doc("EN"), None, None),
                 ("unknown", doc("fr"), None, None), ("missing-key", {"version": 1}, None, None),
                 ("broken", None, "{broken", None), ("no-file", None, None, None),
                 ("hyprland-only", None, None, {"language": "en"})]
        for name, settings, raw, hyprland in cases:
            with self.subTest(name):
                out = dump(self.new, config_dir(f"value-{name}", settings, raw, hyprland))
                self.assertEqual(out["button.1"], "Zoeken")
                self.assertEqual(out["failure.argsObject"], self.nl["failure.argsObject"])
        out = dump(self.new, config_dir("value-en", doc("en")))
        self.assertEqual(out["meta.reads"].split()[0], "win11-gnome/settings.json")

    def test_extension_creates_no_files(self):
        writes = re.compile(r"\b(replace_contents|make_directory|make_directory_with_parents|create_readwrite|"
                            r"set_contents|file_set_contents|\.create\(|\.replace\(null)")
        # D-G69: the one allowed write is AccentPreview's generated .tile-preview sheet in $XDG_RUNTIME_DIR/win11-gnome
        # (runtime only, never in the config or data dirs).
        allowed = {"extension.js": ["replace_contents"]}
        for path in sorted((G/"extension").glob("*.js")):
            text = path.read_text()
            self.assertEqual(writes.findall(text), allowed.get(path.name, []), path.name)
        self.assertIn("GLib.get_user_runtime_dir(), 'win11-gnome'", (G/"extension/extension.js").read_text())
        before = sorted(p.relative_to(RUN) for p in (RUN/"value-no-file").rglob("*"))
        dump(self.new, RUN/"value-no-file/config")
        self.assertEqual(sorted(p.relative_to(RUN) for p in (RUN/"value-no-file").rglob("*")), before)


if __name__ == "__main__":
    unittest.main(verbosity=2)
