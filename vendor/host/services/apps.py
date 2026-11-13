"""apps.* (SPEC §6.3): installed apps, launching, recent files, default apps.

* ``apps.list``      visible desktop entries (``should_show()``) + our 4 own apps, sorted by name.
                     Names follow the UI language (``settings.language``): inline ``Name[nl]``,
                     then the entry's gettext domain, then a small built-in Dutch table for
                     common GNOME apps, then the plain name.
* ``apps.launch``    Gio launch (dangerous); our own apps go through ``ctx.open_app``.
* ``apps.recent``    ``~/.local/share/recently-used.xbel`` (newest first, existing files only).
* ``apps.clearRecent`` empties that file (dangerous; the old one is kept as ``recently-used.xbel.bak``).
* ``apps.forFile`` / ``apps.defaults`` / ``apps.setDefault`` / ``apps.openUri``.
* event ``apps.changed {}`` (application dirs watched with Gio.FileMonitor, debounced).

Shared helpers used by the other host-services-a modules live here too:
``get_catalog(ctx)`` (app index incl. window-class matching), ``Debouncer``,
``launch_file()``, ``ensure_glib_loop()``.
"""

from __future__ import annotations

import contextlib
import gettext
import logging
import os
import re
import shlex
import threading
import time
import unicodedata
import warnings
import xml.etree.ElementTree as ET
from datetime import datetime
from pathlib import Path
from typing import Any, Callable
from urllib.parse import quote, unquote, urlsplit

from .. import APP_IDS
from ..util import ApiError, atomic_write_bytes, atomic_write_json, atomic_write_text, dry_for, dry_run_result, \
    first_existing_icon, icon_exists, mime_icon_names, own_app, read_json, xdg_config_home, xdg_data_home

log = logging.getLogger("win11shell.apps")

PRIORITY = 20

try:  # PyGObject is a hard requirement of the project, but degrade gracefully anyway
    import gi
    gi.require_version("Gio", "2.0")
    from gi.repository import Gio, GLib
    try:
        gi.require_version("GioUnix", "2.0")
        from gi.repository import GioUnix
        DesktopAppInfo = GioUnix.DesktopAppInfo
    except (ImportError, ValueError, AttributeError):
        DesktopAppInfo = Gio.DesktopAppInfo
except Exception:  # noqa: BLE001
    Gio = GLib = DesktopAppInfo = None  # type: ignore[assignment]


# ---------------------------------------------------------------------------
# Small shared helpers
# ---------------------------------------------------------------------------

class Debouncer:
    """``trigger()`` many times → ``fn()`` once, ``delay`` seconds after the first trigger.

    Thread-safe; ``fn`` runs in a short-lived timer thread.
    """

    def __init__(self, delay: float, fn: Callable[[], None], name: str = "debounce"):
        self.delay = delay
        self.fn = fn
        self.name = name
        self._timer: threading.Timer | None = None
        self._lock = threading.Lock()
        self._closed = False

    def trigger(self) -> None:
        with self._lock:
            if self._closed or self._timer is not None:
                return
            t = threading.Timer(self.delay, self._fire)
            t.daemon = True
            t.name = self.name
            self._timer = t
            t.start()

    def _fire(self) -> None:
        with self._lock:
            self._timer = None
            if self._closed:
                return
        try:
            self.fn()
        except Exception:  # noqa: BLE001
            log.exception("%s faalde (failed)", self.name)

    def close(self) -> None:
        with self._lock:
            self._closed = True
            if self._timer is not None:
                self._timer.cancel()
                self._timer = None


_loop_lock = threading.Lock()
_loop_thread: threading.Thread | None = None


def ensure_glib_loop(ctx, delay: float = 3.0) -> None:
    """D-Bus servers/signals and Gio monitors need a running GLib main loop.

    host/__main__.py runs one on the main thread in every mode; this is a safety net for
    embeddings without one (it checks ``ctx.has_main_loop`` after ``delay`` seconds and only
    then iterates the default main context in a daemon thread).
    """
    if GLib is None:
        return

    def check():
        global _loop_thread
        if ctx.has_main_loop or ctx.shutting_down:
            return
        with _loop_lock:
            if _loop_thread is not None:
                return
            loop = GLib.MainLoop()

            def run():
                log.info("Geen GLib-hoofdlus gevonden; eigen lus gestart (started fallback GLib loop)")
                loop.run()

            _loop_thread = threading.Thread(target=run, name="glib-fallback-loop", daemon=True)
            _loop_thread.start()
            ctx.on_shutdown(loop.quit)

    t = threading.Timer(delay, check)
    t.daemon = True
    t.start()


def path_to_uri(path: str) -> str:
    return "file://" + quote(path, safe="/!$&'()*+,;=:@~")


def uri_to_path(uri: str) -> str | None:
    if not isinstance(uri, str) or not uri.startswith("file://"):
        return None
    parts = urlsplit(uri)
    if parts.netloc not in ("", "localhost"):
        return None
    return unquote(parts.path) or None


def _lang(ctx) -> str:
    try:
        v = ctx.settings.get("language", "nl")
    except Exception:  # noqa: BLE001
        v = "nl"
    return v if v in ("nl", "en") else "nl"


def _system_lang() -> str:
    try:
        names = GLib.get_language_names() if GLib else []
    except Exception:  # noqa: BLE001
        names = []
    for n in names:
        n = n.split(".")[0].split("@")[0]
        if n and n != "C":
            return n.split("_")[0].lower()
    return "en"


def sort_key(text: str) -> tuple:
    """Natural, accent- and case-insensitive sort key ("Bestand 2" < "Bestand 10", "é" ~ "e")."""
    t = unicodedata.normalize("NFKD", str(text or "")).casefold()
    t = "".join(ch for ch in t if not unicodedata.combining(ch))
    return tuple((0, int(p), "") if p.isdigit() else (1, 0, p) for p in re.split(r"(\d+)", t) if p)


def _desktop_info_new(app_id: str):
    if DesktopAppInfo is None or not app_id:
        return None
    try:
        with warnings.catch_warnings():
            warnings.simplefilter("ignore")
            return DesktopAppInfo.new(app_id)
    except (TypeError, GLib.Error):  # type: ignore[union-attr]
        return None


def desktop_info_from_file(path: str):
    if DesktopAppInfo is None:
        return None
    try:
        with warnings.catch_warnings():
            warnings.simplefilter("ignore")
            return DesktopAppInfo.new_from_filename(path)
    except (TypeError, GLib.Error):  # type: ignore[union-attr]
        return None


def icon_to_str(icon) -> str:
    """Gio.Icon → themed icon name (first) or absolute path ('' if none)."""
    if icon is None or Gio is None:
        return ""
    try:
        if isinstance(icon, Gio.ThemedIcon):
            names = icon.get_names() or []
            return names[0] if names else ""
        if isinstance(icon, Gio.FileIcon):
            f = icon.get_file()
            return (f.get_path() or "") if f is not None else ""
        s = icon.to_string() or ""
        return s if not s.startswith(". ") else ""
    except Exception:  # noqa: BLE001
        return ""


def dry_skip(what: str, detail: Any = "", ctx: Any = None) -> dict | None:
    """In --dry-run (of ``ctx``, or the process default): log and return the dry-run result (else
    None). Used where only *part* of a method is dangerous (our own app windows still open)."""
    if dry_for(ctx):
        log.info("[dry-run] %s %s overgeslagen (skipped)", what, str(detail)[:200])
        return dry_run_result()
    return None


def launch_context():
    """Gio.AppLaunchContext without our private environment (token, test overrides) and with the user's own
    WebKit/NVIDIA/GSK renderer switches — exactly as they were before Win11 Shell started, never one our GPU code
    set or an older host leaked (util.user_gpu_values; also for Terminal=true apps and the default terminal). An app it
    launches leaves our high-priority scope at once (``launched`` → priority.place_launched): Gio neither calls
    setsid nor makes a scope, so it would otherwise share the shell's cgroup and weight (SPEC §2.1)."""
    lc = Gio.AppLaunchContext()
    for k in list(os.environ):
        if k.startswith("WIN11SHELL_"):
            lc.unsetenv(k)
    _user_gpu_env(lc)
    try:
        lc.connect("launched", _on_launched)
    except Exception:  # noqa: BLE001 - an old GLib without the signal: the scope sweeper still moves it
        log.debug("launched signal unavailable", exc_info=True)
    return lc


def _launched_pid(platform_data) -> int:
    """The pid from GAppLaunchContext::launched's platform data (a{sv} with 'pid'). Pure (tested)."""
    try:
        data = platform_data.unpack() if hasattr(platform_data, "unpack") else platform_data
        pid = int((data or {}).get("pid") or 0)
    except (TypeError, ValueError, AttributeError):
        return 0
    return pid if pid > 1 else 0


