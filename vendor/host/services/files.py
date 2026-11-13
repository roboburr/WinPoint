"""files.* (SPEC §6.5): listing, places, opening, rename/create, trash, copy/move, clipboard,
properties, archives, find, directory watching.

FileItem = {name, displayName, path, uri, isDir, isDesktopEntry, icon, mime, size, modified, hidden,
isSymlink} + extras ``typeName`` (Windows-style type in the UI language: "Bestandsmap",
"Tekstdocument", "PNG-bestand" …) and, for items in the Prullenbak, ``originalPath``,
``originalDir``, ``deletedAt`` (ms) and ``trashName``.

Safety: nothing is ever deleted permanently except by ``files.emptyTrash`` (dangerous; the UI
confirms). Every method that moves/copies/renames/trashes files is dangerous (``--dry-run`` skips
it). Creating a new folder/file is not (it never touches existing data).

Test hooks: ``WIN11SHELL_TRASH_DIR`` replaces every trash dir by one sandbox dir and trashing then
moves files there itself (freedesktop layout) instead of using Gio — never the real Prullenbak.
"""

from __future__ import annotations

import ctypes
import errno
import fnmatch
import logging
import os
import pwd
import grp
import re
import shutil
import stat
import tarfile
import threading
import time
import uuid
import zipfile
from collections import deque
from datetime import datetime
from pathlib import Path
from typing import Any, Callable
from urllib.parse import quote, unquote

from ..util import ApiError, atomic_write_text, dry_for, first_existing_icon, mime_icon_names, run, spawn, \
    user_env, user_info, which, xdg_data_home, xdg_user_dir
from .apps import Debouncer, Gio, GLib, OWN_APPS, _lang, default_terminal, dry_skip, in_application_dirs, \
    installed_copy, launch_app, launch_file, launcher_trust, path_to_uri, read_desktop_entry, sort_key, \
    trust_launcher

log = logging.getLogger("win11shell.files")

PRIORITY = 30

TRASH_URI = "trash:///"
MAX_LIST = 50_000

# ---------------------------------------------------------------------------
# Type names (Windows-like, nl/en)
# ---------------------------------------------------------------------------

_TYPES: dict[str, tuple[str, str]] = {
    "inode/directory": ("Bestandsmap", "File folder"),
    "text/plain": ("Tekstdocument", "Text Document"),
    "application/zip": ("Gecomprimeerde (gezipte) map", "Compressed (zipped) Folder"),
    "application/pdf": ("PDF-document", "PDF Document"),
    "application/x-desktop": ("Snelkoppeling", "Shortcut"),
    "inode/symlink": ("Snelkoppeling (verbroken)", "Shortcut (broken)"),
    "application/msword": ("Microsoft Word-document", "Microsoft Word Document"),
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document":
        ("Microsoft Word-document", "Microsoft Word Document"),
    "application/vnd.ms-excel": ("Microsoft Excel-werkblad", "Microsoft Excel Worksheet"),
    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet":
        ("Microsoft Excel-werkblad", "Microsoft Excel Worksheet"),
    "application/vnd.ms-powerpoint": ("Microsoft PowerPoint-presentatie", "Microsoft PowerPoint Presentation"),
    "application/vnd.openxmlformats-officedocument.presentationml.presentation":
        ("Microsoft PowerPoint-presentatie", "Microsoft PowerPoint Presentation"),
    "application/vnd.oasis.opendocument.text": ("OpenDocument-tekst", "OpenDocument Text"),
    "application/vnd.oasis.opendocument.spreadsheet": ("OpenDocument-werkblad", "OpenDocument Spreadsheet"),
    "application/vnd.oasis.opendocument.presentation": ("OpenDocument-presentatie", "OpenDocument Presentation"),
    "text/html": ("HTML-document", "HTML Document"),
    "application/xhtml+xml": ("HTML-document", "HTML Document"),
    "application/x-executable": ("Toepassing", "Application"),
    "application/x-pie-executable": ("Toepassing", "Application"),
    "application/x-sharedlib": ("Toepassingsuitbreiding", "Application extension"),
    "application/x-shellscript": ("Shell-script", "Shell Script"),
    "application/x-iso9660-image": ("Schijfkopiebestand", "Disc Image File"),
    "application/json": ("JSON-bestand", "JSON File"),
    "text/markdown": ("Markdown-bestand", "Markdown File"),
    "text/csv": ("CSV-bestand", "CSV File"),
    "text/x-python": ("Python-bestand", "Python File"),
    "text/x-python3": ("Python-bestand", "Python File"),
    "application/vnd.debian.binary-package": ("Softwarepakket", "Software Package"),
    "application/x-deb": ("Softwarepakket", "Software Package"),
    "application/x-msdownload": ("Windows-programma", "Windows Program"),
    "application/x-ms-dos-executable": ("Windows-programma", "Windows Program"),
    "application/x-msi": ("Windows Installer-pakket", "Windows Installer Package"),
    "application/vnd.appimage": ("AppImage-toepassing", "AppImage Application"),
    "application/x-iso9660-appimage": ("AppImage-toepassing", "AppImage Application"),
    "application/x-compressed-tar": ("Gecomprimeerd archief", "Compressed archive"),
    "application/x-xz-compressed-tar": ("Gecomprimeerd archief", "Compressed archive"),
    "application/x-bzip-compressed-tar": ("Gecomprimeerd archief", "Compressed archive"),
    "application/x-zstd-compressed-tar": ("Gecomprimeerd archief", "Compressed archive"),
    "application/x-tar": ("Archief", "Archive"),
    "application/x-7z-compressed": ("7Z-archief", "7Z Archive"),
    "application/vnd.rar": ("RAR-archief", "RAR Archive"),
    "application/x-rar": ("RAR-archief", "RAR Archive"),
    "application/gzip": ("GZ-archief", "GZ Archive"),
    "inode/fifo": ("Pijp", "Pipe"),
    "inode/socket": ("Socket", "Socket"),
    "inode/blockdevice": ("Apparaat", "Device"),
    "inode/chardevice": ("Apparaat", "Device"),
}

_mime_comment_cache: dict[tuple[str, str], str | None] = {}
_COMMENT_RE = re.compile(r'<comment(?:\s+xml:lang="([^"]+)")?\s*>([^<]*)</comment>')


def _mime_comment(mime: str, lang: str) -> str | None:
    """Localised comment from shared-mime-info's per-type XML (/usr/share/mime/<type>.xml)."""
    key = (mime, lang)
    if key in _mime_comment_cache:
        return _mime_comment_cache[key]
    result = None
    if re.match(r"^[a-z0-9.+-]+/[a-zA-Z0-9.+_-]+$", mime or ""):
        dirs = [xdg_data_home()] + [Path(d) for d in (os.environ.get("XDG_DATA_DIRS") or "/usr/share").split(":") if d]
        for d in dirs:
            p = d / "mime" / (mime + ".xml")
            try:
                text = p.read_text(encoding="utf-8", errors="replace")
            except OSError:
                continue
            default = None
            for m in _COMMENT_RE.finditer(text):
                lg, val = m.group(1), m.group(2).strip()
                if lg is None and default is None:
                    default = val
                elif lg in (lang, f"{lang}_NL", f"{lang}_BE") and val:
                    result = val
                    break
            if result is None and lang == "en":
                result = default
            break
    if result:
        result = result.replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">")
        result = result[:1].upper() + result[1:]
    _mime_comment_cache[key] = result
    return result


def _ext(name: str) -> str:
    base = name.lstrip(".")
    if "." not in base:
        return ""
    return base.rsplit(".", 1)[1]


def type_name(mime: str, name: str, lang: str) -> str:
    li = 0 if lang == "nl" else 1
    t = _TYPES.get(mime)
    if t:
        return t[li]
    ext = _ext(name)
    major = (mime or "").split("/", 1)[0]
    if ext and major in ("image", "audio", "video") and len(ext) <= 6:
        return f"{ext.upper()}-bestand" if li == 0 else f"{ext.upper()} File"
    if major == "font":
        return "Lettertypebestand" if li == 0 else "Font File"
    if mime and mime != "application/octet-stream":
        c = _mime_comment(mime, lang)
        if c:
            return c
    if ext and len(ext) <= 8:
        return f"{ext.upper()}-bestand" if li == 0 else f"{ext.upper()} File"
    return "Bestand" if li == 0 else "File"


# ---------------------------------------------------------------------------
# FileItem
# ---------------------------------------------------------------------------

_mime_icon_cache: dict[str, str] = {}
_ICON_LOCK = threading.Lock()


def mime_icon(mime: str) -> str:
    with _ICON_LOCK:
        hit = _mime_icon_cache.get(mime)
    if hit is not None:
        return hit
    fb = "text-x-generic" if mime.startswith("text/") else "application-x-generic"
    icon = first_existing_icon(mime_icon_names(mime), fb) or fb
    with _ICON_LOCK:
        _mime_icon_cache[mime] = icon
    return icon


def guess_mime(path: str, name: str, size: int, sniff: list[int] | None = None) -> str:
    if Gio is None:
        return "application/octet-stream"
    try:
        ct, uncertain = Gio.content_type_guess(name, None)
        if uncertain and size > 0 and sniff is not None and sniff[0] > 0:
            sniff[0] -= 1
            try:
                with open(path, "rb") as f:
                    head = f.read(4096)
                ct2, _ = Gio.content_type_guess(name, head)
                ct = ct2 or ct
            except OSError:
                pass
        mime = Gio.content_type_get_mime_type(ct) if ct else None
        return mime or ct or "application/octet-stream"
    except Exception:  # noqa: BLE001
        return "application/octet-stream"


def read_hidden(dirpath: str) -> frozenset[str]:
    try:
        with open(os.path.join(dirpath, ".hidden"), encoding="utf-8", errors="replace") as f:
            return frozenset(l.strip() for l in f.read(256 * 1024).splitlines() if l.strip())
    except OSError:
        return frozenset()


def _utf8_ok(s: str) -> bool:
    try:
        s.encode("utf-8")
        return True
    except UnicodeEncodeError:
        return False


def launcher_ok(path: str) -> bool:
    """May this .desktop file start (and show its own name/icon)? Installed apps, copies of installed
    apps and launchers the user explicitly allowed ("Starten toestaan") — see apps.launcher_trust."""
    if in_application_dirs(path):
        return True
    if launcher_trust(path)[0]:
        return True
    return installed_copy(path) is not None


