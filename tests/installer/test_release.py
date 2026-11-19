#!/usr/bin/env python3
"""tools/release.py (D-G74a): the parts that need no build, no tag and no GitHub. Run by tests/installer/run.sh
(tests.py), next to the installer tests; every tree is a temporary folder and release.G points at it.

* the tarball self-test refuses a symbolic link that climbs above the extraction folder, also when it comes back
  into a folder that is only named like the top folder (a ".." is never clamped at the root);
* ``bump`` changes all places (the README lines included) or none (a Cargo.lock without the win11-host block leaves
  Cargo.toml alone);
* the build check compares DIR's programs with cargo's own files and takes cargo's times against the sources of
  their dep-info files, so a plain cp of an old build after a source edit no longer passes (and a test-only edit
  needs no rebuild that cargo would not do);
* the source check covers CHANGELOG.md, whose section of the tagged commit becomes the release notes, and the two
  READMEs, whose install line the tarball carries;
* no signing (D-G74: no key, GitHub's digest is the trust anchor): no ``--key``, no key report in ``check``, and the
  printed publishing lines upload the tarball and its ``.sha256`` only, then list the assets with the digests to
  confirm;
* step 7 compares what lands on the system as the new install.py --check judges it, not the files' text: v0.9.0's
  own install-session.py and install-system.py (from git) in a repository of their own, where a change to a root
  script, a message or host/data/releases.json is no change, a session file is ("session"; for a script without
  stale(), as v0.9.0's, byte for byte, also one that differs only on the fake root with Ubuntu's shell mode) and so
  is one of host/data's four files ("system"); with the working tree's install-session.py (stale(): winpoint.json by
  meaning) the formatting of winpoint.json or the keys it copies from Ubuntu's mode are no change, another meaning
  is;
* the notes give the exact line to paste with the version filled in, and the checksum line; both run as written on
  a tarball laid out as the release's (its install.sh a stand-in) in a temporary HOME's download folder, named as
  Ubuntu names it in another language (xdg-user-dir);
* README.md's and README.nl.md's install line is the same and runs as written with an older download and an older
  unpacked folder next to the new tarball.
"""
from __future__ import annotations

import hashlib
import importlib.util
import io
import os
import re
import shutil
import subprocess
import tarfile
import tempfile
import time
import unittest
from argparse import Namespace
from pathlib import Path
from unittest import mock

G = Path(__file__).resolve().parents[2]
_spec = importlib.util.spec_from_file_location("release", G/"tools/release.py")
release = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(release)

TOP = "winpoint-9.9.9"
CARGO_TOML = '[workspace]\nmembers = ["host"]\n\n[workspace.package]\nversion = "0.9.0"\nedition = "2024"\n'
CARGO_LOCK = ('version = 4\n\n[[package]]\nname = "url"\nversion = "2.5.0"\n\n'
              '[[package]]\nname = "win11-host"\nversion = "0.9.0"\ndependencies = []\n')
METADATA = '{\n  "uuid": "win11@example",\n  "version": 5,\n  "version-name": "0.9.0"\n}\n'
README = ("## Install\n\nPaste this line:\n\n```bash\ncd \"$(xdg-user-dir DOWNLOAD)\" && tar -xzf "
          "winpoint-0.9.0-linux-x86_64.tar.gz && ./winpoint-0.9.0/scripts/install.sh\n```\n\nSee 0.9.0's notes.\n")


def tarball(path: Path, members: list[tuple[str, str, str]]) -> Path:
    """A .tar.gz of (name, kind, link) members: kind d(ir), f(ile), s(ymlink), h(ard link) or p (fifo)."""
    with tarfile.open(path, "w:gz") as tar:
        for name, kind, link in members:
            info = tarfile.TarInfo(name)
            info.type = {"d": tarfile.DIRTYPE, "f": tarfile.REGTYPE, "s": tarfile.SYMTYPE, "h": tarfile.LNKTYPE,
                         "p": tarfile.FIFOTYPE}[kind]
            info.linkname = link
            data = b"x" if kind == "f" else b""
            info.size = len(data)
            tar.addfile(info, io.BytesIO(data) if data else None)
    return path


class Temp(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="release-test-"))
        self.addCleanup(shutil.rmtree, self.dir, ignore_errors=True)


