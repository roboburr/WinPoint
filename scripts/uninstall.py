#!/usr/bin/env python3
"""Remove only the marked installation; preserve personal settings and original Win11Shell.

Usage: scripts/uninstall.sh [--files-only] [--lang nl|en]. Messages: --lang, else Win11's language (C1)."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import install
from install import MAGIC, MARKER, UUID, BINS, ENTRIES, LEGACY_BINS, LEGACY_ENTRIES, LANGUAGES, absolute_path, language_of_file, set_language
import icon_bundle
import file_portal

# i18n: pairs (tools/i18n/check_pairs.py); inline, as these texts appear when something is broken (contracts C3).
T = {
    "nothingRemoved": {"nl": "Niets verwijderd: {payload} bevat geen installatiemarker van dit project.", "en": "Nothing removed: {payload} has no installation marker of this project."},
    "usage": {"nl": "Gebruik: scripts/uninstall.sh [--files-only] [--lang nl|en]", "en": "Usage: scripts/uninstall.sh [--files-only] [--lang nl|en]"},
    "sessionHint": {"nl": "WinPoint staat nog als keuze bij het inloggen. Weghalen: {command}", "en": "WinPoint is still a choice at the login screen. To remove it: {command}"},
    "removed": {"nl": "Win11 voor GNOME verwijderd. Je instellingen en oorspronkelijke Win11Shell zijn behouden.", "en": "Win11 for GNOME removed. Your settings and the original Win11Shell were kept."},
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
prefix = absolute_path(os.environ.get("WIN11_INSTALL_PREFIX", str(Path.home()/".local")), install.t("labelPrefix"))
data = absolute_path(os.environ.get("XDG_DATA_HOME", str(Path.home()/".local/share")), "XDG_DATA_HOME")
config = absolute_path(os.environ.get("XDG_CONFIG_HOME", str(Path.home()/".config")), "XDG_CONFIG_HOME")
cache = install.cache_home()
set_language(explicit or language_of_file(config/"win11-gnome/settings.json"))
payload = prefix/"share/win11-gnome"
if not (payload/MARKER).is_file() or (payload/MARKER).read_text() != MAGIC:
    raise SystemExit(t("nothingRemoved", payload=payload))
if arguments not in ([], ["--files-only"]):
    raise SystemExit(t("usage"))
# WinPoint's own login session (install-session.py): GNOME's own look back now, while win11-app is still there
if "--files-only" not in arguments and (prefix/"bin/win11-app").is_file() and (config/"autostart"/install.GNOME_LOOK).is_file():
    subprocess.run([str(prefix/"bin/win11-app"), "--gnome-look"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
if "--files-only" not in arguments and shutil.which("gnome-extensions"):
    subprocess.run(["gnome-extensions", "disable", UUID], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
# A previous Shell crash or an unavailable extension service can leave the owned
# logout value behind. Recover before removing the extension's local schema.
schema_dir = data/"gnome-shell/extensions"/UUID/"schemas"
if (schema_dir/"gschemas.compiled").is_file():
    from gi.repository import Gio
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
if (schema_dir/"gschemas.compiled").is_file() and schema and schema.has_key("task-view-restore"):
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
# named as it goes (ge1c-review m3: nothing goes without a trace).
SHIPPED = {"extension.js", "panelBridge.js", "shortcutBridge.js", "desktopBridge.js", "notificationsBridge.js",
           "frostedBridge.js", "metadata.json", "stylesheet.css", "start.svg"}
own_tree = Path(__file__).resolve().parents[1]/"extension"
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
install.rescue_calendar(payload, data)
shutil.rmtree(payload)
if shutil.which("update-desktop-database") and (data/"applications").is_dir():
    subprocess.run(["update-desktop-database", str(data/"applications")], check=True)
print(t("removed"))
# GW5: one hint line on a terminal while the separate system files are still there (install.system_hint)
hint = install.system_hint(True)
if hint and sys.stdout.isatty():
    print(hint)
# The login session's four system files stay until removed on purpose (they need the password)
if install.session_script().installed() and sys.stdout.isatty():
    print(t("sessionHint", command="python3 " + str(Path(__file__).resolve().with_name("install-session.py")) + " --remove"))
