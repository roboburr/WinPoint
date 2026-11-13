"""sysmon.* (SPEC §6.11): the Task Manager backend.

* ``sysmon.processes``  – /proc scan (stat, statm, io for own processes, cmdline/cgroup/exe cached per
  process), CPU % as share of all logical CPUs between samples, memory = private resident memory
  (``statm`` resident − shared = RssAnon, Windows' "private working set"; PSS via smaps_rollup costs
  ≈1.5 ms per process and is far too slow for a 1 s refresh). Processes are grouped into
  * ``apps``       – the process tree of a desktop app: systemd app scopes (``app-gnome-firefox-123.scope``,
                     ``app-flatpak-org.telegram.desktop-….scope``, ``snap.firefox.firefox-….scope``),
                     windows from ``windows.list`` (Hyprland), our own app windows, or the executable of a
                     visible desktop entry; children inherit the app.
  * ``background`` – other processes of the user, grouped by systemd user unit or executable.
  * ``system``     – (extra) processes of other users + kernel threads, grouped by executable.
* ``sysmon.endTask`` / ``sysmon.setEfficiency`` (dangerous; own processes only; never pid 1, the host,
  its parents or the desktop session's critical processes).
* ``sysmon.restartProcess {pid, confirm?}`` (dangerous; own processes only) — "Opnieuw starten" for a process that
  hangs: the shell itself → its restart path; our app windows and apps from a desktop entry → ended gracefully
  (SIGTERM, SIGKILL after 5 s) and started again through ``apps.launch``; systemd --user services →
  ``systemctl --user restart``; anything else → the same argv, cwd and environment again, detached — only when
  that is safe (see ``restart_plan``).
* ``sysmon.performance`` – CPU (per core, frequency, temperature), memory, physical disks, network adapters,
  GPUs (nvidia-smi in a background refresher, or amdgpu sysfs).
* ``sysmon.startup`` / ``sysmon.setStartup`` – XDG autostart entries (user override in ~/.config/autostart).
* ``sysmon.services`` / ``sysmon.serviceAction`` – ``systemctl --user`` units.
* ``sysmon.health`` – plain-language checks with a suggested action.

A light background sampler (5 s, only /proc/*/stat, only while a UI is connected) keeps CPU history for the
"high CPU for more than 30 s" health check and gives the first ``sysmon.processes`` call real CPU values.
"""

from __future__ import annotations

import logging
import os
import pwd
import re
import shlex
import signal
import threading
import time
import warnings
from collections import deque
from pathlib import Path
from typing import Any, Callable

from ..util import ApiError, atomic_write_text, json_loads, run, which, xdg_config_home

log = logging.getLogger("win11shell.sysmon")

PRIORITY = 60

CLK = os.sysconf("SC_CLK_TCK") if hasattr(os, "sysconf") else 100
PAGE = os.sysconf("SC_PAGE_SIZE") if hasattr(os, "sysconf") else 4096
NCPU = os.cpu_count() or 1
UID = os.getuid()
MYPID = os.getpid()
SAMPLER_S = 5.0
HIGH_CPU_CORE_PCT = 80.0          # a process using ≥ 80 % of one core …
HIGH_CPU_SECONDS = 30.0           # … for more than 30 s → health notice

_lang: Callable[[], str] = lambda: "nl"
_api = None

# ---------------------------------------------------------------------------
# texts (nl/en; error messages stay Dutch as per SPEC)
# ---------------------------------------------------------------------------

_STR = {
    "nl": {
        "disk_n": "Schijf {n}",
        "ethernet": "Ethernet", "ethernet_n": "Ethernet {n}", "wifi": "Wi-Fi", "wifi_n": "Wi-Fi {n}", "vpn": "VPN",
        "kernel": "Linux-kernel", "host": "Win11 Shell", "other_app": "{name}",
        "h_disk_ok_t": "Opslag", "h_disk_ok_m": "Er is op al je schijven genoeg ruimte vrij.",
        "h_disk_warn_t": "{name} is bijna vol",
        "h_disk_warn_m": "Er is nog {free} vrij van {total} ({pct}% gebruikt). Ruim bestanden op die je niet meer nodig hebt.",
        "h_disk_crit_t": "{name} is vol",
        "h_disk_crit_m": "Er is nog maar {free} vrij. Apps en updates kunnen hierdoor vastlopen. Maak zo snel mogelijk ruimte vrij.",
        "a_free": "Ruimte vrijmaken",
        "h_mem_ok_t": "Geheugen", "h_mem_ok_m": "Er is genoeg geheugen vrij ({avail} van {total}).",
        "h_mem_warn_t": "Weinig geheugen vrij",
        "h_mem_warn_m": "Er is nog {avail} van {total} vrij. Sluit apps die je niet gebruikt, dan blijft alles vlot.",
        "h_mem_crit_t": "Het geheugen is bijna op",
        "h_mem_crit_m": "Er is nog maar {avail} vrij. De computer kan traag worden of apps sluiten. Sluit een paar apps.",
        "a_procs": "Processen bekijken", "a_storage": "Opslag bekijken", "a_perf": "Prestaties bekijken",
        "h_swap_ok_t": "Wisselgeheugen", "h_swap_ok_m": "Het wisselgeheugen wordt nauwelijks gebruikt.",
        "h_swap_none_m": "Er is geen wisselgeheugen ingesteld.",
        "h_swap_warn_t": "Veel wisselgeheugen in gebruik",
        "h_swap_warn_m": "{used} van {total} wisselgeheugen is in gebruik. Daardoor kan de computer trager aanvoelen.",
        "h_cpu_ok_t": "Processor", "h_cpu_ok_m": "Geen enkele app gebruikt langdurig veel rekenkracht.",
        "h_cpu_warn_t": "{name} gebruikt veel rekenkracht",
        "h_cpu_warn_m": "Deze app gebruikt al {dur} veel van de processor. Reageert hij niet, sluit hem dan via Taakbeheer.",
        "a_view": "Proces bekijken",
        "h_temp_ok_t": "Temperatuur", "h_temp_ok_m": "De processor is lekker koel ({temp} °C).",
        "h_temp_na_m": "De temperatuur kan hier niet worden gemeten.",
        "h_temp_warn_t": "{what} wordt warm",
        "h_temp_warn_m": "De temperatuur is {temp} °C. Zorg dat de ventilatieopeningen vrij zijn.",
        "h_temp_crit_t": "{what} is te heet",
        "h_temp_crit_m": "De temperatuur is {temp} °C. Sluit zware apps en laat de computer even afkoelen.",
        "cpu_word": "De processor", "gpu_word": "De videokaart",
        "h_svc_ok_t": "Achtergronddiensten", "h_svc_ok_m": "Alle achtergronddiensten werken.",
        "h_svc_warn_t": "{n} achtergronddienst werkt niet", "h_svc_warn_t_pl": "{n} achtergronddiensten werken niet",
        "h_svc_warn_m": "Niet gestart: {names}.",
        "a_services": "Services bekijken",
        "h_upd_ok_t": "Updates", "h_upd_ok_m": "Je computer is bijgewerkt.",
        "h_upd_unknown_m": "Er is nog niet gecontroleerd op updates.",
        "h_upd_info_t": "Er zijn updates beschikbaar",
        "h_upd_info_m1": "Er is 1 update klaar om te installeren.",
        "h_upd_info_m": "Er zijn {n} updates klaar om te installeren.",
        "h_upd_sec_m": " Daarvan {s} voor de beveiliging.",
        "a_updates": "Updates installeren",
        "h_trash_ok_t": "Prullenbak", "h_trash_ok_m": "De prullenbak neemt weinig ruimte in.",
        "h_trash_info_t": "De prullenbak is groot",
        "h_trash_info_m": "Er staat {size} in de prullenbak. Leeg hem om ruimte vrij te maken.",
        "h_all_ok": "Alles ziet er goed uit",
    },
    "en": {
        "disk_n": "Disk {n}",
        "ethernet": "Ethernet", "ethernet_n": "Ethernet {n}", "wifi": "Wi-Fi", "wifi_n": "Wi-Fi {n}", "vpn": "VPN",
        "kernel": "Linux kernel", "host": "Win11 Shell", "other_app": "{name}",
        "h_disk_ok_t": "Storage", "h_disk_ok_m": "All your drives have enough free space.",
        "h_disk_warn_t": "{name} is almost full",
        "h_disk_warn_m": "{free} free of {total} ({pct}% used). Remove files you no longer need.",
        "h_disk_crit_t": "{name} is full",
        "h_disk_crit_m": "Only {free} left. Apps and updates may stop working. Free up space soon.",
        "a_free": "Free up space",
        "h_mem_ok_t": "Memory", "h_mem_ok_m": "Plenty of memory is free ({avail} of {total}).",
        "h_mem_warn_t": "Memory is running low",
        "h_mem_warn_m": "{avail} of {total} is free. Close apps you are not using to keep things smooth.",
        "h_mem_crit_t": "Memory is almost full",
        "h_mem_crit_m": "Only {avail} is free. Your PC may slow down or apps may close. Close a few apps.",
        "a_procs": "View processes", "a_storage": "View storage", "a_perf": "View performance",
        "h_swap_ok_t": "Swap", "h_swap_ok_m": "Swap is hardly used.",
        "h_swap_none_m": "No swap is configured.",
        "h_swap_warn_t": "A lot of swap in use",
        "h_swap_warn_m": "{used} of {total} swap is in use, which can make your PC feel slower.",
        "h_cpu_ok_t": "Processor", "h_cpu_ok_m": "No app is using a lot of processing power for long.",
        "h_cpu_warn_t": "{name} is using a lot of CPU",
        "h_cpu_warn_m": "This app has been using a lot of CPU for {dur}. If it does not respond, end it in Task Manager.",
        "a_view": "View process",
        "h_temp_ok_t": "Temperature", "h_temp_ok_m": "The processor is nice and cool ({temp} °C).",
        "h_temp_na_m": "The temperature cannot be measured here.",
        "h_temp_warn_t": "{what} is getting warm",
        "h_temp_warn_m": "The temperature is {temp} °C. Make sure the vents are not blocked.",
        "h_temp_crit_t": "{what} is too hot",
        "h_temp_crit_m": "The temperature is {temp} °C. Close heavy apps and let your PC cool down.",
        "cpu_word": "The processor", "gpu_word": "The graphics card",
        "h_svc_ok_t": "Background services", "h_svc_ok_m": "All background services are working.",
        "h_svc_warn_t": "{n} background service is not working", "h_svc_warn_t_pl": "{n} background services are not working",
        "h_svc_warn_m": "Not started: {names}.",
        "a_services": "View services",
        "h_upd_ok_t": "Updates", "h_upd_ok_m": "Your PC is up to date.",
        "h_upd_unknown_m": "Updates have not been checked yet.",
        "h_upd_info_t": "Updates are available",
        "h_upd_info_m1": "1 update is ready to install.",
        "h_upd_info_m": "{n} updates are ready to install.",
        "h_upd_sec_m": " {s} of them are security updates.",
        "a_updates": "Install updates",
        "h_trash_ok_t": "Recycle Bin", "h_trash_ok_m": "The Recycle Bin is small.",
        "h_trash_info_t": "The Recycle Bin is large",
        "h_trash_info_m": "The Recycle Bin holds {size}. Empty it to free up space.",
        "h_all_ok": "Everything looks good",
    },
}

# Friendly names for common background executables (nl, en).
_FRIENDLY = {
    "pipewire": ("PipeWire-geluidsserver", "PipeWire sound server"),
    "pipewire-pulse": ("PipeWire (PulseAudio-laag)", "PipeWire (PulseAudio layer)"),
    "wireplumber": ("Geluidsbeheer (WirePlumber)", "Sound manager (WirePlumber)"),
    "dbus-daemon": ("D-Bus-berichtendienst", "D-Bus message bus"),
    "dbus-broker": ("D-Bus-berichtendienst", "D-Bus message bus"),
    "dbus-broker-launch": ("D-Bus-berichtendienst", "D-Bus message bus"),
    "gnome-shell": ("GNOME Shell", "GNOME Shell"),
    "Hyprland": ("Hyprland (vensterbeheer)", "Hyprland (window manager)"),
    "Xwayland": ("Xwayland (oudere apps)", "Xwayland (legacy apps)"),
    "ibus-daemon": ("Invoermethode (IBus)", "Input method (IBus)"),
    "xdg-desktop-portal": ("Bureaubladportaal", "Desktop portal"),
    "xdg-document-portal": ("Documentportaal", "Document portal"),
    "xdg-permission-store": ("Machtigingenopslag", "Permission store"),
    "gvfsd": ("Bestandssysteemdiensten (GVfs)", "File system services (GVfs)"),
    "gnome-keyring-daemon": ("Sleutelbos (wachtwoorden)", "Keyring (passwords)"),
    "at-spi-bus-launcher": ("Toegankelijkheidsdienst", "Accessibility service"),
    "at-spi2-registryd": ("Toegankelijkheidsdienst", "Accessibility service"),
    "tracker-miner-fs-3": ("Bestanden indexeren", "File indexer"),
    "localsearch-3": ("Bestanden indexeren", "File indexer"),
    "evolution-source-registry": ("Agenda- en contactendienst", "Calendar and contacts service"),
    "evolution-calendar-factory": ("Agenda-dienst", "Calendar service"),
    "evolution-addressbook-factory": ("Adresboekdienst", "Address book service"),
    "goa-daemon": ("Online-accounts", "Online accounts"),
    "systemd": ("Systeembeheer (systemd)", "System manager (systemd)"),
    "hypridle": ("Inactiviteitsbeheer", "Idle manager"),
    "hyprpaper": ("Achtergrond (hyprpaper)", "Wallpaper (hyprpaper)"),
    "hyprsunset": ("Nachtlampje", "Night light"),
    "mutter-x11-frames": ("Vensterranden (Mutter)", "Window frames (Mutter)"),
    "gsd-xsettings": ("GNOME-instellingen", "GNOME settings"),
    "snapd-desktop-integration": ("Snap-integratie", "Snap integration"),
    "gjs": ("GNOME JavaScript", "GNOME JavaScript"),
    "WebKitWebProcess": ("Webinhoud", "Web content"),
    "WebKitNetworkProcess": ("Webnetwerk", "Web network"),
    "speech-dispatcher": ("Spraakdienst", "Speech service"),
    "sd_dummy": ("Spraakdienst", "Speech service"),
    "polkit-gnome-authentication-agent-1": ("Beheerdersvragen", "Admin prompts"),
    "hyprpolkitagent": ("Beheerdersvragen", "Admin prompts"),
    "sshd": ("SSH-server", "SSH server"),
    "ssh-agent": ("SSH-sleutels", "SSH keys"),
    "gcr-ssh-agent": ("SSH-sleutels", "SSH keys"),
    "obexd": ("Bluetooth-bestandsoverdracht", "Bluetooth file transfer"),
    "(sd-pam)": ("Systeembeheer (systemd)", "System manager (systemd)"),
    "gdm-wayland-session": ("Aanmeldsessie", "Login session"),
    "gdm-x-session": ("Aanmeldsessie", "Login session"),
    "gnome-session-init-worker": ("GNOME-sessiebeheer", "GNOME session manager"),
    "gnome-session-binary": ("GNOME-sessiebeheer", "GNOME session manager"),
    "gnome-session-service": ("GNOME-sessiebeheer", "GNOME session manager"),
    "gnome-session-ctl": ("GNOME-sessiebeheer", "GNOME session manager"),
    "mpris-proxy": ("Bluetooth-mediabediening", "Bluetooth media control"),
    "dconf-service": ("Instellingendatabase", "Settings database"),
    "update-notifier": ("Updatemeldingen", "Update notifier"),
    "evolution-alarm-notify": ("Herinneringen", "Reminders"),
    "gvfsd-metadata": ("Bestandsgegevens", "File metadata"),
    "gvfsd-fuse": ("Bestandssysteemdiensten (GVfs)", "File system services (GVfs)"),
    "flatpak-portal": ("Flatpak-dienst", "Flatpak service"),
    "flatpak-session-helper": ("Flatpak-dienst", "Flatpak service"),
    "xdg-desktop-portal-gnome": ("Bureaubladportaal (GNOME)", "Desktop portal (GNOME)"),
    "xdg-desktop-portal-gtk": ("Bureaubladportaal (GTK)", "Desktop portal (GTK)"),
    "xdg-desktop-portal-hyprland": ("Bureaubladportaal (Hyprland)", "Desktop portal (Hyprland)"),
    "filter-chain": ("Geluidsfilters (PipeWire)", "Sound filters (PipeWire)"),
    "earlyoom": ("Geheugenbewaking", "Memory watchdog"),
    "systemd-oomd": ("Geheugenbewaking", "Memory watchdog"),
    "gpg-agent": ("GPG-sleutels", "GPG keys"),
}