def _on_launched(_lc, info, platform_data) -> None:
    pid = _launched_pid(platform_data)
    if not pid:
        return
    app_id = ""
    with contextlib.suppress(Exception):
        app_id = info.get_id() or os.path.basename(getattr(info, "get_filename", lambda: "")() or "") or \
            info.get_executable() or ""
    from .. import priority      # a D-Bus call: never on the main loop
    threading.Thread(target=priority.place_launched, args=(pid, app_id or "app"), name="app-scope",
                     daemon=True).start()


def _user_gpu_env(lc) -> None:
    """The launch context's environment (ours + its changes) gets the user's own renderer switches only."""
    from ..util import USER_GPU_KEYS, user_gpu_values
    want = user_gpu_values()
    for k in USER_GPU_KEYS:
        if k in want:
            if os.environ.get(k) != want[k]:
                lc.setenv(k, want[k])
        elif k in os.environ:
            lc.unsetenv(k)


# ---------------------------------------------------------------------------
# Own apps (always listed; launching them opens our windows)
# ---------------------------------------------------------------------------

OWN_APPS: dict[str, dict] = {
    "settings": {
        "name": {"nl": "Instellingen", "en": "Settings"},
        "generic": {"nl": "Systeeminstellingen", "en": "System settings"},
        "comment": {"nl": "Pas je pc aan: uiterlijk, geluid, netwerk en meer", "en": "Personalise your PC"},
        "icons": ["win11shell-settings", "preferences-system", "org.gnome.Settings", "preferences-desktop"],
        "categories": ["Settings", "System"],
        "keywords": ["instellingen", "configuratie", "voorkeuren", "settings", "control panel",
                     "configuratiescherm", "personaliseren"],
    },
    "taskmgr": {
        "name": {"nl": "Taakbeheer", "en": "Task Manager"},
        "generic": {"nl": "Systeemmonitor", "en": "System monitor"},
        "comment": {"nl": "Bekijk wat je pc doet en stop apps die vastlopen", "en": "See what your PC is doing"},
        "icons": ["win11shell-taskmgr", "utilities-system-monitor", "org.gnome.SystemMonitor", "gnome-system-monitor"],
        "categories": ["System", "Monitor"],
        "keywords": ["taakbeheer", "processen", "prestaties", "task manager", "cpu", "geheugen", "opstarten"],
    },
    "explorer": {
        "name": {"nl": "Verkenner", "en": "File Explorer"},
        "generic": {"nl": "Bestandsbeheer", "en": "File manager"},
        "comment": {"nl": "Je bestanden en mappen bekijken en ordenen", "en": "Browse and organise your files"},
        "icons": ["win11shell-explorer", "system-file-manager", "org.gnome.Nautilus", "folder"],
        "categories": ["System", "FileManager", "Utility"],
        "keywords": ["verkenner", "bestanden", "mappen", "explorer", "files", "folders", "bestandsbeheer"],
    },
    "alles": {
        "name": {"nl": "Alles", "en": "Everything"},
        "generic": {"nl": "Bestanden zoeken", "en": "File search"},
        "comment": {"nl": "Vind elk bestand op je pc in een oogwenk", "en": "Find any file instantly"},
        "icons": ["win11shell-alles", "system-search", "edit-find", "search"],
        "categories": ["Utility", "System"],
        "keywords": ["alles", "zoeken", "everything", "search", "vinden", "bestanden zoeken"],
    },
    "disks": {
        "name": {"nl": "Schijven", "en": "Disks"},
        "generic": {"nl": "Schijfbeheer", "en": "Disk Management"},
        "comment": {"nl": "Bekijk je schijven, koppel ze, formatteer ze en verwijder ze veilig",
                    "en": "View, mount, format and safely remove your disks"},
        "icons": ["win11shell-disks", "org.gnome.DiskUtility", "gnome-disks", "drive-harddisk"],
        "categories": ["System", "Utility"],
        "keywords": ["schijven", "schijfbeheer", "disk management", "partities", "formatteren", "usb-stick",
                     "harde schijf", "ssd", "volumes", "smart", "gnome-disks"],
    },
}

_own_icon_cache: dict[str, str] = {}


def own_app_icon(app: str) -> str:
    if app not in _own_icon_cache:
        cands = OWN_APPS[app]["icons"]
        _own_icon_cache[app] = first_existing_icon(cands, cands[1]) or cands[1]
    return _own_icon_cache[app]


def _own_app_dict(app: str, lang: str, info=None) -> dict:
    meta = OWN_APPS[app]
    icon = ""
    if info is not None:  # installed by install.sh: prefer its icon when the theme has it
        icon = icon_to_str(info.get_icon())
        if icon and not icon_exists(icon):
            icon = ""
    return {
        "id": f"win11shell-{app}.desktop",
        "name": meta["name"][lang],
        "genericName": meta["generic"][lang],
        "comment": meta["comment"][lang],
        "icon": icon or own_app_icon(app),
        "categories": list(meta["categories"]),
        "keywords": list(meta["keywords"]),
        "noDisplay": False,
        "own": True,
        "added": 0,
        "installedAt": 0,
    }


# Dutch names for common GNOME/Ubuntu apps whose .desktop files carry no inline Dutch name
# (translations normally come from language packs that may not be installed).
NL_NAMES = {
    "org.gnome.Nautilus.desktop": "Bestanden",
    "org.gnome.TextEditor.desktop": "Teksteditor",
    "org.gnome.gedit.desktop": "Teksteditor",
    "org.gnome.Calculator.desktop": "Rekenmachine",
    "org.gnome.Calendar.desktop": "Agenda",
    "org.gnome.clocks.desktop": "Klokken",
    "org.gnome.Snapshot.desktop": "Camera",
    "org.gnome.Loupe.desktop": "Afbeeldingenviewer",
    "org.gnome.Papers.desktop": "Documentviewer",
    "org.gnome.Evince.desktop": "Documentviewer",
    "org.gnome.Ptyxis.desktop": "Terminal",
    "org.gnome.Terminal.desktop": "Terminal",
    "org.gnome.Console.desktop": "Console",
    "org.gnome.Logs.desktop": "Logboeken",
    "org.gnome.SystemMonitor.desktop": "Systeemmonitor",
    "gnome-system-monitor.desktop": "Systeemmonitor",
    # org.gnome.DiskUtility.desktop: SAME_NAME_AS_OURS (below), in every language and on every path
    "org.gnome.baobab.desktop": "Schijfgebruik",
    "org.gnome.Characters.desktop": "Tekens",
    "org.gnome.font-viewer.desktop": "Lettertypen",
    "org.gnome.Weather.desktop": "Weer",
    "org.gnome.Maps.desktop": "Kaarten",
    "org.gnome.Contacts.desktop": "Contacten",
    "org.gnome.FileRoller.desktop": "Archiefbeheer",
    "org.gnome.Totem.desktop": "Video's",
    "org.gnome.Showtime.desktop": "Video's",
    "org.gnome.Decibels.desktop": "Audiospeler",
    "org.gnome.Music.desktop": "Muziek",
    "org.gnome.Photos.desktop": "Foto's",
    "org.gnome.Screenshot.desktop": "Schermafdruk",
    "org.gnome.seahorse.Application.desktop": "Wachtwoorden en sleutels",
    "org.gnome.Extensions.desktop": "Uitbreidingen",
    "org.gnome.Settings.desktop": "GNOME-instellingen",
    "gnome-control-center.desktop": "GNOME-instellingen",
    "org.gnome.Software.desktop": "Software",
    "org.gnome.Tour.desktop": "Rondleiding",
    "org.gnome.Connections.desktop": "Verbindingen",
    "org.gnome.Boxes.desktop": "Boxes",
    "org.gnome.SimpleScan.desktop": "Documentscanner",
    "simple-scan.desktop": "Documentscanner",
    "org.gnome.Yelp.desktop": "Hulp",
    "yelp.desktop": "Hulp",
    "org.gnome.PowerStats.desktop": "Energiestatistieken",
    "org.gnome.eog.desktop": "Afbeeldingenviewer",
    "eog.desktop": "Afbeeldingenviewer",
    "gnome-language-selector.desktop": "Taalondersteuning",
    "software-properties-gtk.desktop": "Software en updates",
    "update-manager.desktop": "Software-updater",
    "firmware-updater_firmware-updater.desktop": "Firmware-updater",
    "snap-store_snap-store.desktop": "App Center",
    "org.gnome.Epiphany.desktop": "Web",
    "org.gnome.Geary.desktop": "Geary (e-mail)",
    "nm-connection-editor.desktop": "Netwerkverbindingen",
    "gnome-disk-image-mounter.desktop": "Schijfkopie koppelen",
    "org.gnome.Settings-sharing-panel.desktop": "Delen",
    "info.desktop": "Info",
    "vim.desktop": "Vim",
    "htop.desktop": "Htop",
}

