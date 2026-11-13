#!/usr/bin/env python3
"""Install only this project's files; do not change the current desktop session.

Usage: scripts/install.sh [--lang nl|en]   (install.sh runs: install.py ROOT [--lang nl|en])

Win11's language (notes/claude/contracts.md C1, C6): an existing win11-gnome settings.json always decides; without
one, --lang; on an update of an existing install without a stored choice, nl (H D13); on a first install, the
language of Win11 Shell for Hyprland (~/.config/win11shell/settings.json), else one guess from the locale (H D4).
A first install stores that choice in a complete settings.json (the defaults of host/src/settings-defaults.json).
Messages follow --lang when it is given, else that language.
"""
from __future__ import annotations
import fcntl
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import struct
import subprocess
import sys
import tempfile
import icon_bundle
import file_portal

MARKER = ".win11-gnome-owned"
MAGIC = "win11-gnome-rust-host-v1\n"
UUID = "win11-gnome@win11shell.local"
BINS = ("win11-explorer", "win11-panel", "win11-app", "win11-launch", "win11-index-worker", "win11-image-worker", "win11-default-apps", "win11-file-portal", "win11-installer", "win11-software-worker", "win11-shell", "win11-calendar")
# D-G70: the native GTK 4 shell (native/shell: taskbar, flyouts, Start, desktop) is the shell; win11-panel execs it.
# win11-calendar (Robin 2026-11-13): the native Calendar app (native/calendar), opened on a day by the taskbar calendar.
# Both are built in their own workspace (native/; scripts/build.sh puts them next to the others), so they are looked
# up in WIN11_NATIVE_BUILD_DIR first, then in the build directory.
NATIVE_BINS = ("win11-shell", "win11-calendar")
ENTRIES = ("Explorer", "Panel", "TaskManager", "Settings", "Search", "Disks", "DefaultApps", "Installer", "Calendar")
# D-G70: the WebKit Start and desktop hosts of earlier versions. An install over an own payload removes them (they are
# no longer built); they are not conflicts of a fresh install.
LEGACY_BINS = ("win11-start", "win11-desktop")
LEGACY_ENTRIES = ("Start", "Desktop")
# The Calendar's own URI scheme: win11-calendar:YYYY-MM-DD opens that day. Only this scheme gets a default here.
CALENDAR_SCHEME = "x-scheme-handler/win11-calendar"
CALENDAR_DESKTOP = "io.github.win11shell.Calendar.desktop"
LANGUAGES = ("nl", "en")

