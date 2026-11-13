"""storage.* (SPEC §6.12): volumes, folder-size scans, storage categories, cleanup and large files.

* ``storage.volumes``     /proc/self/mountinfo + os.statvfs + lsblk (labels, models, transport). Pseudo,
                          snap/loop squashfs, docker overlays, bind duplicates, network mounts and the
                          EFI/boot partitions are left out (``{all:true}`` adds the boot ones with ``hidden``).
* ``storage.scan``        background thread (nice +10, idle I/O class), os.scandir, allocated size
                          (st_blocks × 512, hard links counted once), never crosses filesystems, progress events
                          every ~250 ms, cancellable, result cached per (path, maxDepth, topN). The tree lists the
                          top N children per folder plus one "Overige" aggregate; sizes are always complete.
* ``storage.categories``  one background walk over the home folder attributing every byte to a category.
* ``storage.cleanup.*``   only well-known, regenerable data; admin steps run together in ONE pkexec prompt.
* ``storage.largeFiles``  from the last scan that covered the path, or a bounded walk.

Additive (notes/host-services-b.md): ``storage.scanResult {jobId}``, ``storage.trashSize {}``,
event ``storage.volumesChanged {volumes}``, event ``storage.categoriesProgress {files, bytes}``.

``storage.scanDone`` is a small event ``{jobId, done:true, path, bytes, files, tookMs, error?, errorCode?}``: a tree
can be megabytes and the event goes to every connected page, so the UI fetches it with ``storage.scanResult {jobId}``.
Small trees (≤ 16 KiB JSON) are still included as ``tree`` (no extra round trip; older UIs keep working).
"""

from __future__ import annotations

import ctypes
import heapq
import itertools
import json
import logging
import os
import platform
import re
import select
import shutil
import stat
import threading
import time
from pathlib import Path
from typing import Any, Callable

from ..util import ApiError, atomic_write_json, first_existing_icon, json_dumps, mime_icon_names, read_json, run, \
    which, xdg_user_dir

log = logging.getLogger("win11shell.storage")

PRIORITY = 60
UID = os.getuid()

_lang: Callable[[], str] = lambda: "nl"

_STR = {
    "nl": {
        "system": "Systeemschijf", "home": "Persoonlijke bestanden", "vm": "Virtuele machines",
        "sized": "{size}-schijf", "other": "Overige",
        "c_documents": "Documenten", "c_pictures": "Afbeeldingen", "c_videos": "Video's", "c_music": "Muziek",
        "c_downloads": "Downloads", "c_desktop": "Bureaublad", "c_trash": "Prullenbak", "c_browser": "Browsers",
        "c_chat": "Chat-apps", "c_dev": "Ontwikkelaarsbestanden", "c_appdata": "App-gegevens",
        "c_snaps": "Snap-apps", "c_flatpak": "Flatpak-apps", "c_cache": "Tijdelijke bestanden", "c_other": "Overig",
        "k_trash": "Prullenbak",
        "d_trash": "Bestanden die je hebt verwijderd. Na het legen zijn ze echt weg.",
        "k_thumbnails": "Miniaturen",
        "d_thumbnails": "Kleine voorbeeldplaatjes van je foto's en video's. Die worden vanzelf opnieuw gemaakt.",
        "k_user-cache-safe": "Tijdelijke app-bestanden",
        "d_user-cache-safe": "Caches die apps zelf opnieuw aanmaken, zoals shader- en downloadcaches.",
        "k_browser-cache": "Tijdelijke internetbestanden",
        "d_browser-cache": "De cache van je browsers. Wachtwoorden, bladwijzers en geschiedenis blijven bewaard.",
        "d_browser-running": " Sluit eerst {names}, anders wordt die overgeslagen.",
        "k_old-downloads": "Oude downloads",
        "d_old-downloads": "Bestanden in Downloads die je al meer dan 90 dagen niet hebt geopend. "
                           "Bekijk ze eerst: ze gaan naar de prullenbak.",
        "k_apt-cache": "Opgeslagen updatebestanden",
        "d_apt-cache": "Installatiebestanden van updates die al zijn geïnstalleerd. Vraagt om je wachtwoord.",
        "k_old-snaps": "Oude versies van Snap-apps",
        "d_old-snaps": "Eerdere versies die Snap bewaart om terug te kunnen gaan. Vraagt om je wachtwoord.",
        "k_journal": "Oude systeemlogboeken",
        "d_journal": "Logboeken van meer dan 14 dagen oud. Alleen nodig om oude problemen te onderzoeken. "
                     "Vraagt om je wachtwoord.",
        "k_flatpak-unused": "Ongebruikte Flatpak-onderdelen",
        "d_flatpak-unused": "Onderdelen die geen enkele Flatpak-app meer gebruikt.",
        "k_crash-reports": "Foutrapporten",
        "d_crash-reports": "Rapporten van apps die zijn vastgelopen.",
        "b_firefox": "Firefox", "b_zen": "Zen", "b_chrome": "Google Chrome", "b_chromium": "Chromium",
        "b_brave": "Brave", "b_vivaldi": "Vivaldi", "b_edge": "Microsoft Edge", "b_librewolf": "LibreWolf",
        "b_opera": "Opera",
    },
    "en": {
        "system": "System drive", "home": "Personal files", "vm": "Virtual machines",
        "sized": "{size} Drive", "other": "Other",
        "c_documents": "Documents", "c_pictures": "Pictures", "c_videos": "Videos", "c_music": "Music",
        "c_downloads": "Downloads", "c_desktop": "Desktop", "c_trash": "Recycle Bin", "c_browser": "Browsers",
        "c_chat": "Chat apps", "c_dev": "Developer files", "c_appdata": "App data", "c_snaps": "Snap apps",
        "c_flatpak": "Flatpak apps", "c_cache": "Temporary files", "c_other": "Other",
        "k_trash": "Recycle Bin",
        "d_trash": "Files you deleted. Once emptied they are gone for good.",
        "k_thumbnails": "Thumbnails",
        "d_thumbnails": "Small previews of your photos and videos. They are made again automatically.",
        "k_user-cache-safe": "Temporary app files",
        "d_user-cache-safe": "Caches that apps rebuild by themselves, such as shader and download caches.",
        "k_browser-cache": "Temporary Internet Files",
        "d_browser-cache": "Your browsers' cache. Passwords, bookmarks and history are kept.",
        "d_browser-running": " Close {names} first, otherwise it is skipped.",
        "k_old-downloads": "Old downloads",
        "d_old-downloads": "Files in Downloads you have not opened for more than 90 days. "
                           "Review them first: they go to the Recycle Bin.",
        "k_apt-cache": "Saved update files",
        "d_apt-cache": "Installation files of updates that are already installed. Asks for your password.",
        "k_old-snaps": "Old versions of Snap apps",
        "d_old-snaps": "Earlier versions Snap keeps so it can roll back. Asks for your password.",
        "k_journal": "Old system logs",
        "d_journal": "Logs older than 14 days. Only needed to investigate old problems. Asks for your password.",
        "k_flatpak-unused": "Unused Flatpak parts",
        "d_flatpak-unused": "Parts that no Flatpak app uses any more.",
        "k_crash-reports": "Error reports",
        "d_crash-reports": "Reports of apps that crashed.",
        "b_firefox": "Firefox", "b_zen": "Zen", "b_chrome": "Google Chrome", "b_chromium": "Chromium",
        "b_brave": "Brave", "b_vivaldi": "Vivaldi", "b_edge": "Microsoft Edge", "b_librewolf": "LibreWolf",
        "b_opera": "Opera",
    },
}


def _tr(key: str, **kw) -> str:
    table = _STR.get(_lang(), _STR["nl"])
    text = table.get(key) or _STR["nl"].get(key) or key
    try:
        return text.format(**kw) if kw else text
    except (KeyError, IndexError):
        return text


def fmt_size_short(n: int) -> str:
    """Rounded drive size like Windows: 563 GB, 1,8 TB, 22 TB (binary units)."""
    units = ["KB", "MB", "GB", "TB", "PB"]
    v = float(n)
    i = -1
    while v >= 1024 and i < len(units) - 1:
        v /= 1024
        i += 1
    if i < 0:
        return f"{int(v)} bytes"
    s = f"{v:.0f}" if v >= 10 else f"{v:.1f}"
    if _lang() != "en":
        s = s.replace(".", ",")
    return f"{s} {units[i]}"


# ---------------------------------------------------------------------------
# fast size helpers
# ---------------------------------------------------------------------------

_PSEUDO_ROOTS = ("/proc", "/sys", "/dev", "/run")
_PSEUDO_SET = frozenset(_PSEUDO_ROOTS)


