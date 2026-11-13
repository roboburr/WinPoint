"""Alles indexer — instant file name search for Win11 Shell (SPEC §6.13).

Runs as a separate, nice'd process so crawling millions of files never blocks the
HTTP server (GIL isolation)::

    python3 -m host.indexer --cache DIR [--roots / /srv/vm ...] [--exclude P ...] [--no-live]

Protocol: JSON lines on stdin (requests) / stdout (responses + events)::

    → {"id": 1, "method": "configure", "args": {"roots": [...] | "auto", "excluded": [...], "liveUpdate": true}}
    → {"id": 2, "method": "search", "args": {query, offset, limit, sort, desc, filter, matchCase,
                                             matchPath, regex, wholeWord, showHidden, channel}}
    → {"id": 3, "method": "status"}   {"method": "rebuild"}   {"method": "ping"}   {"method": "shutdown"}
    ← {"id": 1, "ok": true, "result": ...}  |  {"id": 1, "ok": false, "error": {"code", "message"}}
    ← {"event": "status", "data": {...}}  |  {"event": "changed", "data": {"generation": n}}

A newer search on the same ``channel`` with a different query cancels older ones
(error code ``cancelled``); pages of the same query never cancel each other.

Data model (designed for millions of entries across many TB):

* One *segment* per root (a mount), entries in BFS order, so ``parent`` is
  non-decreasing inside a segment: the children of a directory — and the children of
  any contiguous range of directories — are one contiguous gid range (``bisect``).
* Global gid-indexed arrays (``array`` module): parent, size (folders = total of
  their contents), mtime (ms), flags, category. Names live in chunks of 65 536
  entries: ``b"\\n" + b"\\n".join(names) + b"\\n"`` once in original case and once
  lower-cased with a byte-length-preserving lower(), sharing one offsets array.
* Search = C-speed ``bytes.find`` loops over the blobs mapped back to gids with
  ``bisect`` on the offsets, AND/OR/NOT evaluated as sorted gid lists (small
  sets, rarest term first, the rest tested per candidate) or 0/1 ``bytearray``
  masks (dense sets, combined with big-int AND/OR, ``bytes.translate`` for flag /
  category filters). Name order comes from a precomputed global order + rank;
  size / date / path / type orders are computed lazily and cached.
* Live updates go to an *overlay* (added entries) + tombstones (deleted flag) +
  in-place size/mtime updates; it is folded into the base in the background
  (BFS re-layout) when it grows. Nothing rebuilds the blobs per change.
* Persisted to ``<cache>/index/`` (versioned binary arrays + blobs, overlay as
  JSON) and used straight from the files: read-only mmaps for the immutable
  base, private copy-on-write maps for size / mtime / flags / cat, so the
  process holds little anonymous memory (~50 MiB for 6.4 M entries). Loaded in
  ~0.1 s; then refreshed by comparing directory mtimes, kept live with inotify
  (ctypes) on the home tree within a watch budget, and rescanned every 10
  minutes elsewhere. Sort orders are persisted next to the base; they and folds
  are built by short-lived helper processes (``--helper``). A save never changes
  a file in place: new dir, unchanged files hard-linked, renamed over the old.
  ``WIN11SHELL_INDEX_MMAP=0`` (heap), ``WIN11SHELL_INDEX_FORK=0`` (no helpers),
  ``WIN11SHELL_INDEX_MIN_FREE`` (free disk a helper fold needs; default 2× the dir).

Stdlib only. Never writes anywhere except the cache directory.
"""

from __future__ import annotations

import array
import bisect
import errno
import ctypes
import heapq
import itertools
import json
import mimetypes
import mmap
import operator
import os
import platform
import queue
import re
import select
import shutil
import stat as _st
import struct
import sys
import threading
import time
import traceback
import unicodedata
from collections import OrderedDict, deque
from functools import lru_cache
from itertools import compress, islice, repeat

FORMAT_VERSION = 2          # 2: a root entry stores its display name (basename), its path lives in Seg.path
CHUNK = 1 << 16
LIST_MAX = 150_000          # above this a result set is kept as a 0/1 mask
OVERLAY_COMPACT = int(os.environ.get("WIN11SHELL_INDEX_COMPACT") or 20_000)   # fold the overlay into the base past this
RESCAN_INTERVAL = 600.0     # periodic directory-mtime rescan (seconds)
PERSIST_INTERVAL = 900.0
WATCH_CAP = 40_000
DPATH_MAX = 100_000         # folder path cache entries (400 000 used to cost ~94 MiB at the peak)
DPATH_KEEP = 20_000         # after a search, a bigger folder path cache is dropped
MERGE_STREAM_MAX = 50_000   # pages up to this offset merge the overlay extras lazily (no materialisation)
RESULT_MASKS = 1            # cached results that may hold an N-byte mask or a big gid list
DIRECT_MAX = 12_000         # estimated hits up to this: a name term searches the mapped file itself
SMALL_FILTER = 20_000       # candidate lists up to this test flags/cat per entry (no N-byte translate)
USE_MMAP = os.environ.get("WIN11SHELL_INDEX_MMAP", "1") != "0"    # 0: read the index into the heap
FORK_OK = os.environ.get("WIN11SHELL_INDEX_FORK", "1") != "0"     # 0: sorts and folds in this process
ORDER_KINDS = ("size", "modified", "path", "type")
ODIRTY_MAX = 50_000         # an order with more changed entries than this is rebuilt
HELPER_TIMEOUT = 900.0      # seconds a helper process may take (order build: ~3 s, fold: ~20 s here)
HELPER_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))   # holds the host package
# The helper is whatever host/indexer.py is on disk when it starts, not necessarily the code this
# process runs (an install replaces the file while the shell runs for days). Bump HELPER_PROTO
# whenever the helper's command line, or the meaning or layout of a file it writes, changes: a
# helper refuses a parent with another protocol, and that parent then stops using helpers.
HELPER_PROTO = 1
ORDERS_FORMAT = 1           # orders.json + order.*.bin / key.*.bin layout; another value is ignored
H_PROTO_RC = 6              # exit code of a helper whose protocol differs from its parent's
H_OFF = -1000               # run_helper: not started (switched off, version skew, no code to run)
H_TIMEOUT = -1001           # run_helper: killed after HELPER_TIMEOUT
_helpers_off: str | None = None   # why helpers are off until this process restarts
FOLD_RETRY = 900.0          # a fold that could not run waits this long (doubling) ...
FOLD_RETRY_MAX = 3600.0     # ... up to this; a helper killed by a signal waits at least this
OVERLAY_HARD = 10 * OVERLAY_COMPACT   # past this an overlay is folded in this process if nothing else can

try:
    _LIBC = ctypes.CDLL(None, use_errno=True)
    _LIBC.malloc_trim.argtypes = [ctypes.c_size_t]
    _LIBC.malloc_trim.restype = ctypes.c_int
except (OSError, AttributeError):
    _LIBC = None


def helpers_on() -> bool:
    return FORK_OK and _helpers_off is None


def _helpers_stop(why: str) -> None:
    global _helpers_off
    if _helpers_off is None:
        _helpers_off = why
        log("hulpprocessen uit tot de volgende start: %s", why)


def run_helper(args: list, timeout: float | None = None) -> int:
    """Run ``python3 -m host.indexer --helper ARGS`` and wait for it. Returns its exit code: 0 =
    success, > 0 = it failed, < 0 = killed by that signal (e.g. -9: the OOM killer), or H_OFF /
    H_TIMEOUT.

    Heavy one-off work (an order build, a fold) runs there instead of here: its big temporaries
    die with it, so they never fragment this long-lived heap. It is a freshly started process
    (not a fork() of this one): this process has 5-6 threads, and a forked child could inherit
    locks another thread held at that moment (logging, the stdout writer, allocator or module
    locks) and run finalizers of objects those threads owned. The helper maps the persisted index
    itself; the caller saved what it needs first. It never outlives us (PR_SET_PDEATHSIG).
    A helper of another protocol (exit 6), or one that does not know an option (2: an older file),
    or code that is gone switches helpers off until this process restarts."""
    if not helpers_on():
        return H_OFF
    import subprocess
    env = dict(os.environ)
    env["PYTHONPATH"] = HELPER_ROOT + (os.pathsep + env["PYTHONPATH"] if env.get("PYTHONPATH") else "")
    cmd = [sys.executable, "-m", "host.indexer", "--helper", *args, "--proto", str(HELPER_PROTO),
           "--parent", str(os.getpid())]
    try:
        p = subprocess.Popen(cmd, cwd=HELPER_ROOT, env=env, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                             close_fds=True)
    except (FileNotFoundError, NotADirectoryError, PermissionError) as e:
        _helpers_stop(f"hulpproces starten mislukt: {e}")      # the install dir moved or is gone
        return H_OFF
    except OSError as e:
        log("hulpproces starten mislukt: %s", e)
        return H_OFF
    try:
        rc = p.wait(timeout)
    except subprocess.TimeoutExpired:
        p.kill()
        p.wait()
        log("hulpproces %s duurde te lang; gestopt", args[0])
        return H_TIMEOUT
    if rc in (2, H_PROTO_RC):
        _helpers_stop(f"hulpproces {args[0]} past niet bij deze versie (code {rc}; host/indexer.py vervangen?)")
    elif rc:
        log("hulpproces %s mislukt (code %s)", args[0], rc)
    return rc


def trim_heap() -> None:
    """Give free heap memory back to the OS. glibc keeps what big temporaries (load, sorts, a
    fold, dense searches) leave behind otherwise: +360 MiB after a single fold was measured."""
    if _LIBC is not None:
        try:
            _LIBC.malloc_trim(0)
        except Exception:
            pass

F_DIR, F_HIDDEN, F_EXEC, F_LINK, F_DEL, F_MOUNT, F_NOSHOW, F_ROOT = 1, 2, 4, 8, 16, 32, 64, 128

C_AUDIO, C_ZIP, C_DOC, C_EXE, C_PIC, C_VIDEO = 1, 2, 3, 4, 5, 6
FILTERS = {"audio": C_AUDIO, "compressed": C_ZIP, "document": C_DOC, "executable": C_EXE,
           "picture": C_PIC, "video": C_VIDEO}

_CAT_EXTS = {
    C_AUDIO: "aac ac3 aif aifc aiff amr ape au cda dts fla flac it m1a m2a m3u m3u8 m4a m4b m4p mid midi mka "
             "mod mp2 mp3 mpa ogg oga opus pls ra rmi s3m snd spx tta voc vqf w64 wav wma wv xm",
    C_ZIP: "7z ace arj bz2 cab gz gzip jar lz lz4 lzh lzma r00 r01 rar tar taz tbz tbz2 tgz tlz txz xz z zip "
           "zst zstd deb rpm iso dmg apk xpi cpio img",
    C_DOC: "c cc chm cpp cs css csv cxx doc docm docx dot dotm dotx epub h hh hpp htm html hxx ini java js "
           "json key log lua md mht mhtml mobi nfo odp ods odt pdf php pl pot potm potx ppa ppam pps ppsm "
           "ppsx ppt pptm pptx ps py rb rst rtf sldm sldx sql tex thmx toml ts tsv txt vsd wpd wps wri xlam "
           "xls xlsb xlsm xlsx xlt xltm xltx xml yaml yml",
    C_EXE: "appimage bat cmd com desktop exe msi msp ps1 run scr sh bash vbs flatpakref snap",
    C_PIC: "ani avif bmp cr2 cr3 dng gif heic heif ico jfif jpe jpeg jpg jxl nef orf pcx png psd raf raw "
           "rw2 arw svg svgz tga tif tiff webp wmf xcf",
    C_VIDEO: "3g2 3gp 3gp2 3gpp amv asf avi bik divx f4v flv hdmov m1v m2t m2ts m2v m4v mkv mov mp2v mp4 "
             "mp4v mpe mpeg mpg mpv2 mts ogm ogv qt ram rm rmvb ts vob webm wm wmv",
}
EXT_CAT: dict[str, int] = {}
for _c, _s in _CAT_EXTS.items():
    for _e in _s.split():
        EXT_CAT.setdefault(_e, _c)
EXT_CAT["ts"] = C_VIDEO   # MPEG transport stream wins over TypeScript for normal users

# Windows system folders on NTFS/exFAT/FAT drives: treated as hidden (noise for normal users)
WIN_HIDDEN = {"$recycle.bin", "system volume information", "$windows.~bt", "$windows.~ws", "$winreagent",
              "$sysreset", "recovery", "config.msi", "msocache", "pagefile.sys", "hiberfil.sys", "swapfile.sys",
              "dumpstack.log.tmp", "dumpstack.log", "bootmgr", "bootnxt", "$extend", "found.000"}
WIN_FS = {"ntfs", "ntfs3", "fuseblk", "exfat", "vfat", "msdos", "fat", "fat32"}
REAL_FS = {"ext2", "ext3", "ext4", "btrfs", "xfs", "f2fs", "zfs", "jfs", "reiserfs", "bcachefs", "nilfs2",
           "ntfs", "ntfs3", "fuseblk", "exfat", "vfat", "msdos", "hfsplus", "apfs", "udf"}
DEFAULT_EXCLUDES = ("/proc", "/sys", "/dev", "/run/user", "/tmp", "/var/tmp", "/snap", "/var/lib/snapd",
                    "/var/lib/docker", "/var/cache", "/run/snapd", "/var/lib/containers", "/var/snap")
AUTO_PREFIXES = ("/home", "/run/media", "/media", "/mnt", "/srv")


def _table(pred) -> bytes:
    return bytes(1 if pred(i) else 0 for i in range(256))


T_ALIVE = _table(lambda f: not f & (F_DEL | F_NOSHOW))
T_VIS = _table(lambda f: not f & (F_DEL | F_NOSHOW | F_HIDDEN))
T_DIR = _table(lambda f: bool(f & F_DIR))
T_FILE = _table(lambda f: not f & F_DIR)
T_ADIR = _table(lambda f: bool(f & F_DIR) and not f & (F_DEL | F_NOSHOW))
T_AFILE = _table(lambda f: not f & F_DIR and not f & (F_DEL | F_NOSHOW))
CAT_TABLES = {c: _table(lambda x, c=c: x == c) for c in range(1, 7)}

log_lock = threading.Lock()


def log(msg: str, *args) -> None:
    try:
        text = msg % args if args else msg
    except Exception:
        text = f"{msg} {args}"
    with log_lock:
        try:
            sys.stderr.write(f"[indexer] {text}\n")
            sys.stderr.flush()
        except Exception:
            pass


class Cancelled(Exception):
    pass


class QueryError(Exception):
    def __init__(self, message: str, code: str = "bad_query"):
        super().__init__(message)
        self.message = message
        self.code = code


# ---------------------------------------------------------------------------
# text helpers
# ---------------------------------------------------------------------------

def enc(s: str) -> bytes:
    return s.encode("utf-8", "surrogateescape")


def dec(b: bytes) -> str:
    return b.decode("utf-8", "surrogateescape")


def store_name(name: str) -> bytes:
    """Name as stored in a blob: UTF-8 (surrogateescape); a newline becomes NUL."""
    b = enc(name)
    return b.replace(b"\n", b"\x00") if b"\n" in b else b


def stored_to_str(b: bytes) -> str:
    if b"\x00" in b:
        b = b.replace(b"\x00", b"\n")
    return dec(b)


@lru_cache(maxsize=8192)
def _lower_char(c: str) -> str:
    lo = c.lower()
    if lo == c or len(lo) != 1:
        return c
    try:
        if len(lo.encode("utf-8")) != len(c.encode("utf-8")):
            return c
    except UnicodeEncodeError:
        return c
    return lo


def lower_b(b: bytes) -> bytes:
    """Byte-length preserving lower-case (so original and lower blobs share offsets)."""
    if b.isascii():
        return b.lower()
    return enc("".join(map(_lower_char, dec(b))))


def fold_key(low: bytes) -> bytes:
    """Sort key for a lower-cased stored name: accents folded (é sorts with e)."""
    if low.isascii():
        return low
    try:
        s = unicodedata.normalize("NFKD", dec(low))
        folded = enc("".join(ch for ch in s if not unicodedata.combining(ch)))
    except Exception:
        folded = low
    return folded + b"\x00" + low


def join_path(d: str, name: str) -> str:
    if not d:
        return name
    return d + name if d.endswith("/") else d + "/" + name


def norm_path(p: str) -> str:
    p = os.path.expanduser(str(p or "").strip())
    if not p:
        return ""
    p = os.path.normpath(p)
    if p.startswith("//"):
        p = "/" + p.lstrip("/")
    return p


def ext_of(name: str, is_dir: bool = False) -> str:
    if is_dir:
        return ""
    dot = name.rfind(".")
    if dot <= 0 or dot == len(name) - 1:
        return ""
    return name[dot + 1:].lower()


def classify(name: str, mode: int, parent_hidden: bool, exec_ok: bool, win: bool) -> tuple[int, int]:
    """→ (flags, category) for one directory entry."""
    fl = 0
    ct = 0
    isdir = _st.S_ISDIR(mode)
    if isdir:
        fl = F_DIR
    elif _st.S_ISLNK(mode):
        fl = F_LINK
    if parent_hidden or name[:1] == "." or name[-1:] == "~" or (win and name.lower() in WIN_HIDDEN):
        fl |= F_HIDDEN
    if not isdir:
        dot = name.rfind(".")
        if dot > 0:
            ct = EXT_CAT.get(name[dot + 1:].lower(), 0)
        if exec_ok and not ct and _st.S_ISREG(mode) and mode & 0o111:
            ct = C_EXE
            fl |= F_EXEC
    return fl, ct


def make_chunk(names: list[bytes]):
    """names (stored bytes) → (orig blob, lower blob, offsets, local name order)."""
    orig = b"\n" + b"\n".join(names) + b"\n"
    if orig.isascii():
        low = orig.lower()
        keys = low[1:-1].split(b"\n") if names else []
    else:
        lows = list(map(lower_b, names))
        low = b"\n" + b"\n".join(lows) + b"\n"
        keys = lows
    if orig == low:
        low = orig
    offs = array.array("I", itertools.accumulate(map((1).__add__, map(len, names)), initial=1))
    keys = [fold_key(k) for k in keys] if not low.isascii() else keys
    lorder = array.array("H", sorted(range(len(names)), key=keys.__getitem__)) if names else array.array("H")
    return orig, low, offs, lorder


# ---------------------------------------------------------------------------
# mounts / roots
# ---------------------------------------------------------------------------

def _unescape_mount(s: str) -> str:
    return re.sub(r"\\([0-7]{3})", lambda m: chr(int(m.group(1), 8)), s)


def read_mounts() -> list[dict]:
    out = []
    try:
        with open("/proc/self/mountinfo", encoding="utf-8", errors="surrogateescape") as f:
            for line in f:
                parts = line.split()
                if "-" not in parts:
                    continue
                sep = parts.index("-")
                if len(parts) < sep + 3 or len(parts) < 5:
                    continue
                out.append({"root": _unescape_mount(parts[3]), "mount": _unescape_mount(parts[4]),
                            "fs": parts[sep + 1], "source": _unescape_mount(parts[sep + 2]),
                            "dev": parts[2]})
    except OSError:
        pass
    return out


def _disk_of(source: str) -> str:
    """Physical disk for a block device (/dev/nvme0n1p2 → nvme0n1); '' if unknown."""
    if not source.startswith("/dev/"):
        return ""
    try:
        real = os.path.realpath(source)
        name = os.path.basename(real)
        sysp = os.path.realpath(f"/sys/class/block/{name}")
        if os.path.exists(os.path.join(sysp, "partition")):
            return os.path.basename(os.path.dirname(sysp))
        return name
    except OSError:
        return os.path.basename(source)


def mount_for(path: str, mounts: list[dict] | None = None) -> dict | None:
    mounts = mounts if mounts is not None else read_mounts()
    best = None
    for m in mounts:
        mp = m["mount"]
        if path == mp or path.startswith(mp.rstrip("/") + "/"):
            if best is None or len(mp) >= len(best["mount"]):
                best = m
    return best


def auto_roots(mounts: list[dict] | None = None) -> list[str]:
    """SPEC default: '/' plus every mounted local disk under /home, /run/media, /media, /mnt, /srv."""
    mounts = mounts if mounts is not None else read_mounts()
    roots = ["/"]
    seen = {"/"}
    for m in mounts:
        mp, fs = m["mount"], m["fs"]
        if mp in seen or fs not in REAL_FS:
            continue
        if not any(mp == p or mp.startswith(p + "/") for p in AUTO_PREFIXES):
            continue
        if m["root"] not in ("/", "") and fs not in ("btrfs",):
            continue   # bind mount of a sub directory
        if any(mp == x or mp.startswith(x + "/") for x in DEFAULT_EXCLUDES):
            continue
        roots.append(mp)
        seen.add(mp)
    return roots


# ---------------------------------------------------------------------------
# crawling
# ---------------------------------------------------------------------------

class SegData:
    """One crawled root: local arrays (parent = local index, -1 for the root dir)."""

    __slots__ = ("path", "fs", "dev", "disk", "n", "parent", "size", "mtime", "flags", "cat", "chunks",
                 "built", "errors", "noshow_root")

    def __init__(self, path: str):
        self.path = path
        self.fs = ""
        self.dev = 0
        self.disk = ""
        self.n = 0
        self.parent = array.array("i")
        self.size = array.array("q")
        self.mtime = array.array("q")
        self.flags = bytearray()
        self.cat = bytearray()
        self.chunks: list[tuple] = []   # (orig, low, offs, lorder)
        self.built = 0.0
        self.errors = 0
        self.noshow_root = False


class Progress:
    __slots__ = ("done", "current", "estimate", "state", "error", "started")

    def __init__(self, estimate: int = 0):
        self.done = 0
        self.current = ""
        self.estimate = estimate
        self.state = "pending"
        self.error = None
        self.started = 0.0


def estimate_entries(path: str) -> int:
    try:
        s = os.statvfs(path)
        if s.f_files and s.f_files > s.f_ffree:
            used = s.f_files - s.f_ffree
            if used < 50_000_000:
                return int(used)
        used_bytes = (s.f_blocks - s.f_bfree) * s.f_frsize
        return max(1000, int(used_bytes / (4 << 20)))   # ~4 MB per file on media drives
    except OSError:
        return 0


