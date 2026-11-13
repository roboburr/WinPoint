"""Decode one untrusted image in a short-lived child process (used by host/server.py).

    python3 -I host/thumbnailer.py <image> <max size px, 0 = original (≤ 4096)> <png|jpeg>
    → stdout: b"P" + PNG bytes  or  b"J" + JPEG bytes ; exit 0.  Anything else: exit 1, nothing on stdout.

GdkPixbuf's loaders for TIFF, HEIC/AVIF, JPEG XL, RAW, TGA, ICO, XPM, OpenEXR, DDS … parse bytes from photos
people download. A crash (or worse) in one of them must never take down the desktop process — the one that
draws the taskbar and holds the API token. So the host only decodes PNG/JPEG/GIF/WebP itself and runs this
helper for everything else: separate process, low priority, CPU/memory/file-size limits, and (when ``bwrap``
is installed) a read-only, network-less sandbox. Standalone on purpose (stdlib + PyGObject, no ``host``
imports), so it can run with ``python3 -I``.
"""

from __future__ import annotations

import os
import resource
import sys

MAX_SIDE = 4096


def _limits() -> None:
    for lim, val in ((resource.RLIMIT_AS, 2 << 30), (resource.RLIMIT_CPU, 15), (resource.RLIMIT_FSIZE, 128 << 20),
                     (resource.RLIMIT_CORE, 0)):
        try:
            soft, hard = resource.getrlimit(lim)
            new = val if hard == resource.RLIM_INFINITY else min(val, hard)
            resource.setrlimit(lim, (new, new if hard == resource.RLIM_INFINITY else hard))
        except (ValueError, OSError):
            pass
    try:
        os.nice(10)
    except OSError:
        pass


def main(argv: list[str]) -> int:
    if len(argv) != 3:
        return 2
    path, size_s, fmt = argv
    try:
        size = max(0, min(MAX_SIDE, int(size_s)))
    except ValueError:
        return 2
    _limits()
    try:
        import gi
        gi.require_version("GdkPixbuf", "2.0")
        from gi.repository import GdkPixbuf
    except Exception:  # noqa: BLE001
        return 1
    try:
        if size:
            pb = GdkPixbuf.Pixbuf.new_from_file_at_scale(path, size, size, True)
        else:
            pb = GdkPixbuf.Pixbuf.new_from_file(path)
            if pb.get_width() > MAX_SIDE or pb.get_height() > MAX_SIDE:
                pb = pb.scale_simple(*_fit(pb.get_width(), pb.get_height(), MAX_SIDE),
                                     GdkPixbuf.InterpType.BILINEAR)
        pb = pb.apply_embedded_orientation() or pb
        if fmt == "jpeg" and not pb.get_has_alpha():
            ok, data = pb.save_to_bufferv("jpeg", ["quality"], ["85"])
            tag = b"J"
        else:
            ok, data = pb.save_to_bufferv("png", [], [])
            tag = b"P"
    except Exception:  # noqa: BLE001 - unreadable / unsupported / corrupt
        return 1
    if not ok:
        return 1
    out = sys.stdout.buffer
    out.write(tag)
    out.write(bytes(data))
    out.flush()
    return 0


def _fit(w: int, h: int, side: int) -> tuple[int, int]:
    if w >= h:
        return side, max(1, round(h * side / w))
    return max(1, round(w * side / h)), side


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
