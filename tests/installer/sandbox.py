"""GE1c installer test sandbox: run scripts/install.py (or uninstall.py) for one case, never against Robin's HOME.

Every run happens in a case directory under tests/installer/run/ and, inside it:
* bubblewrap: the whole filesystem read-only, Robin's home hidden behind a tmpfs (only this repository is bound
  back, read-only, and the case directory read-write), private /tmp, /var/tmp, /run/user and /run/dbus, no network,
  its own PID namespace; so nothing outside the case directory can be written or even read from the live session;
* a complete replacement environment (HOME, XDG_*, WIN11_INSTALL_PREFIX, PATH with stubs for gnome-extensions,
  gsettings, dconf, systemctl, … that only log and fail), keyfile GSettings, no session or system bus;
* optionally strace -f: every mutating file syscall must name a path inside the case directory.
The suite itself is started through tests/installer/run.sh (tools/heavy.sh + tests/i18n/private.sh).
"""
from __future__ import annotations
import hashlib
import json
import os
from pathlib import Path
import pwd
import re
import shutil
import subprocess

G = Path(__file__).resolve().parents[2]
RUN = G/"tests/installer/run"
REAL_HOME = Path(pwd.getpwuid(os.getuid()).pw_dir)
# GW7: the XDG autostart entry of the post-upgrade check (scripts/install.py AUTOSTART), relative to a case dir
AUTOSTART_DIR = "config/autostart"
AUTOSTART = AUTOSTART_DIR + "/io.github.win11shell.PostUpgradeCheck.desktop"
STUBS = ("gnome-extensions gsettings dconf systemctl systemd-run loginctl busctl pkexec sudo su apt apt-get pkcon "
         "gnome-shell xdg-open notify-send gnome-session-quit snap flatpak").split()

WRITE_SYSCALL = re.compile(r'^(?:\[pid\s+\d+\]\s+)?(\d+\s+)?(openat|open|creat|mkdir|mkdirat|rename|renameat|renameat2|'
                           r'unlink|unlinkat|rmdir|symlink|symlinkat|link|linkat|chmod|fchmodat|fchmodat2|chown|'
                           r'fchownat|lchown|truncate|utimensat|setxattr|lsetxattr)\((.*)$')
QUOTED = r'"((?:[^"\\]|\\.)*)"'


def strace_paths(line: str):
    """Locations one mutating `strace -f -y` line touches (None when it only reads); after tests/gnome/harness.py."""
    m = WRITE_SYSCALL.match(line)
    if not m:
        return None
    call, rest = m.group(2), m.group(3)
    if call in ("openat", "open") and not re.search(r"O_WRONLY|O_RDWR|O_CREAT|O_TRUNC", rest):
        return None
    if call in ("symlink", "symlinkat"):
        rest = re.sub("^" + QUOTED + ", ", "", rest, count=1)        # the link target is content, not a location
    if call in ("setxattr", "lsetxattr"):
        rest = rest.split(", ", 1)[0]
    paths = [name if name.startswith("/") or not fd else f"{fd}/{name}"
             for fd, name in re.findall(r"(?:\d+<([^>]*)>|AT_FDCWD), " + QUOTED, rest)]
    if not paths:
        paths = re.findall(QUOTED, rest)
    if not paths:
        paths = re.findall(r"\d+<([^>]*)>", rest)
    return call, [bytes(p, "utf-8").decode("unicode_escape").encode("latin-1").decode("utf-8", "replace") for p in paths]


