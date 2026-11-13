"""Folders, files and URIs that other apps hand to Verkenner → Explorer params (SPEC §12 "Verkenner als standaard").

Two ways in, one format out:

* the desktop entry ``win11shell-explorer.desktop`` (``Exec=…/win11shell explorer %U``: ``xdg-open`` / ``gio open`` of a
  folder, a desktop's "Map openen", apps that start "the file manager" through GIO):
  ``python3 -m host --mode app --open explorer:<first> <more…>`` → ``host/__main__.py`` → :func:`explorer_params`
  with ``how='auto'``: a folder opens that folder, a file opens its folder with the file selected;
* ``org.freedesktop.FileManager1`` (``host/services/filemanager1.py``, "Weergeven in map" in browsers):
  ShowFolders → ``how='auto'``, ShowItems → ``how='items'`` (every item selected in its parent folder, folders too),
  ShowItemProperties → ``how='items'`` + ``properties=True``.

Explorer params (``ui/js/apps/explorer/main.js``)::

    {path, select?: [abs path…], tabs?: [{path, select?}…], properties?: true}

``path`` is the folder of the first tab (or a place: ``trash:///``, ``computer:///``, ``recent:///``, a place word such
as ``thispc``); every further folder gets its own tab, in order of first appearance; items of one folder share a tab
and are all selected. At most :data:`MAX_TABS` tabs and :data:`MAX_SELECT` selected items per tab; the JSON stays below
:data:`MAX_JSON` bytes (``shell.openApp`` takes ≤ 64 KiB). Whatever is cut is logged.

Input rules (:func:`classify`):

* ``file://`` URIs: percent-decoded (``%20`` → space, UTF-8 sequences → characters); host ``''``, ``localhost`` or
  this machine's name (another host: skipped); query and fragment ignored — every skip writes one clear log line;
* absolute paths, ``~`` paths and paths relative to ``cwd``; a relative word that does not exist (``thispc``,
  ``trash``, ``downloads``) is passed on as a place word, which the Explorer resolves itself;
* a name that is not valid UTF-8 (``\xff.txt`` from an old zip or FAT disk; the UI and the JSON params cannot carry
  it), as a path or in a ``file://`` URI: the nearest folder above it whose path is valid UTF-8 opens instead, nothing
  selected (``('folder', …)`` from :func:`classify`; logged);
* ``trash:``, ``computer:``, ``recent:`` → the Explorer's Prullenbak, Deze pc and Start;
* any other scheme (``sftp://``, ``smb://``, ``mtp://`` …): the local path GIO knows for a *mounted* gvfs location
  (``/run/user/<uid>/gvfs/…``, via ``Gio.File.get_path``); not mounted → skipped with a log line. Nothing is mounted
  for the caller: that may need a password, and a click in a browser must never pop up a login prompt;
* ``http(s)``, ``mailto`` and the like: skipped, logged.

Missing paths: ``auto`` opens the nearest existing folder above it; ``items`` opens the parent and selects the name
(the Explorer simply finds nothing to select when it is gone).

Pure Python (only the default gvfs resolver imports Gio, lazily); no host state, so it is used by the CLI launcher
process as well as by the host.
"""

from __future__ import annotations

import json
import logging
import os
import re
import socket
from typing import Callable, Iterable
from urllib.parse import unquote, urlsplit

log = logging.getLogger("win11shell.explorer-args")

__all__ = ["MAX_TABS", "MAX_SELECT", "MAX_JSON", "MAX_ITEMS", "classify", "explorer_params", "file_uri_to_path",
           "gio_local_path", "is_place"]

MAX_TABS = 20          # tabs per request (Windows opens one window per folder; we stop at a sane number)
MAX_SELECT = 500       # selected items per tab
MAX_JSON = 48 * 1024   # whole params object, JSON (shell.openApp limit is 64 KiB)
MAX_ITEMS = 2000       # inputs looked at per request (a D-Bus caller can send any number)

_SCHEME_RE = re.compile(r"^([A-Za-z][A-Za-z0-9+.-]*):")
_PLACE_SCHEMES = {"trash": "trash:///", "computer": "computer:///", "recent": "recent:///"}
_PLACE_WORD_RE = re.compile(r"^[A-Za-z][A-Za-z'_-]{0,31}$")
_NOT_FILES = {"http", "https", "mailto", "ftp", "javascript", "data", "about", "tel", "sms", "magnet", "news", "irc",
              "callto", "x-scheme-handler"}
_LOCAL_HOSTS = {"", "localhost", "localhost.localdomain", "127.0.0.1", "::1"}


def is_place(s: str) -> bool:
    """A place the Explorer resolves itself (``trash:///``, ``thispc``…), not a path."""
    return not s.startswith("/")


