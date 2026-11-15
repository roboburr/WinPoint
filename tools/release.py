#!/usr/bin/env python3
"""WinPoint's releases on GitHub (D-G74a; the updater design §3): the version in its places, the checks, and the
local half of a release. Standard library only; run it from the checkout.

    python3 -B tools/release.py bump X.Y.Z
    python3 -B tools/release.py check [X.Y.Z]
    python3 -B tools/release.py build X.Y.Z --build-dir DIR [--host-target T] [--native-target T] [--source REV]
                                  [--allow-os] [--offline]

* ``bump`` rewrites the three places of the version and needs no cargo: the root Cargo.toml's ``[workspace.package]
  version`` (the one source; the host inherits it), the ``win11-host`` block of Cargo.lock (scripts/build.sh builds
  with ``--locked``) and ``version-name`` in extension/metadata.json (GNOME's integer ``version`` is its own counter
  and stays). It finds all three first and writes nothing unless each has exactly one version (each file is then
  replaced atomically). Add the CHANGELOG.md section and build again afterwards.
* ``check`` says whether those three agree (and equal X.Y.Z when it is given) and whether CHANGELOG.md has a
  ``## X.Y.Z`` section.
* ``build`` does everything that stays on this machine and **prints** the publishing commands; it never tags, pushes,
  uploads or contacts GitHub except for one ``git ls-remote`` of the tag (``--offline`` skips it):
  1. ``check X.Y.Z``; no tag ``vX.Y.Z`` here or (``git ls-remote``) on origin;
  2. the source is the commit to be tagged: ``--source`` (default ``winpoint-main``, the branch of the publish
     recipe's clean commits, whose tree equals the local HEAD's) has the working tree's host, native, scripts,
     extension, vendor, Cargo files and CHANGELOG.md (``git diff --quiet``; untracked files do not count), and its
     own CHANGELOG.md has the ``## X.Y.Z`` section (the release notes are taken from there, the text of the tag);
  3. the build is current: the 19 programs of scripts/install.py BINS in DIR, each byte for byte the one cargo
     left in its target folder (``--host-target`` for the host's, ``--native-target`` for the GTK 4 ones, NATIVE_BINS;
     both default to ``$CARGO_TARGET_DIR/release``, else ``target/release``, where scripts/build.sh puts all 19); each
     of cargo's files there built from this checkout and none older than a file of its dep-info (``NAME.d``: the
     sources cargo built it from, so a test-only edit needs no rebuild) or a Cargo manifest of its workspace (these
     are cargo's times: a plain ``cp`` into DIR stamps the copy with the time of the copy); and DIR/win11-panel
     carries the marker ``WinPoint X.Y.Z\\n`` (read, never run);
  4. the build machine is a supported Ubuntu (``SUPPORTED``) with a GNOME the extension lists in ``shell-version``
     (``--allow-os`` overrides: a release built on a newer Ubuntu is the main way to break users);
  5. ``git archive REV`` unpacked in a temporary folder, then its own scripts/package.py with ``WIN11_BUILD_DIR=DIR``
     and ``WIN11_PACKAGE_DIR=dist``: ``dist/winpoint-X.Y.Z-linux-ARCH.tar.gz`` and its ``.sha256``;
  6. the tarball's self-test with the update check's rules: one top folder ``winpoint-X.Y.Z/``, no hard links,
     devices or fifos, relative symbolic links that stay inside it; the ``.sha256`` names the archive and matches;
  7. a warning when host/data or scripts/install-session.py changed since the previous ``v*`` tag: machines with
     those system files installed must install that release by hand (it goes into the notes);
  8. ``dist/notes-X.Y.Z.md`` from that commit's CHANGELOG section, then the ``git tag``, ``git push`` and ``gh release``
     lines: the two assets go up to a draft, the next line lists them with the digest GitHub computed for each, which
     must be the printed ``sha256:`` of the tarball (and of its ``.sha256``) before the last line publishes the draft
     (with immutable releases a published asset can no longer be replaced; a draft's can, with ``gh release upload
     --clobber`` or ``gh release delete-asset``).

There is no signature and no signing key (D-G74, Robin 14-11: "Zonder sleutel, via GitHub"): WinPoint's updater
trusts GitHub, over validated TLS, and the releases API's ``digest`` of the tarball, which must equal the tarball's
SHA-256 and the ``.sha256`` asset. Immutable releases (the repository's settings) keep published assets from being
swapped. The names are frozen from 0.9.0 on (WinPoint's updater looks for them): tag ``vX.Y.Z`` (no prerelease
part), assets ``winpoint-X.Y.Z-linux-ARCH.tar.gz`` and ``.tar.gz.sha256``, top folder ``winpoint-X.Y.Z/``. Exit 0
when everything held, 1 with the reason otherwise.
"""

