#!/usr/bin/env python3
"""Install only this project's files; do not change the current desktop session.

Usage: scripts/install.sh [--lang nl|en]   (install.sh runs: /usr/bin/python3 -E -s -B install.py ROOT [...])

WinPoint's own choice at the login screen (Robin, 13-11), and since 0.9.1 the default one with one password (Robin,
16-11: "zolang de originele gnome blijft mag je doen wat je wil"). A terminal install (a tty on stdin, no
--no-session, WIN11_NO_SESSION not 1) always goes through WinPoint's own login session:
  * when one of the eight root files (scripts/install-session.py's four, install-system.py's four from host/data) is
    missing or differs from what this tree writes (winpoint.json by meaning, install-session.py stale()), root_step()
    runs install-session.py --system once, after every check and before the first change: one pkexec, one password.
    Without the session files afterwards it refuses (sessionNeeded) and nothing has changed; when only some of the
    rest did not make it, one warning line, and the install goes on. apply()'s rollback cannot take those files
    back, so a failure after this step says so and names install-session.py --remove (rolledBackRoot);
  * after the commit it makes WinPoint this user's choice at the next sign-in (AccountsService SetSession, busctl;
    polkit allows it without a password, and busctl never lets it ask for one; when it fails the gear instruction
    follows) and takes WinPoint's UUID out of org.gnome.shell enabled-extensions (an in-GNOME install of 0.9.0: the
    extension would also run in the normal session; one line says so) and then disabled-extensions (the WinPoint
    session's mode skips an extension listed there);
  * not on a terminal it refuses (sessionTerminal), without gnome-session or pkexec too (missingParts); as root
    always (noRoot).
The user's normal GNOME and Ubuntu sessions stay as they were: the extension runs in the WinPoint session only (its
shell mode turns it on), the FileChooser portal choice goes to winpoint-portals.conf, and the autostart entry
GnomeLook (NotShowIn=WinPoint) gives them their own look back at every login (host/src/gnome_look.rs). --no-session
and WIN11_NO_SESSION=1 (WinPoint's updater, below) keep 0.9.0's behaviour exactly: no root step, no AccountsService,
no GSettings; the session counts when its four files are installed, else the extension is turned on by hand and runs
in every GNOME session.
The payload carries the scripts the uninstaller needs (UNINSTALL_SCRIPTS) and <prefix>/bin/winpoint-uninstall runs
that copy: one command removes WinPoint, the root files with one password included (scripts/uninstall.py). Without
that copy (0.9.0's uninstall.sh removed the payload, or 0.9.0's installer replaced it) it names a release's
uninstall.sh in nl and en.

Win11's language (notes/claude/contracts.md C1, C6): an existing win11-gnome settings.json always decides; without
one, --lang; on an update of an existing install without a stored choice, nl (H D13); on a first install, the
language of Win11 Shell for Hyprland (~/.config/win11shell/settings.json), else one guess from the locale (H D4).
A first install stores that choice in a complete settings.json (the defaults of host/src/settings-defaults.json).
Messages follow --lang when it is given, else that language, also the refusals before prepare() (busy): locate()
reads it the same way, and as root or for a path it refuses (noRoot, notAbsolute) it takes the locale guess.

An install is prepare() (every check, nothing written) and apply() (every change in one Transaction): an error,
SIGTERM, SIGHUP or Ctrl+C puts everything back, the binaries' original inodes included (before any change they stop
it with exit 1 and "interrupted"). The installer holds a flock
on the payload's parent folder, so a second one is refused (busy), and it first repairs what a killed install left:
running the same tree again rolls forward. Before any change it also refuses a GNOME Shell other than the extension's
shell-version (/usr/bin/gnome-shell --version) and a binary that ldd -r cannot load (missing symbols or versions).

WinPoint's updater (D-G74; frozen from 0.9.0 on, since version N's updater runs version N+1's installer) runs the
release bundle's own copy, only after a click in Settings:
  /usr/bin/python3 -E -s -B <bundle>/scripts/install.py <bundle> --lang nl|en --no-session [--check | --status FILE --log FILE]
with exactly HOME, USER, LOGNAME, PATH=/usr/bin:/bin, LANG=C.UTF-8, XDG_DATA_HOME, XDG_CONFIG_HOME, XDG_CACHE_HOME,
XDG_RUNTIME_DIR, WIN11_BUILD_DIR=<bundle>/bin, WIN11_NO_SESSION=1 and, only for a prefix other than ~/.local,
WIN11_INSTALL_PREFIX (never WIN11_NATIVE_BUILD_DIR, a session bus address or a tty).
  --check        every check of an install; nothing is written (what a killed install left is repaired first, as
                 an install does). Exactly one JSON line on stdout, then exit 0 or 1:
                 {"ok":true,"version":"V"|null,"extensionChanged":bool,"systemFiles":["session"|"system",...]}
                 {"ok":false,"code":KEY,"params":{...},"message":TEXT}   KEY: a key of T (or of icon_bundle's or
                 file_portal's T), params its placeholders, TEXT in --lang. extensionChanged: the extension's files
                 differ from the installed ones (a sign-out is needed); systemFiles: root-owned files of
                 install-session.py ("session") or install-system.py ("system") that are installed and differ.
  exit codes     0 installed; 1 refused before any change; 3 failed and everything was put back; 4 failed and
                 putting it back was incomplete (run it again); 2 a bad argument.
  --status FILE  the exit code and a newline, written atomically at the very end, also after a SIGTERM rollback.
  --log FILE     fds 1 and 2 go there from the start (os.dup2), with the output of the programs this runs.
"""
from __future__ import annotations
from dataclasses import dataclass
import fcntl
import importlib.util
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import signal
import stat
import struct
import subprocess
import sys
import tempfile
import time
import traceback
import icon_bundle
import file_portal

MARKER = ".win11-gnome-owned"
MAGIC = "win11-gnome-rust-host-v1\n"
UUID = "win11-gnome@win11shell.local"
BINS = ("win11-explorer", "win11-panel", "win11-app", "win11-launch", "win11-index-worker", "win11-image-worker", "win11-default-apps", "win11-file-portal", "win11-installer", "win11-software-worker", "win11-shell", "win11-calendar",
        "win11-gtk-explorer", "win11-gtk-settings", "win11-gtk-taskmgr", "win11-gtk-alles", "win11-gtk-disks", "win11-gtk-defaultapps", "win11-gtk-installer")
# D-G70: the native GTK 4 shell (native/shell: taskbar, flyouts, Start, desktop) is the shell; win11-panel execs it.
# win11-calendar (Robin 2026-11-13): the native Calendar app (native/calendar), opened on a day by the taskbar calendar.
# win11-gtk-<kind> (Robin 2026-11-13, native/APPS.md): the native GTK 4 apps (native/apps). The host binaries that
# launch an app (win11-app --app <kind>, win11-explorer, win11-default-apps, win11-installer) exec them from their own
# directory; a missing one is an error that names it (exit 127), never a fallback.
# All are built in their own workspace (native/; scripts/build.sh puts them next to the others), so they are looked
# up in WIN11_NATIVE_BUILD_DIR first, then in the build directory.
NATIVE_BINS = ("win11-shell", "win11-calendar",
               "win11-gtk-explorer", "win11-gtk-settings", "win11-gtk-taskmgr", "win11-gtk-alles", "win11-gtk-disks", "win11-gtk-defaultapps", "win11-gtk-installer")
ENTRIES = ("Explorer", "Panel", "TaskManager", "Settings", "Search", "Disks", "DefaultApps", "Installer", "Calendar")
# D-G70: the WebKit Start and desktop hosts of earlier versions. An install over an own payload removes them (they are
# no longer built); they are not conflicts of a fresh install.
LEGACY_BINS = ("win11-start", "win11-desktop")
LEGACY_ENTRIES = ("Start", "Desktop")
# Icon= of these entries: a file of the bundled Win11 icon theme in the payload (icons/Win11/<path>), the icons the
# apps show under Win11Shell, so that GNOME's own UI (Alt+Tab, the overview) finds them under any icon theme; the
# stock Adwaita has none of these names but drive-harddisk. The theme name is the fallback when the file is missing.
# Settings and DefaultApps use the payload's ui/assets/icons/settings.svg, Calendar office-calendar.svg (below);
# Panel's view-app-grid-symbolic is in Adwaita.
BUNDLED_ICONS = {
    "Installer": "apps/scalable/system-software-install.svg",
    "Search": "apps/scalable/system-search.svg",
    "Disks": "devices/scalable/drive-harddisk.svg",
    "TaskManager": "apps/scalable/utilities-system-monitor.svg",
    "Explorer": "apps/scalable/system-file-manager.svg",
}
# The Calendar's own URI scheme: win11-calendar:YYYY-MM-DD opens that day. Only this scheme gets a default here.
CALENDAR_SCHEME = "x-scheme-handler/win11-calendar"
CALENDAR_DESKTOP = "io.github.win11shell.Calendar.desktop"
LANGUAGES = ("nl", "en")