# i18n: pairs (tools/i18n/check_pairs.py). Inline on purpose: these texts appear when something is broken
# (contracts C3, H D14). nl is byte-identical to 3b898e6 except the rows in tests/i18n/BASELINE-CHANGES.ext.
T = {
    "labelPrefix": {"nl": "Installatiepad", "en": "Install path"},
    "controlChars": {"nl": "{label} mag geen besturingstekens bevatten", "en": "{label} must not contain control characters"},
    "notAbsolute": {"nl": "{label} moet een absoluut pad zijn: {value}", "en": "{label} must be an absolute path: {value}"},
    "missingBinary": {"nl": "Ontbreekt: {source}. Bouw eerst met scripts/build.sh.", "en": "Missing: {source}. Build it first with scripts/build.sh."},
    "missingSource": {"nl": "Ontbrekende interface/compatibiliteitsbron: {directory}", "en": "Missing interface/compatibility source: {directory}"},
    "badTable": {"nl": "Ontbrekend of ongeldig projectbestand: {file}", "en": "Missing or invalid project file: {file}"},
    "notOwned": {"nl": "Niet overschreven: {payload} bevat geen installatiemarker van dit project.", "en": "Not overwritten: {payload} has no installation marker of this project."},
    "conflicts": {"nl": "Niet overschreven: bestaande bestanden zonder installatiemarker:", "en": "Not overwritten: existing files without an installation marker:"},
    "previousCopy": {"nl": "Eerdere installatiekopie bestaat nog: {backup}", "en": "A previous installation copy still exists: {backup}"},
    "missingParts": {"nl": "Ontbrekende systeemonderdelen: {missing}.", "en": "Missing system components: {missing}."},
    "installParts": {"nl": "Installeer ze op Ubuntu met: sudo apt install {packages}", "en": "On Ubuntu, install them with: sudo apt install {packages}"},
    "noDesktopDatabase": {"nl": "Let op: update-desktop-database ontbreekt (pakket desktop-file-utils); de app-database van GNOME wordt niet bijgewerkt.",
                          "en": "Note: update-desktop-database is missing (package desktop-file-utils); GNOME’s app database is not updated."},
    "configNotOwn": {"nl": "Niet overschreven: {path} moet een gewone map van deze gebruiker zijn.", "en": "Not overwritten: {path} must be a plain folder of this user."},
    "badLang": {"nl": "Onbekende taal ‘{lang}’: kies nl of en.", "en": "Unknown language ‘{lang}’: choose nl or en."},
    "usage": {"nl": "Gebruik: scripts/install.sh [--lang nl|en]\n  --lang nl|en  de taal van Win11 bij een eerste installatie; een keuze die al in settings.json staat, blijft gelden",
              "en": "Usage: scripts/install.sh [--lang nl|en]\n  --lang nl|en  Win11’s language on a first install; a choice already in settings.json is kept"},
    "badOption": {"nl": "Onbekende optie: {option}", "en": "Unknown option: {option}"},
    "langChosen": {"nl": "Taal: Nederlands. In English: choose English in Instellingen › Tijd en taal › Taal en regio.",
                   "en": "Language: English. In het Nederlands: kies Nederlands in Settings › Time & language › Language & region."},
    # langZone and langFormats are printed only when the locale guess gave nl without --lang, so the message
    # language is then nl and their en halves are never shown (ge1c-review m6); they stay for the pairs checker.
    "langZone": {"nl": "Taal: Nederlands, vanwege de tijdzone {zone}. In English: choose English in Instellingen › Tijd en taal › Taal en regio.",
                 "en": "Language: Dutch, because of the time zone {zone}. In English: choose English in Instellingen › Tijd en taal › Taal en regio."},
    "langFormats": {"nl": "Taal: Nederlands, vanwege de Nederlandse notatie in {var}. In English: choose English in Instellingen › Tijd en taal › Taal en regio.",
                    "en": "Language: Dutch, because of the Dutch formats in {var}. In English: choose English in Instellingen › Tijd en taal › Taal en regio."},
    "langMigrated": {"nl": "Taal: {name}, overgenomen van Win11 Shell voor Hyprland ({file}).", "en": "Language: {name}, taken over from Win11 Shell for Hyprland ({file})."},
    "langKept": {"nl": "Let op: --lang {lang} geldt alleen zonder opgeslagen keuze. Win11 blijft {name} ({file}); wijzig de taal in {path}.",
                 "en": "Note: --lang {lang} only applies without a stored choice. Win11 stays in {name} ({file}); change the language in {path}."},
    "name_nl": {"nl": "Nederlands", "en": "Dutch"},
    "name_en": {"nl": "Engels", "en": "English"},
    "settingsPath": {"nl": "Instellingen › Tijd en taal › Taal en regio", "en": "Settings › Time & language › Language & region"},
    "staleRemoved": {"nl": "Verwijderd uit de extensiemap (geen bestand van deze versie): {path}",
                     "en": "Removed from the extension folder (not a file of this version): {path}"},
    "done1": {"nl": "Geïnstalleerd in {prefix}/bin", "en": "Installed in {prefix}/bin"},
    "done2": {"nl": "Verkenner is los te openen via het applicatieoverzicht.", "en": "File Explorer can be opened from the app overview."},
    "done3": {"nl": "De taakbalk is geïnstalleerd, maar nog niet aangezet.", "en": "The taskbar is installed, but not turned on yet."},
    "done4": {"nl": "Meld je aan bij Ubuntu/GNOME; na eerste installatie zo nodig af- en aanmelden.", "en": "Sign in to Ubuntu/GNOME; after a first installation, sign out and back in if needed."},
    "done5": {"nl": "Taakbalk aan: gnome-extensions enable {uuid}", "en": "Turn the taskbar on: gnome-extensions enable {uuid}"},
    "done6": {"nl": "Je huidige Win11Shell, standaardbestandsbeheerder en desktopinstellingen zijn niet gewijzigd.", "en": "Your current Win11Shell, default file manager and desktop settings have not been changed."},
    # GW5: the one hint line about scripts/install-system.py (install: the files are missing; uninstall: still there)
    "systemHint": {"nl": "Optioneel, één keer met je wachtwoord: {command} installeert de systeembestanden voor ‘Nieuwe Ubuntu-versies’ en ‘Venster weer openen’.",
                   "en": "Optional, once with your password: {command} installs the system files for ‘New Ubuntu versions’ and ‘Show the upgrade window’."},
    "systemHintRemove": {"nl": "De systeembestanden van Win11 staan er nog; verwijder ze met: {command}",
                         "en": "Win11’s system files are still installed; remove them with: {command}"},
    # GW7: Name= of the post-upgrade check's autostart entry (design §6.6), in Win11's language (D-G4)
    "postUpgradeName": {"nl": "Win11-controle na een Ubuntu-upgrade", "en": "Win11 check after an Ubuntu upgrade"},
}
LANGUAGE = "nl"          # the language of the messages; set_language() also sets it in icon_bundle and file_portal


def t(key: str, language: str | None = None, **params) -> str:
    pair = T[key]
    return (pair.get(language or LANGUAGE) or pair["nl"]).format_map(params)


