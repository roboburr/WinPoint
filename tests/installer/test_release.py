#!/usr/bin/env python3
"""tools/release.py (D-G74a): the parts that need no build, no tag and no GitHub. Run by tests/installer/run.sh
(tests.py), next to the installer tests; every tree is a temporary folder and release.G points at it.

* the tarball self-test refuses a symbolic link that climbs above the extraction folder, also when it comes back
  into a folder that is only named like the top folder (a ".." is never clamped at the root);
* ``bump`` changes all three places or none (a Cargo.lock without the win11-host block leaves Cargo.toml alone);
* the build check compares DIR's programs with cargo's own files and takes cargo's times against the sources of
  their dep-info files, so a plain cp of an old build after a source edit no longer passes (and a test-only edit
  needs no rebuild that cargo would not do);
* the source check covers CHANGELOG.md, whose section of the tagged commit becomes the release notes;
* no signing (D-G74: no key, GitHub's digest is the trust anchor): no ``--key``, no key report in ``check``, and the
  printed publishing lines upload the tarball and its ``.sha256`` only, then list the assets with the digests to
  confirm.
"""
from __future__ import annotations

import importlib.util
import io
import os
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
        for name, text in (("Cargo.toml", CARGO_TOML), ("Cargo.lock", lock), ("extension/metadata.json", METADATA)):
            (self.dir/name).write_text(text)
            os.chmod(self.dir/name, 0o644)
        return {name: (self.dir/name).read_bytes() for name, _ in release.PLACES}

    def bump(self, version: str):
        with mock.patch.object(release, "G", self.dir), mock.patch("sys.stdout", io.StringIO()):
            return release.cmd_bump(Namespace(version=version))

    def test_all_three_places_or_none(self):
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
        self.assertEqual(sorted(p.name for p in self.dir.rglob(".*.tmp")), [], "no temporary file left")

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


class Source(Temp):
    def git(self, *args: str) -> None:
        env = {**os.environ, "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": os.devnull}
        subprocess.run(["git", "-C", str(self.dir), "-c", "user.name=test", "-c", "user.email=test@example.org",
                        "-c", "commit.gpgsign=false", *args], check=True, capture_output=True, env=env)

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
        lines = release.publish_commands("0.9.0", "winpoint-main", "dist/notes-0.9.0.md", assets, ["a" * 64, "b" * 64])
        self.assertEqual(lines[0], 'git tag -a v0.9.0 winpoint-main -m "WinPoint 0.9.0"')
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
