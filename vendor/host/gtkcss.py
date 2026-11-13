"""Windows 11 caption buttons in GTK apps: a managed block in the user's gtk.css (SPEC §5 theme.captionButtons).

Our CSS lives in the package (``config/gtk/caption-buttons-gtk3.css`` / ``-gtk4.css`` + ``glyphs/``); the user's
``~/.config/gtk-3.0/gtk.css`` and ``~/.config/gtk-4.0/gtk.css`` only get one ``@import`` between two marker
comments, appended at the END of the file so it comes after (and wins over) whatever is there already, e.g. the
``@import`` of Fluent's gtk-4.0 css. Every other line is kept byte for byte. GTK honours an ``@import`` anywhere in
the file (checked with GTK 3.24 and 4.22).

* idempotent: nothing is written when the block is already exactly right (wherever it stands in the file);
  a stale block (other package path) is replaced at the end;
* one-time backup: before the first change to an existing, non-empty file that has no block yet, the file is
  copied to ``gtk.css.win11shell-backup`` (never overwritten later);
* removable: ``enabled=False`` removes the block; a file that held nothing else is deleted again;
* a ``gtk.css`` that is a symlink (``ln -s ~/.themes/X/gtk-4.0/gtk.css``) becomes a regular file that
  ``@import``s the link's target first (same effect), the backup keeps the link itself;
* an unreadable file (or a directory) is never touched: the result says why;
* thread-safe: one change at a time per process (``_LOCK``), each write through its own temp file
  (``mkstemp``) renamed over gtk.css, so a reader sees the old or the new file, never a partial one.

No policy here; the caller (services/settings.py) guards these with ``@dangerous`` (dry-run, unit tests) and
decides which state is current.
"""

from __future__ import annotations

import logging
import os
import shutil
import tempfile
import threading
from pathlib import Path

log = logging.getLogger("win11shell.gtkcss")

BEGIN_TAG = ">>> win11shell caption-buttons >>>"
END_TAG = "<<< win11shell caption-buttons <<<"
BEGIN_LINE = (f"/* {BEGIN_TAG} Windows 11-vensterknoppen, beheerd door Win11 Shell "
              "(uitzetten: Instellingen > Persoonlijke instellingen > Kleuren). Niet wijzigen. */")
END_LINE = f"/* {END_TAG} */"
BACKUP_SUFFIX = ".win11shell-backup"

#: one read-modify-write of a gtk.css at a time (the settings watcher may start several threads)
_LOCK = threading.RLock()

#: gtk.css (relative to $XDG_CONFIG_HOME) → our stylesheet (relative to the package root)
TARGETS = {
    "gtk-3.0/gtk.css": "config/gtk/caption-buttons-gtk3.css",
    "gtk-4.0/gtk.css": "config/gtk/caption-buttons-gtk4.css",
}


def _uri(path: Path) -> str:
    return Path(os.path.abspath(path)).as_uri()


def block_text(css_path: Path) -> str:
    """The managed block for ``css_path`` (with a trailing newline)."""
    return f'{BEGIN_LINE}\n@import url("{_uri(css_path)}");\n{END_LINE}\n'


def strip_block(text: str) -> tuple[str, bool]:
    """``text`` without our block(s); also a dangling BEGIN marker (and the import right after it)."""
    lines = text.splitlines(keepends=True)
    out: list[str] = []
    found = False
    i = 0
    while i < len(lines):
        if BEGIN_TAG in lines[i]:
            found = True
            j = i + 1
            while j < len(lines) and END_TAG not in lines[j]:
                j += 1
            if j < len(lines):                      # complete block: drop BEGIN..END
                i = j + 1
            else:                                   # no END marker: drop BEGIN + our own import lines after it
                i += 1
                while i < len(lines) and "@import" in lines[i] and "caption-buttons-gtk" in lines[i]:
                    i += 1
            continue
        if END_TAG in lines[i]:                     # stray END marker
            found = True
            i += 1
            continue
        out.append(lines[i])
        i += 1
    return "".join(out), found


def _read(path: Path) -> str:
    # surrogateescape: bytes that are not UTF-8 come back out unchanged
    return path.read_bytes().decode("utf-8", errors="surrogateescape")


def _write_atomic(path: Path, text: str, mode: int) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    data = text.encode("utf-8", errors="surrogateescape")
    # a temp file of its own (never shared with another thread or process), renamed over the target
    fd, tmp = tempfile.mkstemp(dir=path.parent, prefix=f".{path.name}.", suffix=".win11shell.tmp")
    try:
        with os.fdopen(fd, "wb") as f:
            f.write(data)
            f.flush()
            os.fsync(f.fileno())
        os.chmod(tmp, mode)
        os.replace(tmp, path)
    except BaseException:
        try:
            os.unlink(tmp)
        except OSError:
            pass
        raise


