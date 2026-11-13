"""settings.* (SPEC §5, §6.1): JSON store with defaults, deep merge, atomic writes, theme sync.

In-process use (other services):  ctx.settings.get("taskbar.pinned"), ctx.settings.set(path, value),
ctx.settings.patch({...}), ctx.settings.on_change(cb)  with cb(settings, path, changed_paths).
"""

from __future__ import annotations

import copy
import fcntl
from contextlib import contextmanager
import inspect
import json
import logging
import os
import re
import threading
import time
from pathlib import Path
from typing import Any, Callable

from .. import ROOT, gtkcss
from ..util import ApiError, atomic_write_json, dangerous, dry_for, deep_merge, del_path, diff_paths, get_path, home, \
    json_dumps, run, set_path, split_path, xdg_config_home, xdg_data_home

log = logging.getLogger("win11shell.settings")

PRIORITY = 0

# SPEC §5 plus the additive keys agreed at integration (theme.accentOnTaskbar, theme.captionButtons, theme.gtkFamily,
# explorer.*; taskbar.pinned
# starts with our Verkenner + the Software store). Must stay equal to ui/js/mock/settings.js DEFAULTS.
# explorer.pinned is deliberately absent (absent = Windows default pins; becomes a list once the user pins).
DEFAULTS: dict = json.loads(r"""
{
  "version": 1,
  "language": "nl",
  "theme": { "mode": "dark", "accent": "#0067c0", "accentFromWallpaper": false,
             "transparency": true, "syncSystem": true, "accentOnTaskbar": false, "captionButtons": true,
             "gtkFamily": "fluent" },
  "wallpaper": { "type": "image", "path": "builtin:lotus-dark", "color": "#0b2a4a",
                 "fit": "fill", "followTheme": true },
  "taskbar": { "alignment": "center", "search": "box", "taskView": true,
               "widgets": false, "autoHide": false, "showSeconds": false,
               "pinned": ["win11shell-explorer.desktop", "firefox_firefox.desktop", "snap-store_snap-store.desktop",
                          "win11shell-settings.desktop"],
               "tray": { "hiddenMenu": true, "shown": [], "order": [], "known": {} } },
  "start": { "pinned": [], "showRecommended": true, "showRecentApps": true,
             "layout": "default", "folders": ["settings", "explorer", "documents", "downloads"] },
  "desktop": { "showIcons": true, "iconSize": "medium", "autoArrange": false,
               "alignToGrid": true, "sortBy": "name", "positions": {},
               "showThisPC": true, "showTrash": true, "showHome": false },
  "notifications": { "dnd": false, "sounds": true, "showToasts": true },
  "time": { "h24": true, "showSeconds": false, "showDate": true, "firstDayOfWeek": 1 },
  "accessibility": { "textScale": 100, "reduceMotion": false },
  "nightLight": { "enabled": false, "temperature": 4500 },
  "taskmgr": { "updateSpeed": "normal", "defaultPage": "summary", "alwaysOnTop": false,
               "confirmEndTask": true, "colorKeyedGraphs": true, "graphSeconds": 60 },
  "alles": { "roots": "auto", "excluded": [], "matchCase": false, "matchPath": false,
             "regex": false, "wholeWord": false, "showHidden": false, "liveUpdate": true },
  "explorer": { "view": "details", "showHidden": false, "showExtensions": true, "sortBy": "name",
                "sortDesc": false, "groupBy": "none", "navPane": true, "detailsPane": false, "previewPane": false,
                "checkboxes": false, "compact": false, "navWidth": 248, "columns": {},
                "hiddenColumns": [], "favorites": [], "previewPane": false },
  "privacy": { "recentFiles": true },
  "power": { "confirm": true },
  "search": { "webEngine": "https://duckduckgo.com/?q=" }
}
""")

_MISSING = object()


# Settings whose value may be a string *or* a list (e.g. alles.roots: "auto" | ["/", "/mnt/data"]).
_STR_OR_LIST = {"alles.roots"}

# Lists that may only hold strings (tray app keys, SPEC §6.15).
_STR_LISTS = {"taskbar.tray.shown", "taskbar.tray.order"}