from __future__ import annotations

import argparse
import filecmp
import hashlib
import io
import json
import os
import platform
import re
import subprocess
import sys
import tarfile
import tempfile
import tomllib
from pathlib import Path, PurePosixPath

G = Path(__file__).resolve().parent.parent
REPO = "roboburr/WinPoint"
PUBLISHED = "winpoint-main"
SUPPORTED = ("26.04",)
SOURCE_PATHS = ("host", "native", "scripts", "extension", "vendor", "Cargo.toml", "Cargo.lock", "CHANGELOG.md")
SYSTEM_PATHS = ("host/data", "scripts/install-session.py")
VERSION_RE = re.compile(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)")
# the three places of the version (bump rewrites the second group of each, exactly once)
PLACES = (
    ("Cargo.toml", re.compile(r'(\[workspace\.package\]\n(?:[^\[\n][^\n]*\n|\n)*?version = ")([^"]*)(")')),
    ("Cargo.lock", re.compile(r'(\[\[package\]\]\nname = "win11-host"\nversion = ")([^"]*)(")')),
    ("extension/metadata.json", re.compile(r'("version-name": ")([^"]*)(")')),
)


def fail(message: str) -> None:
    raise SystemExit(f"release: {message}")


def version_arg(text: str) -> str:
    if not VERSION_RE.fullmatch(text):
        raise argparse.ArgumentTypeError(f"{text!r} is not X.Y.Z (no prerelease part, no leading zeros)")
    return text


def git(*args: str, check: bool = True, **kw) -> subprocess.CompletedProcess:
    return subprocess.run(["git", "-C", str(G), *args], check=check, text=kw.pop("text", True),
                          capture_output=kw.pop("capture_output", True), **kw)


def versions() -> dict[str, str | None]:
    """Each place's version (None when a place has none)."""
    out = {}
    for name, pattern in PLACES:
        match = pattern.search((G / name).read_text(encoding="utf-8"))
        out[name] = match.group(2) if match else None
    return out


def changelog_section(version: str, text: str | None = None) -> str | None:
    """The text of CHANGELOG.md's ``## version`` section (up to the next ``## ``), or None; `text` is another
    CHANGELOG.md (a commit's) instead of the working tree's."""
    if text is None:
        try:
            text = (G / "CHANGELOG.md").read_text(encoding="utf-8")
        except OSError:
            return None
    match = re.search(rf"^## {re.escape(version)}[ \t]*\n(.*?)(?=^## |\Z)", text, re.M | re.S)
    return match.group(1).strip() + "\n" if match else None


# ------------------------------------------------------------------------------------------ bump and check

def write_atomic(path: Path, text: str) -> None:
    """`text` into `path` through a temporary file in the same folder (its mode kept) and a rename."""
    mode = path.stat().st_mode & 0o7777
    fd, temp = tempfile.mkstemp(prefix=f".{path.name}.", suffix=".tmp", dir=path.parent)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as out:
            out.write(text)
        os.chmod(temp, mode)
        os.replace(temp, path)
    except BaseException:
        Path(temp).unlink(missing_ok=True)
        raise


def cmd_bump(args) -> int:
    old = versions()
    # every place first: a tree is bumped whole or not at all
    changed, problems = [], []
    for name, pattern in PLACES:
        path = G / name
        text = path.read_text(encoding="utf-8")
        new, count = pattern.subn(lambda m: m.group(1) + args.version + m.group(3), text)
        if count != 1:
            problems.append(f"{name}: expected one version, found {count}")
        elif new != text:
            changed.append((path, new))
    if problems:
        fail("; ".join(problems) + " (nothing was changed)")
    for path, new in changed:
        write_atomic(path, new)
    print(f"version {', '.join(sorted({v or '?' for v in old.values()}))} -> {args.version} in "
          f"{', '.join(name for name, _ in PLACES)}")
    if not changelog_section(args.version):
        print(f"next: add a '## {args.version}' section to CHANGELOG.md, then build again (scripts/build.sh)")
    return 0


