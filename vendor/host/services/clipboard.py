"""clipboard.* (SPEC §6.9): clipboard history (Win+V) via ``wl-paste --watch``.

Needs a compositor with the wlr/ext data-control protocol (Hyprland: yes; GNOME: no →
``supported:false`` and wl-paste is never started there, so no focus-stealing helper window).

* Watcher: ``wl-paste --watch sh -c 'cat >/dev/null; printf "%s\\n" "${CLIPBOARD_STATE:-data}"'``
  prints one line per clipboard change; the content is then read with ``wl-paste --list-types`` and
  ``wl-paste -n --type text`` / ``--type image/png``.
* History: up to 25 items (pinned ones are kept on top of that and survive restarts in
  ``<config>/clipboard-pinned.json``), text ≤ 64 KB, images as PNG data URLs ≤ 1 MB.
  Password-manager copies (``x-kde-passwordManagerHint`` / ``CLIPBOARD_STATE=sensitive``) are never stored.
* Item = {id, kind:'text'|'image', text?, preview?, time, pinned} + ``size``, ``mime`` (images).
* ``clipboard.paste {id, insert?}`` puts the item back on the clipboard (wl-copy; skipped in
  --dry-run); ``insert:true`` also sends Ctrl+V to the active window (Hyprland ``sendshortcut``).
* event ``clipboard.changed {}``.
"""

from __future__ import annotations

import base64
import hashlib
import logging
import os
import subprocess
import threading
import time
import uuid

from ..util import ApiError, atomic_write_json, dry_for, read_json, run, which
from .windows import Hypr

log = logging.getLogger("win11shell.clipboard")

PRIORITY = 45

MAX_ITEMS = 25
MAX_TEXT = 64 * 1024
MAX_IMAGE = 1024 * 1024
MAX_PINNED = 25
_TEXT_TYPES = ("text/plain;charset=utf-8", "UTF8_STRING", "text/plain", "STRING", "TEXT")
_SECRET_TYPES = ("x-kde-passwordManagerHint",)
WATCH_SCRIPT = 'cat >/dev/null; printf "%s\\n" "${CLIPBOARD_STATE:-data}"'


PREVIEW_MAX = 96 * 1024        # larger images get a small preview for clipboard.list (full data kept apart)
PREVIEW_SIDE = 480
_PREVIEW_MIMES = ("image/png", "image/jpeg", "image/gif", "image/webp")


def _digest(kind: str, data: bytes) -> str:
    return kind + ":" + hashlib.sha1(data).hexdigest()


def _small_preview(data: bytes, mime: str) -> str | None:
    """PNG/JPEG data URL of at most PREVIEW_SIDE px for a big clipboard image (common formats only,
    decoded in-process like the host's own thumbnails), or None."""
    if mime not in _PREVIEW_MIMES:
        return None
    try:
        import gi
        gi.require_version("GdkPixbuf", "2.0")
        from gi.repository import GdkPixbuf
        loader = GdkPixbuf.PixbufLoader()
        loader.write(data)
        loader.close()
        pb = loader.get_pixbuf()
        if pb is None:
            return None
        w, h = pb.get_width(), pb.get_height()
        scale = min(1.0, PREVIEW_SIDE / max(w, h, 1))
        if scale < 1.0:
            pb = pb.scale_simple(max(1, int(w * scale)), max(1, int(h * scale)), GdkPixbuf.InterpType.BILINEAR)
        if pb.get_has_alpha():
            ok, out = pb.save_to_bufferv("png", [], [])
            kind = "image/png"
        else:
            ok, out = pb.save_to_bufferv("jpeg", ["quality"], ["85"])
            kind = "image/jpeg"
        if not ok:
            return None
        return f"data:{kind};base64," + base64.b64encode(bytes(out)).decode("ascii")
    except Exception:  # noqa: BLE001
        return None


