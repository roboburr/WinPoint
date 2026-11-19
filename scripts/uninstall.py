#!/usr/bin/env python3
"""Remove only the marked installation; preserve personal settings and original Win11Shell.

Usage: scripts/uninstall.sh [--files-only] [--keep-session] [--lang nl|en], or after an install
<prefix>/bin/winpoint-uninstall with the same options (it runs the payload's own copy: uninstall.py PAYLOAD [...];
PAYLOAD, <prefix>/share/win11-gnome, names the prefix, so WIN11_INSTALL_PREFIX is needed only without it).
Messages: --lang, else Win11's language (C1).

0.9.1 (Robin, 16-11: one command, one password): not in the WinPoint session itself (sign in to Ubuntu first) unless
--files-only. Every module it needs is loaded before anything goes, then the one root removal comes first:
install-session.py --remove takes the login session's four files and install-system.py's four in one pkexec (only
those that are there; none: no password). A cancelled or wrong password is exit 1 with nothing removed;
--keep-session leaves those eight, --files-only (as before) everything but the user's own files. Then this user's
AccountsService Session goes back to "" (GDM's default) when it is "winpoint", WinPoint's UUID leaves enabled-extensions
and disabled-extensions (GSettings, as host/src/gnome_look.rs), win11-app --gnome-look gives GNOME its own look back
(also after an install inside the normal GNOME session), and the files go; its own running scripts (the wrapper, the
payload with its scripts/) last. One closing line; on a terminal, a hint before it when root files stay.
Without a payload (0.9.0's uninstall.sh removed it and knows nothing of the rest) but with this prefix's own
winpoint-uninstall, only that rest goes: the root files with one password, Session, the two lists and the wrapper.
Without either it is nothingRemoved, as before, with the hint on a terminal when root files are there (a first
install rolled back after its root step); it never removes them then: they may belong to an install in another prefix.
"""
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import sys
import install
from install import MAGIC, MARKER, UUID, BINS, ENTRIES, LEGACY_BINS, LEGACY_ENTRIES, LANGUAGES, absolute_path, language_of_file, set_language
import icon_bundle
import file_portal
try:
    from gi.repository import Gio
except ImportError:
    Gio = None

# i18n: pairs (tools/i18n/check_pairs.py); inline, as these texts appear when something is broken (contracts C3).
T = {
    "nothingRemoved": {"nl": "Niets verwijderd: {payload} bevat geen installatiemarker van dit project.", "en": "Nothing removed: {payload} has no installation marker of this project."},
    "usage": {"nl": "Gebruik: winpoint-uninstall of scripts/uninstall.sh [--files-only] [--keep-session] [--lang nl|en]",
              "en": "Usage: winpoint-uninstall or scripts/uninstall.sh [--files-only] [--keep-session] [--lang nl|en]"},
    # the hint when root files stay (--keep-session, --files-only): sessionHint while GDM's list entry is there, else
    # systemHint; from the payload, which is gone afterwards, {command} is releaseCommand
    "sessionHint": {"nl": "WinPoint staat nog als keuze bij het inloggen. Weghalen: {command}", "en": "WinPoint is still a choice at the login screen. To remove it: {command}"},
    "systemHint": {"nl": "De systeembestanden van WinPoint staan er nog. Weghalen: {command}", "en": "WinPoint’s system files are still installed. To remove them: {command}"},
    "releaseCommand": {"nl": "python3 scripts/install-session.py --remove in de map van een uitgepakte WinPoint-uitgave (0.9.1 of nieuwer), met het wachtwoord van een beheerder",
                       "en": "python3 scripts/install-session.py --remove in the folder of an unpacked WinPoint release (0.9.1 or newer), with an administrator’s password"},
    "insideWinPoint": {"nl": "Je bent nu aangemeld bij WinPoint. Meld je af, kies Ubuntu bij het inloggen (het tandwiel rechtsonder, na je naam) en verwijder WinPoint daar. Er is niets verwijderd.",
                       "en": "You’re signed in to WinPoint now. Sign out, choose Ubuntu at the login screen (the gear at the bottom right, after your name) and remove WinPoint there. Nothing was removed."},
    "rootCancelled": {"nl": "WinPoint is niet verwijderd. Start het verwijderen opnieuw en vul het wachtwoord in, of laat de systeembestanden van WinPoint staan met --keep-session.",
                      "en": "WinPoint was not removed. Run the uninstall again and enter the password, or keep WinPoint’s system files with --keep-session."},
    "removed": {"nl": "WinPoint is verwijderd. Je instellingen en oorspronkelijke Win11Shell zijn behouden.", "en": "WinPoint removed. Your settings and the original Win11Shell were kept."},
}