def problems_of_check(version: str | None) -> list[str]:
    found = versions()
    problems = [f"{name} has no version" for name, value in found.items() if value is None]
    values = {value for value in found.values() if value}
    if len(values) > 1:
        problems.append("the versions differ: " + ", ".join(f"{name} {value}" for name, value in found.items()))
    elif version and values != {version}:
        problems.append(f"the version is {', '.join(values) or 'missing'}, not {version}")
    wanted = version or next(iter(values), None)
    if wanted and not changelog_section(wanted):
        problems.append(f"CHANGELOG.md has no '## {wanted}' section")
    return problems


def cmd_check(args) -> int:
    problems = problems_of_check(args.version)
    found = versions()
    print(f"version: {found['Cargo.toml']} (Cargo.toml, Cargo.lock {found['Cargo.lock']}, metadata.json "
          f"{found['extension/metadata.json']})")
    for problem in problems:
        print(f"FAIL {problem}")
    print("check: " + ("OK" if not problems else f"{len(problems)} problem(s)"))
    return 0 if not problems else 1


# ------------------------------------------------------------------------------------------ build

def install_bins() -> tuple[tuple[str, ...], tuple[str, ...]]:
    """scripts/install.py's BINS (the 19 programs of a payload) and NATIVE_BINS (those built in native/)."""
    sys.path.insert(0, str(G / "scripts"))
    import install
    return tuple(install.BINS), tuple(install.NATIVE_BINS)


def manifests(native: bool) -> list[Path]:
    """The Cargo files of a workspace (the root's for the host, native/'s): cargo rebuilds when one changes, but its
    dep-info files do not list them."""
    if not native:
        return [G / "Cargo.toml", G / "Cargo.lock", G / "host" / "Cargo.toml"]
    return [G / rel for rel in filter(None, git("ls-files", "-z", "native").stdout.split("\0"))
            if rel.endswith(("/Cargo.toml", "/Cargo.lock"))]


def built_from(target: Path, name: str) -> list[Path]:
    """The source files cargo built ``target/name`` from: its dep-info file ``target/name.d`` (``out: dep dep …`` on
    the first line, a space in a path written ``\\ ``; a relative path is relative to the checkout)."""
    try:
        line = (target / f"{name}.d").read_text(encoding="utf-8").split("\n", 1)[0]
    except OSError:
        fail(f"{target} has no {name}.d, cargo's list of the files it built {name} from: build it again with cargo")
    deps = (part.replace("\\ ", " ") for part in re.split(r"(?<!\\) ", line.partition(": ")[2].strip()))
    return [path if path.is_absolute() else G / path for path in map(Path, filter(None, deps))]


def check_source(rev: str) -> str:
    commit = git("rev-parse", "--verify", f"{rev}^{{commit}}", check=False)
    if commit.returncode:
        fail(f"--source {rev} is not a commit")
    diff = git("diff", "--name-only", rev, "--", *SOURCE_PATHS, check=False)
    if diff.returncode or diff.stdout.strip():
        names = diff.stdout.split()
        fail(f"the working tree differs from {rev} in {len(names)} file(s) ({', '.join(names[:5])}): commit and "
             f"publish first (the tag goes on {PUBLISHED}), or pass --source")
    return commit.stdout.strip()


def check_tag(version: str, offline: bool) -> None:
    tag = f"v{version}"
    if git("rev-parse", "-q", "--verify", f"refs/tags/{tag}", check=False).returncode == 0:
        fail(f"the tag {tag} exists here already")
    if offline:
        print(f"note: --offline: run 'git ls-remote --tags origin {tag}' before pushing (it must print nothing)")
        return
    remote = git("ls-remote", "--tags", "origin", f"refs/tags/{tag}", check=False, timeout=60)
    if remote.returncode:
        fail(f"git ls-remote origin failed: {remote.stderr.strip()[:200]} (pass --offline to skip it)")
    if remote.stdout.strip():
        fail(f"the tag {tag} exists on origin already")