def _type_ok(default: Any, value: Any, path: str = "") -> bool:
    """Is ``value`` acceptable where the default is ``default``?"""
    if default is None or default is _MISSING:
        return True
    if isinstance(default, bool):
        return isinstance(value, bool)
    if isinstance(default, (int, float)):
        return isinstance(value, (int, float)) and not isinstance(value, bool)
    if isinstance(default, str):
        return isinstance(value, str) or (path in _STR_OR_LIST and isinstance(value, list))
    if isinstance(default, list):
        return isinstance(value, list) and (path not in _STR_LISTS or all(isinstance(v, str) for v in value))
    if isinstance(default, dict):
        return isinstance(value, dict)
    return True


def _sanitize(value: Any, default: Any, prefix: str, dropped: list[str]) -> Any:
    """Drop leaves whose type does not match the defaults (unknown keys are kept)."""
    if isinstance(default, dict) and isinstance(value, dict):
        out = {}
        for k, v in value.items():
            k = str(k)
            dv = default.get(k, _MISSING)
            p = f"{prefix}.{k}" if prefix else k
            if dv is _MISSING:
                out[k] = v
            elif not _type_ok(dv, v, p):
                dropped.append(p)
            else:
                out[k] = _sanitize(v, dv, p, dropped)
        return out
    return value


def _wrap_cb(cb: Callable) -> Callable[[dict, Any, list], None]:
    try:
        params = [p for p in inspect.signature(cb).parameters.values()
                  if p.kind in (p.POSITIONAL_ONLY, p.POSITIONAL_OR_KEYWORD)]
        has_var = any(p.kind == p.VAR_POSITIONAL for p in inspect.signature(cb).parameters.values())
        n = 3 if has_var else len(params)
    except (TypeError, ValueError):
        n = 3
    if n >= 3:
        return cb
    if n == 2:
        return lambda s, p, c: cb(s, p)
    if n == 1:
        return lambda s, p, c: cb(s)
    return lambda s, p, c: cb()


