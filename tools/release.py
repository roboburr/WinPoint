#!/usr/bin/env python3
"""WinPoint's releases on GitHub (D-G74a; the updater design §3): the version in its places, the checks, and the
local half of a release. Standard library only; run it from the checkout.

Publishing also includes updating winpoint.nl in the same release pass, every time; see docs/RELEASING.md.

    python3 -B tools/release.py bump X.Y.Z
    python3 -B tools/release.py check [X.Y.Z]
    python3 -B tools/release.py build X.Y.Z --build-dir DIR [--host-target T] [--native-target T] [--source REV]
                                  [--allow-os] [--offline]

* ``bump`` rewrites the places of the version and needs no cargo: the root Cargo.toml's ``[workspace.package]
  version`` (the one source; the host inherits it), the ``win11-host`` block of Cargo.lock (scripts/build.sh builds
  with ``--locked``), ``version-name`` in extension/metadata.json (GNOME's integer ``version`` is its own counter
  and stays) and the install line of README.md and README.nl.md (``install_line``, the line the release notes give:
  it names the release, so an older download next to it does not matter). It finds all of them first and writes
  nothing unless each has exactly one version (each file is then replaced atomically). Add the CHANGELOG.md section
  and build again afterwards.
* ``check`` says whether those places agree (and equal X.Y.Z when it is given) and whether CHANGELOG.md has a
  ``## X.Y.Z`` section.
* ``build`` does everything that stays on this machine and **prints** the publishing commands; it never tags, pushes,
  uploads or contacts GitHub except for one ``git ls-remote`` of the tag (``--offline`` skips it):
  1. ``check X.Y.Z``; no tag ``vX.Y.Z`` here or (``git ls-remote``) on origin;
  2. the source is the commit to be tagged: ``--source`` (default ``main``, whose tree equals the local HEAD's)
     has the working tree's host, native, scripts,
     extension, vendor, Cargo files, CHANGELOG.md and the two READMEs with their install line (``git diff
     --quiet``; untracked files do not count), and its own CHANGELOG.md has the ``## X.Y.Z`` section (the release
     notes are taken from there, the text of the tag);
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
  7. a warning when what lands on the system changed since the previous ``v*`` tag, as the commit's install.py
     --check would report it on a PC set up with that tag (its names): "session" when the four files the tag's
     scripts/install-session.py ``contents()`` writes, installed on a fake system root, are what the commit's
     install-session.py ``installed()`` and ``stale()`` call different (0.9.1: winpoint.json by meaning, without the
     keys it copies from Ubuntu's mode; a script without ``stale()``, as 0.9.0's, compares the bytes), on two roots,
     one with Ubuntu's shell mode ``UBUNTU_MODE`` and one without, each revision's own script unpacked from git;
     "system" when one of install-system.py's four files in host/data differs byte for byte. WinPoint's updater
     refuses such a release on a PC that has those files ("manual", host/src/services/winpoint/job.rs), so the notes
     then say to install it with the line; a change to a root script, a message, the formatting of winpoint.json or
     host/data/releases.json alone is no reason (0.9.1: the one root step moved into install-session.py, its four
     files stayed);
  8. ``dist/notes-X.Y.Z.md``: that commit's CHANGELOG section, then the install (``release_notes``): the one line to
     paste, ``cd "$(xdg-user-dir DOWNLOAD)" && tar -xzf winpoint-X.Y.Z-linux-ARCH.tar.gz &&
     ./winpoint-X.Y.Z/scripts/install.sh`` (the browser's download folder, which Ubuntu names in the user's language),
     and an optional ``sha256sum -c`` of the ``.sha256`` asset; then the ``git tag``, ``git push`` and ``gh release``
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
import importlib.util
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
PUBLISHED = "main"
SUPPORTED = ("26.04",)
# step 2; the READMEs too: two of bump's version places, packed into the tarball (scripts/package.py), whose install
# line must be the tagged commit's
SOURCE_PATHS = ("host", "native", "scripts", "extension", "vendor", "Cargo.toml", "Cargo.lock", "CHANGELOG.md",
                "README.md", "README.nl.md")
# step 7: what a revision needs to say what it writes on the system (install-session.py, install-system.py, host/data)
SYSTEM_PATHS = ("scripts", "host/data")
# Ubuntu 26.04's /usr/share/gnome-shell/modes/ubuntu.json: step 7's second fake system root has it (winpoint.json copies
# its stylesheetName, themeResourceName and iconsResourceName), the first has none, as on a GNOME without Ubuntu's mode
UBUNTU_MODE = {
    "parentMode": "user",
    "stylesheetName": "Yaru/gnome-shell.css",
    "colorScheme": "prefer-light",
    "themeResourceName": "theme/Yaru/gnome-shell-theme.gresource",
    "iconsResourceName": "theme/Yaru/gnome-shell-icons.gresource",
    "debugFlags": ["backtrace-crashes-all"],
    "enabledExtensions": ["ubuntu-dock@ubuntu.com", "ubuntu-appindicators@ubuntu.com", "ding@rastersoft.com",
                          "tiling-assistant@ubuntu.com", "snapd-prompting@canonical.com",
                          "snapd-search-provider@canonical.com", "web-search-provider@ubuntu.com"],
}
# step 7's names (install.py --check's systemFiles) in the warning
CHANGED = {"session": "the login-session files of scripts/install-session.py",
           "system": "the system files of host/data (scripts/install-system.py)"}
VERSION_RE = re.compile(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)")
# the places of the version (bump rewrites the second group of each, exactly once, and the group "again" where a
# place names the version twice: the README's install line, install_line())
README_LINE = re.compile(r'(cd "\$\(xdg-user-dir DOWNLOAD\)" && tar -xzf winpoint-)([^-\s]*)(-linux-x86_64\.tar\.gz && '
                         r'\./winpoint-)(?P<again>[^/\s]*)(/scripts/install\.sh)')
PLACES = (
    ("Cargo.toml", re.compile(r'(\[workspace\.package\]\n(?:[^\[\n][^\n]*\n|\n)*?version = ")([^"]*)(")')),
    ("Cargo.lock", re.compile(r'(\[\[package\]\]\nname = "win11-host"\nversion = ")([^"]*)(")')),
    ("extension/metadata.json", re.compile(r'("version-name": ")([^"]*)(")')),
    ("README.md", README_LINE),
    ("README.nl.md", README_LINE),
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


def install_line(name: str, top: str) -> str:
    """The line to paste for the tarball `name` with its top folder `top` (README's and the release notes')."""
    return f'cd "$(xdg-user-dir DOWNLOAD)" && tar -xzf {name} && ./{top}/scripts/install.sh'


def versions() -> dict[str, str | None]:
    """Each place's version (None when a place has none; both of a line that names it twice, when they differ)."""
    out = {}
    for name, pattern in PLACES:
        match = pattern.search((G / name).read_text(encoding="utf-8"))
        if match and "again" in pattern.groupindex and match.group("again") != match.group(2):
            out[name] = f"{match.group(2)} and {match.group('again')}"
        else:
            out[name] = match.group(2) if match else None
    return out


