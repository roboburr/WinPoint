"""Scenario support for tests/gnome/harness.py (GE1h).

A scenario is a Python file with ``def run(h):``. The harness executes it inside the bubblewrap sandbox, inside
the private dbus-run-session, after its own GNOME Shell is up and the extension has started the native shell
(D-G70: win11-shell, one GTK 4 process with the taskbar, its flyouts, Start and the desktop, and its backend
``win11-panel --http-only``), which has reported its boot. ``h`` is a :class:`Helper`. Everything a scenario does
goes through ``h``; every helper call is recorded (``steps``) in ``results.json`` next to the screenshots.

Run one (from G; the heavy lock and the memory check come from tools/heavy.sh, contracts C8):

    tools/heavy.sh --min-mb 2560 --tag <step> -- \\
        python3 -B tests/gnome/harness.py launch run --lang nl --scenario tests/gnome/scenarios/native-shell.py

Surfaces (``h.open(spec)``): ``panel``, ``desktop``, ``start`` (windows of the native shell, win11-shell),
``explorer``, ``settings`` or ``settings:<page>`` (a page id from ``native/apps/settings/src/registry.rs``, e.g.
``settings:language``), ``taskmgr``, ``alles``, ``disks``, ``defaultapps``, ``installer`` (the native GTK 4 apps,
win11-gtk-<kind>, started through their launchers). After ``open``, a surface is addressed by its name (the spec
without the page, or the ``name=`` given to ``open``).

Text, widget state and input come from the dev channels on WIN11_DEV_COMMAND/WIN11_DEV_REPORT: the shell's
(``h.native``, ``h.native_state``; native/shell/src/dev.rs) and each app's (``h.state``, ``h.find``, ``h.dev``;
native/kit/src/dev.rs), where widgets are found by their ``set_widget_name``; GNOME facts (window titles, app
names, the extension's own St widgets) through org.gnome.Shell.Eval on the private bus; real pointer input through
a Clutter virtual pointer device inside the private shell, at a widget's centre: the window's frame rectangle as
Mutter has it (the window geometry GTK sets, so without the CSD shadow of a dialog or popup; a kit window draws
none) plus the widget's rectangle in the window from the dev state (a popover's widgets included).
"""
from __future__ import annotations

import json
import os
import re
import subprocess
import time
import traceback
import urllib.request
from pathlib import Path

# install.py's desktop entries (D-G70: no Start and no Desktop entry; the Calendar app's entry instead)
OWN_ENTRIES = ('Explorer', 'Panel', 'TaskManager', 'Settings', 'Search', 'Disks', 'DefaultApps', 'Installer', 'Calendar')
OWN_DESKTOP_IDS = tuple(f'io.github.win11shell.{name}.desktop' for name in OWN_ENTRIES)
WM = {'panel': 'io.github.win11shell.Panel', 'desktop': 'io.github.win11shell.Desktop',
      'start': 'io.github.win11shell.Start', 'explorer': 'io.github.win11shell.Explorer',
      'settings': 'io.github.win11shell.Settings', 'taskmgr': 'io.github.win11shell.TaskManager',
      'alles': 'io.github.win11shell.Search', 'disks': 'io.github.win11shell.Disks',
      'defaultapps': 'io.github.win11shell.DefaultApps', 'installer': 'io.github.win11shell.Installer'}
KINDS = tuple(WM)
# Started by the extension, not by h.open: windows of the one native shell process (win11-shell)
RESIDENT = ('panel', 'desktop', 'start')
# The option label prefixes of Settings > Tijd en taal > Taal en regio's display language drop-down
# (native/apps/settings/src/pages/time.rs language(): "Nederlands", "English (United Kingdom)")
LANG_OPTION = {'nl': 'Nederlands', 'en': 'English (United Kingdom)'}
VIAS = ('dropdown', 'rename', 'rewrite')

# Shell side (org.gnome.Shell.Eval; harness.Session.ev provides Main and Meta).
POINTER_CLICK_JS = (
    "(async()=>{{const Clutter=(await import('gi://Clutter')).default;const GLib=(await import('gi://GLib')).default;"
    "const wait=ms=>new Promise(r=>GLib.timeout_add(GLib.PRIORITY_DEFAULT,ms,()=>{{r();return GLib.SOURCE_REMOVE;}}));"
    "const seat=Clutter.get_default_backend().get_default_seat();"
    "globalThis.__ge1hPointer??=seat.create_virtual_device(Clutter.InputDeviceType.POINTER_DEVICE);"
    "const d=globalThis.__ge1hPointer,us=()=>GLib.get_monotonic_time();"
    "d.notify_absolute_motion(us(),{x},{y});await wait(80);const [px,py]=global.get_pointer();"
    "d.notify_button(us(),Clutter.BUTTON_PRIMARY,Clutter.ButtonState.PRESSED);await wait(50);"
    "d.notify_button(us(),Clutter.BUTTON_PRIMARY,Clutter.ButtonState.RELEASED);await wait(30);"
    "const w=global.display.focus_window;return {{pointer:[px,py],focusPid:w?w.get_pid():null}};}})()")

# Created once, before the first click: Wayland clients bind wl_pointer only after the seat gains a pointer,
# so a click sent right after creating the device can be lost. Parks the pointer on empty wallpaper.
POINTER_PREPARE_JS = (
    "(async()=>{{const Clutter=(await import('gi://Clutter')).default;const GLib=(await import('gi://GLib')).default;"
    "const seat=Clutter.get_default_backend().get_default_seat();"
    "globalThis.__ge1hPointer??=seat.create_virtual_device(Clutter.InputDeviceType.POINTER_DEVICE);"
    "globalThis.__ge1hPointer.notify_absolute_motion(GLib.get_monotonic_time(),{x},{y});"
    "return global.get_pointer().slice(0,2);}})()")

WINDOW_TITLES_JS = ("global.get_window_actors().map(a=>({wm:a.meta_window.get_wm_class(),title:a.meta_window.get_title(),"
                    "visible:a.visible}))")

# The windows of one process (%d: its pid) as Mutter has them: the frame rectangle is the window geometry GTK sets,
# which leaves out a CSD shadow, so a widget's rect in the dev state starts at its top left corner.
PID_WINDOWS_JS = ("(()=>{const type=t=>Object.keys(Meta.WindowType).find(k=>Meta.WindowType[k]===t)||String(t);"
                  "return global.get_window_actors().map(a=>a.meta_window).filter(w=>w.get_pid()===%d)"
                  ".map(w=>{const r=w.get_frame_rect();return {title:w.get_title(),type:type(w.get_window_type()),"
                  "x:r.x,y:r.y,w:r.width,h:r.height,focus:w.has_focus(),minimized:w.minimized};});})()")

APP_NAMES_JS = ("(async()=>{{const Shell=(await import('gi://Shell')).default;const s=Shell.AppSystem.get_default();"
                "return Object.fromEntries({ids}.map(id=>{{const a=s.lookup_app(id);return [id,a?a.get_name():null];}}));}})()")

