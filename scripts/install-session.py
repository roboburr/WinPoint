#!/usr/bin/env python3
"""Install or remove WinPoint's own choice at the login screen, next to Ubuntu, and nothing else.

Usage: python3 scripts/install-session.py [--dry-run] [--remove] [--system] [--lang nl|en]

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
FileChooser portal choice lives in ~/.config/xdg-desktop-portal/winpoint-portals.conf (install.py). contents() is
frozen: its bytes for a system are 0.9.0's (D-G74: --check of a newer installer reports a difference as "session",
and WinPoint's updater then sends every user to the terminal); tests/installer compares it with v0.9.0's.

0.9.1 (Robin, 16-11: one password for everything): --system adds install-system.py's four files (host/data next to
this script, SYSTEM_FILES) to the same pkexec, so a terminal install of scripts/install.py asks the password once.
scripts/install.py runs this on a terminal when one of those files is missing or differs (stale()); it can also run
on its own. It refuses to run as root (pkexec asks the password), stages the files under $XDG_RUNTIME_DIR, runs
exactly one
  pkexec /bin/sh -c '<INSTALL_SCRIPT>' win11-gnome-session-install <stage> <sha256 of each file, in FILES order,
                                                                           then SYSTEM_FILES order with --system>
(--remove: pkexec /bin/sh -c '<REMOVE_SCRIPT>' win11-gnome-session-remove) and removes the stage again. The root
script accepts only the known names (the system ones only with their four checksums), no link and nothing but plain
files in the stage, checks each checksum in its own copy and installs root:root 0644 (the two helpers 0755), GDM's
list entry winpoint.desktop last of the session's four; removal deletes exactly the eight paths that exist, then the
drop-in directory and /usr/libexec/win11-gnome when they are empty, and asks no password when none of them exists
(scripts/uninstall.py runs it; its texts speak of the login screen only while winpoint.desktop is there).
Idempotent. --dry-run asks no password. The password line before pkexec says what the dialog shows (/bin/sh) and
that a non-administrator needs an administrator's password; install.py adds no explanation of its own.
Messages: --lang (install.py passes the language it resolved: a first install has no settings.json yet), else Win11's
own language (settings.json, contracts C1: "en" is English, anything else Dutch).
WIN11_SYSTEM_ROOT (tests only): as for the root scripts, it counts only when this does not run as root; the system's
files are then read below it (system_root()). pkexec clears it, so a real root script never sees it.
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
# install-system.py's four files (its FILES: the same names, directories and modes; tests/installer holds them equal),
# from host/data next to this script
LIBEXEC = "/usr/libexec/win11-gnome"
ACTIONS = "/usr/share/polkit-1/actions"
SYSTEM_FILES = (
    ("release-upgrades-helper", LIBEXEC, 0o755),
    ("release-upgrade-attach", LIBEXEC, 0o755),
    ("io.github.win11shell.gnome.release-upgrades.policy", ACTIONS, 0o644),
    ("io.github.win11shell.gnome.release-upgrade-attach.policy", ACTIONS, 0o644),
)
DATA = Path(__file__).resolve().parents[1]/"host/data"
# The keys mode_json() copies from the system's ubuntu.json: an Ubuntu update may change them, winpoint.json stays
# WinPoint's (same_mode)
MODE_KEYS = ("stylesheetName", "themeResourceName", "iconsResourceName")
MARKER = f"{SESSIONS}/winpoint.desktop"
SHELL = "/bin/sh"
INSTALL_NAME = "win11-gnome-session-install"
REMOVE_NAME = "win11-gnome-session-remove"
LANGUAGES = ("nl", "en")

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
lib=$root/usr/libexec/win11-gnome
pol=$root/usr/share/polkit-1/actions
"""
# 5 arguments: the stage and the four session checksums; 9: also install-system.py's four (--system). GDM's list
# entry (winpoint.desktop) is the last of the four it installs: a run that stops halfway (a full disk, Ctrl+C after
# the password) never leaves a "WinPoint" at the login screen without its shell mode; install-system.py's four follow,
# so when only they fail the session is complete (install.py warns and goes on).
INSTALL_SCRIPT = _COMMON + r"""[ "$#" -eq 5 ] || [ "$#" -eq 9 ] || fail usage 2
stage=$1
shift
system=
[ "$#" -eq 8 ] && system=1
case "$stage" in /*) ;; *) fail stage 2 ;; esac
{ [ -d "$stage" ] && [ ! -L "$stage" ]; } || fail stage 2
for entry in "$stage"/* "$stage"/.[!.]* "$stage"/..?*; do
  [ -e "$entry" ] || [ -L "$entry" ] || continue
  case "${entry##*/}" in
    winpoint.desktop|winpoint.session|winpoint.session.conf|winpoint.json) ;;
    release-upgrades-helper|release-upgrade-attach) [ -n "$system" ] || fail unknown 2 ;;
    io.github.win11shell.gnome.release-upgrades.policy|io.github.win11shell.gnome.release-upgrade-attach.policy)
      [ -n "$system" ] || fail unknown 2 ;;
    *) fail unknown 2 ;;
  esac
done
names="winpoint.desktop winpoint.session winpoint.session.conf winpoint.json"
if [ -n "$system" ]; then
  names="$names release-upgrades-helper release-upgrade-attach"
  names="$names io.github.win11shell.gnome.release-upgrades.policy io.github.win11shell.gnome.release-upgrade-attach.policy"
fi
work=$(mktemp -d) || fail tmp
trap 'rm -rf -- "$work"' EXIT
for name in $names; do
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
install $own -m 0644 -- "$work/winpoint.session" "$gses/winpoint.session" || fail install
echo "@@ok $gses/winpoint.session"
install $own -m 0644 -- "$work/winpoint.session.conf" "$drop/winpoint.session.conf" || fail install
echo "@@ok $drop/winpoint.session.conf"
install $own -m 0644 -- "$work/winpoint.json" "$modes/winpoint.json" || fail install
echo "@@ok $modes/winpoint.json"
install $own -m 0644 -- "$work/winpoint.desktop" "$ses/winpoint.desktop" || fail install
echo "@@ok $ses/winpoint.desktop"
if [ -n "$system" ]; then
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
fi
if [ -z "$root" ] && command -v systemctl >/dev/null; then systemctl --global daemon-reload 2>/dev/null || true; fi
sync
echo "@@done"
"""
REMOVE_SCRIPT = _COMMON + r"""[ "$#" -eq 0 ] || fail usage 2
for path in "$ses/winpoint.desktop" "$gses/winpoint.session" "$drop/winpoint.session.conf" "$modes/winpoint.json" \
    "$lib/release-upgrades-helper" "$lib/release-upgrade-attach" \
    "$pol/io.github.win11shell.gnome.release-upgrades.policy" "$pol/io.github.win11shell.gnome.release-upgrade-attach.policy"; do
  if [ -e "$path" ] || [ -L "$path" ]; then
    rm -f -- "$path" || fail remove
    echo "@@ok $path"
  fi
done
for dir in "$drop" "$lib"; do
  if [ -d "$dir" ] && [ ! -L "$dir" ] && rmdir -- "$dir" 2>/dev/null; then
    echo "@@ok $dir"
  fi
done
sync
echo "@@done"
"""