def set_language(language: str) -> None:
    global LANGUAGE
    LANGUAGE = icon_bundle.LANGUAGE = file_portal.LANGUAGE = language if language in LANGUAGES else "nl"


def absolute_path(value: str, label: str) -> Path:
    if any(ord(c) < 32 for c in value):
        raise SystemExit(t("controlChars", label=label))
    path = Path(value).expanduser()
    if not path.is_absolute():
        raise SystemExit(t("notAbsolute", label=label, value=value))
    return path


def desktop_exec(path: Path) -> str:
    # Desktop-entry unescaping precedes Exec argument unescaping (not shell parsing).
    value = str(path)
    for character in ("\\", '"', "`", "$"):
        value = value.replace(character, "\\" + character)
    value = value.replace("%", "%%").replace("\\", "\\\\")
    return '"' + value + '"'


# The system files of scripts/install-system.py (GW5, notes/claude/windows-update-gnome.md §7): installed and removed
# only by that separate command, never by this installer or uninstall.py. Both only name the command in one line, and
# only on a terminal: the line is advice for the person at the keyboard, so captured output (the installer tests'
# byte-for-byte comparison with 3b898e6) stays as it was.
SYSTEM_FILES = ("usr/libexec/win11-gnome/release-upgrades-helper", "usr/libexec/win11-gnome/release-upgrade-attach",
                "usr/share/polkit-1/actions/io.github.win11shell.gnome.release-upgrades.policy",
                "usr/share/polkit-1/actions/io.github.win11shell.gnome.release-upgrade-attach.policy")


# GW7 (design §6.6): the post-upgrade check runs from XDG autostart, outside the extension, so it also works when
# GNOME turned the extension off after an Ubuntu upgrade, and after upgrades done through Ubuntu's Software Updater.
AUTOSTART = "io.github.win11shell.PostUpgradeCheck.desktop"


def autostart_entry(prefix: Path, language: str) -> str:
    """Plain Name= in Win11's language (D-G4), NoDisplay, OnlyShowIn=GNOME; and no X-GNOME-Autostart-Delay (systemd's
    generator ignores it; the check waits up to 30 s for GNOME Shell itself)."""
    return ("[Desktop Entry]\nType=Application\n"
            f"Name={t('postUpgradeName', language)}\n"
            f"Exec=/usr/bin/env -- {desktop_exec(prefix/'bin'/'win11-launch')} --post-upgrade-check\n"
            "OnlyShowIn=GNOME;\nNoDisplay=true\n")


def system_hint(remove: bool, root: Path = Path("/")) -> str | None:
    """install (remove=False): the hint when any system file is missing; uninstall: when any is still there."""
    present = [(root/name).is_file() for name in SYSTEM_FILES]
    if (remove and not any(present)) or (not remove and all(present)):
        return None
    command = "python3 " + shlex.quote(str(Path(__file__).resolve().parent/"install-system.py"))
    if remove:
        return t("systemHintRemove", command=command + " --remove")
    return t("systemHint", command=command)


# ---------------------------------------------------------------------------------------------------------------
# Language (C1, C6; H's host/i18n.py detect_info, notes/english-option-design.md §2.6.2, D2-D4, D13)
# ---------------------------------------------------------------------------------------------------------------

DUTCH_ZONES = frozenset({"Europe/Amsterdam", "America/Paramaribo", "America/Curacao", "America/Aruba",
                         "America/Kralendijk", "America/Lower_Princes"})        # not Europe/Brussels (H D4)
SIGNAL_CATEGORIES = ("LC_TIME", "LC_NUMERIC", "LC_MONETARY", "LC_PAPER", "LC_MEASUREMENT")
_NL_LOCALE = re.compile(r"^(?:nl(?:[_.@]|$)|fy_|li_|nds_NL)")
_NL_FORMAT = re.compile(r"^(?:nl|fy|li)_")
_EN_LOCALE = re.compile(r"^en(?:[_.@]|$)")
_ZONEINFO = re.compile(r"(?:^|/)zoneinfo/(?:posix/|right/)?(.+)$")


def language_of_file(path: Path) -> str:
    """C1: "en" only for exactly "en"; a missing or unreadable file, other JSON or another value means nl."""
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError, UnicodeError):
        return "nl"
    return "en" if isinstance(data, dict) and data.get("language") == "en" else "nl"


def _is_c(locale: str) -> bool:
    return re.split(r"[.@]", locale, maxsplit=1)[0] in ("", "C", "POSIX")


def messages_locale(env) -> str:
    """The first non-empty of LC_ALL, LC_MESSAGES and LANG; LANGUAGE's first entry wins unless that is C/POSIX."""
    effective = next((env[k] for k in ("LC_ALL", "LC_MESSAGES", "LANG") if env.get(k)), "")
    if not _is_c(effective):
        first = next((x for x in (env.get("LANGUAGE") or "").split(":") if x), "")
        if first:
            return first
    return effective