class Tarball(Temp):
    def test_a_link_out_of_the_top_folder_is_refused_also_back_into_a_namesake(self):
        good = [(TOP, "d", ""), (f"{TOP}/a", "d", ""), (f"{TOP}/a/b", "d", ""), (f"{TOP}/a/f", "f", ""),
                (f"{TOP}/a/link", "s", "f"), (f"{TOP}/a/b/up", "s", "../../c"), (f"{TOP}/here", "s", "./a/f"),
                (f"{TOP}/a/b/top", "s", "../../../" + TOP + "/a/f")]
        self.assertEqual(release.tarball_problems(tarball(self.dir/"good.tar.gz", good), TOP), [])
        bad = {
            f"{TOP}/a/b2": f"../../../{TOP}/c",                    # the review's case: D/../winpoint-9.9.9/c
            f"{TOP}/a/b/climb": f"../../../../{TOP}/c",             # the same from one folder deeper
            f"{TOP}/a/root": "../..",                             # D itself
            f"{TOP}/a/out": "../../other/c",
            f"{TOP}/abs": "/etc/passwd",
            f"{TOP}/far": "../../../../../../../../../../../../" + TOP,
        }
        members = good + [(name, "s", link) for name, link in bad.items()]
        members += [("other/x", "f", ""), (f"{TOP}/../evil", "f", ""), ("/abs", "f", ""),
                    (f"{TOP}/hard", "h", f"{TOP}/a/f"), (f"{TOP}/fifo", "p", "")]
        problems = release.tarball_problems(tarball(self.dir/"bad.tar.gz", members), TOP)
        flagged = sorted(p.split(": ", 1)[0] for p in problems)
        self.assertEqual(flagged, sorted([*bad, "other/x", f"{TOP}/../evil", "/abs", f"{TOP}/hard", f"{TOP}/fifo"]),
                         problems)
        self.assertTrue(any(p.startswith(f"{TOP}/a/b/climb: a symbolic link out of") for p in problems), problems)


class Bump(Temp):
    def tree(self, lock: str = CARGO_LOCK) -> dict[str, bytes]:
        (self.dir/"extension").mkdir(exist_ok=True)
        for name, text in (("Cargo.toml", CARGO_TOML), ("Cargo.lock", lock), ("extension/metadata.json", METADATA),
                           ("README.md", README), ("README.nl.md", README)):
            (self.dir/name).write_text(text)
            os.chmod(self.dir/name, 0o644)
        return {name: (self.dir/name).read_bytes() for name, _ in release.PLACES}

    def bump(self, version: str):
        with mock.patch.object(release, "G", self.dir), mock.patch("sys.stdout", io.StringIO()):
            return release.cmd_bump(Namespace(version=version))

    def test_all_places_or_none(self):
        # the fields of the win11-host block in another order: the pattern finds no version there
        before = self.tree(CARGO_LOCK.replace('name = "win11-host"\nversion = "0.9.0"\n',
                                              'version = "0.9.0"\nname = "win11-host"\n'))
        with self.assertRaises(SystemExit) as refused:
            self.bump("0.9.1")
        self.assertIn("Cargo.lock: expected one version, found 0", str(refused.exception))
        self.assertIn("nothing was changed", str(refused.exception))
        self.assertEqual({name: (self.dir/name).read_bytes() for name in before}, before, "a half-bumped tree")
        self.tree()
        self.assertEqual(self.bump("0.9.1"), 0)
        with mock.patch.object(release, "G", self.dir):
            self.assertEqual(set(release.versions().values()), {"0.9.1"})
        self.assertIn('[workspace.package]\nversion = "0.9.1"\nedition', (self.dir/"Cargo.toml").read_text())
        self.assertIn('name = "url"\nversion = "2.5.0"', (self.dir/"Cargo.lock").read_text(), "another package")
        for name, _ in release.PLACES:
            self.assertEqual((self.dir/name).stat().st_mode & 0o777, 0o644, name)
        self.assertEqual((self.dir/"README.md").read_text(), README.replace("winpoint-0.9.0", "winpoint-0.9.1"),
                         "both versions of the install line, nothing else (the prose's 0.9.0 stays)")
        self.assertEqual(sorted(p.name for p in self.dir.rglob(".*.tmp")), [], "no temporary file left")

    def test_the_readme_line_names_one_version(self):
        self.tree()
        (self.dir/"README.nl.md").write_text(README.replace("./winpoint-0.9.0/", "./winpoint-0.8.0/"))
        with mock.patch.object(release, "G", self.dir):
            self.assertEqual(release.versions()["README.nl.md"], "0.9.0 and 0.8.0")
            (self.dir/"CHANGELOG.md").write_text("## 0.9.0\n\n* first\n")
            self.assertIn("the versions differ: ", "; ".join(release.problems_of_check("0.9.0")))
        (self.dir/"README.md").write_text("## Install\n\nNo line.\n")
        before = {name: (self.dir/name).read_bytes() for name, _ in release.PLACES}
        with self.assertRaises(SystemExit) as refused:
            self.bump("0.9.1")
        self.assertIn("README.md: expected one version, found 0", str(refused.exception))
        self.assertEqual({name: (self.dir/name).read_bytes() for name, _ in release.PLACES}, before)

    def test_changelog_section_of_a_given_text(self):
        text = "# Changes\n\n## 0.9.1\n\n* fixed\n\n## 0.9.0\n\n* first\n"
        self.assertEqual(release.changelog_section("0.9.1", text), "* fixed\n")
        self.assertEqual(release.changelog_section("0.9.0", text), "* first\n")
        self.assertIsNone(release.changelog_section("0.9.2", text))