class History:
    """In-memory clipboard history (thread-safe)."""

    def __init__(self, pinned_file=None):
        self.lock = threading.RLock()
        self.items: list[dict] = []        # newest first
        self.pinned_file = pinned_file
        self._load_pinned()

    def _load_pinned(self) -> None:
        if not self.pinned_file:
            return
        data = read_json(self.pinned_file, [])
        if not isinstance(data, list):
            return
        for it in data[:MAX_PINNED]:
            if isinstance(it, dict) and it.get("kind") in ("text", "image") and it.get("id"):
                it["pinned"] = True
                if it["kind"] == "text":
                    raw = (it.get("text") or "").encode("utf-8")
                else:
                    try:
                        raw = base64.b64decode((it.get("data") or it.get("preview") or "").partition(",")[2])
                    except ValueError:
                        continue
                it["digest"] = _digest(it["kind"], raw)
                self.items.append(it)

    def _save_pinned(self) -> None:
        if not self.pinned_file:
            return
        pinned = [{k: v for k, v in it.items()} for it in self.items if it.get("pinned")]
        try:
            atomic_write_json(self.pinned_file, pinned, pretty=False)
        except OSError as e:
            log.warning("Vastgemaakte klembord-items niet opgeslagen (cannot save pinned): %s", e)

    def public(self) -> list[dict]:
        """Items for clipboard.list: big images only as a small preview (the flyout asks on every change)."""
        with self.lock:
            return [{k: v for k, v in it.items() if k not in ("digest", "raw", "data")} for it in self.items]

    def get(self, cid: str) -> dict | None:
        with self.lock:
            return next((it for it in self.items if it["id"] == cid), None)

    def add(self, kind: str, *, text: str | None = None, png: bytes | None = None, mime: str = "image/png") -> dict:
        if kind == "text":
            raw = (text or "").encode("utf-8")
        else:
            raw = png or b""
        dig = _digest(kind, raw if kind == "image" else (text or "").encode("utf-8"))
        with self.lock:
            for i, it in enumerate(self.items):
                if it.get("digest") == dig:          # already known: move to the top
                    it["time"] = int(time.time() * 1000)
                    self.items.insert(0, self.items.pop(i))
                    return it
            it = {"id": uuid.uuid4().hex[:10], "kind": kind, "time": int(time.time() * 1000), "pinned": False,
                  "digest": dig, "size": len(raw)}
            if kind == "text":
                it["text"] = text
            else:
                full = f"data:{mime};base64," + base64.b64encode(raw).decode("ascii")
                small = _small_preview(raw, mime) if len(raw) > PREVIEW_MAX else None
                if small is not None and len(small) < len(full):
                    it["preview"] = small
                    it["data"] = full            # what clipboard.paste puts back
                else:
                    it["preview"] = full
                it["mime"] = mime
            self.items.insert(0, it)
            unpinned = [x for x in self.items if not x.get("pinned")]
            while len(unpinned) > MAX_ITEMS:
                victim = unpinned.pop()
                self.items.remove(victim)
            return it

    def remove(self, cid: str) -> bool:
        with self.lock:
            it = self.get(cid)
            if it is None:
                return False
            self.items.remove(it)
            if it.get("pinned"):
                self._save_pinned()
            return True

    def clear(self) -> int:
        """Remove every non-pinned item (like Win11 "Alles wissen")."""
        with self.lock:
            before = len(self.items)
            self.items = [it for it in self.items if it.get("pinned")]
            return before - len(self.items)

    def pin(self, cid: str, pinned: bool) -> dict:
        with self.lock:
            it = self.get(cid)
            if it is None:
                raise ApiError("not_found", "Dit item staat niet meer in het klembord.")
            if pinned and not it.get("pinned") and sum(1 for x in self.items if x.get("pinned")) >= MAX_PINNED:
                raise ApiError("too_many", f"Je kunt maximaal {MAX_PINNED} items vastmaken.")
            it["pinned"] = bool(pinned)
            self._save_pinned()
            return it

    def raw(self, it: dict) -> tuple[bytes, str]:
        if it["kind"] == "text":
            return (it.get("text") or "").encode("utf-8"), "text/plain;charset=utf-8"
        prev = it.get("data") or it.get("preview") or ""
        head, _, b64 = prev.partition(",")
        mime = head[5:].split(";")[0] if head.startswith("data:") else "image/png"
        return base64.b64decode(b64 or b""), mime