class ItemFactory:
    """Builds FileItems; one per request (holds caches + sniff budget)."""

    def __init__(self, ctx, lang: str | None = None):
        self.ctx = ctx
        self.lang = lang or _lang(ctx)
        self.sniff = [300]
        self._special = _special_dirs(ctx)

    def make(self, path: str, hidden_names: frozenset[str] = frozenset(), lst: os.stat_result | None = None) -> dict:
        name = os.path.basename(path.rstrip("/")) or path
        if lst is None:
            lst = os.lstat(path)
        is_link = stat.S_ISLNK(lst.st_mode)
        st = lst
        broken = False
        if is_link:
            try:
                st = os.stat(path)
            except OSError:
                broken = True
        is_dir = stat.S_ISDIR(st.st_mode) and not broken
        is_desktop = False
        untrusted = False
        display = name
        if is_dir:
            mime = "inode/directory"
            icon = self._special.get(path) or "folder"
        elif broken:
            mime = "inode/symlink"
            icon = "emblem-symbolic-link" if first_existing_icon(["emblem-symbolic-link"]) else "text-x-generic"
        elif stat.S_ISREG(st.st_mode):
            if name.endswith(".desktop"):
                mime = "application/x-desktop"
                entry = read_desktop_entry(path, self.lang)
                is_desktop = entry is not None
                if entry:
                    display = entry.get("name") or name
                    icon = entry.get("icon") or "application-x-executable"
                    if entry.get("type") == "Application" and not launcher_ok(path):
                        # Not trusted: never let it pose as something else ("Vakantiefoto.jpg" with a photo
                        # icon). Show the real file name and a plain launcher icon until the user allows it.
                        display = name
                        icon = first_existing_icon(["application-x-desktop", "application-x-executable"],
                                                   "text-x-generic")
                        untrusted = True
                else:
                    icon = mime_icon(mime)
            else:
                mime = guess_mime(path, name, st.st_size, self.sniff)
                icon = mime_icon(mime)
        else:
            mime = ("inode/fifo" if stat.S_ISFIFO(st.st_mode) else "inode/socket" if stat.S_ISSOCK(st.st_mode)
                    else "inode/blockdevice" if stat.S_ISBLK(st.st_mode) else "inode/chardevice")
            icon = "application-x-generic"
        return {
            "name": name,
            "displayName": display,
            "path": path,
            "uri": path_to_uri(path),
            "isDir": is_dir,
            "isDesktopEntry": is_desktop,
            "icon": icon,
            "mime": mime,
            "size": 0 if is_dir else int(st.st_size),
            "modified": int(st.st_mtime * 1000),
            "hidden": name.startswith(".") or name.endswith("~") or name in hidden_names,
            "isSymlink": is_link,
            "typeName": type_name(mime, name, self.lang),
            **({"untrusted": True} if untrusted else {}),
        }

    def listdir(self, dirpath: str, show_hidden: bool, limit: int = MAX_LIST) -> tuple[list[dict], bool]:
        hidden = read_hidden(dirpath)
        items: list[dict] = []
        truncated = False
        skipped = 0
        with os.scandir(dirpath) as it:
            for e in it:
                if not _utf8_ok(e.name):
                    skipped += 1
                    continue
                if len(items) >= limit:
                    truncated = True
                    break
                try:
                    item = self.make(e.path, hidden, e.stat(follow_symlinks=False))
                except OSError:
                    continue
                if item["hidden"] and not show_hidden:
                    continue
                items.append(item)
        if skipped:
            log.info("%d namen met ongeldige tekens overgeslagen in (non-UTF-8 names skipped) %s", skipped, dirpath)
        items.sort(key=lambda i: (not i["isDir"], sort_key(i["displayName"])))
        return items, truncated


def _special_dirs(ctx) -> dict[str, str]:
    cands = {
        str(ctx.home): ["user-home"],
        str(ctx.desktop_dir): ["user-desktop"],
        str(xdg_user_dir("DOCUMENTS")): ["folder-documents"],
        str(xdg_user_dir("DOWNLOAD")): ["folder-download", "folder-downloads"],
        str(xdg_user_dir("MUSIC")): ["folder-music"],
        str(xdg_user_dir("PICTURES")): ["folder-pictures"],
        str(xdg_user_dir("VIDEOS")): ["folder-videos"],
        str(xdg_user_dir("TEMPLATES")): ["folder-templates"],
        str(xdg_user_dir("PUBLICSHARE")): ["folder-publicshare"],
    }
    out = {}
    for p, names in cands.items():
        icon = _special_icon_cache.get(names[0])
        if icon is None:
            icon = first_existing_icon(names, "folder") or "folder"
            _special_icon_cache[names[0]] = icon
        out[p] = icon
    return out


_special_icon_cache: dict[str, str] = {}

# ---------------------------------------------------------------------------
# Errors + validation
# ---------------------------------------------------------------------------


def os_error(e: OSError, name: str = "") -> ApiError:
    n = f"‘{name}’" if name else "dit item"
    code = e.errno
    if code in (errno.EACCES, errno.EPERM):
        return ApiError("no_access", f"Je hebt geen toegang tot {n}.")
    if code == errno.ENOENT:
        return ApiError("not_found", f"{n[:1].upper() + n[1:]} bestaat niet (meer).")
    if code == errno.ENOSPC:
        return ApiError("no_space", "Er is niet genoeg ruimte op de schijf.")
    if code == errno.EROFS:
        return ApiError("read_only", "Deze schijf is alleen-lezen.")
    if code == errno.EEXIST:
        return ApiError("exists", "Er bestaat al een item met deze naam.")
    if code == errno.ENAMETOOLONG:
        return ApiError("name_too_long", "Deze naam is te lang.")
    if code == errno.EBUSY:
        return ApiError("busy", f"{n[:1].upper() + n[1:]} is in gebruik.")
    if code == errno.ENOTDIR:
        return ApiError("not_dir", "Dit is geen map.")
    if code == errno.EINVAL:
        return ApiError("bad_name", "Deze naam kan niet op deze schijf.")
    return ApiError("failed", f"Dat lukte niet met {n}.", str(e))


def _abs(p: Any) -> str:
    if not isinstance(p, str) or not p or "\0" in p or len(p) > 4096:
        raise ApiError("bad_args", "Ongeldig pad.")
    if p.startswith("file://"):
        p = unquote(p[7:])
    p = os.path.expanduser(p)
    if not os.path.isabs(p):
        raise ApiError("bad_args", "Ongeldig pad.")
    p = os.path.normpath(p)
    return "/" + p.lstrip("/") if p.startswith("//") else p


def _paths(v: Any, max_n: int = 10_000) -> list[str]:
    if isinstance(v, str):
        v = [v]
    if not isinstance(v, list) or not v or len(v) > max_n:
        raise ApiError("bad_args", "Kies eerst een of meer items.")
    out, seen = [], set()
    for p in v:
        a = _abs(p)
        if a not in seen:
            seen.add(a)
            out.append(a)
    return out


def _dir(p: Any) -> str:
    d = _abs(p)
    if not os.path.isdir(d):
        raise ApiError("not_found", "Deze map bestaat niet (meer).")
    return d


def validate_name(new: Any) -> str:
    if not isinstance(new, str):
        raise ApiError("bad_name", "Geef een naam op.")
    n = new.strip()
    if not n:
        raise ApiError("bad_name", "Geef een naam op.")
    if "/" in n or "\0" in n:
        raise ApiError("bad_name", "Een naam mag geen schuine streep (/) bevatten.")
    if n in (".", ".."):
        raise ApiError("bad_name", "Deze naam kan niet. Kies een andere naam.")
    if any(ord(c) < 32 for c in n):
        raise ApiError("bad_name", "Deze naam bevat ongeldige tekens.")
    if len(n.encode("utf-8")) > 255:
        raise ApiError("name_too_long", "Deze naam is te lang.")
    return n


_COMPOUND_EXT = (".tar.gz", ".tar.xz", ".tar.bz2", ".tar.zst", ".tar.lz", ".tar.lzma")


def split_name(name: str) -> tuple[str, str]:
    low = name.lower()
    for ce in _COMPOUND_EXT:
        if low.endswith(ce) and len(name) > len(ce):
            return name[: -len(ce)], name[-len(ce):]
    stem, ext = os.path.splitext(name)
    if not stem:  # ".bashrc"
        return name, ""
    return stem, ext


def unique_path(directory: str, name: str, is_dir: bool = False) -> str:
    """'naam.ext' → 'naam (2).ext', 'naam (3).ext' … (first free name)."""
    target = os.path.join(directory, name)
    if not os.path.lexists(target):
        return target
    stem, ext = (name, "") if is_dir else split_name(name)
    m = re.match(r"^(.*) \((\d+)\)$", stem)
    base, start = (m.group(1), int(m.group(2)) + 1) if m else (stem, 2)
    for i in range(start, start + 100_000):
        cand = os.path.join(directory, f"{base} ({i}){ext}")
        if not os.path.lexists(cand):
            return cand
    raise ApiError("exists", "Er bestaat al een item met deze naam.")


def copy_path(directory: str, name: str, is_dir: bool = False, word: str = "kopie") -> str:
    """Windows naming for a copy in the same folder: 'naam - kopie.ext', 'naam - kopie (2).ext' …"""
    stem, ext = (name, "") if is_dir else split_name(name)
    base = f"{stem} - {word}"
    cand = os.path.join(directory, base + ext)
    if not os.path.lexists(cand):
        return cand
    for i in range(2, 100_000):
        cand = os.path.join(directory, f"{base} ({i}){ext}")
        if not os.path.lexists(cand):
            return cand
    raise ApiError("exists", "Er bestaat al een item met deze naam.")


def _inside(child: str, parent: str) -> bool:
    try:
        c, p = os.path.realpath(child), os.path.realpath(parent)
        return c == p or os.path.commonpath([c, p]) == p
    except ValueError:
        return False


# ---------------------------------------------------------------------------
# Trash (freedesktop.org spec)
# ---------------------------------------------------------------------------

_DISK_FS = {"ext2", "ext3", "ext4", "xfs", "btrfs", "f2fs", "jfs", "reiserfs", "zfs", "ntfs", "ntfs3", "fuseblk",
            "exfat", "vfat", "msdos", "hfsplus", "bcachefs", "nilfs2"}


def home_trash() -> Path:
    o = os.environ.get("WIN11SHELL_TRASH_DIR")
    return Path(o) if o else xdg_data_home() / "Trash"


def _mount_points() -> list[str]:
    out = []
    try:
        with open("/proc/self/mounts", encoding="utf-8", errors="replace") as f:
            for line in f:
                parts = line.split()
                if len(parts) >= 3 and parts[2] in _DISK_FS:
                    out.append(parts[1].replace("\\040", " ").replace("\\011", "\t").replace("\\134", "\\"))
    except OSError:
        pass
    return out


