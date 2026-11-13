#!/usr/bin/env python3
"""Install or remove the four system files of Win11 voor GNOME's Windows Update page, and nothing else.

Usage: python3 scripts/install-system.py [--dry-run] [--remove]

The files (notes/claude/windows-update-gnome.md §7), from host/data/ next to this script:
  /usr/libexec/win11-gnome/release-upgrades-helper                                   root:root 0755
  /usr/libexec/win11-gnome/release-upgrade-attach                                    root:root 0755
  /usr/share/polkit-1/actions/io.github.win11shell.gnome.release-upgrades.policy     root:root 0644
  /usr/share/polkit-1/actions/io.github.win11shell.gnome.release-upgrade-attach.policy root:root 0644
With them, Ubuntu's password dialog says what Instellingen › Update en herstel › Nieuwe Ubuntu-versies and "Venster
weer openen" do; without them everything else works (Settings then runs the Prompt change through /bin/sh).

Standalone on purpose: it never runs install.py, never touches ~/.local, the extension or GSettings, and
install.py/uninstall.py never call it (they print one hint line). It refuses to run as root (the password is asked
by pkexec) and refuses every argument but --dry-run and --remove. It copies the four files into a private stage
directory under $XDG_RUNTIME_DIR (fixed names), runs exactly one
  pkexec /bin/sh -c '<INSTALL_SCRIPT>' win11-gnome-system-install <stage> <sha256 of each file, in FILES order>
(--remove: pkexec /bin/sh -c '<REMOVE_SCRIPT>' win11-gnome-system-remove), and removes the stage again. The root
script accepts only the four known names, copies each into its own temporary directory, checks the checksum given on
the command line, and installs with install -o root -g root -m 0755|0644; removal deletes exactly the four paths and
the directory /usr/libexec/win11-gnome when it is empty. Re-running is idempotent. --dry-run prints the command and
the checksums, asks no password and installs nothing.
Messages: Win11's own language (settings.json, contracts C1: "en" is English, anything else Dutch).
"""
from __future__ import annotations
import hashlib
import json
import os
from pathlib import Path
import shlex
import shutil
import stat
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
DATA = ROOT/"host/data"
LIBEXEC = "/usr/libexec/win11-gnome"
ACTIONS = "/usr/share/polkit-1/actions"
# (name in host/data and in the stage, directory it is installed in, mode)
FILES = (
    ("release-upgrades-helper", LIBEXEC, 0o755),
    ("release-upgrade-attach", LIBEXEC, 0o755),
    ("io.github.win11shell.gnome.release-upgrades.policy", ACTIONS, 0o644),
    ("io.github.win11shell.gnome.release-upgrade-attach.policy", ACTIONS, 0o644),
)
SHELL = "/bin/sh"
INSTALL_NAME = "win11-gnome-system-install"
REMOVE_NAME = "win11-gnome-system-remove"

