"""windows.* (SPEC §6.4): open windows via Hyprland IPC (unsupported elsewhere).

Request socket ``$XDG_RUNTIME_DIR/hypr/$HYPRLAND_INSTANCE_SIGNATURE/.socket.sock``
(``j/clients``, ``j/activewindow``, ``j/activeworkspace``, ``j/monitors``, ``dispatch …``,
``[[BATCH]]cmd;cmd``) and the event socket ``.socket2.sock`` (read in a thread; window and
workspace events → debounced ``windows.changed {windows}`` + ``windows.focused {id}``).

Behaviour (Hyprland 0.53 dispatcher syntax):
* minimize   ``movetoworkspacesilent special:minimized,address:0x…``
* restore    ``movetoworkspacesilent <active ws>,address:…`` + ``focuswindow`` + ``alterzorder top``
* maximize   ``focuswindow`` + ``fullscreen 1`` (Hyprland's maximize mode: fills the monitor minus
             reserved areas such as our taskbar spacer, also for floating windows; toggles)
* snap       ``setfloating`` + ``resizewindowpixel exact W H,address:…`` + ``movewindowpixel exact X Y,…``
             computed from ``j/monitors`` (logical size = pixels / scale, minus ``reserved``
             [left, top, right, bottom]); the pre-snap geometry is remembered for ``restore``.
* apps asking to be minimized (xdg ``set_minimized`` → event ``minimized>>ADDR,1``) really get minimized.
* always on top ``windows.setPinned {id|pid, pinned}`` → Hyprland ``pin`` (floating windows only; a pinned
             window is drawn above the others and shown on every workspace). Pinned windows cannot live on a
             special workspace, so minimize / show-desktop unpin first and restore pins them again.
* ``desktopId``: window class → desktop entry (StartupWMClass, desktop basename, snap ``name_name``,
             flatpak/reverse-DNS ids, executable, icon, name); when the class says nothing, the window's pid
             is used (``GIO_LAUNCHED_DESKTOP_FILE`` of that pid, systemd app scope, executable).

* window animations (SPEC §6.4.1, plugin win11-csd ≥ 1.6): minimize / restore batches start with the
             optional hint ``win11csd:anim minimize|restore 0x… [x y w h] [thumb]`` (global logical px of the
             taskbar button from ``windows.setIconRects``, or of the Task View / Alt+Tab card from
             ``fromRect``), right before that window's ``movetoworkspacesilent``. Only sent while the plugin
             answers ``dispatch win11csd:anim probe`` with ``ok`` (cached; probed again after
             ``configreloaded`` / ``win11csd>>loaded``) and neither reduce motion nor safe mode is on, so
             without the plugin every batch is exactly what it was before. Replies are checked per command;
             the optional ones (hint, ``keyword plugin:win11-csd:…``) never fail a batch.
             ``accessibility.reduceMotion`` → ``keyword plugin:win11-csd:reduce_motion``, the crash guard's
             ``<config>/win11-csd-safe-mode`` → ``keyword plugin:win11-csd:safe_mode`` (pushed again after
             every config reload, which resets runtime keywords).
* maximized / fullscreen windows keep that state through minimize + restore: Hyprland un-maximizes a
             window on ``special:minimized`` when another maximized window arrives there, so the state is
             remembered at minimize and applied again on restore. With plugin win11-csd ≥ 1.6.1 (it answers
             ``dispatch win11csd:fullscreen probe`` with ``ok``) inside the restore batch, right after the
             window's move: ``win11csd:fullscreen 0x… 1|2`` (maximized / fullscreen of exactly that window), so
             the restore animation grows into the maximized box (also with reduce motion; never in safe mode).
             Otherwise (and when that command is not answered ``ok``) ``fullscreen 1|0`` after the batch, once
             the window really is the active one.
* two maximized windows on one workspace (Windows keeps both maximized; Hyprland has one fullscreen window per
             workspace and un-maximizes the other one, which then hides behind it as a floating window): such a
             "displaced" window is remembered (window list: ``maximized`` true, ``displaced`` true) and comes back
             maximized when it is activated (taskbar, Alt+Tab, Task View, an app's activation), when the window that
             displaced it is minimized, closed or moved away: ``win11csd:fullscreen 0x… 1 instant`` (plugin ≥ 1.6.2:
             no zoom, no morph; it answers ``dispatch win11csd:fullscreen probe instant`` with ``ok``), else
             ``fullscreen 1`` once it is the active window (only on activation). An explicit restore / snap / maximize
             of it, or of the window that displaced it (un-maximize), ends that: it stays floating, as before.

Window extras beyond SPEC: ``maximized``, ``urgent``, ``snapped``, ``x y w h``, ``monitor``,
``workspaceName``, ``xwayland``, ``pinned``, ``initialClass``.
"""

from __future__ import annotations

import json
import logging
import math
import os
import re
import socket
import threading
import time
from pathlib import Path
from typing import Any

from ..util import ApiError, hypr_socket_path, json_dumps
from .apps import Debouncer, get_catalog

log = logging.getLogger("win11shell.windows")

PRIORITY = 30

MIN_WS = "special:minimized"

# Commands of the win11-csd plugin: their answers never make a batch fail (the plugin may be missing,
# or unloaded between the probe and the batch: Hyprland then answers "Invalid dispatcher" / an error).
OPTIONAL_CMDS = ("dispatch win11csd:", "keyword plugin:win11-csd:")
ANIM_PROBE = "dispatch win11csd:anim probe"
FS_PROBE = "dispatch win11csd:fullscreen probe"   # plugin ≥ 1.6.1: fullscreen / maximize of one given window
FS_INSTANT_PROBE = "dispatch win11csd:fullscreen probe instant"   # plugin ≥ 1.6.2: '… 0x… 1 instant' (no zoom / morph)
EXPLICIT_HOLD = 1.5   # s: a window the shell itself just maximized / restored / snapped is never "displaced"
SAFE_MODE_FILE = "win11-csd-safe-mode"   # in the config dir; written by win11shell-session's crash guard
SHELL_LAYER = "win11shell-shell"         # the layer surface the taskbar / Task View rects are relative to
_HINT_ADDR_RE = re.compile(r"^0x[0-9a-f]{1,16}$")   # what the plugin's dispatcher accepts
REAPPLY_WAIT = 5.0   # s: a P-1 re-maximize whose focuswindow Hyprland refused waits this long for that window's focus

# Event names that (may) change the window list. Everything else is ignored.
_REFRESH_EVENTS = {
    "openwindow", "closewindow", "movewindow", "movewindowv2", "windowtitle", "windowtitlev2",
    "activewindow", "activewindowv2", "fullscreen", "changefloatingmode", "workspace", "workspacev2",
    "focusedmon", "focusedmonv2", "createworkspace", "createworkspacev2", "destroyworkspace",
    "destroyworkspacev2", "moveworkspace", "moveworkspacev2", "renameworkspace", "activespecial",
    "activespecialv2", "monitorremoved", "monitorremovedv2", "monitoradded", "monitoraddedv2", "pin",
    "minimized", "urgent", "togglegroup", "moveintogroup", "moveoutofgroup", "configreloaded",
}


# ---------------------------------------------------------------------------
# Hyprland IPC client
# ---------------------------------------------------------------------------

def unix_connect(path: str, timeout: float) -> socket.socket:
    """connect() that also works for socket paths longer than sun_path (108 bytes)."""
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.settimeout(timeout)
    p = os.fspath(path)
    try:
        if len(os.fsencode(p)) < 100:
            s.connect(p)
        else:
            fd = os.open(os.path.dirname(p) or ".", os.O_PATH | os.O_DIRECTORY | os.O_CLOEXEC)
            try:
                s.connect(f"/proc/self/fd/{fd}/{os.path.basename(p)}")
            finally:
                os.close(fd)
    except BaseException:
        s.close()
        raise
    return s