def _hostname() -> str:
    try:
        return socket.gethostname().lower()
    except OSError:
        return ""


def _utf8_ok(path: str) -> bool:
    try:
        path.encode("utf-8")
        return True
    except UnicodeEncodeError:
        return False


def _utf8_ancestor(path: str) -> str:
    """The longest leading part of an absolute ``path`` that is valid UTF-8 (at least ``/``)."""
    good: list[str] = []
    for seg in path.split("/"):
        if not seg:
            continue
        if not _utf8_ok(seg):
            break
        good.append(seg)
    return "/" + "/".join(good)


def _decode_file_uri(uri: str) -> str | None:
    """``file://`` URI → absolute path (may hold surrogate escapes for non-UTF-8 bytes); None: not a local file."""
    try:
        parts = urlsplit(uri)
    except ValueError:
        return None
    if parts.scheme.lower() != "file":
        return None
    host = (parts.hostname or "").lower() if parts.netloc else ""
    if host not in _LOCAL_HOSTS and host != _hostname():
        log.info("Andere computer genegeerd (file URI of another host ignored): %s", uri[:200])
        return None
    path = unquote(parts.path or "/", encoding="utf-8", errors="surrogateescape")
    if not path.startswith("/"):
        return None
    return os.path.normpath(path) if path != "/" else "/"


def file_uri_to_path(uri: str) -> str | None:
    """``file:///home/x/a%20b`` → ``/home/x/a b``; None for another host or a name that is not valid UTF-8."""
    path = _decode_file_uri(uri)
    if path is not None and not _utf8_ok(path):
        log.info("Naam is geen geldige UTF-8 (name is not valid UTF-8): %s", uri[:200])
        return None
    return path


def gio_local_path(uri: str) -> str | None:
    """Local (FUSE) path of a mounted gvfs location (sftp://, smb://, mtp:// …), else None. Never mounts."""
    try:
        import gi
        gi.require_version("Gio", "2.0")
        from gi.repository import Gio
    except Exception:  # noqa: BLE001
        return None
    try:
        p = Gio.File.new_for_uri(uri).get_path()
    except Exception:  # noqa: BLE001
        return None
    return p if p and p.startswith("/") else None


def classify(item: str, *, cwd: str | None = None,
             remote: Callable[[str], str | None] | None = gio_local_path) -> tuple[str, str] | None:
    """One input → ``('path', abs)``, ``('place', place)`` or ``('folder', abs)`` (a folder to open as it is, nothing
    selected: what is left of a path whose name is not valid UTF-8); None when it is skipped (logged)."""
    c = _classify(item, cwd=cwd, remote=remote)
    if c is not None and c[0] == "path" and not _utf8_ok(c[1]):
        up = _utf8_ancestor(c[1])
        log.info("Naam is geen geldige UTF-8; ik open %s (name is not valid UTF-8, opening the folder above it): %r",
                 up, c[1][:200])
        return ("folder", up)
    return c


def _classify(item: str, *, cwd: str | None,
              remote: Callable[[str], str | None] | None) -> tuple[str, str] | None:
    if not isinstance(item, str):
        return None
    s = item if item.startswith("/") else item.strip()   # '/x/map ' is a real name; ' file:///x ' is not
    if not s or "\0" in s or len(s) > 8192 or any(ord(c) < 32 for c in s):
        log.info("Ongeldige invoer genegeerd (invalid argument ignored): %r", s[:80])
        return None
    if s.startswith("/"):
        return ("path", os.path.normpath(s) if s != "/" else "/")
    if s == "~" or s.startswith("~/"):
        return ("path", os.path.normpath(os.path.expanduser(s)))
    m = _SCHEME_RE.match(s)
    scheme = m.group(1).lower() if m else ""
    # 'a:b.txt' in the current folder is a file, not a URI: only '<scheme>://' or one of the known schemes count
    if m and (s[m.end():].startswith("//") or scheme in _PLACE_SCHEMES or scheme == "file" or scheme in _NOT_FILES):
        if scheme == "file":
            p = _decode_file_uri(s)
            return ("path", p) if p else None
        if scheme in _PLACE_SCHEMES:
            return ("place", _PLACE_SCHEMES[scheme])
        if scheme in _NOT_FILES:
            log.info("Geen map of bestand, genegeerd (not a folder or file, ignored): %s:…", scheme)
            return None
        local = None
        if remote is not None:
            try:
                local = remote(s)
            except Exception:  # noqa: BLE001
                local = None
        if local and local.startswith("/"):
            return ("path", os.path.normpath(local))
        log.info("%s-locatie is niet gekoppeld; koppel hem eerst in Verkenner of Bestanden "
                 "(%s location not mounted, ignored): %s", scheme, scheme, _redact(s))
        return None
    base = cwd or os.getcwd()
    p = os.path.normpath(os.path.join(base, s))
    if not os.path.lexists(p) and _PLACE_WORD_RE.match(s):
        return ("place", s)
    return ("path", p)


