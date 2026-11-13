#!/usr/bin/env python3
"""Win11 Shell preflight: can this Python run the desktop (layer mode), and can the session start?

Loads exactly what the host loads, in the host's order (surfaces.gi_modules / _layer_shell / _start_layer), but
never opens a display: as a script it drops WAYLAND_DISPLAY/DISPLAY first, so Gtk.init_check() just returns False.
Stdlib + gi only and no package-relative imports: it runs as a script under ``python3 -I`` (install.sh, the session
script, bin/win11shell, the host's retry poll) and is imported as ``host.preflight`` by the host itself.
Design: notes/session-failure-ux.md §3.

  python3 -I host/preflight.py [--scope layer|session] [--packages | --degraded | --json | --quiet]
      exit 0: nothing blocking is missing   1: something blocking is missing   2: usage

--packages prints the missing Ubuntu packages (blocking ones) on one line, e.g. "hyprland python3-gi-cairo";
--degraded the missing non-blocking ones (scope session).
"""
from __future__ import annotations

import json
import os
import shutil
import sys
import sysconfig

# key: (Ubuntu package, Dutch, English) — blocking: without these the desktop cannot start
PARTS = {
    "python3-gi": ("python3-gi", "de Python-koppeling met GTK (PyGObject)", "the Python GTK bindings (PyGObject)"),
    "gtk3": ("gir1.2-gtk-3.0", "GTK 3", "GTK 3"),
    "webkit": ("gir1.2-webkit2-4.1", "de webweergave WebKitGTK 4.1", "the WebKitGTK 4.1 web view"),
    "layershell": ("gir1.2-gtklayershell-0.1", "gtk-layer-shell (taakbalk en bureaublad als vaste lagen)",
                   "gtk-layer-shell (taskbar and desktop layers)"),
    "gicairo": ("python3-gi-cairo", "de koppeling tussen PyGObject en cairo (klikgebieden van de taakbalk)",
                "the PyGObject-cairo bridge (taskbar input regions)"),
    "hyprland": ("hyprland", "Hyprland (de vensterbeheerder van Win11 Shell)", "Hyprland (the compositor)"),
}
# key: (Ubuntu package, command or absolute path, Dutch, English) — the session starts without these
DEGRADED = {
    "hyprlock": ("hyprlock", "hyprlock", "het scherm vergrendelen", "locking the screen"),
    "portal": ("xdg-desktop-portal-hyprland", "/usr/libexec/xdg-desktop-portal-hyprland",
               "scherm delen en bestandsvensters in sommige apps", "screen sharing and some file dialogs"),
    "grim": ("grim", "grim", "schermafbeeldingen", "screenshots"),
    "slurp": ("slurp", "slurp", "een schermafbeelding van een gebied", "area screenshots"),
    "wlclip": ("wl-clipboard", "wl-paste", "de klembordgeschiedenis", "clipboard history"),
    "notify": ("libnotify-bin", "notify-send", "meldingen van de hulpscripts", "notifications from helper scripts"),
}
TYPELIBS = {"gtk3": "Gtk-3.0", "webkit": "WebKit2-4.1", "layershell": "GtkLayerShell-0.1"}


def _typelib_present(name: str) -> bool:
    dirs = [d for d in os.environ.get("GI_TYPELIB_PATH", "").split(":") if d]
    arch = sysconfig.get_config_var("MULTIARCH") or ""
    dirs += [f"/usr/lib/{arch}/girepository-1.0", "/usr/lib64/girepository-1.0", "/usr/lib/girepository-1.0"]
    return any(os.path.exists(os.path.join(d, name + ".typelib")) for d in dirs)