class Hypr:
    """Tiny synchronous Hyprland IPC client (no hyprctl subprocess)."""

    def __init__(self, timeout: float = 2.0):
        self.timeout = timeout
        # called when a batch shows that the win11-csd dispatcher is gone ("Invalid dispatcher")
        self.on_optional_missing = lambda: None

    @staticmethod
    def socket_path(kind: str = ".socket.sock") -> str | None:
        p = hypr_socket_path(kind)
        return str(p) if p else None

    def available(self) -> bool:
        return self.socket_path() is not None

    def raw(self, msg: str) -> str | None:
        path = self.socket_path()
        if not path:
            return None
        try:
            with unix_connect(path, self.timeout) as s:
                s.sendall(msg.encode("utf-8"))
                chunks = []
                while True:
                    b = s.recv(65536)
                    if not b:
                        break
                    chunks.append(b)
        except OSError as e:
            log.debug("Hyprland-verzoek mislukt (request failed) %r: %s", msg[:80], e)
            return None
        return b"".join(chunks).decode("utf-8", "replace")

    def json(self, command: str) -> Any:
        text = self.raw("j/" + command)
        if text is None:
            return None
        try:
            return json.loads(text)
        except ValueError:
            log.debug("Geen JSON van Hyprland (no JSON) voor %s: %r", command, text[:120])
            return None

    def batch(self, commands: list[str], replies: list | None = None) -> bool:
        """Run 'dispatch …' / 'keyword …' commands in one request. True when all answered ok.
        replies: a list that gets (command, answer) per command when Hyprland's answer has that shape
        (stays empty otherwise, e.g. no answer at all)."""
        commands = [c.strip() for c in commands if c and c.strip()]
        if not commands:
            return True
        if any(";" in c for c in commands):
            raise ApiError("bad_args", "Ongeldige opdracht.")
        msg = commands[0] if len(commands) == 1 else "[[BATCH]]" + ";".join(commands)
        reply = self.raw(msg)
        if reply is None:
            return False
        # Hyprland answers "ok" per command; a batch joins the answers with "\n\n\n" (HyprCtl.cpp
        # dispatchBatch). Anything else is an error, except for the optional plugin commands.
        parts = reply.split("\n\n\n") if len(commands) > 1 else [reply]
        if len(parts) == len(commands):
            if replies is not None:
                replies.extend((c, p.strip()) for c, p in zip(commands, parts))
            if any(c.startswith("dispatch win11csd:") and "Invalid dispatcher" in p for c, p in zip(commands, parts)):
                try:
                    self.on_optional_missing()
                except Exception:  # noqa: BLE001
                    log.exception("on_optional_missing faalde (failed)")
            bad = [p for c, p in zip(commands, parts) if not c.startswith(OPTIONAL_CMDS) and p.replace("ok", "").strip()]
            rest = " ".join(" ".join(p.split()) for p in bad)
            for c, p in zip(commands, parts):
                if c.startswith(OPTIONAL_CMDS) and p.strip() != "ok":
                    log.debug("Optionele opdracht (optional command) %s → %s", c[:120], " ".join(p.split())[:120])
        else:   # unexpected shape: the old rule, minus one "Invalid dispatcher" per optional command
            rest = reply
            for c in commands:
                if c.startswith(OPTIONAL_CMDS):
                    rest = rest.replace("Invalid dispatcher", "", 1)
            rest = rest.replace("ok", "").strip()
        if rest:
            log.info("Hyprland meldt (reports): %s → %s", msg[:200], " ".join(rest.split())[:300])
            return False
        return True

    def dispatch(self, *commands: str, replies: list | None = None) -> bool:
        return self.batch([f"dispatch {c}" for c in commands], replies=replies)


def _answered_ok(replies: list, command: str) -> bool:
    """Did `command` (without 'dispatch ') answer 'ok' in a batch whose replies were collected?"""
    return any(c == "dispatch " + command and p == "ok" for c, p in replies)


def _addr(value: Any) -> str | None:
    """Normalise a window address ('55d0…', '0x55d0…') → '0x55d0…'."""
    if value is None:
        return None
    s = str(value).strip().lower()
    if not s or s in ("0x0", "0", ","):
        return None
    if not s.startswith("0x"):
        s = "0x" + s
    if not all(c in "0123456789abcdef" for c in s[2:]) or len(s) > 20:
        return None
    return s


def _fs_state(c: dict) -> tuple[bool, bool]:
    """(fullscreen, maximized) from a client dict (0.42+: int bitmask; older: bool + fullscreenMode)."""
    fs = c.get("fullscreen")
    if isinstance(fs, bool):
        mode = c.get("fullscreenMode", 0)
        return (fs and mode != 1, fs and mode == 1)
    try:
        fs = int(fs or 0)
    except (TypeError, ValueError):
        fs = 0
    full = bool(fs & 2)
    return full, bool(fs & 1) and not full


def _real_max(w: dict) -> bool:
    """Maximized in Hyprland (the window list also reports a 'displaced' window as maximized)."""
    return bool(w["maximized"]) and not w.get("displaced")


def _num(v: Any, lo: float, hi: float, positive: bool = False) -> float:
    if isinstance(v, bool) or not isinstance(v, (int, float)) or not math.isfinite(v):
        raise ApiError("bad_args", "Ongeldige rechthoeken.")
    if not lo <= v <= hi or (positive and v <= 0):
        raise ApiError("bad_args", "Ongeldige rechthoeken.")
    return float(v)


def _rect_arg(r: Any, max_wh: float) -> dict:
    """{x, y, w, h} in layer-local CSS px: finite, |x|,|y| ≤ 1e5, 0 < w,h ≤ max_wh (else bad_args)."""
    if not isinstance(r, dict):
        raise ApiError("bad_args", "Ongeldige rechthoeken.")
    return {"x": _num(r.get("x"), -1e5, 1e5), "y": _num(r.get("y"), -1e5, 1e5),
            "w": _num(r.get("w"), 0, max_wh, positive=True), "h": _num(r.get("h"), 0, max_wh, positive=True)}


def _hint_box(rect: dict | None) -> list[str] | None:
    """The four integers of a hint (global logical px), or None when the plugin would refuse them
    (|x|,|y| ≤ 100000, 1 ≤ w,h ≤ 4096): then the hint goes without a rect (plugin: bottom-centre fallback)."""
    if not rect:
        return None
    try:
        x, y, w, h = (int(round(float(rect[k]))) for k in ("x", "y", "w", "h"))
    except (KeyError, TypeError, ValueError, OverflowError):
        return None
    if abs(x) > 100000 or abs(y) > 100000 or not (1 <= w <= 4096 and 1 <= h <= 4096):
        return None
    return [str(x), str(y), str(w), str(h)]


def logical_monitor(m: dict) -> dict:
    """Monitor geometry in layout (logical) coordinates + its work area."""
    scale = float(m.get("scale") or 1.0) or 1.0
    w, h = int(m.get("width") or 0), int(m.get("height") or 0)
    if int(m.get("transform") or 0) % 2 == 1:
        w, h = h, w
    lw, lh = int(round(w / scale)), int(round(h / scale))
    x, y = int(m.get("x") or 0), int(m.get("y") or 0)
    res = m.get("reserved") or [0, 0, 0, 0]
    try:
        rl, rt, rr, rb = (int(v) for v in list(res)[:4])
    except (TypeError, ValueError):
        rl = rt = rr = rb = 0
    return {"x": x, "y": y, "w": lw, "h": lh,
            "work": {"x": x + rl, "y": y + rt, "w": max(100, lw - rl - rr), "h": max(100, lh - rt - rb)}}


def snap_rect(work: dict, edge: str) -> dict | None:
    x0, y0, W, H = work["x"], work["y"], work["w"], work["h"]
    hw, hh = W // 2, H // 2
    return {
        "left": {"x": x0, "y": y0, "w": hw, "h": H},
        "right": {"x": x0 + hw, "y": y0, "w": W - hw, "h": H},
        "top-left": {"x": x0, "y": y0, "w": hw, "h": hh},
        "top-right": {"x": x0 + hw, "y": y0, "w": W - hw, "h": hh},
        "bottom-left": {"x": x0, "y": y0 + hh, "w": hw, "h": H - hh},
        "bottom-right": {"x": x0 + hw, "y": y0 + hh, "w": W - hw, "h": H - hh},
    }.get(edge)


SNAP_EDGES = ("left", "right", "top-left", "top-right", "bottom-left", "bottom-right", "maximize", "restore",
              "up", "down")


# ---------------------------------------------------------------------------
# Service
# ---------------------------------------------------------------------------