def du(path: str | os.PathLike, deadline: float | None = None, same_fs: bool = True) -> tuple[int, int]:
    """(allocated bytes, files) of a file or tree. Never follows symlinks, never crosses filesystems."""
    path = os.fspath(path)
    try:
        st = os.lstat(path)
    except OSError:
        return 0, 0
    if not stat.S_ISDIR(st.st_mode):
        return st.st_blocks * 512, 1
    dev = st.st_dev
    total = st.st_blocks * 512
    files = 0
    stack = [path]
    seen: set[tuple[int, int]] = set()
    n = 0
    while stack:
        d = stack.pop()
        try:
            it = os.scandir(d)
        except OSError:
            continue
        with it:
            for e in it:
                try:
                    s = e.stat(follow_symlinks=False)
                except OSError:
                    continue
                if stat.S_ISDIR(s.st_mode):
                    if same_fs and s.st_dev != dev:
                        continue
                    total += s.st_blocks * 512
                    stack.append(e.path)
                else:
                    if s.st_nlink > 1:
                        k = (s.st_dev, s.st_ino)
                        if k in seen:
                            continue
                        seen.add(k)
                    total += s.st_blocks * 512
                    files += 1
                n += 1
                if deadline is not None and n % 4096 == 0 and time.monotonic() > deadline:
                    return total, files
    return total, files


def _clear_dir_contents(path: str, keep_dirs: bool = False) -> list[str]:
    """Delete everything INSIDE ``path`` (not ``path`` itself). Symlinks are removed, never followed.
    Returns errors (paths that could not be removed)."""
    errors: list[str] = []
    try:
        st = os.lstat(path)
    except OSError:
        return errors
    if not stat.S_ISDIR(st.st_mode):
        return errors
    try:
        entries = list(os.scandir(path))
    except OSError:
        return [path]
    for e in entries:
        try:
            if e.is_dir(follow_symlinks=False):
                if keep_dirs:
                    errors += _clear_dir_contents(e.path, keep_dirs=False)
                else:
                    shutil.rmtree(e.path, onexc=lambda *_a: errors.append(e.path))
            else:
                os.unlink(e.path)
        except OSError:
            errors.append(e.path)
    return errors


def _set_scan_priority() -> None:
    """Lower CPU (nice +10) and I/O (idle class) priority of the *current thread* (Linux is per-thread)."""
    tid = threading.get_native_id()
    try:
        os.setpriority(os.PRIO_PROCESS, tid, 10)
    except (OSError, AttributeError):
        pass
    nr = {"x86_64": 251, "aarch64": 30, "riscv64": 30, "i686": 289, "i386": 289, "armv7l": 314}.get(
        platform.machine())
    if nr is None:
        return
    try:
        libc = ctypes.CDLL(None, use_errno=True)
        libc.syscall(nr, 1, tid, (3 << 13))  # IOPRIO_WHO_PROCESS, IOPRIO_CLASS_IDLE
    except Exception:  # noqa: BLE001
        pass


# ---------------------------------------------------------------------------
# volumes
# ---------------------------------------------------------------------------

_REAL_FS = {"ext2", "ext3", "ext4", "xfs", "btrfs", "f2fs", "vfat", "msdos", "exfat", "ntfs", "ntfs3", "fuseblk",
            "jfs", "reiserfs", "hfsplus", "hfs", "udf", "iso9660", "zfs", "bcachefs", "nilfs2", "apfs"}
_FS_LABEL = {"vfat": "FAT32", "msdos": "FAT", "exfat": "exFAT", "ntfs": "NTFS", "ntfs3": "NTFS",
             "fuseblk": "NTFS", "ext4": "ext4", "ext3": "ext3", "ext2": "ext2", "btrfs": "Btrfs", "xfs": "XFS",
             "f2fs": "F2FS", "zfs": "ZFS", "iso9660": "CDFS", "udf": "UDF", "hfsplus": "HFS+", "bcachefs": "bcachefs"}
_SKIP_MOUNT = ("/snap/", "/var/snap/", "/var/lib/docker/", "/var/lib/snapd/", "/var/lib/containers/", "/proc",
               "/sys", "/dev")
_HIDDEN_MOUNTS = ("/boot", "/boot/efi", "/efi", "/recovery")
_VM_RE = re.compile(r"^(vm|vms|vmstore|vm-?store|virtual|virtualmachines|virtual[-_ ]machines|libvirt|kvm|qemu)$",
                    re.I)


def _unescape_mount(s: str) -> str:
    return re.sub(r"\\([0-7]{3})", lambda m: chr(int(m.group(1), 8)), s)


def parse_mountinfo(text: str) -> list[dict]:
    """Pure (tested): [{mm, root, mount, fstype, source, opts}]."""
    out = []
    for line in text.splitlines():
        parts = line.split(" ")
        try:
            sep = parts.index("-")
        except ValueError:
            continue
        if len(parts) < sep + 3 or sep < 6:
            continue
        out.append({"mm": parts[2], "root": _unescape_mount(parts[3]), "mount": _unescape_mount(parts[4]),
                    "opts": parts[5], "fstype": parts[sep + 1], "source": _unescape_mount(parts[sep + 2])})
    return out


class _Lsblk:
    def __init__(self):
        self._t = 0.0
        self._by_mm: dict[str, dict] = {}
        self._lock = threading.Lock()

    def get(self) -> dict[str, dict]:
        with self._lock:
            if time.monotonic() - self._t < 10 and self._by_mm:
                return self._by_mm
            by_mm: dict[str, dict] = {}
            r = run(["lsblk", "-J", "-b", "-o", "NAME,LABEL,MODEL,ROTA,RM,TRAN,MOUNTPOINTS,UUID,MAJ:MIN,SIZE,FSTYPE,"
                     "TYPE,HOTPLUG"], timeout=4)
            data = None
            if r.ok:
                try:
                    data = json.loads(r.out)
                except ValueError:
                    data = None

            def walk(node: dict, top: dict):
                by_mm[str(node.get("maj:min") or node.get("maj_min") or "")] = {"node": node, "disk": top}
                for c in node.get("children") or []:
                    walk(c, top)

            for d in (data or {}).get("blockdevices", []):
                walk(d, d)
            self._by_mm = by_mm
            self._t = time.monotonic()
            return by_mm


_LSBLK = _Lsblk()


def _truthy(v: Any) -> bool:
    return v in (True, 1, "1", "true")


def volumes(include_hidden: bool = False) -> list[dict]:
    text = ""
    try:
        with open("/proc/self/mountinfo", "r", encoding="utf-8", errors="replace") as f:
            text = f.read()
    except OSError:
        pass
    blk = _LSBLK.get()
    home = os.path.expanduser("~")
    try:
        home_dev = os.stat(home).st_dev
    except OSError:
        home_dev = None
    mounts = parse_mountinfo(text)
    mounts.sort(key=lambda m: (m["mount"] != "/", len(m["mount"]), m["mount"]))
    seen: set[str] = set()
    out = []
    for m in mounts:
        fs, mp = m["fstype"], m["mount"]
        if fs not in _REAL_FS:
            continue
        if m["source"].startswith("/dev/loop") or mp.startswith(_SKIP_MOUNT) or mp.startswith("/run/user/"):
            continue
        if mp.startswith("/run/") and not mp.startswith("/run/media/"):
            continue
        hidden = mp in _HIDDEN_MOUNTS or mp.startswith("/boot/")
        if hidden and not include_hidden:
            continue
        if m["mm"] in seen:
            continue           # bind mount / second subvolume of the same filesystem
        seen.add(m["mm"])
        try:
            sv = os.statvfs(mp)
        except OSError:
            continue
        total = sv.f_blocks * sv.f_frsize
        if total <= 0:
            continue
        free = sv.f_bavail * sv.f_frsize
        used = max(0, (sv.f_blocks - sv.f_bfree) * sv.f_frsize)
        info = blk.get(m["mm"]) or {}
        node, disk = info.get("node") or {}, info.get("disk") or {}
        label = (node.get("label") or "").strip() or None
        uuid = node.get("uuid")
        tran = (disk.get("tran") or "").lower()
        removable = _truthy(disk.get("rm")) or _truthy(disk.get("hotplug")) or tran == "usb"
        if fs in ("iso9660", "udf") or str(disk.get("name", "")).startswith("sr"):
            kind = "optical"
        elif tran == "nvme" or str(disk.get("name", "")).startswith("nvme"):
            kind = "nvme"
        elif tran == "usb":
            kind = "usb"
        elif _truthy(disk.get("rota")):
            kind = "hdd"
        else:
            kind = "ssd"
        try:
            dev = os.stat(mp).st_dev
        except OSError:
            dev = None
        is_system = mp == "/"
        is_home = home_dev is not None and dev == home_dev
        base = os.path.basename(mp.rstrip("/"))
        if is_system:
            name = _tr("system")
        elif label and _VM_RE.match(label) or (not label and _VM_RE.match(base or "")):
            name = _tr("vm")
        elif label:
            name = label
        elif is_home and mp == home or mp == "/home":
            name = _tr("home")
        else:
            name = _tr("sized", size=fmt_size_short(total))
        vid = re.sub(r"[^A-Za-z0-9_-]", "", str(uuid)) if uuid else "mm-" + m["mm"].replace(":", "-")
        out.append({
            "id": vid, "device": m["source"] if m["source"].startswith("/dev/") else f"/dev/{node.get('name', '')}",
            "label": label, "name": name, "mount": mp, "fsType": _FS_LABEL.get(fs, fs), "total": total,
            "used": used, "free": free, "removable": bool(removable), "isSystem": is_system, "isHome": bool(is_home),
            "kind": kind,
            # extra
            "fsTypeRaw": fs, "percent": round(used * 100.0 / max(1, used + free), 1),
            "readOnly": "ro" in m["opts"].split(","), "model": (disk.get("model") or "").strip() or None,
            "hidden": hidden,
        })
    out.sort(key=lambda v: (not v["isSystem"], not v["isHome"], v["removable"], v["hidden"], v["name"].lower()))
    return out


