#!/usr/bin/env python3
"""Make the apt history fixtures of update.history (GW4, notes/claude/wu/gw4.md; host/src/services/update/history.rs).

    python3 -I -B tests/update/fixtures/history/make.py        (from the project root; writes only below this dir)

The files are a fixture root for WIN11_UPDATE_FIXTURE_ROOT (the Rust tests copy var/ into theirs):

* var/log/apt/history.log       Oct 29 - Nov 5 2026: unattended-upgrades, PackageKit (ours on Nov 4 12:00 and an
                                app's at 18:30), an app install with an automatic dependency, a removal (not
                                listed), apt in a terminal with bytes that are not UTF-8, a failed dpkg run (Error:)
                                and, newest, a run cut off without End-Date inside its package list.
* var/log/apt/history.log.1.gz  Sep 15 (unattended) and Oct 2 (aptdaemon, the Software Updater).
* var/log/apt/history.log.2.gz  Aug 1 (unattended): older than 90 days before the newest entry, so page two.
* var/log/apt/history.log.3.gz  July: a gzip file cut in the middle of its third run (the first two decompress).
* var/log/apt/history.log.4.gz  June: two gzip members (cat a.gz b.gz), one run each.
* var/log/dist-upgrade/history.log                Oct 10: Software Updater's partial upgrade (do-partial-upgrade runs the
                                                   release upgrader's code, so its apt log lands here too): upgrader
                                                   packages within 26.04, so no release upgrade but updates of an app.
* var/log/dist-upgrade/20261010-1830/history.log   Aug 20: the release upgrade to 26.04 (ok, two runs); the partial upgrade
                                                   moved it there when it started (DistUpgradeMain.setup_logging).
* var/log/dist-upgrade/20260820-1012/history.log   June 1: an earlier attempt that failed (Error:); the upgrader moved
                                                   it there when the Aug 20 run started.

Deterministic: gzip with mtime 0. Requested-By is uid 1000 (the tests take the uid as a parameter).
"""

from __future__ import annotations

import gzip
import io
import zlib
from pathlib import Path

HERE = Path(__file__).resolve().parent
APT = HERE / "var" / "log" / "apt"
DIST = HERE / "var" / "log" / "dist-upgrade"


def run(start: str, end: str | None, *lines: str) -> str:
    body = "".join(f"{ln}\n" for ln in lines)
    tail = f"End-Date: {end}\n" if end else ""
    return f"\nStart-Date: {start}\n{body}{tail}"


def gz(text: bytes) -> bytes:
    out = io.BytesIO()
    with gzip.GzipFile(fileobj=out, mode="wb", mtime=0, compresslevel=9, filename="") as f:
        f.write(text)
    return out.getvalue()


ROBIN = "Requested-By: robin (1000)"
SSL = "3.5.1-1ubuntu2"
WEBKIT = ("2.52.6-0ubuntu0.26.04.1", "2.52.7-0ubuntu0.26.04.1")