# The root scripts (constants: never built from input). Machine markers only: @@ok <path>, @@done, @@fail <code>.
# WIN11_SYSTEM_ROOT counts only when the script does not run as root (tests/update/test_helpers.sh runs them as the
# user into a temporary root); pkexec clears the environment anyway.
_COMMON = r"""set -u
PATH=/usr/sbin:/usr/bin:/sbin:/bin
export PATH
umask 022
fail() { echo "@@fail $1"; exit "${2:-1}"; }
root=
own="-o root -g root"
if [ "$(id -u)" != 0 ]; then
  [ -n "${WIN11_SYSTEM_ROOT:-}" ] || fail notroot 3
  case "$WIN11_SYSTEM_ROOT" in /*) ;; *) fail root 3 ;; esac
  root=$WIN11_SYSTEM_ROOT
  own=
fi
lib=$root/usr/libexec/win11-gnome
pol=$root/usr/share/polkit-1/actions
"""
INSTALL_SCRIPT = _COMMON + r"""[ "$#" -eq 5 ] || fail usage 2
stage=$1
shift
case "$stage" in /*) ;; *) fail stage 2 ;; esac
{ [ -d "$stage" ] && [ ! -L "$stage" ]; } || fail stage 2
for entry in "$stage"/* "$stage"/.[!.]* "$stage"/..?*; do
  [ -e "$entry" ] || [ -L "$entry" ] || continue
  case "${entry##*/}" in
    release-upgrades-helper|release-upgrade-attach) ;;
    io.github.win11shell.gnome.release-upgrades.policy|io.github.win11shell.gnome.release-upgrade-attach.policy) ;;
    *) fail unknown 2 ;;
  esac
done
work=$(mktemp -d) || fail tmp
trap 'rm -rf -- "$work"' EXIT
for name in release-upgrades-helper release-upgrade-attach \
    io.github.win11shell.gnome.release-upgrades.policy io.github.win11shell.gnome.release-upgrade-attach.policy; do
  want=$1
  shift
  case "$want" in *[!0-9a-f]*|"") fail checksum 2 ;; esac
  [ "${#want}" -eq 64 ] || fail checksum 2
  { [ -f "$stage/$name" ] && [ ! -L "$stage/$name" ]; } || fail missing 2
  cat -- "$stage/$name" > "$work/$name" || fail copy
  got=$(sha256sum -- "$work/$name") || fail checksum
  [ "${got%% *}" = "$want" ] || fail checksum 2
done
[ -d "$lib" ] || install -d $own -m 0755 -- "$lib" || fail mkdir
[ -d "$pol" ] || install -d $own -m 0755 -- "$pol" || fail mkdir
for name in release-upgrades-helper release-upgrade-attach; do
  install $own -m 0755 -- "$work/$name" "$lib/$name" || fail install
  echo "@@ok $lib/$name"
done
for name in io.github.win11shell.gnome.release-upgrades.policy io.github.win11shell.gnome.release-upgrade-attach.policy; do
  install $own -m 0644 -- "$work/$name" "$pol/$name" || fail install
  echo "@@ok $pol/$name"
done
sync
echo "@@done"
"""
REMOVE_SCRIPT = _COMMON + r"""[ "$#" -eq 0 ] || fail usage 2
for path in "$lib/release-upgrades-helper" "$lib/release-upgrade-attach" \
    "$pol/io.github.win11shell.gnome.release-upgrades.policy" "$pol/io.github.win11shell.gnome.release-upgrade-attach.policy"; do
  if [ -e "$path" ] || [ -L "$path" ]; then
    rm -f -- "$path" || fail remove
    echo "@@ok $path"
  fi
done
if [ -d "$lib" ] && [ ! -L "$lib" ] && rmdir -- "$lib" 2>/dev/null; then
  echo "@@ok $lib"
fi
sync
echo "@@done"
"""