# Apps called exactly like one of OUR apps (OWN_APPS, always listed): with a Dutch install + language pack GNOME Disks
# is "Schijven", in English "Disks" — Start, search and the app list would show two. They always get these names,
# whatever gave the name (inline Name[xx], a language pack's gettext domain, NL_NAMES, Gio in the system language).
SAME_NAME_AS_OURS = {
    "org.gnome.DiskUtility.desktop": {"nl": "Schijven (GNOME)", "en": "Disks (GNOME)"},
}


# Short Dutch descriptions for common apps whose .desktop file has no Dutch Comment (Ubuntu keeps those
# in language packs that may not be installed). Untranslated English descriptions are not shown in Dutch.
NL_COMMENTS = {
    "org.gnome.Loupe.desktop": "Afbeeldingen bekijken en bewerken",
    "org.gnome.Calendar.desktop": "Je agenda's bekijken en bijhouden",
    "org.gnome.FileRoller.desktop": "Archieven (zoals zip-bestanden) maken en uitpakken",
    "org.gnome.DejaDup.desktop": "Maak reservekopieën, zodat je nooit bestanden kwijtraakt",
    "org.gnome.Nautilus.desktop": "Je bestanden en mappen bekijken en ordenen",
    "simple-scan.desktop": "Maak een digitale kopie van je foto's en documenten",
    "org.gnome.Papers.desktop": "Documenten en pdf-bestanden bekijken",
    "org.gnome.Evince.desktop": "Documenten en pdf-bestanden bekijken",
    "org.gnome.Yelp.desktop": "Hulp en handleidingen",
    "org.gnome.clocks.desktop": "Wereldklokken, wekkers, stopwatch en timer",
    "org.gnome.font-viewer.desktop": "De lettertypen op je computer bekijken",
    "org.gnome.Logs.desktop": "Gebeurtenissen van het systeem bekijken",
    "nm-connection-editor.desktop": "Je netwerkverbindingen beheren",
    "org.gnome.Calculator.desktop": "Rekenen: gewoon, wetenschappelijk of financieel",
    "org.gnome.Rhythmbox3.desktop": "Je muziek afspelen en ordenen",
    "org.gnome.baobab.desktop": "Bekijk hoe groot mappen zijn en hoeveel ruimte er vrij is",
    "org.gnome.DiskUtility.desktop": "Schijven en USB-sticks beheren",
    "org.gnome.Shotwell.desktop": "Je foto's ordenen",
    "update-manager.desktop": "Beschikbare updates bekijken en installeren",
    "usb-creator-gtk.desktop": "Een opstart-USB-stick maken",
    "firmware-updater_firmware-updater.desktop": "De firmware van je apparaten bijwerken",
    "org.gnome.TextEditor.desktop": "Tekstbestanden bewerken",
    "org.gnome.Terminal.desktop": "Opdrachten typen in een terminal",
    "org.gnome.Ptyxis.desktop": "Opdrachten typen in een terminal",
    "org.gnome.SystemMonitor.desktop": "Bekijk wat je computer doet",
    "org.gnome.Characters.desktop": "Speciale tekens en emoji zoeken",
    "org.gnome.Contacts.desktop": "Je contactpersonen beheren",
    "org.gnome.Weather.desktop": "Het weerbericht bekijken",
    "org.gnome.Maps.desktop": "Kaarten en routes",
    "org.gnome.Totem.desktop": "Video's afspelen",
    "org.gnome.Showtime.desktop": "Video's afspelen",
    "org.gnome.Decibels.desktop": "Geluidsbestanden afspelen",
    "org.gnome.seahorse.Application.desktop": "Wachtwoorden en sleutels beheren",
    "org.gnome.Software.desktop": "Apps zoeken en installeren",
    "org.gnome.Epiphany.desktop": "Surfen op het web",
    "org.gnome.Geary.desktop": "E-mail lezen en versturen",
    "org.gnome.Extensions.desktop": "Uitbreidingen voor GNOME beheren",
    "org.gnome.Snapshot.desktop": "Foto's en video's maken",
    "gnome-language-selector.desktop": "Talen en taalondersteuning instellen",
    "software-properties-gtk.desktop": "Softwarebronnen en updates instellen",
    "nvidia-settings.desktop": "Instellingen van je NVIDIA-videokaart",
    "org.gnome.Settings.desktop": "De instellingen van GNOME",
}


class _Translator:
    """Name/Comment/Keywords of a desktop entry in a given language."""

    def __init__(self, lang: str):
        self.lang = lang
        self._domains: dict[str, Any] = {}
        self._files: dict[str, tuple[float, Any]] = {}

    def _domain(self, name: str):
        if name not in self._domains:
            t = None
            for d in ("/usr/share/locale-langpack", "/usr/share/locale", "/usr/local/share/locale"):
                try:
                    t = gettext.translation(name, localedir=d, languages=[self.lang])
                    break
                except (OSError, ValueError):
                    continue
            self._domains[name] = t
        return self._domains[name]

    def _keyfile(self, path: str):
        try:
            mt = os.stat(path).st_mtime
        except OSError:
            return None
        hit = self._files.get(path)
        if hit and hit[0] == mt:
            return hit[1]
        kf = GLib.KeyFile()
        try:
            kf.load_from_file(path, GLib.KeyFileFlags.KEEP_TRANSLATIONS)
        except GLib.Error:
            kf = None
        self._files[path] = (mt, kf)
        return kf

    def get(self, path: str | None, key: str, fallback: str) -> str:
        if not path or GLib is None:
            return fallback
        kf = self._keyfile(path)
        if kf is None:
            return fallback
        g = "Desktop Entry"
        try:
            raw = kf.get_string(g, key)
        except GLib.Error:
            return fallback
        try:
            loc = kf.get_locale_string(g, key, self.lang)
        except GLib.Error:
            loc = raw
        if loc and loc != raw:
            return loc
        for dk in ("X-GNOME-Gettext-Domain", "X-Ubuntu-Gettext-Domain", "X-Gettext-Domain"):
            try:
                dom = kf.get_string(g, dk)
            except GLib.Error:
                continue
            t = self._domain(dom) if dom else None
            if t is not None:
                tr = t.gettext(raw)
                if tr and tr != raw:
                    return tr
        return fallback

    def keywords(self, path: str | None) -> list[str]:
        if not path or GLib is None:
            return []
        kf = self._keyfile(path)
        if kf is None:
            return []
        try:
            return [k for k in kf.get_locale_string_list("Desktop Entry", "Keywords", self.lang) if k]
        except GLib.Error:
            return []


# ---------------------------------------------------------------------------
# Catalog: app list + window-class index
# ---------------------------------------------------------------------------

_SNAP_RE = re.compile(r"^([a-z0-9-]+)_([a-z0-9-]+)$")


