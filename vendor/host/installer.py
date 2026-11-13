#!/usr/bin/env python3
"""Win11 Shell installer helpers: stdlib only, called by install.sh, uninstall.sh and tools/make-themes.sh.

  python3 -I host/installer.py treehash DIR
        content hash of an installed theme (paths, file contents, symlink targets; icon-theme.cache ignored)
  python3 -I host/installer.py manifest THEMES_DIR
        validated themes/themes.tsv as tab-separated lines: name kind archive rev treehash
  python3 -I host/installer.py theme-targets MODE [FAMILY]
        what the theme sync (host/services/settings.py theme_targets) picks for a fresh install: key=value lines
        (FAMILY fluent, the default and the Win11 look, or system: Ubuntu's own look, as theme.gtkFamily)
  python3 -I host/installer.py settings-theme CONFIG_DIR
        mode=dark|light, sync=0|1 and family=fluent|system from CONFIG_DIR/settings.json (defaults when it does not
        exist)
  python3 -I host/installer.py gtk4-theme add GTK_CSS DARK_CSS LIGHT_CSS | remove GTK_CSS | status GTK_CSS
        the managed "win11shell theme" block at the top of ~/.config/gtk-4.0/gtk.css (libadwaita/GTK 4 apps read only
        that file): Fluent's gtk-4.0 css, the dark or light one following the colour scheme (GTK >= 4.20 evaluates
        @media (prefers-color-scheme) per provider, also around @import). The caption-buttons block of host/gtkcss.py
        stays where it is (at the end, after the theme). One-time backup gtk.css.win11shell-backup, shared with gtkcss.
  python3 -I host/installer.py caption-remove CONFIG_HOME
        remove the caption-buttons blocks of host/gtkcss.py from both gtk.css files (uninstall: they point into the
        program directory that is about to go)
  python3 -I host/installer.py backup-cleanup GTK_CSS
        delete gtk.css.win11shell-backup when it equals gtk.css again, or when it holds nothing but our own managed
        blocks (theme and caption buttons): nothing of the user's is lost
  python3 -I host/installer.py mimeapps-drop FILE APP TYPE...
        take APP out of [Default Applications] / [Added Associations] for TYPE in a mimeapps.list (rest kept)
  python3 -I host/installer.py bundle OUT.tar.xz SRCDIR NAME...
        deterministic tar.xz of SRCDIR/NAME… (uid 0, sorted, icon-theme.cache left out, absolute symlinks into SRCDIR
        stored as the equivalent relative link); prints "NAME<TAB>treehash" per theme

Every command prints one result word (or lines) on stdout and exits 0; 1 = could not do it (reason on stderr); 2 = usage.
"""
from __future__ import annotations

import hashlib
import io
import json
import os
import shutil
import stat
import subprocess
import sys
import tarfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

SKIP_NAMES = {"icon-theme.cache"}
THEME_BEGIN = ">>> win11shell theme >>>"
THEME_END = "<<< win11shell theme <<<"
THEME_BEGIN_LINE = (f"/* {THEME_BEGIN} Windows 11-uiterlijk (Fluent) voor GTK 4-apps, beheerd door Win11 Shell "
                    "(install.sh; ./uninstall.sh haalt het weg). Niet wijzigen. */")
THEME_END_LINE = f"/* {THEME_END} */"
BACKUP_SUFFIX = ".win11shell-backup"          # the same one-time backup as host/gtkcss.py


def _err(msg: str) -> int:
    print(f"installer.py: {msg}", file=sys.stderr)
    return 1


# ---------------------------------------------------------------- tree hash
def _entries(base: Path, fix_link=None):
    """(relpath, kind, payload) for everything under ``base`` in a stable order; kind d|f|l."""
    for dirpath, dirnames, filenames in os.walk(base, followlinks=False):
        dirnames.sort()
        rel_dir = os.path.relpath(dirpath, base)
        names = sorted(dirnames + filenames)
        for name in names:
            if name in SKIP_NAMES:
                continue
            p = Path(dirpath) / name
            rel = name if rel_dir == "." else f"{rel_dir}/{name}"
            st = os.lstat(p)
            if stat.S_ISLNK(st.st_mode):
                target = os.readlink(p)
                if fix_link:
                    target = fix_link(p, target)
                yield rel, "l", target
            elif stat.S_ISDIR(st.st_mode):
                yield rel, "d", ""
            elif stat.S_ISREG(st.st_mode):
                yield rel, "f", p
        # symlinked directories are listed by os.walk in dirnames but not followed: handled above as links
        dirnames[:] = [d for d in dirnames if not os.path.islink(os.path.join(dirpath, d))]