def default_target() -> Path:
    """Where cargo leaves a release build: ``$CARGO_TARGET_DIR/release``, else the checkout's target/release."""
    return Path(os.environ.get("CARGO_TARGET_DIR") or G / "target") / "release"


def check_build(build: Path, version: str, host_target: Path, native_target: Path) -> None:
    bins, native = install_bins()
    missing = [name for name in bins if not (build / name).is_file() or not os.access(build / name, os.X_OK)]
    if missing:
        fail(f"{build} lacks {len(missing)} of the {len(bins)} programs: {', '.join(missing)}")
    # the times are those of cargo's own files (a copy made with plain cp has the time of the copy), against the files
    # cargo built each one from (its dep-info: a test-only file is no reason to rebuild) and its workspace's manifests
    root = G.resolve()
    targets = {False: (host_target, "--host-target"), True: (native_target, "--native-target")}
    for is_native in (False, True):
        target, option = targets[is_native]
        names = [name for name in bins if (name in native) == is_native]
        absent = [name for name in names if not (target / name).is_file()]
        if absent:
            fail(f"{target} has no {', '.join(absent[:4])}{' …' if len(absent) > 4 else ''}: pass the folder cargo "
                 f"built them in as {option}")
        other = [name for name in names if not filecmp.cmp(build / name, target / name, shallow=False)]
        if other:
            fail(f"{', '.join(other[:4])}{' …' if len(other) > 4 else ''} in {build} are not the ones in {target}: "
                 f"copy the programs again (or pass the folder they were built in as {option})")
        cargo_files = manifests(is_native)
        stale = []
        for name in names:
            built = (target / name).stat().st_mtime
            sources = [path for path in built_from(target, name) if path.resolve().is_relative_to(root)]
            if not sources:
                fail(f"{target / name} was not built from {G} (its {name}.d names no file here): build it here")
            newer = [path for path in sources + cargo_files if not path.exists() or path.stat().st_mtime > built]
            if newer:
                # name the newest (a gone file first)
                stale.append((name, max(newer, key=lambda path: path.stat().st_mtime if path.exists() else 1e18)))
        if stale:
            listed = ", ".join(name for name, _ in stale[:4]) + (" …" if len(stale) > 4 else "")
            changed = stale[0][1].resolve()
            changed = changed.relative_to(root) if changed.is_relative_to(root) else changed
            fail(f"{listed} in {target} are older than {changed} (or it is gone): build again and copy the programs")
    if f"WinPoint {version}\n".encode() not in (build / "win11-panel").read_bytes():
        fail(f"{build / 'win11-panel'} has no 'WinPoint {version}' marker: it was built for another version")


def gnome_major() -> str | None:
    """The major of the installed gnome-shell package (dpkg's record; nothing is started)."""
    try:
        out = subprocess.run(["dpkg-query", "-W", "-f", "${Version}", "gnome-shell"], capture_output=True, text=True,
                             timeout=20)
    except (OSError, subprocess.TimeoutExpired):
        return None
    match = re.match(r"(?:\d+:)?(\d+)", out.stdout.strip()) if out.returncode == 0 else None
    return match.group(1) if match else None


def check_os(allow: bool) -> None:
    try:
        fields = dict(line.split("=", 1) for line in Path("/etc/os-release").read_text().splitlines() if "=" in line)
    except OSError:
        fields = {}
    release = fields.get("VERSION_ID", "").strip('"')
    shells = json.loads((G / "extension" / "metadata.json").read_text(encoding="utf-8")).get("shell-version", [])
    major = gnome_major()
    problems = []
    if release not in SUPPORTED:
        problems.append(f"Ubuntu {release or '?'} is not one of {', '.join(SUPPORTED)}")
    if major not in shells:
        problems.append(f"GNOME {major or '?'} is not in metadata.json shell-version {shells}")
    for problem in problems:
        if not allow:
            fail(f"build machine: {problem} (--allow-os overrides)")
        print(f"warning: build machine: {problem} (--allow-os)")