# Processes that keep the desktop session alive: never ended or throttled from Task Manager.
_CRITICAL_COMM = {"systemd", "Hyprland", "gnome-shell", "gnome-session-b", "gnome-session-c", "Xwayland",
                  "dbus-daemon", "dbus-broker", "dbus-broker-lau", "gdm-wayland-ses", "gdm-x-session",
                  "mutter-x11-fram", "sway", "kwin_wayland", "plasmashell", "uwsm", "start-hyprland",
                  "(sd-pam)", "login", "systemd-logind", "Xorg"}
_CRITICAL_UNITS = re.compile(r"^(dbus|dbus-broker|pipewire|pipewire-pulse|wireplumber|org\.gnome\.Shell@.*|"
                             r"gnome-session.*|xdg-desktop-portal.*|at-spi-dbus-bus|init|"
                             r"wayland-wm@.*|graphical-session.*)\.(service|target)$")
_INTERPRETERS = {"python", "python3", "node", "nodejs", "bash", "sh", "dash", "zsh", "fish", "perl", "ruby",
                 "java", "env", "gjs", "lua", "php", "deno", "bun", "electron", "qemu-system-x86_64"}
_GENERIC_EXE = _INTERPRETERS | {"flatpak", "snap", "bwrap", "sleep", "cat", "xdg-open", "sudo", "pkexec",
                                "dbus-launch", "dbus-run-session", "systemd-run", "gio", "exo-open", "false",
                                "true", "wine", "wine64", "steam", "env", "nohup", "setsid", "chrt", "nice"}
_LAUNCHERS = {"gnome", "flatpak", "kde", "hyprland", "Hyprland", "sway", "niri", "cosmic", "xdg", "uwsm",
              "dbus", "systemd", "gtk", "ubuntu"}
_STATE_MAP = {"R": "running", "S": "sleeping", "D": "waiting", "T": "stopped", "t": "stopped", "Z": "zombie",
              "X": "zombie", "I": "idle", "W": "waiting", "P": "stopped"}
_OWN_APPS = {"settings": ("Instellingen", "Settings", "preferences-system"),
             "taskmgr": ("Taakbeheer", "Task Manager", "utilities-system-monitor"),
             "explorer": ("Verkenner", "File Explorer", "system-file-manager"),
             "alles": ("Alles", "Alles", "system-search"),
             "disks": ("Schijven", "Disks", "org.gnome.DiskUtility")}


def _tr(key: str, **kw) -> str:
    table = _STR.get(_lang(), _STR["nl"])
    text = table.get(key) or _STR["nl"].get(key) or key
    try:
        return text.format(**kw) if kw else text
    except (KeyError, IndexError):
        return text


_GSD_NL = {"A11ySettings": "toegankelijkheid", "Color": "kleurbeheer", "Datetime": "datum en tijd",
           "Housekeeping": "opruimen", "Keyboard": "toetsenbord", "MediaKeys": "mediatoetsen", "Power": "energiebeheer",
           "PrintNotifications": "printermeldingen", "Rfkill": "draadloos", "ScreensaverProxy": "schermbeveiliging",
           "Sharing": "delen", "Smartcard": "smartcards", "Sound": "geluid", "UsbProtection": "USB-beveiliging",
           "Wwan": "mobiel netwerk", "XSettings": "app-instellingen", "DiskUtilityNotify": "schijfmeldingen",
           "Wacom": "tekentablet"}
_GVFS_NL = {"afc": "Apple-apparaten", "gphoto2": "camera's", "udisks2": "schijven", "goa": "online-accounts",
            "mtp": "telefoons"}


def _friendly(*names: str | None) -> str | None:
    en = _lang() == "en"
    for comm in names:
        if not comm:
            continue
        f = _FRIENDLY.get(comm)
        if f is not None:
            return f[1] if en else f[0]
        if comm.startswith("gsd-"):
            return "GNOME settings daemon" if en else "GNOME-instellingendienst"
    return None


def _unit_friendly(unit: str) -> str | None:
    """Dutch names for well-known user units (English uses the unit description)."""
    if _lang() == "en":
        return None
    m = re.match(r"^org\.gnome\.SettingsDaemon\.([A-Za-z0-9]+)(\.service)?$", unit)
    if m:
        part = _GSD_NL.get(m.group(1))
        return f"GNOME-instellingen: {part}" if part else "GNOME-instellingendienst"
    m = re.match(r"^gvfs-([a-z0-9]+)-volume-monitor\.service$", unit)
    if m:
        return f"Stationsbewaking ({_GVFS_NL.get(m.group(1), m.group(1))})"
    table = {"speech-dispatcher.service": "Spraakdienst", "at-spi-dbus-bus.service": "Toegankelijkheidsdienst",
             "gnome-keyring-daemon.service": "Sleutelbos (wachtwoorden)", "dconf.service": "Instellingendatabase",
             "xdg-permission-store.service": "Machtigingenopslag", "xdg-document-portal.service": "Documentportaal",
             "xdg-desktop-portal.service": "Bureaubladportaal", "gvfs-daemon.service": "Bestandssysteemdiensten (GVfs)",
             "gvfs-metadata.service": "Bestandsgegevens", "evolution-source-registry.service": "Agenda en contacten",
             "evolution-calendar-factory.service": "Agenda-dienst",
             "evolution-addressbook-factory.service": "Adresboekdienst",
             "gnome-session-monitor.service": "GNOME-sessiebeheer", "mpris-proxy.service": "Bluetooth-mediabediening",
             "obex.service": "Bluetooth-bestandsoverdracht", "filter-chain.service": "Geluidsfilters (PipeWire)",
             "flatpak-portal.service": "Flatpak-dienst", "flatpak-session-helper.service": "Flatpak-dienst",
             "gcr-ssh-agent.service": "SSH-sleutels", "ssh-agent.service": "SSH-sleutels",
             "localsearch-3.service": "Bestanden indexeren", "tracker-miner-fs-3.service": "Bestanden indexeren",
             "xdg-desktop-portal-gnome.service": "Bureaubladportaal (GNOME)",
             "xdg-desktop-portal-gtk.service": "Bureaubladportaal (GTK)",
             "xdg-desktop-portal-hyprland.service": "Bureaubladportaal (Hyprland)",
             "org.gnome.Evolution-alarm-notify": "Herinneringen", "update-notifier": "Updatemeldingen",
             "snapd-desktop-integration": "Snap-integratie"}
    if unit in table:
        return table[unit]
    if unit.startswith("gnome-session-manager@"):
        return "GNOME-sessiebeheer"
    if unit.startswith("org.gnome.Shell@"):
        return "GNOME Shell"
    if unit.startswith("snap.") and unit.endswith(".service"):
        return f"Snap-dienst ({unit.split('.')[1]})"
    return None


def fmt_bytes(n: float | None) -> str:
    """Dutch/English byte formatting matching ui/js/core/format.js bytes() (binary units)."""
    if n is None:
        return "–"
    units = ["KB", "MB", "GB", "TB", "PB"]
    v = float(abs(n))
    if v < 1024:
        return f"{int(round(v))} bytes"
    i = -1
    while v >= 1024 and i < len(units) - 1:
        v /= 1024
        i += 1
    d = 0 if v >= 100 else 1
    s = f"{v:.{d}f}"
    if _lang() != "en":
        s = s.replace(".", ",")
    return f"{'-' if n < 0 else ''}{s} {units[i]}"


def _fmt_duration(sec: float) -> str:
    sec = int(sec)
    en = _lang() == "en"
    if sec < 60:
        return f"{sec} s" if en else f"{sec} seconden"
    m = sec // 60
    if m < 60:
        return (f"{m} min" if en else f"{m} minuten") if m != 1 else ("1 minute" if en else "1 minuut")
    h, m = divmod(m, 60)
    return f"{h} h {m} min" if en else f"{h} u {m} min"


# ---------------------------------------------------------------------------
# low-level readers
# ---------------------------------------------------------------------------

def _read_bytes(path: str, size: int = 65536) -> bytes | None:
    """Read up to ``size`` bytes (loops: /proc seq files hand out one page per read)."""
    try:
        fd = os.open(path, os.O_RDONLY | os.O_CLOEXEC)
    except OSError:
        return None
    try:
        chunks = []
        got = 0
        while got < size:
            b = os.read(fd, min(65536, size - got))
            if not b:
                break
            chunks.append(b)
            got += len(b)
        return b"".join(chunks)
    except OSError:
        return None
    finally:
        os.close(fd)


def _read(path: str | Path, default: str | None = None) -> str | None:
    try:
        with open(path, "r", encoding="utf-8", errors="replace") as f:
            return f.read().strip()
    except OSError:
        return default


def _int(v: Any, default: int | None = None) -> int | None:
    try:
        return int(str(v).strip())
    except (TypeError, ValueError):
        return default


def boot_time() -> float:
    data = _read_bytes("/proc/stat") or b""
    i = data.find(b"\nbtime ")
    if i >= 0:
        try:
            return float(data[i + 7:data.index(b"\n", i + 7)])
        except ValueError:
            pass
    up = _read("/proc/uptime", "0").split()[0]
    return time.time() - float(up)


BOOT = boot_time()


def read_cpu_times() -> list[tuple[int, int]]:
    """[(busy, total)] for the aggregate (index 0) and every core (1..n)."""
    out = []
    data = _read_bytes("/proc/stat") or b""
    for line in data.split(b"\n"):
        if not line.startswith(b"cpu"):
            break
        parts = line.split()
        vals = [int(x) for x in parts[1:9]]  # user nice system idle iowait irq softirq steal
        idle = vals[3] + vals[4]
        total = sum(vals)
        out.append((total - idle, total))
    return out


def read_meminfo() -> dict[str, int]:
    out: dict[str, int] = {}
    data = _read_bytes("/proc/meminfo") or b""
    for line in data.split(b"\n"):
        k, _, rest = line.partition(b":")
        parts = rest.split()
        if parts:
            try:
                out[k.decode()] = int(parts[0]) * (1024 if len(parts) > 1 else 1)
            except ValueError:
                continue
    return out


def read_diskstats() -> dict[str, tuple[int, int, int, int, int]]:
    """{name: (sectors_read, sectors_written, io_ticks_ms, ios, io_ms)}"""
    out = {}
    data = _read_bytes("/proc/diskstats", 1 << 20) or b""
    for line in data.split(b"\n"):
        p = line.split()
        if len(p) < 14:
            continue
        try:
            out[p[2].decode()] = (int(p[5]), int(p[9]), int(p[12]), int(p[3]) + int(p[7]), int(p[6]) + int(p[10]))
        except ValueError:
            continue
    return out


def read_netdev() -> dict[str, tuple[int, int]]:
    out = {}
    data = _read_bytes("/proc/net/dev", 1 << 20) or b""
    for line in data.split(b"\n")[2:]:
        name, sep, rest = line.partition(b":")
        if not sep:
            continue
        p = rest.split()
        if len(p) >= 9:
            out[name.strip().decode()] = (int(p[0]), int(p[8]))
    return out


# ---------------------------------------------------------------------------
# desktop entries (app names / icons)
# ---------------------------------------------------------------------------

class DesktopIndex:
    """Maps scope ids, window classes, flatpak/snap ids and executables to desktop entries. Cached."""

    def __init__(self):
        self._lock = threading.Lock()
        self.built = 0.0
        self.lang = None
        self.by_id: dict[str, dict] = {}
        self.by_wm: dict[str, dict] = {}
        self.by_exec: dict[str, dict] = {}
        self.autostart: set[str] = set()

    def ensure(self, max_age: float = 600.0) -> None:
        if self.built and time.monotonic() - self.built < max_age and self.lang == _lang():
            return
        with self._lock:
            if self.built and time.monotonic() - self.built < max_age and self.lang == _lang():
                return
            self._build()

    def invalidate(self) -> None:
        self.built = 0.0

    def _build(self) -> None:
        lang = _lang()
        by_id: dict[str, dict] = {}
        by_wm: dict[str, dict] = {}
        by_exec: dict[str, dict] = {}
        try:
            import gi
            gi.require_version("Gio", "2.0")
            from gi.repository import Gio, GLib
            with warnings.catch_warnings():
                warnings.simplefilter("ignore")
                infos = Gio.AppInfo.get_all()
        except Exception as e:  # noqa: BLE001
            log.info("Geen app-informatie (no desktop entries): %s", e)
            infos = []
            GLib = None
        for a in infos:
            try:
                did = a.get_id() or ""
                if not did.endswith(".desktop"):
                    continue
                get_s = getattr(a, "get_string", None)
                name = a.get_name() or did[:-8]
                fn = a.get_filename() if hasattr(a, "get_filename") else None
                if fn and GLib is not None and lang:
                    try:
                        kf = GLib.KeyFile()
                        kf.load_from_file(fn, GLib.KeyFileFlags.NONE)
                        name = kf.get_locale_string("Desktop Entry", "Name", lang) or name
                    except Exception:  # noqa: BLE001
                        pass
                icon = a.get_icon()
                entry = {"id": did, "name": name, "icon": icon.to_string() if icon else None,
                         "nodisplay": bool(a.get_nodisplay()) if hasattr(a, "get_nodisplay") else False,
                         "terminal": (get_s("Terminal") == "true") if get_s else False}
                base = did[:-8].lower()
                vis = not entry["nodisplay"]

                def put(table, key, e=entry, visible=vis):
                    if not key:
                        return
                    old = table.get(key)
                    if old is None or (visible and old["nodisplay"]):
                        table[key] = e

                put(by_id, base)
                if get_s:
                    put(by_id, (get_s("X-Flatpak") or "").lower())
                    snap = get_s("X-SnapInstanceName")
                    if snap:
                        put(by_wm, snap.lower())
                    for wm in (get_s("StartupWMClass") or "").split(","):
                        put(by_wm, wm.strip().lower())
                exe = a.get_executable() or ""
                exe_base = os.path.basename(exe.strip().strip('"')).lower()
                if exe_base and exe_base not in _GENERIC_EXE and not entry["terminal"]:
                    put(by_exec, exe_base)
                    if exe_base.endswith("-stable"):
                        put(by_exec, exe_base[:-7])
                # reverse-DNS ids: org.gnome.Nautilus → nautilus (executable name usually)
                if "." in base:
                    put(by_exec, base.rsplit(".", 1)[1])
            except Exception:  # noqa: BLE001
                continue
        autostart = set()
        for d in (Path("/etc/xdg/autostart"), xdg_config_home() / "autostart"):
            try:
                for f in d.iterdir():
                    if f.suffix == ".desktop":
                        autostart.add(f.stem.lower())
            except OSError:
                continue
        # Chrome's process is "chrome" while its entry runs google-chrome-stable
        for alias, target in (("chrome", "google-chrome"), ("brave", "brave-browser"), ("code", "code"),
                              ("soffice.bin", "libreoffice-startcenter")):
            if alias not in by_exec and target in by_id:
                by_exec[alias] = by_id[target]
        self.by_id, self.by_wm, self.by_exec, self.autostart = by_id, by_wm, by_exec, autostart
        self.lang = lang
        self.built = time.monotonic()

    def lookup(self, key: str | None) -> dict | None:
        if not key:
            return None
        k = key.lower()
        hit = self.by_id.get(k) or self.by_wm.get(k)
        if hit is None and k.endswith(".desktop"):
            k = k[:-8]
            hit = self.by_id.get(k) or self.by_wm.get(k)
        if hit is None and "." in k:
            # reverse-DNS id without an installed entry (org.chromium.Chromium → chromium)
            last = k.rsplit(".", 1)[1]
            hit = self.by_wm.get(last) or self.by_id.get(last) or self.by_exec.get(last)
        return hit

    def by_executable(self, name: str | None) -> dict | None:
        if not name:
            return None
        e = self.by_exec.get(name.lower())
        return e if e and not e["nodisplay"] else None