# i18n: pairs (tools/i18n/check_pairs.py). Inline on purpose: these texts appear when something is broken
# (contracts C3, H D14). nl is byte-identical to 3b898e6 except the rows in tests/i18n/BASELINE-CHANGES.ext.
T = {
    "labelPrefix": {"nl": "Installatiepad", "en": "Install path"},
    "controlChars": {"nl": "{label} mag geen besturingstekens bevatten", "en": "{label} must not contain control characters"},
    "notAbsolute": {"nl": "{label} moet een absoluut pad zijn: {value}", "en": "{label} must be an absolute path: {value}"},
    "missingBinary": {"nl": "Ontbreekt: {source}. Bouw eerst met scripts/build.sh.", "en": "Missing: {source}. Build it first with scripts/build.sh."},
    "missingSource": {"nl": "Ontbrekende interfacebron: {directory}", "en": "Missing interface source: {directory}"},
    "badTable": {"nl": "Ontbrekend of ongeldig projectbestand: {file}", "en": "Missing or invalid project file: {file}"},
    "packageVersion": {"nl": "{file} heeft version-name {found}, maar Cargo.toml versie {version}. Gebruik tools/release.py bump.", "en": "{file} has version-name {found}, but Cargo.toml has version {version}. Use tools/release.py bump."},
    "packageMarker": {"nl": "{binary} is niet gebouwd voor versie {version}. Bouw eerst opnieuw met scripts/build.sh.", "en": "{binary} wasn’t built for version {version}. Build it again first with scripts/build.sh."},
    "notOwned": {"nl": "Niet overschreven: {payload} bevat geen installatiemarker van dit project.", "en": "Not overwritten: {payload} has no installation marker of this project."},
    "conflicts": {"nl": "Niet overschreven: bestaande bestanden zonder installatiemarker:", "en": "Not overwritten: existing files without an installation marker:"},
    "previousCopy": {"nl": "Eerdere installatiekopie bestaat nog: {backup}", "en": "A previous installation copy still exists: {backup}"},
    "missingParts": {"nl": "Ontbrekende systeemonderdelen: {missing}.", "en": "Missing system components: {missing}."},
    "installParts": {"nl": "Installeer ze op Ubuntu met: sudo apt install {packages}", "en": "On Ubuntu, install them with: sudo apt install {packages}"},
    "noDesktopDatabase": {"nl": "Let op: update-desktop-database ontbreekt (pakket desktop-file-utils); de app-database van GNOME wordt niet bijgewerkt.",
                          "en": "Note: update-desktop-database is missing (package desktop-file-utils); GNOME’s app database is not updated."},
    "configNotOwn": {"nl": "Niet overschreven: {path} moet een gewone map van deze gebruiker zijn.", "en": "Not overwritten: {path} must be a plain folder of this user."},
    "badLang": {"nl": "Onbekende taal ‘{lang}’: kies nl of en.", "en": "Unknown language ‘{lang}’: choose nl or en."},
    "usage": {"nl": "Gebruik: scripts/install.sh [--lang nl|en]\n  --lang nl|en  de taal van Win11 bij een eerste installatie; een keuze die al in settings.json staat, blijft gelden",
              "en": "Usage: scripts/install.sh [--lang nl|en]\n  --lang nl|en  Win11’s language on a first install; a choice already in settings.json is kept"},
    "badOption": {"nl": "Onbekende optie: {option}", "en": "Unknown option: {option}"},
    "langChosen": {"nl": "Taal: Nederlands. In English: choose English in Instellingen › Tijd en taal › Taal en regio.",
                   "en": "Language: English. In het Nederlands: kies Nederlands in Settings › Time & language › Language & region."},
    # langZone and langFormats are printed only when the locale guess gave nl without --lang, so the message
    # language is then nl and their en halves are never shown (ge1c-review m6); they stay for the pairs checker.
    "langZone": {"nl": "Taal: Nederlands, vanwege de tijdzone {zone}. In English: choose English in Instellingen › Tijd en taal › Taal en regio.",
                 "en": "Language: Dutch, because of the time zone {zone}. In English: choose English in Instellingen › Tijd en taal › Taal en regio."},
    "langFormats": {"nl": "Taal: Nederlands, vanwege de Nederlandse notatie in {var}. In English: choose English in Instellingen › Tijd en taal › Taal en regio.",
                    "en": "Language: Dutch, because of the Dutch formats in {var}. In English: choose English in Instellingen › Tijd en taal › Taal en regio."},
    "langMigrated": {"nl": "Taal: {name}, overgenomen van Win11 Shell voor Hyprland ({file}).", "en": "Language: {name}, taken over from Win11 Shell for Hyprland ({file})."},
    "langKept": {"nl": "Let op: --lang {lang} geldt alleen zonder opgeslagen keuze. Win11 blijft {name} ({file}); wijzig de taal in {path}.",
                 "en": "Note: --lang {lang} only applies without a stored choice. Win11 stays in {name} ({file}); change the language in {path}."},
    "name_nl": {"nl": "Nederlands", "en": "Dutch"},
    "name_en": {"nl": "Engels", "en": "English"},
    "settingsPath": {"nl": "Instellingen › Tijd en taal › Taal en regio", "en": "Settings › Time & language › Language & region"},
    "staleRemoved": {"nl": "Verwijderd uit de extensiemap (geen bestand van deze versie): {path}",
                     "en": "Removed from the extension folder (not a file of this version): {path}"},
    "webkitRemoved": {"nl": "Verwijderd (gegevens van de WebKit-apps van een eerdere versie): {path}",
                      "en": "Removed (data of an earlier version’s WebKit apps): {path}"},
    "done1": {"nl": "Geïnstalleerd in {prefix}/bin", "en": "Installed in {prefix}/bin"},
    "done2": {"nl": "Verkenner is los te openen via het applicatieoverzicht.", "en": "File Explorer can be opened from the app overview."},
    "done3": {"nl": "De taakbalk is geïnstalleerd, maar nog niet aangezet.", "en": "The taskbar is installed, but not turned on yet."},
    "done4": {"nl": "Meld je aan bij Ubuntu/GNOME; na eerste installatie zo nodig af- en aanmelden.", "en": "Sign in to Ubuntu/GNOME; after a first installation, sign out and back in if needed."},
    "done5": {"nl": "Taakbalk aan: gnome-extensions enable {uuid}", "en": "Turn the taskbar on: gnome-extensions enable {uuid}"},
    "session1": {"nl": "WinPoint is een eigen keuze bij het inloggen. Je gewone Ubuntu-sessie blijft zoals hij was.", "en": "WinPoint is its own choice at the login screen. Your normal Ubuntu session stays as it was."},
    "session2": {"nl": "Meld je af en kies WinPoint bij het inloggen: klik op je naam en daarna op het tandwiel rechtsonder.", "en": "Sign out and choose WinPoint at the login screen: click your name, then the gear at the bottom right."},
    # 0.9.1 (Robin, 16-11): the terminal install's one password, its refusals and the choice at the next sign-in
    "sessionNext": {"nl": "Meld je af: bij de volgende keer aanmelden start WinPoint. Ubuntu kies je bij het inloggen met het tandwiel rechtsonder, na je naam.",
                    "en": "Sign out: WinPoint starts the next time you sign in. To use Ubuntu, choose it with the gear at the bottom right of the login screen, after your name."},
    "sessionTerminal": {"nl": "Start de installatie in een terminal: scripts/install.sh vraagt daar één keer om je wachtwoord. Er is niets veranderd.",
                        "en": "Run the installation in a terminal: scripts/install.sh asks for your password there, once. Nothing was changed."},
    # after install-session.py's own lines (what it did install is listed there): nothing of the user's side changed
    "sessionNeeded": {"nl": "WinPoint is geen keuze bij het inloggen geworden, dus de installatie is gestopt; er is niets in je thuismap veranderd. Start haar opnieuw en vul het wachtwoord in.",
                      "en": "WinPoint didn’t become a choice at the login screen, so the installation stopped; nothing in your home folder was changed. Run it again and enter the password."},
    "rootPartly": {"nl": "Let op: niet alle systeembestanden zijn bijgewerkt; de installatie gaat door. Start haar later opnieuw om ze bij te werken.",
                   "en": "Note: not all system files were updated; the installation goes on. Run it again later to update them."},
    "noRoot": {"nl": "Start de installatie niet als root of met sudo: WinPoint hoort bij je eigen account, en pkexec vraagt zelf om het wachtwoord.",
               "en": "Don’t run the installation as root or with sudo: WinPoint belongs to your own account, and pkexec asks for the password itself."},
    # after the commit, when the UUID was in enabled-extensions (an in-GNOME install of 0.9.0): it is out now
    "sessionMoved": {"nl": "De taakbalk van WinPoint draait voortaan alleen in de WinPoint-sessie; in je gewone Ubuntu-sessie staat hij nu uit.",
                     "en": "From now on WinPoint’s taskbar runs only in the WinPoint session; in your normal Ubuntu session it’s off now."},
    # rolledBack after root_step installed files: apply()'s Transaction cannot take them back
    "rolledBackRoot": {"nl": "De installatie is mislukt; je thuismap is teruggezet zoals hij was, maar de systeembestanden van WinPoint blijven staan en WinPoint is een keuze bij het inloggen. Start de installatie opnieuw om haar af te ronden, of verwijder ze met: {command}",
                       "en": "The installation failed; your home folder was put back as it was, but WinPoint’s system files stay and WinPoint is a choice at the login screen. Run the installation again to finish it, or remove them with: {command}"},
    # <prefix>/bin/winpoint-uninstall without the payload's uninstall.py (uninstaller_text prints both): the payload is
    # gone (0.9.0's uninstall.sh) or is 0.9.0's (installed by hand over 0.9.1), and a release's uninstall.sh takes both
    "uninstallerGone": {"nl": "{script} ontbreekt, dus dit verwijderprogramma werkt niet meer. Voer ./scripts/uninstall.sh uit in de map van een uitgepakte WinPoint-uitgave (0.9.1 of nieuwer): dat verwijdert wat er nog van WinPoint is.",
                        "en": "{script} is missing, so this uninstaller no longer works. Run ./scripts/uninstall.sh in the folder of an unpacked WinPoint release (0.9.1 or newer): it removes what is left of WinPoint."},
    "gnomeLookName": {"nl": "GNOME-uiterlijk terugzetten (WinPoint)", "en": "Restore the GNOME look (WinPoint)"},
    "done6": {"nl": "Je huidige Win11Shell, standaardbestandsbeheerder en desktopinstellingen zijn niet gewijzigd.", "en": "Your current Win11Shell, default file manager and desktop settings have not been changed."},
    # GW5: the one hint line about scripts/install-system.py (install: the files are missing; uninstall: still there).
    # 0.9.1: the terminal install's root step installs them, so only --no-session or a tree without host/data prints it
    "systemHint": {"nl": "Optioneel, één keer met je wachtwoord: {command} installeert de systeembestanden voor ‘Nieuwe Ubuntu-versies’ en ‘Venster weer openen’.",
                   "en": "Optional, once with your password: {command} installs the system files for ‘New Ubuntu versions’ and ‘Show the upgrade window’."},
    "systemHintRemove": {"nl": "De systeembestanden van Win11 staan er nog; verwijder ze met: {command}",
                         "en": "Win11’s system files are still installed; remove them with: {command}"},
    # GW7: Name= of the post-upgrade check's autostart entry (design §6.6), in Win11's language (D-G4)
    "postUpgradeName": {"nl": "Win11-controle na een Ubuntu-upgrade", "en": "Win11 check after an Ubuntu upgrade"},
    # D-G74: the installer's lock, its two new checks, the rollback and --check
    "busy": {"nl": "Er loopt al een andere installatie in {path}; probeer het opnieuw als die klaar is.",
             "en": "Another installation is running in {path}; try again when it’s done."},
    "gnomeMismatch": {"nl": "Deze versie van WinPoint is gemaakt voor GNOME {needs}; deze pc heeft GNOME {running}.",
                      "en": "This version of WinPoint is made for GNOME {needs}; this PC has GNOME {running}."},
    "abi": {"nl": "{binary} werkt niet met de systeembibliotheken van deze pc: {detail}",
            "en": "{binary} doesn’t work with this PC’s system libraries: {detail}"},
    # abi's {detail} for a bundle of another architecture (check_abi; the code stays abi for 0.9.0's job.rs)
    "abiMachine": {"nl": "gemaakt voor {needs}, deze pc is {running}", "en": "made for {needs}, this PC is {running}"},
    "interrupted": {"nl": "De installatie is onderbroken.", "en": "The installation was interrupted."},
    "rolledBack": {"nl": "De installatie is mislukt; alles is teruggezet zoals het was.",
                   "en": "The installation failed; everything was put back as it was."},
    "rollbackIncomplete": {"nl": "De installatie is mislukt en kon niet helemaal worden teruggezet. Start de installatie opnieuw om haar af te ronden.",
                           "en": "The installation failed and couldn’t be fully undone. Run the installation again to finish it."},
    "notRestored": {"nl": "Niet teruggezet: {path} ({error})", "en": "Not restored: {path} ({error})"},
    "checkFailed": {"nl": "De controle is mislukt: {detail}", "en": "The check failed: {detail}"},
    "testFail": {"nl": "Testfout na stap {step} (WIN11_INSTALL_TEST_FAIL)", "en": "Test failure after step {step} (WIN11_INSTALL_TEST_FAIL)"},
}
LANGUAGE = "nl"          # the language of the messages; set_language() also sets it in icon_bundle and file_portal


def t(key: str, language: str | None = None, **params) -> str:
    pair = T[key]
    return (pair.get(language or LANGUAGE) or pair["nl"]).format_map(params)


def set_language(language: str) -> None:
    global LANGUAGE
    LANGUAGE = icon_bundle.LANGUAGE = file_portal.LANGUAGE = language if language in LANGUAGES else "nl"


class Refusal(SystemExit):
    """An install refused before any change (exit 1; the message on stderr, as SystemExit always printed it). The key
    of T and its params go into --check's answer."""

    def __init__(self, key: str, params: dict, message: str):
        super().__init__(message)
        self.key = key
        self.params = {name: str(value) if isinstance(value, Path) else value for name, value in params.items()}


def refuse(key: str, **params):
    raise Refusal(key, params, t(key, **params))


def absolute_path(value: str, label: str) -> Path:
    if any(ord(c) < 32 for c in value):
        refuse("controlChars", label=label)
    path = Path(value).expanduser()
    if not path.is_absolute():
        refuse("notAbsolute", label=label, value=value)
    return path


def desktop_exec(path: Path) -> str:
    # Desktop-entry unescaping precedes Exec argument unescaping (not shell parsing).
    value = str(path)
    for character in ("\\", '"', "`", "$"):
        value = value.replace(character, "\\" + character)
    value = value.replace("%", "%%").replace("\\", "\\\\")
    return '"' + value + '"'


# The system files of scripts/install-system.py (GW5, notes/claude/windows-update-gnome.md §7). Since 0.9.1 a terminal
# install puts them in place with the login session's four in one password (root_step, install-session.py --system)
# and uninstall.py removes all eight in one; install-system.py still does it on its own. Without that step (--no-session)
# this installer names the command in one line, only on a terminal: the line is advice for the person at the
# keyboard, so captured output (the installer tests' byte-for-byte comparison with 3b898e6) stays as it was.
SYSTEM_FILES = ("usr/libexec/win11-gnome/release-upgrades-helper", "usr/libexec/win11-gnome/release-upgrade-attach",
                "usr/share/polkit-1/actions/io.github.win11shell.gnome.release-upgrades.policy",
                "usr/share/polkit-1/actions/io.github.win11shell.gnome.release-upgrade-attach.policy")