def _own_real_dir(p: str | os.PathLike) -> bool:
    """A real directory (not a symlink) owned by this user."""
    try:
        st = os.lstat(p)
    except OSError:
        return False
    return stat.S_ISDIR(st.st_mode) and st.st_uid == os.getuid()


def valid_trash_dir(tdir: Path, home: bool = False) -> bool:
    """Is ``tdir`` a trash dir we may list, restore from and empty? (freedesktop spec + GLib rules)

    ``files`` (and ``info`` when present) must be real directories owned by the user — a symlinked
    ``files`` on a crafted USB stick could otherwise make "Prullenbak leegmaken" wipe any folder. A
    topdir trash (``.Trash-<uid>`` / ``.Trash/<uid>``) must itself be a real, own directory too."""
    if not home and not _own_real_dir(tdir):
        return False
    if not _own_real_dir(tdir / "files"):
        return False
    info = tdir / "info"
    return not os.path.lexists(info) or _own_real_dir(info)


def trash_dirs() -> list[tuple[Path, str | None]]:
    """[(trash dir, topdir or None for the home trash)] that exist and pass ``valid_trash_dir``."""
    home = home_trash()
    out: list[tuple[Path, str | None]] = []
    if valid_trash_dir(home, home=True):
        out.append((home, None))
    if os.environ.get("WIN11SHELL_TRASH_DIR"):
        return out
    uid = os.getuid()
    seen = {str(home.resolve()) if home.exists() else str(home)}
    for mp in _mount_points():
        for cand in (Path(mp) / ".Trash" / str(uid), Path(mp) / f".Trash-{uid}"):
            try:
                if not valid_trash_dir(cand):
                    continue
                if cand.parent.name == ".Trash":
                    pst = os.lstat(cand.parent)
                    if stat.S_ISLNK(pst.st_mode) or not (pst.st_mode & stat.S_ISVTX):
                        continue
                key = str(cand.resolve())
            except OSError:
                continue
            if key not in seen:
                seen.add(key)
                out.append((cand, mp))
    return out


def parse_trashinfo(path: Path) -> dict:
    out = {"path": None, "deleted": 0}
    try:
        text = path.read_text(encoding="utf-8", errors="replace")
    except OSError:
        return out
    section = False
    for line in text.splitlines():
        line = line.strip()
        if line.startswith("["):
            section = line == "[Trash Info]"
            continue
        if not section or "=" not in line:
            continue
        k, v = line.split("=", 1)
        if k == "Path":
            out["path"] = unquote(v)
        elif k == "DeletionDate":
            try:
                out["deleted"] = int(datetime.fromisoformat(v).timestamp() * 1000)
            except ValueError:
                pass
    return out


def _trash_entries() -> list[dict]:
    """Every top-level item in every trash dir: {tdir, topdir, name, path, original, deleted}."""
    out = []
    for tdir, top in trash_dirs():
        files = tdir / "files"
        try:
            with os.scandir(files) as it:
                names = [e.name for e in it]
        except OSError:
            continue
        for n in names:
            info = parse_trashinfo(tdir / "info" / (n + ".trashinfo"))
            out.append({"tdir": tdir, "topdir": top, "name": n, "path": str(files / n),
                        "original": original_path(info["path"], top), "deleted": info["deleted"]})
    return out


def original_path(path: str | None, top: str | None) -> str | None:
    """Absolute original location of a trash entry. For a trash on another drive (``top``) it must lie
    on that drive: a crafted ``.trashinfo`` (``Path=/home/me/.config/autostart/x.desktop`` or
    ``../../..``) may never make "Terugzetten" write outside it. Returns None when not acceptable."""
    if not path:
        return None
    if top is None:
        return os.path.normpath(path) if path.startswith("/") else None
    full = os.path.normpath(path if path.startswith("/") else os.path.join(top, path))
    topn = os.path.normpath(top)
    try:
        if os.path.commonpath([full, topn]) != topn or full == topn:
            return None
    except ValueError:
        return None
    return full


def trash_count() -> int:
    n = 0
    for tdir, _top in trash_dirs():
        try:
            with os.scandir(tdir / "files") as it:
                n += sum(1 for _ in it)
        except OSError:
            continue
    return n


def _du(path: str, deadline: float) -> tuple[int, int, int, bool]:
    """(bytes, files, folders, complete) of a tree, stopping at ``deadline`` (monotonic)."""
    size = files = folders = 0
    complete = True
    try:
        lst = os.lstat(path)
    except OSError:
        return 0, 0, 0, True
    if not stat.S_ISDIR(lst.st_mode):
        return lst.st_size, 1, 0, True
    dev = lst.st_dev
    stack = [path]
    n = 0
    while stack:
        d = stack.pop()
        try:
            with os.scandir(d) as it:
                for e in it:
                    n += 1
                    if n % 512 == 0 and time.monotonic() > deadline:
                        return size, files, folders, False
                    try:
                        st = e.stat(follow_symlinks=False)
                    except OSError:
                        continue
                    if stat.S_ISDIR(st.st_mode):
                        folders += 1
                        if st.st_dev == dev:
                            stack.append(e.path)
                    else:
                        files += 1
                        size += st.st_size
        except OSError:
            continue
        if time.monotonic() > deadline:
            complete = not stack
            if stack:
                return size, files, folders, False
    return size, files, folders, complete


def _rm_tree(path: str) -> None:
    """Remove a file/dir/symlink (never follows symlinks)."""
    parent, name = os.path.split(os.path.abspath(path))
    dfd = os.open(parent or "/", os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC)
    try:
        rm_at(dfd, name)
    finally:
        os.close(dfd)


def _make_writable_at(dfd: int, name: str, depth: int = 0) -> None:
    """chmod u+rwx every own directory below ``name`` (relative to ``dfd``) so it can be removed.
    Works through O_PATH fds, so a directory swapped for a symlink is never followed."""
    try:
        pfd = os.open(name, os.O_PATH | os.O_NOFOLLOW | os.O_DIRECTORY | os.O_CLOEXEC, dir_fd=dfd)
    except OSError:
        return
    try:
        st = os.fstat(pfd)
        if st.st_uid == os.getuid() and (st.st_mode & 0o700) != 0o700:
            os.chmod(f"/proc/self/fd/{pfd}", (st.st_mode | 0o700) & 0o7777)
    except OSError:
        pass
    finally:
        os.close(pfd)
    if depth > 256:
        return
    try:
        fd = os.open(name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC, dir_fd=dfd)
    except OSError:
        return
    try:
        with os.scandir(fd) as it:
            subs = [e.name for e in it if e.is_dir(follow_symlinks=False)]
        for n in subs:
            _make_writable_at(fd, n, depth + 1)
    finally:
        os.close(fd)


def rm_at(dfd: int, name: str) -> None:
    """Permanently remove ``name`` inside the directory open as ``dfd``. Never follows symlinks and
    never leaves that directory, even when a component is swapped while it runs."""
    if not name or name in (".", "..") or "/" in name:
        raise OSError(errno.EINVAL, "bad name", name)
    st = os.stat(name, dir_fd=dfd, follow_symlinks=False)
    if not stat.S_ISDIR(st.st_mode):
        os.unlink(name, dir_fd=dfd)
        return
    try:
        shutil.rmtree(name, dir_fd=dfd)
    except OSError:
        _make_writable_at(dfd, name)          # read-only folders: make them writable once, retry
        shutil.rmtree(name, dir_fd=dfd)


def empty_trash_dirs(dirs: list[tuple[Path, str | None]] | None = None) -> tuple[int, int, list[str]]:
    """Empty every (valid) trash dir → (removed items, failed items, failed paths).

    Each trash dir is opened with O_NOFOLLOW and items are deleted by name relative to that fd, so a
    symlinked or swapped ``files`` folder can never redirect the deletion."""
    removed = failed = 0
    errors: list[str] = []
    flags = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC
    for tdir, top in (trash_dirs() if dirs is None else dirs):
        if not valid_trash_dir(tdir, home=top is None):
            continue
        try:
            ffd = os.open(tdir / "files", flags)
        except OSError:
            continue
        ifd = None
        try:
            if os.fstat(ffd).st_uid != os.getuid():
                continue
            try:
                ifd = os.open(tdir / "info", flags)
                if os.fstat(ifd).st_uid != os.getuid():
                    os.close(ifd)
                    ifd = None
            except OSError:
                ifd = None
            for n in os.listdir(ffd):
                try:
                    rm_at(ffd, n)
                    removed += 1
                    if ifd is not None:
                        try:
                            os.unlink(n + ".trashinfo", dir_fd=ifd)
                        except OSError:
                            pass
                except OSError as e:
                    failed += 1
                    errors.append(str(tdir / "files" / n))
                    log.warning("Kan %s niet verwijderen (cannot delete): %s", tdir / "files" / n, e)
            if ifd is not None:   # orphaned info files
                for n in os.listdir(ifd):
                    if n.endswith(".trashinfo"):
                        try:
                            os.stat(n[:-10], dir_fd=ffd, follow_symlinks=False)
                        except FileNotFoundError:
                            try:
                                os.unlink(n, dir_fd=ifd)
                            except OSError:
                                pass
                        except OSError:
                            pass
        except OSError as e:
            log.warning("Prullenbak %s niet leeg te maken (cannot empty): %s", tdir, e)
        finally:
            os.close(ffd)
            if ifd is not None:
                os.close(ifd)
        try:
            tfd = os.open(tdir, flags)
            try:
                os.unlink("directorysizes", dir_fd=tfd)
            except OSError:
                pass
            try:   # GVfs' half-deleted leftovers
                efd = os.open("expunged", flags, dir_fd=tfd)
                try:
                    if os.fstat(efd).st_uid == os.getuid():
                        for n in os.listdir(efd):
                            try:
                                rm_at(efd, n)
                            except OSError:
                                pass
                finally:
                    os.close(efd)
            except OSError:
                pass
            finally:
                os.close(tfd)
        except OSError:
            pass
    return removed, failed, errors


_RENAME_NOREPLACE = 1
_renameat2_fn: Any = False


def _renameat2():
    global _renameat2_fn
    if _renameat2_fn is False:
        try:
            libc = ctypes.CDLL(None, use_errno=True)
            fn = libc.renameat2
            fn.argtypes = [ctypes.c_int, ctypes.c_char_p, ctypes.c_int, ctypes.c_char_p, ctypes.c_uint]
            fn.restype = ctypes.c_int
            _renameat2_fn = fn
        except (OSError, AttributeError):
            _renameat2_fn = None
    return _renameat2_fn