def _python_missing(load_webkit: bool) -> list[tuple[str, str]]:
    """[(key, error)] for each missing Python part, loaded the way the host loads it."""
    try:
        import gi
        if not hasattr(gi, "require_version"):    # a leftover empty gi/ directory imports as a namespace package
            raise ImportError(f"gi in {list(getattr(gi, '__path__', []))} is leeg")
    except Exception as e:  # noqa: BLE001
        # without gi nothing can be imported: look at the files, so one apt command covers everything
        out = [("python3-gi", f"{type(e).__name__}: {e}"), ("gicairo", "python3-gi ontbreekt")]
        return out + [(k, "typelib ontbreekt") for k, t in TYPELIBS.items() if not _typelib_present(t)]
    missing: list[tuple[str, str]] = []

    def step(key, fn):
        try:
            fn()
        except Exception as e:  # noqa: BLE001 - ValueError 'Namespace … not available', ImportError, GLib.Error
            missing.append((key, f"{type(e).__name__}: {e}"))

    def gtk():
        gi.require_version("Gdk", "3.0")
        gi.require_version("Gtk", "3.0")
        from gi.repository import Gdk, Gio, GLib, Gtk  # noqa: F401

    def webkit():
        gi.require_version("WebKit2", "4.1")      # typelib lookup only
        if load_webkit:                           # the host itself must not: its GPU switches come first
            from gi.repository import WebKit2  # noqa: F401

    def layershell():
        gi.require_version("GtkLayerShell", "0.1")
        from gi.repository import GtkLayerShell  # noqa: F401

    def gicairo():
        gi.require_foreign("cairo")
        import cairo  # noqa: F401

    step("gtk3", gtk)
    if missing:                                   # WebKit2 and GtkLayerShell need Gtk: check their files
        missing += [(k, "typelib ontbreekt") for k in ("webkit", "layershell") if not _typelib_present(TYPELIBS[k])]
    else:
        step("webkit", webkit)
        step("layershell", layershell)
    step("gicairo", gicairo)
    return missing


def _exe(cmd: str) -> bool:
    return os.access(cmd, os.X_OK) if os.path.isabs(cmd) else shutil.which(cmd) is not None


def check(scope: str = "layer", in_process: bool = False) -> dict:
    """{'ok', 'missing': [{key, package, nl, en, error}], 'degraded': [...], 'packages': [...], 'command'}.
    scope 'layer': what the host needs for layer mode; 'session': also Hyprland and the non-blocking helpers.
    in_process: called by the running host (Gtk already imported; WebKit2 is only looked up, not loaded)."""
    found = _python_missing(load_webkit=not in_process)
    if scope == "session" and not (_exe("Hyprland") or _exe("start-hyprland")):
        found.append(("hyprland", "Hyprland staat niet in PATH"))
    missing = [{"key": k, "package": PARTS[k][0], "nl": PARTS[k][1], "en": PARTS[k][2], "error": err}
               for k, err in found]
    degraded = []
    if scope == "session":
        degraded = [{"key": k, "package": p, "nl": nl, "en": en} for k, (p, cmd, nl, en) in DEGRADED.items()
                    if not _exe(cmd)]
    pkgs = list(dict.fromkeys(m["package"] for m in missing))
    return {"ok": not missing, "missing": missing, "degraded": degraded, "packages": pkgs,
            "command": ("sudo apt install " + " ".join(pkgs)) if pkgs else ""}


def main(argv: list[str]) -> int:
    scope, out = "layer", "text"
    it = iter(argv)
    for a in it:
        if a == "--scope":
            scope = next(it, "")
        elif a in ("--packages", "--degraded", "--json", "--quiet"):
            out = a[2:]
        else:
            print(__doc__, file=sys.stderr)
            return 2
    if scope not in ("layer", "session"):
        return 2
    for k in ("WAYLAND_DISPLAY", "DISPLAY", "WAYLAND_SOCKET"):   # never touch a display
        os.environ.pop(k, None)
    os.environ["NO_AT_BRIDGE"] = "1"
    r = check(scope)
    if out == "json":
        print(json.dumps(r, ensure_ascii=False))
    elif out == "packages":
        if r["packages"]:
            print(" ".join(r["packages"]))
    elif out == "degraded":
        pk = list(dict.fromkeys(d["package"] for d in r["degraded"]))
        if pk:
            print(" ".join(pk))
    elif out == "text":
        for m in r["missing"]:
            print(f"ontbreekt: {m['package']} — {m['nl']} ({m['error']})")
        for d in r["degraded"]:
            print(f"handig: {d['package']} — {d['nl']}")
        print(f"installeer: {r['command']}" if r["command"] else "alles aanwezig")
    return 0 if r["ok"] else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
