"""HTTP server (SPEC §3): 127.0.0.1 only, token protected.

Routes
  GET  /                       → 302 /ui/index.html?surface=preview&token=… (token only for our own uid)
  GET  /ui/*                   static UI (public, Cache-Control: no-store)
  POST /api/call/<method>      {"args": {...}} → {"ok": true, "result": …} | {"ok": false, "error": …}
  GET  /api/events             Server-Sent Events (hello on connect, keep-alive every 15 s)
  GET  /api/methods            registered method names (debug)
  GET  /icon?name=|mime=&size= themed icon (svg/png; xpm converted)
  GET  /thumb?path=&size=      image thumbnail (GdkPixbuf, disk cache)
  GET  /file?path=             image files only (403 otherwise)
Protected routes need header ``X-Shell-Token`` or ``?token=``.
"""

from __future__ import annotations

import collections
import hashlib
import hmac
import logging
import os
import queue
import re
import shutil
import stat
import struct
import subprocess
import threading
import time
from http import HTTPStatus
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, quote, unquote, urlsplit

from . import VERSION
from .util import BROWSER_IMAGE_MIMES, atomic_write_bytes, find_icon, find_mime_icon, image_mime, json_dumps, \
    json_loads

log = logging.getLogger("win11shell.server")

__all__ = ["ShellServer"]

MAX_BODY = 16 * 1024 * 1024
MAX_THUMB_SOURCE = 256 * 1024 * 1024
# /icon?name=<absolute path> outside the system icon dirs: tray / notification apps choose these files
MAX_USER_ICON = 2 * 1024 * 1024
MAX_USER_ICON_PX = 1024
THUMB_BUCKETS = (64, 128, 256, 512, 1024)

STATIC_MIME = {
    ".html": "text/html; charset=utf-8", ".htm": "text/html; charset=utf-8",
    ".js": "text/javascript; charset=utf-8", ".mjs": "text/javascript; charset=utf-8",
    ".css": "text/css; charset=utf-8", ".json": "application/json; charset=utf-8",
    ".map": "application/json; charset=utf-8", ".svg": "image/svg+xml", ".png": "image/png",
    ".jpg": "image/jpeg", ".jpeg": "image/jpeg", ".gif": "image/gif", ".webp": "image/webp",
    ".avif": "image/avif", ".ico": "image/vnd.microsoft.icon", ".woff2": "font/woff2", ".woff": "font/woff",
    ".ttf": "font/ttf", ".otf": "font/otf", ".txt": "text/plain; charset=utf-8",
    ".md": "text/markdown; charset=utf-8", ".wasm": "application/wasm", ".mp3": "audio/mpeg",
    ".ogg": "audio/ogg", ".oga": "audio/ogg", ".wav": "audio/wav", ".webm": "video/webm", ".mp4": "video/mp4",
}

# Image responses are never documents: block scripts if someone navigates to one.
IMAGE_CSP = "default-src 'none'; style-src 'unsafe-inline'; img-src data:; sandbox"
# UI pages: only our own pages may frame them (preview WM); nothing else is restricted here.
PAGE_CSP = "frame-ancestors 'self'"

_METHOD_RE = re.compile(r"^[A-Za-z0-9_.\-]{1,100}$")


# ---------------------------------------------------------------------------
# Image helpers (GdkPixbuf lazily)
# ---------------------------------------------------------------------------

_pixbuf_mod = None
_pixbuf_failed = False
_thumb_sem = threading.BoundedSemaphore(2)
_conv_cache: collections.OrderedDict = collections.OrderedDict()
_conv_lock = threading.Lock()
_conv_bytes = 0
_CONV_CACHE_MAX = 64 * 1024 * 1024        # converted PNGs kept in memory (bytes, not entries)
_CONV_ENTRY_MAX = 8 * 1024 * 1024


def _gdkpixbuf():
    global _pixbuf_mod, _pixbuf_failed
    if _pixbuf_mod is None and not _pixbuf_failed:
        try:
            import gi
            gi.require_version("GdkPixbuf", "2.0")
            from gi.repository import GdkPixbuf
            _pixbuf_mod = GdkPixbuf
        except Exception as e:  # noqa: BLE001
            _pixbuf_failed = True
            log.warning("GdkPixbuf niet beschikbaar (not available): %s", e)
    return _pixbuf_mod