def _file_sha(p: Path) -> str:
    h = hashlib.sha256()
    with open(p, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def treehash(base: Path, fix_link=None) -> str:
    h = hashlib.sha256()
    for rel, kind, payload in _entries(base, fix_link):
        val = _file_sha(payload) if kind == "f" else payload
        h.update(f"{kind}\0{rel}\0{val}\n".encode("utf-8", "surrogateescape"))
    return h.hexdigest()


# ---------------------------------------------------------------- bundle (tools/make-themes.sh)
def bundle(out: Path, srcdir: Path, names: list[str]) -> int:
    srcdir = srcdir.resolve()

    def fix_link(p: Path, target: str) -> str:
        if os.path.isabs(target):
            t = Path(os.path.normpath(target))
            try:
                t.relative_to(srcdir)
            except ValueError:
                return target                     # points outside: kept as is (none in the themes we ship)
            return os.path.relpath(t, p.parent)
        return target

    raw = io.BytesIO()
    hashes = []
    with tarfile.open(fileobj=raw, mode="w", format=tarfile.GNU_FORMAT) as tar:
        for name in names:
            base = srcdir / name
            if not base.is_dir():
                return _err(f"{base} bestaat niet")
            h = hashlib.sha256()
            info = tar.gettarinfo(str(base), arcname=name)
            _norm(info)
            tar.addfile(info)
            for rel, kind, payload in _entries(base, fix_link):
                arc = f"{name}/{rel}"
                p = base / rel
                if kind == "l":
                    info = tarfile.TarInfo(arc)
                    info.type = tarfile.SYMTYPE
                    info.linkname = payload
                    info.mode = 0o777
                    info.mtime = int(os.lstat(p).st_mtime)
                    _norm(info)
                    tar.addfile(info)
                    h.update(f"l\0{rel}\0{payload}\n".encode("utf-8", "surrogateescape"))
                elif kind == "d":
                    info = tar.gettarinfo(str(p), arcname=arc)
                    _norm(info)
                    tar.addfile(info)
                    h.update(f"d\0{rel}\0\n".encode("utf-8", "surrogateescape"))
                else:
                    info = tar.gettarinfo(str(p), arcname=arc)
                    _norm(info)
                    with open(p, "rb") as f:
                        tar.addfile(info, f)
                    h.update(f"f\0{rel}\0{_file_sha(p)}\n".encode("utf-8", "surrogateescape"))
            hashes.append((name, h.hexdigest()))
    data = raw.getvalue()
    out.parent.mkdir(parents=True, exist_ok=True)
    tmp = out.with_name(out.name + ".tmp")
    if shutil.which("xz"):
        with open(tmp, "wb") as f:
            subprocess.run(["xz", "-9", "-T0", "-c"], input=data, stdout=f, check=True)
    else:
        import lzma
        tmp.write_bytes(lzma.compress(data, preset=9))
    os.replace(tmp, out)
    for name, hx in hashes:
        print(f"{name}\t{hx}")
    return 0


def _norm(info: tarfile.TarInfo) -> None:
    info.uid = info.gid = 0
    info.uname = info.gname = "root"
    if info.isdir():
        info.mode = 0o755
    elif info.isfile():
        info.mode = 0o755 if info.mode & 0o111 else 0o644


# ---------------------------------------------------------------- manifest
def manifest(themes_dir: Path) -> int:
    f = themes_dir / "themes.tsv"
    try:
        lines = f.read_text(encoding="utf-8").splitlines()
    except OSError as e:
        return _err(f"{f}: {e.strerror}")
    rows = []
    for ln in lines:
        if not ln.strip() or ln.startswith("#"):
            continue
        parts = ln.split("\t")
        if len(parts) != 5:
            return _err(f"{f}: kapotte regel: {ln!r}")
        name, kind, archive, rev, hx = parts
        ok = (kind in ("icons", "gtk") and name and "/" not in name and not name.startswith(".")
              and "/" not in archive and archive.endswith((".tar.xz", ".tar.gz")) and rev.isdigit()
              and len(hx) == 64 and all(c in "0123456789abcdef" for c in hx))
        if not ok:
            return _err(f"{f}: ongeldige regel: {ln!r}")
        if not (themes_dir / archive).is_file():
            return _err(f"{themes_dir / archive} ontbreekt")
        rows.append(parts)
    for r in rows:
        print("\t".join(r))
    return 0


# ---------------------------------------------------------------- theme sync (host/services/settings.py)
def theme_targets(mode: str, family: str = "fluent") -> int:
    if mode not in ("light", "dark"):
        return _err("mode moet light of dark zijn")
    if family not in ("fluent", "system"):
        return _err("family moet fluent of system zijn")
    from host.services.settings import theme_targets as tt
    if family == "fluent":
        # current=None: "no stock theme chosen yet" → the sync's own pick for the Win11 look (Fluent-round/Fluent)
        targets = tt(mode, None)
    else:
        # the user chose Ubuntu's own look in Instellingen › Kleuren: the sync's pick from the current theme
        targets = tt(mode, family="system")
    for k, v in targets.items():
        print(f"{k}={v}")
    return 0


def settings_theme(config_dir: Path) -> int:
    mode, sync, family = "dark", True, "fluent"
    try:
        from host.services.settings import DEFAULTS
        t = DEFAULTS.get("theme") or {}
        mode, sync, family = t.get("mode", mode), t.get("syncSystem", sync), t.get("gtkFamily", family)
    except Exception:  # noqa: BLE001
        pass
    try:
        data = json.loads((config_dir / "settings.json").read_text(encoding="utf-8"))
        t = data.get("theme") or {}
        if isinstance(t, dict):
            mode = t.get("mode", mode)
            sync = t.get("syncSystem", sync)
            family = t.get("gtkFamily", family)
    except (OSError, ValueError, AttributeError):
        pass
    if mode not in ("light", "dark"):
        mode = "dark"
    if family not in ("fluent", "system"):          # host/services/settings.py gtk_family()
        family = "fluent"
    print(f"mode={mode}")
    print(f"sync={1 if sync is not False else 0}")
    print(f"family={family}")
    return 0


# ---------------------------------------------------------------- gtk-4.0/gtk.css theme block
def _uri(p: str | Path) -> str:
    return Path(os.path.abspath(p)).as_uri()


def theme_block(dark: str | None, light: str | None) -> str:
    lines = [THEME_BEGIN_LINE]
    if dark and light:
        lines.append(f'@media (prefers-color-scheme: dark) {{ @import url("{_uri(dark)}"); }}')
        lines.append(f'@media (prefers-color-scheme: light) {{ @import url("{_uri(light)}"); }}')
    else:
        lines.append(f'@import url("{_uri(dark or light)}");')
    lines.append(THEME_END_LINE)
    return "\n".join(lines) + "\n"


def _strip(text: str, begin: str, end: str) -> tuple[str, bool]:
    lines = text.splitlines(keepends=True)
    out, found, i = [], False, 0
    while i < len(lines):
        if begin in lines[i]:
            found = True
            j = i + 1
            while j < len(lines) and end not in lines[j]:
                j += 1
            i = j + 1 if j < len(lines) else i + 1
            continue
        if end in lines[i]:
            found = True
            i += 1
            continue
        out.append(lines[i])
        i += 1
    return "".join(out), found


def _read(p: Path) -> str:
    return p.read_bytes().decode("utf-8", errors="surrogateescape")


def _write_atomic(p: Path, text: str, mode: int) -> None:
    p.parent.mkdir(parents=True, exist_ok=True)
    tmp = p.with_name(f".{p.name}.win11shell-{os.getpid()}.tmp")
    fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_TRUNC | os.O_CLOEXEC, mode)
    try:
        with os.fdopen(fd, "wb") as f:
            f.write(text.encode("utf-8", errors="surrogateescape"))
            f.flush()
            os.fsync(f.fileno())
        os.chmod(tmp, mode)
        os.replace(tmp, p)
    except BaseException:
        try:
            os.unlink(tmp)
        except OSError:
            pass
        raise