# GW7 (design §6.6): the post-upgrade check runs from XDG autostart, outside the extension, so it also works when
# GNOME turned the extension off after an Ubuntu upgrade, and after upgrades done through Ubuntu's Software Updater.
AUTOSTART = "io.github.win11shell.PostUpgradeCheck.desktop"


def autostart_entry(prefix: Path, language: str) -> str:
    """Plain Name= in Win11's language (D-G4), NoDisplay, OnlyShowIn=GNOME; and no X-GNOME-Autostart-Delay (systemd's
    generator ignores it; the check waits up to 30 s for GNOME Shell itself)."""
    return ("[Desktop Entry]\nType=Application\n"
            f"Name={t('postUpgradeName', language)}\n"
            f"Exec=/usr/bin/env -- {desktop_exec(prefix/'bin'/'win11-launch')} --post-upgrade-check\n"
            "OnlyShowIn=GNOME;\nNoDisplay=true\n")


GNOME_LOOK = "io.github.win11shell.GnomeLook.desktop"


def gnome_look_entry(prefix: Path, language: str) -> str:
    """Every session but WinPoint's: GNOME's own look again (host/src/gnome_look.rs, win11-app --gnome-look)."""
    return ("[Desktop Entry]\nType=Application\n"
            f"Name={t('gnomeLookName', language)}\n"
            f"Exec=/usr/bin/env -- {desktop_exec(prefix/'bin'/'win11-app')} --gnome-look\n"
            "NotShowIn=WinPoint;\nNoDisplay=true\n")


# Uninstall with one command (0.9.1): the payload's scripts/ holds what uninstall.py needs (stage_payload), and
# <prefix>/bin/winpoint-uninstall runs that copy. The line after its #! marks it as ours (own_uninstaller): an update
# rewrites only that file, a first install refuses another one at its path (conflicts), uninstall.py removes only it.
# apply() writes it as the binaries: a temporary UNINSTALLER_NEW first, then one rename (repair() removes a leftover).
UNINSTALLER = "winpoint-uninstall"
UNINSTALLER_NEW = f".{UNINSTALLER}.new"
UNINSTALL_SCRIPTS = ("uninstall.py", "install.py", "install-session.py", "install-system.py", "file_portal.py",
                     "icon_bundle.py")
UNINSTALLER_MARK = "# WinPoint's uninstaller (scripts/install.py UNINSTALLER); your settings stay.\n"


def uninstaller_text(payload: Path) -> str:
    """A small sh wrapper; the paths and texts are quoted for sh (a prefix may hold spaces, $ or quotes). Without the
    payload's uninstall.py (0.9.0's uninstall.sh removes the payload and leaves this file; 0.9.0's installer writes a
    payload without scripts/) it prints uninstallerGone in both languages, as the language of that install is
    unknown, and exits 1 (uninstall.py of a release takes the rest: this file, the root files)."""
    script = shlex.quote(str(payload/"scripts/uninstall.py"))
    gone = " ".join(shlex.quote(t("uninstallerGone", language, script=payload/"scripts/uninstall.py"))
                    for language in LANGUAGES)
    return ("#!/bin/sh\n" + UNINSTALLER_MARK + f"[ -f {script} ] || {{ printf '%s\\n' {gone} >&2; exit 1; }}\n"
            f"exec /usr/bin/python3 -E -s -B {script} {shlex.quote(str(payload))} \"$@\"\n")


def own_uninstaller(path: Path) -> bool:
    """`path` is a plain file of this user (no link) with UNINSTALLER_MARK as its second line, or empty: what a power
    cut right after apply()'s rename can leave of a new one (its data not on disk yet), so a rerun rewrites it and
    uninstall.py removes it."""
    if not own_entry(path, stat.S_ISREG):
        return False
    try:
        with open(path, encoding="utf-8") as stream:
            first = stream.readline()
            return first == "" or (first == "#!/bin/sh\n" and stream.readline() == UNINSTALLER_MARK)
    except (OSError, UnicodeError):
        return False


_SESSION = []


def session_script():
    """scripts/install-session.py next to this file, loaded once (uninstall.py loads it before it removes the payload
    it may run from)."""
    if not _SESSION:
        spec = importlib.util.spec_from_file_location("install_session", Path(__file__).with_name("install-session.py"))
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        _SESSION.append(module)
    return _SESSION[0]


def system_root() -> Path:
    """Where the root files are read: / (WIN11_SYSTEM_ROOT in tests only, install-session.py system_root)."""
    return session_script().system_root()


def terminal_install(check: bool = False) -> bool:
    """The terminal path (module docstring): a tty on stdin, no --no-session, WIN11_NO_SESSION not 1, not --check.
    --no-session and WIN11_NO_SESSION=1 are 0.9.0's path (False); no tty refuses (sessionTerminal), and so does a
    system without gnome-session or pkexec (missingParts, with their packages)."""
    if check or NO_SESSION or os.environ.get("WIN11_NO_SESSION") == "1":
        return False
    if not sys.stdin.isatty():
        refuse("sessionTerminal")
    parts = [(name, package) for name, package, present in (
        ("gnome-session", "gnome-session-bin", Path("/usr/bin/gnome-session").is_file()),
        ("pkexec", "pkexec", shutil.which("pkexec") is not None)) if not present]
    if parts:
        missing, packages = ", ".join(name for name, _ in parts), " ".join(package for _, package in parts)
        raise Refusal("missingParts", {"missing": missing, "packages": packages},
                      t("missingParts", missing=missing) + "\n" + t("installParts", packages=packages))
    return True


def root_step(plan: Plan) -> None:
    """The terminal path's one password (Robin, 16-11), after every check of prepare() and before apply()'s first
    change: when one of the eight root files is missing or differs (install-session.py stale() and system_stale(); the
    system files only when the tree has host/data), install-session.py --system [--lang] installs all of them in one
    pkexec. Without the session's files afterwards: sessionNeeded, and nothing was changed (a cancelled password
    writes nothing). When some of the rest still differs: one warning line (rootPartly), and the install goes on. Once
    it ran, plan.root_changed: apply()'s Transaction cannot undo it, so its failure ends with rolledBackRoot. The
    systemHint line is for a tree without host/data only. install-session.py says what it does before the password
    (the files, /bin/sh in the dialog, an administrator's password), so this prints nothing of its own first."""
    if not plan.terminal:
        return
    session, system = session_script(), system_root()
    sources = session.system_sources()
    plan.hint = not sources
    if not session.stale(system) and not (sources and session.system_stale(system)):
        return
    sys.stdout.flush()
    subprocess.run([sys.executable, "-E", "-s", "-B", str(Path(__file__).with_name("install-session.py")),
                    "--lang", LANGUAGE, *(["--system"] if sources else [])])
    if not session.installed(system):
        refuse("sessionNeeded")
    # from here on apply()'s rollback leaves root files behind: its last line says so (rolledBackRoot)
    plan.root_changed = True
    if session.stale(system) or (sources and session.system_stale(system)):
        print(t("rootPartly"), file=sys.stderr)


# AccountsService (D-Bus, system bus) through busctl: short, and a failure is no error (install: the gear
# instruction follows; uninstall: the login screen falls back to its default once the session's files are gone)
ACCOUNTS = ("org.freedesktop.Accounts", "/org/freedesktop/Accounts", "org.freedesktop.Accounts")
ACCOUNTS_USER = "org.freedesktop.Accounts.User"


# Never a second password prompt (Robin, 16-11: one password): a polkit policy that wants one for
# change-own-user-data (not Ubuntu's: allow_active=yes) makes the call fail at once instead, and the fallback follows.
# busctl's own timeout ends a slow call cleanly, before the subprocess timeout would kill it.
BUSCTL_OPTIONS = ("--system", "--allow-interactive-authorization=no", "--timeout=5")


def busctl(*args: str) -> str | None:
    """busctl BUSCTL_OPTIONS ARGS (stdout), or None when it is missing, fails or takes longer than 5 s (10 s at the
    most)."""
    tool = shutil.which("busctl")
    if not tool:
        return None
    try:
        proc = subprocess.run([tool, *BUSCTL_OPTIONS, *args], stdin=subprocess.DEVNULL, capture_output=True, text=True,
                              errors="replace", timeout=10)
    except (OSError, subprocess.SubprocessError):
        return None
    return proc.stdout if proc.returncode == 0 else None


def accounts_user() -> str | None:
    """This user's AccountsService object (FindUserById)."""
    out = busctl("call", *ACCOUNTS, "FindUserById", "x", str(os.getuid()))
    match = re.fullmatch(r'o "(/org/freedesktop/Accounts/[A-Za-z0-9_]+)"', (out or "").strip())
    return match.group(1) if match else None


def accounts_session(user: str) -> str | None:
    """The Session property of `user` (GDM preselects it at the login screen)."""
    out = busctl("get-property", ACCOUNTS[0], user, ACCOUNTS_USER, "Session")
    match = re.fullmatch(r's "([^"\\]*)"', (out or "").strip())
    return match.group(1) if match else None


def preselect() -> bool:
    """Decision 1 (Robin, 16-11): WinPoint (Wayland) is this user's session at the next sign-in, no gear step.
    polkit's change-own-user-data allows it without a password in the active session."""
    user = accounts_user()
    return bool(user) and all(busctl("call", ACCOUNTS[0], user, ACCOUNTS_USER, method, "s", value) is not None
                              for method, value in (("SetSession", "winpoint"), ("SetSessionType", "wayland")))


def shell_settings():
    """org.gnome.shell through GSettings (as host/src/gnome_look.rs: the default schema source, the user's backend),
    or None without GSettings or that schema."""
    try:
        from gi.repository import Gio
        source = Gio.SettingsSchemaSource.get_default()
        schema = source.lookup("org.gnome.shell", True) if source else None
        return Gio.Settings.new_full(schema, None, None) if schema else None
    except (ImportError, ValueError, RuntimeError, TypeError):
        return None


def drop_uuid(keys: tuple[str, ...], settings=None) -> None:
    """WinPoint's UUID out of org.gnome.shell's lists `keys`; every other value and key stays as it is."""
    settings = settings or shell_settings()
    if settings is None:
        return
    changed = False
    for key in keys:
        if settings.props.settings_schema.has_key(key) and UUID in settings.get_strv(key):
            settings.set_strv(key, [uuid for uuid in settings.get_strv(key) if uuid != UUID])
            changed = True
    if changed:
        from gi.repository import Gio
        Gio.Settings.sync()


def allow_mode_extension() -> bool:
    """The terminal path after the commit (decision 2, Robin 16-11: the normal sessions stay as they are): WinPoint's
    UUID out of enabled-extensions, where an in-GNOME install of 0.9.0 has it (the extension would load in the next
    Ubuntu session before GnomeLook could take it out; in the WinPoint session its mode turns it on all the same),
    then out of disabled-extensions (an earlier in-GNOME install that was turned off: GNOME's mode skips the
    extensions listed there, so the WinPoint session would lack its taskbar). In that order, so the extension is
    never on in the normal session that runs now; that session turns it off at once. Only those values go. True when
    it was in enabled-extensions (sessionMoved)."""
    try:
        settings = shell_settings()
        moved = settings is not None and UUID in settings.get_strv("enabled-extensions")
        drop_uuid(("enabled-extensions", "disabled-extensions"), settings)
        return moved
    except Exception:                   # after the commit: a GSettings failure is no failed install
        return False


def system_hint(remove: bool, root: Path | None = None) -> str | None:
    """install (remove=False): the hint when any system file is missing; remove: when any is still there (0.9.1's
    uninstall.py removes them with the session's and names install-session.py instead; tests/update keeps both)."""
    root = system_root() if root is None else root
    present = [(root/name).is_file() for name in SYSTEM_FILES]
    if (remove and not any(present)) or (not remove and all(present)):
        return None
    command = "python3 " + shlex.quote(str(Path(__file__).resolve().parent/"install-system.py"))
    if remove:
        return t("systemHintRemove", command=command + " --remove")
    return t("systemHint", command=command)


# ---------------------------------------------------------------------------------------------------------------
# Language (C1, C6; H's host/i18n.py detect_info, notes/english-option-design.md §2.6.2, D2-D4, D13)
# ---------------------------------------------------------------------------------------------------------------