_DESKTOP = DesktopIndex()


def _unescape_unit(s: str) -> str:
    return re.sub(r"\\x([0-9a-fA-F]{2})", lambda m: chr(int(m.group(1), 16)), s)


def parse_cgroup_unit(cgroup: str) -> tuple[str, str, str] | None:
    """'…/app-gnome-zen-3657452.scope' → ('scope', 'zen', unit). Services → ('service', id, unit).
    Other units in the user manager → ('unit', unit, unit). Pure (tested)."""
    if not cgroup:
        return None
    last = cgroup.rstrip("/").rsplit("/", 1)[-1]
    if last.startswith("app-") and last.endswith(".scope"):
        parts = last[4:-6].split("-")
        if len(parts) >= 3 and parts[0] in _LAUNCHERS:
            app_id = "-".join(parts[1:-1])
        elif len(parts) >= 2:
            app_id = "-".join(parts[:-1])
        else:
            app_id = parts[0]
        return "scope", _unescape_unit(app_id), last
    if last.startswith("app-") and last.endswith(".service"):
        base = last[4:-8].split("@", 1)[0]
        parts = base.split("-")
        app_id = "-".join(parts[1:]) if len(parts) >= 2 and parts[0] in _LAUNCHERS else base
        return "service", _unescape_unit(app_id), last
    m = re.match(r"^snap\.([a-z0-9-]+)\.([A-Za-z0-9-]+?)(?:[.-][0-9a-f]{8}-[0-9a-f-]{20,})?\.(scope|service)$", last)
    if m:
        return ("scope" if m.group(3) == "scope" else "service"), f"{m.group(1)}_{m.group(2)}", last
    if last.endswith((".service", ".scope")) and "/user@" in cgroup:
        return "unit", last, last
    return None


# ---------------------------------------------------------------------------
# GPU
# ---------------------------------------------------------------------------

_lspci_cache: dict[str, str | None] = {}


def _pci_name(slot: str) -> str | None:
    """'0000:06:00.0' → 'NVIDIA GeForce RTX 3080 Lite Hash Rate' (cached)."""
    if slot in _lspci_cache:
        return _lspci_cache[slot]
    name = None
    if which("lspci"):
        r = run(["lspci", "-mm", "-s", slot], timeout=3)
        if r.ok and r.out.strip():
            try:
                f = shlex.split(r.out.strip().splitlines()[0])
                name = _nice_pci(f[2] if len(f) > 2 else "", f[3] if len(f) > 3 else "")
            except ValueError:
                name = None
    _lspci_cache[slot] = name
    return name


def _nice_pci(vendor: str, device: str) -> str:
    v = vendor
    if "NVIDIA" in vendor:
        v = "NVIDIA"
    elif "Advanced Micro Devices" in vendor or "AMD" in vendor or "ATI" in vendor:
        v = "AMD"
    elif "Intel" in vendor:
        v = "Intel"
    else:
        v = re.sub(r"\s+(Corporation|Corp\.?|Inc\.?|Co\.,? Ltd\.?|Technology|Semiconductor)\b.*$", "", vendor)
    m = re.search(r"\[([^\]]+)\]", device)
    model = m.group(1) if m else device
    if v and model.lower().startswith(v.lower()):
        return model
    return f"{v} {model}".strip()


def _drm_cards() -> list[dict]:
    out = []
    try:
        names = sorted(os.listdir("/sys/class/drm"))
    except OSError:
        return out
    for n in names:
        if not re.match(r"^card\d+$", n):
            continue
        dev = Path("/sys/class/drm") / n / "device"
        vendor = _read(dev / "vendor", "") or ""
        try:
            slot = os.path.basename(os.path.realpath(dev))
        except OSError:
            slot = ""
        out.append({"card": n, "dev": dev, "vendor": {"0x10de": "nvidia", "0x1002": "amd", "0x8086": "intel"}
                    .get(vendor.lower(), "other"), "slot": slot})
    return out


class Gpu:
    """GPU stats; nvidia-smi runs in a background refresher (≈50 ms) so callers never wait."""

    FIELDS = "name,utilization.gpu,memory.used,memory.total,temperature.gpu,utilization.encoder," \
             "utilization.decoder,power.draw,clocks.gr,driver_version,pci.bus_id"

    def __init__(self):
        self._lock = threading.Lock()
        self._nv: list[dict] | None = None
        self._nv_time = 0.0
        self._busy = False
        self._event = threading.Event()

    def _query_nvidia(self) -> None:
        try:
            r = run(["nvidia-smi", f"--query-gpu={self.FIELDS}", "--format=csv,noheader,nounits"], timeout=3)
            res: list[dict] = []
            if r.ok:
                for line in r.out.strip().splitlines():
                    f = [x.strip() for x in line.split(",")]
                    if len(f) < 5:
                        continue

                    def num(x):
                        try:
                            return float(x)
                        except ValueError:
                            return None

                    used, total = num(f[2]), num(f[3])
                    res.append({
                        "name": f[0] if f[0].upper().startswith("NVIDIA") else f"NVIDIA {f[0]}",
                        "usage": num(f[1]), "memUsed": int(used * 1048576) if used is not None else None,
                        "memTotal": int(total * 1048576) if total is not None else None,
                        "temperature": num(f[4]), "vendor": "nvidia",
                        "engines": {"3d": num(f[1]), "encode": num(f[5]) if len(f) > 5 else None,
                                    "decode": num(f[6]) if len(f) > 6 else None},
                        "power": num(f[7]) if len(f) > 7 else None,
                        "clockMHz": num(f[8]) if len(f) > 8 else None,
                        "driver": f[9] if len(f) > 9 else None,
                        "slot": f[10].lower()[-12:] if len(f) > 10 else None,
                    })
            with self._lock:
                self._nv = res
                self._nv_time = time.monotonic()
        finally:
            with self._lock:
                self._busy = False
            self._event.set()

    def nvidia(self, max_age: float = 1.0, wait: float = 1.5) -> list[dict]:
        if not which("nvidia-smi"):
            return []
        start_thread = False
        with self._lock:
            fresh = self._nv is not None and time.monotonic() - self._nv_time < max_age
            if not fresh and not self._busy:
                self._busy = True
                self._event.clear()
                start_thread = True
            first = self._nv is None
        if start_thread:
            threading.Thread(target=self._query_nvidia, name="sysmon-nvidia", daemon=True).start()
        if first:
            self._event.wait(wait)
        with self._lock:
            return [dict(g) for g in (self._nv or [])]

    def stats(self) -> list[dict]:
        out = self.nvidia()
        nv_done = bool(out)
        for c in _drm_cards():
            if c["vendor"] == "nvidia" and (nv_done or which("nvidia-smi")):
                continue
            dev = c["dev"]
            usage = _int(_read(dev / "gpu_busy_percent"), None)
            used = _int(_read(dev / "mem_info_vram_used"), None)
            total = _int(_read(dev / "mem_info_vram_total"), None)
            temp = None
            for hw in sorted(dev.glob("hwmon/hwmon*")):
                t = _int(_read(hw / "temp1_input"), None)
                if t is not None:
                    temp = round(t / 1000, 1)
                    break
            out.append({"name": _pci_name(c["slot"]) or c["card"], "usage": usage, "memUsed": used,
                        "memTotal": total, "temperature": temp, "vendor": c["vendor"], "slot": c["slot"]})
        return out


_GPU = Gpu()


def gpu_names() -> list[str]:
    """Names of every GPU (used by system.info)."""
    names = [g["name"] for g in _GPU.nvidia(max_age=3600, wait=3)]
    have_nv = bool(names)
    for c in _drm_cards():
        if c["vendor"] == "nvidia" and have_nv:
            continue
        n = _pci_name(c["slot"]) if c["slot"] else None
        if n and n not in names:
            names.append(n)
    if not names and which("lspci"):
        r = run(["lspci", "-mm"], timeout=3)
        for line in r.out.splitlines() if r.ok else []:
            try:
                f = shlex.split(line)
            except ValueError:
                continue
            if len(f) > 3 and re.search(r"VGA|3D|Display", f[1]):
                names.append(_nice_pci(f[2], f[3]))
    return names


# ---------------------------------------------------------------------------
# temperatures
# ---------------------------------------------------------------------------

_hwmon_cpu: tuple[float, str | None] = (0.0, None)


def cpu_temperature() -> float | None:
    global _hwmon_cpu
    now = time.monotonic()
    path = _hwmon_cpu[1]
    if now - _hwmon_cpu[0] > 300:
        path = None
        best = None
        for hw in sorted(Path("/sys/class/hwmon").glob("hwmon*")):
            name = _read(hw / "name", "") or ""
            if name not in ("k10temp", "coretemp", "zenpower", "cpu_thermal", "acpitz_0", "acpitz"):
                continue
            rank = {"k10temp": 0, "zenpower": 0, "coretemp": 0, "cpu_thermal": 1}.get(name, 2)
            for lab in sorted(hw.glob("temp*_label")):
                label = _read(lab, "") or ""
                lrank = {"Tdie": 0, "Tctl": 1, "Package id 0": 0}.get(label, 5)
                cand = (rank, lrank, str(lab).replace("_label", "_input"))
                if best is None or cand < best:
                    best = cand
            if best is None or best[0] > rank:
                inp = hw / "temp1_input"
                if inp.exists():
                    cand = (rank, 6, str(inp))
                    if best is None or cand < best:
                        best = cand
        path = best[2] if best else None
        _hwmon_cpu = (now, path)
    if not path:
        return None
    v = _int(_read(path), None)
    return round(v / 1000, 1) if v is not None else None


# ---------------------------------------------------------------------------
# processes
# ---------------------------------------------------------------------------

class _Static:
    """Per-process data that never changes (cached by (pid, starttime))."""
    __slots__ = ("cmd", "argv", "exe", "cgroup", "unit", "user")

    def __init__(self, pid: int, uid: int):
        raw = _read_bytes(f"/proc/{pid}/cmdline", 8192) or b""
        argv = [a.decode("utf-8", "replace") for a in raw.split(b"\0") if a]
        self.argv = argv
        cmd = " ".join(argv)
        self.cmd = cmd if len(cmd) <= 300 else cmd[:299] + "…"
        self.exe = None
        if uid == UID:
            try:
                self.exe = os.readlink(f"/proc/{pid}/exe")
            except OSError:
                self.exe = None
        cg = (_read_bytes(f"/proc/{pid}/cgroup", 4096) or b"").decode("utf-8", "replace")
        line = next((ln for ln in cg.splitlines() if ln.startswith("0::")), cg.splitlines()[0] if cg else "")
        self.cgroup = line.split(":", 2)[-1] if line else ""
        self.unit = parse_cgroup_unit(self.cgroup)
        self.user = _username(uid)


_users: dict[int, str] = {}


def _username(uid: int) -> str:
    u = _users.get(uid)
    if u is None:
        try:
            u = pwd.getpwuid(uid).pw_name
        except KeyError:
            u = str(uid)
        _users[uid] = u
    return u


def scan_proc() -> tuple[float, dict[int, tuple]]:
    """{pid: (ppid, comm, state, ticks, nice, threads, start, private_bytes, uid)} – the hot path (~8 ms)."""
    now = time.monotonic()
    out: dict[int, tuple] = {}
    try:
        entries = os.listdir("/proc")
    except OSError:
        return now, out
    o_open, o_read, o_close, o_stat, flags = os.open, os.read, os.close, os.stat, os.O_RDONLY | os.O_CLOEXEC
    for e in entries:
        if not e.isdigit():
            continue
        base = "/proc/" + e
        try:
            fd = o_open(base + "/stat", flags)
            try:
                data = o_read(fd, 2048)
            finally:
                o_close(fd)
            fd = o_open(base + "/statm", flags)
            try:
                statm = o_read(fd, 256)
            finally:
                o_close(fd)
            uid = o_stat(base).st_uid
        except OSError:
            continue
        lp = data.find(b"(")
        rp = data.rfind(b")")
        if lp < 0 or rp < 0:
            continue
        f = data[rp + 2:].split()
        try:
            sm = statm.split()
            private = (int(sm[1]) - int(sm[2])) * PAGE if len(sm) > 2 else 0
            out[int(e)] = (int(f[1]), data[lp + 1:rp].decode("utf-8", "replace"), f[0].decode(),
                           int(f[11]) + int(f[12]), int(f[16]), int(f[17]), int(f[19]), max(0, private), uid)
        except (ValueError, IndexError):
            continue
    return now, out


def scan_ticks() -> tuple[float, dict[int, tuple[int, int]]]:
    """Cheap sampler pass: {pid: (start, ticks)} from /proc/*/stat only."""
    now = time.monotonic()
    out = {}
    try:
        entries = os.listdir("/proc")
    except OSError:
        return now, out
    for e in entries:
        if not e.isdigit():
            continue
        data = _read_bytes(f"/proc/{e}/stat", 2048)
        if not data:
            continue
        rp = data.rfind(b")")
        f = data[rp + 2:].split()
        try:
            out[int(e)] = (int(f[19]), int(f[11]) + int(f[12]))
        except (ValueError, IndexError):
            continue
    return now, out


def _io(pid: int) -> tuple[int, int] | None:
    data = _read_bytes(f"/proc/{pid}/io", 1024)
    if not data:
        return None
    rd = wr = 0
    for line in data.split(b"\n"):
        if line.startswith(b"read_bytes:"):
            rd = int(line[11:])
        elif line.startswith(b"write_bytes:"):
            wr = int(line[12:])
    return rd, wr


def _cpu_weight(cgroup: str) -> int | None:
    v = _read(f"/sys/fs/cgroup{cgroup}/cpu.weight")
    return _int(v, None) if v else None


