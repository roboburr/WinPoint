"""index.* — Alles (SPEC §6.13): proxy to the indexer process ``host/indexer.py``.

The indexer runs as a separate, nice'd child process (JSON lines over its stdin/stdout)
so crawling millions of files never competes with the HTTP server for the GIL.

* Started lazily on the first ``index.*`` call, or right away in ``layer`` mode
  (``WIN11SHELL_INDEX_AUTOSTART=1`` forces it, ``=0`` disables it).
* Restarted when it dies (with back-off; gives up after 5 crashes in 5 minutes).
* Roots: ``settings.alles.roots`` (``"auto"`` = the SPEC default list) unless
  ``$WIN11SHELL_INDEX_ROOTS`` overrides them; ``alles.excluded`` and
  ``alles.liveUpdate`` are forwarded whenever they change.
* In the Rust GNOME port ``WIN11_GNOME_APP`` restricts ownership to ``alles``.
  Panel, resident Start, Desktop and other apps never start an index worker,
  even when an inherited environment forces autostart. Closing Alles stops its
  owned worker through the normal backend lifecycle. The original standalone
  host (without this GNOME role) retains its existing behavior.
* Events: ``index.status`` (throttled to 500 ms) and ``index.changed``.

Methods: ``index.status``, ``index.search``, ``index.rebuild``, ``index.setRoots`` and
(additive) ``index.locations`` — candidate drives/folders for the Alles options dialog.
"""

from __future__ import annotations

import json
import os
import re
import subprocess
import sys
import threading
import time
from collections import deque

from ..util import ApiError

PRIORITY = 60

_SEARCH_TIMEOUT = 20.0
_CALL_TIMEOUT = 8.0