DUTCH_ZONES = frozenset({"Europe/Amsterdam", "America/Paramaribo", "America/Curacao", "America/Aruba",
                         "America/Kralendijk", "America/Lower_Princes"})        # not Europe/Brussels (H D4)
SIGNAL_CATEGORIES = ("LC_TIME", "LC_NUMERIC", "LC_MONETARY", "LC_PAPER", "LC_MEASUREMENT")
_NL_LOCALE = re.compile(r"^(?:nl(?:[_.@]|$)|fy_|li_|nds_NL)")
_NL_FORMAT = re.compile(r"^(?:nl|fy|li)_")
_EN_LOCALE = re.compile(r"^en(?:[_.@]|$)")
_ZONEINFO = re.compile(r"(?:^|/)zoneinfo/(?:posix/|right/)?(.+)$")


def language_of_file(path: Path) -> str:
    """C1: "en" only for exactly "en"; a missing or unreadable file, other JSON or another value means nl."""
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError, UnicodeError):
        return "nl"
    return "en" if isinstance(data, dict) and data.get("language") == "en" else "nl"


def _is_c(locale: str) -> bool:
    return re.split(r"[.@]", locale, maxsplit=1)[0] in ("", "C", "POSIX")


def messages_locale(env) -> str:
    """The first non-empty of LC_ALL, LC_MESSAGES and LANG; LANGUAGE's first entry wins unless that is C/POSIX."""
    effective = next((env[k] for k in ("LC_ALL", "LC_MESSAGES", "LANG") if env.get(k)), "")
    if not _is_c(effective):
        first = next((x for x in (env.get("LANGUAGE") or "").split(":") if x), "")
        if first:
            return first
    return effective


def _zone_of_path(path: str) -> str | None:
    match = _ZONEINFO.search(path)          # read by its text, never resolved (Amsterdam may link to Brussels)
    return match.group(1) if match else None


def time_zone(env, timezone_file: str = "/etc/timezone", localtime: str = "/etc/localtime") -> str | None:
    zone = env.get("TZ") or ""
    if zone:
        zone = zone[1:] if zone.startswith(":") else zone
        return _zone_of_path(zone) if zone.startswith("/") else (zone or None)
    try:
        with open(timezone_file, encoding="utf-8") as stream:
            value = stream.readline().strip()
        if value:
            return value
    except (OSError, UnicodeError):
        pass
    try:
        return _zone_of_path(os.readlink(localtime))
    except OSError:
        return None


def dutch_signal(env, timezone_file: str = "/etc/timezone", localtime: str = "/etc/localtime") -> str | None:
    """H D4: an nl_/fy_/li_ formats locale ("lc:LC_TIME") or a Dutch-speaking time zone ("tz:Europe/Amsterdam")."""
    for key in SIGNAL_CATEGORIES:
        if _NL_FORMAT.match(env.get("LC_ALL") or env.get(key) or env.get("LANG") or ""):
            return f"lc:{key}"
    zone = time_zone(env, timezone_file, localtime)
    return f"tz:{zone}" if zone in DUTCH_ZONES else None


def guess_language(env, timezone_file: str = "/etc/timezone",
                   localtime: str = "/etc/localtime") -> tuple[str, str, str | None]:
    """The one locale guess of a first install: (language, "locale" | "default", D4 signal)."""
    locale = messages_locale(env)
    if _is_c(locale):
        return "nl", "default", None
    if _NL_LOCALE.match(locale):
        return "nl", "locale", None
    if _EN_LOCALE.match(locale):
        signal = dutch_signal(env, timezone_file, localtime)
        return ("nl", "locale", signal) if signal else ("en", "locale", None)
    return "en", "locale", None                                                     # H D3


def resolve_language(explicit: str | None, config: Path, data: Path, payload: Path, env=os.environ,
                     timezone_file: str = "/etc/timezone", localtime: str = "/etc/localtime") -> dict:
    """{"lang", "source", "detail", "store"}; source: settings | arg | existing | hyprland | locale | default."""
    settings = config/"win11-gnome/settings.json"
    if os.path.lexists(settings):
        return {"lang": language_of_file(settings), "source": "settings", "detail": str(settings), "store": False}
    if explicit:
        return {"lang": explicit, "source": "arg", "detail": None, "store": True}
    if os.path.lexists(payload):                       # an update without a stored choice: what it showed (H D13)
        return {"lang": "nl", "source": "existing", "detail": None, "store": False}
    h_settings = config/"win11shell/settings.json"
    if os.path.lexists(h_settings):                    # plan-critique F20: H's choice moves over once
        return {"lang": language_of_file(h_settings), "source": "hyprland", "detail": str(h_settings), "store": True}
    if os.path.lexists(data/"win11shell"):             # H installed without a stored choice: H showed nl (H D13)
        return {"lang": "nl", "source": "hyprland", "detail": str(data/"win11shell"), "store": True}
    lang, source, signal = guess_language(env, timezone_file, localtime)
    return {"lang": lang, "source": source, "detail": signal, "store": True}


NO_SESSION = False   # --no-session (or WIN11_NO_SESSION=1): 0.9.0's path, never the terminal one (terminal_install)


@dataclass
class Options:
    explicit: str | None = None      # --lang
    check: bool = False              # --check
    status: str | None = None        # --status FILE
    log: str | None = None           # --log FILE


FILE_OPTIONS = ("--status", "--log")


def option_value(argv: list[str], name: str) -> str | None:
    """The value of --status/--log (FILE or =FILE), read before anything else: the status file is written and the
    output goes to the log whatever else the arguments hold."""
    for index, argument in enumerate(argv):
        if argument == name and index + 1 < len(argv):
            return argv[index + 1] or None
        if argument.startswith(name + "="):
            return argument.split("=", 1)[1] or None
    return None


def parse_arguments(argv: list[str]) -> Options:
    """--lang nl|en (also --lang=X, any case), the updater's --no-session (no advice in the usage since 0.9.1, still
    accepted), --check, --status FILE and --log FILE (module docstring); -h/--help prints the usage. Exit 2 on a bad
    value or option."""
    global NO_SESSION
    options = Options()
    explicit = None
    rest = []
    index = 0
    while index < len(argv):
        argument = argv[index]
        if argument == "--no-session":
            NO_SESSION = True
        elif argument == "--check":
            options.check = True
        elif argument in FILE_OPTIONS or argument.startswith(tuple(f"{name}=" for name in FILE_OPTIONS)):
            name, _, value = argument.partition("=")
            if argument in FILE_OPTIONS:
                index += 1
                value = argv[index] if index < len(argv) else ""
            if not value:
                rest.append(name)
            setattr(options, name[2:], value or None)
        elif argument == "--lang" or argument.startswith("--lang="):
            if argument == "--lang":
                index += 1
                value = argv[index] if index < len(argv) else ""
            else:
                value = argument.split("=", 1)[1]
            explicit = value.strip().lower()
            if explicit not in LANGUAGES:
                print(t("badLang", "nl", lang=value) + "\n" + t("badLang", "en", lang=value), file=sys.stderr)
                raise SystemExit(2)
        else:
            rest.append(argument)
        index += 1
    if explicit:
        set_language(explicit)
    for argument in rest:
        if argument in ("-h", "--help"):
            print(t("usage"))
            raise SystemExit(0)
        print(t("badOption", option=argument) + "\n" + t("usage"), file=sys.stderr)
        raise SystemExit(2)
    options.explicit = explicit
    return options


# ---------------------------------------------------------------------------------------------------------------
# Build directory and runtime preflight
# ---------------------------------------------------------------------------------------------------------------

def executable(path: Path) -> bool:
    return path.is_file() and os.access(path, os.X_OK)


def binary_source(build: Path, name: str) -> Path:
    """Where `name` is installed from: the build directory; for the native binaries WIN11_NATIVE_BUILD_DIR first."""
    native = os.environ.get("WIN11_NATIVE_BUILD_DIR")
    if name in NATIVE_BINS and native and executable(Path(native)/name):
        return Path(native)/name
    return build/name


def missing_binaries(build: Path) -> list[Path]:
    return [binary_source(build, name) for name in BINS if not executable(binary_source(build, name))]


def choose_build(root: Path) -> Path:
    """WIN11_BUILD_DIR, else the first of target/release and bin/ that itself holds every binary of BINS (a
    checkout's bin/ is stale; a package has only bin/): WIN11_NATIVE_BUILD_DIR never makes an incomplete directory
    the choice, so the old binaries of a checkout's bin/ are never installed. All 19 binaries are required; every
    missing one is named."""
    if os.environ.get("WIN11_BUILD_DIR"):
        build = Path(os.environ["WIN11_BUILD_DIR"])
        missing = missing_binaries(build)
    else:
        candidates = [root/"target/release", root/"bin"]
        build = next((c for c in candidates if all(executable(c/name) for name in BINS)),
                     next((c for c in candidates if c.is_dir()), candidates[0]))
        missing = [build/name for name in BINS if not executable(build/name)]
    if missing:
        raise Refusal("missingBinary", {"source": missing[0], "sources": [str(path) for path in missing]},
                      "\n".join(t("missingBinary", source=path) for path in missing))
    return build


PACKAGES = {   # the Ubuntu 26.04 package of each library the binaries need (dpkg -S, 2026-11-05)
    "libgdk_pixbuf-2.0.so.0": "libgdk-pixbuf-2.0-0", "libcairo.so.2": "libcairo2", "libz.so.1": "zlib1g",
    "libglib-2.0.so.0": "libglib2.0-0t64", "libgio-2.0.so.0": "libglib2.0-0t64", "libgobject-2.0.so.0": "libglib2.0-0t64",
    "libarchive.so.13": "libarchive13t64", "libgcc_s.so.1": "libgcc-s1",
    "libgtk-4.so.1": "libgtk-4-1", "libgraphene-1.0.so.0": "libgraphene-1.0-0",   # win11-shell, win11-calendar, win11-gtk-*
    "libpango-1.0.so.0": "libpango-1.0-0",   # win11-gtk-{explorer,settings,taskmgr,alles} (dpkg -S, 2026-11-13)
}
# The rest of DT_NEEDED is libc6 (libc.so.6, libm.so.6, ld-linux-*.so.2): every Ubuntu has it.
DLOPENED = ("libarchive.so.13",)          # host/src/files/archive_ffi.rs opens it at run time


def elf_needed(path: Path) -> list[str] | None:
    """DT_NEEDED of a 64-bit little-endian ELF file, read without running it (None: another format)."""
    try:
        with open(path, "rb") as stream:
            head = stream.read(64)
            if head[:4] != b"\x7fELF" or head[4] != 2 or head[5] != 1:
                return None
            phoff, = struct.unpack_from("<Q", head, 0x20)
            phentsize, phnum = struct.unpack_from("<HH", head, 0x36)
            stream.seek(phoff)
            table = stream.read(phentsize * phnum)
            loads, dynamic = [], None
            for i in range(phnum):
                kind, _flags, offset, vaddr, _paddr, filesz, _memsz, _align = struct.unpack_from("<IIQQQQQQ", table, i * phentsize)
                if kind == 1:
                    loads.append((vaddr, offset, filesz))
                elif kind == 2:
                    dynamic = (offset, filesz)
            if dynamic is None:
                return []
            stream.seek(dynamic[0])
            raw = stream.read(dynamic[1])
            entries = [struct.unpack_from("<qQ", raw, i) for i in range(0, len(raw) - 15, 16)]
            strtab = next((v for tag, v in entries if tag == 5), None)
            strsz = next((v for tag, v in entries if tag == 10), 0)
            needed = [v for tag, v in entries if tag == 1]
            base = next((o + strtab - va for va, o, size in loads if va <= strtab < va + size), None) if strtab else None
            if base is None:
                return None
            stream.seek(base)
            strings = stream.read(strsz)
            return [strings[i:strings.index(b"\0", i)].decode("utf-8", "replace") for i in needed]
    except (OSError, struct.error, ValueError):
        return None


