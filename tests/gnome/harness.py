#!/usr/bin/env python3
"""GE0c: private headless GNOME Shell harness for Win11 voor GNOME.

Robin's live desktop is this GNOME version. Nothing here may reach it. Layers, outermost first:

1. ``launch`` (the only command meant to start from a live shell) starts nothing but bubblewrap, with a
   complete replacement environment (``private_env``). R3-NATIVE (D-G42) built the root from GW1's allowlist
   (tests/update/run.sh), not ``--ro-bind / /``: read-only /usr (+ the /bin, /lib… links), /etc, /sys, the
   font cache, udev's database and the checkout; writable only tests/gnome/run/ and the run's output
   directory; private tmpfs /tmp (with XDG_RUNTIME_DIR), /run (empty dbus, user and systemd directories) and
   /var; the real home, /var/lib, /var/snap, /srv, /opt and /media are not there, and / is read-only (an empty
   directory, run/root, on the checkout's file system, so the storage service sees a system drive at /; other
   host mounts stay out). Every host socket the binds would still show (from /proc/net/unix, read outside) is
   covered by /dev/null; the guard refuses if one is still a socket or another socket is in the checkout or in a
   bound host directory, and during the run a socket watch stops the session when a socket file appears there
   (every 2 s; a read-only bind does not stop connect()). A minimal ``/dev`` (GPU nodes
   only), its own PID namespace (nothing outside can be signalled, and everything dies with the sandbox) and
   its own network namespace (loopback only; no downloads, and the live session's abstract sockets are
   unreachable). ``run --strace FILE`` logs every connect() inside and fails the run if one reached a socket
   outside the sandbox; a run ends by removing the socket files it left in run/ and failing if any socket file
   is left in the checkout. Every run gets GIO_USE_VFS=local and the update service's private test seams
   (WIN11_UPDATE_TOOL_DIR, WIN11_UPDATE_FIXTURE_ROOT in run/; D-G44). WinPoint's own update check (D-G74a) gets
   its seam only with ``--winpoint-feed`` (WIN11_WINPOINT_TEST=/tmp/w11-winpoint, a fake GitHub on loopback,
   tests/gnome/lib/fake_github.py); without it the seeded settings turn its daily check off.
2. ``run`` and ``_inner`` refuse to start unless the guard passes: HOME is not the real home,
   XDG_RUNTIME_DIR is not /run/user/<uid>, the session bus address does not point into /run/user/<uid>,
   DISPLAY and WAYLAND_DISPLAY are unset, GSETTINGS_BACKEND=keyfile, plus the stricter checks in
   ``env_problems`` and the sandbox checks in ``sandbox_problems``. ``guard-selftest`` proves each refusal.
3. ``run`` takes $G/.heavy.lock (like ``flock -w 3600``), checks MemAvailable > 2 GB, empties
   tests/gnome/run/, installs target/debug into a private prefix with scripts/install.py
   under strace (every written path must be inside tests/gnome/run/), seeds settings.json and a keyfile
   GSettings database, then starts ``dbus-run-session`` (private config, no service directories) with
   ``_inner``. A sampler records the memory of every process in the sandbox and aborts below 1.5 GB.
4. ``_inner`` starts an empty private system bus, then ``gnome-shell --headless --unsafe-mode
   --virtual-monitor=1440x900 --wayland-display=w11-wl --no-x11`` with the unmodified extension copy, waits for
   the native GTK 4 shell (D-G70: win11-panel, started by the extension, execs win11-shell, which holds the
   taskbar, its flyouts, Start and the desktop in one process with ``win11-panel --http-only`` as its backend;
   no WebKit) to report its boot on WIN11_DEV_REPORT, opens each surface (the shell's and the native GTK 4
   apps' through screenshots and their dev state on WIN11_DEV_COMMAND/WIN11_DEV_REPORT, native/shell/src/dev.rs
   and native/kit/src/dev.rs), takes screenshots with org.gnome.Shell.Screenshot on the private bus, optionally
   switches the language live by rewriting the private settings.json, and shuts down only its own descendants.

GE1h added: ``--build-dir`` (which 19 binaries to install; default tests/build/native-release), ``--source``
(install the assets, extension and scripts of the working tree or of a git revision), ``--scenario FILE`` (a Python
file with ``def run(h)``, run inside the sandbox after the session is up; ``h`` is tests/gnome/lib/scenario.py's
Helper), the F24 guard refusals, and the heavy lock taken by an ancestor ``tools/heavy.sh`` (contracts C8).

Usage (from G):
  python3 -B tests/gnome/harness.py guard-selftest
  tools/heavy.sh --min-mb 2560 --tag <step> -- python3 -B tests/gnome/harness.py launch run --lang nl --switch
  tools/heavy.sh --min-mb 2560 --tag <step> -- python3 -B tests/gnome/harness.py launch run --lang nl \\
      --scenario tests/gnome/scenarios/native-shell.py
See tests/gnome/README.md.
"""
from __future__ import annotations

import argparse
import ctypes
import fcntl
import glob
import hashlib
import importlib.util
import json
import os
import pwd
import re
import shutil
import signal
import socket
import stat
import subprocess
import sys
import threading
import time
import urllib.parse
from pathlib import Path

HERE = Path(__file__).resolve().parent            # G/tests/gnome
G = HERE.parent.parent
RUN = HERE / 'run'                                # all state of one run; wiped at the start of each run
# XDG_RUNTIME_DIR, inside the sandbox's own /tmp tmpfs (R3-NATIVE): RUN/rt in a worktree such as .wt/r3-native
# made RT/bus 100 bytes, over dbus-daemon's 99-byte socket path limit; and no runtime socket (the buses, Wayland,
# the hosts' event sockets) can be left in the checkout, since that tmpfs ends with the sandbox.
RT = Path('/tmp/w11-rt')
BASELINE = HERE / 'baseline'
LIB = HERE / 'lib'
# The machine-wide heavy lock (tools/heavy.sh honours the same override; outside the sandbox only).
LOCK = Path(os.environ.get('WIN11_HEAVY_LOCK') or G / '.heavy.lock')
# D-G70: a build dir with the host binaries plus win11-shell, win11-calendar and the win11-gtk-<kind> apps (native/APPS.md)
# copied from the native workspace's target (an older dir may still hold win11-start/-desktop: they are not installed,
# install.py BINS no longer has them)
DEFAULT_BUILD = G / 'tests' / 'build' / 'native-release'
BUILD = DEFAULT_BUILD                             # set from --build-dir (set_build_dir) before private_env()
# --session-mode winpoint (13-11): GNOME Shell in WinPoint's own login session mode (scripts/install-session.py), from
# a private system data dir RUN/sysdata, with the extension turned on by that mode only (not in enabled-extensions)
SESSION_MODE = None
SRC = G                                           # tree that is installed: G, or RUN/src for --source <rev>
BINS_FALLBACK = ('win11-shell', 'win11-explorer', 'win11-panel', 'win11-app', 'win11-calendar', 'win11-launch',
                 'win11-index-worker', 'win11-image-worker', 'win11-default-apps', 'win11-file-portal',
                 'win11-installer', 'win11-software-worker',
                 'win11-gtk-explorer', 'win11-gtk-settings', 'win11-gtk-taskmgr', 'win11-gtk-alles',
                 'win11-gtk-disks', 'win11-gtk-defaultapps', 'win11-gtk-installer')
# D-G70: the native GTK 4 shell is the only shell (no WebKit panel, Start or desktop); it, the Calendar app and the
# native GTK 4 apps (13-11, native/APPS.md) are required, part of install.py BINS, and verified after the install even
# if an install.py still lists them apart.
NATIVE_BINS = ('win11-shell', 'win11-calendar',
               'win11-gtk-explorer', 'win11-gtk-settings', 'win11-gtk-taskmgr', 'win11-gtk-alles',
               'win11-gtk-disks', 'win11-gtk-defaultapps', 'win11-gtk-installer')
LEGACY_BINS = ('win11-start', 'win11-desktop')    # the WebKit Start and desktop hosts, gone with D-G70
# install.py's desktop entries: Explorer, Panel, TaskManager, Settings, Search, Disks, DefaultApps, Installer and
# Calendar (no Start and no Desktop entry since D-G70)
DESKTOP_ENTRIES = 9
UUID = 'win11-gnome@win11shell.local'
WL = 'w11-wl'                                     # private Wayland display name (socket RT/w11-wl)
WIDTH, HEIGHT = 1440, 900
TASKBAR = 48
# --second-monitor WxH (16-11, multi-monitor taskbars): a second virtual monitor. Mutter makes the larger one (by area)
# the primary and lays them out left to right from it, so one smaller than 1440x900 lands right of the first, which
# stays the primary (scenarios check that rather than assume it)
SECOND_MONITOR = re.compile(r'^(\d{3,4})x(\d{3,4})$')
UID = os.getuid()
REAL_HOME = Path(pwd.getpwuid(UID).pw_dir)
LIVE_RUNTIME = Path(f'/run/user/{UID}')
MARK = 'WIN11_GE0C_RUN'
MIN_START_KB = 2560 * 1024                        # contracts C8: gnome-shell runs need 2560 MB (heavy.sh --min-mb)
ABORT_KB = 1536 * 1024
INNER_TIMEOUT = 1500
EXIT_REFUSED, EXIT_MEMORY, EXIT_LOCK, EXIT_ABORTED = 2, 3, 4, 5

SETTINGS_PAGES = ['home', 'system', 'display', 'power', 'about', 'personalization', 'language', 'update']
APPS = [('taskmgr', 'Taakbeheer'), ('alles', 'Alles'), ('disks', 'Schijven')]
STUBS = ('pkexec sudo su systemctl systemd-run loginctl busctl gnome-session-quit gnome-extensions gsettings dconf '
         'apt apt-get aptitude pkcon do-release-upgrade update-manager gnome-software snap xdg-open notify-send '
         'nmcli bluetoothctl rfkill wpctl pactl pw-dump udisksctl pkill killall hyprctl hyprlock hyprsunset '
         'xdg-terminal-exec screen apt-cache apt-config check-new-release ptyxis gnome-terminal kgx unattended-upgrade '
         'firefox shutdown reboot poweroff timedatectl hostnamectl localectl gnome-control-center '
         # settings parity (notes/claude/settings-parity.md §6.6, amendment 61c): the tools the Settings services
         # spawn through host/src/tools.rs system_tool (WIN11_SYSTEM_TOOL_DIR = run/stubs) and GNOME's pkexec helpers
         'grdctl tecla bluetooth-sendto passwd localedef ubuntu-insights simple-scan orca ibus ibus-setup snap-store '
         'gnome-remote-desktop-enable-service gnome-control-center-set-debian-keyboard').split()
# plan-critique F24: each of these must be absent (set but empty counts as set), with its own refusal code.
F24_POINTERS = {'WIN11_REFERENCE_SETTINGS': 'reference-settings-set', 'WIN11_REFERENCE_INSTANCE': 'reference-instance-set',
                'WIN11_GNOME_CONFIG_DIR': 'gnome-config-dir-set', 'AT_SPI_BUS_ADDRESS': 'at-spi-bus-set',
                'XDG_SESSION_ID': 'xdg-session-id-set'}
LIVE_POINTERS = ('WIN11_UI_DIR',
                 'WIN11SHELL_CONFIG_DIR', 'WIN11SHELL_RUNTIME_DIR', 'WIN11SHELL_DATA_DIR', 'WIN11SHELL_CACHE_DIR',
                 'HYPRLAND_INSTANCE_SIGNATURE', 'GNOME_SHELL_SESSION_MODE', 'DBUS_STARTER_ADDRESS',
                 'DBUS_STARTER_BUS_TYPE', 'SSH_AUTH_SOCK', 'GNOME_KEYRING_CONTROL',
                 'DESKTOP_STARTUP_ID', 'XDG_ACTIVATION_TOKEN', 'WAYLAND_SOCKET', 'XAUTHORITY', 'SESSION_MANAGER',
                 'XDG_SEAT', 'XDG_VTNR', 'XDG_SEAT_PATH', 'XDG_SESSION_PATH', 'GNOME_SETUP_DISPLAY')
# D-G42 (GW1's layer, tests/update/run.sh root_argv): the sandbox root is an allowlist, not `--ro-bind / /`.
ROOT_LINKS = ('bin', 'sbin', 'lib', 'lib64', 'lib32', 'libx32')     # links into /usr (or plain directories)
FONTCONFIG_CACHE = Path('/var/cache/fontconfig')  # the system font cache, read-only (as GW3's update-ui/nonet.sh)
# Public system dictionary data only, never the rest of /var; libpwquality needs all three files.
CRACKLIB_FILES = tuple(Path('/var/cache/cracklib') / f'cracklib_dict.{ext}' for ext in ('pwd', 'pwi', 'hwm'))
# udev's database, read-only: mutter finds the GPU's render node through udev's "seat" tag. Its sockets
# (control, io.systemd.Udev) are in /proc/net/unix, so launch masks them like every other host socket.
UDEV_DATA = Path('/run/udev')
RUN_EMPTY = ('/run/dbus', '/run/user', '/run/systemd')            # empty directories inside the sandbox
MASKED_VAR = 'WIN11_GE0C_MASKED'                  # the host sockets launch masked (newline separated), for the guard
# Fix round M2: the sandbox's / is a read-only bind of this empty directory, on the checkout's own file system,
# instead of a tmpfs: services/storage/volumes.rs names the mount at "/" the system drive (Settings › System ›
# Storage, This PC, Task Manager), as on a real system. bwrap creates the mount points in it; prepare() never empties
# it (inside, its subdirectories are mount points). Other host mounts (/boot/efi, /media, a VM store) stay out.
ROOTFS = RUN / 'root'
# Fix round m3: the small host directories bound into the sandbox, which the guard (once) and the socket watch
# (during a run) scan for socket files; /usr and /sys are not scanned (no sockets, and far too many entries).
HOST_BINDS = (Path('/etc'), UDEV_DATA, FONTCONFIG_CACHE)
EXTRA_RO_VAR = 'WIN11_GE0C_EXTRA_RO'             # launch's extra read-only host binds (--source <rev>: the git dir)
SOCKET_WATCH_SECONDS = 2.0                        # fix round m2: the socket watch's scan interval
# D-G44: the update service's test seams for every run (host/src/services/update/paths.rs): every program only
# from a stub directory and every system file below a fixture root, both private (0700, ours, marker file).
UPDATE_MARKER = '.win11-update-fixture'
UPDATE_TOOLS = RUN / 'update-tools'
UPDATE_FIXTURE = RUN / 'update-fixture'
UPDATE_SEAMS = {'WIN11_UPDATE_TOOL_DIR': UPDATE_TOOLS, 'WIN11_UPDATE_FIXTURE_ROOT': UPDATE_FIXTURE}
# paths.rs Tool table (the names a stub must have); each stub only logs to logs/update-tool-calls.log and fails
UPDATE_TOOL_NAMES = ('loginctl xdg-terminal-exec pkexec check-new-release apt-get apt-cache apt-config apt-mark '
                     'dpkg-query snap screen nice ionice sh do-release-upgrade release-upgrades-helper '
                     'release-upgrade-attach').split()
UPDATE_SCHEMAS = G / 'tests' / 'update' / 'fixtures' / 'options' / 'schemas'   # update-notifier's (GW4 N2/N3)
# D-G74a: WinPoint's own update check (host/src/services/winpoint/seam.rs). --winpoint-feed gives every process
# WIN11_WINPOINT_TEST, a 0700 directory on the sandbox's own /tmp (so the seam's mode checks hold on any checkout
# file system) with the update fixtures' marker and seam.json {origin, running}: the hosts then ask the fake GitHub
# (tests/gnome/lib/fake_github.py, started by run inside the sandbox, loopback only) and never api.github.com. Without
# the option the variable must be unset, and the seeded settings turn the daily check off (update.winpointAutoCheck).
# D-G74: the fake offers a real release, scripts/package.py's bundle of this run's build dir and source tree
# (winpoint_bundle, RUN/winpoint-dist), and the hosts pretend to run a lower version, so they can install it.
WINPOINT_FEED = False                             # set from --winpoint-feed (set_winpoint_feed) before private_env()
WINPOINT_SEAM = Path('/tmp/w11-winpoint')
WINPOINT_VAR = 'WIN11_WINPOINT_TEST'
WINPOINT_PORT = 47811
WINPOINT_DIST = RUN / 'winpoint-dist'
# D-G74: WinPoint starts one Python program of its own, the release bundle's installer, and only after a click in
# Settings. A scenario marks each such click in the session trace by running /usr/bin/true <WINPOINT_CLICK>-<n> first
# (strace_exec_report: no WinPoint python3 before the first mark).
WINPOINT_CLICK = 'w11-winpoint-click'


# --------------------------------------------------------------------------------------------------------
# Environment and guard
# --------------------------------------------------------------------------------------------------------

def private_env() -> dict[str, str]:
    """The complete environment of every process in the sandbox (nothing is inherited)."""
    name = pwd.getpwuid(UID).pw_name
    return {
        'PATH': f"{RUN / 'stubs'}:{RUN / 'prefix' / 'bin'}:/usr/bin:/bin",
        'HOME': str(RUN / 'home'), 'USER': name, 'LOGNAME': name, 'SHELL': '/bin/sh',
        'LANG': 'en_US.UTF-8', 'TERM': 'dumb',          # the live gnome-shell runs LANG=en_US (plan D-G3)
        'XDG_CONFIG_HOME': str(RUN / 'config'), 'XDG_DATA_HOME': str(RUN / 'data'),
        'XDG_CACHE_HOME': str(RUN / 'cache'), 'XDG_RUNTIME_DIR': str(RT),
        'XDG_DATA_DIRS': (f"{RUN / 'sysdata'}:" if SESSION_MODE else '') + '/usr/local/share:/usr/share',
        'XDG_CONFIG_DIRS': '/etc/xdg',
        'XDG_CURRENT_DESKTOP': 'WinPoint:GNOME' if SESSION_MODE else 'GNOME', 'XDG_SESSION_TYPE': 'wayland',
        'TMPDIR': str(RUN / 'tmp'),
        'GSETTINGS_BACKEND': 'keyfile', 'NO_AT_BRIDGE': '1', 'GTK_A11Y': 'none',
        # D-G44 (gw4.md N1): GIO's local VFS only, so no GVfs client contacts a session bus on its own
        'GIO_USE_VFS': 'local',
        **{key: str(path) for key, path in UPDATE_SEAMS.items()},
        # settings parity: host/src/tools.rs system_tool resolves every tool to its stub here
        'WIN11_SYSTEM_TOOL_DIR': str(RUN / 'stubs'),
        **({WINPOINT_VAR: str(WINPOINT_SEAM)} if WINPOINT_FEED else {}),
        # Outer stage: a socket that never exists, so an accidental session-bus use fails.
        # dbus-run-session replaces it with RT/bus for the inner stage.
        'DBUS_SESSION_BUS_ADDRESS': f'unix:path={RT}/no-session-bus',
        'DBUS_SYSTEM_BUS_ADDRESS': f'unix:path={RT}/sbus',
        'WIN11_INSTALL_PREFIX': str(RUN / 'prefix'), 'WIN11_BUILD_DIR': str(BUILD),
        # install.py takes win11-shell from here (D-G63): the build dir, never the native workspace's own target
        'WIN11_NATIVE_BUILD_DIR': str(BUILD),
        'WIN11_DEV_COMMAND': str(RUN / 'logs' / 'command.json'), 'WIN11_DEV_REPORT': str(RUN / 'logs' / 'report.jsonl'),
        'WIN11SHELL_INDEX_AUTOSTART': '0', 'WIN11SHELL_TRASH_DIR': str(RUN / 'data' / 'Trash'),
        'WIN11SHELL_DISKS_READONLY': '1',
        'MUTTER_DEBUG_DISABLE_HW_CURSORS': '1', 'RUST_BACKTRACE': '1', 'PYTHONDONTWRITEBYTECODE': '1',
        MARK: str(RUN),
    }


def _inside(path: str | Path, root: Path) -> bool:
    try:
        return Path(os.path.realpath(path)).is_relative_to(os.path.realpath(root))
    except (OSError, ValueError):
        return False


def _same(a: str | Path, b: str | Path) -> bool:
    try:
        return os.path.realpath(a) == os.path.realpath(b) or os.path.samefile(a, b)
    except OSError:
        return False


def bus_paths(address: str, runtime: str | None) -> list[tuple[str, str, str]]:
    """(transport, key, value) for every location in a D-Bus address list (values unescaped)."""
    out = []
    for part in filter(None, address.split(';')):
        transport, _, rest = part.partition(':')
        pairs = dict(item.split('=', 1) for item in rest.split(',') if '=' in item)
        found = False
        for key in ('path', 'dir', 'tmpdir', 'abstract'):
            if key in pairs:
                out.append((transport, key, urllib.parse.unquote(pairs[key])))
                found = True
        if pairs.get('runtime') == 'yes':
            out.append((transport, 'path', f'{runtime}/bus' if runtime else '/run/user/?/bus'))
            found = True
        if not found:
            out.append((transport, 'other', rest))
    return out


def env_problems(env: dict[str, str], stage: str) -> list[tuple[str, str]]:
    """Every reason this environment could reach Robin's live session. Empty means safe."""
    p: list[tuple[str, str]] = []
    add = lambda code, msg: p.append((code, msg))
    want = private_env()
    if UID == 0:
        add('root', 'running as root')
    home = env.get('HOME')
    if not home:
        add('home-missing', 'HOME is not set')
    elif _same(home, REAL_HOME) or os.path.normpath(home) == str(REAL_HOME):
        add('home-real', f'HOME is the real home {REAL_HOME}')
    elif os.path.normpath(home) != want['HOME']:
        add('home-not-private', f'HOME={home} is not {want["HOME"]}')
    runtime = env.get('XDG_RUNTIME_DIR')
    if not runtime:
        add('runtime-missing', 'XDG_RUNTIME_DIR is not set')
    elif (os.path.normpath(runtime).startswith('/run/user/') or _same(runtime, LIVE_RUNTIME)
          or _inside(runtime, LIVE_RUNTIME)):
        add('runtime-live', f'XDG_RUNTIME_DIR={runtime} is the live runtime directory')
    elif os.path.normpath(runtime) != want['XDG_RUNTIME_DIR']:
        add('runtime-not-private', f'XDG_RUNTIME_DIR={runtime} is not {want["XDG_RUNTIME_DIR"]}')
    address = env.get('DBUS_SESSION_BUS_ADDRESS')
    if not address:
        add('session-bus-missing', 'DBUS_SESSION_BUS_ADDRESS is not set (GLib would guess one)')
    else:
        for transport, key, value in bus_paths(address, runtime):
            live = value.startswith('/run/user/') or _inside(value, LIVE_RUNTIME) or _same(value, LIVE_RUNTIME / 'bus')
            if live:
                add('session-bus-live', f'session bus {address} points into {LIVE_RUNTIME}')
            elif transport != 'unix' or key != 'path':
                add('session-bus-not-private', f'session bus transport {transport}:{key} is not a private unix path')
            elif stage == 'inner' and value != str(RT / 'bus'):
                add('session-bus-not-private', f'session bus {value} is not {RT / "bus"} (dbus-run-session)')
            elif stage != 'inner' and value != str(RT / 'no-session-bus'):
                add('session-bus-not-private', f'session bus {value} is not the placeholder {RT / "no-session-bus"}')
    system = env.get('DBUS_SYSTEM_BUS_ADDRESS')
    if system != want['DBUS_SYSTEM_BUS_ADDRESS']:
        add('system-bus-not-private', f'DBUS_SYSTEM_BUS_ADDRESS={system} is not the private {want["DBUS_SYSTEM_BUS_ADDRESS"]}')
    if 'DISPLAY' in env:
        add('display-set', f'DISPLAY={env["DISPLAY"]!r} is set')
    if 'WAYLAND_DISPLAY' in env:
        add('wayland-display-set', f'WAYLAND_DISPLAY={env["WAYLAND_DISPLAY"]!r} is set')
    if env.get('GSETTINGS_BACKEND') != 'keyfile':
        add('gsettings-not-keyfile', f'GSETTINGS_BACKEND={env.get("GSETTINGS_BACKEND")!r} is not keyfile')
    if env.get('NO_AT_BRIDGE') != '1':
        add('at-bridge', 'NO_AT_BRIDGE=1 is missing (GTK would load atk-bridge on the private bus)')
    if env.get('GTK_A11Y') != 'none':
        add('gtk-a11y', 'GTK_A11Y=none is missing')
    if env.get('GIO_USE_VFS') != 'local':
        add('gio-vfs-not-local', f'GIO_USE_VFS={env.get("GIO_USE_VFS")!r} is not local (a GVfs client would contact '
                                 'the session bus on its own; gw4.md N1, D-G44)')
    for key in UPDATE_SEAMS:
        if env.get(key) != want[key]:
            add('update-seam-not-private', f'{key}={env.get(key)!r} is not {want[key]} (D-G44: Settings would run '
                                           '/usr/bin/snap or apt-config, or read the real /var/log/apt)')
    for key in env:
        if key.startswith('WIN11_UPDATE_') and key not in UPDATE_SEAMS:
            add('update-seam-extra', f'{key} is set (this harness has no fake PackageKit yet: D-G42, GW8)')
    if env.get(WINPOINT_VAR) != want.get(WINPOINT_VAR):
        add('winpoint-seam-not-private', f'{WINPOINT_VAR}={env.get(WINPOINT_VAR)!r} is not '
                                         f'{want.get(WINPOINT_VAR)!r} (D-G74a: unset, or exactly {WINPOINT_SEAM} '
                                         'with --winpoint-feed: else the update check could ask GitHub)')
    for key in ('XDG_CONFIG_HOME', 'XDG_DATA_HOME', 'XDG_CACHE_HOME', 'TMPDIR', 'WIN11_INSTALL_PREFIX'):
        if os.path.normpath(env.get(key, '')) != want[key]:
            add('xdg-not-private', f'{key}={env.get(key)!r} is not {want[key]}')
    if env.get('XDG_DATA_DIRS') != want['XDG_DATA_DIRS'] or env.get('XDG_CONFIG_DIRS') != want['XDG_CONFIG_DIRS']:
        add('xdg-not-private', 'XDG_DATA_DIRS/XDG_CONFIG_DIRS differ from the private values')
    for entry in env.get('XDG_DATA_DIRS', '').split(':'):
        # F24: ~/.local/share holds the live extension, the live .desktop entries and the live payload.
        if entry and (_inside(entry, REAL_HOME / '.local' / 'share')
                      or os.path.normpath(entry).startswith(str(REAL_HOME / '.local' / 'share'))):
            add('xdg-data-dirs-home', f'XDG_DATA_DIRS entry {entry} is the live {REAL_HOME}/.local/share')
    build = env.get('WIN11_BUILD_DIR')
    if not build or not _inside(build, G) or _inside(build, REAL_HOME / '.local'):
        add('build-dir-not-repo', f'WIN11_BUILD_DIR={build!r} is not a build directory inside {G}')
    path = env.get('PATH', '')
    for entry in dict.fromkeys(path.split(':')):
        if entry and (_inside(entry, REAL_HOME) or entry.startswith(str(REAL_HOME))) and not _inside(entry, RUN):
            add('path-live-home', f'PATH entry {entry} is in the real home (live win11-* binaries)')
    if path != want['PATH']:
        add('path-not-private', f'PATH={path} is not {want["PATH"]}')
    for key, code in F24_POINTERS.items():
        if key in env:
            add(code, f'{key}={env[key]!r} is set (it can point the private session at live files or buses)')
    for key in LIVE_POINTERS:
        if key in env:
            add('live-pointer-var', f'{key} is set')
    # settings parity: the tool seam names this run's stubs, nothing else (a real directory would run real tools)
    if 'WIN11_SYSTEM_TOOL_DIR' in env and env['WIN11_SYSTEM_TOOL_DIR'] != str(RUN / 'stubs'):
        add('system-tool-dir', f"WIN11_SYSTEM_TOOL_DIR={env['WIN11_SYSTEM_TOOL_DIR']!r} is not {RUN / 'stubs'}")
    for key, value in env.items():
        if '/run/user/' in value and key not in ('DBUS_SESSION_BUS_ADDRESS', 'XDG_RUNTIME_DIR'):
            add('live-runtime-ref', f'{key} refers to /run/user/')
    if env.get(MARK) != str(RUN):
        add('marker-missing', f'{MARK} is not set to {RUN}')
    return p