def crawl_into(sd: SegData, start_dir: str, start_parent: int, start_hidden: bool, *, dev: int,
               excludes: set, exec_ok: bool, win: bool, cancel: threading.Event | None,
               progress: Progress | None, names: list, include_start: bool = False,
               start_name: str | None = None, start_stat=None) -> list:
    """BFS crawl of ``start_dir`` appending entries to ``sd`` (local indices).

    ``names`` collects stored names of the current, unfinished chunk; finished
    chunks go to ``sd.chunks``. Returns the pending ``names`` list.
    """
    parent, size, mtime, flags, cat = sd.parent, sd.size, sd.mtime, sd.flags, sd.cat
    p_ap, s_ap, m_ap, f_ap, c_ap = parent.append, size.append, mtime.append, flags.append, cat.append
    S_ISDIR, S_ISLNK = _st.S_ISDIR, _st.S_ISLNK
    q = deque()
    if include_start:
        st = start_stat
        fl, ct = classify(start_name, st.st_mode, start_hidden, exec_ok, win)
        names.append(store_name(start_name))
        p_ap(start_parent); s_ap(0); m_ap(st.st_mtime_ns // 1_000_000); f_ap(fl); c_ap(ct)
        idx = sd.n
        sd.n += 1
        if len(names) >= CHUNK:
            sd.chunks.append(make_chunk(names)); names = []
        q.append((idx, start_dir, bool(fl & F_HIDDEN)))
    else:
        q.append((start_parent, start_dir, start_hidden))
    n = sd.n
    tick = 0
    scandir = os.scandir
    while q:
        di, dp, dh = q.popleft()
        if cancel is not None and cancel.is_set():
            break
        if progress is not None:
            progress.current = dp
        try:
            it = scandir(dp)
        except OSError:
            sd.errors += 1
            continue
        prefix = dp if dp.endswith("/") else dp + "/"
        with it:
            while True:
                try:
                    e = next(it)
                except StopIteration:
                    break
                except OSError:
                    sd.errors += 1
                    break
                try:
                    st = e.stat(follow_symlinks=False)
                except OSError:
                    continue
                name = e.name
                mode = st.st_mode
                isdir = S_ISDIR(mode)
                if isdir:
                    full = prefix + name
                    if full in excludes:
                        continue
                    fl = F_DIR
                    ct = 0
                    if dh or name[0] == "." or name[-1] == "~" or (win and name.lower() in WIN_HIDDEN):
                        fl |= F_HIDDEN
                    names.append(store_name(name))
                    p_ap(di); s_ap(0); m_ap(st.st_mtime_ns // 1_000_000)
                    if st.st_dev != dev:
                        fl |= F_MOUNT
                    else:
                        q.append((n, full, bool(fl & F_HIDDEN)))
                    f_ap(fl); c_ap(0)
                else:
                    fl, ct = classify(name, mode, dh, exec_ok, win)
                    names.append(store_name(name))
                    p_ap(di); s_ap(0 if S_ISLNK(mode) else st.st_size); m_ap(st.st_mtime_ns // 1_000_000)
                    f_ap(fl); c_ap(ct)
                n += 1
                if len(names) >= CHUNK:
                    sd.n = n
                    sd.chunks.append(make_chunk(names))
                    names = []
                tick += 1
                if tick >= 1024:
                    tick = 0
                    if progress is not None:
                        progress.done += 1024
                    if cancel is not None and cancel.is_set():
                        break
        sd.n = n
    sd.n = n
    if progress is not None:
        progress.done += tick
    return names


def root_name(path: str) -> str:
    """Searchable name of an index root: what the user sees ("BACKUP", not "/run/media/robin/BACKUP"),
    so a query only matches a root by its own name. The full path is kept in ``Seg.path``."""
    p = path.rstrip("/")
    return os.path.basename(p) or "/"


def crawl_root(path: str, *, excludes: set, fs: str = "", disk: str = "", cancel=None,
               progress: Progress | None = None) -> SegData:
    sd = SegData(path)
    sd.fs = fs
    sd.disk = disk
    st = os.stat(path)
    if not _st.S_ISDIR(st.st_mode):
        raise NotADirectoryError(path)
    sd.dev = st.st_dev
    win = fs in WIN_FS
    exec_ok = not win
    names = [store_name(root_name(path))]
    sd.parent.append(-1); sd.size.append(0); sd.mtime.append(st.st_mtime_ns // 1_000_000)
    sd.flags.append(F_DIR | F_ROOT); sd.cat.append(0)
    sd.n = 1
    names = crawl_into(sd, path, 0, False, dev=st.st_dev, excludes=excludes, exec_ok=exec_ok, win=win,
                       cancel=cancel, progress=progress, names=names)
    if names:
        sd.chunks.append(make_chunk(names))
    accumulate_sizes(sd.parent, sd.size, sd.flags, 0, sd.n)
    sd.built = time.time() * 1000
    return sd


def synthetic_segdata(n: int, root: str = "/synthetic", seed: int = 1) -> SegData:
    """An in-memory segment of ``n`` realistic-looking entries (benchmarks/tests; no disk access)."""
    import random
    rnd = random.Random(seed)
    words = ("foto vakantie factuur rapport img dsc scan document notitie muziek film backup project test "
             "data config readme index main style app lib src build cache log verslag offerte contract "
             "brief recept kerst verjaardag strand bergen camping screenshot schermafbeelding opname "
             "presentatie begroting belasting hypotheek zorg school huiswerk scriptie cv sollicitatie "
             "album nummer podcast serie aflevering seizoen trailer concert bruiloft familie oma opa "
             "IMG DSC VID Screenshot Document Factuur Rapport README CHANGELOG LICENSE Makefile").split()
    exts = ("jpg jpg jpg png png heic mp3 flac mp4 mkv mov pdf pdf docx xlsx pptx txt md py js ts json html "
            "css zip tar.gz 7z iso deb sh log csv odt ods svg webp gif wav ogg avi").split()
    sd = SegData(root)
    sd.fs = "ext4"
    parent, size, mtime, flags, cat = sd.parent, sd.size, sd.mtime, sd.flags, sd.cat
    names = [store_name(root_name(root))]
    parent.append(-1); size.append(0); mtime.append(1_700_000_000_000); flags.append(F_DIR | F_ROOT); cat.append(0)
    dirs = deque([(0, False, 0)])
    count = 1
    now_ms = int(time.time() * 1000)
    rr = rnd.random
    ri = rnd.randint
    ch = rnd.choice
    while count < n and dirs:
        d, dh, depth = dirs.popleft()
        k = ri(3, 35)
        for _ in range(k):
            if count >= n:
                break
            isdir = rr() < (0.2 if depth < 4 else 0.1)
            w = ch(words)
            if isdir:
                nm = w if rr() < 0.5 else f"{w} {ri(2000, 2026)}"
                if rr() < 0.03:
                    nm = "." + nm.lower()
                fl = F_DIR | (F_HIDDEN if dh or nm[0] == "." else 0)
                names.append(store_name(nm))
                parent.append(d); size.append(0); mtime.append(now_ms - ri(0, 3 * 365 * 86400) * 1000)
                flags.append(fl); cat.append(0)
                dirs.append((count, bool(fl & F_HIDDEN), depth + 1))
            else:
                e = ch(exts)
                r = rr()
                if r < 0.35:
                    nm = f"{w}_{ri(0, 99999):05d}.{e}"
                elif r < 0.6:
                    nm = f"{w} {ch(words)} {ri(1, 300)}.{e}"
                elif r < 0.8:
                    nm = f"{w}-{ri(2010, 2026)}{ri(1, 12):02d}{ri(1, 28):02d}.{e}"
                else:
                    nm = f"{w}.{e}"
                fl, ct = classify(nm, 0o100644, dh, True, False)
                names.append(store_name(nm))
                parent.append(d); size.append(int(rnd.lognormvariate(11, 2.2)))
                mtime.append(now_ms - ri(0, 5 * 365 * 86400) * 1000)
                flags.append(fl); cat.append(ct)
            count += 1
            if len(names) >= CHUNK:
                sd.chunks.append(make_chunk(names))
                names = []
    if names:
        sd.chunks.append(make_chunk(names))
    sd.n = count
    accumulate_sizes(parent, size, flags, 0, count)
    sd.built = time.time() * 1000
    return sd


def accumulate_sizes(parent, size, flags, start: int, end: int, offset: int = 0) -> None:
    """Folder size = total of everything below it (BFS order: children after parents)."""
    for i in range(end - 1, start, -1):
        s = size[i]
        if s:
            p = parent[i] - offset
            if p >= start:
                size[p] += s


# ---------------------------------------------------------------------------
# the index
# ---------------------------------------------------------------------------

class Seg:
    __slots__ = ("path", "start", "n", "fs", "dev", "disk", "built", "dead", "errors", "noshow_root", "win",
                 "exec_ok")

    def __init__(self, path, start, n, fs="", dev=0, disk="", built=0.0, errors=0):
        self.path = path
        self.start = start
        self.n = n
        self.fs = fs
        self.dev = dev
        self.disk = disk
        self.built = built
        self.dead = 0
        self.errors = errors
        self.noshow_root = False
        self.win = fs in WIN_FS
        self.exec_ok = not self.win

    @property
    def end(self):
        return self.start + self.n


class Chunk:
    """Names of up to 65 536 consecutive gids: ``b"\\n" + b"\\n".join(names) + b"\\n"`` in original
    case and lower-cased (one ``offs`` array for both; the very same blob when they are equal).

    A blob is either heap ``bytes`` (``om``/``lm`` = the blob, ``ob``/``lb`` = 0) or a window of a
    read-only mmap of names.orig / names.low (``om``/``lm`` = the mmap, ``ob``/``lb`` = the chunk's
    byte offset in the file). Name i is ``om[ob + offs[i]:ob + offs[i + 1] - 1]`` either way.
    ``.orig`` / ``.low`` give the whole blob as bytes: a transient 2 MB copy for a mapped chunk."""
    __slots__ = ("start", "n", "om", "ob", "olen", "lm", "lb", "llen", "offs", "lorder", "vruns")

    def __init__(self, start, orig, low, offs, lorder, *, ob=0, olen=None, lb=0, llen=None):
        self.start = start
        self.om = orig
        self.ob = ob
        self.olen = len(orig) if olen is None else olen
        self.lm = low
        self.lb = lb
        self.llen = len(low) if llen is None else llen
        self.offs = offs
        self.lorder = lorder
        self.n = len(offs) - 1
        self.vruns = None

    @property
    def orig(self) -> bytes:
        om = self.om
        if type(om) is bytes:
            return om
        return om[self.ob:self.ob + self.olen]

    @property
    def low(self) -> bytes:
        lm = self.lm
        if type(lm) is bytes:
            return lm
        if lm is self.om and self.lb == self.ob:
            return self.orig
        return lm[self.lb:self.lb + self.llen]

    def low_is_orig(self) -> bool:
        """The lower-case blob is the original one (a chunk without upper-case names)."""
        return self.lm is self.om and self.lb == self.ob


T_HIDDEN_ANY = _table(lambda f: bool(f & F_HIDDEN))
_ZERO_RUNS = re.compile(rb"\x00+")


def chunk_vis_runs(idx: "Index", c: Chunk):
    """Runs [a, b) of non-hidden entries of a chunk (small hidden gaps merged), or False when the
    chunk is mostly visible and should simply be scanned whole. Hidden flags never change for
    base entries, so this is computed once per chunk."""
    r = c.vruns
    if r is None:
        h = bytes(idx.flags[c.start:c.start + c.n].translate(T_HIDDEN_ANY))
        if h.count(1) < c.n * 0.2:
            r = False
        else:
            runs: list = []
            for m in _ZERO_RUNS.finditer(h):
                a, b = m.span()
                if runs and a - runs[-1][1] < 48:
                    runs[-1] = (runs[-1][0], b)
                else:
                    runs.append((a, b))
            r = runs
        c.vruns = r
    return r


class Overlay:
    """Entries added since the base was built. gid = base + k."""

    def __init__(self, base: int):
        self.base = base
        self.orig: list[bytes] = []
        self.low: list[bytes] = []
        self.parent: list[int] = []
        self.size: list[int] = []
        self.mtime: list[int] = []
        self.flags = bytearray()
        self.cat = bytearray()
        self.seg: list[int] = []
        self.kids: dict[int, list[int]] = {}
        self.dead = 0

    def __len__(self):
        return len(self.orig)

    def live(self) -> int:
        return len(self.orig) - self.dead


class Index:
    def __init__(self):
        self.segs: list[Seg] = []
        self.chunks: list[Chunk] = []
        self.cstarts: list[int] = []
        self.N = 0
        self.parent = array.array("i")
        self.size = array.array("q")
        self.mtime = array.array("q")
        self.flags = bytearray()
        self.cat = bytearray()
        self.order = array.array("I")
        self.rank = array.array("I")
        self.ov = Overlay(0)
        self.gen = 0          # bumps on every content change (results cache key)
        self.sgen = 0         # bumps when the gid space changes
        self.orders: dict[str, array.array] = {}
        self.odirty: dict[str, set] = {}
        self._dpath: dict[int, str] = {}
        self._kidmap: OrderedDict[int, dict] = OrderedDict()
        self._counts = None
        self.home: str | None = None        # index dir the base was loaded from (None: built in the heap)
        self.home_id: tuple | None = None   # (st_dev, st_ino) of home/meta.json as loaded / saved
        self.base_id: str | None = None     # identifies the base layout (names, parent, order) on disk
        self.maps: dict[str, object] = {}   # file name -> mmap the arrays / blobs are views of
        self.mapped = False                 # the base arrays and blobs are views of the files in home
        self.order_files: dict[str, str] = {}   # orders key -> its file in home (a save links it)
        self._olock = threading.RLock()     # one order build at a time (a path order needs its key)

    # -- construction ------------------------------------------------------------
    @classmethod
    def assemble(cls, segdatas: list[SegData], cancel=None) -> "Index":
        idx = cls()
        gid = 0
        for sd in segdatas:
            seg = Seg(sd.path, gid, sd.n, sd.fs, sd.dev, sd.disk, sd.built, sd.errors)
            idx.segs.append(seg)
            if gid:
                par = array.array("i", map(gid.__add__, sd.parent))
                par[0] = -1
            else:
                par = sd.parent
            idx.parent.extend(par)
            idx.size.extend(sd.size)
            idx.mtime.extend(sd.mtime)
            fl = bytearray(sd.flags)
            if sd.noshow_root and len(fl):
                fl[0] |= F_NOSHOW
            idx.flags.extend(fl)
            idx.cat.extend(sd.cat)
            local = 0
            for (orig, low, offs, lorder) in sd.chunks:
                c = Chunk(gid + local, orig, low, offs, lorder)
                idx.chunks.append(c)
                idx.cstarts.append(c.start)
                local += c.n
            if local != sd.n:
                raise RuntimeError(f"segment {sd.path}: {local} names for {sd.n} entries")
            gid += sd.n
        idx.N = gid
        idx.ov = Overlay(gid)
        idx.compute_order(cancel)
        return idx

    def compute_order(self, cancel=None) -> None:
        """Global name order by k-way merging the per-chunk sorted runs (low memory)."""
        def run(c: Chunk):
            low, offs, base = c.low, c.offs, c.start
            asc = low.isascii()
            for i in c.lorder:
                k = low[offs[i]:offs[i + 1] - 1]
                yield (k if asc else fold_key(k)), base + i
        order = array.array("I")
        if len(self.chunks) == 1:
            c = self.chunks[0]
            order.extend(map(c.start.__add__, c.lorder))
        else:
            order.extend(g for _k, g in heapq.merge(*map(run, self.chunks)))
        rank = array.array("I", bytes(4 * self.N))
        for r, g in enumerate(order):
            rank[g] = r
        self.order = order
        self.rank = rank

    # -- access --------------------------------------------------------------------
    def chunk_of(self, g: int) -> tuple[Chunk, int]:
        c = self.chunks[bisect.bisect_right(self.cstarts, g) - 1]
        return c, g - c.start

    def seg_index(self, g: int) -> int:
        if g >= self.N:
            return self.ov.seg[g - self.N]
        lo, hi = 0, len(self.segs) - 1
        while lo < hi:
            mid = (lo + hi + 1) // 2
            if self.segs[mid].start <= g:
                lo = mid
            else:
                hi = mid - 1
        return lo

    def seg_of(self, g: int) -> Seg:
        return self.segs[self.seg_index(g)]

    def name_low(self, g: int) -> bytes:
        if g >= self.N:
            return self.ov.low[g - self.N]
        c = self.chunks[bisect.bisect_right(self.cstarts, g) - 1]
        i = g - c.start
        o = c.offs
        b = c.lb
        return c.lm[b + o[i]:b + o[i + 1] - 1]

    def name_orig(self, g: int) -> bytes:
        if g >= self.N:
            return self.ov.orig[g - self.N]
        c = self.chunks[bisect.bisect_right(self.cstarts, g) - 1]
        i = g - c.start
        o = c.offs
        b = c.ob
        return c.om[b + o[i]:b + o[i + 1] - 1]

    def name(self, g: int) -> str:
        return stored_to_str(self.name_orig(g))

    def flags_of(self, g: int) -> int:
        return self.flags[g] if g < self.N else self.ov.flags[g - self.N]

    def size_of(self, g: int) -> int:
        return self.size[g] if g < self.N else self.ov.size[g - self.N]

    def mtime_of(self, g: int) -> int:
        return self.mtime[g] if g < self.N else self.ov.mtime[g - self.N]

    def cat_of(self, g: int) -> int:
        return self.cat[g] if g < self.N else self.ov.cat[g - self.N]

    def parent_of(self, g: int) -> int:
        return self.parent[g] if g < self.N else self.ov.parent[g - self.N]

    def alive(self, g: int) -> bool:
        return not self.flags_of(g) & (F_DEL | F_NOSHOW)

    def dir_path(self, d: int, cache: dict | None = None) -> str:
        """Full path of folder d. ``cache`` (default: the shared, capped ``_dpath``) memoises the
        paths of d and its ancestors; a caller walking many folders passes its own."""
        if cache is None:
            cache = self._dpath
        p = cache.get(d)
        if p is not None:
            return p
        chain = []
        x = d
        while True:
            p = cache.get(x)
            if p is not None:
                break
            par = self.parent_of(x)
            if par < 0:
                p = self.seg_of(x).path          # a root: its name is only the basename
                cache[x] = p
                break
            chain.append(x)
            x = par
        for x in reversed(chain):
            p = join_path(p, self.name(x))
            cache[x] = p
        if len(cache) > DPATH_MAX and cache is self._dpath:
            cache.clear()
            cache[d] = p
        return p

    def path(self, g: int) -> str:
        par = self.parent_of(g)
        if par < 0:
            return self.seg_of(g).path
        if self.flags_of(g) & F_DIR:
            return self.dir_path(g)
        return join_path(self.dir_path(par), self.name(g))

    def ancestors(self, g: int):
        p = self.parent_of(g)
        while p >= 0:
            yield p
            p = self.parent_of(p)

    def base_kid_range(self, g: int) -> tuple[int, int]:
        """Children of base dir g as the contiguous gid range [lo, hi)."""
        seg = self.seg_of(g)
        lo = bisect.bisect_left(self.parent, g, seg.start + 1, seg.end)
        hi = bisect.bisect_right(self.parent, g, lo, seg.end)
        return lo, hi

    def kids(self, g: int, include_dead: bool = False) -> list[int]:
        out: list[int] = []
        if g < self.N:
            lo, hi = self.base_kid_range(g)
            if include_dead:
                out = list(range(lo, hi))
            else:
                fl = self.flags
                out = [c for c in range(lo, hi) if not fl[c] & F_DEL]
        ok = self.ov.kids.get(g)
        if ok:
            if include_dead:
                out.extend(ok)
            else:
                ofl, base = self.ov.flags, self.N
                out.extend(c for c in ok if not ofl[c - base] & F_DEL)
        return out

    def child_by_name(self, d: int, nb: bytes) -> int | None:
        m = self._kidmap.get(d)
        if m is None:
            m = {self.name_orig(c): c for c in self.kids(d)}
            self._kidmap[d] = m
            if len(self._kidmap) > 512:
                self._kidmap.popitem(last=False)
        else:
            self._kidmap.move_to_end(d)
        return m.get(nb)

    def seg_for_path(self, path: str) -> int:
        best, blen = -1, -1
        for i, s in enumerate(self.segs):
            r = s.path
            if path == r or path.startswith(r if r.endswith("/") else r + "/"):
                if len(r) > blen:
                    best, blen = i, len(r)
        return best

    def lookup(self, path: str) -> int | None:
        path = norm_path(path)
        si = self.seg_for_path(path)
        if si < 0:
            return None
        seg = self.segs[si]
        g = seg.start
        if self.flags[g] & F_DEL:
            return None
        rest = path[len(seg.path):].strip("/")
        if not rest:
            return g
        for comp in rest.split("/"):
            if not comp:
                continue
            g2 = self.child_by_name(g, store_name(comp))
            if g2 is None:
                return None
            g = g2
        return g

    def counts(self) -> tuple[int, int]:
        if self._counts and self._counts[0] == self.gen:
            return self._counts[1], self._counts[2]
        dirs = self.flags.translate(T_ADIR).count(1)
        files = self.flags.translate(T_AFILE).count(1)
        ofl = self.ov.flags
        if len(ofl):
            dirs += ofl.translate(T_ADIR).count(1)
            files += ofl.translate(T_AFILE).count(1)
        self._counts = (self.gen, files, dirs)
        return files, dirs

    # -- mutation (writer thread, under the engine lock) -------------------------
    def touch(self) -> None:
        self.gen += 1

    def _forget_kid(self, g: int) -> None:
        par = self.parent_of(g)
        m = self._kidmap.get(par)
        if m is not None:
            nb = self.name_orig(g)
            if m.get(nb) == g:
                del m[nb]

    def add_entry(self, pg: int, name: str, st, seg_i: int) -> int:
        seg = self.segs[seg_i]
        pfl = self.flags_of(pg)
        fl, ct = classify(name, st.st_mode, bool(pfl & F_HIDDEN), seg.exec_ok, seg.win)
        isdir = bool(fl & F_DIR)
        if isdir and st.st_dev != seg.dev:
            fl |= F_MOUNT
        sz = 0 if (isdir or fl & F_LINK) else st.st_size
        ov = self.ov
        g = ov.base + len(ov.orig)
        nb = store_name(name)
        ov.orig.append(nb)
        ov.low.append(lower_b(nb))
        ov.parent.append(pg)
        ov.size.append(sz)
        ov.mtime.append(st.st_mtime_ns // 1_000_000)
        ov.flags.append(fl)
        ov.cat.append(ct)
        ov.seg.append(seg_i)
        ov.kids.setdefault(pg, []).append(g)
        m = self._kidmap.get(pg)
        if m is not None:
            m[nb] = g
        if sz:
            self._add_size(pg, sz)
        return g

    def _add_size(self, g: int, delta: int) -> None:
        x = g
        N = self.N
        while x >= 0:
            if x < N:
                self.size[x] += delta
                self._mark_dirty(x, "size")
                x = self.parent[x]
            else:
                k = x - N
                self.ov.size[k] += delta
                x = self.ov.parent[k]

    def _mark_dirty(self, g: int, kind: str) -> None:
        d = self.odirty.get(kind)
        if d is not None:
            d.add(g)
            if len(d) > ODIRTY_MAX:
                self.orders.pop(kind, None)
                self.odirty.pop(kind, None)
                self.order_files.pop(kind, None)

    def update_stat(self, g: int, st) -> bool:
        fl = self.flags_of(g)
        mt = st.st_mtime_ns // 1_000_000
        changed = False
        if fl & F_DIR:
            if self.mtime_of(g) != mt:
                self._set_mtime(g, mt)
                changed = True
            return changed
        sz = 0 if fl & F_LINK else st.st_size
        old = self.size_of(g)
        if old != sz:
            par = self.parent_of(g)
            if g < self.N:
                self.size[g] = sz
                self._mark_dirty(g, "size")
            else:
                self.ov.size[g - self.N] = sz
            if par >= 0:
                self._add_size(par, sz - old)
            changed = True
        if self.mtime_of(g) != mt:
            self._set_mtime(g, mt)
            changed = True
        seg = self.seg_of(g)
        if seg.exec_ok and _st.S_ISREG(st.st_mode):
            want = bool(st.st_mode & 0o111)
            has = bool(fl & F_EXEC)
            if want != has:
                nfl = fl ^ F_EXEC
                ct = self.cat_of(g)
                if want and not ct:
                    ct = C_EXE
                elif not want and ct == C_EXE and not EXT_CAT.get(ext_of(self.name(g))):
                    ct = 0
                if g < self.N:
                    self.flags[g] = nfl
                    self.cat[g] = ct
                else:
                    self.ov.flags[g - self.N] = nfl
                    self.ov.cat[g - self.N] = ct
                changed = True
        return changed

    def _set_mtime(self, g: int, mt: int) -> None:
        if g < self.N:
            self.mtime[g] = mt
            self._mark_dirty(g, "modified")
        else:
            self.ov.mtime[g - self.N] = mt

    def delete_subtree(self, g: int) -> int:
        if self.flags_of(g) & F_DEL:
            return 0
        sz = self.size_of(g)
        par = self.parent_of(g)
        self._forget_kid(g)
        if sz and par >= 0:
            self._add_size(par, -sz)
        n = 0
        stack = [g]
        N = self.N
        while stack:
            x = stack.pop()
            if x < N:
                if self.flags[x] & F_DEL:
                    continue
                self.flags[x] |= F_DEL
                self.segs[self.seg_index(x)].dead += 1
            else:
                k = x - N
                if self.ov.flags[k] & F_DEL:
                    continue
                self.ov.flags[k] |= F_DEL
                self.ov.dead += 1
            n += 1
            if self.flags_of(x) & F_DIR:
                stack.extend(self.kids(x))
                self._kidmap.pop(x, None)
        return n

    # -- sort orders: lazily built, persisted next to a mapped base ---------------------
    # orders[kind] = base gids sorted by kind (ties in name order); orders["key:path"|"key:type"] =
    # per-gid sort keys. For a mapped base they are built once in a helper process (the sort of
    # 6.4 M Python ints costs ~0.6 GB for a few seconds, which then never touches this process),
    # written to home/order.<kind>.bin / key.<kind>.bin, mapped, and listed in home/orders.json
    # (with the dirty sets) so the next start and every save reuse them. odirty[kind] = base gids
    # whose size / mtime changed since the order was built: they are merged in at query time.
    def get_order(self, kind: str, cancel=None) -> array.array:
        o = self.orders.get(kind)
        if o is not None:
            return o
        if kind not in ORDER_KINDS:
            return self.order
        return self._built(kind)

    def key_array(self, kind: str) -> array.array:
        """Per-gid integer sort key: 'path' = rank of the parent folder path, 'type' = extension rank."""
        a = self.orders.get("key:" + kind)
        return a if a is not None else self._built("key:" + kind)

    @staticmethod
    def order_file(k: str) -> str:
        return ("key." + k[4:] if k.startswith("key:") else "order." + k) + ".bin"

    def _built(self, k: str):
        with self._olock:
            a = self.orders.get(k)
            if a is not None:
                return a
            kind = k[4:] if k.startswith("key:") else k
            got = self._helper_build(k, kind)
            if got is None:                              # in this process (heap base, no helper, failed)
                a = self._compute(k)
                self.orders[k] = a
                if kind in ("size", "modified"):
                    self.odirty[kind] = set()
            return self.orders[k]

    def _compute(self, k: str) -> array.array:
        if k == "key:path":
            return self._frank_array()
        if k == "key:type":
            return self._ext_rank_array()
        if k == "size":
            return array.array("I", sorted(self.order, key=self.size.__getitem__))
        if k == "modified":
            return array.array("I", sorted(self.order, key=self.mtime.__getitem__))
        return array.array("I", sorted(self.order, key=self.key_array(k).__getitem__))

    def _disk_dirty(self, kind: str) -> set | None:
        """Base gids whose live size / mtime differs from home's file (the changes since the last
        save), or None when that cannot be told or they are too many (the helper then sorts the
        live values instead)."""
        fn = "size.bin" if kind == "size" else "mtime.bin"
        cur = self.maps.get(fn)
        if cur is None:
            return None
        try:
            disk = map_file(os.path.join(self.home, fn))
        except OSError:                          # home was removed or replaced meanwhile
            return None
        if disk is None or len(disk) != len(cur):
            if disk is not None:
                disk.close()
            return None
        sv = self.size if kind == "size" else self.mtime
        dv = memoryview(disk).cast("q")
        out: set = set()
        BS = 1 << 16
        try:
            for off in range(0, len(cur), BS):
                if cur[off:off + BS] != disk[off:off + BS]:
                    lo = off // 8
                    out.update(g for g in range(lo, min(self.N, lo + BS // 8)) if sv[g] != dv[g])
                    if len(out) > ODIRTY_MAX:
                        return None
        finally:
            dv.release()
            disk.close()
        return out

    def _helper_build(self, k: str, kind: str):
        """Build k (and the key it needs) in a helper process that maps home and writes the files
        there; map them. None = not possible here (the caller sorts in this process)."""
        if not (helpers_on() and self.mapped and self.home and self.base_id and self.N):
            return None
        if _dir_id(self.home) != self.home_id:
            return None                                 # home was removed or replaced (a save follows)
        targets = [k]
        if not k.startswith("key:") and kind in ("path", "type") and "key:" + kind not in self.order_files:
            targets.insert(0, "key:" + kind)
        J = os.path.join
        sfx = f".tmp-{os.getpid()}"
        dirty = None
        live = None
        if kind in ("size", "modified") and k == kind:
            dirty = self._disk_dirty(kind)              # the helper sorts the values on disk ...
            if dirty is None:                           # ... or, after many changes, the live ones
                live = J(self.home, ("size" if kind == "size" else "mtime") + ".live" + sfx)
                dirty = set()
        tmps = [J(self.home, self.order_file(t)) + sfx for t in targets]
        try:
            _check_space(self.home, 4 * self.N * len(targets) + (8 * self.N if live else 0))
            args = ["order", "--index", self.home, "--base-id", self.base_id,
                    "--targets", ",".join(targets), "--suffix", sfx]
            if live:
                _write_array(live, self.size if kind == "size" else self.mtime)
                args += ["--values", live]
            if run_helper(args, timeout=HELPER_TIMEOUT) != 0:
                return None
            views = {}
            for t, tmp in zip(targets, tmps):
                fn = self.order_file(t)
                os.rename(tmp, J(self.home, fn))        # a new file (an older one may still be mapped)
                views[t] = (fn, map_view(J(self.home, fn), "I", self.N, self.maps, fn))
        except (OSError, ValueError) as e:
            log("sorteervolgorde %s in een hulpproces mislukt: %s", k, e)
            return None
        finally:
            for tmp in tmps + ([live] if live else []):
                if os.path.exists(tmp):
                    try:
                        os.unlink(tmp)
                    except OSError:
                        pass
        for t, (fn, v) in views.items():
            self.orders[t] = v
            self.order_files[t] = fn
        if dirty is not None:
            self.odirty[kind] = dirty
        # sorted by the live values: home's size / mtime file does not match it, so home's
        # orders.json must not list it (the next save does, with dirty sets relative to its files)
        self._record_orders([t for t in views if not (live and t == k)], drop=(k,) if live else ())
        return views

    def _record_orders(self, new: list, drop=()) -> None:
        """List new order files in home/orders.json (new file + rename), so the next start reuses
        them. Dirty sets there are relative to home's size / mtime files: none for a fresh build."""
        p = os.path.join(self.home, "orders.json")
        od = _read_orders(self.home)
        if not od or od.get("base_id") != self.base_id or od.get("N") != self.N:
            od = {"base_id": self.base_id, "N": self.N, "files": {}, "dirty": {}}
        od["format"] = ORDERS_FORMAT
        od.setdefault("dirty", {})
        for t in drop:
            od["files"].pop(t, None)
            od["dirty"].pop(t, None)
        for t in new:
            od["files"][t] = self.order_files[t]
            if t in ("size", "modified"):
                od["dirty"][t] = []
        tmp = p + f".tmp-{os.getpid()}"
        try:
            with open(tmp, "w") as f:
                json.dump(od, f)
            os.rename(tmp, p)
        except OSError as e:
            log("orders.json schrijven mislukt: %s", e)

    def _frank_array(self) -> array.array:
        """Per gid: rank of its parent folder's lower-cased full path (roots: 0)."""
        dirs = list(compress(range(self.N), self.flags.translate(T_DIR)))
        keys = {}
        paths: dict[int, str] = {}       # all folders at once, then freed (not the shared _dpath)
        for d in dirs:
            keys[d] = lower_b(enc(self.dir_path(d, paths)))
        del paths
        ranked = sorted(dirs, key=keys.__getitem__)
        frank_of_dir = array.array("I", bytes(4 * self.N))
        for r, d in enumerate(ranked, 1):
            frank_of_dir[d] = r
        par = self.parent
        out = array.array("I", bytes(4 * self.N))
        for g in range(self.N):
            p = par[g]
            out[g] = frank_of_dir[p] if p >= 0 else 0
        return out

    def _ext_rank_array(self) -> array.array:
        """Per gid: rank of its (lower-case) extension; folders sort first."""
        ids: dict[bytes, int] = {b"": 0}
        per = array.array("I", bytes(4 * self.N))
        fl = self.flags
        for c in self.chunks:
            if not c.n:
                continue
            base = c.start
            for i, nb in enumerate(c.low[1:-1].split(b"\n")):
                if fl[base + i] & F_DIR:
                    continue
                dot = nb.rfind(b".")
                e = nb[dot + 1:] if dot > 0 else b""
                v = ids.get(e)
                if v is None:
                    v = ids[e] = len(ids)
                per[base + i] = v
        ranked = sorted(ids, key=lambda e: (e != b"", e))
        rank_of = array.array("I", bytes(4 * len(ids)))
        for r, e in enumerate(ranked):
            rank_of[ids[e]] = r
        return array.array("I", map(rank_of.__getitem__, per))

    # -- result keys (for merging overlay / dirty entries into a sorted base) --------
    def sort_key(self, kind: str):
        nk = lambda g: fold_key(self.name_low(g))  # noqa: E731
        if kind == "size":
            return lambda g: (self.size_of(g), nk(g))
        if kind == "modified":
            return lambda g: (self.mtime_of(g), nk(g))
        if kind == "path":
            def pk(g):
                p = self.parent_of(g)
                return (lower_b(enc(self.dir_path(p))) if p >= 0 else b"", nk(g))
            return pk
        if kind == "type":
            def tk(g):
                if self.flags_of(g) & F_DIR:
                    return (b"", nk(g))
                nb = self.name_low(g)
                dot = nb.rfind(b".")
                return (nb[dot + 1:] if dot > 0 else b"", nk(g))
            return tk
        return nk


# ---------------------------------------------------------------------------
# query language
# ---------------------------------------------------------------------------

MOD_ALIASES = {
    "ext": "ext", "extensie": "ext", "extension": "ext",
    "size": "size", "grootte": "size",
    "dm": "dm", "datemodified": "dm", "gewijzigd": "dm", "datum": "dm",
    "folder": "folder", "map": "folder", "dir": "folder", "mappen": "folder", "folders": "folder",
    "file": "file", "bestand": "file", "files": "file", "bestanden": "file",
    "path": "path", "pad": "path", "nopath": "nopath",
    "parent": "parent", "infolder": "parent", "inmap": "parent",
    "in": "in", "under": "in", "onder": "in",
    "regex": "regex", "case": "case", "nocase": "nocase",
    "ww": "ww", "wholeword": "ww", "heelwoord": "ww", "noww": "noww", "nowholeword": "noww",
    "empty": "empty", "leeg": "empty",
    "audio": "cat:audio", "muziek": "cat:audio", "music": "cat:audio",
    "zip": "cat:compressed", "compressed": "cat:compressed", "archief": "cat:compressed",
    "doc": "cat:document", "document": "cat:document", "documenten": "cat:document",
    "exe": "cat:executable", "executable": "cat:executable", "programma": "cat:executable",
    "pic": "cat:picture", "picture": "cat:picture", "image": "cat:picture", "afbeelding": "cat:picture",
    "afbeeldingen": "cat:picture",
    "video": "cat:video", "videos": "cat:video",
}
TERM_MODS = {"folder", "file", "path", "nopath", "regex", "case", "nocase", "ww", "noww"}
_MOD_RE = re.compile(r"^([A-Za-z]+):")


class N_And:
    def __init__(self, items):
        self.items = items


class N_Or:
    def __init__(self, items):
        self.items = items


class N_Not:
    def __init__(self, item):
        self.item = item


class N_Term:
    def __init__(self, text, case, path, whole, regex, quoted):
        self.text, self.case, self.path, self.whole, self.regex, self.quoted = text, case, path, whole, regex, quoted


class N_Mod:
    def __init__(self, kind, value):
        self.kind, self.value = kind, value


class Parser:
    def __init__(self, s: str, case=False, path=False, whole=False):
        self.s = s
        self.i = 0
        self.depth = 0
        self.case, self.path, self.whole = case, path, whole

    def parse(self):
        items = self.parse_and()
        return N_And(items)

    def _ws(self):
        s = self.s
        while self.i < len(s) and s[self.i] in " \t\r\n":
            self.i += 1

    def parse_and(self):
        items = []
        s = self.s
        while True:
            self._ws()
            if self.i >= len(s):
                break
            c = s[self.i]
            if c == ">" and self.depth > 0:
                break
            if c == "|":
                self.i += 1
                continue
            it = self.parse_or()
            if it is not None:
                items.append(it)
        return items

    def parse_or(self):
        alts = [self.parse_unary()]
        s = self.s
        while True:
            j = self.i
            while j < len(s) and s[j] in " \t\r\n":
                j += 1
            if j < len(s) and s[j] == "|":
                self.i = j + 1
                self._ws()
                if self.i >= len(s) or (s[self.i] == ">" and self.depth > 0):
                    break
                alts.append(self.parse_unary())
                continue
            break
        alts = [a for a in alts if a is not None]
        if not alts:
            return None
        return alts[0] if len(alts) == 1 else N_Or(alts)

    def parse_unary(self):
        s = self.s
        if self.i >= len(s):
            return None
        c = s[self.i]
        if c == "!":
            self.i += 1
            inner = self.parse_unary()
            return N_Not(inner) if inner is not None else None
        if c == "<":
            self.i += 1
            self.depth += 1
            items = self.parse_and()
            if self.i < len(s) and s[self.i] == ">":
                self.i += 1
            self.depth -= 1
            if not items:
                return None
            return items[0] if len(items) == 1 else N_And(items)
        return self.parse_word()

    def parse_word(self):
        s = self.s
        start = self.i
        buf = []
        inq = False
        quoted = False
        while self.i < len(s):
            c = s[self.i]
            if c == '"':
                inq = not inq
                quoted = True
                self.i += 1
                continue
            if not inq and (c in " \t\r\n|" or (c == ">" and self.depth > 0)):
                break
            buf.append(c)
            self.i += 1
        raw = s[start:self.i]
        text = "".join(buf)
        if not raw:
            self.i += 1
            return None
        return self.make(raw, text, quoted, self.case, self.path, self.whole, False)

    def make(self, raw, text, quoted, case, path, whole, regex):
        m = _MOD_RE.match(raw)
        if m and m.group(1).lower() in MOD_ALIASES:
            kind = MOD_ALIASES[m.group(1).lower()]
            plen = len(m.group(1)) + 1
            vraw = raw[plen:]
            vtext = text[plen:] if text.startswith(raw[:plen]) else text.split(":", 1)[-1]
            vquoted = '"' in vraw
            if kind in TERM_MODS or kind.startswith("cat:"):
                if kind == "case":
                    case = True
                elif kind == "nocase":
                    case = False
                elif kind == "path":
                    path = True
                elif kind == "nopath":
                    path = False
                elif kind == "ww":
                    whole = True
                elif kind == "noww":
                    whole = False
                elif kind == "regex":
                    regex = True
                inner = None
                if vraw:
                    if regex:
                        inner = N_Term(vtext, case, path, whole, True, vquoted)
                    else:
                        inner = self.make(vraw, vtext, vquoted, case, path, whole, regex)
                if kind in ("folder", "file") or kind.startswith("cat:"):
                    mod = N_Mod(kind, None)
                    return N_And([mod, inner]) if inner is not None else mod
                return inner
            return N_Mod(kind, vtext)
        if regex:
            return N_Term(text, case, path, whole, True, quoted)
        if not text:
            return None
        return N_Term(text, case, path, whole, False, quoted)


# ---- value parsers ----------------------------------------------------------------

_UNITS = {"": 1, "b": 1, "byte": 1, "bytes": 1, "k": 1 << 10, "kb": 1 << 10, "kib": 1 << 10,
          "m": 1 << 20, "mb": 1 << 20, "mib": 1 << 20, "g": 1 << 30, "gb": 1 << 30, "gib": 1 << 30,
          "t": 1 << 40, "tb": 1 << 40, "tib": 1 << 40}
_SIZE_WORDS = {
    "empty": (0, 0), "leeg": (0, 0),
    "tiny": (1, 10 << 10), "piepklein": (1, 10 << 10),
    "small": ((10 << 10) + 1, 100 << 10), "klein": ((10 << 10) + 1, 100 << 10),
    "medium": ((100 << 10) + 1, 1 << 20), "middel": ((100 << 10) + 1, 1 << 20), "gemiddeld": ((100 << 10) + 1, 1 << 20),
    "large": ((1 << 20) + 1, 16 << 20), "groot": ((1 << 20) + 1, 16 << 20),
    "huge": ((16 << 20) + 1, 128 << 20), "enorm": ((16 << 20) + 1, 128 << 20),
    "gigantic": ((128 << 20) + 1, None), "gigantisch": ((128 << 20) + 1, None),
}
_NUM_RE = re.compile(r"^\s*(\d+(?:[.,]\d+)?)\s*([a-z]*)\s*$")


def parse_size_value(s: str) -> int:
    m = _NUM_RE.match(s.lower())
    if not m or m.group(2) not in _UNITS:
        raise QueryError(f"Ik begrijp de grootte “{s}” niet. Probeer bijvoorbeeld size:>10mb.")
    return int(float(m.group(1).replace(",", ".")) * _UNITS[m.group(2)])


def parse_size(v: str) -> tuple[int | None, int | None]:
    v = (v or "").strip().lower()
    if not v:
        raise QueryError("Geef een grootte op, bijvoorbeeld size:>10mb.")
    if v in _SIZE_WORDS:
        return _SIZE_WORDS[v]
    for sep in ("..", "-"):
        if sep in v and not v.startswith(sep):
            a, b = v.split(sep, 1)
            return parse_size_value(a), parse_size_value(b)
    for op in (">=", "<=", ">", "<", "="):
        if v.startswith(op):
            n = parse_size_value(v[len(op):])
            return {">=": (n, None), "<=": (None, n), ">": (n + 1, None), "<": (None, n - 1), "=": (n, n)}[op]
    n = parse_size_value(v)
    return n, n


def _day_start(t: time.struct_time, add_days: int = 0) -> float:
    """Local midnight of t's day (+ add_days), DST-safe (mktime normalises the day)."""
    return time.mktime((t.tm_year, t.tm_mon, t.tm_mday + add_days, 0, 0, 0, 0, 0, -1))


def _date_range_word(w: str, now: float) -> tuple[float, float] | None:
    lt = time.localtime(now)
    today = _day_start(lt)
    tomorrow = _day_start(lt, 1)
    day = 86400
    if w in ("today", "vandaag"):
        return today, tomorrow
    if w in ("yesterday", "gisteren"):
        return _day_start(lt, -1), today
    if w in ("thisweek", "dezeweek"):
        return _day_start(lt, -lt.tm_wday), tomorrow
    if w in ("lastweek", "vorigeweek"):
        this = _day_start(lt, -lt.tm_wday)
        return _day_start(lt, -lt.tm_wday - 7), this
    if w in ("thismonth", "dezemaand"):
        return time.mktime((lt.tm_year, lt.tm_mon, 1, 0, 0, 0, 0, 0, -1)), now + day
    if w in ("lastmonth", "vorigemaand"):
        y, mth = (lt.tm_year, lt.tm_mon - 1) if lt.tm_mon > 1 else (lt.tm_year - 1, 12)
        return (time.mktime((y, mth, 1, 0, 0, 0, 0, 0, -1)),
                time.mktime((lt.tm_year, lt.tm_mon, 1, 0, 0, 0, 0, 0, -1)))
    if w in ("thisyear", "ditjaar", "dit-jaar"):
        return time.mktime((lt.tm_year, 1, 1, 0, 0, 0, 0, 0, -1)), now + day
    if w in ("lastyear", "vorigjaar", "vorig-jaar"):
        return (time.mktime((lt.tm_year - 1, 1, 1, 0, 0, 0, 0, 0, -1)),
                time.mktime((lt.tm_year, 1, 1, 0, 0, 0, 0, 0, -1)))
    m = re.match(r"^(?:last|afgelopen)?(\d+)(days?|dagen|weeks?|weken|months?|maanden|years?|jaar|jaren|hours?|uur)$", w)
    if m:
        n = int(m.group(1))
        u = m.group(2)
        if u.startswith(("hour", "uur")):
            return now - n * 3600, now + day
        if u.startswith(("day", "dag")):
            return _day_start(lt, -(n - 1)), now + day
        if u.startswith(("week", "wek")):
            return now - n * 7 * day, now + day
        if u.startswith(("month", "maand")):
            return now - n * 30.44 * day, now + day
        return now - n * 365.25 * day, now + day
    return None


def _date_range_point(s: str) -> tuple[float, float]:
    s = s.strip()
    m = re.match(r"^(\d{4})$", s)
    if m:
        y = int(m.group(1))
        return time.mktime((y, 1, 1, 0, 0, 0, 0, 0, -1)), time.mktime((y + 1, 1, 1, 0, 0, 0, 0, 0, -1))
    m = re.match(r"^(\d{4})[-/](\d{1,2})$", s)
    if m:
        y, mo = int(m.group(1)), int(m.group(2))
        y2, mo2 = (y, mo + 1) if mo < 12 else (y + 1, 1)
        return time.mktime((y, mo, 1, 0, 0, 0, 0, 0, -1)), time.mktime((y2, mo2, 1, 0, 0, 0, 0, 0, -1))
    m = re.match(r"^(\d{4})[-/](\d{1,2})[-/](\d{1,2})$", s) or None
    if m:
        y, mo, d = int(m.group(1)), int(m.group(2)), int(m.group(3))
    else:
        m = re.match(r"^(\d{1,2})[-/.](\d{1,2})[-/.](\d{4})$", s)
        if not m:
            w = _date_range_word(s.lower(), time.time())
            if w:
                return w
            raise QueryError(f"Ik begrijp de datum “{s}” niet. Probeer bijvoorbeeld dm:vandaag of dm:2026-10-01..2026-10-25.")
        d, mo, y = int(m.group(1)), int(m.group(2)), int(m.group(3))
    if not (1 <= mo <= 12 and 1 <= d <= 31):
        raise QueryError(f"Ik begrijp de datum “{s}” niet.")
    try:
        start = time.mktime((y, mo, d, 0, 0, 0, 0, 0, -1))
        end = time.mktime((y, mo, d + 1, 0, 0, 0, 0, 0, -1))
    except (OverflowError, ValueError):
        raise QueryError(f"Ik begrijp de datum “{s}” niet.") from None
    return start, end


def parse_date(v: str) -> tuple[int | None, int | None]:
    """→ (lo_ms, hi_ms) inclusive/exclusive range on modification time."""
    v = (v or "").strip().lower()
    if not v:
        raise QueryError("Geef een datum op, bijvoorbeeld dm:vandaag.")
    now = time.time()
    w = _date_range_word(v, now)
    if w:
        return int(w[0] * 1000), int(w[1] * 1000)
    if ".." in v:
        a, b = v.split("..", 1)
        lo = _date_range_point(a)[0] if a else None
        hi = _date_range_point(b)[1] if b else None
        return (int(lo * 1000) if lo is not None else None), (int(hi * 1000) if hi is not None else None)
    for op in (">=", "<=", ">", "<", "="):
        if v.startswith(op):
            a, b = _date_range_point(v[len(op):])
            a, b = int(a * 1000), int(b * 1000)
            return {">=": (a, None), ">": (b, None), "<=": (None, b), "<": (None, a), "=": (a, b)}[op]
    a, b = _date_range_point(v)
    return int(a * 1000), int(b * 1000)


def wildcard_regex(pat: bytes) -> bytes:
    out = []
    for ch in re.split(rb"([*?])", pat):
        if ch == b"*":
            out.append(rb".*")
        elif ch == b"?":
            out.append(rb"(?:[^\n\x80-\xff]|[\xc0-\xff][\x80-\xbf]+)")
        elif ch:
            out.append(re.escape(ch))
    return b"".join(out)


# ---------------------------------------------------------------------------
# query evaluation
# ---------------------------------------------------------------------------

def ids_to_mask(ids, n: int) -> bytearray:
    m = bytearray(n)
    deque(map(m.__setitem__, ids, repeat(1)), maxlen=0)
    return m


def mask_to_ids(m: bytearray) -> list[int]:
    cnt = m.count(1)
    if cnt * 40 > len(m):
        return list(compress(range(len(m)), m))
    out = []
    ap = out.append
    find = m.find
    p = find(1)
    while p >= 0:
        ap(p)
        p = find(1, p + 1)
    return out


def m_and(a: bytearray, b: bytearray) -> bytearray:
    n = len(a)
    return bytearray((int.from_bytes(a, "little") & int.from_bytes(b, "little")).to_bytes(n, "little"))


def m_or(a: bytearray, b: bytearray) -> bytearray:
    n = len(a)
    return bytearray((int.from_bytes(a, "little") | int.from_bytes(b, "little")).to_bytes(n, "little"))


def m_andnot(a: bytearray, b: bytearray) -> bytearray:
    n = len(a)
    return bytearray((int.from_bytes(a, "little") & ~int.from_bytes(b, "little")).to_bytes(n, "little"))


class Ctx:
    """Per-query evaluation context."""

    def __init__(self, idx: Index, cancel=None, show_hidden=False):
        self.idx = idx
        self.N = idx.N
        self.cancel = cancel or (lambda: False)
        self.show_hidden = show_hidden
        self._cache = {}
        self._dirb = ({}, {})            # [case] {folder gid: its path + "/" as (lower-cased) bytes}

    def path_bytes(self, g: int, case: bool) -> bytes:
        """``enc(idx.path(g))``, lower-cased unless ``case``, with the folder part memoised for this
        query (a path: term tests up to ~0.5 M candidates that share far fewer folders). Exact:
        enc / lower_b work per character, and "/" never splits a UTF-8 sequence."""
        idx = self.idx
        par = idx.parent_of(g)
        if par < 0:
            p = enc(idx.seg_of(g).path)
            return p if case else lower_b(p)
        memo = self._dirb[case]
        d = memo.get(par)
        if d is None:
            d = enc(idx.dir_path(par))
            if not case:
                d = lower_b(d)
            if not d.endswith(b"/"):
                d += b"/"
            memo[par] = d
        nb = enc(idx.name(g))
        return d + (nb if case else lower_b(nb))

    def check(self):
        if self.cancel():
            raise Cancelled()

    def flagmask(self, table: bytes) -> bytearray:
        key = ("flags", table)
        m = self._cache.get(key)
        if m is None:
            m = self.idx.flags.translate(table)
            self._cache[key] = m
        return m

    def sample_ratio(self, fn) -> float:
        """Run fn(chunk) → count on a few spread-out chunks; → matches per entry."""
        chunks = self.idx.chunks
        if not chunks:
            return 0.0
        if len(chunks) <= 3:
            pick = chunks
        else:
            step = len(chunks) / 3
            pick = [chunks[int(i * step)] for i in range(3)]
        hits = sum(fn(c) for c in pick)
        total = sum(c.n for c in pick) or 1
        return hits / total

    def descendants_mask(self, roots, out: bytearray) -> None:
        """Mark every base descendant of the given base dirs (BFS level ranges).

        Children of a contiguous range of folders are one contiguous range (the parent
        array is non-decreasing inside a segment), so each level is a single slice
        assignment. Folders inside an already marked subtree are skipped (gids are in
        BFS order, so ancestors come first).
        """
        idx = self.idx
        par = idx.parent
        bl, br = bisect.bisect_left, bisect.bisect_right
        marked = bytearray(idx.N)
        for d in sorted(roots):
            if d >= idx.N or marked[d]:
                continue
            seg = idx.seg_of(d)
            lo, hi = d, d + 1
            send = seg.end
            while True:
                nlo = bl(par, lo, seg.start + 1, send)
                nhi = br(par, hi - 1, nlo, send)
                if nlo >= nhi:
                    break
                ones = b"\x01" * (nhi - nlo)
                out[nlo:nhi] = ones
                marked[nlo:nhi] = ones
                lo, hi = nlo, nhi


def chunk_filter(cx: Ctx, lst, pred, case: bool, lit: bytes | None = None) -> list[int]:
    """[g for g in lst if pred(name_bytes(g))] without a bisect per candidate (base + overlay).

    With ``lit`` the predicate is the inlined test ``lit in name`` (the common case).
    """
    idx = cx.idx
    N = idx.N
    out = []
    ap = out.append
    chunks, cstarts = idx.chunks, idx.cstarts
    c_lo = c_hi = -1
    blob = offs = None
    base = b0 = 0
    k = 0
    for g in lst:
        if g >= N:
            nb = idx.ov.orig[g - N] if case else idx.ov.low[g - N]
            if (lit in nb) if lit is not None else pred(nb):
                ap(g)
            continue
        if not (c_lo <= g < c_hi):
            c = chunks[bisect.bisect_right(cstarts, g) - 1]
            c_lo, c_hi, base = c.start, c.start + c.n, c.start
            blob, b0 = (c.om, c.ob - 1) if case else (c.lm, c.lb - 1)    # heap blob or mmap window
            offs = c.offs
            k += 1
            if not k & 0x3F:
                cx.check()
        i = g - base
        nb = blob[b0 + offs[i] + 1:b0 + offs[i + 1]]
        if lit is not None:
            if lit in nb:
                ap(g)
        elif pred(nb):
            ap(g)
    return out


class Atom:
    """A query predicate.

    est()        approximate number of base matches (cheap)
    ids()        list of matching base gids
    mask()       0/1 bytearray over base gids
    test(g)      one entry (base or overlay)
    filter(lst)  keep the matching gids of a candidate list
    full_cost()  µs to compute ids()/mask();  filter_cost(n)  µs to filter n candidates
    """
    unit = 1.0   # µs per candidate in filter()

    def est(self, cx: Ctx) -> int:
        return cx.N

    def ids(self, cx: Ctx) -> list[int]:
        return mask_to_ids(self.mask(cx))

    def mask(self, cx: Ctx) -> bytearray:
        return ids_to_mask(self.ids(cx), cx.N)

    def test(self, cx: Ctx, g: int) -> bool:
        raise NotImplementedError

    def filter(self, cx: Ctx, lst) -> list[int]:
        t = self.test
        return [g for g in lst if t(cx, g)]

    def full_cost(self, cx: Ctx) -> float:
        return cx.N * 0.1

    def filter_cost(self, cx: Ctx, n: int) -> float:
        return n * self.unit

    def highlight(self, out: list) -> None:
        pass


class AllAtom(Atom):
    unit = 0.0

    def est(self, cx):
        return cx.N

    def mask(self, cx):
        return bytearray(b"\x01") * cx.N

    def filter(self, cx, lst):
        return list(lst)

    def full_cost(self, cx):
        return cx.N * 0.0005

    def test(self, cx, g):
        return True


class FlagAtom(Atom):
    """Flag based predicate (folder:, file:, visibility) via bytes.translate."""
    unit = 0.03

    def __init__(self, table: bytes):
        self.table = table

    def est(self, cx):
        return cx.flagmask(self.table).count(1)

    def mask(self, cx):
        return bytearray(cx.flagmask(self.table))

    def filter(self, cx, lst):
        N = cx.N
        if not isinstance(lst, list):
            lst = list(lst)
        if not lst:
            return []
        if len(lst) <= SMALL_FILTER and ("flags", self.table) not in cx._cache:
            t, fl, ofl = self.table, cx.idx.flags, cx.idx.ov.flags      # no N-byte translate
            return [g for g in lst if t[fl[g] if g < N else ofl[g - N]]]
        m = cx.flagmask(self.table)
        if max(lst) < N:
            return list(compress(lst, map(m.__getitem__, lst)))
        t = self.table
        idx = cx.idx
        return [g for g in lst if (m[g] if g < N else t[idx.ov.flags[g - N]])]

    def full_cost(self, cx):
        return cx.N * 0.0005

    def test(self, cx, g):
        return bool(self.table[cx.idx.flags_of(g)])


class CatAtom(Atom):
    unit = 0.03

    def __init__(self, code: int):
        self.code = code
        self.table = CAT_TABLES[code]

    def _m(self, cx):
        key = ("cat", self.code)
        m = cx._cache.get(key)
        if m is None:
            m = cx._cache[key] = cx.idx.cat.translate(self.table)
        return m

    def est(self, cx):
        return cx.idx.cat.count(self.code)

    def mask(self, cx):
        return bytearray(self._m(cx))

    def filter(self, cx, lst):
        N = cx.N
        idx = cx.idx
        code = self.code
        if not isinstance(lst, list):
            lst = list(lst)
        if len(lst) <= SMALL_FILTER and ("cat", code) not in cx._cache:
            cat, ocat = idx.cat, idx.ov.cat                          # no N-byte translate
            return [g for g in lst if (cat[g] if g < N else ocat[g - N]) == code]
        m = self._m(cx)
        return [g for g in lst if (m[g] if g < N else idx.ov.cat[g - N] == code)]

    def full_cost(self, cx):
        return cx.N * 0.0005

    def test(self, cx, g):
        return cx.idx.cat_of(g) == self.code


def _bisect_clean(o, x, key, dirty, right: bool) -> int:
    """bisect_left / bisect_right of x in order o by key, skipping the dirty entries: their value
    changed after o was sorted, so they sit at their old place and could send a plain bisect the
    wrong way (and lose a whole range). They are merged in separately anyway."""
    if not dirty:
        return (bisect.bisect_right if right else bisect.bisect_left)(o, x, key=key)
    lo, hi = 0, len(o)
    while lo < hi:
        mid = (lo + hi) // 2
        m = mid
        while m < hi and o[m] in dirty:
            m += 1
        if m == hi:                      # only dirty entries in [mid, hi)
            hi = mid
            continue
        v = key(o[m])
        if v < x or (right and v == x):
            lo = m + 1
        else:
            hi = mid
    return lo


class RangeAtom(Atom):
    """size:/dm: — lo <= value <= hi (None = open). Uses the cached sorted order + bisect."""
    unit = 0.12

    def __init__(self, field: str, lo, hi):
        self.field, self.lo, self.hi = field, lo, hi
        self._b = None

    def _arr(self, cx):
        return cx.idx.size if self.field == "size" else cx.idx.mtime

    def _bounds(self, cx):
        if self._b is None:
            kind = "size" if self.field == "size" else "modified"
            o = cx.idx.get_order(kind)
            key = self._arr(cx).__getitem__
            dirty = cx.idx.odirty.get(kind) or ()
            k1 = _bisect_clean(o, self.lo, key, dirty, False) if self.lo is not None else 0
            k2 = _bisect_clean(o, self.hi, key, dirty, True) if self.hi is not None else len(o)
            self._b = (o, k1, max(k1, k2), dirty)
        return self._b

    def _ok(self, v):
        return (self.lo is None or v >= self.lo) and (self.hi is None or v <= self.hi)

    def est(self, cx):
        _o, k1, k2, dirty = self._bounds(cx)
        return k2 - k1 + len(dirty)

    def ids(self, cx):
        o, k1, k2, dirty = self._bounds(cx)
        out = list(o[k1:k2])
        if dirty:
            arr = self._arr(cx)
            ok = self._ok
            ds = set(dirty)
            out = [g for g in out if g not in ds or ok(arr[g])]
            have = set(out) if len(ds) < len(out) else None
            for g in ds:
                if g < cx.N and ok(arr[g]) and (have is None or g not in have):
                    out.append(g)
            out = sorted(set(out))
        return out

    def mask(self, cx):
        o, k1, k2, dirty = self._bounds(cx)
        if k2 - k1 > cx.N // 3 and not dirty:
            arr = self._arr(cx)
            lo, hi = self.lo, self.hi
            if lo is not None and hi is not None:
                return bytearray(map(operator.and_, map(lo.__le__, arr), map(hi.__ge__, arr)))
            if lo is not None:
                return bytearray(map(lo.__le__, arr))
            return bytearray(map(hi.__ge__, arr))
        return ids_to_mask(self.ids(cx), cx.N)

    def full_cost(self, cx):
        _o, k1, k2, dirty = self._bounds(cx)
        return (k2 - k1) * 0.06 + len(dirty) * 0.2

    def filter(self, cx, lst):
        idx = cx.idx
        N = idx.N
        arr = self._arr(cx)
        ov = idx.ov.size if self.field == "size" else idx.ov.mtime
        lo, hi = self.lo, self.hi
        if lo is None:
            lo = -(1 << 62)
        if hi is None:
            hi = 1 << 62
        return [g for g in lst if lo <= (arr[g] if g < N else ov[g - N]) <= hi]

    def test(self, cx, g):
        v = cx.idx.size_of(g) if self.field == "size" else cx.idx.mtime_of(g)
        return self._ok(v)


class ParentAtom(Atom):
    unit = 0.25

    def __init__(self, path: str, recursive: bool):
        self.path = norm_path(path)
        self.recursive = recursive
        self._g = "unset"
        self._r = None

    def _resolve(self, cx):
        if self._g == "unset":
            g = cx.idx.lookup(self.path)
            if g is None and os.path.exists(self.path):
                g = cx.idx.lookup(os.path.realpath(self.path))
            self._g = g
        return self._g

    def _ranges(self, cx):
        if self._r is not None:
            return self._r
        g = self._resolve(cx)
        out = []
        if g is not None and g < cx.N:
            idx = cx.idx
            seg = idx.seg_of(g)
            par = idx.parent
            lo, hi = g, g + 1
            while True:
                nlo = bisect.bisect_left(par, lo, seg.start + 1, seg.end)
                nhi = bisect.bisect_right(par, hi - 1, nlo, seg.end)
                if nlo >= nhi:
                    break
                out.append((nlo, nhi))
                if not self.recursive:
                    break
                lo, hi = nlo, nhi
        self._r = out
        return out

    def est(self, cx):
        return sum(b - a for a, b in self._ranges(cx))

    def ids(self, cx):
        out = []
        for a, b in self._ranges(cx):
            out.extend(range(a, b))
        return out

    def mask(self, cx):
        m = bytearray(cx.N)
        for a, b in self._ranges(cx):
            m[a:b] = b"\x01" * (b - a)
        return m

    def full_cost(self, cx):
        return self.est(cx) * 0.02 + 5

    def filter(self, cx, lst):
        if not self.recursive:
            target = self._resolve(cx)
            if target is None:
                return []
            pof = cx.idx.parent_of
            return [g for g in lst if pof(g) == target]
        rs = self._ranges(cx)
        N = cx.N
        out = []
        t = self.test
        for g in lst:
            if g < N:
                for a, b in rs:
                    if a <= g < b:
                        out.append(g)
                        break
            elif t(cx, g):
                out.append(g)
        return out

    def test(self, cx, g):
        target = self._resolve(cx)
        if target is None:
            return False
        p = cx.idx.parent_of(g)
        if not self.recursive:
            return p == target
        while p >= 0:
            if p == target:
                return True
            p = cx.idx.parent_of(p)
        return False


class EmptyAtom(Atom):
    unit = 1.5

    def est(self, cx):
        return cx.flagmask(T_ADIR).count(1)

    def ids(self, cx):
        idx = cx.idx
        par = idx.parent
        out = []
        for seg in idx.segs:
            a, b = seg.start, seg.end
            fl = idx.flags
            for d in compress(range(a, b), fl[a:b].translate(T_ADIR)):
                lo = bisect.bisect_left(par, d, max(a + 1, d + 1), b)
                if lo < b and par[lo] == d:
                    # has base children; empty only if all of them are deleted
                    hi = bisect.bisect_right(par, d, lo, b)
                    if any(not fl[c] & F_DEL for c in range(lo, hi)):
                        continue
                if idx.ov.kids.get(d) and idx.kids(d):
                    continue
                out.append(d)
        return out

    def full_cost(self, cx):
        return self.est(cx) * 1.0

    def test(self, cx, g):
        idx = cx.idx
        return bool(idx.flags_of(g) & F_DIR) and not idx.kids(g)


class NameAtom(Atom):
    """Literal (optionally anchored) name match, optional whole-word, case, path mode."""
    unit = 0.35

    def __init__(self, lit: bytes, anchor: str, case: bool, path: bool, whole: bool):
        self.case = case
        self.path = path
        self.whole = whole
        self.lit = lit if case else lower_b(lit)
        self.anchor = anchor   # '' | 'start' | 'end' | 'exact'
        lit2 = self.lit
        self.needle = {"": lit2, "start": b"\n" + lit2, "end": lit2 + b"\n",
                       "exact": b"\n" + lit2 + b"\n"}[anchor]
        self.adj = 1 if anchor in ("start", "exact") else 0
        self.wre = None
        if whole:
            w = rb"[0-9A-Za-z_\x80-\xff]"
            self.wre = re.compile(rb"(?<!" + w + rb")" + re.escape(lit2) + rb"(?!" + w + rb")",
                                  0 if case else re.IGNORECASE)
            self.unit = 1.0
        self.slash = b"/" in lit2
        self.path_simple = bool(path and anchor in ("", "end") and not self.slash)
        if path:
            self.unit = 3.0
        self._est = None
        self._ids = None
        self._pf = None

    def _blob(self, c: Chunk):
        return c.orig if self.case else c.low

    def _pred(self):
        a, lit, w = self.anchor, self.lit, self.wre
        if a == "":
            base = lambda nb: lit in nb  # noqa: E731
        elif a == "start":
            base = lambda nb: nb.startswith(lit)  # noqa: E731
        elif a == "end":
            base = lambda nb: nb.endswith(lit)  # noqa: E731
        else:
            base = lit.__eq__
        if w is None:
            return base
        ws = w.search
        return lambda nb: base(nb) and ws(nb) is not None

    def name_est(self, cx):
        if self._est is None:
            needle = self.needle
            self._est = int(cx.sample_ratio(lambda c: self._blob(c).count(needle)) * cx.N) + 1
        return self._est

    def est(self, cx):
        return cx.N if self.path else self.name_est(cx)

    def full_cost(self, cx):
        c = cx.N * 0.005 + self.name_est(cx) * (0.3 if self.wre is None else 1.0)
        if self.path:
            c += cx.N * 0.01
        return c

    def name_ids(self, cx):
        if self._ids is not None:
            return self._ids
        needle, adj = self.needle, self.adj
        verify = self.wre.search if self.wre is not None else None
        out = []
        ap = out.append
        NL = b"\n"
        idx = cx.idx
        skip_hidden = not cx.show_hidden
        # a selective term on a mapped base searches the mmap itself (a bisect on offs per hit)
        # instead of copying all 101 chunks; mmap has no count(), so a broad term copies chunks
        direct = self.name_est(cx) <= DIRECT_MAX
        bl = bisect.bisect_left
        for c in idx.chunks:
            cx.check()
            mm, b0 = (c.om, c.ob) if self.case else (c.lm, c.lb)
            if direct and type(mm) is not bytes:
                runs = chunk_vis_runs(idx, c) if skip_hidden else False
                spans = ((0, c.n),) if runs is False else runs
                offs = c.offs
                find, rfind = mm.find, mm.rfind
                cbase = c.start
                for a, b in spans:
                    if a >= b:
                        continue
                    stop = b0 + offs[b]
                    p = find(needle, b0 + offs[a] - adj, stop)
                    while p >= 0:
                        q = p + adj
                        ls = q if adj else rfind(NL, b0, q) + 1      # first byte of the name
                        end = find(NL, q)
                        if verify is None or verify(mm, ls, end) is not None:
                            ap(cbase + bl(offs, ls - b0, a, b))
                        p = find(needle, end, stop)
                continue
            blob = self._blob(c)
            find, cnt = blob.find, blob.count
            runs = chunk_vis_runs(idx, c) if skip_hidden else False
            if runs is False:
                spans = ((0, c.n),)
            else:
                spans = runs
            offs = c.offs
            for a, b in spans:
                if a >= b:
                    continue
                # newlines in [last, p+adj) advance the line index (the blob starts with "\n")
                last = offs[a] - 1
                stop = offs[b]
                g = c.start + a - 1
                p = find(needle, offs[a] - adj, stop)
                while p >= 0:
                    g += cnt(NL, last, p + adj)
                    end = find(NL, p + adj)
                    if verify is None or verify(blob, blob.rfind(NL, 0, p + adj) + 1, end) is not None:
                        ap(g)
                    last = end
                    p = find(needle, end, stop)
        self._ids = out
        return out

    def name_mask(self, cx):
        lit = self.lit
        if (len(lit) == 1 and self.anchor == "" and self.wre is None and lit != b"\n"
                and self.name_est(cx) > cx.N // 20):
            # dense single byte: delete every other byte, then one split per chunk
            dele = bytes(b for b in range(256) if b not in (lit[0], 10))
            m = bytearray(cx.N)
            for c in cx.idx.chunks:
                cx.check()
                t = self._blob(c).translate(None, dele)
                piece = bytearray(map(bool, t[1:-1].split(b"\n"))) if c.n else bytearray()
                m[c.start:c.start + c.n] = piece
            return m
        return ids_to_mask(self.name_ids(cx), cx.N)

    def ids(self, cx):
        if not self.path:
            if self.name_est(cx) > LIST_MAX and len(self.lit) == 1:
                return mask_to_ids(self.name_mask(cx))
            return self.name_ids(cx)
        return mask_to_ids(self.mask(cx))

    def mask(self, cx):
        if not self.path:
            return self.name_mask(cx)
        # path mode: match in the name, or in any ancestor folder name
        piece = self._piece()
        if piece is None:
            cand = bytearray(b"\x01") * cx.N
        else:
            sub = NameAtom(piece, self.anchor if self.path_simple else "", self.case, False,
                           self.whole if self.path_simple else False)
            cand = sub.name_mask(cx)
            if self.path_simple and self.anchor == "end":
                return cand          # the path ends with it ⇔ the name ends with it
            dirs = list(compress(range(cx.N), m_and(cand, cx.flagmask(T_DIR))))
            if dirs:
                cx.descendants_mask(dirs, cand)
        if self.path_simple:
            return cand
        out = bytearray(cx.N)
        t = self.test
        for g in mask_to_ids(cand):
            if t(cx, g):
                out[g] = 1
        return out

    def _piece(self):
        if self.path_simple:
            return self.lit
        parts = [p for p in self.lit.split(b"/") if p]
        if not parts:
            return None
        return max(parts, key=len)

    def filter(self, cx, lst):
        if self.path:
            t = self.test
            return [g for g in lst if t(cx, g)]
        if self.anchor == "" and self.wre is None:
            return chunk_filter(cx, lst, None, self.case, self.lit)
        return chunk_filter(cx, lst, self._pred(), self.case)

    def test(self, cx, g):
        idx = cx.idx
        pf = self._pf
        if pf is None:
            pf = self._pf = self._pred()
        if not self.path:
            return pf(idx.name_orig(g) if self.case else idx.name_low(g))
        return pf(cx.path_bytes(g, self.case))

    def highlight(self, out):
        if b"/" not in self.lit:
            out.append(dec(self.lit))


class WildAtom(Atom):
    """Wildcard pattern matching the whole name (or whole path in path mode)."""
    unit = 1.0

    def __init__(self, pat: bytes, case: bool, path: bool):
        self.case = case
        self.path = path
        p = pat if case else lower_b(pat)
        self.rx = re.compile(wildcard_regex(p), re.DOTALL)
        pieces = re.split(rb"[*?]", p)
        best = None
        cands = []
        for k, piece in enumerate(pieces):
            if not piece:
                continue
            anchor = ""
            if k == 0:
                anchor = "start"
            if k == len(pieces) - 1 and len(pieces) > 1:
                anchor = "end" if anchor == "" else "exact"
            if path:
                sub = [x for x in piece.split(b"/") if x]
                if not sub:
                    continue
                piece = max(sub, key=len)
                anchor = ""
            cands.append((len(piece) + (2 if anchor else 0), piece, anchor))
        if cands:
            best = max(cands)
        self.pre = NameAtom(best[1], best[2], case, False, False) if best else None
        self.lits = [dec(x) for x in pieces if x and b"/" not in x]
        if path:
            self.unit = 4.0

    def est(self, cx):
        if self.pre is None:
            return cx.N
        return self.pre.name_est(cx)

    def full_cost(self, cx):
        return cx.N * 0.005 + self.est(cx) * 1.2 * (3 if self.path else 1)

    def _cands(self, cx):
        idx = cx.idx
        if self.pre is None:
            return range(cx.N)
        if self.path:
            m = self.pre.name_mask(cx)
            dirs = list(compress(range(cx.N), m_and(m, cx.flagmask(T_DIR))))
            if dirs:
                cx.descendants_mask(dirs, m)
            return mask_to_ids(m)
        return self.pre.name_ids(cx)

    def ids(self, cx):
        return self.filter(cx, self._cands(cx))

    def filter(self, cx, lst):
        fm = self.rx.fullmatch
        if self.path:
            idx = cx.idx
            out = []
            for g in lst:
                p = enc(idx.path(g))
                if fm(p if self.case else lower_b(p)):
                    out.append(g)
            return out
        return chunk_filter(cx, lst, lambda nb: fm(nb) is not None, self.case)

    def test(self, cx, g):
        idx = cx.idx
        if self.path:
            p = enc(idx.path(g))
            return self.rx.fullmatch(p if self.case else lower_b(p)) is not None
        return self.rx.fullmatch(idx.name_orig(g) if self.case else idx.name_low(g)) is not None

    def highlight(self, out):
        out.extend(x for x in self.lits if x)


def regex_literal(pattern: str) -> str | None:
    """Longest literal run that every match must contain (None if unknown)."""
    try:
        from re import _parser as sre_parse  # Python 3.11+
    except ImportError:  # pragma: no cover
        try:
            import sre_parse  # type: ignore
        except ImportError:
            return None
    try:
        parsed = sre_parse.parse(pattern)
    except Exception:
        return None
    best, run = "", []
    LIT = sre_parse.LITERAL if hasattr(sre_parse, "LITERAL") else None
    for op, av in parsed:
        if LIT is not None and op == LIT:
            run.append(chr(av))
            continue
        if str(op) in ("AT", "AT_BEGINNING", "AT_END"):
            if run:
                s = "".join(run)
                best = s if len(s) > len(best) else best
                run = []
            continue
        s = "".join(run)
        if len(s) > len(best):
            best = s
        run = []
    s = "".join(run)
    if len(s) > len(best):
        best = s
    return best or None


# Nested quantifiers – "(a+)+", "(\w*\s?)*", "(.*x){20}" – make Python's re backtrack for minutes on a long
# name, inside C where a search cannot be cancelled. Such patterns are refused with a friendly message.
_NESTED_QUANT = re.compile(r"\((?:[^()\\]|\\.)*(?:[+*]|\{\d*,?\d*\})(?:[^()\\]|\\.)*\)(?:[+*]|\{\d*,?\d*\})")
_BACKREF = re.compile(r"\\[1-9]|\(\?P=")
REGEX_MAX = 400


def regex_too_complex(pattern: str) -> bool:
    return len(pattern) > REGEX_MAX or bool(_NESTED_QUANT.search(pattern)) or bool(_BACKREF.search(pattern))


class RegexAtom(Atom):
    unit = 1.2

    def __init__(self, pattern: str, case: bool, path: bool):
        self.case = case
        self.path = path
        self.pattern = pattern
        if regex_too_complex(pattern):
            raise QueryError("Deze reguliere expressie is te ingewikkeld om snel te zoeken. Maak hem eenvoudiger "
                             "(bijvoorbeeld zonder herhaling binnen een herhaling).", "regex_too_complex")
        flags = 0 if case else re.IGNORECASE
        try:
            if pattern.isascii():
                self.brx = re.compile(enc(pattern), flags | re.MULTILINE)
                self.srx = None
            else:
                self.brx = None
                self.srx = re.compile(pattern, flags | re.MULTILINE)
        except re.error as e:
            raise QueryError(f"Deze reguliere expressie klopt niet: {e.msg}.", "bad_regex") from None
        lit = regex_literal(pattern) if not path else None
        self.pre = None
        if lit and len(lit) >= 2 and "\n" not in lit:
            self.pre = NameAtom(enc(lit), "", case, False, False)
        if path:
            self.unit = 4.0
        self._est = None

    def _search_line(self):
        if self.brx is not None:
            s = self.brx.search
            return lambda nb: s(nb) is not None
        s = self.srx.search
        return lambda nb: s(stored_to_str(nb)) is not None

    def _sample(self, c: Chunk) -> int:
        if self.brx is None:
            return sum(1 for _ in islice(self.srx.finditer(dec(c.orig)), 20000))
        return sum(1 for _ in islice(self.brx.finditer(c.orig), 20000))

    def est(self, cx):
        if self.path:
            return cx.N
        if self._est is None:
            if self.pre is not None:
                self._est = self.pre.name_est(cx)
            else:
                self._est = int(cx.sample_ratio(self._sample) * cx.N) + 1
        return self._est

    def full_cost(self, cx):
        if self.path:
            return cx.N * 4.0
        if self.pre is not None:
            return cx.N * 0.005 + self.est(cx) * 1.5
        return cx.N * 0.3

    def ids(self, cx):
        idx = cx.idx
        if self.path:
            vis = cx.flagmask(T_ALIVE)
            return self.filter(cx, compress(range(cx.N), vis))
        if self.pre is not None:
            return self.filter(cx, self.pre.name_ids(cx))
        out = []
        ap = out.append
        br = bisect.bisect_right
        for c in idx.chunks:
            cx.check()
            offs = c.offs
            base = c.start
            if self.brx is not None:
                blob = c.orig
                search = self.brx.search
                last = -1
                n = c.n
                for m in self.brx.finditer(blob):
                    i = br(offs, m.start()) - 1
                    if i < 0:
                        i = 0
                    elif i >= n:
                        continue
                    if i == last:
                        continue
                    end = offs[i + 1] - 1
                    if m.end() > end and search(blob, offs[i], end) is None:
                        continue      # the match ran over a line break
                    last = i
                    ap(base + i)
            else:
                lines = dec(c.om[c.ob + 1:c.ob + c.olen - 1]).split("\n") if c.n else []
                search = self.srx.search
                for i in compress(range(len(lines)), map(bool, map(search, lines))):
                    ap(base + i)
        return out

    def filter(self, cx, lst):
        if self.path:
            idx = cx.idx
            out = []
            k = 0
            if self.brx is not None:
                s = self.brx.search
                for g in lst:
                    k += 1
                    if not k & 0x3FFF:
                        cx.check()
                    if s(enc(idx.path(g))):
                        out.append(g)
            else:
                s = self.srx.search
                for g in lst:
                    k += 1
                    if not k & 0x3FFF:
                        cx.check()
                    if s(idx.path(g)):
                        out.append(g)
            return out
        return chunk_filter(cx, lst, self._search_line(), True)

    def test(self, cx, g):
        idx = cx.idx
        if self.path:
            s = idx.path(g)
        else:
            s = stored_to_str(idx.name_orig(g))
        if self.brx is not None:
            return self.brx.search(enc(s)) is not None
        return self.srx.search(s) is not None

    def highlight(self, out):
        pass


class ExtAtom(Atom):
    unit = 0.35

    def __init__(self, exts: list[str]):
        clean = []
        for e in exts:
            e = e.strip().lower().lstrip("*").lstrip(".")
            if e:
                clean.append(e)
        if not clean:
            raise QueryError("Geef een extensie op, bijvoorbeeld ext:jpg of ext:mp3;flac.")
        self.exts = clean
        self.suffixes = tuple(lower_b(enc("." + e)) for e in clean)
        self.needles = [s + b"\n" for s in self.suffixes]
        self._est = None

    def est(self, cx):
        if self._est is None:
            needles = self.needles

            def hits(c):
                low = c.low                  # one copy per chunk for a mapped base
                return sum(low.count(nd) for nd in needles)
            self._est = int(cx.sample_ratio(hits) * cx.N) + 1
        return self._est

    def full_cost(self, cx):
        return cx.N * 0.005 * len(self.needles) + self.est(cx) * 0.3

    def ids(self, cx):
        out = []
        for sfx in self.suffixes:
            part = NameAtom(sfx, "end", False, False, False).name_ids(cx)
            out = part if not out else out + part
        if len(self.suffixes) > 1:
            out = sorted(set(out))
        return out

    def filter(self, cx, lst):
        sfx = self.suffixes
        return chunk_filter(cx, lst, lambda nb: nb.endswith(sfx), False)

    def test(self, cx, g):
        return cx.idx.name_low(g).endswith(self.suffixes)


class OrNode(Atom):
    def __init__(self, items):
        self.items = items
        self.unit = sum(i.unit for i in items)

    def est(self, cx):
        return min(cx.N, sum(i.est(cx) for i in self.items))

    def full_cost(self, cx):
        return sum(i.full_cost(cx) for i in self.items)

    def ids(self, cx):
        s = set()
        for it in self.items:
            s.update(it.ids(cx))
        return sorted(s)

    def mask(self, cx):
        m = None
        for it in self.items:
            mm = it.mask(cx)
            m = mm if m is None else m_or(m, mm)
        return m if m is not None else bytearray(cx.N)

    def filter(self, cx, lst):
        lst = list(lst)
        keep = set()
        rest = lst
        for it in self.items:
            if not rest:
                break
            hit = it.filter(cx, rest)
            keep.update(hit)
            if len(self.items) > 1:
                hs = set(hit)
                rest = [g for g in rest if g not in hs]
        return [g for g in lst if g in keep]

    def test(self, cx, g):
        return any(it.test(cx, g) for it in self.items)

    def highlight(self, out):
        for it in self.items:
            it.highlight(out)


class NotAtom(Atom):
    def __init__(self, item):
        self.item = item
        self.unit = item.unit

    def est(self, cx):
        return cx.N

    def mask(self, cx):
        return m_andnot(bytearray(b"\x01") * cx.N, self.item.mask(cx))

    def full_cost(self, cx):
        return self.item.full_cost(cx) + cx.N * 0.003

    def filter(self, cx, lst):
        lst = list(lst)
        hit = set(self.item.filter(cx, lst))
        return [g for g in lst if g not in hit]

    def test(self, cx, g):
        return not self.item.test(cx, g)


class AndNode(Atom):
    def __init__(self, pos, neg):
        self.pos = pos
        self.neg = neg
        self.unit = sum(i.unit for i in pos + neg)

    def est(self, cx):
        if not self.pos:
            return cx.N
        return min(i.est(cx) for i in self.pos)

    def full_cost(self, cx):
        return sum(i.full_cost(cx) for i in self.pos + self.neg)

    def test(self, cx, g):
        for it in self.pos:
            if not it.test(cx, g):
                return False
        for it in self.neg:
            if it.test(cx, g):
                return False
        return True

    def filter(self, cx, lst):
        cur = list(lst)
        for it in sorted(self.pos, key=lambda a: a.unit):
            if not cur:
                return cur
            cur = it.filter(cx, cur)
        for it in self.neg:
            if not cur:
                return cur
            hit = set(it.filter(cx, cur))
            cur = [g for g in cur if g not in hit]
        return cur

    def evaluate(self, cx) -> tuple[str, object]:
        """→ ('L', list) or ('M', mask). Rarest term first, the rest filter the candidates."""
        ests = sorted(((it.est(cx), k, it) for k, it in enumerate(self.pos)), key=lambda t: (t[0], t[1]))
        if not ests:
            cur = ("M", bytearray(b"\x01") * cx.N)
            rest = []
        else:
            e0, _k, first = ests[0]
            cur = ("L", first.ids(cx)) if e0 <= LIST_MAX else ("M", first.mask(cx))
            rest = [t[2] for t in ests[1:]]
        # cheapest filters first
        rest.sort(key=lambda a: a.unit)
        for it in rest:
            cx.check()
            cur = self._intersect(cx, cur, it)
        for it in self.neg:
            cx.check()
            cur = self._subtract(cx, cur, it)
        return cur

    @staticmethod
    def _intersect(cx, cur, it):
        kind, data = cur
        if kind == "L":
            n = len(data)
            if not n:
                return cur
            if it.filter_cost(cx, n) <= it.full_cost(cx) + n * 0.05:
                return ("L", it.filter(cx, data))
            m = it.mask(cx)
            return ("L", list(compress(data, map(m.__getitem__, data))))
        e = it.est(cx)
        if e <= LIST_MAX:
            ids = it.ids(cx)
            return ("L", list(compress(ids, map(data.__getitem__, ids))))
        return ("M", m_and(data, it.mask(cx)))

    @staticmethod
    def _subtract(cx, cur, it):
        kind, data = cur
        if kind == "L":
            n = len(data)
            if it.filter_cost(cx, n) <= it.full_cost(cx) + n * 0.05:
                hit = set(it.filter(cx, data))
                return ("L", [g for g in data if g not in hit])
            m = it.mask(cx)
            return ("L", [g for g in data if not m[g]])
        return ("M", m_andnot(data, it.mask(cx)))

    def ids(self, cx):
        k, d = self.evaluate(cx)
        return d if k == "L" else mask_to_ids(d)

    def mask(self, cx):
        k, d = self.evaluate(cx)
        return d if k == "M" else ids_to_mask(d, cx.N)

    def highlight(self, out):
        for it in self.pos:
            it.highlight(out)


def build_atom(node, opts) -> Atom | None:
    if node is None:
        return None
    if isinstance(node, N_And):
        pos, neg = [], []
        for it in node.items:
            if isinstance(it, N_Not):
                a = build_atom(it.item, opts)
                if a is not None:
                    neg.append(a)
            else:
                a = build_atom(it, opts)
                if a is not None:
                    pos.append(a)
        if not pos and not neg:
            return None
        if len(pos) == 1 and not neg:
            return pos[0]
        return AndNode(pos, neg)
    if isinstance(node, N_Or):
        items = [build_atom(i, opts) for i in node.items]
        items = [i for i in items if i is not None]
        if not items:
            return None
        if any(isinstance(i, AllAtom) for i in items):
            return AllAtom()
        return items[0] if len(items) == 1 else OrNode(items)
    if isinstance(node, N_Not):
        a = build_atom(node.item, opts)
        return NotAtom(a) if a is not None else None
    if isinstance(node, N_Mod):
        return build_mod(node)
    if isinstance(node, N_Term):
        return build_term(node)
    return None


def build_mod(node: N_Mod) -> Atom:
    k, v = node.kind, node.value
    if k == "folder":
        return FlagAtom(T_DIR)
    if k == "file":
        return FlagAtom(T_FILE)
    if k.startswith("cat:"):
        return CatAtom(FILTERS[k[4:]])
    if k == "ext":
        return ExtAtom(re.split(r"[;,|]", v or ""))
    if k == "size":
        lo, hi = parse_size(v)
        return RangeAtom("size", lo, hi)
    if k == "dm":
        lo, hi = parse_date(v)
        if hi is not None:
            hi -= 1
        return RangeAtom("mtime", lo, hi)
    if k in ("parent", "in"):
        if not (v or "").strip():
            raise QueryError("Geef een map op, bijvoorbeeld parent:/home/robin/Downloads.")
        return ParentAtom(v, recursive=(k == "in"))
    if k == "empty":
        return EmptyAtom()
    raise QueryError(f"Onbekende zoekfunctie “{k}:”.")


def build_term(t: N_Term) -> Atom:
    text = t.text
    if t.regex:
        if not text:
            return AllAtom()
        return RegexAtom(text, t.case, t.path)
    b = enc(text)
    if b"*" in b or b"?" in b:
        if not b.strip(b"*"):
            return AllAtom()
        if b"?" not in b and not t.whole:
            core = b.strip(b"*")
            if b"*" not in core:
                lead, trail = b.startswith(b"*"), b.endswith(b"*")
                if not (t.path and b"/" in core):
                    anchor = "" if (lead and trail) else ("end" if lead else "start")
                    return NameAtom(core, anchor, t.case, t.path, False)
        return WildAtom(b, t.case, t.path)
    if t.path and not t.case:
        pass
    return NameAtom(b, "", t.case, t.path, t.whole)


class Compiled:
    def __init__(self, atom: Atom | None, highlight: list[str], regex: str | None, case: bool, empty: bool):
        self.atom = atom
        self.highlight = highlight
        self.regex = regex
        self.case = case
        self.empty = empty


def compile_query(query: str, *, case=False, path=False, whole=False, regex=False, flt="all") -> Compiled:
    query = (query or "").strip()
    if len(query) > 4000:
        raise QueryError("Deze zoekopdracht is te lang.")
    atoms: list[Atom] = []
    hl: list[str] = []
    rx = None
    if regex and query:
        atom = RegexAtom(query, case, path)
        rx = query
    else:
        node = Parser(query, case=case, path=path, whole=whole).parse()
        atom = build_atom(node, None)
        if atom is not None:
            atom.highlight(hl)
            rx = _first_regex(node)
    if atom is not None:
        atoms.append(atom)
    if flt and flt != "all":
        if flt == "folder":
            atoms.append(FlagAtom(T_DIR))
        elif flt in FILTERS:
            atoms.append(CatAtom(FILTERS[flt]))
        else:
            raise QueryError("Onbekend filter.")
    empty = not query
    if not atoms:
        root = None
    elif len(atoms) == 1:
        root = atoms[0]
    else:
        pos, neg = [], []
        for a in atoms:
            if isinstance(a, AndNode):
                pos.extend(a.pos)
                neg.extend(a.neg)
            else:
                pos.append(a)
        root = AndNode(pos, neg)
    seen = set()
    hl2 = []
    for x in sorted(hl, key=len, reverse=True):
        k = x if case else x.lower()
        if x and k not in seen:
            seen.add(k)
            hl2.append(x)
    return Compiled(root, hl2, rx, case, empty)


def _first_regex(node):
    if isinstance(node, N_Term) and node.regex:
        return node.text
    for attr in ("items",):
        for it in getattr(node, attr, []) or []:
            r = _first_regex(it)
            if r:
                return r
    if isinstance(node, N_Not):
        return None
    return None


# ---------------------------------------------------------------------------
# result materialisation
# ---------------------------------------------------------------------------

def _bisect_key(seq, x, keyf, reverse: bool) -> int:
    lo, hi = 0, len(seq)
    if not reverse:
        while lo < hi:
            mid = (lo + hi) // 2
            if keyf(seq[mid]) < x:
                lo = mid + 1
            else:
                hi = mid
    else:
        while lo < hi:
            mid = (lo + hi) // 2
            if keyf(seq[mid]) > x:
                lo = mid + 1
            else:
                hi = mid
    return lo


class Result:
    """An ordered result: base sequence (list, or mask over an order array) + merged extras."""

    def __init__(self, gen, total, base_kind, base, order=None, reverse=False, extras=None, keyf=None):
        self.gen = gen
        self.total = total
        self.base_kind = base_kind     # 'L' (materialised list/array) | 'M' (mask + order)
        self.base = base
        self.order = order
        self.reverse = reverse
        self.extras = extras or []     # gids in final order
        self.keyf = keyf
        self._pos = None
        self._cursor = None            # [merged index, iterator] of the lazy merge (sequential pages)
        self.created = time.monotonic()

    def _stream(self):
        o = self.order
        if self.reverse:
            return compress(reversed(o), map(self.base.__getitem__, reversed(o)))
        return compress(o, map(self.base.__getitem__, o))

    def _materialise(self):
        if self.base_kind == "M":
            self.base = array.array("I", self._stream())
            self.base_kind = "L"

    def _merged_page(self, offset: int, limit: int) -> list[int]:
        """A page of base stream + extras without materialising the base (4.7 M gids = 18 MiB for an
        empty query): heapq.merge with the extras first, so an extra goes before base entries with
        an equal key, where the bisect in page() puts it too. Costs O(offset + limit) keys; the
        next page continues the same iterator."""
        cur = self._cursor
        if cur is None or cur[0] > offset:
            cur = [0, heapq.merge(self.extras, self._stream(), key=self.keyf, reverse=self.reverse)]
        if offset > cur[0]:
            deque(islice(cur[1], offset - cur[0]), maxlen=0)
            cur[0] = offset
        out = list(islice(cur[1], limit))
        cur[0] += len(out)
        self._cursor = cur
        return out

    def page(self, offset: int, limit: int) -> list[int]:
        if not self.extras:
            if self.base_kind == "L":
                return list(self.base[offset:offset + limit])
            return list(islice(self._stream(), offset, offset + limit))
        if self.base_kind == "M" and offset + limit <= MERGE_STREAM_MAX:
            return self._merged_page(offset, limit)
        self._cursor = None
        self._materialise()
        if self._pos is None:
            base = self.base
            self._pos = [_bisect_key(base, self.keyf(e), self.keyf, self.reverse) for e in self.extras]
        out = []
        base, ex, pos = self.base, self.extras, self._pos
        # merged index of extra k = pos[k] + k
        end = offset + limit
        bi = offset
        k = 0
        # number of extras placed before merged index `offset`
        while k < len(ex) and pos[k] + k < offset:
            k += 1
        bi = offset - k
        while len(out) < limit:
            mi = bi + k
            if mi >= end:
                break
            if k < len(ex) and pos[k] + k == mi:
                out.append(ex[k])
                k += 1
            elif bi < len(base):
                out.append(base[bi])
                bi += 1
            elif k < len(ex):
                out.append(ex[k])
                k += 1
            else:
                break
        return out


# ---------------------------------------------------------------------------
# inotify (ctypes)
# ---------------------------------------------------------------------------

IN_MODIFY, IN_ATTRIB, IN_CLOSE_WRITE = 0x2, 0x4, 0x8
IN_MOVED_FROM, IN_MOVED_TO, IN_CREATE, IN_DELETE = 0x40, 0x80, 0x100, 0x200
IN_DELETE_SELF, IN_MOVE_SELF, IN_UNMOUNT, IN_Q_OVERFLOW, IN_IGNORED = 0x400, 0x800, 0x2000, 0x4000, 0x8000
IN_ONLYDIR, IN_DONT_FOLLOW, IN_EXCL_UNLINK, IN_ISDIR = 0x01000000, 0x02000000, 0x04000000, 0x40000000
WATCH_MASK = (IN_CREATE | IN_DELETE | IN_MOVED_FROM | IN_MOVED_TO | IN_CLOSE_WRITE | IN_ATTRIB | IN_DELETE_SELF
              | IN_MOVE_SELF | IN_ONLYDIR | IN_DONT_FOLLOW | IN_EXCL_UNLINK)


def inotify_watches_in_use(exclude: tuple[int, int] | None = None) -> int:
    """inotify watches held right now by all processes of this user (they share max_user_watches),
    from /proc/<pid>/fdinfo. ``exclude`` = (pid, fd) of our own inotify fd (its watches are ours to reuse)."""
    uid = os.getuid()
    total = 0
    try:
        pids = [p for p in os.listdir("/proc") if p.isdigit()]
    except OSError:
        return 0
    for pid in pids:
        try:
            if os.stat(f"/proc/{pid}").st_uid != uid:
                continue
            fds = os.listdir(f"/proc/{pid}/fd")
        except OSError:
            continue
        for fd in fds:
            try:
                if os.readlink(f"/proc/{pid}/fd/{fd}") != "anon_inode:inotify":
                    continue
                if exclude and int(pid) == exclude[0] and int(fd) == exclude[1]:
                    continue
                with open(f"/proc/{pid}/fdinfo/{fd}", "rb") as f:
                    total += sum(1 for line in f if line.startswith(b"inotify wd:"))
            except (OSError, ValueError):
                continue
    return total


def watch_budget(own_fd: int | None = None) -> int:
    """How many directories we may watch: at most half of what the user's other programs (editors, sync
    tools, file managers) leave free, keeping a reserve for them, never more than WATCH_CAP."""
    try:
        with open("/proc/sys/fs/inotify/max_user_watches") as f:
            mx = int(f.read().strip())
    except (OSError, ValueError):
        mx = 8192
    in_use = inotify_watches_in_use((os.getpid(), own_fd) if own_fd is not None and own_fd >= 0 else None)
    free = max(0, mx - in_use)
    reserve = min(16384, mx // 4)
    return max(0, min(WATCH_CAP, int(free * 0.5), free - reserve))


class Watcher:
    def __init__(self, sink):
        self.sink = sink   # callable(list of (dirpath, mask, name))
        self.fd = -1
        self.ok = False
        self.wd_path: dict[int, str] = {}
        self.path_wd: dict[str, int] = {}
        self.lock = threading.Lock()
        self.budget = 0                   # set once the inotify fd exists (see refresh_budget)
        self._stop_r, self._stop_w = os.pipe()
        self.thread = None
        self.overflow = False
        try:
            self.libc = ctypes.CDLL(None, use_errno=True)
            self.libc.inotify_init1.argtypes = [ctypes.c_int]
            self.libc.inotify_add_watch.argtypes = [ctypes.c_int, ctypes.c_char_p, ctypes.c_uint32]
            self.libc.inotify_rm_watch.argtypes = [ctypes.c_int, ctypes.c_int]
            fd = self.libc.inotify_init1(os.O_NONBLOCK | os.O_CLOEXEC)
            if fd >= 0:
                self.fd = fd
                self.ok = True
        except (OSError, AttributeError) as e:
            log("inotify niet beschikbaar: %s", e)
        self.refresh_budget()

    def refresh_budget(self) -> int:
        """Recompute the budget from what the user's other programs use now (our own watches excluded)."""
        self.budget = watch_budget(self.fd)
        return self.budget

    def start(self):
        if not self.ok or self.thread:
            return
        self.thread = threading.Thread(target=self._run, name="inotify", daemon=True)
        self.thread.start()

    def close(self):
        try:
            os.write(self._stop_w, b"x")
        except OSError:
            pass
        if self.fd >= 0:
            try:
                os.close(self.fd)
            except OSError:
                pass
            self.fd = -1
        self.ok = False

    def count(self) -> int:
        return len(self.wd_path)

    def add(self, path: str) -> bool:
        if not self.ok:
            return False
        with self.lock:
            if path in self.path_wd:
                return True
            if len(self.wd_path) >= self.budget:
                return False
            wd = self.libc.inotify_add_watch(self.fd, enc(path), WATCH_MASK)
            if wd < 0:
                err = ctypes.get_errno()
                if err == 28:   # ENOSPC: the user limit is reached → stop adding and give 10 % back,
                    #             so the user's other programs can still watch their folders
                    shed = max(1, len(self.wd_path) // 10)
                    for p, w in list(self.path_wd.items())[-shed:]:   # the least important: added last
                        self.path_wd.pop(p, None)
                        self.wd_path.pop(w, None)
                        try:
                            self.libc.inotify_rm_watch(self.fd, w)
                        except Exception:
                            pass
                    self.budget = len(self.wd_path)
                    log("inotify-limiet bereikt; %d mappen vrijgegeven (watch limit reached, released)", shed)
                return False
            old = self.wd_path.get(wd)
            if old is not None and old != path:
                self.path_wd.pop(old, None)
            self.wd_path[wd] = path
            self.path_wd[path] = wd
            return True

    def remove_prefix(self, path: str) -> None:
        if not self.ok:
            return
        pre = path.rstrip("/") + "/"
        with self.lock:
            victims = [(p, wd) for p, wd in self.path_wd.items() if p == path or p.startswith(pre)]
            for p, wd in victims:
                self.path_wd.pop(p, None)
                self.wd_path.pop(wd, None)
                try:
                    self.libc.inotify_rm_watch(self.fd, wd)
                except Exception:
                    pass

    def set_paths(self, paths: list[str]) -> None:
        if not self.ok:
            return
        want = set(paths)
        with self.lock:
            extra = [p for p in self.path_wd if p not in want]
        for p in extra:
            with self.lock:
                wd = self.path_wd.pop(p, None)
                if wd is not None:
                    self.wd_path.pop(wd, None)
                    try:
                        self.libc.inotify_rm_watch(self.fd, wd)
                    except Exception:
                        pass
        for p in paths:
            if not self.add(p):
                if len(self.wd_path) >= self.budget:
                    break

    def clear(self):
        self.set_paths([])

    def watched(self, path: str) -> bool:
        return path in self.path_wd

    def _run(self):
        poll = select.poll()
        poll.register(self.fd, select.POLLIN)
        poll.register(self._stop_r, select.POLLIN)
        pending = []
        last_flush = time.monotonic()
        while self.ok:
            try:
                ev = poll.poll(150 if pending else 1000)
            except OSError:
                break
            stop = any(fd == self._stop_r for fd, _ in ev)
            if stop:
                break
            if any(fd == self.fd for fd, _ in ev):
                try:
                    buf = os.read(self.fd, 256 * 1024)
                except BlockingIOError:
                    buf = b""
                except OSError:
                    break
                off = 0
                while off + 16 <= len(buf):
                    wd, mask, _cookie, ln = struct.unpack_from("iIII", buf, off)
                    name = buf[off + 16:off + 16 + ln].split(b"\0", 1)[0]
                    off += 16 + ln
                    if mask & IN_Q_OVERFLOW:
                        self.overflow = True
                        pending.append(("", IN_Q_OVERFLOW, ""))
                        continue
                    with self.lock:
                        dpath = self.wd_path.get(wd)
                        if mask & IN_IGNORED:
                            if dpath is not None:
                                self.wd_path.pop(wd, None)
                                if self.path_wd.get(dpath) == wd:
                                    self.path_wd.pop(dpath, None)
                            continue
                    if dpath is None:
                        continue
                    pending.append((dpath, mask, dec(name) if name else ""))
            now = time.monotonic()
            if pending and (now - last_flush > 0.15 or len(pending) > 5000):
                batch, pending = pending, []
                last_flush = now
                try:
                    self.sink(batch)
                except Exception:
                    log("inotify sink faalde:\n%s", traceback.format_exc())
            elif not pending:
                last_flush = now


# ---------------------------------------------------------------------------
# persistence
# ---------------------------------------------------------------------------

def _write_array(path, arr):
    with open(path, "wb") as f:
        f.write(arr)          # array.array or a typed memoryview: the raw machine bytes either way


def _read_array(path, typecode, count):
    a = array.array(typecode)
    with open(path, "rb") as f:
        a.fromfile(f, count)
    return a


class ByteMap(mmap.mmap):
    """flags.bin / cat.bin as a private copy-on-write map: a live change stays in this process and
    only the 4 KiB pages it touches become anonymous memory. Adds the bytearray methods the query
    code uses (an mmap has no translate/count)."""

    def translate(self, table, delete=b""):
        b = self[:]
        return b.translate(table, delete) if delete else b.translate(table)

    def count(self, x, *args):
        return self[:].count(x, *args)


def map_file(path: str, access=mmap.ACCESS_READ, cls=mmap.mmap):
    """mmap of a whole file (None when it is empty). The fd is closed right away.

    ACCESS_READ pages are clean page cache (reclaimable, never swapped); ACCESS_COPY (MAP_PRIVATE)
    pages turn anonymous only when written. Index files are never modified in place (a write
    under a mapping would show through, a truncation would SIGBUS): every save writes new files."""
    with open(path, "rb") as f:
        size = os.fstat(f.fileno()).st_size
        if not size:
            return None
        try:
            return cls(f.fileno(), size, access=access, trackfd=False)
        except TypeError:            # Python < 3.13: no trackfd
            return cls(f.fileno(), size, access=access)


def map_view(path: str, code: str, count: int, maps: dict, name: str, writable: bool = False):
    """Typed view (memoryview.cast) of a persisted array: read-only, or private copy-on-write."""
    mm = map_file(path, mmap.ACCESS_COPY if writable else mmap.ACCESS_READ)
    if mm is None:
        if count:
            raise ValueError(f"{name} is empty")
        return array.array(code)
    v = memoryview(mm).cast(code)
    if len(v) != count:
        raise ValueError(f"{name}: {len(v)} entries, expected {count}")
    maps[name] = mm
    return v


def map_bytes(path: str, count: int, maps: dict, name: str):
    mm = map_file(path, mmap.ACCESS_COPY, ByteMap)
    if mm is None:
        if count:
            raise ValueError(f"{name} is empty")
        return bytearray()
    if len(mm) != count:
        raise ValueError(f"{name}: {len(mm)} entries, expected {count}")
    maps[name] = mm
    return mm


def _to_array(code: str, v) -> array.array:
    """A heap copy of a typed memoryview (or an array: returned as is)."""
    if isinstance(v, memoryview):
        out = array.array(code)
        out.frombytes(v.cast("B"))
        return out
    return v


def _array_slice(code: str, v, a: int, b: int) -> array.array:
    """v[a:b] as a fresh array (v: array.array or a typed memoryview of an mmap)."""
    if isinstance(v, memoryview):
        out = array.array(code)
        out.frombytes(v[a:b].cast("B"))
        return out
    return v[a:b]


def _base_id_of(meta: dict) -> str:
    """Identifies the base layout of an index dir: a fold or crawl makes a new one; a save of the
    same base keeps it (v2 dirs of the old indexer have none: derived from their save time)."""
    return str(meta.get("base_id") or f"v2-{meta.get('saved')}")


def _new_base_id() -> str:
    return f"{time.time_ns():x}-{os.getpid()}"


def _parse_bytes(s: str | None) -> int | None:
    """"4000000000", "4G", "1.5GB", "600M" → bytes (None: not set or not understood)."""
    m = re.fullmatch(r"\s*([0-9]+(?:[.,][0-9]+)?)\s*([kmgt]?)i?b?\s*", (s or "").lower())
    if not m:
        return None
    return int(float(m.group(1).replace(",", ".")) * 1024 ** "_kmgt".index(m.group(2) or "_"))


MIN_FREE = _parse_bytes(os.environ.get("WIN11SHELL_INDEX_MIN_FREE"))   # free disk a helper fold needs
# (default: twice the index dir, see Engine._fold_guard; 0 = no check; below it a fold waits, see _t_compact)


def index_dir_bytes(d: str) -> int:
    seen, tot = set(), 0
    try:
        for e in os.scandir(d):
            st = e.stat(follow_symlinks=False)
            if _st.S_ISREG(st.st_mode) and st.st_ino not in seen:
                seen.add(st.st_ino)
                tot += st.st_size
    except OSError:
        pass
    return tot


BASE_FILES = ("names.orig", "names.low", "offs.bin", "lorder.bin", "parent.bin", "order.bin", "rank.bin")
SAVE_MARGIN = 64 << 20      # a save never leaves less free disk space than this


# link(2) errors that mean "this file system has no hard links here" (exFAT / vfat, some FUSE fs)
_NO_LINK_ERRNOS = {errno.EPERM, errno.EXDEV, errno.EMLINK, errno.ENOTSUP, errno.EOPNOTSUPP, errno.ENOSYS}
_LINKS_OK: dict = {}        # st_dev of an index dir's file system -> whether link(2) works there


def _links_work(src: str, d: str) -> bool:
    """Can files of src be hard-linked into d? Probed once per file system (a link of
    src/meta.json); a save without links copies the base and orders (~0.6 GB here) and must
    count them before it writes anything."""
    dev = os.stat(d).st_dev
    ok = _LINKS_OK.get(dev)
    if ok is None:
        probe = os.path.join(d, ".linkprobe")
        try:
            os.link(os.path.join(src, "meta.json"), probe)
            os.unlink(probe)
            ok = True
        except OSError as e:
            if e.errno not in _NO_LINK_ERRNOS:
                raise
            ok = False
            log("geen harde links in %s (%s): elke save kopieert de hele index", d, e)
        _LINKS_OK[dev] = ok
    return ok


def _link_or_copy(src: str, dst: str) -> None:
    try:
        os.link(src, dst)
    except OSError as e:
        if e.errno not in _NO_LINK_ERRNOS:
            raise
        shutil.copyfile(src, dst)


def _dir_id(d: str | None) -> tuple | None:
    """(st_dev, st_ino) of d/meta.json: tells whether an index dir is still the one we use."""
    if not d:
        return None
    try:
        st = os.stat(os.path.join(d, "meta.json"))
    except OSError:
        return None
    return (st.st_dev, st.st_ino)


def disk_free(path: str) -> int:
    st = os.statvfs(path)
    return st.f_bavail * st.f_frsize


def _check_space(d: str, need: int, keep: int = SAVE_MARGIN) -> None:
    free = disk_free(d)
    if free - need < keep:
        raise OSError(28, f"te weinig vrije schijfruimte ({free / 1e6:.0f} MB vrij, {need / 1e6:.0f} MB nodig)")


def _read_meta(d: str) -> dict | None:
    try:
        with open(os.path.join(d, "meta.json"), encoding="utf-8", errors="surrogateescape") as f:
            return json.load(f)
    except (OSError, ValueError):
        return None


def _read_orders(d: str) -> dict | None:
    try:
        with open(os.path.join(d, "orders.json")) as f:
            od = json.load(f)
    except (OSError, ValueError):
        return None
    # no "format": written before it existed, by code with exactly the layout of format 1
    if not isinstance(od, dict) or not isinstance(od.get("files"), dict) or od.get("format", 1) != ORDERS_FORMAT:
        return None
    return od


def _load_orders(idx: Index, base: str) -> None:
    """Adopt the persisted sort orders of base (orders.json) when they were built for exactly this
    base: mapped, or read into the heap for a heap index."""
    od = _read_orders(base)
    if not od or od.get("base_id") != idx.base_id or od.get("N") != idx.N:
        return
    dirty = od.get("dirty") or {}
    for k, fn in od["files"].items():
        kind = k[4:] if k.startswith("key:") else k
        if kind not in ORDER_KINDS or fn != Index.order_file(k):
            continue
        p = os.path.join(base, fn)
        try:
            if idx.mapped:
                v = map_view(p, "I", idx.N, idx.maps, fn)
            elif os.path.getsize(p) == 4 * idx.N:
                v = _read_array(p, "I", idx.N)
            else:
                continue
        except (OSError, ValueError):
            continue
        idx.orders[k] = v
        idx.order_files[k] = fn
        if k in ("size", "modified"):
            idx.odirty[k] = set(dirty.get(k) or ())


def write_index_dir(idx: Index, d: str, meta_extra: dict) -> dict:
    """Write idx as a complete index directory d (a new directory: nothing is ever rewritten in
    place, the old files may be mapped). When idx's base still lives in idx.home (same base_id),
    its files are hard-linked instead of written; the mutable columns (size, mtime, flags, cat),
    the overlay and meta.json are always written."""
    os.makedirs(d, mode=0o700, exist_ok=True)
    J = os.path.join
    N = idx.N
    src_meta = _read_meta(idx.home) if idx.home and idx.base_id else None
    src = idx.home if src_meta and _base_id_of(src_meta) == idx.base_id and src_meta.get("N") == N else None
    need = 26 * N + 128 * len(idx.ov) + 4 * N * sum(1 for k in idx.orders if not (src and k in idx.order_files))
    if not src:
        need += sum(c.olen + c.llen for c in idx.chunks) + 18 * N
    elif not _links_work(src, d):                       # every "link" below is a copy
        for fn in (*BASE_FILES, *(idx.order_files[k] for k in idx.orders if k in idx.order_files)):
            try:
                need += os.path.getsize(J(src, fn))
            except OSError:
                pass
    _check_space(d, need)
    if src:
        for fn in BASE_FILES:
            p = J(src, fn)
            if fn == "rank.bin" and not (os.path.isfile(p) and os.path.getsize(p) == 4 * N):
                _write_array(J(d, fn), idx.rank)        # computed at load (an old dir without it)
            elif os.path.isfile(p):
                _link_or_copy(p, J(d, fn))
            elif fn == "names.low":
                open(J(d, fn), "wb").close()             # the heap loader (and the old indexer) opens it
        chunks_meta = src_meta["chunks"]                 # describes exactly the linked name files
    else:
        chunks_meta = []
        with open(J(d, "names.orig"), "wb") as fo, open(J(d, "names.low"), "wb") as fl, \
                open(J(d, "offs.bin"), "wb") as fof, open(J(d, "lorder.bin"), "wb") as flo:
            for c in idx.chunks:
                orig = c.orig
                fo.write(orig)
                llen = 0
                if not c.low_is_orig():
                    low = c.low
                    if low != orig:
                        fl.write(low)
                        llen = len(low)
                fof.write(c.offs)
                flo.write(c.lorder)
                chunks_meta.append([c.start, c.n, len(orig), llen])
        _write_array(J(d, "parent.bin"), idx.parent)
        _write_array(J(d, "order.bin"), idx.order)
        _write_array(J(d, "rank.bin"), idx.rank)
    _write_array(J(d, "size.bin"), idx.size)
    _write_array(J(d, "mtime.bin"), idx.mtime)
    with open(J(d, "flags.bin"), "wb") as f:
        f.write(idx.flags)
    with open(J(d, "cat.bin"), "wb") as f:
        f.write(idx.cat)
    ov = idx.ov
    ov_rows = []
    for k in range(len(ov)):
        if ov.flags[k] & F_DEL:
            continue
        ov_rows.append([k, ov.parent[k], dec(ov.orig[k]), ov.size[k], ov.mtime[k], ov.flags[k], ov.cat[k], ov.seg[k]])
    with open(J(d, "overlay.json"), "w", encoding="utf-8", errors="surrogateescape") as f:
        json.dump(ov_rows, f, ensure_ascii=True)
    base_id = idx.base_id if src else _new_base_id()
    # the sort orders: files of home are linked, orders built in the heap written; with the dirty
    # sets relative to the size / mtime written above (an old indexer ignores these files)
    files = {}
    for k, o in list(idx.orders.items()):
        fn = Index.order_file(k)
        have = idx.order_files.get(k)
        if src and have and os.path.isfile(J(src, have)):
            _link_or_copy(J(src, have), J(d, fn))
        else:
            _write_array(J(d, fn), o)
        files[k] = fn
    with open(J(d, "orders.json"), "w") as f:
        json.dump({"base_id": base_id, "N": N, "format": ORDERS_FORMAT, "files": files,
                   "dirty": {k: sorted(v) for k, v in idx.odirty.items() if k in files}}, f)
    meta = {
        "version": FORMAT_VERSION, "N": N, "saved": time.time() * 1000, "base_id": base_id,
        "segs": [{"path": s.path, "start": s.start, "n": s.n, "fs": s.fs, "dev": s.dev, "disk": s.disk,
                  "built": s.built, "dead": s.dead, "errors": s.errors, "noshow": bool(idx.flags[s.start] & F_NOSHOW)
                  if s.n else False} for s in idx.segs],
        "chunks": chunks_meta,
        **meta_extra,
    }
    with open(J(d, "meta.json"), "w", encoding="utf-8", errors="surrogateescape") as f:
        json.dump(meta, f, ensure_ascii=True)
        f.flush()
        os.fsync(f.fileno())
    return meta


def install_index_dir(cache_dir: str, tmp: str) -> None:
    """Make the complete dir tmp the current <cache>/index. The old dir is removed; files of it
    that are still mapped stay readable (and keep their disk space) until they are unmapped."""
    base = os.path.join(cache_dir, "index")
    old = None
    if os.path.isdir(base):
        old = os.path.join(cache_dir, f"index.old-{os.getpid()}-{time.time_ns()}")
        os.rename(base, old)
    try:
        os.rename(tmp, base)
    except OSError:
        if old:
            os.rename(old, base)                 # never leave index/ missing (it is mapped)
        raise
    if old:
        shutil.rmtree(old, ignore_errors=True)


def save_index(idx: Index, cache_dir: str, meta_extra: dict) -> None:
    tmp = os.path.join(cache_dir, f"index.tmp-{os.getpid()}")
    if os.path.isdir(tmp):
        shutil.rmtree(tmp, ignore_errors=True)
    try:
        meta = write_index_dir(idx, tmp, meta_extra)
    except BaseException:
        shutil.rmtree(tmp, ignore_errors=True)
        raise
    install_index_dir(cache_dir, tmp)
    idx.base_id = meta["base_id"]


def remap_index(idx: Index, base: str) -> None:
    """After a save that linked idx's base into ``base``: map everything from there. The base and
    order files are the same inodes (hard links), size / mtime / flags / cat the files just written
    (which drops their copy-on-write pages). Afterwards every mapping is a file of ``base``: none
    keeps a removed file alive, and /proc/<pid>/maps shows no "(deleted)" entries."""
    J = os.path.join
    N = idx.N
    meta = _read_meta(base)
    if not meta or _base_id_of(meta) != idx.base_id:
        raise ValueError("the saved index is not this one")
    m = _load_index_mapped(base, meta)
    for k in list(idx.orders):
        fn = Index.order_file(k)
        idx.orders[k] = map_view(J(base, fn), "I", N, m.maps, fn)
        idx.order_files[k] = fn
    idx.parent, idx.size, idx.mtime, idx.order, idx.rank = m.parent, m.size, m.mtime, m.order, m.rank
    idx.flags, idx.cat = m.flags, m.cat
    idx.chunks, idx.cstarts = m.chunks, m.cstarts
    idx.maps = m.maps
    idx.home = base
    idx.home_id = _dir_id(base)
    idx.mapped = True
    idx._counts = None


def _pid_gone(s: str) -> bool:
    return s.isdigit() and int(s) != os.getpid() and not os.path.exists(f"/proc/{s}")


def cleanup_cache_dir(cache_dir: str) -> None:
    """At start: remove what a killed indexer (or helper) left behind in the cache dir, and recover
    the index when a save died between its two renames (index/ missing, index.old-* complete)."""
    J = os.path.join
    base = J(cache_dir, "index")
    try:
        names = sorted(os.listdir(cache_dir))
    except OSError:
        return
    if not os.path.exists(base):
        for pre in ("index.old-", "index.tmp-"):        # the dir that was in use, else the new one
            done = [(m.get("saved") or 0, n) for n in names if n.startswith(pre) and not n.endswith("-fold")
                    for m in (_read_meta(J(cache_dir, n)),) if m]
            if done:
                n = max(done)[1]
                try:
                    os.rename(J(cache_dir, n), base)
                    log("index hersteld uit %s", n)
                except OSError:
                    pass
                break
    for n in names:
        if n.startswith(("index.tmp-", "index.old-")) and _pid_gone(n.split("-")[1]) and os.path.isdir(J(cache_dir, n)):
            shutil.rmtree(J(cache_dir, n), ignore_errors=True)
    try:
        for n in os.listdir(base):
            if ".tmp-" in n and _pid_gone(n.rsplit(".tmp-", 1)[1]):
                os.unlink(J(base, n))
    except OSError:
        pass


def _load_segs(idx: Index, meta: dict) -> None:
    for s in meta["segs"]:
        seg = Seg(s["path"], s["start"], s["n"], s.get("fs", ""), s.get("dev", 0), s.get("disk", ""),
                  s.get("built", 0), s.get("errors", 0))
        seg.dead = s.get("dead", 0)
        idx.segs.append(seg)


def _load_overlay(idx: Index, base: str) -> None:
    N = idx.N
    idx.ov = Overlay(N)
    op = os.path.join(base, "overlay.json")
    if os.path.isfile(op):
        with open(op, encoding="utf-8", errors="surrogateescape") as f:
            rows = json.load(f)
        remap = {}
        ov = idx.ov
        for k, par, name, sz, mt, fl, ct, si in rows:
            g = N + len(ov.orig)
            remap[N + k] = g
            par = remap.get(par, par)
            nb = enc(name)
            ov.orig.append(nb)
            ov.low.append(lower_b(nb))
            ov.parent.append(par)
            ov.size.append(sz)
            ov.mtime.append(mt)
            ov.flags.append(fl)
            ov.cat.append(ct)
            ov.seg.append(si)
            ov.kids.setdefault(par, []).append(g)


def load_index(cache_dir: str) -> tuple[Index, dict] | None:
    return load_index_dir(os.path.join(cache_dir, "index"))


def load_index_dir(base: str, heap: bool | None = None) -> tuple[Index, dict] | None:
    """Load an index dir: mapped from the files (default), or read into the heap
    (``heap=True``, or WIN11SHELL_INDEX_MMAP=0)."""
    mp = os.path.join(base, "meta.json")
    if not os.path.isfile(mp):
        return None
    with open(mp, encoding="utf-8", errors="surrogateescape") as f:
        meta = json.load(f)
    if meta.get("version") != FORMAT_VERSION:
        return None
    if heap if heap is not None else not USE_MMAP:
        idx = _load_index_heap(base, meta)
    else:
        idx = _load_index_mapped(base, meta)
    idx.home = base
    idx.home_id = _dir_id(base)
    idx.base_id = _base_id_of(meta)
    _load_segs(idx, meta)
    _load_overlay(idx, base)
    _load_orders(idx, base)
    return idx, meta


def _load_index_mapped(base: str, meta: dict) -> Index:
    """The base as views of the files: read-only maps for the immutable parts, private
    copy-on-write maps for size / mtime / flags / cat (live updates change them in place)."""
    N = int(meta["N"])
    idx = Index()
    idx.N = N
    idx.mapped = True
    maps = idx.maps
    J = os.path.join
    idx.parent = map_view(J(base, "parent.bin"), "i", N, maps, "parent.bin")
    idx.size = map_view(J(base, "size.bin"), "q", N, maps, "size.bin", writable=True)
    idx.mtime = map_view(J(base, "mtime.bin"), "q", N, maps, "mtime.bin", writable=True)
    idx.order = map_view(J(base, "order.bin"), "I", N, maps, "order.bin")
    idx.flags = map_bytes(J(base, "flags.bin"), N, maps, "flags.bin")
    idx.cat = map_bytes(J(base, "cat.bin"), N, maps, "cat.bin")
    chunks = meta["chunks"]
    mo = map_file(J(base, "names.orig")) if chunks else None
    ml = map_file(J(base, "names.low")) if any(c[3] for c in chunks) else None
    if mo is not None:
        maps["names.orig"] = mo
    if ml is not None:
        maps["names.low"] = ml
    offs_all = map_view(J(base, "offs.bin"), "I", sum(c[1] + 1 for c in chunks), maps, "offs.bin")
    lorder_all = map_view(J(base, "lorder.bin"), "H", sum(c[1] for c in chunks), maps, "lorder.bin")
    po = pl = ko = kl = 0
    for start, n, olen, llen in chunks:
        offs = offs_all[ko:ko + n + 1]
        lorder = lorder_all[kl:kl + n]
        ko += n + 1
        kl += n
        if llen:
            c = Chunk(start, mo, ml, offs, lorder, ob=po, olen=olen, lb=pl, llen=llen)
        else:
            c = Chunk(start, mo, mo, offs, lorder, ob=po, olen=olen, lb=po, llen=olen)
        po += olen
        pl += llen
        idx.chunks.append(c)
        idx.cstarts.append(start)
    if (len(mo) if mo is not None else 0) != po or (len(ml) if ml is not None else 0) != pl:
        raise ValueError("names file truncated")
    rp = J(base, "rank.bin")
    if os.path.isfile(rp) and os.path.getsize(rp) == 4 * N:
        idx.rank = map_view(rp, "I", N, maps, "rank.bin")
    else:
        rank = array.array("I", bytes(4 * N))
        for r, g in enumerate(idx.order):
            rank[g] = r
        idx.rank = rank
    return idx


def _load_index_heap(base: str, meta: dict) -> Index:
    """Everything read into the heap (WIN11SHELL_INDEX_MMAP=0: the loader before the mmap one)."""
    N = int(meta["N"])
    idx = Index()
    idx.N = N
    idx.parent = _read_array(os.path.join(base, "parent.bin"), "i", N)
    idx.size = _read_array(os.path.join(base, "size.bin"), "q", N)
    idx.mtime = _read_array(os.path.join(base, "mtime.bin"), "q", N)
    idx.order = _read_array(os.path.join(base, "order.bin"), "I", N)
    with open(os.path.join(base, "flags.bin"), "rb") as f:
        idx.flags = bytearray(f.read())
    with open(os.path.join(base, "cat.bin"), "rb") as f:
        idx.cat = bytearray(f.read())
    if len(idx.flags) != N or len(idx.cat) != N:
        raise ValueError("index files are inconsistent")
    with open(os.path.join(base, "names.orig"), "rb") as fo, open(os.path.join(base, "names.low"), "rb") as fl, \
            open(os.path.join(base, "offs.bin"), "rb") as fof, open(os.path.join(base, "lorder.bin"), "rb") as flo:
        for start, n, olen, llen in meta["chunks"]:
            orig = fo.read(olen)
            low = fl.read(llen) if llen else orig
            offs = array.array("I")
            offs.fromfile(fof, n + 1)
            lorder = array.array("H")
            lorder.fromfile(flo, n)
            if len(orig) != olen:
                raise ValueError("names file truncated")
            c = Chunk(start, orig, low, offs, lorder)
            idx.chunks.append(c)
            idx.cstarts.append(start)
    rp = os.path.join(base, "rank.bin")
    if os.path.isfile(rp) and os.path.getsize(rp) == 4 * N:
        idx.rank = _read_array(rp, "I", N)
    else:
        rank = array.array("I", bytes(4 * N))
        for r, g in enumerate(idx.order):
            rank[g] = r
        idx.rank = rank
    return idx


# ---------------------------------------------------------------------------
# relayout (fold overlay + tombstones into a fresh segment)
# ---------------------------------------------------------------------------

def extract_segdata(idx: Index, si: int, cancel=None) -> SegData:
    seg = idx.segs[si]
    sd = SegData(seg.path)
    sd.fs, sd.dev, sd.disk, sd.built, sd.errors = seg.fs, seg.dev, seg.disk, seg.built, seg.errors
    sd.noshow_root = bool(seg.n and idx.flags[seg.start] & F_NOSHOW)
    has_ov = any(s == si for s, f in zip(idx.ov.seg, idx.ov.flags) if not f & F_DEL)
    if not seg.dead and not has_ov:
        a, b = seg.start, seg.end
        sd.n = seg.n
        sd.parent = array.array("i", map((-a).__add__, idx.parent[a:b]))
        sd.parent[0] = -1
        sd.size = _array_slice("q", idx.size, a, b)
        sd.mtime = _array_slice("q", idx.mtime, a, b)
        sd.flags = bytearray(idx.flags[a:b])
        sd.flags[0] &= ~F_NOSHOW & 0xFF
        sd.cat = bytearray(idx.cat[a:b])
        for c in idx.chunks:
            if a <= c.start < b:
                # heap copies: the new index must not keep the maps of this one alive
                orig = c.orig
                sd.chunks.append((orig, orig if c.low_is_orig() else c.low, _to_array("I", c.offs),
                                  _to_array("H", c.lorder)))
        return sd
    # BFS re-layout over the live tree
    root = seg.start
    names: list[bytes] = []
    newidx: dict[int, int] = {}
    q = deque([root])
    p_ap, s_ap, m_ap = sd.parent.append, sd.size.append, sd.mtime.append
    n = 0
    while q:
        g = q.popleft()
        if cancel is not None and cancel.is_set():
            raise Cancelled()
        fl = idx.flags_of(g)
        par = idx.parent_of(g)
        newidx[g] = n
        names.append(idx.name_orig(g))
        p_ap(newidx[par] if par >= 0 and g != root else -1)
        s_ap(idx.size_of(g))
        m_ap(idx.mtime_of(g))
        sd.flags.append(fl & ~F_NOSHOW & 0xFF)
        sd.cat.append(idx.cat_of(g))
        n += 1
        if len(names) >= CHUNK:
            sd.chunks.append(make_chunk(names))
            names = []
        if fl & F_DIR:
            q.extend(idx.kids(g))
    if names:
        sd.chunks.append(make_chunk(names))
    sd.n = n
    return sd


# ---------------------------------------------------------------------------
# engine
# ---------------------------------------------------------------------------

class Engine:
    def __init__(self, cache_dir: str, emit, *, nice: bool = True):
        self.cache_dir = cache_dir
        self.emit = emit
        self.lock = threading.RLock()
        self.idx = Index()
        self.loaded = False
        self.cfg_roots: list[str] = []
        self.cfg_excluded: list[str] = []
        self.live = True
        self.configured = False
        self.tasks: queue.Queue = queue.Queue()
        self.jobs: dict[str, Progress] = {}
        self.job_cancel = threading.Event()
        self.root_errors: dict[str, str] = {}
        self.watcher: Watcher | None = None
        self.stopping = threading.Event()
        self.dirty_persist = False
        self.last_persist = time.monotonic()
        self.results: OrderedDict = OrderedDict()
        self.rescan_now = threading.Event()
        self.rescanning = False
        self.changed_pending = False
        self.last_changed = 0.0
        self.last_status_emit = 0.0
        self.built_at = None
        self.error = None
        self.mounts_cache = (0.0, [])
        self.fold_retry_at = 0.0         # monotonic time before which no fold is tried again
        self.fold_delay = FOLD_RETRY     # the next wait after a fold that could not run
        self.home_lost = False           # the index dir in use was removed; a save is queued
        self.writer = threading.Thread(target=self._writer, name="writer", daemon=True)
        self.rescanner = threading.Thread(target=self._rescanner, name="rescan", daemon=True)
        self.ticker = threading.Thread(target=self._ticker, name="ticker", daemon=True)

    # -- lifecycle ---------------------------------------------------------------------
    def start(self):
        try:
            t0 = time.monotonic()
            cleanup_cache_dir(self.cache_dir)
            got = load_index(self.cache_dir)
            if got:
                self.idx, meta = got
                self.loaded = True
                self.built_at = meta.get("built") or meta.get("saved")
                self.cfg_excluded_saved = meta.get("excluded")
                log("index geladen: %d items in %.2f s (%s)", self.idx.N + len(self.idx.ov), time.monotonic() - t0,
                    "mmap" if self.idx.mapped else "heap")
        except Exception as e:
            log("index laden mislukt (%s); opnieuw opbouwen", e)
            self.idx = Index()
        trim_heap()                  # the load temporaries (~30 MiB) would stay in the heap otherwise
        self.writer.start()
        self.rescanner.start()
        self.ticker.start()

    def stop(self):
        self.stopping.set()
        self.job_cancel.set()
        self.tasks.put(("stop",))
        self.writer.join(timeout=8)
        if self.watcher:
            self.watcher.close()

    # -- helpers ------------------------------------------------------------------------
    def mounts(self):
        t, m = self.mounts_cache
        if time.monotonic() - t > 10:
            m = read_mounts()
            self.mounts_cache = (time.monotonic(), m)
        return m

    def excludes_set(self) -> set:
        s = set(DEFAULT_EXCLUDES)
        for p in self.cfg_excluded:
            p = norm_path(p)
            if p:
                s.add(p)
                try:
                    s.add(os.path.realpath(p))
                except OSError:
                    pass
        return s

    def is_excluded(self, path: str, ex: set | None = None) -> bool:
        """Is path inside an excluded folder? Only exclusions *below* its root count, so a root
        the user picked inside e.g. /tmp is still indexed completely."""
        ex = ex if ex is not None else self.excludes_set()
        root = ""
        for s in self.idx.segs:
            r = s.path
            if (path == r or path.startswith(r.rstrip("/") + "/")) and len(r) > len(root):
                root = r
        rpre = root.rstrip("/") + "/" if root else "/"
        for e in ex:
            if e == root or not (e.startswith(rpre) or not root):
                continue
            if path == e or path.startswith(e.rstrip("/") + "/"):
                return True
        return False

    def state(self) -> str:
        if any(j.state in ("pending", "building") for j in self.jobs.values()):
            return "building"
        if self.idx.segs:
            return "ready"
        if self.configured and self.root_errors:
            return "error"
        return "idle"

    def status(self) -> dict:
        with self.lock:
            idx = self.idx
            files, dirs = idx.counts()
            roots = []
            seen = set()
            for s in idx.segs:
                seen.add(s.path)
                j = self.jobs.get(s.path)
                lo, hi = s.start, s.end
                fl = idx.flags[lo:hi]
                nd = fl.translate(T_ADIR).count(1)
                nf = fl.translate(T_AFILE).count(1)
                roots.append({"path": s.path, "files": nf, "folders": nd, "fsType": s.fs,
                              "state": j.state if j and j.state in ("pending", "building") else "ready",
                              "builtAt": s.built or None})
            for p, j in self.jobs.items():
                if p not in seen and j.state != "done":
                    roots.append({"path": p, "files": j.done, "folders": 0, "fsType": "", "state": j.state,
                                  "error": j.error})
            for p, err in self.root_errors.items():
                if p not in seen and p not in self.jobs:
                    roots.append({"path": p, "files": 0, "folders": 0, "state": "error", "error": err})
            running = [j for j in self.jobs.values() if j.state in ("pending", "building")]
            progress = None
            if running:
                done = sum(j.done for j in running)
                est = sum(max(j.estimate, j.done + 1) for j in running)
                cur = next((j.current for j in running if j.state == "building" and j.current), "")
                progress = {"done": done, "current": cur, "total": est,
                            "percent": min(99, int(done * 100 / est)) if est else None}
            st = {
                "state": self.state(), "files": files, "folders": dirs, "roots": roots,
                "builtAt": self.built_at, "progress": progress,
                "watching": self.watcher.count() if self.watcher else 0,
                "liveUpdate": self.live, "refreshing": self.rescanning,
                "generation": idx.gen, "overlay": idx.ov.live(),
                "excluded": list(self.cfg_excluded),
            }
        return st

    def emit_status(self, force=False):
        now = time.monotonic()
        if not force and now - self.last_status_emit < 0.4:
            return
        self.last_status_emit = now
        try:
            self.emit("status", self.status())
        except Exception:
            log("status faalde:\n%s", traceback.format_exc())

    def mark_changed(self):
        self.idx.touch()
        self.changed_pending = True
        self.dirty_persist = True
        self.results.clear()

    def _ticker(self):
        n = 0
        while not self.stopping.wait(0.25):
            n += 1
            if not n % 8:
                self._check_home()
            if self.changed_pending and time.monotonic() - self.last_changed > 0.3:
                self.changed_pending = False
                self.last_changed = time.monotonic()
                self.emit("changed", {"generation": self.idx.gen})
            if self.jobs and any(j.state == "building" for j in self.jobs.values()):
                self.emit_status()
            if self.dirty_persist and time.monotonic() - self.last_persist > PERSIST_INTERVAL:
                self.tasks.put(("persist",))
                self.last_persist = time.monotonic()

    def _check_home(self):
        """Every 2 s: is the index dir we map still there? A user or a cleaner may remove
        ~/.cache/win11shell/index while we run. The maps keep working, but the removed files keep
        their disk space and the helpers cannot map them: save the index again, which moves the
        maps to the new files (a failed save is retried every PERSIST_INTERVAL)."""
        idx = self.idx
        if not (idx.mapped and idx.home) or not self.lock.acquire(blocking=False):
            return                       # a save or fold holds the lock: it sets home_id itself
        try:
            if self.idx is not idx:
                return
            if _dir_id(idx.home) == idx.home_id:
                self.home_lost = False
                return
            if self.home_lost:
                return
            self.home_lost = True
            log("indexmap %s is verwijderd of vervangen; opnieuw opslaan", idx.home)
            self.dirty_persist = True
            self.last_persist = time.monotonic()
            self.tasks.put(("persist",))
        finally:
            self.lock.release()

    # -- configuration -------------------------------------------------------------
    def configure(self, args: dict):
        self.tasks.put(("configure", dict(args or {})))

    def rebuild(self):
        self.tasks.put(("rebuild",))

    def _resolve_roots(self, roots) -> list[str]:
        if roots in (None, "", "auto") or roots == ["auto"]:
            roots = auto_roots(self.mounts())
        out = []
        devs = {}
        for r in roots:
            p = norm_path(r)
            if not p:
                continue
            try:
                rp = os.path.realpath(p)
                st = os.stat(rp)
            except OSError as e:
                self.root_errors[p] = "Deze locatie is niet gevonden."
                log("root %s overgeslagen: %s", p, e)
                continue
            if not _st.S_ISDIR(st.st_mode):
                self.root_errors[p] = "Dit is geen map."
                continue
            if rp in out:
                continue
            out.append(rp)
            devs[rp] = st.st_dev
        # drop roots inside another root on the same device (already covered)
        keep = []
        for r in out:
            covered = False
            for o in out:
                if o != r and (r.startswith(o.rstrip("/") + "/")) and devs.get(o) == devs.get(r):
                    covered = True
                    break
            if not covered:
                keep.append(r)
        return keep

    # -- writer thread -------------------------------------------------------------------
    def _writer(self):
        while True:
            task = self.tasks.get()
            kind = task[0]
            if kind == "stop":
                try:
                    if self.dirty_persist and self.idx.segs:
                        self._persist()
                except Exception:
                    log("opslaan bij afsluiten mislukt:\n%s", traceback.format_exc())
                return
            try:
                getattr(self, "_t_" + kind)(*task[1:])
            except Cancelled:
                pass
            except Exception:
                log("taak %s faalde:\n%s", kind, traceback.format_exc())
                self.error = f"{kind} failed"

    def _t_configure(self, args):
        first = not self.configured
        self.configured = True
        if "liveUpdate" in args:
            self.live = bool(args.get("liveUpdate"))
        if "excluded" in args and isinstance(args.get("excluded"), list):
            new_ex = [norm_path(p) for p in args["excluded"] if isinstance(p, str) and norm_path(p)]
        else:
            new_ex = list(self.cfg_excluded)
        old_ex = set(self.cfg_excluded) if not first else set(getattr(self, "cfg_excluded_saved", None) or [])
        self.cfg_excluded = new_ex
        self.root_errors = {}
        roots = self._resolve_roots(args.get("roots", "auto"))
        self.cfg_roots = roots
        have = {s.path for s in self.idx.segs}
        want = set(roots)
        remove = [s for s in have if s not in want]
        add = [r for r in roots if r not in have]
        if remove:
            self._drop_segments(remove)
        # exclusions changed: prune newly excluded, re-crawl un-excluded
        if self.idx.segs:
            ex_now = set(new_ex)
            newly = [p for p in ex_now if p not in old_ex]
            unex = [p for p in old_ex if p not in ex_now]
            for p in newly:
                self._prune_path(p)
            for p in unex:
                self._recrawl_path(p)
        if add:
            self._start_crawl(add)
        self._setup_watches()
        if self.loaded and first:
            self.rescan_now.set()
            self.tasks.put(("warm",))
        self.emit_status(force=True)

    def _t_rebuild(self):
        roots = list(self.cfg_roots) or [s.path for s in self.idx.segs]
        if not roots:
            return
        self._start_crawl(roots, rebuild=True)
        self.emit_status(force=True)

    def _start_crawl(self, roots: list[str], rebuild=False):
        mounts = self.mounts()
        ex = self.excludes_set()
        groups: dict[str, list] = {}
        prev = {s.path: s.n for s in self.idx.segs}
        for r in roots:
            if r in self.jobs and self.jobs[r].state in ("pending", "building"):
                continue
            m = mount_for(r, mounts) or {"fs": "", "source": "", "mount": r}
            disk = _disk_of(m.get("source", "")) or m.get("source", "") or r
            est = prev.get(r) or estimate_entries(r)
            pr = Progress(est)
            self.jobs[r] = pr
            groups.setdefault(disk, []).append((r, m.get("fs", ""), disk, pr, m.get("mount", "")))
        self.job_cancel.clear()
        for disk, items in groups.items():
            t = threading.Thread(target=self._crawl_group, args=(items, ex), name=f"crawl-{disk}", daemon=True)
            t.start()

    def _crawl_group(self, items, ex):
        for (r, fs, disk, pr, mnt) in items:
            if self.job_cancel.is_set() or self.stopping.is_set():
                pr.state = "cancelled"
                continue
            pr.state = "building"
            pr.started = time.monotonic()
            try:
                t0 = time.monotonic()
                sd = crawl_root(r, excludes=ex, fs=fs, disk=disk, cancel=self.job_cancel, progress=pr)
                if self.job_cancel.is_set():
                    pr.state = "cancelled"
                    continue
                # a root that is a mount inside another indexed root: its own dir entry already exists there
                for other in self.cfg_roots:
                    if other != r and r.startswith(other.rstrip("/") + "/"):
                        sd.noshow_root = True
                log("root %s gecrawld: %d items in %.1f s", r, sd.n, time.monotonic() - t0)
                self.tasks.put(("segment", sd, pr))
            except Exception as e:
                pr.state = "error"
                pr.error = "Deze locatie kon niet worden gelezen."
                self.root_errors[r] = pr.error
                log("crawl %s mislukt: %s", r, e)

    def _t_segment(self, sd: SegData, pr: Progress):
        if sd.path not in self.cfg_roots and self.cfg_roots:
            pr.state = "done"
            self.jobs.pop(sd.path, None)
            return
        segdatas = []
        idx = self.idx
        for i, s in enumerate(idx.segs):
            if s.path == sd.path:
                continue
            segdatas.append(extract_segdata(idx, i))
        segdatas.append(sd)
        segdatas.sort(key=lambda d: (d.path != "/", d.path))
        new = Index.assemble(segdatas)
        new.gen = idx.gen + 1
        new.sgen = idx.sgen + 1
        with self.lock:
            self.idx = new
            self.results.clear()
        pr.state = "done"
        self.jobs.pop(sd.path, None)
        self.built_at = time.time() * 1000
        self.changed_pending = True
        self.dirty_persist = True
        if not any(j.state in ("pending", "building") for j in self.jobs.values()):
            self._persist()
            self.tasks.put(("warm",))
        del segdatas, idx, new
        trim_heap()
        self._setup_watches()
        self.emit_status(force=True)

    def _drop_segments(self, paths):
        idx = self.idx
        segdatas = [extract_segdata(idx, i) for i, s in enumerate(idx.segs) if s.path not in paths]
        new = Index.assemble(segdatas)
        new.gen = idx.gen + 1
        new.sgen = idx.sgen + 1
        with self.lock:
            self.idx = new
            self.results.clear()
        self.changed_pending = True
        self.dirty_persist = True
        if self.watcher:
            for p in paths:
                self.watcher.remove_prefix(p)
        del segdatas, idx, new
        if USE_MMAP and self.idx.segs:
            self._persist()          # → mapped from the files just written; the heap copy goes
        trim_heap()

    def _t_compact(self):
        """Fold the overlay and tombstones into a new base: in a helper process (mapped index);
        in this process only in heap mode, with WIN11SHELL_INDEX_FORK=0, or as a last resort past
        OVERLAY_HARD, and then only with room for the complete save that maps the result. When no
        fold can run (low disk, a failed or killed helper), the overlay stays and the fold waits
        FOLD_RETRY, doubling up to FOLD_RETRY_MAX: an in-process fold would cost ~1.2 GB here, and
        unsaved, its result would stay in the heap (~0.6 GB)."""
        idx = self.idx
        if idx.ov.live() == 0 and not any(s.dead for s in idx.segs):
            return
        t0 = time.monotonic()
        if not USE_MMAP:
            self._fold_here(t0)          # heap mode: the index lives in the heap anyway
            return
        if t0 < self.fold_retry_at:      # waiting after a fold that could not run
            if len(idx.ov) > OVERLAY_HARD and self._room_to_fold_here():
                self._fold_here(t0)
            return
        if helpers_on():
            res, why = self._compact_in_helper(t0)
            if res == "ok":
                self.fold_delay = FOLD_RETRY
                return
            if res == "again":           # the index changed meanwhile: the next compact task retries
                return
        elif _helpers_off:               # version skew / no code: until a restart
            why = (_helpers_off, None)
        elif self._room_to_fold_here():  # WIN11SHELL_INDEX_FORK=0
            self._fold_here(t0)
            return
        else:
            why = ("te weinig vrije schijfruimte om het resultaat op te slaan", None)
        self._fold_later(*why)
        if len(self.idx.ov) > OVERLAY_HARD and self._room_to_fold_here():
            self._fold_here(t0)

    def _fold_later(self, why: str, rc: int | None = None) -> None:
        hard = rc is not None and rc < 0 and rc != H_OFF          # killed by a signal, or timed out
        d = max(self.fold_delay, FOLD_RETRY_MAX) if hard else self.fold_delay
        self.fold_retry_at = time.monotonic() + d
        self.fold_delay = min(2 * d, FOLD_RETRY_MAX * (6 if hard else 1))
        log("samenvoegen uitgesteld (%.0f min; overlay %d): %s", d / 60, len(self.idx.ov), why)

    def _room_to_fold_here(self) -> bool:
        """Is there room for the complete save of a base folded in this process (names + 44 bytes
        per entry + SAVE_MARGIN)? Without it the fold's result could not be mapped."""
        idx = self.idx
        n = idx.N + len(idx.ov)
        need = 44 * n + sum(c.olen + c.llen for c in idx.chunks) + 2 * sum(len(b) for b in idx.ov.orig)
        try:
            os.makedirs(self.cache_dir, mode=0o700, exist_ok=True)
            return disk_free(self.cache_dir) - need >= SAVE_MARGIN
        except OSError:
            return False

    def _fold_here(self, t0: float) -> None:
        idx = self.idx
        if idx.ov.live() == 0 and not any(s.dead for s in idx.segs):
            return
        segdatas = [extract_segdata(idx, i) for i in range(len(idx.segs))]
        new = Index.assemble(segdatas)
        del segdatas
        with self.lock:
            if self.idx is not idx:
                return
            new.gen = idx.gen + 1
            new.sgen = idx.sgen + 1
            self.idx = new
            self.results.clear()
        self.dirty_persist = True
        log("index samengevoegd in %.1f s (%d items; in dit proces)", time.monotonic() - t0, new.N)
        del idx, new
        if USE_MMAP:
            self._persist()          # → mapped from the files just written; the heap copy goes
        trim_heap()                  # a fold leaves ~360 MiB of freed temporaries in the heap

    def _fold_guard(self) -> str | None:
        """Why a helper may not fold now (None = it may): the fold writes a complete second copy of
        the index (~0.6 GB here) before the old one goes, on a disk that may be nearly full."""
        home = os.path.join(self.cache_dir, "index")
        size = index_dir_bytes(home)
        need = MIN_FREE if MIN_FREE is not None else 2 * size
        try:
            free = disk_free(self.cache_dir)
        except OSError as e:                     # the cache dir is gone: the save that follows makes it
            return f"cachemap onleesbaar: {e}"
        if free < need:
            return (f"te weinig vrije schijfruimte voor een tweede kopie ({free / 1e9:.2f} GB vrij, "
                    f"{need / 1e9:.2f} GB nodig; WIN11SHELL_INDEX_MIN_FREE)")
        return None

    def _compact_in_helper(self, t0: float) -> tuple[str, tuple]:
        """Fold in a helper process: save the current state, let the helper fold that dir into a
        complete new one (with the size / date orders), install it and map it. The fold's ~1.2 GB
        of temporaries never enter this process. This runs on the writer thread, the only one that
        changes the index, so nothing changes meanwhile (checked through the generation).
        Returns ("ok", ()), ("again", ()) or ("failed", (why, helper exit code or None))."""
        why = self._fold_guard()
        if why:
            return "failed", (why, None)
        if self.dirty_persist or not self.idx.mapped:
            self._persist()                      # the helper folds what is on disk
        idx = self.idx
        if self.dirty_persist or not (idx.mapped and idx.home and idx.base_id):
            return "failed", ("de index kon niet worden opgeslagen", None)
        gen = idx.gen
        tmp = os.path.join(self.cache_dir, f"index.tmp-{os.getpid()}-fold")
        shutil.rmtree(tmp, ignore_errors=True)
        try:
            rc = run_helper(["fold", "--index", idx.home, "--base-id", idx.base_id, "--out", tmp],
                            timeout=HELPER_TIMEOUT)
            if rc != 0 or not _read_meta(tmp):
                shutil.rmtree(tmp, ignore_errors=True)
                return "failed", (f"hulpproces mislukt (code {rc})", rc)
            t1 = time.monotonic()
            with self.lock:
                if self.idx is not idx or idx.gen != gen:
                    log("index veranderde tijdens het samenvoegen; later opnieuw")
                    shutil.rmtree(tmp, ignore_errors=True)
                    return "again", ()
                install_index_dir(self.cache_dir, tmp)
                got = load_index(self.cache_dir)
                if not got:
                    raise ValueError("the folded index could not be loaded")
                new, _meta = got
                new.gen = idx.gen + 1
                new.sgen = idx.sgen + 1
                self.idx = new
                self.results.clear()
                self.dirty_persist = False
                self.last_persist = time.monotonic()
        except Exception as e:
            log("samenvoegen (hulpproces) mislukt:\n%s", traceback.format_exc())
            shutil.rmtree(tmp, ignore_errors=True)
            return "failed", (f"{type(e).__name__}: {e}", None)
        del idx
        trim_heap()
        log("index samengevoegd in %.1f s (%d items; hulpproces, geladen in %.2f s)",
            time.monotonic() - t0, self.idx.N, time.monotonic() - t1)
        return "ok", ()

    def _t_persist(self):
        self._persist()

    def _t_warm(self):
        """Pre-compute the size and date orders (used by size:/dm: and by sorting) in the background."""
        idx = self.idx
        for kind in ("size", "modified"):
            if self.stopping.is_set() or self.idx is not idx or not self.tasks.empty():
                return
            idx.get_order(kind)
            trim_heap()              # the sort's ~0.5 GB of int objects and keys

    def _persist(self):
        if not self.idx.segs:
            return
        t0 = time.monotonic()
        try:
            os.makedirs(self.cache_dir, mode=0o700, exist_ok=True)
            with self.lock:
                idx = self.idx
                prev = idx.base_id
                save_index(idx, self.cache_dir, self._meta_extra())
                self.dirty_persist = False
                self.last_persist = time.monotonic()
                base = os.path.join(self.cache_dir, "index")
                try:
                    if idx.mapped and prev == idx.base_id:
                        remap_index(idx, base)      # the base was hard-linked: map the new dir
                        self.results.clear()        # (cached results would keep the old maps alive)
                    elif USE_MMAP:
                        self._adopt_saved(idx)      # built in the heap (fold, crawl): map the new files
                    else:
                        idx.home = base             # heap mode: the next save links this base
                        for k in idx.orders:        # ... and the orders it just wrote
                            idx.order_files[k] = Index.order_file(k)
                except (OSError, ValueError) as e:
                    log("opgeslagen index opnieuw inlezen mislukt: %s", e)
                    if idx.mapped:                  # never keep running on the maps of removed files
                        self._adopt_saved(idx)
                self.idx.home_id = _dir_id(base)    # the dir in use now (see _check_home)
                self.home_lost = False
            del idx
            trim_heap()
            log("index opgeslagen in %.2f s", time.monotonic() - t0)
        except (OSError, ValueError) as e:
            log("index opslaan mislukt: %s", e)

    def _meta_extra(self) -> dict:
        return {"built": self.built_at, "excluded": list(self.cfg_excluded), "roots": list(self.cfg_roots)}

    def _adopt_saved(self, idx: Index) -> None:
        """After a save (under the lock): continue on the files just written, mapped, instead of
        the old maps (their files are gone: they would keep their disk space) or a heap copy."""
        got = load_index(self.cache_dir)
        if not got:
            raise ValueError("the saved index could not be loaded")
        new, _meta = got
        for k, o in idx.orders.items():      # same base: the sort orders stay valid
            if k not in new.orders:
                new.orders[k] = o
                if k in idx.odirty:
                    new.odirty[k] = idx.odirty[k]
        new.gen, new.sgen = idx.gen, idx.sgen
        if len(new.ov) != len(idx.ov):       # deleted overlay rows are not saved: overlay gids moved
            new.gen += 1
            new.sgen += 1
        self.idx = new
        self.results.clear()

    # -- exclusions ------------------------------------------------------------------
    def _prune_path(self, path: str):
        with self.lock:
            g = self.idx.lookup(path)
            if g is None:
                return
            n = self.idx.delete_subtree(g)
            if n:
                self.mark_changed()
        if self.watcher:
            self.watcher.remove_prefix(path)

    def _recrawl_path(self, path: str):
        parent = os.path.dirname(path.rstrip("/")) or "/"
        with self.lock:
            pg = self.idx.lookup(parent)
            if pg is None or self.idx.lookup(path) is not None:
                return
        self._add_path(pg, parent, os.path.basename(path.rstrip("/")))

    # -- live changes ----------------------------------------------------------------
    def _add_path(self, pg: int, dpath: str, name: str) -> int | None:
        """Add dpath/name (and, for a folder, everything below it) to the overlay."""
        full = join_path(dpath, name)
        ex = self.excludes_set()
        if self.is_excluded(full, ex):
            return None
        try:
            st = os.lstat(full)
        except OSError:
            return None
        idx = self.idx
        with self.lock:
            si = idx.seg_index(pg)
            g = idx.add_entry(pg, name, st, si)
            self.mark_changed()
        if _st.S_ISDIR(st.st_mode) and not idx.flags_of(g) & F_MOUNT:
            # watch it BEFORE listing it: files created in between were lost (1-2 of 60 quick
            # creates); an entry both listed and reported is found again by child_by_name
            if self.watcher and self.live and self._watchable(idx, g):
                self.watcher.add(full)
            self._crawl_overlay(g, full, si, ex)
        return g

    def _crawl_overlay(self, g0: int, path0: str, si: int, ex: set):
        idx = self.idx
        q = deque([(g0, path0)])
        added = 0
        while q:
            if self.stopping.is_set():
                return
            g, p = q.popleft()
            try:
                with os.scandir(p) as it:
                    entries = []
                    for e in it:
                        try:
                            entries.append((e.name, e.stat(follow_symlinks=False)))
                        except OSError:
                            continue
            except OSError:
                continue
            with self.lock:
                if self.idx is not idx:
                    return
                for name, st in entries:
                    full = join_path(p, name)
                    if _st.S_ISDIR(st.st_mode) and full in ex:
                        continue
                    c = idx.add_entry(g, name, st, si)
                    added += 1
                    if _st.S_ISDIR(st.st_mode) and not idx.flags_of(c) & F_MOUNT:
                        q.append((c, full))
                        if self.watcher and self.live and self._watchable(idx, c):
                            self.watcher.add(full)
                if entries:
                    self.mark_changed()
        return added

    def _watchable(self, idx: Index, g: int) -> bool:
        return not idx.flags_of(g) & (F_HIDDEN | F_LINK | F_MOUNT | F_DEL)

    def _t_events(self, batch):
        idx = self.idx
        changed = False
        rescan_all = False
        dir_cache: dict[str, int | None] = {}

        def resolve(dp):
            if dp not in dir_cache:
                dir_cache[dp] = idx.lookup(dp)
            return dir_cache[dp]

        for dpath, mask, name in batch:
            if self.stopping.is_set():
                return
            if mask & IN_Q_OVERFLOW:
                rescan_all = True
                continue
            if not name:
                continue   # events about the watched dir itself: the parent reports them
            if self.idx is not idx:
                idx = self.idx
                dir_cache.clear()
            pg = resolve(dpath)
            if pg is None:
                continue
            full = join_path(dpath, name)
            nb = store_name(name)
            if mask & (IN_DELETE | IN_MOVED_FROM):
                with self.lock:
                    g = idx.child_by_name(pg, nb)
                    if g is not None:
                        isdir = idx.flags_of(g) & F_DIR
                        idx.delete_subtree(g)
                        changed = True
                if g is not None and isdir and self.watcher:
                    self.watcher.remove_prefix(full)
                dir_cache.pop(full, None)
                continue
            try:
                st = os.lstat(full)
            except OSError:
                continue
            with self.lock:
                g = idx.child_by_name(pg, nb)
                if g is not None:
                    was_dir = bool(idx.flags_of(g) & F_DIR)
                    if was_dir != _st.S_ISDIR(st.st_mode):
                        idx.delete_subtree(g)
                        g = None
                    else:
                        if idx.update_stat(g, st):
                            changed = True
                        continue
            if mask & (IN_CREATE | IN_MOVED_TO | IN_CLOSE_WRITE | IN_ATTRIB):
                if self.is_excluded(full):
                    continue
                g = self._add_path(pg, dpath, name)
                if g is not None:
                    changed = True
                    if _st.S_ISDIR(st.st_mode) and self.watcher and self.live and self._watchable(idx, g):
                        self.watcher.add(full)
        if changed:
            with self.lock:
                self.mark_changed()
            self._maybe_compact()
        if rescan_all:
            self.rescan_include_watched = True
            self.rescan_now.set()

    def _maybe_compact(self):
        idx = self.idx
        dead = sum(s.dead for s in idx.segs)
        if len(idx.ov) > OVERLAY_COMPACT or (dead > 200_000 and dead > idx.N * 0.1):
            self.tasks.put(("compact",))

    # -- watches -----------------------------------------------------------------------
    def _setup_watches(self):
        if not self.live:
            if self.watcher:
                self.watcher.clear()
            return
        if self.watcher is None:
            self.watcher = Watcher(lambda batch: self.tasks.put(("events", batch)))
            self.watcher.start()
        if not self.watcher.ok:
            return
        home = os.path.realpath(os.path.expanduser("~"))
        with self.lock:
            idx = self.idx
            paths = []
            starts = []
            g = idx.lookup(home)
            if g is not None:
                starts.append(g)
            for s in idx.segs:
                # roots inside home (e.g. only ~/Documenten indexed) and folders chosen by hand
                # (not drives/mount points) are watched too; drives get periodic rescans
                if g is not None and (s.path == home or s.path.startswith(home + "/")):
                    continue
                if s.path == home or s.path.startswith(home + "/") or (s.path != "/" and not os.path.ismount(s.path)):
                    starts.append(s.start)
            budget = self.watcher.refresh_budget()
            level = [g for g in starts if self._watchable(idx, g) or idx.flags_of(g) & F_ROOT]
            while level and len(paths) < budget:
                level.sort(key=idx.mtime_of, reverse=True)
                nxt = []
                for d in level:
                    if len(paths) >= budget:
                        break
                    paths.append(idx.dir_path(d))
                    for c in idx.kids(d):
                        if idx.flags_of(c) & F_DIR and self._watchable(idx, c):
                            nxt.append(c)
                level = nxt
        self.watcher.set_paths(paths)
        log("live bijwerken: %d mappen bewaakt (budget %d)", self.watcher.count(), self.watcher.budget)

    # -- periodic rescans ----------------------------------------------------------------
    def _rescanner(self):
        self.rescan_include_watched = False
        while not self.stopping.is_set():
            self.rescan_now.wait(float(os.environ.get("WIN11SHELL_INDEX_RESCAN") or RESCAN_INTERVAL))
            self.rescan_now.clear()
            if self.stopping.is_set():
                return
            if any(j.state in ("pending", "building") for j in self.jobs.values()):
                continue
            include_watched = self.rescan_include_watched or not self._did_first
            self.rescan_include_watched = False
            try:
                self.rescanning = True
                self.emit_status(force=True)
                self._rescan_pass(include_watched)
            except Exception:
                log("rescan faalde:\n%s", traceback.format_exc())
            finally:
                self.rescanning = False
                self._did_first = True
                self.emit_status(force=True)

    _did_first = False

    def _rescan_pass(self, include_watched: bool):
        """Stat every indexed folder (in batches, without holding the lock during I/O) and queue
        a re-listing for each folder whose mtime changed. Watched folders are live already."""
        idx = self.idx
        w = self.watcher
        changed = 0
        BATCH = 20_000

        def handle(batch):
            nonlocal changed
            for k, (p, mt) in enumerate(batch):
                if self.stopping.is_set() or self.idx is not idx:
                    return False
                if not include_watched and w is not None and w.watched(p):
                    continue
                try:
                    st = os.stat(p)
                except OSError:
                    self.tasks.put(("sync", p))
                    changed += 1
                    continue
                if st.st_mtime_ns // 1_000_000 != mt:
                    self.tasks.put(("sync", p))
                    changed += 1
                if not k & 0x3FF:
                    time.sleep(0)
            return True

        # folder paths in a sliding window instead of the shared cache (a pass used to fill it with
        # ~400k paths, +94 MiB): base folders come in BFS order, so a later folder's parent is never
        # below the parent of the first folder of the next batch
        win: OrderedDict[int, str] = OrderedDict()
        for lo in range(0, idx.N, BATCH):
            with self.lock:
                if self.idx is not idx:
                    return
                hi = min(idx.N, lo + BATCH)
                fl = idx.flags
                batch = [(idx.dir_path(g, win), idx.mtime[g])
                         for g in compress(range(lo, hi), fl[lo:hi].translate(T_ADIR))
                         if not fl[g] & (F_MOUNT | F_LINK)]
                cut = idx.parent[hi] if hi < idx.N else idx.N
                if cut < 0:              # the next batch starts a new segment
                    win.clear()
                else:
                    while win:
                        k = next(iter(win))
                        if k >= cut:
                            break
                        win.popitem(last=False)
            if not handle(batch):
                return
        win.clear()
        with self.lock:
            if self.idx is not idx:
                return
            ov = idx.ov
            batch = [(idx.dir_path(ov.base + k, win), ov.mtime[k]) for k in range(len(ov))
                     if ov.flags[k] & F_DIR and not ov.flags[k] & (F_DEL | F_MOUNT | F_LINK)]
        win.clear()
        handle(batch)
        del batch
        trim_heap()
        if changed:
            log("rescan: %d mappen veranderd", changed)

    def _t_sync(self, path: str):
        self._sync_dir(path)

    def _sync_dir(self, path: str):
        idx = self.idx
        with self.lock:
            g = idx.lookup(path)
        if g is None:
            return
        try:
            st = os.stat(path)
            with os.scandir(path) as it:
                listing = {}
                for e in it:
                    try:
                        listing[e.name] = e.stat(follow_symlinks=False)
                    except OSError:
                        continue
        except OSError:
            # gone: the parent's sync removes it
            parent = os.path.dirname(path)
            if parent and parent != path:
                self._sync_dir_gone(path)
            return
        ex = self.excludes_set()
        changed = False
        new_dirs = []
        with self.lock:
            if self.idx is not idx:
                return
            kids = {idx.name(c): c for c in idx.kids(g)}
            for name, c in kids.items():
                est = listing.get(name)
                if est is None or bool(idx.flags_of(c) & F_DIR) != _st.S_ISDIR(est.st_mode):
                    idx.delete_subtree(c)
                    changed = True
                elif idx.update_stat(c, est):
                    changed = True
            for name, est in listing.items():
                if name in kids and not idx.flags_of(kids[name]) & F_DEL:
                    continue
                full = join_path(path, name)
                if _st.S_ISDIR(est.st_mode) and self.is_excluded(full, ex):
                    continue
                si = idx.seg_index(g)
                c = idx.add_entry(g, name, est, si)
                changed = True
                if _st.S_ISDIR(est.st_mode) and not idx.flags_of(c) & F_MOUNT:
                    new_dirs.append((c, full, si))
            idx.update_stat(g, st)
            if changed:
                self.mark_changed()
        for c, full, si in new_dirs:
            self._crawl_overlay(c, full, si, ex)
        if changed:
            self._maybe_compact()

    def _sync_dir_gone(self, path: str):
        with self.lock:
            g = self.idx.lookup(path)
            if g is not None and not self.idx.flags_of(g) & F_ROOT:
                self.idx.delete_subtree(g)
                self.mark_changed()

    # -- search ------------------------------------------------------------------------
    def search(self, args: dict, cancel=None) -> dict:
        t0 = time.perf_counter()
        query = str(args.get("query") or "")
        try:
            offset = max(0, int(args.get("offset") or 0))
            limit = max(0, min(5000, int(args.get("limit") if args.get("limit") is not None else 200)))
        except (TypeError, ValueError):
            raise QueryError("Ongeldige pagina.") from None
        sort = str(args.get("sort") or "name")
        if sort not in ("name", "path", "size", "modified", "type"):
            sort = "name"
        desc = bool(args.get("desc"))
        flt = str(args.get("filter") or "all")
        case = bool(args.get("matchCase"))
        path = bool(args.get("matchPath"))
        regex = bool(args.get("regex"))
        whole = bool(args.get("wholeWord"))
        show_hidden = bool(args.get("showHidden"))
        key = (query, flt, case, path, regex, whole, show_hidden, sort, desc)
        comp = compile_query(query, case=case, path=path, whole=whole, regex=regex, flt=flt)
        with self.lock:
            idx = self.idx
            res = None if args.get("nocache") else self.results.get(key)
            if res is not None and res.gen == idx.gen:
                self.results.move_to_end(key)
            else:
                res = self._evaluate(idx, comp, sort, desc, show_hidden, cancel)
                self.results[key] = res
                while len(self.results) > 6:
                    self.results.popitem(last=False)
                big = [k for k, r in self.results.items() if r.base_kind == "M" or len(r.base) > LIST_MAX]
                for k in big[:max(0, len(big) - RESULT_MASKS)]:      # each holds an N-byte mask
                    del self.results[k]
            gids = res.page(offset, limit)
            items = [self._item(idx, g) for g in gids]
            total = res.total
            if len(idx._dpath) > DPATH_KEEP:
                idx._dpath.clear()       # ~250 B per path; the next search refills what it needs
        took = (time.perf_counter() - t0) * 1000
        return {"total": total, "items": items, "tookMs": round(took, 1), "offset": offset,
                "state": self.state(), "generation": idx.gen,
                "highlight": {"terms": comp.highlight, "regex": comp.regex, "case": comp.case}}

    def _evaluate(self, idx: Index, comp: Compiled, sort: str, desc: bool, show_hidden: bool, cancel) -> Result:
        cx = Ctx(idx, cancel, show_hidden)
        vis_table = T_ALIVE if show_hidden else T_VIS
        atom = comp.atom if comp.atom is not None else AllAtom()
        vis = FlagAtom(vis_table)
        if isinstance(atom, AndNode):
            root = AndNode(atom.pos + [vis], atom.neg)
        else:
            root = AndNode([atom, vis], [])
        kind, data = root.evaluate(cx) if idx.N else ("L", [])
        # overlay entries (live additions) are tested one by one
        ov = idx.ov
        extras = []
        if len(ov):
            base = ov.base
            test = root.test
            for k in range(len(ov)):
                if vis_table[ov.flags[k]] and test(cx, base + k):
                    extras.append(base + k)
        keyf = idx.sort_key(sort)
        if kind == "L" and len(data) > 20_000:
            kind, data = "M", ids_to_mask(data, idx.N)    # page straight from the sorted order
        if kind == "L":
            lst = data
            lst.sort(key=idx.rank.__getitem__, reverse=desc and sort == "name")
            if sort in ("size", "modified"):
                arr = idx.size if sort == "size" else idx.mtime
                lst.sort(key=arr.__getitem__, reverse=desc)
            elif sort in ("path", "type"):
                lst.sort(key=idx.key_array(sort).__getitem__, reverse=desc)
            extras.sort(key=keyf, reverse=desc)
            return Result(idx.gen, len(lst) + len(extras), "L", lst, extras=extras, keyf=keyf, reverse=desc)
        mask = data
        order = idx.get_order(sort) if sort != "name" else idx.order
        dirty = idx.odirty.get(sort) if sort in ("size", "modified") else None
        if dirty:
            dl = [g for g in dirty if g < idx.N and mask[g]]
            if dl:
                for g in dl:
                    mask[g] = 0
                extras.extend(dl)
        extras.sort(key=keyf, reverse=desc)
        return Result(idx.gen, mask.count(1) + len(extras), "M", mask, order=order, reverse=desc,
                      extras=extras, keyf=keyf)

    def _item(self, idx: Index, g: int) -> dict:
        fl = idx.flags_of(g)
        isdir = bool(fl & F_DIR)
        par = idx.parent_of(g)
        name = idx.name(g)
        if par < 0:
            path = idx.seg_of(g).path
            d = os.path.dirname(path.rstrip("/")) if path != "/" else ""
            name = os.path.basename(path.rstrip("/")) or path
        else:
            d = idx.dir_path(par)
            path = join_path(d, name)
        ext = ext_of(name, isdir)
        if isdir:
            mime = "inode/directory"
        else:
            mime = guess_mime(name, ext, fl)
        return {"path": path, "name": name, "dir": d, "isDir": isdir, "size": idx.size_of(g),
                "modified": idx.mtime_of(g), "ext": ext, "mime": mime, "hidden": bool(fl & F_HIDDEN),
                "isSymlink": bool(fl & F_LINK)}


_MIME_EXTRA = {"desktop": "application/x-desktop", "appimage": "application/vnd.appimage", "sh": "application/x-shellscript",
               "md": "text/markdown", "mkv": "video/x-matroska", "flac": "audio/flac", "opus": "audio/ogg",
               "webp": "image/webp", "heic": "image/heic", "avif": "image/avif", "7z": "application/x-7z-compressed",
               "rar": "application/vnd.rar", "deb": "application/vnd.debian.binary-package", "iso": "application/x-cd-image",
               "py": "text/x-python", "js": "text/javascript", "json": "application/json", "log": "text/plain",
               "kdbx": "application/x-keepass2", "torrent": "application/x-bittorrent", "epub": "application/epub+zip",
               "m4a": "audio/mp4", "mts": "video/mp2t", "ts": "video/mp2t", "odt": "application/vnd.oasis.opendocument.text"}
_mime_ready = False


def guess_mime(name: str, ext: str, fl: int) -> str:
    global _mime_ready
    if fl & F_LINK and not ext:
        return "inode/symlink"
    if ext in _MIME_EXTRA:
        return _MIME_EXTRA[ext]
    if ext:
        if not _mime_ready:
            try:
                mimetypes.init()
            except Exception:
                pass
            _mime_ready = True
        m = mimetypes.types_map.get("." + ext) or mimetypes.common_types.get("." + ext)
        if m:
            return m
    if fl & F_EXEC:
        return "application/x-executable"
    return "application/octet-stream"


# ---------------------------------------------------------------------------
# stdio server
# ---------------------------------------------------------------------------

def set_low_priority():
    try:
        os.nice(10)
    except OSError:
        pass
    try:
        nr = {"x86_64": 251, "aarch64": 30, "i686": 289, "i386": 289, "armv7l": 314, "riscv64": 30}.get(platform.machine())
        if nr:
            libc = ctypes.CDLL(None, use_errno=True)
            # IOPRIO_WHO_PROCESS=1, this process, best-effort class (2) lowest level (7)
            libc.syscall(nr, 1, 0, (2 << 13) | 7)
    except Exception:
        pass


class StdioServer:
    def __init__(self, engine_factory, out_fd: int):
        self.out = os.fdopen(out_fd, "wb", buffering=0)
        self.out_lock = threading.Lock()
        self.engine = engine_factory(self.emit)
        self.sq: deque = deque()
        self.cv = threading.Condition()
        self.latest: dict[str, tuple] = {}
        self.worker = threading.Thread(target=self._search_worker, name="search", daemon=True)

    def write(self, obj):
        data = (json.dumps(obj, ensure_ascii=True, separators=(",", ":"), default=str) + "\n").encode()
        with self.out_lock:
            try:
                self.out.write(data)
            except (BrokenPipeError, OSError):
                os._exit(0)

    def emit(self, event, data):
        self.write({"event": event, "data": data})

    def reply(self, rid, result=None, error=None):
        if rid is None:
            return
        if error is not None:
            self.write({"id": rid, "ok": False, "error": error})
        else:
            self.write({"id": rid, "ok": True, "result": result})

    @staticmethod
    def qkey(args):
        return tuple(str(args.get(k)) for k in ("query", "filter", "matchCase", "matchPath", "regex", "wholeWord",
                                                "showHidden", "sort", "desc"))

    def _search_worker(self):
        while True:
            with self.cv:
                while not self.sq:
                    self.cv.wait()
                rid, args = self.sq.popleft()
            ch = str(args.get("channel") or "default")
            mykey = self.qkey(args)
            if self.latest.get(ch, (None, mykey))[1] != mykey:
                self.reply(rid, error={"code": "cancelled", "message": "Vervangen door een nieuwere zoekopdracht."})
                continue
            cancel = lambda: self.latest.get(ch, (None, mykey))[1] != mykey  # noqa: E731
            try:
                res = self.engine.search(args, cancel)
                self.reply(rid, res)
                if res.get("tookMs", 0) > 20:
                    trim_heap()          # after the reply: the masks / candidate lists of this search
            except Cancelled:
                self.reply(rid, error={"code": "cancelled", "message": "Vervangen door een nieuwere zoekopdracht."})
            except QueryError as e:
                self.reply(rid, error={"code": e.code, "message": e.message})
            except Exception as e:
                log("zoeken faalde:\n%s", traceback.format_exc())
                self.reply(rid, error={"code": "internal", "message": "Er ging iets mis bij het zoeken.",
                                       "detail": f"{type(e).__name__}: {e}"[:300]})

    def handle(self, req: dict):
        rid = req.get("id")
        m = req.get("method")
        args = req.get("args") or {}
        if not isinstance(args, dict):
            args = {}
        try:
            if m == "search":
                ch = str(args.get("channel") or "default")
                with self.cv:
                    self.latest[ch] = (rid, self.qkey(args))
                    self.sq.append((rid, args))
                    self.cv.notify()
            elif m == "status":
                self.reply(rid, self.engine.status())
            elif m == "configure":
                self.engine.configure(args)
                self.reply(rid, {"ok": True})
            elif m == "rebuild":
                self.engine.rebuild()
                self.reply(rid, {"ok": True})
            elif m == "compact":
                self.engine.tasks.put(("compact",))
                self.reply(rid, {"ok": True})
            elif m == "persist":
                self.engine.tasks.put(("persist",))
                self.reply(rid, {"ok": True})
            elif m == "ping":
                self.reply(rid, {"pong": True, "pid": os.getpid()})
            elif m == "autoRoots":
                self.reply(rid, {"roots": auto_roots()})
            elif m == "shutdown":
                self.reply(rid, {"ok": True})
                return False
            else:
                self.reply(rid, error={"code": "unknown_method", "message": "Onbekende opdracht."})
        except Exception as e:
            log("verzoek %s faalde:\n%s", m, traceback.format_exc())
            self.reply(rid, error={"code": "internal", "message": "Er ging iets mis.", "detail": str(e)[:300]})
        return True

    def serve(self, initial_config: dict | None):
        self.worker.start()
        self.engine.start()
        if initial_config is not None:
            self.engine.configure(initial_config)
        stdin = sys.stdin.buffer
        for line in stdin:
            line = line.strip()
            if not line:
                continue
            try:
                req = json.loads(line)
            except ValueError:
                continue
            if not isinstance(req, dict):
                continue
            if self.handle(req) is False:
                break
        self.engine.stop()


def _die_with_parent(parent: int) -> None:
    """A helper never outlives the indexer that started it (killed by SIGKILL when it dies)."""
    try:
        libc = ctypes.CDLL(None, use_errno=True)
        libc.prctl(1, 9, 0, 0, 0)               # PR_SET_PDEATHSIG, SIGKILL
    except Exception:
        pass
    if parent and os.getppid() != parent:       # it died before the prctl
        os._exit(5)


def prefer_oom_kill(score: int) -> None:
    """Ask the kernel to pick this process first when memory runs out (raising oom_score_adj needs no
    privileges). Without it a 0.6-1.2 GiB helper on a full machine could make the OOM killer take the
    biggest process instead, e.g. the user's virtual machine. Best effort."""
    try:
        with open("/proc/self/oom_score_adj", "w") as f:
            f.write(str(score))
    except OSError:
        pass


def helper_main(argv) -> int:
    """``python3 -m host.indexer --helper order|fold ...``: one heavy job on a persisted index dir,
    started by the indexer (run_helper). Writes only new files; the indexer installs them.

    order --index DIR --base-id ID --targets K[,K] --suffix S [--values FILE]
        build the sort orders / keys K of DIR's base, write DIR/<file>S for each; with --values
        the one size or date order K sorts the column in FILE (the parent's live values)
    fold --index DIR --base-id ID --out NEW
        fold DIR's overlay and tombstones into a fresh base (+ size and date orders), write it as
        the complete index dir NEW
    Both take --proto N (HELPER_PROTO of the parent: another value exits 6) and --parent PID."""
    import argparse
    global FORK_OK
    FORK_OK = False                             # never a helper of a helper
    ap = argparse.ArgumentParser(prog="python3 -m host.indexer --helper")
    ap.add_argument("job", choices=("order", "fold"))
    ap.add_argument("--index", required=True)
    ap.add_argument("--base-id", required=True)
    ap.add_argument("--parent", type=int, default=0)
    ap.add_argument("--targets", default="")
    ap.add_argument("--suffix", default="")
    ap.add_argument("--out", default="")
    ap.add_argument("--values", default="")
    ap.add_argument("--proto", type=int, default=0)
    a = ap.parse_args(argv)
    _die_with_parent(a.parent)
    prefer_oom_kill(1000)                       # a helper is always the right victim; the indexer retries later
    if a.proto != HELPER_PROTO:
        log("hulpproces: protocol %s, deze code heeft %s", a.proto, HELPER_PROTO)
        return H_PROTO_RC
    t0 = time.monotonic()
    got = load_index_dir(a.index, heap=False)
    if not got:
        return 3
    idx, meta = got
    if idx.base_id != a.base_id:
        log("hulpproces: %s is intussen veranderd", a.index)
        return 4
    if a.job == "order":
        targets = [t for t in a.targets.split(",") if t]
        if a.values:
            if len(targets) != 1 or targets[0] not in ("size", "modified"):
                return 2
            col = _read_array(a.values, "q", idx.N)
            if targets[0] == "size":
                idx.size = col
            else:
                idx.mtime = col
        for t in targets:
            kind = t[4:] if t.startswith("key:") else t
            if kind not in ORDER_KINDS or not a.suffix:
                return 2
            arr = idx._compute(t)
            idx.orders[t] = arr                  # an order of path/type uses the key built just before
            _write_array(os.path.join(a.index, Index.order_file(t)) + a.suffix, arr)
            log("sorteervolgorde %s gebouwd in %.1f s (hulpproces)", t, time.monotonic() - t0)
        return 0
    # fold
    if not a.out:
        return 2
    segdatas = [extract_segdata(idx, i) for i in range(len(idx.segs))]
    new = Index.assemble(segdatas)
    del segdatas
    for kind in ("size", "modified"):           # the orders of the old base do not fit the new one
        new.orders[kind] = new._compute(kind)
        new.odirty[kind] = set()
    extra = {k: meta[k] for k in ("built", "excluded", "roots") if k in meta}
    try:
        write_index_dir(new, a.out, extra)
    except BaseException:
        shutil.rmtree(a.out, ignore_errors=True)
        raise
    log("samengevoegd in %.1f s (hulpproces, %d items)", time.monotonic() - t0, new.N)
    return 0


def main(argv=None):
    import argparse
    argv = sys.argv[1:] if argv is None else list(argv)
    if argv[:1] == ["--helper"]:
        return helper_main(argv[1:])
    ap = argparse.ArgumentParser(prog="python3 -m host.indexer", description="Alles file name indexer")
    ap.add_argument("--cache", required=True, help="cache directory (index stored in <cache>/index)")
    ap.add_argument("--roots", nargs="*", default=None, help="roots to index (default: wait for configure)")
    ap.add_argument("--exclude", nargs="*", default=[], help="excluded folders")
    ap.add_argument("--no-live", action="store_true", help="no inotify live updates")
    ap.add_argument("--no-nice", action="store_true", help="keep normal CPU/IO priority")
    ap.add_argument("--auto-roots", action="store_true", help="print the default roots and exit")
    a = ap.parse_args(argv)
    if a.auto_roots:
        print(json.dumps(auto_roots(), indent=1))
        return 0
    if not a.no_nice:
        set_low_priority()
    prefer_oom_kill(300)                        # the host restarts a killed indexer; the user's apps come first
    # keep stdout for the protocol only; stray prints go to stderr
    out_fd = os.dup(1)
    os.dup2(2, 1)
    cache = os.path.abspath(os.path.expanduser(a.cache))
    os.makedirs(cache, mode=0o700, exist_ok=True)
    cfg = None
    if a.roots is not None:
        cfg = {"roots": a.roots or "auto", "excluded": a.exclude, "liveUpdate": not a.no_live}
    srv = StdioServer(lambda emit: Engine(cache, emit), out_fd)
    try:
        srv.serve(cfg)
    except KeyboardInterrupt:
        srv.engine.stop()
    return 0


if __name__ == "__main__":
    sys.exit(main())