def unpack_source(rev: str, into: Path) -> None:
    archive = subprocess.run(["git", "-C", str(G), "archive", "--format=tar", rev], check=True, capture_output=True)
    into.mkdir()
    # our own commit: the "tar" filter keeps link targets as they are ("data" rewrites "./go-up.svg" to "go-up.svg",
    # and scripts/icon_bundle.py validate compares them byte for byte with vendor/icon-bundle.json)
    with tarfile.open(fileobj=io.BytesIO(archive.stdout)) as tar:
        tar.extractall(into, filter="tar")


def tarball_problems(path: Path, top: str) -> list[str]:
    """The update check's rules for a release tarball (design §3.2 step 6)."""
    problems = []
    with tarfile.open(path, "r:gz") as tar:
        for member in tar:
            name = PurePosixPath(member.name)
            if name.is_absolute() or ".." in name.parts or name.parts[:1] != (top,):
                problems.append(f"{member.name}: outside {top}/")
            elif member.islnk() or member.isdev() or member.isfifo():
                problems.append(f"{member.name}: a hard link, device or fifo")
            elif member.issym():
                # resolved from the link's folder; a ".." above the extraction folder is an escape, never clamped
                # (``a/b -> ../../../winpoint-V/c`` climbs out and back into a folder that is only named like ours)
                target = PurePosixPath(member.linkname)
                parts, escaped = [*name.parent.parts], False
                for part in target.parts:
                    if part == "..":
                        if not parts:
                            escaped = True
                            break
                        parts.pop()
                    elif part != ".":
                        parts.append(part)
                if escaped or target.is_absolute() or parts[:1] != [top]:
                    problems.append(f"{member.name}: a symbolic link out of {top}/ ({member.linkname})")
            elif not (member.isfile() or member.isdir()):
                problems.append(f"{member.name}: an unknown member type")
    return problems


def publish_commands(version: str, source: str, notes: str, assets: list[str], digests: list[str]) -> list[str]:
    """The publishing lines `build` prints (none of them runs here): the annotated tag on `source`, its push, a draft
    release with the two assets, the draft's asset list with the digest GitHub computed for each (the updater's trust
    anchor, which must be `digests`, in the order of `assets`), and only then publishing it: with immutable releases
    the assets of a published release cannot be replaced any more, a draft's can."""
    shown = ", ".join(f"{Path(asset).name} sha256:{digest}" for asset, digest in zip(assets, digests))
    return [
        f'git tag -a v{version} {source} -m "WinPoint {version}"',
        f"git push origin v{version}",
        f'gh release create v{version} --repo {REPO} --draft --verify-tag --title "WinPoint {version}" \\',
        f"   --notes-file {notes} \\",
        "   " + " ".join(assets),
        f"gh release view v{version} --repo {REPO} --json assets   # {len(assets)} assets; confirm each shows its "
        f"digest: {shown}; publish only when every one matches, else fix the draft first (gh release upload "
        "--clobber, gh release delete-asset)",
        f"gh release edit v{version} --repo {REPO} --draft=false",
    ]


def previous_tag(version: str) -> str | None:
    tags = [t for t in git("tag", "-l", "v*").stdout.split() if VERSION_RE.fullmatch(t[1:])]
    older = [t for t in tags if tuple(map(int, t[1:].split("."))) < tuple(map(int, version.split(".")))]
    return max(older, key=lambda t: tuple(map(int, t[1:].split("."))), default=None)