CURRENT = b"".join([
    run("2026-10-29  08:15:02", "2026-10-29  08:15:09",
        "Commandline: /usr/bin/unattended-upgrade",
        f"Upgrade: libssl3t64:amd64 ({SSL}, {SSL}.1), openssl:amd64 ({SSL}, {SSL}.1), libssl3t64:i386 ({SSL}, {SSL}.1)").encode(),
    run("2026-11-01  07:51:58", "2026-11-01  07:52:20",
        "Commandline: packagekit role='install-packages'", ROBIN,
        "Install: gnome-boxes:amd64 (49.1-1), libvirt-glib-1.0-0:amd64 (5.0.0-2, automatic)").encode(),
    run("2026-11-02  10:01:25", "2026-11-02  10:01:29",
        "Commandline: apt-get remove -y rhythmbox", ROBIN,
        "Remove: rhythmbox:amd64 (3.4.9-3ubuntu3)").encode(),
    # a terminal command line with Latin-1 bytes (not UTF-8): the parser must survive it
    b"\nStart-Date: 2026-11-03  19:20:11\nCommandline: apt install vim # caf\xe9 \xff\n" + "\n".join([
        ROBIN,
        "Install: vim:amd64 (2:9.1.0967-1ubuntu4), vim-runtime:amd64 (2:9.1.0967-1ubuntu4, automatic)",
        "Upgrade: vim-common:amd64 (2:9.1.0967-1ubuntu3, 2:9.1.0967-1ubuntu4)",
        "End-Date: 2026-11-03  19:20:30", ""]).encode(),
    run("2026-11-04  12:00:01", "2026-11-04  12:03:40",
        "Commandline: packagekit role='update-packages'", ROBIN,
        f"Upgrade: libwebkit2gtk-4.1-0:amd64 ({WEBKIT[0]}, {WEBKIT[1]}), libjavascriptcoregtk-4.1-0:amd64 ({WEBKIT[0]}, {WEBKIT[1]})").encode(),
    run("2026-11-04  18:30:00", "2026-11-04  18:30:40",
        "Commandline: packagekit role='update-packages'", ROBIN,
        "Upgrade: fwupd:amd64 (2.0.16-1, 2.0.16-1ubuntu0.1)").encode(),
    run("2026-11-05  06:53:43", "2026-11-05  06:53:45",
        "Commandline: /usr/bin/unattended-upgrade",
        "Upgrade: libsoup-3.0-0:amd64 (3.6.6-1ubuntu0.1, 3.6.6-1ubuntu0.2)",
        "Error: Sub-process /usr/bin/dpkg returned an error code (1)").encode(),
    # the newest run, cut off inside its package list (no newline, no End-Date)
    ("\nStart-Date: 2026-11-05  07:10:00\nCommandline: apt upgrade\n" + ROBIN + "\n"
     "Upgrade: gnome-shell:amd64 (50.1-0ubuntu1.4, 50.1-0ubuntu1.5), gnome-shell-common:amd64 (50.1-0ub").encode(),
]).lstrip(b"\n")

ONE = (run("2026-09-15  06:40:10", "2026-09-15  06:40:31",
           "Commandline: /usr/bin/unattended-upgrade",
           "Upgrade: tzdata:amd64 (2026a-1ubuntu1, 2026b-0ubuntu0.26.04)")
       + run("2026-10-02  20:11:05", "2026-10-02  20:11:50",
             "Commandline: aptdaemon role='role-commit-packages' sender=':1.97'", ROBIN,
             "Upgrade: ubuntu-drivers-common:amd64 (1:0.10.4, 1:0.10.4.1)")).lstrip("\n")

TWO = run("2026-08-01  06:30:00", "2026-08-01  06:30:12",
          "Commandline: /usr/bin/unattended-upgrade",
          "Upgrade: libexpat1:amd64 (2.7.1-1, 2.7.1-1ubuntu0.1)").lstrip("\n")

BIG = ", ".join(f"lib{w}{n}:amd64 (1.{n}-1, 1.{n}-1ubuntu0.1)"
                for n, w in enumerate(["alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf", "hotel"] * 40))
THREE = (run("2026-07-10  06:25:00", "2026-07-10  06:25:08",
             "Commandline: /usr/bin/unattended-upgrade",
             "Upgrade: libpng16-16t64:amd64 (1.6.48-1, 1.6.48-1ubuntu0.1)")
         + run("2026-07-20  19:00:00", "2026-07-20  19:00:30",
               "Commandline: apt-get install -y htop", ROBIN,
               "Install: htop:amd64 (3.4.1-1)")
         + run("2026-07-28  06:10:00", "2026-07-28  06:12:00",
               "Commandline: /usr/bin/unattended-upgrade",
               f"Upgrade: {BIG}")).lstrip("\n")

FOUR_A = run("2026-06-05  06:20:00", "2026-06-05  06:20:09",
             "Commandline: /usr/bin/unattended-upgrade",
             "Upgrade: libtiff6:amd64 (4.7.0-3ubuntu1, 4.7.0-3ubuntu1.1)").lstrip("\n")