class ProcessMonitor:
    def __init__(self, api):
        self.api = api
        self._lock = threading.Lock()
        self.static: dict[tuple[int, int], _Static] = {}
        # ring of (time, {pid: (start, ticks)}) for CPU deltas
        self.samples: deque = deque(maxlen=4)
        self.io_prev: dict[tuple[int, int], tuple[float, int, int]] = {}
        self.disk_prev: tuple[float, dict] | None = None
        self.net_prev: tuple[float, dict] | None = None
        self.high_since: dict[tuple[int, int], float] = {}
        self.last_cpu: dict[tuple[int, int], float] = {}
        self.last_names: dict[tuple[int, int], tuple[str, str]] = {}
        self.cpu_prev: tuple[int, int] | None = None
        self.weights: tuple[float, dict[str, int | None]] = (0.0, {})
        self.windows_cache: tuple[float, list] = (0.0, [])
        self.units_desc: tuple[float, dict[str, str]] = (0.0, {})
        self.critical: set[int] = set()
        self._crit_time = 0.0

    # -- CPU accounting --------------------------------------------------------
    def _baseline(self, now: float) -> tuple[float, dict] | None:
        best = None
        for t, m in reversed(self.samples):
            if now - t >= 0.4:
                return t, m
            best = (t, m)
        return best

    def _record(self, now: float, ticks: dict[int, tuple[int, int]]) -> dict[tuple[int, int], float]:
        """Store a sample, return {(pid, start): cpu% of the machine} and update high-CPU tracking."""
        base = self._baseline(now)
        cpu: dict[tuple[int, int], float] = {}
        if base is not None:
            dt = now - base[0]
            prev = base[1]
            if dt > 0:
                scale = 100.0 / (dt * CLK * NCPU)
                for pid, (start, t) in ticks.items():
                    p = prev.get(pid)
                    if p is not None and p[0] == start:
                        d = t - p[1]
                        if d > 0:
                            cpu[(pid, start)] = d * scale
        self.samples.append((now, ticks))
        # high CPU tracking (health)
        thresh = HIGH_CPU_CORE_PCT / NCPU
        for key, c in cpu.items():
            if c >= thresh:
                self.high_since.setdefault(key, now)
            else:
                self.high_since.pop(key, None)
        if base is not None:
            for key in list(self.high_since):
                if key not in cpu:
                    self.high_since.pop(key, None)
        self.last_cpu = cpu
        return cpu

    def sample_only(self) -> None:
        now, ticks = scan_ticks()
        with self._lock:
            self._record(now, ticks)

    # -- helpers ---------------------------------------------------------------
    def _windows(self) -> list[dict]:
        now = time.monotonic()
        if now - self.windows_cache[0] < 0.8:
            return self.windows_cache[1]
        wins: list = []
        api = self.api
        if api is not None and api.has("windows.list"):
            try:
                res = api.call("windows.list", {})
                if isinstance(res, dict) and res.get("supported"):
                    wins = [w for w in res.get("windows") or [] if isinstance(w, dict)]
            except Exception:  # noqa: BLE001
                wins = []
        self.windows_cache = (now, wins)
        return wins

    def unit_descriptions(self, max_age: float = 300.0) -> dict[str, str]:
        now = time.monotonic()
        if now - self.units_desc[0] < max_age and self.units_desc[1]:
            return self.units_desc[1]
        out = {}
        for u in list_user_units():
            out[u["unit"]] = u.get("description") or ""
        self.units_desc = (now, out)
        return out

    def _critical_pids(self, procs: dict[int, tuple]) -> set[int]:
        """The host, its ancestors and the session's critical processes."""
        crit = {1, MYPID}
        p = os.getppid()
        seen = 0
        while p > 1 and seen < 64:
            crit.add(p)
            info = procs.get(p)
            p = info[0] if info else 0
            seen += 1
        for pid, info in procs.items():
            if info[1] in _CRITICAL_COMM:
                crit.add(pid)
        return crit

    def _weights(self, cgroups: set[str]) -> dict[str, int | None]:
        now = time.monotonic()
        cached_t, cached = self.weights
        if now - cached_t > 5:
            cached = {}
            self.weights = (now, cached)
        out = {}
        for cg in cgroups:
            if cg not in cached:
                cached[cg] = _cpu_weight(cg)
            out[cg] = cached[cg]
        return out

    # -- main ------------------------------------------------------------------
    def processes(self, include_list: bool = True) -> dict:
        _DESKTOP.ensure()
        now, procs = scan_proc()
        wall = time.time()
        windows = self._windows()
        with self._lock:
            ticks = {pid: (p[6], p[3]) for pid, p in procs.items()}
            cpu = self._record(now, ticks)
            # static info (cmdline, cgroup, exe) for new processes
            static = self.static
            live: set[tuple[int, int]] = set()
            for pid, p in procs.items():
                key = (pid, p[6])
                live.add(key)
                if key not in static:
                    static[key] = _Static(pid, p[8])
            for key in [k for k in static if k not in live]:
                del static[key]
            # disk io for own processes
            io_rate: dict[int, tuple[float, float]] = {}
            new_io = {}
            for pid, p in procs.items():
                if p[8] != UID or p[1] == "kthreadd":
                    continue
                v = _io(pid)
                if v is None:
                    continue
                key = (pid, p[6])
                prev = self.io_prev.get(key)
                new_io[key] = (now, v[0], v[1])
                if prev and now > prev[0]:
                    dt = now - prev[0]
                    io_rate[pid] = (max(0, v[0] - prev[1]) / dt, max(0, v[1] - prev[2]) / dt)
            self.io_prev = new_io
            if now - self._crit_time > 10 or not self.critical:
                self.critical = self._critical_pids(procs)
                self._crit_time = now
            critical = self.critical

            # -- classification ------------------------------------------------
            app_of: dict[int, str] = {}
            apps_meta: dict[str, dict] = {}

            def app_meta(key: str, name: str, icon: str | None, desktop_id: str | None) -> str:
                if key not in apps_meta:
                    apps_meta[key] = {"name": name, "icon": icon, "desktopId": desktop_id}
                return key

            win_by_pid: dict[int, list[dict]] = {}
            for w in windows:
                try:
                    win_by_pid.setdefault(int(w.get("pid") or 0), []).append(w)
                except (TypeError, ValueError):
                    continue

            en = _lang() == "en"
            for pid, p in procs.items():
                if p[8] != UID or pid == MYPID:
                    continue
                st = static[(pid, p[6])]
                # our own app windows (python3 -m host.appwindow --app taskmgr)
                if "host.appwindow" in st.argv and "--app" in st.argv:
                    i = st.argv.index("--app")
                    own = st.argv[i + 1] if i + 1 < len(st.argv) else ""
                    if own in _OWN_APPS:
                        nl_name, en_name, icon = _OWN_APPS[own]
                        app_of[pid] = app_meta(f"win11shell-{own}.desktop", en_name if en else nl_name, icon,
                                               f"win11shell-{own}.desktop")
                        continue
                unit = st.unit
                if unit and unit[0] == "scope":
                    entry = _DESKTOP.lookup(unit[1])
                    if entry is not None and not entry["nodisplay"]:
                        app_of[pid] = app_meta(entry["id"], entry["name"], entry["icon"], entry["id"])
                        continue
                    if entry is None and unit[1].lower() not in _DESKTOP.autostart:
                        pretty = unit[1].rsplit(".", 1)[-1] if "." in unit[1] else unit[1]
                        app_of[pid] = app_meta("scope:" + unit[1], pretty[:1].upper() + pretty[1:], unit[1], None)
                        continue
                    # autostart helpers / hidden entries are background … unless they show a window
                if pid in win_by_pid:
                    w = win_by_pid[pid][0]
                    entry = _DESKTOP.lookup(w.get("desktopId")) or _DESKTOP.lookup(w.get("appId"))
                    if entry is not None:
                        app_of[pid] = app_meta(entry["id"], entry["name"], entry["icon"], entry["id"])
                    else:
                        aid = str(w.get("appId") or p[1])
                        app_of[pid] = app_meta("win:" + aid, aid, w.get("icon") or aid, w.get("desktopId"))

            # inheritance (children of an app belong to it) + executable match for unscoped processes
            memo: dict[int, str | None] = {}

            def resolve(pid: int, depth: int = 0) -> str | None:
                if pid in app_of:
                    return app_of[pid]
                if pid in memo:
                    return memo[pid]
                p = procs.get(pid)
                if p is None or p[8] != UID or pid == MYPID or depth > 64:
                    return None
                st = static[(pid, p[6])]
                res = None
                if st.unit and st.unit[0] in ("service", "unit") and not st.unit[2].startswith("session-"):
                    memo[pid] = None
                    return None
                parent = p[0]
                if parent > 1 and parent != MYPID and parent not in critical:
                    res = resolve(parent, depth + 1)
                if res is None and (st.unit is None or st.unit[2].startswith(("session-", "init.scope"))):
                    exe_name = os.path.basename(st.exe) if st.exe else p[1]
                    entry = _DESKTOP.by_executable(exe_name) or _DESKTOP.by_executable(p[1])
                    if entry is not None and p[1] not in _CRITICAL_COMM:
                        res = app_meta(entry["id"], entry["name"], entry["icon"], entry["id"])
                memo[pid] = res
                return res

            for pid, p in procs.items():
                if p[8] == UID and pid not in app_of and pid != MYPID:
                    r = resolve(pid)
                    if r is not None:
                        app_of[pid] = r
            # the host's own helper processes (WebKit etc.) belong to "Win11 Shell"
            host_children = set()
            for pid, p in procs.items():
                if p[8] == UID and pid not in app_of:
                    q, n = p[0], 0
                    while q > 1 and n < 32:
                        if q == MYPID:
                            host_children.add(pid)
                            break
                        if q in app_of:
                            break
                        qi = procs.get(q)
                        q = qi[0] if qi else 0
                        n += 1

            unit_desc = self.unit_descriptions()
            groups: dict[str, dict] = {}
            out_procs = []
            total_threads = 0
            weights = self._weights({static[(pid, p[6])].cgroup for pid, p in procs.items()
                                     if p[8] == UID and static[(pid, p[6])].unit is not None})

            for pid, p in procs.items():
                ppid, comm, state, _t, nice, threads, start, private, uid = p
                total_threads += threads
                key = (pid, start)
                st = static[key]
                c = round(cpu.get(key, 0.0), 2)
                rd, wr = io_rate.get(pid, (0.0, 0.0))
                kernel = pid == 2 or ppid == 2
                if pid == MYPID or pid in host_children:
                    group, gkey = "background", "win11shell"
                    gname, gicon, gdesk = _tr("host"), "preferences-desktop", None
                elif uid == UID and pid in app_of:
                    group, gkey = "app", app_of[pid]
                    meta = apps_meta[gkey]
                    gname, gicon, gdesk = meta["name"], meta["icon"], meta["desktopId"]
                elif uid == UID and not kernel:
                    group = "background"
                    unit = st.unit
                    exe_name = _exe_label(st, comm)
                    if unit and unit[0] in ("service", "unit") and unit[2].endswith(".service"):
                        gkey = "unit:" + unit[2]
                        gname = (_unit_friendly(unit[2]) or _friendly(exe_name, comm)
                                 or _clean_desc(unit_desc.get(unit[2])) or exe_name)
                    elif unit and unit[0] == "scope":  # autostart / helper scope
                        entry = _DESKTOP.lookup(unit[1])
                        gkey = "scope:" + unit[1]
                        gname = (_unit_friendly(unit[1]) or _friendly(exe_name, comm)
                                 or (entry["name"] if entry else None) or exe_name)
                    else:
                        gkey = "exe:" + exe_name
                        gname = _friendly(exe_name, comm) or exe_name
                    entry = _DESKTOP.by_executable(comm) if gkey.startswith("exe:") else (
                        _DESKTOP.lookup(unit[1]) if unit and unit[0] == "scope" else None)
                    gicon = entry["icon"] if entry else None
                    gdesk = entry["id"] if entry else None
                else:
                    group = "system"
                    if kernel:
                        gkey, gname = "kernel", _tr("kernel")
                    else:
                        exe_name = _exe_label(st, comm)
                        gkey, gname = "exe:" + exe_name, _friendly(exe_name, comm) or exe_name
                    gicon, gdesk = None, None
                display = gname if group == "app" else (_friendly(os.path.basename(st.exe) if st.exe else None, comm)
                                                         or gname)
                own = uid == UID
                eff = nice >= 10
                if own and st.unit is not None:
                    w = weights.get(st.cgroup)
                    if w is not None and w < 100:
                        eff = True
                can_kill = own and not kernel and pid not in critical
                g = groups.get(gkey)
                if g is None:
                    g = groups[gkey] = {"key": gkey, "name": gname, "icon": gicon, "desktopId": gdesk, "pids": [],
                                        "cpu": 0.0, "memory": 0, "diskRate": 0.0, "windows": [], "status": "running",
                                        "_group": group, "_states": set(), "_eff": True, "_kill": False,
                                        "threads": 0, "user": st.user}
                g["pids"].append(pid)
                g["cpu"] += c
                g["memory"] += private
                g["diskRate"] += rd + wr
                g["threads"] += threads
                g["_states"].add(state)
                g["_eff"] = g["_eff"] and eff
                g["_kill"] = g["_kill"] or can_kill
                if pid in win_by_pid:
                    for w in win_by_pid[pid]:
                        title = w.get("title")
                        if title and title not in g["windows"]:
                            g["windows"].append(title)
                if include_list:
                    out_procs.append({
                        "pid": pid, "ppid": ppid, "name": comm, "displayName": display, "cmd": st.cmd,
                        "user": st.user, "state": _STATE_MAP.get(state, "sleeping"), "cpu": c, "memory": private,
                        "diskRead": round(rd), "diskWrite": round(wr), "threads": threads,
                        "started": int((BOOT + start / CLK) * 1000), "group": group, "groupKey": gkey,
                        "canKill": can_kill, "efficiency": eff, "nice": nice, "exe": st.exe,
                        "critical": pid in critical,
                    })
                self.last_names[key] = (display, gkey)
            for key in [k for k in self.last_names if k not in live]:
                del self.last_names[key]

        apps, background, system = [], [], []
        for g in groups.values():
            states = g.pop("_states")
            grp = g.pop("_group")
            eff = g.pop("_eff")
            g["canKill"] = g.pop("_kill")
            g["cpu"] = round(min(100.0, g["cpu"]), 2)
            g["diskRate"] = round(g["diskRate"])
            g["efficiency"] = eff
            if states and states <= {"T", "t"}:
                g["status"] = "suspended"
            elif eff:
                g["status"] = "efficiency"
            g["pids"].sort()
            (apps if grp == "app" else background if grp == "background" else system).append(g)
        for lst in (apps, background, system):
            lst.sort(key=lambda g: str(g["name"]).lower())

        mem = read_meminfo()
        totals = self._totals(now, mem)
        totals["processes"] = len(procs)
        totals["threads"] = total_threads
        result = {"timestamp": int(wall * 1000), "totals": totals, "apps": apps, "background": background,
                  "system": system}
        if include_list:
            out_procs.sort(key=lambda x: x["pid"])
            result["processes"] = out_procs
        return result

    def _totals(self, now: float, mem: dict) -> dict:
        times = read_cpu_times()
        cpu_total = 0.0
        if times:
            busy, total = times[0]
            if self.cpu_prev and total > self.cpu_prev[1]:
                cpu_total = (busy - self.cpu_prev[0]) * 100.0 / (total - self.cpu_prev[1])
            self.cpu_prev = (busy, total)
        disks = read_diskstats()
        phys = set(physical_disks())
        rd = wr = 0.0
        active = 0.0
        if self.disk_prev:
            dt = now - self.disk_prev[0]
            if dt > 0:
                for name in phys:
                    a, b = disks.get(name), self.disk_prev[1].get(name)
                    if a and b:
                        rd += max(0, a[0] - b[0]) * 512 / dt
                        wr += max(0, a[1] - b[1]) * 512 / dt
                        active = max(active, min(100.0, max(0, a[2] - b[2]) / (dt * 10)))
        self.disk_prev = (now, disks)
        net = read_netdev()
        rx = tx = 0.0
        if self.net_prev:
            dt = now - self.net_prev[0]
            if dt > 0:
                for name, (r, t) in net.items():
                    if name.startswith(("lo", "veth", "docker", "br-", "virbr")):
                        continue
                    pr = self.net_prev[1].get(name)
                    if pr:
                        rx += max(0, r - pr[0]) / dt
                        tx += max(0, t - pr[1]) / dt
        self.net_prev = (now, net)
        total_mem = mem.get("MemTotal", 0)
        return {"cpu": round(max(0.0, min(100.0, cpu_total)), 1),
                "memoryUsed": max(0, total_mem - mem.get("MemAvailable", 0)), "memoryTotal": total_mem,
                "processes": 0, "threads": 0, "diskRead": round(rd), "diskWrite": round(wr),
                "diskActive": round(active, 1), "networkRx": round(rx), "networkTx": round(tx), "cpuCount": NCPU}

    def high_cpu(self, min_seconds: float) -> list[dict]:
        now = time.monotonic()
        out = []
        with self._lock:
            for key, since in self.high_since.items():
                if now - since >= min_seconds:
                    name, gkey = self.last_names.get(key, (None, None))
                    if name is None:
                        data = _read_bytes(f"/proc/{key[0]}/comm") or b"?"
                        name = data.decode("utf-8", "replace").strip()
                    out.append({"pid": key[0], "name": name, "since": now - since,
                                "cpu": self.last_cpu.get(key, 0.0), "groupKey": gkey})
        out.sort(key=lambda x: -x["cpu"])
        return out