class IndexerProcess:
    def __init__(self, api, ctx, log):
        self.api = api
        self.ctx = ctx
        # Separate GNOME hosts keep the original API, but only the visible Alles
        # application may own the expensive index. Start already falls back to
        # recent files when index.search is unavailable.
        app_role = os.environ.get("WIN11_GNOME_APP")
        self.allowed = app_role is None or app_role == "alles"
        self.log = log
        self.lock = threading.RLock()
        self.write_lock = threading.Lock()
        self.proc: subprocess.Popen | None = None
        self.pending: dict[int, dict] = {}
        self.next_id = 1
        self.crashes: deque = deque()
        self.stopping = False
        self.gave_up = False
        self.last_status: dict | None = None
        self._status_last_emit = 0.0
        self._status_timer: threading.Timer | None = None
        self._restart_timer: threading.Timer | None = None
        self.started_once = False
        self.backoff = 1.0

    # -- configuration --------------------------------------------------------------
    def config(self) -> dict:
        s = self.ctx.settings.get("alles", {}) or {}
        roots = self.ctx.index_roots if self.ctx.index_roots else s.get("roots", "auto")
        if isinstance(roots, list):
            roots = [str(r) for r in roots if isinstance(r, str) and r.strip()] or "auto"
        elif roots != "auto":
            roots = "auto"
        excluded = [str(p) for p in (s.get("excluded") or []) if isinstance(p, str) and p.strip()]
        return {"roots": roots, "excluded": excluded, "liveUpdate": bool(s.get("liveUpdate", True))}

    # -- process management -----------------------------------------------------------
    def running(self) -> bool:
        return self.proc is not None and self.proc.poll() is None

    def ensure(self) -> bool:
        with self.lock:
            if not self.allowed:
                return False
            if self.running():
                return True
            if self.stopping:
                return False
            if self.gave_up:
                return False
            return self._start()

    def _start(self) -> bool:
        if not self.allowed:
            return False
        cache = str(self.ctx.cache_dir)
        cmd = [sys.executable, "-m", "host.indexer", "--cache", cache]
        env = os.environ.copy()
        root = str(self.ctx.root)
        env["PYTHONPATH"] = root + (os.pathsep + env["PYTHONPATH"] if env.get("PYTHONPATH") else "")
        env.pop("WIN11SHELL_TOKEN", None)
        try:
            self.proc = subprocess.Popen(cmd, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                         stderr=subprocess.PIPE, cwd=root, env=env, close_fds=True)
        except OSError as e:
            self.log.error("Indexer kon niet starten (cannot start indexer): %s", e)
            self.proc = None
            return False
        self.started_once = True
        proc = self.proc
        threading.Thread(target=self._read_stdout, args=(proc,), name="index-out", daemon=True).start()
        threading.Thread(target=self._read_stderr, args=(proc,), name="index-err", daemon=True).start()
        self.log.info("Indexer gestart (indexer started), pid %s", proc.pid)
        self._send({"method": "configure", "args": self.config()})
        return True

    def _send(self, obj: dict) -> None:
        proc = self.proc
        if proc is None or proc.stdin is None:
            raise ApiError("index_unavailable", "Alles is nog niet klaar. Probeer het zo opnieuw.")
        data = (json.dumps(obj, ensure_ascii=True, separators=(",", ":")) + "\n").encode()
        with self.write_lock:
            try:
                proc.stdin.write(data)
                proc.stdin.flush()
            except (BrokenPipeError, OSError, ValueError):
                raise ApiError("index_unavailable", "Alles wordt opnieuw gestart. Probeer het zo nog eens.") from None

    def call(self, method: str, args: dict | None = None, timeout: float = _CALL_TIMEOUT):
        if not self.ensure():
            raise ApiError("index_unavailable", "Open Alles om bestanden te doorzoeken." if not self.allowed
                           else "Alles is op dit moment niet beschikbaar.")
        with self.lock:
            rid = self.next_id
            self.next_id += 1
            slot = {"ev": threading.Event(), "result": None, "error": None}
            self.pending[rid] = slot
        try:
            self._send({"id": rid, "method": method, "args": args or {}})
            if not slot["ev"].wait(timeout):
                if method == "search":
                    self._unwedge()
                raise ApiError("timeout", "Dit duurt te lang. Probeer het opnieuw.")
        finally:
            with self.lock:
                self.pending.pop(rid, None)
        err = slot["error"]
        if err is not None:
            raise ApiError(str(err.get("code") or "internal"),
                           str(err.get("message") or "Er ging iets mis bij het zoeken."))
        return slot["result"]

    def _unwedge(self) -> None:
        """A search did not answer in time. The indexer has ONE search thread, so a search stuck inside C
        (a pathological pattern) would block every later search until the next login: restart the
        indexer (it reloads its cache in about a second; _on_exit schedules the restart)."""
        proc = self.proc
        if proc is None or proc.poll() is not None:
            return
        self.log.warning("Zoekopdracht hangt; indexer wordt herstart (search stuck, restarting indexer)")
        try:
            proc.kill()
        except OSError:
            pass

    def _read_stdout(self, proc: subprocess.Popen) -> None:
        out = proc.stdout
        try:
            for line in out:
                line = line.strip()
                if not line:
                    continue
                try:
                    msg = json.loads(line)
                except ValueError:
                    self.log.debug("indexer: %s", line[:200])
                    continue
                if not isinstance(msg, dict):
                    continue
                if "event" in msg:
                    self._on_event(str(msg.get("event")), msg.get("data") or {})
                    continue
                rid = msg.get("id")
                with self.lock:
                    slot = self.pending.get(rid)
                if slot is None:
                    continue
                if msg.get("ok"):
                    slot["result"] = msg.get("result")
                else:
                    slot["error"] = msg.get("error") or {}
                slot["ev"].set()
        except (OSError, ValueError):
            pass
        self._on_exit(proc)

    def _read_stderr(self, proc: subprocess.Popen) -> None:
        try:
            for line in proc.stderr:
                text = line.decode("utf-8", "replace").rstrip()
                if text:
                    self.log.info("%s", text[:500])
        except (OSError, ValueError):
            pass

    def _on_exit(self, proc: subprocess.Popen) -> None:
        try:
            proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            pass
        with self.lock:
            if self.proc is not proc:
                return
            self.proc = None
            pending = list(self.pending.values())
            self.pending.clear()
        for slot in pending:
            slot["error"] = {"code": "index_restarting", "message": "Alles wordt opnieuw gestart. Probeer het zo nog eens."}
            slot["ev"].set()
        if self.stopping:
            return
        rc = proc.returncode
        self.log.warning("Indexer gestopt (indexer exited) met code %s", rc)
        now = time.monotonic()
        self.crashes.append(now)
        while self.crashes and now - self.crashes[0] > 300:
            self.crashes.popleft()
        if len(self.crashes) >= 5:
            self.gave_up = True
            self.log.error("Indexer crasht steeds; Alles is uitgeschakeld tot de volgende herstart")
            self._emit_status({"state": "error", "files": 0, "folders": 0, "roots": [], "builtAt": None,
                               "progress": None, "watching": 0,
                               "message": "Alles kon het overzicht van je bestanden niet bijhouden."})
            return
        delay = min(30.0, self.backoff)
        self.backoff = min(30.0, self.backoff * 2)
        t = threading.Timer(delay, self._restart)
        t.daemon = True
        self._restart_timer = t
        t.start()

    def _restart(self):
        if not self.stopping and not self.running():
            if self.ensure():
                self.backoff = max(1.0, self.backoff / 2)

    def reconfigure(self) -> None:
        if self.running():
            try:
                self._send({"method": "configure", "args": self.config()})
            except ApiError:
                pass

    # -- events ---------------------------------------------------------------------------
    def _on_event(self, name: str, data) -> None:
        if name == "status":
            self.last_status = data
            self._emit_status(data)
        elif name == "changed":
            self.api.emit("index.changed", data if isinstance(data, dict) else {})

    def _emit_status(self, data) -> None:
        now = time.monotonic()
        wait = 0.5 - (now - self._status_last_emit)
        if wait <= 0:
            self._status_last_emit = now
            self.api.emit("index.status", data)
            return
        if self._status_timer is None or not self._status_timer.is_alive():
            def fire():
                self._status_timer = None
                self._status_last_emit = time.monotonic()
                if self.last_status is not None:
                    self.api.emit("index.status", self.last_status)
            t = threading.Timer(wait, fire)
            t.daemon = True
            self._status_timer = t
            t.start()

    # -- shutdown -------------------------------------------------------------------------
    def stop(self) -> None:
        self.stopping = True
        for t in (self._status_timer, self._restart_timer):
            if t is not None:
                t.cancel()
        proc = self.proc
        if proc is None or proc.poll() is not None:
            return
        try:
            self._send({"id": 0, "method": "shutdown"})
            proc.stdin.close()
        except (ApiError, OSError, ValueError):
            pass
        try:
            proc.wait(timeout=6)
        except subprocess.TimeoutExpired:
            proc.terminate()
            try:
                proc.wait(timeout=2)
            except subprocess.TimeoutExpired:
                proc.kill()