# ---------------------------------------------------------------------------
# scans
# ---------------------------------------------------------------------------

class _Node:
    __slots__ = ("name", "path", "size", "files", "dirs", "parent", "children", "top_files", "rest_size",
                 "rest_count", "depth")

    def __init__(self, name: str, path: str, parent: "_Node | None", depth: int):
        self.name = name
        self.path = path
        self.size = 0
        self.files = 0
        self.dirs = 0
        self.parent = parent
        self.children: list[_Node] = []
        self.top_files: list[tuple[int, str]] = []   # min-heap (size, name)
        self.rest_size = 0
        self.rest_count = 0
        self.depth = depth


class ScanJob:
    _ids = itertools.count(1)

    def __init__(self, path: str, max_depth: int, top_n: int, emit: Callable, on_done: Callable,
                 large_limit: int = 500, node_budget: int = 8000):
        self.id = f"scan-{next(self._ids)}-{os.urandom(3).hex()}"
        self.path = path
        self.max_depth = max_depth
        self.top_n = top_n
        self.emit = emit
        self.on_done = on_done
        self.cancel = threading.Event()
        self.done = threading.Event()
        self.bytes = 0
        self.files = 0
        self.current = path
        self.started = time.monotonic()
        self.tree: dict | None = None
        self.error: str | None = None
        self.error_code: str | None = None
        self.large: list[tuple[int, str, float]] = []   # min-heap (size, path, mtime)
        self.large_limit = large_limit
        self.node_budget = node_budget
        self.skipped = 0
        self.thread: threading.Thread | None = None

    def start(self) -> None:
        self.thread = threading.Thread(target=self._run, name=f"storage-{self.id}", daemon=True)
        self.thread.start()

    def _run(self) -> None:
        _set_scan_priority()
        try:
            root = self._walk()
            if self.cancel.is_set():
                self.error_code, self.error = "cancelled", "Het scannen is gestopt."
            else:
                self.tree = self._serialize(root)
        except FileNotFoundError:
            self.error_code, self.error = "not_found", "Deze map bestaat niet (meer)."
        except PermissionError:
            self.error_code, self.error = "denied", "Je hebt geen toegang tot deze map."
        except Exception as e:  # noqa: BLE001
            log.exception("Scan mislukt (scan failed): %s", self.path)
            self.error_code, self.error = "failed", f"Het scannen is mislukt ({type(e).__name__})."
        # never let 'done' overtake the HTTP answer that carries the jobId
        wait = 0.15 - (time.monotonic() - self.started)
        if wait > 0:
            time.sleep(wait)
        self.done.set()
        self.on_done(self)
        payload = scan_done_event(self.id, self.tree, path=self.path, bytes=self.bytes, files=self.files,
                                  tookMs=int((time.monotonic() - self.started) * 1000))
        if self.error:
            payload["error"] = self.error
            payload["errorCode"] = self.error_code
        self.emit("storage.scanDone", payload)

    def _walk(self) -> _Node:
        st = os.stat(self.path)
        if not stat.S_ISDIR(st.st_mode):
            raise FileNotFoundError(self.path)
        os.listdir(self.path)  # raises PermissionError early
        dev = st.st_dev
        root = _Node(os.path.basename(self.path.rstrip("/")) or "/", self.path, None, 0)
        root.size = st.st_blocks * 512
        seen: set[tuple[int, int]] = set()
        stack: list[tuple[str, _Node, int]] = [(self.path, root, 0)]
        last_emit = time.monotonic()
        max_depth, top_n = self.max_depth, self.top_n
        large, large_limit = self.large, self.large_limit
        heappush, heapreplace = heapq.heappush, heapq.heapreplace
        S_ISDIR = stat.S_ISDIR
        count = 0
        while stack:
            if self.cancel.is_set():
                break
            dpath, node, depth = stack.pop()
            self.current = dpath
            try:
                it = os.scandir(dpath)
            except OSError:
                self.skipped += 1
                continue
            local = 0
            nfiles = 0
            ndirs = 0
            real = node.depth == depth      # False: deeper than maxDepth, sizes go to the ancestor node
            with it:
                for e in it:
                    try:
                        s = e.stat(follow_symlinks=False)
                    except OSError:
                        continue
                    if S_ISDIR(s.st_mode):
                        if s.st_dev != dev:
                            continue                       # another filesystem is mounted here
                        full = e.path
                        if full in _PSEUDO_SET:
                            continue
                        local += s.st_blocks * 512
                        ndirs += 1
                        if real and depth + 1 <= max_depth:
                            child = _Node(e.name, full, node, depth + 1)
                            child.size = s.st_blocks * 512   # its own directory entry (already in our total)
                            node.children.append(child)
                            stack.append((full, child, depth + 1))
                        else:
                            stack.append((full, node, depth + 1))
                    else:
                        if s.st_nlink > 1:
                            k = (s.st_dev, s.st_ino)
                            if k in seen:
                                continue
                            seen.add(k)
                        size = s.st_blocks * 512
                        local += size
                        nfiles += 1
                        if real:
                            tf = node.top_files
                            if len(tf) < top_n:
                                heappush(tf, (size, e.name))
                            elif size > tf[0][0]:
                                old = heapreplace(tf, (size, e.name))
                                node.rest_size += old[0]
                                node.rest_count += 1
                            else:
                                node.rest_size += size
                                node.rest_count += 1
                        if size >= 1048576:
                            if len(large) < large_limit:
                                heappush(large, (size, e.path, s.st_mtime))
                            elif size > large[0][0]:
                                heapreplace(large, (size, e.path, s.st_mtime))
                    count += 1
                    if count & 2047 == 0:
                        if self.cancel.is_set():
                            break
                        time.sleep(0)               # let the HTTP threads breathe
            n = node
            while n is not None:
                n.size += local
                n.files += nfiles
                n.dirs += ndirs
                n = n.parent
            self.bytes += local
            self.files += nfiles
            now = time.monotonic()
            if now - last_emit >= 0.25:
                last_emit = now
                self.emit("storage.scanProgress", {"jobId": self.id, "bytes": self.bytes, "files": self.files,
                                                   "current": dpath})
        self.bytes = root.size
        return root

    def _serialize(self, root: _Node) -> dict:
        other_name = _tr("other")
        budget = self.node_budget

        def leaf(n: _Node) -> dict:
            return {"name": n.name, "path": n.path, "size": n.size, "files": n.files, "dirs": n.dirs,
                    "isDir": True, "children": []}

        out_root = leaf(root)
        if self.skipped:
            out_root["skipped"] = self.skipped
        queue: list[tuple[_Node, dict]] = [(root, out_root)]
        qi = 0
        while qi < len(queue):
            node, out = queue[qi]
            qi += 1
            items: list[tuple[int, int, Any]] = []
            for c in node.children:
                items.append((c.size, 1, c))
            for size, name in node.top_files:
                items.append((size, 0, name))
            items.sort(key=lambda x: -x[0])
            top, rest = items[:self.top_n], items[self.top_n:]
            if budget <= 0:
                out["truncated"] = bool(items)
                continue
            children = []
            for size, is_dir, obj in top:
                if is_dir:
                    d = leaf(obj)
                    children.append(d)
                    if obj.depth < self.max_depth:
                        queue.append((obj, d))
                    elif obj.dirs or obj.files:
                        d["truncated"] = True
                else:
                    children.append({"name": obj, "path": os.path.join(node.path, obj), "size": size, "files": 1,
                                     "isDir": False})
            budget -= len(children)
            rest_size = sum(x[0] for x in rest) + node.rest_size
            rest_count = len(rest) + node.rest_count
            rest_files = sum((x[2].files if x[1] else 1) for x in rest) + node.rest_count
            if rest_count:
                children.append({"name": other_name, "other": True, "path": node.path, "size": rest_size,
                                 "files": rest_files, "count": rest_count, "isDir": False})
            out["children"] = children
        return out_root

    def progress(self) -> dict:
        return {"bytes": self.bytes, "files": self.files, "current": self.current}


SCAN_EVENT_TREE_MAX = 16 * 1024


def scan_done_event(job_id: str, tree: dict | None, **extra) -> dict:
    """The (small) storage.scanDone payload; the tree only when it is small."""
    out = {"jobId": job_id, "done": True, **extra}
    if tree is not None:
        if len(json_dumps(tree)) <= SCAN_EVENT_TREE_MAX:
            out["tree"] = tree
        else:
            out["hasTree"] = True       # fetch it: storage.scanResult {jobId}
    else:
        out["tree"] = None
    return out