def _exe_label(st: _Static, comm: str) -> str:
    """Executable name; for interpreters the script name (python3 /usr/bin/foo → foo)."""
    name = os.path.basename(st.exe).replace(" (deleted)", "") if st.exe else comm
    if (name in _INTERPRETERS or re.match(r"^python3(\.\d+)?$", name)) and len(st.argv) > 1:
        for a in st.argv[1:]:
            if a.startswith("-"):
                if a == "-m":
                    continue
                continue
            base = os.path.basename(a)
            if base and not base.startswith("-"):
                return base[:60]
            break
    return name or comm


def _clean_desc(desc: str | None) -> str | None:
    if not desc:
        return None
    return desc.strip() or None


# ---------------------------------------------------------------------------
# physical disks / network adapters
# ---------------------------------------------------------------------------

_phys_cache: tuple[float, list[str]] = (0.0, [])


def physical_disks() -> list[str]:
    global _phys_cache
    now = time.monotonic()
    if now - _phys_cache[0] < 30:
        return _phys_cache[1]
    out = []
    try:
        for n in os.listdir("/sys/block"):
            if n.startswith(("loop", "ram", "zram", "dm-", "md", "sr", "fd", "nbd")):
                continue
            if n.startswith("mmcblk") and ("boot" in n or "rpmb" in n):
                continue
            if not os.path.exists(f"/sys/block/{n}/device"):
                continue
            if _int(_read(f"/sys/block/{n}/size"), 0) == 0:
                continue
            out.append(n)
    except OSError:
        pass
    out.sort(key=lambda s: (not s.startswith("nvme"), [int(t) if t.isdigit() else t for t in re.split(r"(\d+)", s)]))
    _phys_cache = (now, out)
    return out


def _mount_map() -> dict[str, list[str]]:
    """{'maj:min': [mount points]} from mountinfo (real filesystems only)."""
    out: dict[str, list[str]] = {}
    data = _read_bytes("/proc/self/mountinfo", 1 << 20) or b""
    for line in data.decode("utf-8", "replace").splitlines():
        parts = line.split(" ")
        if len(parts) < 10:
            continue
        try:
            sep = parts.index("-")
        except ValueError:
            continue
        fstype = parts[sep + 1]
        if fstype in ("squashfs", "overlay", "tmpfs", "proc", "sysfs", "devtmpfs") or parts[4].startswith("/snap/"):
            continue
        mp = parts[4].replace("\\040", " ")
        if mp.startswith(("/var/lib/docker", "/run/user", "/proc", "/sys")):
            continue
        out.setdefault(parts[2], []).append(mp)
    return out


def _disk_mounts(disk: str, mm: dict[str, list[str]]) -> list[str]:
    mounts: list[str] = []
    devs = [disk]
    try:
        devs += [p for p in os.listdir(f"/sys/block/{disk}") if p.startswith(disk)]
    except OSError:
        pass
    # device-mapper children (LUKS / LVM) of this disk or its partitions
    try:
        for dm in os.listdir("/sys/block"):
            if not dm.startswith("dm-"):
                continue
            slaves = os.listdir(f"/sys/block/{dm}/slaves")
            if any(s == disk or s.startswith(disk) for s in slaves):
                devs.append(dm)
    except OSError:
        pass
    for d in devs:
        base = f"/sys/block/{disk}/{d}" if d != disk and not d.startswith("dm-") else f"/sys/block/{d}"
        dev = _read(f"{base}/dev")
        if dev:
            for m in mm.get(dev, []):
                if m not in mounts:
                    mounts.append(m)
    mounts.sort(key=lambda m: (m != "/", len(m), m))
    return mounts


_disk_static: dict[str, dict] = {}


def _udev_model(name: str) -> str | None:
    """Full model name from the udev database (sysfs truncates SCSI models to 16 chars, USB bridges blank)."""
    dev = _read(f"/sys/block/{name}/dev")
    data = _read(f"/run/udev/data/b{dev}") if dev else None
    if not data:
        return None
    enc = re.search(r"^E:ID_MODEL_ENC=(.*)$", data, re.M)
    if enc:
        v = re.sub(r"\\x([0-9a-fA-F]{2})", lambda m: chr(int(m.group(1), 16)), enc.group(1)).strip()
        if v:
            return re.sub(r"\s+", " ", v)
    plain = re.search(r"^E:ID_MODEL=(.*)$", data, re.M)
    return plain.group(1).replace("_", " ").strip() if plain else None


def _disk_info(name: str) -> dict:
    info = _disk_static.get(name)
    if info is None:
        rot = _read(f"/sys/block/{name}/queue/rotational", "0") == "1"
        try:
            real = os.path.realpath(f"/sys/block/{name}")
        except OSError:
            real = ""
        model = _udev_model(name) or (_read(f"/sys/block/{name}/device/model")
                                      or _read(f"/sys/block/{name}/device/name") or "").strip() or name
        vendor = (_read(f"/sys/block/{name}/device/vendor") or "").strip()
        if _udev_model(name):
            vendor = ""
        if vendor and vendor.upper() not in ("ATA", "NVME") and not model.lower().startswith(vendor.lower()):
            model = f"{vendor} {model}"
        kind = "nvme" if name.startswith("nvme") else "usb" if "/usb" in real else "hdd" if rot else "ssd"
        info = {"model": model, "kind": kind, "removable": _read(f"/sys/block/{name}/removable", "0") == "1"}
        _disk_static[name] = info
    return info


_ip_cache: tuple[float, dict] = (0.0, {})


def _ip_addrs() -> dict[str, dict]:
    global _ip_cache
    now = time.monotonic()
    if now - _ip_cache[0] < 10:
        return _ip_cache[1]
    out: dict[str, dict] = {}
    r = run(["ip", "-j", "addr"], timeout=3)
    data = json_loads(r.out, []) if r.ok else []
    for itf in data or []:
        name = itf.get("ifname")
        v4 = v6 = v6ll = None
        for a in itf.get("addr_info", []):
            if a.get("family") == "inet" and not v4:
                v4 = a.get("local")
            elif a.get("family") == "inet6":
                if a.get("scope") == "global" and not v6:
                    v6 = a.get("local")
                elif a.get("scope") == "link" and not v6ll:
                    v6ll = a.get("local")
        out[name] = {"ipv4": v4, "ipv6": v6 or v6ll, "mac": itf.get("address"),
                     "state": itf.get("operstate")}
    _ip_cache = (now, out)
    return out


def network_adapters() -> list[dict]:
    out = []
    try:
        names = sorted(os.listdir("/sys/class/net"))
    except OSError:
        return out
    for n in names:
        base = f"/sys/class/net/{n}"
        if os.path.isdir(f"{base}/wireless") or os.path.isdir(f"{base}/phy80211"):
            kind = "wifi"
        elif os.path.exists(f"{base}/device"):
            kind = "ethernet"
        elif n.startswith(("tailscale", "wg", "tun", "zt", "nordlynx", "proton", "ppp")):
            if _read(f"{base}/operstate", "down") == "down":
                continue
            kind = "other"
        else:
            continue
        slot = ""
        try:
            slot = os.path.basename(os.path.realpath(f"{base}/device")) if kind != "other" else ""
        except OSError:
            pass
        out.append({"iface": n, "kind": kind, "slot": slot})
    return out


# ---------------------------------------------------------------------------
# performance
# ---------------------------------------------------------------------------

class Performance:
    def __init__(self, procmon: ProcessMonitor):
        self.pm = procmon
        self._lock = threading.Lock()
        self.cpu_prev: list[tuple[int, int]] | None = None
        self.disk_prev: tuple[float, dict] | None = None
        self.net_prev: tuple[float, dict] | None = None
        self.cpuinfo: dict | None = None
        self.prime()

    def prime(self) -> None:
        with self._lock:
            self.cpu_prev = read_cpu_times()
            self.disk_prev = (time.monotonic(), read_diskstats())
            self.net_prev = (time.monotonic(), read_netdev())

    def _primary(self) -> dict | None:
        cached = getattr(self, "_prim", (0.0, None))
        if time.monotonic() - cached[0] < 5:
            return cached[1]
        prim = None
        try:
            if _api is not None and _api.has("system.network.get"):
                prim = _api.call("system.network.get", {}).get("primary")
        except Exception:  # noqa: BLE001
            prim = None
        self._prim = (time.monotonic(), prim)
        return prim

    def _cpu_static(self) -> dict:
        if self.cpuinfo is None:
            from .system import cpu_info
            self.cpuinfo = cpu_info()
            self.cpuinfo["baseMHz"] = None
            mx = _int(_read("/sys/devices/system/cpu/cpu0/cpufreq/cpuinfo_max_freq"), None)
            base = _int(_read("/sys/devices/system/cpu/cpu0/cpufreq/base_frequency"), None)
            self.cpuinfo["maxMHz"] = round(mx / 1000) if mx else None
            self.cpuinfo["baseMHz"] = round(base / 1000) if base else None
        return self.cpuinfo

    def get(self) -> dict:
        now = time.monotonic()
        wall = time.time()
        with self._lock:
            times = read_cpu_times()
            prev = self.cpu_prev or times
            usage = []
            for i, (busy, total) in enumerate(times):
                pb, pt = prev[i] if i < len(prev) else (busy, total)
                usage.append(round(max(0.0, min(100.0, (busy - pb) * 100.0 / (total - pt))), 1)
                             if total > pt else 0.0)
            self.cpu_prev = times
            disks_now = read_diskstats()
            dprev = self.disk_prev
            self.disk_prev = (now, disks_now)
            net_now = read_netdev()
            nprev = self.net_prev
            self.net_prev = (now, net_now)
        freqs = []
        for i in range(NCPU):
            v = _read_bytes(f"/sys/devices/system/cpu/cpu{i}/cpufreq/scaling_cur_freq", 64)
            if v:
                try:
                    freqs.append(int(v) / 1000)
                except ValueError:
                    pass
        if not freqs:
            data = (_read_bytes("/proc/cpuinfo", 1 << 20) or b"").decode("utf-8", "replace")
            freqs = [float(m) for m in re.findall(r"cpu MHz\s*:\s*([0-9.]+)", data)]
        st = self._cpu_static()
        load = (_read("/proc/loadavg", "") or "").split()
        threads_total = _int(load[3].split("/")[1], None) if len(load) > 3 and "/" in load[3] else None
        nprocs = sum(1 for e in os.listdir("/proc") if e.isdigit())
        try:
            uptime = float((_read("/proc/uptime", "0") or "0").split()[0])
        except ValueError:
            uptime = 0.0
        mem = read_meminfo()
        total = mem.get("MemTotal", 0)
        avail = mem.get("MemAvailable", 0)
        cached = mem.get("Cached", 0) + mem.get("SReclaimable", 0) - mem.get("Shmem", 0)
        cpu = {
            "model": st["model"], "usage": usage[0] if usage else 0.0, "perCore": usage[1:],
            "freqMHz": round(sum(freqs) / len(freqs)) if freqs else None, "cores": st["cores"],
            "threads": st["threads"], "uptime": int(uptime), "processes": nprocs, "threadsTotal": threads_total,
            "temperature": cpu_temperature(), "maxMHz": st.get("maxMHz"), "baseMHz": st.get("baseMHz"),
            "load": [float(x) for x in load[:3]] if len(load) >= 3 else None,
        }
        memory = {
            "total": total, "used": max(0, total - avail), "available": avail, "cached": max(0, cached),
            "buffers": mem.get("Buffers", 0), "swapTotal": mem.get("SwapTotal", 0),
            "swapUsed": max(0, mem.get("SwapTotal", 0) - mem.get("SwapFree", 0)),
            "committed": mem.get("Committed_AS", 0), "commitLimit": mem.get("CommitLimit", 0),
            "shared": mem.get("Shmem", 0), "kernel": mem.get("Slab", 0) + mem.get("KernelStack", 0)
            + mem.get("PageTables", 0),
        }
        # disks
        mm = _mount_map()
        disks = []
        dt = now - dprev[0] if dprev else 0
        for idx, name in enumerate(physical_disks()):
            info = _disk_info(name)
            cur = disks_now.get(name)
            old = dprev[1].get(name) if dprev else None
            rbps = wbps = act = 0.0
            resp = None
            if cur and old and dt > 0:
                rbps = max(0, cur[0] - old[0]) * 512 / dt
                wbps = max(0, cur[1] - old[1]) * 512 / dt
                act = min(100.0, max(0, cur[2] - old[2]) / (dt * 10))
                ios = cur[3] - old[3]
                resp = round((cur[4] - old[4]) / ios, 2) if ios > 0 else 0.0
            disks.append({"name": _tr("disk_n", n=idx), "device": name, "index": idx, "model": info["model"],
                          "kind": info["kind"], "readBps": round(rbps), "writeBps": round(wbps),
                          "activePct": round(act, 1), "responseMs": resp,
                          "capacity": (_int(_read(f"/sys/block/{name}/size"), 0) or 0) * 512,
                          "mounts": _disk_mounts(name, mm), "removable": info["removable"]})
        # network
        ips = _ip_addrs()
        net = []
        dtn = now - nprev[0] if nprev else 0
        counts: dict[str, int] = {}
        primary_name = self._primary()
        for a in network_adapters():
            n = a["iface"]
            cur = net_now.get(n)
            old = nprev[1].get(n) if nprev else None
            rx = tx = 0.0
            if cur and old and dtn > 0:
                rx = max(0, cur[0] - old[0]) / dtn
                tx = max(0, cur[1] - old[1]) / dtn
            kind = a["kind"]
            counts[kind] = counts.get(kind, 0) + 1
            if kind == "wifi":
                label = _tr("wifi") if counts[kind] == 1 else _tr("wifi_n", n=counts[kind])
            elif kind == "ethernet":
                label = _tr("ethernet") if counts[kind] == 1 else _tr("ethernet_n", n=counts[kind])
            else:
                label = f"{_tr('vpn')} ({n})"
            ip = ips.get(n, {})
            speed = _int(_read(f"/sys/class/net/{n}/speed"), None)
            item = {"iface": n, "kind": kind, "name": label, "rxBps": round(rx), "txBps": round(tx),
                    "ipv4": ip.get("ipv4"), "ipv6": ip.get("ipv6"),
                    "connected": (_read(f"/sys/class/net/{n}/operstate", "down") in ("up", "unknown")),
                    "linkSpeedMbps": speed if speed and speed > 0 else None,
                    "adapter": _pci_name(a["slot"]) if a["slot"] and re.match(r"^[0-9a-f]{4}:", a["slot"]) else None}
            if isinstance(primary_name, dict) and primary_name.get("device") == n:
                item["connection"] = primary_name.get("name")
            net.append(item)
        return {"timestamp": int(wall * 1000), "cpu": cpu, "memory": memory, "disks": disks, "network": net,
                "gpus": _GPU.stats()}


# ---------------------------------------------------------------------------
# startup apps (XDG autostart)
# ---------------------------------------------------------------------------

def _parse_desktop(path: Path) -> dict[str, str]:
    out: dict[str, str] = {}
    try:
        text = path.read_text(encoding="utf-8", errors="replace")
    except OSError:
        return out
    group = None
    for line in text.splitlines():
        s = line.strip()
        if not s or s.startswith("#"):
            continue
        if s.startswith("[") and s.endswith("]"):
            group = s[1:-1]
            continue
        if group != "Desktop Entry" or "=" not in s:
            continue
        k, v = s.split("=", 1)
        out.setdefault(k.strip(), v.strip())
    return out


def _loc(d: dict[str, str], key: str) -> str:
    lang = _lang()
    for k in (f"{key}[{lang}]", f"{key}[{lang}_NL]" if lang == "nl" else f"{key}[{lang}_US]", key):
        if d.get(k):
            return d[k]
    return ""


def _current_desktops() -> list[str]:
    return [x.lower() for x in os.environ.get("XDG_CURRENT_DESKTOP", "").split(":") if x]


def _autostart_dirs() -> tuple[Path, list[Path]]:
    user = xdg_config_home() / "autostart"
    dirs = [Path(p) / "autostart" for p in (os.environ.get("XDG_CONFIG_DIRS") or "/etc/xdg").split(":") if p]
    return user, dirs