# Formats whose GdkPixbuf loaders are decoded inside the host process (widely used, heavily fuzzed).
# Everything else (TIFF, HEIC/AVIF, JXL, RAW, TGA, ICO, XPM, EXR, DDS, …) from user files is decoded by
# host/thumbnailer.py in a sandboxed child process: a crash there cannot take the taskbar down.
INPROC_MIMES = frozenset({"image/png", "image/jpeg", "image/gif", "image/webp"})
_SYSTEM_IMAGE_DIRS = ("/usr/share/", "/usr/local/share/", "/var/lib/flatpak/exports/", "/var/lib/snapd/desktop/",
                      "/snap/")
_THUMBNAILER = Path(__file__).resolve().parent / "thumbnailer.py"
_bwrap_ok: bool | None = None


def _bwrap_cmd(src: str | None = None) -> list[str]:
    """bwrap prefix for the decoder (read-only root, private /tmp, no network/IPC, new pid ns), or [] if
    unusable. ``src`` is made visible when it lives in /tmp (GdkPixbuf/glycin needs a writable /tmp)."""
    global _bwrap_ok
    exe = shutil.which("bwrap")
    if not exe or os.environ.get("WIN11SHELL_NO_BWRAP") == "1":
        return []
    cmd = [exe, "--ro-bind", "/", "/", "--dev", "/dev", "--proc", "/proc", "--tmpfs", "/tmp"]
    if src and src.startswith("/tmp/"):
        cmd += ["--ro-bind", src, src]
    cmd += ["--unshare-all", "--die-with-parent", "--new-session", "--"]
    if _bwrap_ok is None:
        try:
            r = subprocess.run(cmd + ["/bin/true"], capture_output=True, timeout=5)
            _bwrap_ok = r.returncode == 0
        except (OSError, subprocess.TimeoutExpired):
            _bwrap_ok = False
        if not _bwrap_ok:
            log.info("bwrap niet bruikbaar; afbeeldingen worden zonder sandbox (wel apart) gedecodeerd "
                     "(bwrap unusable, decoding in a plain child process)")
    return cmd if _bwrap_ok else []


def decode_isolated(path: str, size: int | None, jpeg_ok: bool = False) -> tuple[bytes, str] | None:
    """Decode ``path`` in the sandboxed helper → (data, 'image/png'|'image/jpeg') or None."""
    import sys
    real = os.path.realpath(path)
    cmd = _bwrap_cmd(real) + [sys.executable, "-I", str(_THUMBNAILER), real, str(int(size or 0)),
                              "jpeg" if jpeg_ok else "png"]
    try:
        with _thumb_sem:
            r = subprocess.run(cmd, stdin=subprocess.DEVNULL, capture_output=True, timeout=20, close_fds=True)
    except (OSError, subprocess.TimeoutExpired) as e:
        log.info("Afbeelding niet te lezen (image decoder failed) %s: %s", path, e)
        return None
    out = r.stdout
    if r.returncode != 0 or len(out) < 9 or out[:1] not in (b"P", b"J"):
        if r.returncode < 0:
            log.warning("Afbeeldingslezer gecrasht op (decoder crashed on) %s: signaal %d", path, -r.returncode)
        return None
    return out[1:], ("image/jpeg" if out[:1] == b"J" else "image/png")


def _is_system_image(path: str) -> bool:
    return os.path.realpath(path).startswith(_SYSTEM_IMAGE_DIRS)


def _header_size(head: bytes) -> tuple[int, int] | None:
    """(width, height) from a PNG or GIF header (read without any decoder), else None."""
    if len(head) >= 24 and head[:8] == b"\x89PNG\r\n\x1a\n" and head[12:16] == b"IHDR":
        return struct.unpack(">II", head[16:24])
    if len(head) >= 10 and head[:6] in (b"GIF87a", b"GIF89a"):
        return struct.unpack("<HH", head[6:10])
    return None