def known_libraries() -> set[str]:
    names = set()
    ldconfig = next((p for p in ("/sbin/ldconfig", "/usr/sbin/ldconfig") if os.access(p, os.X_OK)), None)
    if ldconfig:
        try:
            out = subprocess.run([ldconfig, "-p"], capture_output=True, text=True, timeout=30).stdout
            names |= {m.group(1) for m in re.finditer(r"^\s+(\S+) \(([^)]*)\)", out, re.M) if "64" in m.group(2)}
        except (OSError, subprocess.SubprocessError):
            pass
    dirs = [d for d in os.environ.get("LD_LIBRARY_PATH", "").split(":") if d]
    dirs += ["/lib/x86_64-linux-gnu", "/usr/lib/x86_64-linux-gnu", "/lib/aarch64-linux-gnu", "/usr/lib/aarch64-linux-gnu",
             "/lib64", "/usr/lib64", "/lib", "/usr/lib"]
    for directory in dirs:
        try:
            names |= set(os.listdir(directory))
        except OSError:
            pass
    return names


def preflight_runtime(build: Path) -> None:
    """The system parts the installed apps and this installer need (gnome-map §6 NucBox; plan-critique F21)."""
    known = known_libraries()
    needed = []
    for name in BINS:
        for library in elf_needed(binary_source(build, name)) or []:
            if library not in needed:
                needed.append(library)
    needed += [library for library in DLOPENED if library not in needed]
    missing = [library for library in needed if library not in known]
    packages = [PACKAGES[library] for library in missing if library in PACKAGES]
    try:
        import gi
        gi.require_version("Gio", "2.0")
        from gi.repository import Gio  # noqa: F401
    except (ImportError, ValueError):
        missing.append("python3-gi")
        packages.append("python3-gi")
    if not shutil.which("glib-compile-schemas"):
        missing.append("glib-compile-schemas")
        packages.append("libglib2.0-bin")
    if missing:
        packages = " ".join(dict.fromkeys(packages))
        lines = [t("missingParts", missing=", ".join(missing))]
        if packages:
            lines.append(t("installParts", packages=packages))
        raise Refusal("missingParts", {"missing": ", ".join(missing), "packages": packages}, "\n".join(lines))
    if not shutil.which("update-desktop-database"):
        print(t("noDesktopDatabase"), file=sys.stderr)


def load_entries(root: Path) -> dict:
    """host/src/desktop-entries.json (C6): Name= and Comment= of the own .desktop files in nl and en."""
    path = root/"host/src/desktop-entries.json"
    try:
        table = json.loads(path.read_text(encoding="utf-8"))
        pairs = [table["comment"]] + [table["entries"][name] for name in ENTRIES]
        if table.get("version") != 1 or not all(isinstance(p.get(k), str) and p[k] and "\n" not in p[k]
                                                 for p in pairs for k in LANGUAGES):
            raise ValueError
    except (OSError, ValueError, KeyError, TypeError, AttributeError):
        refuse("badTable", file=path)
    return table


def load_defaults(root: Path) -> dict:
    path = root/"host/src/settings-defaults.json"
    try:
        defaults = json.loads(path.read_text(encoding="utf-8"))
        if not isinstance(defaults, dict):
            raise ValueError
    except (OSError, ValueError):
        refuse("badTable", file=path)
    return defaults


def check_config_dir(directory: Path) -> None:
    """The host's own rule for its settings folder (settings.rs SettingsStore::new): ours, a folder, no link."""
    if os.path.lexists(directory):
        info = directory.lstat()
        if directory.is_symlink() or not directory.is_dir() or info.st_uid != os.geteuid():
            refuse("configNotOwn", path=directory)


def store_language(directory: Path, defaults: dict, language: str) -> bool:
    """Write a complete settings.json (never a partial one: GE0c F3) with this language, the way the host writes:
    under .settings.lock, a 0600 temporary file, then rename. An existing settings.json is never replaced."""
    check_config_dir(directory)
    directory.mkdir(mode=0o700, parents=True, exist_ok=True)
    target = directory/"settings.json"
    lock = os.open(directory/".settings.lock", os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW | os.O_CLOEXEC, 0o600)
    try:
        fcntl.flock(lock, fcntl.LOCK_EX)
        if os.path.lexists(target):
            return False
        temporary = directory/f".settings-install-{os.getpid()}.tmp"
        fd = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC, 0o600)
        try:
            with os.fdopen(fd, "w", encoding="utf-8") as stream:
                stream.write(json.dumps({**defaults, "language": language}, indent=2, ensure_ascii=False) + "\n")
                stream.flush()
                os.fsync(stream.fileno())
            os.rename(temporary, target)
        finally:
            if os.path.lexists(temporary):
                temporary.unlink()
        return True
    finally:
        os.close(lock)


def language_lines(choice: dict, explicit: str | None) -> list[str]:
    lang, source, detail = choice["lang"], choice["source"], choice["detail"]
    if source in ("locale", "default"):
        if detail and detail.startswith("tz:"):
            return [t("langZone", zone=detail[3:])]
        if detail and detail.startswith("lc:"):
            return [t("langFormats", var=detail[3:])]
        return [t("langChosen")]
    name = t("name_en") if lang == "en" else t("name_nl")
    if source == "hyprland":
        return [t("langMigrated", name=name, file=detail)]
    if source == "settings" and explicit and explicit != lang:
        return [t("langKept", lang=explicit, name=name, file=detail, path=t("settingsPath", lang))]
    return []


def set_scheme_default(config: Path, desktop_id: str, mime: str) -> None:
    """xdg-mime default (else gio mime) for one of this project's own URI schemes, never for another type: one line
    in $XDG_CONFIG_HOME/mimeapps.list, which uninstall.py removes again. A missing tool is no error: the entry's
    MimeType= alone already makes it the only handler."""
    env = {**os.environ, "XDG_CONFIG_HOME": str(config)}
    if shutil.which("xdg-mime"):
        command = ["xdg-mime", "default", desktop_id, mime]
    elif shutil.which("gio"):
        command = ["gio", "mime", mime, desktop_id]
    else:
        return
    subprocess.run(command, env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False)


def install_extension(root: Path, extension: Path, tx: Transaction | None = None) -> None:
    """Every file of extension/ except the schemas (contracts C7, plan-critique F18), so a new module is never
    left out; a top-level file this version does not ship is removed from its own extension folder, and each one
    is named (ge1c-review m3: nothing goes without a trace). tx saves the whole folder first."""
    if tx:
        tx.save(extension)
    extension.mkdir(parents=True, exist_ok=True)
    shipped = {source.name for source in (root/"extension").iterdir() if source.is_file()}
    for name in sorted(shipped):
        shutil.copy2(root/"extension"/name, extension/name)
    for stale in sorted(extension.iterdir()):
        if stale.name not in shipped and stale.is_file() and not stale.is_symlink():
            stale.unlink()
            print(t("staleRemoved", path=stale))
    schemas = extension/"schemas"
    schemas.mkdir(exist_ok=True)
    shutil.copy2(root/"extension/schemas/org.gnome.shell.extensions.win11-gnome.gschema.xml", schemas)
    subprocess.run(["glib-compile-schemas", "--strict", str(schemas)], check=True)


def rescue_calendar(payload: Path, data: Path) -> None:
    """The Calendar's events: $XDG_DATA_HOME/win11-gnome/calendar until 13-11, which is this payload with the default
    prefix, so a payload swap or an uninstall removed them. Move them to win11-gnome-calendar, the Calendar's folder
    now (native/calendar/src/config.rs data_dir, which also moves them itself when it starts first)."""
    old, new = payload/"calendar", data/"win11-gnome-calendar"
    if payload == data/"win11-gnome" and old.is_dir() and not old.is_symlink() and not new.exists():
        old.rename(new)


# The WebKit apps' own storage, up to commit 6ebf169f: WebKitGTK kept each app's in $XDG_DATA_HOME/<app id> and
# $XDG_CACHE_HOME/<app id> (the file picker's under the portal's bus name), and the first builds' caches (1 Nov) in
# $XDG_CACHE_HOME/win11-gnome/{WebKitCache,CacheStorage} and $XDG_CACHE_HOME/win11-explorer. Nothing reads them now.
WEBKIT_APPS = ("Explorer", "Settings", "TaskManager", "Search", "Disks", "DefaultApps", "Installer", "Panel", "Start",
               "Desktop")
WEBKIT_PORTAL = "org.freedesktop.impl.portal.desktop.win11shell"
# Everything WebKitGTK puts in such a folder (its base data and cache directories); a folder holding anything else
# is not WebKit's alone and stays.
WEBKIT_ENTRIES = frozenset({"hsts-storage.sqlite", "hsts-storage.sqlite-wal", "hsts-storage.sqlite-shm",
                            "hsts-storage.sqlite-journal", "localstorage", "storage", "mediakeys", "databases",
                            "deviceidhashsalts", "serviceworkers", "itp", "WebKitCache", "CacheStorage"})


def own_entry(path: Path, kind) -> bool:
    """`path` is of this user and of this kind (stat.S_ISDIR, stat.S_ISREG), not a link to one."""
    try:
        info = path.lstat()
    except OSError:
        return False
    return kind(info.st_mode) and info.st_uid == os.geteuid()


def webkit_only(folder: Path) -> bool:
    try:
        return set(os.listdir(folder)) <= WEBKIT_ENTRIES
    except OSError:
        return False


def remove_webkit_state(data: Path, cache: Path, runtime: Path) -> None:
    """The WebKit apps' storage (WEBKIT_APPS, WEBKIT_PORTAL and win11-explorer, above) and the WebKit Explorer's
    restart lock, by this exact list only: a folder only when it is a real folder of this user holding nothing but
    WebKit's own entries, the two caches in win11-gnome only as real folders of this user, the lock only as a file of
    this user. Each removed path is named (ge1c-review m3). Win11 Shell for Hyprland's win11shell* folders, the
    w11gpuprobe-* probes and webkit.py are never on the list. window-state.json and .window-state.lock stay: the
    native Explorer keeps its session there (native/apps/explorer/src/transfer.rs)."""
    folders = [base/f"io.github.win11shell.{name}" for base in (data, cache) for name in WEBKIT_APPS]
    folders += [data/WEBKIT_PORTAL, cache/WEBKIT_PORTAL, cache/"win11-explorer"]
    doomed = [folder for folder in folders if own_entry(folder, stat.S_ISDIR) and webkit_only(folder)]
    if own_entry(cache/"win11-gnome", stat.S_ISDIR):
        doomed += [cache/"win11-gnome"/name for name in ("WebKitCache", "CacheStorage")
                   if own_entry(cache/"win11-gnome"/name, stat.S_ISDIR)]
    lock = runtime/"win11-gnome/explorer-restart.lock"
    if own_entry(lock.parent, stat.S_ISDIR) and own_entry(lock, stat.S_ISREG):
        doomed.append(lock)
    for path in doomed:
        try:
            if path == lock:
                path.unlink()
            else:
                shutil.rmtree(path)
        except OSError:
            continue
        print(t("webkitRemoved", path=path))


VERSION_FILE = "VERSION"
VERSION_RE = re.compile(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-[0-9A-Za-z.-]+)?")


def bundle_version(root: Path) -> str | None:
    """WinPoint's version (D-G74a): `[workspace.package] version` of the tree's root Cargo.toml, the one source.
    None without that file (the installer tests' trees) or without a plain X.Y.Z[-pre] there. The payload's VERSION
    holds it: WinPoint's update check compares with it (host/src/services/winpoint.rs); a payload without one is
    older than 0.9.0."""
    try:
        import tomllib
        version = tomllib.loads((root/"Cargo.toml").read_text(encoding="utf-8"))["workspace"]["package"]["version"]
    except (ImportError, OSError, UnicodeDecodeError, ValueError, KeyError, TypeError):
        return None
    return version if isinstance(version, str) and VERSION_RE.fullmatch(version) else None


def process_start(pid: int, proc: Path = Path("/proc")) -> str | None:
    """Field 22 of /proc/<pid>/stat (the start time), as the host's update::store::process_start reads it."""
    try:
        return (proc/str(pid)/"stat").read_text().rsplit(") ", 1)[1].split()[19]
    except (OSError, IndexError, UnicodeError):
        return None


def installer_running(folder: Path, proc: Path = Path("/proc")) -> bool:
    """job.json of WinPoint's updater (D-G74) says it is installing and its installer {pid, start} still runs."""
    try:
        job = json.loads((folder/"job.json").read_text(encoding="utf-8"))
        installer = job["installer"]
        pid, start = installer["pid"], installer["start"]
    except (OSError, UnicodeError, ValueError, KeyError, TypeError):
        return False
    if job.get("state") != "installing" or not isinstance(pid, int) or isinstance(pid, bool) or pid <= 0:
        return False
    return isinstance(start, str) and process_start(pid, proc) == start