class SettingsStore:
    def __init__(self, path: Path, api=None, emit: bool = True):
        self.path = Path(path)
        self.api = api
        self._emit = emit
        self._lock = threading.RLock()
        self._listeners: list[tuple[Callable, Callable]] = []
        self._data = self._load()
        self._disk_bytes = self._read_disk_bytes()

    # -- persistence ------------------------------------------------------------
    def _load(self) -> dict:
        raw = None
        if self.path.exists():
            try:
                raw = json.loads(self.path.read_text(encoding="utf-8"))
                if not isinstance(raw, dict):
                    raise ValueError("not an object")
            except (OSError, ValueError) as e:
                broken = self.path.with_name(f"settings.broken-{time.strftime('%Y%m%d-%H%M%S')}.json")
                log.error("Instellingen onleesbaar (%s); bewaard als %s, standaardwaarden gebruikt "
                          "(settings unreadable; moved aside, using defaults)", e, broken.name)
                try:
                    os.replace(self.path, broken)
                except OSError:
                    pass
                raw = None
        dropped: list[str] = []
        clean = _sanitize(raw or {}, DEFAULTS, "", dropped)
        if dropped:
            log.warning("Ongeldige instellingen genegeerd (invalid settings ignored): %s", ", ".join(dropped[:20]))
        return deep_merge(DEFAULTS, clean)

    def _read_disk_bytes(self):
        try:
            return self.path.read_bytes()
        except OSError:
            return None

    @contextmanager
    def _disk_lock(self):
        # The Rust host and surviving compatibility listeners share one write lock.
        self.path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
        fd = os.open(self.path.parent / ".settings.lock", os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW | os.O_CLOEXEC, 0o600)
        try:
            fcntl.flock(fd, fcntl.LOCK_EX)
            yield
        finally:
            os.close(fd)

    def _adopt_disk_locked(self, force=False):
        raw = self._read_disk_bytes()
        if not force and raw == self._disk_bytes:
            return
        try:
            data = json.loads(raw) if raw is not None else copy.deepcopy(DEFAULTS)
            if not isinstance(data, dict):
                return
        except (ValueError, TypeError):
            return
        self._data = data
        self._disk_bytes = raw

    def adopt_native(self):
        """Refresh listeners from Rust-owned persistence without writing the file again."""
        with self._lock, self._disk_lock():
            old = self._data
            self._adopt_disk_locked(force=True)
            snap = copy.deepcopy(self._data)
            changed = diff_paths(old, snap)
        if changed:
            self._notify(snap, None, changed)
        return snap

    def _save(self, data: dict) -> None:
        try:
            atomic_write_json(self.path, data, pretty=True, mode=0o600)
        except OSError as e:
            log.error("Instellingen niet opgeslagen (settings not saved): %s", e)
            raise ApiError("save_failed", "De instellingen konden niet worden opgeslagen.", str(e)) from None

    # -- read -----------------------------------------------------------------
    def get(self, path: Any = None, default: Any = None) -> Any:
        with self._lock:
            if path is None or path == "":
                return copy.deepcopy(self._data)
            return copy.deepcopy(get_path(self._data, path, default))

    def snapshot(self) -> dict:
        return self.get()

    # -- write ----------------------------------------------------------------
    def _apply(self, mutate: Callable[[dict], dict], path: Any) -> dict:
        """Atomically: copy → mutate → save (if different). Listeners run AFTER the lock is released."""
        with self._lock, self._disk_lock():
            observed = self._data
            self._adopt_disk_locked()
            old = self._data
            new = mutate(copy.deepcopy(old))
            if diff_paths(old, new):
                self._save(new)
                self._disk_bytes = self._read_disk_bytes()
                self._data = new
            else:
                new = old
            changed = diff_paths(observed, new)
            snap = copy.deepcopy(new)
        if not changed:
            return snap
        label = ".".join(split_path(path)) if path not in (None, "") else None
        self._notify(snap, label, changed)
        return snap

    def _notify(self, snap: dict, path: str | None, changed: list[str]) -> None:
        if self.api is not None and self._emit:
            self.api.emit("settings.changed", {"settings": snap, "path": path, "changed": changed})
        for _, cb in list(self._listeners):
            try:
                cb(copy.deepcopy(snap), path, list(changed))
            except Exception:  # noqa: BLE001
                log.exception("Instellingen-listener faalde (settings listener failed)")

    def _check_value(self, segs: list[str], value: Any) -> Any:
        default = get_path(DEFAULTS, segs, _MISSING)
        if default is _MISSING:
            return value
        if not _type_ok(default, value, ".".join(segs)):
            raise ApiError("bad_value", "Deze waarde past niet bij deze instelling.",
                           {"path": ".".join(segs)})
        if isinstance(default, dict):
            dropped: list[str] = []
            value = _sanitize(value, default, ".".join(segs), dropped)
            if dropped:
                raise ApiError("bad_value", "Deze waarde past niet bij deze instelling.", {"paths": dropped})
        return value

    def set(self, path: Any, value: Any) -> dict:
        segs = split_path(path)
        if segs == ["version"]:
            raise ApiError("read_only", "Deze instelling kan niet worden gewijzigd.")
        try:
            json_dumps(value)
        except (TypeError, ValueError):
            raise ApiError("bad_value", "Deze waarde kan niet worden opgeslagen.") from None
        value = copy.deepcopy(self._check_value(segs, value))
        return self._apply(lambda d: set_path(d, segs, value), segs)

    def patch(self, patch: dict) -> dict:
        if not isinstance(patch, dict):
            raise ApiError("bad_value", "Ongeldige wijziging.")
        dropped: list[str] = []
        clean = _sanitize(patch, DEFAULTS, "", dropped)
        if dropped:
            raise ApiError("bad_value", "Deze waarde past niet bij deze instelling.", {"paths": dropped})
        clean.pop("version", None)
        return self._apply(lambda d: deep_merge(d, clean), None)

    def reset(self, path: Any = None) -> dict:
        if path in (None, ""):
            return self._apply(lambda d: copy.deepcopy(DEFAULTS), None)
        segs = split_path(path)
        default = get_path(DEFAULTS, segs, _MISSING)

        def mutate(d: dict) -> dict:
            if default is _MISSING:
                del_path(d, segs)
            else:
                set_path(d, segs, copy.deepcopy(default))
            return d

        return self._apply(mutate, segs)

    def export_json(self) -> str:
        with self._lock:
            return json_dumps(self._data, pretty=True)

    def import_json(self, text: Any) -> dict:
        if isinstance(text, dict):
            data = text
        else:
            if not isinstance(text, str) or len(text) > 4 * 1024 * 1024:
                raise ApiError("invalid_settings", "Dit bestand bevat geen geldige instellingen.")
            try:
                data = json.loads(text)
            except ValueError:
                raise ApiError("invalid_settings", "Dit bestand bevat geen geldige instellingen.") from None
        if not isinstance(data, dict) or not (set(data) & (set(DEFAULTS) - {"version"})):
            raise ApiError("invalid_settings", "Dit bestand bevat geen geldige instellingen.")
        dropped: list[str] = []
        clean = _sanitize(data, DEFAULTS, "", dropped)
        if dropped:
            log.warning("Import: ongeldige waarden overgeslagen (invalid values skipped): %s", ", ".join(dropped[:20]))
        clean["version"] = DEFAULTS["version"]
        merged = deep_merge(DEFAULTS, clean)
        return self._apply(lambda d: merged, None)

    def backup_file(self) -> dict:
        stamp = time.strftime("%Y%m%d-%H%M%S")
        target = self.path.with_name(f"settings-backup-{stamp}.json")
        n = 2
        while target.exists():
            target = self.path.with_name(f"settings-backup-{stamp}-{n}.json")
            n += 1
        with self._lock:
            atomic_write_json(target, self._data, pretty=True, mode=0o600)
        return {"path": str(target), "name": target.name, "dir": str(target.parent)}

    # -- listeners --------------------------------------------------------------
    def on_change(self, cb: Callable) -> Callable[[], None]:
        """cb(settings, path, changed) – also accepts cb(settings, path) / cb(settings)."""
        entry = (cb, _wrap_cb(cb))
        with self._lock:
            self._listeners.append(entry)

        def off():
            with self._lock:
                if entry in self._listeners:
                    self._listeners.remove(entry)

        return off