# i18n: pairs (tools/i18n/check_pairs.py); inline, as these texts appear when something is broken (contracts C3).
T = {
    "usage": {"nl": "Gebruik: python3 scripts/install-session.py [--dry-run] [--remove] [--system] [--lang nl|en]",
              "en": "Usage: python3 scripts/install-session.py [--dry-run] [--remove] [--system] [--lang nl|en]"},
    "noRoot": {"nl": "Start dit niet als root of met sudo: pkexec vraagt zelf om je wachtwoord.",
               "en": "Don’t run this as root or with sudo: pkexec asks for your password itself."},
    "noRuntime": {"nl": "Er is geen eigen XDG_RUNTIME_DIR (een map van jou met rechten 0700), dus er is niets gedaan.",
                  "en": "There’s no private XDG_RUNTIME_DIR (a folder of yours with mode 0700), so nothing was done."},
    "noPkexec": {"nl": "pkexec ontbreekt (pakket pkexec), dus er is niets gedaan.",
                 "en": "pkexec is missing (package pkexec), so nothing was done."},
    "noGnome": {"nl": "gnome-session ontbreekt, dus WinPoint kan geen eigen keuze bij het inloggen krijgen.",
                "en": "gnome-session is missing, so WinPoint can’t get its own choice at the login screen."},
    "noSource": {"nl": "Ontbrekend projectbestand: {file}", "en": "Missing project file: {file}"},
    "intro": {"nl": "WinPoint wordt een eigen keuze bij het inloggen, naast Ubuntu. Daarvoor komen er vier systeembestanden:",
              "en": "WinPoint becomes its own choice at the login screen, next to Ubuntu. That takes four system files:"},
    "introSystem": {"nl": "En de vier systeembestanden voor ‘Nieuwe Ubuntu-versies’ en ‘Venster weer openen’ in Instellingen:",
                    "en": "And the four system files for ‘New Ubuntu versions’ and ‘Show the upgrade window’ in Settings:"},
    "introRemove": {"nl": "WinPoint verdwijnt als keuze bij het inloggen. Deze systeembestanden gaan weg:",
                    "en": "WinPoint is removed as a choice at the login screen. These system files go:"},
    # --remove without winpoint.desktop (GDM's list entry): only install-system.py's files of an install inside the
    # normal GNOME session, or what is left of the session's
    "introRemoveSystem": {"nl": "Deze systeembestanden van WinPoint gaan weg:", "en": "These system files of WinPoint go:"},
    # 0.9.1: the one explanation before the password (install.py prints none of its own): what the window names, and
    # whose password
    "password": {"nl": "Ubuntu vraagt nu één keer om je wachtwoord. Het wachtwoordvenster noemt /bin/sh: het korte script dat de bestanden plaatst. Ben je geen beheerder, vul dan het wachtwoord van een beheerder in.",
                 "en": "Ubuntu now asks for your password once. The password window names /bin/sh: the short script that copies the files. If you’re not an administrator, enter an administrator’s password."},
    "passwordRemove": {"nl": "Ubuntu vraagt nu één keer om je wachtwoord. Het wachtwoordvenster noemt /bin/sh: het korte script dat de bestanden verwijdert. Ben je geen beheerder, vul dan het wachtwoord van een beheerder in.",
                       "en": "Ubuntu now asks for your password once. The password window names /bin/sh: the short script that removes the files. If you’re not an administrator, enter an administrator’s password."},
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
    # true on its own and from install.py, which then makes WinPoint the choice at the next sign-in itself
    "doneInstall": {"nl": "Klaar. Na het afmelden is WinPoint een keuze bij het inloggen.",
                    "en": "Done. Once you sign out, WinPoint is a choice at the login screen."},
    "doneRemove": {"nl": "Klaar. Bij het inloggen start weer je gewone Ubuntu-sessie.",
                   "en": "Done. The login screen starts your normal Ubuntu session again."},
    "doneRemoveSystem": {"nl": "Klaar. De systeembestanden van WinPoint zijn verwijderd.",
                         "en": "Done. WinPoint’s system files are removed."},
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


def system_root() -> Path:
    """/, or WIN11_SYSTEM_ROOT (absolute) when this does not run as root: the root scripts' own rule (tests only)."""
    value = os.environ.get("WIN11_SYSTEM_ROOT", "")
    return Path(value) if value.startswith("/") and os.geteuid() != 0 else Path("/")


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


def read_bytes(path: Path) -> bytes | None:
    try:
        return path.read_bytes() if path.is_file() else None
    except OSError:
        return None


def same_mode(installed_mode: bytes | None, wanted: bytes) -> bool:
    """winpoint.json by meaning: the same JSON object once the keys copied from the system's ubuntu.json (MODE_KEYS)
    are left out of both."""
    try:
        have, want = json.loads(installed_mode), json.loads(wanted)
    except (TypeError, ValueError, UnicodeError):
        return False
    if not isinstance(have, dict) or not isinstance(want, dict):
        return False
    return {k: v for k, v in have.items() if k not in MODE_KEYS} == {k: v for k, v in want.items() if k not in MODE_KEYS}


def stale(root: Path = Path("/")) -> list[str]:
    """The names of FILES that are missing below root or differ from contents(root): winpoint.json by meaning
    (same_mode), the other three byte for byte."""
    wanted = contents(root)
    found = []
    for name, directory in FILES:
        have = read_bytes(root/directory.lstrip("/")/name)
        if not (same_mode(have, wanted[name]) if name == "winpoint.json" else have == wanted[name]):
            found.append(name)
    return found


def system_sources() -> bool:
    """host/data has install-system.py's four files, plain files (a release bundle and a checkout have them)."""
    return all((DATA/name).is_file() and not (DATA/name).is_symlink() for name, _directory, _mode in SYSTEM_FILES)


def system_stale(root: Path = Path("/")) -> list[str]:
    """The names of SYSTEM_FILES that are missing below root or differ from host/data, byte for byte."""
    return [name for name, directory, _mode in SYSTEM_FILES
            if read_bytes(root/directory.lstrip("/")/name) != read_bytes(DATA/name)]


def removable(root: Path = Path("/")) -> list[str]:
    """Those of the eight paths that are there below root, a file or a link (REMOVE_SCRIPT removes exactly them), as
    installed: /usr/…"""
    paths = [f"{directory}/{name}" for name, directory in FILES]
    paths += [f"{directory}/{name}" for name, directory, _mode in SYSTEM_FILES]
    return [path for path in paths if (root/path.lstrip("/")).is_symlink() or (root/path.lstrip("/")).is_file()]


def parse(argv: list[str]) -> dict | None:
    """{"dry_run", "remove", "system", "lang"}, or None for anything else, a flag twice or a language but nl/en."""
    seen, options, index = set(), {"dry_run": False, "remove": False, "system": False, "lang": None}, 0
    while index < len(argv):
        argument = argv[index]
        name, value = argument, None
        if argument == "--lang":
            index += 1
            value = argv[index] if index < len(argv) else ""
        elif argument.startswith("--lang="):
            name, value = "--lang", argument.split("=", 1)[1]
        if name in seen or name not in ("--dry-run", "--remove", "--system", "--lang"):
            return None
        seen.add(name)
        if name == "--lang":
            if value.strip().lower() not in LANGUAGES:
                return None
            options["lang"] = value.strip().lower()
        else:
            options[name[2:].replace("-", "_")] = True
        index += 1
    return options


def lang_option(argv: list[str]) -> str | None:
    """A valid --lang of argv, also when the rest is wrong (the usage message follows it)."""
    for index, argument in enumerate(argv):
        value = argument.split("=", 1)[1] if argument.startswith("--lang=") else \
            argv[index + 1] if argument == "--lang" and index + 1 < len(argv) else None
        if value is not None and value.strip().lower() in LANGUAGES:
            return value.strip().lower()
    return None


def destinations(system: bool = False) -> list[str]:
    paths = [f"{directory}/{name}" for name, directory in FILES]
    return paths + ([f"{directory}/{name}" for name, directory, _mode in SYSTEM_FILES] if system else [])


def runtime_dir() -> Path | None:
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


def stage_files(stage: Path, system: bool = False) -> list[str]:
    """The four session files (contents(), for this system) and with system install-system.py's four from host/data
    into the stage, fixed names; their sha256 in FILES, then SYSTEM_FILES order."""
    sums = []
    data = contents(system_root())
    rows = [(name, data[name], 0o644) for name, _directory in FILES]
    if system:
        rows += [(name, (DATA/name).read_bytes(), mode) for name, _directory, mode in SYSTEM_FILES]
    for name, value, mode in rows:
        fd = os.open(stage/name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
        with os.fdopen(fd, "wb") as out:
            out.write(value)
        os.chmod(stage/name, mode)
        sums.append(hashlib.sha256(value).hexdigest())
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
    LANGUAGE = lang_option(argv) or language()
    if os.geteuid() == 0:
        print(t("noRoot"), file=sys.stderr)
        return 1
    parsed = parse(argv)
    if parsed is None:
        print(t("usage"), file=sys.stderr)
        return 2
    dry_run, remove, system = parsed["dry_run"], parsed["remove"], parsed["system"] and not parsed["remove"]
    if not remove and not Path("/usr/bin/gnome-session").is_file():
        print(t("noGnome"), file=sys.stderr)
        return 1
    if system and not system_sources():
        missing = next(DATA/name for name, _directory, _mode in SYSTEM_FILES
                       if not (DATA/name).is_file() or (DATA/name).is_symlink())
        print(t("noSource", file=missing), file=sys.stderr)
        return 1
    runtime = runtime_dir()
    if runtime is None:
        print(t("noRuntime"), file=sys.stderr)
        return 1
    pkexec = shutil.which("pkexec")
    if pkexec is None:
        print(t("noPkexec"), file=sys.stderr)
        return 1
    # --remove speaks of the login screen only while its list entry is there (removable() says what is)
    login = False
    if remove:
        present = removable(system_root())
        if not present:
            print(t("nothingRemoved"))
            return 0
        login = MARKER in present
        print(t("introRemove") if login else t("introRemoveSystem"))
        for path in present:
            print(f"  {path}")
    else:
        print(t("intro"))
        for path in destinations():
            print(f"  {path}")
        if system:
            print(t("introSystem"))
            for path in destinations(True)[len(FILES):]:
                print(f"  {path}")
    stage = None if remove else Path(tempfile.mkdtemp(prefix="win11-gnome-session-", dir=runtime))
    try:
        sums = [] if stage is None else stage_files(stage, system)
        argv_ = command(pkexec, stage, sums, remove)
        if dry_run:
            print(t("dryRun"))
            print(t("command"))
            print(shlex.join(argv_[:3] + ["<script>"] + argv_[4:]))
            if sums:
                print(t("checksums"))
                for digest, path in zip(sums, destinations(system)):
                    print(f"{digest}  {path}")
            return 0
        print(t("passwordRemove") if remove else t("password"))
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
        print(t("doneInstall") if not remove else t("doneRemove") if login else t("doneRemoveSystem"))
        return 0
    finally:
        if stage is not None:
            shutil.rmtree(stage, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
