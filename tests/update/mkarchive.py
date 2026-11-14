#!/usr/bin/env python3
"""GW6: a signed ``file:`` Ubuntu archive for the compatibility check's D-G27 test (notes/claude/wu/gw6.md).

    python3 -I tests/update/mkarchive.py key     --gnupghome G --keyring K.gpg
    python3 -I tests/update/mkarchive.py archive --gnupghome G --case CASE --out DIR
    python3 -I tests/update/mkarchive.py status  --out FILE [--extended FILE]
    python3 -I tests/update/mkarchive.py cases

* ``key``: a throwaway ed25519 signing key in the GnuPG home G (which must lie under tests/update/.run/, 0700), and
  its public key exported as a binary keyring K (the fixture's ``usr/share/keyrings/ubuntu-archive-keyring.gpg``,
  re-rooted by paths.rs in test mode). gpg runs with --batch, no passphrase, no keyserver.
* ``archive``: DIR/ubuntu/dists/{stonking,stonking-updates,stonking-security}/ with ``Release`` (written here:
  Origin/Label Ubuntu, Suite, Version, Codename, Date, Architectures, Components, SHA256 of every index),
  ``InRelease`` (clearsigned) and ``Release.gpg`` (detached), and ``<component>/binary-amd64/Packages{,.gz,.xz}`` for
  main restricted universe multiverse. The packages of stonking are the case's (below); -updates and -security
  are empty. No .deb file exists: apt only reads the indexes (``apt-get -s`` never downloads).
* ``status``: the fixture dpkg status (Robin's 26.04 shape for every package the check looks at: the blocking
  libraries, gnome-shell, the degraded tools, ubuntu-desktop-minimal), every dependency inside it, and with
  ``--extended`` apt's extended_states: the libraries are automatically installed (as on a real install; apt keeps
  a manually installed package rather than replace or remove it, so a renamed library would otherwise hold
  everything back).
* The cases (``CASES``): ok (everything newer, GNOME 50), warn (ffmpeg dropped; GDK Pixbuf renamed (a t64-style
  transition), which is no missing part; the third-party sources are the fixture root's), hold (GNOME 51 in the archive), blocked
  (libgtk-4-1 dropped, which every native program of ours needs, and an upgraded ubuntu-desktop-minimal breaks
  libarchive13t64, so the simulation removes it), ubuntuHold (the ok archive; meta-release says UpgradeBroken).

Synthetic data: made-up versions in Robin's 26.04 shape. The packages are those of the machine (WebKitGTK, libsoup
and GTK 3 stay installed as Ubuntu's libraries for other apps; our programs do not need them, 14-11). The warn case
renamed GTK 3 until GTK left the host; a renamed library counts only when it is one of ours. The recordings under
fixtures/compat/recorded/ (``--record``) were made from the GW6 version of these archives, whose blocked case
dropped libwebkit2gtk-4.1 and its JavaScriptCore instead and which had no GTK 4: compat.rs's unit tests replay them
as algorithm data.

Standard library + /usr/bin/gpg only. Never contacts a network (the caller runs it inside bwrap --unshare-net).
"""
from __future__ import annotations

import argparse
import gzip
import hashlib
import lzma
import os
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
RUNS = HERE / ".run"
CODENAME = "stonking"
VERSION = "26.10"
COMPONENTS = ("main", "restricted", "universe", "multiverse")
SUITES = (CODENAME, f"{CODENAME}-updates", f"{CODENAME}-security")
DATE = "Thu, 15 Oct 2026 12:00:00 UTC"
UID = "Win11 GW6 throwaway test archive key <gw6-test@invalid>"
GPG = "/usr/bin/gpg"

