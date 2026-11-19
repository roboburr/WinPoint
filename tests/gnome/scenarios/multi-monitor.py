"""Taskbars and desktop on two monitors (16-11, Robin: "hoofdscherm met hoofdtaakbalk. en tweede scherm met taakbalk
met alleen de draaiende app op dat scherm").

Run (from G), with a second virtual monitor right of the 1440 x 900 primary (4:3 next to 16:10 and taller, so part of
the bounding box lies outside every monitor; smaller by area, so Mutter keeps the first one primary):
    tools/heavy.sh --min-mb 2560 --tag mm -- python3 -B tests/gnome/harness.py launch run --lang nl --source worktree \\
        --second-monitor 1280x960 --scenario tests/gnome/scenarios/multi-monitor.py \\
        --out tests/gnome/scenarios/multi-monitor.out
and the same with --lang en --out tests/gnome/scenarios/multi-monitor.en.out.

What it checks (every check is language-independent; the screenshots carry the language):
  1. two monitors, the first one primary; the panel window covers monitor 1 (the primary), the taskbar of monitor 2
     (win11-shell's io.github.win11shell.SecondaryTaskbar.Monitor1) is a dock at its bottom, 48 px high and as wide
     as the monitor, frosted, with a strut (monitor 2's work area is 48 px shorter); its strip (tb-strip-m1) has only
     app buttons: no Start, search, Task View, tray or clock
  2. the desktop: a desktop window stays in one monitor's work area (Mutter), so the desktop with the icons covers
     monitor 1 and monitor 2 has a desktop window of its own (io.github.win11shell.SecondaryDesktop.Monitor1);
     each shows the whole four-colour test picture by itself (fill); with "Uitspreiden" (span) one picture covers
     both, each monitor showing its own part; the icons stay on monitor 1 above its taskbar, a right click on the
     empty desktop of monitor 2 opens the desktop menu there, and its Nieuw > Map makes the folder on monitor 1 with
     the keyboard there: the name typed right after (real keys) is its name
  3. Task Manager (not pinned) moved to monitor 2 shows on its taskbar only (the default "Taakbalk waar het venster
     is geopend"); Settings there too: the main taskbar keeps its Settings pin, not running
  4. "Alle taakbalken": on both, the pins on both; "Hoofdtaakbalk en taakbalk waar het venster is geopend": on both,
     no pins on monitor 2; back to the default
  5. moved to monitor 1 it moves to the main taskbar, and back
  6. real clicks on its button on monitor 2's taskbar: focus, minimize (into that button: its icon geometry lies in
     that taskbar), restore
  7. a right click on the button: the jump list as a popover above it; on empty space: the taskbar menu; no window
     list opens over either while the pointer rests on an app button
  8. Settings > Personalisatie > Taakbalk: both rows enabled with two displays; "Mijn taakbalk weergeven op alle
     beeldschermen" off (a click on the real switch) removes the taskbar of monitor 2 and its strut, the main taskbar
     shows Task Manager again and the second row is greyed out; on again: back
  9. the lock screen's session-mode path (the extension disabled and enabled): the taskbar of monitor 2 comes back,
     and both taskbars send their button rectangles again (windows.setIconRects)
  9b. the language switched and back (settings.json): the taskbar and desktop windows of monitor 2 take their new
     titles, as the desktop of monitor 1 does
  10. best effort (Mutter may refuse ApplyMonitorsConfig for headless virtual monitors; known 'headless-displayconfig'):
     monitor 2 made the primary (the panel, the desktop and the icons move there, the other taskbar and desktop
     window are Monitor0's on monitor 1), back; monitor 2 placed left of monitor 1 (both taskbars move without
     changing size: the windows still minimize into their buttons there), back; monitor 2 switched off (no other
     taskbar or desktop window, the desktop is monitor 1, Settings greys both rows out) and on again (the rows
     enabled)
"""
import json
import sys
import time

STRIP = 'io.github.win11shell.SecondaryTaskbar.Monitor'
EXT = 'win11-gnome@win11shell.local'
BAR = 48
KNOWN_DISPLAYCONFIG = 'headless-displayconfig'

SECONDARY_DESKTOP = 'io.github.win11shell.SecondaryDesktop.Monitor'
BRIDGE_JS = ("(()=>{const e=Main.extensionManager.lookup('" + EXT + "');const b=e&&e.stateObj&&e.stateObj._panelBridge;"
             "if(!b)return null;const r=w=>{const f=w.get_frame_rect();return [f.x,f.y,f.width,f.height];};"
             "const a=s=>s?[Math.round(s.x),Math.round(s.y),Math.round(s.width),Math.round(s.height)]:null;"
             "return {panel:b._panel?r(b._panel):null,present:b._panelPresent,"
             "strips:[...b._strips].map(([w,x])=>({wm:w.get_wm_class(),monitor:x.monitor,inhibited:x.inhibited,"
             "rect:r(w),strut:a(x.strut)})),iconSources:[...(b._iconSources?.keys()??[])].map(String),"
             "frosted:(b._frostedBridge?.status().entries||[]).map(x=>({kind:x.kind,rect:x.rect,effect:x.effect}))};})()")
# The Meta.Window of a process: frame, monitor, state, icon geometry (where it minimizes to)
WINDOW_JS = ("(()=>{const w=global.get_window_actors().map(a=>a.meta_window).find(w=>w.get_pid()===__PID__"
             "&&w.get_window_type()===Meta.WindowType.NORMAL);if(!w)return null;const f=w.get_frame_rect();"
             "const [ok,g]=w.get_icon_geometry();return {rect:[f.x,f.y,f.width,f.height],monitor:w.get_monitor(),"
             "minimized:w.minimized,focus:w.has_focus(),icon:ok&&g?[g.x,g.y,g.width,g.height]:null};})()")
MOVE_JS = ("(()=>{const w=global.get_window_actors().map(a=>a.meta_window).find(w=>w.get_pid()===__PID__"
           "&&w.get_window_type()===Meta.WindowType.NORMAL);if(!w)return false;w.unminimize();"
           "if(w.is_maximized())w.unmaximize();w.move_to_monitor(__MON__);Main.activateWindow(w);return w.get_monitor();})()")
FOCUS_JS = ("(()=>{const w=global.get_window_actors().map(a=>a.meta_window).find(w=>w.get_pid()===__PID__"
            "&&w.get_window_type()===Meta.WindowType.NORMAL);if(!w)return false;w.unminimize();"
            "Main.activateWindow(w);return true;})()")
# steps: ["move", x, y] | ["down", button] | ["up", button] | ["wait", ms]; 30 ms between steps (kit-ui.py)
POINTER_JS = (
    "(async()=>{const Clutter=(await import('gi://Clutter')).default;const GLib=(await import('gi://GLib')).default;"
    "const wait=ms=>new Promise(r=>GLib.timeout_add(GLib.PRIORITY_DEFAULT,ms,()=>{r();return GLib.SOURCE_REMOVE;}));"
    "const seat=Clutter.get_default_backend().get_default_seat();"
    "globalThis.__ge1hPointer??=seat.create_virtual_device(Clutter.InputDeviceType.POINTER_DEVICE);"
    "const d=globalThis.__ge1hPointer,us=()=>GLib.get_monotonic_time();"
    "for(const s of __STEPS__){if(s[0]==='move')d.notify_absolute_motion(us(),s[1],s[2]);"
    "else if(s[0]==='down')d.notify_button(us(),s[1],Clutter.ButtonState.PRESSED);"
    "else if(s[0]==='up')d.notify_button(us(),s[1],Clutter.ButtonState.RELEASED);"
    "await wait(s[0]==='wait'?s[1]:30);}"
    "return global.get_pointer().slice(0,2);})()")