class ClipboardService:
    def __init__(self, api, ctx):
        self.api = api
        self.ctx = ctx
        self.history = History(ctx.config_dir / "clipboard-pinned.json")
        self.supported = False
        self.reason = ""
        self.proc: subprocess.Popen | None = None
        self._stop = threading.Event()
        self._thread: threading.Thread | None = None

    # -- detection -----------------------------------------------------------------
    def detect(self) -> bool:
        if not os.environ.get("WAYLAND_DISPLAY"):
            self.reason = "no-wayland"
            return False
        if not which("wl-paste") or not which("wl-copy"):
            self.reason = "no-wl-clipboard"
            return False
        if self.ctx.compositor == "gnome":
            self.reason = "gnome"     # no data-control protocol; never start wl-paste here
            return False
        if self.ctx.compositor == "hyprland":
            return True
        r = run(["wl-paste", "--list-types"], timeout=1.5)
        err = (r.err or "").lower()
        if r.rc == 124 or "data-control" in err or "not support" in err or "focus" in err:
            self.reason = "no-data-control"
            return False
        return True

    # -- reading ---------------------------------------------------------------------
    @staticmethod
    def _read(args: list[str], limit: int, timeout: float = 3.0) -> bytes | None:
        """Clipboard content, or None when it is larger than ``limit`` — never more than limit+1 bytes are
        read (copying a 200 MB image must not make the desktop process buffer it)."""
        try:
            p = subprocess.Popen(["wl-paste", "--no-newline"] + args, stdout=subprocess.PIPE,
                                 stderr=subprocess.DEVNULL, stdin=subprocess.DEVNULL, close_fds=True)
        except OSError:
            return None
        timer = threading.Timer(timeout, p.kill)
        timer.daemon = True
        timer.start()
        try:
            assert p.stdout is not None
            chunks, n = [], 0
            while n <= limit:
                b = p.stdout.read(min(65536, limit + 1 - n))
                if not b:
                    break
                chunks.append(b)
                n += len(b)
            if n > limit:
                p.kill()
                return None
            rc = p.wait(timeout=timeout)
        except (OSError, subprocess.TimeoutExpired, ValueError):
            p.kill()
            return None
        finally:
            timer.cancel()
            try:
                p.stdout.close()
            except (OSError, AttributeError):
                pass
        return b"".join(chunks) if rc == 0 else None

    def on_change(self, state: str) -> None:
        state = (state or "data").strip()
        if state in ("nil", "clear", "sensitive"):
            return
        r = run(["wl-paste", "--list-types"], timeout=2)
        if r.rc != 0:
            return
        types = [t.strip() for t in r.out.splitlines() if t.strip()]
        self.ingest(types, self._read)

    def ingest(self, types: list[str], reader) -> dict | None:
        """Store the current clipboard given its MIME ``types``; ``reader(args, limit)`` → bytes|None."""
        if any(t in _SECRET_TYPES for t in types):
            log.debug("Wachtwoord gekopieerd: niet bewaard (password copy not stored)")
            return None
        item = None
        img = next((t for t in types if t == "image/png"), None) or next((t for t in types if t.startswith("image/")
                                                                          and t != "image/svg+xml"), None)
        if img:
            data = reader(["--type", img], MAX_IMAGE)
            if data:
                item = self.history.add("image", png=data, mime=img)
        if item is None and any(t in _TEXT_TYPES or t.startswith("text/") for t in types):
            data = reader(["--type", "text"], MAX_TEXT)
            if data:
                text = data.decode("utf-8", "replace")
                if text.strip():
                    item = self.history.add("text", text=text)
        if item is not None:
            self.api.emit("clipboard.changed", {})
        return item

    def _watch_loop(self) -> None:
        backoff = 1.0
        while not self._stop.is_set():
            try:
                self.proc = subprocess.Popen(["wl-paste", "--watch", "sh", "-c", WATCH_SCRIPT],
                                             stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
                                             stdin=subprocess.DEVNULL, close_fds=True)
            except OSError as e:
                log.warning("wl-paste start mislukt (failed): %s", e)
                self.supported = False
                return
            started = time.monotonic()
            assert self.proc.stdout is not None
            for line in self.proc.stdout:
                if self._stop.is_set():
                    break
                try:
                    self.on_change(line.decode("utf-8", "replace"))
                except Exception:  # noqa: BLE001
                    log.exception("Klembord lezen mislukt (clipboard read failed)")
            try:
                self.proc.wait(timeout=2)
            except subprocess.TimeoutExpired:
                pass
            if self._stop.is_set():
                break
            if time.monotonic() - started > 30:
                backoff = 1.0
            log.info("wl-paste --watch gestopt; opnieuw over %.0f s (restarting)", backoff)
            self._stop.wait(backoff)
            backoff = min(60.0, backoff * 2)

    def start(self) -> None:
        self.supported = self.detect()
        if not self.supported:
            log.info("Klembordgeschiedenis niet beschikbaar (clipboard history unsupported): %s", self.reason)
            return
        self._thread = threading.Thread(target=self._watch_loop, name="clipboard-watch", daemon=True)
        self._thread.start()

    def stop(self) -> None:
        self._stop.set()
        p = self.proc
        if p is not None and p.poll() is None:
            try:
                p.terminate()
                p.wait(timeout=2)
            except (OSError, subprocess.TimeoutExpired):
                try:
                    p.kill()
                except OSError:
                    pass

    # -- paste -------------------------------------------------------------------------
    def paste(self, it: dict, insert: bool) -> dict:
        data, mime = self.history.raw(it)
        if dry_for(self.ctx):
            log.info("[dry-run] clipboard.paste %s overgeslagen (skipped)", it["id"])
            return {"ok": True, "dryRun": True}
        try:
            p = subprocess.run(["wl-copy", "--type", mime], input=data, capture_output=True, timeout=3)
        except (OSError, subprocess.TimeoutExpired) as e:
            raise ApiError("failed", "Kopiëren naar het klembord lukte niet.", str(e)) from None
        if p.returncode != 0:
            raise ApiError("failed", "Kopiëren naar het klembord lukte niet.", p.stderr.decode("utf-8", "replace")[:200])
        with self.history.lock:   # move to the top
            if it in self.history.items:
                self.history.items.remove(it)
                self.history.items.insert(0, it)
                it["time"] = int(time.time() * 1000)
        if insert and self.ctx.compositor == "hyprland":
            def send():
                h = Hypr()
                act = h.json("activewindow")
                addr = act.get("address") if isinstance(act, dict) else None
                if addr:
                    h.dispatch(f"sendshortcut CTRL, V, address:{addr}")
            t = threading.Timer(0.25, send)
            t.daemon = True
            t.start()
        self.api.emit("clipboard.changed", {})
        return {"ok": True}