def _backup(p: Path) -> str | None:
    bak = p.with_name(p.name + BACKUP_SUFFIX)
    if bak.exists() or bak.is_symlink():
        return None
    if p.is_symlink():
        os.symlink(os.readlink(p), bak)
    else:
        shutil.copy2(p, bak)
    return str(bak)


def _foreign_fluent(text: str) -> str | None:
    """An @import of an existing Fluent gtk-4.0 css outside our block (the user set it up himself)."""
    import re
    for m in re.finditer(r'@import\s+url\(\s*["\']?([^"\')]+)["\']?\s*\)', text):
        u = m.group(1)
        path = u[7:] if u.startswith("file://") else u
        from urllib.parse import unquote
        path = unquote(path)
        if "Fluent" in path and "/gtk-4.0/" in path and os.path.isfile(path):
            return path
    return None


def gtk4_add(gtk_css: Path, dark: str, light: str) -> int:
    for c in (dark, light):
        if c and not os.path.isfile(c):
            return _err(f"{c} ontbreekt")
    if gtk_css.is_dir():
        print("skipped:directory")
        return 0
    if gtk_css.is_symlink():
        target = os.path.realpath(gtk_css)
        if "Fluent" in target and "/gtk-4.0/" in target and os.path.isfile(target):
            print(f"foreign:{target}")             # Fluent's own `install.sh -l` link: already themed
            return 0
    block = theme_block(dark, light)
    existed = gtk_css.exists() or gtk_css.is_symlink()
    old = ""
    try:
        if gtk_css.is_symlink():
            t = Path(os.path.realpath(gtk_css))
            old = f'@import url("{_uri(t)}");\n' if t.is_file() else ""
        elif existed:
            old = _read(gtk_css)
    except OSError as e:
        print(f"skipped:unreadable {e.strerror}")
        return 0
    rest, had = _strip(old, THEME_BEGIN, THEME_END)
    if not had:
        foreign = _foreign_fluent(rest)
        if foreign:
            print(f"foreign:{foreign}")
            return 0
    if not gtk_css.is_symlink() and old.count(THEME_BEGIN) == 1 and old.startswith(block):
        print("unchanged")
        return 0
    rest = rest.lstrip("\n")
    new = block + ("\n" + rest if rest else "")
    try:
        mode = (gtk_css.stat().st_mode & 0o777) if (existed and not gtk_css.is_symlink()) else 0o644
        if existed and not had and (old.strip() or gtk_css.is_symlink()):
            _backup(gtk_css)
        if gtk_css.is_symlink():
            gtk_css.unlink()
        _write_atomic(gtk_css, new, mode)
    except OSError as e:
        return _err(f"{gtk_css} niet geschreven: {e.strerror}")
    print("updated" if had else ("added" if existed else "created"))
    return 0