FOUR_B = run("2026-06-18  21:00:00", "2026-06-18  21:01:00",
             "Commandline: aptitude install gimp", ROBIN,
             "Install: gimp:amd64 (3.0.4-1)")

UPGRADER = ("1:25.10.9", "1:26.04.14")
RELEASE = (run("2026-08-20  10:15:00", "2026-08-20  10:41:12", ROBIN,
               f"Upgrade: ubuntu-release-upgrader-core:amd64 ({UPGRADER[0]}, {UPGRADER[1]}), "
               "base-files:amd64 (14ubuntu1, 14ubuntu5), gnome-shell:amd64 (49.0-1ubuntu1, 50.1-0ubuntu1)",
               "Install: linux-image-7.0.0-14-generic:amd64 (7.0.0-14.14, automatic)")
           + run("2026-08-20  10:41:20", "2026-08-20  10:42:01", ROBIN,
                 "Remove: libgtk2.0-0t64:amd64 (2.24.33-7ubuntu1)")).lstrip("\n")
FAILED = run("2026-06-01  09:30:10", "2026-06-01  09:52:44", ROBIN,
             f"Upgrade: ubuntu-release-upgrader-core:amd64 ({UPGRADER[0]}, 1:26.04.11), base-files:amd64 (14ubuntu1, 14ubuntu5)",
             "Error: Sub-process /usr/bin/dpkg returned an error code (1)").lstrip("\n")
# Software Updater's partial upgrade (pkexec: Requested-By, python-apt: no Commandline), within one release
PARTIAL = run("2026-10-10  18:31:05", "2026-10-10  18:33:40", ROBIN,
              "Upgrade: update-manager-core:all (1:26.04.3, 1:26.04.4), "
              "ubuntu-release-upgrader-core:all (1:26.04.14, 1:26.04.15), "
              "libgtk-3-0t64:amd64 (3.24.50-1ubuntu1, 3.24.50-1ubuntu1.1)",
              "Install: linux-image-7.0.0-16-generic:amd64 (7.0.0-16.16, automatic)").lstrip("\n")


def cut_gzip(text: bytes, marker: bytes, absent: bytes) -> bytes:
    """The shortest prefix of gz(text) whose decompressed prefix contains `marker` but not yet `absent`."""
    full = gz(text)
    for n in range(20, len(full)):
        d = zlib.decompressobj(16 + zlib.MAX_WBITS)
        try:
            out = d.decompress(full[:n])
        except zlib.error:
            continue
        if marker in out and absent not in out:
            return full[:n]
    raise SystemExit("no cut point found")


def main() -> None:
    APT.mkdir(parents=True, exist_ok=True)
    (DIST / "20260820-1012").mkdir(parents=True, exist_ok=True)
    (DIST / "20261010-1830").mkdir(parents=True, exist_ok=True)
    (APT / "history.log").write_bytes(CURRENT)
    (APT / "history.log.1.gz").write_bytes(gz(ONE.encode()))
    (APT / "history.log.2.gz").write_bytes(gz(TWO.encode()))
    (APT / "history.log.3.gz").write_bytes(cut_gzip(THREE.encode(), b"libcharlie2:amd64 (1.2-1, 1.2-1ubuntu0.1)",
                                                    b"End-Date: 2026-07-28"))
    (APT / "history.log.4.gz").write_bytes(gz(FOUR_A.encode()) + gz(FOUR_B.encode()))
    (DIST / "history.log").write_text(PARTIAL, encoding="utf-8")
    (DIST / "20261010-1830" / "history.log").write_text(RELEASE, encoding="utf-8")
    (DIST / "20260820-1012" / "history.log").write_text(FAILED, encoding="utf-8")
    for p in sorted(HERE.rglob("*")):
        if p.is_file() and p.name != "make.py":
            print(p.relative_to(HERE), p.stat().st_size)


if __name__ == "__main__":
    main()