class Case:
    """One private install world: prefix, data, config, home, tmp, stubs and logs under RUN/<name>."""

    def __init__(self, name: str, fresh: bool = True):
        self.dir = RUN/name
        if fresh and self.dir.exists():
            shutil.rmtree(self.dir)
        for sub in ("home", "config", "data", "cache", "prefix", "tmp", "stubs", "logs"):
            (self.dir/sub).mkdir(parents=True, exist_ok=True)
        os.chmod(self.dir/"home", 0o700)
        self.calls = self.dir/"logs/stub-calls.log"
        self.calls.touch()
        for stub in STUBS:
            path = self.dir/"stubs"/stub
            path.write_text("#!/bin/sh\n"
                            f"printf '%s\\t%s\\n' '{stub}' \"$*\" >> '{self.calls}'\n"
                            f"echo 'GE1c stub: {stub} is blocked in the installer sandbox' >&2\nexit 97\n")
            path.chmod(0o755)
        self.runs = 0

    @property
    def prefix(self) -> Path:
        return self.dir/"prefix"

    @property
    def data(self) -> Path:
        return self.dir/"data"

    @property
    def config(self) -> Path:
        return self.dir/"config"

    def env(self, build: Path, locale: dict[str, str] | None = None, extra: dict[str, str] | None = None) -> dict:
        env = {
            "PATH": f"{self.dir/'stubs'}:/usr/bin:/bin",
            "HOME": str(self.dir/"home"), "USER": pwd.getpwuid(os.getuid()).pw_name, "SHELL": "/bin/sh",
            "XDG_CONFIG_HOME": str(self.config), "XDG_DATA_HOME": str(self.data), "XDG_CACHE_HOME": str(self.dir/"cache"),
            "XDG_RUNTIME_DIR": str(self.dir/"tmp"), "TMPDIR": str(self.dir/"tmp"),
            "XDG_DATA_DIRS": "/usr/local/share:/usr/share", "XDG_CONFIG_DIRS": "/etc/xdg", "XDG_CURRENT_DESKTOP": "ubuntu:GNOME",
            "WIN11_INSTALL_PREFIX": str(self.prefix), "WIN11_BUILD_DIR": str(build),
            "GSETTINGS_BACKEND": "keyfile", "NO_AT_BRIDGE": "1", "GTK_A11Y": "none", "GIO_USE_VFS": "local",
            "GVFS_DISABLE_FUSE": "1", "GIO_USE_VOLUME_MONITOR": "unix",
            "DBUS_SESSION_BUS_ADDRESS": f"unix:path={self.dir/'tmp/no-session-bus'}",
            "DBUS_SYSTEM_BUS_ADDRESS": f"unix:path={self.dir/'tmp/no-system-bus'}",
            "PYTHONDONTWRITEBYTECODE": "1", "TZ": "UTC", "LANG": "C.UTF-8",
        }
        env.update(locale or {})
        env.update(extra or {})
        return {k: v for k, v in env.items() if v is not None}

    def run(self, script: Path, root: Path, args: list[str], env: dict, strace: bool = False,
            timeout: int = 900) -> subprocess.CompletedProcess:
        """script (install.py/uninstall.py, any copy) inside bwrap (+ strace); returns stdout/stderr/returncode."""
        self.runs += 1
        tag = f"{self.runs:02d}-{script.stem}"
        interpreter = ["/bin/bash"] if script.suffix == ".sh" else ["/usr/bin/python3", "-B"]
        argv = interpreter + [str(script)] + ([str(root)] if root else []) + list(args)
        trace = self.dir/"logs"/f"{tag}.strace"
        if strace:
            argv = ["strace", "-f", "-qq", "-y", "-e", "trace=%file", "-e", "signal=none", "-o", str(trace)] + argv
        bw = ["/usr/bin/bwrap", "--ro-bind", "/", "/", "--dev", "/dev", "--proc", "/proc",
              "--tmpfs", "/tmp", "--tmpfs", "/var/tmp", "--tmpfs", "/run/user", "--tmpfs", "/run/dbus",
              "--tmpfs", str(REAL_HOME), "--ro-bind", str(G), str(G), "--bind", str(self.dir), str(self.dir),
              "--unshare-all", "--die-with-parent", "--new-session", "--chdir", str(self.dir), "--"]
        proc = subprocess.run(bw + argv, env=env, capture_output=True, text=True, timeout=timeout)
        (self.dir/"logs"/f"{tag}.out").write_text(proc.stdout)
        (self.dir/"logs"/f"{tag}.err").write_text(proc.stderr)
        proc.trace = trace if strace else None
        return proc

    def writes_outside(self, trace: Path) -> tuple[int, list[str], list[str]]:
        """(mutating paths, successful writes outside the case dir, failed attempts outside it). Read line by line: a
        REAL install traces ~600 MB, and reading it at once took the memory watchdog's whole margin."""
        count, outside, failed = 0, [], []
        root = os.path.realpath(self.dir)
        with open(trace, encoding="utf-8", errors="replace") as stream:
            for line in stream:
                parsed = strace_paths(line.rstrip("\n"))
                if not parsed:
                    continue
                call, paths = parsed
                for path in paths:
                    absolute = path if path.startswith("/") else os.path.join(self.dir, path)
                    count += 1
                    inside = os.path.realpath(absolute) == root or os.path.realpath(absolute).startswith(root + "/")
                    if not inside and not absolute.startswith(("/dev/null", "/dev/tty", "/proc/self/")):
                        (failed if " = -1 " in line else outside).append(f"{call} {absolute}")
        return count, sorted(set(outside)), sorted(set(failed))

    def stub_calls(self) -> list[str]:
        return [line for line in self.calls.read_text().splitlines() if line]

    def tree(self, skip=("logs", "stubs", "tmp"), autostart: bool = False) -> dict[str, str]:
        """Every path below the case dir (minus logs/stubs/tmp) → type, mode and content hash (no times).

        GW7: without ``autostart`` the post-upgrade check's autostart entry (design §6.6) and an
        ``config/autostart`` folder holding nothing else are left out: install.py writes them since GW7 and the
        3b898e6 installer never did, so the byte-for-byte comparisons with 3b898e6 compare everything else, and the
        ``Autostart`` cases of test_installer.py check that entry and those two paths exactly."""
        out = self._tree(skip)
        if not autostart:
            out.pop(AUTOSTART, None)
            if not any(key.startswith(AUTOSTART_DIR + "/") for key in out):
                out.pop(AUTOSTART_DIR, None)
        return out

    def _tree(self, skip) -> dict[str, str]:
        out = {}
        for directory, dirs, files in os.walk(self.dir):
            rel_dir = os.path.relpath(directory, self.dir)
            if rel_dir.split(os.sep)[0] in skip:
                dirs[:] = []
                continue
            for name in sorted(dirs + files):
                path = os.path.join(directory, name)
                rel = os.path.relpath(path, self.dir)
                if rel.split(os.sep)[0] in skip:
                    continue
                st = os.lstat(path)
                if os.path.islink(path):
                    out[rel] = f"link {os.readlink(path)}"
                elif os.path.isdir(path):
                    out[rel] = f"dir {oct(st.st_mode & 0o7777)}"
                else:
                    out[rel] = f"file {oct(st.st_mode & 0o7777)} {hashlib.sha256(Path(path).read_bytes()).hexdigest()}"
        return out