class AppCatalog:
    def __init__(self, ctx):
        self.ctx = ctx
        self._lock = threading.RLock()
        self._dirty = True
        self._lang = None
        self._apps: list[dict] = []
        self._by_id: dict[str, dict] = {}
        self._index: dict[str, list[tuple[int, str]]] = {}
        self._translator: _Translator | None = None
        self._built_at = 0.0

    def invalidate(self) -> None:
        with self._lock:
            self._dirty = True

    # -- building -----------------------------------------------------------------
    def _ensure(self) -> None:
        lang = _lang(self.ctx)
        with self._lock:
            if not self._dirty and lang == self._lang:
                return
            self._build(lang)

    def _build(self, lang: str) -> None:
        t0 = time.monotonic()
        translate = lang != _system_lang()
        if translate and (self._translator is None or self._translator.lang != lang):
            self._translator = _Translator(lang)
        tr = self._translator if translate else None
        apps: list[dict] = []
        by_id: dict[str, dict] = {}
        index: dict[str, list[tuple[int, str]]] = {}

        def add_key(key: str | None, prio: int, app_id: str):
            if not key:
                return
            index.setdefault(key.lower(), []).append((prio, app_id))

        # First-seen registry: the .desktop mtime changes on every snap refresh / apt upgrade, so it cannot
        # tell "Onlangs toegevoegd". Apps present on the first run count as old (0).
        seen_file = Path(self.ctx.config_dir) / "apps-seen.json"
        seen = read_json(seen_file, None)
        first_run = not isinstance(seen, dict)
        if first_run:
            seen = {}
        seen = {k: v for k, v in seen.items() if isinstance(k, str) and isinstance(v, int)}
        seen_dirty = first_run

        infos = []
        if Gio is not None:
            try:
                with warnings.catch_warnings():
                    warnings.simplefilter("ignore")
                    infos = Gio.AppInfo.get_all()
            except Exception:  # noqa: BLE001
                log.exception("Kan apps niet lezen (cannot list apps)")
        own_installed = {}
        for info in infos:
            try:
                app_id = info.get_id()
            except Exception:  # noqa: BLE001
                continue
            if not app_id:
                continue
            own = own_app(app_id)
            if own and app_id == f"win11shell-{own}.desktop":
                own_installed[own] = info
                continue
            try:
                visible = bool(info.should_show())
            except Exception:  # noqa: BLE001
                visible = False
            base = app_id[:-8] if app_id.endswith(".desktop") else app_id
            path = None
            try:
                path = info.get_filename()
            except Exception:  # noqa: BLE001
                pass
            # window-class index (hidden apps too, but visible ones win)
            p = 0 if visible else 10
            wm = None
            try:
                wm = info.get_startup_wm_class()
            except Exception:  # noqa: BLE001
                pass
            add_key(wm, p + 0, app_id)
            add_key(base, p + 1, app_id)
            m = _SNAP_RE.match(base.lower())
            if m:
                add_key(m.group(2), p + 2, app_id)
                add_key(m.group(1), p + 3, app_id)
            if "." in base:
                add_key(base.rsplit(".", 1)[-1], p + 4, app_id)
            try:
                exe = info.get_executable() or ""
            except Exception:  # noqa: BLE001
                exe = ""
            if exe:
                add_key(os.path.basename(exe), p + 5, app_id)
            icon = icon_to_str(info.get_icon())
            if icon and not icon.startswith("/"):
                add_key(icon, p + 6, app_id)
            name = info.get_name() or base
            add_key(name, p + 7, app_id)
            if not visible:
                by_id[app_id] = {"id": app_id, "name": name, "icon": icon, "visible": False}
                continue
            generic = ""
            comment = ""
            try:
                generic = info.get_generic_name() or ""
            except Exception:  # noqa: BLE001
                pass
            try:
                comment = info.get_description() or ""
            except Exception:  # noqa: BLE001
                pass
            try:
                keywords = list(info.get_keywords() or [])
            except Exception:  # noqa: BLE001
                keywords = []
            local_name = name
            if tr is not None:
                local_name = tr.get(path, "Name", "")
                if not local_name and lang == "nl":
                    local_name = NL_NAMES.get(app_id, "")
                local_name = local_name or name
                en_generic = generic
                generic_tr = tr.get(path, "GenericName", "")
                comment_tr = tr.get(path, "Comment", "")
                if lang == "nl":
                    # Never a half-English list: an untranslated description is left out (the UI then shows
                    # the Dutch category); a few common apps get a Dutch one from NL_COMMENTS.
                    generic = generic_tr
                    comment = comment_tr or NL_COMMENTS.get(app_id, "")
                else:
                    generic = generic_tr or generic
                    comment = comment_tr or comment
                extra = tr.keywords(path)
                keywords = extra + [k for k in keywords if k not in extra]
                for k in (name, en_generic):          # the English words still find the app
                    if k and k != local_name and k not in keywords:
                        keywords.append(k)
            twin = SAME_NAME_AS_OURS.get(app_id)
            if twin and lang in twin:
                for k in (local_name, name):          # "Schijven" / "Disks" still finds it
                    if k and k != twin[lang] and k not in keywords:
                        keywords.append(k)
                local_name = twin[lang]
            try:
                cats = [c for c in (info.get_categories() or "").split(";") if c]
            except Exception:  # noqa: BLE001
                cats = []
            modified = 0
            if path:
                try:
                    modified = int(os.stat(path).st_mtime * 1000)
                except OSError:
                    pass
            added = seen.get(app_id)
            if added is None:            # first time we see this app: that is (about) when it was installed
                added = seen[app_id] = 0 if first_run else int(time.time() * 1000)
                seen_dirty = True
            d = {
                "id": app_id, "name": local_name, "genericName": generic, "comment": comment,
                "icon": icon or "application-x-executable", "categories": cats, "keywords": keywords,
                "noDisplay": False, "own": False, "added": added, "installedAt": added, "modified": modified,
            }
            apps.append(d)
            by_id[app_id] = dict(d, visible=True)
        for app in APP_IDS:
            d = _own_app_dict(app, lang, own_installed.get(app))
            apps.append(d)
            by_id[d["id"]] = dict(d, visible=True)
            add_key(f"win11shell-{app}", -1, d["id"])
        apps.sort(key=lambda a: sort_key(a["name"]))
        for v in index.values():
            v.sort()
        if seen_dirty and os.environ.get("WIN11SHELL_NATIVE_APPS") != "1":
            try:
                atomic_write_json(seen_file, seen, pretty=False)
            except OSError as e:
                log.debug("apps-seen.json niet opgeslagen (not saved): %s", e)
        self._apps = apps
        self._by_id = by_id
        self._index = index
        self._lang = lang
        self._dirty = False
        self._built_at = time.time()
        log.debug("Apps geladen (apps loaded): %d in %.0f ms", len(apps), (time.monotonic() - t0) * 1000)

    # -- queries -----------------------------------------------------------------
    def list(self) -> list[dict]:
        self._ensure()
        with self._lock:
            return [dict(a, categories=list(a["categories"]), keywords=list(a["keywords"])) for a in self._apps]

    def get(self, app_id: str) -> dict | None:
        self._ensure()
        with self._lock:
            d = self._by_id.get(app_id)
            return dict(d) if d else None

    def brief(self, app_id: str | None) -> dict | None:
        """{id, name, icon} for an app id (also hidden ones), or None."""
        if not app_id:
            return None
        d = self.get(app_id)
        if d is None:
            info = _desktop_info_new(app_id)
            if info is None:
                return None
            return {"id": app_id, "name": info.get_name() or app_id,
                    "icon": icon_to_str(info.get_icon()) or "application-x-executable"}
        return {"id": d["id"], "name": d["name"], "icon": d.get("icon") or "application-x-executable"}

    def match_window(self, cls: str | None, initial_cls: str | None = None) -> tuple[str | None, str]:
        """Window class → (desktopId | None, icon name/path)."""
        self._ensure()
        cands = []
        for c in (cls, initial_cls):
            if not c:
                continue
            c = c.strip()
            cands.append(c)
            low = c.lower()
            for suffix in ("-bin", "-stable", "-browser", ".bin", "-wrapped"):
                if low.endswith(suffix):
                    cands.append(c[: -len(suffix)])
            if "." in c:
                cands.append(c.rsplit(".", 1)[-1])
        with self._lock:
            best = None
            for c in cands:
                hits = self._index.get(c.lower())
                if hits:
                    if best is None or hits[0][0] < best[0]:
                        best = hits[0]
            if best is not None:
                d = self._by_id.get(best[1]) or {}
                icon = d.get("icon") or ""
                return best[1], icon or _class_icon(cls, initial_cls)
        return None, _class_icon(cls, initial_cls)


def _class_icon(cls: str | None, initial_cls: str | None) -> str:
    for c in (cls, initial_cls):
        if c and icon_exists(c):
            return c
        if c and icon_exists(c.lower()):
            return c.lower()
    return "application-x-executable"


def get_catalog(ctx) -> AppCatalog:
    cat = getattr(ctx, "_apps_catalog", None)
    if cat is None:
        cat = AppCatalog(ctx)
        ctx._apps_catalog = cat
    return cat


# ---------------------------------------------------------------------------
# Launching
# ---------------------------------------------------------------------------

def _clean_uris(uris) -> list[str]:
    if uris is None:
        return []
    if isinstance(uris, str):
        uris = [uris]
    if not isinstance(uris, list) or len(uris) > 500:
        raise ApiError("bad_args", "Ongeldige invoer.")
    out = []
    for u in uris:
        if not isinstance(u, str) or not u or len(u) > 8192 or any(ord(ch) < 32 for ch in u):
            raise ApiError("bad_args", "Ongeldige invoer.")
        if u.startswith("/"):
            u = path_to_uri(os.path.normpath(u))
        elif not re.match(r"^[a-zA-Z][a-zA-Z0-9+.-]*:", u):
            raise ApiError("bad_args", "Ongeldige invoer.")
        out.append(u)
    return out


def _own_params(app: str, uris: list[str]) -> dict:
    if not uris:
        return {}
    if app == "explorer":   # folders open as themselves, files in their folder (selected), more → tabs
        from ..explorer_args import explorer_params
        return explorer_params(uris, how="auto") or {}
    path = uri_to_path(uris[0])
    if app == "alles" and path:
        return {"query": os.path.basename(path)}
    return {}


def application_dirs() -> list[str]:
    """Real paths of the XDG ``applications`` dirs (+ flatpak/snap exports): where installed apps live."""
    dirs = [str(xdg_data_home() / "applications")]
    data_dirs = os.environ.get("XDG_DATA_DIRS") or "/usr/local/share:/usr/share"
    dirs += [os.path.join(d, "applications") for d in data_dirs.split(":") if d and os.path.isabs(d)]
    dirs += ["/var/lib/flatpak/exports/share/applications", "/var/lib/snapd/desktop/applications",
             str(xdg_data_home() / "flatpak/exports/share/applications")]
    out = []
    for d in dirs:
        r = os.path.realpath(d)
        if r not in out:
            out.append(r)
    return out