# i18n: pairs (tools/i18n/check_pairs.py); inline, as these texts appear when something is broken (contracts C3).
T = {
    "usage": {"nl": "Gebruik: python3 scripts/install-system.py [--dry-run] [--remove]",
              "en": "Usage: python3 scripts/install-system.py [--dry-run] [--remove]"},
    "noRoot": {"nl": "Start dit niet als root of met sudo: pkexec vraagt zelf om je wachtwoord.",
               "en": "Don’t run this as root or with sudo: pkexec asks for your password itself."},
    "noRuntime": {"nl": "Er is geen eigen XDG_RUNTIME_DIR (een map van jou met rechten 0700), dus er is niets gedaan.",
                  "en": "There’s no private XDG_RUNTIME_DIR (a folder of yours with mode 0700), so nothing was done."},
    "noSource": {"nl": "Ontbrekend projectbestand: {file}", "en": "Missing project file: {file}"},
    "noPkexec": {"nl": "pkexec ontbreekt (pakket pkexec), dus er is niets gedaan.",
                 "en": "pkexec is missing (package pkexec), so nothing was done."},
    "intro": {"nl": "Win11 installeert vier systeembestanden. Daarmee zegt Ubuntu bij ‘Nieuwe Ubuntu-versies’ en ‘Venster weer openen’ waarvoor je wachtwoord nodig is:",
              "en": "Win11 installs four system files, so that Ubuntu’s password prompt says what it’s for when you use ‘New Ubuntu versions’ and ‘Show the upgrade window’:"},
    "introRemove": {"nl": "Win11 verwijdert zijn vier systeembestanden:", "en": "Win11 removes its four system files:"},
    "password": {"nl": "Ubuntu vraagt nu één keer om je wachtwoord.", "en": "Ubuntu now asks for your password once."},
    "dryRun": {"nl": "Proefdraaien: er wordt niets geïnstalleerd of verwijderd en niet om een wachtwoord gevraagd.",
               "en": "Dry run: nothing is installed or removed, and you won’t be asked for a password."},
    "command": {"nl": "Opdracht:", "en": "Command:"},
    "checksums": {"nl": "Controlegetallen (sha256):", "en": "Checksums (sha256):"},
    "noPassword": {"nl": "Je hebt het wachtwoord niet ingevuld, dus er is niets veranderd.",
                   "en": "You didn’t enter the password, so nothing was changed."},
    "wrongPassword": {"nl": "Het wachtwoord klopte niet, of dit account mag geen beheerder zijn. Er is niets veranderd.",
                      "en": "The password was wrong, or this account can’t act as an administrator. Nothing was changed."},
    "failed": {"nl": "Het is niet gelukt ({code}). Probeer het opnieuw.",
               "en": "That didn’t work ({code}). Try again."},
    "failedPartly": {"nl": "Het is niet gelukt ({code}). Hierboven staat wat al wel is gebeurd; probeer het opnieuw.",
                     "en": "That didn’t work ({code}). What was already done is listed above; try again."},
    "installed": {"nl": "Geïnstalleerd: {path}", "en": "Installed: {path}"},
    "removed": {"nl": "Verwijderd: {path}", "en": "Removed: {path}"},
    "doneInstall": {"nl": "Klaar. Instellingen gebruikt de nieuwe bestanden meteen; je hoeft niet opnieuw aan te melden.",
                    "en": "Done. Settings uses the new files right away; you don’t need to sign in again."},
    "doneRemove": {"nl": "Klaar. ‘Nieuwe Ubuntu-versies’ werkt verder via /bin/sh.",
                   "en": "Done. ‘New Ubuntu versions’ keeps working through /bin/sh."},
    "nothingRemoved": {"nl": "Er was niets te verwijderen.", "en": "There was nothing to remove."},
}
LANGUAGE = "nl"


def t(key: str, **params) -> str:
    pair = T[key]
    return (pair.get(LANGUAGE) or pair["nl"]).format_map(params)


def language() -> str:
    """Win11's language (contracts C1): settings.json `language` "en" is English, anything else (or no file) nl."""
    config = os.environ.get("XDG_CONFIG_HOME", "")
    base = Path(config) if config.startswith("/") else Path.home()/".config"
    try:
        value = json.loads((base/"win11-gnome/settings.json").read_text(encoding="utf-8")).get("language")
    except (OSError, ValueError, AttributeError):
        return "nl"
    return "en" if value == "en" else "nl"


def parse(argv: list[str]) -> tuple[bool, bool] | None:
    """(dry_run, remove), or None for anything but --dry-run and --remove, each at most once."""
    if len(argv) != len(set(argv)) or any(a not in ("--dry-run", "--remove") for a in argv):
        return None
    return "--dry-run" in argv, "--remove" in argv


def destinations() -> list[str]:
    return [f"{directory}/{name}" for name, directory, _mode in FILES]


def runtime_dir() -> Path | None:
    """$XDG_RUNTIME_DIR when it is an absolute directory of this user with mode 0700 (the stage stays private)."""
    value = os.environ.get("XDG_RUNTIME_DIR", "")
    if not value.startswith("/"):
        return None
    try:
        st = os.lstat(value)
    except OSError:
        return None
    if not stat.S_ISDIR(st.st_mode) or st.st_uid != os.geteuid() or st.st_mode & 0o077:
        return None
    return Path(value)