class Scanner:
    def __init__(self, emit: Callable):
        self.emit = emit
        self._lock = threading.Lock()
        self.jobs: dict[str, ScanJob] = {}
        self.cached_ids: dict[str, tuple[str, int, int]] = {}   # 'cached-…' job ids → cache key
        self.cache: dict[tuple[str, int, int], dict] = {}
        self.large: dict[str, tuple[float, list[tuple[int, str, float]]]] = {}

    def _done(self, job: ScanJob) -> None:
        with self._lock:
            if job.tree is not None:
                self.cache[(job.path, job.max_depth, job.top_n)] = {"tree": job.tree, "time": time.time(),
                                                                    "jobId": job.id}
                if len(self.cache) > 8:
                    oldest = min(self.cache, key=lambda k: self.cache[k]["time"])
                    del self.cache[oldest]
                self.large[job.path] = (time.time(), sorted(job.large, reverse=True))
                if len(self.large) > 8:
                    del self.large[min(self.large, key=lambda k: self.large[k][0])]
            # forget finished jobs after a while
            cutoff = time.monotonic() - 600
            for jid in [j for j, jb in self.jobs.items() if jb.done.is_set() and jb.started < cutoff]:
                del self.jobs[jid]

    def start(self, path: Any, max_depth: Any, top_n: Any, refresh: bool) -> dict:
        p = _check_dir(path)
        try:
            md = max(1, min(8, int(max_depth if max_depth is not None else 4)))
            tn = max(5, min(200, int(top_n if top_n is not None else 40)))
        except (TypeError, ValueError):
            raise ApiError("bad_args", "Ongeldige invoer.") from None
        key = (p, md, tn)
        with self._lock:
            if not refresh and key in self.cache:
                c = self.cache[key]
                jid = f"cached-{os.urandom(4).hex()}"
                self.cached_ids[jid] = key
                while len(self.cached_ids) > 64:
                    self.cached_ids.pop(next(iter(self.cached_ids)))

                def late():
                    self.emit("storage.scanDone", scan_done_event(jid, c["tree"], path=p, cached=True,
                                                                  scannedAt=int(c["time"] * 1000)))
                t = threading.Timer(0.15, late)
                t.daemon = True
                t.start()
                return {"jobId": jid, "cached": True, "tree": c["tree"], "scannedAt": int(c["time"] * 1000)}
            for j in self.jobs.values():
                if not j.done.is_set() and (j.path, j.max_depth, j.top_n) == key:
                    return {"jobId": j.id, "running": True}
            running = [j for j in self.jobs.values() if not j.done.is_set()]
            if len(running) >= 3:
                raise ApiError("busy", "Er lopen al een paar scans. Wacht even tot die klaar zijn.")
            job = ScanJob(p, md, tn, self.emit, self._done)
            self.jobs[job.id] = job
        job.start()
        return {"jobId": job.id}

    def cancel(self, job_id: Any) -> dict:
        with self._lock:
            job = self.jobs.get(str(job_id))
        if job is None:
            return {"ok": False}
        job.cancel.set()
        return {"ok": True}

    def result(self, job_id: Any) -> dict:
        with self._lock:
            job = self.jobs.get(str(job_id))
            key = self.cached_ids.get(str(job_id))
            c = self.cache.get(key) if key else None
        if job is None and c is not None:   # answered from the cache
            return {"jobId": str(job_id), "state": "done", "path": key[0], "tree": c["tree"], "cached": True,
                    "scannedAt": int(c["time"] * 1000), "progress": {"bytes": c["tree"].get("size", 0),
                                                                     "files": c["tree"].get("files", 0),
                                                                     "current": key[0]}}
        if job is None:
            raise ApiError("not_found", "Deze scan is niet (meer) bekend.")
        state = "running" if not job.done.is_set() else ("error" if job.error else "done")
        out = {"jobId": job.id, "state": state, "progress": job.progress(), "path": job.path}
        if state == "done":
            out["tree"] = job.tree
        if job.error:
            out["error"] = job.error
            out["errorCode"] = job.error_code
        return out

    def large_for(self, path: str) -> tuple[list[tuple[int, str, float]], bool] | None:
        """(largest files ≥ 1 MB under ``path`` from the most specific cached scan, complete?) or None."""
        with self._lock:
            best = None
            for root, (t, lst) in self.large.items():
                if path == root or path.startswith(root.rstrip("/") + "/"):
                    if best is None or len(root) > len(best[0]):
                        best = (root, lst)
        if best is None:
            return None
        full = len(best[1]) >= 500          # the scan kept only its 500 largest files
        if best[0] == path:
            return best[1], True
        pre = path.rstrip("/") + "/"
        sub = [x for x in best[1] if x[1].startswith(pre)]
        return sub, not full


def _check_dir(path: Any) -> str:
    if not isinstance(path, str) or not path or "\x00" in path:
        raise ApiError("bad_args", "Geef een map op.")
    p = os.path.realpath(os.path.expanduser(path))
    if not os.path.isabs(p):
        raise ApiError("bad_args", "Geef een volledig pad op.")
    if p != "/" and (p in _PSEUDO_ROOTS or p.startswith(tuple(r + "/" for r in _PSEUDO_ROOTS))) \
            and not p.startswith("/run/media/"):
        raise ApiError("unsupported", "Deze map kan niet worden gescand.")
    if not os.path.isdir(p):
        raise ApiError("not_found", "Deze map bestaat niet.")
    return p


# ---------------------------------------------------------------------------
# categories
# ---------------------------------------------------------------------------

_CATEGORY_IDS = ("documents", "pictures", "videos", "music", "downloads", "desktop", "trash", "browser", "chat",
                 "dev", "appdata", "snaps", "flatpak", "cache", "other")

_BROWSER_ROOTS = {
    ".mozilla": "Firefox", ".cache/mozilla": "Firefox", "snap/firefox": "Firefox",
    ".var/app/org.mozilla.firefox": "Firefox", ".zen": "Zen", ".cache/zen": "Zen",
    ".var/app/app.zen_browser.zen": "Zen", ".config/google-chrome": "Google Chrome",
    ".cache/google-chrome": "Google Chrome", ".var/app/com.google.Chrome": "Google Chrome",
    ".config/chromium": "Chromium", ".cache/chromium": "Chromium", "snap/chromium": "Chromium",
    ".var/app/org.chromium.Chromium": "Chromium", ".config/BraveSoftware": "Brave", ".cache/BraveSoftware": "Brave",
    "snap/brave": "Brave", ".var/app/com.brave.Browser": "Brave", ".config/vivaldi": "Vivaldi",
    ".cache/vivaldi": "Vivaldi", ".config/microsoft-edge": "Microsoft Edge", ".cache/microsoft-edge": "Microsoft Edge",
    ".librewolf": "LibreWolf", ".cache/librewolf": "LibreWolf", ".config/opera": "Opera", "snap/opera": "Opera",
}
_CHAT_ROOTS = {
    ".config/discord": "Discord", "snap/discord": "Discord", ".var/app/com.discordapp.Discord": "Discord",
    ".local/share/TelegramDesktop": "Telegram", "snap/telegram-desktop": "Telegram",
    ".var/app/org.telegram.desktop": "Telegram", ".config/Slack": "Slack", "snap/slack": "Slack",
    ".var/app/com.slack.Slack": "Slack", ".config/Signal": "Signal", "snap/signal-desktop": "Signal",
    ".var/app/org.signal.Signal": "Signal", "snap/whatsapp-desktop-linux": "WhatsApp",
    ".config/whatsapp-desktop-linux": "WhatsApp", ".var/app/io.github.mimbrero.WhatsAppDesktop": "WhatsApp",
    ".config/Element": "Element", ".var/app/im.riot.Riot": "Element", ".config/teams-for-linux": "Teams",
    ".config/zoom.us": "Zoom", ".zoom": "Zoom", ".var/app/us.zoom.Zoom": "Zoom",
}
_DEV_ROOTS = {
    ".cargo": "Rust (cargo)", ".rustup": "Rust (rustup)", ".npm": "npm", ".cache/pip": "pip", ".gradle": "Gradle",
    ".m2": "Maven", "go": "Go", ".cache/go-build": "Go", ".cache/yarn": "Yarn", ".nvm": "Node (nvm)",
    ".pyenv": "Python (pyenv)", ".local/share/pnpm": "pnpm", ".bun": "Bun", ".deno": "Deno", ".dotnet": ".NET",
    ".nuget": ".NET (NuGet)", ".cache/node-gyp": "node-gyp", ".android": "Android", "Android/Sdk": "Android SDK",
}


def _category_roots(home: str) -> tuple[dict[str, tuple[str, str | None]], dict[str, str]]:
    """{abs path: (category, item name|None)} and {category: main path}."""
    roots: dict[str, tuple[str, str | None]] = {}
    main: dict[str, str] = {"other": home}

    def add(rel_or_abs: str, cat: str, item: str | None = None):
        p = rel_or_abs if os.path.isabs(rel_or_abs) else os.path.join(home, rel_or_abs)
        p = os.path.normpath(p)
        if p == home:
            return
        roots[p] = (cat, item)
        main.setdefault(cat, p)

    for cat, kind in (("documents", "DOCUMENTS"), ("pictures", "PICTURES"), ("videos", "VIDEOS"),
                      ("music", "MUSIC"), ("downloads", "DOWNLOAD"), ("desktop", "DESKTOP")):
        try:
            add(str(xdg_user_dir(kind)), cat)
        except Exception:  # noqa: BLE001
            pass
    add(".config", "appdata")
    add(".local/share", "appdata")
    add(".local/state", "appdata")
    add(".cache", "cache")
    add("snap", "snaps")
    add(".var/app", "flatpak")
    add(".local/share/flatpak", "flatpak")
    for rel, name in _DEV_ROOTS.items():
        add(rel, "dev", name)
    for rel, name in _BROWSER_ROOTS.items():
        add(rel, "browser", name)
    for rel, name in _CHAT_ROOTS.items():
        add(rel, "chat", name)
    add(".local/share/Trash", "trash")
    main["trash"] = os.path.join(home, ".local/share/Trash/files")
    main["dev"] = home
    main["browser"] = home
    main["chat"] = home
    return roots, main