# ---------------------------------------------------------------------------
# Theme sync with GNOME (gsettings), only installed themes
# ---------------------------------------------------------------------------

def _xdg_data_dirs() -> list[Path]:
    dirs = [d for d in (os.environ.get("XDG_DATA_DIRS") or "/usr/local/share:/usr/share").split(":") if d]
    return [Path(d) for d in dirs if os.path.isabs(d)]


def _icon_theme_dirs() -> list[Path]:
    return [xdg_data_home() / "icons", home() / ".icons"] + [d / "icons" for d in _xdg_data_dirs()]


def _gtk_theme_dirs() -> list[Path]:
    # GTK's own search order: ~/.themes, $XDG_DATA_HOME/themes, $XDG_DATA_DIRS/themes (/usr/share/themes)
    return [home() / ".themes", xdg_data_home() / "themes"] + [d / "themes" for d in _xdg_data_dirs()]


def _icon_theme_installed(name: str) -> bool:
    return any((d / name / "index.theme").is_file() for d in _icon_theme_dirs())


def _gtk_theme_installed(name: str) -> bool:
    return any((d / name).is_dir() and ((d / name / "gtk-3.0").is_dir() or (d / name / "index.theme").is_file())
               for d in _gtk_theme_dirs())


# Ubuntu's own themes: Yaru, Yaru-dark, Yaru-<accent>, Yaru-<accent>-dark (Yaru-dark-hdpi etc. are not GTK themes)
_YARU = re.compile(r"^Yaru(?:-(?!dark$)([a-z]+))?(-dark)?$")
_ADWAITA = ("Adwaita", "Adwaita-dark")        # built into GTK: always available