def write_json(path: Path, data, mode: int = 0o600) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.ge1c")
    temporary.write_text(json.dumps(data, indent=2, ensure_ascii=False) + "\n")
    os.chmod(temporary, mode)
    os.replace(temporary, path)


def settings_doc(language) -> dict:
    defaults = json.loads((G/"host/src/settings-defaults.json").read_text())
    return {**defaults, "language": language}


def bins_of(text: str) -> list[str]:
    """The names in an install.py's BINS tuple, plus its NATIVE_BINS when it has one (D-G63)."""
    names = re.findall(r'"(win11-[a-z-]+)"', re.search(r"^BINS = \((.*?)\)", text, re.S | re.M).group(1))
    native = re.search(r"^NATIVE_BINS = \((.*?)\)", text, re.S | re.M)
    if native:
        names += re.findall(r'"(win11-[a-z-]+)"', native.group(1))
    return list(dict.fromkeys(names))


def baseline_bins(commit: str = "3b898e6") -> list[str]:
    """BINS of scripts/install.py at the baseline commit (with win11-start and win11-desktop, D-G70 removed them)."""
    text = subprocess.run(["git", "-C", str(G), "show", f"{commit}:scripts/install.py"], capture_output=True,
                          text=True, check=True).stdout
    return bins_of(text)


def stub_build(directory: Path) -> Path:
    """Non-ELF placeholder executables (never run) for cases that test language and files, not libraries: every
    binary of the current install.py and of the 3b898e6 one, so both installers of a byte-for-byte case find theirs."""
    directory.mkdir(parents=True, exist_ok=True)
    names = list(dict.fromkeys(bins_of((G/"scripts/install.py").read_text()) + baseline_bins()))
    for name in names:
        path = directory/name
        if not path.exists():
            path.write_text(f"#!/bin/sh\n# GE1c placeholder for {name}; installer tests never run it\nexit 97\n")
            path.chmod(0o755)
    return directory