def _removable(source: str) -> bool:
    """USB / removable media (sysfs), not just "mounted under /run/media"."""
    from ..indexer import _disk_of
    disk = _disk_of(source)
    if not disk:
        return False
    try:
        with open(f"/sys/block/{disk}/removable") as f:
            if f.read().strip() == "1":
                return True
    except OSError:
        pass
    try:
        return "/usb" in os.path.realpath(f"/sys/block/{disk}")
    except OSError:
        return False


def _fs_labels() -> dict:
    """device (realpath) → filesystem label, from /dev/disk/by-label."""
    out = {}
    d = "/dev/disk/by-label"
    try:
        for name in os.listdir(d):
            try:
                dev = os.path.realpath(os.path.join(d, name))
            except OSError:
                continue
            # udev escapes spaces etc. as \x20
            out[dev] = re.sub(r"\\x([0-9a-fA-F]{2})", lambda m: chr(int(m.group(1), 16)), name)
    except OSError:
        pass
    return out


def _size_label(total: int) -> str:
    if total >= 1 << 40:
        v = f"{total / (1 << 40):.1f}".replace(".", ",")
        return (v[:-2] if v.endswith(",0") else v) + " TB"
    return f"{round(total / (1 << 30))} GB"


def _friendly_label(mount: str, source: str, total: int, labels: dict) -> str:
    """"Systeemschijf", the filesystem label, the folder name, or "564 GB-schijf" for UUID-named mounts."""
    if mount == "/":
        return "Systeemschijf"
    lab = None
    if source.startswith("/dev/"):
        try:
            lab = labels.get(os.path.realpath(source))
        except OSError:
            lab = None
    if lab:
        return lab
    base = os.path.basename(mount.rstrip("/")) or mount
    if re.fullmatch(r"[0-9A-Fa-f-]{8,}", base) and total:
        return f"{_size_label(total)}-schijf"
    return base


def _locations(ctx) -> list[dict]:
    """Drives/folders that can be indexed (for the options dialog)."""
    from ..indexer import REAL_FS, auto_roots, read_mounts
    mounts = read_mounts()
    auto = set(auto_roots(mounts))
    s = ctx.settings.get("alles", {}) or {}
    roots = ctx.index_roots or s.get("roots", "auto")
    active = auto if roots == "auto" or not isinstance(roots, list) else {os.path.realpath(os.path.expanduser(r)) for r in roots}
    out = []
    seen = set()
    home = os.path.realpath(os.path.expanduser("~"))
    labels = _fs_labels()
    for m in mounts:
        mp, fs = m["mount"], m["fs"]
        if mp in seen or fs not in REAL_FS:
            continue
        if mp != "/" and mp not in auto:
            continue
        seen.add(mp)
        try:
            st = os.statvfs(mp)
            total = st.f_blocks * st.f_frsize
            free = st.f_bavail * st.f_frsize
        except OSError:
            total = free = 0
        label = _friendly_label(mp, m.get("source", ""), total, labels)
        out.append({"path": mp, "label": label, "fsType": fs, "total": total, "free": free,
                    "auto": mp in auto, "indexed": mp in active, "kind": "drive",
                    "removable": _removable(m.get("source", ""))})
    # explicit folder roots that are not mount points
    if isinstance(roots, list):
        for r in roots:
            p = os.path.realpath(os.path.expanduser(r))
            if p in seen:
                continue
            seen.add(p)
            out.append({"path": p, "label": os.path.basename(p) or p, "fsType": "", "total": 0, "free": 0,
                        "auto": False, "indexed": True, "kind": "folder", "removable": False,
                        "isHome": p == home})
    return out