class Build(Temp):
    """check_build with two programs, the host's win11-panel and the GTK 4 win11-shell, in a checkout of its own: each
    target folder has cargo's dep-info file (NAME.d) naming the sources it was built from."""
    SOURCE_TIME = time.time() - 3600   # the newest edit of a source file, an hour ago

    def setUp(self):
        super().setUp()
        self.g = self.dir/"g"
        self.host, self.native, self.build = self.dir/"host-target", self.dir/"native-target", self.dir/"build"
        for folder in (self.host, self.native, self.build, self.g/"host/src", self.g/"native/shell/src"):
            folder.mkdir(parents=True)
        self.sources = {"win11-panel": self.g/"host/src/services/winpoint/check.rs",
                        "win11-shell": self.g/"native/shell/src/main.rs"}
        self.manifest = self.g/"Cargo.toml"
        self.write(self.host/"win11-panel", b"\x7fELF panel WinPoint 9.9.9\n")
        self.write(self.native/"win11-shell", b"\x7fELF shell")
        for name, target in (("win11-panel", self.host), ("win11-shell", self.native)):
            source = self.sources[name]
            source.parent.mkdir(parents=True, exist_ok=True)
            source.write_text("// source\n")
            os.utime(source, (self.SOURCE_TIME, self.SOURCE_TIME))
            (target/f"{name}.d").write_text(f"{target/name}: {self.g/'host/src/lib.rs'} {source}\n")
            self.touch(target/name, 60)
            shutil.copy(target/name, self.build)   # a plain copy: the time of the copy
        (self.g/"host/src/lib.rs").write_text("// lib\n")
        os.utime(self.g/"host/src/lib.rs", (self.SOURCE_TIME - 600, self.SOURCE_TIME - 600))
        self.manifest.write_text("[workspace]\n")
        os.utime(self.manifest, (self.SOURCE_TIME - 600, self.SOURCE_TIME - 600))

    @staticmethod
    def write(path: Path, data: bytes) -> None:
        path.write_bytes(data)
        os.chmod(path, 0o755)

    def touch(self, path: Path, after: float) -> None:
        os.utime(path, (self.SOURCE_TIME + after, self.SOURCE_TIME + after))

    def check(self):
        with mock.patch.object(release, "G", self.g), \
                mock.patch.object(release, "install_bins", return_value=(("win11-panel", "win11-shell"),
                                                                      ("win11-shell",))), \
                mock.patch.object(release, "manifests", return_value=[self.manifest]):
            release.check_build(self.build, "9.9.9", self.host, self.native)

    def refused(self) -> str:
        with self.assertRaises(SystemExit) as refused:
            self.check()
        return str(refused.exception)

    def test_a_current_build_passes(self):
        self.check()

    def test_an_old_build_copied_after_a_source_edit_is_stale(self):
        """The review's case: built at 10:00, check.rs edited at 10:30, the 10:00 programs copied at 10:40."""
        self.touch(self.host/"win11-panel", -1800)
        self.assertGreater((self.build/"win11-panel").stat().st_mtime, self.SOURCE_TIME, "the copy is newer")
        message = self.refused()
        self.assertIn("win11-panel in", message)
        self.assertIn("are older than host/src/services/winpoint/check.rs", message)

    def test_a_test_only_edit_needs_no_rebuild(self):
        """A file cargo did not build the program from (a #[cfg(test)] module) is newer: cargo would not relink."""
        tests = self.g/"host/src/services/winpoint/tests.rs"
        tests.write_text("// tests\n")
        self.touch(tests, 600)
        self.check()

    def test_a_manifest_or_a_gone_source_is_stale(self):
        self.touch(self.manifest, 600)
        self.assertIn("are older than Cargo.toml", self.refused())
        self.touch(self.manifest, -600)
        self.sources["win11-shell"].unlink()
        self.assertIn("are older than native/shell/src/main.rs (or it is gone)", self.refused())

    def test_a_build_of_another_checkout_is_refused(self):
        (self.host/"win11-panel.d").write_text(f"{self.host/'win11-panel'}: /elsewhere/host/src/lib.rs\n")
        self.assertIn("was not built from", self.refused())
        (self.host/"win11-panel.d").unlink()
        self.assertIn("has no win11-panel.d", self.refused())

    def test_a_program_that_is_not_cargos_is_refused(self):
        self.write(self.build/"win11-shell", b"\x7fELF another shell")
        self.assertIn("win11-shell in", self.refused())
        self.assertIn("are not the ones in", self.refused())

    def test_a_missing_target_names_its_option(self):
        (self.native/"win11-shell").unlink()
        self.assertIn("--native-target", self.refused())

    def test_the_marker_is_still_read(self):
        for path in (self.host/"win11-panel", self.build/"win11-panel"):
            self.write(path, b"\x7fELF panel WinPoint 9.9.8\n")
            self.touch(path, 60)
        self.assertIn("has no 'WinPoint 9.9.9' marker", self.refused())

    def test_a_path_with_a_space_in_the_dep_info(self):
        spaced = self.g/"host/src/with space.rs"
        spaced.write_text("//\n")
        self.touch(spaced, 600)
        escaped = str(spaced).replace(" ", "\\ ")
        (self.host/"win11-panel.d").write_text(f"{self.host/'win11-panel'}: {self.sources['win11-panel']} {escaped}\n")
        self.assertIn("are older than host/src/with space.rs", self.refused())