def stock_theme_twin(current: str | None, dark: bool) -> str | None:
    """The light/dark twin of a stock theme (Yaru[-accent][-dark] → same accent, Adwaita ↔ Adwaita-dark), or
    None for any other theme (a user theme is never replaced) or a Yaru twin that is not installed."""
    if not current:
        return None
    if current in _ADWAITA:
        return "Adwaita-dark" if dark else "Adwaita"
    m = _YARU.match(current)
    if not m:
        return None
    twin = "Yaru" + (f"-{m.group(1)}" if m.group(1) else "") + ("-dark" if dark else "")
    return twin if (twin == current or _gtk_theme_installed(twin)) else None


def current_gtk_theme() -> str | None:
    """org.gnome.desktop.interface gtk-theme (read-only), or None."""
    r = run(["gsettings", "get", "org.gnome.desktop.interface", "gtk-theme"], timeout=5)
    value = r.out.strip().strip("'\"") if r.ok else ""
    return value or None


def current_accent_color() -> str | None:
    """org.gnome.desktop.interface accent-color (GNOME 47+, read-only), or None."""
    r = run(["gsettings", "get", "org.gnome.desktop.interface", "accent-color"], timeout=5)
    value = r.out.strip().strip("'\"") if r.ok else ""
    return value if re.fullmatch(r"[a-z]+", value or "") else None


#: theme.gtkFamily: which GTK theme family the light/dark sync keeps other apps in
GTK_FAMILIES = ("fluent", "system")


def fluent_theme(dark: bool) -> str | None:
    """The installed Fluent theme for light/dark (Fluent-round-* first), or None."""
    for gtk in (["Fluent-round-Dark", "Fluent-Dark"] if dark else ["Fluent-round-Light", "Fluent-Light"]):
        if _gtk_theme_installed(gtk):
            return gtk
    return None


def stock_default(dark: bool, accent: str | None = None) -> str:
    """Ubuntu's own look for a theme that is not a stock one (e.g. coming from Fluent): Yaru-<accent>[-dark] when the
    GNOME accent colour names an installed Yaru variant, else Yaru[-dark] when installed, else Adwaita[-dark]."""
    suffix = "-dark" if dark else ""
    if accent and accent != "orange":                 # orange is plain Yaru
        name = f"Yaru-{accent}{suffix}"
        if _YARU.match(name) and _gtk_theme_installed(name):
            return name
    if _gtk_theme_installed("Yaru" + suffix):
        return "Yaru" + suffix
    return "Adwaita" + suffix


def theme_targets(mode: str, current: str | None = _MISSING, family: str = "fluent",  # type: ignore[assignment]
                  accent: str | None = _MISSING) -> dict:  # type: ignore[assignment]
    """What a sync would set for ``mode`` ('light'|'dark'); only installed themes.

    GTK theme by ``family`` (theme.gtkFamily, an explicit choice in Instellingen › Kleuren — never guessed):
    * ``fluent`` (default, the Win11 look): Fluent-round-* / Fluent-* when installed; without Fluent the stock
      family keeps its accent and only flips light/dark (Yaru[-accent][-dark] → a twin that is installed,
      Adwaita ↔ Adwaita-dark); any other theme is left alone;
    * ``system`` (Ubuntu's own look): the stock twin of the current theme; a theme that is not a stock one
      (Fluent, …) becomes Yaru-<GNOME accent>[-dark] / Yaru[-dark] / Adwaita[-dark] (``stock_default``).
    ``current`` (the gtk-theme now) and ``accent`` (GNOME accent-color) are read from gsettings when needed and
    omitted."""
    dark = mode == "dark"
    out = {"color-scheme": "prefer-dark" if dark else "prefer-light"}
    icon = "Win11-dark" if dark else "Win11"
    if _icon_theme_installed(icon):
        out["icon-theme"] = icon
    if family != "system":
        gtk = fluent_theme(dark)
        if gtk:
            out["gtk-theme"] = gtk
            return out
    if current is _MISSING:
        current = current_gtk_theme()
    twin = stock_theme_twin(current, dark)
    if twin:
        out["gtk-theme"] = twin
    elif family == "system" and not (current and _is_stock(current)):
        if accent is _MISSING:
            accent = current_accent_color()
        out["gtk-theme"] = stock_default(dark, accent)
    # else: a stock theme whose twin is missing, or (fluent without Fluent) a user theme: left alone
    return out