def in_application_dirs(path: str) -> bool:
    real = os.path.realpath(path)
    for d in application_dirs():
        try:
            if os.path.commonpath([real, d]) == d:
                return True
        except ValueError:
            continue
    return False


def launch_app(ctx, app_id: str, uris: list[str] | None = None) -> dict:
    """Launch a desktop entry (NOT dry-run guarded: callers are dangerous API methods).

    ``app_id`` is a desktop id. An absolute path is only taken as an installed app when it lies in
    an XDG applications dir; any other .desktop file goes through the launcher trust check."""
    uris = uris or []
    own = own_app(app_id)
    if own and str(app_id).removesuffix(".desktop") == f"win11shell-{own}":
        ctx.open_app(own, _own_params(own, uris))
        return {}
    if Gio is None:
        raise ApiError("unsupported", "Apps starten werkt hier niet.")
    if str(app_id).startswith("/") and not in_application_dirs(app_id):
        return launch_desktop_file(ctx, app_id, uris)
    return _launch_installed(ctx, app_id, uris)


def _launch_installed(ctx, app_id: str, uris: list[str], info=None) -> dict:
    skip = dry_skip("apps.launch", app_id, ctx)
    if skip:
        return skip
    if info is None:
        info = _desktop_info_new(app_id) if not str(app_id).startswith("/") else desktop_info_from_file(app_id)
    if info is None:
        raise ApiError("not_found", "Deze app is niet gevonden. Misschien is hij verwijderd.", {"id": app_id})

    def do():
        try:
            ok = info.launch_uris(uris, launch_context())
        except GLib.Error as e:
            log.warning("Starten mislukt (launch failed) %s: %s", app_id, e.message)
            raise ApiError("launch_failed", "Deze app kon niet worden gestart.", e.message) from None
        if not ok:
            raise ApiError("launch_failed", "Deze app kon niet worden gestart.")
        return {}

    log.info("App starten (launching) %s %s", app_id, uris[:3])
    return ctx.run_on_main(do)


def launch_file(ctx, path: str) -> dict:
    """Open a file/folder like a double click (NOT dry-run guarded).

    Folders open in our Verkenner; ``.desktop`` files are launched; everything else goes to
    the default app for its type.
    """
    if os.path.isdir(path):
        ctx.open_app("explorer", {"path": path})
        return {}
    if path.endswith(".desktop") and os.path.isfile(path):
        return launch_desktop_file(ctx, path)
    if Gio is None:
        raise ApiError("unsupported", "Openen werkt hier niet.")
    skip = dry_skip("files.open", path, ctx)
    if skip:
        return skip
    uri = path_to_uri(path)

    def do():
        try:
            Gio.AppInfo.launch_default_for_uri(uri, launch_context())
        except GLib.Error as e:
            log.info("Geen app voor (no app for) %s: %s", path, e.message)
            raise ApiError("no_app", "Er is geen app geïnstalleerd die dit bestand kan openen.", e.message) from None
        return {}

    return ctx.run_on_main(do)


def read_desktop_entry(path: str, lang: str | None = None) -> dict | None:
    """{type, name, icon, url, exec, hidden} of a .desktop file (None if unreadable)."""
    if GLib is None:
        return None
    kf = GLib.KeyFile()
    try:
        kf.load_from_file(path, GLib.KeyFileFlags.KEEP_TRANSLATIONS)
    except (GLib.Error, TypeError):
        return None
    g = "Desktop Entry"

    def s(key, loc=False):
        try:
            if loc:
                return kf.get_locale_string(g, key, lang) if lang else kf.get_locale_string(g, key, None)
            return kf.get_string(g, key)
        except GLib.Error:
            return ""

    def b(key):
        try:
            return kf.get_boolean(g, key)
        except GLib.Error:
            return False

    if not kf.has_group(g):
        return None
    name = s("Name", True) or s("Name")
    if lang == "nl" and name == s("Name"):
        name = NL_NAMES.get(os.path.basename(path), name)
    twin = SAME_NAME_AS_OURS.get(os.path.basename(path))
    if twin and lang in twin:
        name = twin[lang]
    icon = s("Icon")
    if icon and not icon.startswith("/"):
        low = icon.lower()
        for ext in (".png", ".svg", ".xpm"):
            if low.endswith(ext):
                icon = icon[: -len(ext)]
                break
    elif icon and icon.startswith("/") and not os.path.isabs(icon):
        icon = ""
    return {"type": s("Type") or "Application", "name": name, "icon": icon, "url": s("URL"),
            "exec": s("Exec"), "hidden": b("NoDisplay") or b("Hidden")}


# Filesystems without real Unix owners/permissions (every file looks "mine" and executable there):
# a launcher on such a drive is never trusted.
_NO_PERMISSION_FS = {"vfat", "msdos", "exfat", "ntfs", "ntfs3", "fuseblk", "fuse.ntfs-3g", "fuse.exfat", "udf",
                     "iso9660", "hfsplus", "fuse.sshfs", "cifs", "smb3", "9p"}


def fs_type_of(path: str) -> str:
    """Filesystem type of the mount that holds ``path`` ('' if unknown)."""
    real = os.path.realpath(path)
    best, fstype = "", ""
    try:
        with open("/proc/self/mountinfo", encoding="utf-8", errors="replace") as f:
            for line in f:
                parts = line.split()
                if len(parts) < 10 or "-" not in parts:
                    continue
                mp = parts[4].replace("\\040", " ").replace("\\011", "\t").replace("\\134", "\\")
                sep = parts.index("-")
                if len(mp) >= len(best) and (real == mp or real.startswith(mp.rstrip("/") + "/")):
                    best, fstype = mp, parts[sep + 1] if len(parts) > sep + 1 else ""
    except OSError:
        return ""
    return fstype


def _gio_trusted(path: str) -> bool:
    if Gio is None:
        return False
    try:
        info = Gio.File.new_for_path(path).query_info("metadata::trusted", Gio.FileQueryInfoFlags.NONE, None)
        v = info.get_attribute_as_string("metadata::trusted") if info.has_attribute("metadata::trusted") else None
        return (v or "").strip().lower() in ("true", "yes", "1")
    except Exception:  # noqa: BLE001 - no metadata daemon → not trusted
        return False


def launcher_trust(path: str) -> tuple[bool, str]:
    """(trusted, reason) for a .desktop file that is not an installed app.

    Like GNOME's "Starten toestaan": the user must have marked it (Gio ``metadata::trusted``), it must be
    executable and owned by the user, and it may not live on a drive without Unix permissions
    (FAT/exFAT/NTFS, where every file looks executable and owned by the user)."""
    try:
        st = os.stat(path)
    except OSError:
        return False, "missing"
    if not os.path.isfile(path):
        return False, "not_file"
    if fs_type_of(path) in _NO_PERMISSION_FS:
        return False, "filesystem"
    if st.st_uid != os.getuid() or not (st.st_mode & 0o100):
        return False, "not_executable"
    if not _gio_trusted(path):
        return False, "not_marked"
    return True, ""


def installed_copy(path: str):
    """DesktopAppInfo of the installed app this file is a copy of (same desktop id), else None."""
    return _desktop_info_new(os.path.basename(path))


def trust_launcher(path: str) -> dict:
    """Mark a launcher as trusted (metadata::trusted + u+x), like GNOME's "Starten toestaan"."""
    if not path.endswith(".desktop") or not os.path.isfile(path):
        raise ApiError("bad_args", "Dit is geen snelkoppeling.")
    if fs_type_of(path) in _NO_PERMISSION_FS:
        raise ApiError("filesystem", "Snelkoppelingen op deze schijf kunnen uit veiligheid niet worden gestart. "
                                     "Kopieer de snelkoppeling eerst naar je eigen map.")
    st = os.stat(path)
    if st.st_uid != os.getuid():
        raise ApiError("no_access", "Deze snelkoppeling is van iemand anders en kan niet worden vertrouwd.")
    if Gio is None:
        raise ApiError("unsupported", "Dat werkt hier niet.")
    try:
        f = Gio.File.new_for_path(path)
        f.set_attribute_string("metadata::trusted", "true", Gio.FileQueryInfoFlags.NONE, None)
    except Exception as e:  # noqa: BLE001
        raise ApiError("failed", "Vertrouwen instellen lukte niet.", str(e)[:200]) from None
    os.chmod(path, (st.st_mode | 0o100) & 0o7777)
    return {"ok": True, "trusted": launcher_trust(path)[0]}