# (name, version, priority, section/component, depends, extra fields) as installed on 26.04 (the status file)
INSTALLED = [
    ("gnome-shell", "50.1-0ubuntu1.5", "optional", "main/gnome", "libgtk-3-0t64, libglib2.0-0t64", {}),
    ("libwebkit2gtk-4.1-0", "2.52.6-0ubuntu0.26.04.1", "optional", "main/libs",
     "libjavascriptcoregtk-4.1-0 (= 2.52.6-0ubuntu0.26.04.1), libsoup-3.0-0, libgtk-3-0t64", {}),
    ("libjavascriptcoregtk-4.1-0", "2.52.6-0ubuntu0.26.04.1", "optional", "main/libs", "libglib2.0-0t64", {}),
    ("libgtk-3-0t64", "3.24.52-1ubuntu1", "optional", "main/libs", "libglib2.0-0t64, libcairo2, libgdk-pixbuf-2.0-0", {}),
    ("libgtk-4-1", "4.22.4+ds-0ubuntu0.1", "optional", "main/libs",
     "libglib2.0-0t64, libcairo2, libgdk-pixbuf-2.0-0, libgraphene-1.0-0, libpango-1.0-0", {}),
    ("libgraphene-1.0-0", "1.10.8-5build1", "optional", "main/libs", "libglib2.0-0t64", {}),
    ("libpango-1.0-0", "1.57.0-1", "optional", "main/libs", "libglib2.0-0t64", {}),
    ("libsoup-3.0-0", "3.6.6-1ubuntu0.2", "optional", "main/libs", "libglib2.0-0t64", {}),
    ("libglib2.0-0t64", "2.88.0-1", "optional", "main/libs", "zlib1g", {}),
    ("libgdk-pixbuf-2.0-0", "2.42.12+dfsg-4", "optional", "main/libs", "libglib2.0-0t64", {}),
    ("libcairo2", "1.18.4-1", "optional", "main/libs", "", {}),
    ("zlib1g", "1:1.3.dfsg+really1.3.1-1ubuntu1", "required", "main/libs", "", {}),
    ("libarchive13t64", "3.8.5-1", "optional", "main/libs", "zlib1g", {}),
    ("ffmpeg", "7:7.1.1-1ubuntu1", "optional", "universe/video", "", {}),
    ("bubblewrap", "0.11.1-1", "optional", "main/admin", "", {}),
    ("squashfs-tools", "1:4.6.1-1", "optional", "main/admin", "zlib1g", {}),
    ("flatpak", "1.16.1-1ubuntu1", "optional", "universe/admin", "bubblewrap, libglib2.0-0t64", {}),
    ("packagekit", "1.3.4-3ubuntu1.2", "optional", "main/admin", "libglib2.0-0t64", {}),
    ("ptyxis", "50.1-1", "optional", "main/gnome", "libgtk-3-0t64", {}),
    ("screen", "4.9.1-3build1", "optional", "main/misc", "", {}),
    ("xdg-terminal-exec", "0.14.0-1", "optional", "universe/x11", "", {}),
    ("gnome-shell-ubuntu-extensions", "50.0-1", "optional", "main/gnome", "gnome-shell", {}),
    ("ubuntu-desktop-minimal", "1.539", "important", "main/metapackages", "gnome-shell, ptyxis", {}),
]

# stonking's packages for the ok case: every installed package one version newer, GNOME 50
OK = [
    ("gnome-shell", "50.3-0ubuntu1", "optional", "main/gnome", "libgtk-3-0t64, libglib2.0-0t64", {}),
    ("libwebkit2gtk-4.1-0", "2.54.1-0ubuntu1", "optional", "main/libs",
     "libjavascriptcoregtk-4.1-0 (= 2.54.1-0ubuntu1), libsoup-3.0-0, libgtk-3-0t64", {}),
    ("libjavascriptcoregtk-4.1-0", "2.54.1-0ubuntu1", "optional", "main/libs", "libglib2.0-0t64", {}),
    ("libgtk-3-0t64", "3.24.60-1ubuntu1", "optional", "main/libs", "libglib2.0-0t64, libcairo2, libgdk-pixbuf-2.0-0", {}),
    ("libgtk-4-1", "4.24.1+ds-1ubuntu1", "optional", "main/libs",
     "libglib2.0-0t64, libcairo2, libgdk-pixbuf-2.0-0, libgraphene-1.0-0, libpango-1.0-0", {}),
    ("libgraphene-1.0-0", "1.10.8-6", "optional", "main/libs", "libglib2.0-0t64", {}),
    ("libpango-1.0-0", "1.57.1-1", "optional", "main/libs", "libglib2.0-0t64", {}),
    ("libsoup-3.0-0", "3.6.7-1", "optional", "main/libs", "libglib2.0-0t64", {}),
    ("libglib2.0-0t64", "2.90.1-1", "optional", "main/libs", "zlib1g", {}),
    ("libgdk-pixbuf-2.0-0", "2.42.12+dfsg-5", "optional", "main/libs", "libglib2.0-0t64", {}),
    ("libcairo2", "1.18.4-2", "optional", "main/libs", "", {}),
    ("zlib1g", "1:1.3.dfsg+really1.3.1-1ubuntu2", "required", "main/libs", "", {}),
    ("libarchive13t64", "3.8.6-1", "optional", "main/libs", "zlib1g", {}),
    ("ffmpeg", "7:7.1.2-1ubuntu1", "optional", "universe/video", "", {}),
    ("bubblewrap", "0.11.2-1", "optional", "main/admin", "", {}),
    ("squashfs-tools", "1:4.7-1", "optional", "main/admin", "zlib1g", {}),
    ("flatpak", "1.16.2-1", "optional", "universe/admin", "bubblewrap, libglib2.0-0t64", {}),
    ("packagekit", "1.3.5-1", "optional", "main/admin", "libglib2.0-0t64", {}),
    ("ptyxis", "50.2-1", "optional", "main/gnome", "libgtk-3-0t64", {}),
    ("screen", "4.9.1-4", "optional", "main/misc", "", {}),
    ("xdg-terminal-exec", "0.14.1-1", "optional", "universe/x11", "", {}),
    ("gnome-shell-ubuntu-extensions", "50.1-1", "optional", "main/gnome", "gnome-shell", {}),
    ("ubuntu-desktop-minimal", "1.540", "important", "main/metapackages", "gnome-shell, ptyxis", {}),
]