def _is_stock(name: str) -> bool:
    return name in _ADWAITA or bool(_YARU.match(name))


# Windows' caption buttons — at the top right. Apps that draw their own title bar (GTK headerbars,
# Chromium/Electron such as Claude or VS Code) show only the buttons this GNOME key lists; GNOME's own
# default (':close' / 'appmenu:close') leaves just ×.
BUTTON_LAYOUT = ":minimize,maximize,close"


@dangerous(name="settings.syncSystemTheme.buttonLayout")
def sync_button_layout() -> dict:
    """org.gnome.desktop.wm.preferences button-layout → BUTTON_LAYOUT (only written when it differs)."""
    cur = run(["gsettings", "get", "org.gnome.desktop.wm.preferences", "button-layout"], timeout=5)
    if cur.ok and cur.out.strip().strip("'\"") == BUTTON_LAYOUT:
        return {"ok": True, "changed": False}
    r = run(["gsettings", "set", "org.gnome.desktop.wm.preferences", "button-layout", BUTTON_LAYOUT], timeout=5)
    if not r.ok:
        log.warning("gsettings button-layout mislukt (failed): %s", r.err.strip()[:200])
    else:
        log.info("Vensterknoppen: minimaliseren, maximaliseren, sluiten (button layout set)")
    return {"ok": r.ok, "changed": r.ok}


# One system sync at a time (gsettings theme keys, the gtk.css block): the watchers start a thread per change, and
# each worker re-reads the CURRENT settings under this lock, so the last toggle always wins (never a stale value
# captured when an earlier thread started, never a gtk-theme read by one sync and written after the next).
_SYNC_LOCK = threading.RLock()


@dangerous(name="settings.syncSystemTheme")
def sync_system_theme(mode: str, family: str = "fluent") -> dict:
    """Write org.gnome.desktop.interface color-scheme / icon-theme / gtk-theme, and the button layout."""
    with _SYNC_LOCK:
        targets = theme_targets(mode, family=family)
        results = {}
        for key, value in targets.items():
            r = run(["gsettings", "set", "org.gnome.desktop.interface", key, value], timeout=5)
            results[key] = r.ok
            if not r.ok:
                log.warning("gsettings %s=%s mislukt (failed): %s", key, value, r.err.strip()[:200])
        sync_button_layout()
        log.info("Systeemthema bijgewerkt (system theme synced): %s", targets)
        return {"ok": all(results.values()), "set": targets}


# libadwaita / GTK 4 apps read only ~/.config/gtk-4.0/gtk.css, never the gtk-theme key: install.sh puts Fluent's
# gtk-4.0 css there in a managed block (host/installer.py "win11shell theme", follows light/dark by itself through
# @media (prefers-color-scheme)). 'Thema voor apps' in Instellingen › Kleuren (theme.gtkFamily) must move GTK 4 along
# with GTK 3: 'system' takes the block out, 'fluent' puts it back. Only on that switch (a light/dark change needs no
# rewrite, and a block the user removed by hand is not forced back by it) — the same rule install.sh follows.
def _gtk4_css_of(name: str | None) -> str | None:
    """<theme dir>/<name>/gtk-4.0/gtk.css in GTK's own search order (as _gtk_theme_installed), or None."""
    if not name:
        return None
    for d in _gtk_theme_dirs():
        f = d / name / "gtk-4.0" / "gtk.css"
        if f.is_file():
            return str(f)
    return None


@dangerous(name="settings.syncSystemTheme.gtk4")
def sync_gtk4_block(family: str) -> dict:
    """The Fluent block in ~/.config/gtk-4.0/gtk.css for theme.gtkFamily: removed for 'system', added/updated for
    'fluent' (when a Fluent theme with a gtk-4.0 css is installed). {ok, action}; never raises."""
    import contextlib
    import io
    from .. import installer as _inst          # stdlib-only module next to gtkcss.py
    css = Path(xdg_config_home()) / "gtk-4.0" / "gtk.css"
    out = io.StringIO()
    try:
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(out):
            if family == "system":
                rc = _inst.gtk4_remove(css)
            else:
                dark, light = _gtk4_css_of(fluent_theme(True)), _gtk4_css_of(fluent_theme(False))
                if not (dark or light):
                    return {"ok": True, "action": "no-fluent"}
                rc = _inst.gtk4_add(css, dark, light)
    except Exception as e:  # noqa: BLE001
        log.warning("GTK 4-thema niet bijgewerkt (GTK 4 block not updated): %s", e)
        return {"ok": False, "action": "error"}
    action = (out.getvalue().strip().splitlines() or ["?"])[-1]
    if rc != 0:
        log.warning("GTK 4-thema: %s niet bijgewerkt (not updated): %s", css, action[:200])
    elif action not in ("unchanged",):
        log.info("GTK 4-apps volgen 'Thema voor apps' (%s): %s %s", family, css, action)
    return {"ok": rc == 0, "action": action}