def rename_noreplace(src: str, dst: str) -> None:
    """``os.rename`` that never replaces an existing ``dst`` (raises FileExistsError instead).

    renameat2(RENAME_NOREPLACE) where the filesystem supports it; otherwise a hard link + unlink for
    files (also atomic), or a last-moment existence check (FAT/NTFS-3g directories)."""
    fn = _renameat2()
    if fn is not None:
        if fn(-100, os.fsencode(src), -100, os.fsencode(dst), _RENAME_NOREPLACE) == 0:
            return
        err = ctypes.get_errno()
        if err not in (errno.EINVAL, errno.ENOSYS, errno.EOPNOTSUPP, errno.ENOTSUP):
            raise OSError(err, os.strerror(err), src, None, dst)
    try:
        is_dir = stat.S_ISDIR(os.lstat(src).st_mode)
    except OSError:
        is_dir = False
    if not is_dir:
        try:
            os.link(src, dst, follow_symlinks=False)
        except FileExistsError:
            raise
        except OSError:
            pass                      # no hard links here (FAT, exFAT, protected_hardlinks): fall through
        else:
            os.unlink(src)
            return
    if os.path.lexists(dst):
        raise FileExistsError(errno.EEXIST, os.strerror(errno.EEXIST), dst)
    os.rename(src, dst)


def move_noreplace(src: str, dst: str) -> None:
    """Move without ever overwriting: rename_noreplace, or copy + remove across filesystems."""
    try:
        rename_noreplace(src, dst)
    except OSError as e:
        if e.errno != errno.EXDEV:
            raise
        if os.path.lexists(dst):
            raise FileExistsError(errno.EEXIST, os.strerror(errno.EEXIST), dst) from None
        st = os.lstat(src)
        if stat.S_ISDIR(st.st_mode):
            shutil.copytree(src, dst, symlinks=True)        # fails when dst appeared meanwhile
        elif stat.S_ISLNK(st.st_mode):
            os.symlink(os.readlink(src), dst)
        else:
            with open(src, "rb") as fi:
                fd = os.open(dst, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_CLOEXEC, 0o600)
                with os.fdopen(fd, "wb") as fo:
                    shutil.copyfileobj(fi, fo, 1 << 20)
            shutil.copystat(src, dst)
        _rm_tree(src)


def _publish_noreplace(tmp: str, target: str, directory: str, is_dir: bool) -> str:
    """Give a finished ``.partial`` result its final name without ever replacing something that appeared
    there meanwhile ("naam (2).zip" then) → the final path."""
    for _attempt in range(50):
        try:
            rename_noreplace(tmp, target)
            return target
        except FileExistsError:
            target = unique_path(directory, os.path.basename(target), is_dir=is_dir)
    raise FileExistsError(errno.EEXIST, os.strerror(errno.EEXIST), target)


def _manual_trash(path: str) -> str:
    """Test mode (WIN11SHELL_TRASH_DIR): move into the sandbox trash ourselves → name inside Trash/files."""
    t = home_trash()
    (t / "files").mkdir(parents=True, exist_ok=True)
    (t / "info").mkdir(parents=True, exist_ok=True)
    base = os.path.basename(path)
    dest = unique_path(str(t / "files"), base)
    name = os.path.basename(dest)
    info = t / "info" / (name + ".trashinfo")
    info.write_text("[Trash Info]\nPath=" + quote(path, safe="/") + "\nDeletionDate="
                    + datetime.now().strftime("%Y-%m-%dT%H:%M:%S") + "\n", encoding="utf-8")
    try:
        rename_noreplace(path, dest)
    except OSError:
        info.unlink(missing_ok=True)
        raise
    return name


def trash_path(p: str) -> str | None:
    """Move one item to the Prullenbak (never deletes) → its name inside Trash/files when known
    (sandbox trash), else None. Raises ApiError with a Dutch message."""
    if os.environ.get("WIN11SHELL_TRASH_DIR"):
        return _manual_trash(p)
    if Gio is None:
        raise OSError(errno.ENOTSUP, "no gio")
    try:
        Gio.File.new_for_path(p).trash(None)
    except GLib.Error as e:
        if e.matches(Gio.io_error_quark(), Gio.IOErrorEnum.NOT_SUPPORTED):
            raise ApiError("no_trash", "Deze schijf heeft geen Prullenbak. Het item is niet verwijderd.") from None
        if e.matches(Gio.io_error_quark(), Gio.IOErrorEnum.PERMISSION_DENIED):
            raise ApiError("no_access", "Je hebt geen toegang om dit item te verwijderen.") from None
        if e.matches(Gio.io_error_quark(), Gio.IOErrorEnum.NOT_FOUND):
            raise ApiError("not_found", "Dit item bestaat niet (meer).") from None
        raise ApiError("failed", "Dit item kon niet naar de Prullenbak.", e.message) from None
    return None


def _mount_top(path: str, dev: int) -> str:
    """Top directory of the filesystem that holds ``path`` (where its .Trash-<uid> lives)."""
    cur = os.path.dirname(os.path.abspath(path))
    while cur != "/":
        parent = os.path.dirname(cur)
        try:
            if os.stat(parent).st_dev != dev:
                return cur
        except OSError:
            return cur
        cur = parent
    return "/"


def _trash_dirs_for(paths: list[str]) -> list[tuple[Path, str | None]]:
    """The trash dirs these items will land in (home trash, or <mount>/.Trash/<uid> | .Trash-<uid>).
    Only these are looked at, so other (sleeping) drives are not woken up."""
    home = home_trash()
    try:
        home_dev = os.stat(home if home.exists() else home.parent).st_dev
    except OSError:
        home_dev = None
    uid = os.getuid()
    out: dict[str, tuple[Path, str | None]] = {}
    for p in paths:
        try:
            dev = os.lstat(p).st_dev
        except OSError:
            continue
        if os.environ.get("WIN11SHELL_TRASH_DIR") or dev == home_dev:
            out[str(home)] = (home, None)
            continue
        top = _mount_top(p, dev)
        for cand in (Path(top) / ".Trash" / str(uid), Path(top) / f".Trash-{uid}"):
            out[str(cand)] = (cand, top)
    return list(out.values())


def _trash_info_names(paths: list[str]) -> dict[str, tuple[Path, str | None, set[str]]]:
    """{trash dir: (dir, topdir, names of its info files)} — a cheap snapshot (no parsing) taken before
    trashing, to find out afterwards which names Gio gave the items."""
    out: dict[str, tuple[Path, str | None, set[str]]] = {}
    for tdir, top in _trash_dirs_for(paths):
        try:
            names = set(os.listdir(tdir / "info"))
        except OSError:
            names = set()
        out[str(tdir)] = (tdir, top, names)
    return out


def _new_trash_entries(before: dict[str, tuple[Path, str | None, set[str]]], originals: set[str]) -> dict[str, dict]:
    """Entries added to those trash dirs since ``before`` whose original path is in ``originals``:
    {original path: {name, trashPath, deleted}} (newest wins)."""
    found: dict[str, dict] = {}
    for tdir, top, old in before.values():
        try:
            names = set(os.listdir(tdir / "info")) - old
        except OSError:
            continue
        for info_name in names:
            if not info_name.endswith(".trashinfo"):
                continue
            n = info_name[:-10]
            info = parse_trashinfo(tdir / "info" / info_name)
            orig = info["path"]
            if orig and not orig.startswith("/") and top:
                orig = os.path.join(top, orig)
            if not orig:
                continue
            orig = os.path.normpath(orig)
            if orig in originals and (orig not in found or info["deleted"] >= found[orig]["deleted"]):
                found[orig] = {"name": n, "trashPath": str(tdir / "files" / n), "deleted": info["deleted"]}
    return found


# ---------------------------------------------------------------------------
# Copy / move with progress
# ---------------------------------------------------------------------------

class Progress:
    def __init__(self, api, op: str, job: str | None, total: int | None, count: int):
        self.api = api
        self.op = op
        self.job = job or uuid.uuid4().hex[:12]
        self.total = total
        self.count = count
        self.done = 0
        self.items = 0
        self.last = 0.0
        self.current = ""

    def emit(self, finished: bool = False, force: bool = False):
        now = time.monotonic()
        if not finished and not force and now - self.last < 0.25:
            return
        self.last = now
        self.api.emit("files.progress", {"jobId": self.job, "op": self.op, "done": self.done, "total": self.total,
                                         "items": self.items, "count": self.count, "current": self.current,
                                         "finished": finished})

    def add(self, n: int):
        self.done += n
        self.emit()


_CHUNK = 1 << 20


def _copy_file(src: str, dst: str, prog: Progress) -> None:
    with open(src, "rb") as fi:
        fd = os.open(dst, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_CLOEXEC, 0o600)
        try:
            with os.fdopen(fd, "wb") as fo:
                while True:
                    b = fi.read(_CHUNK)
                    if not b:
                        break
                    fo.write(b)
                    prog.add(len(b))
        except BaseException:
            try:
                os.unlink(dst)
            except OSError:
                pass
            raise
    try:
        shutil.copystat(src, dst)
    except OSError:
        pass


def _copy_any(src: str, dst: str, prog: Progress) -> None:
    st = os.lstat(src)
    if stat.S_ISLNK(st.st_mode):
        os.symlink(os.readlink(src), dst)
    elif stat.S_ISDIR(st.st_mode):
        os.mkdir(dst, 0o700)
        try:
            with os.scandir(src) as it:
                entries = list(it)
            for e in entries:
                prog.current = e.name
                _copy_any(e.path, os.path.join(dst, e.name), prog)
            shutil.copystat(src, dst)
        except BaseException:
            shutil.rmtree(dst, ignore_errors=True)
            raise
    elif stat.S_ISREG(st.st_mode):
        _copy_file(src, dst, prog)
    else:
        raise OSError(errno.EINVAL, "special file")


def _total_size(paths: list[str], budget: float = 1.5) -> int | None:
    deadline = time.monotonic() + budget
    total = 0
    for p in paths:
        s, _f, _d, complete = _du(p, deadline)
        total += s
        if not complete:
            return None
    return total