def register(api, ctx) -> None:
    svc = ClipboardService(api, ctx)
    ctx._clipboard_service = svc

    def item_of(args) -> dict:
        cid = args.get("id")
        it = svc.history.get(cid) if isinstance(cid, str) else None
        if it is None:
            raise ApiError("not_found", "Dit item staat niet meer in het klembord.")
        return it

    @api.method("clipboard.list")
    def _list(args):
        """{supported, items} (newest first)."""
        return {"supported": svc.supported, "items": svc.history.public() if svc.supported else []}

    @api.method("clipboard.paste")
    def _paste(args):
        """Put an item back on the clipboard (``insert:true`` also pastes it into the active app)."""
        if not svc.supported:
            return {"supported": False}
        return svc.paste(item_of(args), bool(args.get("insert")))

    @api.method("clipboard.remove")
    def _remove(args):
        if not svc.supported:
            return {"supported": False}
        ok = svc.history.remove(args.get("id"))
        if ok:
            api.emit("clipboard.changed", {})
        return {"ok": ok}

    @api.method("clipboard.clear")
    def _clear(args):
        """Remove everything except pinned items."""
        if not svc.supported:
            return {"supported": False}
        n = svc.history.clear()
        api.emit("clipboard.changed", {})
        return {"removed": n}

    @api.method("clipboard.pin")
    def _pin(args):
        if not svc.supported:
            return {"supported": False}
        it = svc.history.pin(item_of(args)["id"], bool(args.get("pinned", True)))
        api.emit("clipboard.changed", {})
        return {"id": it["id"], "pinned": it["pinned"]}


def start(api, ctx) -> None:
    svc = getattr(ctx, "_clipboard_service", None)
    if svc is not None:
        svc.start()


def stop(api, ctx) -> None:
    svc = getattr(ctx, "_clipboard_service", None)
    if svc is not None:
        svc.stop()