class Repo(Temp):
    """A temporary folder that is a git repository of its own."""

    def git(self, *args: str) -> None:
        env = {**os.environ, "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": os.devnull}
        subprocess.run(["git", "-C", str(self.dir), "-c", "user.name=test", "-c", "user.email=test@example.org",
                        "-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false", *args], check=True,
                       capture_output=True, env=env)


class Source(Repo):

    def test_changelog_md_is_part_of_the_source(self):
        (self.dir/"host").mkdir()
        (self.dir/"host/x.rs").write_text("fn main() {}\n")
        (self.dir/"CHANGELOG.md").write_text("# Changes\n\n## 0.9.0\n\n* first\n")
        self.git("init", "-q", "-b", "main")
        self.git("add", ".")
        self.git("commit", "-q", "-m", "one")
        with mock.patch.object(release, "G", self.dir):
            self.assertEqual(len(release.check_source("main")), 40)
            (self.dir/"CHANGELOG.md").write_text("# Changes\n\n## 0.9.1\n\n* not published\n\n## 0.9.0\n\n* first\n")
            with self.assertRaises(SystemExit) as refused:
                release.check_source("main")
        self.assertIn("CHANGELOG.md", str(refused.exception))
        self.assertIn("commit and publish first", str(refused.exception))

    def test_the_readmes_are_part_of_the_source(self):
        """bump's two README places: a line fixed after the commit (or not committed with the bump) is refused, so the
        tagged commit, the tarball and GitHub main carry the line check read."""
        for name in ("README.md", "README.nl.md"):
            (self.dir/name).write_text(README)
        self.git("init", "-q", "-b", "main")
        self.git("add", ".")
        self.git("commit", "-q", "-m", "one")
        with mock.patch.object(release, "G", self.dir):
            self.assertEqual(len(release.check_source("main")), 40)
            for name in ("README.md", "README.nl.md"):
                with self.subTest(name=name):
                    (self.dir/name).write_text(README.replace("0.9.0", "0.9.1"))
                    with self.assertRaises(SystemExit) as refused:
                        release.check_source("main")
                    self.assertIn(name, str(refused.exception))
                    (self.dir/name).write_text(README)