def remove_winpoint_cache(cache: Path) -> None:
    """The update check's state (D-G74a: $XDG_CACHE_HOME/win11-gnome/winpoint, the host's 0700 folder of this
    user), on uninstall. Only that real folder of this user, never a link; nothing else of win11-gnome/ goes. It stays
    while job.json says an update's installer is still running (D-G74): that installer writes there."""
    folder = cache/"win11-gnome/winpoint"
    if own_entry(cache/"win11-gnome", stat.S_ISDIR) and own_entry(folder, stat.S_ISDIR) and not installer_running(folder):
        shutil.rmtree(folder, ignore_errors=True)


def cache_home() -> Path:
    """The XDG cache folder: $XDG_CACHE_HOME when it is absolute, else ~/.cache (the spec ignores a relative value;
    GLib's g_get_user_cache_dir an empty one). Only remove_webkit_state, remove_winpoint_cache and runtime_dir read
    it, so a bad value never stops an install or an uninstall."""
    value = os.environ.get("XDG_CACHE_HOME", "")
    return Path(value) if value.startswith("/") else Path.home()/".cache"


def runtime_dir(cache: Path) -> Path:
    """GLib's g_get_user_runtime_dir: $XDG_RUNTIME_DIR when it is absolute, else the cache folder."""
    value = os.environ.get("XDG_RUNTIME_DIR", "")
    return Path(value) if value.startswith("/") else cache


def remove_legacy(prefix: Path, applications: Path) -> None:
    """D-G70: the binaries and entries of an earlier version's WebKit Start and desktop (LEGACY_BINS, LEGACY_ENTRIES).
    Only files and links are removed; anything else at those paths is left alone."""
    paths = [prefix/"bin"/name for name in LEGACY_BINS]
    paths += [applications/f"io.github.win11shell.{name}.desktop" for name in LEGACY_ENTRIES]
    for path in paths:
        if path.is_symlink() or path.is_file():
            path.unlink()


# ---------------------------------------------------------------------------------------------------------------
# The lock, the repair of a killed install, and the checks of D-G74
# ---------------------------------------------------------------------------------------------------------------

PREVIOUS = ".win11-gnome-previous"           # the old payload until the commit
STAGE = ".win11-gnome-install-"              # the new payload while it is built
TX = ".win11-gnome-tx-"                      # Transaction's copies until the commit
LOCKED: list[tuple[Path, int]] = []          # (folder, fd) of the installer's flocks, held until the process ends


def owned(payload: Path) -> bool:
    try:
        return (payload/MARKER).is_file() and (payload/MARKER).read_text() == MAGIC
    except (OSError, UnicodeError):
        return False


def lock_share(share: Path) -> None:
    """flock(LOCK_EX | LOCK_NB) on a directory fd of the payload's parent (no new file, so a payload tree stays
    3b898e6's), or of its nearest existing folder before a first install creates it; held until this process ends.
    A second installer (or uninstall.py) meanwhile is refused: busy."""
    folder = share
    while not folder.is_dir() and folder != folder.parent:
        folder = folder.parent
    if any(path == folder for path, _fd in LOCKED):
        return
    fd = os.open(folder, os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC)
    try:
        fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except OSError:
        os.close(fd)
        refuse("busy", path=share)
    LOCKED.append((folder, fd))


def discard(folder: Path) -> None:
    """Remove a folder of ours (an old payload) without ever leaving half of it in its place: first one atomic rename
    to a Transaction name next to it, which repair() and uninstall.py remove without asking for the marker, then the
    tree. shutil.rmtree unlinks the top-level files first (the marker, the portal's and the icons' records) and the
    35,000 icons a second later; a kill in between used to leave a payload nobody would touch again."""
    aside = folder.with_name(f"{TX}discard-{os.getpid()}-{time.time_ns()}")
    folder.rename(aside)
    shutil.rmtree(aside, ignore_errors=True)


def leftovers(share: Path) -> list[Path]:
    """The stage and Transaction folders of this installer in the payload's parent, real folders of this user."""
    try:
        names = sorted(os.listdir(share))
    except OSError:
        return []
    return [share/name for name in names
            if name.startswith((STAGE, TX)) and own_entry(share/name, stat.S_ISDIR)]


def repair(payload: Path, prefix: Path) -> bool:
    """Inside the lock, what a killed install (SIGKILL, a power cut) left: an own previous payload is removed next to
    an own payload and renamed back without one; stage and Transaction folders and the binaries' and the uninstaller
    wrapper's .new copies go (remove_new_copies). Running the same tree again then rolls forward. True when an install
    was killed after its first visible change (a previous payload or a Transaction folder), so --check cannot tell
    which extension GNOME Shell runs."""
    share = payload.parent
    previous = share/PREVIOUS
    killed = False
    if own_entry(previous, stat.S_ISDIR) and owned(previous):
        killed = True
        if owned(payload):
            discard(previous)
        elif not os.path.lexists(payload):
            previous.rename(payload)
    for folder in leftovers(share):
        killed = killed or folder.name.startswith(TX)
        shutil.rmtree(folder, ignore_errors=True)
    remove_new_copies(prefix)
    return killed


def remove_new_copies(prefix: Path) -> None:
    """The binaries' and the uninstaller wrapper's temporary copies (apply()) that a killed install left, only plain
    files of this user (repair(), uninstall.py)."""
    for name in (*(f".{name}.new" for name in BINS), UNINSTALLER_NEW):
        temporary = prefix/"bin"/name
        if own_entry(temporary, stat.S_ISREG):
            temporary.unlink()


GNOME_SHELL = "/usr/bin/gnome-shell"


def gnome_major() -> str | None:
    """The major of the GNOME Shell the next login runs: `gnome-shell --version` prints it and exits while it parses
    its options. None when it is missing or says something else."""
    try:
        out = subprocess.run([GNOME_SHELL, "--version"], stdin=subprocess.DEVNULL, capture_output=True, text=True,
                             errors="replace", timeout=30, env={"PATH": "/usr/bin:/bin", "LC_ALL": "C"}).stdout
    except (OSError, subprocess.SubprocessError):
        return None
    match = re.match(r"GNOME Shell (\d+)\b", out.strip())
    return match.group(1) if match else None


def check_gnome(root: Path) -> None:
    """GNOME's own rule for an extension (host/src/services/update/compat.rs shell_matches): the running major is
    one of metadata.json's shell-version, or one of them starts with it and a dot. An unknown major is no refusal."""
    path = root/"extension/metadata.json"
    try:
        shells = json.loads(path.read_text(encoding="utf-8"))["shell-version"]
        if not isinstance(shells, list) or not all(isinstance(value, str) for value in shells):
            raise ValueError
    except (OSError, UnicodeError, ValueError, KeyError, TypeError):
        refuse("badTable", file=path)
    running = gnome_major()
    if running and not any(value == running or value.startswith(running + ".") for value in shells):
        refuse("gnomeMismatch", needs=", ".join(shells), running=running)


ABI_PROBLEM = re.compile(r"not found|undefined symbol|not a dynamic executable")
MACHINES = {3: "i386", 40: "arm", 62: "x86_64", 183: "aarch64", 243: "riscv64"}     # ELF e_machine


def elf_machine(path: Path) -> int | None:
    """e_machine of an ELF file of either class and byte order (None: no ELF)."""
    try:
        with open(path, "rb") as stream:
            head = stream.read(20)
    except OSError:
        return None
    if len(head) < 20 or head[:4] != b"\x7fELF" or head[5] not in (1, 2):
        return None
    return struct.unpack_from("<H" if head[5] == 1 else ">H", head, 18)[0]


def check_abi(build: Path) -> None:
    """Every binary that is ELF (the installer tests' stubs are not) must be made for this PC's machine, the e_machine
    of the python3 running this (a bundle of another architecture: abi), and `LC_ALL=C /usr/bin/ldd -r` must load it:
    a library, symbol version or symbol that this system lacks, e.g. a build made on a newer Ubuntu, or ldd's "not a
    dynamic executable", refuses before any change."""
    own = elf_machine(Path(os.path.realpath(sys.executable)))
    for name in BINS:
        machine = elf_machine(binary_source(build, name))
        if machine is not None and own is not None and machine != own:
            refuse("abi", binary=name, detail=t("abiMachine", needs=MACHINES.get(machine, f"e_machine {machine}"),
                                                running=MACHINES.get(own, f"e_machine {own}")))
    if not os.access("/usr/bin/ldd", os.X_OK):
        return
    for name in BINS:
        path = binary_source(build, name)
        if elf_needed(path) is None:
            continue
        try:
            proc = subprocess.run(["/usr/bin/ldd", "-r", str(path)], stdin=subprocess.DEVNULL, capture_output=True,
                                  text=True, errors="replace", timeout=60, env={**os.environ, "LC_ALL": "C"})
        except (OSError, subprocess.SubprocessError):
            continue
        problems = [line.strip() for line in (proc.stdout + proc.stderr).splitlines() if ABI_PROBLEM.search(line)]
        if problems:
            refuse("abi", binary=name, detail="; ".join(problems[:3]))


def read_bytes(path: Path) -> bytes | None:
    try:
        return path.read_bytes()
    except OSError:
        return None


def system_files(root: Path, system: Path | None = None) -> list[str]:
    """--check's report of root-owned files that only a password installs: "session" when the four files of
    install-session.py are there and one differs from what this tree's would write (stale(): winpoint.json by
    meaning, so an Ubuntu update of its own ubuntu.json is no difference), "system" when one of install-system.py's
    (SYSTEM_FILES) is there and differs from root/host/data. No refusal: a terminal install puts them right with one
    password (root_step), and the updater sends the user to the terminal."""
    found = []
    session = session_script()
    system = system_root() if system is None else system
    if session.installed(system) and session.stale(system):
        found.append("session")
    if any((system/rel).is_file() and read_bytes(system/rel) != read_bytes(root/"host/data"/Path(rel).name)
           for rel in SYSTEM_FILES):
        found.append("system")
    return found


def extension_files(folder: Path) -> dict[str, bytes | None]:
    """The top-level files and schemas/*.gschema.xml of an extension folder (install_extension's set), by name."""
    paths = [*folder.glob("*"), *folder.glob("schemas/*.gschema.xml")] if folder.is_dir() else []
    return {path.relative_to(folder).as_posix(): read_bytes(path) for path in sorted(paths) if path.is_file()}


def extension_changed(root: Path, extension: Path) -> bool:
    """The file set and bytes of extension/'s top-level files and schemas/*.gschema.xml differ from the installed
    folder's: GNOME Shell loads an extension at login, so the new one runs after a sign-out."""
    return not extension.is_dir() or extension_files(root/"extension") != extension_files(extension)


# ---------------------------------------------------------------------------------------------------------------
# The transaction
# ---------------------------------------------------------------------------------------------------------------

SIGNALS = (signal.SIGTERM, signal.SIGHUP, signal.SIGINT)


class Interrupted(BaseException):
    """SIGTERM (the updater's timeout, systemd's stop), SIGHUP (a closed terminal) or Ctrl+C: raised once, and the
    handler ignores all three first, so no later signal interrupts the rollback."""


def hold_signals() -> None:
    for each in SIGNALS:
        signal.signal(each, signal.SIG_IGN)


def interrupted(number, _frame):
    hold_signals()
    raise Interrupted(number)


TEST_FAIL = os.environ.get("WIN11_INSTALL_TEST_FAIL", "")


def test_point(step: str) -> None:
    """Test seam (tests/installer only; the updater's environment never has it): WIN11_INSTALL_TEST_FAIL=<step> makes
    the install fail right after that step, <step>:wait first waits up to 30 s for a signal (the variable's value goes
    to stderr then), <step>:pause only waits 3 s the same way and goes on, <step>:rollback also leaves the first undo
    step out. It can only make an install fail or wait."""
    wanted, _, mode = TEST_FAIL.partition(":")
    if wanted != step:
        return
    if mode in ("wait", "pause"):
        sys.stderr.write(TEST_FAIL + "\n")
        sys.stderr.flush()
        deadline = time.monotonic() + (30 if mode == "wait" else 3)
        while time.monotonic() < deadline:
            time.sleep(0.05)
    if mode != "pause":
        raise RuntimeError(t("testFail", step=step))


def remove(path: Path) -> None:
    if path.is_dir() and not path.is_symlink():
        shutil.rmtree(path)
    elif os.path.lexists(path):
        path.unlink()