def _to_png_bytes(path: str, size: int | None = None) -> bytes | None:
    """Convert any GdkPixbuf-readable image to PNG (for XPM/TIFF/HEIC… that browsers can't show).
    User files are decoded in the sandboxed helper; system icons (/usr/share…) in-process."""
    try:
        st = os.stat(path)
    except OSError:
        return None
    key = (path, st.st_mtime_ns, st.st_size, size)
    with _conv_lock:
        hit = _conv_cache.get(key)
        if hit is not None:
            _conv_cache.move_to_end(key)
            return hit
    if not _is_system_image(path) or image_mime(path) not in INPROC_MIMES | {"image/x-xpixmap", "image/x-xpm"}:
        res = decode_isolated(path, size or 0)
        if res is None:
            return None
        data = res[0]
    else:
        gp = _gdkpixbuf()
        if gp is None:
            return None
        try:
            with _thumb_sem:
                if size:
                    pb = gp.Pixbuf.new_from_file_at_scale(path, size, size, True)
                else:
                    pb = gp.Pixbuf.new_from_file(path)
                    if pb.get_width() > 4096 or pb.get_height() > 4096:
                        pb = gp.Pixbuf.new_from_file_at_scale(path, 4096, 4096, True)
                ok, data = pb.save_to_bufferv("png", [], [])
        except Exception as e:  # noqa: BLE001
            log.debug("Conversie mislukt (conversion failed) %s: %s", path, e)
            return None
        if not ok:
            return None
        data = bytes(data)
    if len(data) > _CONV_ENTRY_MAX:
        return data                       # huge conversions are not kept in memory
    global _conv_bytes
    with _conv_lock:
        old = _conv_cache.pop(key, None)
        if old is not None:
            _conv_bytes -= len(old)
        _conv_cache[key] = data
        _conv_bytes += len(data)
        while _conv_cache and (_conv_bytes > _CONV_CACHE_MAX or len(_conv_cache) > 256):
            _k, v = _conv_cache.popitem(last=False)
            _conv_bytes -= len(v)
    return data


def _file_uri(path: str) -> str:
    try:
        import gi
        gi.require_version("Gio", "2.0")
        from gi.repository import Gio
        return Gio.File.new_for_path(path).get_uri()
    except Exception:  # noqa: BLE001
        return "file://" + quote(path)


def _freedesktop_thumb(path: str, size: int, mtime: float) -> str | None:
    """Existing thumbnail from ~/.cache/thumbnails (made by Files/Nautilus), if fresh."""
    digest = hashlib.md5(_file_uri(path).encode("utf-8")).hexdigest()
    base = Path(os.environ.get("XDG_CACHE_HOME") or Path.home() / ".cache") / "thumbnails"
    order = ["normal", "large", "x-large", "xx-large"] if size <= 128 else \
        ["large", "x-large", "xx-large", "normal"] if size <= 256 else ["x-large", "xx-large", "large", "normal"]
    for sub in order:
        p = base / sub / f"{digest}.png"
        try:
            if p.stat().st_mtime >= mtime:
                return str(p)
        except OSError:
            continue
    return None


THUMB_CACHE_MAX = 512 * 1024 * 1024
THUMB_CACHE_MAX_AGE = 60 * 86400


def prune_thumb_cache(root: Path, max_bytes: int = THUMB_CACHE_MAX, max_age: float = THUMB_CACHE_MAX_AGE,
                      now: float | None = None) -> tuple[int, int]:
    """Delete cached thumbnails unused for ``max_age`` s, then the least recently used until the cache is
    below 90 % of ``max_bytes`` → (files removed, bytes freed). Only regular files two levels deep."""
    now = time.time() if now is None else now
    entries: list[tuple[float, int, str]] = []
    try:
        subs = [e for e in os.scandir(root) if e.is_dir(follow_symlinks=False) and len(e.name) == 2]
    except OSError:
        return 0, 0
    for sub in subs:
        try:
            with os.scandir(sub.path) as it:
                for e in it:
                    if not e.is_file(follow_symlinks=False):
                        continue
                    try:
                        st = e.stat(follow_symlinks=False)
                    except OSError:
                        continue
                    entries.append((max(st.st_atime, st.st_mtime), st.st_blocks * 512 or st.st_size, e.path))
        except OSError:
            continue
    removed = freed = 0
    keep = []
    for used, size, path in entries:
        if now - used > max_age:
            try:
                os.unlink(path)
                removed += 1
                freed += size
            except OSError:
                pass
        else:
            keep.append((used, size, path))
    total = sum(size for _u, size, _p in keep)
    if total > max_bytes:
        keep.sort()
        for _used, size, path in keep:
            if total <= max_bytes * 0.9:
                break
            try:
                os.unlink(path)
                removed += 1
                freed += size
                total -= size
            except OSError:
                pass
    return removed, freed


# ---------------------------------------------------------------------------
# Server
# ---------------------------------------------------------------------------

class _HTTPServer(ThreadingHTTPServer):
    daemon_threads = True
    allow_reuse_address = True
    request_queue_size = 64

    def __init__(self, shell: "ShellServer", port: int):
        self.shell = shell
        super().__init__(("127.0.0.1", port), _Handler)

    def handle_error(self, request, client_address):  # quieter than the default traceback dump
        log.debug("Verbindingsfout (connection error) %s", client_address, exc_info=True)