def _zone_of_path(path: str) -> str | None:
    match = _ZONEINFO.search(path)          # read by its text, never resolved (Amsterdam may link to Brussels)
    return match.group(1) if match else None


def time_zone(env, timezone_file: str = "/etc/timezone", localtime: str = "/etc/localtime") -> str | None:
    zone = env.get("TZ") or ""
    if zone:
        zone = zone[1:] if zone.startswith(":") else zone
        return _zone_of_path(zone) if zone.startswith("/") else (zone or None)
    try:
        with open(timezone_file, encoding="utf-8") as stream:
            value = stream.readline().strip()
        if value:
            return value
    except (OSError, UnicodeError):
        pass
    try:
        return _zone_of_path(os.readlink(localtime))
    except OSError:
        return None


def dutch_signal(env, timezone_file: str = "/etc/timezone", localtime: str = "/etc/localtime") -> str | None:
    """H D4: an nl_/fy_/li_ formats locale ("lc:LC_TIME") or a Dutch-speaking time zone ("tz:Europe/Amsterdam")."""
    for key in SIGNAL_CATEGORIES:
        if _NL_FORMAT.match(env.get("LC_ALL") or env.get(key) or env.get("LANG") or ""):
            return f"lc:{key}"
    zone = time_zone(env, timezone_file, localtime)
    return f"tz:{zone}" if zone in DUTCH_ZONES else None


def guess_language(env, timezone_file: str = "/etc/timezone",
                   localtime: str = "/etc/localtime") -> tuple[str, str, str | None]:
    """The one locale guess of a first install: (language, "locale" | "default", D4 signal)."""
    locale = messages_locale(env)
    if _is_c(locale):
        return "nl", "default", None
    if _NL_LOCALE.match(locale):
        return "nl", "locale", None
    if _EN_LOCALE.match(locale):
        signal = dutch_signal(env, timezone_file, localtime)
        return ("nl", "locale", signal) if signal else ("en", "locale", None)
    return "en", "locale", None                                                     # H D3


def resolve_language(explicit: str | None, config: Path, data: Path, payload: Path, env=os.environ,
                     timezone_file: str = "/etc/timezone", localtime: str = "/etc/localtime") -> dict:
    """{"lang", "source", "detail", "store"}; source: settings | arg | existing | hyprland | locale | default."""
    settings = config/"win11-gnome/settings.json"
    if os.path.lexists(settings):
        return {"lang": language_of_file(settings), "source": "settings", "detail": str(settings), "store": False}
    if explicit:
        return {"lang": explicit, "source": "arg", "detail": None, "store": True}
    if os.path.lexists(payload):                       # an update without a stored choice: what it showed (H D13)
        return {"lang": "nl", "source": "existing", "detail": None, "store": False}
    h_settings = config/"win11shell/settings.json"
    if os.path.lexists(h_settings):                    # plan-critique F20: H's choice moves over once
        return {"lang": language_of_file(h_settings), "source": "hyprland", "detail": str(h_settings), "store": True}
    if os.path.lexists(data/"win11shell"):             # H installed without a stored choice: H showed nl (H D13)
        return {"lang": "nl", "source": "hyprland", "detail": str(data/"win11shell"), "store": True}
    lang, source, signal = guess_language(env, timezone_file, localtime)
    return {"lang": lang, "source": source, "detail": signal, "store": True}


def parse_arguments(argv: list[str]) -> str | None:
    """--lang nl|en (also --lang=X, any case); -h/--help prints the usage. Exit 2 on a bad value or option."""
    explicit = None
    rest = []
    index = 0
    while index < len(argv):
        argument = argv[index]
        if argument == "--lang" or argument.startswith("--lang="):
            if argument == "--lang":
                index += 1
                value = argv[index] if index < len(argv) else ""
            else:
                value = argument.split("=", 1)[1]
            explicit = value.strip().lower()
            if explicit not in LANGUAGES:
                print(t("badLang", "nl", lang=value) + "\n" + t("badLang", "en", lang=value), file=sys.stderr)
                raise SystemExit(2)
        else:
            rest.append(argument)
        index += 1
    if explicit:
        set_language(explicit)
    for argument in rest:
        if argument in ("-h", "--help"):
            print(t("usage"))
            raise SystemExit(0)
        print(t("badOption", option=argument) + "\n" + t("usage"), file=sys.stderr)
        raise SystemExit(2)
    return explicit


# ---------------------------------------------------------------------------------------------------------------
# Build directory and runtime preflight
# ---------------------------------------------------------------------------------------------------------------

def executable(path: Path) -> bool:
    return path.is_file() and os.access(path, os.X_OK)