def cmd_build(args) -> int:
    version = args.version
    problems = problems_of_check(version)
    if problems:
        fail("; ".join(problems))
    check_tag(version, args.offline)
    commit = check_source(args.source)
    # the notes come from the commit that is tagged and packed, never from a newer working tree
    shown = git("show", f"{commit}:CHANGELOG.md", check=False)
    section = changelog_section(version, shown.stdout) if shown.returncode == 0 else None
    if not section:
        fail(f"{args.source} has no '## {version}' section in its CHANGELOG.md: commit and publish it first")
    build = Path(args.build_dir).resolve()
    check_build(build, version, Path(args.host_target or default_target()).resolve(),
                Path(args.native_target or default_target()).resolve())
    check_os(args.allow_os)
    dist = G / "dist"
    dist.mkdir(exist_ok=True)
    name = f"winpoint-{version}-linux-{platform.machine()}.tar.gz"
    archive, sha = dist / name, dist / (name + ".sha256")
    for stale in (archive, sha):
        stale.unlink(missing_ok=True)
    with tempfile.TemporaryDirectory(prefix="winpoint-release-") as temp:
        src = Path(temp) / "src"
        unpack_source(commit, src)
        env = {k: v for k, v in os.environ.items() if k != "WIN11_NATIVE_BUILD_DIR"}
        env.update(WIN11_BUILD_DIR=str(build), WIN11_PACKAGE_DIR=str(dist))
        made = subprocess.run([sys.executable, "-B", str(src / "scripts" / "package.py"), str(src), "--lang", "en"],
                              env=env, capture_output=True, text=True)
        if made.returncode:
            fail(f"scripts/package.py failed: {(made.stderr or made.stdout).strip()[-400:]}")
    if not archive.is_file() or not sha.is_file():
        fail(f"scripts/package.py made no {name}: {made.stdout.strip()[-200:]}")
    top = f"winpoint-{version}"
    bad = tarball_problems(archive, top)
    if bad:
        fail(f"{name}: {len(bad)} member(s) break the rules: {'; '.join(bad[:5])}")
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    if sha.read_text(encoding="utf-8") != f"{digest}  {name}\n":
        fail(f"{sha.name} does not name {name} with its SHA-256")
    assets = [archive, sha]
    notes = [section,
             f"Install: download {name}, unpack it and run `./scripts/install.sh` in its `{top}` folder. Your "
             "settings are kept.\n"]
    prev = previous_tag(version)
    if prev and git("diff", "--quiet", prev, commit, "--", *SYSTEM_PATHS, check=False).returncode:
        print(f"warning: host/data or scripts/install-session.py changed since {prev}: machines with those system "
              "files installed must install this release by hand (it is in the notes)")
        notes.append("This release changes system files: if you installed them, run `scripts/install-system.py` "
                     "or `scripts/install-session.py` again after the install.\n")
    note_file = dist / f"notes-{version}.md"
    note_file.write_text("\n".join(notes), encoding="utf-8")
    size = archive.stat().st_size
    print(f"built {archive.relative_to(G)} ({size / 1048576:.1f} MiB, top folder {top}/, sha256:{digest}), "
          f"{sha.name}; notes {note_file.relative_to(G)}")
    source = args.source if args.source == PUBLISHED else commit[:12]
    if args.source != PUBLISHED:
        print(f"warning: --source {args.source}: the tag must go on the published commit ({PUBLISHED}); build again "
              "without --source after publishing before you run these")
    rel = [str(p.relative_to(G)) for p in assets]
    digests = [digest, hashlib.sha256(sha.read_bytes()).hexdigest()]
    print("\nWinPoint's updater trusts GitHub's digest of the tarball (no signature, D-G74): turn on immutable "
          f"releases in {REPO}'s settings (General › Releases) before you publish.")
    print("\nPublish (nothing of this has run):\n")
    for line in publish_commands(version, source, str(note_file.relative_to(G)), rel, digests):
        print(line)
    return 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    sub = parser.add_subparsers(dest="command", required=True)
    p = sub.add_parser("bump", help="set the version in Cargo.toml, Cargo.lock and extension/metadata.json")
    p.add_argument("version", type=version_arg)
    p = sub.add_parser("check", help="the three places agree; CHANGELOG.md has the section")
    p.add_argument("version", type=version_arg, nargs="?")
    p = sub.add_parser("build", help="build the release assets in dist/ and print the publishing commands")
    p.add_argument("version", type=version_arg)
    p.add_argument("--build-dir", required=True, help="the 19 programs (e.g. tests/build/native-release)")
    p.add_argument("--host-target", help="where cargo built the host's programs (default: $CARGO_TARGET_DIR/release, "
                   "else target/release)")
    p.add_argument("--native-target", help="where cargo built the GTK 4 programs (default: as --host-target)")
    p.add_argument("--source", default=PUBLISHED, help=f"the commit to pack and tag (default {PUBLISHED})")
    p.add_argument("--allow-os", action="store_true", help="build on an Ubuntu or GNOME the release is not for")
    p.add_argument("--offline", action="store_true", help="skip 'git ls-remote' of the tag on origin")
    args = parser.parse_args(argv)
    return {"bump": cmd_bump, "check": cmd_check, "build": cmd_build}[args.command](args)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