SCROLL_JS = (
    "(async()=>{const Clutter=(await import('gi://Clutter')).default;const GLib=(await import('gi://GLib')).default;"
    "const wait=ms=>new Promise(r=>GLib.timeout_add(GLib.PRIORITY_DEFAULT,ms,()=>{r();return GLib.SOURCE_REMOVE;}));"
    "const seat=Clutter.get_default_backend().get_default_seat();"
    "globalThis.__ge1hPointer??=seat.create_virtual_device(Clutter.InputDeviceType.POINTER_DEVICE);"
    "const d=globalThis.__ge1hPointer,us=()=>GLib.get_monotonic_time();d.notify_absolute_motion(us(),__X__,__Y__);"
    "await wait(80);for(let i=0;i<__N__;i++){d.notify_discrete_scroll(us(),Clutter.ScrollDirection.DOWN,"
    "Clutter.ScrollSource.WHEEL);await wait(40);}return true;})()")
KEY_JS = (
    "(async()=>{const Clutter=(await import('gi://Clutter')).default;const GLib=(await import('gi://GLib')).default;"
    "const seat=Clutter.get_default_backend().get_default_seat();"
    "globalThis.__mmKeyboard??=seat.create_virtual_device(Clutter.InputDeviceType.KEYBOARD_DEVICE);"
    "const d=globalThis.__mmKeyboard,us=()=>GLib.get_monotonic_time();"
    "d.notify_keyval(us(),Clutter.KEY___KEY__,Clutter.KeyState.PRESSED);"
    "await new Promise(r=>GLib.timeout_add(GLib.PRIORITY_DEFAULT,40,()=>{r();return GLib.SOURCE_REMOVE;}));"
    "d.notify_keyval(us(),Clutter.KEY___KEY__,Clutter.KeyState.RELEASED);return true;})()")
# made once at the start: a key sent right after its device was created can be lost (as the pointer's first click)
KEYBOARD_PREPARE_JS = (
    "(async()=>{const Clutter=(await import('gi://Clutter')).default;"
    "const seat=Clutter.get_default_backend().get_default_seat();"
    "globalThis.__mmKeyboard??=seat.create_virtual_device(Clutter.InputDeviceType.KEYBOARD_DEVICE);return true;})()")
FOCUS_WM_JS = "global.display.focus_window?.get_wm_class() ?? null"
# the popup surfaces (menus) of a process as Mutter has them, the newest last: their whole surface (buffer rectangle)
POPUPS_JS = ("(()=>{const t=[Meta.WindowType.DROPDOWN_MENU,Meta.WindowType.POPUP_MENU,Meta.WindowType.MENU];"
             "return global.get_window_actors().map(a=>a.meta_window).filter(w=>w.get_pid()===__PID__&&"
             "t.includes(w.get_window_type())).sort((a,b)=>a.get_stable_sequence()-b.get_stable_sequence()).map(w=>{"
             "const b=w.get_buffer_rect();return [b.x,b.y,b.width,b.height];});})()")
STATE_JS = ("(()=>{const e=Main.extensionManager.lookup('" + EXT + "');return {state:e?e.state:null,"
            "mode:Main.sessionMode.currentMode,locked:Main.sessionMode.isLocked};})()")
COLOURS = {'red': (255, 0, 0), 'green': (0, 255, 0), 'blue': (0, 0, 255), 'yellow': (255, 255, 0)}


def lib(h):
    return sys.modules[type(h).__module__]


def wait(h, fn, label, timeout=20.0, interval=0.3):
    """fn() until it is truthy; None after `timeout` (the check that follows records it)."""
    try:
        return h.until(fn, label, timeout, interval)
    except TimeoutError as error:
        h.log(f'timeout: {error}')
        return None


def pointer(h, steps):
    return h.eval(POINTER_JS.replace('__STEPS__', json.dumps(steps)))


def click_at(h, x, y, button=1):
    return pointer(h, [['move', int(x), int(y)], ['wait', 120], ['down', button], ['wait', 60], ['up', button], ['wait', 80]])


def key(h, name):
    return h.eval(KEY_JS.replace('__KEY__', name))


def window(h, pid):
    return h.eval(WINDOW_JS.replace('__PID__', str(int(pid))))


def strips(h):
    return h.eval(lib(h).STRIP_WINDOWS_JS) or []


def strip_of(h, monitor, mons):
    """The taskbar window of `monitor`, a mapped dock at its bottom, or None."""
    m = next((m for m in mons if m['index'] == monitor), None)
    for w in strips(h):
        if (m and w['wm'] == f'{STRIP}{monitor}' and w['type'] == 'DOCK' and w['mapped']
                and (w['x'], w['y'], w['w'], w['h']) == (m['x'], m['y'] + m['h'] - BAR, m['w'], BAR)):
            return w
    return None


def nodes(top):
    return lib_tree(top)


def lib_tree(top):
    out, todo = [], [top.get('tree')]
    while todo:
        node = todo.pop()
        if isinstance(node, dict):
            out.append(node)
            todo.extend(reversed(node.get('children') or []))
    return out


def tops(h):
    return [t for t in h.native_state() if t.get('visible')]


def strip_top(state, monitor):
    """The dev state toplevel of the taskbar of `monitor` (its tree has tb-strip-m<N>)."""
    return next((t for t in state if any(n.get('name') == f'tb-strip-m{monitor}' for n in lib_tree(t))), None)


def main_top(state):
    return next((t for t in state if any(n.get('name') == 'tb-strip' for n in lib_tree(t))), None)


def buttons(top, prefix):
    """The app buttons of a taskbar's tree: {key: classes}."""
    if not top:
        return {}
    start = f'{prefix}-app-'
    return {n['name'][len(start):]: [c for c in n.get('classes') or [] if c.startswith('is-')]
            for n in lib_tree(top) if str(n.get('name') or '').startswith(start)}


# what a button key of an app looks like: its catalogue id (win11shell-taskmgr.desktop) or, without the catalogue,
# its window's app id (win:io.github.win11shell.TaskManager)
NEEDLES = {'taskmgr': ('taskmgr', 'taskmanager'), 'settings': ('settings',), 'explorer': ('explorer',)}


def has(buttons_, needle, running=None):
    keys = [k for k in buttons_ if any(n in k.lower() for n in NEEDLES.get(needle, (needle,)))]
    if not keys:
        return False
    if running is None:
        return True
    return any(('is-running' in buttons_[k]) == running for k in keys)


def taskbars(h):
    state = tops(h)
    return buttons(main_top(state), 'tb'), buttons(strip_top(state, 1), 'tb-m1')


def node_named(top, name):
    return next((n for n in lib_tree(top) if n.get('name') == name), None) if top else None


def close_to(rgb, want, tolerance=48):
    return bool(rgb) and all(abs(a - b) <= tolerance for a, b in zip(rgb, want))


def colour_name(rgb):
    return next((name for name, want in COLOURS.items() if close_to(rgb, want)), None)


def quad_png(h, path):
    """A 1600 x 1000 picture in four colours: red top left, green top right, blue bottom left, yellow bottom right."""
    GdkPixbuf = h.s.GdkPixbuf
    pix = GdkPixbuf.Pixbuf.new(GdkPixbuf.Colorspace.RGB, False, 8, 1600, 1000)
    for (x, y), colour in (((0, 0), 0xff0000ff), ((800, 0), 0x00ff00ff), ((0, 500), 0x0000ffff), ((800, 500), 0xffff00ff)):
        pix.new_subpixbuf(x, y, 800, 500).fill(colour)
    pix.savev(str(path), 'png', [], [])
    return str(path)


