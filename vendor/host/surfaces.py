"""GTK3 + WebKit2 4.1 surfaces.

* layer mode (Hyprland, GtkLayerShell 0.1, primary monitor only):
    - ``win11shell-desktop``  BACKGROUND layer, all edges, exclusive zone -1, keyboard none; a click on
                              the desktop takes the keyboard (exclusive → on-demand), losing it → none
    - ``win11shell-shell``    TOP layer, all edges, exclusive zone -1, transparent, input region from JS,
                              keyboard none; exclusive while a flyout/menu is open
  (Hyprland gives an ON_DEMAND layer the keyboard as soon as the pointer merely hovers it, even with
  input:follow_mouse = 2, so on-demand would steal typing — e.g. Enter — from the app in use.)
    - ``win11shell-spacer``   BOTTOM layer, bottom+left+right, 48 px high, exclusive zone 48
                              (0 when taskbar.autoHide), empty input region, keyboard none
* preview mode: one normal window "Win11 Shell (voorbeeld)" (1440×900, clamped to the work area).

IMPORTANT: Gtk is imported lazily (PyGObject initialises GTK on import, and app windows must set
their prgname first). Nothing in this module touches Gtk at import time.

``create_webview()`` is shared with host/appwindow.py.
"""

from __future__ import annotations

import contextlib
import json
import logging
import os
import sys
import threading
import time
from typing import Any, Callable
from urllib.parse import urlsplit

from . import VERSION

log = logging.getLogger("win11shell.surfaces")

__all__ = ["SurfaceManager", "init_gtk", "create_webview", "gi_modules", "gpu_compositing", "gpu_info", "gpu_mode",
           "kill_webkit_leftovers", "layer_shell_installed", "reap_inherited_children", "start_gpu_leak_guard",
           "webkit_snapshot", "TASKBAR_HEIGHT"]

TASKBAR_HEIGHT = 48

_MODS: dict[str, Any] = {}

# GPU rendering for our WebViews (SPEC §8.1). Ubuntu/Debian build WebKitGTK with "disable-nvidia-dmabuf.patch":
# on the NVIDIA driver WebKit then renders without the GPU (webkit://gpu: policy "never"), and without GPU
# compositing `backdrop-filter` paints nothing. The pages are told (gpu_compositing() → html[data-backdrop]) and
# their in-app Acrylic overlays turn opaque instead of see-through (tokens.css).
# The default is OFF. On 2026-10-26 the earlier default (WEBKIT_FORCE_DMABUF_RENDERER=1 +
# __NV_DISABLE_EXPLICIT_SYNC=1) froze the taskbar after 15–20 min: with Skia painting on the GPU every compositing
# layer that comes and goes leaves ~1.25 sync_file fds open in WebKitWebProcess; at the fd limit (1024) the web
# process hangs without exiting (nvidia-drm "invalid sync FD: -24"), and SkiaGPUWorker crashed in
# libnvidia-eglcore (tests/nested/gpu-research/). WIN11SHELL_GPU:
#   unset | 0 | off  no GPU for our WebViews; an inherited WEBKIT_FORCE_DMABUF_RENDERER /
#                    WEBKIT_DMABUF_RENDERER_FORCE_SHM is dropped for our process (it leaked from GPU-forcing hosts
#                    into terminals, and a shell started there would have forced the GPU again)
#   safe             opt-in, exactly the variant measured stable (tests/nested/gpu-research: 0 fd growth in
#                    12.7-min runs, 60 fps, blur works): WEBKIT_FORCE_DMABUF_RENDERER=1
#                    WEBKIT_DMABUF_RENDERER_FORCE_SHM=1 WEBKIT_SKIA_ENABLE_CPU_RENDERING=1 — Skia paints on the CPU,
#                    WebKit composites on the GPU and hands frames over in shared memory (no dmabufs, no NVIDIA EGL
#                    in the UI process: no explicit-sync clash) — and none of SAFE_DROP (__NV_DISABLE_EXPLICIT_SYNC,
#                    WEBKIT_DISABLE_DMABUF_RENDERER, WEBKIT_DISABLE_COMPOSITING_MODE) in our process, whoever set
#                    them. Not the default yet: one live app-window probe is still open (notes/gpu-safe-probe.md).
#   env              leave WebKit's own switches as inherited (developers)
# <config>/no-gpu always wins. Fallbacks while we force (safe) — each writes no-gpu: 3 web-process crashes within
# 2 min, 2 starts in a row that did not survive 30 s (<config>/gpu-trial), one of our processes holding more
# than SYNC_FD_LIMIT sync_file fds (that leak), or 2 page-hang verdicts within GPU_HANG_WINDOW_S (note_page_hang: a
# watchdog stops the hung web process through the API, which is no crash, and its restart survives the trial).
# Every process of ours first resets the switches to the user's own values (util.user_gpu_values: as they were before
# the first Win11 Shell process of the session, kept in util.USER_GPU_MARK) and then decides; programs started for
# the user get exactly those values (util.user_env,
# apps.launch_context). What we changed is also recorded in util.GPU_ENV_MARK (older code reads it).
# Taakbeheer never uses the GPU (appwindow.force_software_rendering).
NO_GPU_FILE = "no-gpu"
GPU_TRIAL_FILE = "gpu-trial"
GPU_TRIAL_SECONDS = 30
GPU_SAFE_ENV = {"WEBKIT_FORCE_DMABUF_RENDERER": "1", "WEBKIT_DMABUF_RENDERER_FORCE_SHM": "1",
                "WEBKIT_SKIA_ENABLE_CPU_RENDERING": "1"}
_GPU_OFF_DROP = ("WEBKIT_FORCE_DMABUF_RENDERER", "WEBKIT_DMABUF_RENDERER_FORCE_SHM")
# safe = exactly GPU_SAFE_ENV: these never stay in our own process then (the explicit-sync workaround belongs to the
# dmabuf path that leaked; the two "disable" switches would silently turn the GPU path off again)
SAFE_DROP = ("__NV_DISABLE_EXPLICIT_SYNC", "WEBKIT_DISABLE_DMABUF_RENDERER", "WEBKIT_DISABLE_COMPOSITING_MODE")
SYNC_FD_LIMIT = 256          # the leak reached the 1024 fd limit; a healthy process holds a handful
SYNC_FD_CHECK_S = 30
GPU_HANG_WINDOW_S = 600      # 2 page-hang verdicts this close together while we force the GPU → give it up
GPU_HANG_SAME_S = 10.0       # verdicts this close together are one event (both host pages hanging at once)
# forced: we switched WebKit to the GPU path in this process (safe). mode: gpu_mode() as decided (None before).
# compositing: does WebKit in THIS process composite with the GPU (so in-page backdrop-filter paints)?
# Fixed when WebKit is imported (WebKit reads its switches once); None before that. See gpu_compositing().
_gpu = {"forced": False, "crashes": [], "hangs": [], "fallback": None, "compositing": None, "mode": None}
# The NVIDIA proprietary driver is loaded (Ubuntu's patch then keeps WebKit off the GPU unless forced).
_NVIDIA_DRIVER_PATHS = ("/proc/driver/nvidia/version", "/sys/module/nvidia_drm")


def _config_dir() -> str:
    from .util import xdg_config_home
    return os.environ.get("WIN11SHELL_CONFIG_DIR") or str(xdg_config_home() / "win11shell")


def gpu_mode(quiet: bool = False) -> str:
    """'off' | 'safe' | 'env': what <config>/no-gpu and WIN11SHELL_GPU ask for (see above)."""
    if os.path.exists(os.path.join(_config_dir(), NO_GPU_FILE)):
        return "off"
    value = (os.environ.get("WIN11SHELL_GPU") or "").strip().lower()
    if value in ("safe", "env"):
        return value
    if value not in ("", "0", "off", "no", "false") and not quiet:
        log.warning("WIN11SHELL_GPU=%s onbekend: zonder GPU (unknown value, GPU rendering off; opt in with 'safe')",
                    value)
    return "off"


def gpu_info() -> dict:
    """``shell.info``'s ``gpu``: ``{mode, compositing, forced}`` — the mode this process chose (gpu_mode; before
    WebKit was imported, e.g. in serve mode: what it would choose), whether WebKit here composites with the GPU
    (in-page blur paints in the host's own WebViews; app windows decide for themselves) and whether we still force
    the GPU path (False again after a fallback wrote no-gpu)."""
    return {"mode": _gpu["mode"] or gpu_mode(quiet=True), "compositing": bool(gpu_compositing()),
            "forced": bool(_gpu["forced"])}


