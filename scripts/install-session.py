#!/usr/bin/env python3
"""Install or remove WinPoint's own choice at the login screen, next to GNOME and Ubuntu, and nothing else.

Usage: python3 scripts/install-session.py [--dry-run] [--remove]

Robin (2026-11-13): "WinPoint als optie bij inloggen als keuze", "zodat nieuwe pc's netjes het kunnen installeren".
The four system files (GNOME 50: gnome-session starts gnome-session@<name>.target, which pulls GNOME Shell in the
session mode of that name; Ubuntu's own "Ubuntu" choice is made the same way):

  /usr/share/wayland-sessions/winpoint.desktop                                   GDM's list: "WinPoint"
  /usr/share/gnome-session/sessions/winpoint.session                             the session's name
  /usr/lib/systemd/user/gnome-session@winpoint.target.d/winpoint.session.conf    GNOME Shell in mode "winpoint"
  /usr/share/gnome-shell/modes/winpoint.json                                     GNOME's user mode with the extension

The mode turns win11-gnome@win11shell.local on in the WinPoint session only (it is not in the user's
enabled-extensions any more, see host/src/gnome_look.rs), with Ubuntu's Yaru shell theme when the system has it and
without Ubuntu's dock. XDG_CURRENT_DESKTOP is WinPoint:GNOME, so GNOME apps and portals keep working, and the
FileChooser portal choice lives in ~/.config/xdg-desktop-portal/winpoint-portals.conf (install.py).

scripts/install.py runs this when the files are missing (unless --no-session); it can also run on its own. It
refuses to run as root (pkexec asks the password), stages the files under $XDG_RUNTIME_DIR, runs exactly one
  pkexec /bin/sh -c '<INSTALL_SCRIPT>' win11-gnome-session-install <stage> <sha256 of each file, in FILES order>
(--remove: pkexec /bin/sh -c '<REMOVE_SCRIPT>' win11-gnome-session-remove) and removes the stage again. The root
script accepts only the four known names, checks each checksum and installs them root:root 0644; removal deletes
exactly the four paths and the drop-in directory when it is empty. Idempotent. --dry-run asks no password.
Messages: Win11's own language (settings.json, contracts C1: "en" is English, anything else Dutch).
"""
from __future__ import annotations
import hashlib
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import sys
import tempfile

UUID = "win11-gnome@win11shell.local"
NAME = "winpoint"
SESSIONS = "/usr/share/wayland-sessions"
GSESSIONS = "/usr/share/gnome-session/sessions"
DROPIN = "/usr/lib/systemd/user/gnome-session@winpoint.target.d"
MODES = "/usr/share/gnome-shell/modes"
# (name in the stage, directory it is installed in)
FILES = (
    ("winpoint.desktop", SESSIONS),
    ("winpoint.session", GSESSIONS),
    ("winpoint.session.conf", DROPIN),
    ("winpoint.json", MODES),
)
MARKER = f"{SESSIONS}/winpoint.desktop"
SHELL = "/bin/sh"
INSTALL_NAME = "win11-gnome-session-install"
REMOVE_NAME = "win11-gnome-session-remove"

# The root scripts (constants: never built from input). Machine markers only: @@ok <path>, @@done, @@fail <code>.
# WIN11_SYSTEM_ROOT counts only when the script does not run as root (a test into a temporary root).
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
ses=$root/usr/share/wayland-sessions
gses=$root/usr/share/gnome-session/sessions
drop=$root/usr/lib/systemd/user/gnome-session@winpoint.target.d
modes=$root/usr/share/gnome-shell/modes
"""
INSTALL_SCRIPT = _COMMON + r"""[ "$#" -eq 5 ] || fail usage 2
stage=$1
shift
case "$stage" in /*) ;; *) fail stage 2 ;; esac
{ [ -d "$stage" ] && [ ! -L "$stage" ]; } || fail stage 2
for entry in "$stage"/* "$stage"/.[!.]* "$stage"/..?*; do
  [ -e "$entry" ] || [ -L "$entry" ] || continue
  case "${entry##*/}" in
    winpoint.desktop|winpoint.session|winpoint.session.conf|winpoint.json) ;;
    *) fail unknown 2 ;;
  esac