def _entry_shows(d: dict[str, str], desktops: list[str]) -> bool:
    only = [x.lower() for x in d.get("OnlyShowIn", "").split(";") if x]
    notin = [x.lower() for x in d.get("NotShowIn", "").split(";") if x]
    if only and not any(x in desktops for x in only):
        return False
    if notin and any(x in desktops for x in notin):
        return False
    te = d.get("TryExec")
    if te and not (os.path.isabs(te) and os.access(te, os.X_OK)) and not which(te):
        return False
    return True


def _entry_enabled(d: dict[str, str]) -> bool:
    return d.get("Hidden", "false").lower() != "true" and \
        d.get("X-GNOME-Autostart-enabled", "true").lower() != "false"


_ID_RE = re.compile(r"^[A-Za-z0-9._@+\-]{1,200}\.desktop$")


def startup_list(include_hidden: bool = False) -> list[dict]:
    """Autostart apps. Like GNOME's "Opstarttoepassingen", entries marked NoDisplay=true (session plumbing such
    as the keyring or accessibility bus) are left out unless ``include_hidden``; they come back with
    ``hidden: true``."""
    user_dir, sys_dirs = _autostart_dirs()
    desktops = _current_desktops()
    files: dict[str, tuple[str, Path]] = {}
    for d in reversed(sys_dirs):
        try:
            for f in d.iterdir():
                if f.suffix == ".desktop":
                    files[f.name] = ("system", f)
        except OSError:
            continue
    sys_files = dict(files)
    try:
        for f in user_dir.iterdir():
            if f.suffix == ".desktop":
                files[f.name] = ("user", f)
    except OSError:
        pass
    out = []
    for fid, (source, path) in files.items():
        d = _parse_desktop(path)
        if not d or d.get("Type", "Application") != "Application":
            continue
        merged = dict(d)
        if source == "user" and fid in sys_files and (not d.get("Exec") or not d.get("Name")):
            base = _parse_desktop(sys_files[fid][1])
            base.update(d)
            merged = base
        if not _entry_shows(merged, desktops):
            continue
        hidden = merged.get("NoDisplay", "false").lower() == "true" or bool(merged.get("X-GNOME-Autostart-Phase"))
        if hidden and not include_hidden:
            continue
        if source == "user" and merged.get("Hidden", "").lower() == "true" and fid not in sys_files \
                and not merged.get("Exec"):
            continue
        name = _loc(merged, "Name") or fid[:-8]
        out.append({"id": fid, "name": name, "icon": merged.get("Icon") or "application-x-executable",
                    "enabled": _entry_enabled(merged), "command": merged.get("Exec", ""),
                    "source": "system" if fid in sys_files else "user",
                    "description": _loc(merged, "Comment") or None,
                    "systemDefault": fid in sys_files, "path": str(path), "hidden": hidden})
    out.sort(key=lambda x: x["name"].lower())
    return out


def _set_key(text: str, key: str, value: str) -> str:
    """Set ``key=value`` in the [Desktop Entry] group (replacing every existing ``key=`` line). Pure (tested)."""
    lines = text.splitlines()
    start = end = None
    for i, line in enumerate(lines):
        s = line.strip()
        if s.startswith("[") and s.endswith("]"):
            if start is not None and end is None:
                end = i
            if s == "[Desktop Entry]" and start is None:
                start = i
    if start is None:
        return f"[Desktop Entry]\n{key}={value}\n" + (("\n" + text.strip("\n") + "\n") if text.strip() else "")
    if end is None:
        end = len(lines)
    body = [ln for ln in lines[start + 1:end] if not (("=" in ln) and ln.split("=", 1)[0].strip() == key)]
    while body and not body[-1].strip():
        body.pop()
    tail = lines[end:]
    out = lines[:start + 1] + body + [f"{key}={value}"] + ([""] if tail else []) + tail
    return "\n".join(out).rstrip("\n") + "\n"


def set_startup(fid: Any, enabled: bool) -> dict:
    if not isinstance(fid, str) or not _ID_RE.match(fid) or "/" in fid:
        raise ApiError("bad_args", "Onbekend opstartitem.")
    user_dir, sys_dirs = _autostart_dirs()
    user_file = user_dir / fid
    src = None
    if user_file.is_file():
        src = user_file
    else:
        for d in sys_dirs:
            if (d / fid).is_file():
                src = d / fid
                break
    if src is None:
        raise ApiError("not_found", "Dit opstartitem bestaat niet meer.")
    try:
        text = src.read_text(encoding="utf-8", errors="replace")
    except OSError:
        raise ApiError("failed", "Dit opstartitem kon niet worden gelezen.") from None
    text = _set_key(text, "Hidden", "false" if enabled else "true")
    text = _set_key(text, "X-GNOME-Autostart-enabled", "true" if enabled else "false")
    try:
        user_dir.mkdir(parents=True, exist_ok=True)
        atomic_write_text(user_file, text, mode=0o644)
    except OSError as e:
        log.warning("Opstartitem schrijven mislukt (write failed): %s", e)
        raise ApiError("failed", "De instelling kon niet worden opgeslagen.") from None
    return {"id": fid, "enabled": enabled}


# ---------------------------------------------------------------------------
# user services
# ---------------------------------------------------------------------------

def list_user_units(state: str | None = None) -> list[dict]:
    cmd = ["systemctl", "--user", "list-units", "--type=service", "--all", "--no-pager"]
    if state:
        cmd = ["systemctl", "--user", "list-units", "--type=service", f"--state={state}", "--no-pager"]
    r = run(cmd + ["--output=json"], timeout=6)
    if r.ok:
        data = json_loads(r.out, None)
        if isinstance(data, list):
            return data
    r = run(cmd + ["--plain", "--no-legend"], timeout=6)
    out = []
    for line in r.out.splitlines() if r.ok else []:
        p = line.split(None, 4)
        if len(p) >= 4:
            out.append({"unit": p[0], "load": p[1], "active": p[2], "sub": p[3], "description": p[4] if len(p) > 4 else ""})
    return out


_UNIT_RE = re.compile(r"^[A-Za-z0-9:_.@\\\-]{1,250}\.service$")


_unit_files: tuple[float, dict[str, str]] = (0.0, {})


def _unit_file_states(max_age: float = 120.0) -> dict[str, str]:
    """{unit file: enabled|disabled|static|…} (≈200 ms, so cached)."""
    global _unit_files
    if _unit_files[1] and time.monotonic() - _unit_files[0] < max_age:
        return _unit_files[1]
    out: dict[str, str] = {}
    r = run(["systemctl", "--user", "list-unit-files", "--type=service", "--no-pager", "--output=json"], timeout=6)
    if r.ok:
        for u in json_loads(r.out, []) or []:
            out[u.get("unit_file", "")] = u.get("state", "")
    _unit_files = (time.monotonic(), out)
    return out


def services_list(pm: ProcessMonitor) -> list[dict]:
    units = list_user_units()
    enabled = _unit_file_states()
    pid_of: dict[str, int] = {}
    with pm._lock:
        for (pid, _s), st in pm.static.items():
            if st.unit and st.unit[2].endswith(".service"):
                if st.unit[2] not in pid_of or pid < pid_of[st.unit[2]]:
                    pid_of[st.unit[2]] = pid
    out = []
    for u in units:
        name = u.get("unit", "")
        if not name.endswith(".service"):
            continue
        templ = re.sub(r"@[^.]*\.service$", "@.service", name)
        out.append({"name": name, "description": u.get("description") or _unescape_unit(name[:-8]),
                    "state": u.get("active", ""), "sub": u.get("sub", ""), "scope": "user",
                    "load": u.get("load", ""), "enabled": enabled.get(name) or enabled.get(templ) or None,
                    "pid": pid_of.get(name), "critical": bool(_CRITICAL_UNITS.match(name)),
                    "displayName": _unescape_unit(name[:-8])})
    out.sort(key=lambda x: x["displayName"].lower())
    return out


def service_action(name: Any, action: Any) -> dict:
    if not isinstance(name, str):
        raise ApiError("bad_args", "Onbekende service.")
    if not name.endswith(".service"):
        name += ".service"
    if not _UNIT_RE.match(name) or name.startswith("-"):
        raise ApiError("bad_args", "Onbekende service.")
    if action not in ("start", "stop", "restart"):
        raise ApiError("bad_args", "Onbekende actie.")
    if action != "start" and _CRITICAL_UNITS.match(name):
        raise ApiError("protected", "Deze service is nodig voor je bureaublad en kan hier niet worden gestopt.")
    global _unit_files
    _unit_files = (0.0, {})
    r = run(["systemctl", "--user", action, "--", name], timeout=25)
    if not r.ok:
        log.info("systemctl --user %s %s: %s", action, name, r.err.strip()[:200])
        raise ApiError("failed", {"start": "Starten lukte niet.", "stop": "Stoppen lukte niet.",
                                  "restart": "Opnieuw starten lukte niet."}[action])
    return {"ok": True}


# ---------------------------------------------------------------------------
# end task / efficiency
# ---------------------------------------------------------------------------

def _pid_list(v: Any) -> list[int]:
    if isinstance(v, (int, str)):
        v = [v]
    if not isinstance(v, list) or not v or len(v) > 2000:
        raise ApiError("bad_args", "Geen processen opgegeven.")
    out = []
    for p in v:
        try:
            n = int(p)
        except (TypeError, ValueError):
            raise ApiError("bad_args", "Ongeldig proces.") from None
        if n <= 0:
            raise ApiError("bad_args", "Ongeldig proces.")
        if n not in out:
            out.append(n)
    return out


def _check_own(pid: int, pm: ProcessMonitor) -> str | None:
    """Error message (Dutch) when we must not touch ``pid``, else None."""
    if pid in (1, MYPID):
        return "Dit proces is nodig voor je bureaublad en kan niet worden beëindigd."
    try:
        st = os.stat(f"/proc/{pid}")
    except OSError:
        return "Dit proces bestaat niet meer."
    if st.st_uid != UID:
        return "Dit proces is van een andere gebruiker of van het systeem."
    stat = _read_bytes(f"/proc/{pid}/stat", 2048) or b""
    rp = stat.rfind(b")")
    comm = stat[stat.find(b"(") + 1:rp].decode("utf-8", "replace")
    f = stat[rp + 2:].split()
    if len(f) > 1 and f[1] == b"2" or pid == 2:
        return "Dit is een onderdeel van het systeem."
    crit = pm.critical or set()
    if pid in crit or comm in _CRITICAL_COMM:
        return "Dit proces is nodig voor je bureaublad en kan niet worden beëindigd."
    # never the host's ancestors
    p, n = os.getppid(), 0
    while p > 1 and n < 64:
        if p == pid:
            return "Dit proces is nodig voor je bureaublad en kan niet worden beëindigd."
        data = _read_bytes(f"/proc/{p}/stat", 2048) or b""
        rp2 = data.rfind(b")")
        try:
            p = int(data[rp2 + 2:].split()[1])
        except (ValueError, IndexError):
            break
        n += 1
    return None


def end_task(pids: list[int], force: bool, pm: ProcessMonitor) -> dict:
    results = []
    for pid in pids:
        err = _check_own(pid, pm)
        if err:
            results.append({"pid": pid, "ok": False, "error": err})
            continue
        try:
            os.kill(pid, signal.SIGKILL if force else signal.SIGTERM)
            results.append({"pid": pid, "ok": True, "error": None})
        except ProcessLookupError:
            results.append({"pid": pid, "ok": True, "error": None})  # already gone
        except PermissionError:
            results.append({"pid": pid, "ok": False, "error": "Hiervoor heb je geen toestemming."})
        except OSError as e:
            results.append({"pid": pid, "ok": False, "error": f"Beëindigen lukte niet ({e.strerror})."})
    return {"results": results}


# Win11 Shell's own high-priority scopes (taskbar host, Taakbeheer, the fallback Taakbeheer's host): they keep their
# priority over every other app, always (SPEC §2.1) — efficiency mode on them is refused.
_OWN_HIGH_UNITS = re.compile(r"^(win11shell-shell-|win11shell-apphost-|app-win11shell-taskmgr-)[0-9]+\.scope$")
_EFF_PREV: dict[str, str] = {}      # unit → its CPUWeight before efficiency mode (restored when it is turned off)


def _unit_cpu_weight(unit: str) -> str:
    """The unit's CPUWeight as systemd has it ('' when not set or unknown)."""
    r = run(["systemctl", "--user", "show", "-p", "CPUWeight", "--value", "--", unit], timeout=4)
    v = (r.out or "").strip() if r.ok else ""
    return v if v.isdigit() else ""


def set_efficiency(pids: list[int], enabled: bool, pm: ProcessMonitor) -> dict:
    """Efficiency mode: CPUWeight=10 on the app's own systemd scope (reversible, no root) when the process
    lives in an app scope/service of the user manager — turning it off gives the unit back the weight it had
    (default 100); otherwise renice every thread to +10 (Linux does not allow lowering it again without
    privileges, so turning it off may fail there). Never on Win11 Shell's own high-priority scopes."""
    results = []
    done_units: dict[str, tuple[bool, str | None]] = {}
    for pid in pids:
        err = _check_own(pid, pm)
        if err:
            results.append({"pid": pid, "ok": False, "error": err})
            continue
        cg = (_read_bytes(f"/proc/{pid}/cgroup", 4096) or b"").decode("utf-8", "replace")
        line = next((ln for ln in cg.splitlines() if ln.startswith("0::")), "")
        unit = parse_cgroup_unit(line.split(":", 2)[-1] if line else "")
        if unit and "/user@" in line and unit[2].endswith((".scope", ".service")) and unit[2] != "init.scope":
            uname = unit[2]
            if _CRITICAL_UNITS.match(uname):
                results.append({"pid": pid, "ok": False, "method": "cgroup",
                                "error": "Dit proces is nodig voor je bureaublad en kan niet worden afgeremd."})
                continue
            if _OWN_HIGH_UNITS.match(uname):
                results.append({"pid": pid, "ok": False, "method": "cgroup",
                                "error": "Dit hoort bij Win11 Shell (taakbalk of Taakbeheer). Dat houdt altijd "
                                         "voorrang en kan niet worden afgeremd."})
                continue
            if uname not in done_units:
                if enabled:
                    prev = _unit_cpu_weight(uname)
                    if prev and prev != "10":
                        _EFF_PREV.setdefault(uname, prev)
                    weight = "10"
                else:
                    weight = _EFF_PREV.pop(uname, "") or "100"
                r = run(["systemctl", "--user", "set-property", "--runtime", "--", uname,
                         f"CPUWeight={weight}"], timeout=8)
                done_units[uname] = (r.ok, None if r.ok else "Efficiëntiemodus aanpassen lukte niet.")
            ok, e = done_units[uname]
            results.append({"pid": pid, "ok": ok, "error": e, "method": "cgroup"})
            continue
        ok, error = True, None
        try:
            tids = [int(t) for t in os.listdir(f"/proc/{pid}/task")]
        except OSError:
            tids = [pid]
        for tid in tids:
            try:
                os.setpriority(os.PRIO_PROCESS, tid, 10 if enabled else 0)
            except PermissionError:
                ok, error = False, "Terugzetten kan alleen door de app opnieuw te starten."
                break
            except OSError:
                continue
        results.append({"pid": pid, "ok": ok, "error": error, "method": "nice"})
    pm.weights = (0.0, {})
    return {"results": results}


# ---------------------------------------------------------------------------
# restart a process (Taakbeheer › Opnieuw starten)
# ---------------------------------------------------------------------------

RESTART_TERM_S = 5.0
RESTART_KILL_S = 2.0
# processes of the shell itself (children of the host): restarting them alone would break the shell
_HOST_HELPERS = ("host.indexer", "thumbnailer.py", "wl-paste", "pw-dump", "glycin", "hyprsunset", "xdg-dbus-proxy")


def _argv_of(pid: int) -> list[str]:
    raw = _read_bytes(f"/proc/{pid}/cmdline", 65536) or b""
    return [a.decode("utf-8", "replace") for a in raw.split(b"\0") if a]