def launch_desktop_file(ctx, path: str, uris: list[str] | None = None) -> dict:
    uris = uris or []
    base = os.path.basename(path)
    own = own_app(base)
    if own and base == f"win11shell-{own}.desktop":
        ctx.open_app(own, _own_params(own, uris))
        return {}
    if not base.endswith(".desktop") or not os.path.isfile(path):
        raise ApiError("not_found", "Deze app is niet gevonden. Misschien is hij verwijderd.", {"id": path})
    entry = read_desktop_entry(path)
    if entry is None:
        raise ApiError("bad_entry", "Deze snelkoppeling is beschadigd.")
    if entry["type"] == "Link":
        url = entry.get("url") or ""
        return open_uri(ctx, url)
    if entry["type"] != "Application":
        raise ApiError("bad_entry", "Deze snelkoppeling kan niet worden geopend.")
    # Safety: a .desktop file can run any command. Like GNOME, only start it when the user marked it as
    # trusted ("Starten toestaan"), or when it is a copy of an installed app (then the installed one runs).
    if in_application_dirs(path):
        return _launch_installed(ctx, path, uris)
    trusted, reason = launcher_trust(path)
    if not trusted:
        installed = installed_copy(path)
        if installed is not None:
            return _launch_installed(ctx, base, uris, info=installed)
        msg = ("Snelkoppelingen op deze schijf worden uit veiligheid niet gestart." if reason == "filesystem" else
               "Deze snelkoppeling wordt (nog) niet vertrouwd, dus starten is uit veiligheid geblokkeerd. "
               "Vertrouw je de bron? Kies dan ‘Starten toestaan’.")
        raise ApiError("untrusted", msg, {"path": path, "name": entry.get("name") or base,
                                          "exec": (entry.get("exec") or "")[:500], "reason": reason,
                                          "canTrust": reason != "filesystem"})
    return _launch_installed(ctx, path, uris, info=desktop_info_from_file(path))


_URI_SCHEMES = ("http", "https", "mailto", "file")


def open_uri(ctx, uri: str) -> dict:
    """Open http(s)/mailto/file URIs with the default app (NOT dry-run guarded)."""
    if not isinstance(uri, str) or not uri or len(uri) > 8192 or any(ord(c) < 32 or c == " " for c in uri.strip()):
        raise ApiError("bad_uri", "Deze link kan niet worden geopend.")
    uri = uri.strip()
    scheme = uri.split(":", 1)[0].lower() if ":" in uri else ""
    if scheme not in _URI_SCHEMES:
        raise ApiError("bad_uri", "Dit soort link kan niet worden geopend.", {"scheme": scheme})
    if scheme in ("http", "https"):
        parts = urlsplit(uri)
        if not parts.netloc:
            raise ApiError("bad_uri", "Deze link is ongeldig.")
    if scheme == "file":
        path = uri_to_path(uri)
        if not path or not os.path.exists(path):
            raise ApiError("not_found", "Dit bestand bestaat niet (meer).")
        return launch_file(ctx, path)
    if Gio is None:
        raise ApiError("unsupported", "Openen werkt hier niet.")
    skip = dry_skip("apps.openUri", scheme, ctx)
    if skip:
        return skip

    def do():
        try:
            Gio.AppInfo.launch_default_for_uri(uri, launch_context())
        except GLib.Error as e:
            raise ApiError("no_app", "Er is geen app om deze link te openen.", e.message) from None
        return {}

    log.info("Link openen (opening uri) %s://…", scheme)
    return ctx.run_on_main(do)


# ---------------------------------------------------------------------------
# Recent files (recently-used.xbel)
# ---------------------------------------------------------------------------

_NS_MIME = "{http://www.freedesktop.org/standards/shared-mime-info}"
_NS_BM = "{http://www.freedesktop.org/standards/desktop-bookmarks}"


def recent_file_path() -> Path:
    override = os.environ.get("WIN11SHELL_RECENT_FILE")
    if override:
        return Path(override)
    return xdg_data_home() / "recently-used.xbel"


def _iso_ms(text: str | None) -> int:
    if not text:
        return 0
    try:
        return int(datetime.fromisoformat(text.replace("Z", "+00:00")).timestamp() * 1000)
    except ValueError:
        return 0


def parse_xbel(path: str | os.PathLike) -> list[dict]:
    """All file:// bookmarks, newest first: [{path, uri, mime, modified, app}] (no existence check)."""
    try:
        tree = ET.parse(path)
    except (OSError, ET.ParseError) as e:
        log.debug("recently-used.xbel onleesbaar (unreadable): %s", e)
        return []
    out: dict[str, dict] = {}
    for bm in tree.getroot().iter("bookmark"):
        href = bm.get("href") or ""
        p = uri_to_path(href)
        if not p:
            continue
        t = max(_iso_ms(bm.get("modified")), _iso_ms(bm.get("visited")), _iso_ms(bm.get("added")))
        mime = ""
        app = ""
        for el in bm.iter():
            if el.tag == _NS_MIME + "mime-type":
                mime = el.get("type") or mime
            elif el.tag == _NS_BM + "application":
                at = _iso_ms(el.get("modified"))
                if at >= t:
                    t = at
                app = el.get("name") or app
        prev = out.get(p)
        if prev is None or t > prev["modified"]:
            out[p] = {"path": p, "uri": href, "mime": mime, "modified": t, "app": app}
    return sorted(out.values(), key=lambda r: r["modified"], reverse=True)


class RecentFiles:
    def __init__(self):
        self._lock = threading.Lock()
        self._key = None
        self._items: list[dict] = []

    def entries(self) -> list[dict]:
        p = recent_file_path()
        try:
            st = p.stat()
            key = (str(p), st.st_mtime_ns, st.st_size)
        except OSError:
            return []
        with self._lock:
            if key != self._key:
                self._items = parse_xbel(p)
                self._key = key
            return self._items

    def recent(self, limit: int) -> list[dict]:
        out = []
        checked = 0
        trash = str(xdg_data_home() / "Trash") + "/"
        for r in self.entries():
            if len(out) >= limit or checked > 2000:
                break
            checked += 1
            p = r["path"]
            if p.startswith(trash):
                continue
            try:
                st = os.stat(p)
            except OSError:
                continue
            is_dir = os.path.isdir(p)
            mime = "inode/directory" if is_dir else (r["mime"] or _guess_type(p))
            icon = "folder" if is_dir else first_existing_icon(mime_icon_names(mime), "text-x-generic")
            out.append({"path": p, "uri": r["uri"], "name": os.path.basename(p) or p, "mime": mime,
                        "icon": icon, "modified": r["modified"] or int(st.st_mtime * 1000),
                        "isDir": is_dir, "app": r["app"]})
        return out


EMPTY_XBEL = """<?xml version="1.0" encoding="UTF-8"?>
<xbel version="1.0"
      xmlns:bookmark="http://www.freedesktop.org/standards/desktop-bookmarks"
      xmlns:mime="http://www.freedesktop.org/standards/shared-mime-info"
>
</xbel>
"""


def clear_recent_file(path: Path) -> dict:
    """Replace recently-used.xbel by an empty (valid) one; the previous file becomes ``<name>.bak`` (0600)."""
    count = len(parse_xbel(path)) if path.exists() else 0
    backup = None
    if path.exists():
        backup = path.with_name(path.name + ".bak")
        try:
            data = path.read_bytes()
            atomic_write_bytes(backup, data, mode=0o600)
        except OSError as e:
            log.warning("Reservekopie van recente bestanden mislukt (backup failed): %s", e)
            raise ApiError("failed", "De geschiedenis kon niet worden gewist.") from None
    try:
        atomic_write_text(path, EMPTY_XBEL, mode=0o600)
    except OSError as e:
        log.warning("Recente bestanden niet gewist (not cleared): %s", e)
        raise ApiError("failed", "De geschiedenis kon niet worden gewist.") from None
    log.info("Geschiedenis van recente bestanden gewist (recent files cleared): %d items", count)
    return {"cleared": count, "backup": str(backup) if backup else None}


def _guess_type(path: str) -> str:
    if Gio is None:
        return "application/octet-stream"
    try:
        ct, _ = Gio.content_type_guess(path, None)
        mime = Gio.content_type_get_mime_type(ct) if ct else None
        return mime or ct or "application/octet-stream"
    except Exception:  # noqa: BLE001
        return "application/octet-stream"


# ---------------------------------------------------------------------------
# Default apps
# ---------------------------------------------------------------------------

DEFAULT_KINDS: dict[str, dict] = {
    "browser": {"main": "x-scheme-handler/https",
                "types": ["x-scheme-handler/http", "x-scheme-handler/https", "text/html", "application/xhtml+xml"]},
    "mail": {"main": "x-scheme-handler/mailto", "types": ["x-scheme-handler/mailto"]},
    # folders + Nautilus' saved searches (opened as a file: its folder with it selected), like Nautilus registers
    "files": {"main": "inode/directory", "types": ["inode/directory", "application/x-gnome-saved-search"]},
    "music": {"main": "audio/mpeg",
              "types": ["audio/mpeg", "audio/mp4", "audio/flac", "audio/x-flac", "audio/ogg", "audio/x-vorbis+ogg",
                        "audio/vnd.wave", "audio/x-wav", "audio/aac", "audio/opus", "audio/x-opus+ogg", "audio/webm",
                        "audio/x-m4a", "audio/x-mpegurl"]},
    "video": {"main": "video/mp4",
              "types": ["video/mp4", "video/x-matroska", "video/webm", "video/quicktime", "video/x-msvideo",
                        "video/mpeg", "video/ogg", "video/x-flv", "video/3gpp"]},
    "image": {"main": "image/png",
              "types": ["image/png", "image/jpeg", "image/gif", "image/webp", "image/bmp", "image/tiff",
                        "image/svg+xml", "image/avif", "image/heif", "image/x-icon", "image/jxl"]},
    "text": {"main": "text/plain", "types": ["text/plain", "text/markdown", "text/x-log"]},
    "terminal": {"main": None, "types": []},
}