def transfer(api, paths: list[str], dest: str, move: bool, job: str | None = None, copy_word: str = "kopie") -> dict:
    """Copy or move ``paths`` into ``dest`` with conflict-safe names ('naam (2).ext'; a copy into its own
    folder is 'naam - kopie.ext' like Windows). Never overwrites."""
    if not os.path.isdir(dest):
        raise ApiError("not_found", "De doelmap bestaat niet (meer).")
    prog = Progress(api, "move" if move else "copy", job, _total_size(paths), len(paths))
    prog.emit(force=True)
    done_paths, failed = [], []
    try:
        dest_dev = os.stat(dest).st_dev
        for src in paths:
            name = os.path.basename(src)
            prog.current = name
            try:
                lst = os.lstat(src)
            except OSError as e:
                failed.append({"path": src, "error": os_error(e, name).message})
                continue
            is_dir = stat.S_ISDIR(lst.st_mode)
            if is_dir and _inside(dest, src):
                failed.append({"path": src, "error": "Je kunt een map niet in zichzelf plaatsen."})
                continue
            if move and os.path.dirname(src) == dest:
                done_paths.append(src)       # already there: nothing to do
                continue
            try:
                for attempt in range(20):
                    if not move and os.path.dirname(src) == dest:
                        target = copy_path(dest, name, is_dir=is_dir, word=copy_word)
                    else:
                        target = unique_path(dest, name, is_dir=is_dir)
                    try:
                        renamed = False
                        if move and lst.st_dev == dest_dev:
                            try:
                                rename_noreplace(src, target)     # never overwrites
                                renamed = True
                                prog.add(0 if is_dir else lst.st_size)
                            except OSError as e:
                                if e.errno != errno.EXDEV:   # bind mounts: same device, still EXDEV
                                    raise
                        if not renamed:
                            _copy_any(src, target, prog)      # O_EXCL / mkdir: never overwrites either
                            if move:
                                _rm_tree(src)
                        break
                    except FileExistsError:
                        if attempt == 19:
                            raise
                        continue      # an item with that name appeared meanwhile: take the next free name
                done_paths.append(target)
                prog.items += 1
            except OSError as e:
                failed.append({"path": src, "error": os_error(e, name).message})
    finally:
        prog.emit(finished=True)
    return {"ok": not failed, "paths": done_paths, "failed": failed, "jobId": prog.job}


# ---------------------------------------------------------------------------
# Archives
# ---------------------------------------------------------------------------

def _zip_tree(zf: zipfile.ZipFile, path: str, arc: str, prog: Progress) -> None:
    st = os.lstat(path)
    if stat.S_ISLNK(st.st_mode):
        try:
            st = os.stat(path)
        except OSError:
            return
        if stat.S_ISDIR(st.st_mode):
            return  # never follow directory symlinks (loops)
    if stat.S_ISDIR(st.st_mode):
        zf.write(path, arc + "/")
        with os.scandir(path) as it:
            entries = sorted(it, key=lambda e: e.name)
        for e in entries:
            _zip_tree(zf, e.path, f"{arc}/{e.name}", prog)
    elif stat.S_ISREG(st.st_mode):
        prog.current = os.path.basename(path)
        zf.write(path, arc)
        prog.add(st.st_size)


_ARCHIVE_EXTS = (".zip", ".tar", ".tar.gz", ".tgz", ".tar.bz2", ".tbz2", ".tar.xz", ".txz", ".tar.zst", ".tzst")


def _archive_stem(name: str) -> str:
    low = name.lower()
    for ext in sorted(_ARCHIVE_EXTS, key=len, reverse=True):
        if low.endswith(ext) and len(name) > len(ext):
            return name[: -len(ext)]
    return split_name(name)[0]


def _safe_member(name: str) -> bool:
    n = name.replace("\\", "/")
    if n.startswith("/") or re.match(r"^[A-Za-z]:", n):
        return False
    parts = [p for p in n.split("/") if p not in ("", ".")]
    return ".." not in parts


def check_archive(path: str) -> tuple[str, int]:
    """('zip'|'tar', total uncompressed bytes). Raises ApiError for unsafe/unsupported archives."""
    if zipfile.is_zipfile(path):
        with zipfile.ZipFile(path) as zf:
            total = 0
            for info in zf.infolist():
                if not _safe_member(info.filename):
                    raise ApiError("unsafe_archive", "Dit archief bevat onveilige paden en wordt niet uitgepakt.")
                total += info.file_size
        return "zip", total
    try:
        is_tar = tarfile.is_tarfile(path)
    except OSError:
        is_tar = False
    if is_tar:
        try:
            with tarfile.open(path) as tf:
                total = 0
                for m in tf:
                    if not _safe_member(m.name):
                        raise ApiError("unsafe_archive", "Dit archief bevat onveilige paden en wordt niet uitgepakt.")
                    if m.issym() or m.islnk():
                        target = m.linkname
                        joined = os.path.normpath(os.path.join(os.path.dirname(m.name), target)) if m.issym() else target
                        if target.startswith("/") or not _safe_member(joined):
                            raise ApiError("unsafe_archive",
                                           "Dit archief bevat onveilige koppelingen en wordt niet uitgepakt.")
                    if m.isdev():
                        raise ApiError("unsafe_archive", "Dit archief bevat apparaatbestanden en wordt niet uitgepakt.")
                    total += max(0, m.size)
        except (tarfile.TarError, EOFError, OSError) as e:
            raise ApiError("bad_archive", "Dit archief is beschadigd of onvolledig.", str(e)) from None
        return "tar", total
    raise ApiError("unsupported_archive", "Dit soort archief kan Win11 Shell niet uitpakken. "
                                          "Open het met Archiefbeheer.")


# ---------------------------------------------------------------------------
# Directory watching
# ---------------------------------------------------------------------------

class Watches:
    MAX = 64

    def __init__(self, api, ctx):
        self.api = api
        self.ctx = ctx
        self.lock = threading.Lock()
        self.mons: dict[str, list] = {}      # path → [monitor, refcount, last_used]
        self.debs: dict[str, Debouncer] = {}
        self.desktop = str(ctx.desktop_dir)

    def notify(self, path: str) -> None:
        with self.lock:
            deb = self.debs.get(path)
            if deb is None:
                if len(self.debs) > 256:
                    for d in self.debs.values():
                        d.close()
                    self.debs.clear()
                deb = Debouncer(0.3, lambda p=path: self._emit(p), name="files-dir-changed")
                self.debs[path] = deb
        deb.trigger()

    def _emit(self, path: str) -> None:
        self.api.emit("files.dirChanged", {"path": path})
        if path == self.desktop:
            self.api.emit("files.desktopChanged", {})

    def changed(self, *paths: str) -> None:
        """Our own operations: tell the UI right away (also without Gio monitors)."""
        for p in {os.path.normpath(p) for p in paths if p}:
            self.notify(p)

    def watch(self, path: str, pinned: bool = False) -> bool:
        if Gio is None:
            return False
        with self.lock:
            ent = self.mons.get(path)
            if ent is not None:
                ent[1] += 1
                ent[2] = time.monotonic()
                return True
            if len(self.mons) >= self.MAX:
                victims = sorted((v[2], k) for k, v in self.mons.items() if k != self.desktop)
                if victims:
                    self._drop(victims[0][1])

        def create():
            mon = Gio.File.new_for_path(path).monitor_directory(Gio.FileMonitorFlags.WATCH_MOVES, None)
            mon.set_rate_limit(300)
            mon.connect("changed", lambda *_a: self.notify(path))
            return mon

        try:
            mon = self.ctx.run_on_main(create)
        except GLib.Error as e:
            log.debug("Kan %s niet volgen (cannot watch): %s", path, e.message)
            return False
        with self.lock:
            self.mons[path] = [mon, 1 if not pinned else 10**6, time.monotonic()]
        return True

    def _drop(self, path: str) -> None:
        ent = self.mons.pop(path, None)
        if ent:
            mon = ent[0]
            self.ctx.run_on_main(mon.cancel, wait=False)

    def unwatch(self, path: str) -> None:
        with self.lock:
            ent = self.mons.get(path)
            if ent is None or path == self.desktop:
                return
            ent[1] -= 1
            if ent[1] <= 0:
                self._drop(path)

    def close(self) -> None:
        with self.lock:
            for p in list(self.mons):
                self._drop(p)
            for d in self.debs.values():
                d.close()
            self.debs.clear()


# ---------------------------------------------------------------------------
# Service
# ---------------------------------------------------------------------------

def _places(ctx, lang: str, trash_count: int) -> list[dict]:
    nl = lang == "nl"
    ui = user_info()
    rows = [
        ("home", ui.get("realName") or ("Persoonlijke map" if nl else "Home"), str(ctx.home), ["user-home"]),
        ("desktop", "Bureaublad" if nl else "Desktop", str(ctx.desktop_dir), ["user-desktop"]),
        ("documents", "Documenten" if nl else "Documents", None, ["folder-documents"]),
        ("downloads", "Downloads", None, ["folder-download", "folder-downloads"]),
        ("music", "Muziek" if nl else "Music", None, ["folder-music"]),
        ("pictures", "Afbeeldingen" if nl else "Pictures", None, ["folder-pictures"]),
        ("videos", "Video's" if nl else "Videos", None, ["folder-videos"]),
        ("trash", "Prullenbak" if nl else "Recycle Bin", TRASH_URI,
         ["user-trash-full", "user-trash"] if trash_count else ["user-trash"]),
        ("root", "Systeemschijf" if nl else "System drive", "/", ["drive-harddisk", "drive-harddisk-system"]),
    ]
    kinds = {"documents": "DIRECTORY_DOCUMENTS", "downloads": "DIRECTORY_DOWNLOAD", "music": "DIRECTORY_MUSIC",
             "pictures": "DIRECTORY_PICTURES", "videos": "DIRECTORY_VIDEOS"}
    xdg = {"documents": "DOCUMENTS", "downloads": "DOWNLOAD", "music": "MUSIC", "pictures": "PICTURES",
           "videos": "VIDEOS"}
    out = []
    for pid, name, path, icons in rows:
        if path is None:
            path = None
            if GLib is not None:
                try:
                    path = GLib.get_user_special_dir(getattr(GLib.UserDirectory, kinds[pid]))
                except Exception:  # noqa: BLE001
                    path = None
            path = path or str(xdg_user_dir(xdg[pid]))
            if not path or os.path.normpath(path) == str(ctx.home):
                continue
        exists = path == TRASH_URI or os.path.isdir(path)
        if not exists and pid not in ("home", "trash", "root", "desktop"):
            continue
        out.append({"id": pid, "name": name, "path": path, "icon": first_existing_icon(icons, "folder") or "folder",
                    "exists": exists})
    return out