class Transaction:
    """Every change of apply(), undone in reverse order after an error or a signal (D-G74). Before a file, link or
    folder is written, save() copies it into <prefix>/share/.win11-gnome-tx-*/ (or notes that it is new, with the
    folders that do not exist yet); the old payload waits as .win11-gnome-previous; a replaced binary keeps its
    original inode as a hard link there, so rolling back renames that very file back (running programs and the
    host's restart check see their own inode again). The commit removes the copies and the old payload. Not rolled
    back, on purpose: store_language (it only adds a missing settings.json), rescue_calendar (a one-way move of the
    Calendar's events that both versions read) and set_scheme_default (one idempotent line)."""

    def __init__(self, share: Path):
        self.share = share
        self.dir: Path | None = None
        self.log: list[tuple] = []
        self.seen: set[Path] = set()                # saved paths
        self.folders: set[Path] = set()             # folders noted as missing (a writer makes them)
        self.problems: list[tuple[Path, OSError]] = []
        self.skip_first = TEST_FAIL.endswith(":rollback")

    def begin(self) -> None:
        self.dir = Path(tempfile.mkdtemp(prefix=TX, dir=self.share))

    def makedirs(self, path: Path) -> None:
        missing = []
        while not os.path.lexists(path):
            missing.append(path)
            path = path.parent
        for folder in reversed(missing):
            self.log.append(("dir", folder))
            folder.mkdir()

    def track(self, path: Path) -> Path:
        """A new folder or file of this install (the stage, a .new copy): removed on a rollback."""
        self.log.append(("new", path))
        return path

    def save(self, path: Path) -> None:
        path = Path(path)
        if path in self.seen:
            return
        self.seen.add(path)
        if path.is_symlink():
            self.log.append(("link", path, os.readlink(path)))
        elif os.path.lexists(path):
            copy = self.dir/str(len(self.log))
            if path.is_dir():
                shutil.copytree(path, copy, symlinks=True)
            else:
                shutil.copy2(path, copy, follow_symlinks=False)
            self.log.append(("copy", path, copy))
        else:
            missing, folder = [], path.parent
            while not os.path.lexists(folder) and folder not in self.folders:
                self.folders.add(folder)
                missing.append(folder)
                folder = folder.parent
            self.log += [("dir", folder) for folder in reversed(missing)]
            self.log.append(("new", path))

    def swap(self, stage: Path, payload: Path) -> None:
        """The payload swap: the old one waits as PREVIOUS until the commit (it used to go right away). One log entry,
        so its undo is two renames (undo())."""
        previous = payload.with_name(PREVIOUS) if os.path.lexists(payload) else None
        self.log.append(("swapped", payload, previous))     # first: undo() copes with either rename not done yet
        if previous is not None:
            payload.rename(previous)
        stage.rename(payload)

    def binary(self, temporary: Path, destination: Path) -> None:
        """The second phase of the binaries: one rename each (the slow copies to .new came first)."""
        backup = None
        if os.path.lexists(destination):
            (self.dir/"bin").mkdir(exist_ok=True)
            backup = self.dir/"bin"/destination.name
            try:
                os.link(destination, backup, follow_symlinks=False)
            except OSError:
                shutil.copy2(destination, backup, follow_symlinks=False)
        self.log.append(("bin", destination, backup))
        os.replace(temporary, destination)

    def undo(self, entry: tuple) -> None:
        kind, path = entry[0], entry[1]
        if kind == "dir":
            try:
                path.rmdir()
            except OSError:
                pass                                    # gone already, or it holds something else: it stays
        elif kind == "new":
            remove(path)
        elif kind == "copy":
            remove(path)
            shutil.move(entry[2], path)
        elif kind == "link":
            remove(path)
            os.symlink(entry[2], path)
        elif kind == "swapped":
            # the new payload first goes aside into the Transaction folder (one rename in the same folder; that
            # folder goes at the end of rollback(), or with repair()), then the old one comes back: never a half
            # removed payload in its place
            previous = entry[2]
            if os.path.lexists(path) and (previous is None or os.path.lexists(previous)):
                path.rename(self.dir/"payload")
            if previous is not None and os.path.lexists(previous):
                previous.rename(path)
        elif kind == "bin":
            if entry[2] is None:
                remove(path)
            else:
                shutil.move(entry[2], path)             # a rename: the original inode is back

    def rollback(self) -> bool:
        """Everything back, the last change first; False when something could not be put back (problems). The new
        folders go last, after the copies' own folder (which sits in one of them on a first install)."""
        complete = True
        for entry in reversed(self.log):
            if entry[0] == "dir":
                continue
            try:
                if self.skip_first:
                    self.skip_first = False
                    raise OSError(t("testFail", step="rollback"))
                self.undo(entry)
            except OSError as error:
                complete = False
                self.problems.append((entry[1], error))
        if complete and self.dir:
            shutil.rmtree(self.dir, ignore_errors=True)
        for entry in reversed(self.log):
            if entry[0] == "dir":
                self.undo(entry)
        self.log = []
        return complete

    def commit(self) -> None:
        previous = next((entry[2] for entry in self.log if entry[0] == "swapped"), None)
        self.log = []
        if previous is not None and os.path.lexists(previous):
            try:
                previous.rename(self.dir/"previous")
            except OSError:
                shutil.rmtree(previous, ignore_errors=True)
        if self.dir:
            shutil.rmtree(self.dir, ignore_errors=True)


# ---------------------------------------------------------------------------------------------------------------
# prepare, apply, --check
# ---------------------------------------------------------------------------------------------------------------

@dataclass
class Plan:
    root: Path
    explicit: str | None
    prefix: Path
    data: Path
    config: Path
    cache: Path
    payload: Path
    choice: dict | None = None
    language: str = "nl"
    build: Path | None = None
    extension: Path | None = None
    applications: Path | None = None
    entries: dict | None = None
    defaults: dict | None = None
    updating: bool = False
    previous_icons: dict | None = None
    session: bool = False
    portal: dict | None = None
    terminal: bool = False          # the terminal path (terminal_install): root_step, preselect, the two lists
    hint: bool = True               # the systemHint line may follow (root_step: not when it saw to the system files)
    root_changed: bool = False      # root_step ran install-session.py: a rollback leaves its files (rolledBackRoot)
    uninstaller: bool = True        # <prefix>/bin/winpoint-uninstall is written (not over another one on an update)


def locate(root: Path, options: Options) -> Plan:
    """Where everything goes; as root nothing at all (noRoot): WinPoint belongs to the user's own account, and even
    repair() would leave root-owned files in it. Without --lang the messages until prepare() are in the language it
    will pick (module docstring; README: "picks Dutch or English from your system's language"): first the locale
    guess, which reads no home folder (sudo's is root's), then, once the paths are known, resolve_language's choice,
    read only."""
    if options.explicit is None:
        set_language(guess_language(os.environ)[0])
    if os.geteuid() == 0:
        refuse("noRoot")
    prefix = absolute_path(os.environ.get("WIN11_INSTALL_PREFIX", str(Path.home()/".local")), t("labelPrefix"))
    data = absolute_path(os.environ.get("XDG_DATA_HOME", str(Path.home()/".local/share")), "XDG_DATA_HOME")
    config = absolute_path(os.environ.get("XDG_CONFIG_HOME", str(Path.home()/".config")), "XDG_CONFIG_HOME")
    payload = prefix/"share/win11-gnome"
    if options.explicit is None:
        set_language(resolve_language(None, config, data, payload)["lang"])
    return Plan(root, options.explicit, prefix, data, config, cache_home(), payload)


def prepare(plan: Plan, check: bool = False) -> Plan:
    """Every check of an install, nothing written; the order and messages of 3b898e6's are kept, the new checks
    (GNOME, ABI, previousCopy) sit where they refuse least surprisingly. The terminal path's own checks come last
    (terminal_install); its one password follows in root_step(), before apply(). check: --check (never that path)."""
    root, payload, data, config = plan.root, plan.payload, plan.data, plan.config
    plan.choice = resolve_language(plan.explicit, config, data, payload)
    plan.language = plan.choice["lang"]
    set_language(plan.explicit or plan.language)
    plan.build = choose_build(root)
    plan.extension = data/"gnome-shell/extensions"/UUID
    plan.applications = data/"applications"
    # The wallpapers and icons the native shell and apps fetch (/ui/assets/). vendor/host, the Python reference, is
    # not installed since D-G72 and left the tree with D-G73 (git show 6ebf169f:vendor/host).
    if not (root/"vendor/ui/assets").is_dir():
        refuse("missingSource", directory=root/"vendor/ui/assets")
    plan.entries = load_entries(root)
    plan.defaults = load_defaults(root) if plan.choice["store"] else None
    check_gnome(root)
    preflight_runtime(plan.build)
    check_abi(plan.build)
    if payload.exists() and (not (payload/MARKER).is_file() or (payload/MARKER).read_text() != MAGIC):
        refuse("notOwned", payload=payload)
    plan.updating = payload.exists()
    if plan.updating and os.path.lexists(payload.with_name(PREVIOUS)):
        refuse("previousCopy", backup=payload.with_name(PREVIOUS))
    uninstaller = plan.prefix/"bin"/UNINSTALLER
    foreign = os.path.lexists(uninstaller) and not own_uninstaller(uninstaller)
    # an update over its own payload never refuses for it (the updater's path stays 0.9.0's): it leaves that file
    plan.uninstaller = not (plan.updating and foreign)
    if not plan.updating:
        conflicts = [plan.prefix/"bin"/name for name in BINS]
        conflicts += [plan.applications/f"io.github.win11shell.{name}.desktop" for name in ENTRIES]
        conflicts += [plan.extension, config/"autostart"/AUTOSTART]
        existing = [str(path) for path in conflicts if path.exists()] + ([str(uninstaller)] if foreign else [])
        if existing:
            raise Refusal("conflicts", {"paths": existing}, t("conflicts") + "\n" + "\n".join(existing))
    if plan.choice["store"]:
        check_config_dir(config/"win11-gnome")
    icon_bundle.validate(root/"vendor")
    plan.previous_icons = icon_bundle.preflight_aliases(payload, data)
    plan.terminal = terminal_install(check)
    plan.session = plan.terminal or session_script().installed(system_root())
    plan.portal = file_portal.preflight(payload, plan.prefix, data, config, plan.session)
    return plan


def stage_payload(plan: Plan, stage: Path) -> None:
    """The new payload in its stage, before the swap. The old payload's own records (the portal's and the icon
    aliases') go along until register_aliases and file_portal.register rewrite them, so a killed install still finds
    them when it is run again (repair)."""
    root, payload = plan.root, plan.payload
    # ui/: only vendor/ui/assets (wallpapers, the Settings icon), whatever else vendor/ui still holds: the HTML
    # apps are gone (git show 6ebf169f:vendor/ui). config/: the Windows 11 caption buttons for other GTK apps
    # (D-G65), imported from gtk.css by the panel host.
    for source, target in (("ui/assets", "ui/assets"), ("config", "config")):
        if not (root/"vendor"/source).is_dir():
            continue
        shutil.copytree(root/"vendor"/source, stage/target, ignore=shutil.ignore_patterns("__pycache__", "*.pyc"))
    for manifest in ("ui-sha256.json", "ui-original-sha256.json", "ui-patches.json"):
        if (root/"vendor"/manifest).is_file():
            shutil.copy2(root/"vendor"/manifest, stage/manifest)
    icon_bundle.copy_into_payload(root, stage)
    for record in (file_portal.MANIFEST, icon_bundle.TRACKER, icon_bundle.STATE):
        if plan.updating and own_entry(payload/record, stat.S_ISREG):
            shutil.copy2(payload/record, stage/record)
    (stage/MARKER).write_text(MAGIC)
    version = bundle_version(root)
    if version:
        (stage/VERSION_FILE).write_text(version + "\n")
    # 0.9.1: what <prefix>/bin/winpoint-uninstall runs, from this installer's own folder (also when ROOT holds no
    # scripts/, as the installer tests' trees)
    (stage/"scripts").mkdir()
    for name in UNINSTALL_SCRIPTS:
        shutil.copy2(Path(__file__).resolve().with_name(name), stage/"scripts"/name)