def t(key, **params):
    pair = T[key]
    return (pair.get(install.LANGUAGE) or pair["nl"]).format_map(params)


def drop_scheme_default(config):
    """install.set_scheme_default's line in $XDG_CONFIG_HOME/mimeapps.list: only the Calendar's own scheme, only when
    it still names our entry; every other line stays as it is. A linked or unreadable file is left alone."""
    path = config/"mimeapps.list"
    if path.is_symlink() or not path.is_file():
        return
    try:
        lines = path.read_text(encoding="utf-8").splitlines(keepends=True)
    except (OSError, UnicodeError):
        return
    key = install.CALENDAR_SCHEME + "="
    own = (install.CALENDAR_DESKTOP, install.CALENDAR_DESKTOP + ";")
    kept = [line for line in lines if not (line.startswith(key) and line[len(key):].strip() in own)]
    if len(kept) == len(lines):
        return
    temporary = path.with_name(f".mimeapps.list.win11-gnome-{os.getpid()}.tmp")
    try:
        temporary.write_text("".join(kept), encoding="utf-8")
        temporary.chmod(path.stat().st_mode & 0o7777)
        temporary.replace(path)
    except OSError:
        temporary.unlink(missing_ok=True)


def root_hint(session, system):
    """The hint line when root files stay (--keep-session, --files-only, no install of this prefix), or None:
    sessionHint while GDM's list entry is there, else systemHint, naming a script that is still there afterwards:
    this folder's install-session.py, or from the payload (gone by then) that of a release (releaseCommand);
    install-session.py --remove takes all eight."""
    left = session.removable(system)
    if not left:
        return None
    command = t("releaseCommand") if from_payload else \
        "python3 " + str(Path(__file__).resolve().with_name("install-session.py")) + " --remove"
    return t("sessionHint", command=command) if session.MARKER in left else t("systemHint", command=command)


def closing(session, system):
    """The hint on a terminal (root_hint), then the one closing line."""
    hint = root_hint(session, system)
    if hint and sys.stdout.isatty():
        print(hint)
    print(t("removed"))


arguments, explicit, index = [], None, 1
while index < len(sys.argv):
    argument = sys.argv[index]
    if argument == "--lang" or argument.startswith("--lang="):
        if argument == "--lang":
            index += 1
            value = sys.argv[index] if index < len(sys.argv) else ""
        else:
            value = argument.split("=", 1)[1]
        explicit = value.strip().lower()
        if explicit not in LANGUAGES:
            print(install.t("badLang", "nl", lang=value) + "\n" + install.t("badLang", "en", lang=value), file=sys.stderr)
            raise SystemExit(2)
        set_language(explicit)
    else:
        arguments.append(argument)
    index += 1
# The tree these scripts belong to: PAYLOAD from winpoint-uninstall (install.uninstaller_text), else this file's own
given = Path(arguments.pop(0)) if arguments and arguments[0].startswith("/") else None
tree = given or Path(__file__).resolve().parents[1]
# PAYLOAD is <prefix>/share/win11-gnome: the wrapper finds its own install without WIN11_INSTALL_PREFIX, also in
# another prefix (by hand, or WinPoint's updater, which passes that variable only for a prefix other than ~/.local)
if given is not None and given.parts[-2:] == ("share", "win11-gnome"):
    prefix = absolute_path(str(given.parent.parent), install.t("labelPrefix"))
else:
    prefix = absolute_path(os.environ.get("WIN11_INSTALL_PREFIX", str(Path.home()/".local")), install.t("labelPrefix"))
data = absolute_path(os.environ.get("XDG_DATA_HOME", str(Path.home()/".local/share")), "XDG_DATA_HOME")
config = absolute_path(os.environ.get("XDG_CONFIG_HOME", str(Path.home()/".config")), "XDG_CONFIG_HOME")
cache = install.cache_home()
set_language(explicit or language_of_file(config/"win11-gnome/settings.json"))
payload = prefix/"share/win11-gnome"
if not set(arguments) <= {"--files-only", "--keep-session"} or len(arguments) != len(set(arguments)):
    raise SystemExit(t("usage"))
files_only = "--files-only" in arguments
from_payload = os.path.realpath(tree) == os.path.realpath(payload)
# 0.9.1: not from inside the WinPoint session (XDG_CURRENT_DESKTOP WinPoint:GNOME, install-session.py); the files only
# touch nothing of the session
if not files_only and "winpoint" in os.environ.get("XDG_CURRENT_DESKTOP", "").lower().split(":"):
    raise SystemExit(t("insideWinPoint"))