done
work=$(mktemp -d) || fail tmp
trap 'rm -rf -- "$work"' EXIT
for name in winpoint.desktop winpoint.session winpoint.session.conf winpoint.json; do
  want=$1
  shift
  case "$want" in *[!0-9a-f]*|"") fail checksum 2 ;; esac
  [ "${#want}" -eq 64 ] || fail checksum 2
  { [ -f "$stage/$name" ] && [ ! -L "$stage/$name" ]; } || fail missing 2
  cat -- "$stage/$name" > "$work/$name" || fail copy
  got=$(sha256sum -- "$work/$name") || fail checksum
  [ "${got%% *}" = "$want" ] || fail checksum 2
done
for dir in "$ses" "$gses" "$drop" "$modes"; do
  [ -d "$dir" ] || install -d $own -m 0755 -- "$dir" || fail mkdir
done
install $own -m 0644 -- "$work/winpoint.desktop" "$ses/winpoint.desktop" || fail install
echo "@@ok $ses/winpoint.desktop"
install $own -m 0644 -- "$work/winpoint.session" "$gses/winpoint.session" || fail install
echo "@@ok $gses/winpoint.session"
install $own -m 0644 -- "$work/winpoint.session.conf" "$drop/winpoint.session.conf" || fail install
echo "@@ok $drop/winpoint.session.conf"
install $own -m 0644 -- "$work/winpoint.json" "$modes/winpoint.json" || fail install
echo "@@ok $modes/winpoint.json"
if [ -z "$root" ] && command -v systemctl >/dev/null; then systemctl --global daemon-reload 2>/dev/null || true; fi
sync
echo "@@done"
"""
REMOVE_SCRIPT = _COMMON + r"""[ "$#" -eq 0 ] || fail usage 2
for path in "$ses/winpoint.desktop" "$gses/winpoint.session" "$drop/winpoint.session.conf" "$modes/winpoint.json"; do
  if [ -e "$path" ] || [ -L "$path" ]; then
    rm -f -- "$path" || fail remove
    echo "@@ok $path"
  fi
done
if [ -d "$drop" ] && [ ! -L "$drop" ] && rmdir -- "$drop" 2>/dev/null; then
  echo "@@ok $drop"