def binary_source(build: Path, name: str) -> Path:
    """Where `name` is installed from: the build directory; for the native binaries WIN11_NATIVE_BUILD_DIR first."""
    native = os.environ.get("WIN11_NATIVE_BUILD_DIR")
    if name in NATIVE_BINS and native and executable(Path(native)/name):
        return Path(native)/name
    return build/name


def missing_binaries(build: Path) -> list[Path]:
    return [binary_source(build, name) for name in BINS if not executable(binary_source(build, name))]


def choose_build(root: Path) -> Path:
    """WIN11_BUILD_DIR, else the first complete one of target/release and bin/ (a checkout's bin/ is stale; a
    package has only bin/). All 12 binaries are required; every missing one is named."""
    if os.environ.get("WIN11_BUILD_DIR"):
        build = Path(os.environ["WIN11_BUILD_DIR"])
    else:
        candidates = [root/"target/release", root/"bin"]
        build = next((c for c in candidates if not missing_binaries(c)),
                     next((c for c in candidates if c.is_dir()), candidates[0]))
    missing = missing_binaries(build)
    if missing:
        raise SystemExit("\n".join(t("missingBinary", source=path) for path in missing))
    return build


PACKAGES = {   # the Ubuntu 26.04 package of each library the binaries need (dpkg -S, 2026-11-05)
    "libwebkit2gtk-4.1.so.0": "libwebkit2gtk-4.1-0", "libjavascriptcoregtk-4.1.so.0": "libjavascriptcoregtk-4.1-0",
    "libsoup-3.0.so.0": "libsoup-3.0-0", "libgtk-3.so.0": "libgtk-3-0t64", "libgdk-3.so.0": "libgtk-3-0t64",
    "libgdk_pixbuf-2.0.so.0": "libgdk-pixbuf-2.0-0", "libcairo.so.2": "libcairo2", "libz.so.1": "zlib1g",
    "libglib-2.0.so.0": "libglib2.0-0t64", "libgio-2.0.so.0": "libglib2.0-0t64", "libgobject-2.0.so.0": "libglib2.0-0t64",
    "libarchive.so.13": "libarchive13t64", "libgcc_s.so.1": "libgcc-s1",
    "libgtk-4.so.1": "libgtk-4-1", "libgraphene-1.0.so.0": "libgraphene-1.0-0",   # win11-shell, win11-calendar
}
DLOPENED = ("libarchive.so.13",)          # host/src/files/archive_ffi.rs opens it at run time


def elf_needed(path: Path) -> list[str] | None:
    """DT_NEEDED of a 64-bit little-endian ELF file, read without running it (None: another format)."""
    try:
        with open(path, "rb") as stream:
            head = stream.read(64)
            if head[:4] != b"\x7fELF" or head[4] != 2 or head[5] != 1:
                return None
            phoff, = struct.unpack_from("<Q", head, 0x20)
            phentsize, phnum = struct.unpack_from("<HH", head, 0x36)
            stream.seek(phoff)
            table = stream.read(phentsize * phnum)
            loads, dynamic = [], None
            for i in range(phnum):
                kind, _flags, offset, vaddr, _paddr, filesz, _memsz, _align = struct.unpack_from("<IIQQQQQQ", table, i * phentsize)
                if kind == 1:
                    loads.append((vaddr, offset, filesz))
                elif kind == 2:
                    dynamic = (offset, filesz)
            if dynamic is None:
                return []
            stream.seek(dynamic[0])
            raw = stream.read(dynamic[1])
            entries = [struct.unpack_from("<qQ", raw, i) for i in range(0, len(raw) - 15, 16)]
            strtab = next((v for tag, v in entries if tag == 5), None)
            strsz = next((v for tag, v in entries if tag == 10), 0)
            needed = [v for tag, v in entries if tag == 1]
            base = next((o + strtab - va for va, o, size in loads if va <= strtab < va + size), None) if strtab else None
            if base is None:
                return None
            stream.seek(base)
            strings = stream.read(strsz)
            return [strings[i:strings.index(b"\0", i)].decode("utf-8", "replace") for i in needed]
    except (OSError, struct.error, ValueError):
        return None


def known_libraries() -> set[str]:
    names = set()
    ldconfig = next((p for p in ("/sbin/ldconfig", "/usr/sbin/ldconfig") if os.access(p, os.X_OK)), None)
    if ldconfig:
        try:
            out = subprocess.run([ldconfig, "-p"], capture_output=True, text=True, timeout=30).stdout
            names |= {m.group(1) for m in re.finditer(r"^\s+(\S+) \(([^)]*)\)", out, re.M) if "64" in m.group(2)}
        except (OSError, subprocess.SubprocessError):
            pass
    dirs = [d for d in os.environ.get("LD_LIBRARY_PATH", "").split(":") if d]
    dirs += ["/lib/x86_64-linux-gnu", "/usr/lib/x86_64-linux-gnu", "/lib/aarch64-linux-gnu", "/usr/lib/aarch64-linux-gnu",
             "/lib64", "/usr/lib64", "/lib", "/usr/lib"]
    for directory in dirs:
        try:
            names |= set(os.listdir(directory))
        except OSError:
            pass
    return names