_TERMINAL_PREFERENCE = [
    "org.gnome.Ptyxis.desktop", "org.gnome.Terminal.desktop", "org.gnome.Console.desktop", "kitty.desktop",
    "com.mitchellh.ghostty.desktop", "Alacritty.desktop", "foot.desktop", "org.wezfurlong.wezterm.desktop",
    "org.kde.konsole.desktop", "xfce4-terminal.desktop", "terminator.desktop", "tilix.desktop",
    "com.gexperts.Tilix.desktop", "xterm.desktop", "debian-xterm.desktop",
]


def _terminal_lists() -> list[Path]:
    desktops = [d.lower() for d in os.environ.get("XDG_CURRENT_DESKTOP", "").split(":") if d]
    dirs = [xdg_config_home()] + [Path(d) for d in (os.environ.get("XDG_CONFIG_DIRS") or "/etc/xdg").split(":") if d]
    out = []
    for d in dirs:
        for de in desktops:
            out.append(d / f"{de}-xdg-terminals.list")
        out.append(d / "xdg-terminals.list")
    return out


def _all_terminals() -> list:
    if Gio is None:
        return []
    with warnings.catch_warnings():
        warnings.simplefilter("ignore")
        apps = [a for a in Gio.AppInfo.get_all() if "TerminalEmulator" in (a.get_categories() or "").split(";")]
    return apps


def default_terminal():
    for f in _terminal_lists():
        try:
            lines = f.read_text(encoding="utf-8", errors="replace").splitlines()
        except OSError:
            continue
        for line in lines:
            line = line.strip()
            if not line or line.startswith("#") or line.startswith("-"):
                continue
            app_id = line.lstrip("+").split(":", 1)[0]
            info = _desktop_info_new(app_id)
            if info is not None:
                return info
    terms = {a.get_id(): a for a in _all_terminals()}
    for t in _TERMINAL_PREFERENCE:
        if t in terms:
            return terms[t]
    for t in _TERMINAL_PREFERENCE:
        info = _desktop_info_new(t)
        if info is not None:
            return info
    return next(iter(terms.values()), None)


def _default_for(kind: str):
    if Gio is None:
        return None
    if kind == "terminal":
        return default_terminal()
    spec = DEFAULT_KINDS[kind]
    with warnings.catch_warnings():
        warnings.simplefilter("ignore")
        if kind == "browser":
            return (Gio.AppInfo.get_default_for_uri_scheme("https") or Gio.AppInfo.get_default_for_uri_scheme("http")
                    or Gio.AppInfo.get_default_for_type("text/html", False))
        if kind == "mail":
            return Gio.AppInfo.get_default_for_uri_scheme("mailto")
        for t in [spec["main"]] + spec["types"]:
            info = Gio.AppInfo.get_default_for_type(t, False)
            if info is not None:
                return info
    return None


def _candidates(kind: str) -> list:
    if Gio is None:
        return []
    if kind == "terminal":
        return _all_terminals()
    spec = DEFAULT_KINDS[kind]
    seen, out = set(), []
    with warnings.catch_warnings():
        warnings.simplefilter("ignore")
        for t in [spec["main"]] + spec["types"][:3]:
            for a in Gio.AppInfo.get_all_for_type(t):
                if a.get_id() not in seen and a.should_show():
                    seen.add(a.get_id())
                    out.append(a)
    if kind == "files" and OWN_FILES_ID not in seen:
        # our Verkenner, also when its installed entry predates MimeType=inode/directory (old install.sh)
        own = _desktop_info_new(OWN_FILES_ID)
        if own is not None:
            out.insert(0, own)
    return out


#: Our File Explorer's desktop id (install.sh writes it to ~/.local/share/applications).
OWN_FILES_ID = "win11shell-explorer.desktop"


def _desktop_mimeapps() -> list[str]:
    """Existing ``$XDG_CONFIG_HOME/<desktop>-mimeapps.list`` files: they win over ``mimeapps.list``."""
    out = []
    for d in os.environ.get("XDG_CURRENT_DESKTOP", "").split(":"):
        if d:
            f = xdg_config_home() / f"{d.lower()}-mimeapps.list"
            if f.is_file():
                out.append(str(f))
    return out


def _drop_default_entry(content_type: str, app_id: str) -> bool:
    """Take ``app_id`` out of ``[Default Applications]`` and ``[Added Associations]`` for ``content_type`` in the user's
    ``mimeapps.list`` (where ``Gio.AppInfo.set_as_default_for_type`` put it; other entries and comments stay). Used
    when the ``files`` default moves from Verkenner to an app that does not handle a secondary type (Nautilus has no
    ``application/x-gnome-saved-search``): the old default must not linger for it. True when the file changed."""
    if GLib is None:
        return False
    f = Path(GLib.get_user_config_dir()) / "mimeapps.list"
    kf = GLib.KeyFile()
    try:
        kf.load_from_file(str(f), GLib.KeyFileFlags.KEEP_COMMENTS | GLib.KeyFileFlags.KEEP_TRANSLATIONS)
    except GLib.Error:
        return False
    changed = False
    for group in ("Default Applications", "Added Associations"):
        try:
            vals = list(kf.get_string_list(group, content_type))
        except GLib.Error:
            continue
        keep = [v for v in vals if v != app_id]
        if keep == vals:
            continue
        changed = True
        if keep:
            kf.set_string_list(group, content_type, keep)
        else:
            kf.remove_key(group, content_type)
    if changed:
        data = kf.to_data()[0]
        atomic_write_text(f, data if data.endswith("\n") else data + "\n", mode=0o644)
    return changed


def _brief_info(cat: AppCatalog, info) -> dict | None:
    if info is None:
        return None
    app_id = info.get_id()
    b = cat.brief(app_id) if app_id else None
    if b:
        return b
    return {"id": app_id, "name": info.get_name(), "icon": icon_to_str(info.get_icon()) or "application-x-executable"}


def _content_type(path: str) -> str:
    if os.path.isdir(path):
        return "inode/directory"
    if Gio is None:
        return "application/octet-stream"
    try:
        info = Gio.File.new_for_path(path).query_info("standard::content-type", Gio.FileQueryInfoFlags.NONE, None)
        ct = info.get_content_type()
        if ct:
            return ct
    except GLib.Error:
        pass
    return _guess_type(path)


# ---------------------------------------------------------------------------
# Service
# ---------------------------------------------------------------------------