def _prefer_gpu_renderer() -> None:
    """Before WebKit initialises, once per process: apply gpu_mode() to our own environment (see above)."""
    if _gpu["mode"] is not None:
        return
    from .util import GPU_ENV_MARK, apply_user_gpu_env, remember_user_gpu_env
    # the user's own switches (before any Win11 Shell process changed them; kept for an exec and our children), and
    # nothing else: what a parent host or an older image of this process set, removed or leaked is gone
    apply_user_gpu_env(os.environ, remember_user_gpu_env())
    mode = _gpu["mode"] = gpu_mode()
    changes: dict = {"set": [], "user": {}}
    if mode == "safe":
        for k, v in GPU_SAFE_ENV.items():
            if os.environ.get(k) == v:
                continue
            if k in os.environ:
                changes["user"][k] = os.environ[k]
            else:
                changes["set"].append(k)
            os.environ[k] = v
        for k in SAFE_DROP:
            if k in os.environ:
                changes["user"][k] = os.environ.pop(k)
        _gpu["forced"] = True
        log.info("WebKit: GPU-compositing met CPU-tekenen via gedeeld geheugen (WIN11SHELL_GPU=safe: GPU "
                 "compositing, CPU painting, shared-memory frames)")
    elif mode == "off":
        for k in _GPU_OFF_DROP:
            if k in os.environ:
                changes["user"][k] = os.environ.pop(k)
        why = "no-gpu" if os.path.exists(os.path.join(_config_dir(), NO_GPU_FILE)) else "WIN11SHELL_GPU"
        if changes["user"]:
            log.warning("WebKit zonder GPU (%s); overgenomen %s genegeerd (GPU rendering off, inherited switch "
                        "ignored)", why, ", ".join(changes["user"]))
        else:
            log.info("WebKit zonder GPU (GPU rendering off: %s)", why)
    else:
        log.info("WebKit-GPU-schakelaars uit de omgeving (WIN11SHELL_GPU=env: WebKit's own switches as inherited)")
    if changes["set"] or changes["user"]:
        os.environ[GPU_ENV_MARK] = json.dumps(changes, separators=(",", ":"))


def software_only() -> None:
    """This process has made its own rendering decision (Taakbeheer: appwindow.force_software_rendering set
    software switches in our environment): _prefer_gpu_renderer must neither undo them nor decide again."""
    _gpu["mode"], _gpu["forced"] = "off", False


def _give_up_gpu(why: str) -> str:
    """Stop forcing the GPU for good: no-gpu, and WIN11SHELL_GPU=0 in our environment — an in-place restart (exec)
    and the app windows we start then render without the GPU even when no-gpu cannot be written (disk full,
    read-only home), instead of forcing it again and looping."""
    _gpu["forced"] = False
    os.environ["WIN11SHELL_GPU"] = "0"
    return _write_no_gpu(why)


def _write_no_gpu(why: str) -> str:
    path = os.path.join(_config_dir(), NO_GPU_FILE)
    try:
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "w", encoding="utf-8") as fh:
            fh.write(f"Win11 Shell rendert zonder GPU: {why}.\n"
                     "Verwijder dit bestand om het opnieuw te proberen (menu's zijn dan weer matglas).\n")
    except OSError as e:
        log.warning("Kan %s niet schrijven (cannot write): %s", path, e)
    return path


def gpu_trial_begin() -> None:
    """Desktop/preview start, before any WebView: guard the GPU renderer against crashes of this very
    process (a Wayland protocol error or a crash in the UI process never reaches the web-process crash
    counter). The trial file counts starts that did not stay up GPU_TRIAL_SECONDS; the third such
    start renders without the GPU and writes no-gpu."""
    if "WebKit2" in _MODS:
        return
    path = os.path.join(_config_dir(), GPU_TRIAL_FILE)
    try:
        with open(path, encoding="utf-8") as fh:
            failed = int(fh.read().strip() or 0)
    except (OSError, ValueError):
        failed = 0
    if failed >= 2:
        _give_up_gpu(f"de shell startte {failed}× niet goed met GPU-rendering")
        log.error("Shell overleefde GPU-rendering %d× niet; voortaan zonder GPU "
                  "(shell did not survive GPU rendering %d times; falling back)", failed, failed)
        try:
            os.remove(path)
        except OSError:
            pass
        return
    _prefer_gpu_renderer()
    if not _gpu["forced"]:
        return
    try:
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "w", encoding="utf-8") as fh:
            fh.write(str(failed + 1))
    except OSError:
        return

    def survived():
        try:
            os.remove(path)
        except OSError:
            pass
        return False
    gi_modules(webkit=False)["GLib"].timeout_add_seconds(GPU_TRIAL_SECONDS, survived)


def _env_switch(name: str) -> bool:
    """Is a WebKit environment switch on? (WebKit: set and not "0".)"""
    value = os.environ.get(name)
    return value is not None and value != "0"


def _webkit_gpu_from_env() -> bool:
    """Will WebKit composite with the GPU, given the switches it reads at start-up? In WebKitGTK 2.52 GTK3
    the GPU path is the DMA-BUF renderer: WEBKIT_DISABLE_COMPOSITING_MODE or WEBKIT_DISABLE_DMABUF_RENDERER
    turn it off (they win over forcing), WEBKIT_FORCE_DMABUF_RENDERER turns it on even on NVIDIA, and
    otherwise Ubuntu/Debian's disable-nvidia-dmabuf.patch keeps it off on the NVIDIA driver. When in doubt
    this says no: an opaque menu always looks intended, a "frosted" one that does not blur looks broken."""
    if _env_switch("WEBKIT_DISABLE_COMPOSITING_MODE") or _env_switch("WEBKIT_DISABLE_DMABUF_RENDERER"):
        return False
    if _gpu["forced"] or _env_switch("WEBKIT_FORCE_DMABUF_RENDERER"):
        return True
    return not any(os.path.exists(p) for p in _NVIDIA_DRIVER_PATHS)


def gpu_compositing() -> bool:
    """Does WebKit in this process composite with the GPU, i.e. does in-page ``backdrop-filter`` paint?
    (Without it WebKitGTK paints no backdrop-filter at all, although ``CSS.supports()`` says yes.) Fixed
    once WebKit is imported; before that the value it would get. Every WebView we create passes it to its
    pages (``window.__win11shellHost.backdrop``, ``html[data-backdrop]``; see create_webview)."""
    if _gpu["compositing"] is None:
        return _webkit_gpu_from_env()
    return bool(_gpu["compositing"])


def set_gpu_fallback(cb: Callable | None) -> None:
    """Called (once, main thread) when the GPU renderer was given up (repeated crashes, the sync_file leak) —
    e.g. restart the shell; the restart then renders without the GPU (no-gpu)."""
    _gpu["fallback"] = cb


def _proc_table() -> dict[int, tuple[int, bytes]]:
    """pid → (ppid, comm) for every process (/proc/*/stat)."""
    out: dict[int, tuple[int, bytes]] = {}
    try:
        names = os.listdir("/proc")
    except OSError:
        return out
    for d in names:
        if not d.isdigit():
            continue
        try:
            with open(f"/proc/{d}/stat", "rb") as f:
                data = f.read()
            rp = data.rfind(b")")
            out[int(d)] = (int(data[rp + 2:].split()[1]), data[data.find(b"(") + 1:rp])
        except (OSError, ValueError, IndexError):
            continue
    return out


_WEBKIT_GLUE = (b"bwrap", b"xdg-dbus-proxy")


def webkit_children(root: int, table: dict | None = None) -> list[int]:
    """Our WebKit helper processes (WebKitWebProcess, WebKitNetworkProcess …, also behind bwrap): processes whose
    comm starts with 'WebKit' and that descend from ``root`` within 4 generations through WebKit/bwrap processes only
    (an app window's WebKit processes, grandchildren of the host through its python3, are not the host's)."""
    table = _proc_table() if table is None else table
    out = []
    for pid, (ppid, comm) in table.items():
        if not comm.startswith(b"WebKit"):
            continue
        p, hops = ppid, 0
        while p > 1 and hops < 4:
            if p == root:
                out.append(pid)
                break
            pp, pcomm = table.get(p, (0, b""))
            if not (pcomm.startswith(b"WebKit") or pcomm in _WEBKIT_GLUE):
                break
            p, hops = pp, hops + 1
    return out