def preflight_runtime(build: Path) -> None:
    """The system parts the installed apps and this installer need (gnome-map §6 NucBox; plan-critique F21)."""
    known = known_libraries()
    needed = []
    for name in BINS:
        for library in elf_needed(binary_source(build, name)) or []:
            if library not in needed:
                needed.append(library)
    needed += [library for library in DLOPENED if library not in needed]
    missing = [library for library in needed if library not in known]
    packages = [PACKAGES[library] for library in missing if library in PACKAGES]
    try:
        import gi
        gi.require_version("Gio", "2.0")
        from gi.repository import Gio  # noqa: F401
    except (ImportError, ValueError):
        missing.append("python3-gi")
        packages.append("python3-gi")
    if not shutil.which("glib-compile-schemas"):
        missing.append("glib-compile-schemas")
        packages.append("libglib2.0-bin")
    if missing:
        lines = [t("missingParts", missing=", ".join(missing))]
        if packages:
            lines.append(t("installParts", packages=" ".join(dict.fromkeys(packages))))
        raise SystemExit("\n".join(lines))
    if not shutil.which("update-desktop-database"):
        print(t("noDesktopDatabase"), file=sys.stderr)


def load_entries(root: Path) -> dict:
    """host/src/desktop-entries.json (C6): Name= and Comment= of the own .desktop files in nl and en."""
    path = root/"host/src/desktop-entries.json"
    try:
        table = json.loads(path.read_text(encoding="utf-8"))
        pairs = [table["comment"]] + [table["entries"][name] for name in ENTRIES]
        if table.get("version") != 1 or not all(isinstance(p.get(k), str) and p[k] and "\n" not in p[k]
                                                 for p in pairs for k in LANGUAGES):
            raise ValueError
    except (OSError, ValueError, KeyError, TypeError, AttributeError):
        raise SystemExit(t("badTable", file=path))
    return table


def load_defaults(root: Path) -> dict:
    path = root/"host/src/settings-defaults.json"
    try:
        defaults = json.loads(path.read_text(encoding="utf-8"))
        if not isinstance(defaults, dict):
            raise ValueError
    except (OSError, ValueError):
        raise SystemExit(t("badTable", file=path))
    return defaults


def check_config_dir(directory: Path) -> None:
    """The host's own rule for its settings folder (settings.rs SettingsStore::new): ours, a folder, no link."""
    if os.path.lexists(directory):
        info = directory.lstat()
        if directory.is_symlink() or not directory.is_dir() or info.st_uid != os.geteuid():
            raise SystemExit(t("configNotOwn", path=directory))


def store_language(directory: Path, defaults: dict, language: str) -> bool:
    """Write a complete settings.json (never a partial one: GE0c F3) with this language, the way the host writes:
    under .settings.lock, a 0600 temporary file, then rename. An existing settings.json is never replaced."""
    check_config_dir(directory)
    directory.mkdir(mode=0o700, parents=True, exist_ok=True)
    target = directory/"settings.json"
    lock = os.open(directory/".settings.lock", os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW | os.O_CLOEXEC, 0o600)
    try:
        fcntl.flock(lock, fcntl.LOCK_EX)
        if os.path.lexists(target):
            return False
        temporary = directory/f".settings-install-{os.getpid()}.tmp"
        fd = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC, 0o600)
        try:
            with os.fdopen(fd, "w", encoding="utf-8") as stream:
                stream.write(json.dumps({**defaults, "language": language}, indent=2, ensure_ascii=False) + "\n")
                stream.flush()
                os.fsync(stream.fileno())
            os.rename(temporary, target)
        finally:
            if os.path.lexists(temporary):
                temporary.unlink()
        return True
    finally:
        os.close(lock)


def language_lines(choice: dict, explicit: str | None) -> list[str]:
    lang, source, detail = choice["lang"], choice["source"], choice["detail"]
    if source in ("locale", "default"):
        if detail and detail.startswith("tz:"):
            return [t("langZone", zone=detail[3:])]
        if detail and detail.startswith("lc:"):
            return [t("langFormats", var=detail[3:])]
        return [t("langChosen")]
    name = t("name_en") if lang == "en" else t("name_nl")
    if source == "hyprland":
        return [t("langMigrated", name=name, file=detail)]
    if source == "settings" and explicit and explicit != lang:
        return [t("langKept", lang=explicit, name=name, file=detail, path=t("settingsPath", lang))]
    return []