class SystemFiles(Repo):
    """Step 7 on v0.9.0's own install-session.py, install-system.py and host/data, tagged v0.9.0 in a repository of
    their own; each case changes that tree and compares v0.9.0 with the commit."""
    TAG = "v0.9.0"
    FILES = ("scripts/install-session.py", "scripts/install-system.py", "host/data/releases.json",
             "host/data/release-upgrades-helper", "host/data/release-upgrade-attach",
             "host/data/io.github.win11shell.gnome.release-upgrades.policy",
             "host/data/io.github.win11shell.gnome.release-upgrade-attach.policy")

    def setUp(self):
        super().setUp()
        self.original = {}
        for rel in self.FILES:
            shown = subprocess.run(["git", "-C", str(G), "show", f"{self.TAG}:{rel}"], capture_output=True)
            if shown.returncode:
                self.skipTest(f"this checkout has no {self.TAG}:{rel}")
            self.original[rel] = shown.stdout
        self.git("init", "-q", "-b", "main")
        self.commit({})
        self.git("tag", self.TAG)

    def commit(self, edits: dict[str, list[tuple[str, str]]]) -> None:
        """v0.9.0's files with `edits` ({file: [(old, new), ...]}, each old exactly once), committed."""
        for rel, data in self.original.items():
            text = data.decode()
            for old, new in edits.get(rel, ()):
                self.assertEqual(text.count(old), 1, f"{rel}: {old!r}")
                text = text.replace(old, new)
            (self.dir/rel).parent.mkdir(parents=True, exist_ok=True)
            (self.dir/rel).write_text(text)
        self.git("add", "-A")
        self.git("commit", "-q", "--allow-empty", "-m", "change")

    def changes(self, edits: dict[str, list[tuple[str, str]]]) -> list[str]:
        self.commit(edits)
        with mock.patch.object(release, "G", self.dir):
            return release.system_changes(self.TAG, "HEAD")

    def current(self, edits: list[tuple[str, str]] = ()) -> list[str]:
        """changes() with the working tree's install-session.py (0.9.1's: stale(), winpoint.json by meaning) and
        `edits` on it as the commit's."""
        self.original["scripts/install-session.py"] = (G/"scripts/install-session.py").read_bytes()
        return self.changes({"scripts/install-session.py": list(edits)})

    def test_v0_9_0_against_itself(self):
        self.assertEqual(self.changes({}), [])

    def test_a_root_script_a_message_or_releases_json_is_no_change(self):
        """The case of 0.9.1: the root step changes, the four session files stay; the old whole-file diff warned."""
        edits = {
            "scripts/install-session.py": [
                ('echo "@@done"\n"""\nREMOVE_SCRIPT', 'echo "@@system"\necho "@@done"\n"""\nREMOVE_SCRIPT'),
                ("Ubuntu now asks for your password once.", "Ubuntu now asks for one password."),
                ('INSTALL_NAME = "win11-gnome-session-install"', 'INSTALL_NAME = "win11-gnome-install"'),
            ],
            "scripts/install-system.py": [('REMOVE_NAME = "win11-gnome-system-remove"',
                                           'REMOVE_NAME = "win11-gnome-remove"')],
            "host/data/releases.json": [('{"version":1,', '{"version":1,"x":1,')],
        }
        self.assertEqual(self.changes(edits), [])
        old_rule = subprocess.run(["git", "-C", str(self.dir), "diff", "--quiet", self.TAG, "HEAD", "--", "host/data",
                                   "scripts/install-session.py"])
        self.assertEqual(old_rule.returncode, 1, "the whole-file diff would have warned")

    SESSION_NAME = ('b"[GNOME Session]\\nName=WinPoint\\n"', 'b"[GNOME Session]\\nName=WinPoint 2\\n"')

    def test_a_session_file_is_a_change(self):
        self.assertEqual(self.changes({"scripts/install-session.py": [self.SESSION_NAME]}), ["session"])

    def test_a_change_only_on_the_root_with_ubuntus_mode_counts(self):
        """winpoint.json without the icons key of Ubuntu's mode: the same on a system without ubuntu.json. v0.9.0's
        script has no stale(), so the bytes count (its --check compared them)."""
        self.assertEqual(self.changes({"scripts/install-session.py": [
            ('("stylesheetName", "themeResourceName", "iconsResourceName")',
             '("stylesheetName", "themeResourceName")')]}), ["session"])
        # the same commits with a mode of Ubuntu's that has none of the keys: only that root showed the change
        with mock.patch.object(release, "UBUNTU_MODE", {"parentMode": "user"}), \
                mock.patch.object(release, "G", self.dir):
            self.assertEqual(release.system_changes(self.TAG, "HEAD"), [])

    def test_the_current_tree_judges_winpoint_json_by_meaning(self):
        """With 0.9.1's install-session.py as the commit's: its four files are v0.9.0's (no change, the 0.9.1 case);
        winpoint.json written with another indent, or without a key it copies from Ubuntu's mode, is the same to
        stale() (MODE_KEYS), so --check reports nothing and neither does step 7; another meaning (one more extension)
        or another session file is a change. The same indent edit on v0.9.0's script (no stale()) is one."""
        self.assertEqual(self.current(), [])
        indent = ('return json.dumps(mode, indent=4) + "\\n"', 'return json.dumps(mode, indent=2) + "\\n"')
        self.assertEqual(self.current([indent]), [])
        self.assertEqual(self.current([('for key in ("stylesheetName", "themeResourceName", "iconsResourceName"):',
                                        'for key in ("stylesheetName", "themeResourceName"):')]), [])
        self.assertEqual(self.current([('mode["enabledExtensions"] = [UUID]',
                                        'mode["enabledExtensions"] = [UUID, "other@ge1c"]')]), ["session"])
        self.assertEqual(self.current([self.SESSION_NAME]), ["session"])
        self.original["scripts/install-session.py"] = subprocess.run(
            ["git", "-C", str(G), "show", f"{self.TAG}:scripts/install-session.py"], capture_output=True, check=True).stdout
        self.assertEqual(self.changes({"scripts/install-session.py": [indent]}), ["session"])

    def test_a_system_file_is_a_change_byte_for_byte(self):
        attach = "host/data/release-upgrade-attach"
        last = self.original[attach].decode().rstrip("\n").rsplit("\n", 1)[1]
        self.assertEqual(self.changes({attach: [(last + "\n", last + " \n")]}), ["system"])
        self.assertEqual(self.changes({attach: [(last + "\n", last + " \n")],
                                       "scripts/install-session.py": [self.SESSION_NAME]}), ["session", "system"])

    def test_a_file_v0_9_0_did_not_install_is_no_change(self):
        """A fifth file of install-system.py: no PC has it from v0.9.0, so --check finds nothing installed."""
        self.original["host/data/new-helper"] = b"#!/bin/sh\n"
        self.assertEqual(self.changes({"scripts/install-system.py": [
            ('    ("release-upgrade-attach", LIBEXEC, 0o755),\n',
             '    ("release-upgrade-attach", LIBEXEC, 0o755),\n    ("new-helper", LIBEXEC, 0o755),\n')]}), [])

    def test_a_script_that_does_not_load_is_refused_by_name(self):
        with self.assertRaises(SystemExit) as refused:
            self.changes({"scripts/install-session.py": [('def contents(root: Path = Path("/"))',
                                                          "def contents(root, lang)")]})
        self.assertIn("HEAD's install-session.py or install-system.py: TypeError", str(refused.exception))
        # a stale() that fails on a root: refused by name too
        with self.assertRaises(SystemExit) as refused:
            self.current([("    wanted = contents(root)\n", "    wanted = contents(root, 1)\n")])
        self.assertIn("HEAD's install-session.py: TypeError", str(refused.exception))


