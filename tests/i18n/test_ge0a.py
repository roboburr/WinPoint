"""The i18n checker and the private session of Win11 voor GNOME (GE0a, notes/claude/round0/ge0a.md; ported from H's
tests/i18n/test_e0b.py, dropping its Python-host, bash, audit and transcript parts).

    python3 -m unittest tests/i18n/test_ge0a.py -v            (from the project root; a few seconds)

* the checker (tools/i18n/check.py): no errors, and its native layer read the GTK 4 programs' texts;
* dutch-words.txt is a sorted list of lower-case words;
* the private session (tests/i18n/private.sh) refuses a work dir outside tests/ and hands its command a clean
  environment: no display, no live bus, private HOME and XDG dirs, keyfile GSettings, NO_AT_BRIDGE=1.

Until the WebKit removal this file also held the UI ratchet, the mock coverage of the HTML apps, the nl/en UI text
dumps (baseline-nl.txt, baseline-en.txt of surfaces.json, BASELINE-CHANGES) and the dump server; they went with
the HTML apps (git show last-webkit:tests/i18n/test_ge0a.py).

Temporary files go to tests/i18n/.run/ (never /tmp: notes/claude/plan.md).
"""

from __future__ import annotations

import contextlib
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
HERE = ROOT / "tests" / "i18n"
TOOLS = ROOT / "tools" / "i18n"
RUN = HERE / ".run"
sys.path.insert(0, str(TOOLS))

import check  # noqa: E402


def tmpdir(prefix: str) -> tempfile.TemporaryDirectory:
    RUN.mkdir(exist_ok=True)
    return tempfile.TemporaryDirectory(prefix=prefix, dir=RUN)


def tearDownModule() -> None:
    with contextlib.suppress(OSError):
        RUN.rmdir()                  # only when empty


class Checker(unittest.TestCase):
    def test_no_errors(self):
        rep = check.run(ROOT)
        self.assertEqual([i.text() for i in rep.errors], [])
        self.assertIn("native", rep.stats.get("layers", "").split(","))
        self.assertGreater(rep.stats.get("native_pairs", 0), 3000)
        self.assertGreater(rep.stats.get("native_files", 0), 150)


class Data(unittest.TestCase):
    def test_dutch_words(self):
        raw = (HERE / "dutch-words.txt").read_text(encoding="utf-8").split("\n")
        words = [w for w in raw if w and not w.startswith("#")]
        self.assertGreater(len(words), 1000)
        self.assertEqual(words, sorted(set(words)), "sorted and unique")
        for w in words:
            self.assertEqual(w, w.lower(), w)
            self.assertRegex(w, r"^[a-zà-öø-ÿ]+('[a-zë]+)?$", w)


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


if __name__ == "__main__":
    unittest.main()