def bumped(match: re.Match, version: str) -> str:
    """`match`'s text with its version (group 2, and the group "again" when the place has it) set to `version`."""
    spans = [match.span(2)] + ([match.span("again")] if "again" in match.re.groupindex else [])
    text, at = "", match.start()
    for start, end in spans:
        text += match.string[at:start] + version
        at = end
    return text + match.string[at:match.end()]


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
        new, count = pattern.subn(lambda m: bumped(m, args.version), text)
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
          f"{found['extension/metadata.json']}, README.md {found['README.md']}, README.nl.md {found['README.nl.md']})")
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


def unpack_source(rev: str, into: Path, *paths: str) -> None:
    """`rev`'s tree (only `paths` when given) unpacked into the new folder `into`."""
    archive = subprocess.run(["git", "-C", str(G), "archive", "--format=tar", rev, *paths], check=True,
                             capture_output=True)
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


def load_script(path: Path, name: str):
    """A script of an unpacked revision as a module of its own, the way install.py loads install-session.py (not in
    sys.modules, so two revisions never mix); None when the revision has no such file."""
    if not path.is_file():
        return None
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def written(tree: Path, roots: list[Path]):
    """What the revision unpacked in `tree` puts on the system: its install-session.py (the module, None without
    one), per fake system root in `roots` the session files of it ({destination: bytes}, None without the script),
    and its install-system.py's files in host/data ({name: bytes, None when missing})."""
    session = load_script(tree/"scripts/install-session.py", "release_install_session")
    sessions = None
    if session is not None:
        sessions = []
        for root in roots:
            files = session.contents(root)
            sessions.append({f"{entry[1]}/{entry[0]}": files[entry[0]] for entry in session.FILES})
    system = load_script(tree/"scripts/install-system.py", "release_install_system")
    data = {}
    for entry in system.FILES if system is not None else ():
        try:
            data[entry[0]] = (tree/"host/data"/entry[0]).read_bytes()
        except OSError:
            data[entry[0]] = None
    return session, sessions, data