def fixture_root(directory: Path, extension_extra: dict[str, str] | None = None,
                 icons: dict[str, str] | None = None) -> Path:
    """A small source tree install.py accepts: a real copy of extension/ and host/src's two JSON tables, plus a
    tiny but valid vendor/ (ui/assets, a two-theme icon bundle with its manifest and source archive, and a host/
    that only 3b898e6's installer requires: the current one no longer reads or installs vendor/host). ``icons``:
    more files of the Win11 theme (path below vendor/icons/Win11 -> text), in the bundle's manifest too."""
    if directory.exists():
        shutil.rmtree(directory)
    shutil.copytree(G/"extension", directory/"extension")
    for name, text in (extension_extra or {}).items():
        (directory/"extension"/name).write_text(text)
    (directory/"host/src").mkdir(parents=True)
    for name in ("desktop-entries.json", "settings-defaults.json"):
        shutil.copy2(G/"host/src"/name, directory/"host/src"/name)
    vendor = directory/"vendor"
    (vendor/"ui/assets/icons").mkdir(parents=True)
    (vendor/"ui/assets/icons/settings.svg").write_text("<svg xmlns='http://www.w3.org/2000/svg'/>\n")
    (vendor/"host").mkdir()
    (vendor/"host/README").write_text("GE1c fixture\n")
    entries = {}
    for theme in ("Win11", "Win11-dark"):
        base = vendor/"icons"/theme
        (base/"16x16/apps").mkdir(parents=True)
        files = {"COPYING": "GPL fixture\n", "AUTHORS": "GE1c\n",
                 "index.theme": f"[Icon Theme]\nName={theme}\nDirectories=16x16/apps\n\n[16x16/apps]\nSize=16\n",
                 "16x16/apps/ge1c.svg": "<svg xmlns='http://www.w3.org/2000/svg'/>\n"}
        for rel, text in files.items():
            (base/rel).write_text(text)
        entries[theme] = None
    (vendor/"icons/Win11/16x16/apps/alias.svg").symlink_to("ge1c.svg")
    for rel, text in (icons or {}).items():
        (vendor/"icons/Win11"/rel).parent.mkdir(parents=True, exist_ok=True)
        (vendor/"icons/Win11"/rel).write_text(text)
    manifest_entries = {}
    for path in sorted((vendor/"icons").rglob("*")):
        rel = path.relative_to(vendor/"icons").as_posix()
        if path.is_symlink():
            manifest_entries[rel] = {"type": "symlink", "target": os.readlink(path)}
        elif path.is_file():
            data = path.read_bytes()
            manifest_entries[rel] = {"type": "file", "size": len(data), "sha256": hashlib.sha256(data).hexdigest()}
    (vendor/"icon-source").mkdir()
    archive = vendor/"icon-source/fixture.tar"
    archive.write_bytes(b"GE1c fixture archive\n")
    manifest = {"schema": 1, "aliases": {"Win11Shell": "Win11", "Win11Shell-dark": "Win11-dark"},
                "entries": manifest_entries,
                "upstream": {"source_archive": "icon-source/fixture.tar",
                             "archive_sha256": hashlib.sha256(archive.read_bytes()).hexdigest()}}
    (vendor/"icon-bundle.json").write_text(json.dumps(manifest, indent=1) + "\n")
    return directory


def old_scripts(directory: Path, commit: str = "3b898e6") -> Path:
    """scripts/*.py as they were at the baseline commit (for byte-for-byte output comparisons)."""
    directory.mkdir(parents=True, exist_ok=True)
    for name in ("install.py", "uninstall.py", "icon_bundle.py", "file_portal.py"):
        text = subprocess.run(["git", "-C", str(G), "show", f"{commit}:scripts/{name}"], capture_output=True,
                              text=True, check=True).stdout
        (directory/name).write_text(text)
    return directory


def desktop_entry(path: Path) -> dict[str, str]:
    out = {}
    for line in path.read_text().splitlines():
        if "=" in line and not line.startswith("["):
            key, value = line.split("=", 1)
            out[key] = value
    return out