def gtk4_remove(gtk_css: Path) -> int:
    if not gtk_css.is_file() or gtk_css.is_symlink():
        print("unchanged")
        return 0
    try:
        old = _read(gtk_css)
    except OSError as e:
        print(f"skipped:unreadable {e.strerror}")
        return 0
    rest, had = _strip(old, THEME_BEGIN, THEME_END)
    if not had:
        print("unchanged")
        return 0
    rest = rest.lstrip("\n")
    try:
        if not rest.strip():
            gtk_css.unlink()
            print("deleted")
        else:
            _write_atomic(gtk_css, rest, gtk_css.stat().st_mode & 0o777)
            print("removed")
    except OSError as e:
        return _err(f"{gtk_css} niet geschreven: {e.strerror}")
    return 0


def gtk4_status(gtk_css: Path) -> int:
    try:
        text = _read(gtk_css) if gtk_css.is_file() else ""
    except OSError:
        print("unreadable")
        return 0
    print("present" if THEME_BEGIN in text else "absent")
    return 0


def caption_remove(config_home: Path) -> int:
    from host import gtkcss
    res = gtkcss.apply_all(False, ROOT, config_home)
    for f in res["files"]:
        print(f"{f['path']}\t{f['action']}" + (f"\t{f['error']}" if f.get("error") else ""))
    return 0