def _start_time(pid: int) -> bytes | None:
    try:
        with open(f"/proc/{pid}/stat", "rb") as f:
            data = f.read()
        return data[data.rfind(b")") + 2:].split()[19]
    except (OSError, IndexError):
        return None


def webkit_snapshot(root: int) -> dict[int, bytes]:
    """{pid: start time} of ``root``'s WebKit processes (webkit_children), for kill_webkit_leftovers."""
    return {p: st for p in webkit_children(root) if (st := _start_time(p)) is not None}


def kill_webkit_leftovers(snap: dict[int, bytes], grace: float = 1.5, kill=os.kill) -> list[int]:
    """After their UI process is gone, WebKit processes exit by themselves (their sockets closed) — except one whose
    main thread hangs (the GPU fd leak of 2026-10-26: 99.9 % CPU, never exits) or one that is stopped. The ones of
    ``snap`` (webkit_snapshot, taken while their UI process lived) still there after ``grace`` s, and still the same
    process (start time), get SIGKILL. → the pids killed."""
    end = time.monotonic() + grace
    left = dict(snap)
    while left:
        left = {p: st for p, st in left.items() if _start_time(p) == st and _alive_not_zombie(p)}
        if not left or time.monotonic() >= end:
            break
        time.sleep(0.1)
    killed = []
    for p, st in left.items():
        if _start_time(p) != st:
            continue
        try:
            kill(p, 9)
            killed.append(p)
        except OSError:
            pass
    if killed:
        log.warning("Achtergebleven WebKit- of hulpprocessen gestopt (leftover WebKit/helper processes killed): %s", killed)
    return killed


def _alive_not_zombie(pid: int) -> bool:
    try:
        with open(f"/proc/{pid}/stat", "rb") as f:
            data = f.read()
        return data[data.rfind(b")") + 2:data.rfind(b")") + 3] not in (b"Z", b"X")
    except OSError:
        return False


def reap_inherited_children(grace: float = 2.0, kill=os.kill) -> list[int]:
    """At start-up of a host or app window: children we already have come from the image before our in-place
    restart (os.execve keeps the pid, and with it the children). They are never reaped by anyone else (the live
    shell had ~30 WebKit/bwrap zombies after a night of restarts), and a hung WebKit process among them would run
    for good. In a thread: wait for each (reaps it); WebKit/bwrap ones still running after ``grace`` s get SIGKILL
    (our own unreaped child: its pid cannot have been reused). Other leftovers are reaped whenever they end.
    → the inherited pids (empty for a fresh process)."""
    me = os.getpid()
    kids = {pid: comm for pid, (ppid, comm) in _proc_table().items() if ppid == me}
    if not kids:
        return []

    def wait_one(pid: int, comm: bytes) -> None:
        try:
            if comm.startswith(b"WebKit") or comm in _WEBKIT_GLUE:
                end = time.monotonic() + grace
                while time.monotonic() < end:
                    if os.waitpid(pid, os.WNOHANG)[0]:
                        return
                    time.sleep(0.1)
                log.warning("Proces %d (%s) van vóór de herstart hangt; gestopt (leftover from before the restart "
                            "killed)", pid, comm.decode("utf-8", "replace"))
                with contextlib.suppress(OSError):
                    kill(pid, 9)
            os.waitpid(pid, 0)
        except ChildProcessError:
            pass

    for pid, comm in kids.items():
        threading.Thread(target=wait_one, args=(pid, comm), name=f"reap-{pid}", daemon=True).start()
    log.info("Processen van vóór de herstart worden opgeruimd (reaping children from before the restart): %d",
             len(kids))
    return sorted(kids)


def sync_file_fds(pid: int) -> int:
    """How many sync_file fds (GPU fences, ``anon_inode:sync_file``) ``pid`` holds; 0 when unreadable."""
    n = 0
    try:
        with os.scandir(f"/proc/{pid}/fd") as it:
            for e in it:
                try:
                    if os.readlink(e.path).endswith("sync_file"):
                        n += 1
                except OSError:
                    pass
    except OSError:
        return 0
    return n


def start_gpu_leak_guard(interval: float = SYNC_FD_CHECK_S, limit: int = SYNC_FD_LIMIT) -> bool:
    """While we force the GPU (WIN11SHELL_GPU=safe): every ``interval`` s, does this process or one of its WebKit
    processes hold more than ``limit`` sync_file fds? Then the leak of 2026-10-26 is back (the page would hang at
    the fd limit without its web process exiting): write no-gpu and call the GPU fallback (restart). Runs in a
    thread (a /proc scan), the fallback on the main loop. → True when the guard runs."""
    if not _gpu["forced"]:
        return False
    me = os.getpid()

    def loop():
        while _gpu["forced"]:
            time.sleep(interval)
            if not _gpu["forced"]:
                return
            counts = {p: sync_file_fds(p) for p in [me] + webkit_children(me)}
            pid, n = max(counts.items(), key=lambda kv: kv[1], default=(me, 0))
            if n <= limit:
                continue
            path = _give_up_gpu(f"proces {pid} hield {n} GPU-synchronisatiebestanden open (sync_file-lek)")
            log.error("GPU-lek: proces %d houdt %d sync_file-fd's open; voortaan zonder GPU (%s) "
                      "(sync_file fd leak with GPU rendering; falling back)", pid, n, path)
            cb = _gpu["fallback"]
            if cb:
                gi_modules(webkit=False)["GLib"].idle_add(lambda: (cb(), False)[1])
            return

    threading.Thread(target=loop, name="gpu-leak-guard", daemon=True).start()
    return True


def _note_crash(reason) -> None:
    """3 web-process crashes within 2 minutes while we forced the GPU renderer → stop forcing it."""
    if not _gpu["forced"]:
        return
    now = time.monotonic()
    _gpu["crashes"] = [t for t in _gpu["crashes"] if now - t < 120] + [now]
    if len(_gpu["crashes"]) < 3:
        return
    # WebKit's switches stay as they are: this process keeps its renderer (web processes it starts later must
    # match it), and a restart decides afresh — no-gpu / WIN11SHELL_GPU=0 win (util.GPU_ENV_MARK undoes our switches).
    path = _give_up_gpu("de webweergave liep herhaaldelijk vast met GPU-rendering")
    log.error("Webproces crasht herhaaldelijk met GPU-rendering (%s); voortaan zonder GPU (%s) "
              "(web process keeps crashing with GPU rendering; falling back)", reason, path)
    cb = _gpu["fallback"]
    if cb:
        try:
            cb()
        except Exception:  # noqa: BLE001
            log.exception("GPU-terugval mislukt (GPU fallback failed)")