class Categories:
    def __init__(self, emit: Callable, cache_file: Path | None):
        self.emit = emit
        self.cache_file = cache_file
        self._lock = threading.Lock()
        self.data: dict | None = read_json(cache_file, None) if cache_file else None
        if self.data is not None and not isinstance(self.data, dict):
            self.data = None
        self.fresh = False
        self.thread: threading.Thread | None = None
        self.progress = {"files": 0, "bytes": 0}

    def get(self, refresh: bool) -> dict:
        with self._lock:
            running = self.thread is not None and self.thread.is_alive()
            stale = self.data is None or not self.fresh or refresh or \
                (time.time() * 1000 - self.data.get("scannedAt", 0) > 15 * 60 * 1000)
            if stale and not running:
                self.thread = threading.Thread(target=self._safe_run, name="storage-categories", daemon=True)
                self.thread.start()
                running = True
            out = {"ready": self.data is not None and not refresh, "data": self.data, "running": running}
            if running:
                out["progress"] = dict(self.progress)
        return out

    def _safe_run(self) -> None:
        try:
            data = self.compute()
            with self._lock:
                self.data = data
                self.fresh = True
            if self.cache_file:
                try:
                    atomic_write_json(self.cache_file, data, pretty=False)
                except OSError:
                    pass
            self.emit("storage.categoriesReady", {"data": data})
        except Exception:  # noqa: BLE001
            log.exception("Opslagcategorieën mislukt (categories failed)")

    def compute(self, budget_s: float = 180.0) -> dict:
        _set_scan_priority()
        home = os.path.realpath(os.path.expanduser("~"))
        roots, main = _category_roots(home)
        sizes = {c: 0 for c in _CATEGORY_IDS}
        items: dict[tuple[str, str], int] = {}
        item_paths: dict[tuple[str, str], str] = {}
        per_child = {"appdata", "cache", "flatpak", "snaps", "other", "documents", "downloads", "videos", "music",
                     "pictures"}
        t0 = time.monotonic()
        deadline = t0 + budget_s
        try:
            hst = os.stat(home)
        except OSError:
            raise ApiError("not_found", "Je persoonlijke map is niet gevonden.") from None
        dev = hst.st_dev
        seen: set[tuple[int, int]] = set()
        # (dir, depth, category, item, children-become-items)
        stack: list[tuple[str, int, str, str | None, bool]] = [(home, 0, "other", None, True)]
        files = 0
        total = hst.st_blocks * 512
        partial = False
        last = time.monotonic()
        n = 0
        while stack:
            d, depth, cat, item, split = stack.pop()
            try:
                it = os.scandir(d)
            except OSError:
                continue
            with it:
                for e in it:
                    try:
                        s = e.stat(follow_symlinks=False)
                    except OSError:
                        continue
                    size = s.st_blocks * 512
                    if stat.S_ISDIR(s.st_mode):
                        if s.st_dev != dev:
                            continue
                        full = e.path
                        r = roots.get(full)
                        csplit = False
                        if r is not None:
                            ccat, citem = r[0], r[1]
                            csplit = citem is None and ccat in per_child
                        elif e.name == "node_modules" and depth < 4:
                            ccat, citem = "dev", "node_modules"
                        elif split:
                            ccat, citem = cat, e.name
                            item_paths.setdefault((ccat, citem), full)
                        else:
                            ccat, citem = cat, item
                        sizes[ccat] += size
                        if citem:
                            items[(ccat, citem)] = items.get((ccat, citem), 0) + size
                            if r is not None:
                                item_paths.setdefault((ccat, citem), full)
                        stack.append((full, depth + 1, ccat, citem, csplit))
                    else:
                        if s.st_nlink > 1:
                            k = (s.st_dev, s.st_ino)
                            if k in seen:
                                continue
                            seen.add(k)
                        sizes[cat] += size
                        if item:
                            items[(cat, item)] = items.get((cat, item), 0) + size
                        files += 1
                    total += size
                    n += 1
                    if n & 4095 == 0:
                        now = time.monotonic()
                        if now > deadline:
                            partial = True
                            stack.clear()
                            break
                        if now - last > 1.0:
                            last = now
                            self.progress = {"files": files, "bytes": total}
                            self.emit("storage.categoriesProgress", {"files": files, "bytes": total})
                        time.sleep(0)
        cats = []
        for cid in _CATEGORY_IDS:
            entry = {"id": cid, "name": _tr("c_" + cid), "size": sizes[cid], "path": main.get(cid, home)}
            its = sorted(((name, sz) for (c, name), sz in items.items() if c == cid), key=lambda x: -x[1])
            if its:
                entry["items"] = [{"name": nm, "size": sz, "path": item_paths.get((cid, nm))}
                                  for nm, sz in its[:12] if sz > 0]
            cats.append(entry)
        cats.sort(key=lambda c: (c["id"] == "other", -c["size"]))
        return {"home": home, "categories": cats, "total": total, "files": files, "partial": partial,
                "scannedAt": int(time.time() * 1000), "tookMs": int((time.monotonic() - t0) * 1000)}


# ---------------------------------------------------------------------------
# cleanup
# ---------------------------------------------------------------------------

_CLEANUP_IDS = ("trash", "thumbnails", "user-cache-safe", "browser-cache", "old-downloads", "apt-cache",
                "old-snaps", "journal", "flatpak-unused", "crash-reports")

# Caches that apps rebuild on their own (relative to home). ~/.cache is NEVER cleared as a whole.
_SAFE_CACHES = [
    (".cache/pip", "pip"), (".npm/_cacache", "npm"), (".cache/yarn", "Yarn"), (".cache/go-build", "Go"),
    (".cache/mesa_shader_cache", "Mesa-shaders"), (".cache/mesa_shader_cache_db", "Mesa-shaders"),
    (".cache/nvidia/GLCache", "NVIDIA-shaders"), (".cache/radv_builtin_shaders64", "AMD-shaders"),
    (".cache/fontconfig", "Lettertypen"), (".cache/gstreamer-1.0", "GStreamer"), (".cache/typescript", "TypeScript"),
    (".cache/jedi", "Jedi"), (".cache/matplotlib", "Matplotlib"), (".cache/node-gyp", "node-gyp"),
    (".cache/electron", "Electron"), (".cache/babel", "Babel"), (".cache/vscode-cpptools", "VS Code C++"),
]
_SAFE_CACHE_GLOBS = [(".cache", "qtshadercache-*", "Qt-shaders")]

_BROWSER_CACHES = {
    "firefox": [".cache/mozilla/firefox", "snap/firefox/common/.cache/mozilla/firefox",
                ".var/app/org.mozilla.firefox/cache/mozilla/firefox"],
    "zen": [".cache/zen", ".var/app/app.zen_browser.zen/cache/zen"],
    "chrome": [".cache/google-chrome", ".var/app/com.google.Chrome/cache/google-chrome"],
    "chromium": [".cache/chromium", "snap/chromium/common/.cache/chromium",
                 ".var/app/org.chromium.Chromium/cache/chromium"],
    "brave": [".cache/BraveSoftware", "snap/brave/common/.cache/BraveSoftware",
              ".var/app/com.brave.Browser/cache/BraveSoftware"],
    "vivaldi": [".cache/vivaldi"], "edge": [".cache/microsoft-edge"], "librewolf": [".cache/librewolf"],
    "opera": [".cache/opera"],
}


def _browser_running() -> set[str]:
    """Which browsers have a process of this user right now."""
    found: set[str] = set()
    try:
        entries = os.listdir("/proc")
    except OSError:
        return found
    for e in entries:
        if not e.isdigit():
            continue
        try:
            if os.stat(f"/proc/{e}").st_uid != UID:
                continue
            exe = os.readlink(f"/proc/{e}/exe")
        except OSError:
            continue
        low = exe.lower()
        base = os.path.basename(low)
        if base in ("firefox", "firefox-bin", "firefox-esr") or "/firefox/" in low:
            found.add("firefox")
        elif base in ("zen", "zen-bin") or "/zen/" in low:
            found.add("zen")
        elif "/opt/google/chrome" in low:
            found.add("chrome")
        elif "chromium" in low:
            found.add("chromium")
        elif "brave" in low:
            found.add("brave")
        elif "vivaldi" in low:
            found.add("vivaldi")
        elif "msedge" in low or "microsoft/edge" in low:
            found.add("edge")
        elif "librewolf" in low:
            found.add("librewolf")
        elif base == "opera" or "/opera/" in low:
            found.add("opera")
    return found