fi
sync
echo "@@done"
"""

# i18n: pairs (tools/i18n/check_pairs.py); inline, as these texts appear when something is broken (contracts C3).
T = {
    "usage": {"nl": "Gebruik: python3 scripts/install-session.py [--dry-run] [--remove]",
              "en": "Usage: python3 scripts/install-session.py [--dry-run] [--remove]"},
    "noRoot": {"nl": "Start dit niet als root of met sudo: pkexec vraagt zelf om je wachtwoord.",
               "en": "Don’t run this as root or with sudo: pkexec asks for your password itself."},
    "noRuntime": {"nl": "Er is geen eigen XDG_RUNTIME_DIR (een map van jou met rechten 0700), dus er is niets gedaan.",
                  "en": "There’s no private XDG_RUNTIME_DIR (a folder of yours with mode 0700), so nothing was done."},
    "noPkexec": {"nl": "pkexec ontbreekt (pakket pkexec), dus er is niets gedaan.",
                 "en": "pkexec is missing (package pkexec), so nothing was done."},
    "noGnome": {"nl": "gnome-session ontbreekt, dus WinPoint kan geen eigen keuze bij het inloggen krijgen.",
                "en": "gnome-session is missing, so WinPoint can’t get its own choice at the login screen."},
    "intro": {"nl": "WinPoint wordt een eigen keuze bij het inloggen, naast GNOME en Ubuntu. Daarvoor komen er vier systeembestanden:",
              "en": "WinPoint becomes its own choice at the login screen, next to GNOME and Ubuntu. That takes four system files:"},
    "introRemove": {"nl": "WinPoint verdwijnt als keuze bij het inloggen. Deze systeembestanden gaan weg:",
                    "en": "WinPoint is removed as a choice at the login screen. These system files go:"},
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
    "doneInstall": {"nl": "Klaar. Meld je af en kies WinPoint bij het inloggen (het tandwiel rechtsonder, na je naam).",
                    "en": "Done. Sign out and choose WinPoint at the login screen (the gear at the bottom right, after your name)."},
    "doneRemove": {"nl": "Klaar. Kies bij het inloggen weer GNOME of Ubuntu.",
                   "en": "Done. Choose GNOME or Ubuntu at the login screen again."},
    "nothingRemoved": {"nl": "Er was niets te verwijderen.", "en": "There was nothing to remove."},
}
LANGUAGE = "nl"


def t(key: str, **params) -> str:
    pair = T[key]
    return (pair.get(LANGUAGE) or pair["nl"]).format_map(params)


def language() -> str:
    config = os.environ.get("XDG_CONFIG_HOME", "")
    base = Path(config) if config.startswith("/") else Path.home()/".config"
    try:
        value = json.loads((base/"win11-gnome/settings.json").read_text(encoding="utf-8")).get("language")
    except (OSError, ValueError, AttributeError):
        return "nl"
    return "en" if value == "en" else "nl"


def installed(root: Path = Path("/")) -> bool:
    """All four files are there (install.py then sets up the user side for the WinPoint session)."""
    return all((root/directory.lstrip("/")/name).is_file() for name, directory in FILES)


def mode_json(root: Path = Path("/")) -> str:
    """GNOME's user mode with only our extension; Ubuntu's Yaru shell theme when the system has its mode file."""
    mode = {"parentMode": "user"}
    try:
        ubuntu = json.loads((root/"usr/share/gnome-shell/modes/ubuntu.json").read_text(encoding="utf-8"))
        for key in ("stylesheetName", "themeResourceName", "iconsResourceName"):
            if isinstance(ubuntu.get(key), str):
                mode[key] = ubuntu[key]
    except (OSError, ValueError, AttributeError):
        pass
    mode["enabledExtensions"] = [UUID]
    return json.dumps(mode, indent=4) + "\n"


def contents(root: Path = Path("/")) -> dict[str, bytes]:
    return {
        "winpoint.desktop": (
            "[Desktop Entry]\n"
            "Name=WinPoint\n"
            "Comment=A Windows 11 style desktop on GNOME\n"
            "Comment[nl]=Een Windows 11-achtig bureaublad op GNOME\n"
            "Exec=/usr/bin/gnome-session --session=winpoint\n"
            "TryExec=/usr/bin/gnome-session\n"
            "Type=Application\n"
            "DesktopNames=WinPoint;GNOME\n"
            "X-GDM-SessionRegisters=true\n"
            "X-GDM-CanRunHeadless=true\n").encode(),
        "winpoint.session": b"[GNOME Session]\nName=WinPoint\n",
        "winpoint.session.conf": (
            "[Unit]\n"
            "Requires=gnome-session-services.target\n"
            "Requires=org.gnome.Shell@winpoint.service\n").encode(),
        "winpoint.json": mode_json(root).encode(),
    }


def parse(argv: list[str]) -> tuple[bool, bool] | None:
    if len(argv) != len(set(argv)) or any(a not in ("--dry-run", "--remove") for a in argv):
        return None
    return "--dry-run" in argv, "--remove" in argv


def destinations() -> list[str]:
    return [f"{directory}/{name}" for name, directory in FILES]


def runtime_dir() -> Path | None:
    value = os.environ.get("XDG_RUNTIME_DIR", "")
    if not value.startswith("/"):
        return None
    try:
        st = os.lstat(value)
    except OSError:
        return None
    import stat
    if not stat.S_ISDIR(st.st_mode) or st.st_uid != os.geteuid() or st.st_mode & 0o077:
        return None
    return Path(value)


def stage_files(stage: Path) -> list[str]:
    sums = []
    data = contents()
    for name, _directory in FILES:
        fd = os.open(stage/name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
        with os.fdopen(fd, "wb") as out:
            out.write(data[name])
        os.chmod(stage/name, 0o644)
        sums.append(hashlib.sha256(data[name]).hexdigest())
    return sums


def command(pkexec: str, stage: Path | None, sums: list[str], remove: bool) -> list[str]:
    if remove:
        return [pkexec, SHELL, "-c", REMOVE_SCRIPT, REMOVE_NAME]
    return [pkexec, SHELL, "-c", INSTALL_SCRIPT, INSTALL_NAME, str(stage), *sums]


def outcome(rc: int, out: str) -> tuple[str, dict]:
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
    if not remove and not Path("/usr/bin/gnome-session").is_file():
        print(t("noGnome"), file=sys.stderr)
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
    stage = None if remove else Path(tempfile.mkdtemp(prefix="win11-gnome-session-", dir=runtime))
    try:
        sums = [] if stage is None else stage_files(stage)
        argv_ = command(pkexec, stage, sums, remove)
        if dry_run:
            print(t("dryRun"))
            print(t("command"))
            print(shlex.join(argv_[:3] + ["<script>"] + argv_[4:]))
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