def stage_files(stage: Path) -> list[str]:
    """Copy the four files into the stage (fixed names, modes as installed); their sha256 in FILES order."""
    sums = []
    for name, _directory, mode in FILES:
        data = (DATA/name).read_bytes()
        fd = os.open(stage/name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
        with os.fdopen(fd, "wb") as out:
            out.write(data)
        os.chmod(stage/name, mode)
        sums.append(hashlib.sha256(data).hexdigest())
    return sums


def command(pkexec: str, stage: Path | None, sums: list[str], remove: bool) -> list[str]:
    if remove:
        return [pkexec, SHELL, "-c", REMOVE_SCRIPT, REMOVE_NAME]
    return [pkexec, SHELL, "-c", INSTALL_SCRIPT, INSTALL_NAME, str(stage), *sums]


def outcome(rc: int, out: str) -> tuple[str, dict]:
    """The message after pkexec: (key, params); params["paths"] are the @@ok paths, also of a run that failed halfway.
    126/127 count only when no @@ marker came (contracts of cleanup.rs)."""
    lines = out.splitlines()
    paths = [line[len("@@ok "):] for line in lines if line.startswith("@@ok ")]
    fail = next((line[len("@@fail "):].strip() for line in lines if line.startswith("@@fail ")), None)
    if fail is not None:
        return "failed", {"code": fail or "?", "paths": paths}
    if "@@" not in out and rc in (126, 127):
        return ("noPassword" if rc == 126 else "wrongPassword"), {"paths": []}
    if rc != 0 or "@@done" not in lines:
        return "failed", {"code": f"rc {rc}", "paths": paths}
    return "ok", {"paths": paths}


def main(argv: list[str]) -> int:
    global LANGUAGE
    LANGUAGE = language()
    if os.geteuid() == 0:
        print(t("noRoot"), file=sys.stderr)
        return 1
    parsed = parse(argv)
    if parsed is None:
        print(t("usage"), file=sys.stderr)
        return 2
    dry_run, remove = parsed
    for name, _directory, _mode in FILES:
        if not (DATA/name).is_file() or (DATA/name).is_symlink():
            print(t("noSource", file=DATA/name), file=sys.stderr)
            return 1
    runtime = runtime_dir()
    if runtime is None:
        print(t("noRuntime"), file=sys.stderr)
        return 1
    pkexec = shutil.which("pkexec")
    if pkexec is None:
        print(t("noPkexec"), file=sys.stderr)
        return 1
    print(t("introRemove") if remove else t("intro"))
    for path in destinations():
        print(f"  {path}")
    stage = None if remove else Path(tempfile.mkdtemp(prefix="win11-gnome-system-", dir=runtime))
    try:
        sums = [] if stage is None else stage_files(stage)
        argv_ = command(pkexec, stage, sums, remove)
        if dry_run:
            print(t("dryRun"))
            print(t("command"))
            print(shlex.join(argv_))
            if sums:
                print(t("checksums"))
                for digest, path in zip(sums, destinations()):
                    print(f"{digest}  {path}")
            return 0
        print(t("password"))
        sys.stdout.flush()
        proc = subprocess.run(argv_, stdout=subprocess.PIPE, text=True, errors="replace")
        key, params = outcome(proc.returncode, proc.stdout)
        if key == "noPassword":
            print(t("noPassword"), file=sys.stderr)
            return 1
        if key == "wrongPassword":
            print(t("wrongPassword"), file=sys.stderr)
            return 1
        # what was done, also when the run stopped halfway (then the failure says so)
        for path in params["paths"]:
            print(t("removed", path=path) if remove else t("installed", path=path))
        sys.stdout.flush()
        if key == "failed":
            code = params["code"]
            print(t("failedPartly", code=code) if params["paths"] else t("failed", code=code), file=sys.stderr)
            return 1
        if remove and not params["paths"]:
            print(t("nothingRemoved"))
        print(t("doneRemove") if remove else t("doneInstall"))
        return 0
    finally:
        if stage is not None:
            shutil.rmtree(stage, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