def _redact(uri: str) -> str:
    """sftp://user:secret@host/x → sftp://host/x (log lines never carry a password)."""
    try:
        parts = urlsplit(uri)
        host = parts.hostname or ""
        if parts.port:
            host += f":{parts.port}"
        return f"{parts.scheme}://{host}{parts.path}"[:200]
    except ValueError:
        return uri.split("://", 1)[0] + "://…"


def _nearest_dir(p: str, isdir: Callable[[str], bool]) -> str:
    d = p
    while True:
        if isdir(d):
            return d
        nd = os.path.dirname(d)
        if nd == d:
            return "/"
        d = nd


def explorer_params(items: Iterable[str], *, how: str = "auto", properties: bool = False, cwd: str | None = None,
                    remote: Callable[[str], str | None] | None = gio_local_path,
                    isdir: Callable[[str], bool] = os.path.isdir,
                    exists: Callable[[str], bool] = os.path.lexists) -> dict | None:
    """Inputs (paths / URIs / place words) → Explorer params, or None when nothing usable is left.

    ``how='auto'``: folders open as themselves, everything else in its folder, selected (CLI, ShowFolders);
    ``how='items'``: every item is selected in its parent folder (ShowItems, ShowItemProperties)."""
    if how not in ("auto", "items"):
        raise ValueError(how)
    entries: list[dict] = []           # {"path": folder|place, "select": [..]} in order of first appearance
    by_folder: dict[str, dict] = {}
    seen_sel: dict[str, set] = {}
    count = 0
    for raw in items:
        count += 1
        if count > MAX_ITEMS:
            log.info("Meer dan %d items; de rest wordt genegeerd (too many items, rest ignored)", MAX_ITEMS)
            break
        c = classify(raw, cwd=cwd, remote=remote)
        if c is None:
            continue
        kind, val = c
        if kind == "place":
            if val not in by_folder:
                by_folder[val] = {"path": val, "select": []}
                entries.append(by_folder[val])
            continue
        folder, sel = None, None
        if kind == "folder":
            folder = val if isdir(val) else _nearest_dir(val, isdir)
        elif how == "auto" and isdir(val):
            folder = val
        elif how == "auto" and not exists(val):
            folder = _nearest_dir(os.path.dirname(val), isdir)
            log.info("Bestaat niet; ik open %s (does not exist, opening the nearest folder): %s", folder, val)
        else:
            parent = os.path.dirname(val) if val != "/" else "/"
            folder = _nearest_dir(parent, isdir) if not isdir(parent) else parent
            sel = val if val != "/" and folder == parent else None
        ent = by_folder.get(folder)
        if ent is None:
            ent = by_folder[folder] = {"path": folder, "select": []}
            seen_sel[folder] = set()
            entries.append(ent)
        if sel and sel not in seen_sel[folder]:
            seen_sel[folder].add(sel)
            ent["select"].append(sel)
    if not entries:
        return None
    if len(entries) > MAX_TABS:
        log.info("%d mappen gevraagd, %d geopend in tabbladen (folders asked, tabs opened)", len(entries), MAX_TABS)
        entries = entries[:MAX_TABS]
    for e in entries:
        if len(e["select"]) > MAX_SELECT:
            log.info("%d items in %s, de eerste %d worden geselecteerd (selection capped)",
                     len(e["select"]), e["path"], MAX_SELECT)
            e["select"] = e["select"][:MAX_SELECT]
    params = _build(entries, properties)
    while len(json.dumps(params, ensure_ascii=False).encode("utf-8")) > MAX_JSON:
        if len(entries) > 1:
            entries = entries[:-1]
        else:
            biggest = max(entries, key=lambda e: len(e["select"]))
            if not biggest["select"]:
                break
            biggest["select"] = biggest["select"][:len(biggest["select"]) // 2]
        log.info("Te veel gegevens; minder tabbladen of selectie (too much data, trimmed)")
        params = _build(entries, properties)
    if properties and len(entries) > 1:
        log.info("Eigenschappen alleen voor de eerste map; de andere openen als tabblad "
                 "(properties for the first folder only)")
    return params


def _build(entries: list[dict], properties: bool) -> dict:
    def one(e: dict) -> dict:
        d = {"path": e["path"]}
        if e["select"]:
            d["select"] = list(e["select"])
        return d

    params = one(entries[0])
    if len(entries) > 1:
        params["tabs"] = [one(e) for e in entries[1:]]
    if properties:
        params["properties"] = True
    return params