# The extension's own St widgets: the fallback taskbar (shown while win11-shell is missing), every accessible
# name and label in it, the clock text and the Ctrl+Alt+Tab group. Walks the actor tree, so it does not depend
# on the field names GE1c may add.
EXTENSION_LABELS_JS = (
    "(()=>{{const ext=Main.extensionManager.lookup('{uuid}');const s=ext&&ext.stateObj;"
    "if(!s)return {{error:'extension has no stateObj',state:ext?ext.state:null}};const bar=s._bar;const nodes=[];"
    "const walk=(a,depth)=>{{if(!a||depth>14)return;const row={{type:(a.constructor&&a.constructor.name)||'?',"
    "style:a.style_class||null,visible:!!a.visible}};"
    "if(a.accessible_name)row.accessibleName=a.accessible_name;"
    "if(typeof a.text==='string'&&a.text)row.text=a.text;"
    "if(typeof a.label==='string'&&a.label)row.label=a.label;"
    "if(row.accessibleName||row.text||row.label)nodes.push(row);"
    "if(a.clutter_text)return;for(const c of (a.get_children?a.get_children():[]))walk(c,depth+1);}};"
    "walk(bar,0);const cat=(Main.ctrlAltTabManager&&Main.ctrlAltTabManager._items)||[];"
    "return {{present:!!bar,barVisible:!!(bar&&bar.visible),barMapped:!!(bar&&bar.mapped),"
    "nativePanelActive:!!s._nativePanelActive,panelPresent:!!(s._panelBridge&&s._panelBridge._panelPresent),"
    "clock:s._clock?s._clock.text:null,"
    "accessibleNames:nodes.filter(n=>n.accessibleName).map(n=>n.accessibleName),"
    "labels:nodes.filter(n=>n.text||n.label).map(n=>n.text||n.label),nodes,"
    "ctrlAltTab:cat.filter(i=>i.root===bar).map(i=>i.name),ctrlAltTabAll:cat.map(i=>i.name)}};}})()")

# More than one monitor (--second-monitor): the monitors as GNOME has them, the taskbar windows of the other monitors
# (win11-shell, app id io.github.win11shell.SecondaryTaskbar.Monitor<N>) and a monitor's work area.
MONITORS_JS = ("Main.layoutManager.monitors.map(m=>({index:m.index,x:m.x,y:m.y,w:m.width,h:m.height,"
               "primary:m.index===Main.layoutManager.primaryIndex,"
               "fullscreen:global.display.get_monitor_in_fullscreen(m.index)}))")
STRIP_WINDOWS_JS = ("(()=>{const type=t=>Object.keys(Meta.WindowType).find(k=>Meta.WindowType[k]===t)||String(t);"
                    "return global.get_window_actors().map(a=>a.meta_window).filter(w=>/^io\\.github\\.win11shell\\."
                    "SecondaryTaskbar\\.Monitor\\d+$/.test(w.get_wm_class()||'')).map(w=>{const r=w.get_frame_rect();"
                    "const a=w.get_compositor_private();return {wm:w.get_wm_class(),title:w.get_title(),pid:w.get_pid(),"
                    "type:type(w.get_window_type()),x:r.x,y:r.y,w:r.width,h:r.height,monitor:w.get_monitor(),"
                    "mapped:!!(a&&a.mapped),visible:!!(a&&a.visible),above:w.is_above(),sticky:w.is_on_all_workspaces(),"
                    "skipTaskbar:w.skip_taskbar};});})()")
WORK_AREA_JS = ("(()=>{const r=global.workspace_manager.get_active_workspace().get_work_area_for_monitor(%d);"
                "return {x:r.x,y:r.y,w:r.width,h:r.height};})()")

PANEL_OPEN_JS = ("(async()=>{const Shell=(await import('gi://Shell')).default;"
                 "const app=Shell.AppSystem.get_default().lookup_app('io.github.win11shell.Panel.desktop');"
                 "if(!app)return false;app.open_new_window(-1);return true;})()")


def secret_methods(root: Path) -> frozenset[str]:
    """Use the host's is_secret manifests, so new secret methods stay private automatically."""
    source = root / 'host' / 'src'
    files = [source / 'api-methods.json', *sorted((source / 'api-methods').glob('*.json'))]
    return frozenset(row['name'] for path in files for row in json.loads(path.read_text())
                     if row.get('secret') is True)


def redact_api(value, secrets: frozenset[str]):
    """Redact recorded API payloads without changing the values the scenario itself checks."""
    if isinstance(value, list):
        return [redact_api(item, secrets) for item in value]
    if not isinstance(value, dict):
        return value
    out = {key: redact_api(item, secrets) for key, item in value.items()}
    args = value.get('args')
    method = value.get('method') or (args.get('method') if isinstance(args, dict) else None)
    if isinstance(method, str) and method in secrets:
        for key in ('args', 'result', 'value', 'error', 'data'):
            if key not in out:
                continue
            if key == 'args' and isinstance(args, dict) and args.get('method') == method:
                out[key] = {k: v for k, v in args.items() if k in ('method', 'surface')}
            elif key == 'result' and isinstance(out[key], dict) and set(out[key]) == {'ok'}:
                pass
            else:
                out[key] = '[redacted]'
    return out


SCROLL_JS = "(async()=>{{const Clutter=(await import('gi://Clutter')).default;const GLib=(await import('gi://GLib')).default;const wait=ms=>new Promise(r=>GLib.timeout_add(GLib.PRIORITY_DEFAULT,ms,()=>{{r();return GLib.SOURCE_REMOVE;}}));const seat=Clutter.get_default_backend().get_default_seat();globalThis.__ge1hPointer??=seat.create_virtual_device(Clutter.InputDeviceType.POINTER_DEVICE);const d=globalThis.__ge1hPointer,us=()=>GLib.get_monotonic_time();d.notify_absolute_motion(us(),{x},{y});await wait(60);for(let i=0;i<{n};i++){{d.notify_discrete_scroll(us(),Clutter.ScrollDirection.{dir},Clutter.ScrollSource.WHEEL);await wait(35);}}await wait(200);return true;}})()"


class Surface:
    """One open Win11 surface: a window of the native shell (``shell``: panel, desktop or Start, all of the one
    win11-shell process) or a native app, with the process the helper started (its launcher, which the host turns
    into win11-gtk-<kind> in place) and the app's pid from its boot row. ``window`` is the id its dev channel
    answers with: "native" for the shell, "native-<kind>" for an app."""

    def __init__(self, name: str, kind: str, window: str, proc=None, pid: int | None = None):
        self.name, self.kind, self.window, self.proc = name, kind, window, proc
        self.pid = pid if pid is not None else (proc.pid if proc else None)
        self.page: str | None = None
        self.dead = False

    @property
    def shell(self) -> bool:
        return self.kind in RESIDENT

    def alive(self) -> bool:
        if self.dead or not self.pid:
            return False
        try:
            stat = Path(f'/proc/{self.pid}/stat').read_text()
        except OSError:
            return False
        return not stat[stat.rindex(')') + 2:].startswith('Z')

    def describe(self) -> dict:
        return {'name': self.name, 'kind': self.kind, 'window': self.window, 'pid': self.pid, 'page': self.page,
                'alive': self.alive(), **({'shell': True} if self.shell else {})}