STAND_IN = b"#!/bin/sh\nprintf 'ran %s in %s with %s args\\n' \"$0\" \"$(pwd)\" \"$#\"\n"


def download_folder(home: Path) -> Path:
    """HOME's download folder as Ubuntu names it for a French user (~/.config/user-dirs.dirs), made."""
    downloads = home/"Téléchargements"
    downloads.mkdir(parents=True)
    (home/".config").mkdir()
    (home/".config/user-dirs.dirs").write_text('XDG_DOWNLOAD_DIR="$HOME/Téléchargements"\n')
    return downloads


def release_tarball(folder: Path, name: str, top: str) -> Path:
    """`name` in `folder`, laid out as a release's: top/scripts/install.sh, a stand-in that says how it ran."""
    with tarfile.open(folder/name, "w:gz") as tar:
        for member, kind in ((top, tarfile.DIRTYPE), (f"{top}/scripts", tarfile.DIRTYPE),
                             (f"{top}/scripts/install.sh", tarfile.REGTYPE)):
            info = tarfile.TarInfo(member)
            info.type, info.mode = kind, 0o755
            info.size = len(STAND_IN) if kind == tarfile.REGTYPE else 0
            tar.addfile(info, io.BytesIO(STAND_IN) if kind == tarfile.REGTYPE else None)
    return folder/name


