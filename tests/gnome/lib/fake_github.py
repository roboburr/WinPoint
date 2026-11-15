#!/usr/bin/env python3
"""A fake GitHub for WinPoint's updater in the private GNOME harness (D-G74a, D-G74; ``harness.py run --winpoint-feed``).

harness.py ``run`` starts it inside the sandbox (its own network namespace, loopback only) before the session and
outside the strace'd tree. The hosts reach it through the updater's test seam (``WIN11_WINPOINT_TEST``,
host/src/services/winpoint/seam.rs: origin ``http://127.0.0.1:PORT/``), so they never ask api.github.com. Standard
library only.

    python3 -B -I tests/gnome/lib/fake_github.py --port 47811 --running 0.8.9 \\
        --bundle RUN/winpoint-dist/winpoint-0.9.0-linux-x86_64.tar.gz --log RUN/logs/fake-github.jsonl

``--bundle`` is a real release: the tarball scripts/package.py made from the run's build dir, with its ``.sha256``
next to it (harness.py ``winpoint_bundle``). Its version V (from the name) is the release the fake offers; the hosts
pretend to run ``--running`` (seam.json), a lower version, so V is an update they can install.

* ``GET /repos/roboburr/WinPoint/releases?per_page=10``: the release list of the current mode, as GitHub's REST API
  answers it (newest first; ``html_url`` and ``browser_download_url`` on this origin, assets ``state: uploaded``,
  each with GitHub's ``digest`` "sha256:<hex>" of its bytes), with a Content-Length; mode ``newer-chunked`` sends the
  same list chunked.
* ``GET /roboburr/WinPoint/releases/download/vV/<tarball>``: a 302 to ``/cdn/<tarball>`` (GitHub sends release assets
  to its CDN host), which sends the tarball with a Content-Length; ``…/<tarball>.sha256`` answers at once. Assets of
  the other releases in the list are never downloaded (404).
* ``GET /_fake/mode/<mode>[?delay=S][&slow=S]``: switch the mode (the scenario's control; no host asks for it);
  ``delay`` holds every list answer S seconds (at most 10), so the card's "checking" state can be seen; ``slow``
  spreads the tarball over about S seconds (at most 30), so its download can be seen.
* ``GET /_fake/requests``: every list request so far ({at, path, userAgent, accept, apiVersion, mode, status}).
* ``GET /_fake/downloads``: every request for a release asset or the CDN ({at, path, mode, status, bytes}).
* ``GET /_fake/info``: {running, offered, tarball, size, digest}.

Modes: ``newer`` (the start): V with this PC's asset set (x86_64: the real tarball and its ``.sha256``), plus a higher
draft and a higher prerelease that the check must skip; ``newer-chunked``: the same, chunked; ``damaged``: V as in
``newer`` (the digest and the ``.sha256`` of the real tarball), but the CDN sends it with one byte changed, as a broken
download would arrive; ``nodigest``: V without GitHub's ``digest`` on its assets (releases from before GitHub had it);
``current``: the newest release is ``running`` itself; ``error``: HTTP 500; ``empty``: ``[]``; ``missing``: 404.

Every list request is appended to ``--log`` (JSON lines). The first line on stdout is ``ready`` once it listens.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, urlsplit

FEED = '/repos/roboburr/WinPoint/releases'
DOWNLOAD = '/roboburr/WinPoint/releases/download/'
CDN = '/cdn/'
MODES = ('newer', 'newer-chunked', 'damaged', 'nodigest', 'current', 'error', 'empty', 'missing')
# the asset set of a release that is only listed (never downloaded): sizes as GitHub would show them
FAKE_SIZE = 71_234_567
TARBALL = re.compile(r'winpoint-(\d+\.\d+\.\d+)-linux-([\w]+)\.tar\.gz')
CHUNK = 1 << 20


def bump(version: str, step: int = 1) -> str:
    major, minor, patch = (int(x) for x in version.split('.'))
    return f'{major}.{minor}.{patch + step}'


class Bundle:
    """The real release: the tarball, its .sha256 (scripts/package.py's own) and its SHA-256."""

    def __init__(self, path: Path):
        match = TARBALL.fullmatch(path.name)
        if not match:
            raise SystemExit(f'fake GitHub: {path.name} is not winpoint-V-linux-ARCH.tar.gz')
        self.path, self.version, self.arch = path, match.group(1), match.group(2)
        self.size = path.stat().st_size
        hasher = hashlib.sha256()
        with path.open('rb') as data:
            for block in iter(lambda: data.read(CHUNK), b''):
                hasher.update(block)
        self.digest = hasher.hexdigest()
        self.sha256 = path.with_name(path.name + '.sha256').read_bytes()
        if self.sha256 != f'{self.digest}  {path.name}\n'.encode():
            raise SystemExit(f'fake GitHub: {path.name}.sha256 does not name {path.name} with its SHA-256')


def asset(base: str, tag: str, name: str, size: int, kind: str, digest: str | None) -> dict:
    return {'name': name, 'label': '', 'state': 'uploaded', 'content_type': kind, 'size': size, 'download_count': 3,
            'browser_download_url': f'{base}/download/{tag}/{name}',
            'digest': None if digest is None else 'sha256:' + digest}


def release(origin: str, version: str, *, published: str, draft: bool = False, prerelease: bool = False,
            bundle: Bundle | None = None, digests: bool = True) -> dict:
    """One release as GitHub's list has it: the real bundle's two assets, or a listed-only set (x86_64)."""
    tag = f'v{version}'
    base = f'{origin}roboburr/WinPoint/releases'
    if bundle:
        tarball = bundle.path.name
        assets = [asset(base, tag, tarball, bundle.size, 'application/gzip', bundle.digest if digests else None),
                  asset(base, tag, tarball + '.sha256', len(bundle.sha256), 'text/plain',
                        hashlib.sha256(bundle.sha256).hexdigest() if digests else None)]
    else:
        tarball = f'winpoint-{version}-linux-x86_64.tar.gz'
        assets = [asset(base, tag, name, size, kind, hashlib.sha256(name.encode()).hexdigest())
                  for name, size, kind in ((tarball, FAKE_SIZE, 'application/gzip'),
                                           (tarball + '.sha256', 99, 'text/plain'))]
    return {'html_url': f'{base}/tag/{tag}', 'tag_name': tag, 'target_commitish': 'main', 'name': f'WinPoint {version}',
            'draft': draft, 'prerelease': prerelease, 'created_at': published, 'published_at': published,
            'assets': assets, 'body': f'WinPoint {version} (fake GitHub of the GNOME harness)'}


class Fake:
    def __init__(self, origin: str, running: str, bundle: Bundle, log: str):
        self.origin, self.running, self.bundle, self.log = origin, running, bundle, log
        self.mode, self.delay, self.slow = 'newer', 0.0, 0.0
        self.requests: list[dict] = []
        self.downloads: list[dict] = []
        self.lock = threading.Lock()

    def releases(self, mode: str) -> list[dict]:
        o, r, v = self.origin, self.running, self.bundle.version
        newest = [release(o, r, published='2026-11-14T19:00:00Z')]
        if mode in ('newer', 'newer-chunked', 'damaged', 'nodigest'):
            newest = [release(o, f'{bump(v, 2)}-rc.1', published='2026-11-21T09:00:00Z', prerelease=True),
                      release(o, bump(v, 3), published='2026-11-21T08:00:00Z', draft=True),
                      release(o, v, published='2026-11-20T10:00:00Z', bundle=self.bundle,
                              digests=mode != 'nodigest')] + newest
        return newest


def handler(fake: Fake):
    class Handler(BaseHTTPRequestHandler):
        protocol_version = 'HTTP/1.1'
        server_version = 'FakeGitHub/1'

        def log_message(self, fmt, *args):   # noqa: D401 - quiet: the JSON log has every list request
            pass

        def head(self, status: int, kind: str, length: int | None) -> None:
            self.send_response(status)
            self.send_header('Content-Type', kind)
            self.send_header('Connection', 'close')
            self.send_header('X-GitHub-Api-Version-Selected', '2022-11-28')
            if length is None:
                self.send_header('Transfer-Encoding', 'chunked')
            else:
                self.send_header('Content-Length', str(length))

        def send(self, status: int, body: bytes, kind: str = 'application/json; charset=utf-8',
                 chunked: bool = False, location: str | None = None) -> None:
            self.head(status, kind, None if chunked else len(body))
            if location:
                self.send_header('Location', location)
            self.end_headers()
            if chunked:
                for start in range(0, len(body), 4096):
                    part = body[start:start + 4096]
                    self.wfile.write(f'{len(part):x}\r\n'.encode() + part + b'\r\n')
                self.wfile.write(b'0\r\n\r\n')
            else:
                self.wfile.write(body)
            self.close_connection = True

        def note(self, mode: str, status: int, sent: int = 0) -> None:
            row = {'at': int(time.time() * 1000), 'path': self.path, 'mode': mode, 'status': status, 'bytes': sent}
            with fake.lock:
                fake.downloads.append(row)

        def tarball(self, mode: str, slow: float) -> None:
            """The real tarball from the file, with one byte changed in mode damaged, spread over `slow` seconds."""
            bundle = fake.bundle
            self.head(200, 'application/octet-stream', bundle.size)
            self.end_headers()
            middle, sent = bundle.size // 2, 0
            pause = slow / max(1, bundle.size // CHUNK) if slow else 0.0
            try:
                with bundle.path.open('rb') as data:
                    for block in iter(lambda: data.read(CHUNK), b''):
                        if mode == 'damaged' and sent <= middle < sent + len(block):
                            changed = bytearray(block)
                            changed[middle - sent] ^= 0xFF
                            block = bytes(changed)
                        self.wfile.write(block)
                        sent += len(block)
                        if pause:
                            time.sleep(pause)
            except (BrokenPipeError, ConnectionResetError):
                pass                                  # a Cancel: the client went away
            self.note(mode, 200, sent)
            self.close_connection = True

        def do_GET(self):   # noqa: N802 - http.server's name
            url = urlsplit(self.path)
            if url.path.startswith('/_fake/mode/'):
                mode = url.path.rsplit('/', 1)[1]
                if mode not in MODES:
                    return self.send(400, json.dumps({'error': f'unknown mode {mode}'}).encode())
                query = parse_qs(url.query)
                delay = query.get('delay', ['0'])[0]
                slow = query.get('slow', ['0'])[0]
                with fake.lock:
                    fake.mode = mode
                    fake.delay = min(10.0, max(0.0, float(delay or 0)))
                    fake.slow = min(30.0, max(0.0, float(slow or 0)))
                return self.send(200, json.dumps({'mode': fake.mode, 'delay': fake.delay, 'slow': fake.slow}).encode())
            if url.path == '/_fake/requests':
                with fake.lock:
                    return self.send(200, json.dumps(fake.requests).encode())
            if url.path == '/_fake/downloads':
                with fake.lock:
                    return self.send(200, json.dumps(fake.downloads).encode())
            if url.path == '/_fake/info':
                b = fake.bundle
                return self.send(200, json.dumps({'running': fake.running, 'offered': b.version, 'tarball': b.path.name,
                                                  'size': b.size, 'digest': b.digest}).encode())
            with fake.lock:
                mode, delay, slow = fake.mode, fake.delay, fake.slow
            name = fake.bundle.path.name
            if url.path.startswith(DOWNLOAD):
                if url.path == f'{DOWNLOAD}v{fake.bundle.version}/{name}':
                    self.note(mode, 302)
                    return self.send(302, b'', 'text/html; charset=utf-8', location=f'{CDN}{name}?token=fake')
                if url.path == f'{DOWNLOAD}v{fake.bundle.version}/{name}.sha256':
                    self.note(mode, 200, len(fake.bundle.sha256))
                    return self.send(200, fake.bundle.sha256, 'application/octet-stream')
                self.note(mode, 404)
                return self.send(404, b'{"message":"Not Found"}')
            if url.path == f'{CDN}{name}':
                return self.tarball(mode, slow)
            if url.path != FEED:
                return self.send(404, b'{"message":"Not Found"}')
            if delay:
                time.sleep(delay)
            status = {'error': 500, 'missing': 404}.get(mode, 200)
            row = {'at': int(time.time() * 1000), 'path': self.path, 'userAgent': self.headers.get('User-Agent'),
                   'accept': self.headers.get('Accept'), 'apiVersion': self.headers.get('X-GitHub-Api-Version'),
                   'mode': mode, 'status': status}
            with fake.lock:
                fake.requests.append(row)
                with open(fake.log, 'a') as out:
                    out.write(json.dumps(row) + '\n')
            if status == 500:
                return self.send(500, b'{"message":"Server Error"}')
            if status == 404:
                return self.send(404, b'{"message":"Not Found"}')
            body = json.dumps([] if mode == 'empty' else fake.releases(mode), indent=1).encode()
            return self.send(200, body, chunked=mode == 'newer-chunked')
    return Handler


def main(argv: list[str]) -> int:
    p = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    p.add_argument('--port', type=int, required=True)
    p.add_argument('--running', required=True, help='the version the hosts pretend to be (seam.json running)')
    p.add_argument('--bundle', required=True, help='the real release tarball (its .sha256 next to it)')
    p.add_argument('--log', required=True, help='JSON lines file of the list requests')
    args = p.parse_args(argv)
    fake = Fake(f'http://127.0.0.1:{args.port}/', args.running, Bundle(Path(args.bundle)), args.log)
    server = ThreadingHTTPServer(('127.0.0.1', args.port), handler(fake))
    server.daemon_threads = True
    print('ready', flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
