"""Read-only static server for the UI text dump (H: notes/english-option-design.md §3.3, step E0b; ported to Win11
voor GNOME in GE0a, notes/claude/round0/ge0a.md).

    python3 -I tests/i18n/serve.py ROOT          prints the port on the first line, then serves until killed

ROOT is the project root. Only two trees are served, under the paths the dump uses (the Rust host serves the UI
under /ui/ as well, so module paths and the seeded Math.random streams of freeze.js see the same /ui/js/… names):

* ``/ui/…``          → ``ROOT/vendor/ui/…`` (G's vendored UI; H served ``ROOT/ui``)
* ``/tests/i18n/…``  → ``ROOT/tests/i18n/…`` (dump.html, surfaces.json, freeze.js)

Everything else is 404. Like ``python3 -m http.server -b 127.0.0.1``, with these differences:

* ``/ui/index.html`` gets ``<script src="/tests/i18n/freeze.js"></script>`` as the first element of its <head>, so
  the fixed clock, the seeded ``Math.random`` and the timer order are in place before any of the page's own scripts
  (the inline theme script included) run. Nothing on disk changes; the tree is only read.
* every answer carries ``Cache-Control: no-store`` (a module must never come from an older run's cache);
* only GET/HEAD, only 127.0.0.1, no directory listings, quiet log.
"""

from __future__ import annotations

import http.server
import io
import os
import posixpath
import socketserver
import sys
import urllib.parse
from pathlib import Path

INJECT = b'<script src="/tests/i18n/freeze.js"></script>\n'
MOUNTS = (("/ui/", ("vendor", "ui")), ("/tests/i18n/", ("tests", "i18n")))


def make_handler(root: Path):
    mounts = [(prefix, root.joinpath(*rel).resolve()) for prefix, rel in MOUNTS]

    class Handler(http.server.SimpleHTTPRequestHandler):
        def __init__(self, *a, **kw):
            super().__init__(*a, directory=str(root), **kw)

        def log_message(self, fmt, *args):  # quiet
            pass

        def end_headers(self):
            self.send_header("Cache-Control", "no-store")
            super().end_headers()

        def list_directory(self, path):
            self.send_error(404)
            return None

        def translate_path(self, path: str) -> str:
            """A URL path → a file inside one of the two mounts; anything else → a path that does not exist."""
            path = urllib.parse.unquote(path.split("?", 1)[0].split("#", 1)[0], errors="surrogatepass")
            path = posixpath.normpath(path) + ("/" if path.endswith("/") else "")
            for prefix, base in mounts:
                if path.startswith(prefix) or path + "/" == prefix:
                    rest = path[len(prefix):] if path.startswith(prefix) else ""
                    parts = [p for p in rest.split("/") if p and p not in (".", "..")]
                    target = base.joinpath(*parts)
                    try:
                        target.resolve().relative_to(base)
                    except ValueError:
                        break
                    return str(target)
            return str(root / ".i18n-serve-not-found")

        def send_head(self):
            path = self.path.split("?", 1)[0].split("#", 1)[0]
            if path not in ("/ui/index.html", "/ui/"):
                return super().send_head()
            body = (root / "vendor" / "ui" / "index.html").read_bytes()
            i = body.find(b"<head>")
            body = body[:i + 6] + b"\n" + INJECT + body[i + 6:] if i >= 0 else INJECT + body
            self.send_response(200)
            self.send_header("Content-Type", "text/html; charset=utf-8")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            return io.BytesIO(body)

        def do_POST(self):  # read-only
            self.send_error(405)

        do_PUT = do_DELETE = do_PATCH = do_POST

    return Handler


class Server(socketserver.ThreadingMixIn, http.server.HTTPServer):
    daemon_threads = True
    allow_reuse_address = True


def main(argv: list[str]) -> int:
    root = Path(argv[1] if len(argv) > 1 else os.getcwd()).resolve()
    srv = Server(("127.0.0.1", 0), make_handler(root))
    print(srv.server_address[1], flush=True)
    try:
        srv.serve_forever(poll_interval=0.2)
    except KeyboardInterrupt:
        pass
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