def backup_cleanup(gtk_css: Path) -> int:
    bak = gtk_css.with_name(gtk_css.name + BACKUP_SUFFIX)
    if not (bak.exists() or bak.is_symlink()):
        print("none")
        return 0
    same = False
    try:
        if bak.is_symlink() or gtk_css.is_symlink():
            same = bak.is_symlink() and gtk_css.is_symlink() and os.readlink(bak) == os.readlink(gtk_css)
        elif gtk_css.is_file():
            same = bak.read_bytes() == gtk_css.read_bytes()
    except OSError:
        same = False
    if not same and not bak.is_symlink():
        # a backup made of a file that held only our own blocks (install.sh created gtk.css with the theme block, then
        # host/gtkcss.py backed it up before adding the caption block): there is nothing of the user's in it
        try:
            rest, _ = _strip(_read(bak), THEME_BEGIN, THEME_END)
            try:
                from host import gtkcss
                rest, _ = gtkcss.strip_block(rest)
            except Exception:  # noqa: BLE001 - without gtkcss: only the theme block is recognised
                pass
            same = not rest.strip()
        except OSError:
            same = False
    if same:
        bak.unlink()
        print("removed")
    else:
        print(f"kept:{bak}")
    return 0


# ---------------------------------------------------------------- mimeapps.list
def mimeapps_drop(path: Path, app: str, types: list[str]) -> int:
    try:
        lines = _read(path).splitlines(keepends=True)
    except FileNotFoundError:
        print("unchanged")
        return 0
    except OSError as e:
        return _err(f"{path}: {e.strerror}")
    out, section, changed = [], "", False
    for ln in lines:
        s = ln.strip()
        if s.startswith("[") and s.endswith("]"):
            section = s[1:-1]
            out.append(ln)
            continue
        if section in ("Default Applications", "Added Associations") and "=" in s and not s.startswith("#"):
            key, _, val = s.partition("=")
            if key.strip() in types:
                apps = [a for a in val.split(";") if a]
                keep = [a for a in apps if a != app]
                if keep != apps:
                    changed = True
                    if keep:
                        out.append(f"{key.strip()}={';'.join(keep)};\n")
                    continue
        out.append(ln)
    if not changed:
        print("unchanged")
        return 0
    _write_atomic(path, "".join(out), path.stat().st_mode & 0o777)
    print("changed")
    return 0


# ---------------------------------------------------------------- main
def main(argv: list[str]) -> int:
    if not argv:
        print(__doc__, file=sys.stderr)
        return 2
    cmd, args = argv[0], argv[1:]
    try:
        if cmd == "treehash" and len(args) == 1:
            if not Path(args[0]).is_dir():
                return _err(f"{args[0]} is geen map")
            print(treehash(Path(args[0])))
            return 0
        if cmd == "manifest" and len(args) == 1:
            return manifest(Path(args[0]))
        if cmd == "theme-targets" and len(args) in (1, 2):
            return theme_targets(*args)
        if cmd == "settings-theme" and len(args) == 1:
            return settings_theme(Path(args[0]))
        if cmd == "gtk4-theme" and args[:1] == ["add"] and len(args) == 4:
            return gtk4_add(Path(args[1]), args[2] or None, args[3] or None)
        if cmd == "gtk4-theme" and args[:1] == ["remove"] and len(args) == 2:
            return gtk4_remove(Path(args[1]))
        if cmd == "gtk4-theme" and args[:1] == ["status"] and len(args) == 2:
            return gtk4_status(Path(args[1]))
        if cmd == "caption-remove" and len(args) == 1:
            return caption_remove(Path(args[0]))
        if cmd == "backup-cleanup" and len(args) == 1:
            return backup_cleanup(Path(args[0]))
        if cmd == "mimeapps-drop" and len(args) >= 3:
            return mimeapps_drop(Path(args[0]), args[1], args[2:])
        if cmd == "bundle" and len(args) >= 3:
            return bundle(Path(args[0]), Path(args[1]), args[2:])
    except OSError as e:
        return _err(f"{cmd}: {e}")
    print(__doc__, file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