class WindowsService:
    def __init__(self, api, ctx, hypr: Hypr | None = None):
        self.api = api
        self.ctx = ctx
        self.hypr = hypr or Hypr()
        self.catalog = get_catalog(ctx)
        self.lock = threading.RLock()
        self.windows: list[dict] = []
        self.last_json = ""
        self.focused: str | None = None
        self.urgent: set[str] = set()
        self.order: dict[str, int] = {}
        self.seq = 0
        self.snapped: dict[str, dict] = {}   # address → {edge, x, y, w, h, saved:{x,y,w,h}}
        self.desktop: dict | None = None     # show-desktop record {ws, addrs, focused}
        self.refresh_soon = Debouncer(0.06, self.refresh, name="windows-refresh")
        self._stop = threading.Event()
        self._evsock: socket.socket | None = None
        self._thread: threading.Thread | None = None
        self._match_cache: dict[tuple, tuple] = {}
        self._pid_cache: dict[tuple, tuple | None] = {}
        self.repin: set[str] = set()          # pinned windows we unpinned to minimize them
        self.event_listeners: list = []      # cb(name, data) for every raw Hyprland event (display.py)
        # window animations (plugin win11-csd ≥ 1.6, SPEC §6.4.1)
        self.min_state: dict[str, str] = {}  # address → "max" | "fs": its state when we minimized it
        self.icon_rects: dict[str, dict] = {}   # address → taskbar button {x,y,w,h}, shell-layer-local px
        self.icon_hidden = False             # the taskbar is auto-hidden right now
        self._layer: tuple | None = None     # ((x, y, w, h) | None, monotonic time) of the shell layer
        self._anim_state: bool | None = None  # plugin probe: None = ask, True / False = cached answer
        self._anim_checked = 0.0
        self._anim_gen = 0                   # bumped on every reset / "Invalid dispatcher": older probe answers lose
        self._fs_cap: bool | None = None   # FS_PROBE (asked only while the anim probe is ok): None = ask
        self._anim_emitted: bool | None = None   # last windows.anim {on} sent to the UI
        self._pushed: tuple | None = None    # (reduce_motion, safe_mode) last pushed to the plugin
        self._push_lock = threading.Lock()   # one push at a time, from reading the settings to Hyprland's answer
        # P-1 re-maximize waiting for its window's focus: address → ("fullscreen 0|1", monotonic deadline)
        self.pending_reapply: dict[str, tuple[str, float]] = {}
        # maximized windows that Hyprland un-maximized because another window of their workspace was maximized:
        # address → {"ws": workspace id, "by": address of that window, "t": monotonic time it was seen}
        self.displaced: dict[str, dict] = {}
        self._fs_prev: dict[str, tuple] = {}     # address → (workspace, maximized, fullscreen, minimized) at the last fetch
        self._explicit: dict[str, float] = {}    # address → monotonic time of the shell's own state change of it
        self._fs_instant: bool | None = None     # FS_INSTANT_PROBE (asked only while FS_PROBE is ok): None = ask
        self._unsub_settings = None
        self.hypr.on_optional_missing = self._anim_missing

    # -- state ------------------------------------------------------------------
    def supported(self) -> bool:
        return self.hypr.available()

    def _window(self, c: dict, active: str | None) -> dict | None:
        addr = _addr(c.get("address"))
        if not addr or c.get("mapped") is False:
            return None
        cls = c.get("class") or ""
        icls = c.get("initialClass") or ""
        title = c.get("title") or c.get("initialTitle") or ""
        if not cls and not icls and not title:
            return None
        ws = c.get("workspace") or {}
        ws_name = str(ws.get("name") or "")
        key = (cls, icls)
        hit = self._match_cache.get(key)
        if hit is None:
            hit = self.catalog.match_window(cls, icls)
            self._match_cache[key] = hit
        desktop_id, icon = hit
        pid = c.get("pid") if isinstance(c.get("pid"), int) and c.get("pid") > 0 else None
        if desktop_id is None and pid:
            phit = self._match_pid(pid, cls or icls)
            if phit is not None:
                desktop_id, icon = phit
        full, maxi = _fs_state(c)
        at = c.get("at") or [0, 0]
        size = c.get("size") or [0, 0]
        if addr not in self.order:
            self.seq += 1
            self.order[addr] = self.seq
        try:
            geo = {"x": int(at[0]), "y": int(at[1]), "w": int(size[0]), "h": int(size[1])}
        except (TypeError, ValueError, IndexError):
            geo = {"x": 0, "y": 0, "w": 0, "h": 0}
        snap = self.snapped.get(addr)
        if snap and any(abs(geo[k] - snap[k]) > 3 for k in ("x", "y", "w", "h")) and not maxi:
            self.snapped.pop(addr, None)   # moved/resized by the user: no longer snapped
            snap = None
        return {
            "id": addr,
            "appId": cls or icls,
            "title": title,
            "workspace": ws.get("id"),
            "workspaceName": ws_name,
            "minimized": ws_name == MIN_WS,
            "focused": addr == active,
            "desktopId": desktop_id,
            "icon": icon,
            "pid": pid,
            "fullscreen": full,
            "maximized": maxi,
            "floating": bool(c.get("floating")),
            "urgent": addr in self.urgent and addr != active,
            "snapped": snap["edge"] if snap else None,
            "monitor": c.get("monitor"),
            "xwayland": bool(c.get("xwayland")),
            "pinned": bool(c.get("pinned")),
            "initialClass": icls,
            "displaced": False,
            **geo,
        }

    def _match_pid(self, pid: int, cls: str) -> tuple | None:
        """Desktop entry of a window whose class matched nothing, from its process (cached per pid)."""
        key = (pid, cls)
        if key in self._pid_cache:
            return self._pid_cache[key]
        hit = None
        for hint in _pid_hints(pid):
            did, icon = self.catalog.match_window(hint)
            if did:
                hit = (did, icon)
                break
        if len(self._pid_cache) > 500:
            self._pid_cache.clear()
        self._pid_cache[key] = hit
        return hit

    def fetch(self) -> list[dict]:
        clients = self.hypr.json("clients")
        if not isinstance(clients, list):
            return []
        active = self.hypr.json("activewindow")
        active_addr = _addr(active.get("address")) if isinstance(active, dict) else None
        with self.lock:
            if len(self._match_cache) > 500:
                self._match_cache.clear()
            out = [w for w in (self._window(c, active_addr) for c in clients if isinstance(c, dict)) if w]
            live = {w["id"] for w in out}
            for d in (self.order, self.snapped, self.min_state, self.pending_reapply, self.displaced, self._explicit):
                for k in [k for k in d if k not in live]:
                    d.pop(k, None)
            self._track_displaced(out)
            self.urgent &= live
            self.repin &= live
            out.sort(key=lambda w: self.order.get(w["id"], 0))
            self.focused = active_addr
        return out

    def _track_displaced(self, out: list[dict]) -> None:
        """(lock held, every fetch) Windows that lost maximized because another window of the same workspace became
        maximized since the previous fetch (Hyprland: one fullscreen window per workspace, Compositor.cpp
        setWindowFullscreenState) are remembered; an entry lives while that window floats on that workspace and some
        other window there is maximized (mode 1). The list then reports it as maximized (Windows keeps both)."""
        now = time.monotonic()
        prev = self._fs_prev
        cur = {w["id"]: (w["workspace"], w["maximized"], w["fullscreen"], w["minimized"]) for w in out}
        newly: dict = {}   # workspace → a window that became maximized there since the previous fetch
        for a, (ws, mx, fs, mn) in cur.items():
            p = prev.get(a)
            if mx and not fs and not mn and (p is None or not p[1] or p[2] or p[3] or p[0] != ws):
                newly[ws] = a
        for a, (ws, mx, fs, mn) in cur.items():
            p = prev.get(a)
            if (p is None or mn or mx or fs or ws not in newly or newly[ws] == a or p[0] != ws or not p[1] or p[2] or p[3]
                    or now - self._explicit.get(a, -1e9) < EXPLICIT_HOLD):
                continue
            self.displaced[a] = {"ws": ws, "by": newly[ws], "t": now}
        self._fs_prev = cur
        for a in list(self.displaced):
            d, c = self.displaced[a], cur.get(a)
            other = any(b != a and ws == d["ws"] and mx and not fs and not mn for b, (ws, mx, fs, mn) in cur.items())
            if c is None or c[0] != d["ws"] or c[1] or c[2] or c[3] or not other:
                del self.displaced[a]
        for w in out:
            if w["id"] in self.displaced:
                w["maximized"] = w["displaced"] = True

    def _explicit_change(self, a: str) -> None:
        """The shell itself changes this window's maximized / fullscreen / floating state: never 'displaced' by it."""
        with self.lock:
            self._explicit[a] = time.monotonic()
            self.displaced.pop(a, None)

    def _instant_on(self) -> bool:
        """Does the loaded plugin take 'win11csd:fullscreen 0x… N instant' (≥ 1.6.2)? Asked only while FS_PROBE is ok,
        cached until the same resets."""
        if not self._fs_on():
            return False
        with self.lock:
            state, gen = self._fs_instant, self._anim_gen
        if state is not None:
            return state
        reply = self.hypr.raw(FS_INSTANT_PROBE)
        if reply is None:
            return False
        state = reply.strip() == "ok"
        with self.lock:
            if self._anim_gen != gen:
                return bool(self._fs_instant)
            self._fs_instant = state
        return state

    def _instant_max(self, a: str) -> str | None:
        """The command that maximizes window a at once (no zoom, no morph), or None (plugin < 1.6.2, safe mode)."""
        if not _HINT_ADDR_RE.match(a) or self._motion()[1] or not self._instant_on():
            return None
        return f"win11csd:fullscreen {a} 1 instant"

    def _displaced_by(self, b: str, ws: Any) -> str | None:
        """The window most recently displaced on workspace ws (by b or by any window; b is the one leaving)."""
        with self.lock:
            cands = [(d["t"], a) for a, d in self.displaced.items() if d["ws"] == ws and a != b]
        return max(cands)[1] if cands else None

    def _redisplace_after(self, b: str, ws: Any) -> None:
        """The maximized window b left workspace ws (closed, moved away): the window it displaced there comes back
        maximized at once, as in Windows it was maximized behind it all along (only with the plugin's 'instant')."""
        a = self._displaced_by(b, ws)
        if a is None:
            return
        try:
            w = self.find({"id": a})
        except ApiError:
            return
        if w["minimized"] or w["fullscreen"] or w["workspace"] != ws:
            return
        wins = self.fetch()
        if any(x["workspace"] == ws and (x["fullscreen"] or (x["maximized"] and not x["displaced"])) for x in wins):
            return   # another window is the maximized one there now
        cmd = self._instant_max(a)
        if cmd:
            self.hypr.dispatch(cmd)
            self._after()

    def refresh(self) -> list[dict]:
        if not self.supported():
            return []
        wins = self.fetch()
        payload = json_dumps(wins)
        with self.lock:
            changed = payload != self.last_json
            prev_focus = next((w["id"] for w in self.windows if w["focused"]), None)
            self.windows = wins
            self.last_json = payload
        if changed:
            self.api.emit("windows.changed", {"windows": wins})
            focus = next((w["id"] for w in wins if w["focused"]), None)
            if focus != prev_focus:
                self.api.emit("windows.focused", {"id": focus})
        return wins

    def list(self) -> list[dict]:
        if not self.supported():
            return []
        wins = self.fetch()
        with self.lock:
            self.windows = wins
            self.last_json = json_dumps(wins)
        return wins

    def find(self, args: dict, allow_active: bool = False) -> dict:
        wid = args.get("id")
        pid = args.get("pid")
        wins = self.fetch()
        if wid is not None:
            a = _addr(wid)
            for w in wins:
                if w["id"] == a:
                    return w
        elif pid is not None:
            try:
                pid = int(pid)
            except (TypeError, ValueError):
                raise ApiError("bad_args", "Ongeldig proces.") from None
            matches = [w for w in wins if w["pid"] == pid]
            if matches:
                return next((w for w in matches if w["focused"]), matches[0])
        elif allow_active:
            for w in wins:
                if w["focused"]:
                    return w
            raise ApiError("not_found", "Er is geen actief venster.")
        raise ApiError("not_found", "Dit venster bestaat niet meer.")

    def active_workspace(self) -> int | None:
        ws = self.hypr.json("activeworkspace")
        if isinstance(ws, dict) and isinstance(ws.get("id"), int):
            return ws["id"]
        return None

    def monitor_for(self, w: dict) -> dict | None:
        mons = self.hypr.json("monitors")
        if not isinstance(mons, list) or not mons:
            return None
        for m in mons:
            if isinstance(m, dict) and m.get("id") == w.get("monitor"):
                return m
        return next((m for m in mons if isinstance(m, dict) and m.get("focused")), mons[0])

    # -- window animations (plugin win11-csd ≥ 1.6) ---------------------------------
    def _anim_missing(self) -> None:
        """A batch answered "Invalid dispatcher" for a hint: the plugin is gone (until the next probe)."""
        with self.lock:
            self._anim_state = False
            self._anim_checked = time.monotonic()
            self._anim_gen += 1
            self._fs_cap = None
            self._fs_instant = None
        self._emit_anim()

    def _anim_on(self) -> bool:
        """Does the plugin answer the probe? Cached; False is asked again after 30 s, everything again
        after configreloaded / win11csd>>loaded (plugin loads and unloads both reload the config).
        A probe answer is only stored when nothing reset the state while it was under way (generation):
        a probe sent just before the plugin finished loading must not overwrite the newer answer."""
        now = time.monotonic()
        with self.lock:
            state, checked, gen = self._anim_state, self._anim_checked, self._anim_gen
        if state is True or (state is False and now - checked < 30.0):
            return state
        reply = self.hypr.raw(ANIM_PROBE)
        state = None if reply is None else reply.strip() == "ok"
        with self.lock:
            if self._anim_gen != gen:
                return bool(self._anim_state)
            self._anim_state, self._anim_checked = state, now
        self._emit_anim()
        return bool(state)

    def anim_active(self) -> bool:
        """Will a hint animate right now (as far as the host knows; no I/O): the plugin answered the
        probe and neither reduce motion nor safe mode is on. The UI hides a Task View card's thumbnail
        only then (windows.anim, windows.setIconRects)."""
        with self.lock:
            state = self._anim_state
        if state is not True:
            return False
        rm, safe = self._motion()
        return not rm and not safe

    def _emit_anim(self) -> None:
        on = self.anim_active()
        with self.lock:
            if on == self._anim_emitted:
                return
            self._anim_emitted = on
        self.api.emit("windows.anim", {"on": on})

    def _motion(self) -> tuple[bool, bool]:
        """(reduce motion, safe mode): Settings › Toegankelijkheid and the crash guard's file."""
        try:
            rm = bool(self.ctx.settings.get("accessibility.reduceMotion", False))
        except Exception:  # noqa: BLE001
            rm = False
        try:
            safe = (Path(self.ctx.config_dir) / SAFE_MODE_FILE).exists()
        except (OSError, TypeError, AttributeError):
            safe = False
        return rm, safe

    def _push_motion(self, force: bool = False) -> tuple[bool, bool]:
        """Tell the plugin about reduce motion and safe mode (only when it answers the probe). One push at
        a time, the settings read inside the lock: pushes from several threads (settings change, hints,
        events) can never reach Hyprland in the wrong order and leave an older value in the plugin."""
        if not self.supported() or not self._anim_on():
            return self._motion()
        with self._push_lock:
            state = self._motion()
            with self.lock:
                fresh = force or self._pushed != state
                self._pushed = state
            if fresh:
                rm, safe = state
                # both are optional commands: their answers are ignored (False only when Hyprland did not answer)
                if not self.hypr.batch([f"keyword plugin:win11-csd:reduce_motion {int(rm)}",
                                        f"keyword plugin:win11-csd:safe_mode {int(safe)}"]):
                    with self.lock:
                        self._pushed = None
        self._emit_anim()
        return state

    def _push_motion_async(self) -> None:
        threading.Thread(target=self._push_motion, name="win11csd-motion", daemon=True).start()

    def _plugin_changed(self) -> None:
        """configreloaded / win11csd>>loaded: probe again, and push again (a reload resets keywords)."""
        with self.lock:
            self._anim_state = None
            self._anim_gen += 1
            self._fs_cap = None
            self._fs_instant = None
            self._pushed = None
            self._layer = None
        self._push_motion(force=True)
        self._emit_anim()   # also when the plugin is gone (no push then)

    def _hints_ok(self) -> bool:
        if not self._anim_on():
            return False
        rm, safe = self._push_motion()     # also notices a safe-mode file that came or went
        return not rm and not safe

    def _layer_origin(self) -> tuple | None:
        """(x, y, w, h) of OUR shell layer surface in global logical px (``j/layers``; cached 2 s).
        Only a layer of this process counts: in preview mode (no layer of ours) another shell's layer
        would give wrong coordinates, so then there is no rect and the plugin uses its fallback."""
        now = time.monotonic()
        with self.lock:
            if self._layer is not None and now - self._layer[1] < 2.0:
                return self._layer[0]
        data = self.hypr.json("layers")
        geo = None
        if isinstance(data, dict):
            for mon in data.values():
                levels = mon.get("levels") if isinstance(mon, dict) else None
                for items in (levels.values() if isinstance(levels, dict) else ()):
                    for item in (items if isinstance(items, list) else ()):
                        if not isinstance(item, dict) or item.get("namespace") != SHELL_LAYER:
                            continue
                        if item.get("pid") is not None and item.get("pid") != os.getpid():
                            continue
                        try:
                            g = tuple(int(item[k]) for k in ("x", "y", "w", "h"))
                        except (KeyError, TypeError, ValueError):
                            continue
                        if g[2] > 0 and g[3] > 0 and geo is None:
                            geo = g
        with self.lock:
            self._layer = (geo, now)
        return geo

    def _to_global(self, r: dict | None, hidden: bool = False) -> dict | None:
        """Shell-layer-local px → global logical px. With an auto-hidden taskbar the rect is centred on the
        layer's bottom edge (the window shrinks into the edge the taskbar slides out of)."""
        if not r:
            return None
        lay = self._layer_origin()
        if lay is None:
            return None
        lx, ly, _lw, lh = lay
        y = ly + lh - r["h"] / 2 if hidden else ly + r["y"]
        return {"x": lx + r["x"], "y": y, "w": r["w"], "h": r["h"]}

    def _icon_rect(self, w: dict) -> dict | None:
        with self.lock:
            r = self.icon_rects.get(w["id"])
            hidden = self.icon_hidden
        return self._to_global(r, hidden)

    def _hint(self, kind: str, w: dict, from_rect: dict | None = None) -> list[str]:
        """[] or the plugin hint for this window ('win11csd:anim <kind> 0x… [x y w h] [thumb]'), to go right
        before its movetoworkspacesilent in the same batch. from_rect: a Task View / Alt+Tab card
        (shell-layer-local px); otherwise the window's taskbar button."""
        a = w["id"]
        if not _HINT_ADDR_RE.match(a) or not self._hints_ok():
            return []
        box = _hint_box(self._to_global(from_rect) if from_rect else self._icon_rect(w))
        parts = [kind, a] + (box or [])
        if box and from_rect:
            parts.append("thumb")
        return ["win11csd:anim " + " ".join(parts)]

    def set_icon_rects(self, args: dict) -> dict:
        """windows.setIconRects {hidden, layer?, rects:[{ids:[id…] | id, x, y, w, h}]}: where the taskbar
        buttons settle (px of the layer surface ``layer``, only 'shell' today). Replaces the whole map."""
        rects = args.get("rects", [])
        if args.get("layer", "shell") != "shell" or not isinstance(rects, list) or len(rects) > 64:
            raise ApiError("bad_args", "Ongeldige rechthoeken.")
        out: dict[str, dict] = {}
        total = 0
        for r in rects:
            box = _rect_arg(r, 512)
            ids = r.get("ids") if "ids" in r else ([r["id"]] if "id" in r else None)
            if not isinstance(ids, list) or len(ids) > 64:
                raise ApiError("bad_args", "Ongeldige rechthoeken.")
            total += len(ids)
            if total > 256:
                raise ApiError("bad_args", "Ongeldige rechthoeken.")
            for i in ids:
                a = _addr(i) if isinstance(i, str) else None
                if a is None:
                    raise ApiError("bad_args", "Ongeldige rechthoeken.")
                out[a] = box
        with self.lock:
            self.icon_rects = out
            self.icon_hidden = bool(args.get("hidden"))
        return {"ok": True, "count": len(out), "anim": self.anim_active()}

    def _remember_state(self, w: dict) -> None:
        """P-1: remember maximized / fullscreen at minimize (Hyprland may drop it on special:minimized)."""
        with self.lock:
            self.pending_reapply.pop(w["id"], None)
            if w["fullscreen"]:
                self.min_state[w["id"]] = "fs"
            elif w["maximized"]:
                self.min_state[w["id"]] = "max"
            else:
                self.min_state.pop(w["id"], None)

    def _reapply(self, w: dict) -> tuple[list[str], bool, bool]:
        """P-1 on restore of a minimized window: (commands after its focuswindow, maximized after,
        fullscreen after). Forgets the remembered state."""
        with self.lock:
            st = self.min_state.pop(w["id"], None)
        maxi, full = w["maximized"], w["fullscreen"]
        if not w["minimized"]:
            return [], maxi, full
        if st == "max" and not maxi and not full:
            return ["fullscreen 1"], True, False
        if st == "fs" and not full:
            return ["fullscreen 0"], False, True
        return [], maxi, full

    def _active_addr(self) -> str | None:
        j = self.hypr.json("activewindow")
        return _addr(j.get("address")) if isinstance(j, dict) else None

    def _reapply_when_active(self, a: str, cmd: str) -> None:
        """P-1: 'fullscreen 0|1' acts on the ACTIVE window, and Hyprland may have refused the focuswindow just
        sent (an exclusive layer such as a closing flyout, a modal dialog, the lock screen). So it goes only
        when `a` really is the active window: now, or when it becomes active within REAPPLY_WAIT seconds
        (activewindowv2); never on whichever other window happens to be active."""
        if self._active_addr() == a:
            self.hypr.dispatch(cmd)
            return
        with self.lock:
            self.pending_reapply[a] = (cmd, time.monotonic() + REAPPLY_WAIT)

    def _pending_active(self, a: str) -> None:
        """activewindowv2 of a window with a waiting re-maximize (event thread)."""
        with self.lock:
            p = self.pending_reapply.pop(a, None)
        if p is None or time.monotonic() > p[1]:
            return
        cmd = p[0]
        try:
            w = self.find({"id": a})
        except ApiError:
            return
        if w["minimized"] or w["fullscreen"] or (cmd == "fullscreen 1" and _real_max(w)):
            return
        if self._active_addr() != a:      # active again only briefly: wait for the next time
            with self.lock:
                self.pending_reapply.setdefault(a, p)
            return
        self.hypr.dispatch(cmd)
        self._after()

    def _fs_on(self) -> bool:
        """Does the loaded plugin have 'win11csd:fullscreen' (≥ 1.6.1; 1.6 only has the hint)? Asked only while
        the anim probe says the plugin is there (so never without it), cached until the same resets."""
        if not self._anim_on():
            return False
        with self.lock:
            state, gen = self._fs_cap, self._anim_gen
        if state is not None:
            return state
        reply = self.hypr.raw(FS_PROBE)
        if reply is None:
            return False
        state = reply.strip() == "ok"
        with self.lock:
            if self._anim_gen != gen:     # reset meanwhile: this answer may be older than the reset
                return bool(self._fs_cap)
            self._fs_cap = state
        return state

    def _fs_targeted(self, a: str, cmd: str) -> str | None:
        """P-1 with plugin ≥ 1.6.1: 'win11csd:fullscreen 0x… 1|2' re-applies maximized (1) / fullscreen (2) on
        exactly this window, so it goes INTO the restore batch right after the window's move: Hyprland's own
        'fullscreen 0|1' acts on the active window and had to wait for the focus. The restore animation the
        hint armed then starts (preRender, after the batch) with the maximized box as its goal instead of the
        floating one. None without that dispatcher, and in safe mode (the crash guard: exactly the behaviour without
        the plugin, SPEC §6.4.1; reduce motion keeps it, it is no animation): then 'fullscreen …' after the batch."""
        if not _HINT_ADDR_RE.match(a) or self._motion()[1] or not self._fs_on():
            return None
        return f"win11csd:fullscreen {a} {1 if cmd == 'fullscreen 1' else 2}"

    def _needs_reapply(self, a: str, cmd: str) -> bool:
        """Is the state that 'fullscreen 0|1' (P-1) would give still missing on window a?"""
        try:
            w = self.find({"id": a})
        except ApiError:
            return False
        return not (w["minimized"] or w["fullscreen"] or (cmd == "fullscreen 1" and _real_max(w)))

    # -- actions ----------------------------------------------------------------
    def _after(self) -> None:
        self.refresh_soon.trigger()

    def focus(self, w: dict, from_rect: dict | None = None) -> dict:
        a = w["id"]
        if w["minimized"]:
            return self.restore(w, from_rect=from_rect)
        cmds = [f"focuswindow address:{a}", f"alterzorder top,address:{a}"]
        redo = self._instant_max(a) if w.get("displaced") else None
        if redo:   # a maximized window Hyprland had un-maximized behind another one: in front, maximized, at once
            cmds.append(redo)
        replies: list = []
        ok = self.hypr.dispatch(*cmds, replies=replies)
        if w.get("displaced") and (not redo or not _answered_ok(replies, redo)) and self._needs_reapply(a, "fullscreen 1"):
            self._reapply_when_active(a, "fullscreen 1")   # without the plugin's 'instant': Hyprland's own, once active
        with self.lock:
            self.urgent.discard(a)
            self.displaced.pop(a, None)
        self._after()
        return {"ok": ok}

    def _displaced_active(self, a: str) -> None:
        """activewindowv2 of a displaced window that the shell did not activate itself (an app's activation, a
        keybind): maximized again (event thread)."""
        try:
            w = self.find({"id": a})
        except ApiError:
            return
        if not w.get("displaced") or self._active_addr() != a:
            return
        redo = self._instant_max(a)
        replies: list = []
        if not (redo and self.hypr.dispatch(redo, replies=replies) and _answered_ok(replies, redo)):
            self.hypr.dispatch("fullscreen 1")   # it is the active window: Hyprland's own maximize
        with self.lock:
            self.displaced.pop(a, None)
        self._after()

    def restore(self, w: dict, from_rect: dict | None = None) -> dict:
        a = w["id"]
        ws = self.active_workspace()
        reapply, maxi, full = self._reapply(w)
        cmds = []
        targeted = None
        if ws is not None:
            if w["minimized"]:
                cmds += self._hint("restore", w, from_rect)
            cmds.append(f"movetoworkspacesilent {ws},address:{a}")
            if reapply:   # plugin ≥ 1.6.1: re-maximized on exactly this window, before the restore animation starts
                targeted = self._fs_targeted(a, reapply[0])
                if targeted:
                    cmds.append(targeted)
        cmds += [f"focuswindow address:{a}", f"alterzorder top,address:{a}"]
        cmds += self._repin_cmds(dict(w, fullscreen=full), maximized=maxi)   # (none when re-maximized)
        replies: list = []
        ok = self.hypr.dispatch(*cmds, replies=replies)
        if reapply and not targeted:
            # 'fullscreen 0|1' acts on the active window: not in the batch, only once the focuswindow above
            # really made it the active window
            self._reapply_when_active(a, reapply[0])
        elif targeted and not _answered_ok(replies, targeted) and self._needs_reapply(a, reapply[0]):
            self._reapply_when_active(a, reapply[0])   # refused (plugin unloaded since the probe …): as without it
        self._after()
        return {"ok": ok}

    def _unpin_cmds(self, w: dict) -> list[str]:
        """Pinned windows cannot be moved to special:minimized: unpin first, pin again on restore."""
        if not w.get("pinned"):
            return []
        with self.lock:
            self.repin.add(w["id"])
        return [f"pin address:{w['id']}"]

    def _repin_cmds(self, w: dict, maximized: bool | None = None) -> list[str]:
        """Pin again what we unpinned (only possible for floating, non-fullscreen windows)."""
        a = w["id"]
        maxi = w["maximized"] if maximized is None else maximized
        with self.lock:
            if a not in self.repin or maxi or w["fullscreen"]:
                return []
            self.repin.discard(a)
        return [f"pin address:{a}"]

    def minimize(self, w: dict) -> dict:
        if w["minimized"]:
            return {"ok": True}
        self._remember_state(w)   # (a displaced window is reported maximized: its restore re-maximizes it, P-1)
        # a maximized window leaves: the one it displaced on this workspace is maximized behind it (Windows)
        behind = None
        if (w["maximized"] or w["fullscreen"]) and not w.get("displaced"):
            b = self._displaced_by(w["id"], w["workspace"])
            behind = self._instant_max(b) if b else None
        # the hint goes right before the move (after a possible unpin): the plugin snapshots the window there
        ok = self.hypr.dispatch(*self._unpin_cmds(w), *self._hint("minimize", w),
                                f"movetoworkspacesilent {MIN_WS},address:{w['id']}", *([behind] if behind else []))
        self._after()
        return {"ok": ok}

    def set_pinned(self, w: dict, pinned: bool) -> dict:
        """Always on top (Hyprland ``pin`` toggles, so only dispatch when the state differs)."""
        a = w["id"]
        if not pinned:
            with self.lock:   # also forget a pin we would restore after minimize/maximize
                self.repin.discard(a)
        if w["minimized"]:
            if pinned:
                with self.lock:   # applied by restore()
                    self.repin.add(a)
            return {"ok": True, "pinned": pinned, "deferred": True}
        if bool(w.get("pinned")) == pinned:
            return {"ok": True, "pinned": pinned}
        if pinned and (w["maximized"] or w["fullscreen"]):
            # Hyprland only pins floating, non-fullscreen windows: pinned when it is restored
            with self.lock:
                self.repin.add(a)
            return {"ok": False, "pinned": False, "deferred": True}
        cmds = []
        if pinned and not w["floating"]:
            cmds.append(f"setfloating address:{a}")
        cmds.append(f"pin address:{a}")
        ok = self.hypr.dispatch(*cmds)
        self._after()
        return {"ok": ok, "pinned": pinned if ok else not pinned}

    def close(self, w: dict) -> dict:
        with self.lock:
            self.min_state.pop(w["id"], None)
            self.pending_reapply.pop(w["id"], None)
            self.displaced.pop(w["id"], None)
        ok = self.hypr.dispatch(f"closewindow address:{w['id']}")
        self._after()
        return {"ok": ok}

    def toggle_maximize(self, w: dict, want: bool | None = None) -> dict:
        a = w["id"]
        if w.get("displaced"):
            # reported maximized, but Hyprland has it floating behind the workspace's maximized window
            if want is None or want is False:   # restore: it stays floating, now in front (Windows: restore + activate)
                self._explicit_change(a)
                ok = self.hypr.dispatch(f"focuswindow address:{a}", f"alterzorder top,address:{a}")
                self._after()
                return {"ok": ok, "maximized": False}
            return dict(self.focus(w), maximized=True)   # maximize: in front, maximized again
        # 'fullscreen 1' acts on the active window; focus it only when it is not (focuswindow can warp the cursor)
        cmds = [] if (w.get("focused") and not w["minimized"]) else [f"focuswindow address:{a}"]
        with self.lock:   # an explicit maximize / restore wins over a waiting P-1 re-maximize
            self.pending_reapply.pop(a, None)
        if w["minimized"]:
            with self.lock:   # its own target wins over the state remembered at minimize
                self.min_state.pop(a, None)
            ws = self.active_workspace()
            if ws is not None:
                cmds[0:0] = [*self._hint("restore", w), f"movetoworkspacesilent {ws},address:{a}"]
        if w["fullscreen"]:
            cmds.append("fullscreen 0")          # leave real fullscreen first
            if want is None:
                want = False
        currently = w["maximized"]
        if want is None:
            want = not currently
        if not want:
            self._explicit_change(a)   # the shell un-maximizes it: not 'displaced' by a window maximized right after
        else:
            with self.lock:            # maximized again on purpose: a window maximized after it displaces it as usual
                self._explicit.pop(a, None)
        if want != currently:
            if want:
                # Hyprland refuses to fullscreen pinned windows (binds:allow_pin_fullscreen = false):
                # unpin while maximized, pin again when restored
                cmds += self._unpin_cmds(w)
            cmds.append("fullscreen 1")
            if not want:
                cmds += self._repin_cmds(dict(w, fullscreen=False), maximized=False)
        ok = self.hypr.dispatch(*cmds)
        self._after()
        return {"ok": ok, "maximized": want}

    def _move_resize(self, a: str, rect: dict, floating: bool, focus: bool = True) -> list[str]:
        cmds = [f"focuswindow address:{a}"] if focus else []
        if not floating:
            cmds.append(f"setfloating address:{a}")
        cmds += [f"resizewindowpixel exact {rect['w']} {rect['h']},address:{a}",
                 f"movewindowpixel exact {rect['x']} {rect['y']},address:{a}",
                 f"alterzorder top,address:{a}"]
        return cmds

    def snap(self, w: dict, edge: str, mon: dict | None = None) -> dict:
        a = w["id"]
        if w.get("displaced"):   # Hyprland has it floating: snap that (Super+Up / Down semantics of a floating window)
            w = dict(w, maximized=False, displaced=False)
            self._explicit_change(a)
        with self.lock:   # an explicit snap wins over a waiting P-1 re-maximize
            self.pending_reapply.pop(a, None)
        snap = self.snapped.get(a)
        cur = snap["edge"] if snap else None
        # Win11 keyboard semantics for Super+Up / Super+Down
        if edge == "up":
            edge = {"left": "top-left", "right": "top-right", "bottom-left": "left",
                    "bottom-right": "right"}.get(cur or "", "maximize")
            if w["maximized"]:
                return {"ok": True, "edge": "maximize"}
        elif edge == "down":
            if w["maximized"]:
                edge = "restore"
            elif cur in ("left", "right"):
                edge = "bottom-" + cur
            elif cur in ("top-left", "top-right"):
                edge = cur.replace("top-", "")
            else:  # restored window or bottom quadrant → minimize (like Win11)
                return dict(self.minimize(w), edge="minimize")
        if edge == "maximize":
            return dict(self.toggle_maximize(w, want=True), edge="maximize")
        if edge == "restore":
            if w["maximized"] or w["fullscreen"]:
                return dict(self.toggle_maximize(w, want=False), edge="restore")
            if snap and snap.get("saved"):
                cmds = self._move_resize(a, snap["saved"], w["floating"], focus=not w.get("focused"))
                ok = self.hypr.dispatch(*cmds)
                with self.lock:
                    self.snapped.pop(a, None)
                self._after()
                return {"ok": ok, "edge": "restore"}
            return {"ok": True, "edge": "restore"}
        mon = mon or self.monitor_for(w)
        if mon is None:
            raise ApiError("unsupported", "Geen beeldscherm gevonden.")
        rect = snap_rect(logical_monitor(mon)["work"], edge)
        if rect is None:
            raise ApiError("bad_args", "Onbekende positie.")
        cmds = []
        if w["maximized"] or w["fullscreen"]:
            self._explicit_change(a)   # un-maximized for a snap by the shell itself
            if not w.get("focused"):
                cmds.append(f"focuswindow address:{a}")
            cmds.append("fullscreen 1" if w["maximized"] else "fullscreen 0")
        if w["minimized"]:
            with self.lock:   # snapped wins over the state remembered at minimize
                self.min_state.pop(a, None)
            ws = self.active_workspace()
            if ws is not None:
                cmds += self._hint("restore", w)
                cmds.append(f"movetoworkspacesilent {ws},address:{a}")
        cmds += self._move_resize(a, rect, w["floating"], focus=not w.get("focused"))
        cmds += self._repin_cmds(dict(w, fullscreen=False), maximized=False)
        saved = snap.get("saved") if snap else {k: w[k] for k in ("x", "y", "w", "h")}
        ok = self.hypr.dispatch(*cmds)
        if ok:
            with self.lock:
                self.snapped[a] = dict(rect, edge=edge, saved=saved)
        self._after()
        return {"ok": ok, "edge": edge}

    def _snap_after_drag(self, a: str, x: int, y: int) -> None:
        """Let go at the top edge = maximize, at a side = half, in a corner = quarter (Win11)."""
        if self._stop.is_set():
            return
        try:
            w = self.find({"id": a})
            if w["fullscreen"] or w["minimized"] or not w["floating"]:
                return
            mons = [m for m in (self.hypr.json("monitors") or []) if isinstance(m, dict)]
            geos = [(m, logical_monitor(m)) for m in mons]
            # the pointer is clamped to the last pixel, which may round to x+w: accept one pixel of slack
            inside = lambda g, s: g["x"] <= x < g["x"] + g["w"] + s and g["y"] <= y < g["y"] + g["h"] + s  # noqa: E731
            hit = next((mg for mg in geos if inside(mg[1], 0)), None) or next((mg for mg in geos if inside(mg[1], 1)), None)
            if hit is None:
                return
            mon, g = hit
            left, right = x <= g["x"] + 1, x >= g["x"] + g["w"] - 2
            top, bottom = y <= g["y"] + 1, y >= g["y"] + g["h"] - 2
            edge = ("top-left" if left and top else "top-right" if right and top else
                    "bottom-left" if left and bottom else "bottom-right" if right and bottom else
                    "left" if left else "right" if right else "maximize" if top else None)
            if not edge:
                return
            if edge == "maximize" and mon.get("id") is not None and mon.get("id") != w.get("monitor"):
                # dropped on another screen than the window's centre: move it there first, then maximize
                work = g["work"]
                self.hypr.dispatch(f"movewindowpixel exact {work['x']} {work['y']},address:{a}")
                w = self.find({"id": a})
            self.snap(w, edge, mon=mon)
        except ApiError as e:
            log.info("Vastklikken na slepen (snap after drag): %s", e.message)
        except Exception:  # noqa: BLE001
            log.exception("Vastklikken na slepen mislukt (snap after drag failed)")

    def show_desktop(self) -> dict:
        wins = self.fetch()
        ws = self.active_workspace()
        visible = [w for w in wins if w["workspace"] == ws and not w["minimized"]]
        with self.lock:
            rec = self.desktop
        if visible:
            prev = [a for a in (rec or {}).get("addrs", []) if any(w["id"] == a and w["minimized"] for w in wins)]
            focused = next((w["id"] for w in visible if w["focused"]), (rec or {}).get("focused"))
            cmds = []
            for w in visible:   # no animation hints (phases 1-3): Hyprland's own fade, like Win11's Win+D
                self._remember_state(w)
                cmds += self._unpin_cmds(w) + [f"movetoworkspacesilent {MIN_WS},address:{w['id']}"]
            ok = self.hypr.dispatch(*cmds)
            with self.lock:
                self.desktop = {"ws": ws, "addrs": prev + [w["id"] for w in visible], "focused": focused}
            self._after()
            return {"ok": ok, "desktop": True}
        if rec:
            still = [a for a in rec["addrs"] if any(w["id"] == a and w["minimized"] for w in wins)]
            cmds = []
            later = []    # P-1 (address, 'fullscreen …'): it acts on the active window, so one by one below
            targeted = []  # P-1 with plugin ≥ 1.6.1: (address, 'fullscreen …', 'win11csd:fullscreen …') in the batch
            if ws is not None:
                by_id = {w["id"]: w for w in wins}
                for a in still:
                    reapply, maxi, full = self._reapply(by_id[a])
                    if reapply:
                        t = self._fs_targeted(a, reapply[0])
                        if t:
                            targeted.append((a, reapply[0], t))
                        else:
                            later.append((a, reapply[0]))
                    cmds += ([f"movetoworkspacesilent {ws},address:{a}"]
                             + self._repin_cmds(dict(by_id[a], fullscreen=full), maximized=maxi))
                # after every move, like the re-maximize after the batch without the plugin (a window that comes
                # back maximized un-maximizes the workspace's other one: Hyprland has one per workspace)
                cmds += [t for _a, _c, t in targeted]
            refocus = []
            if rec.get("focused") in still:
                refocus = [f"focuswindow address:{rec['focused']}", f"alterzorder top,address:{rec['focused']}"]
            if not later:
                cmds += refocus
            replies: list = []
            ok = self.hypr.dispatch(*cmds, replies=replies) if cmds else True
            for a, cmd, t in targeted:   # refused (plugin unloaded since the probe …): as without the plugin
                if not _answered_ok(replies, t) and self._needs_reapply(a, cmd):
                    later.append((a, cmd))
            if later:
                for a, cmd in later:   # focus it, and re-maximize only once it really is the active window
                    self.hypr.dispatch(f"focuswindow address:{a}")
                    self._reapply_when_active(a, cmd)
                if refocus:
                    ok = self.hypr.dispatch(*refocus) and ok
            with self.lock:
                self.desktop = None
            self._after()
            return {"ok": ok, "desktop": False}
        return {"ok": True, "desktop": True}

    # -- events -------------------------------------------------------------------
    def handle_event(self, name: str, data: str) -> None:
        for cb in list(self.event_listeners):
            try:
                cb(name, data)
            except Exception:  # noqa: BLE001
                log.exception("Luisteraar voor %s faalde (listener failed)", name)
        if name == "activewindowv2":
            a = _addr(data)
            with self.lock:
                changed = a != self.focused
                self.focused = a
                if a:
                    self.urgent.discard(a)
                waiting = a in self.pending_reapply
                displaced = a in self.displaced
            if changed:
                self.api.emit("windows.focused", {"id": a})
            if waiting:
                self._pending_active(a)
            elif displaced:
                self._displaced_active(a)
        elif name == "urgent":
            a = _addr(data)
            if a:
                with self.lock:
                    self.urgent.add(a)
        elif name == "minimized":
            addr, _, flag = data.partition(",")
            a = _addr(addr)
            if a:
                try:
                    w = self.find({"id": a})
                except ApiError:
                    w = None
                if w is not None:
                    if flag.strip() == "1":
                        self.minimize(w)
                    elif w["minimized"]:
                        self.restore(w)
        elif name == "win11dragend":
            # win11-csd plugin: a title-bar drag ended at x,y → Windows snap at the screen edges
            parts = data.split(",")
            if len(parts) == 3:
                try:
                    a, x, y = _addr(parts[0]), int(parts[1]), int(parts[2])
                except ValueError:
                    a = None
                if a:
                    t = threading.Timer(0.06, self._snap_after_drag, args=(a, x, y))
                    t.daemon = True
                    t.start()
        elif name == "closewindow":
            a = _addr(data)
            with self.lock:
                self.snapped.pop(a, None)
                self.urgent.discard(a)
                self.min_state.pop(a, None)
                self.pending_reapply.pop(a, None)
                self.displaced.pop(a, None)
                prev = self._fs_prev.get(a)
            if a and prev and prev[1] and not prev[3]:   # a maximized window closed: the one it displaced comes back
                self._redisplace_after(a, prev[0])
        elif name == "movewindowv2":
            parts = data.split(",")
            a = _addr(parts[0]) if parts else None
            with self.lock:
                prev = self._fs_prev.get(a) if a else None
            if a and prev and prev[1] and not prev[3] and len(parts) >= 2 and parts[1].strip() != str(prev[0]) \
                    and not parts[-1].startswith("special:"):   # moved to another desktop (minimize has its own path)
                self._redisplace_after(a, prev[0])
        elif name == "configreloaded" or (name == "win11csd" and data.startswith("loaded")):
            # plugin loaded or unloaded (both reload the config, which also resets runtime keywords)
            self._plugin_changed()
        elif name.startswith(("monitoradded", "monitorremoved")):
            with self.lock:
                self._layer = None
        if name in _REFRESH_EVENTS:
            self.refresh_soon.trigger()

    def _event_loop(self) -> None:
        backoff = 0.5
        while not self._stop.is_set():
            path = self.hypr.socket_path(".socket2.sock")
            if not path:
                if not os.environ.get("HYPRLAND_INSTANCE_SIGNATURE"):
                    return  # not running under Hyprland: nothing to watch
                self._stop.wait(5)
                continue
            try:
                s = unix_connect(path, 5)
                s.settimeout(None)
            except OSError as e:
                log.debug("Gebeurtenissocket niet bereikbaar (event socket unreachable): %s", e)
                self._stop.wait(backoff)
                backoff = min(10.0, backoff * 2)
                continue
            backoff = 0.5
            self._evsock = s
            self.refresh_soon.trigger()
            try:   # (re)connected, e.g. at start or after a Hyprland restart: probe the plugin, push motion
                self._plugin_changed()
            except Exception:  # noqa: BLE001
                log.exception("Animatie-instellingen doorgeven mislukt (motion push failed)")
            buf = b""
            try:
                while not self._stop.is_set():
                    chunk = s.recv(65536)
                    if not chunk:
                        break
                    buf += chunk
                    while b"\n" in buf:
                        line, buf = buf.split(b"\n", 1)
                        text = line.decode("utf-8", "replace")
                        name, sep, data = text.partition(">>")
                        if sep:
                            try:
                                self.handle_event(name.strip(), data)
                            except Exception:  # noqa: BLE001
                                log.exception("Hyprland-gebeurtenis %s faalde (event failed)", name)
            except OSError:
                pass
            finally:
                self._evsock = None
                try:
                    s.close()
                except OSError:
                    pass
            if not self._stop.is_set():
                self._stop.wait(backoff)

    def start(self) -> None:
        if self._thread is None and (os.environ.get("HYPRLAND_INSTANCE_SIGNATURE") or self.supported()):
            if self._unsub_settings is None:
                try:
                    self._unsub_settings = self.ctx.settings.on_change(self._on_settings)
                except Exception:  # noqa: BLE001
                    log.debug("Geen instellingen (no settings) voor reduce motion", exc_info=True)
            self._thread = threading.Thread(target=self._event_loop, name="hypr-events", daemon=True)
            self._thread.start()

    def _on_settings(self, _settings=None, path=None, changed=None) -> None:
        """accessibility.reduceMotion changed (or everything: reset / import) → tell the plugin."""
        paths = [str(p) for p in (changed or [path]) if p]
        if not paths or any(p.startswith("accessibility") for p in paths):
            self._push_motion_async()

    def stop(self) -> None:
        self._stop.set()
        unsub, self._unsub_settings = self._unsub_settings, None
        if callable(unsub):
            try:
                unsub()
            except Exception:  # noqa: BLE001
                pass
        self.refresh_soon.close()
        s = self._evsock
        if s is not None:
            try:
                s.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass


def _read_proc(pid: int, name: str, limit: int = 65536) -> bytes:
    try:
        with open(f"/proc/{pid}/{name}", "rb") as fh:
            return fh.read(limit)
    except OSError:
        return b""


# executables that say nothing about the app they run (also versioned: python3.14, perl5.40, wine64 …)
_INTERPRETER_RE = re.compile(r"^(python[0-9.]*|perl[0-9.]*|ruby[0-9.]*|node|nodejs|electron[0-9]*|java|bash|sh|dash|"
                             r"env|bwrap|gjs|mono|wine[0-9]*)$")


def _pid_hints(pid: int) -> list[str]:
    """Names to look up in the app catalog for a process: the desktop file GLib launched it from
    (only when it is this very pid, children inherit the variable), its systemd app scope, its executable."""
    hints: list[str] = []
    env = {}
    for item in _read_proc(pid, "environ").split(b"\0"):
        k, sep, v = item.partition(b"=")
        if sep and k in (b"GIO_LAUNCHED_DESKTOP_FILE", b"GIO_LAUNCHED_DESKTOP_FILE_PID", b"BAMF_DESKTOP_FILE_HINT"):
            env[k.decode()] = v.decode("utf-8", "replace")
    if env.get("GIO_LAUNCHED_DESKTOP_FILE") and env.get("GIO_LAUNCHED_DESKTOP_FILE_PID") == str(pid):
        hints.append(os.path.basename(env["GIO_LAUNCHED_DESKTOP_FILE"]).removesuffix(".desktop"))
    if env.get("BAMF_DESKTOP_FILE_HINT"):   # snaps
        hints.append(os.path.basename(env["BAMF_DESKTOP_FILE_HINT"]).removesuffix(".desktop"))
    try:
        from .sysmon import parse_cgroup_unit
        for line in _read_proc(pid, "cgroup", 8192).decode("utf-8", "replace").splitlines():
            unit = parse_cgroup_unit(line.split(":", 2)[-1])
            if unit and unit[0] in ("scope", "service"):
                hints.append(unit[1])
    except Exception:  # noqa: BLE001
        pass
    try:
        exe = os.path.basename(os.readlink(f"/proc/{pid}/exe")).removesuffix(" (deleted)")
        if exe and not _INTERPRETER_RE.match(exe):     # python3.14 would match the system's "Python (v3.14)"
            hints.append(exe)
    except OSError:
        pass
    return [h for h in dict.fromkeys(hints) if h]