_trash_dirs_cache: tuple[float, list] = (0.0, [])


def _trash_dirs(max_age: float = 600.0) -> list[str]:
    """Trash folders of this user: home trash + $topdir/.Trash-$uid / .Trash/$uid on mounted volumes.
    Same rules as files.* (``WIN11SHELL_TRASH_DIR`` sandbox, only real own folders, no symlinks).
    Cached: looking for them can wake sleeping hard disks."""
    global _trash_dirs_cache
    if _trash_dirs_cache[1] and time.monotonic() - _trash_dirs_cache[0] < max_age:
        return [str(t) for t, _top in _trash_dirs_cache[1]]
    from .files import trash_dirs as files_trash_dirs
    dirs = files_trash_dirs()
    _trash_dirs_cache = (time.monotonic(), dirs)
    return [str(t) for t, _top in dirs]


def thumbnail_dir() -> str:
    """The freedesktop thumbnail cache (``WIN11SHELL_THUMBNAIL_DIR`` overrides it for tests)."""
    o = os.environ.get("WIN11SHELL_THUMBNAIL_DIR")
    if o:
        return o
    return os.path.join(os.environ.get("XDG_CACHE_HOME") or os.path.join(os.path.expanduser("~"), ".cache"),
                        "thumbnails")


def download_dir() -> str:
    """The Downloads folder (``WIN11SHELL_DOWNLOAD_DIR`` overrides it for tests)."""
    return os.environ.get("WIN11SHELL_DOWNLOAD_DIR") or str(xdg_user_dir("DOWNLOAD"))


def trash_size() -> dict:
    size = count = 0
    deadline = time.monotonic() + 5
    for t in _trash_dirs():
        files = os.path.join(t, "files")
        s, _f = du(files, deadline=deadline)
        size += s
        try:
            count += sum(1 for _ in os.scandir(files))
        except OSError:
            pass
        s2, _ = du(os.path.join(t, "info"), deadline=deadline)
        size += s2
    return {"size": size, "count": count}


def _old_downloads(days: int = 90) -> list[dict]:
    d = download_dir()
    cutoff = time.time() - days * 86400
    out = []
    try:
        entries = list(os.scandir(d))
    except OSError:
        return out
    for e in entries:
        try:
            s = e.stat(follow_symlinks=False)
        except OSError:
            continue
        if e.name.startswith(".") or stat.S_ISLNK(s.st_mode):
            continue
        if max(s.st_mtime, s.st_atime) >= cutoff:
            continue
        size = s.st_blocks * 512 if not stat.S_ISDIR(s.st_mode) else du(e.path, deadline=time.monotonic() + 2)[0]
        out.append({"path": e.path, "name": e.name, "size": size, "modified": int(s.st_mtime * 1000),
                    "accessed": int(s.st_atime * 1000), "isDir": stat.S_ISDIR(s.st_mode)})
    out.sort(key=lambda x: -x["size"])
    return out


def _snap_disabled() -> list[dict]:
    if not which("snap"):
        return []
    r = run(["snap", "list", "--all"], timeout=10, env={**os.environ, "LC_ALL": "C"})
    out = []
    for line in r.out.splitlines()[1:] if r.ok else []:
        p = line.split()
        if len(p) >= 6 and "disabled" in p[-1].split(","):
            name, rev = p[0], p[2]
            if not re.match(r"^[a-z0-9][a-z0-9-]*$", name) or not re.match(r"^x?\d+$", rev):
                continue
            f = f"/var/lib/snapd/snaps/{name}_{rev}.snap"
            try:
                size = os.stat(f).st_blocks * 512
            except OSError:
                size = 0
            out.append({"name": name, "revision": rev, "size": size})
    return out


def _journal_estimate(days: int = 14) -> tuple[int | None, int | None]:
    """(bytes freed by --vacuum-time=14d estimated from archived journal files, total usage)."""
    cutoff = time.time() - days * 86400
    est = 0
    total = 0
    readable = False
    for base in ("/var/log/journal", "/run/log/journal"):
        try:
            for mid in os.scandir(base):
                if not mid.is_dir(follow_symlinks=False):
                    continue
                for f in os.scandir(mid.path):
                    readable = True
                    try:
                        s = f.stat(follow_symlinks=False)
                    except OSError:
                        continue
                    size = s.st_blocks * 512
                    total += size
                    # archived files (name@…journal / *.journal~) are what vacuum removes
                    if ("@" in f.name or f.name.endswith("~")) and s.st_mtime < cutoff:
                        est += size
        except OSError:
            continue
    if not readable:
        r = run(["journalctl", "--disk-usage"], timeout=5, env={**os.environ, "LC_ALL": "C"})
        m = re.search(r"take up ([0-9.]+)([KMGT]?)", r.out or "")
        if m:
            mult = {"": 1, "K": 1024, "M": 1024 ** 2, "G": 1024 ** 3, "T": 1024 ** 4}[m.group(2)]
            return None, int(float(m.group(1)) * mult)
        return None, None
    return est, total


def _apt_cache_size() -> int:
    total = 0
    for d in ("/var/cache/apt/archives", "/var/cache/apt/archives/partial"):
        try:
            for e in os.scandir(d):
                if e.name.endswith(".deb"):
                    try:
                        total += e.stat(follow_symlinks=False).st_blocks * 512
                    except OSError:
                        pass
        except OSError:
            pass
    return total


def _crash_files() -> list[str]:
    out = []
    try:
        for e in os.scandir("/var/crash"):
            try:
                s = e.stat(follow_symlinks=False)
            except OSError:
                continue
            if s.st_uid == UID and stat.S_ISREG(s.st_mode):
                out.append(e.path)
    except OSError:
        pass
    return out


def _safe_cache_paths(home: str) -> list[tuple[str, str]]:
    out = []
    for rel, name in _SAFE_CACHES:
        p = os.path.join(home, rel)
        if os.path.isdir(p) and not os.path.islink(p):
            out.append((p, name))
    for rel_dir, pattern, name in _SAFE_CACHE_GLOBS:
        for p in Path(home, rel_dir).glob(pattern):
            if p.is_dir() and not p.is_symlink():
                out.append((str(p), name))
    return out


def _browser_cache_paths(home: str) -> list[tuple[str, str, str]]:
    out = []
    for key, rels in _BROWSER_CACHES.items():
        for rel in rels:
            p = os.path.join(home, rel)
            if os.path.isdir(p) and not os.path.islink(p):
                out.append((key, _tr("b_" + key), p))
    return out