def _replace(rows, package, **changes):
    out = []
    for row in rows:
        if row[0] == package:
            n, v, p, s, d, x = row
            row = (changes.get("name", n), changes.get("version", v), p, changes.get("section", s),
                   changes.get("depends", d), {**x, **changes.get("extra", {})})
        out.append(row)
    return out


def _drop(rows, *names):
    return [r for r in rows if r[0] not in names]


def _warn():
    rows = _drop(OK, "ffmpeg")
    # GDK Pixbuf renamed (a t64-style transition): every user depends on the new name, which breaks and replaces the
    # old one
    old, new = "libgdk-pixbuf-2.0-0", "libgdk-pixbuf-2.0-0t65"
    rows = _replace(rows, old, name=new, version="2.44.0-1",
                    extra={"Breaks": f"{old} (<< 2.44.0-1)", "Replaces": f"{old} (<< 2.44.0-1)",
                           "Provides": f"{old} (= 2.44.0-1)"})
    for user in ("libgtk-3-0t64", "libgtk-4-1"):
        rows = [(n, v, p, s, d.replace(old, new) if n == user else d, x) for (n, v, p, s, d, x) in rows]
    return rows


def _hold():
    rows = _replace(OK, "gnome-shell", version="51.0-0ubuntu1")
    return _replace(rows, "gnome-shell-ubuntu-extensions", version="51.0-1")


def _blocked():
    rows = _drop(OK, "libgtk-4-1")
    return _replace(rows, "ubuntu-desktop-minimal", extra={"Breaks": "libarchive13t64"})


CASES = {"ok": OK, "warn": _warn(), "hold": _hold(), "blocked": _blocked(), "ubuntuHold": OK}


def stanza(row, installed=False):
    name, version, priority, section, depends, extra = row
    component, sect = section.split("/", 1)
    fields = [("Package", name)]
    if installed:
        fields.append(("Status", "install ok installed"))
    fields += [("Priority", priority), ("Section", sect if component == "main" else section),
               ("Installed-Size", "100"), ("Maintainer", "Ubuntu Developers <ubuntu-devel-discuss@lists.ubuntu.com>"),
               ("Architecture", "amd64"), ("Version", version)]
    if depends:
        fields.append(("Depends", depends))
    fields += sorted(extra.items())
    if not installed:
        initial = name[:4] if name.startswith("lib") else name[0]
        fields += [("Filename", f"pool/{component}/{initial}/{name}/{name}_{version.split(':')[-1]}_amd64.deb"),
                   ("Size", "1000"), ("SHA256", hashlib.sha256(f"{name} {version}".encode()).hexdigest())]
    fields.append(("Description", f"{name} (GW6 fixture)"))
    return "".join(f"{k}: {v}\n" for k, v in fields)


def write_status(path: Path, extended: Path | None):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("\n".join(stanza(r, installed=True) for r in INSTALLED))
    if extended:
        auto = [r[0] for r in INSTALLED if r[0].startswith("lib") or r[0] == "zlib1g"]
        extended.parent.mkdir(parents=True, exist_ok=True)
        extended.write_text("\n".join(f"Package: {n}\nArchitecture: amd64\nAuto-Installed: 1\n" for n in auto))