def register(api, ctx) -> None:
    watches = Watches(api, ctx)
    clip = {"paths": [], "op": "copy", "time": 0}
    clip_lock = threading.Lock()
    trash_cache: dict[str, Any] = {"key": None, "value": None, "at": 0.0}
    ctx._files_watches = watches

    def factory() -> ItemFactory:
        return ItemFactory(ctx)

    def lang_defaults() -> dict:
        nl = _lang(ctx) == "nl"
        return {"folder": "Nieuwe map" if nl else "New folder",
                "text": "Nieuw tekstdocument.txt" if nl else "New Text Document.txt",
                "empty": "Nieuw bestand" if nl else "New file"}

    def copy_word() -> str:
        return "kopie" if _lang(ctx) == "nl" else "Copy"

    def dirs_of(*paths: str) -> list[str]:
        return [os.path.dirname(p) for p in paths]

    # -- listing --------------------------------------------------------------------
    @api.method("files.desktop")
    def _desktop(args):
        """{dir, items} of the desktop folder."""
        d = str(ctx.desktop_dir)
        if not os.path.isdir(d):
            return {"dir": d, "items": [], "exists": False}
        try:
            items, truncated = factory().listdir(d, bool(args.get("showHidden")), limit=5000)
        except OSError as e:
            raise os_error(e, os.path.basename(d)) from None
        return {"dir": d, "items": items, "exists": True, "truncated": truncated}

    def list_trash() -> dict:
        f = factory()
        items = []
        for ent in _trash_entries():
            try:
                it = f.make(ent["path"])
            except OSError:
                continue
            orig = ent["original"] or ""
            it["displayName"] = os.path.basename(orig) if orig and not it["isDesktopEntry"] else it["displayName"]
            it["hidden"] = False
            it.update({"originalPath": orig, "originalDir": os.path.dirname(orig) if orig else "",
                       "deletedAt": ent["deleted"], "trashName": ent["name"], "inTrash": True})
            items.append(it)
        items.sort(key=lambda i: -(i.get("deletedAt") or 0))
        return {"path": TRASH_URI, "parent": None, "items": items, "writable": False, "trash": True}

    @api.method("files.list")
    def _list(args):
        """{path, parent, items} of a folder (``trash:///`` lists the Prullenbak)."""
        raw = args.get("path")
        if isinstance(raw, str) and raw.startswith("trash:"):
            return list_trash()
        path = _abs(raw if raw else str(ctx.home))
        try:
            st = os.stat(path)
        except OSError as e:
            if e.errno == errno.ENOENT:
                raise ApiError("not_found", "Deze map bestaat niet (meer).") from None
            raise os_error(e, os.path.basename(path) or path) from None
        if not stat.S_ISDIR(st.st_mode):
            raise ApiError("not_dir", "Dit is geen map.")
        show_hidden = args.get("showHidden")
        if show_hidden is None:
            show_hidden = bool(ctx.settings.get("explorer.showHidden", False))
        try:
            items, truncated = factory().listdir(path, bool(show_hidden))
        except OSError as e:
            raise os_error(e, os.path.basename(path) or path) from None
        parent = None if path == "/" else os.path.dirname(path)
        return {"path": path, "parent": parent, "items": items, "truncated": truncated,
                "writable": os.access(path, os.W_OK)}

    @api.method("files.stat")
    def _stat(args):
        """One FileItem."""
        path = _abs(args.get("path"))
        try:
            return factory().make(path, read_hidden(os.path.dirname(path)))
        except OSError as e:
            raise os_error(e, os.path.basename(path)) from None

    @api.method("files.places")
    def _places_m(args):
        """Standard places: home desktop documents downloads music pictures videos trash root."""
        return _places(ctx, _lang(ctx), trash_count())

    # -- opening -------------------------------------------------------------------
    @api.method("files.open")
    def _open(args):
        """Open like a double click (folders → Verkenner, .desktop → start, else default app).
        Dangerous: in --dry-run only our own windows open."""
        raw = args.get("path")
        if isinstance(raw, str) and raw.startswith("trash:"):
            ctx.open_app("explorer", {"path": TRASH_URI})
            return {}
        path = _abs(raw)
        if not os.path.exists(path):
            raise ApiError("not_found", "Dit item bestaat niet (meer).")
        return launch_file(ctx, path)

    @api.method("files.openWith")
    def _open_with(args):
        """Open a file with a chosen app (dry-run guarded inside)."""
        path = _abs(args.get("path"))
        app_id = args.get("appId")
        if not isinstance(app_id, str) or not app_id:
            raise ApiError("bad_args", "Kies een app.")
        if not os.path.exists(path):
            raise ApiError("not_found", "Dit item bestaat niet (meer).")
        return launch_app(ctx, app_id, [path_to_uri(path)])

    @api.method("files.trustLauncher", dangerous=True, dry_result=lambda a: {"trusted": True})
    def _trust_launcher(args):
        """Allow a .desktop launcher to start ("Starten toestaan", like GNOME): sets Gio metadata::trusted
        and the owner's execute bit. The UI must first show the user what it runs (error data ``exec``)."""
        path = _abs(args.get("path"))
        r = trust_launcher(path)
        watches.changed(os.path.dirname(path))
        return r

    @api.method("files.openTerminal", dangerous=True)
    def _open_terminal(args):
        """Open the default terminal in a folder."""
        d = _dir(args.get("dir") or args.get("path") or str(ctx.home))
        info = default_terminal()
        if info is None:
            raise ApiError("no_terminal", "Er is geen terminal-app geïnstalleerd.")
        exe = info.get_executable() or ""
        exe_path = which(exe) if exe and not os.path.isabs(exe) else exe
        if not exe_path:
            raise ApiError("no_terminal", "De terminal-app is niet gevonden.")
        base = os.path.basename(exe_path)
        extra = {
            "gnome-terminal": [f"--working-directory={d}"], "ptyxis": ["--new-window", "-d", d],
            "kgx": [f"--working-directory={d}"], "kitty": ["--directory", d], "foot": [f"--working-directory={d}"],
            "alacritty": ["--working-directory", d], "ghostty": [f"--working-directory={d}"],
            "konsole": ["--workdir", d], "xfce4-terminal": [f"--working-directory={d}"],
            "wezterm": ["start", "--cwd", d], "tilix": [f"--working-directory={d}"],
        }.get(base, [])
        env = user_env(drop_private=True)      # no token / test overrides, the user's own GPU switches
        p = spawn([exe_path] + extra, env=env, cwd=d)
        if p is None:
            raise ApiError("launch_failed", "De terminal kon niet worden gestart.")
        return {}

    # -- create / rename -----------------------------------------------------------------
    @api.method("files.createFolder")
    def _create_folder(args):
        """New folder ("Nieuwe map", "Nieuwe map (2)" …) → {path, item}."""
        d = _dir(args.get("dir"))
        name = validate_name(args["name"]) if args.get("name") else lang_defaults()["folder"]
        target = unique_path(d, name, is_dir=True)
        try:
            os.mkdir(target, 0o755)
        except OSError as e:
            raise os_error(e, name) from None
        watches.changed(d)
        return {"path": target, "item": factory().make(target)}

    @api.method("files.createFile")
    def _create_file(args):
        """New file ("Nieuw tekstdocument.txt") → {path, item}."""
        d = _dir(args.get("dir"))
        kind = args.get("kind") or "text"
        if kind not in ("text", "empty"):
            raise ApiError("bad_args", "Onbekend soort bestand.")
        name = validate_name(args["name"]) if args.get("name") else lang_defaults()[kind]
        target = unique_path(d, name)
        try:
            fd = os.open(target, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_CLOEXEC, 0o644)
            os.close(fd)
        except OSError as e:
            raise os_error(e, name) from None
        watches.changed(d)
        return {"path": target, "item": factory().make(target)}

    def _dry_rename(args):
        """--dry-run: same validation (Dutch errors) as a real rename, nothing renamed."""
        p = _abs(args.get("path"))
        new = validate_name(args.get("newName"))
        if not os.path.lexists(p):
            raise ApiError("not_found", "Dit item bestaat niet (meer).")
        target = os.path.join(os.path.dirname(p), new)
        if target != p and os.path.lexists(target):
            try:
                same = os.path.samefile(target, p)
            except OSError:
                same = False
            if not same:
                raise ApiError("exists", f"Er bestaat al een item met de naam ‘{new}’ in deze map.")
        return {"path": target}

    @api.method("files.rename", dangerous=True, dry_result=_dry_rename)
    def _rename(args):
        """Rename in place → {path, item}. Dutch errors for empty/invalid/existing names."""
        path = _abs(args.get("path"))
        new = validate_name(args.get("newName"))
        if not os.path.lexists(path):
            raise ApiError("not_found", "Dit item bestaat niet (meer).")
        parent = os.path.dirname(path)
        target = os.path.join(parent, new)
        if target == path:
            return {"path": path, "item": factory().make(path)}
        if os.path.lexists(target):
            same = False
            try:
                same = os.path.samefile(target, path)   # case-only rename on NTFS/exFAT
            except OSError:
                pass
            if not same:
                raise ApiError("exists", f"Er bestaat al een item met de naam ‘{new}’ in deze map.")
            tmp = os.path.join(parent, f".{new}.{uuid.uuid4().hex[:6]}.tmp")
            try:
                rename_noreplace(path, tmp)
                try:
                    rename_noreplace(tmp, target)
                except OSError:
                    os.rename(tmp, path)      # put it back under its old name
                    raise
            except FileExistsError:
                raise ApiError("exists", f"Er bestaat al een item met de naam ‘{new}’ in deze map.") from None
            except OSError as e:
                raise os_error(e, os.path.basename(path)) from None
        else:
            try:
                rename_noreplace(path, target)      # never overwrites an item that appeared meanwhile
            except FileExistsError:
                raise ApiError("exists", f"Er bestaat al een item met de naam ‘{new}’ in deze map.") from None
            except OSError as e:
                raise os_error(e, os.path.basename(path)) from None
        watches.changed(parent)
        return {"path": target, "item": factory().make(target)}

    # -- trash ---------------------------------------------------------------------------
    trash_one = trash_path

    @api.method("files.trash", dangerous=True, dry_result=lambda a: {"failed": [], "trashed": []})
    def _trash(args):
        """Move items to the Prullenbak → {ok, failed:[{path, error}], trashed:[{path, name, trashPath}]}
        (never deletes permanently). ``name`` = the item's name inside Trash/files, i.e. what
        ``files.restoreTrash {names}`` takes ("Ongedaan maken")."""
        paths = _paths(args.get("paths"))
        failed = []
        trashed: list[dict] = []
        unknown: list[str] = []
        before = None if os.environ.get("WIN11SHELL_TRASH_DIR") else _trash_info_names(paths)
        for p in paths:
            try:
                if not os.path.lexists(p):
                    raise ApiError("not_found", "Dit item bestaat niet (meer).")
                name = trash_one(p)
                if name:
                    trashed.append({"path": p, "name": name, "trashPath": str(home_trash() / "files" / name)})
                else:
                    unknown.append(p)
            except ApiError as e:
                failed.append({"path": p, "error": e.message, "code": e.code})
            except OSError as e:
                err = os_error(e, os.path.basename(p))
                failed.append({"path": p, "error": err.message, "code": err.code})
        if unknown and before is not None:
            # Gio does not say which name it gave the item: find the new .trashinfo files
            found = _new_trash_entries(before, set(unknown))
            for p in unknown:
                hit = found.get(p)
                if hit:
                    trashed.append({"path": p, "name": hit["name"], "trashPath": hit["trashPath"]})
            order = {p: i for i, p in enumerate(paths)}
            trashed.sort(key=lambda t: order.get(t["path"], 0))
        trash_cache["key"] = None
        watches.changed(*dirs_of(*paths))
        api.emit("files.trashChanged", {})
        return {"ok": not failed, "failed": failed, "trashed": trashed}

    @api.method("files.trashInfo")
    def _trash_info(args):
        """{count, size, approx} of the Prullenbak."""
        ents = _trash_entries()
        key = tuple(sorted((e["path"] for e in ents)))
        if trash_cache["key"] == key and time.monotonic() - trash_cache["at"] < 30:
            return dict(trash_cache["value"])
        deadline = time.monotonic() + 1.5
        size = 0
        complete = True
        for e in ents:
            s, _f, _d, c = _du(e["path"], deadline)
            size += s
            complete = complete and c
            if not c:
                break
        value = {"count": len(ents), "size": size, "approx": not complete}
        trash_cache.update(key=key, value=value, at=time.monotonic())
        return dict(value)

    @api.method("files.emptyTrash", dangerous=True, dry_result=lambda a: {"freed": 0})
    def _empty_trash(args):
        """Permanently delete everything in the Prullenbak (the UI must confirm first)."""
        removed, failed, _errors = empty_trash_dirs()
        trash_cache["key"] = None
        api.emit("files.trashChanged", {})
        watches.changed(str(home_trash() / "files"))
        if failed:
            raise ApiError("partial", f"{failed} item(s) konden niet worden verwijderd.", {"removed": removed})
        return {"ok": True, "removed": removed}

    @api.method("files.restoreTrash", dangerous=True, dry_result=lambda a: {"restored": [], "failed": []})
    def _restore(args):
        """Put items back where they came from ({names} = trash names or paths; {all:true} = everything)."""
        ents = _trash_entries()
        if args.get("all"):
            wanted = ents
        else:
            names = args.get("names") or args.get("paths")
            if isinstance(names, str):
                names = [names]
            if not isinstance(names, list) or not names:
                raise ApiError("bad_args", "Kies eerst een of meer items.")
            by_name: dict[str, dict] = {}
            for e in sorted(ents, key=lambda e: e["topdir"] is not None):   # home trash wins on equal names
                by_name.setdefault(e["name"], e)
            by_path = {e["path"]: e for e in ents}
            wanted = []
            for n in names:
                if not isinstance(n, str):
                    continue
                e = by_path.get(os.path.normpath(n)) if n.startswith("/") else by_name.get(n)
                if e is None:
                    wanted.append({"missing": n})
                else:
                    wanted.append(e)
        restored, failed = [], []
        touched = set()
        for e in wanted:
            if "missing" in e:
                failed.append({"name": e["missing"], "error": "Dit item staat niet (meer) in de Prullenbak."})
                continue
            orig = e["original"]
            top = e["topdir"]
            if not orig or not orig.startswith("/"):
                msg = ("De oorspronkelijke locatie ligt niet op deze schijf. Sleep het item zelf naar een map."
                       if top else "De oorspronkelijke locatie is onbekend.")
                failed.append({"name": e["name"], "error": msg})
                continue
            parent = os.path.dirname(orig)
            if top is not None and not _inside(parent, top):
                # a symlink on the drive pointing elsewhere: never write outside that drive
                failed.append({"name": e["name"], "error": "De oorspronkelijke locatie ligt niet op deze schijf. "
                                                           "Sleep het item zelf naar een map."})
                continue
            try:
                os.makedirs(parent, exist_ok=True)
                is_dir = os.path.isdir(e["path"]) and not os.path.islink(e["path"])
                for _attempt in range(20):
                    target = unique_path(parent, os.path.basename(orig), is_dir=is_dir)
                    try:
                        move_noreplace(e["path"], target)
                        break
                    except FileExistsError:
                        continue          # something appeared there meanwhile: next free name
                else:
                    raise FileExistsError(errno.EEXIST, "exists", target)
                try:
                    os.unlink(e["tdir"] / "info" / (e["name"] + ".trashinfo"))
                except OSError:
                    pass
                restored.append(target)
                touched.add(parent)
            except OSError as ex:
                failed.append({"name": e["name"], "error": os_error(ex, os.path.basename(orig)).message})
        trash_cache["key"] = None
        watches.changed(*touched)
        api.emit("files.trashChanged", {})
        return {"ok": not failed, "restored": restored, "failed": failed}

    # -- copy / move / clipboard ----------------------------------------------------------
    def _dry_transfer(args):
        _paths(args.get("paths"))
        _dir(args.get("dest"))
        return {"paths": [], "failed": []}

    @api.method("files.copy", dangerous=True, dry_result=_dry_transfer)
    def _copy(args):
        """Copy items into ``dest`` ('naam (2).ext' on conflicts) → {ok, paths, failed}."""
        paths = _paths(args.get("paths"))
        dest = _dir(args.get("dest"))
        res = transfer(api, paths, dest, move=False, job=args.get("jobId"), copy_word=copy_word())
        watches.changed(dest)
        return res

    @api.method("files.move", dangerous=True, dry_result=_dry_transfer)
    def _move(args):
        """Move items into ``dest`` → {ok, paths, failed}."""
        paths = _paths(args.get("paths"))
        dest = _dir(args.get("dest"))
        res = transfer(api, paths, dest, move=True, job=args.get("jobId"))
        watches.changed(dest, *dirs_of(*paths))
        return res

    @api.method("files.clipboardSet")
    def _clip_set(args):
        """Remember items for Plakken (also offered to other apps as text/uri-list on Hyprland)."""
        op = args.get("op") or "copy"
        if op not in ("copy", "cut"):
            raise ApiError("bad_args", "Ongeldige bewerking.")
        paths = [p for p in _paths(args.get("paths")) if os.path.lexists(p)]
        with clip_lock:
            clip.update(paths=paths, op=op, time=int(time.time() * 1000))
        api.emit("files.clipboardChanged", {"paths": paths, "op": op})
        if paths and ctx.compositor == "hyprland" and which("wl-copy") and not dry_for(ctx):
            data = "".join(path_to_uri(p) + "\r\n" for p in paths)
            ctx.thread(run, ["wl-copy", "--type", "text/uri-list"], timeout=5, input=data, name="wl-copy")
        return {"paths": paths, "op": op}

    @api.method("files.clipboardGet")
    def _clip_get(args):
        """{paths, op} (only items that still exist)."""
        with clip_lock:
            paths = [p for p in clip["paths"] if os.path.lexists(p)]
            return {"paths": paths, "op": clip["op"]}

    @api.method("files.clipboardClear")
    def _clip_clear(args):
        with clip_lock:
            clip.update(paths=[], op="copy", time=0)
        api.emit("files.clipboardChanged", {"paths": [], "op": "copy"})
        return {}

    @api.method("files.paste", dangerous=True, dry_result=lambda a: {"paths": [], "failed": []})
    def _paste(args):
        """Paste the remembered items into ``dest`` (cut → move, then the clipboard is cleared)."""
        dest = _dir(args.get("dest"))
        with clip_lock:
            paths = [p for p in clip["paths"] if os.path.lexists(p)]
            op = clip["op"]
        if not paths:
            raise ApiError("empty", "Er is niets om te plakken.")
        res = transfer(api, paths, dest, move=op == "cut", job=args.get("jobId"), copy_word=copy_word())
        if op == "cut":
            with clip_lock:
                clip.update(paths=[], op="copy", time=0)
            api.emit("files.clipboardChanged", {"paths": [], "op": "copy"})
            watches.changed(*dirs_of(*paths))
        watches.changed(dest)
        return res

    # -- properties ------------------------------------------------------------------------
    @api.method("files.properties")
    def _properties(args):
        """Details for the Eigenschappen dialog (folders: bounded count with a time budget)."""
        try:
            budget = max(0.2, min(10.0, float(args.get("budget", 3.0))))
        except (TypeError, ValueError):
            budget = 3.0
        if args.get("paths") and not args.get("path"):
            return _multi_properties(_paths(args.get("paths")), budget)
        path = _abs(args.get("path"))
        try:
            lst = os.lstat(path)
        except OSError as e:
            raise os_error(e, os.path.basename(path)) from None
        f = factory()
        item = f.make(path, read_hidden(os.path.dirname(path)), lst)
        try:
            st = os.stat(path)
        except OSError:
            st = lst
        contains = None
        size = item["size"]
        on_disk = st.st_blocks * 512 if hasattr(st, "st_blocks") else size
        partial = False
        if item["isDir"]:
            s, nf, nd, complete = _du(path, time.monotonic() + budget)
            size = s
            contains = {"files": nf, "folders": nd}
            partial = not complete
            on_disk = None
        created = None
        if Gio is not None:
            try:
                gi = Gio.File.new_for_path(path).query_info("time::created", Gio.FileQueryInfoFlags.NONE, None)
                if gi.has_attribute("time::created"):
                    created = int(gi.get_attribute_uint64("time::created") * 1000)
            except GLib.Error:
                pass
        try:
            owner = pwd.getpwuid(st.st_uid).pw_name
        except KeyError:
            owner = str(st.st_uid)
        try:
            group = grp.getgrgid(st.st_gid).gr_name
        except KeyError:
            group = str(st.st_gid)
        return {
            "name": item["name"], "displayName": item["displayName"], "path": path,
            "location": os.path.dirname(path), "type": item["typeName"], "mime": item["mime"],
            "icon": item["icon"], "isDir": item["isDir"], "isDesktopEntry": item["isDesktopEntry"],
            "size": size, "sizeOnDisk": on_disk, "contains": contains, "partial": partial,
            "created": created, "modified": int(st.st_mtime * 1000), "accessed": int(st.st_atime * 1000),
            "owner": owner, "group": group, "permissions": stat.filemode(st.st_mode)[1:],
            "mode": oct(stat.S_IMODE(st.st_mode)), "readOnly": not os.access(path, os.W_OK),
            "hidden": item["hidden"], "isSymlink": item["isSymlink"],
            "target": os.readlink(path) if item["isSymlink"] else None,
            "executable": bool(st.st_mode & 0o100) and not item["isDir"],
        }

    def _multi_properties(paths: list[str], budget: float) -> dict:
        deadline = time.monotonic() + budget
        size = files = folders = 0
        complete = True
        mimes = set()
        for p in paths:
            try:
                lst = os.lstat(p)
            except OSError:
                continue
            if stat.S_ISDIR(lst.st_mode):
                folders += 1
                mimes.add("inode/directory")
                s, nf, nd, c = _du(p, deadline)
                size += s
                files += nf
                folders += nd
                complete = complete and c
            else:
                files += 1
                size += lst.st_size
                mimes.add(guess_mime(p, os.path.basename(p), 0))
        nl = _lang(ctx) == "nl"
        if len(mimes) == 1:
            m = next(iter(mimes))
            t = ("Alle van het type " if nl else "All of type ") + type_name(m, os.path.basename(paths[0]), _lang(ctx))
        else:
            t = "Meerdere typen" if nl else "Multiple types"
        parents = {os.path.dirname(p) for p in paths}
        return {"name": (f"{len(paths)} items"), "path": None, "type": t, "size": size,
                "contains": {"files": files, "folders": folders}, "partial": not complete,
                "location": parents.pop() if len(parents) == 1 else None, "count": len(paths)}

    @api.method("files.setAttributes", dangerous=True)
    def _set_attrs(args):
        """Properties dialog checkboxes: {path, readOnly?, hidden?} (owner write bit / .hidden file)."""
        path = _abs(args.get("path"))
        if not os.path.lexists(path):
            raise ApiError("not_found", "Dit item bestaat niet (meer).")
        if "readOnly" in args and args["readOnly"] is not None and os.path.islink(path):
            # chmod follows the link (Linux has no lchmod): it would change the target, maybe elsewhere
            raise ApiError("symlink", "Dit is een koppeling. Wijzig de eigenschappen van het doel zelf.")
        if "readOnly" in args and args["readOnly"] is not None:
            try:
                mode = stat.S_IMODE(os.stat(path).st_mode)
                mode = (mode & ~0o222) if args["readOnly"] else (mode | 0o200)
                os.chmod(path, mode)
            except OSError as e:
                raise os_error(e, os.path.basename(path)) from None
        if "hidden" in args and args["hidden"] is not None:
            parent, name = os.path.dirname(path), os.path.basename(path)
            if name.startswith("."):
                if not args["hidden"]:
                    raise ApiError("dotfile", "Dit item is verborgen omdat de naam met een punt begint. "
                                              "Wijzig de naam om het zichtbaar te maken.")
            else:
                hf = os.path.join(parent, ".hidden")
                names = list(read_hidden(parent))
                names = [n for n in names if n != name] + ([name] if args["hidden"] else [])
                try:
                    if names:
                        atomic_write_text(hf, "\n".join(sorted(names)) + "\n", mode=0o644)
                    elif os.path.exists(hf):
                        os.unlink(hf)
                except OSError as e:
                    raise os_error(e, ".hidden") from None
        watches.changed(os.path.dirname(path))
        return {"ok": True}

    # -- archives -------------------------------------------------------------------------
    @api.method("files.compress", dangerous=True, dry_result=lambda a: {"path": None})
    def _compress(args):
        """Zip items → "<naam>.zip" next to the first item (or in ``dest``) → {path}."""
        paths = _paths(args.get("paths"))
        for p in paths:
            if not os.path.lexists(p):
                raise ApiError("not_found", f"‘{os.path.basename(p)}’ bestaat niet (meer).")
        dest = args.get("dest")
        out_dir = _dir(dest) if dest else os.path.dirname(paths[0])
        first = os.path.basename(paths[0])
        stem = first if os.path.isdir(paths[0]) else split_name(first)[0]
        target = unique_path(out_dir, f"{stem}.zip")
        tmp = os.path.join(out_dir, f".{os.path.basename(target)}.{uuid.uuid4().hex[:6]}.partial")
        prog = Progress(api, "compress", args.get("jobId"), _total_size(paths), len(paths))
        try:
            with zipfile.ZipFile(tmp, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=6, allowZip64=True) as zf:
                for p in paths:
                    _zip_tree(zf, p, os.path.basename(p), prog)
                    prog.items += 1
            target = _publish_noreplace(tmp, target, out_dir, is_dir=False)
        except OSError as e:
            try:
                os.unlink(tmp)
            except OSError:
                pass
            raise os_error(e, os.path.basename(target)) from None
        finally:
            prog.emit(finished=True)
        watches.changed(out_dir)
        return {"path": target}

    @api.method("files.extract", dangerous=True, dry_result=lambda a: {"path": None})
    def _extract(args):
        """Unpack zip/tar.* into a new folder "<naam>" (refuses unsafe paths) → {path}."""
        path = _abs(args.get("path"))
        if not os.path.isfile(path):
            raise ApiError("not_found", "Dit archief bestaat niet (meer).")
        kind, total = check_archive(path)
        dest = args.get("dest")
        out_dir = _dir(dest) if dest else os.path.dirname(path)
        try:
            free = shutil.disk_usage(out_dir).free
            if total > free:
                raise ApiError("no_space", "Er is niet genoeg ruimte op de schijf om dit archief uit te pakken.")
        except OSError:
            pass
        target = unique_path(out_dir, _archive_stem(os.path.basename(path)), is_dir=True)
        tmp = os.path.join(out_dir, f".{os.path.basename(target)}.{uuid.uuid4().hex[:6]}.partial")
        prog = Progress(api, "extract", args.get("jobId"), total, 1)
        prog.emit(force=True)
        try:
            os.mkdir(tmp, 0o755)
            if kind == "zip":
                with zipfile.ZipFile(path) as zf:
                    for info in zf.infolist():
                        prog.current = info.filename
                        zf.extract(info, tmp)
                        prog.add(info.file_size)
            else:
                with tarfile.open(path) as tf:
                    tf.extractall(tmp, filter="data")
                prog.add(total)
            target = _publish_noreplace(tmp, target, out_dir, is_dir=True)
        except ApiError:
            shutil.rmtree(tmp, ignore_errors=True)
            raise
        except (OSError, zipfile.BadZipFile, tarfile.TarError, EOFError) as e:
            shutil.rmtree(tmp, ignore_errors=True)
            if isinstance(e, OSError):
                raise os_error(e, os.path.basename(path)) from None
            raise ApiError("bad_archive", "Dit archief is beschadigd of onvolledig.", str(e)) from None
        finally:
            prog.emit(finished=True)
        watches.changed(out_dir)
        return {"path": target}

    # -- find -----------------------------------------------------------------------------
    @api.method("files.find")
    def _find(args):
        """Bounded recursive name search below ``path`` → {items, truncated}."""
        root = _dir(args.get("path"))
        query = args.get("query") or ""
        if not isinstance(query, str) or len(query) > 500:
            raise ApiError("bad_args", "Ongeldige zoekopdracht.")
        try:
            limit = max(1, min(5000, int(args.get("limit", 500))))
        except (TypeError, ValueError):
            limit = 500
        show_hidden = bool(args.get("showHidden"))
        mime_prefix = args.get("mimePrefix") if isinstance(args.get("mimePrefix"), str) else None
        q = query.strip().casefold()
        wild = any(c in q for c in "*?[")
        terms = q.split() if not wild else []
        f = factory()
        items: list[dict] = []
        truncated = False
        deadline = time.monotonic() + max(0.5, min(10.0, float(args.get("budget", 3.0) or 3.0)))
        queue = deque([root])
        dirs_seen = 0
        while queue:
            d = queue.popleft()
            dirs_seen += 1
            if dirs_seen > 50_000 or time.monotonic() > deadline:
                truncated = True
                break
            try:
                with os.scandir(d) as it:
                    entries = list(it)
            except OSError:
                continue
            hidden = read_hidden(d) if not show_hidden else frozenset()
            for e in entries:
                name = e.name
                if not _utf8_ok(name):
                    continue
                if not show_hidden and (name.startswith(".") or name in hidden):
                    continue
                try:
                    is_dir = e.is_dir(follow_symlinks=False)
                except OSError:
                    is_dir = False
                if is_dir:
                    queue.append(e.path)
                low = name.casefold()
                if wild:
                    ok = fnmatch.fnmatchcase(low, q if any(c in q for c in "*?") else f"*{q}*")
                else:
                    ok = all(t in low for t in terms)
                if not ok:
                    continue
                try:
                    it_ = f.make(e.path, hidden, e.stat(follow_symlinks=False))
                except OSError:
                    continue
                if mime_prefix and not it_["mime"].startswith(mime_prefix):
                    continue
                items.append(it_)
                if len(items) >= limit:
                    truncated = True
                    break
            if len(items) >= limit:
                break
        return {"items": items, "truncated": truncated}

    # -- watching ---------------------------------------------------------------------------
    @api.method("files.watch")
    def _watch(args):
        """Send ``files.dirChanged {path}`` when the folder changes."""
        if isinstance(args.get("path"), str) and args["path"].startswith("trash:"):
            for tdir, _ in trash_dirs():
                watches.watch(str(tdir / "files"))
            return {"ok": True}
        d = _dir(args.get("path"))
        return {"ok": watches.watch(d)}

    @api.method("files.unwatch")
    def _unwatch(args):
        if isinstance(args.get("path"), str) and args["path"].startswith("trash:"):
            for tdir, _ in trash_dirs():
                watches.unwatch(str(tdir / "files"))
            return {}
        watches.unwatch(_abs(args.get("path")))
        return {}

    # Prullenbak changes also reach the UI as files.trashChanged {}
    def _trash_dir_changed(_event, data):
        p = (data or {}).get("path") or ""
        if p.endswith("/files") and any(p == str(t / "files") for t, _ in trash_dirs()):
            trash_cache["key"] = None
            api.emit("files.trashChanged", {})
            api.emit("files.dirChanged", {"path": TRASH_URI})

    api.on("files.dirChanged", _trash_dir_changed)


def start(api, ctx) -> None:
    watches: Watches | None = getattr(ctx, "_files_watches", None)
    if watches is None:
        return
    if os.path.isdir(watches.desktop):
        watches.watch(watches.desktop, pinned=True)
    t = home_trash() / "files"
    if t.is_dir():
        watches.watch(str(t), pinned=True)


def stop(api, ctx) -> None:
    watches: Watches | None = getattr(ctx, "_files_watches", None)
    if watches is not None:
        watches.close()