class ShellServer:
    def __init__(self, ctx, port: int = 0):
        self.ctx = ctx
        self.api = ctx.api
        self.httpd = _HTTPServer(self, port)
        self.port = self.httpd.server_address[1]
        self.stopping = False
        self.keepalive = float(os.environ.get("WIN11SHELL_SSE_KEEPALIVE", "15") or 15)
        self.ui_root = Path(ctx.ui_dir).resolve()
        self.thumb_dir = Path(ctx.cache_dir) / "thumbs"
        self._thread: threading.Thread | None = None
        self._thumb_writes = 0
        self._prune_lock = threading.Lock()

    def start(self) -> "ShellServer":
        self._thread = threading.Thread(target=self.httpd.serve_forever, kwargs={"poll_interval": 0.25},
                                        name="http", daemon=True)
        self._thread.start()
        log.info("HTTP-server luistert op (listening on) http://127.0.0.1:%d", self.port)
        # Warm the icon-theme index (first lookup scans the theme dirs, ~100 ms) off the request path.
        threading.Thread(target=lambda: find_icon("folder", 32), name="icon-warmup", daemon=True).start()
        t = threading.Timer(30.0, self.prune_thumbs)        # after start-up, off the request path
        t.daemon = True
        t.start()
        return self

    # -- own thumbnail cache (<cache>/thumbs): bounded by age and size ----------------
    def thumb_written(self) -> None:
        self._thumb_writes += 1
        if self._thumb_writes % 500 == 0:
            threading.Thread(target=self.prune_thumbs, name="thumb-prune", daemon=True).start()

    def prune_thumbs(self) -> None:
        if not self._prune_lock.acquire(blocking=False):
            return
        try:
            removed, freed = prune_thumb_cache(self.thumb_dir)
            if removed:
                log.info("Miniatuurcache opgeruimd (thumbnail cache pruned): %d bestanden, %d MB",
                         removed, freed // (1024 * 1024))
        except Exception:  # noqa: BLE001
            log.debug("prune failed", exc_info=True)
        finally:
            self._prune_lock.release()

    def stop(self) -> None:
        if self.stopping:
            return
        self.stopping = True
        try:
            self.api.close()
        except Exception:  # noqa: BLE001
            pass
        try:
            self.httpd.shutdown()
        except Exception:  # noqa: BLE001
            pass
        try:
            self.httpd.server_close()
        except Exception:  # noqa: BLE001
            pass

    # -- auth helpers -----------------------------------------------------------
    def token_ok(self, given: str | None) -> bool:
        tok = self.ctx.token or ""
        return bool(given) and bool(tok) and hmac.compare_digest(str(given).encode(), tok.encode())

    def allowed_hosts(self) -> set[str]:
        return {f"127.0.0.1:{self.port}", f"localhost:{self.port}"}

    def allowed_origins(self) -> set[str]:
        return {f"http://{h}" for h in self.allowed_hosts()}


def _peer_uid(client_port: int, server_port: int) -> int | None:
    """uid owning the client end of a 127.0.0.1 TCP connection (Linux /proc/net/tcp)."""
    loopback = ("0100007F", "7F000001")  # 127.0.0.1 in little/big-endian host order
    want_local = f"{client_port:04X}"
    want_remote = f"{server_port:04X}"
    try:
        with open("/proc/net/tcp", "r", encoding="ascii", errors="replace") as f:
            next(f, None)
            for line in f:
                parts = line.split()
                if len(parts) <= 7:
                    continue
                lip, _, lport = parts[1].partition(":")
                rip, _, rport = parts[2].partition(":")
                if lport == want_local and rport == want_remote and lip in loopback and rip in loopback:
                    return int(parts[7])
    except (OSError, ValueError):
        return None
    return None


class _Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    server_version = f"Win11Shell/{VERSION}"
    sys_version = ""
    timeout = 120  # idle keep-alive connections
    # Headers and body are separate writes: without TCP_NODELAY, Nagle + delayed ACK add ~40 ms per call.
    disable_nagle_algorithm = True

    # -- plumbing -------------------------------------------------------------
    @property
    def shell(self) -> ShellServer:
        return self.server.shell  # type: ignore[attr-defined]

    def version_string(self):
        return self.server_version

    def log_message(self, fmt, *args):
        log.debug("%s %s", self.address_string(), fmt % args)

    def do_GET(self):
        self._route("GET")

    def do_HEAD(self):
        self._route("HEAD")

    def do_POST(self):
        self._route("POST")

    def do_OPTIONS(self):
        self._send_text(405, "Method Not Allowed")

    do_PUT = do_DELETE = do_PATCH = do_OPTIONS

    def _route(self, method: str) -> None:
        try:
            self._route_inner(method)
        except (BrokenPipeError, ConnectionResetError):
            self.close_connection = True
        except Exception:  # noqa: BLE001
            log.exception("Fout bij verzoek (request failed) %s %s", method, self.path)
            try:
                self._send_text(500, "Internal Server Error")
            except Exception:  # noqa: BLE001
                self.close_connection = True

    def _route_inner(self, method: str) -> None:
        u = urlsplit(self.path)
        path = u.path
        self.query = {k: v[-1] for k, v in parse_qs(u.query, keep_blank_values=True).items()}
        host = self.headers.get("Host")
        if host and host.lower() not in self.shell.allowed_hosts():
            return self._send_text(403, "Forbidden host")
        if path == "/":
            return self._root()
        if path in ("/ui", "/ui/"):
            return self._redirect("/ui/index.html" + (f"?{u.query}" if u.query else ""))
        if path.startswith("/ui/"):
            if method not in ("GET", "HEAD"):
                return self._send_text(405, "Method Not Allowed")
            return self._static(path[4:], head=(method == "HEAD"))
        if path == "/favicon.ico":
            return self._send_bytes(204, b"", "image/x-icon")
        protected = path.startswith("/api/") or path in ("/icon", "/thumb", "/file")
        if not protected:
            return self._send_text(404, "Not Found")
        if not self.shell.token_ok(self.headers.get("X-Shell-Token") or self.query.get("token")):
            if path.startswith("/api/"):
                return self._send_json(401, {"ok": False, "error": {"code": "unauthorized", "message": "Geen toegang."}})
            return self._send_text(401, "Unauthorized")
        origin = self.headers.get("Origin")
        if origin and origin not in self.shell.allowed_origins():
            return self._send_json(403, {"ok": False, "error": {"code": "forbidden", "message": "Geen toegang."}})
        if method == "HEAD" and path.startswith("/api/"):
            return self._send_text(405, "Method Not Allowed")
        if path.startswith("/api/call/"):
            if method != "POST":
                return self._send_json(405, {"ok": False, "error": {"code": "method_not_allowed",
                                                                    "message": "Gebruik POST."}})
            return self._api_call(path[len("/api/call/"):])
        if method == "POST":
            return self._send_text(405, "Method Not Allowed")
        if path == "/api/events":
            return self._events()
        if path == "/api/methods":
            return self._send_json(200, {"ok": True, "result": self.shell.api.describe()})
        head = method == "HEAD"
        if path == "/icon":
            return self._icon(head)
        if path == "/thumb":
            return self._thumb(head)
        if path == "/file":
            return self._file(head)
        return self._send_text(404, "Not Found")

    # -- responses --------------------------------------------------------------
    def _common_headers(self, cache: str = "no-store") -> None:
        self.send_header("Cache-Control", cache)
        self.send_header("X-Content-Type-Options", "nosniff")
        self.send_header("Referrer-Policy", "no-referrer")
        # No other web page may frame the shell (click-jacking); the preview WM's iframes are same-origin.
        self.send_header("X-Frame-Options", "SAMEORIGIN")

    def _send_bytes(self, status: int, body: bytes, ctype: str, headers: dict | None = None,
                    head: bool = False, cache: str = "no-store") -> None:
        self.send_response(status)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        self._common_headers(cache)
        for k, v in (headers or {}).items():
            self.send_header(k, v)
        self.end_headers()
        if not head and body:
            self.wfile.write(body)

    def _send_text(self, status: int, text: str) -> None:
        self._send_bytes(status, (text + "\n").encode("utf-8"), "text/plain; charset=utf-8")

    def _send_json(self, status: int, obj) -> None:
        try:
            body = json_dumps(obj).encode("utf-8")
        except (TypeError, ValueError) as e:
            log.error("Antwoord niet als JSON te schrijven (response not serialisable): %s", e)
            body = json_dumps({"ok": False, "error": {"code": "internal",
                                                      "message": "Er ging iets mis. Probeer het opnieuw.",
                                                      "detail": str(e)[:300]}}).encode("utf-8")
        self._send_bytes(status, body, "application/json; charset=utf-8")

    def _redirect(self, location: str) -> None:
        self.send_response(302)
        self.send_header("Location", location)
        self.send_header("Content-Length", "0")
        self._common_headers()
        self.end_headers()

    def _not_modified(self, etag: str) -> bool:
        inm = self.headers.get("If-None-Match")
        if inm and etag in [t.strip() for t in inm.split(",")]:
            self.send_response(304)
            self.send_header("ETag", etag)
            self._common_headers("private, no-cache")
            self.end_headers()
            return True
        return False

    def _send_path(self, path: str | Path, ctype: str, head: bool, cache: str = "no-store",
                   etag: bool = False, extra: dict | None = None, fobj=None) -> None:
        """``fobj``: the file, already opened (and checked) by the caller."""
        if fobj is None:
            try:
                fobj = open(path, "rb")
            except OSError:
                return self._send_text(404, "Not Found")
        f = fobj
        with f:
            st = os.fstat(f.fileno())
            headers = dict(extra or {})
            if etag:
                tag = '"' + hashlib.sha1(f"{path}|{st.st_mtime_ns}|{st.st_size}".encode()).hexdigest()[:20] + '"'
                if self._not_modified(tag):
                    return
                headers["ETag"] = tag
            self.send_response(200)
            self.send_header("Content-Type", ctype)
            self.send_header("Content-Length", str(st.st_size))
            self._common_headers(cache)
            for k, v in headers.items():
                self.send_header(k, v)
            self.end_headers()
            if not head:
                left = st.st_size      # never more than Content-Length, even when the file grows meanwhile
                while left > 0:
                    chunk = f.read(min(256 * 1024, left))
                    if not chunk:
                        break
                    self.wfile.write(chunk)
                    left -= len(chunk)

    # -- routes ---------------------------------------------------------------
    def _root(self) -> None:
        """``/`` → the preview page. The token is added only for a navigation the user made (typed address,
        bookmark: ``Sec-Fetch-Site: none`` as a top-level document) from a process of this user. A web page
        that frames or opens ``http://127.0.0.1:<port>/`` gets the token-less (mock) page."""
        loc = "/ui/index.html?surface=preview"
        site = (self.headers.get("Sec-Fetch-Site") or "none").strip().lower()
        dest = (self.headers.get("Sec-Fetch-Dest") or "document").strip().lower()
        if site == "none" and dest == "document":
            uid = _peer_uid(self.client_address[1], self.shell.port)
            if uid is not None and uid == os.getuid():
                loc += "&token=" + quote(self.shell.ctx.token)
        return self._redirect(loc)

    def _static(self, rel: str, head: bool) -> None:
        rel = unquote(rel)
        if "\x00" in rel or "\\" in rel:
            return self._send_text(400, "Bad Request")
        parts = [p for p in rel.split("/") if p not in ("", ".")]
        if any(p == ".." or p.startswith(".") for p in parts):
            return self._send_text(404, "Not Found")
        root = self.shell.ui_root
        target = root.joinpath(*parts) if parts else root
        try:
            real = target.resolve()
        except (OSError, RuntimeError):
            return self._send_text(404, "Not Found")
        if real != root and not real.is_relative_to(root):
            return self._send_text(404, "Not Found")
        if real.is_dir():
            real = real / "index.html"
        if not real.is_file():
            return self._send_text(404, "Not Found")
        ctype = STATIC_MIME.get(real.suffix.lower(), "application/octet-stream")
        extra = {"Content-Security-Policy": PAGE_CSP} if ctype.startswith("text/html") else None
        return self._send_path(real, ctype, head, cache="no-store", extra=extra)

    def _api_call(self, name: str) -> None:
        name = unquote(name)
        if not _METHOD_RE.match(name):
            return self._send_json(400, {"ok": False, "error": {"code": "bad_request", "message": "Ongeldige aanvraag."}})
        if "chunked" in (self.headers.get("Transfer-Encoding") or "").lower():
            self.close_connection = True
            return self._send_json(411, {"ok": False, "error": {"code": "length_required",
                                                                "message": "Ongeldige aanvraag."}})
        try:
            length = int(self.headers.get("Content-Length") or 0)
        except ValueError:
            length = -1
        if length < 0 or length > MAX_BODY:
            self.close_connection = True
            return self._send_json(413, {"ok": False, "error": {"code": "too_large", "message": "Te veel gegevens."}})
        raw = self.rfile.read(length) if length else b""
        args: object = {}
        if raw.strip():
            body = json_loads(raw, None)
            if not isinstance(body, dict):
                return self._send_json(200, {"ok": False, "error": {"code": "bad_request",
                                                                    "message": "Ongeldige aanvraag."}})
            args = body.get("args", {})
            if args is None:
                args = {}
        env = self.shell.api.dispatch(name, args)
        return self._send_json(200, env)

    def _events(self) -> None:
        shell = self.shell
        api = shell.api
        sub = api.subscribe(remote=self.address_string())
        self.close_connection = True
        try:
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream; charset=utf-8")
            self.send_header("Cache-Control", "no-store")
            self.send_header("Connection", "close")
            self.send_header("X-Accel-Buffering", "no")
            self.end_headers()
            hello = json_dumps({"mode": shell.ctx.mode, "version": VERSION, "dryRun": shell.ctx.dry_run,
                                "compositor": shell.ctx.compositor})
            self.wfile.write(f"retry: 1500\n\nevent: hello\ndata: {hello}\n\n".encode("utf-8"))
            api.connected(sub)
            while not shell.stopping and not sub.closed:
                try:
                    item = sub.next(shell.keepalive)
                except queue.Empty:
                    self.wfile.write(b": keep-alive\n\n")
                    continue
                if item is None:
                    break
                event, data = item
                self.wfile.write(f"event: {event}\ndata: {data}\n\n".encode("utf-8"))
        except (BrokenPipeError, ConnectionResetError, TimeoutError, OSError):
            pass
        finally:
            api.unsubscribe(sub)

    def _size_arg(self, default: int = 32) -> int:
        try:
            return max(8, min(1024, int(float(self.query.get("size", default)))))
        except (TypeError, ValueError):
            return default

    def _send_image(self, path: str, head: bool, size: int | None = None, convert: bool = False) -> None:
        """Serve an image file; formats a browser can't show (or ``convert``) are converted to PNG."""
        mime = image_mime(path)
        if not mime:
            return self._send_text(404, "Not Found")
        extra = {"Content-Security-Policy": IMAGE_CSP}
        if not convert and mime in BROWSER_IMAGE_MIMES and not path.lower().endswith(".svgz"):
            return self._send_path(path, mime, head, cache="private, no-cache", etag=True, extra=extra)
        data = _to_png_bytes(path, size)
        if data is None:
            return self._send_text(404, "Not Found")
        tag = '"' + hashlib.sha1(data).hexdigest()[:20] + '"'
        if self._not_modified(tag):
            return
        extra["ETag"] = tag
        return self._send_bytes(200, data, "image/png", extra, head=head, cache="private, no-cache")

    def _icon(self, head: bool) -> None:
        size = self._size_arg(32)
        name = self.query.get("name", "").strip()
        mime = self.query.get("mime", "").strip()
        fallback = self.query.get("fallback", "").strip()
        p = None
        asked = name
        if name:
            p = find_icon(name, size)
        elif mime:
            p = find_mime_icon(mime, size)
        if p is None and fallback:
            p = find_icon(fallback, size)
            asked = fallback
        if p is None:
            return self._send_text(404, "Not Found")
        if asked.startswith(("/", "file://")) and not _is_system_image(str(p)):
            return self._send_user_icon(str(p), head, size)
        return self._send_image(str(p), head, size)

    def _send_user_icon(self, path: str, head: bool, size: int) -> None:
        """An image file an app picked (absolute tray / notification icon): ≤ 2 MiB, else 404. PNG/GIF up to
        1024 px and SVG are streamed from the file checked here; anything larger, or a format whose size is
        not read here, is decoded in the sandboxed helper and served at the requested size."""
        mime = image_mime(path)
        if not mime:
            return self._send_text(404, "Not Found")
        try:
            f = open(path, "rb")
        except OSError:
            return self._send_text(404, "Not Found")
        with f:
            st = os.fstat(f.fileno())
            if not stat.S_ISREG(st.st_mode) or st.st_size > MAX_USER_ICON:
                return self._send_text(404, "Not Found")
            dims = _header_size(f.read(32))
            f.seek(0)
            svg = mime == "image/svg+xml" and not path.lower().endswith(".svgz")
            if svg or (dims and max(dims) <= MAX_USER_ICON_PX and mime in BROWSER_IMAGE_MIMES):
                return self._send_path(path, mime, head, cache="private, no-cache", etag=True,
                                       extra={"Content-Security-Policy": IMAGE_CSP}, fobj=f)
        return self._send_image(path, head, size, convert=True)

    def _abs_path_arg(self) -> str | None:
        raw = self.query.get("path", "")
        if raw.startswith("file://"):
            raw = unquote(urlsplit(raw).path)
        if not raw or "\x00" in raw or not os.path.isabs(raw):
            return None
        return os.path.realpath(raw)

    def _thumb(self, head: bool) -> None:
        path = self._abs_path_arg()
        if not path or not os.path.isfile(path):
            return self._send_text(404, "Not Found")
        size = self._size_arg(256)
        try:
            st = os.stat(path)
        except OSError:
            return self._send_text(404, "Not Found")
        mime = image_mime(path)
        if not mime:
            fd = _freedesktop_thumb(path, size, st.st_mtime)
            if fd:
                return self._send_image(fd, head)
            return self._send_text(404, "Not Found")
        if mime == "image/svg+xml" and not path.lower().endswith(".svgz"):
            return self._send_image(path, head)
        bucket = next((b for b in THUMB_BUCKETS if b >= size), THUMB_BUCKETS[-1])
        key = hashlib.sha1(f"{path}\0{st.st_mtime_ns}\0{st.st_size}\0{bucket}".encode("utf-8", "surrogateescape")).hexdigest()
        for ext, ctype in ((".jpg", "image/jpeg"), (".png", "image/png")):
            cached = self.shell.thumb_dir / key[:2] / (key + ext)
            if cached.is_file():
                return self._send_path(cached, ctype, head, cache="private, max-age=3600", etag=True,
                                       extra={"Content-Security-Policy": IMAGE_CSP})
        gp = _gdkpixbuf()
        if gp is None or st.st_size > MAX_THUMB_SOURCE:
            if mime in BROWSER_IMAGE_MIMES and st.st_size < 12 * 1024 * 1024:
                return self._send_image(path, head)
            return self._send_text(404, "Not Found")
        if mime not in INPROC_MIMES:
            res = decode_isolated(path, bucket, jpeg_ok=True)
            if res is None:
                return self._send_text(404, "Not Found")
            data, ctype = res
            ext = ".jpg" if ctype == "image/jpeg" else ".png"
            return self._store_thumb(key, ext, data, ctype, head)
        try:
            with _thumb_sem:
                fmt = gp.Pixbuf.get_file_info(path)
                w = (fmt[1] if fmt and fmt[0] else 0) or 0
                h = (fmt[2] if fmt and fmt[0] else 0) or 0
                if w and h and max(w, h) <= bucket and mime in BROWSER_IMAGE_MIMES and st.st_size < 4 * 1024 * 1024:
                    small = True
                else:
                    small = False
                    pb = gp.Pixbuf.new_from_file_at_scale(path, bucket, bucket, True)
                    pb = pb.apply_embedded_orientation() or pb
                    if pb.get_has_alpha():
                        ok, data = pb.save_to_bufferv("png", [], [])
                        ext, ctype = ".png", "image/png"
                    else:
                        ok, data = pb.save_to_bufferv("jpeg", ["quality"], ["85"])
                        ext, ctype = ".jpg", "image/jpeg"
        except Exception as e:  # noqa: BLE001
            log.debug("Miniatuur mislukt (thumbnail failed) %s: %s", path, e)
            return self._send_text(404, "Not Found")
        if small:
            return self._send_image(path, head)
        if not ok:
            return self._send_text(404, "Not Found")
        return self._store_thumb(key, ext, bytes(data), ctype, head)

    def _store_thumb(self, key: str, ext: str, data: bytes, ctype: str, head: bool) -> None:
        try:
            atomic_write_bytes(self.shell.thumb_dir / key[:2] / (key + ext), data, mode=0o600)
            self.shell.thumb_written()
        except OSError as e:
            log.debug("Miniatuur niet bewaard (thumbnail not cached): %s", e)
        tag = '"' + key[:20] + '"'
        return self._send_bytes(200, data, ctype, {"ETag": tag, "Content-Security-Policy": IMAGE_CSP},
                                head=head, cache="private, max-age=3600")

    def _file(self, head: bool) -> None:
        raw = self.query.get("path", "")
        if raw.startswith("builtin:"):
            ident = raw[len("builtin:"):]
            # Renamed built-in wallpapers: settings saved before the rename still name the old id.
            ident = {"bloom-dark": "lotus-dark", "bloom-light": "lotus-light"}.get(ident, ident)
            if re.match(r"^[a-z0-9-]{1,64}$", ident):
                return self._redirect(f"/ui/assets/wallpapers/{ident}.svg")
            return self._send_text(404, "Not Found")
        path = self._abs_path_arg()
        if not path or not os.path.exists(path):
            return self._send_text(404, "Not Found")
        if not os.path.isfile(path) or not image_mime(path):
            return self._send_text(403, "Forbidden")
        return self._send_image(path, head)