def set_scheme_default(config: Path, desktop_id: str, mime: str) -> None:
    """xdg-mime default (else gio mime) for one of this project's own URI schemes, never for another type: one line
    in $XDG_CONFIG_HOME/mimeapps.list, which uninstall.py removes again. A missing tool is no error: the entry's
    MimeType= alone already makes it the only handler."""
    env = {**os.environ, "XDG_CONFIG_HOME": str(config)}
    if shutil.which("xdg-mime"):
        command = ["xdg-mime", "default", desktop_id, mime]
    elif shutil.which("gio"):
        command = ["gio", "mime", mime, desktop_id]
    else:
        return
    subprocess.run(command, env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False)


def install_extension(root: Path, extension: Path) -> None:
    """Every file of extension/ except the schemas (contracts C7, plan-critique F18), so a new module is never
    left out; a top-level file this version does not ship is removed from its own extension folder, and each one
    is named (ge1c-review m3: nothing goes without a trace)."""
    extension.mkdir(parents=True, exist_ok=True)
    shipped = {source.name for source in (root/"extension").iterdir() if source.is_file()}
    for name in sorted(shipped):
        shutil.copy2(root/"extension"/name, extension/name)
    for stale in sorted(extension.iterdir()):
        if stale.name not in shipped and stale.is_file() and not stale.is_symlink():
            stale.unlink()
            print(t("staleRemoved", path=stale))
    schemas = extension/"schemas"
    schemas.mkdir(exist_ok=True)
    shutil.copy2(root/"extension/schemas/org.gnome.shell.extensions.win11-gnome.gschema.xml", schemas)
    subprocess.run(["glib-compile-schemas", "--strict", str(schemas)], check=True)


def remove_legacy(prefix: Path, applications: Path) -> None:
    """D-G70: the binaries and entries of an earlier version's WebKit Start and desktop (LEGACY_BINS, LEGACY_ENTRIES).
    Only files and links are removed; anything else at those paths is left alone."""
    paths = [prefix/"bin"/name for name in LEGACY_BINS]
    paths += [applications/f"io.github.win11shell.{name}.desktop" for name in LEGACY_ENTRIES]
    for path in paths:
        if path.is_symlink() or path.is_file():
            path.unlink()