class Cleanup:
    def __init__(self, own_thumbs: str | os.PathLike | None = None):
        self.own_thumbs = os.fspath(own_thumbs) if own_thumbs else None     # <cache>/thumbs (our /thumb cache)
        self._lock = threading.Lock()
        self._cache: tuple[float, list[dict], str] | None = None

    def list(self, refresh: bool = False) -> list[dict]:
        with self._lock:
            if not refresh and self._cache and time.monotonic() - self._cache[0] < 60 and \
                    self._cache[2] == _lang():
                return [dict(x) for x in self._cache[1]]
        home = os.path.realpath(os.path.expanduser("~"))
        deadline = time.monotonic() + 20
        items = []

        def item(cid: str, size: int | None, *, needs_admin=False, safe=True, default=False, details=None,
                 available=True, description=None) -> None:
            items.append({"id": cid, "name": _tr("k_" + cid), "description": description or _tr("d_" + cid),
                          "size": size, "needsAdmin": needs_admin, "safe": safe,
                          "defaultSelected": bool(default and available and (size is None or size > 0)),
                          "details": details or {}, "available": available})

        ts = trash_size()
        item("trash", ts["size"], default=False, details={"count": ts["count"]})
        thumbs_size = du(thumbnail_dir(), deadline=deadline)[0]
        if self.own_thumbs:
            thumbs_size += du(self.own_thumbs, deadline=deadline)[0]
        item("thumbnails", thumbs_size, default=True)
        caches = []
        for p, name in _safe_cache_paths(home):
            caches.append({"name": name, "path": p, "size": du(p, deadline=deadline)[0]})
        caches.sort(key=lambda x: -x["size"])
        item("user-cache-safe", sum(c["size"] for c in caches), default=True, details={"items": caches})
        running = _browser_running()
        bitems = []
        for key, name, p in _browser_cache_paths(home):
            bitems.append({"browser": key, "name": name, "path": p, "size": du(p, deadline=deadline)[0],
                           "running": key in running})
        cleanable = sum(b["size"] for b in bitems if not b["running"])
        desc = _tr("d_browser-cache")
        busy = sorted({b["name"] for b in bitems if b["running"] and b["size"] > 0})
        if busy:
            desc += _tr("d_browser-running", names=" / ".join(busy))
        item("browser-cache", cleanable, default=True, available=bool(bitems),
             details={"items": bitems, "running": busy, "total": sum(b["size"] for b in bitems)}, description=desc)
        old = _old_downloads()
        item("old-downloads", sum(o["size"] for o in old), safe=False, default=False,
             details={"items": old[:300], "count": len(old), "days": 90, "toTrash": True})
        apt_size = _apt_cache_size() if os.path.isdir("/var/cache/apt") else 0
        item("apt-cache", apt_size, needs_admin=True, default=False, available=bool(which("apt-get")))
        snaps = _snap_disabled()
        item("old-snaps", sum(s["size"] for s in snaps), needs_admin=True, default=False,
             available=bool(which("snap")), details={"items": snaps})
        est, jtotal = _journal_estimate()
        item("journal", est, needs_admin=True, default=False, available=bool(which("journalctl")),
             details={"total": jtotal, "keepDays": 14, "estimate": True})
        fp_user = os.path.join(home, ".local/share/flatpak")
        item("flatpak-unused", None, default=False,
             available=bool(which("flatpak")) and os.path.isdir(os.path.join(fp_user, "runtime")),
             details={"scope": "user"})
        crash = _crash_files()
        crash_size = sum(du(p)[0] for p in crash)
        item("crash-reports", crash_size, default=True, available=os.path.isdir("/var/crash"),
             details={"count": len(crash)})
        with self._lock:
            self._cache = (time.monotonic(), items, _lang())
        return [dict(x) for x in items]

    def run(self, ids: Any, paths: Any = None) -> dict:
        if isinstance(ids, str):
            ids = [ids]
        if not isinstance(ids, list) or not ids or any(i not in _CLEANUP_IDS for i in ids):
            raise ApiError("bad_args", "Kies wat je wilt opruimen.")
        home = os.path.realpath(os.path.expanduser("~"))
        results: list[dict] = []
        admin_steps: list[str] = []
        admin_ids: list[str] = []
        admin_before: dict[str, int | None] = {}
        for cid in dict.fromkeys(ids):
            try:
                if cid == "trash":
                    from .files import empty_trash_dirs
                    _trash_dirs(max_age=0)
                    before = trash_size()["size"]
                    _removed, _failed, errs = empty_trash_dirs(_trash_dirs_cache[1])
                    after = trash_size()["size"]
                    results.append(self._res(cid, before - after, errs))
                elif cid == "thumbnails":
                    freed, errs = 0, []
                    for d in [thumbnail_dir()] + ([self.own_thumbs] if self.own_thumbs else []):
                        before = du(d)[0]
                        errs += _clear_dir_contents(d, keep_dirs=True)
                        freed += before - du(d)[0]
                    results.append(self._res(cid, freed, errs))
                elif cid == "user-cache-safe":
                    freed, errs = 0, []
                    for p, _name in _safe_cache_paths(home):
                        b = du(p)[0]
                        errs += _clear_dir_contents(p)
                        freed += b - du(p)[0]
                    results.append(self._res(cid, freed, errs))
                elif cid == "browser-cache":
                    running = _browser_running()
                    freed, errs, skipped = 0, [], []
                    for key, name, p in _browser_cache_paths(home):
                        if key in running:
                            skipped.append(name)
                            continue
                        b = du(p)[0]
                        errs += _clear_dir_contents(p)
                        freed += b - du(p)[0]
                    r = self._res(cid, freed, errs)
                    if skipped:
                        r["skipped"] = sorted(set(skipped))
                        if not freed:
                            r["ok"] = False
                            r["error"] = "Sluit eerst " + " / ".join(sorted(set(skipped))) + "."
                    results.append(r)
                elif cid == "old-downloads":
                    results.append(self._trash_downloads(paths))
                elif cid == "apt-cache":
                    admin_before[cid] = _apt_cache_size()
                    admin_steps.append("apt")
                    admin_ids.append(cid)
                elif cid == "old-snaps":
                    snaps = _snap_disabled()
                    admin_before[cid] = sum(s["size"] for s in snaps)
                    for s in snaps:
                        admin_steps.append(f"snap:{s['name']}={s['revision']}")
                    if snaps:
                        admin_ids.append(cid)
                    else:
                        results.append({"id": cid, "ok": True, "freed": 0, "error": None})
                elif cid == "journal":
                    admin_before[cid] = _journal_estimate()[1]
                    admin_steps.append("journal")
                    admin_ids.append(cid)
                elif cid == "flatpak-unused":
                    if not which("flatpak"):
                        results.append({"id": cid, "ok": False, "freed": 0, "error": "Flatpak is niet geïnstalleerd."})
                        continue
                    d = os.path.join(home, ".local/share/flatpak")
                    before = du(d, deadline=time.monotonic() + 30)[0]
                    r = run(["flatpak", "uninstall", "--unused", "--user", "-y", "--noninteractive"], timeout=600)
                    after = du(d, deadline=time.monotonic() + 30)[0]
                    results.append({"id": cid, "ok": r.ok, "freed": max(0, before - after),
                                    "error": None if r.ok else "Opruimen van Flatpak lukte niet."})
                elif cid == "crash-reports":
                    freed, errs = 0, []
                    for p in _crash_files():
                        try:
                            size = os.lstat(p).st_blocks * 512
                            os.unlink(p)
                            freed += size
                        except OSError:
                            errs.append(p)
                    results.append(self._res(cid, freed, errs))
            except Exception as e:  # noqa: BLE001
                log.exception("Opruimen %s mislukt (cleanup failed)", cid)
                results.append({"id": cid, "ok": False, "freed": 0, "error": f"Dat lukte niet ({type(e).__name__})."})
        if admin_steps:
            results += self._run_admin(admin_steps, admin_ids, admin_before)
        with self._lock:
            self._cache = None
        return {"results": results, "freed": sum(max(0, r.get("freed") or 0) for r in results)}

    @staticmethod
    def _res(cid: str, freed: int, errs: list[str]) -> dict:
        r = {"id": cid, "ok": not errs or freed > 0, "freed": max(0, int(freed)), "error": None}
        if errs:
            r["failed"] = len(errs)
            if not r["ok"]:
                r["error"] = "Sommige bestanden konden niet worden verwijderd."
        return r

    @staticmethod
    def _trash_downloads(paths: Any) -> dict:
        old = {o["path"]: o for o in _old_downloads()}
        if paths is not None:
            if not isinstance(paths, list):
                raise ApiError("bad_args", "Ongeldige lijst met bestanden.")
            chosen = [p for p in paths if isinstance(p, str) and p in old]
        else:
            chosen = list(old)
        moved, size, errs = 0, 0, []
        try:
            from .files import trash_path      # same path as files.trash (sandbox trash in tests, else Gio)
        except Exception:  # noqa: BLE001
            return {"id": "old-downloads", "ok": False, "freed": 0, "error": "De prullenbak is niet beschikbaar."}
        for p in chosen:
            try:
                trash_path(p)
                moved += 1
                size += old[p]["size"]
            except Exception:  # noqa: BLE001
                errs.append(p)
        r = {"id": "old-downloads", "ok": not errs or moved > 0, "freed": 0, "movedToTrash": size, "count": moved,
             "error": None if not errs else "Sommige bestanden konden niet naar de prullenbak."}
        return r

    @staticmethod
    def _run_admin(steps: list[str], ids: list[str], before: dict[str, int | None]) -> list[dict]:
        if not which("pkexec"):
            return [{"id": i, "ok": False, "freed": 0, "error": "Beheerdersrechten zijn hier niet beschikbaar."}
                    for i in ids]
        # One password prompt for all steps. The script is constant; every value is a validated positional arg.
        script = (
            'for step in "$@"; do\n'
            '  case "$step" in\n'
            '    apt) if apt-get clean; then echo "@@ok apt"; else echo "@@fail apt"; fi ;;\n'
            '    journal) if journalctl --vacuum-time=14d >/dev/null 2>&1; then echo "@@ok journal"; '
            'else echo "@@fail journal"; fi ;;\n'
            '    snap:*) s="${step#snap:}"; n="${s%%=*}"; r="${s#*=}";\n'
            '      if snap remove "$n" --revision="$r" >/dev/null 2>&1; then echo "@@ok snap"; '
            'else echo "@@fail snap"; fi ;;\n'
            '  esac\n'
            'done\n')
        for s in steps:
            if not re.match(r"^(apt|journal|snap:[a-z0-9][a-z0-9-]*=x?\d+)$", s):
                return [{"id": i, "ok": False, "freed": 0, "error": "Ongeldige opdracht."} for i in ids]
        env = {k: v for k, v in os.environ.items() if k in ("DISPLAY", "WAYLAND_DISPLAY", "XAUTHORITY", "PATH",
                                                            "DBUS_SESSION_BUS_ADDRESS", "XDG_RUNTIME_DIR", "LANG")}
        helper = _admin_helper()
        if helper:     # installed helper + polkit policy: the password dialog explains it in plain Dutch
            cmd = ["pkexec", helper] + steps
        else:
            cmd = ["pkexec", "/bin/sh", "-c", script, "win11shell-cleanup"] + steps
        r = run(cmd, timeout=900, env=env)
        out = r.out or ""
        results = []
        if r.rc in (126, 127) and "@@" not in out:
            msg = "Je hebt het wachtwoord niet ingevuld, dus er is niets veranderd." if r.rc == 126 else \
                "Het wachtwoord klopte niet. Er is niets veranderd."
            return [{"id": i, "ok": False, "freed": 0, "error": msg} for i in ids]
        for cid in ids:
            tag = {"apt-cache": "apt", "journal": "journal", "old-snaps": "snap"}[cid]
            ok = f"@@ok {tag}" in out and f"@@fail {tag}" not in out
            if cid == "apt-cache":
                after = _apt_cache_size()
            elif cid == "old-snaps":
                after = sum(s["size"] for s in _snap_disabled())
            else:
                after = _journal_estimate()[1]
            b = before.get(cid)
            freed = max(0, (b or 0) - (after or 0)) if b is not None and after is not None else 0
            results.append({"id": cid, "ok": ok, "freed": freed, "error": None if ok else "Dat lukte niet."})
        return results