class Notes(Temp):
    NAME, TOP = "winpoint-9.9.9-linux-x86_64.tar.gz", "winpoint-9.9.9"

    @staticmethod
    def blocks(text: str) -> list[str]:
        return re.findall(r"^```bash\n(.*?)\n```$", text, re.M | re.S)

    def test_the_exact_line_with_the_version_and_the_checksum_line(self):
        notes = release.release_notes("* fixed\n", self.NAME, self.TOP, [])
        self.assertTrue(notes.startswith("* fixed\n\n**Install**: download `winpoint-9.9.9-linux-x86_64.tar.gz` "),
                        notes)
        self.assertEqual(self.blocks(notes), [
            'cd "$(xdg-user-dir DOWNLOAD)" && tar -xzf winpoint-9.9.9-linux-x86_64.tar.gz && '
            './winpoint-9.9.9/scripts/install.sh',
            'cd "$(xdg-user-dir DOWNLOAD)" && sha256sum -c winpoint-9.9.9-linux-x86_64.tar.gz.sha256'])
        self.assertIn("password once", notes)
        self.assertIn("**Download and install** in Settings › Update & recovery does it for you.", notes)
        self.assertNotIn("—", notes)
        manual = release.release_notes("* fixed\n", self.NAME, self.TOP, ["session"])
        self.assertIn("This release changes WinPoint’s system files: install it with this line", manual)
        self.assertNotIn("does it for you", manual)
        self.assertEqual(self.blocks(manual), self.blocks(notes))

    def test_both_lines_run_as_written(self):
        """A tarball laid out as the release's, with a stand-in install.sh, and its .sha256 as package.py writes it,
        in a download folder that Ubuntu named in French (user-dirs.dirs, which xdg-user-dir reads)."""
        if not shutil.which("xdg-user-dir"):
            self.skipTest("no xdg-user-dir (package xdg-user-dirs)")
        home = self.dir/"home"
        downloads = download_folder(home)
        release_tarball(downloads, self.NAME, self.TOP)
        digest = hashlib.sha256((downloads/self.NAME).read_bytes()).hexdigest()
        (downloads/f"{self.NAME}.sha256").write_text(f"{digest}  {self.NAME}\n")
        install, check = self.blocks(release.release_notes("* fixed\n", self.NAME, self.TOP, []))
        env = {"PATH": "/usr/bin:/bin", "HOME": str(home), "LANG": "C.UTF-8"}
        ok = subprocess.run(["/bin/bash", "-c", check], env=env, cwd=self.dir, capture_output=True, text=True)
        self.assertEqual((ok.returncode, ok.stdout), (0, f"{self.NAME}: OK\n"), ok.stderr)
        ran = subprocess.run(["/bin/bash", "-c", install], env=env, cwd=self.dir, capture_output=True, text=True)
        self.assertEqual((ran.returncode, ran.stdout), (0, f"ran ./{self.TOP}/scripts/install.sh in {downloads} with 0 "
                                                           "args\n"), ran.stderr)
        (downloads/self.NAME).write_bytes(b"not the tarball")
        bad = subprocess.run(["/bin/bash", "-c", check], env=env, cwd=self.dir, capture_output=True, text=True)
        self.assertNotEqual(bad.returncode, 0)