def gpg(gnupghome: Path, *args: str, stdin: bytes | None = None) -> bytes:
    env = {"GNUPGHOME": str(gnupghome), "PATH": "/usr/bin:/bin", "LANG": "C", "HOME": str(gnupghome)}
    r = subprocess.run([GPG, "--batch", "--no-tty", "--pinentry-mode", "loopback", "--passphrase", "",
                        "--no-auto-key-locate", *args], input=stdin, capture_output=True, env=env, check=False)
    if r.returncode != 0:
        sys.stderr.write(r.stderr.decode(errors="replace"))
        raise SystemExit(f"mkarchive: gpg {' '.join(args)} failed ({r.returncode})")
    return r.stdout


def check_home(gnupghome: Path):
    home = gnupghome.resolve()
    if RUNS.resolve() not in home.parents:
        raise SystemExit(f"mkarchive: the GnuPG home must lie under {RUNS} (got {home})")
    home.mkdir(mode=0o700, parents=True, exist_ok=True)
    os.chmod(home, 0o700)


def make_key(gnupghome: Path, keyring: Path):
    check_home(gnupghome)
    gpg(gnupghome, "--quick-gen-key", UID, "ed25519", "sign", "never")
    keyring.parent.mkdir(parents=True, exist_ok=True)
    keyring.write_bytes(gpg(gnupghome, "--export", UID))
    if not keyring.read_bytes():
        raise SystemExit("mkarchive: the exported keyring is empty")


def write_suite(gnupghome: Path, dists: Path, suite: str, rows):
    files = []
    for component in COMPONENTS:
        rel = f"{component}/binary-amd64"
        d = dists / suite / rel
        d.mkdir(parents=True, exist_ok=True)
        mine = [r for r in rows if r[3].split("/", 1)[0] == component]
        text = "\n".join(stanza(r) for r in mine).encode()
        (d / "Packages").write_bytes(text)
        (d / "Packages.gz").write_bytes(gzip.compress(text, mtime=0))
        (d / "Packages.xz").write_bytes(lzma.compress(text))
        (d / "Release").write_text(f"Archive: {suite}\nVersion: {VERSION}\nComponent: {component}\nOrigin: Ubuntu\n"
                                   f"Label: Ubuntu\nArchitecture: amd64\n")
        files += [f"{rel}/Packages", f"{rel}/Packages.gz", f"{rel}/Packages.xz", f"{rel}/Release"]
    lines = [f"Origin: Ubuntu", "Label: Ubuntu", f"Suite: {suite}", f"Version: {VERSION}", f"Codename: {CODENAME}",
             f"Date: {DATE}", "Architectures: amd64", f"Components: {' '.join(COMPONENTS)}",
             "Description: Ubuntu Stonking Stingray 26.10 (GW6 test archive)", "SHA256:"]
    for rel in files:
        data = (dists / suite / rel).read_bytes()
        lines.append(f" {hashlib.sha256(data).hexdigest()} {len(data):>16} {rel}")
    release = ("\n".join(lines) + "\n").encode()
    (dists / suite / "Release").write_bytes(release)
    (dists / suite / "InRelease").write_bytes(
        gpg(gnupghome, "--local-user", UID, "--digest-algo", "SHA512", "--clearsign", stdin=release))
    (dists / suite / "Release.gpg").write_bytes(
        gpg(gnupghome, "--local-user", UID, "--digest-algo", "SHA512", "--armor", "--detach-sign", stdin=release))


def make_archive(gnupghome: Path, case: str, out: Path):
    if case not in CASES:
        raise SystemExit(f"mkarchive: unknown case {case} (cases: {' '.join(CASES)})")
    check_home(gnupghome)
    dists = out / "ubuntu" / "dists"
    for suite in SUITES:
        write_suite(gnupghome, dists, suite, CASES[case] if suite == CODENAME else [])


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    k = sub.add_parser("key")
    k.add_argument("--gnupghome", type=Path, required=True)
    k.add_argument("--keyring", type=Path, required=True)
    a = sub.add_parser("archive")
    a.add_argument("--gnupghome", type=Path, required=True)
    a.add_argument("--case", required=True)
    a.add_argument("--out", type=Path, required=True)
    s = sub.add_parser("status")
    s.add_argument("--out", type=Path, required=True)
    s.add_argument("--extended", type=Path)
    sub.add_parser("cases")
    args = ap.parse_args()
    if args.cmd == "key":
        make_key(args.gnupghome, args.keyring)
    elif args.cmd == "archive":
        make_archive(args.gnupghome, args.case, args.out)
    elif args.cmd == "status":
        write_status(args.out, args.extended)
    else:
        print(" ".join(CASES))
    return 0


if __name__ == "__main__":
    sys.exit(main())