# D-G74: an uninstall (or an install) stopped halfway may have left no payload but an own old copy next to it and
# its Transaction and stage folders: install.repair, inside the lock as in install.py, puts a lone own old copy back
# in the payload's place (removed below like any payload) and removes the folders, so running it again finishes it.
if not os.path.lexists(payload) and payload.parent.is_dir():
    install.lock_share(payload.parent)
    install.repair(payload, prefix)
# 0.9.1: what 0.9.0's uninstall.sh leaves of a 0.9.1 install: no payload, this prefix's own wrapper (module docstring)
wrapper = prefix/"bin"/install.UNINSTALLER
rest_only = not os.path.lexists(payload) and install.own_uninstaller(wrapper)
if not rest_only and (not (payload/MARKER).is_file() or (payload/MARKER).read_text() != MAGIC):
    if not os.path.lexists(payload):
        hint = root_hint(install.session_script(), install.system_root())
        if hint and sys.stdout.isatty():
            print(hint)
    raise SystemExit(t("nothingRemoved", payload=payload))
# D-G74: not while an install runs (install.py's flock on the payload's parent, or on its nearest folder before a
# first install made it; busy, nothing removed)
install.lock_share(payload.parent)
# 0.9.1: everything this needs is loaded now (it may run from the payload it removes): install-session.py too
session = install.session_script()
system = install.system_root()
# the one root removal first (install-session.py --remove: the eight root files that are there, one password)
if not files_only and "--keep-session" not in arguments and session.removable(system):
    removal = subprocess.run([sys.executable, "-E", "-s", "-B", str(Path(__file__).resolve().with_name("install-session.py")),
                              "--remove", "--lang", install.LANGUAGE])
    if removal.returncode != 0:
        raise SystemExit(t("rootCancelled"))
# AccountsService: GDM's default again instead of the WinPoint session (install.py preselected it)
if not files_only:
    account = install.accounts_user()
    if account and install.accounts_session(account) == "winpoint":
        install.busctl("call", install.ACCOUNTS[0], account, install.ACCOUNTS_USER, "SetSession", "s", "")
# the extension off in every session: its UUID out of both lists (what gnome-extensions disable did, without a
# running GNOME Shell's D-Bus service and without leaving it in disabled-extensions)
if not files_only:
    install.drop_uuid(("enabled-extensions", "disabled-extensions"))
# without a payload that was all, but the wrapper of this prefix (it names a release's uninstall.sh)
if rest_only:
    if install.own_uninstaller(wrapper):
        wrapper.unlink()
    closing(session, system)
    raise SystemExit(0)