def note_page_hang(clock: Callable[[], float] = time.monotonic) -> bool:
    """A page watchdog verdict (host/appwindow.HangWatchdog: a page that stopped answering) while we force the GPU.
    Its web process is stopped through the API (TERMINATED_BY_API, never counted by _note_crash), and the in-place
    restart that follows survives the 30 s gpu-trial — so without this a GPU-related hang without the sync_file
    signature (a deadlock in the NVIDIA EGL path) would reload and restart for ever with the GPU still forced.
    The second verdict within GPU_HANG_WINDOW_S gives the GPU up (no-gpu, WIN11SHELL_GPU=0) → True: the caller
    restarts its process instead of reloading (WebKit reads its switches once per process). Verdicts within
    GPU_HANG_SAME_S of the last counted one are the same event."""
    if not _gpu["forced"]:
        return False
    now = clock()
    hangs = [t for t in _gpu.get("hangs") or [] if now - t < GPU_HANG_WINDOW_S]
    if hangs and now - hangs[-1] < GPU_HANG_SAME_S:
        _gpu["hangs"] = hangs
        return False
    hangs.append(now)
    _gpu["hangs"] = hangs
    if len(hangs) < 2:
        return False
    path = _give_up_gpu("de pagina liep herhaaldelijk vast met GPU-rendering")
    log.error("Pagina liep %d× vast binnen %d min met GPU-rendering; voortaan zonder GPU (%s) "
              "(page keeps hanging with GPU rendering; falling back)", len(hangs), GPU_HANG_WINDOW_S // 60, path)
    return True


def gi_modules(webkit: bool = True) -> dict[str, Any]:
    """Import (once) and return {'GLib','Gio','Gdk','Gtk','WebKit2'?}. Initialises GTK."""
    if "Gtk" not in _MODS:
        import gi
        gi.require_version("Gdk", "3.0")
        gi.require_version("Gtk", "3.0")
        from gi.repository import Gdk, Gio, GLib, Gtk
        _MODS.update(GLib=GLib, Gio=Gio, Gdk=Gdk, Gtk=Gtk)
    if webkit and "WebKit2" not in _MODS:
        import gi
        _prefer_gpu_renderer()
        _gpu["compositing"] = _webkit_gpu_from_env()
        if _gpu["compositing"]:
            log.info("WebKit-weergave met GPU: matglas in de pagina (GPU compositing: in-page blur on)")
        else:
            log.info("WebKit-weergave zonder GPU: menu's in apps ondoorzichtig "
                     "(no GPU compositing: in-page blur off, in-app Acrylic overlays opaque)")
        gi.require_version("WebKit2", "4.1")
        from gi.repository import WebKit2
        _MODS["WebKit2"] = WebKit2
    return _MODS


def init_gtk(prgname: str = "win11shell", app_name: str | None = None) -> bool:
    """Set prgname/WM class BEFORE GTK initialises, then init. False when no display."""
    try:
        import gi
        from gi.repository import GLib
        GLib.set_prgname(prgname)
        if app_name:
            GLib.set_application_name(app_name)
        gi.require_version("Gdk", "3.0")
        from gi.repository import Gdk
        Gdk.set_program_class(prgname)
        mods = gi_modules(webkit=False)
        Gtk = mods["Gtk"]
        ok = Gtk.init_check(sys.argv[:1])
        ok = ok[0] if isinstance(ok, tuple) else bool(ok)
        return bool(ok) and Gdk.Display.get_default() is not None
    except Exception as e:  # noqa: BLE001
        # an ImportError / ValueError ('Namespace Gtk not available') here is a missing package, not "no display":
        # host.__main__ asks host/preflight.py which one (EXIT_MISSING)
        log.error("GTK kon niet starten (GTK init failed): %s: %s", type(e).__name__, e)
        return False


_LS_INSTALLED: bool | None = None


def layer_shell_installed() -> bool:
    """Is the GtkLayerShell 0.1 typelib (gir1.2-gtklayershell-0.1) + its library installed?

    Checked without importing Gtk (safe from any thread and in serve mode, never touches the display)."""
    global _LS_INSTALLED
    if _LS_INSTALLED is None:
        ok = False
        try:
            import gi
            gi.require_version("GtkLayerShell", "0.1")   # only looks the typelib up, loads nothing
            import ctypes.util
            ok = ctypes.util.find_library("gtk-layer-shell") is not None or any(
                os.path.exists(os.path.join(d, "libgtk-layer-shell.so.0"))
                for d in ("/usr/lib/x86_64-linux-gnu", "/usr/lib/aarch64-linux-gnu", "/usr/lib64", "/usr/lib"))
        except (ImportError, ValueError, AttributeError):
            ok = False
        _LS_INSTALLED = ok
    return _LS_INSTALLED


def _layer_shell():
    """GtkLayerShell module or None (import guarded; not installed everywhere)."""
    try:
        import gi
        gi.require_version("GtkLayerShell", "0.1")
        from gi.repository import GtkLayerShell
        return GtkLayerShell
    except (ImportError, ValueError, AttributeError) as e:
        log.debug("GtkLayerShell niet beschikbaar (not available): %s", e)
        return None


def _primary_monitor(Gdk):
    display = Gdk.Display.get_default()
    if display is None:
        return None
    mon = display.get_primary_monitor()
    if mon is None and display.get_n_monitors() > 0:
        mon = display.get_monitor(0)
    return mon


def _parse_rgba(Gdk, color: str | None, fallback=(0.125, 0.125, 0.125, 1.0)):
    rgba = Gdk.RGBA()
    if not color or not rgba.parse(color):
        rgba.red, rgba.green, rgba.blue, rgba.alpha = fallback
    return rgba


def _make_rgba_window(Gtk, win) -> bool:
    """Give a window an RGBA visual and a transparent background. False if no compositing."""
    screen = win.get_screen()
    visual = screen.get_rgba_visual() if screen is not None else None
    win.set_app_paintable(True)
    if visual is not None:
        win.set_visual(visual)

    def on_draw(widget, cr):
        import cairo
        cr.save()
        cr.set_source_rgba(0, 0, 0, 0)
        cr.set_operator(cairo.OPERATOR_SOURCE)
        cr.paint()
        cr.restore()
        return False

    win.connect("draw", on_draw)
    return visual is not None


# ---------------------------------------------------------------------------
# WebView factory
# ---------------------------------------------------------------------------

_context_ready = False
_crash_times: dict[int, list[float]] = {}
_dead_views: set[int] = set()     # id(view): its web process ended and no load committed since


def view_dead(view) -> bool:
    """The view has no web process right now (terminated, not reloaded yet). A JS probe on it fails at once —
    that is no answer from a page (see probe_page)."""
    return id(view) in _dead_views


def reload_delay_ms(n: int) -> int:
    """Delay before reloading a view after its ``n``-th web-process termination within a minute: quick at first,
    then backing off — never giving up for good (a crash loop would otherwise leave a blank taskbar or an
    invisible Taakbeheer). Pure (tested)."""
    return 800 if n <= 5 else 5000 if n == 6 else 15000 if n == 7 else 60000


def _setup_context(WebKit2, dry_run: bool, log_: logging.Logger) -> None:
    global _context_ready
    if _context_ready:
        return
    _context_ready = True
    wctx = WebKit2.WebContext.get_default()
    try:
        wctx.set_cache_model(WebKit2.CacheModel.DOCUMENT_VIEWER)
    except Exception:  # noqa: BLE001
        pass

    def on_download(_ctx, download):
        req = download.get_request()
        uri = req.get_uri() if req is not None else ""
        if uri.startswith(("blob:", "data:")) and not dry_run:
            # Our own UI exporting something (e.g. settings): save into Downloads, never overwrite.
            def decide(dl, suggested):
                from .util import xdg_user_dir
                folder = xdg_user_dir("DOWNLOAD")
                name = os.path.basename(suggested or "download") or "download"
                target = folder / name
                stem, ext = os.path.splitext(name)
                n = 2
                while target.exists():
                    target = folder / f"{stem} ({n}){ext}"
                    n += 1
                try:
                    folder.mkdir(parents=True, exist_ok=True)
                except OSError:
                    dl.cancel()
                    return True
                dl.set_allow_overwrite(False)
                dl.set_destination(target.as_uri() if hasattr(dl, "set_destination") else str(target))
                log_.info("Bestand opgeslagen in Downloads (saved): %s", target)
                return True

            download.connect("decide-destination", decide)
            return
        log_.info("Download geblokkeerd (blocked): %s", uri[:200])
        download.cancel()

    wctx.connect("download-started", on_download)


def host_script(backdrop: bool) -> str:
    """Script that runs in every frame of our pages before their own scripts: the facts about this WebView
    the page cannot find out itself. ``backdrop``: does in-page backdrop-filter paint (gpu_compositing())?
    ui/js/core/theme.js reads ``window.__win11shellHost`` and keeps ``html[data-backdrop]`` (none|blur)."""
    value = "blur" if backdrop else "none"
    return ("(function(){try{window.__win11shellHost=Object.freeze({backdrop:'%s'});"
            "var d=document.documentElement;if(d)d.setAttribute('data-backdrop','%s');}catch(e){}})();"
            % (value, value))


def _add_host_script(WebKit2, view) -> None:
    try:
        view.get_user_content_manager().add_script(WebKit2.UserScript.new(
            host_script(gpu_compositing()), WebKit2.UserContentInjectedFrames.ALL_FRAMES,
            WebKit2.UserScriptInjectionTime.START, None, None))
    except Exception as e:  # noqa: BLE001 - pages then assume blur works (as in a browser)
        log.warning("Paginascript niet toegevoegd (host script not added): %s", e)


def open_external(uri: str, dry_run: bool) -> None:
    if dry_run:
        log.info("[dry-run] externe link niet geopend (external link not opened): %s", uri)
        return
    try:
        from gi.repository import Gio
        try:   # without our token/test variables and with the user's own GPU switches
            from .services.apps import launch_context
            lc = launch_context()
        except Exception:  # noqa: BLE001
            lc = None
        Gio.AppInfo.launch_default_for_uri(uri, lc)
    except Exception as e:  # noqa: BLE001
        log.warning("Kan link niet openen (cannot open link) %s: %s", uri, e)


def create_webview(*, origin: str, transparent: bool, bg: str | None = None, dev: bool = False,
                   dry_run: bool = False, on_crash: Callable | None = None):
    """WebKit2.WebView configured for our surfaces.

    * no navigation away from ``origin``; http(s)/mailto links open in the default app
    * no context menu (we draw our own) unless ``dev``; developer extras with ``dev``
    * clipboard access, no back/forward gestures, downloads blocked (blob:/data: exports go to Downloads)
    """
    m = gi_modules(webkit=True)
    WebKit2, Gdk = m["WebKit2"], m["Gdk"]
    _setup_context(WebKit2, dry_run, log)
    view = WebKit2.WebView()
    s = view.get_settings()
    for prop, val in (
        ("enable-developer-extras", bool(dev)),
        ("javascript-can-access-clipboard", True),
        ("enable-back-forward-navigation-gestures", False),
        ("enable-write-console-messages-to-stdout", bool(dev)),
        ("allow-file-access-from-file-urls", False),
        ("allow-universal-access-from-file-urls", False),
        ("enable-smooth-scrolling", True),
        ("enable-page-cache", False),
        ("javascript-can-open-windows-automatically", False),
        ("enable-media-stream", False),
        ("enable-webrtc", False),
        ("enable-html5-database", False),
        ("media-playback-requires-user-gesture", True),
    ):
        try:
            s.set_property(prop, val)
        except (TypeError, ValueError, AttributeError):
            pass
    try:
        s.set_user_agent_with_application_details("Win11Shell", VERSION)
    except Exception:  # noqa: BLE001
        pass
    _add_host_script(WebKit2, view)    # in-page blur available? → html[data-backdrop] (theme.js)
    view.set_background_color(Gdk.RGBA(red=0, green=0, blue=0, alpha=0) if transparent else _parse_rgba(Gdk, bg))

    origin_parts = urlsplit(origin)

    def same_origin(uri: str) -> bool:
        u = urlsplit(uri)
        return (u.scheme, u.hostname, u.port) == (origin_parts.scheme, origin_parts.hostname, origin_parts.port)

    def on_policy(_view, decision, dtype):
        P = WebKit2.PolicyDecisionType
        if dtype == P.RESPONSE:
            try:
                if not decision.is_mime_type_supported():
                    decision.ignore()
                    return True
            except Exception:  # noqa: BLE001
                pass
            return False
        try:
            action = decision.get_navigation_action()
            uri = action.get_request().get_uri() or ""
        except Exception:  # noqa: BLE001
            return False
        if dtype == P.NAVIGATION_ACTION and (same_origin(uri) or uri in ("about:blank", "about:srcdoc")):
            decision.use()
            return True
        scheme = urlsplit(uri).scheme.lower()
        gesture = True
        try:
            gesture = bool(action.is_user_gesture())
        except Exception:  # noqa: BLE001
            pass
        if (scheme == "blob" and same_origin(uri[5:])) or (scheme == "data" and gesture):
            # Our own UI saving something (<a download href="blob:…">): becomes a download
            # (saved in Downloads by the context's download handler; cancelled in dry-run).
            try:
                decision.download()
            except Exception:  # noqa: BLE001
                decision.ignore()
            return True
        if scheme in ("http", "https", "mailto") and not same_origin(uri):
            if gesture:
                open_external(uri, dry_run)
            else:
                log.info("Navigatie zonder klik geblokkeerd (blocked non-user navigation): %s", uri[:200])
        elif not same_origin(uri):
            log.info("Navigatie geblokkeerd (navigation blocked): %s", uri[:200])
        decision.ignore()
        return True

    view.connect("decide-policy", on_policy)
    view.connect("context-menu", lambda *_a: not dev)
    view.connect("create", lambda *_a: None)

    def on_permission(_view, request):
        allow = type(request).__name__ in ("ClipboardPermissionRequest",)
        (request.allow if allow else request.deny)()
        return True

    view.connect("permission-request", on_permission)

    def on_terminated(v, reason):
        now = time.monotonic()
        times = [t for t in _crash_times.get(id(v), []) if now - t < 60] + [now]
        _crash_times[id(v)] = times
        _dead_views.add(id(v))
        delay = reload_delay_ms(len(times))
        log.error("Webproces gestopt (web process terminated): %s; opnieuw laden over %.1f s", reason, delay / 1000)
        if reason == m["WebKit2"].WebProcessTerminationReason.CRASHED:
            _note_crash(reason)
        if on_crash:
            on_crash(v, reason)
        m["GLib"].timeout_add(delay, lambda: (view_dead(v) and v.reload(), False)[1])

    def on_load_changed(v, event):
        if event == m["WebKit2"].LoadEvent.COMMITTED:
            _dead_views.discard(id(v))

    view.connect("web-process-terminated", on_terminated)
    view.connect("load-changed", on_load_changed)
    view.connect("destroy", lambda v: (_dead_views.discard(id(v)), _crash_times.pop(id(v), None)))

    retries = {"n": 0}

    def on_failed(v, _event, failing_uri, error):
        if not same_origin(failing_uri or ""):
            return False
        retries["n"] += 1
        if retries["n"] <= 20:
            log.warning("Laden mislukt, opnieuw (load failed, retrying): %s", getattr(error, "message", error))
            m["GLib"].timeout_add(1000, lambda: (v.load_uri(failing_uri), False)[1])
        return False

    view.connect("load-failed", on_failed)
    return view


PAGE_PROBE_S = 2
PAGE_HANG_S = 10.0
PAGE_FIRST_S = 30.0          # the first answer after a load commit (a cold web-process start at login may be slow)
PAGE_RESTARTS_ENV = "WIN11SHELL_PAGE_RESTARTS"


def probe_page(view, done: Callable[[], Any]) -> bool:
    """Ask the page for ``1``; ``done()`` when it answered (an error answer is an answer too — except on a view
    without a web process: WebKit fails such a call at once, and a page that is not there is not answering).
    → False when nothing was asked (no web process right now): HangWatchdog asks again on its next tick."""
    if view_dead(view):
        return False
    def finish(v, result, *_a):
        try:
            (v.evaluate_javascript_finish if hasattr(v, "evaluate_javascript_finish") else v.run_javascript_finish)(result)
        except Exception:  # noqa: BLE001
            pass
        done()
    if hasattr(view, "evaluate_javascript"):
        view.evaluate_javascript("1", -1, None, None, None, finish, None)
    else:
        view.run_javascript("1", None, finish, None)
    return True


def _is_commit(event) -> bool:
    W = _MODS.get("WebKit2")
    return W is not None and event == W.LoadEvent.COMMITTED


def run_js(view, script: str) -> None:
    """Fire-and-forget JavaScript (evaluate_javascript on new WebKit, run_javascript on old)."""
    try:
        if hasattr(view, "evaluate_javascript"):
            view.evaluate_javascript(script, -1, None, None, None, None, None)
        else:
            view.run_javascript(script, None, None, None)
    except Exception as e:  # noqa: BLE001
        log.debug("JavaScript mislukt (failed): %s", e)


# ---------------------------------------------------------------------------
# Surface manager
# ---------------------------------------------------------------------------

class SurfaceManager:
    def __init__(self, ctx):
        self.ctx = ctx
        self.gtk_ready = False
        self.layer_active = False
        self.layer_installed = False
        self.windows: dict[str, Any] = {}
        self.views: dict[str, Any] = {}
        self._keyboard: str | None = None
        self._kb_commit: threading.Event | None = None   # set once the last shell keyboard change is committed
        self._desktop_keyboard = "none"
        self._gls = None
        self._monitor_geo = None
        self._monitor_origin = (0, 0)    # logical layout position of the layer surfaces' monitor
        self._dialogs: list = []
        self._probe: dict | None = None
        # layer mode cannot start (host/problem.py): {code, missing?, packages?, command?, detail?}; show_problem is set
        # by main() inside the Win11 Shell session only, where the notice replaces the preview window
        self.problem: dict | None = None
        self.show_problem = False
        self._notice = None

    # -- init -------------------------------------------------------------------
    def init_gtk(self) -> bool:
        self.gtk_ready = init_gtk("win11shell", "Win11 Shell")
        self._gls = _layer_shell() if self.gtk_ready else None
        self.layer_installed = self._gls is not None
        return self.gtk_ready

    def layer_support(self) -> tuple[bool, str]:
        """(ok, reason). Sets self.problem ({code, …}) when layer mode cannot start. Checked BEFORE the services
        load (host.__main__), so a missing package never leaves services set up for layer mode while the host runs
        something else (notes/session-failure-ux.md §6.1)."""
        self.problem = None
        if not self.gtk_ready:
            return False, "geen grafische omgeving (no display)"
        from . import preflight
        r = preflight.check("layer", in_process=True)      # gi-cairo, the WebKit2 4.1 typelib, GtkLayerShell
        if not r["ok"]:
            self.problem = {"code": "missing", **r}
            return False, "ontbreekt: " + ", ".join(r["packages"]) + " (missing)"
        if self._gls is None:      # the typelib loads but the library does not: the same package
            pk = preflight.PARTS["layershell"][0]
            self.problem = {"code": "missing", "packages": [pk], "command": f"sudo apt install {pk}",
                            "missing": [{"key": "layershell", "package": pk, "nl": preflight.PARTS["layershell"][1],
                                         "en": preflight.PARTS["layershell"][2], "error": "GtkLayerShell"}]}
            return False, f"GtkLayerShell ontbreekt (GtkLayerShell missing: install {pk})"
        Gdk = gi_modules(webkit=False)["Gdk"]
        display = Gdk.Display.get_default()
        tname = display.__gtype__.name if display is not None else ""
        if "Wayland" not in tname:
            self.problem = {"code": "no_wayland"}
            return False, "geen Wayland-sessie (not a Wayland session)"
        is_supported = getattr(self._gls, "is_supported", None)
        if callable(is_supported):
            try:
                if not is_supported():
                    self.problem = {"code": "no_protocol"}
                    return False, "compositor ondersteunt layer-shell niet (compositor lacks layer-shell)"
            except Exception:  # noqa: BLE001
                pass
        return True, ""

    @property
    def gpu_compositing(self) -> bool:
        """Do the host's own WebViews (shell, desktop, preview) composite with the GPU, i.e. does in-page
        blur work in them? App windows decide for themselves (each its own process, same rules)."""
        return gpu_compositing()

    def start(self, mode: str) -> str:
        # restart without GPU rendering if it keeps crashing / leaks (_give_up_gpu set WIN11SHELL_GPU=0 for the exec)
        set_gpu_fallback(self.ctx.request_restart)
        gpu_trial_begin()
        if mode == "layer":
            self._start_layer()
            from .problem import RETRY_ENV, write_problem
            write_problem(None)                        # a stale problem.json from an earlier attempt
            os.environ.pop(RETRY_ENV, None)
        elif mode == "preview":
            if self.problem and self.show_problem:
                # the Win11 Shell session: never the desktop in a window by itself (Robin: "I don't want to see
                # win11shell running in a window again"); the notice says why and how to fix it, the preview
                # opens only from its button
                self._show_problem(can_preview=not self._webkit_missing())
            else:
                self._start_preview()
        start_gpu_leak_guard()
        self._start_page_watchdogs()
        return mode

    def _webkit_missing(self) -> bool:
        return any(m.get("key") == "webkit" for m in (self.problem or {}).get("missing", []) or [])

    def _show_problem(self, can_preview: bool) -> None:
        from .problem import ProblemNotice

        def on_preview():
            if "preview" in self.windows:
                self.windows["preview"].present()
                return
            self._start_preview(on_close=self._preview_closed)
            self._start_page_watchdogs()

        self._notice = ProblemNotice(self.ctx, self.problem, can_preview=can_preview, on_preview=on_preview,
                                     preview_open=lambda: "preview" in self.windows)
        self._notice.show()

    def _preview_closed(self) -> None:
        """The preview opened from the notice was closed: back to the notice (the host keeps running)."""
        win = self.windows.pop("preview", None)
        self.views.pop("preview", None)
        if win is not None:
            try:
                win.destroy()
            except Exception:  # noqa: BLE001
                pass
        notice = self._notice
        if notice is not None and notice.win is not None:
            notice.win.show_all()
            notice.win.present()

    # -- never stuck: a hung page (SPEC §2.2) -------------------------------------
    def _start_page_watchdogs(self) -> None:
        """A page can hang without its web process exiting (the GPU fd leak of 2026-10-26: no
        web-process-terminated, so the crash reload never came and the taskbar stayed frozen). Every
        PAGE_PROBE_S a JS probe per surface page; no answer for PAGE_HANG_S → that web process is stopped and the
        page reloads (create_webview's crash path; the shell gives up its keyboard first); hung again within a
        minute → the whole shell restarts in place (≤ 2× per 10 min, then only reloads) through
        ``ctx.request_restart()``, whose exit deadline is host/lifeguard.py's (a teardown that hangs → emergency
        restart, same port + token). No verdict while the host is already stopping or restarting, nor for a probe
        sent before our own main loop stood still (lifeguard.loop_stalled_since, the beat behind
        ``mainLoopLagMs``): the answer may be queued behind the stalled loop — and a loop that stays stuck is the
        lifeguard's to restart (30 s), not a hung page. A probe that could not be sent (no web process between a
        crash and the reload's commit) is no unanswered one; after each load commit the first answer may take
        PAGE_FIRST_S; memory/IO thrashing postpones a verdict up to 30 s after the probe (HangWatchdog). While we
        force the GPU the second verdict within 10 min gives it up (note_page_hang) and restarts without it.
        Ctrl+Shift+Win+B and Taakbeheer do not depend on this (they work when the host itself hangs)."""
        if not self.views:
            return
        from .appwindow import HangWatchdog
        from .lifeguard import loop_stalled_since, system_thrashing
        GLib = gi_modules(webkit=False)["GLib"]
        ctx = self.ctx
        self._watchdogs = {}
        for name, view in self.views.items():
            wd = self._watchdogs[name] = HangWatchdog(
                lambda done, v=view: probe_page(v, done),
                lambda n=name, v=view: self._reload_hung(n, v),
                lambda n=name: self._restart_hung(n),
                self._may_restart_hung,
                lambda stage, n=name, v=view: self._on_page_stage(n, v, stage),
                host_ok=lambda: not (ctx.shutting_down or ctx.restart_requested),
                loop_stalled=lambda since: loop_stalled_since(ctx, since),
                hang_s=PAGE_HANG_S, tick_s=PAGE_PROBE_S, first_s=PAGE_FIRST_S, busy=system_thrashing)
            connect = getattr(view, "connect", None)
            if connect is not None:     # a probe sent to the page before a (re)load commit can only time out
                connect("load-changed", lambda _v, event, wd=wd: (_is_commit(event) and wd.page_committed(), None)[1])

        def tick():
            if not self.windows:
                return False
            for wd in list(self._watchdogs.values()):
                wd.tick()
            return True
        GLib.timeout_add_seconds(PAGE_PROBE_S, tick)

    def _reload_hung(self, name: str, view) -> None:
        if note_page_hang():    # forced GPU, 2nd verdict within 10 min: no-gpu written; a reload keeps the GPU path
            log.error("Pagina %s reageert weer niet; Win11 Shell start opnieuw zonder GPU (page hung again with GPU "
                      "rendering: restarting without it)", name)
            self._stop_and_restart(name)
            return
        log.error("Pagina %s reageert niet; webproces gestopt en opnieuw geladen (page hung: web process stopped, "
                  "reloading)", name)
        if name == "shell":
            self._release_shell("vastgelopen")
        try:
            if view_dead(view):
                view.reload()                # no web process to stop (a crash loop's back-off): load now
            else:
                view.terminate_web_process()     # → web-process-terminated → reload (create_webview)
        except Exception:  # noqa: BLE001
            view.reload()

    def _may_restart_hung(self) -> bool:
        from .appwindow import self_restart_allowed
        ok, value = self_restart_allowed(os.environ.get(PAGE_RESTARTS_ENV), time.time(), limit=2, window=600)
        if ok:
            os.environ[PAGE_RESTARTS_ENV] = value    # survives the in-place restart (os.execve)
        return ok

    def _restart_hung(self, name: str) -> None:
        gpu_off = note_page_hang()           # forced GPU: the in-place restart then renders without it
        log.error("Pagina %s bleef vastlopen; Win11 Shell start opnieuw%s (page keeps hanging: restarting the shell)",
                  name, " zonder GPU" if gpu_off else "")
        self._stop_and_restart(name)

    def _stop_and_restart(self, name: str) -> None:
        if name == "shell":
            self._release_shell("vastgelopen")
        view = self.views.get(name)
        try:     # the hung web process must not hold up the teardown (it is SIGKILLed after the exec anyway)
            if view is not None and not view_dead(view):
                view.terminate_web_process()
        except Exception:  # noqa: BLE001
            pass
        self.ctx.request_restart()           # lifeguard: exit deadline → emergency restart if the teardown hangs

    def _on_page_stage(self, name: str, view, stage: str) -> None:
        if stage == "stuck":                  # no restart budget left: keep reloading
            self._reload_hung(name, view)
        elif stage == "ok":
            log.info("Pagina %s reageert weer (page answers again)", name)

    def _settings(self, path: str, default=None):
        try:
            return self.ctx.settings.get(path, default)
        except Exception:  # noqa: BLE001
            return default

    def _webview(self, transparent: bool, bg: str | None = None, on_crash: Callable | None = None):
        return create_webview(origin=self.ctx.base_url, transparent=transparent, bg=bg, dev=self.ctx.dev,
                              dry_run=self.ctx.dry_run, on_crash=on_crash)

    # -- layer mode -------------------------------------------------------------
    def _set_keyboard(self, win, mode: str) -> None:
        name = next((k for k, v in self.windows.items() if v is win), "?")
        log.debug("Toetsenbord %s → %s (keyboard mode)", name, mode)
        gls = self._gls
        km = getattr(gls, "KeyboardMode", None)
        if km is not None and hasattr(gls, "set_keyboard_mode"):
            value = {"none": km.NONE, "on-demand": getattr(km, "ON_DEMAND", km.NONE),
                     "exclusive": km.EXCLUSIVE}.get(mode, km.NONE)
            gls.set_keyboard_mode(win, value)
        elif hasattr(gls, "set_keyboard_interactivity"):
            # Old protocol: True == exclusive. On-demand does not exist → no keyboard.
            gls.set_keyboard_interactivity(win, mode == "exclusive")

    def _layer_window(self, name: str, layer: str, anchors: tuple[str, ...], exclusive: int,
                      keyboard: str, rgba: bool, monitor):
        m = gi_modules(webkit=False)
        Gtk = m["Gtk"]
        gls = self._gls
        win = Gtk.Window(type=Gtk.WindowType.TOPLEVEL)
        win.set_title(f"win11shell-{name}")
        win.set_decorated(False)
        if rgba:
            _make_rgba_window(Gtk, win)
        gls.init_for_window(win)  # must happen before the window is realized
        gls.set_layer(win, getattr(gls.Layer, layer))
        gls.set_namespace(win, f"win11shell-{name}")
        if monitor is not None and hasattr(gls, "set_monitor"):
            gls.set_monitor(win, monitor)
        for edge in ("TOP", "BOTTOM", "LEFT", "RIGHT"):
            gls.set_anchor(win, getattr(gls.Edge, edge), edge in anchors)
        gls.set_exclusive_zone(win, exclusive)
        self._set_keyboard(win, keyboard)
        return win

    def _start_layer(self) -> None:
        m = gi_modules(webkit=True)
        Gdk = m["Gdk"]
        import gi
        gi.require_foreign("cairo")
        import cairo

        monitor = _primary_monitor(Gdk)
        geo = monitor.get_geometry() if monitor is not None else None
        self._monitor_geo = (geo.width, geo.height) if geo is not None else (1920, 1080)
        # the pages see monitor-local coordinates; tray apps (SNI) want screen coordinates
        self._monitor_origin = (geo.x, geo.y) if geo is not None else (0, 0)
        if monitor is not None:
            def on_geometry(mon, _pspec):
                g = mon.get_geometry()
                self._monitor_origin = (g.x, g.y)

            monitor.connect("notify::geometry", on_geometry)
        width, height = self._monitor_geo
        auto_hide = bool(self._settings("taskbar.autoHide", False))

        # Spacer: reserves the taskbar strip for normal windows.
        spacer = self._layer_window("spacer", "BOTTOM", ("BOTTOM", "LEFT", "RIGHT"),
                                    0 if auto_hide else TASKBAR_HEIGHT, "none", True, monitor)
        spacer.set_size_request(-1, TASKBAR_HEIGHT)
        spacer.input_shape_combine_region(cairo.Region())
        spacer.show_all()
        self.windows["spacer"] = spacer

        # Desktop: wallpaper + icons. No keyboard until the user clicks the desktop. Then briefly EXCLUSIVE
        # (Hyprland focuses it at once, so F2 / Delete / Ctrl+C/V/Z / arrows work on the icons like on
        # Windows) and straight to ON_DEMAND (Hyprland keeps the focus but it is no longer exclusive, so a
        # click on a window takes it). When the desktop loses the keyboard it goes back to NONE, so it
        # can never steal typing by mere hovering.
        desktop = self._layer_window("desktop", "BACKGROUND", ("TOP", "BOTTOM", "LEFT", "RIGHT"), -1,
                                     "none", False, monitor)
        dview = self._webview(transparent=False, bg=self._settings("wallpaper.color", "#0b2a4a"))
        dview.load_uri(self.ctx.ui_url(surface="desktop"))
        desktop.add(dview)

        def desktop_keyboard(mode: str) -> None:
            if mode != self._desktop_keyboard:
                self._desktop_keyboard = mode
                self._set_keyboard(desktop, mode)

        def csd_plugin_loaded() -> bool:
            # without the win11-csd plugin Hyprland would not give the keyboard back when the user then
            # clicks a window, so the desktop only takes it when the plugin is there
            from .util import hyprctl
            return "win11-csd" in (hyprctl("plugin list", timeout=0.3) or "")

        def on_desktop_press(_w, _e):
            if self._desktop_keyboard == "none" and csd_plugin_loaded():
                desktop_keyboard("exclusive")
                m["GLib"].timeout_add(60, lambda: (self._desktop_keyboard == "exclusive" and
                                                   desktop_keyboard("on-demand"), False)[1])
            return False

        def on_desktop_focus_out(_w, _e):
            desktop_keyboard("none")
            return False

        dview.connect("button-press-event", on_desktop_press)
        desktop.connect("focus-out-event", on_desktop_focus_out)
        desktop.show_all()
        dview.grab_focus()   # the page gets the keys as soon as the compositor gives the surface focus
        self.windows["desktop"] = desktop
        self.views["desktop"] = dview

        # Shell: taskbar, flyouts, toasts, OSD. Input only where JS says (initially the taskbar).
        shell = self._layer_window("shell", "TOP", ("TOP", "BOTTOM", "LEFT", "RIGHT"), -1,
                                   "none", True, monitor)
        # If the page dies (or hangs) while a flyout holds the keyboard exclusively, the user could not
        # type anywhere: give the keyboard back and shrink the input region to the taskbar strip.
        sview = self._webview(transparent=True, on_crash=lambda _v, _r: self._release_shell("gestopt"))
        sview.load_uri(self.ctx.ui_url(surface="shell"))
        shell.add(sview)
        shell.input_shape_combine_region(cairo.Region([cairo.RectangleInt(0, height - TASKBAR_HEIGHT,
                                                                          width, TASKBAR_HEIGHT)]))

        def on_shell_leave(_w, event):
            # The pointer left the shell surface's input region (e.g. straight up off the taskbar). WebKit
            # moves its pointer to the leave position, which is still on the bar (the view is screen-sized,
            # only the input region is small), so the page would never see pointerleave there (auto-hide,
            # hover previews) and the button under it would keep its :hover look. So WebKit gets the leave
            # just outside the view (it runs after this handler), and the page also gets a DOM event.
            try:
                if event.detail == Gdk.NotifyType.INFERIOR:
                    return False
                event.y = -1.0
            except Exception:  # noqa: BLE001
                log.debug("leave-notify niet aangepast (not adjusted)", exc_info=True)
            try:
                sview.run_javascript("dispatchEvent(new Event('shell-pointerleft'))", None, None, None)
            except Exception:  # noqa: BLE001
                log.debug("shell-pointerleft niet doorgegeven (not passed on)", exc_info=True)
            return False

        sview.connect("leave-notify-event", on_shell_leave)
        shell.show_all()
        self._keyboard = "none"
        self.windows["shell"] = shell
        self.views["shell"] = sview
        self.layer_active = True
        m["GLib"].timeout_add_seconds(15, self._keyboard_watchdog)

        def on_settings(settings, path, changed):
            if "taskbar.autoHide" in changed:
                hide = bool((settings.get("taskbar") or {}).get("autoHide"))
                self.ctx.run_on_main(lambda: self._gls.set_exclusive_zone(spacer, 0 if hide else TASKBAR_HEIGHT),
                                     wait=False)

        try:
            self.ctx.settings.on_change(on_settings)
        except Exception:  # noqa: BLE001
            pass
        log.info("Laag-oppervlakken actief op primaire monitor (layer surfaces up on primary monitor) %dx%d",
                 width, height)

    def _taskbar_rect(self) -> dict:
        w, h = self._monitor_geo or (1920, 1080)
        return {"x": 0, "y": h - TASKBAR_HEIGHT, "w": w, "h": TASKBAR_HEIGHT}

    def _release_shell(self, why: str) -> None:
        """Main thread: keyboard back to none, input only on the taskbar (the page re-sends its regions
        once it runs again)."""
        if self.windows.get("shell") is None:
            return
        if self._keyboard == "exclusive":
            log.warning("Toetsenbord vrijgegeven: de taakbalkpagina is %s (keyboard released: shell page %s)",
                        why, "terminated" if why == "gestopt" else "not responding")
        self._apply_regions([self._taskbar_rect()], "none")

    def _keyboard_watchdog(self) -> bool:
        """Every 15 s while the shell holds the keyboard exclusively: is its page still alive? A probe that
        did not come back within 15 s means a hung page → release the keyboard."""
        view = self.views.get("shell")
        if view is None or not self.layer_active:
            return False
        if self._keyboard != "exclusive":
            self._probe = None
            return True
        probe = getattr(self, "_probe", None)
        if probe is not None and not probe["done"]:
            self._probe = None
            self._release_shell("vastgelopen")
            return True
        probe = {"done": False}
        self._probe = probe

        def done(v, result, _data=None):
            probe["done"] = True
            try:
                v.run_javascript_finish(result)
            except Exception:  # noqa: BLE001
                pass

        try:
            view.run_javascript("1", None, done, None)
        except Exception:  # noqa: BLE001
            probe["done"] = True
        return True

    def set_regions(self, rects: list[dict], keyboard: str | None = None, sync: bool = False) -> bool:
        """Thread-safe: set the shell surface input region (+ keyboard mode). ``sync``: return only once a
        keyboard change has been committed to the compositor (≤ ~150 ms; also one an earlier call made
        that is not committed yet: the page's throttled send may have come just before; the shell gives the keyboard
        back before a tray app is activated, so the window it shows can take the focus)."""
        if not self.layer_active:
            return False
        if not sync or threading.current_thread() is threading.main_thread():
            self.ctx.run_on_main(self._apply_regions, rects, keyboard, wait=False)
            return True
        committed = threading.Event()
        try:
            self.ctx.run_on_main(self._apply_regions, rects, keyboard, committed, wait=True, timeout=1.0)
        except Exception:  # noqa: BLE001 - a busy loop: do not hold the request any longer
            return True
        committed.wait(0.2)
        pending = self._kb_commit          # this call's change, or an earlier (not sync) one still on its way
        if pending is not None:
            pending.wait(0.2)
        return True

    def _after_commit(self, win, done: threading.Event) -> None:
        """Main thread: set ``done`` after the window's next frame (gtk-layer-shell commits a keyboard-mode
        change with it), or after 150 ms when no frame comes."""
        clock = win.get_frame_clock() if hasattr(win, "get_frame_clock") else None
        if clock is None:
            done.set()
            return
        from gi.repository import GLib
        hid = [0]

        def finish(*_a):
            done.set()
            if hid[0]:
                try:
                    clock.disconnect(hid[0])
                except Exception:  # noqa: BLE001
                    pass
                hid[0] = 0
            return False

        hid[0] = clock.connect("after-paint", finish)
        win.queue_draw()
        GLib.timeout_add(150, finish)

    def _apply_regions(self, rects: list[dict], keyboard: str | None, committed: threading.Event | None = None) -> None:
        import cairo
        shell = self.windows.get("shell")
        if shell is None:
            if committed is not None:
                committed.set()
            return
        w, h = self._monitor_geo or (100000, 100000)
        boxes = []
        for r in rects:
            x0, y0 = max(0, int(r["x"])), max(0, int(r["y"]))
            x1, y1 = min(w, int(r["x"]) + int(r["w"])), min(h, int(r["y"]) + int(r["h"]))
            if x1 > x0 and y1 > y0:
                boxes.append(cairo.RectangleInt(x0, y0, x1 - x0, y1 - y0))
        shell.input_shape_combine_region(cairo.Region(boxes))
        if keyboard:
            # the page says 'on-demand' when nothing holds the keyboard; on Hyprland that would steal the
            # keyboard on hover, so the shell layer only ever has none or exclusive
            keyboard = "exclusive" if keyboard == "exclusive" else "none"
        changed = bool(keyboard) and keyboard != self._keyboard
        if changed:
            self._set_keyboard(shell, keyboard)
            self._keyboard = keyboard
            pending = threading.Event()
            self._kb_commit = pending
            self._after_commit(shell, pending)
        if committed is not None:
            committed.set()    # applied; set_regions(sync) then waits for self._kb_commit

    # -- preview mode -----------------------------------------------------------
    def _start_preview(self, on_close: Callable[[], None] | None = None) -> None:
        """The desktop in one window. Closing it quits the host, or calls ``on_close`` (preview opened from the
        problem notice: the host keeps running)."""
        m = gi_modules(webkit=True)
        Gtk, Gdk = m["Gtk"], m["Gdk"]
        en = self._settings("language", "nl") == "en"
        win = Gtk.Window(title="Win11 Shell (preview)" if en else "Win11 Shell (voorbeeld)")
        win.set_icon_name("preferences-desktop")
        w, h = 1440, 900
        monitor = _primary_monitor(Gdk)
        if monitor is not None:
            wa = monitor.get_workarea()
            w, h = min(w, max(800, wa.width - 48)), min(h, max(600, wa.height - 48))
        win.set_default_size(w, h)
        win.set_position(Gtk.WindowPosition.CENTER)
        dark = self._settings("theme.mode", "dark") != "light"
        view = self._webview(transparent=False, bg="#202020" if dark else "#f3f3f3")
        view.load_uri(self.ctx.ui_url(surface="preview"))
        win.add(view)

        def on_delete(*_a):
            if on_close is not None:
                on_close()
                return True
            self.ctx.request_quit(0)
            return False

        win.connect("delete-event", on_delete)
        win.show_all()
        self.windows["preview"] = win
        self.views["preview"] = view
        log.info("Voorbeeldvenster geopend (preview window open) %dx%d", w, h)

    # -- file picker ------------------------------------------------------------
    def pick_file(self, title: str | None = None, filters: list | None = None, folder: bool = False,
                  timeout: float = 1800.0) -> str | None:
        """Native file chooser (blocks the calling worker thread, never the main loop)."""
        if not self.gtk_ready:
            return None
        done = threading.Event()
        box: dict[str, Any] = {"path": None}
        en = self._settings("language", "nl") == "en"

        def show():
            Gtk = gi_modules(webkit=False)["Gtk"]
            action = Gtk.FileChooserAction.SELECT_FOLDER if folder else Gtk.FileChooserAction.OPEN
            default_title = ("Choose a folder" if folder else "Choose a file") if en else \
                ("Map kiezen" if folder else "Bestand kiezen")
            parent = self.windows.get("preview")
            dlg = Gtk.FileChooserNative.new(str(title or default_title)[:200], parent, action,
                                            "Choose" if en else "Kiezen", "Cancel" if en else "Annuleren")
            for f in (filters or [])[:20]:
                if not isinstance(f, dict):
                    continue
                ff = Gtk.FileFilter()
                ff.set_name(str(f.get("name") or "")[:100])
                for pat in (f.get("patterns") or [])[:50]:
                    pat = str(pat)
                    (ff.add_mime_type if "/" in pat else ff.add_pattern)(pat)
                dlg.add_filter(ff)

            def on_response(d, resp):
                if resp == Gtk.ResponseType.ACCEPT:
                    box["path"] = d.get_filename()
                d.destroy()
                if d in self._dialogs:
                    self._dialogs.remove(d)
                done.set()

            dlg.connect("response", on_response)
            self._dialogs.append(dlg)
            dlg.show()
            return False

        if threading.current_thread() is threading.main_thread():
            log.warning("pick_file op hoofdthread niet ondersteund (not supported on main thread)")
            return None
        self.ctx.run_on_main(show, wait=False)
        done.wait(timeout)
        return box["path"]

    # -- shutdown ---------------------------------------------------------------
    def destroy(self) -> None:
        notice, self._notice = self._notice, None
        if notice is not None and getattr(notice, "win", None) is not None:
            try:
                notice.win.destroy()
            except Exception:  # noqa: BLE001
                pass
        for w in list(self.windows.values()):
            try:
                w.destroy()
            except Exception:  # noqa: BLE001
                pass
        self.windows.clear()
        self.views.clear()
        self.layer_active = False