def _stat_of(pid: int) -> tuple[str, list[bytes]] | None:
    """(comm, fields after comm: [state, ppid, pgrp, session, tty_nr, …, starttime at 19])."""
    data = _read_bytes(f"/proc/{pid}/stat", 2048)
    if not data:
        return None
    rp = data.rfind(b")")
    return data[data.find(b"(") + 1:rp].decode("utf-8", "replace"), data[rp + 2:].split()


def _status_ids(pid: int) -> tuple[list[int], list[int]] | None:
    """(real, effective, saved, fs uid), (… gid) from /proc/<pid>/status."""
    data = _read_bytes(f"/proc/{pid}/status", 16384)
    if not data:
        return None
    uids = gids = None
    for line in data.decode("utf-8", "replace").splitlines():
        if line.startswith("Uid:"):
            uids = [int(x) for x in line.split()[1:5]]
        elif line.startswith("Gid:"):
            gids = [int(x) for x in line.split()[1:5]]
    return (uids, gids) if uids and gids else None


def _environ_of(pid: int) -> dict[str, str] | None:
    try:
        with open(f"/proc/{pid}/environ", "rb") as f:
            raw = f.read(1 << 20)
    except OSError:
        return None
    env = {}
    for item in raw.split(b"\0"):
        if b"=" in item:
            k, _, v = item.partition(b"=")
            env[k.decode("utf-8", "surrogateescape")] = v.decode("utf-8", "surrogateescape")
    return env


def _same_mount_ns(pid: int) -> bool:
    try:
        return os.readlink(f"/proc/{pid}/ns/mnt") == os.readlink("/proc/self/ns/mnt")
    except OSError:
        return False


def _ancestors(pid: int, limit: int = 64) -> list[int]:
    out, p = [], pid
    while len(out) < limit:
        st = _stat_of(p)
        if st is None:
            break
        try:
            p = int(st[1][1])
        except (ValueError, IndexError):
            break
        if p <= 1:
            break
        out.append(p)
    return out


def _own_window_of(pid: int) -> tuple[str, int] | None:
    """('taskmgr', appwindow pid) when ``pid`` is one of our app windows or one of its WebKit helpers."""
    for p in [pid] + _ancestors(pid, 8):
        argv = _argv_of(p)
        if "host.appwindow" in argv and "--app" in argv:
            i = argv.index("--app")
            app = argv[i + 1] if i + 1 < len(argv) else ""
            if app in _OWN_APPS:
                return app, p
        if p == MYPID:
            break
    return None


def _host_part(pid: int, comm: str, argv: list[str], mypid: int) -> str | None:
    """'webkit' — the host's own WebKit processes (only WebKit/bwrap between them and the host); 'helper' — one
    of its helper processes (indexer, image loaders, monitors); None — anything else (also apps it launched)."""
    chain, p = [], pid
    for _ in range(4):
        st = _stat_of(p)
        if st is None:
            return None
        try:
            pp = int(st[1][1])
        except (ValueError, IndexError):
            return None
        chain.append(st[0])
        if pp == mypid:
            break
        p = pp
    else:
        return None
    if pp != mypid:
        return None
    joined = " ".join(argv).lower()
    if all(c.startswith("WebKit") or c in ("bwrap", "xdg-dbus-proxy") for c in chain) and \
            (comm.startswith("WebKit") or "webkit" in joined):
        return "webkit"
    if len(chain) <= 2 and any(h in joined for h in _HOST_HELPERS):
        return "helper"
    return None


def _refuse(code: str, message: str, data: Any = None):
    raise ApiError(code, message, data)


def restart_plan(pid: int, pm: "ProcessMonitor", mypid: int | None = None) -> dict:
    """What "restart" means for ``pid`` — no side effects. → {how: 'shell'|'app'|'service'|'exec', name, …};
    raises ApiError (Dutch message) when it cannot or must not be done."""
    mypid = mypid or MYPID
    if pid == mypid:
        return {"how": "shell", "pids": [], "name": _tr("host")}
    err = _check_own(pid, pm)
    if err:
        _refuse("not_found" if "bestaat niet" in err else "protected", err, {"pid": pid})
    st = _stat_of(pid)
    if st is None:
        _refuse("not_found", "Dit proces bestaat niet meer.", {"pid": pid})
    comm, fields = st
    argv = _argv_of(pid)
    if not argv:
        _refuse("unsupported", "Dit proces kan niet opnieuw worden gestart (het heeft geen opdrachtregel).",
                {"pid": pid})
    own = _own_window_of(pid)
    if own is not None:
        app, wpid = own
        nl, en, _icon = _OWN_APPS[app]
        return {"how": "app", "desktopId": f"win11shell-{app}.desktop", "pids": [wpid], "root": wpid,
                "ownWindow": True, "name": en if _lang() == "en" else nl,
                "via": None if pid == wpid else (comm or None)}
    part = _host_part(pid, comm, argv, mypid)
    if part == "webkit":                  # the taskbar's own web content: restarting it = restarting the shell
        return {"how": "shell", "pids": [], "name": _tr("host")}
    if part == "helper":
        _refuse("protected", "Dit is een onderdeel van Win11 Shell. Start Win11 Shell opnieuw om het "
                             "opnieuw te starten.", {"pid": pid})
    cg = (_read_bytes(f"/proc/{pid}/cgroup", 4096) or b"").decode("utf-8", "replace")
    line = next((ln for ln in cg.splitlines() if ln.startswith("0::")), "")
    cgroup = line.split(":", 2)[-1] if line else ""
    unit = parse_cgroup_unit(cgroup)
    if unit and "/user@" in cgroup and unit[2].endswith(".service") and not unit[2].startswith("app-"):
        if _CRITICAL_UNITS.match(unit[2]):
            _refuse("protected", "Deze service is nodig voor je bureaublad en kan hier niet opnieuw worden gestart.",
                    {"pid": pid, "unit": unit[2]})
        return {"how": "service", "unit": unit[2], "pids": [], "name": _unit_friendly(unit[2]) or unit[2]}
    data = pm.processes(include_list=False)
    for g in data.get("apps") or []:
        did = g.get("desktopId")
        if pid in (g.get("pids") or []) and did and str(did).endswith(".desktop"):
            members = {p for p in g["pids"] if _check_own(p, pm) is None}
            if not members:
                continue
            root, tree = app_tree(pid, members)
            if pid not in tree:
                # a program of its own that only shares the app's scope: something run in a terminal opened from
                # Verkenner, a browser started from that terminal — restart just this process (the terminal
                # question follows), never the whole terminal with every tab and editor in it
                return _exec_plan(pid, comm, fields, argv, mypid)
            return {"how": "app", "desktopId": did, "pids": tree, "root": root, "name": g.get("name") or did,
                    "via": None if pid == root else (comm or None)}
    return _exec_plan(pid, comm, fields, argv, mypid)


def _tty_of(pid: int) -> int:
    st = _stat_of(pid)
    try:
        return int(st[1][4]) if st else 0
    except (ValueError, IndexError):
        return 0


def _program_of(pid: int) -> str:
    try:
        return os.readlink(f"/proc/{pid}/exe")
    except OSError:
        st = _stat_of(pid)
        return "comm:" + (st[0] if st else "")


def app_tree(pid: int, members: set[int]) -> tuple[int, list[int]]:
    """(root, pids) of what an app restart ends, for ``pid`` in an app group of ``members``: the root is ``pid``'s
    topmost ancestor inside the group; the pids are that root's process tree inside the group — without programs
    that have a terminal of their own (a shell in a terminal app and everything started from it) — plus other
    top-level processes of the group running the root's own program (a second instance). Pure over /proc."""
    parent = {}
    for p in members:
        st = _stat_of(p)
        try:
            parent[p] = int(st[1][1]) if st else 0
        except (ValueError, IndexError):
            parent[p] = 0
    root, n = pid, 0
    while n < 64 and parent.get(root) in members:
        root, n = parent[root], n + 1
    kids: dict[int, list[int]] = {}
    for p, pp in parent.items():
        kids.setdefault(pp, []).append(p)
    tops = [root] + sorted(p for p in members if parent.get(p) not in members and p != root and
                           _program_of(p) == _program_of(root))
    out, stack = [], list(tops)
    while stack:
        p = stack.pop()
        if p in out:
            continue
        out.append(p)
        tty = _tty_of(p)
        for c in kids.get(p, []):
            ctty = _tty_of(c)
            if ctty and ctty != tty:
                continue                  # it got a terminal of its own: a program of its own
            stack.append(c)
    return root, [root] + sorted(p for p in out if p != root)


def _exec_plan(pid: int, comm: str, fields: list[bytes], argv: list[str], mypid: int) -> dict:
    """Start the very same command again — only when that is safe."""
    ids = _status_ids(pid)
    if ids is None or any(u != UID for u in ids[0]) or len(set(ids[1])) != 1:
        _refuse("unsupported", "Dit proces draait met andere rechten (setuid/setgid) en kan hier niet opnieuw "
                               "worden gestart.", {"pid": pid})
    try:
        exe = os.readlink(f"/proc/{pid}/exe")
    except OSError:
        exe = ""
    env = _environ_of(pid)
    if env is None:
        _refuse("unsupported", "Dit proces laat zijn instellingen niet lezen en kan hier niet opnieuw worden "
                               "gestart.", {"pid": pid})
    if not _same_mount_ns(pid):
        _refuse("unsupported", "Dit proces draait afgeschermd (bijvoorbeeld als Flatpak of Snap) en kan hier niet "
                               "opnieuw worden gestart. Sluit het en open het opnieuw.", {"pid": pid})
    if exe.endswith(" (deleted)") or not exe or not os.path.exists(exe):
        import shutil
        exe = argv[0] if os.path.isabs(argv[0]) and os.path.exists(argv[0]) else \
            (shutil.which(argv[0], path=env.get("PATH") or os.defpath) or "")
        if not exe:
            _refuse("unsupported", "Het programma is bijgewerkt of verwijderd. Start het zelf opnieuw.", {"pid": pid})
    try:
        mode = os.stat(exe).st_mode
    except OSError:
        mode = 0
    import stat as _st
    if mode & (_st.S_ISUID | _st.S_ISGID):
        _refuse("unsupported", "Dit programma draait met andere rechten (setuid/setgid) en kan hier niet opnieuw "
                               "worden gestart.", {"pid": pid})
    # a script started through its #! line: /proc shows [interpreter, script, args…] and comm is the script's name;
    # start the script itself again (the kernel reads the #! line again) — else the new process is named after the
    # interpreter ("python3.14") and lands in the interpreter's app scope
    if len(argv) >= 2 and comm and os.path.basename(argv[1])[:15] == comm and os.path.isabs(argv[1]):
        try:
            with open(argv[1], "rb") as f:
                shebang = f.read(2) == b"#!"
        except OSError:
            shebang = False
        if shebang and os.access(argv[1], os.X_OK):
            argv, exe = argv[1:], argv[1]
    try:
        cwd = os.readlink(f"/proc/{pid}/cwd")
    except OSError:
        cwd = ""
    cwd_changed = False
    if cwd.endswith(" (deleted)") or not os.path.isdir(cwd):
        cwd, cwd_changed = str(Path.home()), True
    try:
        ppid, sid, tty = int(fields[1]), int(fields[3]), int(fields[4])
    except (ValueError, IndexError):
        ppid, sid, tty = 0, 0, 0
    if not tty:
        pst = _stat_of(ppid) if ppid > 1 else None
        parent_ok = ppid <= 1 or ppid == mypid and sid == pid or (
            pst is not None and pst[0] == "systemd" and os.stat(f"/proc/{ppid}").st_uid == UID)
        if not parent_ok:
            pname = (pst[0] if pst else "") or "een ander programma"
            _refuse("unsupported", f"Dit proces is gestart door ‘{pname}’ en hoort daarbij. Start dat programma "
                                   "opnieuw.", {"pid": pid, "parent": ppid})
    return {"how": "exec", "pids": [pid], "argv": argv, "exe": exe, "cwd": cwd, "env": env, "tty": bool(tty),
            "cwdChanged": cwd_changed, "name": comm}


def end_gracefully(pids: list[int], kill: Callable = os.kill, term_s: float = RESTART_TERM_S,
                   kill_s: float = RESTART_KILL_S) -> list[int]:
    """SIGTERM, SIGKILL after ``term_s`` — each signal only to the same process (start time). → pids still alive."""
    starts = {}
    for p in pids:
        st = _stat_of(p)
        if st is not None and len(st[1]) > 19:
            starts[p] = st[1][19]

    def alive(p):                       # a zombie is gone (whoever started it reaps it)
        st = _stat_of(p)
        return st is not None and st[1][0] not in (b"Z", b"X") and st[1][19] == starts.get(p)

    for sig, wait_s in ((signal.SIGTERM, term_s), (signal.SIGKILL, kill_s)):
        left = [p for p in starts if alive(p)]
        if not left:
            return []
        for p in left:
            try:
                kill(p, sig)
            except ProcessLookupError:
                pass
            except PermissionError:
                return left
        end = time.monotonic() + wait_s
        while time.monotonic() < end:
            if not any(alive(p) for p in left):
                return []
            time.sleep(0.05)
    return [p for p in starts if alive(p)]


def _relaunch_env(env: dict) -> dict:
    """The process's own environment for its restart — with the user's renderer switches only (util.user_env): a
    program first started by a GPU-forcing shell (WEBKIT_FORCE_DMABUF_RENDERER=1, __NV_DISABLE_EXPLICIT_SYNC=1,
    2026-10-26) must not get them again. Our private WIN11SHELL_* variables never travel either."""
    from ..util import user_env
    return user_env(env, drop_private=True)


def _spawn_exec(plan: dict) -> int:
    import subprocess
    p = subprocess.Popen(plan["argv"], executable=plan["exe"], cwd=plan["cwd"], env=_relaunch_env(plan["env"]),
                         stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                         start_new_session=True, close_fds=True)
    threading.Thread(target=p.wait, name="restarted-wait", daemon=True).start()
    try:   # not in the shell's own (high priority) scope
        from .. import priority
        priority.start_scope([p.pid], priority.child_app_unit(os.path.basename(plan["argv"][0]), p.pid),
                             description=plan.get("name") or "", wait=0)
    except Exception:  # noqa: BLE001
        pass
    return p.pid


_CONFIRM_TTY = ("Dit proces draait in een terminal. Het wordt opnieuw gestart zonder terminalvenster: je ziet zijn "
                "uitvoer dan niet meer en kunt niets meer typen. Toch opnieuw starten?")


def restart_process(pid: int, confirm: bool, pm: "ProcessMonitor", *, launch: Callable[[str], Any],
                    restart_shell: Callable[[], Any], kill: Callable = os.kill,
                    spawn: Callable[[dict], int] = _spawn_exec, term_s: float = RESTART_TERM_S,
                    kill_s: float = RESTART_KILL_S, mypid: int | None = None) -> dict:
    """Do it (see the module doc). → {ok, pid, how, name, …}."""
    plan = restart_plan(pid, pm, mypid=mypid)
    how, name = plan["how"], plan.get("name")
    if how == "shell":
        restart_shell()
        return {"ok": True, "pid": pid, "how": "shell", "name": name}
    if how == "service":
        service_action(plan["unit"], "restart")
        return {"ok": True, "pid": pid, "how": "service", "unit": plan["unit"], "name": name}
    if how == "exec" and plan["tty"] and not confirm:
        _refuse("confirm_required", _CONFIRM_TTY, {"pid": pid, "reason": "terminal"})
    webkit: dict = {}
    if plan.get("ownWindow"):
        # our app window's WebKit processes: a hung one never notices its window process is gone (SPEC §2.2)
        from ..surfaces import webkit_snapshot
        webkit = webkit_snapshot(plan["pids"][0])
    left = end_gracefully(plan["pids"], kill=kill, term_s=term_s, kill_s=kill_s)
    if left:
        _refuse("failed", "Het proces stopte niet, dus het is niet opnieuw gestart.", {"pids": left})
    if webkit:
        from ..surfaces import kill_webkit_leftovers
        kill_webkit_leftovers(webkit, grace=0.5, kill=kill)
    if how == "app":
        try:
            launch(plan["desktopId"])
        except ApiError as e:
            _refuse("launch_failed", f"Het proces is gestopt, maar opnieuw openen lukte niet: {e.message}",
                    {"desktopId": plan["desktopId"]})
        return {"ok": True, "pid": pid, "how": "app", "desktopId": plan["desktopId"], "ended": plan["pids"],
                "name": name}
    try:
        new_pid = spawn(plan)
    except OSError as e:
        _refuse("launch_failed", f"Het proces is gestopt, maar opnieuw starten lukte niet ({e.strerror}).",
                {"pid": pid})
    return {"ok": True, "pid": pid, "how": "exec", "newPid": new_pid, "name": name,
            "cwdChanged": plan.get("cwdChanged", False)}