def main(root: Path, argv: list[str] | None = None) -> None:
    explicit = parse_arguments(list(argv or []))
    prefix = absolute_path(os.environ.get("WIN11_INSTALL_PREFIX", str(Path.home()/".local")), t("labelPrefix"))
    data = absolute_path(os.environ.get("XDG_DATA_HOME", str(Path.home()/".local/share")), "XDG_DATA_HOME")
    config = absolute_path(os.environ.get("XDG_CONFIG_HOME", str(Path.home()/".config")), "XDG_CONFIG_HOME")
    payload = prefix/"share/win11-gnome"
    choice = resolve_language(explicit, config, data, payload)
    language = choice["lang"]
    set_language(explicit or language)
    build = choose_build(root)
    extension = data/"gnome-shell/extensions"/UUID
    applications = data/"applications"
    for directory in (root/"vendor/ui", root/"vendor/host"):
        if not directory.is_dir():
            raise SystemExit(t("missingSource", directory=directory))
    entries = load_entries(root)
    defaults = load_defaults(root) if choice["store"] else None
    preflight_runtime(build)
    if payload.exists() and (not (payload/MARKER).is_file() or (payload/MARKER).read_text() != MAGIC):
        raise SystemExit(t("notOwned", payload=payload))
    updating = payload.exists()
    if not updating:
        conflicts = [prefix/"bin"/name for name in BINS]
        conflicts += [applications/f"io.github.win11shell.{name}.desktop" for name in ENTRIES]
        conflicts += [extension, config/"autostart"/AUTOSTART]
        existing = [str(path) for path in conflicts if path.exists()]
        if existing:
            raise SystemExit(t("conflicts") + "\n" + "\n".join(existing))
    if choice["store"]:
        check_config_dir(config/"win11-gnome")
    icon_bundle.validate(root/"vendor")
    previous_icons = icon_bundle.preflight_aliases(payload, data)
    portal_plan = file_portal.preflight(payload, prefix, data, config)
    # The choice is stored after every check and before the first change (ge1c-review m1): when a later step
    # fails (glib-compile-schemas, update-desktop-database, a portal or icon step), the rerun finds settings.json
    # and keeps this language, instead of taking the update-without-choice path (nl, H D13). It never replaces
    # an existing file. The language line goes out with it, as the first line, as before.
    if choice["store"]:
        store_language(config/"win11-gnome", defaults, language)
    for line in language_lines(choice, explicit):
        print(line)
    payload.parent.mkdir(parents=True, exist_ok=True)
    stage = Path(tempfile.mkdtemp(prefix=".win11-gnome-install-", dir=payload.parent))
    try:
        # config/: the Windows 11 caption buttons for other GTK apps (D-G65), imported from gtk.css by the panel host
        for directory in ("ui", "host", "config"):
            if not (root/"vendor"/directory).is_dir():
                continue
            shutil.copytree(root/"vendor"/directory, stage/directory,
                            ignore=shutil.ignore_patterns("__pycache__", "*.pyc"))
        for manifest in ("ui-sha256.json", "ui-original-sha256.json", "ui-patches.json"):
            if (root/"vendor"/manifest).is_file():
                shutil.copy2(root/"vendor"/manifest, stage/manifest)
        icon_bundle.copy_into_payload(root, stage)
        (stage/MARKER).write_text(MAGIC)
        if payload.exists():
            backup = payload.with_name(".win11-gnome-previous")
            if backup.exists():
                raise SystemExit(t("previousCopy", backup=backup))
            payload.rename(backup)
            try:
                stage.rename(payload)
            except BaseException:
                backup.rename(payload)
                raise
            shutil.rmtree(backup)
        else:
            stage.rename(payload)
    finally:
        if stage.exists():
            shutil.rmtree(stage)
    (prefix/"bin").mkdir(parents=True, exist_ok=True)
    for binary in BINS:
        # Replace through a temporary file so running app processes keep their original inode.
        destination = prefix/"bin"/binary
        temporary = destination.with_name(f".{binary}.new")
        shutil.copy2(binary_source(build, binary), temporary)
        temporary.chmod(0o755)
        temporary.replace(destination)
    if updating:
        # D-G70: an earlier version's WebKit Start and desktop (the payload is ours, so these files are too)
        remove_legacy(prefix, applications)
    icon_bundle.register_aliases(payload, data, previous_icons)
    file_portal.register(payload, portal_plan)
    install_extension(root, extension)
    applications.mkdir(parents=True, exist_ok=True)
    common = "Terminal=false\nStartupNotify=true\n"
    # Name= and Comment= in Win11's language (C6); win11-panel later rewrites only these two lines after a switch.
    comment = entries["comment"][language]
    # The Windows 11 calendar of the bundled Win11 icon theme (vendor/icons, icon_bundle.py), else the theme's own
    calendar_icon = payload/"icons/Win11/apps/scalable/office-calendar.svg"
    rows = [
        ("Panel", "win11-panel", "view-app-grid-symbolic", "NoDisplay=true\nCategories=Utility;\n"),
        ("Settings", "win11-app", str(payload/"ui/assets/icons/settings.svg"), "NoDisplay=true\nCategories=Settings;\n"),
        ("DefaultApps", "win11-default-apps", str(payload/"ui/assets/icons/settings.svg"), "Categories=Settings;System;\n"),
        ("Installer", "win11-installer", "system-software-install", "Categories=System;PackageManager;\n"),
        ("Search", "win11-app", "system-search", "NoDisplay=true\nCategories=Utility;\n"),
        ("Disks", "win11-app", "drive-harddisk", "NoDisplay=true\nCategories=System;\n"),
        ("TaskManager", "win11-app", "utilities-system-monitor", "NoDisplay=true\nCategories=System;Monitor;\n"),
        ("Explorer", "win11-explorer", "system-file-manager",
         "Categories=System;FileTools;FileManager;\nMimeType=inode/directory;\n"),
        ("Calendar", "win11-calendar", str(calendar_icon) if calendar_icon.is_file() else "x-office-calendar",
         f"Categories=Office;Calendar;\nMimeType={CALENDAR_SCHEME};\n"),
    ]
    for name, binary, icon, extra in rows:
        title = entries["entries"][name][language]
        text = f"[Desktop Entry]\nType=Application\nName={title}\n"
        text += f"Comment={comment}\n"
        arguments = " %U" if name == "Explorer" else {"TaskManager": " --app taskmgr", "Settings": " --app settings", "Search": " --app alles", "Disks": " --app disks", "Calendar": " %u"}.get(name, "")
        text += f"Exec=/usr/bin/env -- {desktop_exec(prefix/'bin'/binary)}{arguments}\n"
        text += f"Icon={icon}\n" + common + f"StartupWMClass=io.github.win11shell.{name}\n" + extra
        (applications/f"io.github.win11shell.{name}.desktop").write_text(text)
    autostart = config/"autostart"
    autostart.mkdir(parents=True, exist_ok=True)
    (autostart/AUTOSTART).write_text(autostart_entry(prefix, language))
    if shutil.which("update-desktop-database"):
        subprocess.run(["update-desktop-database", str(applications)], check=True)
    set_scheme_default(config, CALENDAR_DESKTOP, CALENDAR_SCHEME)
    print(t("done1", prefix=prefix))
    print(t("done2"))
    print(t("done3"))
    print(t("done4"))
    print(t("done5", uuid=UUID))
    print(t("done6"))
    hint = system_hint(False)
    if hint and sys.stdout.isatty():
        print(hint)


if __name__ == "__main__":
    main(Path(sys.argv[1]).resolve(), sys.argv[2:])