def register(api, ctx) -> None:
    log = ctx.logger("index")
    ix = IndexerProcess(api, ctx, log)
    ctx._indexer = ix

    relevant = ("alles.roots", "alles.excluded", "alles.liveUpdate")

    def _settings_changed(settings, path=None, changed=None):
        paths = [str(p) for p in (changed or ([path] if path else []))]
        if not paths or any(p == "alles" or p.startswith(relevant) for p in paths):
            ix.reconfigure()

    try:
        ctx.settings.on_change(_settings_changed)
    except Exception:  # pragma: no cover - settings store always has on_change
        log.warning("Kan instellingen niet volgen (cannot watch settings)")

    @api.method("index.status")
    def _status(args):
        """Index state, counts, roots, progress."""
        try:
            st = ix.call("status")
        except ApiError as e:
            if e.code in ("index_unavailable", "index_restarting", "timeout"):
                st = dict(ix.last_status or {"state": "building" if ix.running() else "error", "files": 0,
                                             "folders": 0, "roots": [], "builtAt": None, "progress": None,
                                             "watching": 0})
                st["message"] = e.message
                return st
            raise
        ix.last_status = st
        return st

    @api.method("index.search")
    def _search(args):
        """Everything-style search: {query, offset, limit, sort, desc, filter, matchCase, matchPath, regex, wholeWord}."""
        a = dict(args or {})
        if "showHidden" not in a:
            a["showHidden"] = bool(ctx.settings.get("alles.showHidden", False))
        try:
            a["limit"] = max(0, min(1000, int(a.get("limit", 200))))
            a["offset"] = max(0, int(a.get("offset", 0)))
        except (TypeError, ValueError):
            raise ApiError("bad_args", "Ongeldige invoer.") from None
        if not isinstance(a.get("query", ""), str):
            raise ApiError("bad_args", "Ongeldige zoekopdracht.")
        return ix.call("search", a, timeout=_SEARCH_TIMEOUT)

    @api.method("index.rebuild")
    def _rebuild(args):
        """Rebuild the whole index (runs in the background)."""
        ix.call("rebuild")
        return {"ok": True}

    @api.method("index.setRoots")
    def _set_roots(args):
        """{roots: 'auto' | [paths], excluded?: [paths]} → persisted in settings.alles."""
        roots = args.get("roots", "auto")
        if roots != "auto":
            if not isinstance(roots, list) or not all(isinstance(r, str) for r in roots):
                raise ApiError("bad_args", "Kies een of meer mappen om te doorzoeken.")
            clean = []
            for r in roots:
                p = os.path.realpath(os.path.expanduser(r.strip()))
                if not os.path.isdir(p):
                    raise ApiError("not_found", f"De map “{r}” bestaat niet.")
                if p not in clean:
                    clean.append(p)
            if not clean:
                raise ApiError("bad_args", "Kies minstens één locatie om te doorzoeken.")
            roots = clean
        patch = {"alles": {"roots": roots}}
        if "excluded" in args:
            ex = args.get("excluded")
            if not isinstance(ex, list) or not all(isinstance(p, str) for p in ex):
                raise ApiError("bad_args", "Ongeldige lijst met uitgesloten mappen.")
            patch["alles"]["excluded"] = [os.path.normpath(os.path.expanduser(p.strip())) for p in ex if p.strip()]
        ctx.settings.patch(patch)
        ix.reconfigure()
        if ctx.index_roots:
            return {"ok": True, "overridden": True, "message": "De mappen worden bepaald door WIN11SHELL_INDEX_ROOTS."}
        return {"ok": True}

    @api.method("index.locations")
    def _locations_m(args):
        """Drives and folders that can be indexed, with sizes and whether they are indexed now."""
        return _locations(ctx)


def start(api, ctx) -> None:
    ix = getattr(ctx, "_indexer", None)
    if ix is None or not ix.allowed:
        return
    flag = os.environ.get("WIN11SHELL_INDEX_AUTOSTART")
    if flag == "1" or (flag != "0" and ctx.mode == "layer"):
        ctx.thread(ix.ensure, name="index-autostart")


def stop(api, ctx) -> None:
    ix = getattr(ctx, "_indexer", None)
    if ix is not None:
        ix.stop()