def settings_set(h, path, value):
    answer = h.backend('settings', 'settings.set', {'path': path, 'value': value})
    h.record(f'settings.set {path}={json.dumps(value)}', answer if isinstance(answer, dict) else {'ok': True})
    return answer


def run(h):
    L = h.lang
    h.record('keyboard', h.eval(KEYBOARD_PREPARE_JS))
    mons = h.monitors()
    h.require('two monitors, the first one primary', len(mons) == 2 and mons[0]['primary'] and not mons[1]['primary'],
              monitors=mons)
    m0, m1 = mons
    h.record('monitors', mons)
    h.check('the second monitor lies right of the first', m1['x'] == m0['x'] + m0['w'] and m1['y'] == m0['y'],
            monitors=mons)

    # ---- 1. the taskbars
    strip = wait(h, lambda: strip_of(h, 1, mons), 'the taskbar of monitor 2 is a dock at its bottom', 45)
    h.record('strip windows', h.strip_windows())
    h.require('the taskbar of monitor 2 is a 48 px dock at its bottom, as wide as the monitor', bool(strip), strip=strip)
    h.check('it is on monitor 2, above the windows, on every workspace and out of the window list',
            strip['monitor'] == 1 and strip['above'] and strip['sticky'] and strip['skipTaskbar'], strip=strip)
    h.check('one taskbar window per other monitor only', len(strips(h)) == 1, strips=strips(h))
    h.sleep(1.5)
    bridge = h.eval(BRIDGE_JS)
    h.record('bridge', bridge)
    h.check('the panel window covers monitor 1 (the primary)',
            bool(bridge) and bridge['panel'] == [m0['x'], m0['y'], m0['w'], m0['h']], bridge=bridge)
    work1, work0 = h.work_area(1), h.work_area(0)
    h.check('monitor 2 has a strut: its work area ends above its taskbar',
            work1 == {'x': m1['x'], 'y': m1['y'], 'w': m1['w'], 'h': m1['h'] - BAR}, work=work1, monitor=m1)
    h.check('monitor 1 keeps its own strut', work0['h'] == m0['h'] - BAR and work0['y'] == m0['y'], work=work0)
    state = tops(h)
    top = strip_top(state, 1)
    h.require('the dev state has the taskbar of monitor 2 (tb-strip-m1)', bool(top),
              toplevels=[[t.get('title'), t.get('width'), t.get('height')] for t in state])
    names = {n.get('name') for n in lib_tree(top)}
    h.check('its window is the monitor wide and 48 px high', (top.get('width'), top.get('height')) == (m1['w'], BAR),
            size=[top.get('width'), top.get('height')])
    h.check('it has no Start, search, Task View, tray or clock',
            not names & {'tb-start', 'tb-search', 'tb-taskview', 'tb-sys', 'tb-clock', 'tb-desktop', 'tb-tray-chevron'},
            names=sorted(n for n in names if n))
    h.check('the main taskbar keeps Start, Task View, the tray and the clock',
            {'tb-start', 'tb-clock', 'tb-sys'} <= {n.get('name') for n in lib_tree(main_top(state))})
    glass = wait(h, lambda: 'w11-glass' in (node_named(strip_top(tops(h), 1), 'tb-strip-m1') or {}).get('classes', []),
                 'the taskbar of monitor 2 frosted', 15)
    h.check('the taskbar of monitor 2 is frosted like the main one (w11-glass, a strip backdrop)',
            bool(glass) and any(e['kind'] == 'strip' and e['effect'] for e in (h.eval(BRIDGE_JS) or {}).get('frosted', [])),
            frosted=(h.eval(BRIDGE_JS) or {}).get('frosted'))
    h.screenshot(f'{L}-both-monitors')
    h.screenshot(f'{L}-monitor1', area=(m0['x'], m0['y'], m0['w'], m0['h']))
    h.screenshot(f'{L}-monitor2', area=(m1['x'], m1['y'], m1['w'], m1['h']))
    h.screenshot(f'{L}-taskbar2', area=(m1['x'], m1['y'] + m1['h'] - BAR, m1['w'], BAR))

    # ---- 2. the desktop (before any app covers it)
    desktop(h, L, mons)

    # ---- 3. an app on monitor 2
    tm = h.open('taskmgr')
    h.record('task manager moved', h.eval(MOVE_JS.replace('__PID__', str(tm.pid)).replace('__MON__', '1')))
    on2 = wait(h, lambda: (lambda w: w and w['monitor'] == 1)(window(h, tm.pid)), 'Task Manager on monitor 2', 10)
    h.check('Task Manager is on monitor 2', bool(on2), window=window(h, tm.pid))

    def only_on_strip():
        main, other = taskbars(h)
        return (main, other) if has(other, 'taskmgr', True) and not has(main, 'taskmgr') else None
    seen = wait(h, only_on_strip, 'Task Manager only on the taskbar of monitor 2', 15)
    main, other = taskbars(h)
    h.check('by default ("Taskbar where window is open") Task Manager shows on the taskbar of monitor 2 only',
            bool(seen), main=sorted(main), other=sorted(other))
    st = h.open('settings:taskbar')
    h.record('settings moved', h.eval(MOVE_JS.replace('__PID__', str(st.pid)).replace('__MON__', '1')))
    seen = wait(h, lambda: (lambda m, o: has(o, 'settings', True) and has(m, 'settings', False))(*taskbars(h)),
                'Settings running on monitor 2, its pin idle on the main taskbar', 15)
    main, other = taskbars(h)
    h.check('Settings on monitor 2: running on its taskbar, the main taskbar keeps its pin without is-running',
            bool(seen), main=main, other=other)
    h.check('the taskbar of monitor 2 shows no pins by default', not has(other, 'explorer'), other=sorted(other))
    h.screenshot(f'{L}-apps-on-monitor2', area=(m1['x'], m1['y'], m1['w'], m1['h']))
    h.screenshot(f'{L}-taskbar1-apps', area=(m0['x'], m0['y'] + m0['h'] - BAR, m0['w'], BAR))
    h.screenshot(f'{L}-taskbar2-apps', area=(m1['x'], m1['y'] + m1['h'] - BAR, m1['w'], BAR))

    # ---- 4. the three modes
    settings_set(h, 'taskbar.multiDisplayApps', 'all')
    seen = wait(h, lambda: (lambda m, o: has(m, 'taskmgr', True) and has(o, 'taskmgr', True) and has(o, 'explorer'))(*taskbars(h)),
                '"All taskbars": everywhere, with the pins', 15)
    main, other = taskbars(h)
    h.check('"All taskbars": Task Manager on both taskbars, the pins on monitor 2 too', bool(seen),
            main=sorted(main), other=sorted(other))
    h.screenshot(f'{L}-taskbar2-all', area=(m1['x'], m1['y'] + m1['h'] - BAR, m1['w'], BAR))
    settings_set(h, 'taskbar.multiDisplayApps', 'mainAndWindow')
    seen = wait(h, lambda: (lambda m, o: has(m, 'taskmgr', True) and has(o, 'taskmgr', True) and not has(o, 'explorer'))(*taskbars(h)),
                '"Main taskbar and taskbar where window is open"', 15)
    main, other = taskbars(h)
    h.check('"Main taskbar and taskbar where window is open": Task Manager on both, no pins on monitor 2', bool(seen),
            main=sorted(main), other=sorted(other))
    settings_set(h, 'taskbar.multiDisplayApps', 'window')
    seen = wait(h, only_on_strip, 'back to "Taskbar where window is open"', 15)
    h.check('back to the default: on the taskbar of monitor 2 only', bool(seen))

    # ---- 5. moving the window between the monitors
    h.eval(MOVE_JS.replace('__PID__', str(tm.pid)).replace('__MON__', '0'))
    seen = wait(h, lambda: (lambda m, o: has(m, 'taskmgr', True) and not has(o, 'taskmgr'))(*taskbars(h)),
                'Task Manager moved to monitor 1: on the main taskbar', 15)
    main, other = taskbars(h)
    h.check('moved to monitor 1 it is on the main taskbar and leaves the other', bool(seen),
            main=sorted(main), other=sorted(other))
    h.eval(MOVE_JS.replace('__PID__', str(tm.pid)).replace('__MON__', '1'))
    seen = wait(h, only_on_strip, 'Task Manager back on monitor 2', 15)
    h.check('moved back to monitor 2 it is on its taskbar again', bool(seen))

    # ---- 6. focus and minimize from the taskbar of monitor 2 (real clicks)
    h.eval(FOCUS_JS.replace('__PID__', str(st.pid)))
    wait(h, lambda: (window(h, st.pid) or {}).get('focus'), 'Settings focused', 10)
    h.sleep(1)

    def tm_button():
        top_ = strip_top(tops(h), 1)
        node = next((n for n in lib_tree(top_) if str(n.get('name') or '').startswith('tb-m1-app-')
                     and any(x in n['name'].lower() for x in NEEDLES['taskmgr'])), None) if top_ else None
        s = strip_of(h, 1, mons)
        if not (node and node.get('rect') and s):
            return None
        x, y, w, hh = node['rect']
        return {'node': node, 'x': s['x'] + x + w // 2, 'y': s['y'] + y + hh // 2, 'strip': s}
    b = tm_button()
    h.require('the Task Manager button on the taskbar of monitor 2', bool(b))
    h.record('click 1', click_at(h, b['x'], b['y']))
    focused = wait(h, lambda: (window(h, tm.pid) or {}).get('focus'), 'Task Manager focused by its button', 10)
    h.check('a click on its button on the taskbar of monitor 2 focuses Task Manager', bool(focused), window=window(h, tm.pid))
    wait(h, lambda: 'is-focused' in (tm_button() or {}).get('node', {}).get('classes', []), 'the button shows the focus', 5)
    h.sleep(0.8)
    b = tm_button() or b
    h.record('click 2', click_at(h, b['x'], b['y']))
    minimized = wait(h, lambda: (window(h, tm.pid) or {}).get('minimized'), 'Task Manager minimized by its button', 10)
    win = window(h, tm.pid) or {}
    s = b['strip']
    icon = win.get('icon')
    inside = bool(icon) and s['x'] <= icon[0] and icon[0] + icon[2] <= s['x'] + s['w'] and \
        s['y'] <= icon[1] and icon[1] + icon[3] <= s['y'] + s['h']
    h.check('a second click minimizes it', bool(minimized), window=win)
    h.check('it minimizes into its button on the taskbar of monitor 2 (icon geometry inside that taskbar)', inside,
            icon=icon, strip=[s['x'], s['y'], s['w'], s['h']], button=[b['x'], b['y']])
    h.sleep(1)
    b = tm_button() or b
    h.record('click 3', click_at(h, b['x'], b['y']))
    restored = wait(h, lambda: (lambda w: w and not w['minimized'] and w['focus'])(window(h, tm.pid)),
                    'Task Manager restored by its button', 10)
    h.check('a third click restores and focuses it', bool(restored), window=window(h, tm.pid))

    # ---- 7. menus as popovers on the taskbar of monitor 2
    h.sleep(0.8)
    b = tm_button() or b
    h.record('right click on the button', click_at(h, b['x'], b['y'], 3))
    jump = wait(h, lambda: (lambda t: t and node_named(t, 'menus-jump-launch') and node_named(t, 'menus-jump-close'))(
        strip_top(tops(h), 1)), 'the jump list above the button', 10)
    h.check('a right click on the button opens its jump list (menus-jump-launch, -pin, -close) on monitor 2',
            bool(jump) and bool(node_named(strip_top(tops(h), 1), 'menus-jump-pin')))
    h.sleep(1)
    lists = [n['name'] for n in lib_tree(strip_top(tops(h), 1) or {}) if str(n.get('name') or '').startswith('tb-m1-pv-')]
    h.check('no window list opens over the jump list while the pointer stays on the button', not lists, lists=lists)
    h.screenshot(f'{L}-taskbar2-jumplist', area=(m1['x'], m1['y'] + m1['h'] - 300, m1['w'], 300))
    key(h, 'Escape')
    closed = wait(h, lambda: not node_named(strip_top(tops(h), 1), 'menus-jump-launch'), 'the jump list closed', 5)
    h.check('Escape closes it', bool(closed))
    if not closed:
        click_at(h, m1['x'] + 20, m1['y'] + m1['h'] - BAR // 2)
        wait(h, lambda: not node_named(strip_top(tops(h), 1), 'menus-jump-launch'), 'the jump list closed by a click', 5)
    empty_x = m1['x'] + m1['w'] - 40
    h.record('right click on empty space', click_at(h, empty_x, m1['y'] + m1['h'] - BAR // 2, 3))
    menu = wait(h, lambda: node_named(strip_top(tops(h), 1), 'menus-taskbar-settings'), 'the taskbar menu', 10)
    h.check('a right click on its empty space opens the taskbar menu (Taakbeheer, Taakbalkinstellingen)',
            bool(menu) and bool(node_named(strip_top(tops(h), 1), 'menus-taskbar-taskmgr')))
    h.screenshot(f'{L}-taskbar2-menu', area=(m1['x'], m1['y'] + m1['h'] - 200, m1['w'], 200))
    # the pointer resting on an app button while that menu is open: no window list over it (as with the jump list)
    b = tm_button() or b
    h.record('pointer on the button with the taskbar menu open', pointer(h, [['move', b['x'], b['y']], ['wait', 1200]]))
    lists = [n['name'] for n in lib_tree(strip_top(tops(h), 1) or {}) if str(n.get('name') or '').startswith('tb-m1-pv-')]
    h.check('no window list opens over the taskbar menu while the pointer rests on an app button',
            not lists and bool(node_named(strip_top(tops(h), 1), 'menus-taskbar-settings')), lists=lists)
    key(h, 'Escape')
    if not wait(h, lambda: not node_named(strip_top(tops(h), 1), 'menus-taskbar-settings'), 'the taskbar menu closed', 5):
        click_at(h, m1['x'] + 20, m1['y'] + m1['h'] - BAR // 2)
        wait(h, lambda: not node_named(strip_top(tops(h), 1), 'menus-taskbar-settings'), 'the menu closed by a click', 5)

    # ---- 8. Settings: "Show my taskbar on all displays"
    settings_page(h, L, mons, st, tm)

    # ---- 9. the lock screen's path: the extension disabled and enabled again
    h.record('lock', h.eval("(()=>{Main.sessionMode.pushMode('unlock-dialog');return Main.sessionMode.currentMode;})()"))
    h.sleep(4)
    h.record('locked state', h.eval(STATE_JS))
    h.check('locked: no taskbar of monitor 2', not any(w['mapped'] for w in strips(h)), strips=strips(h))
    h.record('unlock', h.eval("(()=>{Main.sessionMode.popMode('unlock-dialog');return Main.sessionMode.currentMode;})()"))
    back = wait(h, lambda: strip_of(h, 1, mons), 'the taskbar of monitor 2 after unlock', 40)
    h.check('after unlock the taskbar of monitor 2 is back at its place', bool(back), strips=strips(h))
    seen = wait(h, only_on_strip, 'Task Manager on the taskbar of monitor 2 after unlock', 20)
    h.check('after unlock Task Manager is on it again (and not on the main taskbar)', bool(seen))
    sent = wait(h, lambda: (lambda b: b if b and {'main', '1'} <= set(b.get('iconSources') or []) else None)(h.eval(BRIDGE_JS)),
                'both taskbars sent their button rectangles after unlock', 15)
    h.check('after unlock both taskbars tell GNOME again where their buttons are (windows.setIconRects; the main '
            'taskbar too, whose buttons did not move)', bool(sent), bridge=h.eval(BRIDGE_JS))
    h.screenshot(f'{L}-after-unlock')

    # ---- 9b. the language at run time: the taskbar and desktop windows of monitor 2 take their new titles
    titles(h, L)

    # ---- 10. best effort: another primary, the monitors in another order, a monitor unplugged
    display_changes(h, L, mons, st, tm)
    h.close('settings')
    h.close('taskmgr')


TITLES = {'nl': ('Taakbalk', 'Bureaublad'), 'en': ('Taskbar', 'Desktop')}


def titles(h, L):
    """Ctrl+Alt+Tab and a screen reader name the windows by their titles: after a language switch the windows of the
    other monitor have the new one, as the desktop of the primary monitor has."""
    def named(lang):
        bar, desk = TITLES[lang]
        s, d = strips(h), desktop_windows(h)
        others = [w for k, w in d.items() if k.startswith(SECONDARY_DESKTOP)]
        primary = [w for w in h.eval(lib(h).WINDOW_TITLES_JS) or [] if w.get('wm') == 'io.github.win11shell.Panel']
        ok = bool(primary) and all(w['title'] == bar for w in primary) and bool(s) and all(w['title'] == bar for w in s) and bool(others) and all(w['title'] == desk for w in others) \
            and (d.get('io.github.win11shell.Desktop') or {}).get('title') == desk
        return {'strips': [w['title'] for w in s], 'desktops': {k: w['title'] for k, w in d.items()}} if ok else None
    h.check(f'the windows of monitor 2 carry their {L} titles', bool(named(L)),
            strips=[w['title'] for w in strips(h)], desktops={k: w.get('title') for k, w in desktop_windows(h).items()})
    other = 'en' if L == 'nl' else 'nl'
    h.record(f'language {other}', h.set_language(other, via='rename', skip=('settings', 'taskmgr')))
    seen = wait(h, lambda: named(other), f'the windows of monitor 2 titled in {other}', 15)
    h.check(f'after switching to {other} the taskbar and desktop windows of monitor 2 have their {other} titles',
            bool(seen), strips=[w['title'] for w in strips(h)],
            desktops={k: w.get('title') for k, w in desktop_windows(h).items()})
    h.record(f'language {L}', h.set_language(L, via='rename', skip=('settings', 'taskmgr')))
    back = wait(h, lambda: named(L), f'the windows of monitor 2 titled in {L} again', 15)
    h.check(f'and back in {L}', bool(back))
    if h.surfaces.get('settings'):
        h.wait_lang('settings', L)


DESKTOP_JS = ("(()=>{const type=t=>Object.keys(Meta.WindowType).find(k=>Meta.WindowType[k]===t)||String(t);"
              "const e=Main.extensionManager.lookup('" + EXT + "');const d=e&&e.stateObj&&e.stateObj._panelBridge?._desktopBridge;"
              "const r=x=>x?[x.x,x.y,x.width,x.height]:null;"
              "return global.get_window_actors().map(a=>a.meta_window).filter(w=>/^io\\.github\\.win11shell\\.(Secondary)?Desktop/"
              ".test(w.get_wm_class()||'')).map(w=>({wm:w.get_wm_class(),title:w.get_title(),type:type(w.get_window_type()),"
              "frame:r(w.get_frame_rect()),"
              "buffer:r(w.get_buffer_rect()),monitor:w.get_monitor(),work:r(w.get_work_area_current_monitor()),"
              "mapped:!!w.get_compositor_private()?.mapped,"
              "geometry:d?d.geometry():null}));})()")


def desktop_windows(h):
    """The desktop and the desktop windows of the other monitors as Mutter has them: {app id: {type, frame, ...}}."""
    return {w['wm']: w for w in h.eval(DESKTOP_JS) or []}


def desktop(h, L, mons):
    m0, m1 = mons
    wins = wait(h, lambda: (lambda d: d if f'{SECONDARY_DESKTOP}1' in d and d[f'{SECONDARY_DESKTOP}1']['mapped'] else None)(
        desktop_windows(h)), 'the desktop window of monitor 2', 20) or desktop_windows(h)
    h.record('desktop windows', wins)
    main = wins.get('io.github.win11shell.Desktop') or {}
    other = wins.get(f'{SECONDARY_DESKTOP}1') or {}
    frame, frame2 = main.get('frame') or [0, 0, 0, 0], other.get('frame') or [0, 0, 0, 0]
    on = lambda f, m: f[0] == m['x'] and f[1] == m['y'] and f[2] == m['w'] and 0 < f[3] <= m['h']
    h.check('the desktop window covers monitor 1 (the primary; Mutter keeps a desktop window in one work area)',
            main.get('type') == 'DESKTOP' and on(frame, m0), desktop=main)
    h.check('monitor 2 has a desktop window of its own (SecondaryDesktop.Monitor1, a desktop window on monitor 2)',
            other.get('type') == 'DESKTOP' and on(frame2, m1) and other.get('mapped'), desktop=other)
    h.check('the desktop geometry lists both monitors with the primary marked',
            [(a['index'], a['primary']) for a in (main.get('geometry') or {}).get('areas', [])] == [(0, True), (1, False)],
            geometry=main.get('geometry'))
    picture = quad_png(h, h.hm.RUN / 'home' / 'w11-quad.png')
    defaults = json.loads((h.hm.SRC / 'host/src/settings-defaults.json').read_text())
    # as the store writes it (lock, temp file, rename); the shell's backend sees it within its 150 ms poll
    h.hm.write_settings({'wallpaper': {**defaults['wallpaper'], 'type': 'image', 'path': picture, 'fit': 'fill'}})
    h.record('wallpaper', {'path': picture, 'fit': 'fill', 'size': [1600, 1000]})
    h.sleep(4)

    def wall_page():
        top = next((t for t in h.native_state() if node_named(t, 'desktop-window-m1')), None)
        node = node_named(top, 'desktop-wall-1')
        return (top, node) if node and node.get('rect') else None
    found = wait(h, wall_page, 'the wallpaper of monitor 2', 15)
    top, node = found or (None, None)
    h.check('the desktop window of monitor 2 draws the wallpaper over all of it (desktop-wall-1)',
            bool(node) and node['rect'] == [0, 0, top.get('width'), top.get('height')]
            and top.get('width') == m1['w'] and top.get('height') == frame2[3],
            page=(node or {}).get('rect'), window=[(top or {}).get('width'), (top or {}).get('height')])
    shot = h.screenshot(f'{L}-wallpaper', settle=1.5)
    for m in mons:
        seen = {}
        for name, (fx, fy) in (('red', (0.25, 0.25)), ('green', (0.75, 0.25)), ('blue', (0.25, 0.7)), ('yellow', (0.75, 0.7))):
            x, y = m['x'] + int(m['w'] * fx), m['y'] + int(m['h'] * fy)
            rgb = h.pixel(shot, x, y)
            seen[name] = {'at': [x, y], 'rgb': rgb, 'is': colour_name(rgb)}
        h.check(f'monitor {m["index"] + 1} shows the whole picture by itself (four colours at 25 %/75 %, in our desktop '
                'window there)', all(v['is'] == k for k, v in seen.items()), pixels=seen)
    span(h, L, mons, defaults, picture)
    # the icons: on monitor 1, above its taskbar
    state = h.native_state()
    desk = next((t for t in state if any(n.get('name') == 'desktop-icons' for n in lib_tree(t))), None)
    items = [n for n in lib_tree(desk) if str(n.get('name') or '').startswith('desktop-item-')] if desk else []
    outside = [n['name'] for n in items if not n.get('rect') or n['rect'][0] < m0['x'] or n['rect'][1] < m0['y']
               or n['rect'][0] + n['rect'][2] > m0['x'] + m0['w'] or n['rect'][1] + n['rect'][3] > m0['y'] + m0['h'] - BAR]
    h.check('every desktop icon lies on monitor 1 above its taskbar', bool(items) and not outside,
            icons=len(items), outside=outside)
    # the desktop menu on monitor 2
    x, y = m1['x'] + m1['w'] // 2, m1['y'] + m1['h'] // 3
    h.record('right click on the desktop of monitor 2', click_at(h, x, y, 3))

    def menu():
        top = next((t for t in h.native_state() if node_named(t, 'desktop-window-m1')), None)
        node = node_named(top, 'desktop-menu')
        return node if node and node.get('rect') else None
    node = wait(h, menu, 'the desktop menu on monitor 2', 10)
    rect = (node or {}).get('rect') or [0, 0, 0, 0]
    h.check('a right click on the empty desktop of monitor 2 opens the desktop menu there (in its desktop window)',
            bool(node) and 0 <= rect[0] and rect[0] + rect[2] <= m1['w'], menu=rect)
    h.screenshot(f'{L}-desktop-menu-monitor2', area=(m1['x'], m1['y'], m1['w'], m1['h']))
    key(h, 'Escape')
    closed = wait(h, lambda: not menu(), 'the desktop menu closed', 5)
    h.check('Escape closes the desktop menu', bool(closed))
    if not closed:
        # a click beside it closes it too (autohide), and it must not take the next clicks
        click_at(h, m1['x'] + 40, m1['y'] + m1['h'] - 150)
        wait(h, lambda: not menu(), 'the desktop menu closed by a click', 5)
    new_folder(h, L, mons)
    h.hm.write_settings({'wallpaper': defaults['wallpaper']})
    h.sleep(1.5)


def span(h, L, mons, defaults, picture):
    """"Uitspreiden" (span): as in Windows 11 one picture over all monitors. It covers their bounding box (here
    2720 x 960, so 1.7 times larger, its top and bottom cut) and each monitor shows its own part: monitor 1 the left
    847 picture pixels (red over blue, the green edge at its right), monitor 2 the rest (green over yellow)."""
    h.hm.write_settings({'wallpaper': {**defaults['wallpaper'], 'type': 'image', 'path': picture, 'fit': 'span'}})
    h.record('wallpaper span', {'path': picture, 'fit': 'span', 'size': [1600, 1000]})
    h.sleep(4)
    left, top = min(m['x'] for m in mons), min(m['y'] for m in mons)
    bw = max(m['x'] + m['w'] for m in mons) - left
    bh = max(m['y'] + m['h'] for m in mons) - top
    cover = max(bw / 1600, bh / 1000)
    ox, oy = (1600 * cover - bw) / 2, (1000 * cover - bh) / 2

    def expected(x, y):
        px, py = (x - left + ox) / cover, (y - top + oy) / cover
        return ('red' if px < 800 else 'green') if py < 500 else ('blue' if px < 800 else 'yellow')
    shot = h.screenshot(f'{L}-wallpaper-span', settle=1.5)
    for m in mons:
        seen = {}
        for fx, fy in ((0.25, 0.25), (0.75, 0.25), (0.98, 0.25), (0.25, 0.7), (0.75, 0.7)):
            x, y = m['x'] + int(m['w'] * fx), m['y'] + int(m['h'] * fy)
            rgb = h.pixel(shot, x, y)
            seen[f'{fx}/{fy}'] = {'at': [x, y], 'rgb': rgb, 'is': colour_name(rgb), 'want': expected(x, y)}
        h.check(f'span: monitor {m["index"] + 1} shows its own part of one picture over both monitors',
                all(v['is'] == v['want'] for v in seen.values()), pixels=seen)


def desk_top(h):
    """The dev state toplevel of the desktop with the icons (monitor 1)."""
    return next((t for t in h.native_state() if any(n.get('name') == 'desktop-icons' for n in lib_tree(t))), None)


def subtree(node):
    """A node of a dev state tree and everything below it, depth first."""
    out, todo = [], [node]
    while todo:
        n = todo.pop()
        if isinstance(n, dict):
            out.append(n)
            todo.extend(reversed(n.get('children') or []))
    return out


def open_menu(h):
    """The desktop menu open in the desktop window of monitor 2 (its node), or None."""
    top = next((t for t in h.native_state() if node_named(t, 'desktop-window-m1')), None)
    return node_named(top, 'desktop-menu')


def menu_row(h, text):
    """The row of the open desktop menu on monitor 2 with that label (allocated, so shown), or None."""
    menu_ = open_menu(h)
    return next((n for n in subtree(menu_) if n.get('label') == text and n.get('rect') and n['rect'][2] > 0
                 and n['rect'][3] > 0), None) if menu_ else None


def submenu_row(h, text):
    """The screen point of the row with that label in the open submenu of the desktop menu on monitor 2 (Nieuw's: a
    nested popover, a popup surface of its own). Its rows' rects are right within that popover but its own origin in
    the dev state is not where Mutter placed it, so the point is the newest popup surface plus the row's offset in
    the popover."""
    menu_ = open_menu(h)
    nested = [n for n in subtree(menu_)[1:] if n.get('type') == 'GtkPopoverMenu' and n.get('rect')] if menu_ else []
    for pop in reversed(nested):
        row = next((n for n in subtree(pop) if n.get('label') == text and n.get('rect') and n['rect'][2] > 0), None)
        surfaces = h.eval(POPUPS_JS.replace('__PID__', str(int(h.native_pid)))) or []
        if row and surfaces:
            sx, sy, _sw, _sh = surfaces[-1]
            px, py = pop['rect'][:2]
            x, y, w, hh = row['rect']
            return {'x': sx + x - px + w // 2, 'y': sy + y - py + hh // 2, 'surface': surfaces[-1], 'popover': pop['rect'],
                    'row': row['rect']}
    return None


def new_folder(h, L, mons):
    """Nieuw > Map from the desktop menu of monitor 2: the folder goes to the desktop of monitor 1 with its name
    selected, and the name typed right after goes into it (the desktop there asks for the keyboard, which the click
    on monitor 2 gave to the desktop window there)."""
    m0, m1 = mons
    nl = L == 'nl'
    folder = h.hm.RUN / 'home' / 'Desktop'
    default, name = folder / ('Nieuwe map' if nl else 'New folder'), folder / 'mmreis'
    for leftover in (default, name):
        if leftover.is_dir():
            leftover.rmdir()
    h.record('right click on the desktop of monitor 2 (Nieuw)', click_at(h, m1['x'] + m1['w'] // 2, m1['y'] + m1['h'] // 3, 3))
    row = wait(h, lambda: menu_row(h, 'Nieuw' if nl else 'New'), 'Nieuw in the desktop menu of monitor 2', 10)
    if row:
        x, y, w, hh = row['rect']
        h.record('click on Nieuw', click_at(h, m1['x'] + x + w // 2, m1['y'] + y + hh // 2))
        h.sleep(0.6)
        row = wait(h, lambda: submenu_row(h, 'Map' if nl else 'Folder'), 'Map in its submenu', 10)
    if row:
        h.record('click on Map', {**row, 'pointer': click_at(h, row['x'], row['y'])})
    entry = wait(h, lambda: node_named(desk_top(h), 'desktop-rename'), 'the rename box on the desktop of monitor 1', 15)
    h.check('Nieuw > Map on monitor 2 makes the folder on the desktop of monitor 1, its name ready to type',
            bool(row) and bool(entry) and default.is_dir(), rename=(entry or {}).get('rect'))
    focus = wait(h, lambda: h.eval(FOCUS_WM_JS) == 'io.github.win11shell.Desktop', 'the desktop of monitor 1 has the keyboard', 5)
    h.check('the desktop of monitor 1 has the keyboard now, not the desktop window of monitor 2 that was clicked',
            bool(focus), focus=h.eval(FOCUS_WM_JS))
    h.screenshot(f'{L}-new-folder-rename', area=(m0['x'], m0['y'], m0['w'], m0['h']))
    for letter in name.name:
        key(h, letter)
    key(h, 'Return')
    made = wait(h, lambda: name.is_dir() and not default.exists(), 'the folder renamed by typing', 10)
    h.check(f'the name typed right after goes into it: the folder is called "{name.name}"', bool(made),
            files=sorted(p.name for p in folder.iterdir()))
    if not row:
        h.record('desktop menu rows', [[n.get('type'), n.get('label'), n.get('rect')] for n in subtree(open_menu(h))
                                       if n.get('label')])
    if node_named(desk_top(h), 'desktop-rename'):
        key(h, 'Escape')
    # a menu still open would take the next click (autohide)
    if open_menu(h):
        key(h, 'Escape')
        if not wait(h, lambda: not open_menu(h), 'the desktop menu closed', 3):
            click_at(h, m1['x'] + 40, m1['y'] + m1['h'] - 150)
            wait(h, lambda: not open_menu(h), 'the desktop menu closed by a click', 5)
    for leftover in (default, name):
        if leftover.is_dir():
            leftover.rmdir()
    wait(h, lambda: not any(str(n.get('name') or '') == f'desktop-item-{name.name}' for n in lib_tree(desk_top(h) or {})),
         'the folder gone from the desktop', 10)


def settings_rows(h):
    """Settings' two rows for more than one display on its Taskbar page: {widget name: node or None}."""
    found = {}
    for name in ('taskbar-all-displays', 'taskbar-multi-display-apps'):
        nodes_ = h.find('settings', name)
        found[name] = nodes_[0] if nodes_ else None
    return found


def rows_sensitive(h, want):
    """Both rows there and (in)sensitive as `want` (the page left open follows the monitors itself)."""
    r = settings_rows(h)
    return r if all(r.values()) and all((v.get('sensitive') is not False) == want for v in r.values()) else None


def settings_page(h, L, mons, st, tm):
    m0, m1 = mons

    def rows():
        return settings_rows(h)
    found = wait(h, lambda: (lambda r: r if all(r.values()) else None)(rows()), 'the two rows on the Taskbar page', 15) or rows()
    h.record('rows', {k: ({'sensitive': v.get('sensitive'), 'active': v.get('active'), 'label': v.get('label')} if v else None)
                      for k, v in found.items()})
    h.check('Settings has "Show my taskbar on all displays", on and enabled with two displays',
            bool(found['taskbar-all-displays']) and found['taskbar-all-displays'].get('sensitive') is not False
            and found['taskbar-all-displays'].get('active') is True)
    h.check('and "When using multiple displays, show my taskbar apps on", enabled',
            bool(found['taskbar-multi-display-apps']) and found['taskbar-multi-display-apps'].get('sensitive') is not False)
    text = h.page_text('settings')
    h.record('settings text', {k: text.get(k) for k in ('lang', 'dutchWordHits')})
    # Settings in front, scrolled down to the rows, for the screenshots
    h.eval(FOCUS_JS.replace('__PID__', str(st.pid)))
    h.sleep(0.5)
    win = window(h, st.pid) or {}
    if win.get('rect'):
        x, y, w, hh = win['rect']
        h.record('scroll', h.eval(SCROLL_JS.replace('__X__', str(x + w * 2 // 3)).replace('__Y__', str(y + hh // 2))
                                  .replace('__N__', '12')))
        h.sleep(0.8)
    h.screenshot(f'{L}-settings-taskbar', surface='settings')
    h.record('switch off', h.click('settings', 'taskbar-all-displays'))
    gone = wait(h, lambda: not strips(h), 'the taskbar of monitor 2 gone', 15)
    h.check('"Show my taskbar on all displays" off: the taskbar of monitor 2 is gone', bool(gone), strips=strips(h))
    full = wait(h, lambda: (lambda w: w if w['h'] == m1['h'] else None)(h.work_area(1)), 'monitor 2 without a strut', 10)
    h.check('and its strut: the work area of monitor 2 is the whole monitor', bool(full), work=h.work_area(1))
    seen = wait(h, lambda: (lambda m, o: has(m, 'taskmgr', True))(*taskbars(h)), 'Task Manager on the main taskbar', 15)
    h.check('the main taskbar shows Task Manager again (every window, as with one monitor)', bool(seen))
    greyed = wait(h, lambda: (lambda r: r['taskbar-multi-display-apps'] and r['taskbar-multi-display-apps'].get('sensitive') is False)(rows()),
                  'the second row greyed out', 10)
    h.check('the second row is greyed out while the first is off', bool(greyed))
    h.screenshot(f'{L}-settings-taskbar-off', surface='settings')
    h.screenshot(f'{L}-off-monitor2', area=(m1['x'], m1['y'], m1['w'], m1['h']))
    h.record('switch on', h.click('settings', 'taskbar-all-displays'))
    back = wait(h, lambda: strip_of(h, 1, mons), 'the taskbar of monitor 2 back', 20)
    h.check('on again: the taskbar of monitor 2 is back', bool(back))
    seen = wait(h, lambda: (lambda m, o: has(o, 'taskmgr', True) and not has(m, 'taskmgr'))(*taskbars(h)),
                'Task Manager on the taskbar of monitor 2 again', 15)
    h.check('and Task Manager is on it again', bool(seen))


def inside(icon, rect):
    return bool(icon) and rect[0] <= icon[0] and icon[0] + icon[2] <= rect[0] + rect[2] and \
        rect[1] <= icon[1] and icon[1] + icon[3] <= rect[1] + rect[3]


def reorder(h, L, st, tm):
    """Monitor 2 placed left of monitor 1, the primary staying: both taskbars move without changing size, so neither
    sends new button rectangles (they did not move in their own windows). Where the windows minimize to (their icon
    geometry) must move with the taskbars all the same: Settings, on the primary, into the main taskbar, Task Manager
    into the taskbar of its own monitor."""
    h.eval(MOVE_JS.replace('__PID__', str(st.pid)).replace('__MON__', '0'))

    def places(ms):
        primary = next((m for m in ms if m['primary']), None)
        w_tm, w_st = window(h, tm.pid) or {}, window(h, st.pid) or {}
        strip = strip_of(h, w_tm.get('monitor'), ms) if w_tm else None
        if not primary or not strip or w_st.get('monitor') != primary['index']:
            return None
        bar = [primary['x'], primary['y'] + primary['h'] - BAR, primary['w'], BAR]
        own = [strip['x'], strip['y'], strip['w'], strip['h']]
        return {'settings': w_st.get('icon'), 'mainBar': bar, 'taskmgr': w_tm.get('icon'), 'strip': own,
                'ok': inside(w_st.get('icon'), bar) and inside(w_tm.get('icon'), own)}
    ms = h.eval(lib(h).MONITORS_JS) or []
    before = wait(h, lambda: (lambda r: r if r and r['ok'] else None)(places(ms)), 'the icon geometry before', 15)
    h.check('before: Settings minimizes into the main taskbar, Task Manager into the taskbar of monitor 2', bool(before),
            places=places(ms))
    made = h.display_config(order=[1, 0])
    if not made.get('ok'):
        h.check('monitor 2 placed left of monitor 1 (ApplyMonitorsConfig)', False, known=KNOWN_DISPLAYCONFIG, answer=made)
        return
    now = wait(h, lambda: (lambda ms_: ms_ if len(ms_) == 2 and any(m['primary'] and m['x'] > 0 for m in ms_) else None)(
        h.eval(lib(h).MONITORS_JS) or []), 'monitor 2 on the left', 15)
    if not now:
        h.check('monitor 2 placed left of monitor 1 (ApplyMonitorsConfig)', False, known=KNOWN_DISPLAYCONFIG,
                answer=made, monitors=h.eval(lib(h).MONITORS_JS))
    else:
        h.record('monitors in the other order', now)
        after = wait(h, lambda: (lambda r: r if r and r['ok'] else None)(places(now)), 'the icon geometry moved along', 20)
        h.check('monitor 2 left of monitor 1: Settings still minimizes into the main taskbar and Task Manager into the '
                'taskbar of its monitor, though neither taskbar changed size', bool(after), places=places(now),
                bridge=h.eval(BRIDGE_JS))
        h.screenshot(f'{L}-other-order')
        h.display_config(order=[next(m['index'] for m in now if m['primary']),
                                next(m['index'] for m in now if not m['primary'])])
    wait(h, lambda: (lambda ms_: len(ms_) == 2 and ms_[0]['primary'] and ms_[0]['x'] == 0)(h.eval(lib(h).MONITORS_JS) or []),
         'monitor 1 on the left again', 15)
    restored = wait(h, lambda: strip_of(h, 1, h.eval(lib(h).MONITORS_JS) or []), 'the taskbar of monitor 2 in its place', 20)
    h.check('back in the first order: the taskbar of monitor 2 is in its place again', bool(restored), strips=strips(h))


def display_changes(h, L, mons, st, tm):
    m0, m1 = mons
    made = h.display_config(primary=1)
    if not made.get('ok'):
        h.check('monitor 2 made the primary (ApplyMonitorsConfig)', False, known=KNOWN_DISPLAYCONFIG, answer=made)
        return
    now = wait(h, lambda: (lambda ms: ms if len(ms) == 2 and any(m['primary'] and m['x'] != 0 for m in ms) else None)(
        h.eval(lib(h).MONITORS_JS) or []), 'the other primary', 15)
    if not now:
        h.check('monitor 2 made the primary (ApplyMonitorsConfig)', False, known=KNOWN_DISPLAYCONFIG,
                answer=made, monitors=h.eval(lib(h).MONITORS_JS))
    else:
        primary = next(m for m in now if m['primary'])
        other = next(m for m in now if not m['primary'])
        h.record('monitors with the other primary', now)
        bar = wait(h, lambda: (lambda b: b if b and b['panel'] == [primary['x'], primary['y'], primary['w'], primary['h']] else None)(
            h.eval(BRIDGE_JS)), 'the panel on the new primary', 15)
        h.check('the main taskbar moves to the new primary', bool(bar), bridge=h.eval(BRIDGE_JS), primary=primary)
        moved = wait(h, lambda: strip_of(h, other['index'], now), 'the other taskbar on the old primary', 20)
        h.check(f'the other taskbar is {STRIP}{other["index"]} at the bottom of the other monitor', bool(moved),
                strips=strips(h))
        moved_desk = wait(h, lambda: (lambda d: d if (d.get('io.github.win11shell.Desktop') or {}).get('frame', [None])[0] ==
                                      primary['x'] and f'{SECONDARY_DESKTOP}{other["index"]}' in d else None)(desktop_windows(h)),
                          'the desktop on the new primary', 20)
        h.check('the desktop (with the icons) moves to the new primary, the other monitor gets its own desktop window',
                bool(moved_desk), desktops=desktop_windows(h))
        h.sleep(1)
        state = h.native_state()
        desk = next((t for t in state if any(n.get('name') == 'desktop-icons' for n in lib_tree(t))), None)
        items = [n for n in lib_tree(desk) if str(n.get('name') or '').startswith('desktop-item-')] if desk else []
        off = [n['name'] for n in items if not n.get('rect') or n['rect'][0] + n['rect'][2] > primary['w']]
        h.check('the desktop icons follow the primary', bool(items) and bool(moved_desk) and not off, outside=off)
        h.screenshot(f'{L}-other-primary')
    h.display_config(primary=next(m['index'] for m in (now or mons) if m['x'] == 0) if now else 0)
    wait(h, lambda: (lambda ms: len(ms) == 2 and ms[0]['primary'] and ms[0]['x'] == 0)(h.eval(lib(h).MONITORS_JS) or []),
         'the first monitor primary again', 15)
    restored = wait(h, lambda: strip_of(h, 1, h.eval(lib(h).MONITORS_JS) or []), 'the taskbar of monitor 2 again', 20)
    h.check('back: the taskbar of monitor 2 is there again', bool(restored), strips=strips(h))
    reorder(h, L, st, tm)
    off = h.display_config(disable=1)
    if not off.get('ok'):
        h.check('monitor 2 switched off (ApplyMonitorsConfig)', False, known=KNOWN_DISPLAYCONFIG, answer=off)
        return
    one = wait(h, lambda: (lambda ms: ms if len(ms) == 1 else None)(h.eval(lib(h).MONITORS_JS) or []), 'one monitor', 15)
    if not one:
        h.check('monitor 2 switched off (ApplyMonitorsConfig)', False, known=KNOWN_DISPLAYCONFIG, answer=off,
                monitors=h.eval(lib(h).MONITORS_JS))
    else:
        gone = wait(h, lambda: not strips(h), 'no other taskbar', 15)
        h.check('monitor 2 off: no taskbar of another monitor', bool(gone), strips=strips(h))
        geo = wait(h, lambda: (lambda g: g if g and g.get('w') == one[0]['w'] and g.get('h') == one[0]['h'] else None)(
            h.eval("(()=>{const e=Main.extensionManager.lookup('" + EXT + "');"
                   "return e.stateObj._panelBridge._desktopBridge.geometry();})()")), 'the desktop on monitor 1', 15)
        h.check('the desktop covers monitor 1 alone', bool(geo), geometry=geo)
        alone = wait(h, lambda: not any(k.startswith(SECONDARY_DESKTOP) for k in desktop_windows(h)), 'no other desktop window', 15)
        h.check('and no desktop window of another monitor is left', bool(alone), desktops=desktop_windows(h))
        grey = wait(h, lambda: rows_sensitive(h, False), "Settings' two rows greyed out with one display", 10)
        h.check('Settings, left open on its Taskbar page, greys both rows out with one display', bool(grey),
                rows={k: (v or {}).get('sensitive') for k, v in settings_rows(h).items()})
        h.screenshot(f'{L}-one-monitor')
    h.display_config(enable_all=True)
    two = wait(h, lambda: (lambda ms: ms if len(ms) == 2 else None)(h.eval(lib(h).MONITORS_JS) or []), 'two monitors again', 15)
    back = wait(h, lambda: strip_of(h, 1, two or []), 'the taskbar of monitor 2 after plugging it in', 20) if two else None
    h.check('monitor 2 on again: its taskbar is back', bool(back), strips=strips(h), monitors=two)
    if one:
        again = wait(h, lambda: rows_sensitive(h, True), "Settings' two rows enabled again", 10)
        h.check('and Settings enables both rows again with two displays', bool(again),
                rows={k: (v or {}).get('sensitive') for k, v in settings_rows(h).items()})