def write_entries(plan: Plan, tx: Transaction) -> None:
    payload, applications, language = plan.payload, plan.applications, plan.language
    tx.makedirs(applications)
    common = "Terminal=false\nStartupNotify=true\n"
    # Name= and Comment= in Win11's language (C6); win11-panel later rewrites only these two lines after a switch.
    comment = plan.entries["comment"][language]
    # The Windows 11 calendar of the bundled Win11 icon theme (vendor/icons, icon_bundle.py), else the theme's own
    calendar_icon = payload/"icons/Win11/apps/scalable/office-calendar.svg"

    def bundled(name: str, fallback: str) -> str:
        path = payload/"icons/Win11"/BUNDLED_ICONS[name]
        return str(path) if path.is_file() else fallback

    rows = [
        ("Panel", "win11-panel", "view-app-grid-symbolic", "NoDisplay=true\nCategories=Utility;\n"),
        ("Settings", "win11-app", str(payload/"ui/assets/icons/settings.svg"), "NoDisplay=true\nCategories=Settings;\n"),
        ("DefaultApps", "win11-default-apps", str(payload/"ui/assets/icons/settings.svg"), "Categories=Settings;System;\n"),
        ("Installer", "win11-installer", bundled("Installer", "system-software-install"), "Categories=System;PackageManager;\n"),
        ("Search", "win11-app", bundled("Search", "system-search"), "NoDisplay=true\nCategories=Utility;\n"),
        ("Disks", "win11-app", bundled("Disks", "drive-harddisk"), "NoDisplay=true\nCategories=System;\n"),
        ("TaskManager", "win11-app", bundled("TaskManager", "utilities-system-monitor"), "NoDisplay=true\nCategories=System;Monitor;\n"),
        ("Explorer", "win11-explorer", bundled("Explorer", "system-file-manager"),
         "Categories=System;FileTools;FileManager;\nMimeType=inode/directory;\n"),
        ("Calendar", "win11-calendar", str(calendar_icon) if calendar_icon.is_file() else "x-office-calendar",
         f"Categories=Office;Calendar;\nMimeType={CALENDAR_SCHEME};\n"),
    ]
    for name, binary, icon, extra in rows:
        title = plan.entries["entries"][name][language]
        text = f"[Desktop Entry]\nType=Application\nName={title}\n"
        text += f"Comment={comment}\n"
        arguments = " %U" if name == "Explorer" else {"TaskManager": " --app taskmgr", "Settings": " --app settings", "Search": " --app alles", "Disks": " --app disks", "Calendar": " %u"}.get(name, "")
        text += f"Exec=/usr/bin/env -- {desktop_exec(plan.prefix/'bin'/binary)}{arguments}\n"
        text += f"Icon={icon}\n" + common + f"StartupWMClass=io.github.win11shell.{name}\n" + extra
        path = applications/f"io.github.win11shell.{name}.desktop"
        tx.save(path)
        path.write_text(text)


def apply(plan: Plan) -> int:
    """Every change, in one Transaction (module docstring): 0, or after an error or a signal 3 (all put back) or 4
    (not all). After the commit only the removal of an earlier version's leftovers and the closing lines follow."""
    payload, prefix, data, config = plan.payload, plan.prefix, plan.data, plan.config
    tx = Transaction(payload.parent)
    try:
        # The choice is stored after every check and before the first change (ge1c-review m1): when a later step
        # fails, the rerun finds settings.json and keeps this language, instead of taking the update-without-choice
        # path (nl, H D13). It never replaces an existing file. The language line goes out with it, as the first
        # line, as before.
        if plan.choice["store"]:
            store_language(config/"win11-gnome", plan.defaults, plan.language)
        for line in language_lines(plan.choice, plan.explicit):
            print(line)
        rescue_calendar(payload, data)
        tx.makedirs(payload.parent)
        lock_share(payload.parent)
        tx.begin()
        stage = tx.track(Path(tempfile.mkdtemp(prefix=STAGE, dir=payload.parent)))
        stage_payload(plan, stage)
        # The binaries in two phases: the slow copies first (nothing visible yet), then one rename each, so running
        # app processes keep their original inode and a rollback can rename it back.
        bindir = prefix/"bin"
        tx.makedirs(bindir)
        for binary in BINS:
            temporary = tx.track(bindir/f".{binary}.new")
            shutil.copy2(binary_source(plan.build, binary), temporary)
            temporary.chmod(0o755)
        tx.swap(stage, payload)
        test_point("payload")
        for index, binary in enumerate(BINS):
            tx.binary(bindir/f".{binary}.new", bindir/binary)
            if index == 0:
                test_point("binary")
        if plan.uninstaller:
            # as the binaries: the whole file first, then one rename, so a kill never leaves half a wrapper
            uninstaller, temporary = bindir/UNINSTALLER, tx.track(bindir/UNINSTALLER_NEW)
            temporary.write_text(uninstaller_text(payload), encoding="utf-8")
            temporary.chmod(0o755)
            tx.save(uninstaller)
            os.replace(temporary, uninstaller)
        test_point("binaries")
        icon_bundle.register_aliases(payload, data, plan.previous_icons, tx)
        test_point("aliases")
        file_portal.register(payload, plan.portal, tx)
        test_point("portal")
        install_extension(plan.root, plan.extension, tx)
        test_point("extension")
        write_entries(plan, tx)
        test_point("entries")
        autostart = config/"autostart"
        tx.makedirs(autostart)
        for name in (AUTOSTART, GNOME_LOOK):
            tx.save(autostart/name)
        (autostart/AUTOSTART).write_text(autostart_entry(prefix, plan.language))
        if plan.session:
            (autostart/GNOME_LOOK).write_text(gnome_look_entry(prefix, plan.language))
        else:
            (autostart/GNOME_LOOK).unlink(missing_ok=True)
        test_point("autostart")
        if shutil.which("update-desktop-database"):
            tx.save(plan.applications/"mimeinfo.cache")
            subprocess.run(["update-desktop-database", str(plan.applications)], check=True)
        set_scheme_default(config, CALENDAR_DESKTOP, CALENDAR_SCHEME)
        # the last statement of the try: a signal until all three are ignored still rolls back; once they are,
        # CPython drops any that is pending, so the commit below cannot be interrupted any more
        hold_signals()
    except BaseException as error:
        # a signal handled before this ignores them (also at the first instruction of this clause) raises once
        # more: ignore it and go on, so the rollback always runs
        while True:
            try:
                hold_signals()
                break
            except Interrupted:
                pass
        complete = tx.rollback()
        try:
            if isinstance(error, (Interrupted, KeyboardInterrupt)):
                print(t("interrupted"), file=sys.stderr)
            elif isinstance(error, SystemExit) and isinstance(error.code, str):
                print(error.code, file=sys.stderr)
            else:
                traceback.print_exception(error)
            for path, problem in tx.problems:
                print(t("notRestored", path=path, error=problem), file=sys.stderr)
            if not complete:
                print(t("rollbackIncomplete"), file=sys.stderr)
            elif plan.root_changed:
                # the root step's files stay (WinPoint at the login screen): never "everything was put back"
                command = "python3 " + shlex.quote(str(Path(__file__).resolve().with_name("install-session.py")))
                print(t("rolledBackRoot", command=command + " --remove"), file=sys.stderr)
            else:
                print(t("rolledBack"), file=sys.stderr)
        except OSError:
            pass
        return 3 if complete else 4
    hold_signals()                      # installed: nothing left to put back, so nothing may stop the rest
    tx.commit()
    if plan.updating:
        # D-G70: an earlier version's WebKit Start and desktop (the payload is ours, so these files are too); only
        # now, so a rollback finds that version's own files still there
        remove_legacy(prefix, plan.applications)
        remove_webkit_state(data, plan.cache, runtime_dir(plan.cache))
    # the terminal path (module docstring): WinPoint at the next sign-in, and its UUID out of the two lists
    preselected = plan.terminal and preselect()
    moved = plan.terminal and allow_mode_extension()
    print(t("done1", prefix=prefix))
    print(t("done2"))
    if plan.session:
        print(t("session1"))
        if moved:
            print(t("sessionMoved"))
        print(t("sessionNext") if preselected else t("session2"))
    else:
        print(t("done3"))
        print(t("done4"))
        print(t("done5", uuid=UUID))
    print(t("done6"))
    hint = system_hint(False) if plan.hint else None
    if hint and sys.stdout.isatty():
        print(hint)
    return 0


def answer_of(error: BaseException) -> dict:
    """--check's answer to a refusal: install.py's Refusal, file_portal's SystemExit and icon_bundle's ValueError
    carry their key; anything else is checkFailed."""
    key = getattr(error, "key", None)
    if key:
        message = error.code if isinstance(error, SystemExit) else str(error)
        return {"ok": False, "code": key, "params": getattr(error, "params", {}), "message": str(message)}
    if isinstance(error, (Interrupted, KeyboardInterrupt)):
        return {"ok": False, "code": "interrupted", "params": {}, "message": t("interrupted")}
    detail = error.code if isinstance(error, SystemExit) and isinstance(error.code, str) else \
        "".join(traceback.format_exception_only(error)).strip()
    return {"ok": False, "code": "checkFailed", "params": {"detail": detail}, "message": t("checkFailed", detail=detail)}


def check(root: Path, options: Options) -> int:
    """--check: prepare() and the reports (module docstring); stdout gets the answer only."""
    out, sys.stdout = sys.stdout, sys.stderr
    try:
        plan = locate(root, options)
        lock_share(plan.payload.parent)
        killed = repair(plan.payload, plan.prefix) if plan.payload.parent.is_dir() else False
        prepare(plan, check=True)
        answer = {"ok": True, "version": bundle_version(root),
                  "extensionChanged": killed or extension_changed(root, plan.extension),
                  "systemFiles": system_files(root)}
    except BaseException as error:              # every refusal and error, also a signal, is the answer
        answer = answer_of(error)
    finally:
        sys.stdout = out
    print(json.dumps(answer, ensure_ascii=True, separators=(",", ":")))
    sys.stdout.flush()
    return 0 if answer["ok"] else 1


def main(root: Path, argv: list[str] | None = None) -> int:
    options = parse_arguments(list(argv or []))
    if options.check:
        return check(root, options)
    plan = locate(root, options)
    lock_share(plan.payload.parent)
    if plan.payload.parent.is_dir():
        repair(plan.payload, plan.prefix)
    plan = prepare(plan)
    root_step(plan)
    return apply(plan)


def redirect_output(path: Path) -> None:
    """--log: fds 1 and 2 into FILE (0600, a new file each run), so the programs this runs write there too."""
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC | os.O_NOFOLLOW | os.O_CLOEXEC, 0o600)
    sys.stdout.flush()
    sys.stderr.flush()
    os.dup2(fd, 1)
    os.dup2(fd, 2)
    os.close(fd)
    sys.stdout.reconfigure(line_buffering=True)


def write_status(path: Path, code: int) -> None:
    """--status: "<code>\n" through a temporary file and a rename, so a reader never sees half of it."""
    try:
        fd, temporary = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    except OSError:
        return
    try:
        with os.fdopen(fd, "w") as stream:
            stream.write(f"{code}\n")
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
    except OSError:
        pass
    finally:
        if os.path.lexists(temporary):
            os.unlink(temporary)


def run(argv: list[str]) -> int:
    """install.py ROOT [options]: the exit code of the frozen contract (module docstring). A signal this process was
    started with ignored stays ignored (nohup, the SIGINT of a background job): only the others interrupt it."""
    for number in SIGNALS:
        if signal.getsignal(number) is not signal.SIG_IGN:
            signal.signal(number, interrupted)
    status = option_value(argv[1:], "--status")
    code = 1
    try:
        log = option_value(argv[1:], "--log")
        if log:
            redirect_output(Path(log))
        code = main(Path(argv[0]).resolve(), argv[1:])
    except SystemExit as error:
        if error.code is None or isinstance(error.code, int):
            code = error.code or 0
        else:
            print(error.code, file=sys.stderr)
            code = 1
    except (Interrupted, KeyboardInterrupt):
        print(t("interrupted"), file=sys.stderr)
        code = 1
    except Exception:
        traceback.print_exc()
        code = 1
    finally:
        hold_signals()
        try:
            sys.stdout.flush()
        except OSError:
            pass
        if status:
            write_status(Path(status), code)
    return code


if __name__ == "__main__":
    sys.exit(run(sys.argv[1:]))