def sandbox_problems() -> list[tuple[str, str]]:
    """Facts about this process: is it inside the bubblewrap sandbox that ``launch`` builds?"""
    p = []
    if os.access(REAL_HOME, os.W_OK) or os.access(G / 'scripts', os.W_OK):
        p.append(('not-sandboxed-home', f'{REAL_HOME} is writable (expected a read-only bind)'))
    for path in (LIVE_RUNTIME / 'bus', LIVE_RUNTIME / 'wayland-0', Path('/run/dbus/system_bus_socket'),
                 Path('/tmp/.X11-unix')):
        if os.path.lexists(path):
            p.append(('not-sandboxed-live-socket', f'{path} is visible'))
    try:
        interfaces = [line.split(':')[0].strip() for line in Path('/proc/net/dev').read_text().splitlines()[2:]]
    except OSError:
        interfaces = ['?']
    if [i for i in interfaces if i != 'lo']:
        p.append(('not-sandboxed-network', f'network interfaces {interfaces} (expected loopback only)'))
    try:
        init = Path('/proc/1/comm').read_text().strip()
    except OSError:
        init = '?'
    if init != 'bwrap':
        p.append(('not-sandboxed-pid', f'PID 1 is {init!r} (expected bwrap: own PID namespace)'))
    # D-G42: the allowlisted root. /run is a tmpfs whose dbus, user and systemd directories are empty; the real
    # home (beyond the path to this checkout), /var/lib and /var/snap are not there.
    for name in RUN_EMPTY:
        try:
            entries = os.listdir(name)
        except FileNotFoundError:
            entries = []
        except OSError as error:
            entries = [f'({error.strerror})']
        if entries:
            p.append(('not-sandboxed-run', f'{name} is not empty ({len(entries)} entries; expected a private /run)'))
    for path in (REAL_HOME / '.config', REAL_HOME / '.local', REAL_HOME / '.cache', Path('/var/lib'), Path('/var/snap')):
        if os.path.lexists(path):
            p.append(('not-sandboxed-root', f'{path} is visible (expected the allowlisted root, D-G42)'))
    # D-G42 (GW1 F1): a read-only bind does not stop connect() on a socket. launch masks every host socket the
    # binds would show (from the host's /proc/net/unix) and names them in MASKED_VAR; each must be no socket here,
    # and the checkout (outside the run dir, which prepare() empties) must hold no socket at all.
    if os.access('/', os.W_OK):
        p.append(('not-sandboxed-root', '/ is writable (expected the read-only root of the allowlisted layer, D-G42)'))
    masked = os.environ.get(MASKED_VAR)
    if masked is None:
        p.append(('not-sandboxed-no-mask-list', f'{MASKED_VAR} is not set (launch scans and masks the host sockets)'))
    else:
        for path in filter(None, masked.split('\n')):
            if _is_socket(path):
                p.append(('host-socket-visible', f'{path} is still a socket inside the sandbox (listed, not masked)'))
        for path in checkout_sockets():
            p.append(('host-socket-visible', f'{path} is a socket in the checkout (it appeared after the scan?)'))
        # fix round m3: the bound host directories too, not only the checkout (anything there is a host socket)
        for root in host_bind_roots():
            for path in socket_files([root]):
                p.append(('host-socket-visible', f'{path} is a socket in the host directory {root} (not masked)'))
    return p


def _is_socket(path: str | Path) -> bool:
    try:
        return stat.S_ISSOCK(os.stat(path).st_mode)
    except OSError:
        return False


def socket_files(roots: list[str | Path], skip: tuple = ()) -> list[str]:
    """Every socket file below `roots` (symlinks not followed; directories in `skip` left out). A masked host
    socket is /dev/null (a character device) and is not one. Fast enough for a scan every few seconds: the
    directory entries' types decide, and only an entry that is neither a directory, a file nor a link is stat()ed
    (the whole checkout without run/: about 40,000 entries in 0.04 s)."""
    skipped = {os.path.realpath(s) for s in skip}
    found, stack = [], [os.path.realpath(r) for r in roots]
    while stack:
        top = stack.pop()
        if top in skipped:
            continue
        try:
            entries = os.scandir(top)
        except OSError:
            continue
        with entries:
            for entry in entries:
                try:
                    if entry.is_dir(follow_symlinks=False):
                        stack.append(entry.path)
                    elif not entry.is_file(follow_symlinks=False) and not entry.is_symlink() \
                            and stat.S_ISSOCK(os.lstat(entry.path).st_mode):
                        found.append(entry.path)
                except OSError:
                    continue
    return sorted(found)


def checkout_sockets(include_run: bool = False) -> list[str]:
    """Every socket file in the checkout G. RUN is skipped unless include_run: it holds this run's own sockets
    and prepare() empties it."""
    return socket_files([G], skip=() if include_run else (RUN,))


def host_bind_roots() -> list[Path]:
    """The small host directories bound into the sandbox (HOST_BINDS and launch's extra read-only binds)."""
    extra = [Path(p) for p in os.environ.get(EXTRA_RO_VAR, '').split('\n') if p]
    return [p for p in (*HOST_BINDS, *extra) if p.is_dir()]


def own_socket_paths() -> set[str]:
    """The path sockets bound in this network namespace (inside the sandbox: only the sandbox's own)."""
    try:
        lines = Path('/proc/net/unix').read_text(errors='replace').splitlines()[1:]
    except OSError:
        return set()
    return {f[7] for f in (line.split(None, 7) for line in lines) if len(f) == 8 and f[7].startswith('/')}


def extension_copy_problems(path: str | Path) -> list[tuple[str, str]]:
    """F24: the extension copy GNOME Shell loads (and that --patch-extension may patch) must be the private one."""
    want = RUN / 'data' / 'gnome-shell' / 'extensions' / UUID
    if not path or not _same(path, want) and os.path.normpath(str(path)) != str(want):
        return [('extension-copy-not-private', f'extension copy {path} is not the private {want}')]
    return []


def guard(stage: str) -> list[tuple[str, str]]:
    return env_problems(dict(os.environ), stage) + sandbox_problems()


def install_bins(root: Path | None = None) -> list[str]:
    """The binaries scripts/install.py installs (its BINS tuple); the 19 known names if it cannot be parsed."""
    try:
        text = ((root or SRC) / 'scripts' / 'install.py').read_text()
        bins = re.findall(r'"(win11-[a-z-]+)"', re.search(r'^BINS = \((.*?)\)', text, re.S | re.M).group(1))
        return bins or list(BINS_FALLBACK)
    except (OSError, AttributeError):
        return list(BINS_FALLBACK)


def set_session_mode(value: str | None) -> None:
    global SESSION_MODE
    SESSION_MODE = value or None


def set_winpoint_feed(value: bool) -> None:
    global WINPOINT_FEED
    WINPOINT_FEED = bool(value)


def set_build_dir(value: str | None) -> Path:
    global BUILD
    BUILD = Path(os.path.realpath(value)) if value else DEFAULT_BUILD
    return BUILD


def build_dir_problems(path: Path) -> list[tuple[str, str]]:
    if not _inside(path, G) or _inside(path, REAL_HOME / '.local'):
        return [('build-dir-not-repo', f'--build-dir {path} is not inside {G}')]
    missing = [b for b in BINS_FALLBACK if not (path / b).is_file() or not os.access(path / b, os.X_OK)]
    if missing:
        return [('build-dir-incomplete', f'--build-dir {path} lacks {len(missing)} of {len(BINS_FALLBACK)} binaries '
                                         f'(the host binaries plus win11-shell, win11-calendar and win11-gtk-*): {", ".join(missing)}')]
    return []


def source_problems(source: str) -> list[tuple[str, str]]:
    if source == 'worktree':
        return []
    if not re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9._/-]{0,99}', source):
        return [('source-invalid', f'--source {source!r} is neither "worktree" nor a plain git revision')]
    proc = subprocess.run(['git', '-C', str(G), 'rev-parse', '--verify', '--quiet', f'{source}^{{commit}}'],
                          capture_output=True, text=True)
    if proc.returncode != 0:
        return [('source-invalid', f'--source {source!r} is not a commit in {G}')]
    return []


def guard_or_exit(stage: str) -> None:
    problems = guard(stage)
    if problems:
        for code, msg in problems:
            print(f'GUARD REFUSED [{code}] {msg}', file=sys.stderr, flush=True)
        sys.exit(EXIT_REFUSED)


# --------------------------------------------------------------------------------------------------------
# Small helpers
# --------------------------------------------------------------------------------------------------------

def log(msg: str) -> None:
    line = time.strftime('%H:%M:%S ') + msg
    print(line, flush=True)
    try:
        with open(RUN / 'logs' / 'harness.log', 'a') as f:
            f.write(line + '\n')
    except OSError:
        pass


def mem_available_kb() -> int:
    for line in Path('/proc/meminfo').read_text().splitlines():
        if line.startswith('MemAvailable:'):
            return int(line.split()[1])
    return 0


def set_subreaper() -> None:
    try:
        ctypes.CDLL(None, use_errno=True).prctl(36, 1, 0, 0, 0)   # PR_SET_CHILD_SUBREAPER
    except (OSError, AttributeError):
        pass


def proc_table() -> dict[int, dict]:
    out = {}
    for name in os.listdir('/proc'):
        if not name.isdigit():
            continue
        pid = int(name)
        try:
            stat = Path(f'/proc/{pid}/stat').read_text()
            comm = stat[stat.index('(') + 1:stat.rindex(')')]
            fields = stat[stat.rindex(')') + 2:].split()
            out[pid] = {'pid': pid, 'ppid': int(fields[1]), 'comm': comm}
        except (OSError, ValueError, IndexError):
            continue
    return out


def descendants(root: int, table: dict[int, dict] | None = None) -> list[int]:
    table = table or proc_table()
    children: dict[int, list[int]] = {}
    for pid, row in table.items():
        children.setdefault(row['ppid'], []).append(pid)
    out, todo = [], [root]
    while todo:
        for child in children.get(todo.pop(), []):
            out.append(child)
            todo.append(child)
    return out