def _gtk4_worker(ctx, snapshot: dict | None = None, *, remove_only: bool = False) -> None:
    """Background: the GTK 4 block for the CURRENT family (serialized with the other syncs; the last state wins).
    ``remove_only`` (the desktop start): only take a stale block out for 'system', never add one for 'fluent' — a
    block the user removed by hand stays removed."""
    with _SYNC_LOCK:
        theme = _latest_theme(ctx, snapshot)
        if not theme.get("syncSystem"):
            return
        family = gtk_family(theme)
        if remove_only and family != "system":
            return
        sync_gtk4_block(family)


# Windows 11 caption buttons in GTK 3/4 and Chromium/Electron apps: our CSS (config/gtk/) imported from a managed
# block in ~/.config/gtk-3.0/gtk.css and ~/.config/gtk-4.0/gtk.css (host/gtkcss.py). Light/dark needs no rewrite:
# the CSS takes its colours from the running theme.
@dangerous(name="settings.syncSystemTheme.captionButtons")
def sync_caption_buttons(enabled: bool) -> dict:
    """Add (``enabled``) or remove the managed block in both gtk.css files (only written when it differs)."""
    with _SYNC_LOCK:
        res = gtkcss.apply_all(bool(enabled), ROOT, xdg_config_home())
    for f in res["files"]:
        if not f["ok"]:
            log.warning("Vensterknoppen: %s niet bijgewerkt (caption buttons: not updated): %s", f["path"], f.get("error"))
    if res["changed"]:
        log.info("Windows 11-vensterknoppen %s (caption buttons %s): %s", "aan" if enabled else "uit",
                 "on" if enabled else "off", ", ".join(f"{f['path']} {f['action']}" for f in res["files"]))
    return res


def _latest_theme(ctx, snapshot: dict | None = None) -> dict:
    """theme.* as the store holds it NOW (the snapshot of the change that started the thread may be stale)."""
    try:
        theme = ctx.settings.get("theme")
        if isinstance(theme, dict):
            return theme
    except Exception:  # noqa: BLE001
        pass
    return (snapshot or {}).get("theme") or {}


def caption_enabled(theme: dict) -> bool:
    return theme.get("captionButtons", True) is not False


def gtk_family(theme: dict) -> str:
    fam = theme.get("gtkFamily")
    return fam if fam in GTK_FAMILIES else "fluent"


def _caption_worker(ctx, snapshot: dict | None = None) -> None:
    """Background: the caption-button block for the current setting (serialized; the last state wins)."""
    with _SYNC_LOCK:
        sync_caption_buttons(caption_enabled(_latest_theme(ctx, snapshot)))


def _theme_worker(ctx, snapshot: dict | None = None) -> None:
    """Background: the system theme for the current mode and family (serialized; the last state wins)."""
    with _SYNC_LOCK:
        theme = _latest_theme(ctx, snapshot)
        if not theme.get("syncSystem") or theme.get("mode") not in ("light", "dark"):
            return
        sync_system_theme(theme["mode"], gtk_family(theme))