def _backup(path: Path) -> str | None:
    """One-time copy of ``path`` (a symlink is copied as a symlink) to ``gtk.css.win11shell-backup``."""
    bak = path.with_name(path.name + BACKUP_SUFFIX)
    if bak.exists() or bak.is_symlink():
        return None
    if path.is_symlink():
        os.symlink(os.readlink(path), bak)
    else:
        shutil.copy2(path, bak)
    return str(bak)


def apply_block(gtk_css: str | os.PathLike, css_path: str | os.PathLike, enabled: bool) -> dict:
    """Add (``enabled``) or remove our block in one gtk.css. Never raises for file problems.

    → ``{ok, changed, action, path, backup?, error?}``; action: created | added | updated | removed | deleted |
    unchanged | skipped."""
    with _LOCK:
        return _apply_block(Path(gtk_css), Path(css_path), enabled)


def _apply_block(path: Path, css_path: Path, enabled: bool) -> dict:
    res: dict = {"ok": True, "changed": False, "action": "unchanged", "path": str(path)}
    is_link = path.is_symlink()
    if path.exists() and path.is_dir():
        return {**res, "ok": False, "action": "skipped", "error": "is a directory"}
    existed = path.exists() or is_link
    old = ""
    link_prefix = ""
    try:
        if is_link:
            target = Path(os.path.realpath(path))
            if target.is_file():
                link_prefix = f'@import url("{_uri(target)}");\n'
        elif existed:
            old = _read(path)
    except OSError as e:
        return {**res, "ok": False, "action": "skipped", "error": f"unreadable: {e.strerror or e}"}

    if is_link:
        # the link's own content is never ours to edit: it is replaced by an import of its target
        base, had_block = link_prefix, False
        if not enabled:
            return res                                   # a link never holds our block
    else:
        base, had_block = strip_block(old)

    if enabled:
        if not css_path.is_file():
            return {**res, "ok": False, "action": "skipped", "error": f"missing {css_path}"}
        if not is_link and old.count(BEGIN_TAG) == 1 and block_text(css_path) in old:
            return res                                   # already there (wherever the user keeps it)
        sep = "" if (not base or base.endswith("\n")) else "\n"
        if base and not base.endswith("\n\n"):
            sep += "\n"                                  # one empty line before our block
        new = base + sep + block_text(css_path)
    else:
        if not had_block:
            return res
        new = base.rstrip("\n") + "\n" if base.strip() else ""

    if not is_link and new == old:
        return res
    try:
        mode = (path.stat().st_mode & 0o777) if (existed and not is_link) else 0o644
        backup = None
        if existed and not had_block and (old.strip() or is_link):
            backup = _backup(path)
        if not enabled and not new:
            path.unlink()
            action = "deleted"
        else:
            _write_atomic(path, new, mode)
            action = ("updated" if had_block else "added") if (existed and enabled) else \
                     ("created" if enabled else "removed")
    except OSError as e:
        return {**res, "ok": False, "action": "skipped", "error": f"not written: {e.strerror or e}"}
    res.update(changed=True, action=action)
    if backup:
        res["backup"] = backup
    log.info("Vensterknoppen %s: %s (%s)", "aan" if enabled else "uit", path, action)
    return res


def apply_all(enabled: bool, root: str | os.PathLike, config_home: str | os.PathLike) -> dict:
    """Both gtk.css files. → ``{ok, changed, files: [apply_block results]}``."""
    with _LOCK:
        files = [apply_block(Path(config_home) / rel, Path(root) / css, enabled) for rel, css in TARGETS.items()]
    return {"ok": all(f["ok"] for f in files), "changed": any(f["changed"] for f in files), "files": files}


def status(root: str | os.PathLike, config_home: str | os.PathLike) -> dict:
    """Read-only: is our block present (and pointing at this package) in each gtk.css?"""
    out = {}
    for rel, css in TARGETS.items():
        p = Path(config_home) / rel
        try:
            text = _read(p) if p.is_file() else ""
        except OSError:
            out[rel] = "unreadable"
            continue
        if BEGIN_TAG not in text:
            out[rel] = "absent"
        else:
            out[rel] = "current" if block_text(Path(root) / css) in text else "stale"
    return out