ADMIN_HELPER = "/usr/libexec/win11shell/cleanup-helper"     # host/data/win11shell-cleanup-helper, root-owned


def _admin_helper() -> str | None:
    """The installed cleanup helper, only when it is safe to run as root: a regular file owned by root
    that nobody else can change (otherwise the generic `pkexec /bin/sh -c` fallback is used)."""
    p = os.environ.get("WIN11SHELL_ADMIN_HELPER") or ADMIN_HELPER
    try:
        st = os.lstat(p)
        dst = os.stat(os.path.dirname(p))
    except OSError:
        return None
    if not stat.S_ISREG(st.st_mode) or st.st_uid != 0 or st.st_mode & 0o022 or dst.st_uid != 0 \
            or dst.st_mode & 0o022:
        return None
    return p


# ---------------------------------------------------------------------------
# large files
# ---------------------------------------------------------------------------

_mime_cache: dict[str, tuple[str, str]] = {}


def _mime_icon(name: str) -> tuple[str, str]:
    ext = os.path.splitext(name)[1].lower()
    hit = _mime_cache.get(ext)
    if hit:
        return hit
    mime = "application/octet-stream"
    try:
        import warnings
        with warnings.catch_warnings():
            warnings.simplefilter("ignore", DeprecationWarning)   # PyGObject override noise
            import gi
            gi.require_version("Gio", "2.0")
            from gi.repository import Gio
            ctype, _unc = Gio.content_type_guess(name, None)
            mime = Gio.content_type_get_mime_type(ctype) or mime
    except Exception:  # noqa: BLE001
        pass
    try:
        icon = first_existing_icon(mime_icon_names(mime), "text-x-generic") or "text-x-generic"
    except Exception:  # noqa: BLE001
        icon = "text-x-generic"
    _mime_cache[ext] = (mime, icon)
    return mime, icon


def large_files(scanner: Scanner, path: Any, limit: Any, min_size: Any) -> list[dict]:
    p = _check_dir(path)
    try:
        lim = max(1, min(500, int(limit if limit is not None else 50)))
        mn = max(0, int(min_size or 0))
    except (TypeError, ValueError):
        raise ApiError("bad_args", "Ongeldige invoer.") from None
    cached = scanner.large_for(p)
    heap = None
    if cached is not None:
        lst, complete = cached
        if len(lst) >= lim or (complete and mn >= 1048576):
            heap = lst
    if heap is None:
        found: list[tuple[int, str, float]] = []
        deadline = time.monotonic() + 4.0
        try:
            dev = os.stat(p).st_dev
        except OSError:
            raise ApiError("not_found", "Deze map bestaat niet.") from None
        stack = [p]
        n = 0
        while stack and time.monotonic() < deadline:
            d = stack.pop()
            try:
                it = os.scandir(d)
            except OSError:
                continue
            with it:
                for e in it:
                    try:
                        s = e.stat(follow_symlinks=False)
                    except OSError:
                        continue
                    if stat.S_ISDIR(s.st_mode):
                        if s.st_dev == dev and e.path not in _PSEUDO_SET:
                            stack.append(e.path)
                        continue
                    size = s.st_blocks * 512
                    if size < max(mn, 1):
                        continue
                    if len(found) < lim:
                        heapq.heappush(found, (size, e.path, s.st_mtime))
                    elif size > found[0][0]:
                        heapq.heapreplace(found, (size, e.path, s.st_mtime))
                    n += 1
                    if n > 300000:
                        stack.clear()
                        break
        heap = sorted(found, reverse=True)
    out = []
    for size, fp, mtime in heap:
        if size < mn:
            continue
        name = os.path.basename(fp)
        mime, icon = _mime_icon(name)
        out.append({"path": fp, "name": name, "size": size, "modified": int(mtime * 1000), "mime": mime,
                    "icon": icon})
        if len(out) >= lim:
            break
    return out


# ---------------------------------------------------------------------------
# mount watcher (volumes changed)
# ---------------------------------------------------------------------------

def _watch_mounts(emit: Callable, stop: threading.Event) -> None:
    try:
        f = open("/proc/self/mountinfo", "rb")
    except OSError:
        return
    poller = select.poll()
    poller.register(f.fileno(), select.POLLERR | select.POLLPRI)
    last = None
    with f:
        while not stop.is_set():
            ev = poller.poll(2000)
            if stop.is_set():
                return
            if not ev:
                continue
            f.seek(0)
            f.read()
            time.sleep(0.5)  # let udisks finish mounting
            try:
                vols = volumes()
            except Exception:  # noqa: BLE001
                continue
            sig = [(v["id"], v["mount"]) for v in vols]
            if sig != last:
                if last is not None:
                    emit("storage.volumesChanged", {"volumes": vols})
                last = sig


# ---------------------------------------------------------------------------
# registration
# ---------------------------------------------------------------------------

_state: dict[str, Any] = {}


def register(api, ctx):
    global _lang

    def lang() -> str:
        try:
            return "en" if ctx.settings.get("language", "nl") == "en" else "nl"
        except Exception:  # noqa: BLE001
            return "nl"

    _lang = lang
    scanner = Scanner(api.emit)
    cats = Categories(api.emit, Path(ctx.cache_dir) / "storage-categories.json")
    cleanup = Cleanup(own_thumbs=Path(ctx.cache_dir) / "thumbs")
    _state.update(scanner=scanner, cats=cats, cleanup=cleanup, stop=threading.Event(), trash=(0.0, None))
    vol_cache: dict[str, Any] = {"t": 0.0, "v": None, "all": None}

    @api.method("storage.volumes")
    def _volumes(args):
        """[{id, device, label, name, mount, fsType, total, used, free, removable, isSystem, isHome, kind}]"""
        include = bool(args.get("all"))
        key = "all" if include else "v"
        if vol_cache[key] is not None and time.monotonic() - vol_cache["t"] < 2:
            return vol_cache[key]
        res = volumes(include)
        vol_cache[key] = res
        vol_cache["t"] = time.monotonic()
        vol_cache["v" if key == "all" else "all"] = None
        return res

    @api.method("storage.scan")
    def _scan(args):
        """{path, maxDepth=4, topN=40, refresh} → {jobId}; events storage.scanProgress / storage.scanDone"""
        return scanner.start(args.get("path"), args.get("maxDepth"), args.get("topN"), bool(args.get("refresh")))

    @api.method("storage.scanCancel")
    def _scan_cancel(args):
        return scanner.cancel(args.get("jobId"))

    @api.method("storage.scanResult")
    def _scan_result(args):
        return scanner.result(args.get("jobId"))

    @api.method("storage.categories")
    def _categories(args):
        return cats.get(bool(args.get("refresh")))

    @api.method("storage.cleanup.list")
    def _cleanup_list(args):
        return cleanup.list(bool(args.get("refresh")))

    def _dry(args):
        ids = args.get("ids") if isinstance(args.get("ids"), list) else [args.get("ids")]
        return {"results": [{"id": i, "ok": True, "freed": 0, "error": None} for i in ids if i in _CLEANUP_IDS],
                "freed": 0}

    @api.method("storage.cleanup.run", dangerous=True, dry_result=_dry)
    def _cleanup_run(args):
        """{ids, paths?} → {results:[{id, ok, freed, error}], freed} (paths: chosen old downloads)"""
        res = cleanup.run(args.get("ids"), args.get("paths"))
        _state["trash"] = (0.0, None)
        ids = args.get("ids") if isinstance(args.get("ids"), list) else [args.get("ids")]
        if "trash" in ids or "old-downloads" in ids:
            api.emit("files.trashChanged", {})
        return res

    @api.method("storage.largeFiles")
    def _large(args):
        """{path, limit=50, minSize} → [{path, name, size, modified, mime, icon}]"""
        return large_files(scanner, args.get("path") or os.path.expanduser("~"), args.get("limit"),
                           args.get("minSize"))

    @api.method("storage.trashSize")
    def _trash_size(args):
        t, v = _state["trash"]
        if v is not None and time.monotonic() - t < 30 and not args.get("refresh"):
            return v
        v = trash_size()
        _state["trash"] = (time.monotonic(), v)
        return v


def start(api, ctx):
    stop_ev: threading.Event = _state["stop"]
    threading.Thread(target=_watch_mounts, args=(api.emit, stop_ev), name="storage-mounts", daemon=True).start()


def stop(api, ctx):
    ev = _state.get("stop")
    if ev:
        ev.set()
    sc: Scanner | None = _state.get("scanner")
    if sc:
        for j in list(sc.jobs.values()):
            j.cancel.set()