def _caption_watcher(ctx):
    def on_change(settings: dict, path, changed: list[str]):
        if not any(c in ("theme.captionButtons", "theme") for c in changed):
            return
        enabled = caption_enabled(settings.get("theme") or {})
        if dry_for(ctx):
            log.info("[dry-run] vensterknoppen zouden %s worden (would %s caption buttons)",
                     "aangezet" if enabled else "uitgezet", "enable" if enabled else "disable")
            return
        if getattr(ctx, "mode", None) != "layer":
            # only the desktop session writes the user's gtk.css (its ROOT is the installed package): a preview,
            # serve or app host may run from a development tree, whose path must never end up in every GTK app
            log.info("Vensterknoppen %s: wordt toegepast bij de volgende start van het bureaublad "
                     "(caption buttons applied at the next desktop start; mode %s)",
                     "aan" if enabled else "uit", getattr(ctx, "mode", None))
            return
        ctx.thread(_caption_worker, ctx, settings, name="caption-buttons")

    return on_change


def _theme_watcher(ctx):
    def on_change(settings: dict, path, changed: list[str]):
        if not any(c in ("theme.mode", "theme.syncSystem", "theme.gtkFamily") or c == "theme" for c in changed):
            return
        theme = settings.get("theme") or {}
        if not theme.get("syncSystem"):
            return
        mode = theme.get("mode")
        if mode not in ("light", "dark"):
            return
        family_switch = any(c in ("theme.gtkFamily", "theme") for c in changed)
        if dry_for(ctx):
            log.info("[dry-run] systeemthema zou worden: %s (would sync)", theme_targets(mode, family=gtk_family(theme)))
            if family_switch:
                log.info("[dry-run] GTK 4-blok zou %s (would %s the GTK 4 block)",
                         "weggaan" if gtk_family(theme) == "system" else "Fluent worden",
                         "remove" if gtk_family(theme) == "system" else "add")
            return
        ctx.thread(_theme_worker, ctx, settings, name="theme-sync")
        # GTK 4 follows the family switch in every host that syncs the theme, like gtk-theme above: Instellingen in
        # GNOME is an app host. Unlike the caption block, the theme block points at the installed theme
        # (~/.themes/Fluent-*), never at our ROOT, so a host from a development tree cannot leak its path into it.
        if family_switch:
            ctx.thread(_gtk4_worker, ctx, settings, name="gtk4-theme")

    return on_change


# ---------------------------------------------------------------------------
# register
# ---------------------------------------------------------------------------

def register(api, ctx) -> None:
    store = SettingsStore(Path(ctx.config_dir) / "settings.json", api=api)
    ctx.settings = store
    store.on_change(_theme_watcher(ctx))
    store.on_change(_caption_watcher(ctx))

    @api.method("_compat.settings.adopt")
    def _adopt(args):
        return store.adopt_native()

    @api.method("settings.get")
    def _get(args):
        return store.get()

    @api.method("settings.set")
    def _set(args):
        if "path" not in args:
            raise ApiError("bad_args", "Geef aan welke instelling je wilt wijzigen.")
        return store.set(args["path"], args.get("value"))

    @api.method("settings.patch")
    def _patch(args):
        return store.patch(args.get("patch") or {})

    @api.method("settings.reset")
    def _reset(args):
        return store.reset(args.get("path"))

    @api.method("settings.export")
    def _export(args):
        return {"json": store.export_json(), "name": f"win11shell-instellingen-{time.strftime('%Y-%m-%d')}.json"}

    @api.method("settings.import")
    def _import(args):
        return store.import_json(args.get("json"))

    @api.method("settings.backupFile")
    def _backup(args):
        return store.backup_file()


def start(api, ctx) -> None:
    """Desktop session: Windows' caption buttons in every app — the button order (only while the system theme is
    synced) and their Windows 11 look (theme.captionButtons; a stale block is removed when it is off) — and, with
    'Thema voor apps' on Ubuntu's look, a stale GTK 4 Fluent block out (a switch made while no host could write it)."""
    if getattr(ctx, "mode", None) != "layer" or dry_for(ctx):
        return
    try:
        sync = bool(ctx.settings.get("theme.syncSystem", True))
    except Exception:  # noqa: BLE001
        sync = True
    if sync:
        ctx.thread(sync_button_layout, name="button-layout")
        ctx.thread(_gtk4_worker, ctx, remove_only=True, name="gtk4-theme")
    ctx.thread(_caption_worker, ctx, name="caption-buttons")