def register(api, ctx) -> None:
    cat = get_catalog(ctx)
    recent = RecentFiles()
    state: dict[str, Any] = {"monitors": [], "debounce": None}

    def changed():
        cat.invalidate()
        api.emit("apps.changed", {})

    deb = Debouncer(1.0, changed, name="apps-changed")
    state["debounce"] = deb

    def on_settings(_settings, _path=None, changed_paths=None):
        if changed_paths is None or any(p == "language" or str(p).startswith("language") for p in changed_paths):
            if cat._lang is not None and _lang(ctx) != cat._lang:
                cat.invalidate()
                api.emit("apps.changed", {})

    try:
        ctx.settings.on_change(on_settings)
    except Exception:  # noqa: BLE001
        log.debug("settings.on_change niet beschikbaar (unavailable)")

    # -- methods ------------------------------------------------------------------
    @api.method("apps.list")
    def _list(args):
        """Visible apps (+ our own 4), sorted by name."""
        return cat.list()

    @api.method("apps.launch")
    def _launch(args):
        """Start an app by desktop id (optionally with URIs/paths). Dangerous: external apps are
        not started in --dry-run (our own app windows still open)."""
        app_id = args.get("id")
        # a desktop id is short; an absolute .desktop path may be as long as any path
        if not isinstance(app_id, str) or not app_id or len(app_id) > (4096 if app_id.startswith("/") else 300) \
                or "\0" in app_id:
            raise ApiError("bad_args", "Ongeldige app.")
        return launch_app(ctx, app_id, _clean_uris(args.get("uris")))

    @api.method("apps.recent")
    def _recent(args):
        """Recently used files (newest first, existing files only)."""
        if not ctx.settings.get("privacy.recentFiles", True):
            return []
        try:
            limit = max(1, min(200, int(args.get("limit", 12))))
        except (TypeError, ValueError):
            limit = 12
        return recent.recent(limit)

    @api.method("apps.clearRecent", dangerous=True)
    def _clear_recent(args):
        """Empty the recent-files history (Privacy › "Geschiedenis wissen"); keeps a .bak copy."""
        res = clear_recent_file(recent_file_path())
        api.emit("apps.changed", {})
        return res

    @api.method("apps.forFile")
    def _for_file(args):
        """Apps that can open a file: [{id, name, icon, isDefault}]."""
        path = args.get("path")
        if not isinstance(path, str) or not os.path.isabs(path):
            raise ApiError("bad_args", "Ongeldig pad.")
        if not os.path.exists(path):
            raise ApiError("not_found", "Dit bestand bestaat niet (meer).")
        ct = _content_type(path)
        out: list[dict] = []
        seen: set[str] = set()
        if ct == "inode/directory":
            out.append({"id": "win11shell-explorer.desktop", "name": OWN_APPS["explorer"]["name"][_lang(ctx)],
                        "icon": own_app_icon("explorer"), "isDefault": True})
            seen.add("win11shell-explorer.desktop")
        if Gio is None:
            return out
        with warnings.catch_warnings():
            warnings.simplefilter("ignore")
            default = Gio.AppInfo.get_default_for_type(ct, False)
            alls = Gio.AppInfo.get_all_for_type(ct)
        def_id = default.get_id() if default is not None else None
        ordered = ([default] if default is not None else []) + [a for a in alls if a.get_id() != def_id]
        for a in ordered:
            aid = a.get_id()
            if not aid or aid in seen:
                continue
            if aid != def_id and not a.should_show():
                continue
            seen.add(aid)
            b = _brief_info(cat, a) or {"id": aid, "name": a.get_name(), "icon": ""}
            b["isDefault"] = aid == def_id and ct != "inode/directory"
            out.append(b)
        return out

    @api.method("apps.defaults")
    def _defaults(args):
        """Default apps per kind (+ ``candidates`` per kind with ``{candidates:true}``)."""
        out: dict[str, Any] = {}
        for kind in DEFAULT_KINDS:
            try:
                out[kind] = _brief_info(cat, _default_for(kind))
            except Exception:  # noqa: BLE001
                log.exception("Standaard-app %s onbekend (default app lookup failed)", kind)
                out[kind] = None
        if args.get("candidates"):
            cands = {}
            for kind in DEFAULT_KINDS:
                try:
                    lst = [_brief_info(cat, a) for a in _candidates(kind)]
                    cands[kind] = sorted([b for b in lst if b], key=lambda b: sort_key(b["name"]))
                except Exception:  # noqa: BLE001
                    cands[kind] = []
            out["candidates"] = cands
        return out

    @api.method("apps.setDefault", dangerous=True)
    def _set_default(args):
        """Make ``id`` the default app for ``kind`` (browser, mail, files, music, video, image, text, terminal).

        Written the standard way: ``Gio.AppInfo.set_as_default_for_type`` → ``[Default Applications]`` (and
        ``[Added Associations]``) in ``$XDG_CONFIG_HOME/mimeapps.list``, exactly what ``xdg-mime default`` and GNOME
        Settings do. ``files`` = ``inode/directory`` + ``application/x-gnome-saved-search``; our own Verkenner
        (``win11shell-explorer.desktop``) is a valid choice once install.sh put its entry in the menu. A secondary type
        the new app does not handle is not written; when Verkenner was its default, that entry is removed
        (``cleared``). ``effective`` says whether GIO now really answers with this app (a desktop-specific
        ``<desktop>-mimeapps.list`` wins over ``mimeapps.list``: then ``overriddenBy`` lists those files and
        ``current`` is the app GIO still uses; logged)."""
        kind = args.get("kind")
        app_id = args.get("id")
        if kind not in DEFAULT_KINDS:
            raise ApiError("bad_args", "Onbekend soort standaard-app.")
        if not isinstance(app_id, str) or not app_id or len(app_id) > 300 or "/" in app_id:
            raise ApiError("bad_args", "Ongeldige app.")
        own = own_app(app_id)
        if own is not None:
            app_id = f"win11shell-{own}.desktop"
            if not (kind == "files" and own == "explorer"):
                raise ApiError("bad_args", "Deze app kan hier niet als standaard worden gekozen.")
        info = _desktop_info_new(app_id)
        if info is None:
            if own is not None:
                raise ApiError("not_installed", "Verkenner staat nog niet in het app-menu. Voer install.sh opnieuw "
                                                "uit en probeer het daarna nog eens.", {"id": app_id})
            raise ApiError("not_found", "Deze app is niet gevonden.")
        if kind == "terminal":
            return _set_terminal(app_id)
        spec = DEFAULT_KINDS[kind]
        try:
            supported = set(info.get_supported_types() or [])
        except Exception:  # noqa: BLE001
            supported = set()
        done, skipped = [], []
        for t in spec["types"]:
            if t != spec["main"] and kind not in ("browser",) and supported and t not in supported:
                skipped.append(t)
                continue
            try:
                info.set_as_default_for_type(t)
                done.append(t)
            except GLib.Error as e:
                log.warning("Standaard voor %s niet gezet (not set): %s", t, e.message)
        if not done:
            raise ApiError("failed", "De standaard-app kon niet worden gewijzigd.")
        cleared = []
        if kind == "files" and app_id != OWN_FILES_ID:
            # back from Verkenner to an app without a secondary type: Verkenner must not stay its default
            for t in skipped:
                try:
                    if _drop_default_entry(t, OWN_FILES_ID):
                        cleared.append(t)
                except (OSError, GLib.Error) as e:
                    log.warning("Oude standaard voor %s niet verwijderd (old default not removed): %s", t, e)
        res: dict[str, Any] = {"types": done, "effective": True, "cleared": cleared}
        try:
            with warnings.catch_warnings():
                warnings.simplefilter("ignore")
                now = Gio.AppInfo.get_default_for_type(spec["main"], False) if spec["main"] else None
            if now is not None and now.get_id() != app_id:
                over = _desktop_mimeapps()
                res.update(effective=False, overriddenBy=over, current=now.get_id())
                log.warning("Standaard-app %s is %s geworden, maar %s gaat voor: een bureaubladspecifiek "
                            "mimeapps.list-bestand (%s) wint (overridden by a desktop-specific mimeapps.list)",
                            kind, app_id, now.get_id(), ", ".join(over) or "?")
        except Exception:  # noqa: BLE001
            pass
        log.info("Standaard-app %s → %s (%d types%s)", kind, app_id, len(done),
                 f", {len(cleared)} oude verwijderd (old removed)" if cleared else "")
        return res

    def _set_terminal(app_id: str) -> dict:
        f = xdg_config_home() / "xdg-terminals.list"
        lines = []
        try:
            lines = [l for l in f.read_text(encoding="utf-8").splitlines()
                     if l.strip() and l.strip().lstrip("+").split(":", 1)[0] != app_id]
        except OSError:
            pass
        atomic_write_text(f, "\n".join([app_id] + lines) + "\n", mode=0o644)
        return {"types": ["terminal"]}

    @api.method("apps.openUri")
    def _open_uri(args):
        """Open an http(s)/mailto/file URI with its default app (dry-run guarded inside)."""
        return open_uri(ctx, args.get("uri"))

    ctx._apps_state = state


def start(api, ctx) -> None:
    """Watch the application dirs (Gio monitors live on the main thread)."""
    state = getattr(ctx, "_apps_state", None)
    if state is None or Gio is None:
        return
    ctx.thread(get_catalog(ctx).list, name="apps-prewarm")   # first Start-menu open stays instant
    dirs = [xdg_data_home() / "applications"]
    data_dirs = os.environ.get("XDG_DATA_DIRS") or "/usr/local/share:/usr/share"
    dirs += [Path(d) / "applications" for d in data_dirs.split(":") if d and os.path.isabs(d)]
    seen = set()
    deb: Debouncer = state["debounce"]

    def on_change(_mon, file, _other, event):
        name = file.get_basename() if file is not None else ""
        if name and not name.endswith(".desktop") and event != Gio.FileMonitorEvent.CREATED:
            return
        deb.trigger()

    def setup():
        for d in dirs:
            key = str(d)
            if key in seen or not d.is_dir():
                continue
            seen.add(key)
            try:
                mon = Gio.File.new_for_path(key).monitor_directory(Gio.FileMonitorFlags.NONE, None)
                mon.set_rate_limit(1000)
                mon.connect("changed", on_change)
                state["monitors"].append(mon)
            except GLib.Error as e:
                log.debug("Kan %s niet volgen (cannot watch): %s", d, e.message)

    ctx.run_on_main(setup, wait=False)


def stop(api, ctx) -> None:
    state = getattr(ctx, "_apps_state", None)
    if not state:
        return
    if state.get("debounce"):
        state["debounce"].close()
    for m in state.get("monitors", []):
        try:
            m.cancel()
        except Exception:  # noqa: BLE001
            pass
    state["monitors"] = []