class Helper:
    """The ``h`` object given to ``run(h)``. Methods, all recorded in results.json:

    Surfaces    open(spec, path=None, tabs=1, name=None, env=None), close(name), new_tab(name='explorer'),
                surface(name), spawn(name, argv, env=None)
    Widgets     state(surface), find(surface, name=None, label=None, cls=None, where=None),
                click(surface, name=None, node=None, real=True), type_text(surface, name, text),
                key(surface, key, name=None), dev(surface, command), backend(surface, method, args=None)
    Evidence    screenshot(name, surface=None, area=None, settle=0.5), page_text(surface), text(surface),
                wait_text(surface, text, timeout=20, required=False), wait_lang(surface, lang, timeout=20)
    Language    set_language(lang, via='rename'|'dropdown'|'rewrite', skip=()), settings_file(), lang
    Native      native(command) (win11-shell's dev channel: {"native": "state"|"click"|"flyout", ...}),
                native_state(), native_pid
    GNOME       window_titles(), app_names(ids=None), extension_labels(), notifications(), kill_panel(),
                restart_panel()
    GSettings   gsettings_get(schema, key, path=None), gsettings_set(schema, key, value, path=None) on the run's private
                keyfile (run/config/glib-2.0/settings/keyfile; GVariant text, as GNOME's keyfile backend stores it);
                settings_data_page(parent, item, title=None) (settings parity: a data page is reached by clicking
                through its parent, notes/claude/settings-parity.md §7)
    Recording   record(key, value), check(name, ok, known=None, **detail), require(name, ok, **detail), log(msg)
    Low level   eval(js) (Shell Eval), until(fn, label, timeout), sleep(seconds), session (the harness Session),
                hm (the harness module)
    """

    def __init__(self, session, hm, scenario: Path):
        self.session, self.s, self.hm = session, session, hm
        self.scenario = scenario
        self._secret_methods = secret_methods(hm.SRC)
        self.t0 = time.monotonic()
        self.surfaces: dict[str, Surface] = {}
        self._start_rect = None
        self._shot_names: set[str] = set()
        self._add_native(session.facts.get('windowIds', {}).get('native'))
        self.results = {'scenario': str(scenario), 'startLanguage': session.lang, 'steps': [], 'checks': [],
                        'records': {}, 'shots': [], 'error': None}
        self._results_file = hm.RUN / 'logs' / 'scenario-results.json'
        self._flush()
        try:
            parked = session.ev(POINTER_PREPARE_JS.format(x=hm.WIDTH // 2, y=24))
            time.sleep(0.8)
        except Exception as error:   # noqa: BLE001 - real clicks then fall back to dev clicks (recorded)
            parked = f'failed: {error}'
        self._step('pointer_prepare', None, {'pointer': parked})

    def _add_native(self, pid: int | None) -> None:
        """The native shell's panel, desktop and Start as pseudo-surfaces of its one process (D-G70)."""
        if pid:
            for kind in RESIDENT:
                self.surfaces[kind] = Surface(kind, kind, 'native', pid=pid)

    @property
    def native_pid(self) -> int | None:
        """The pid of the running native shell (win11-shell), or None after kill_panel()."""
        sf = self.surfaces.get('panel')
        return sf.pid if sf and sf.alive() else None

    # ---- recording ---------------------------------------------------------------------------------------
    def _t(self) -> float:
        return round(time.monotonic() - self.t0, 2)

    def _flush(self) -> None:
        self.results['seconds'] = self._t()
        self.results['surfaces'] = {n: s.describe() for n, s in self.surfaces.items()}
        try:
            self._results_file.write_text(json.dumps(redact_api(self.results, self._secret_methods), indent=1, ensure_ascii=False) + '\n')
        except OSError:
            pass

    def _step(self, step: str, args: dict | None = None, result=None, **extra):
        row = {'step': step, 't': self._t()}
        if args:
            row['args'] = args
        if result is not None:
            row['result'] = result
        row.update(extra)
        self.results['steps'].append(row)
        self._flush()
        return result

    def log(self, msg: str) -> None:
        self.hm.log(f'  scenario: {msg}')
        self._step('log', {'msg': msg})

    def record(self, key: str, value):
        """Store a named value in results.json ``records`` (and as a step)."""
        self.results['records'][key] = value
        self._step('record', {'key': key})
        return value

    def check(self, name: str, ok, known: str | None = None, **detail) -> bool:
        """An expectation. Never raises; a failed check does not change the exit code. ``known`` names a known
        baseline defect (for example 'F1'), so a failure there is expected until the fix lands."""
        row = {'check': name, 'ok': bool(ok), 't': self._t(), **({'known': known} if known else {}), **detail}
        self.results['checks'].append(row)
        self.hm.log(f'  check {"PASS" if ok else "FAIL"}{" (known " + known + ")" if known and not ok else ""}: {name}')
        self._flush()
        return bool(ok)

    def require(self, name: str, ok, **detail) -> None:
        """Like check, but a failure stops the scenario (exit code 1)."""
        if not self.check(name, ok, **detail):
            raise AssertionError(f'required check failed: {name}')

    # ---- low level ---------------------------------------------------------------------------------------
    @property
    def lang(self) -> str:
        return self.s.lang

    def sleep(self, seconds: float) -> None:
        time.sleep(seconds)

    def until(self, check, label: str, timeout: float = 20, interval: float = 0.2):
        return self.hm.until(check, label, timeout, interval)

    def eval(self, js: str):
        """org.gnome.Shell.Eval in the private shell (Main and Meta are in scope)."""
        return self.s.ev(js)

    def surface(self, name: str) -> Surface:
        sf = self.surfaces.get(name)
        if not sf:
            raise KeyError(f'surface {name!r} is not open (open: {sorted(self.surfaces)})')
        if not sf.alive():
            raise RuntimeError(f'surface {name!r} is no longer running')
        return sf

    def native(self, command: dict, timeout: float = 15):
        """One command for win11-shell's dev channel (native/shell/src/dev.rs): {"native": "state"} (the
        toplevels' widget trees), {"native": "click", "name": <widget name>}, {"native": "flyout", "name": n}."""
        value = self.s.native(command, timeout)
        self._step('native', command, value if command.get('native') != 'state' else {'toplevels': len(value or [])})
        return value

    def native_state(self) -> list[dict]:
        """win11-shell's toplevels: title, visible, width, height and the widget tree (name, classes, label, rect)."""
        value = self.native({'native': 'state'})
        return value if isinstance(value, list) else []

    def dev(self, surface: str, command: dict, timeout: float = 20):
        """One dev command for the surface's process: the shell's (panel, desktop, start) or the app's
        (native/kit/src/dev.rs: {"native": "state"|"click"|"type"|"key"|"screenshot", ...}; "app" is added)."""
        sf = self.surface(surface)
        value = self.s.native(command, timeout) if sf.shell else self.s.native_app(sf.kind, command, timeout)
        self._step('dev', {'surface': surface, **command},
                   value if command.get('native') != 'state' else {'toplevels': len(value or [])})
        return value

    def state(self, surface: str) -> list[dict]:
        """The surface's toplevels from its dev channel: title, visible, width, height and the widget tree (name, type,
        classes, label (a button's or a tooltip when it has none), rect in the window, visible, sensitive, active)."""
        sf = self.surface(surface)
        value = self.s.native_state() if sf.shell else self.s.native_app(sf.kind, {'native': 'state'}, 20)
        return value if isinstance(value, list) else []

    def find(self, surface: str, name: str | None = None, *, label: str | None = None, cls: str | None = None,
             where=None, state: list[dict] | None = None) -> list[dict]:
        """The visible widgets of the surface with that name, label and CSS class (each given one must match) and for
        which ``where(node)`` holds, depth first; each node carries the index of its toplevel as ``top``."""
        tops = self.state(surface) if state is None else state
        out = []
        for index, top in enumerate(tops):
            if not top.get('visible', True):
                continue
            for node in self.hm.tree_nodes([top]):
                if (node.get('visible', True) and (name is None or node.get('name') == name)
                        and (label is None or node.get('label') == label)
                        and (cls is None or cls in (node.get('classes') or []))
                        and (where is None or where(node))):
                    out.append({**node, 'top': index})
        return out

    def _frame(self, sf: Surface, top: dict) -> dict | None:
        """Mutter's frame rectangle of the toplevel ``top`` (a dev state entry) of the surface's process: the window
        with its title and size, else the one with its title, else the only one (an app's normal windows; the shell's
        taskbar, Start and desktop are other window types, and a popover is not a toplevel of the dev state)."""
        wins = [w for w in self.s.ev(PID_WINDOWS_JS % sf.pid) or [] if not w['minimized']
                and (sf.shell or w['type'] == 'NORMAL') and w['type'] not in ('DROPDOWN_MENU', 'POPUP_MENU')]
        titled = [w for w in wins if w['title'] == top.get('title')] or wins
        sized = [w for w in titled if (w['w'], w['h']) == (top.get('width'), top.get('height'))] or titled
        return sized[0] if sized else None

    def _reveal_click_target(self, surface: str, name: str, node: dict, tops: list[dict]):
        """Scroll the target into its nearest viewport, including a list inside a modal dialog."""
        sf = self.surface(surface)
        sign, previous, wanted_down = 1, None, True
        for attempt in range(24):
            top = tops[node.get('top', 0)] if tops else None
            frame = self._frame(sf, top) if top else None
            rect = node.get('rect')
            if not frame or not rect:
                break

            def ancestors(candidate, parents):
                if candidate.get('name') == name and candidate.get('rect') == rect:
                    return parents
                for child in candidate.get('children', []):
                    found = ancestors(child, parents + [candidate])
                    if found is not None:
                        return found
                return None

            chain = ancestors(top.get('tree', {}), []) or []
            scrollers = [p['rect'] for p in chain if p.get('type') == 'GtkScrolledWindow'
                         and p.get('rect') and p['rect'][2] > 0 and p['rect'][3] > 0]
            clip = scrollers[-1] if scrollers else [0, 0, frame['w'], frame['h']]
            _, y, _, height = rect
            centre = y + height / 2
            low, high = max(0, clip[1]) + 4, min(frame['h'], clip[1] + clip[3]) - 4
            if low <= centre < high:
                break
            # A private desktop may have natural scrolling enabled by an earlier control test.
            if previous is not None and (centre - previous) * (1 if wanted_down else -1) > 1:
                sign = -sign
            wanted_down = centre >= high
            distance = centre - (low + high) / 2
            steps = max(1, min(10, int(abs(distance) / 60)))
            down = wanted_down if sign > 0 else not wanted_down
            previous = centre
            self.s.ev(SCROLL_JS.format(x=frame['x'] + clip[0] + min(12, clip[2] // 2),
                                      y=frame['y'] + int((low + high) / 2), n=steps,
                                      dir='DOWN' if down else 'UP'))
            tops = self.state(surface)
            found = self.find(surface, name, state=tops)
            if not found:
                break
            node = found[0]
        return node, tops

    def click(self, surface: str, name: str | None = None, *, node: dict | None = None, real: bool = True) -> dict:
        """Click a widget: by ``name`` (its set_widget_name) or a ``node`` from find(). real=True moves a Clutter
        virtual pointer to the widget's centre (its window's frame rectangle plus its rect in the window) and presses
        the primary button: real Wayland input. real=False, or a widget without a rect, is the dev channel's click
        (it activates the widget by name: a button's clicked, a switch or check toggled)."""
        sf = self.surface(surface)
        tops = self.state(surface)
        if node is None:
            found = [n for n in self.find(surface, name, state=tops) if n.get('rect')] or self.find(surface, name, state=tops)
            if not found:
                raise LookupError(f'{surface}: no visible widget {name!r}')
            node = found[0]
        name = name or (node.get('name') if not str(node.get('name') or '').startswith('Gtk') else None)
        if real and name:
            node, tops = self._reveal_click_target(surface, name, node, tops)
        frame = self._frame(sf, tops[node.get('top', 0)]) if real and node.get('rect') and tops else None
        x, y, w, hh = node.get('rect') or (0, 0, 0, 0)
        # a widget scrolled out of its window has a rect outside it: the pointer would land on another window
        outside = bool(frame) and not (0 <= x + w // 2 < frame['w'] and 0 <= y + hh // 2 < frame['h'])
        if not frame or outside:
            if not name:
                raise LookupError(f'{surface}: the widget has no name for a dev click and no place for the pointer')
            if real:
                self._step('click_fallback', {'surface': surface, 'name': name,
                                              'reason': 'outside-window' if outside else 'no-pointer-target'})
            answer = self.dev(surface, {'native': 'click', 'name': name})
            if isinstance(answer, dict) and answer.get('error'):
                raise LookupError(f'{surface}: {answer["error"]}')
            return {'input': 'dev', 'name': name, 'answer': answer, **({'outside': node['rect']} if outside else {})}
        px, py = frame['x'] + x + w // 2, frame['y'] + y + hh // 2
        if self.s.ev('global.display.focus_window?.get_pid()') != sf.pid:
            self.s.ev(f'(()=>{{const w=global.get_window_actors().map(a=>a.meta_window).find(w=>w.get_pid()==={sf.pid}'
                      f'&&w.get_title()==={json.dumps(frame["title"])});if(w)Main.activateWindow(w);return Boolean(w);}})()')
            time.sleep(0.3)
        shell = self.s.ev(POINTER_CLICK_JS.format(x=px, y=py))
        time.sleep(0.15)
        return {'input': 'clutter', 'x': px, 'y': py, 'rect': node['rect'], 'name': name,
                'window': [frame['x'], frame['y'], frame['w'], frame['h']], 'shell': shell}

    def type_text(self, surface: str, name: str, text: str) -> dict:
        """Set the text of the entry (or text view) called ``name`` through the dev channel."""
        return self.dev(surface, {'native': 'type', 'name': name, 'text': text})

    def key(self, surface: str, key: str, name: str | None = None) -> dict:
        """A key for an app ("Return", "Escape", "<Control>f", "Down", "Tab"; native/kit/src/dev.rs)."""
        return self.dev(surface, {'native': 'key', 'key': key, **({'name': name} if name else {})})

    def bridge(self, method: str, args: dict | None = None):
        """The public asynchronous Shell bridge, inside this harness's private GNOME only."""
        payload = json.dumps(args or {})
        code = """(async()=>{
const b=Main.extensionManager.lookup('win11-gnome@win11shell.local').stateObj._panelBridge;
const method=METHOD, text=PAYLOAD;
if(typeof b.CallAsync!=='function')return JSON.parse(b.Call(method,text));
return await new Promise(resolve=>b.CallAsync([method,text],{
return_value:value=>resolve(JSON.parse(value.deep_unpack()[0]))}));})()"""
        value = self.eval(code.replace('METHOD', json.dumps(method)).replace('PAYLOAD', json.dumps(payload)))
        self._step('bridge', {'method': method}, {'ok': not (isinstance(value, dict) and value.get('error'))})
        return value

    def backend(self, surface: str, method: str, args: dict | None = None, timeout: float = 15):
        """A call to the app's own backend (`win11-app --app <kind> --http-only`) as the app makes it, through the
        instance file it left in the runtime directory (native/kit/src/backend.rs: {port, token}); the answer's
        result, or {"error": ...}."""
        sf = self.surface(surface)
        data = json.loads((self.hm.RT / 'win11-gnome' / f'native-{sf.kind}-{sf.pid}.json').read_text())
        request = urllib.request.Request(f'http://127.0.0.1:{data["port"]}/api/call/{method}',
                                         data=json.dumps({'args': args or {}}).encode(), method='POST',
                                         headers={'X-Shell-Token': data['token'], 'Content-Type': 'application/json'})
        with urllib.request.urlopen(request, timeout=timeout) as reply:
            answer = json.loads(reply.read())
        value = answer.get('result') if answer.get('ok') else {'error': answer.get('error')}
        self._step('backend', {'surface': surface, 'method': method}, {'ok': bool(answer.get('ok'))})
        return value

    def spawn(self, name: str, argv: list[str], env: dict | None = None) -> subprocess.Popen:
        """Start a helper process in the private session (on its Wayland display; stopped at the end of the run),
        logging to logs/app-<name>.log."""
        full = dict(os.environ, WAYLAND_DISPLAY=self.hm.WL, GDK_BACKEND='wayland', **(env or {}))
        proc = subprocess.Popen(argv, env=full, stdout=open(self.hm.RUN / 'logs' / f'app-{name}.log', 'a'),
                                stderr=subprocess.STDOUT)
        self.s.apps.append(proc)
        self._step('spawn', {'name': name, 'argv': argv[:4], **({'env': sorted(env)} if env else {})}, {'pid': proc.pid})
        return proc

    def _meta_window(self, sf: Surface) -> dict | None:
        if sf.shell:
            return self.s.window_of(WM[sf.kind], sf.pid)
        return self.s.window_of_pid(sf.pid) if sf.pid else self.s.window_of(WM[sf.kind])

    # ---- surfaces ----------------------------------------------------------------------------------------
    def open(self, spec: str, *, path: str | None = None, tabs: int = 1, name: str | None = None,
             env: dict | None = None) -> Surface:
        """Open a surface (see the module docs); an app that is open already is focused (Settings shows the page)."""
        kind, _, page = spec.partition(':')
        if kind not in KINDS:
            raise ValueError(f'unknown surface {spec!r}; expected one of {", ".join(KINDS)} (settings:<page>)')
        if page and kind != 'settings':
            raise ValueError(f'only settings takes a page ({spec!r})')
        name = name or kind
        args = {'spec': spec, **({'path': path} if path else {}), **({'tabs': tabs} if tabs != 1 else {}),
                **({'name': name} if name != kind else {}), **({'env': sorted(env)} if env else {})}
        if kind in ('panel', 'desktop'):
            sf = self.surface(kind)
            self._step('open', args, sf.describe())
            return sf
        if kind == 'start':
            sf = self.surface('start')
            self.s.start_flyout(True, sf.pid)
            w = self.s.window_of(WM['start'], sf.pid)
            if w:
                self._start_rect = (w['x'], w['y'], w['w'], w['h'])
            self._step('open', args, {**sf.describe(), 'rect': self._start_rect})
            return sf
        sf = self.surfaces.get(name)
        if sf and sf.alive():
            if kind == 'settings' and page:
                self.s.settings_page(page)
                sf.page = page
            self.s.focus(sf.pid, name)
            if kind == 'explorer':
                for _ in range(max(0, tabs - self._tab_count(name))):
                    self.new_tab(name)
            self._step('open', args, {**sf.describe(), 'reused': True})
            return sf
        bindir = self.hm.RUN / 'prefix' / 'bin'
        if kind == 'explorer':
            argv = [str(bindir / 'win11-explorer'), path or str(self.hm.RUN / 'home')]
        elif kind == 'settings':
            argv = [str(bindir / 'win11-app'), '--app', 'settings', '--params', json.dumps({'page': page or 'home'})]
        elif kind == 'defaultapps':
            argv = [str(bindir / 'win11-default-apps')]
        elif kind == 'installer':
            argv = [str(bindir / 'win11-installer')] + ([path] if path else [])
        else:
            argv = [str(bindir / 'win11-app'), '--app', kind]
        proc, boot = self.s.launch(argv, kind, name, env)
        sf = Surface(name, kind, f'native-{kind}', proc, boot.get('pid'))
        self.surfaces[name] = sf
        if kind == 'settings':
            sf.page = page or 'home'
            self.s.settings_title(sf.page)
        if kind == 'explorer':
            for _ in range(tabs - 1):
                self.new_tab(name)
        self._step('open', args, {**sf.describe(), 'boot': boot})
        return sf

    def close(self, name: str) -> None:
        sf = self.surfaces.get(name)
        if name == 'start':
            if sf and sf.alive():
                self.s.start_flyout(False, sf.pid)
            self._step('close', {'name': name})
            return
        if name in ('panel', 'desktop'):
            raise ValueError(f'{name} is resident; use kill_panel() for the fallback bar')
        if not sf:
            return
        if sf.proc and sf.proc.poll() is None:
            self.s.close(sf.proc, name)
        if sf.alive():
            # the app is not the process the helper started (a launch handed to a running app)
            self.s.ev(f'global.get_window_actors().map(a=>a.meta_window).filter(w=>w.get_pid()==={sf.pid})'
                      '.forEach(w=>w.delete(global.get_current_time()))||true')
            try:
                self.until(lambda: not sf.alive(), f'{name} ends after its windows closed', 10, 0.2)
            except TimeoutError:
                self.hm.stop_pids([sf.pid] + self.hm.descendants(sf.pid), f'close {name}')
        sf.dead = True
        self.surfaces.pop(name, None)
        self._step('close', {'name': name})

    def _tab_count(self, name: str) -> int:
        """Explorer's tabs: the tab-<n> buttons of its tab strip (each has a tab-<n>-close beside it)."""
        strip = self.find(name, 'explorer-tabs')
        return len({n['name'] for n in self.hm.tree_nodes([{'tree': strip[0]}])
                    if re.fullmatch(r'tab-\d+', str(n.get('name')))}) if strip else 0

    def new_tab(self, name: str = 'explorer') -> int:
        """Open one more Explorer tab with the real "Nieuw tabblad" button; returns the new tab count."""
        before = self._tab_count(name)
        how = self.click(name, 'explorer-new-tab')
        try:
            self.until(lambda: self._tab_count(name) > before, f'{name}: tab {before + 1}', 4)
        except TimeoutError:
            how = {**self.click(name, 'explorer-new-tab', real=False), 'fallback': 'dev'}
            self.until(lambda: self._tab_count(name) > before, f'{name}: tab {before + 1} (dev click)', 6)
        count = self._tab_count(name)
        self._step('new_tab', {'name': name}, {'tabs': count, 'input': how})
        return count

    # ---- evidence ----------------------------------------------------------------------------------------
    def _rect(self, surface: str):
        if surface == 'panel':
            return (0, self.hm.HEIGHT - self.hm.TASKBAR, self.hm.WIDTH, self.hm.TASKBAR)
        if surface == 'desktop':
            return (0, 0, self.hm.WIDTH, self.hm.HEIGHT - self.hm.TASKBAR)
        w = self._meta_window(self.surface(surface))
        return (w['x'], w['y'], w['w'], w['h']) if w else None

    def screenshot(self, name: str, surface: str | None = None, area=None, settle: float = 0.5) -> str:
        """PNG of the whole private screen, of one surface's window, or of area=(x, y, w, h)."""
        base, n = name, 2
        while name in self._shot_names:
            name, n = f'{base}-{n}', n + 1
        self._shot_names.add(name)
        if settle:
            time.sleep(settle)
        rect = area or (self._rect(surface) if surface else None)
        shot = self.s.screenshot(name, 'scenario', area=rect)
        out = {'png': shot['file'], 'area': rect, 'pixels': self.s.pixel_stats(shot['file'])}
        self.results['shots'].append(shot['file'])
        self._step('screenshot', {'name': name, **({'surface': surface} if surface else {})}, out)
        return shot['file']

    def _hits(self, *texts: str) -> list[str]:
        words = set()
        for text in texts:
            words |= set(re.findall(r"[a-zà-ÿ]+(?:-[a-zà-ÿ]+)*", (text or '').lower()))
        return sorted(w for w in words if w in self.s.dutch)

    def text(self, surface: str) -> dict:
        """page_text without the Dutch-word hits and without a step in results.json (for polling)."""
        sf = self.surface(surface)
        tops = self.state(surface)
        shown = [t for t in tops if t.get('visible', True)]
        labels = [str(n['label']) for n in self.hm.tree_nodes(shown) if n.get('label') and n.get('visible', True)]
        return {'lang': None if sf.shell else self.hm.app_lang(shown, sf.kind),
                'title': next((t.get('title') for t in shown if t.get('title')), None),
                'titles': [t.get('title') for t in shown], 'text': '\n'.join(labels)}

    def page_text(self, surface: str) -> dict:
        """The surface's language (an app's: harness app_lang), window title(s), the visible labels and tooltips of
        its dev state as one text, and Dutch-word hits (tests/i18n/dutch-words.txt)."""
        value = self.text(surface)
        value['dutchWordHits'] = self._hits(value['text'])
        self._step('page_text', {'surface': surface},
                   {k: value[k] for k in ('lang', 'title', 'dutchWordHits')} | {'chars': len(value['text'])}, text=value)
        return value

    def wait_text(self, surface: str, text: str, timeout: float = 20, required: bool = False) -> dict | None:
        """Wait until ``text`` occurs in the surface's labels, tooltips or window titles."""
        found = {}

        def probe():
            value = self.text(surface)
            if text in '\n'.join([value['text'], *[t or '' for t in value['titles']]]):
                found.update(value)
                return True
            return False
        started = time.monotonic()
        try:
            self.until(probe, f'{surface}: text {text!r}', timeout, 0.3)
            ok = True
        except TimeoutError:
            ok = False
        self.check(f'wait_text {surface}: {text!r}', ok, seconds=round(time.monotonic() - started, 2))
        if required and not ok:
            raise TimeoutError(f'{surface}: {text!r} did not appear within {timeout} s')
        return found or None

    def wait_lang(self, surface: str, lang: str, timeout: float = 20) -> float | None:
        """Seconds until the app shows ``lang`` (harness app_lang; None on timeout)."""
        started = time.monotonic()
        try:
            self.until(lambda: self.text(surface)['lang'] == lang, f'{surface} lang={lang}', timeout, 0.2)
            return round(time.monotonic() - started, 2)
        except TimeoutError:
            return None

    # ---- GSettings (settings parity) ----------------------------------------------------------------------
    def _keyfile(self) -> Path:
        return self.hm.RUN / 'config' / 'glib-2.0' / 'settings' / 'keyfile'

    def _group(self, schema: str, path: str | None) -> str:
        """The keyfile group of a schema's settings: its path without the outer slashes (a relocatable schema needs
        ``path``)."""
        if path is None:
            path = schema_path(schema)
            if path is None:
                raise ValueError(f'{schema} is relocatable or not installed: give its path')
        return path.strip('/')

    def gsettings_get(self, schema: str, key: str, path: str | None = None) -> str | None:
        """The GVariant text the private keyfile holds for the key, None for GNOME's default (never dconf)."""
        group = self._group(schema, path)
        value = None
        current = None
        try:
            lines = self._keyfile().read_text(encoding='utf-8').splitlines()
        except OSError:
            lines = []
        for line in lines:
            if line.startswith('[') and line.rstrip().endswith(']'):
                current = line.strip()[1:-1]
            elif current == group and '=' in line and line.split('=', 1)[0].strip() == key:
                value = line.split('=', 1)[1].strip()
        self._step('gsettings_get', {'schema': schema, 'key': key, 'path': path}, value)
        return value

    def gsettings_set(self, schema: str, key: str, value: str, path: str | None = None) -> None:
        """Write the key into the private keyfile the way another program would (GVariant text, e.g. 'true',
        "'flat'", '0.5'); GNOME's keyfile backend notices the change and every client hears it (gsettings.changed).
        The file is rewritten in place, so the backend's file monitor sees one change."""
        group = self._group(schema, path)
        kf = self._keyfile()
        try:
            lines = kf.read_text(encoding='utf-8').splitlines()
        except OSError:
            lines = []
        out, current, done, seen_group = [], None, False, False
        for line in lines:
            if line.startswith('[') and line.rstrip().endswith(']'):
                if current == group and not done:
                    blanks = 0
                    while out and not out[-1].strip():
                        out.pop()
                        blanks += 1
                    out.append(f'{key}={value}')
                    out += [''] * blanks
                    done = True
                current = line.strip()[1:-1]
                seen_group |= current == group
            elif current == group and '=' in line and line.split('=', 1)[0].strip() == key:
                if not done:
                    out.append(f'{key}={value}')
                    done = True
                continue
            out.append(line)
        if not done:
            if not seen_group:
                out += ['', f'[{group}]']
            out.append(f'{key}={value}')
        with open(kf, 'r+' if kf.exists() else 'w', encoding='utf-8') as fh:
            fh.seek(0)
            fh.write('\n'.join(out) + '\n')
            fh.truncate()
        self._step('gsettings_set', {'schema': schema, 'key': key, 'path': path, 'value': value})

    def settings_data_page(self, parent: str, item: str, title: str | None = None, timeout: float = 20) -> dict:
        """Open a data page (registry.rs DATA_PAGES: a network, printer, app or device) by clicking ``item`` (a widget
        name) on its parent page, as a user does: its title is the item's name. Waits for ``title`` when given."""
        self.open(f'settings:{parent}')
        self.until(lambda: bool(self.find('settings', item)), f'settings: {item} on {parent}', timeout, 0.3)
        result = self.click('settings', item)
        if title:
            self.wait_text('settings', title, timeout=timeout)
        self._step('settings_data_page', {'parent': parent, 'item': item, 'title': title}, result)
        return result

    # ---- language ----------------------------------------------------------------------------------------
    def settings_file(self) -> dict:
        path = self.hm.RUN / 'config' / 'win11-gnome' / 'settings.json'
        try:
            st = path.stat()
            data = json.loads(path.read_text() or '{}')
            return {'language': data.get('language'), 'inode': st.st_ino, 'mtime': st.st_mtime, 'keys': len(data)}
        except (OSError, ValueError) as error:
            return {'language': None, 'error': str(error)}

    def set_language(self, lang: str, via: str = 'rename', skip=(), timeout: float = 20) -> dict:
        """Switch Win11's language and time each open app until it shows it (harness app_lang).

        via='dropdown': the real Settings > Tijd en taal > Taal en regio drop-down (opens Settings on that page if
                        needed; real pointer clicks on the button and the option, a dev click on the button as
                        recorded fallback). The Settings process then writes settings.json itself (settings.set:
                        invalidation datagram + rename).
        via='rename':   the harness writes the private settings.json as the store does (lock, temp file, rename).
        via='rewrite':  the harness rewrites settings.json in place (same inode, O_TRUNC + one write).
        """
        if lang not in LANG_OPTION or via not in VIAS:
            raise ValueError(f'set_language({lang!r}, via={via!r}): lang nl|en, via {"|".join(VIAS)}')
        before = self.settings_file()
        detail: dict = {}
        if via == 'dropdown':
            detail = self._dropdown(lang)
            t0 = detail.pop('t0')
        else:
            t0 = time.monotonic()
            if via == 'rename':
                self.hm.write_settings({'language': lang})
            else:
                self.hm.rewrite_settings_in_place({'language': lang})
        # the native shell has no language marker (it follows settings.json itself): only the apps are timed
        watch = [n for n, sf in self.surfaces.items() if sf.alive() and not sf.shell and n not in skip]
        timings: dict = {}
        pending = list(watch)
        deadline = t0 + timeout
        while pending and time.monotonic() < deadline:
            for name in list(pending):
                try:
                    if self.text(name)['lang'] == lang:
                        timings[name] = round(time.monotonic() - t0, 2)
                        pending.remove(name)
                except Exception:   # noqa: BLE001 - keep polling the others
                    pass
            time.sleep(0.1)
        for name in pending:
            timings[name] = None
        self.s.lang = lang
        settings = self.surfaces.get('settings')
        if settings and settings.alive() and settings.page and 'settings' not in skip:
            try:
                self.s.settings_title(settings.page, lang)
                timings['settingsTitle'] = round(time.monotonic() - t0, 2)
            except Exception as error:   # noqa: BLE001
                timings['settingsTitle'] = f'not switched: {error}'
        after = self.settings_file()
        result = {'lang': lang, 'via': via, **detail, 'seconds': timings, 'fileBefore': before, 'fileAfter': after,
                  'fileLanguageOk': after.get('language') == lang,
                  'sameInode': before.get('inode') == after.get('inode')}
        self._step('set_language', {'lang': lang, 'via': via, **({'skip': list(skip)} if skip else {})}, result)
        return result

    def _dropdown(self, lang: str) -> dict:
        """The display language drop-down (widget language-display): a pointer click opens its list (a popover of
        w11-list-item buttons), a second one on the option chooses it. The options have no widget name, so only the
        pointer reaches them; a dev click opens the list as the recorded fallback."""
        self.open('settings:language')

        def option():
            return next((n for n in self.find('settings', cls='w11-list-item')
                         if any(str(k.get('label') or '').startswith(LANG_OPTION[lang])
                                for k in self.hm.tree_nodes([{'tree': n}]))), None)
        path, clicks = 'clutter', []
        try:
            clicks.append(self.click('settings', 'language-display'))
            self.until(option, 'language drop-down open (pointer)', 3)
        except Exception as error:   # noqa: BLE001
            path = f'dev (pointer did not open the list: {error})'
            if not option():
                clicks.append(self.click('settings', 'language-display', real=False))
                self.until(option, 'language drop-down open (dev click)', 5)
        t0 = time.monotonic()
        clicks.append(self.click('settings', node=option()))
        try:
            self.until(lambda: self.settings_file().get('language') == lang, 'settings.json written by Settings', 5)
        except TimeoutError:
            # Recorded retry: the list once more, the option once more (both with the pointer).
            path += ' + retry'
            if not option():
                clicks.append(self.click('settings', 'language-display'))
                self.until(option, 'language drop-down open (retry)', 5)
            t0 = time.monotonic()
            clicks.append(self.click('settings', node=option()))
            self.until(lambda: self.settings_file().get('language') == lang, 'settings.json written by Settings', 8)
        return {'t0': t0, 'input': path, 'clicks': clicks}

    # ---- GNOME side --------------------------------------------------------------------------------------
    def window_titles(self) -> list[dict]:
        """Native (Meta) window titles, as GNOME's overview, Alt+Tab and the 'is ready' banners see them."""
        value = self.s.ev(WINDOW_TITLES_JS)
        self._step('window_titles', None, value)
        return value

    def app_names(self, ids=None) -> dict:
        """Shell.AppSystem lookup_app(id).get_name() (the .desktop Name= GNOME shows); default: the 9 own entries."""
        ids = list(ids or OWN_DESKTOP_IDS)
        value = self.s.ev(APP_NAMES_JS.format(ids=json.dumps(ids)))
        self._step('app_names', {'ids': len(ids)}, value)
        return value

    def extension_labels(self) -> dict:
        """The extension's fallback taskbar (labels, accessible names), the clock text and its Ctrl+Alt+Tab group."""
        value = self.s.ev(EXTENSION_LABELS_JS.format(uuid=self.hm.UUID))
        if isinstance(value, dict) and 'labels' in value:
            value['dutchWordHits'] = self._hits(*value['labels'], *value['accessibleNames'], *value['ctrlAltTab'])
        self._step('extension_labels', None, value)
        return value

    def notifications(self) -> list:
        value = self.s.notifications()
        self._step('notifications', None, value)
        return value

    # ---- more than one monitor (--second-monitor) ---------------------------------------------------------
    def monitors(self) -> list[dict]:
        """Main.layoutManager.monitors: index, stage rectangle, primary, in fullscreen."""
        value = self.s.ev(MONITORS_JS) or []
        self._step('monitors', None, value)
        return value

    def strip_windows(self) -> list[dict]:
        """The taskbar windows of the other monitors as Mutter has them: app id, type, frame, monitor, mapped."""
        value = self.s.ev(STRIP_WINDOWS_JS) or []
        self._step('strip_windows', None, value)
        return value

    def work_area(self, index: int) -> dict:
        """The active workspace's work area of monitor `index` (the monitor minus the struts on it)."""
        value = self.s.ev(WORK_AREA_JS % int(index))
        self._step('work_area', {'monitor': index}, value)
        return value

    def pixel(self, file: str, x: int, y: int) -> tuple[int, int, int] | None:
        """The colour at (x, y) of a screenshot this run took (a file name from screenshot())."""
        pix = self.s.GdkPixbuf.Pixbuf.new_from_file(str(self.hm.RUN / 'shots' / file))
        if not (0 <= x < pix.get_width() and 0 <= y < pix.get_height()):
            return None
        data, stride, channels = pix.get_pixels(), pix.get_rowstride(), pix.get_n_channels()
        i = y * stride + x * channels
        return (data[i], data[i + 1], data[i + 2])

    def display_config(self, primary: int | None = None, disable: int | None = None, enable_all: bool = False,
                       order: list[int] | None = None) -> dict:
        """Best effort: a temporary monitor configuration through org.gnome.Mutter.DisplayConfig on the private bus
        (GetCurrentState, then ApplyMonitorsConfig with method 1, temporary): `primary` makes that monitor (a
        Main.layoutManager index) the primary one, `disable` turns that monitor off, `enable_all` turns every monitor
        Mutter knows on again, left to right, `order` places the monitors (Main.layoutManager indices) left to right
        in that order. Mutter may refuse it for headless virtual monitors: the answer then has `error`."""
        GLib, Gio = self.s.GLib, self.s.Gio
        args = {'primary': primary, 'disable': disable, 'enableAll': enable_all, **({'order': order} if order else {})}
        try:
            reply = self.s.bus.call_sync('org.gnome.Mutter.DisplayConfig', '/org/gnome/Mutter/DisplayConfig',
                                         'org.gnome.Mutter.DisplayConfig', 'GetCurrentState', None, None,
                                         Gio.DBusCallFlags.NONE, 10000, None)
            serial, monitors, logical, _props = reply.unpack()
            shell = self.s.ev(MONITORS_JS) or []
            # a layout index -> the connectors of the logical monitor at its place
            by_place = {(int(lm[0]), int(lm[1])): [spec[0] for spec in lm[5]] for lm in logical}
            index_conn = {m['index']: (by_place.get((m['x'], m['y'])) or [None])[0] for m in shell}
            scale = {spec[0]: float(lm[2]) for lm in logical for spec in lm[5]}
            current_primary = next((spec[0] for lm in logical if lm[4] for spec in lm[5]), None)
            modes = {}
            for spec, mode_list, _mprops in monitors:
                mode = next((m for m in mode_list if m[6].get('is-current')), None) or \
                    next((m for m in mode_list if m[6].get('is-preferred')), None) or (mode_list[0] if mode_list else None)
                if mode:
                    modes[spec[0]] = mode
            enabled = [spec[0] for lm in logical for spec in lm[5]]
            if enable_all:
                enabled = [spec[0] for spec, _modes, _p in monitors if spec[0] in modes]
            if disable is not None:
                enabled = [c for c in enabled if c != index_conn.get(disable)]
            if order:
                rank = {index_conn.get(index): n for n, index in enumerate(order)}
                enabled.sort(key=lambda c: rank.get(c, len(rank)))
            want_primary = index_conn.get(primary) if primary is not None else current_primary
            if want_primary not in enabled:
                want_primary = enabled[0] if enabled else None
            config, x = [], 0
            for connector in enabled:
                mode = modes[connector]
                s = scale.get(connector, 1.0)
                config.append((x, 0, s, 0, connector == want_primary, [(connector, mode[0], {})]))
                x += int(round(mode[1] / s))
            variant = GLib.Variant('(uua(iiduba(ssa{sv}))a{sv})', (serial, 1, config, {}))
            self.s.bus.call_sync('org.gnome.Mutter.DisplayConfig', '/org/gnome/Mutter/DisplayConfig',
                                 'org.gnome.Mutter.DisplayConfig', 'ApplyMonitorsConfig', variant, None,
                                 Gio.DBusCallFlags.NONE, 10000, None)
            value = {'ok': True, 'config': [[c[0], c[1], c[4], c[5][0][0], c[5][0][1]] for c in config]}
        except Exception as error:   # noqa: BLE001 - recorded; the scenario marks the step known
            value = {'ok': False, 'error': str(error)}
        self._step('display_config', args, value)
        return value

    def kill_panel(self) -> dict:
        """Stop the private win11-shell (the native taskbar, Start and desktop; its backend, win11-panel --http-only,
        dies with it through PR_SET_PDEATHSIG) so the extension shows its fallback St taskbar; returns the bar's
        labels and a screenshot of the bottom strip."""
        panel = next((w for w in self.s.windows() if w['wm'] == WM['panel']), None)
        if not panel:
            raise RuntimeError('no Panel window (win11-shell) in the private shell')
        pid = panel['pid']
        own = self.hm.descendants(os.getpid())
        if pid not in own:
            raise RuntimeError(f'win11-shell pid {pid} is not a descendant of this harness; not signalling it')
        before = self.extension_labels()
        t0 = time.monotonic()
        self.hm.stop_pids([pid] + self.hm.descendants(pid), 'kill_panel', grace=5)
        for name in RESIDENT:
            if name in self.surfaces:
                self.surfaces[name].dead = True
        try:
            self.until(lambda: (lambda v: isinstance(v, dict) and v.get('barVisible') and not v.get('nativePanelActive'))(
                self.s.ev(EXTENSION_LABELS_JS.format(uuid=self.hm.UUID))), 'extension fallback bar visible', 15, 0.3)
            shown = round(time.monotonic() - t0, 2)
        except TimeoutError:
            shown = None
        shot = self.screenshot('fallback-bar', area=(0, self.hm.HEIGHT - self.hm.TASKBAR, self.hm.WIDTH, self.hm.TASKBAR),
                               settle=1.0)
        after = self.extension_labels()
        result = {'pid': pid, 'fallbackShownSeconds': shown, 'png': shot, 'before': before, 'after': after}
        self._step('kill_panel', None, {'pid': pid, 'fallbackShownSeconds': shown, 'png': shot})
        return result

    def restart_panel(self, timeout: float = 90) -> dict:
        """Start the taskbar again through its .desktop entry (as the extension does): win11-panel execs win11-shell;
        waits for the shell's new boot report and its Panel window, then panel, desktop and Start are surfaces again."""
        known = len(self.s.rows())
        ok = self.s.ev(PANEL_OPEN_JS)
        boot = self.s.native_boot(known, timeout)
        pid = boot.get('pid')
        try:
            self.until(lambda: self.s.window_of(WM['panel'], pid), 'native Panel window mapped', 30, 0.2)
            mapped = True
        except TimeoutError:
            mapped = False
        self._add_native(pid)
        self.s.facts['windowIds'] = {'native': pid}
        result = {'launched': ok, 'pid': pid, 'boot': boot, 'panelMapped': mapped}
        self._step('restart_panel', None, result)
        return result


_SCHEMA_PATHS: dict[str, str | None] = {}


def schema_path(schema: str) -> str | None:
    """A schema's own path from the installed schema files (None: relocatable or missing); read-only, no gsettings."""
    if schema in _SCHEMA_PATHS:
        return _SCHEMA_PATHS[schema]
    import xml.etree.ElementTree as ET  # noqa: PLC0415
    found = None
    for xml in sorted(Path('/usr/share/glib-2.0/schemas').glob('*.gschema.xml')):
        try:
            for node in ET.parse(xml).getroot().iter('schema'):
                if node.get('id') == schema:
                    found = node.get('path')
                    break
        except ET.ParseError:
            continue
        if found is not None:
            break
    _SCHEMA_PATHS[schema] = found
    return found


def load_module(path: Path, name: str):
    """Import a Python file by path under a unique module name (the harness runs with python -I)."""
    import importlib.util
    import sys
    spec = importlib.util.spec_from_file_location(name, path)
    if not spec or not spec.loader:
        raise ImportError(f'cannot load {path}')
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


def run_scenario(session, hm, path: Path) -> dict:
    """Load ``path``, call its ``run(h)``; record an exception (with a screenshot) instead of raising."""
    h = Helper(session, hm, path)
    hm.log(f'scenario {path}')
    try:
        module = load_module(path, 'ge1h_scenario_' + ''.join(c if c.isalnum() else '_' for c in path.stem))
        if not callable(getattr(module, 'run', None)):
            raise TypeError(f'{path} has no run(h)')
        module.run(h)
    except Exception as error:   # noqa: BLE001 - the harness still shuts down and writes results
        h.results['error'] = f'{type(error).__name__}: {error}'
        h.results['traceback'] = traceback.format_exc()
        hm.log(f'  scenario FAILED: {h.results["error"]}')
        try:
            h.screenshot('scenario-FAILED', settle=0)
        except Exception:   # noqa: BLE001
            pass
    h.results['failedChecks'] = [c['check'] for c in h.results['checks'] if not c['ok']]
    h.results['failedChecksUnknown'] = [c['check'] for c in h.results['checks'] if not c['ok'] and not c.get('known')]
    h._flush()
    return redact_api(h.results, h._secret_methods)