def stop_pids(pids: list[int], label: str, grace: float = 8.0) -> list[str]:
    """SIGTERM, then SIGKILL. Only ever called with descendants of this process (own PID namespace)."""
    table = proc_table()
    pids = [p for p in pids if p in table and p not in (1, os.getpid())]
    names = [f'{p}:{table[p]["comm"]}' for p in pids]
    for pid in pids:
        try:
            os.kill(pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
    end = time.monotonic() + grace
    while time.monotonic() < end and any(Path(f'/proc/{p}').exists() and _alive(p) for p in pids):
        time.sleep(0.1)
    for pid in pids:
        if Path(f'/proc/{pid}').exists() and _alive(pid):
            try:
                os.kill(pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
    if names:
        log(f'{label}: signalled own descendants {", ".join(names)}')
    return names


def _alive(pid: int) -> bool:
    try:
        state = Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()[0]
        return state != 'Z'
    except (OSError, IndexError):
        return False


def reap() -> None:
    while True:
        try:
            pid, _ = os.waitpid(-1, os.WNOHANG)
        except ChildProcessError:
            return
        if pid == 0:
            return


class Fatal(Exception):
    """Stops a wait at once (for example: gnome-shell exited)."""


def until(check, label: str, timeout: float = 45, interval: float = 0.1):
    end = time.monotonic() + timeout
    last = None
    while time.monotonic() < end:
        try:
            value = check()
            if value:
                return value
        except Fatal:
            raise
        except Exception as error:   # noqa: BLE001 - keep polling, report the last error
            last = error
        time.sleep(interval)
    raise TimeoutError(f'{label} (after {timeout:.0f} s; last error: {last})')


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def settings_titles() -> dict[str, dict[str, str]]:
    """The Settings page titles in Dutch and English, by page id: the native app's registry
    (native/apps/settings/src/registry.rs, `page!(id, parent, icon, nl, en, ...)`), with Start's copy of the page
    list (native/shell/src/start/settings_pages.json) for an id the registry does not have."""
    titles: dict[str, dict[str, str]] = {}
    try:
        pages = json.loads((SRC / 'native/shell/src/start/settings_pages.json').read_text())
        titles.update({p['id']: {'nl': p['title']['nl'], 'en': p['title']['en']} for p in pages})
    except (OSError, ValueError, KeyError, TypeError):
        pass
    try:
        text = (SRC / 'native/apps/settings/src/registry.rs').read_text()
    except OSError:
        return titles
    unescape = lambda value: re.sub(r'\\u\{([0-9a-fA-F]+)\}', lambda m: chr(int(m.group(1), 16)), value)  # noqa: E731
    pattern = r'page!\("([a-z-]+)",\s*(?:None|Some\("[a-z-]+"\)),\s*"[^"]*",\s*"((?:[^"\\]|\\.)*)",\s*"((?:[^"\\]|\\.)*)"'
    titles.update({m.group(1): {'nl': unescape(m.group(2)), 'en': unescape(m.group(3))} for m in re.finditer(pattern, text)})
    return titles


def tree_nodes(tops) -> list[dict]:
    """Every node of the widget trees in a dev state answer (a list of toplevels with a "tree"; native/kit/src/dev.rs
    and native/shell/src/dev.rs list only visible children), depth first."""
    out, todo = [], [top.get('tree') for top in reversed(tops or []) if isinstance(top, dict)]
    while todo:
        node = todo.pop()
        if isinstance(node, dict):
            out.append(node)
            todo.extend(reversed(node.get('children') or []))
    return out


# The tooltip of a native app's close caption button (native/kit/src/frame.rs, relabelled on every switch), which the
# kit's dev state gives as the button's label: the language the app shows (an HTML app's <html lang> before).
CLOSE_TOOLTIP = {'Sluiten': 'nl', 'Close': 'en'}


def app_lang(tops, app: str) -> str | None:
    """'nl' or 'en' as a native app shows it (its `<app>-close` caption button), None without a window."""
    return next((CLOSE_TOOLTIP[n['label']] for n in tree_nodes(tops)
                 if n.get('name') == f'{app}-close' and n.get('label') in CLOSE_TOOLTIP), None)


def dutch_words() -> set[str]:
    path = G / 'tests/i18n/dutch-words.txt'
    try:
        return {w.strip() for w in path.read_text().splitlines() if w.strip() and not w.startswith('#')}
    except OSError:
        return set()


# --------------------------------------------------------------------------------------------------------
# launch: build the sandbox (the only command that may start from a live shell)
# --------------------------------------------------------------------------------------------------------

def readable_dictionary_files(paths=CRACKLIB_FILES):
    """Only fixed, readable regular dictionary files may be exposed read-only; no symlinks or sockets."""
    out = []
    for path in paths:
        try:
            if stat.S_ISREG(path.lstat().st_mode) and os.access(path, os.R_OK):
                out.append(path)
        except OSError:
            pass
    return out


def root_argv(extra_ro: list[Path] = ()) -> tuple[list[str], list[str]]:
    """D-G42: GW1's allowlisted root (tests/update/run.sh root_argv) instead of `--ro-bind / /`, and the
    directories it makes visible (for the socket scan). Read-only: /usr with the /bin, /sbin, /lib… links, /etc,
    /sys, the font cache, udev's database and the checkout G (plus `extra_ro`). Private: /dev (GPU nodes only),
    /proc, /tmp, /var (/var/run -> /run, /var/tmp) and /run with empty dbus, user and systemd directories. The
    real home beyond the path to G, /var/lib, /var/snap, /srv, /opt, /mnt and /media are not there. / itself is
    ROOTFS (fix round M2: a directory on G's file system, so storage sees a system drive at /), read-only at the
    end (bwrap_argv)."""
    argv, visible = ['--bind', str(ROOTFS), '/', '--ro-bind', '/usr', '/usr'], ['/usr']
    for name in ROOT_LINKS:
        path = Path('/') / name
        if path.is_symlink():
            argv += ['--symlink', os.readlink(path), str(path)]
        elif path.is_dir():
            argv += ['--ro-bind', str(path), str(path)]
            visible.append(str(path))
    argv += ['--ro-bind', '/etc', '/etc', '--ro-bind', '/sys', '/sys', '--dev', '/dev']
    visible += ['/etc', '/sys']
    for node in sorted(glob.glob('/dev/nvidia*')) + ['/dev/dri']:
        if os.path.exists(node):
            argv += ['--dev-bind', node, node]
    argv += ['--proc', '/proc', '--tmpfs', '/tmp',
             '--tmpfs', '/run', *[arg for name in RUN_EMPTY for arg in ('--dir', name)],
             '--tmpfs', '/var', '--symlink', '../run', '/var/run', '--dir', '/var/tmp']
    for path in (FONTCONFIG_CACHE, UDEV_DATA):
        if path.is_dir():
            argv += ['--ro-bind', str(path), str(path)]
            visible.append(str(path))
    for path in readable_dictionary_files():
        argv += ['--ro-bind', str(path), str(path)]
        visible.append(str(path))
    argv += ['--ro-bind', str(G), str(G)]
    visible.append(str(G))
    for path in extra_ro:
        argv += ['--ro-bind', str(path), str(path)]
        visible.append(str(path))
    return argv, visible


def host_sockets(visible: list[str]) -> list[str]:
    """D-G42 (GW1 review F1): every bound path socket of the host network namespace that the binds would still
    show inside, by its real path. /proc/net/unix is read here, outside: inside, --unshare-net lists only the
    sandbox's own sockets. Abstract names are isolated by --unshare-net."""
    found = []
    try:
        lines = Path('/proc/net/unix').read_text(errors='replace').splitlines()[1:]
    except OSError:
        lines = []
    for line in lines:
        fields = line.split(None, 7)
        if len(fields) < 8 or not fields[7].startswith('/'):
            continue
        try:
            if not stat.S_ISSOCK(os.stat(fields[7]).st_mode):
                continue
            real = os.path.realpath(fields[7])
        except OSError:
            continue
        if any(real == root or real.startswith(root.rstrip('/') + '/') for root in visible) and real not in found:
            found.append(real)
    return sorted(found)


def bwrap_argv(inner: list[str], masks: list[str] = (), writable: list[Path] = (),
               extra_ro: list[Path] = ()) -> list[str]:
    """The sandbox: the allowlisted root (root_argv), `writable` (the run dir and the output directory) bound
    read-write over the read-only checkout, every host socket in `masks` covered by /dev/null, then `/`
    remounted read-only (so the directories bwrap creates for the binds, the real home's path among them, are
    read-only too)."""
    argv = ['/usr/bin/bwrap'] + root_argv(extra_ro)[0]
    for path in writable:
        argv += ['--bind', str(path), str(path)]
    for path in masks:
        argv += ['--ro-bind', '/dev/null', path]
    argv += ['--remount-ro', '/', '--unshare-pid', '--unshare-net', '--unshare-ipc', '--die-with-parent',
             '--new-session', '--chdir', str(G), '--', '/usr/bin/python3', '-B', '-I', str(HERE / 'harness.py')] + inner
    return argv


def _ancestors(limit: int = 8) -> list[tuple[int, list[str], str]]:
    """(pid, argv, cwd) of our ancestors, nearest first."""
    out, pid = [], os.getppid()
    for _ in range(limit):
        if pid <= 1:
            break
        try:
            argv = [a.decode(errors='replace') for a in Path(f'/proc/{pid}/cmdline').read_bytes().split(b'\0') if a]
            cwd = os.readlink(f'/proc/{pid}/cwd')
            parent = int(Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()[1])
        except (OSError, ValueError, IndexError):
            break
        out.append((pid, argv, cwd))
        pid = parent
    return out


def _lock_is_held() -> bool:
    """True when some open file description holds LOCK (a non-blocking attempt fails). Never keeps the lock."""
    try:
        fd = os.open(LOCK, os.O_RDONLY | os.O_CLOEXEC)
    except OSError:
        return False
    try:
        fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        fcntl.flock(fd, fcntl.LOCK_UN)
        return False
    except BlockingIOError:
        return True
    finally:
        os.close(fd)


def _has_open(pid: int, target: Path) -> bool:
    try:
        return any(_same(os.readlink(f'/proc/{pid}/fd/{fd}'), target) for fd in os.listdir(f'/proc/{pid}/fd'))
    except OSError:
        return False


def lock_ancestor() -> tuple[str, str] | None:
    """(kind, description) when an ancestor holds the heavy lock: ``tools/heavy.sh`` (contracts C8; bash keeps
    the locked file open on fd 9 and closes it for the command) or ``flock(1)`` on the same file."""
    heavy = G / 'tools' / 'heavy.sh'
    for pid, argv, cwd in _ancestors():
        resolve = lambda a: a if a.startswith('/') else os.path.join(cwd, a)
        kind = None
        if argv and os.path.basename(argv[0]) == 'flock' and any(_same(resolve(a), LOCK) for a in argv[1:]):
            kind = 'flock'
        elif any(_same(resolve(a), heavy) for a in argv[:3]) and _has_open(pid, LOCK):
            kind = 'heavy.sh'
        if kind:
            # The ancestor must really hold it now (heavy.sh only runs the command after flock succeeded).
            return (kind, f'{pid}:{" ".join(argv[:6])}') if _lock_is_held() else None
    return None


def out_problem(out: Path) -> bool:
    """True when `out` may not be the writable output directory: outside HERE, inside RUN, or (fix round n3) HERE
    itself or a directory that contains RUN, which would make all of tests/gnome writable inside the sandbox."""
    return not _inside(out, HERE) or _inside(out, RUN) or _same(out, HERE) or _inside(RUN, out)


def launch_problems(opts) -> list[tuple[str, str]]:
    """Checks of the run options before any lock or sandbox: build dir, source, scenario and output paths."""
    p = []
    set_session_mode(getattr(opts, 'session_mode', None))
    set_winpoint_feed(getattr(opts, 'winpoint_feed', False))
    if getattr(opts, 'native_snap_fixture', False) and getattr(opts, 'session_mode', None) != 'winpoint':
        p.append(('native-snap-fixture-mode', '--native-snap-fixture requires --session-mode winpoint'))
    if not opts.guard_only:
        p += build_dir_problems(set_build_dir(opts.build_dir))
    p += source_problems(opts.source)
    if opts.scenario:
        path = Path(os.path.realpath(opts.scenario))
        if not path.is_file() or path.suffix != '.py' or not _inside(path, G):
            p.append(('scenario-invalid', f'--scenario {opts.scenario} is not a .py file inside {G}'))
    if not opts.guard_only:
        p += run_modes_problems()
    if getattr(opts, 'second_monitor', None) and not second_monitor_ok(opts.second_monitor):
        p.append(('second-monitor-invalid', f'--second-monitor {opts.second_monitor} is not WxH between 640x480 '
                                             'and 3840x2160'))
    if opts.out:
        out = Path(os.path.realpath(opts.out))
        if out_problem(out):
            p.append(('out-invalid', f'--out {opts.out} must be inside {HERE} (the only writable tree in the sandbox) '
                                     f'and outside {RUN} (wiped per run), and neither be {HERE} nor contain {RUN}'))
    if opts.strace:
        trace = Path(os.path.realpath(opts.strace))
        out = Path(os.path.realpath(opts.out)) if opts.out else scenario_out(opts)
        if not _inside(trace, out):
            p.append(('out-invalid', f'--strace {opts.strace} must be inside the output directory {out} '
                                     '(the only writable directory besides run/)'))
    return p


def run_modes_problems() -> list[tuple[str, str]]:
    """run/ must keep Unix modes: the hosts refuse a settings lock others could read (host/src/settings.rs lock(),
    mode & 0o077), and on NTFS through fuseblk or on exFAT every file reads 0755, so the run had no backend at all
    (16-11, a worktree on the 22 TB disk). A probe file made 0600 next to run/ shows it."""
    probe = HERE / f'.mode-probe-{os.getpid()}'
    try:
        fd = os.open(probe, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
        os.close(fd)
        os.chmod(probe, 0o600)
        mode = stat.S_IMODE(os.stat(probe).st_mode)
    except OSError as error:
        return [('run-dir-no-modes', f'cannot probe the file modes of {HERE}: {error}')]
    finally:
        try:
            os.unlink(probe)
        except OSError:
            pass
    if mode & 0o077:
        return [('run-dir-no-modes', f'{RUN} is on a file system without Unix permissions (a file made 0600 reads '
                                     f'{mode:o}): the hosts refuse their settings lock there and the run would have '
                                     'no backend. Run the harness from a checkout on ext4 or tmpfs.')]
    return []


def second_monitor_ok(value: str) -> bool:
    """--second-monitor: WxH with 640 <= W <= 3840 and 480 <= H <= 2160."""
    match = SECOND_MONITOR.match(value or '')
    return bool(match) and 640 <= int(match[1]) <= 3840 and 480 <= int(match[2]) <= 2160


def git_common_dir() -> Path | None:
    """The repository's git directory (outside G when G is a git worktree), for --source <rev>'s git archive."""
    proc = subprocess.run(['git', '-C', str(G), 'rev-parse', '--path-format=absolute', '--git-common-dir'],
                          capture_output=True, text=True)
    path = proc.stdout.strip()
    return Path(os.path.realpath(path)) if proc.returncode == 0 and path else None


CONNECT = re.compile(r'^(\d+)\s+connect\(\d+(?:<[^>]*>)?, \{sa_family=(\w+)(.*?)\}, \d+'
                     r'(?:\) = (-?\d+)(?: (\w+))?|( <unfinished \.\.\.>))')
RESUMED = re.compile(r'^(\d+)\s+<\.\.\. connect resumed>\) = (-?\d+)(?: (\w+))?')
SUN_PATH = re.compile(r'sun_path=(@?)"((?:[^"\\]|\\.)*)"')
INET_HOST = re.compile(r'inet_addr\("([^"]*)"\)|inet_pton\(AF_INET6, "([^"]*)"')


def strace_connect_report(traces: list[Path]) -> dict:
    """D-G42: every connect() inside the sandbox (strace -f over the install and the session). A
    connect that succeeded, or whose outcome strace did not see (the process ended inside it, EINPROGRESS), may
    only reach a socket of the sandbox itself: one under RUN or in the sandbox's own tmpfs mounts (/tmp with
    XDG_RUNTIME_DIR, /run, /var), but not in the host directories bound into them (/run/udev, the font cache), an
    abstract name (the sandbox's own network namespace) or loopback. Anything else is a violation."""
    rows: dict[tuple, int] = {}
    violations, pending, total = [], {}, 0
    private = (str(RUN) + '/', '/tmp/', '/run/', '/var/')
    host_binds = tuple(str(path) + '/' for path in (UDEV_DATA, FONTCONFIG_CACHE))

    # the links bwrap makes at the top of the sandbox (root_argv): /var/run -> /run, /bin -> usr/bin, ...
    links = {('var', 'run'): ['run']}
    for name in ROOT_LINKS:
        if (Path('/') / name).is_symlink():
            links[(name,)] = [c for c in os.readlink(Path('/') / name).split('/') if c and c != '.']

    def sandbox_path(raw: str) -> str | None:
        """Fix round m4: the socket path as the kernel resolves it inside the sandbox, left to right: the
        sandbox's own top-level links (/var/run, /bin, ...) followed where they occur, then '.', '..' and repeated
        slashes folded (so /var/run/../home is /home, not /var/home). Links further down (in the private tmpfs or
        run/) are not known here. None for a relative path (its directory is unknown)."""
        if not raw.startswith('/'):
            return None
        parts: list[str] = []
        for comp in raw.split('/'):
            if comp in ('', '.'):
                continue
            if comp == '..':
                if parts:
                    parts.pop()
                continue
            parts.append(comp)
            target = links.get(tuple(parts))
            if target is not None:
                parts = list(target)
        return '/' + '/'.join(parts)

    def note(family: str, address: str, rc: str, errno: str | None) -> None:
        unix = SUN_PATH.search(address)
        host = INET_HOST.search(address)
        abstract = bool(unix and unix.group(1))
        raw = unix.group(2) if unix else ''
        target = (('@' if abstract else '') + raw) if unix else (host.group(1) or host.group(2)) if host else '?'
        ok = rc == '0'
        shown = re.sub(r'/(dbus|ibus|pulse|wayland)-[\w.-]+$', r'/\1-*', target)
        key = (family, shown, 'ok' if ok else (errno or rc))
        rows[key] = rows.get(key, 0) + 1
        maybe = ok or errno in ('EINPROGRESS', 'UNFINISHED')       # UNFINISHED: the process ended inside it
        path = sandbox_path(raw)
        inside = path is not None and path.startswith(private) and not path.startswith(host_binds)
        if family == 'AF_UNIX' and not abstract and not inside and maybe:
            violations.append(f'connect to {raw}' + (f' (= {path})' if path and path != raw else '')
                              + (' (a relative path)' if path is None else '') + f': {key[2]}')
        if family in ('AF_INET', 'AF_INET6') and target not in ('127.0.0.1', '::1') and maybe:
            violations.append(f'connect to {family} {target}: {key[2]}')

    lines = [line for trace in traces if trace.is_file() for line in trace.read_text(errors='replace').splitlines()]
    for line in lines:
        m = CONNECT.match(line)
        if m:
            total += 1
            pid, family, address, rc, errno, unfinished = m.groups()
            if unfinished:
                pending[pid] = (family, address)
            else:
                note(family, address, rc, errno)
            continue
        m = RESUMED.match(line)
        if m and m.group(1) in pending:
            note(*pending.pop(m.group(1)), m.group(2), m.group(3))
    for family, address in pending.values():
        note(family, address, '?', 'UNFINISHED')
    missing = [str(trace) for trace in traces if not trace.is_file()]
    if missing:
        violations.append(f'no strace log: {", ".join(missing)}')
    return {'files': [str(trace) for trace in traces], 'connects': total, 'ok': not violations, 'violations': violations,
            'byTarget': [{'family': f, 'target': t, 'result': r, 'count': n} for (f, t, r), n in sorted(rows.items())]}


# WU-FIX (D-G51, GW5 review m6/M1): no real updater or privilege tool may run in a session, however it is called: the
# stubs (run/stubs on PATH, run/update-tools for the update service's tool table) are the only allowed copies.
EXECVE = re.compile(r'^(\d+)\s+(execve|execveat)\((?:[^,]*, )?"((?:[^"\\]|\\.)*)"(.*?)'
                    r'(?:\) = (-?\d+)(?: (\w+)(?: \(.*\))?)?|( <unfinished \.\.\.>))\s*$')
# strace pads a short line's result to its column: ')                 = 0'
EXEC_RESUMED = re.compile(r'^(\d+)\s+<\.\.\. (?:execve|execveat) resumed>.*\)\s+=\s+(-?\d+)(?: (\w+))?')
FORBIDDEN_EXEC = ('pkexec', 'check-new-release', 'apt-get', 'apt-cache', 'do-release-upgrade', 'loginctl', 'apt', 'pkcon')
# D-G74: the process tree of a session trace (strace -f with clone, clone3, fork and vfork traced): parent -> child
CLONE = re.compile(r'^(\d+)\s+(?:clone3?|v?fork)\(.*\)\s+=\s+(\d+)\s*$')
CLONE_RESUMED = re.compile(r'^(\d+)\s+<\.\.\. (?:clone3?|v?fork) resumed>.*\)\s+=\s+(\d+)\s*$')
ARGV_ITEM = re.compile(r'\s*"((?:[^"\\]|\\.)*)"(\.\.\.)?\s*([,\]])')
PYTHON_EXEC = re.compile(r'python3(?:\.\d+)?')
VERSION_DIR = re.compile(r'(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)')


def winpoint_python_report(trace: Path) -> dict:
    """D-G74's harness exec rule, on one session trace. WinPoint starts exactly one Python program of its own: the
    release bundle's installer, /usr/bin/python3 -E -s -B <RUN>/cache/win11-gnome/winpoint/work/<V>/bundle/scripts/
    install.py <bundle> --lang nl|en --no-session and then --check or --status <work>/install.status --log
    <work>/install.log (host/src/services/winpoint/job.rs, the frozen contract of scripts/install.py), with exactly
    its 11 or 12 environment variables. Every python3 that a WinPoint process (one that executed a win11-* program) or
    any process below it executes must be exactly that, and each must belong to a click: the scenario marks every click
    on Download and install or Retry (/usr/bin/true WINPOINT_CLICK-<n>, run right before it), and each mark allows at
    most one --check and then at most one install, in that order, until the next mark. A python3 before the first
    mark, a second --check or install after one mark (a restarted owner or a check that ran it on its own), or an
    install before its --check is a violation. The harness's own Python and GNOME's (an ibus engine) are no WinPoint
    processes. Returns {execs, clicks, perClick: {n: [modes]}, violations}."""
    lines = trace.read_text(errors='replace').splitlines() if trace.is_file() else []
    parent: dict[str, str] = {}
    for line in lines:
        m = CLONE.match(line) or CLONE_RESUMED.match(line)
        if m:
            parent[m.group(2)] = m.group(1)
    pending: dict[str, tuple[int, str, str]] = {}
    execs: list[tuple[int, str, str, str]] = []                 # (line, pid, path, rest) of every exec that ran
    for index, line in enumerate(lines):
        m = EXECVE.match(line)
        if m:
            pid, _call, raw, rest, rc, _errno, unfinished = m.groups()
            if unfinished:
                pending[pid] = (index, raw, rest)
            elif rc == '0':
                execs.append((index, pid, raw, rest))
            continue
        m = EXEC_RESUMED.match(line)
        if m and m.group(1) in pending:
            start, raw, rest = pending.pop(m.group(1))
            if m.group(2) == '0':
                execs.append((start, m.group(1), raw, rest))
    execs.sort()
    winpoint = {pid for _i, pid, raw, _r in execs if raw.rsplit('/', 1)[-1].startswith('win11-')}

    def ours(pid: str) -> bool:
        seen = set()
        while pid and pid not in seen:
            if pid in winpoint:
                return True
            seen.add(pid)
            pid = parent.get(pid, '')
        return False

    found, violations, clicks = [], [], []
    per_click: dict[int, list[str]] = {}
    window: int | None = None                                   # the click mark the next installer run belongs to
    for index, pid, raw, rest in execs:
        if raw == '/usr/bin/true' and f'"{WINPOINT_CLICK}-' in rest:
            mark = re.search(rf'"{WINPOINT_CLICK}-(\d+)"', rest)
            window = int(mark.group(1)) if mark else len(clicks) + 1
            clicks.append(index)
            per_click.setdefault(window, [])
            continue
        if not PYTHON_EXEC.fullmatch(raw.rsplit('/', 1)[-1]) or not ours(pid):
            continue
        argv, cut, at = [], False, rest.find('[') + 1
        while at > 0:
            item = ARGV_ITEM.match(rest, at)
            if not item:
                cut = True                           # an array strace shortened ("...") or could not read
                break
            argv.append(item.group(1).encode('latin-1', 'backslashreplace').decode('unicode_escape', 'replace'))
            cut = cut or bool(item.group(2))
            at = item.end()
            if item.group(3) == ']':
                break
        env = re.search(r'/\* (\d+) vars \*/', rest[at:])
        row = {'line': index + 1, 'pid': pid, 'path': raw, 'argv': argv, 'env': int(env.group(1)) if env else None}
        found.append(row)
        work = f'{RUN}/cache/win11-gnome/winpoint/work/'
        version = argv[4][len(work):].split('/', 1)[0] if len(argv) > 4 and argv[4].startswith(work) else ''
        bundle = f'{work}{version}/bundle'
        head = ['/usr/bin/python3', '-E', '-s', '-B', f'{bundle}/scripts/install.py', bundle, '--lang']
        tail = argv[8:]
        check = tail == ['--no-session', '--check']
        install = tail == ['--no-session', '--status', f'{work}{version}/install.status', '--log',
                           f'{work}{version}/install.log']
        frozen = raw == '/usr/bin/python3' and not cut and bool(VERSION_DIR.fullmatch(version)) \
            and argv[:7] == head and len(argv) > 7 and argv[7] in ('nl', 'en') and (check or install)
        row['mode'] = ('check' if check else 'install') if frozen else None
        row['click'] = window
        problems = []
        if not frozen:
            problems.append('not the release bundle\'s installer with its frozen argv')
        if row['env'] not in (11, 12):
            problems.append(f'{row["env"]} environment variables (the frozen contract has 11 or 12)')
        if window is None:
            problems.append('before the first click on Download and install')
        elif row['mode']:
            done = per_click[window]
            allowed = 'check' if not done else ('install' if done == ['check'] else None)
            if row['mode'] != allowed:
                problems.append(f'not tied to a click on Download and install (click {window} already ran '
                                f'{done or "nothing"}; a {row["mode"]} needs a click of its own)')
            done.append(row['mode'])
        for problem in problems:
            violations.append(f'line {index + 1}: python3 of WinPoint (pid {pid}) {problem}: {argv[:12]}')
    return {'execs': found, 'clicks': len(clicks), 'perClick': per_click, 'violations': violations}


def winpoint_expected_problems(per_click: dict, expected) -> list[str]:
    """--winpoint-feed with the winpoint-update scenario: the installer runs of each click mark must be exactly what
    the scenario says it caused (its record ``winpoint installer runs``, {n: [modes]}), so a run in which the installer
    never ran, or ran outside the trace (a transient unit), fails instead of passing."""
    if not isinstance(expected, dict) or not expected:
        return ['the scenario recorded no expected installer runs (records["winpoint installer runs"])']
    want = {int(k): list(v) for k, v in expected.items()}
    got = {int(k): list(v) for k, v in per_click.items()}
    if got != want:
        return [f'installer runs per click {got}, the scenario caused {want} (one --check and one install for the '
                'installing click; none for a click whose job ends before --check)']
    return []


def strace_exec_report(traces: list[Path], winpoint_expected=None) -> dict:
    """WU-FIX (D-G51): every execve()/execveat() in the strace logs of a run (the session traces execve since WU-FIX,
    the install's trace=%file always did). A program of FORBIDDEN_EXEC that was executed (the call returned 0, or the
    process ended inside it) from anywhere but this run's own stub directories (under RUN) is a violation: the real
    /usr/bin/pkexec, check-new-release, apt-get, apt-cache, do-release-upgrade or loginctl ran. A failed attempt
    (ENOENT while PATH is searched) ran nothing. Paths are folded as the kernel resolves them in the sandbox ('..',
    '.', '//' and the top-level /bin, /sbin, /lib… links); a relative path counts as outside RUN."""
    links = {}
    for name in ROOT_LINKS:
        if (Path('/') / name).is_symlink():
            links[(name,)] = [c for c in os.readlink(Path('/') / name).split('/') if c and c != '.']
    links[('var', 'run')] = ['run']

    def folded(raw: str) -> str | None:
        if not raw.startswith('/'):
            return None
        parts: list[str] = []
        for comp in raw.split('/'):
            if comp in ('', '.'):
                continue
            if comp == '..':
                if parts:
                    parts.pop()
                continue
            parts.append(comp)
            target = links.get(tuple(parts))
            if target is not None:
                parts = list(target)
        return '/' + '/'.join(parts)

    rows: dict[tuple, int] = {}
    violations, pending, total = [], {}, 0
    runs = {str(RUN), os.path.realpath(RUN)}

    def note(raw: str, rc: str, errno: str | None) -> None:
        name = raw.rsplit('/', 1)[-1]
        if name not in FORBIDDEN_EXEC:
            return
        path = folded(raw)
        stub = path is not None and any(path.startswith(run + '/') for run in runs)
        ran = rc == '0' or errno == 'UNFINISHED'
        key = (raw, 'stub' if stub else 'real', 'ran' if ran else (errno or rc))
        rows[key] = rows.get(key, 0) + 1
        if ran and not stub:
            violations.append(f'executed {raw}' + (f' (= {path})' if path and path != raw else '')
                              + (' (a relative path)' if path is None else ''))

    lines = [line for trace in traces if trace.is_file() for line in trace.read_text(errors='replace').splitlines()]
    for line in lines:
        m = EXECVE.match(line)
        if m:
            total += 1
            pid, _call, raw, _rest, rc, errno, unfinished = m.groups()
            raw = raw.encode('latin-1', 'backslashreplace').decode('unicode_escape', 'replace')
            if unfinished:
                pending[pid] = raw
            else:
                note(raw, rc, errno)
            continue
        m = EXEC_RESUMED.match(line)
        if m and m.group(1) in pending:
            note(pending.pop(m.group(1)), m.group(2), m.group(3))
    for raw in pending.values():
        note(raw, '?', 'UNFINISHED')
    missing = [str(trace) for trace in traces if not trace.is_file()]
    if missing:
        violations.append(f'no strace log: {", ".join(missing)}')
    # D-G74: the one Python program, per trace (each has its own process tree; the install's has no WinPoint process)
    python = [winpoint_python_report(trace) for trace in traces if trace.is_file()]
    violations += [v for report in python for v in report['violations']]
    per_click: dict[int, list[str]] = {}
    for report in python:
        for click, modes in report['perClick'].items():
            per_click.setdefault(click, []).extend(modes)
    if winpoint_expected is not None:
        violations += winpoint_expected_problems(per_click, winpoint_expected)
    return {'files': [str(trace) for trace in traces], 'execs': total, 'ok': not violations, 'violations': violations,
            'forbidden': list(FORBIDDEN_EXEC),
            'byProgram': [{'path': pth, 'where': w, 'result': res, 'count': n} for (pth, w, res), n in sorted(rows.items())],
            'winpointPython': {'execs': [row for report in python for row in report['execs']],
                               'clicks': sum(report['clicks'] for report in python),
                               'perClick': {str(k): v for k, v in sorted(per_click.items())},
                               **({'expected': winpoint_expected} if winpoint_expected is not None else {})}}
# WU-FIX end


def cmd_launch(args, rest: list[str]) -> int:
    if not rest or rest[0] not in ('run', 'probe'):
        print('launch: expected "run ..." or "probe"', file=sys.stderr)
        return EXIT_REFUSED
    # Fix round m3: the self-test's mask and environment changes never reach a session: an unmasked socket only
    # with probe (which starts nothing), a mutated environment only with probe or run --guard-only.
    if (args.selftest_unmasked or args.selftest_listed_unmasked) and rest[0] != 'probe':
        print('GUARD REFUSED [selftest-option-misuse] --selftest-unmasked/--selftest-listed-unmasked only with '
              'launch probe', file=sys.stderr, flush=True)
        return EXIT_REFUSED
    if args.selftest_mutate and not (rest[0] == 'probe' or '--guard-only' in rest):
        print('GUARD REFUSED [selftest-option-misuse] --selftest-mutate only with launch probe or run --guard-only',
              file=sys.stderr, flush=True)
        return EXIT_REFUSED
    opts = None
    if rest[0] == 'run':
        opts, _ = run_parser().parse_known_args(rest[1:])
        problems = launch_problems(opts)
        if problems:
            for code, msg in problems:
                print(f'GUARD REFUSED [{code}] {msg}', file=sys.stderr, flush=True)
            return EXIT_REFUSED
        # Absolute paths for the sandbox (its cwd is G); argparse keeps the last occurrence.
        rest = rest + ['--build-dir', str(BUILD)]
        if opts.scenario:
            rest += ['--scenario', os.path.realpath(opts.scenario)]
        if opts.out:
            rest += ['--out', os.path.realpath(opts.out)]
        if opts.strace:
            rest += ['--strace', os.path.realpath(opts.strace)]
    if rest[0] == 'run' and '--guard-only' not in rest:
        held = lock_ancestor()
        if held:
            print(f'heavy lock held by ancestor {held[0]} ({held[1]})', flush=True)
            rest = rest + ['--lock', f'parent:{held[0]}']
            if args.lock_check_only:
                print(f'LOCK DECISION ancestor:{held[0]}')
                return 0
        elif args.own_lock:
            if args.lock_check_only:
                fd = acquire_lock(timeout=0)
                print(f'LOCK DECISION {"own" if fd is not None else "own-busy"}')
                if fd is not None:
                    os.close(fd)
                return 0 if fd is not None else EXIT_LOCK
            if LOCK != G / '.heavy.lock':
                print('refused: --own-lock only takes the default .heavy.lock', file=sys.stderr)
                return EXIT_LOCK
            fd = acquire_lock()
            if fd is None:
                print('refused: could not take the heavy lock within 3600 s', file=sys.stderr)
                return EXIT_LOCK
            os.set_inheritable(fd, True)          # held by `run` inside the sandbox until it exits
            rest = rest + ['--lock', f'fd:{fd}']
        else:
            print('refused: no heavy lock (contracts C8). Run it as\n  tools/heavy.sh --min-mb 2560 --tag <step> -- '
                  'python3 -B tests/gnome/harness.py launch run ...\n(or add "launch --own-lock" to let the harness '
                  'take .heavy.lock itself)', file=sys.stderr)
            if args.lock_check_only:
                print('LOCK DECISION refused')
            return EXIT_LOCK
    elif args.lock_check_only:
        print('LOCK DECISION not-needed')
        return 0
    env = private_env()
    # D-G42: the checkout is read-only inside; writable are only this run's RUN and its output directory.
    writable, extra_ro = [RUN], []
    if opts is not None and not opts.guard_only:
        out = scenario_out(opts)
        if not out_problem(out):                               # else `run` refuses it (out-invalid)
            writable.append(out)
            if opts.switch and not opts.scenario:
                writable.append(out.parent / 'switch')
        if opts.source != 'worktree':
            common = git_common_dir()                          # git archive needs the repository's objects
            if common and not _inside(common, G):
                extra_ro.append(common)
    for path in writable:
        path.mkdir(parents=True, exist_ok=True)
    if ROOTFS.is_symlink() or ROOTFS.exists() and not ROOTFS.is_dir():
        print(f'GUARD REFUSED [rootfs-invalid] {ROOTFS} is not a plain directory', file=sys.stderr)
        return EXIT_REFUSED
    ROOTFS.mkdir(mode=0o755, exist_ok=True)
    # self-test only: "appeared after the scan" (not listed, not masked) and "listed but not masked"
    unmasked = {os.path.realpath(p) for p in args.selftest_unmasked or []}
    listed_only = {os.path.realpath(p) for p in args.selftest_listed_unmasked or []}
    scanned = [p for p in host_sockets(root_argv(extra_ro)[1]) if p not in unmasked]
    masks = [p for p in scanned if p not in listed_only]
    busy = [p for p in scanned if _inside(p, RUN)]
    if busy:
        # prepare() empties RUN: a socket there that a host process listens on can be neither masked nor removed
        print(f'GUARD REFUSED [host-socket-in-run-dir] a host process listens on {", ".join(busy)}', file=sys.stderr)
        return EXIT_REFUSED
    env[MASKED_VAR] = '\n'.join(scanned)
    env[EXTRA_RO_VAR] = '\n'.join(str(path) for path in extra_ro)
    for item in args.selftest_mutate or []:          # guard self-test only: prove refusals inside the sandbox
        key, _, value = item.partition('=')
        if value == '<unset>':
            env.pop(key, None)
        else:
            env[key] = value
    argv = bwrap_argv(rest, masks, writable, extra_ro)
    os.execve('/usr/bin/bwrap', argv, env)
    return 0


def cmd_guard_selftest(args) -> int:
    good = private_env()
    live = {**good, 'HOME': str(REAL_HOME), 'XDG_RUNTIME_DIR': str(LIVE_RUNTIME),
            'DBUS_SESSION_BUS_ADDRESS': f'unix:path={LIVE_RUNTIME}/bus', 'DISPLAY': ':0', 'WAYLAND_DISPLAY': 'wayland-0'}
    live.pop('GSETTINGS_BACKEND')
    cases = [
        # (name, mutation, expected code) -- the first six are the coordinator's required refusals
        ('real HOME', {'HOME': str(REAL_HOME)}, 'home-real'),
        ('real HOME with trailing slash', {'HOME': str(REAL_HOME) + '/'}, 'home-real'),
        ('live XDG_RUNTIME_DIR', {'XDG_RUNTIME_DIR': str(LIVE_RUNTIME)}, 'runtime-live'),
        ('live session bus', {'DBUS_SESSION_BUS_ADDRESS': f'unix:path={LIVE_RUNTIME}/bus'}, 'session-bus-live'),
        ('live session bus with guid', {'DBUS_SESSION_BUS_ADDRESS': f'unix:path={LIVE_RUNTIME}/bus,guid=0123abcd'}, 'session-bus-live'),
        ('live session bus escaped', {'DBUS_SESSION_BUS_ADDRESS': 'unix:path=%2frun%2fuser%2f' + str(UID) + '%2fbus'}, 'session-bus-live'),
        ('live session bus second in list', {'DBUS_SESSION_BUS_ADDRESS': f'unix:path={RT}/no-session-bus;unix:path={LIVE_RUNTIME}/bus'}, 'session-bus-live'),
        ('DISPLAY set', {'DISPLAY': ':0'}, 'display-set'),
        ('DISPLAY set but empty', {'DISPLAY': ''}, 'display-set'),
        ('WAYLAND_DISPLAY set', {'WAYLAND_DISPLAY': 'wayland-0'}, 'wayland-display-set'),
        ('GSETTINGS_BACKEND unset', {'GSETTINGS_BACKEND': None}, 'gsettings-not-keyfile'),
        ('GSETTINGS_BACKEND dconf', {'GSETTINGS_BACKEND': 'dconf'}, 'gsettings-not-keyfile'),
        # extra layers
        ('abstract session bus', {'DBUS_SESSION_BUS_ADDRESS': 'unix:abstract=/tmp/dbus-x'}, 'session-bus-not-private'),
        ('tcp session bus', {'DBUS_SESSION_BUS_ADDRESS': 'tcp:host=localhost,port=1234'}, 'session-bus-not-private'),
        ('session bus unset', {'DBUS_SESSION_BUS_ADDRESS': None}, 'session-bus-missing'),
        ('real system bus', {'DBUS_SYSTEM_BUS_ADDRESS': 'unix:path=/run/dbus/system_bus_socket'}, 'system-bus-not-private'),
        ('NO_AT_BRIDGE missing', {'NO_AT_BRIDGE': None}, 'at-bridge'),
        ('GTK_A11Y missing', {'GTK_A11Y': None}, 'gtk-a11y'),
        ('live ~/.local/bin on PATH', {'PATH': f'{REAL_HOME}/.local/bin:' + good['PATH']}, 'path-live-home'),
        ('live XDG_CONFIG_HOME', {'XDG_CONFIG_HOME': f'{REAL_HOME}/.config'}, 'xdg-not-private'),
        ('live XDG_DATA_HOME', {'XDG_DATA_HOME': f'{REAL_HOME}/.local/share'}, 'xdg-not-private'),
        ('live WIN11_REFERENCE_INSTANCE', {'WIN11_REFERENCE_INSTANCE': f'{LIVE_RUNTIME}/win11shell/instance.json'}, 'reference-instance-set'),
        ('GNOME_SHELL_SESSION_MODE=ubuntu', {'GNOME_SHELL_SESSION_MODE': 'ubuntu'}, 'live-pointer-var'),
        ('home outside the run dir', {'HOME': str(G)}, 'home-not-private'),
        # GE1h: plan-critique F24, one case per condition (and a "set but empty" or private-looking variant)
        ('F24 NO_AT_BRIDGE=0', {'NO_AT_BRIDGE': '0'}, 'at-bridge'),
        ('F24 NO_AT_BRIDGE empty', {'NO_AT_BRIDGE': ''}, 'at-bridge'),
        ('F24 GTK_A11Y=atspi', {'GTK_A11Y': 'atspi'}, 'gtk-a11y'),
        ('F24 GTK_A11Y empty', {'GTK_A11Y': ''}, 'gtk-a11y'),
        ('F24 real HOME as /.', {'HOME': str(REAL_HOME) + '/.'}, 'home-real'),
        ('F24 XDG_DATA_DIRS with ~/.local/share first', {'XDG_DATA_DIRS': f'{REAL_HOME}/.local/share:/usr/local/share:/usr/share'}, 'xdg-data-dirs-home'),
        ('F24 XDG_DATA_DIRS with ~/.local/share/ last', {'XDG_DATA_DIRS': f'/usr/local/share:/usr/share:{REAL_HOME}/.local/share/'}, 'xdg-data-dirs-home'),
        ('F24 XDG_DATA_DIRS with the live extensions dir', {'XDG_DATA_DIRS': f'/usr/share:{REAL_HOME}/.local/share/gnome-shell'}, 'xdg-data-dirs-home'),
        ('F24 WIN11_REFERENCE_SETTINGS live', {'WIN11_REFERENCE_SETTINGS': f'{REAL_HOME}/.config/win11shell/settings.json'}, 'reference-settings-set'),
        ('F24 WIN11_REFERENCE_SETTINGS empty', {'WIN11_REFERENCE_SETTINGS': ''}, 'reference-settings-set'),
        ('F24 WIN11_REFERENCE_SETTINGS private path', {'WIN11_REFERENCE_SETTINGS': str(RUN / 'config/win11-gnome/settings.json')}, 'reference-settings-set'),
        ('F24 WIN11_REFERENCE_INSTANCE empty', {'WIN11_REFERENCE_INSTANCE': ''}, 'reference-instance-set'),
        ('F24 WIN11_GNOME_CONFIG_DIR live', {'WIN11_GNOME_CONFIG_DIR': f'{REAL_HOME}/.config/win11-gnome'}, 'gnome-config-dir-set'),
        ('F24 WIN11_GNOME_CONFIG_DIR private path', {'WIN11_GNOME_CONFIG_DIR': str(RUN / 'config/win11-gnome')}, 'gnome-config-dir-set'),
        ('F24 AT_SPI_BUS_ADDRESS live', {'AT_SPI_BUS_ADDRESS': f'unix:path={LIVE_RUNTIME}/at-spi/bus'}, 'at-spi-bus-set'),
        ('F24 AT_SPI_BUS_ADDRESS private path', {'AT_SPI_BUS_ADDRESS': f'unix:path={RT}/at-spi-bus'}, 'at-spi-bus-set'),
        ('F24 XDG_SESSION_ID set', {'XDG_SESSION_ID': '2'}, 'xdg-session-id-set'),
        ('F24 XDG_SESSION_ID empty', {'XDG_SESSION_ID': ''}, 'xdg-session-id-set'),
        ('XDG_SEAT set', {'XDG_SEAT': 'seat0'}, 'live-pointer-var'),
        ('WIN11_BUILD_DIR live ~/.local/bin', {'WIN11_BUILD_DIR': f'{REAL_HOME}/.local/bin'}, 'build-dir-not-repo'),
        ('WIN11_BUILD_DIR unset', {'WIN11_BUILD_DIR': None}, 'build-dir-not-repo'),
        # R3-NATIVE (D-G44): GIO's local VFS and the update service's private seams in every run
        ('GIO_USE_VFS unset', {'GIO_USE_VFS': None}, 'gio-vfs-not-local'),
        ('GIO_USE_VFS=gvfs', {'GIO_USE_VFS': 'gvfs'}, 'gio-vfs-not-local'),
        ('update tool dir unset', {'WIN11_UPDATE_TOOL_DIR': None}, 'update-seam-not-private'),
        ('update fixture root not the private one', {'WIN11_UPDATE_FIXTURE_ROOT': str(G)}, 'update-seam-not-private'),
        ('update PackageKit bus set', {'WIN11_UPDATE_PK_BUS': 'unix:path=/run/dbus/system_bus_socket'}, 'update-seam-extra'),
        # D-G74a: WinPoint's update check seam only with --winpoint-feed (this self-test runs without it)
        ('WinPoint seam without --winpoint-feed', {WINPOINT_VAR: str(WINPOINT_SEAM)}, 'winpoint-seam-not-private'),
        ('WinPoint seam in the run dir', {WINPOINT_VAR: str(RUN / 'winpoint')}, 'winpoint-seam-not-private'),
    ]
    results, failed = [], 0

    def mutate(base, change):
        env = dict(base)
        for key, value in change.items():
            if value is None:
                env.pop(key, None)
            else:
                env[key] = value
        return env

    # 1. In-process checks of env_problems (stage outer, and inner where it matters).
    ok_outer = env_problems(good, 'outer')
    ok_inner = env_problems({**good, 'DBUS_SESSION_BUS_ADDRESS': f'unix:path={RT}/bus,guid=00ff'}, 'inner')
    for label, problems in (('positive control outer', ok_outer), ('positive control inner', ok_inner)):
        passed = problems == []
        failed += not passed
        results.append({'case': label, 'layer': 'in-process', 'expect': 'accepted', 'codes': [c for c, _ in problems], 'passed': passed})
    strict = env_problems({**good, 'DBUS_SESSION_BUS_ADDRESS': f'unix:path={RT}/other-bus'}, 'inner')
    passed = 'session-bus-not-private' in [c for c, _ in strict]
    failed += not passed
    results.append({'case': 'inner stage requires RT/bus', 'layer': 'in-process', 'expect': 'session-bus-not-private',
                    'codes': [c for c, _ in strict], 'passed': passed})
    live_codes = [c for c, _ in env_problems(live, 'outer')]
    required = ['home-real', 'runtime-live', 'session-bus-live', 'display-set', 'wayland-display-set', 'gsettings-not-keyfile']
    passed = all(code in live_codes for code in required)
    failed += not passed
    results.append({'case': 'composite live environment', 'layer': 'in-process', 'expect': required, 'codes': live_codes, 'passed': passed})
    for label, change, code in cases:
        codes = [c for c, _ in env_problems(mutate(good, change), 'outer')]
        passed = code in codes
        failed += not passed
        results.append({'case': label, 'layer': 'in-process', 'expect': code, 'codes': codes, 'passed': passed})
    # GE1h: every F24 condition at once must give every F24 code.
    f24 = {'NO_AT_BRIDGE': '0', 'GTK_A11Y': 'atspi', 'HOME': str(REAL_HOME),
           'XDG_DATA_DIRS': f'{REAL_HOME}/.local/share:/usr/local/share:/usr/share',
           'WIN11_REFERENCE_SETTINGS': f'{REAL_HOME}/.config/win11shell/settings.json',
           'WIN11_REFERENCE_INSTANCE': f'{LIVE_RUNTIME}/win11shell/instance.json',
           'WIN11_GNOME_CONFIG_DIR': f'{REAL_HOME}/.config/win11-gnome',
           'AT_SPI_BUS_ADDRESS': f'unix:path={LIVE_RUNTIME}/at-spi/bus', 'XDG_SESSION_ID': '2'}
    f24_required = ['at-bridge', 'gtk-a11y', 'home-real', 'xdg-data-dirs-home', *F24_POINTERS.values()]
    codes = [c for c, _ in env_problems(mutate(good, f24), 'outer')]
    passed = all(code in codes for code in f24_required)
    failed += not passed
    results.append({'case': 'F24 composite (all conditions)', 'layer': 'in-process', 'expect': f24_required,
                    'codes': codes, 'passed': passed})
    # GE1h: F24's last item, the extension copy GNOME Shell loads (and --patch-extension patches).
    for label, path, expect in (('F24 extension copy private', RUN / 'data/gnome-shell/extensions' / UUID, []),
                                ('F24 extension copy live', REAL_HOME / '.local/share/gnome-shell/extensions' / UUID,
                                 ['extension-copy-not-private']),
                                ('F24 extension copy in the repo', G / 'extension', ['extension-copy-not-private']),
                                ('F24 extension copy system', Path('/usr/share/gnome-shell/extensions') / UUID,
                                 ['extension-copy-not-private'])):
        codes = [c for c, _ in extension_copy_problems(path)]
        passed = codes == expect
        failed += not passed
        results.append({'case': label, 'layer': 'in-process', 'expect': expect or 'accepted', 'codes': codes,
                        'passed': passed})

    script = str(HERE / 'harness.py')

    def run_guard_only(env, via_launch=False, mutations=()):
        if via_launch:
            argv = [sys.executable, '-B', '-I', script, 'launch']
            for m in mutations:
                argv += ['--selftest-mutate', m]
            argv += ['run', '--lang', 'nl', '--guard-only']
        else:
            argv = [sys.executable, '-B', '-I', script, 'run', '--lang', 'nl', '--guard-only']
        proc = subprocess.run(argv, env=env, capture_output=True, text=True, timeout=60)
        codes = re.findall(r'GUARD REFUSED \[([a-z0-9-]+)\]', proc.stderr)
        return proc.returncode, codes, (proc.stdout + proc.stderr).strip()[-600:]

    # 2. Real processes outside the sandbox: the caller's own environment, then every mutation.
    rc, codes, out = run_guard_only(dict(os.environ))
    passed = rc == EXIT_REFUSED
    failed += not passed
    results.append({'case': "caller's own environment, no sandbox", 'layer': 'process', 'expect': 'refused',
                    'exit': rc, 'codes': codes, 'passed': passed,
                    'callerLooksLive': os.environ.get('XDG_RUNTIME_DIR') == str(LIVE_RUNTIME)})
    rc, codes, out = run_guard_only(good)
    passed = rc == EXIT_REFUSED and codes and all(c.startswith('not-sandboxed') for c in codes)
    failed += not passed
    results.append({'case': 'private env but no sandbox', 'layer': 'process', 'expect': 'only not-sandboxed-*',
                    'exit': rc, 'codes': codes, 'passed': bool(passed)})
    rc, codes, out = run_guard_only(live)
    passed = rc == EXIT_REFUSED and all(code in codes for code in required)
    failed += not passed
    results.append({'case': 'composite live environment, no sandbox', 'layer': 'process', 'expect': required,
                    'exit': rc, 'codes': codes, 'passed': passed})
    rc, codes, out = run_guard_only(mutate(good, f24))
    passed = rc == EXIT_REFUSED and all(code in codes for code in f24_required)
    failed += not passed
    results.append({'case': 'F24 composite, no sandbox', 'layer': 'process', 'expect': f24_required,
                    'exit': rc, 'codes': codes, 'passed': passed})

    # 3. Inside the real bubblewrap sandbox: the private env passes; each required mutation is refused.
    rc, codes, out = run_guard_only(dict(os.environ), via_launch=True)
    passed = rc == 0 and 'GUARD OK' in out
    failed += not passed
    results.append({'case': 'sandbox + private env (positive control)', 'layer': 'sandbox', 'expect': 'accepted',
                    'exit': rc, 'codes': codes, 'passed': passed, 'output': out[-200:]})
    for label, change, code in cases:
        mutations = [f'{k}={"<unset>" if v is None else v}' for k, v in change.items()]
        rc, codes, out = run_guard_only(dict(os.environ), via_launch=True, mutations=mutations)
        passed = rc == EXIT_REFUSED and code in codes and not any(c.startswith('not-sandboxed') for c in codes)
        failed += not passed
        results.append({'case': label, 'layer': 'sandbox', 'expect': code, 'exit': rc, 'codes': codes, 'passed': passed})
    rc, codes, out = run_guard_only(dict(os.environ), via_launch=True, mutations=[f'{k}={v}' for k, v in f24.items()])
    passed = rc == EXIT_REFUSED and all(code in codes for code in f24_required) \
        and not any(c.startswith('not-sandboxed') for c in codes)
    failed += not passed
    results.append({'case': 'F24 composite', 'layer': 'sandbox', 'expect': f24_required, 'exit': rc, 'codes': codes,
                    'passed': passed})

    # 3b. R3-NATIVE (D-G42): the allowlisted root and the host-socket masking. A listening decoy socket in the
    #    checkout (bound by this process, outside the sandbox, so launch's /proc/net/unix scan sees it) counts
    #    every connection; nothing inside the sandbox may reach it.
    decoy = Decoy(G / 'tests' / 'logs' / f'd{os.getpid()}.s')
    try:
        def probe(*launch_opts, connect=True):
            argv = [sys.executable, '-B', '-I', script, 'launch', *launch_opts, 'probe']
            if connect:
                argv += ['--connect', str(decoy.path)]
            proc = subprocess.run(argv, env=dict(os.environ), capture_output=True, text=True, timeout=60)
            try:
                report = json.loads(proc.stdout[proc.stdout.index('{'):])
            except ValueError:
                report = {}
            codes = sorted({c for c, _ in report.get('problems', [])})
            return proc.returncode, codes, report, (proc.stdout + proc.stderr).strip()[-400:]

        before = decoy.count()
        decoy.connect_once()
        passed = decoy.count() == before + 1
        failed += not passed
        results.append({'case': 'decoy positive control: a connect from outside the sandbox reaches it',
                        'layer': 'sockets', 'expect': 1, 'codes': [decoy.count() - before], 'passed': passed})
        rc, codes, report, out = probe()
        facts = report.get('facts', {})
        root_ok = (not facts.get('runDbus') and not facts.get('runUser') and not facts.get('runSystemd')
                   and facts.get('realHomeEntries') is not None and set(facts['realHomeEntries']) <= {G.relative_to(REAL_HOME).parts[0]}
                   and not facts.get('varLib') and not facts.get('snapdSocket'))
        hits = decoy.count() - before - 1
        passed = (rc == 0 and not codes and str(report.get('connect', '')).startswith('error')
                  and report.get('connectTarget', {}).get('isSocket') is False and hits == 0 and root_ok
                  and str(decoy.path) in facts.get('masked', []))
        failed += not passed
        results.append({'case': 'sandbox: the decoy is masked (not a socket inside, connect fails, 0 connections); '
                                '/run/dbus, /run/user, /run/systemd empty, no real home, /var/lib or snapd socket',
                        'layer': 'sockets', 'expect': 'accepted, 0 connections', 'exit': rc, 'codes': codes,
                        'connect': report.get('connect'), 'decoyConnections': hits, 'facts': facts, 'passed': passed,
                        **({} if passed else {'output': out})})
        before = decoy.count()
        rc, codes, report, out = probe('--selftest-unmasked', str(decoy.path))
        hits = decoy.count() - before
        passed = rc == EXIT_REFUSED and 'host-socket-visible' in codes and 'connect' not in report and hits == 0
        failed += not passed
        results.append({'case': 'sandbox: a socket in the checkout that is not masked (appeared after the scan) is '
                                'refused before any connect', 'layer': 'sockets', 'expect': 'host-socket-visible',
                        'exit': rc, 'codes': codes, 'decoyConnections': hits, 'passed': passed,
                        **({} if passed else {'output': out})})
        before = decoy.count()
        rc, codes, report, out = probe('--selftest-mutate', f'{MASKED_VAR}=<unset>')
        hits = decoy.count() - before
        passed = rc == EXIT_REFUSED and 'not-sandboxed-no-mask-list' in codes and hits == 0
        failed += not passed
        results.append({'case': 'sandbox: no list of masked host sockets (a sandbox launch did not scan)',
                        'layer': 'sockets', 'expect': 'not-sandboxed-no-mask-list', 'exit': rc, 'codes': codes,
                        'decoyConnections': hits, 'passed': passed, **({} if passed else {'output': out})})
        # Fix round M2: / is ROOTFS, a read-only bind on the checkout's own file system (not a tmpfs), so the
        # storage service sees a system drive at / as on a real system.
        rc, codes, report, out = probe(connect=False)
        mount = report.get('facts', {}).get('rootMount', {})
        passed = (rc == 0 and mount.get('fsType') not in (None, 'tmpfs') and mount.get('readOnly') is True
                  and mount.get('sameDeviceAsCheckout') is True and report['facts'].get('rootWritable') is False
                  and str(mount.get('root', '')).endswith(str(ROOTFS)))
        failed += not passed
        results.append({'case': "sandbox: / is a read-only bind of run/root on the checkout's file system (storage "
                                'sees a system drive at /)', 'layer': 'sockets', 'expect': 'not tmpfs, ro, same device',
                        'exit': rc, 'codes': codes, 'rootMount': mount, 'passed': passed,
                        **({} if passed else {'output': out})})
        # Fix round m3: a host socket that launch lists but does not mask (the guard's "still a socket" branch)
        before = decoy.count()
        rc, codes, report, out = probe('--selftest-listed-unmasked', str(decoy.path))
        hits = decoy.count() - before
        still = [m for c, m in report.get('problems', []) if c == 'host-socket-visible' and 'still a socket' in m]
        passed = rc == EXIT_REFUSED and bool(still) and 'connect' not in report and hits == 0
        failed += not passed
        results.append({'case': 'sandbox: a host socket that is listed but not masked is refused before any connect',
                        'layer': 'sockets', 'expect': 'host-socket-visible (still a socket)', 'exit': rc,
                        'codes': codes, 'messages': still[:2], 'decoyConnections': hits, 'passed': passed,
                        **({} if passed else {'output': out})})
        # Fix round m3: an unmasked socket in a bound host directory (not the checkout): the guard scans those too
        outside = [p for p in host_sockets(root_argv()[1]) if not _inside(p, G)]
        if outside:
            rc, codes, report, out = probe('--selftest-unmasked', outside[0])
            seen = [m for c, m in report.get('problems', []) if c == 'host-socket-visible' and outside[0] in m]
            passed = rc == EXIT_REFUSED and bool(seen) and 'connect' not in report and hits == 0
            results.append({'case': 'sandbox: an unmasked socket in a bound host directory is refused before any connect',
                            'layer': 'sockets', 'expect': 'host-socket-visible (host directory)', 'exit': rc,
                            'codes': codes, 'socket': outside[0], 'messages': seen[:2], 'passed': passed,
                            **({} if passed else {'output': out})})
        else:
            passed = True
            results.append({'case': 'sandbox: an unmasked socket in a bound host directory is refused before any connect',
                            'layer': 'sockets', 'expect': 'host-socket-visible (host directory)', 'codes': [],
                            'skipped': 'no host socket in the bound host directories on this machine', 'passed': True})
        failed += not passed
        # Fix round m3, n3: the self-test's options never reach a session, and --out may not be tests/gnome itself.
        # --lock-check-only stops before bubblewrap whatever the outcome, so no session can start here.
        for label, launch_opts, run_opts in (
                ('--selftest-unmasked with run', ['--selftest-unmasked', str(decoy.path)], []),
                ('--selftest-listed-unmasked with run', ['--selftest-listed-unmasked', str(decoy.path)], []),
                ('--selftest-mutate with a session run', ['--selftest-mutate', 'HOME=/x'], []),
                ('--out tests/gnome itself (n3)', [], ['--out', str(HERE)]),
                ('--out containing run/ (n3)', [], ['--out', str(HERE.parent)])):
            argv = [sys.executable, '-B', '-I', script, 'launch', '--lock-check-only', *launch_opts, 'run', '--lang',
                    'nl', *run_opts]
            proc = subprocess.run(argv, env=dict(os.environ), capture_output=True, text=True, timeout=60)
            codes = re.findall(r'GUARD REFUSED \[([a-z0-9-]+)\]', proc.stderr)
            expect = 'out-invalid' if run_opts else 'selftest-option-misuse'
            passed = proc.returncode == EXIT_REFUSED and codes == [expect] and 'LOCK DECISION' not in proc.stdout
            failed += not passed
            results.append({'case': f'launch: {label} is refused', 'layer': 'sockets', 'expect': expect,
                            'exit': proc.returncode, 'codes': codes, 'passed': passed,
                            **({} if passed else {'output': (proc.stdout + proc.stderr)[-400:]})})
        results.append({'case': 'decoy total', 'layer': 'sockets', 'expect': 1, 'codes': [decoy.count()],
                        'passed': decoy.count() == 1})
        failed += decoy.count() != 1
    finally:
        decoy.close()

    # 3c. Fix round m4: the strace connect check folds '..', '.', '//' and /var/run before it matches, and counts a
    #     relative path as outside (synthetic log; nothing is traced or connected).
    synthetic = G / 'tests' / 'logs' / f'strace-selftest-{os.getpid()}.txt'
    synthetic.parent.mkdir(parents=True, exist_ok=True)
    lines = {'/var/run/udev/control': True, '/run/udev/control': True, '/run/../home/x.sock': True,
             '/var/run/../home/x.s': True,
             '/var/cache/../../home/x/.cache/ibus/dbus-x': True, '//run/udev/control': True, 'rel/x.sock': True,
             '/var/run/../cache/fontconfig/x': True, f'{RT}/bus': False, f'{RUN}/cache/ibus/db': False,
             '/var/run/nscd/socket': False, '/run/./user/../x.s': False}
    synthetic.write_text(''.join(f'{100 + i} connect({i + 3}, {{sa_family=AF_UNIX, sun_path="{path}"}}, 110) = 0\n'
                                 for i, path in enumerate(lines)))
    try:
        report = strace_connect_report([synthetic])
    finally:
        synthetic.unlink(missing_ok=True)
    flagged = {path for path in lines if any(v.startswith(f'connect to {path}') and
                                             v[len(f'connect to {path}'):][:1] in (':', ' ') for v in report['violations'])}
    want = {path for path, bad in lines.items() if bad}
    passed = flagged == want and len(report['violations']) == len(want)
    failed += not passed
    results.append({'case': 'strace check: /var/run, .., . and // are folded and a relative path counts as outside',
                    'layer': 'strace', 'expect': sorted(want), 'codes': sorted(flagged), 'passed': passed,
                    **({} if passed else {'violations': report['violations']})})

    # 3c2. WU-FIX (D-G51, GW5 review m6): the strace execve check fails a run in which a real pkexec, check-new-release,
    #      apt-get, apt-cache, do-release-upgrade or loginctl ran; the stubs under run/, a failed PATH attempt and
    #      other programs do not count (synthetic log; nothing is traced or executed).
    synthetic = G / 'tests' / 'logs' / f'strace-exec-selftest-{os.getpid()}.txt'
    synthetic.parent.mkdir(parents=True, exist_ok=True)
    env_tail = ', 0x7ffd0000 /* 30 vars */'
    exec_lines = [
        ('/usr/bin/pkexec', f'100 execve("/usr/bin/pkexec", ["pkexec", "/bin/sh", "-c", "x"]{env_tail}) = 0', True),
        ('/usr/lib/ubuntu-release-upgrader/check-new-release',
         f'101 execve("/usr/lib/ubuntu-release-upgrader/check-new-release", ["check-new-release", "-q"]{env_tail}) = 0', True),
        ('/bin/loginctl', f'102 execve("/bin/loginctl", ["loginctl", "reboot"]{env_tail}) = 0', True),
        ('/usr/local/bin/apt-get', f'103 execve("/usr/local/bin/apt-get", ["apt-get"]{env_tail}) = -1 ENOENT (No such file or directory)', False),
        (f'{RUN}/stubs/pkexec', f'104 execve("{RUN}/stubs/pkexec", ["pkexec"]{env_tail}) = 0', False),
        (f'{RUN}/update-tools/check-new-release',
         f'105 execve("{RUN}/update-tools/check-new-release", ["check-new-release", "-q"]{env_tail}) = 0', False),
        ('/usr/bin/apt-cache', f'106 execve("/usr/bin/apt-cache", ["apt-cache", "policy"]{env_tail} <unfinished ...>', True),
        ('/usr/bin/do-release-upgrade',
         f'107 execveat(AT_FDCWD, "/usr/bin/do-release-upgrade", ["do-release-upgrade"]{env_tail}, 0) = 0', True),
        ('/usr/bin/gnome-shell', f'108 execve("/usr/bin/gnome-shell", ["gnome-shell", "--headless"]{env_tail}) = 0', False),
        ('/usr/sbin/../bin/apt-get', f'109 execve("/usr/sbin/../bin/apt-get", ["apt-get", "update"]{env_tail}) = 0', True),
        (f'{RUN}/stubs/../../../../../../../usr/bin/pkexec',
         f'110 execve("{RUN}/stubs/../../../../../../../usr/bin/pkexec", ["pkexec"]{env_tail}) = 0', True),
        ('usr/bin/loginctl', f'111 execve("usr/bin/loginctl", ["loginctl"]{env_tail}) = 0', True),
    ]
    synthetic.write_text(''.join(line + '\n' for _path, line, _bad in exec_lines) + '106 <... execve resumed>) = 0\n')
    try:
        report = strace_exec_report([synthetic])
    finally:
        synthetic.unlink(missing_ok=True)
    flagged = {path for path, _line, _bad in exec_lines
               if any(v == f'executed {path}' or v.startswith(f'executed {path} (') for v in report['violations'])}
    want = {path for path, _line, bad in exec_lines if bad}
    passed = flagged == want and len(report['violations']) == len(want) and report['execs'] == len(exec_lines)
    failed += not passed
    results.append({'case': 'strace exec check (WU-FIX, D-G51): a real pkexec, check-new-release, apt-get, apt-cache, '
                            'do-release-upgrade or loginctl fails the run; the run/ stubs and a failed attempt do not',
                    'layer': 'strace', 'expect': sorted(want), 'codes': sorted(flagged), 'passed': passed,
                    **({} if passed else {'violations': report['violations'], 'execs': report['execs']})})

    # 3c3. D-G74: the one Python program. A python3 below a WinPoint process must be the release bundle's installer
    #      with its frozen argv and environment, after the scenario's first click mark; the harness's own Python and
    #      GNOME's are no WinPoint processes (synthetic log; nothing is traced or executed).
    synthetic = G / 'tests' / 'logs' / f'strace-python-selftest-{os.getpid()}.txt'
    work = f'{RUN}/cache/win11-gnome/winpoint/work/0.9.0'
    installer = ['/usr/bin/python3', '-E', '-s', '-B', f'{work}/bundle/scripts/install.py', f'{work}/bundle', '--lang', 'nl',
                 '--no-session']
    check_argv = installer + ['--check']
    install_argv = installer + ['--status', f'{work}/install.status', '--log', f'{work}/install.log']

    def ex(pid: int, argv: list[str], env: int = 12) -> str:
        return f'{pid} execve("{argv[0]}", [{", ".join(json.dumps(a) for a in argv)}], 0x7ffd0000 /* {env} vars */) = 0'
    python_lines = [
        ex(200, ['/usr/bin/python3', '-B', '-I', str(HERE / 'harness.py'), '_inner']),
        ex(201, [f'{RUN}/prefix/bin/win11-panel', '--http-only'], 40),
        '201 clone3({flags=CLONE_VM|CLONE_FS|CLONE_FILES|CLONE_SIGHAND|CLONE_THREAD, exit_signal=0}, 88) = 202',
        '202 clone3({flags=CLONE_VM|CLONE_VFORK|CLONE_CLEAR_SIGHAND, exit_signal=SIGCHLD}, 88 <unfinished ...>',
        ex(203, check_argv),                                                      # before the click: refused
        '202 <... clone3 resumed>) = 203',
        ex(300, ['/usr/bin/true', f'{WINPOINT_CLICK}-1'], 40),
        '202 clone3({flags=CLONE_VM|CLONE_VFORK, exit_signal=SIGCHLD}, 88) = 204',
        ex(204, check_argv),                                                      # click 1's --check: allowed
        '202 clone3({flags=CLONE_VM|CLONE_VFORK, exit_signal=SIGCHLD}, 88) = 209',
        ex(209, install_argv),                                                    # and its install: allowed
        '202 clone3({flags=CLONE_VM|CLONE_VFORK, exit_signal=SIGCHLD}, 88) = 205',
        ex(205, ['/usr/bin/python3', '-c', 'print(1)']),                          # any other Python: refused
        '202 clone(child_stack=NULL, flags=CLONE_CHILD_CLEARTID|SIGCHLD, child_tidptr=0x7f00) = 206',
        ex(206, ['/usr/bin/python3', '-I', '-s', '-B'] + installer[4:] + ['--check']),     # -I: refused
        ex(207, ['/usr/bin/python3', '/usr/share/ibus-table/engine/main.py', '--xml'], 45),  # GNOME's: no WinPoint
        '202 vfork() = 208',
        ex(208, check_argv, 40),                                  # the session's environment, and no click of its own
        '202 clone3({flags=CLONE_VM|CLONE_VFORK, exit_signal=SIGCHLD}, 88) = 210',
        ex(210, install_argv),                                    # a second install with no new click: refused
        ex(301, ['/usr/bin/true', f'{WINPOINT_CLICK}-2'], 40),
        '202 clone3({flags=CLONE_VM|CLONE_VFORK, exit_signal=SIGCHLD}, 88) = 211',
        ex(211, install_argv),                                    # click 2: an install before its --check, refused
    ]
    synthetic.write_text(''.join(line + '\n' for line in python_lines))
    try:
        report = winpoint_python_report(synthetic)
    finally:
        synthetic.unlink(missing_ok=True)
    flagged = sorted({int(v.split()[1].rstrip(':')) for v in report['violations']})
    want = [5, 13, 15, 18, 20, 23]
    modes = [(row['line'], row['mode']) for row in report['execs']]
    passed = flagged == want and report['clicks'] == 2 and modes == [
        (5, 'check'), (9, 'check'), (11, 'install'), (13, None), (15, None), (18, 'check'), (20, 'install'),
        (23, 'install')] and report['perClick'] == {1: ['check', 'install', 'check', 'install'], 2: ['install']}
    # the runs the winpoint-update scenario's clicks caused: exactly those (none missing, none extra, none recorded)
    expectations = [
        ({1: ['check', 'install'], 2: []}, {'1': ['check', 'install'], '2': []}, False),
        ({1: [], 2: []}, {'1': ['check', 'install'], '2': []}, True),             # marks, but no installer in the trace
        ({1: ['check', 'install'], 2: ['check']}, {'1': ['check', 'install'], '2': []}, True),
        ({1: ['check', 'install']}, {}, True),
        ({1: ['check', 'install']}, None, True),
    ]
    expected_ok = [bool(winpoint_expected_problems(got, recorded)) == bad for got, recorded, bad in expectations]
    passed = passed and all(expected_ok)
    failed += not passed
    results.append({'case': "D-G74 exec rule: a python3 below a WinPoint process is only the release bundle's "
                            'installer with its frozen argv and environment, one --check and then one install per '
                            "click mark; the harness and GNOME's own Python are not WinPoint's; with the "
                            'winpoint-update scenario exactly the runs its clicks caused', 'layer': 'strace',
                    'expect': want, 'codes': flagged, 'passed': passed,
                    **({} if passed else {'violations': report['violations'], 'execs': modes,
                                          'perClick': report['perClick'], 'expected': expected_ok})})

    # 3d. Fix round m2: the socket watch finds a socket that appears during a run and calls its stop handler
    #     (in-process, on a scratch directory; the listener is ours and nothing connects to it).
    scratch_dir = G / 'tests' / 'logs' / f'sw{os.getpid()}'
    scratch_dir.mkdir(parents=True, exist_ok=True)
    calls: list = []
    watch = SocketWatch(lambda paths: calls.append((time.monotonic(), paths)), roots=[scratch_dir], interval=0.5)
    late = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    try:
        watch.start()
        time.sleep(1.2)
        quiet = list(calls)
        late.bind(str(scratch_dir / 'late.s'))
        bound = time.monotonic()
        late.listen(1)
        end = bound + 3
        while not calls and time.monotonic() < end:
            time.sleep(0.05)
    finally:
        watch.stop_event.set()
        watch.join(timeout=3)
        late.close()
        (scratch_dir / 'late.s').unlink(missing_ok=True)
        scratch_dir.rmdir()
    delay = round(calls[0][0] - bound, 2) if calls else None
    passed = (not quiet and len(calls) == 1 and calls[0][1] == [str(scratch_dir / 'late.s')] and delay is not None
              and delay <= 2 * watch.interval + 0.5 and watch.summary()['ok'] is False)
    failed += not passed
    results.append({'case': 'socket watch: a socket that appears during the run is found within two scans and stops '
                            'the session', 'layer': 'sockets', 'expect': 'one call, <= 2 scans', 'codes': [delay],
                    'scans': watch.scans, 'passed': passed})

    # 4. GE1h: the heavy-lock decision of `launch run` (contracts C8). A scratch lock file stands in for
    #    .heavy.lock (WIN11_HEAVY_LOCK, honoured by tools/heavy.sh and the harness), so the real lock is never
    #    taken or waited for. --lock-check-only stops before bubblewrap: no session is started.
    scratch = G / 'tests' / 'logs' / f'ge1h-selftest-{os.getpid()}.lock'
    other = scratch.with_name(scratch.name + '.other')
    scratch.parent.mkdir(parents=True, exist_ok=True)
    scratch.touch()
    other.touch()
    lock_env = {**os.environ, 'WIN11_HEAVY_LOCK': str(scratch)}
    probe = [sys.executable, '-B', '-I', script, 'launch', '--lock-check-only', 'run', '--lang', 'nl']
    lock_cases = [
        ('no ancestor holds the lock', probe, 'refused', EXIT_LOCK),
        ('flock(1) ancestor holds the lock file', ['flock', '-n', str(scratch)] + probe, 'ancestor:flock', 0),
        ('flock(1) ancestor holds another file', ['flock', '-n', str(other)] + probe, 'refused', EXIT_LOCK),
        ('tools/heavy.sh ancestor holds the lock', [str(G / 'tools' / 'heavy.sh'), '--min-mb', '1', '--tag',
                                                    'ge1h-selftest', '--'] + probe, 'ancestor:heavy.sh', 0),
        ('--own-lock with the lock free', probe[:5] + ['--own-lock'] + probe[5:], 'own', 0),
    ]
    try:
        for label, argv, expect, want_rc in lock_cases:
            proc = subprocess.run(argv, env=lock_env, capture_output=True, text=True, timeout=60)
            decision = (re.findall(r'LOCK DECISION (\S+)', proc.stdout) or ['none'])[-1]
            passed = decision == expect and proc.returncode == want_rc
            failed += not passed
            results.append({'case': f'lock: {label}', 'layer': 'lock', 'expect': expect, 'exit': proc.returncode,
                            'codes': [decision], 'passed': passed,
                            **({} if passed else {'output': (proc.stdout + proc.stderr)[-400:]})})
    finally:
        scratch.unlink(missing_ok=True)
        other.unlink(missing_ok=True)

    summary = {'cases': len(results), 'failed': failed, 'results': results,
               'when': time.strftime('%Y-%m-%d %H:%M:%S'), 'commit': _git_head()}
    BASELINE.mkdir(parents=True, exist_ok=True)
    (BASELINE / 'guard-selftest.json').write_text(json.dumps(summary, indent=1, ensure_ascii=False) + '\n')
    for row in results:
        print(('PASS ' if row['passed'] else 'FAIL ') + f"[{row['layer']}] {row['case']}: expect {row['expect']} -> "
              f"{row.get('exit', '-')} {row['codes']}")
    print(f'guard self-test: {len(results) - failed}/{len(results)} passed')
    return 0 if failed == 0 else 1


class Decoy:
    """A listening unix socket that only counts the connections it accepts (guard self-test, D-G42)."""

    def __init__(self, path: Path):
        self.path = path
        assert len(str(path)) < 104, f'decoy socket path too long: {path}'
        path.parent.mkdir(parents=True, exist_ok=True)
        path.unlink(missing_ok=True)
        self.server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.server.bind(str(path))
        self.server.listen(8)
        self.hits = 0
        self.lock = threading.Lock()
        self.thread = threading.Thread(target=self._accept, daemon=True)
        self.thread.start()

    def _accept(self):
        while True:
            try:
                conn, _ = self.server.accept()
            except OSError:
                return
            with self.lock:
                self.hits += 1
            conn.close()

    def count(self) -> int:
        time.sleep(0.3)
        with self.lock:
            return self.hits

    def connect_once(self):
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as client:
            client.connect(str(self.path))

    def close(self):
        try:
            self.server.shutdown(socket.SHUT_RDWR)
        except OSError:
            pass
        self.server.close()
        self.path.unlink(missing_ok=True)


def _git_head() -> str:
    try:
        return (G / '.git' / 'HEAD').read_text().strip()
    except OSError:
        return '?'


# --------------------------------------------------------------------------------------------------------
# run (inside the sandbox): lock, memory, private prefix, dbus-run-session, sampler, cleanup
# --------------------------------------------------------------------------------------------------------

BUS_CONF = """<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <!-- GE0c private bus: no <servicedir>, so nothing is ever auto-activated. -->
  <type>session</type>
  <keep_umask/>
  <!-- The sandbox's /sys is read-only, so dbus-daemon cannot query AppArmor; all peers are ours anyway. -->
  <apparmor mode="disabled"/>
  <listen>unix:path={path}</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
"""

KEYFILE = """[org/gnome/shell]
enabled-extensions=['{uuid}']
disabled-extensions=[]
disable-user-extensions=false
welcome-dialog-last-shown-version='999'

[org/gnome/desktop/session]
idle-delay=uint32 0

[org/gnome/desktop/screensaver]
lock-enabled=false
idle-activation-enabled=false
"""


def empty_run_dir() -> None:
    """Remove everything in RUN, not RUN itself: inside the sandbox RUN is a bind mount (D-G42). ROOTFS stays:
    it is the sandbox's / and its subdirectories are the mount points (fix round M2)."""
    RUN.mkdir(parents=True, exist_ok=True)
    for child in RUN.iterdir():
        if child == ROOTFS:
            continue
        if child.is_dir() and not child.is_symlink():
            shutil.rmtree(child)
        else:
            child.unlink()


def clean_run_sockets() -> dict:
    """After the session (every process of it is gone): remove the socket files it left in RUN (GX1's run left
    run/cache/ibus/dbus-*, and tests/update/run.sh then refuses with host-socket-visible), then check that the
    whole checkout holds no socket file (a masked host socket reads as /dev/null). Not ok fails the run."""
    removed = []
    for path in checkout_sockets(include_run=True):
        if _inside(path, RUN):
            try:
                os.unlink(path)
                removed.append(os.path.relpath(path, RUN))
            except OSError as error:
                removed.append(f'{os.path.relpath(path, RUN)} (not removed: {error.strerror})')
    left = checkout_sockets(include_run=True)
    return {'removed': sorted(removed), 'left': left, 'ok': not left}


def prepare_update_seams() -> None:
    """D-G44 (gw4.md N2/N3, gx1.md m5): the update service's test seams for every run (paths.rs): a private tool
    directory whose stubs only log to logs/update-tool-calls.log and fail, and a private fixture root with the
    real /proc (the restart and release-upgrader scans see the sandbox's processes, as before), update-notifier's
    compiled schema (tests/update/fixtures/options) and nothing else: no apt history, no reboot-required, no dpkg
    state. So no Settings of a harness run runs /usr/bin/snap or apt-config or reads the real /var/log/apt."""
    calls = RUN / 'logs' / 'update-tool-calls.log'
    calls.write_text('')
    for seam in UPDATE_SEAMS.values():
        seam.mkdir(mode=0o700)
        os.chmod(seam, 0o700)
        (seam / UPDATE_MARKER).write_text('GE0c harness: update service test seam (D-G44)\n')
    for name in UPDATE_TOOL_NAMES:
        # the service starts a tool with a cleared environment and PATH = this directory: absolute paths only
        stub = UPDATE_TOOLS / name
        stub.write_text('#!/bin/sh\n'
                        f'printf \'%s\\t%s\\t%s\\n\' "$(/usr/bin/date +%H:%M:%S)" "{name}" "$*" >> "{calls}"\n'
                        f'echo "GE0c update tool stub: {name} is blocked in the private GNOME harness" >&2\n'
                        'exit 97\n')
        stub.chmod(0o755)
    (UPDATE_FIXTURE / 'proc').symlink_to('/proc')
    schemas = UPDATE_FIXTURE / 'usr' / 'share' / 'glib-2.0' / 'schemas'
    schemas.mkdir(parents=True)
    for name in ('gschemas.compiled', 'com.ubuntu.update-notifier.gschema.xml'):
        if (UPDATE_SCHEMAS / name).is_file():
            shutil.copyfile(UPDATE_SCHEMAS / name, schemas / name)


def prepare(lang: str, seed: str = 'defaults', profile_picture: str | None = None) -> None:
    profile = Path(profile_picture).resolve() if profile_picture else None
    if profile and (not profile.is_relative_to(G.resolve()) or not profile.is_file()):
        raise Fatal('--profile-picture must be an image file inside the checkout')
    empty_run_dir()
    for sub in ('home', 'config', 'data', 'cache', 'prefix', 'tmp', 'logs', 'stubs', 'shots'):
        (RUN / sub).mkdir(parents=True, exist_ok=True)
    RT.mkdir(mode=0o700, exist_ok=True)
    os.chmod(RT, 0o700)
    os.chmod(RUN / 'home', 0o700)
    os.chmod(RUN / 'stubs', 0o700)
    # dbus-daemon refuses unix socket paths longer than 99 bytes (_DBUS_MAX_SUN_PATH_LENGTH), Wayland 107.
    # The system bus socket is RT/sbus so a git worktree (.wt/<name>) still fits.
    for name in ('bus', 'sbus'):
        assert len(str(RT / name)) < 100, f'socket path too long for dbus: {RT / name}'
    for name in (WL, WL + '.lock'):
        assert len(str(RT / name)) < 104, f'socket path too long: {RT / name}'
    prepare_update_seams()
    guard_log = RUN / 'logs' / 'guard-calls.log'
    guard_log.write_text('')
    for name in STUBS:
        stub = RUN / 'stubs' / name
        stub.write_text('#!/bin/sh\n'
                        f'printf \'%s\\t%s\\t%s\\n\' "$(date +%H:%M:%S)" "{name}" "$*" >> "{guard_log}"\n'
                        f'echo "GE0c guard stub: {name} is blocked in the private GNOME harness" >&2\n'
                        'exit 97\n')
        stub.chmod(0o755)
    (RUN / 'session-bus.conf').write_text(BUS_CONF.format(path=RT / 'bus'))
    (RUN / 'system-bus.conf').write_text(BUS_CONF.format(path=RT / 'sbus'))
    for name in ('command.json', 'report.jsonl'):
        (RUN / 'logs' / name).write_text('')
        os.chmod(RUN / 'logs' / name, 0o600)
    keyfile = RUN / 'config/glib-2.0/settings/keyfile'
    keyfile.parent.mkdir(parents=True)
    keyfile.write_text(KEYFILE.replace("enabled-extensions=['{uuid}']", 'enabled-extensions=@as []')
                       if SESSION_MODE else KEYFILE.replace('{uuid}', UUID))
    if SESSION_MODE:
        # the mode file scripts/install-session.py installs, generated by the same code
        spec = importlib.util.spec_from_file_location('install_session', SRC / 'scripts' / 'install-session.py')
        session = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(session)
        modes = RUN / 'sysdata' / 'gnome-shell' / 'modes'
        modes.mkdir(parents=True, exist_ok=True)
        (modes / f'{SESSION_MODE}.json').write_text(session.mode_json())
    home = RUN / 'home'
    if profile:
        shutil.copyfile(profile, home / '.face')
    dirs = {'DESKTOP': 'Desktop', 'DOCUMENTS': 'Documents', 'DOWNLOAD': 'Downloads', 'MUSIC': 'Music',
            'PICTURES': 'Pictures', 'VIDEOS': 'Videos', 'TEMPLATES': 'Templates', 'PUBLICSHARE': 'Public'}
    for name in dirs.values():
        (home / name).mkdir()
    (RUN / 'config/user-dirs.dirs').write_text(''.join(f'XDG_{k}_DIR="{home / v}"\n' for k, v in dirs.items()))
    for name in ('alpha.txt', 'beta.txt'):
        (home / 'Desktop' / name).write_text('GE0c private desktop fixture\n')
    (home / 'Desktop' / 'Folder').mkdir()
    (home / 'Documents' / 'report.txt').write_text('GE0c private fixture\n')
    (home / 'Documents' / 'Project').mkdir()
    (RUN / 'data' / 'Trash').mkdir()
    config = RUN / 'config' / 'win11-gnome'
    config.mkdir(mode=0o700)
    os.chmod(config, 0o700)
    if seed == 'minimal':
        # Reproduces the GE0c finding: only the first read merges defaults (host/src/settings.rs:327-331),
        # so a live rewrite of a partial file drops theme, pins, ... until the hosts restart.
        write_settings({'language': lang}, create=True)
    else:
        # A file the host has saved once: the complete defaults with the wanted language.
        defaults = json.loads((SRC / 'host/src/settings-defaults.json').read_text())
        write_settings({**seed_defaults(defaults), 'language': lang}, create=True)


def seed_defaults(defaults: dict) -> dict:
    """D-G74a: without --winpoint-feed WinPoint's daily update check is off in the seeded settings (its owner would
    ask the real GitHub 3 to 10 minutes into a run, which the loopback-only sandbox cannot reach: a failed card and a
    resolver connect in the strace report). The page's "Controleren op updates" still works (and fails the same way)."""
    if WINPOINT_FEED:
        return defaults
    return {**defaults, 'update': {**defaults.get('update', {}), 'winpointAutoCheck': False}}


def prepare_native_snap_fixture() -> None:
    """Before the first Shell/panel boot, seed only this run's keyfile and disabled TA stand-in."""
    if SESSION_MODE != 'winpoint':
        raise Fatal('--native-snap-fixture requires --session-mode winpoint')
    keyfile = RUN / 'config/glib-2.0/settings/keyfile'
    with keyfile.open('a') as stream:
        stream.write('\n[org/gnome/mutter]\nedge-tiling=false\n'
                     '\n[org/gnome/mutter/keybindings]\ntoggle-tiled-left=@as []\ntoggle-tiled-right=@as []\n')
    # A real, installed-but-disabled extension in Main.extensionManager (state 6), not a mocked D-Bus reply.
    extension = RUN / 'data/gnome-shell/extensions/tiling-assistant@ubuntu.com'
    extension.mkdir(parents=True)
    (extension / 'metadata.json').write_text(json.dumps({
        'uuid': 'tiling-assistant@ubuntu.com', 'name': 'Inactive TA regression fixture',
        'description': 'Private harness fixture; must never be enabled',
        'shell-version': ['50'], 'session-modes': ['user'], 'version': 99999,
    }))
    (extension / 'extension.js').write_text(
        "export default class InactiveTilingAssistant { enable() { throw new Error('Inactive TA fixture enabled'); } disable() {} }\n")
    (RUN / 'logs/native-snap-fixture.json').write_text(json.dumps({
        'beforeFirstShellStart': True, 'edge': False, 'left': [], 'right': [],
        'ta': 'tiling-assistant@ubuntu.com', 'expectedState': 6,
    }))


def tree_version() -> str:
    """The installed tree's own version (D-G74a, the root Cargo.toml's [workspace.package] version): the release the
    fake GitHub offers (winpoint_bundle)."""
    import tomllib
    return tomllib.loads((SRC / 'Cargo.toml').read_text())['workspace']['package']['version']


def winpoint_running() -> str:
    """The version the run's hosts pretend to be (seam.json running, D-G74): one below the tree's own (0.9.0: 0.8.9,
    as the updater design's harness), so the tree's real bundle is an update they can install."""
    major, minor, patch = (int(x) for x in tree_version().split('-')[0].split('.'))
    if patch:
        return f'{major}.{minor}.{patch - 1}'
    return f'{major}.{minor - 1}.9' if minor else f'{major - 1}.9.9'


def winpoint_bundle() -> Path:
    """--winpoint-feed (D-G74): the release the fake GitHub offers is a real one, made by the project's own packaging
    (scripts/package.py, as tools/release.py makes a release) from this run's build dir and source tree into
    RUN/winpoint-dist: dist/winpoint-V-linux-ARCH.tar.gz and its .sha256. Removed again when the run ends."""
    if WINPOINT_DIST.exists():
        shutil.rmtree(WINPOINT_DIST)
    (WINPOINT_DIST / 'tmp').mkdir(parents=True)
    env = {k: v for k, v in os.environ.items() if k != 'WIN11_NATIVE_BUILD_DIR'}
    env.update(WIN11_BUILD_DIR=str(BUILD), WIN11_PACKAGE_DIR=str(WINPOINT_DIST), WIN11_LANG='en',
               TMPDIR=str(WINPOINT_DIST / 'tmp'), PYTHONDONTWRITEBYTECODE='1')
    t0 = time.monotonic()
    try:
        made = subprocess.run([sys.executable, '-B', str(SRC / 'scripts' / 'package.py'), str(SRC)], env=env,
                              capture_output=True, text=True, timeout=900)
    except subprocess.TimeoutExpired as error:
        raise Fatal(f'scripts/package.py took longer than {error.timeout} s') from error
    shutil.rmtree(WINPOINT_DIST / 'tmp', ignore_errors=True)
    (RUN / 'logs' / 'winpoint-package.log').write_text(made.stdout + made.stderr)
    bundles = sorted(WINPOINT_DIST.glob(f'winpoint-{tree_version()}-linux-*.tar.gz'))
    if made.returncode or len(bundles) != 1:
        raise Fatal(f'scripts/package.py made no bundle (exit {made.returncode}); see logs/winpoint-package.log')
    log(f'WinPoint bundle {bundles[0].name}: {bundles[0].stat().st_size} bytes in {time.monotonic() - t0:.1f} s')
    return bundles[0]


def start_fake_github() -> subprocess.Popen:
    """--winpoint-feed: the seam directory on the sandbox's own /tmp and the fake GitHub on loopback (a child of run,
    outside the strace'd session; it stops with run). Returns once it listens."""
    if WINPOINT_SEAM.exists():
        shutil.rmtree(WINPOINT_SEAM)
    WINPOINT_SEAM.mkdir(mode=0o700)
    os.chmod(WINPOINT_SEAM, 0o700)
    running = winpoint_running()
    bundle = winpoint_bundle()
    # seam.rs: the one tarball this run may install, by its SHA-256 (whoever answers on the port cannot choose it)
    digest = hashlib.sha256()
    with bundle.open('rb') as data:
        for block in iter(lambda: data.read(1 << 20), b''):
            digest.update(block)
    seam = {'origin': f'http://127.0.0.1:{WINPOINT_PORT}/', 'running': running, 'digest': f'sha256:{digest.hexdigest()}'}
    for name, text in ((UPDATE_MARKER, 'GE0c harness: WinPoint update check test seam (D-G74a)\n'),
                       ('seam.json', json.dumps(seam))):
        (WINPOINT_SEAM / name).write_text(text)
        os.chmod(WINPOINT_SEAM / name, 0o644)       # seam.rs: neither group- nor other-writable
    proc = subprocess.Popen([sys.executable, '-B', '-I', str(HERE / 'lib' / 'fake_github.py'), '--port', str(WINPOINT_PORT),
                             '--running', running, '--bundle', str(bundle),
                             '--log', str(RUN / 'logs' / 'fake-github.jsonl')],
                            stdout=subprocess.PIPE, stderr=open(RUN / 'logs' / 'fake-github.log', 'w'), text=True)
    line = proc.stdout.readline().strip() if proc.stdout else ''
    if line != 'ready':
        proc.kill()
        raise Fatal(f'fake GitHub did not start ({line!r}); see logs/fake-github.log')
    log(f'fake GitHub on 127.0.0.1:{WINPOINT_PORT} (running {running}, offers {bundle.name}), seam {WINPOINT_SEAM}')
    return proc


def write_settings(patch: dict, create: bool = False) -> None:
    """Rewrite the private settings.json the way the host does: under .settings.lock, atomic, 0600."""
    config = RUN / 'config' / 'win11-gnome'
    target = config / 'settings.json'
    fd = os.open(config / '.settings.lock', os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW | os.O_CLOEXEC, 0o600)
    try:
        fcntl.flock(fd, fcntl.LOCK_EX)
        data = {} if create or not target.exists() else json.loads(target.read_text() or '{}')
        data.update(patch)
        temporary = config / '.settings.json.ge0c'
        temporary.write_text(json.dumps(data, indent=2, ensure_ascii=False) + '\n')
        os.chmod(temporary, 0o600)
        os.replace(temporary, target)
    finally:
        os.close(fd)


def rewrite_settings_in_place(patch: dict) -> None:
    """GE1h: the other switch path (plan-critique F27). Same inode: O_TRUNC plus one write, under .settings.lock,
    the way an editor or a naive writer saves. Watchers see a change event, not a rename (WATCH_MOVES)."""
    config = RUN / 'config' / 'win11-gnome'
    target = config / 'settings.json'
    lock = os.open(config / '.settings.lock', os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW | os.O_CLOEXEC, 0o600)
    try:
        fcntl.flock(lock, fcntl.LOCK_EX)
        data = json.loads(target.read_text() or '{}')
        data.update(patch)
        payload = (json.dumps(data, indent=2, ensure_ascii=False) + '\n').encode()
        fd = os.open(target, os.O_WRONLY | os.O_TRUNC | os.O_NOFOLLOW | os.O_CLOEXEC)
        try:
            os.write(fd, payload)
        finally:
            os.close(fd)
    finally:
        os.close(lock)


WRITE_SYSCALL = re.compile(r'^(?:\[pid\s+\d+\]\s+)?(\d+\s+)?(openat|open|creat|mkdir|mkdirat|rename|renameat|renameat2|'
                           r'unlink|unlinkat|rmdir|symlink|symlinkat|link|linkat|chmod|fchmodat|fchmodat2|chown|'
                           r'fchownat|lchown|truncate|utimensat|setxattr|lsetxattr)\((.*)$')


QUOTED = r'"((?:[^"\\]|\\.)*)"'


def _strace_paths(line: str) -> tuple[str, list[str]] | None:
    """Locations a mutating file syscall touches, from one ``strace -f -y`` line (None if read-only)."""
    m = WRITE_SYSCALL.match(line)
    if not m:
        return None
    call, rest = m.group(2), m.group(3)
    if call in ('openat', 'open') and not re.search(r'O_WRONLY|O_RDWR|O_CREAT|O_TRUNC', rest):
        return None
    if call in ('symlink', 'symlinkat'):
        rest = re.sub('^' + QUOTED + ', ', '', rest, count=1)        # the link target is content, not a location
    if call in ('setxattr', 'lsetxattr'):
        rest = rest.split(', ', 1)[0]
    # -y prints fds as N</path>; join "N</dir>, "name"" pairs (the *at calls) into absolute paths.
    paths = [name if name.startswith('/') or not fd else f'{fd}/{name}'
             for fd, name in re.findall(r'(?:\d+<([^>]*)>|AT_FDCWD), ' + QUOTED, rest)]
    if not paths:
        paths = re.findall(QUOTED, rest)
    if not paths:
        paths = re.findall(r'\d+<([^>]*)>', rest)                    # e.g. utimensat(fd</path>, NULL, ...)
    return call, paths


def install(env: dict[str, str]) -> dict:
    """scripts/install.py into the private prefix, under strace; every mutated path must be under RUN. --no-session
    when the source's installer knows it: 0.9.1's refuses a run without a terminal otherwise (sessionTerminal), and
    on one would ask the password for the login session's root files; with it, as before, the session counts when
    this machine has it installed."""
    logs = RUN / 'logs'
    trace = logs / 'install.strace'
    script = SRC / 'scripts' / 'install.py'
    argv = ['strace', '-f', '-qq', '-y', '-e', 'trace=%file,connect', '-e', 'signal=none', '-o', str(trace),
            '/usr/bin/python3', '-B', str(script), str(SRC)]
    if '"--no-session"' in script.read_text():
        argv.append('--no-session')
    started = time.monotonic()
    with open(logs / 'install.log', 'w') as out:
        proc = subprocess.run(argv, env=env, stdout=out, stderr=subprocess.STDOUT, cwd=str(RUN), timeout=600)
    seconds = round(time.monotonic() - started, 1)
    outside, writes, failed_writes = [], 0, []
    allowed = ('/dev/null', '/dev/tty', '/proc/self/')
    for line in trace.read_text(errors='replace').splitlines():
        parsed = _strace_paths(line)
        if not parsed:
            continue
        call, paths = parsed
        for path in paths:
            absolute = path if path.startswith('/') else str(RUN / path)   # cwd is RUN
            writes += 1
            if not _inside(absolute, RUN) and not absolute.startswith(allowed):
                (failed_writes if ' = -1 ' in line else outside).append(f'{call} {absolute}')
    prefix, data = RUN / 'prefix', RUN / 'data'
    bins = list(dict.fromkeys(install_bins() + list(NATIVE_BINS)))
    verify = {
        'source': str(SRC), 'buildDir': str(BUILD),
        'exit': proc.returncode, 'seconds': seconds, 'mutatingSyscallPaths': writes,
        'writesOutsideRun': sorted(set(outside)), 'failedWritesOutsideRun': sorted(set(failed_writes)),
        'binaries': {b: (prefix / 'bin' / b).is_file() and sha256(prefix / 'bin' / b) == sha256(BUILD / b) for b in bins},
        'payloadMarker': (prefix / 'share/win11-gnome/.win11-gnome-owned').is_file(),
        'extensionIdentical': {},
        'desktopEntries': sorted(p.name for p in (data / 'applications').glob('io.github.win11shell.*.desktop')),
        # D-G70: neither the WebKit Start and desktop hosts nor their entries are installed any more
        'legacyInstalled': [str(p.relative_to(RUN)) for p in [*(prefix / 'bin' / b for b in LEGACY_BINS),
                            *(data / 'applications' / f'io.github.win11shell.{n}.desktop' for n in ('Start', 'Desktop'))]
                            if p.exists()],
        'desktopExecPrivate': all(str(prefix / 'bin') in p.read_text()
                                  for p in (data / 'applications').glob('io.github.win11shell.*.desktop')),
        'schemasCompiled': (data / 'gnome-shell/extensions' / UUID / 'schemas/gschemas.compiled').is_file(),
        'portalFiles': [str(p.relative_to(RUN)) for p in [data / 'xdg-desktop-portal/portals/win11shell.portal',
                        data / 'dbus-1/services/org.freedesktop.impl.portal.desktop.win11shell.service',
                        RUN / 'config/systemd/user/app-win11-file-portal.service',
                        RUN / 'config/xdg-desktop-portal/gnome-portals.conf'] if p.exists()],
    }
    for name in sorted(os.listdir(SRC / 'extension')):
        source = SRC / 'extension' / name
        if source.is_file():
            copy = data / 'gnome-shell/extensions' / UUID / name
            verify['extensionIdentical'][name] = copy.is_file() and copy.read_bytes() == source.read_bytes()
    verify['extensionCopyProblems'] = extension_copy_problems(data / 'gnome-shell/extensions' / UUID)
    verify['ok'] = (proc.returncode == 0 and not outside and all(verify['binaries'].values())
                    and all(verify['extensionIdentical'].values()) and verify['schemasCompiled']
                    and len(verify['desktopEntries']) == DESKTOP_ENTRIES and verify['desktopExecPrivate']
                    and not verify['legacyInstalled'] and not verify['extensionCopyProblems'])
    return verify


def export_source(rev: str) -> dict:
    """GE1h --source <rev>: the tree of one commit (git archive) in RUN/src, so a run installs a fixed UI,
    extension and installer while other steps edit the working tree. Read-only for the repository."""
    import tarfile
    started = time.monotonic()
    dest = RUN / 'src'
    dest.mkdir()
    commit = subprocess.run(['git', '-C', str(G), 'rev-parse', '--verify', f'{rev}^{{commit}}'],
                            capture_output=True, text=True, check=True).stdout.strip()
    proc = subprocess.Popen(['git', '-C', str(G), 'archive', '--format=tar', commit], stdout=subprocess.PIPE)
    with tarfile.open(fileobj=proc.stdout, mode='r|') as archive:
        archive.extractall(dest, filter='tar')
    if proc.wait() != 0:
        raise RuntimeError(f'git archive {commit} failed ({proc.returncode})')
    return {'rev': rev, 'commit': commit, 'path': str(dest), 'seconds': round(time.monotonic() - started, 1)}


class Sampler(threading.Thread):
    """Memory of every process in the sandbox (own PID namespace), every 0.5 s; aborts when memory runs low."""

    def __init__(self):
        super().__init__(daemon=True)
        self.stop_event = threading.Event()
        self.peak = {'pss_kb': 0, 'rss_kb': 0}
        self.peak_without_harness = {'pss_kb': 0, 'rss_kb': 0}
        self.peak_processes: list = []
        self.seen: dict[str, dict] = {}                  # per process name: the highest PSS one process had, and when
        self.min_available = mem_available_kb()
        self.samples = 0
        self.aborted = False
        self.out = open(RUN / 'logs' / 'memory.tsv', 'w')
        self.out.write('t\tprocesses\tpss_kb\trss_kb\tpss_kb_without_harness\tmem_available_kb\n')
        self.started = time.monotonic()

    def sample(self):
        rows = []
        for pid in proc_table():
            try:
                status = Path(f'/proc/{pid}/status').read_text()
                name = re.search(r'^Name:\s*(.*)$', status, re.M).group(1)
                rss = int((re.search(r'^VmRSS:\s*(\d+)', status, re.M) or [0, 0])[1])
            except (OSError, AttributeError):
                continue
            try:
                pss = int(re.search(r'^Pss:\s*(\d+)', Path(f'/proc/{pid}/smaps_rollup').read_text(), re.M).group(1))
            except (OSError, AttributeError):
                pss = rss
            try:
                cmd = Path(f'/proc/{pid}/cmdline').read_bytes().replace(b'\0', b' ').decode(errors='replace')
            except OSError:
                cmd = ''
            harness = 'harness.py' in cmd or name in ('bwrap', 'strace')
            rows.append({'pid': pid, 'name': name, 'pss_kb': pss, 'rss_kb': rss, 'harness': harness})
        return rows

    def run(self):
        while not self.stop_event.is_set():
            rows = self.sample()
            available = mem_available_kb()
            pss = sum(r['pss_kb'] for r in rows)
            rss = sum(r['rss_kb'] for r in rows)
            pss_wo = sum(r['pss_kb'] for r in rows if not r['harness'])
            rss_wo = sum(r['rss_kb'] for r in rows if not r['harness'])
            self.samples += 1
            self.min_available = min(self.min_available, available)
            for r in rows:
                seen = self.seen.setdefault(r['name'], {'pss_kb': 0, 'first': round(time.monotonic() - self.started, 1)})
                seen['pss_kb'] = max(seen['pss_kb'], r['pss_kb'])
            self.out.write(f'{time.monotonic() - self.started:.1f}\t{len(rows)}\t{pss}\t{rss}\t{pss_wo}\t{available}\n')
            self.out.flush()
            if pss > self.peak['pss_kb']:
                self.peak = {'pss_kb': pss, 'rss_kb': rss, 'processes': len(rows), 't': round(time.monotonic() - self.started, 1)}
                self.peak_processes = sorted(rows, key=lambda r: -r['pss_kb'])
            if pss_wo > self.peak_without_harness['pss_kb']:
                self.peak_without_harness = {'pss_kb': pss_wo, 'rss_kb': rss_wo}
            if available < ABORT_KB and not self.aborted:
                self.aborted = True
                log(f'MEMORY ABORT: MemAvailable {available} kB < {ABORT_KB} kB; stopping the private session')
                stop_pids(descendants(os.getpid()), 'memory abort', grace=3)
            self.stop_event.wait(0.5)

    def summary(self):
        by_name: dict[str, dict] = {}
        for r in self.peak_processes:
            entry = by_name.setdefault(r['name'], {'count': 0, 'pss_kb': 0, 'rss_kb': 0})
            entry['count'] += 1
            entry['pss_kb'] += r['pss_kb']
            entry['rss_kb'] += r['rss_kb']
        return {'samples': self.samples, 'intervalSeconds': 0.5, 'peak': self.peak,
                'peakWithoutHarness': self.peak_without_harness, 'minMemAvailableKb': self.min_available,
                'abortedForMemory': self.aborted,
                'peakByName': dict(sorted(by_name.items(), key=lambda kv: -kv[1]['pss_kb'])),
                'seenByName': dict(sorted(self.seen.items(), key=lambda kv: -kv[1]['pss_kb']))}


class SocketWatch(threading.Thread):
    """Fix round m2: launch masks the host sockets that exist when it starts and the guard checks once, but a
    socket that a host process binds in the checkout, or in a bound host directory, during the run is reachable (a
    read-only bind does not stop connect(); a host program that re-creates a masked socket also unmasks it). So
    this thread scans for socket files every SOCKET_WATCH_SECONDS and calls on_found at the first new one: `run`
    then stops the session and fails (socket-appeared). RUN is skipped: it is shared with the sandbox's own
    processes, and a socket file there cannot be told from a host's without connecting to it. RUN's socket files
    that no process of the sandbox's network namespace had bound when the watch first saw them are recorded
    (runUnlisted) for the evidence, not acted on; launch refuses a host socket in RUN at the start."""

    def __init__(self, on_found, roots: list[Path] | None = None, interval: float = SOCKET_WATCH_SECONDS):
        super().__init__(daemon=True)
        self.stop_event = threading.Event()
        self.on_found = on_found
        self.roots = roots if roots is not None else [G, *host_bind_roots()]
        self.interval = interval
        self.scans, self.max_seconds, self.appeared, self.run_seen = 0, 0.0, [], {}
        self.started = time.monotonic()

    def scan(self) -> list[str]:
        t0 = time.monotonic()
        found = socket_files(self.roots, skip=(RUN,))
        new_in_run = [p for p in socket_files([RUN]) if p not in self.run_seen]
        if new_in_run:
            own = own_socket_paths()
            for path in new_in_run:
                self.run_seen[path] = path in own or os.path.realpath(path) in own
        self.scans += 1
        self.max_seconds = max(self.max_seconds, time.monotonic() - t0)
        return found

    def run(self):
        while not self.stop_event.is_set():
            known = {a['path'] for a in self.appeared}
            found = [p for p in self.scan() if p not in known]
            if found:
                t = round(time.monotonic() - self.started, 1)
                self.appeared += [{'path': p, 't': t} for p in found]
                self.on_found(found)
            self.stop_event.wait(self.interval)

    def summary(self) -> dict:
        return {'intervalSeconds': self.interval, 'scans': self.scans, 'maxScanSeconds': round(self.max_seconds, 3),
                'roots': [str(r) for r in self.roots], 'appeared': self.appeared, 'ok': not self.appeared,
                'runSockets': sorted(os.path.relpath(p, RUN) for p in self.run_seen),
                'runUnlisted': sorted(os.path.relpath(p, RUN) for p, own in self.run_seen.items() if not own)}


def acquire_lock(timeout: float = 3600):
    fd = os.open(LOCK, os.O_RDONLY | os.O_CLOEXEC)
    end = time.monotonic() + timeout
    announced = False
    while True:
        try:
            fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
            return fd
        except BlockingIOError:
            if time.monotonic() > end:
                os.close(fd)
                return None
            if not announced:
                print(f'waiting for {LOCK} (held by another heavy command)', flush=True)
                announced = True
            time.sleep(2)


def scenario_out(args) -> Path:
    if args.out:
        return Path(args.out).resolve()
    if args.scenario:
        # absolute (fix round): launch binds this directory writable, and bwrap needs the absolute path; a relative
        # --scenario without --out left the default <scenario>.out read-only inside the sandbox
        scenario = Path(os.path.realpath(args.scenario))
        return scenario.parent / f'{scenario.stem}.out'
    return BASELINE / args.lang


def winpoint_scenario(args) -> bool:
    """--winpoint-feed with tests/gnome/scenarios/winpoint-update.py: D-G74's exec rule then also requires the
    installer runs the scenario's clicks caused (winpoint_expected_problems), and the run needs --strace."""
    return bool(getattr(args, 'winpoint_feed', False)) and Path(getattr(args, 'scenario', None) or '').name == \
        'winpoint-update.py'


def cmd_run(args) -> int:
    set_build_dir(args.build_dir)
    set_session_mode(args.session_mode)
    set_winpoint_feed(args.winpoint_feed)
    guard_or_exit('outer')
    if os.environ.get('WIN11_BUILD_DIR') != str(BUILD):
        print(f'GUARD REFUSED [build-dir-mismatch] WIN11_BUILD_DIR={os.environ.get("WIN11_BUILD_DIR")!r} '
              f'is not --build-dir {BUILD}', file=sys.stderr)
        return EXIT_REFUSED
    if args.guard_only:
        print('GUARD OK (outer stage: private environment inside the bubblewrap sandbox)')
        return 0
    problems = build_dir_problems(BUILD)
    if args.second_monitor and not second_monitor_ok(args.second_monitor):
        problems.append(('second-monitor-invalid', f'--second-monitor {args.second_monitor} is not WxH between '
                                                   '640x480 and 3840x2160'))
    if args.scenario and not Path(args.scenario).is_file():
        problems.append(('scenario-invalid', f'--scenario {args.scenario} does not exist'))
    if winpoint_scenario(args) and not args.strace:
        problems.append(('winpoint-needs-strace', 'the winpoint-update scenario with --winpoint-feed needs --strace '
                                                  "(D-G74: the exec rule proves the installer ran only after its clicks)"))
    out = scenario_out(args)
    if out_problem(out):
        problems.append(('out-invalid', f'output {out} must be inside {HERE}, outside {RUN}, not {HERE} itself'))
    if problems:
        for code, msg in problems:
            print(f'GUARD REFUSED [{code}] {msg}', file=sys.stderr, flush=True)
        return EXIT_REFUSED
    if args.lock and args.lock.startswith('parent'):
        lock_note = f'held by an ancestor {args.lock.partition(":")[2] or "flock(1)"}'
    elif args.lock and args.lock.startswith('fd:'):
        fd = int(args.lock[3:])
        try:
            same = os.fstat(fd).st_ino == os.stat(LOCK).st_ino and os.fstat(fd).st_dev == os.stat(LOCK).st_dev
        except OSError:
            same = False
        if not same:
            print(f'refused: fd {fd} is not {LOCK}', file=sys.stderr)
            return EXIT_LOCK
        lock_note = f'held by launch (fd {fd})'
    else:
        print('refused: run must be started through "harness.py launch run ..." (heavy lock)', file=sys.stderr)
        return EXIT_LOCK
    available = mem_available_kb()
    if available < MIN_START_KB:
        print(f'refused: MemAvailable {available} kB is below {MIN_START_KB // 1024} MB', file=sys.stderr)
        return EXIT_MEMORY
    set_subreaper()
    started = time.time()
    global SRC
    SRC = G
    prepare(args.lang, args.seed, args.profile_picture)  # wipes RUN; seeds settings from G until --source is in
    if args.native_snap_fixture:
        prepare_native_snap_fixture()
    source = {'rev': 'worktree', 'path': str(G)}
    if args.source != 'worktree':
        source = export_source(args.source)
        SRC = Path(source['path'])
        defaults = json.loads((SRC / 'host/src/settings-defaults.json').read_text())
        if args.seed != 'minimal':
            write_settings({**seed_defaults(defaults), 'language': args.lang}, create=True)
    log(f'GE0c run lang={args.lang} switch={args.switch} seed={args.seed} MemAvailable={available} kB; '
        f'heavy lock {lock_note}; run dir {RUN}; build {BUILD}; source {source["rev"]} '
        f'{source.get("commit", "")[:12]}; scenario {args.scenario or "-"}')
    env = dict(os.environ)
    verify = install(env)
    verify['sourceTree'] = source
    (RUN / 'logs' / 'install-verify.json').write_text(json.dumps(verify, indent=1) + '\n')
    log(f'private install: exit {verify["exit"]} in {verify["seconds"]} s, {verify["mutatingSyscallPaths"]} mutated paths, '
        f'{len(verify["writesOutsideRun"])} outside the run dir, ok={verify["ok"]}')
    if not verify['ok']:
        log('install verification failed; see logs/install.log and logs/install-verify.json')
        return 1
    if args.patch_extension:
        extension = RUN / 'data/gnome-shell/extensions' / UUID / 'extension.js'
        problems = extension_copy_problems(extension.parent)
        if problems:                                   # F24: never patch anything but the private copy
            for code, msg in problems:
                print(f'GUARD REFUSED [{code}] {msg}', file=sys.stderr, flush=True)
            return EXIT_REFUSED
        extension.write_text(extension.read_text().replace('enable() {', 'enable() {\n        global.context.unsafe_mode = true;', 1))
        log('patched the PRIVATE extension copy with unsafe_mode (fallback)')
    # --strace (D-G42): the session under strace (connect(); WU-FIX, D-G51: and every execve, for strace_exec_report);
    # the install has its own. (The GStreamer warm-up the WebKit apps needed, gst-inspect-1.0 under FILE.warm and
    # FILE.warm2, went with them: no native program builds the registry, so no gst-plugin-scanner runs.)
    trace = Path(args.strace) if args.strace else None
    try:
        fake = start_fake_github() if WINPOINT_FEED else None
    except (Fatal, OSError) as error:
        log(f'--winpoint-feed: {error}')
        return 1
    # D-G74: also every new process and thread (clone, fork) for the process tree of the python3 exec rule, and whole
    # argv strings (-s 512) so that rule sees the installer's paths
    traced = lambda name: (['strace', '-f', '-qq', '--seccomp-bpf', '-s', '512', '-e',
                            'trace=connect,execve,execveat,clone,clone3,fork,vfork', '-e',
                            'signal=none', '-o', str(trace.with_name(trace.name + name))] if trace else [])
    sampler = Sampler()
    sampler.start()

    def socket_appeared(paths: list[str]) -> None:
        log(f'SOCKET ABORT: socket file(s) appeared outside the run dir during the run: {", ".join(paths[:5])}; '
            'stopping the private session (fix round m2)')
        stop_pids(descendants(os.getpid()), 'socket watch', grace=3)

    watch = SocketWatch(socket_appeared)
    watch.start()
    inner = [sys.executable, '-B', '-I', str(HERE / 'harness.py'), '_inner', '--lang', args.lang,
             '--build-dir', str(BUILD), '--source', args.source]
    if args.switch:
        inner.append('--switch')
    if args.session_mode:
        inner += ['--session-mode', args.session_mode]
    if args.surfaces:
        inner += ['--surfaces', args.surfaces]
    if args.patch_extension:
        inner.append('--patch-extension')
    if args.winpoint_feed:
        inner.append('--winpoint-feed')
    if args.second_monitor:
        inner += ['--second-monitor', args.second_monitor]
    if args.scenario:
        inner += ['--scenario', str(Path(args.scenario).resolve())]
    argv = traced('') + ['dbus-run-session', f'--config-file={RUN / "session-bus.conf"}', '--'] + inner
    proc = subprocess.Popen(argv, env=env)
    timed_out = False
    try:
        proc.wait(timeout=INNER_TIMEOUT)
    except subprocess.TimeoutExpired:
        timed_out = True
        log(f'inner stage exceeded {INNER_TIMEOUT} s; stopping it')
        stop_pids(descendants(os.getpid()), 'timeout')
        proc.wait()
    sampler.stop_event.set()
    sampler.join(timeout=5)
    watch.stop_event.set()
    watch.join(timeout=5)
    if fake:
        stop_pids([fake.pid], 'fake GitHub', grace=3)
        shutil.rmtree(WINPOINT_DIST, ignore_errors=True)        # the release bundle (about 100 MB): the run's own
    reap()
    leftovers = stop_pids(descendants(os.getpid()), 'final sweep')
    reap()
    memory = sampler.summary()
    try:
        inner_results = json.loads((RUN / 'logs' / 'inner-results.json').read_text())
    except (OSError, ValueError):
        inner_results = {'error': 'inner stage wrote no results'}
    guard_calls = [line for line in (RUN / 'logs' / 'guard-calls.log').read_text().splitlines() if line]
    try:
        update_calls = [line for line in (RUN / 'logs' / 'update-tool-calls.log').read_text().splitlines() if line]
    except OSError:
        update_calls = ['(no update-tool-calls.log)']
    sockets = clean_run_sockets()
    connects = execs = None
    if trace:
        connects = strace_connect_report([RUN / 'logs' / 'install.strace', trace])
        trace.with_name(trace.name + '.json').write_text(json.dumps(connects, indent=1) + '\n')
        for row in connects['byTarget']:
            log(f'strace connect: {row["count"]:6d} x {row["family"]} {row["target"]} -> {row["result"]}')
        log(f'strace connect check: {connects["connects"]} connect() calls, '
            f'{"OK" if connects["ok"] else "FAIL " + "; ".join(connects["violations"][:5])}')
        # WU-FIX (D-G51): no real pkexec, check-new-release, apt-get, apt-cache, do-release-upgrade or loginctl ran;
        # D-G74: with the winpoint-update scenario, exactly the installer runs its clicks caused
        expected = None
        if winpoint_scenario(args):
            records = (inner_results.get('scenario') or {}).get('records') or {}
            expected = records.get('winpoint installer runs') or {}
        execs = strace_exec_report([RUN / 'logs' / 'install.strace', trace], winpoint_expected=expected)
        trace.with_name(trace.name + '.exec.json').write_text(json.dumps(execs, indent=1) + '\n')
        for row in execs['byProgram']:
            log(f'strace exec: {row["count"]:6d} x {row["path"]} ({row["where"]}) -> {row["result"]}')
        log(f'strace exec check: {execs["execs"]} execve() calls, '
            f'{"OK" if execs["ok"] else "FAIL " + "; ".join(execs["violations"][:5])}')
        for row in execs['winpointPython']['execs']:
            log(f'strace exec: WinPoint python3 at line {row["line"]} ({row["mode"] or "?"}, {row["env"]} vars): '
                f'{" ".join(row["argv"][4:6])}')
    scenario = inner_results.get('scenario')
    result = {
        'harness': 'GE0c/GE1h tests/gnome/harness.py', 'lang': args.lang, 'switch': args.switch, 'seed': args.seed,
        'commit': _git_head(), 'startedAt': time.strftime('%Y-%m-%d %H:%M:%S', time.localtime(started)),
        'seconds': round(time.time() - started, 1), 'innerExit': proc.returncode, 'innerTimedOut': timed_out,
        'buildDir': str(BUILD), 'source': source, 'lock': lock_note,
        **({'secondMonitor': args.second_monitor} if args.second_monitor else {}),
        'memory': memory, 'install': verify, 'guardStubCalls': guard_calls, 'updateToolCalls': update_calls,
        'leftoverProcessesAfterInner': leftovers, 'socketCleanup': sockets, 'socketWatch': watch.summary(),
        **({'straceConnect': connects} if connects else {}),
        **({'straceExec': execs} if execs else {}),
        **({'scenario': scenario} if scenario is not None else {}),
        'inner': {k: v for k, v in inner_results.items() if k != 'scenario'},
    }
    # the default baseline/<lang> run puts --switch's shots in baseline/switch; an --out directory keeps them inside
    switch_dir = out / 'switch' if args.out else out.parent / 'switch'
    shots = [s for s in inner_results.get('shots', []) if (RUN / 'shots' / s['file']).is_file()]
    copied = []
    # A scenario keeps all its screenshots next to its results.json; the built-in pass splits off "switch".
    groups = ((out, 'all'),) if args.scenario else ((out, 'surface'), (switch_dir, 'switch'))
    for directory, group in groups:
        mine = [s for s in shots if group == 'all' or (s.get('group') == 'switch') == (group == 'switch')]
        if not mine:
            continue
        directory.mkdir(parents=True, exist_ok=True)
        for png in directory.glob('*.png'):
            png.unlink()
        for shot in mine:
            shutil.copy2(RUN / 'shots' / shot['file'], directory / shot['file'])
            copied.append(str(directory / shot['file']))
    out.mkdir(parents=True, exist_ok=True)
    result['copiedScreenshots'] = [str(Path(c).relative_to(G)) for c in copied]
    (out / 'results.json').write_text(json.dumps(result, indent=1, ensure_ascii=False) + '\n')
    names = ['shell.log', 'harness.log', 'install-verify.json', 'guard-calls.log', 'update-tool-calls.log', 'memory.tsv',
             'fake-github.jsonl', 'fake-github.log']
    if args.scenario:
        names += sorted(p.name for p in (RUN / 'logs').glob('app-*.log'))
    for name in names:
        if (RUN / 'logs' / name).is_file():
            (out / 'logs').mkdir(exist_ok=True)
            shutil.copy2(RUN / 'logs' / name, out / 'logs' / name)
    peak = memory['peak']
    log(f'done in {result["seconds"]} s: inner exit {proc.returncode}, peak PSS {peak.get("pss_kb", 0) / 1024:.0f} MiB '
        f'(RSS {peak.get("rss_kb", 0) / 1024:.0f} MiB, {peak.get("processes", 0)} processes), '
        f'min MemAvailable {memory["minMemAvailableKb"] / 1024:.0f} MiB, {len(copied)} screenshots -> {out}')
    if scenario is not None:
        log(f'scenario: error {scenario.get("error")}, {len(scenario.get("checks", []))} checks, failed '
            f'{len(scenario.get("failedChecks", []))} (not marked known: {len(scenario.get("failedChecksUnknown", []))}), '
            f'{len(scenario.get("shots", []))} screenshots')
    log(f'socket cleanup: {len(sockets["removed"])} socket files removed from the run dir, '
        f'{len(sockets["left"])} left in the checkout {sockets["left"][:3]}; update tool calls {len(update_calls)}')
    watched = watch.summary()
    log(f'socket watch: {watched["scans"]} scans every {watched["intervalSeconds"]} s (longest '
        f'{watched["maxScanSeconds"]} s), {len(watched["appeared"])} appeared {[a["path"] for a in watched["appeared"]][:3]}; '
        f'run dir sockets {watched["runSockets"]}, not bound inside when first seen {watched["runUnlisted"]}')
    if sampler.aborted or not watched['ok']:
        return EXIT_ABORTED
    failures = [s for s in inner_results.get('surfaces', []) if not s.get('ok')]
    if scenario is not None and scenario.get('error'):
        failures.append('scenario')
    if not sockets['ok']:
        failures.append('socket-cleanup')
    if connects and not connects['ok']:
        failures.append('strace-connect')
    if execs and not execs['ok']:
        failures.append('strace-exec')
    return 0 if proc.returncode == 0 and not failures and not timed_out else 1


# --------------------------------------------------------------------------------------------------------
# _inner (inside dbus-run-session): private system bus, gnome-shell, surfaces, screenshots, shutdown
# --------------------------------------------------------------------------------------------------------

class Session:
    def __init__(self, args):
        import gi
        gi.require_version('Gio', '2.0')
        gi.require_version('GdkPixbuf', '2.0')
        from gi.repository import Gio, GLib, GdkPixbuf
        self.Gio, self.GLib, self.GdkPixbuf = Gio, GLib, GdkPixbuf
        self.args = args
        self.lang = args.lang
        self.apps: list[subprocess.Popen] = []
        self.shots: list[dict] = []
        self.surfaces: list[dict] = []
        self.attention: list[dict] = []
        self.counter = 0
        self.logs = RUN / 'logs'
        self.report = self.logs / 'report.jsonl'
        self.command = self.logs / 'command.json'
        self._titles: dict | None = None
        self.dutch = dutch_words()
        self.sysbus = None
        self.shell = None
        self.bus = None
        self.facts: dict = {}
        self.scenario: dict | None = None

    # ---- processes -------------------------------------------------------------------------------------
    def start(self):
        env = dict(os.environ)
        sys_log = open(self.logs / 'system-bus.log', 'w')
        self.sysbus = subprocess.Popen(['dbus-daemon', f'--config-file={RUN / "system-bus.conf"}', '--nofork',
                                        '--nopidfile', '--print-address=1'], env=env, stdout=subprocess.PIPE, stderr=sys_log)
        address = self.sysbus.stdout.readline().decode().strip()
        if not address.startswith(f'unix:path={RT}/sbus'):
            raise RuntimeError(f'private system bus reported {address!r}')
        self.facts['systemBus'] = address
        argv = ['gnome-shell', '--headless', f'--virtual-monitor={WIDTH}x{HEIGHT}', f'--wayland-display={WL}', '--no-x11']
        if self.args.second_monitor:
            argv.insert(3, f'--virtual-monitor={self.args.second_monitor}')
        if SESSION_MODE:
            argv.append(f'--mode={SESSION_MODE}')
        if not self.args.patch_extension:
            argv.insert(2, '--unsafe-mode')
        self.facts['shellArgv'] = argv
        self.shell = subprocess.Popen(argv, env=env, stdout=open(self.logs / 'shell.log', 'w'), stderr=subprocess.STDOUT)
        self.bus = self.Gio.bus_get_sync(self.Gio.BusType.SESSION, None)
        until(lambda: self._shell_alive() and self.ev('true') is True, 'GNOME Shell answers Eval on the private bus', 90, 0.3)
        self.facts['unsafeMode'] = self.ev('global.context.unsafe_mode')
        self.facts['shellVersion'] = self.ev("(await import('resource:///org/gnome/shell/misc/config.js')).PACKAGE_VERSION")
        self.facts['sessionMode'] = self.ev('Main.sessionMode.currentMode')
        until(lambda: self.ev('!Main.layoutManager._startingUp'), 'GNOME Shell startup complete', 90, 0.3)
        self.facts['monitors'] = self.ev('Main.layoutManager.monitors.map(m=>({index:m.index,x:m.x,y:m.y,w:m.width,'
                                         'h:m.height,primary:m.index===Main.layoutManager.primaryIndex}))')
        self.ev('Main.overview.hide()')
        until(lambda: self.ev('!Main.overview.visible && !Main.overview.animationInProgress'), 'overview hidden', 30, 0.2)

    def _shell_alive(self):
        if self.shell.poll() is not None:
            raise Fatal(f'gnome-shell exited with {self.shell.returncode}; see logs/shell.log')
        return True

    def shutdown(self):
        try:
            self.command.write_text('')
        except OSError:
            pass
        for proc in self.apps:
            if proc.poll() is None:
                stop_pids([proc.pid] + descendants(proc.pid), 'shutdown app')
        if self.shell and self.shell.poll() is None:
            hosts = [p for p in descendants(self.shell.pid)]
            stop_pids(hosts, 'shutdown shell clients', grace=6)
            stop_pids([self.shell.pid], 'shutdown gnome-shell', grace=15)
        if self.sysbus and self.sysbus.poll() is None:
            stop_pids([self.sysbus.pid], 'shutdown system bus', grace=5)
        reap()
        stop_pids(descendants(os.getpid()), 'inner sweep')
        reap()

    @property
    def titles(self) -> dict[str, dict[str, str]]:
        """The Settings page titles (settings_titles), read once, when a run first needs them."""
        if self._titles is None:
            self._titles = settings_titles()
        return self._titles

    # ---- D-Bus -----------------------------------------------------------------------------------------
    def ev(self, code: str):
        GLib, Gio = self.GLib, self.Gio
        wrapped = ("(async()=>{const Main=await import('resource:///org/gnome/shell/ui/main.js');"
                   "const Meta=(await import('gi://Meta')).default;return " + code + ";})()")
        ok, out = self.bus.call_sync('org.gnome.Shell', '/org/gnome/Shell', 'org.gnome.Shell', 'Eval',
                                     GLib.Variant('(s)', (wrapped,)), None, Gio.DBusCallFlags.NONE, 15000, None).unpack()
        if not ok:
            raise RuntimeError(f'Eval failed: {out[:300]}')
        return json.loads(out) if out else None

    def screenshot(self, name: str, group: str = 'surface', area=None) -> dict:
        GLib, Gio = self.GLib, self.Gio
        path = RUN / 'shots' / f'{name}.png'
        if area:
            x, y, w, h = area
            ok, used = self.bus.call_sync('org.gnome.Shell', '/org/gnome/Shell/Screenshot', 'org.gnome.Shell.Screenshot',
                                          'ScreenshotArea', GLib.Variant('(iiiibs)', (x, y, w, h, False, str(path))),
                                          None, Gio.DBusCallFlags.NONE, 30000, None).unpack()
        else:
            ok, used = self.bus.call_sync('org.gnome.Shell', '/org/gnome/Shell/Screenshot', 'org.gnome.Shell.Screenshot',
                                          'Screenshot', GLib.Variant('(bbs)', (False, False, str(path))),
                                          None, Gio.DBusCallFlags.NONE, 30000, None).unpack()
        if not ok or not path.is_file():
            raise RuntimeError(f'screenshot {name} failed ({used})')
        shot = {'file': path.name, 'group': group, 'bytes': path.stat().st_size, 'area': area}
        self.shots.append(shot)
        return shot

    def pixel_stats(self, file: str, rect=None) -> dict:
        pix = self.GdkPixbuf.Pixbuf.new_from_file(str(RUN / 'shots' / file))
        data, stride, channels = pix.get_pixels(), pix.get_rowstride(), pix.get_n_channels()
        width, height = pix.get_width(), pix.get_height()
        x0, y0, w, h = rect or (0, 0, width, height)
        x0, y0 = max(0, x0), max(0, y0)
        x1, y1 = min(width, x0 + w), min(height, y0 + h)
        colours, total, count = set(), 0, 0
        for y in range(y0, y1, 5):
            row = y * stride
            for x in range(x0, x1, 5):
                i = row + x * channels
                r, g, b = data[i], data[i + 1], data[i + 2]
                colours.add((r >> 2, g >> 2, b >> 2))
                total += r + g + b
                count += 1
        return {'rect': [x0, y0, x1 - x0, y1 - y0], 'distinctColours': len(colours),
                'meanLevel': round(total / max(count, 1) / 3, 1), 'looksBlank': len(colours) <= 4}

    # ---- dev hooks: the native shell and the native apps share WIN11_DEV_COMMAND/WIN11_DEV_REPORT ---------------
    def rows(self) -> list[dict]:
        out = []
        for line in self.report.read_text(errors='replace').splitlines():
            try:
                out.append(json.loads(line))
            except ValueError:
                pass
        return out

    def native_boot(self, after: int = 0, timeout: float = 90) -> dict:
        """The boot row win11-shell writes once its dev channel is up (native/shell/src/dev.rs):
        {"event":"test","windowId":"native","data":{"tag":"boot","kind":"native-shell","pid":N}}, looked for in the
        report rows from index ``after`` on (every start of the shell appends one; its windowId is always "native")."""
        try:
            return until(lambda: next((r['data'] for r in self.rows()[after:] if r.get('event') == 'test'
                                       and r.get('windowId') == 'native' and r['data'].get('tag') == 'boot'
                                       and r['data'].get('kind') == 'native-shell'), None),
                         'native shell (win11-shell) boot report', timeout, 0.2)
        except TimeoutError as error:
            hosts = sorted(f'{pid}:{row["comm"]}' for pid, row in proc_table().items() if row['comm'].startswith('win11-'))
            self.facts['nativeBootMissing'] = {'win11Processes': hosts}
            raise TimeoutError(f'{error}; win11 processes: {hosts}') from error

    def native(self, command: dict, timeout: float = 15):
        """One dev command for win11-shell ({"native": "state"|"click"|"flyout", ...}); returns its report's value.
        The shell takes only commands with a "native" key and no "app" key (native_app sends an app's)."""
        self.counter += 1
        tag = f'native-{self.counter}'
        try:
            until(lambda: self.command.read_text() == '', 'previous dev command consumed', 10, 0.05)
        except TimeoutError:
            self.command.write_text('')
        self.command.write_text(json.dumps({**command, 'tag': tag}))
        try:
            row = until(lambda: next((r['data'] for r in self.rows() if r.get('event') == 'test'
                                      and r.get('windowId') == 'native' and r['data'].get('tag') == tag), None),
                        f'native {command.get("native")} {tag}', timeout, 0.05)
        except TimeoutError:
            self.command.write_text('')
            raise
        return row.get('value')

    def native_state(self) -> list[dict]:
        """win11-shell's toplevels: title, visible, size and the widget tree (name, classes, label, rect)."""
        value = self.native({'native': 'state'})
        return value if isinstance(value, list) else []

    def native_app_boot(self, app: str, after: int = 0, timeout: float = 60) -> dict:
        """The boot row of a native GTK 4 app (native/kit dev.rs): windowId "native-<app>", kind "native-app"."""
        return until(lambda: next((r['data'] for r in self.rows()[after:] if r.get('event') == 'test'
                                   and r.get('windowId') == f'native-{app}' and r['data'].get('tag') == 'boot'), None),
                     f'native app {app} boot report', timeout, 0.2)

    def native_app(self, app: str, command: dict, timeout: float = 15):
        """One dev command for a native app ({"native": "state"|"click"|"type"|"key", "app": app, ...}); the shell
        skips commands with an "app" key, each app takes only its own."""
        self.counter += 1
        tag = f'native-{app}-{self.counter}'
        try:
            until(lambda: self.command.read_text() == '', 'previous dev command consumed', 10, 0.05)
        except TimeoutError:
            self.command.write_text('')
        self.command.write_text(json.dumps({**command, 'app': app, 'tag': tag}))
        try:
            row = until(lambda: next((r['data'] for r in self.rows() if r.get('event') == 'test'
                                      and r.get('windowId') == f'native-{app}' and r['data'].get('tag') == tag), None),
                        f'native {app} {command.get("native")} {tag}', timeout, 0.05)
        except TimeoutError:
            self.command.write_text('')
            raise
        return row.get('value')

    def native_texts(self, tops: list[dict] | None = None, visible_only: bool = True) -> dict:
        """The labels and tooltips of the (visible) toplevels of a dev state answer, the shell's unless ``tops`` is
        given (an app's: app_texts), with Dutch-word hits."""
        tops = self.native_state() if tops is None else tops
        labels: list[str] = []

        def walk(node):
            if isinstance(node, dict):
                if node.get('label'):
                    labels.append(str(node['label']))
                for child in node.get('children') or []:
                    walk(child)
        windows = []
        for top in tops:
            if visible_only and not top.get('visible'):
                continue
            before = len(labels)
            walk(top.get('tree'))
            windows.append({'title': top.get('title'), 'width': top.get('width'), 'height': top.get('height'),
                            'labels': len(labels) - before})
        labels = list(dict.fromkeys(labels))
        tokens = set(re.findall(r"[a-zà-ÿ]+(?:-[a-zà-ÿ]+)*", '\n'.join(labels).lower()))
        return {'windows': windows, 'labels': labels[:400], 'dutchWordHits': sorted(t for t in tokens if t in self.dutch)}

    def app_texts(self, app: str, tops: list[dict] | None = None) -> dict:
        """A native app's labels and tooltips with Dutch-word hits (native_texts), its window title and its
        language (app_lang): what an HTML app's page text and <html lang> were."""
        tops = self.native_app(app, {'native': 'state'}, 20) if tops is None else tops
        out = self.native_texts(tops)
        out['title'] = next((t.get('title') for t in tops or [] if t.get('visible')), None)
        out['lang'] = app_lang(tops, app)
        return out

    WINDOWS = ("global.get_window_actors().map(a=>{const w=a.meta_window,r=w.get_frame_rect();return{wm:w.get_wm_class(),"
               "title:w.get_title(),pid:w.get_pid(),x:r.x,y:r.y,w:r.width,h:r.height,visible:a.visible,"
               "minimized:w.minimized,focus:global.display.focus_window===w};})")

    def windows(self) -> list[dict]:
        return self.ev(self.WINDOWS)

    NOTIFICATIONS = ("Main.messageTray.getSources().map(s=>({source:s.title,notifications:(s.notifications||[])"
                     ".map(n=>({title:n.title,body:n.body}))}))")

    def notifications(self) -> list:
        try:
            return self.ev(self.NOTIFICATIONS)
        except Exception as error:   # noqa: BLE001
            return [f'unavailable: {error}']

    def window_of(self, wm_class: str, pid: int | None = None) -> dict | None:
        return next((w for w in self.windows() if w['wm'] == wm_class and w['visible'] and not w['minimized']
                     and (pid is None or w['pid'] == pid)), None)

    # ---- one surface -----------------------------------------------------------------------------------
    def surface(self, name: str, action, group: str = 'surface'):
        started = time.monotonic()
        entry = {'name': name, 'group': group, 'lang': self.lang, 'ok': False}
        log(f'surface {name}')
        try:
            self._shell_alive()
            entry.update(action() or {})
            entry['ok'] = 'error' not in entry
        except Exception as error:   # noqa: BLE001 - record, take a failure shot, carry on
            entry['error'] = f'{type(error).__name__}: {error}'
            log(f'  FAILED {name}: {entry["error"]}')
            try:
                entry['failureShot'] = self.screenshot(f'{name}-FAILED', group)['file']
            except Exception as shot_error:   # noqa: BLE001
                entry['failureShot'] = f'none ({shot_error})'
        entry['seconds'] = round(time.monotonic() - started, 1)
        try:
            entry['windows'] = self.windows()
        except Exception:   # noqa: BLE001
            pass
        self.surfaces.append(entry)
        return entry

    def finish_shot(self, name: str, app: str | None, rect=None, group: str = 'surface', settle: float = 2.0):
        """A screenshot with its pixel statistics, the windows GNOME sees and, for an app, its dev state's text and
        language (app_texts)."""
        time.sleep(settle)
        shot = self.screenshot(name, group)
        out = {'png': shot['file'], 'pixels': self.pixel_stats(shot['file'], rect),
               'windowsAtShot': self.windows()}           # native (Meta) titles, as the overview/Alt+Tab see them
        if app:
            out['text'] = self.app_texts(app)
            out['langMatches'] = out['text'].get('lang') == self.lang
        return out

    def launch(self, argv: list[str], kind: str, name: str, env: dict | None = None):
        """Start an app through its launcher (win11-explorer, win11-app --app <kind>, ...: the host execs
        win11-gtk-<kind> in place, native/APPS.md), wait for the app's boot row on the kit's dev channel and for its
        window with a size, and focus it like a launch from Start; returns the process and the boot row (whose pid
        is the app's)."""
        rows = len(self.rows())
        env = dict(os.environ, WAYLAND_DISPLAY=WL, GDK_BACKEND='wayland', **(env or {}))
        log_file = open(self.logs / f'app-{name}.log', 'a')
        proc = subprocess.Popen(argv, env=env, stdout=log_file, stderr=subprocess.STDOUT)
        self.apps.append(proc)
        boot = self.native_app_boot(kind, rows)
        pid = boot.get('pid') or proc.pid
        until(lambda: (lambda w: w if w and w['w'] > 0 else None)(self.window_of_pid(pid)), f'{name} window mapped',
              30, 0.2)
        self.focus(pid, name)
        return proc, boot

    ATTENTION = ("Main.messageTray.getSources().flatMap(s=>(s.notifications||[]).filter(n=>/ is ready$/.test(n.body||''))"
                 ".map(n=>({source:s.title,title:n.title,body:n.body})))")

    def focus(self, pid: int, name: str):
        """A process started without an activation token maps unfocused and GNOME posts '“<title>” is ready'.
        Record that text (it shows the native window title GNOME saw first), then focus the window like a
        launch from Start or the taskbar would, and drop the banner so it does not cover the screenshot."""
        time.sleep(0.3)
        seen = self.ev(self.ATTENTION)
        if seen:
            self.attention.extend({'app': name, **row} for row in seen)
        self.ev(f'(()=>{{const w=global.get_window_actors().map(a=>a.meta_window).find(w=>w.get_pid()==={pid});'
                'if(w)Main.activateWindow(w);return Boolean(w);})()')
        until(lambda: self.ev(f'global.display.focus_window?.get_pid()==={pid}'), f'{name} focused', 10, 0.2)
        self.ev("Main.messageTray.getSources().forEach(s=>(s.notifications||[]).filter(n=>/ is ready$/.test(n.body||''))"
                ".forEach(n=>n.destroy()))||true")

    def window_of_pid(self, pid: int):
        return next((w for w in self.windows() if w['pid'] == pid and w['visible'] and not w['minimized']), None)

    def close(self, proc: subprocess.Popen, name: str):
        """Close like a user (delete the window), then make sure the process tree is gone."""
        try:
            self.ev(f'global.get_window_actors().map(a=>a.meta_window).filter(w=>w.get_pid()==={proc.pid})'
                    '.forEach(w=>w.delete(global.get_current_time()))||true')
            proc.wait(timeout=10)
        except Exception:   # noqa: BLE001
            pass
        if proc.poll() is None:
            stop_pids([proc.pid] + descendants(proc.pid), f'close {name}')
        try:
            proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            pass
        time.sleep(0.5)

    # ---- the scenario ----------------------------------------------------------------------------------
    def start_flyout(self, want_open: bool, pid: int | None = None):
        """Open or close the native Start (a window of win11-shell, ``pid``) with the extension's _toggleStart, as the
        taskbar button and the Super key do; if it does not close, a click on the shell's own tb-start button."""
        pid = pid or self.facts.get('windowIds', {}).get('native')
        visible = lambda: bool(self.window_of('io.github.win11shell.Start', pid))
        if visible() == want_open:
            return None
        result = self.ev(f"Main.extensionManager.lookup('{UUID}').stateObj._panelBridge._toggleStart()")
        try:
            until(lambda: visible() == want_open, f'Start {"shown" if want_open else "hidden"}', 15, 0.2)
        except TimeoutError:
            if want_open:
                raise
            self.native({'native': 'click', 'name': 'tb-start'})
            until(lambda: not visible(), 'Start hidden after a click on tb-start', 15, 0.2)
        return result

    def settings_page(self, page: str):
        """Show a page in the running Settings as Start's settings search does: one more launch with --params
        {"page": ...}, which GApplication hands to the running app (that launch then ends); then its title."""
        argv = [str(RUN / 'prefix/bin/win11-app'), '--app', 'settings', '--params', json.dumps({'page': page})]
        relay = subprocess.Popen(argv, env=dict(os.environ, WAYLAND_DISPLAY=WL, GDK_BACKEND='wayland'),
                                 stdout=open(self.logs / 'app-settings-relay.log', 'a'), stderr=subprocess.STDOUT)
        self.apps.append(relay)
        t0 = time.monotonic()
        try:
            relay.wait(timeout=20)
        except subprocess.TimeoutExpired:
            pass
        self.facts.setdefault('settingsRelays', []).append(
            {'page': page, 'exit': relay.poll(), 'seconds': round(time.monotonic() - t0, 2)})
        return self.settings_title(page)

    def settings_title(self, page: str, lang: str | None = None):
        """Wait until Settings' header shows the page's title (registry.rs) in ``lang``: the `st-title` label of a
        top-level page, or the last item of the "Parent › Page" breadcrumb that carries `st-title` for a sub-page."""
        title = self.titles.get(page, {}).get(lang or self.lang)
        if not title:
            time.sleep(2)
            return None

        def shown():
            heads = [n for n in tree_nodes(self.native_app('settings', {'native': 'state'}, 20))
                     if 'st-title' in (n.get('classes') or [])]
            return title if any(title == n.get('label') for head in heads for n in tree_nodes([{'tree': head}])) else None
        return until(shown, f'Settings page {page} title {title!r}', 20, 0.3)

    def run(self):
        wanted = set(self.args.surfaces.split(',')) if self.args.surfaces else None
        want = lambda name: wanted is None or name in wanted
        self.start()
        log(f'GNOME Shell {self.facts["shellVersion"]} up (mode {self.facts["sessionMode"]}, unsafe {self.facts["unsafeMode"]})')
        extension = until(lambda: (lambda e: e if e and e.get('active') else None)(self.ev(
            "(async()=>{const {ExtensionState}=await import('resource:///org/gnome/shell/misc/extensionUtils.js');"
            f"const e=Main.extensionManager.lookup('{UUID}');"
            "return e?{state:e.state,active:e.state===(ExtensionState.ACTIVE??ExtensionState.ENABLED),"
            "path:e.path,error:String(e.error||'')}:null;})()")),
            'Win11 extension active', 30, 0.3)
        self.facts['extension'] = extension
        self.facts['extensionCopyIdentical'] = not self.args.patch_extension
        problems = extension_copy_problems(extension.get('path'))
        if problems:                                    # F24: the shell must have loaded the private copy
            raise Fatal('; '.join(msg for _, msg in problems))
        # D-G70: the extension starts win11-panel, which execs win11-shell (taskbar, flyouts, Start and desktop in one
        # GTK 4 process; its backend is a child, win11-panel --http-only). No WebKit panel, Start or desktop.
        boot = self.native_boot(0, 90)
        pid = boot.get('pid')
        self.facts['nativeBoot'] = boot
        until(lambda: self.window_of('io.github.win11shell.Panel', pid), 'native Panel window mapped', 30, 0.2)
        self.facts['windowIds'] = {'native': pid}
        time.sleep(3)
        self.facts['notificationsAtStart'] = self.notifications()
        # Harness artifact: no GDM on the private system bus -> GNOME's own "Screen Lock disabled" notice,
        # which the Win11 notification bridge counts as "1" on the taskbar. Dismiss it, keep the record.
        self.ev("Main.messageTray.getSources().forEach(s=>(s.notifications||[]).filter(n=>n.title==='Screen Lock disabled')"
                ".forEach(n=>n.destroy()))||true")
        time.sleep(1)
        self.facts['notificationsAfterDismiss'] = self.notifications()

        if self.args.scenario:
            # GE1h: a scenario replaces the built-in pass (unless --surfaces asks for both, built-in first).
            if self.args.surfaces:
                self.builtin_surfaces(want, pid)
            scenario_lib = load_lib('scenario')
            self.scenario = scenario_lib.run_scenario(self, sys.modules[__name__], Path(self.args.scenario))
            self.facts['notificationsAtEnd'] = self.notifications()
            return
        self.builtin_surfaces(want, pid)
        if self.args.switch:
            self.live_switch(pid)
        self.facts['notificationsAtEnd'] = self.notifications()

    def native_shot(self, name: str, rect, group: str = 'surface', settle: float = 2.0) -> dict:
        """A screenshot of a native shell surface with its pixel statistics, the windows GNOME sees and the shell's
        dev state (labels, Dutch-word hits): the native counterpart of finish_shot()."""
        time.sleep(settle)
        shot = self.screenshot(name, group, area=rect)
        return {'png': shot['file'], 'pixels': self.pixel_stats(shot['file']), 'windowsAtShot': self.windows(),
                'native': self.native_texts()}

    def builtin_surfaces(self, want, pid: int):
        if want('desktop'):
            def desktop_shot():
                if not self.window_of('io.github.win11shell.Desktop', pid):
                    raise RuntimeError(f'no Desktop window of win11-shell (pid {pid})')
                return self.native_shot('desktop', (0, 0, WIDTH, HEIGHT - TASKBAR))
            self.surface('desktop', desktop_shot)
        if want('panel'):
            def panel_shot():
                return self.native_shot('panel', (0, HEIGHT - TASKBAR, WIDTH, TASKBAR), settle=0.5)
            self.surface('panel', panel_shot)
        if want('start'):
            def start_shot():
                self.start_flyout(True, pid)
                window = self.window_of('io.github.win11shell.Start', pid)
                out = self.native_shot('start', (window['x'], window['y'], window['w'], window['h']), settle=1.5)
                self.start_flyout(False, pid)
                return out
            self.surface('start', start_shot)
        if want('explorer'):
            def explorer_shot():
                proc, boot = self.launch([str(RUN / 'prefix/bin/win11-explorer'), str(RUN / 'home')], 'explorer', 'explorer')
                w = self.window_of_pid(boot['pid'])
                out = self.finish_shot('explorer', 'explorer', (w['x'], w['y'], w['w'], w['h']), settle=2.5)
                self.close(proc, 'explorer')
                return out
            self.surface('explorer', explorer_shot)
        pages = [p for p in SETTINGS_PAGES if want('settings') or want(f'settings-{p}')]
        if pages:
            proc = boot = None
            for page in pages:
                def settings_shot(page=page):
                    nonlocal proc, boot
                    if proc is None:
                        proc, boot = self.launch([str(RUN / 'prefix/bin/win11-app'), '--app', 'settings', '--params',
                                                  json.dumps({'page': page})], 'settings', 'settings')
                        self.settings_title(page)
                    else:
                        self.settings_page(page)
                    w = self.window_of_pid(boot['pid'])
                    return self.finish_shot(f'settings-{page}', 'settings', (w['x'], w['y'], w['w'], w['h']), settle=2.0)
                self.surface(f'settings-{page}', settings_shot)
            if proc:
                self.close(proc, 'settings')
        for app, label in APPS:
            if want(app):
                def app_shot(app=app):
                    proc, boot = self.launch([str(RUN / 'prefix/bin/win11-app'), '--app', app], app, app)
                    w = self.window_of_pid(boot['pid'])
                    out = self.finish_shot(app, app, (w['x'], w['y'], w['w'], w['h']), settle=3.0)
                    self.close(proc, app)
                    return out
                self.surface(app, app_shot)

    def live_switch(self, pid: int):
        """Settings (Taal en regio) and the native Start open, then settings.json rewritten to the other language:
        Settings' language (app_lang), its page title and the shell's labels are timed (the shell has no language
        marker: the first moment its labels differ from before counts), with screenshots."""
        target = 'en' if self.lang == 'nl' else 'nl'
        state: dict = {}
        start_rect = lambda: (lambda w: (w['x'], w['y'], w['w'], w['h']) if w else None)(  # noqa: E731
            self.window_of('io.github.win11shell.Start', pid))

        def before():
            proc, boot = self.launch([str(RUN / 'prefix/bin/win11-app'), '--app', 'settings', '--params',
                                      json.dumps({'page': 'language'})], 'settings', 'settings-switch')
            state.update(proc=proc, pid=boot['pid'])
            self.settings_title('language')
            time.sleep(1.5)
            self.start_flyout(True, pid)
            out = self.native_shot(f'switch-1-before-{self.lang}', None, group='switch', settle=1.5)
            state['labels'] = out['native']['labels']
            out['settingsText'] = self.app_texts('settings')
            return out
        self.surface('switch-before', before, 'switch')
        if 'pid' not in state:
            return

        def after():
            t0 = time.monotonic()
            write_settings({'language': target})
            timings = {}
            try:
                until(lambda: app_lang(self.native_app('settings', {'native': 'state'}, 5), 'settings') == target,
                      f'settings lang={target}', 20, 0.2)
                timings['settings'] = round(time.monotonic() - t0, 2)
            except Exception as error:   # noqa: BLE001
                timings['settings'] = f'not switched: {error}'
            try:
                until(lambda: self.native_texts()['labels'] != state.get('labels'), 'native shell labels changed', 20, 0.3)
                timings['native'] = round(time.monotonic() - t0, 2)
            except Exception as error:   # noqa: BLE001
                timings['native'] = f'not switched: {error}'
            try:
                self.settings_title('language', target)
                timings['settingsTitle'] = round(time.monotonic() - t0, 2)
            except Exception as error:   # noqa: BLE001
                timings['settingsTitle'] = f'not switched: {error}'
            self.lang = target
            out = self.native_shot(f'switch-2-after-{target}-start-open', None, group='switch', settle=2.0)
            out['startRect'] = start_rect()
            out['switchSeconds'] = timings
            out['settingsText'] = self.app_texts('settings')
            return out
        self.surface('switch-after', after, 'switch')

        def settings_only():
            self.start_flyout(False, pid)
            w = self.window_of_pid(state['pid'])
            out = self.finish_shot(f'switch-3-after-{target}-settings', 'settings',
                                   (w['x'], w['y'], w['w'], w['h']) if w else None, group='switch', settle=1.5)
            panel_shot = self.screenshot(f'switch-4-after-{target}-panel', 'switch', area=(0, HEIGHT - TASKBAR, WIDTH, TASKBAR))
            out['panelPng'] = panel_shot['file']
            out['native'] = self.native_texts()
            return out
        self.surface('switch-after-settings', settings_only, 'switch')
        self.close(state['proc'], 'settings-switch')

    def results(self) -> dict:
        out = {'facts': self.facts, 'surfaces': self.surfaces, 'shots': self.shots, 'attentionNotifications': self.attention}
        if self.args.scenario:
            out['scenario'] = self.scenario or {'error': 'the scenario did not run (see facts.fatal)'}
        return out


def load_lib(name: str):
    """tests/gnome/lib/<name>.py by path (python -I keeps the script's directory off sys.path)."""
    import importlib.util
    module_name = f'ge1h_lib_{name}'
    if module_name in sys.modules:
        return sys.modules[module_name]
    spec = importlib.util.spec_from_file_location(module_name, LIB / f'{name}.py')
    module = importlib.util.module_from_spec(spec)
    sys.modules[module_name] = module
    spec.loader.exec_module(module)
    return module


def cmd_inner(args) -> int:
    global SRC
    set_build_dir(args.build_dir)
    set_session_mode(args.session_mode)
    set_winpoint_feed(args.winpoint_feed)
    SRC = G if args.source == 'worktree' else RUN / 'src'
    guard_or_exit('inner')
    set_subreaper()
    session = Session(args)
    status = 0
    try:
        session.run()
    except Exception as error:   # noqa: BLE001
        status = 1
        session.facts['fatal'] = f'{type(error).__name__}: {error}'
        log(f'FATAL: {session.facts["fatal"]}')
        try:
            if session.shell and session.shell.poll() is None:
                session.screenshot('FATAL')
        except Exception:   # noqa: BLE001
            pass
    finally:
        try:
            session.facts['shellAliveAtEnd'] = bool(session.shell and session.shell.poll() is None)
        except Exception:   # noqa: BLE001
            pass
        session.shutdown()
        (RUN / 'logs' / 'inner-results.json').write_text(json.dumps(session.results(), indent=1, ensure_ascii=False) + '\n')
    failed = [s['name'] for s in session.surfaces if not s.get('ok')]
    if args.scenario and (not session.scenario or session.scenario.get('error')):
        failed.append('scenario')
    log(f'inner done: {len(session.surfaces)} surfaces, {len(failed)} failed {failed}')
    return status or (1 if failed else 0)


def root_mount() -> dict:
    """The mount at / (from /proc/self/mountinfo, as services/storage/volumes.rs reads it): file system type,
    source, read-only, and whether it is the checkout's device (fix round M2)."""
    for line in Path('/proc/self/mountinfo').read_text().splitlines():
        fields = line.split(' ')
        if len(fields) > 6 and fields[4] == '/' and '-' in fields:
            sep = fields.index('-')
            return {'fsType': fields[sep + 1], 'source': fields[sep + 2], 'majMin': fields[2], 'root': fields[3],
                    'readOnly': 'ro' in fields[5].split(','),
                    'sameDeviceAsCheckout': os.stat('/').st_dev == os.stat(G).st_dev}
    return {}


def cmd_probe(args) -> int:
    """Inside the sandbox: print what the guard sees (no processes are started). With --connect PATH (guard
    self-test, D-G42) and only when the guard passes: try to connect() to PATH and report what happened."""
    problems = guard('outer')
    listing = lambda path: sorted(os.listdir(path)) if os.path.isdir(path) else None
    out = {'problems': problems, 'pid': os.getpid(), 'init': Path('/proc/1/comm').read_text().strip(),
           'facts': {'runDbus': listing('/run/dbus'), 'runUser': listing('/run/user'),
                     'runSystemd': listing('/run/systemd'), 'run': listing('/run'), 'var': listing('/var'),
                     'realHomeEntries': listing(REAL_HOME), 'varLib': os.path.lexists('/var/lib'),
                     'snapdSocket': os.path.lexists('/run/snapd.socket'),
                     'masked': [p for p in os.environ.get(MASKED_VAR, '').split('\n') if p],
                     'rootMount': root_mount(), 'rootWritable': os.access('/', os.W_OK),
                     'hostBindRoots': [str(p) for p in host_bind_roots()]}}
    if not problems and args.connect:
        out['connectTarget'] = {'path': args.connect, 'isSocket': _is_socket(args.connect)}
        client = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        client.settimeout(3)
        try:
            client.connect(args.connect)
            out['connect'] = 'ok'
        except OSError as error:
            out['connect'] = f'error: {error.strerror or error}'
        finally:
            client.close()
    print(json.dumps(out, indent=1))
    return 0 if not problems else EXIT_REFUSED


def add_run_options(p: argparse.ArgumentParser) -> argparse.ArgumentParser:
    p.add_argument('--lang', choices=('nl', 'en'), required=True)
    p.add_argument('--switch', action='store_true', help='end with a live language switch (Settings + Start open)')
    p.add_argument('--surfaces', help='comma list: desktop,panel,start (the native shell), explorer,settings[-page],'
                                      'taskmgr,alles,disks (the native apps): screenshots and their dev state')
    p.add_argument('--patch-extension', action='store_true', help='fallback: unsafe_mode patch in the PRIVATE copy')
    p.add_argument('--guard-only', action='store_true', help='stop after the guard (self-test)')
    p.add_argument('--session-mode', choices=('winpoint',),
                   help="GNOME Shell in WinPoint's login session mode (scripts/install-session.py), the extension "
                        'turned on by that mode only')
    p.add_argument('--native-snap-fixture', action='store_true',
                   help='private first-boot regression seed: native tiling off and installed-but-disabled TA; requires WinPoint mode')
    p.add_argument('--out', help='output directory inside tests/gnome (default: baseline/<lang>, or '
                                 '<scenario dir>/<scenario stem>.out with --scenario)')
    p.add_argument('--lock', help=argparse.SUPPRESS)        # set by launch: "parent[:kind]" or "fd:N"
    p.add_argument('--seed', choices=('defaults', 'minimal'), default='defaults',
                   help='settings.json seed: complete defaults (default) or only {"language"} (bug repro)')
    p.add_argument('--build-dir', help=f'the {len(BINS_FALLBACK)} binaries to install, win11-shell, win11-calendar and '
                                       f'the win11-gtk-* apps included (inside G; default {DEFAULT_BUILD.relative_to(G)})')
    p.add_argument('--source', default='worktree',
                   help='tree to install the assets, extension and scripts from: "worktree" (default) or a git '
                        'revision (exported with git archive into run/src)')
    p.add_argument('--scenario', help='Python file with def run(h), run inside the sandbox after the session is up')
    p.add_argument('--profile-picture', metavar='FILE',
                   help='account picture fixture inside the checkout, copied only to the private HOME/.face')
    p.add_argument('--winpoint-feed', action='store_true',
                   help="D-G74a: WinPoint's update check against a fake GitHub on loopback (tests/gnome/lib/"
                        'fake_github.py) through its test seam WIN11_WINPOINT_TEST; without it the daily check is off')
    p.add_argument('--second-monitor', metavar='WxH',
                   help='a second virtual monitor of this size next to the 1440x900 one (multi-monitor taskbars; '
                        'between 640x480 and 3840x2160)')
    p.add_argument('--strace', metavar='FILE',
                   help='D-G42: every connect() of the install and the whole session under strace into FILE '
                        '(inside tests/gnome, outside run/); the run fails when one reached a socket outside the '
                        'sandbox (summary in FILE.json and results.json straceConnect)')
    return p


def run_parser() -> argparse.ArgumentParser:
    return add_run_options(argparse.ArgumentParser(prog='harness.py run', add_help=False))


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    sub = parser.add_subparsers(dest='cmd', required=True)
    p = sub.add_parser('launch', help='start bubblewrap with the private environment, then run the rest inside it')
    p.add_argument('--selftest-mutate', action='append', help=argparse.SUPPRESS)
    p.add_argument('--selftest-unmasked', action='append', help=argparse.SUPPRESS)   # guard self-test (D-G42)
    p.add_argument('--selftest-listed-unmasked', action='append', help=argparse.SUPPRESS)   # guard self-test (m3)
    p.add_argument('--own-lock', action='store_true',
                   help='take .heavy.lock here when no ancestor (tools/heavy.sh, flock) holds it; default: refuse (C8)')
    p.add_argument('--lock-check-only', action='store_true', help=argparse.SUPPRESS)   # guard self-test
    sub.add_parser('guard-selftest', help='prove each guard refusal (starts no GUI process)')
    sub.add_parser('strace-check', help='D-G42: check strace connect() logs of a run (FILE, ...) again').add_argument(
        'files', nargs='+', metavar='FILE')
    sub.add_parser('probe', help='inside the sandbox: show the guard result').add_argument(
        '--connect', metavar='PATH', help=argparse.SUPPRESS)                          # guard self-test (D-G42)
    for name in ('run', '_inner'):
        add_run_options(sub.add_parser(name))
    if argv and argv[0] == 'launch':
        idx = next((i for i, a in enumerate(argv) if a in ('run', 'probe')), len(argv))
        args = parser.parse_args(argv[:idx])
        return cmd_launch(args, argv[idx:])
    args = parser.parse_args(argv)
    if args.cmd == 'guard-selftest':
        return cmd_guard_selftest(args)
    if args.cmd == 'strace-check':
        report = strace_connect_report([Path(f) for f in args.files])
        for row in report['byTarget']:
            print(f'strace connect: {row["count"]:6d} x {row["family"]} {row["target"]} -> {row["result"]}')
        print(f'strace connect check: {report["connects"]} connect() calls in {len(args.files)} logs, '
              f'{"OK" if report["ok"] else "FAIL " + "; ".join(report["violations"][:5])}')
        # WU-FIX (D-G51)
        execs = strace_exec_report([Path(f) for f in args.files])
        for row in execs['byProgram']:
            print(f'strace exec: {row["count"]:6d} x {row["path"]} ({row["where"]}) -> {row["result"]}')
        # D-G74: WinPoint's one Python program
        for row in execs['winpointPython']['execs']:
            print(f'strace exec: WinPoint python3 at line {row["line"]} ({row["mode"] or "?"}, {row["env"]} vars): '
                  f'{" ".join(row["argv"][4:6])}')
        print(f'strace exec: {execs["winpointPython"]["clicks"]} click mark(s)')
        print(f'strace exec check: {execs["execs"]} execve() calls in {len(args.files)} logs, '
              f'{"OK" if execs["ok"] else "FAIL " + "; ".join(execs["violations"][:5])}')
        return 0 if report['ok'] and execs['ok'] else 1
    if args.cmd == 'probe':
        return cmd_probe(args)
    if args.cmd == 'run':
        return cmd_run(args)
    if args.cmd == '_inner':
        return cmd_inner(args)
    return EXIT_REFUSED


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