# ---------------------------------------------------------------------------
# health
# ---------------------------------------------------------------------------

_SEV = {"critical": 0, "warning": 1, "info": 2, "ok": 3}


def health(api, pm: ProcessMonitor) -> list[dict]:
    items: list[dict] = []
    # disks
    disk_items = []
    try:
        vols = api.call("storage.volumes", {}) if api.has("storage.volumes") else []
    except Exception:  # noqa: BLE001
        vols = []
    for v in vols or []:
        total, free, used = v.get("total") or 0, v.get("free") or 0, v.get("used") or 0
        if total <= 0 or v.get("readOnly") or v.get("kind") == "optical" or v.get("hidden"):
            continue
        pct = used * 100.0 / max(1, used + free)
        if pct >= 90:
            crit = pct >= 97 or (v.get("isSystem") and free < 2 * 1024 ** 3)
            if v.get("isSystem") or v.get("isHome"):   # the cleanup items free space here
                action = {"label": _tr("a_free"), "app": "taskmgr",
                          "params": {"page": "storage", "volume": v.get("id"), "section": "cleanup", "tab": "cleanup"}}
            else:                                      # data drive: show what takes the space
                action = {"label": _tr("a_storage"), "app": "taskmgr", "params": {"page": "storage", "volume": v.get("id")}}
            disk_items.append({
                "id": f"disk:{v.get('id')}", "severity": "critical" if crit else "warning",
                "title": _tr("h_disk_crit_t" if crit else "h_disk_warn_t", name=v.get("name") or v.get("mount")),
                "message": _tr("h_disk_crit_m" if crit else "h_disk_warn_m", free=fmt_bytes(free),
                               total=fmt_bytes(total), pct=int(round(pct))),
                "action": action,
            })
    items += disk_items or [{"id": "disks", "severity": "ok", "title": _tr("h_disk_ok_t"),
                             "message": _tr("h_disk_ok_m"), "action": None}]
    # memory
    mem = read_meminfo()
    total, avail = mem.get("MemTotal", 0), mem.get("MemAvailable", 0)
    psi = None
    pdata = _read("/proc/pressure/memory")
    if pdata:
        m = re.search(r"some avg60=([0-9.]+)", pdata)
        psi = float(m.group(1)) if m else None
    frac = avail / total if total else 1.0
    mem_action = {"label": _tr("a_procs"), "app": "taskmgr", "params": {"page": "processes", "sort": "memory"}}
    if frac < 0.05 or (psi is not None and psi > 30):
        items.append({"id": "memory", "severity": "critical", "title": _tr("h_mem_crit_t"),
                      "message": _tr("h_mem_crit_m", avail=fmt_bytes(avail)), "action": mem_action})
    elif frac < 0.12 or (psi is not None and psi > 10):
        items.append({"id": "memory", "severity": "warning", "title": _tr("h_mem_warn_t"),
                      "message": _tr("h_mem_warn_m", avail=fmt_bytes(avail), total=fmt_bytes(total)),
                      "action": mem_action})
    else:
        items.append({"id": "memory", "severity": "ok", "title": _tr("h_mem_ok_t"),
                      "message": _tr("h_mem_ok_m", avail=fmt_bytes(avail), total=fmt_bytes(total)), "action": None})
    # swap
    st, sf = mem.get("SwapTotal", 0), mem.get("SwapFree", 0)
    su = max(0, st - sf)
    if st > 0 and su / st >= 0.5 and su > 256 * 1024 ** 2:
        items.append({"id": "swap", "severity": "warning" if su / st >= 0.8 else "info", "title": _tr("h_swap_warn_t"),
                      "message": _tr("h_swap_warn_m", used=fmt_bytes(su), total=fmt_bytes(st)), "action": mem_action})
    else:
        items.append({"id": "swap", "severity": "ok", "title": _tr("h_swap_ok_t"),
                      "message": _tr("h_swap_ok_m") if st > 0 else _tr("h_swap_none_m"), "action": None})
    # high CPU
    hot = pm.high_cpu(HIGH_CPU_SECONDS)
    if hot:
        for h in hot[:3]:
            items.append({"id": f"cpu:{h['pid']}", "severity": "warning" if h["cpu"] >= 50 else "info",
                          "title": _tr("h_cpu_warn_t", name=h["name"]),
                          "message": _tr("h_cpu_warn_m", dur=_fmt_duration(h["since"])),
                          "action": {"label": _tr("a_view"), "app": "taskmgr",
                                     "params": {"page": "processes", "pid": h["pid"]}},
                          "pid": h["pid"], "cpu": round(h["cpu"], 1)})
    else:
        items.append({"id": "cpu", "severity": "ok", "title": _tr("h_cpu_ok_t"), "message": _tr("h_cpu_ok_m"),
                      "action": None})
    # temperatures
    temp = cpu_temperature()
    temp_items = []
    perf_action = {"label": _tr("a_perf"), "app": "taskmgr", "params": {"page": "performance"}}
    if temp is not None and temp >= 85:
        crit = temp >= 95
        temp_items.append({"id": "temp:cpu", "severity": "critical" if crit else "warning",
                           "title": _tr("h_temp_crit_t" if crit else "h_temp_warn_t", what=_tr("cpu_word")),
                           "message": _tr("h_temp_crit_m" if crit else "h_temp_warn_m", temp=int(temp)),
                           "action": perf_action})
    for g in _GPU.nvidia(max_age=10, wait=1.0):
        t = g.get("temperature")
        if t is not None and t >= 85:
            crit = t >= 92
            temp_items.append({"id": "temp:gpu", "severity": "critical" if crit else "warning",
                               "title": _tr("h_temp_crit_t" if crit else "h_temp_warn_t", what=_tr("gpu_word")),
                               "message": _tr("h_temp_crit_m" if crit else "h_temp_warn_m", temp=int(t)),
                               "action": perf_action})
    if temp_items:
        items += temp_items
    else:
        items.append({"id": "temperature", "severity": "ok", "title": _tr("h_temp_ok_t"),
                      "message": _tr("h_temp_ok_m", temp=int(temp)) if temp is not None else _tr("h_temp_na_m"),
                      "action": None, "temperature": temp})
    # failed user services
    failed = [u for u in list_user_units(state="failed") if str(u.get("unit", "")).endswith(".service")]
    if failed:
        names = ", ".join((u.get("description") or u.get("unit")) for u in failed[:4])
        if len(failed) > 4:
            names += " …"
        n = len(failed)
        items.append({"id": "services", "severity": "warning",
                      "title": _tr("h_svc_warn_t" if n == 1 else "h_svc_warn_t_pl", n=n),
                      "message": _tr("h_svc_warn_m", names=names),
                      "action": {"label": _tr("a_services"), "app": "taskmgr", "params": {"page": "services"}},
                      "units": [u.get("unit") for u in failed]})
    else:
        items.append({"id": "services", "severity": "ok", "title": _tr("h_svc_ok_t"), "message": _tr("h_svc_ok_m"),
                      "action": None})
    # updates
    upd = None
    try:
        if api.has("system.updates"):
            upd = api.call("system.updates", {})
    except Exception:  # noqa: BLE001
        upd = None
    if upd and upd.get("tool"):
        n = upd.get("available")
        if n:
            msg = _tr("h_upd_info_m1") if n == 1 else _tr("h_upd_info_m", n=n)
            if upd.get("security"):
                msg += _tr("h_upd_sec_m", s=upd["security"])
            items.append({"id": "updates", "severity": "info", "title": _tr("h_upd_info_t"), "message": msg,
                          "action": {"label": _tr("a_updates"), "app": "settings", "params": {"page": "update"},
                                     "method": "system.openUpdater"}})
        else:
            items.append({"id": "updates", "severity": "ok", "title": _tr("h_upd_ok_t"),
                          "message": _tr("h_upd_ok_m") if n == 0 else _tr("h_upd_unknown_m"), "action": None})
    # trash
    trash = None
    try:
        if api.has("storage.trashSize"):
            trash = api.call("storage.trashSize", {})
        elif api.has("files.trashInfo"):
            trash = api.call("files.trashInfo", {})
    except Exception:  # noqa: BLE001
        trash = None
    if isinstance(trash, dict) and trash.get("size") is not None:
        size = trash["size"]
        if size >= 1024 ** 3:
            items.append({"id": "trash", "severity": "info", "title": _tr("h_trash_info_t"),
                          "message": _tr("h_trash_info_m", size=fmt_bytes(size)),
                          "action": {"label": _tr("a_free"), "app": "taskmgr",
                                     "params": {"page": "storage", "section": "cleanup", "tab": "cleanup"}},
                          "size": size})
        else:
            items.append({"id": "trash", "severity": "ok", "title": _tr("h_trash_ok_t"),
                          "message": _tr("h_trash_ok_m"), "action": None, "size": size})
    items.sort(key=lambda x: _SEV.get(x["severity"], 9))
    return items


# ---------------------------------------------------------------------------
# registration
# ---------------------------------------------------------------------------

_state: dict[str, Any] = {}


def register(api, ctx):
    global _lang, _api

    def lang() -> str:
        try:
            return "en" if ctx.settings.get("language", "nl") == "en" else "nl"
        except Exception:  # noqa: BLE001
            return "nl"

    _lang = lang
    _api = api
    pm = ProcessMonitor(api)
    perf = Performance(pm)
    _state.update(pm=pm, perf=perf, stop=threading.Event())

    try:
        api.on("apps.changed", lambda *_a: _DESKTOP.invalidate())
    except Exception:  # noqa: BLE001
        pass

    @api.method("sysmon.processes")
    def _processes(args):
        """{timestamp, totals, apps, background, system, processes} (pass {processes:false} to skip the list)"""
        return pm.processes(include_list=args.get("processes", True) is not False)

    @api.method("sysmon.endTask", dangerous=True,
                dry_result=lambda a: {"results": [{"pid": p, "ok": True, "error": None}
                                                  for p in (a.get("pids") or [])
                                                  if isinstance(p, (int, str))][:2000]})
    def _end_task(args):
        """{pids, force} → {results:[{pid, ok, error}]} (own processes only)"""
        return end_task(_pid_list(args.get("pids")), bool(args.get("force")), pm)

    def _restart_shell():
        """Our own host: in place (exec) like shell.restart; an app-mode host starts the desktop shell instead."""
        import subprocess
        import sys
        if ctx.mode == "app":
            from .. import ROOT
            from ..util import user_env
            env = {k: v for k, v in user_env().items() if k != "WIN11SHELL_TOKEN"}
            subprocess.Popen([sys.executable, "-m", "host.ctl", "restart", "--force", "--no-wait"], cwd=str(ROOT),
                             env=env, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                             stderr=subprocess.DEVNULL, start_new_session=True, close_fds=True)
            return
        t = threading.Timer(0.3, ctx.request_restart)
        t.daemon = True
        t.start()

    def _launch(desktop_id: str):
        return api.call("apps.launch", {"id": desktop_id})

    def _plan_summary(pid: int, plan: dict) -> dict:
        """What Taakbeheer may know of a plan (never the environment or the command line)."""
        out = {"pid": pid, "how": plan["how"], "name": plan.get("name")}
        out.update({k: plan[k] for k in ("desktopId", "unit", "via") if plan.get(k)})
        if plan["how"] == "app":
            out["count"] = len(plan.get("pids") or [])
        if plan["how"] == "exec":
            out["tty"] = bool(plan.get("tty"))
        return out

    def _restart_dry(args):
        """--dry-run: what would happen (restart_plan has no side effects). Refusals and the terminal question come
        back as they would for real, so Taakbeheer's test mode shows the same dialogs."""
        pid = _pid_list(args.get("pid"))
        if len(pid) != 1:
            raise ApiError("bad_args", "Geef één proces op.")
        plan = restart_plan(pid[0], pm)
        if plan["how"] == "exec" and plan.get("tty") and args.get("confirm") is not True:
            _refuse("confirm_required", _CONFIRM_TTY, {"pid": pid[0], "reason": "terminal"})
        return _plan_summary(pid[0], plan)

    @api.method("sysmon.restartPlan")
    def _restart_plan(args):
        """{pid} → {pid, how:'shell'|'app'|'service'|'exec', name, desktopId?, unit?, via?, count?, tty?} — what
        "Opnieuw starten" would do (no side effects; Taakbeheer asks the right question first). Refusals as errors."""
        pid = _pid_list(args.get("pid"))
        if len(pid) != 1:
            raise ApiError("bad_args", "Geef één proces op.")
        return _plan_summary(pid[0], restart_plan(pid[0], pm))

    @api.method("sysmon.restartProcess", dangerous=True, dry_result=_restart_dry)
    def _restart_process(args):
        """{pid, confirm?} → {ok, pid, how:'shell'|'app'|'service'|'exec', name, newPid?, desktopId?, unit?}"""
        pid = _pid_list(args.get("pid"))
        if len(pid) != 1:
            raise ApiError("bad_args", "Geef één proces op.")
        log.info("Proces opnieuw starten (restart process) %d", pid[0])
        return restart_process(pid[0], args.get("confirm") is True, pm, launch=_launch, restart_shell=_restart_shell)

    @api.method("sysmon.setEfficiency", dangerous=True,
                dry_result=lambda a: {"results": [{"pid": p, "ok": True, "error": None}
                                                  for p in (a.get("pids") or [])
                                                  if isinstance(p, (int, str))][:2000]})
    def _set_eff(args):
        return set_efficiency(_pid_list(args.get("pids")), bool(args.get("enabled")), pm)

    @api.method("sysmon.performance")
    def _performance(args):
        return perf.get()

    @api.method("sysmon.startup")
    def _startup(args):
        """[{id, name, icon, enabled, command, source}] ({all:true} also lists hidden session helpers)"""
        return startup_list(bool(args.get("all")))

    @api.method("sysmon.setStartup", dangerous=True)
    def _set_startup(args):
        return set_startup(args.get("id"), bool(args.get("enabled")))

    @api.method("sysmon.services")
    def _services(args):
        return services_list(pm)

    @api.method("sysmon.serviceAction", dangerous=True)
    def _service_action(args):
        return service_action(args.get("name"), args.get("action"))

    @api.method("sysmon.health")
    def _health(args):
        return health(api, pm)


def start(api, ctx):
    pm: ProcessMonitor = _state["pm"]
    stop_ev: threading.Event = _state["stop"]

    def warm():
        try:
            _DESKTOP.ensure()
        except Exception:  # noqa: BLE001
            log.debug("desktop index failed", exc_info=True)
        pm.sample_only()
        try:
            _unit_file_states()
            pm.unit_descriptions()
        except Exception:  # noqa: BLE001
            log.debug("unit prefetch failed", exc_info=True)

    def sampler():
        warm()
        while not stop_ev.wait(SAMPLER_S):
            if api.subscriber_count() == 0:
                continue
            last = pm.samples[-1][0] if pm.samples else 0
            if time.monotonic() - last < SAMPLER_S * 0.8:
                continue   # Task Manager is polling anyway
            try:
                pm.sample_only()
            except Exception:  # noqa: BLE001
                log.debug("sample failed", exc_info=True)

    threading.Thread(target=sampler, name="sysmon-sampler", daemon=True).start()


def stop(api, ctx):
    ev = _state.get("stop")
    if ev:
        ev.set()