# GNOME's own look back (host/src/gnome_look.rs: only keys that still hold WinPoint's value, only its own CSS blocks),
# while win11-app is still there: for WinPoint's own login session and for an install inside the normal GNOME session
# (--no-session, 0.9.0's mode: no GnomeLook entry, and the extension ran there), after the UUID left
# enabled-extensions, so an extension running in this session no longer writes its values
if not files_only and (prefix/"bin/win11-app").is_file():
    subprocess.run([str(prefix/"bin/win11-app"), "--gnome-look"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
# A previous Shell crash or an unavailable extension service can leave the owned
# logout value behind. Recover before removing the extension's local schema.
schema_dir = data/"gnome-shell/extensions"/UUID/"schemas"
schema = None
if Gio and (schema_dir/"gschemas.compiled").is_file():
    source = Gio.SettingsSchemaSource.new_from_directory(str(schema_dir), Gio.SettingsSchemaSource.get_default(), False)
    schema = source.lookup("org.gnome.shell.extensions.win11-gnome", False)
    if schema:
        settings = Gio.Settings.new_full(schema, None, None)
        raw = settings.get_string("logout-restore")
        if raw:
            try:
                saved = json.loads(raw)
                owned, user = saved["owned"], saved["user"]
                valid = lambda value: isinstance(value, list) and len(value) <= 100 and all(isinstance(item, str) and len(item) <= 2048 for item in value)
                if valid(owned) and (user is None or valid(user)):
                    media = Gio.Settings.new("org.gnome.settings-daemon.plugins.media-keys")
                    if list(media.get_strv("logout")) == owned:
                        if user is None:
                            media.reset("logout")
                        else:
                            media.set_strv("logout", user)
                    settings.reset("logout-restore")
                    Gio.Settings.sync()
            except (KeyError, TypeError, ValueError):
                pass
# Recover only Alt+Tab entries still equal to this extension's temporary value.
if schema and schema.has_key("task-view-restore"):
    raw = settings.get_string("task-view-restore")
    if raw:
        try:
            saved = json.loads(raw)
            wm = Gio.Settings.new("org.gnome.desktop.wm.keybindings")
            keys = wm.props.settings_schema.list_keys()
            for key, entry in saved.items():
                owned, user = entry["owned"], entry["user"]
                valid = lambda value: isinstance(value, list) and len(value) <= 100 and all(isinstance(item, str) and len(item) <= 2048 for item in value)
                if key in keys and valid(owned) and (user is None or valid(user)) and list(wm.get_strv(key)) == owned:
                    if user is None:
                        wm.reset(key)
                    else:
                        wm.set_strv(key, user)
            settings.reset("task-view-restore")
            Gio.Settings.sync()
        except (AttributeError, KeyError, TypeError, ValueError):
            pass
icon_bundle.unregister_aliases(payload, data)
file_portal.unregister(payload, prefix, data, config)
for binary in (*BINS, *LEGACY_BINS):  # D-G70: an earlier version's WebKit Start and desktop as well
    (prefix/"bin"/binary).unlink(missing_ok=True)
install.remove_new_copies(prefix)       # what a killed install left of them and of the wrapper
for name in (*ENTRIES, *LEGACY_ENTRIES):
    (data/"applications"/f"io.github.win11shell.{name}.desktop").unlink(missing_ok=True)
drop_scheme_default(config)
# GW7: the post-upgrade check's autostart entry (install.py AUTOSTART) and the GNOME look's (GNOME_LOOK); every other
# autostart entry stays
for name in (install.AUTOSTART, install.GNOME_LOOK):
    autostart = config/"autostart"/name
    if autostart.is_symlink() or autostart.is_file():
        autostart.unlink()
extension = data/"gnome-shell/extensions"/UUID
# Every top-level file of the own extension folder. A file that this version does not ship (the listing of
# extension/ next to this script, minus the schemas, contracts C7) and that 3b898e6 did not remove either is
# named as it goes (ge1c-review m3: nothing goes without a trace). From the payload (winpoint-uninstall), which has no
# extension/, the installed folder is that listing.
SHIPPED = {"extension.js", "panelBridge.js", "shortcutBridge.js", "desktopBridge.js", "notificationsBridge.js",
           "frostedBridge.js", "metadata.json", "stylesheet.css", "start.svg"}
own_tree = extension if from_payload else tree/"extension"
if own_tree.is_dir():
    SHIPPED |= {entry.name for entry in own_tree.iterdir() if entry.is_file()}
if extension.is_dir() and not extension.is_symlink():
    for entry in sorted(extension.iterdir()):
        if entry.is_file() and not entry.is_symlink():
            entry.unlink()
            if entry.name not in SHIPPED:
                print(install.t("staleRemoved", path=entry))
for name in ("org.gnome.shell.extensions.win11-gnome.gschema.xml", "gschemas.compiled"):
    (extension/"schemas"/name).unlink(missing_ok=True)
try:
    (extension/"schemas").rmdir()
except OSError:
    pass
try:
    extension.rmdir()
except OSError:
    pass
# The WebKit apps' storage of an earlier version (install.remove_webkit_state, its exact list); also with --files-only
install.remove_webkit_state(data, cache, install.runtime_dir(cache))
# D-G74a: WinPoint's update check (last result, owner lock, the update's download); also with --files-only, and not
# while job.json says an update's installer still runs (D-G74)
install.remove_winpoint_cache(cache)
install.rescue_calendar(payload, data)
# its own running scripts last: the wrapper (only ours), then the payload with its scripts/, renamed aside first
# (D-G74), as everywhere: a Ctrl+C or a closed terminal halfway leaves a Transaction folder, which the next install or
# uninstall removes, and never a payload without its marker that every installer refuses (notOwned)
if install.own_uninstaller(wrapper):
    wrapper.unlink()
install.discard(payload)
# D-G74: what a killed install left next to the payload (its old payload, stage and Transaction folders, all ours)
previous = payload.with_name(install.PREVIOUS)
if install.own_entry(previous, stat.S_ISDIR) and install.owned(previous):
    install.discard(previous)
for leftover in install.leftovers(payload.parent):
    shutil.rmtree(leftover, ignore_errors=True)
if shutil.which("update-desktop-database") and (data/"applications").is_dir():
    subprocess.run(["update-desktop-database", str(data/"applications")], check=True)
# The root files stay with --keep-session or --files-only (or when they were installed by hand meanwhile): one hint
# line on a terminal (root_hint), then the closing line.
closing(session, system)