def register(api, ctx) -> None:
    svc = WindowsService(api, ctx)
    ctx._windows_service = svc

    def guard() -> bool:
        return svc.supported()

    @api.method("windows.list")
    def _list(args):
        """{supported, windows:[...]} (Hyprland only)."""
        if not guard():
            return {"supported": False, "windows": []}
        return {"supported": True, "windows": svc.list()}

    def action(name: str, fn, allow_active: bool = False):
        @api.method(name)
        def _m(args):
            if not guard():
                return {"supported": False}
            w = svc.find(args, allow_active=allow_active)
            return fn(w, args)
        return _m

    def from_rect(args: dict) -> dict | None:
        """Optional fromRect {x,y,w,h}: the Task View / Alt+Tab card a minimized window grows out of."""
        r = args.get("fromRect")
        return None if r is None else _rect_arg(r, 4096)

    def action_rect(name: str, fn):
        @api.method(name)
        def _m(args):
            rect = from_rect(args)      # bad_args before anything happens
            if not guard():
                return {"supported": False}
            return fn(svc.find(args), rect)
        return _m

    action_rect("windows.focus", lambda w, r: svc.focus(w, from_rect=r))
    action("windows.close", lambda w, a: svc.close(w), allow_active=True)
    action("windows.minimize", lambda w, a: svc.minimize(w), allow_active=True)
    action_rect("windows.restore", lambda w, r: svc.restore(w, from_rect=r))

    @api.method("windows.setIconRects")
    def _set_icon_rects(args):
        """{hidden, rects:[{ids:[id…], x, y, w, h}]} → {ok, count}: where each window's taskbar button settles
        (shell-layer-local px, transform-free; only used to aim the minimize / restore animations).
        Also without Hyprland (then nothing uses them)."""
        return svc.set_icon_rects(args if isinstance(args, dict) else {})
    action("windows.toggleMaximize",
           lambda w, a: svc.toggle_maximize(w, a.get("want") if isinstance(a.get("want"), bool) else None),
           allow_active=True)
    action("windows.setPinned", lambda w, a: svc.set_pinned(w, bool(a.get("pinned", True))), allow_active=True)

    @api.method("windows.snap")
    def _snap(args):
        """Snap a window (default: the active one) to an edge/quadrant, maximize or restore."""
        edge = args.get("edge")
        if edge not in SNAP_EDGES:
            raise ApiError("bad_args", "Onbekende positie.")
        if not guard():
            return {"supported": False}
        w = svc.find(args, allow_active=True)
        return svc.snap(w, edge)

    @api.method("windows.showDesktop")
    def _show_desktop(args):
        """Toggle: minimize every window on the current workspace / bring them back."""
        if not guard():
            return {"supported": False}
        return svc.show_desktop()


def start(api, ctx) -> None:
    svc = getattr(ctx, "_windows_service", None)
    if svc is not None:
        svc.start()


def stop(api, ctx) -> None:
    svc = getattr(ctx, "_windows_service", None)
    if svc is not None:
        svc.stop()