class Readme(Temp):
    """The install line of README.md and README.nl.md: the same in both and the release notes' line for this version
    (bump keeps it current), pasted as it is in a download folder of another language, also with an older tarball
    and an older unpacked folder next to it; and the uninstall line works without ~/.local/bin on PATH."""

    @staticmethod
    def section(readme: str, heading: str) -> list[str]:
        text = (G/readme).read_text(encoding="utf-8")
        part = re.search(rf"^## {heading}\n(.*?)(?=^## |\Z)", text, re.M | re.S).group(1)
        return Notes.blocks(part)

    def test_the_install_line_is_the_notes_line_and_runs(self):
        if not shutil.which("xdg-user-dir"):
            self.skipTest("no xdg-user-dir (package xdg-user-dirs)")
        (line,) = self.section("README.md", "Install")
        self.assertEqual(self.section("README.nl.md", "Installeren"), [line])
        version = release.versions()["Cargo.toml"]
        name, top = f"winpoint-{version}-linux-x86_64.tar.gz", f"winpoint-{version}"
        self.assertEqual(line, release.install_line(name, top))
        # the neighbour: an older release (the previous patch, minor or major), as after a bump
        major, minor, patch = map(int, version.split("."))
        older = f"{major}.{minor}.{patch - 1}" if patch else f"{major}.{minor - 1}.9" if minor else f"{major - 1}.9.9"
        home = self.dir/"home"
        downloads = download_folder(home)
        old = release_tarball(downloads, f"winpoint-{older}-linux-x86_64.tar.gz", f"winpoint-{older}")
        with tarfile.open(old) as tar:
            tar.extractall(downloads, filter="tar")
        os.utime(old, (time.time() - 86400, time.time() - 86400))
        release_tarball(downloads, name, top)
        env = {"PATH": "/usr/bin:/bin", "HOME": str(home), "LANG": "C.UTF-8"}
        ran = subprocess.run(["/bin/bash", "-c", line], env=env, cwd=self.dir, capture_output=True, text=True)
        self.assertEqual((ran.returncode, ran.stdout),
                         (0, f"ran ./{top}/scripts/install.sh in {downloads} with 0 args\n"), ran.stderr)

    def test_the_uninstall_line_needs_no_path(self):
        for readme, heading in (("README.md", "Uninstall"), ("README.nl.md", "Verwijderen")):
            self.assertEqual(self.section(readme, heading), ["~/.local/bin/winpoint-uninstall"], readme)


class Unsigned(unittest.TestCase):
    """D-G74: releases carry no signature; WinPoint's updater trusts GitHub's digest of the tarball."""

    def test_no_key_option(self):
        with mock.patch("sys.stderr", io.StringIO()) as err, self.assertRaises(SystemExit) as refused:
            release.main(["build", "0.9.0", "--build-dir", "x", "--key", "k"])
        self.assertEqual(refused.exception.code, 2)
        self.assertIn("unrecognized arguments: --key", err.getvalue())

    def test_check_reports_no_release_key(self):
        with mock.patch("sys.stdout", io.StringIO()) as out:
            release.cmd_check(Namespace(version=None))
        self.assertNotIn("key", out.getvalue())
        self.assertFalse(hasattr(release, "sign") or hasattr(release, "SIGNERS"), "no signing code left")

    def test_two_assets_and_their_digests(self):
        assets = ["dist/winpoint-0.9.0-linux-x86_64.tar.gz", "dist/winpoint-0.9.0-linux-x86_64.tar.gz.sha256"]
        lines = release.publish_commands("0.9.0", "main", "dist/notes-0.9.0.md", assets, ["a" * 64, "b" * 64])
        self.assertEqual(lines[0], 'git tag -a v0.9.0 main -m "WinPoint 0.9.0"')
        self.assertEqual(lines[1], "git push origin v0.9.0")
        self.assertTrue(lines[2].startswith("gh release create v0.9.0 --repo roboburr/WinPoint --draft --verify-tag "))
        self.assertEqual(lines[4], "   " + " ".join(assets), "exactly the tarball and its .sha256")
        self.assertNotIn(".sig", "\n".join(lines))
        # the digests are confirmed on the draft, while its assets can still be replaced; publishing comes last
        view = lines[5]
        self.assertTrue(view.startswith("gh release view v0.9.0 --repo roboburr/WinPoint --json assets   # 2 assets;"), view)
        self.assertIn("winpoint-0.9.0-linux-x86_64.tar.gz sha256:" + "a" * 64, view)
        self.assertIn("winpoint-0.9.0-linux-x86_64.tar.gz.sha256 sha256:" + "b" * 64, view)
        self.assertIn("publish only when every one matches", view)
        self.assertEqual(lines[6], "gh release edit v0.9.0 --repo roboburr/WinPoint --draft=false")
        self.assertEqual(len(lines), 7)


if __name__ == "__main__":
    unittest.main()