def session_changed(prev_files: list[dict[str, bytes]] | None, new, new_files: list[dict[str, bytes]] | None,
                    roots: list[Path]) -> bool:
    """"session" as the new revision's install.py --check finds it (system_files): on each root, with the previous
    revision's four files installed there, the new install-session.py's installed() and stale(). A script without
    stale() (0.9.0's) compares the bytes, as its --check did; nothing installed before is no change."""
    if prev_files is None:
        return False
    if new is None or not (hasattr(new, "stale") and hasattr(new, "installed")):
        return prev_files != new_files
    for root, files in zip(roots, prev_files):
        for destination, data in files.items():
            path = root/destination.lstrip("/")
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)
        if new.installed(root) and new.stale(root):
            return True
    return False


def system_changes(prev: str, commit: str) -> list[str]:
    """Step 7: what `commit`'s install.py --check would find changed on a PC set up with `prev`, in its names:
    "session" (`prev`'s contents() on the two fake roots, judged by `commit`'s install-session.py: session_changed)
    and "system" (the files of install-system.py in host/data, byte for byte); the updater blocks a release with
    either ("manual"). What `prev` did not install (no install-session.py, a file its install-system.py does not
    list) is no change."""
    with tempfile.TemporaryDirectory(prefix="winpoint-system-") as temp:
        temp = Path(temp)
        plain, ubuntu = temp/"root", temp/"root-ubuntu"
        plain.mkdir()
        mode = ubuntu/"usr/share/gnome-shell/modes/ubuntu.json"
        mode.parent.mkdir(parents=True)
        mode.write_text(json.dumps(UBUNTU_MODE, indent=4) + "\n", encoding="utf-8")
        found = {}
        for label, rev in (("prev", prev), ("new", commit)):
            try:
                unpack_source(rev, temp/label, *SYSTEM_PATHS)
                found[label] = written(temp/label, [plain, ubuntu])
            except subprocess.CalledProcessError as error:
                detail = error.stderr.decode(errors="replace").strip()[:200]
                fail(f"git archive {rev} {' '.join(SYSTEM_PATHS)} failed: {detail}")
            except Exception as error:   # a script of that revision that does not load or has no contents()
                fail(f"{rev}'s install-session.py or install-system.py: {type(error).__name__}: {error}")
        (_prev, prev_session, prev_system), (new, new_session, new_system) = found["prev"], found["new"]
        changed = []
        try:
            if session_changed(prev_session, new, new_session, [plain, ubuntu]):
                changed.append("session")
        except Exception as error:   # the new script's installed() or stale() fails on a root
            fail(f"{commit}'s install-session.py: {type(error).__name__}: {error}")
        if any(name in prev_system and prev_system[name] != data for name, data in new_system.items()):
            changed.append("system")
        return changed


def release_notes(section: str, name: str, top: str, changes: list[str]) -> str:
    """notes-X.Y.Z.md: the CHANGELOG section, then the install (0.9.1, Robin's decisions of 16-11: one password,
    WinPoint preselected at the login screen): the exact line to paste, with the version filled in (so an older
    download next to it does not matter) and the download folder from xdg-user-dir (Ubuntu names it in the user's
    language: Téléchargements, Descargas), and the optional checksum check; then the update button, or, when step 7
    found `changes`, that the button sends PCs with those files to this line (update.winpoint.reason.manual)."""
    if changes:
        button = ("This release changes WinPoint’s system files: install it with this line, also if you already have "
                  "WinPoint (**Download and install** in Settings › Update & recovery sends you here).")
    else:
        button = ("Already on WinPoint 0.9.0 or newer? **Download and install** in Settings › Update & recovery does "
                  "it for you.")
    return (f"{section}\n"
            f"**Install**: download `{name}` into your Downloads folder, open a terminal and paste this line:\n\n"
            "```bash\n"
            f"{install_line(name, top)}\n"
            "```\n\n"
            "It asks for your password once (an account that isn’t an administrator needs an administrator’s "
            "password). Then sign out: WinPoint starts at the next sign-in. Your settings are kept. "
            f"{button}\n\n"
            f"To check the download first (optional), also download `{name}.sha256` and run this line before the "
            "one above; it must say OK:\n\n"
            "```bash\n"
            f"cd \"$(xdg-user-dir DOWNLOAD)\" && sha256sum -c {name}.sha256\n"
            "```\n")


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
    prev = previous_tag(version)
    changes = system_changes(prev, commit) if prev else []
    if changes:
        print(f"warning: {' and '.join(CHANGED[c] for c in changes)} changed since {prev}: WinPoint's updater refuses "
              "this release on PCs that have them installed (\"manual\"); the notes send those users to the line")
    note_file = dist / f"notes-{version}.md"
    note_file.write_text(release_notes(section, name, top, changes), encoding="utf-8")
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
    p = sub.add_parser("bump", help="set the version in Cargo.toml, Cargo.lock, extension/metadata.json and the README install lines")
    p.add_argument("version", type=version_arg)
    p = sub.add_parser("check", help="the version places agree; CHANGELOG.md has the section")
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
