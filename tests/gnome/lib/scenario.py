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
LANG_OPTION = {'nl': 'Nederlands', 'en': 'English'}
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

PANEL_OPEN_JS = ("(async()=>{const Shell=(await import('gi://Shell')).default;"
                 "const app=Shell.AppSystem.get_default().lookup_app('io.github.win11shell.Panel.desktop');"
                 "if(!app)return false;app.open_new_window(-1);return true;})()")


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
    Recording   record(key, value), check(name, ok, known=None, **detail), require(name, ok, **detail), log(msg)
    Low level   eval(js) (Shell Eval), until(fn, label, timeout), sleep(seconds), session (the harness Session),
                hm (the harness module)
    """

    def __init__(self, session, hm, scenario: Path):
        self.session, self.s, self.hm = session, session, hm
        self.scenario = scenario
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
            self._results_file.write_text(json.dumps(self.results, indent=1, ensure_ascii=False) + '\n')
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
        frame = self._frame(sf, tops[node.get('top', 0)]) if real and node.get('rect') and tops else None
        x, y, w, hh = node.get('rect') or (0, 0, 0, 0)
        # a widget scrolled out of its window has a rect outside it: the pointer would land on another window
        outside = bool(frame) and not (0 <= x + w // 2 < frame['w'] and 0 <= y + hh // 2 < frame['h'])
        if not frame or outside:
            if not name:
                raise LookupError(f'{surface}: the widget has no name for a dev click and no place for the pointer')
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
            if kind == 'settings' and page and page != sf.page:
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
    return h.results
