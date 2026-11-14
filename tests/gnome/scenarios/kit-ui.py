"""Kit UI fixes (13-11), in a real GNOME 50 session with real pointer input:

  explorer   Details view: every column header has the x and width of its column's cells (the dev state's
             rects), at start, after dragging the Naam column's edge, after a double click on an edge (fit),
             and again in light theme with the widths the drag saved; "Openen met" on a text file lists apps
             with their icons (screenshots of the menu and its submenu)
  alles      no faint dots under the command bar: screenshots at start (welcome), with results and after the
             query is cleared again; the 24 px band under the command bar is one colour at start and after
             clearing (with results it holds the list, so not then)
  fonts      each app runs with GSK's cairo renderer and manual font rendering (native/kit/src/app.rs: automatic
             font rendering leaves a glyph's top pixel row behind where text moved): the boot report's renderer
             and fontRendering (native/kit/src/dev.rs)
  infobar    (not here: the Settings pages need a canned backend; see the kit builder's report)

Run (from G):
    tools/heavy.sh --min-mb 2560 --tag kit-ui -- python3 -B tests/gnome/harness.py launch run --lang nl \\
        --build-dir tests/build/kitui-release --source worktree --scenario tests/gnome/scenarios/kit-ui.py
"""
import json
import os
import subprocess
from pathlib import Path

WINDOWS_JS = ("(()=>{const Meta=imports.gi.Meta;return global.get_window_actors().map(a=>a.meta_window)"
              ".filter(w=>(w.get_wm_class()==='__ID__'||w.get_gtk_application_id()==='__ID__')"
              "&&w.get_window_type()===Meta.WindowType.NORMAL).map(w=>{const r=w.get_frame_rect();"
              "const b=w.get_buffer_rect();return {pid:w.get_pid(),x:r.x,y:r.y,w:r.width,h:r.height,"
              "bx:b.x,by:b.y,bw:b.width,bh:b.height,focus:w.has_focus()};});})()")
POPUPS_JS = ("global.get_window_actors().map(a=>a.meta_window).filter(w=>w.get_pid()===__PID__"
             "&&w.get_window_type()!==Meta.WindowType.NORMAL).map(w=>{const r=w.get_frame_rect();"
             "return {type:w.get_window_type(),x:r.x,y:r.y,w:r.width,h:r.height};})")
ACTIVATE_JS = ("(()=>{const ws=global.get_window_actors().map(a=>a.meta_window).filter(w=>w.get_pid()===__PID__);"
               "ws.forEach(w=>{w.unminimize();Main.activateWindow(w);});return ws.length;})()")
CLOSE_JS = ("(()=>{const ws=global.get_window_actors().map(a=>a.meta_window).filter(w=>w.get_pid()===__PID__);"
            "ws.forEach(w=>w.delete(global.get_current_time()));return ws.length;})()")
# steps: ["move", x, y] | ["down", button] | ["up", button] | ["wait", ms]; 30 ms between steps
POINTER_JS = (
    "(async()=>{const Clutter=(await import('gi://Clutter')).default;const GLib=(await import('gi://GLib')).default;"
    "const wait=ms=>new Promise(r=>GLib.timeout_add(GLib.PRIORITY_DEFAULT,ms,()=>{r();return GLib.SOURCE_REMOVE;}));"
    "const seat=Clutter.get_default_backend().get_default_seat();"
    "globalThis.__kituiPointer??=seat.create_virtual_device(Clutter.InputDeviceType.POINTER_DEVICE);"
    "const d=globalThis.__kituiPointer,us=()=>GLib.get_monotonic_time();"
    "for(const s of __STEPS__){if(s[0]==='move')d.notify_absolute_motion(us(),s[1],s[2]);"
    "else if(s[0]==='down')d.notify_button(us(),s[1],Clutter.ButtonState.PRESSED);"
    "else if(s[0]==='up')d.notify_button(us(),s[1],Clutter.ButtonState.RELEASED);"
    "await wait(s[0]==='wait'?s[1]:30);}"
    "return global.get_pointer().slice(0,2);})()")


def pointer(h, steps):
    return h.eval(POINTER_JS.replace('__STEPS__', json.dumps(steps)))


def walk(node, out):
    if isinstance(node, dict):
        out.append(node)
        for child in node.get('children') or []:
            walk(child, out)
    return out


def nodes(state):
    out = []
    for top in state if isinstance(state, list) else []:
        walk(top.get('tree'), out)
    return out


def state(h, kind):
    return h.s.native_app(kind, {'native': 'state'}, 20)


def gone(pid):
    try:
        os.waitpid(pid, os.WNOHANG)
    except OSError:
        pass
    try:
        stat = Path(f'/proc/{pid}/stat').read_text()
    except OSError:
        return True
    return stat[stat.rindex(')') + 2:].startswith('Z')


def set_theme(h, mode):
    path = h.hm.RUN / 'config' / 'win11-gnome' / 'settings.json'
    try:
        data = json.loads(path.read_text() or '{}')
    except (OSError, ValueError):
        data = {}
    data.setdefault('theme', {})['mode'] = mode
    path.write_text(json.dumps(data, indent=2))


def spawn(h, kind, argv, env=None):
    bindir = h.hm.RUN / 'prefix' / 'bin'
    argv = [str(bindir / argv[0]), *argv[1:]]
    full = dict(os.environ, WAYLAND_DISPLAY=h.hm.WL, GDK_BACKEND='wayland', **(env or {}))
    log = open(h.hm.RUN / 'logs' / f'kitui-{kind}.log', 'a')
    return subprocess.Popen(argv, env=full, stdout=log, stderr=subprocess.STDOUT)


def boot(h, kind, rows):
    """The app's boot report; its renderer and font rendering checked (the kit's Manual under cairo)."""
    try:
        row = h.s.native_app_boot(kind, rows, 45)
    except TimeoutError as error:
        h.check(f'{kind}: boot report', False, error=str(error))
        return None
    h.check(f'{kind}: cairo with manual font rendering', row.get('renderer') == 'cairo' and row.get('fontRendering') == 'manual',
            renderer=row.get('renderer'), fontRendering=row.get('fontRendering'))
    return row


def one_colour(h, file, rect, label):
    """The rect (screen coordinates, as the full-screen shot) of a shot holds exactly one colour, its top-left pixel's."""
    if not file or not rect:
        h.check(label, False, file=file, rect=rect)
        return
    pix = h.s.GdkPixbuf.Pixbuf.new_from_file(str(h.hm.RUN / 'shots' / file))
    data, stride, channels = pix.get_pixels(), pix.get_rowstride(), pix.get_n_channels()
    x0, y0, w, ht = rect
    x1, y1 = min(pix.get_width(), x0 + w), min(pix.get_height(), y0 + ht)
    x0, y0 = max(0, x0), max(0, y0)
    first = tuple(data[y0 * stride + x0 * channels:y0 * stride + x0 * channels + 3])
    other = {}
    for y in range(y0, y1):
        for x in range(x0, x1):
            i = y * stride + x * channels
            px = tuple(data[i:i + 3])
            if px != first:
                other[px] = other.get(px, 0) + 1
    h.check(label, x1 > x0 and y1 > y0 and not other, rect=[x0, y0, x1 - x0, y1 - y0], colour=first,
            others=sorted(other.items(), key=lambda kv: -kv[1])[:5], otherPixels=sum(other.values()))


def window(h, app_id):
    return h.until(lambda: [w for w in h.eval(WINDOWS_JS.replace('__ID__', app_id)) or [] if w['w'] > 0] or None,
                   f'{app_id} window', 30, 0.3)[0]


def close(h, kind, win):
    pid = win['pid']
    # the pointer off every window first (a tooltip or hover under the pointer while the window goes)
    pointer(h, [['move', h.hm.WIDTH - 3, 330], ['wait', 300]])
    h.eval(CLOSE_JS.replace('__PID__', str(pid)))
    try:
        h.until(lambda: gone(pid), f'{kind} exits', 15, 0.3)
    except TimeoutError:
        h.check(f'{kind}: closes', False)


def shot(h, name):
    try:
        return h.screenshot(name, settle=0.8)
    except Exception as error:   # noqa: BLE001
        h.check(f'screenshot {name}', False, error=str(error))
        return None


# ---- explorer -----------------------------------------------------------------------------------------------
def columns(h):
    """(header cells, item rows) as [x, width] lists from the dev state (window coordinates)."""
    heads, rows = [], {}
    for n in nodes(state(h, 'explorer')):
        classes = n.get('classes') or []
        if 'xp-head-cell' in classes and n.get('rect'):
            heads.append(n['rect'])
        if 'xp-cell' in classes and n.get('rect'):
            rows.setdefault(n['rect'][1], []).append(n['rect'])
    return heads, rows


def check_aligned(h, label):
    heads, rows = columns(h)
    want = [[r[0], r[2]] for r in heads]
    bad = {y: [[r[0], r[2]] for r in cells] for y, cells in rows.items() if [[r[0], r[2]] for r in cells] != want}
    h.check(f'explorer: headers over their columns ({label})', bool(heads) and bool(rows) and not bad,
            headers=want, rows=len(rows), misaligned=dict(list(bad.items())[:3]))
    return heads


def item_rect(h, text):
    for n in nodes(state(h, 'explorer')):
        if n.get('label') == text and n.get('rect') and 'xp-item-name' in (n.get('classes') or []):
            return n['rect']
    return None


def explorer(h, folder, light=False, tag=None, menu=True):
    tag = tag or ('light' if light else 'dark')
    rows = len(h.s.rows())
    proc = spawn(h, f'explorer-{tag}', ['win11-explorer', str(folder)])
    boot(h, 'explorer', rows)
    win = window(h, 'io.github.win11shell.Explorer')
    h.record(f'explorer.{tag}.window', win)
    h.sleep(2.5)
    h.eval(ACTIVATE_JS.replace('__PID__', str(win['pid'])))
    h.sleep(1)
    if not columns(h)[0]:
        h.s.native_app('explorer', {'native': 'key', 'key': '<Control><Shift>6'})
        h.sleep(1)
    heads = check_aligned(h, f'start, {tag}')
    shot(h, f'explorer-{tag}')
    if light or len(heads) < 2:
        close(h, 'explorer', win)
        return proc
    ox, oy = win['x'], win['y']
    # drag the Naam column's edge 80 px to the left (its grip: the cell's last 7 px)
    name = heads[0]
    gx, gy = ox + name[0] + name[2] - 3, oy + name[1] + name[3] // 2
    steps = [['move', gx, gy], ['wait', 150], ['down', 1], ['wait', 120]]
    steps += [['move', gx - 8 * i, gy] for i in range(1, 11)]
    steps += [['wait', 120], ['up', 1], ['wait', 300], ['move', gx - 80, gy + 200], ['wait', 300]]
    pointer(h, steps)
    h.sleep(0.8)
    after = check_aligned(h, 'after a drag on the Naam edge')
    h.check('explorer: the drag resized Naam', bool(after) and abs(after[0][2] - (name[2] - 80)) <= 2,
            before=name[2], after=after[0][2] if after else None)
    shot(h, f'explorer-{tag}-resized' if tag != 'dark' else 'explorer-resized')
    # double click on the Gewijzigd op edge: fit to its widest text
    mod = after[1] if len(after) > 1 else heads[1]
    gx, gy = ox + mod[0] + mod[2] - 3, oy + mod[1] + mod[3] // 2
    pointer(h, [['move', gx, gy], ['wait', 150], ['down', 1], ['wait', 40], ['up', 1], ['wait', 60],
                ['down', 1], ['wait', 40], ['up', 1], ['wait', 300], ['move', gx, gy + 220], ['wait', 300]])
    h.sleep(0.8)
    fit = check_aligned(h, 'after a double click on the Gewijzigd op edge')
    h.check('explorer: the double click fitted Gewijzigd op', len(fit) > 1 and fit[1][2] != mod[2],
            before=mod[2], after=fit[1][2] if len(fit) > 1 else None)
    shot(h, f'explorer-{tag}-fit' if tag != 'dark' else 'explorer-fit')
    # "Openen met" on a text file
    r = item_rect(h, 'notities.txt') if menu else None
    if r:
        x, y = ox + r[0] + 20, oy + r[1] + r[3] // 2
        pointer(h, [['move', x, y], ['wait', 150], ['down', 3], ['wait', 60], ['up', 3], ['wait', 1300]])
        popups = h.eval(POPUPS_JS.replace('__PID__', str(win['pid']))) or []
        h.record('explorer.menu.popups', popups)
        shot(h, 'explorer-menu')
        menu = popups[0] if popups else {'x': x, 'y': y}
        pointer(h, [['move', menu['x'] + 90, menu['y'] + 104], ['wait', 250], ['move', menu['x'] + 96, menu['y'] + 104],
                    ['wait', 1200]])
        popups = h.eval(POPUPS_JS.replace('__PID__', str(win['pid']))) or []
        h.record('explorer.openwith.popups', popups)
        h.check('explorer: Openen met opens a submenu', len(popups) >= 2, popups=popups)
        shot(h, 'explorer-openwith')
        h.s.native_app('explorer', {'native': 'key', 'key': 'Escape'})
        h.sleep(0.5)
    elif menu:
        h.check('explorer: notities.txt in the list', False)
    close(h, 'explorer', win)
    return proc


# ---- alles --------------------------------------------------------------------------------------------------
def band(h, win):
    """The strip under Alles's command bar, in screen coordinates (x, y, w, h), from the dev state."""
    for n in nodes(state(h, 'alles')):
        if 'alles-results' in (n.get('classes') or []) and n.get('rect'):
            x, y, w, _ = n['rect']
            return [win['x'] + x, win['y'] + y, w, 24]
    return None


def alles(h, roots):
    rows = len(h.s.rows())
    proc = spawn(h, 'alles', ['win11-app', '--app', 'alles'], {'WIN11SHELL_INDEX_ROOTS': str(roots)})
    boot(h, 'alles', rows)
    win = window(h, 'io.github.win11shell.Search')
    h.record('alles.window', win)
    h.sleep(3)
    h.eval(ACTIVATE_JS.replace('__PID__', str(win['pid'])))
    h.sleep(1.5)
    start = band(h, win)
    h.record('alles.band', start)
    one_colour(h, shot(h, 'alles-start'), start, 'alles: no dots under the command bar at start')
    h.s.native_app('alles', {'native': 'type', 'name': 'alles-search', 'text': 'a'})
    h.sleep(2.5)
    shot(h, 'alles-results')
    h.s.native_app('alles', {'native': 'type', 'name': 'alles-search', 'text': ''})
    h.sleep(2.5)
    cleared = band(h, win)
    h.record('alles.band.cleared', cleared)
    one_colour(h, shot(h, 'alles-cleared'), cleared, 'alles: no dots under the command bar after clearing')
    close(h, 'alles', win)
    return proc


def run(h):
    home = h.hm.RUN / 'home'
    folder = home / 'KitUi'
    folder.mkdir(parents=True, exist_ok=True)
    (folder / 'Projecten').mkdir(exist_ok=True)
    (folder / 'notities.txt').write_text('hallo\n')
    (folder / 'rapport.pdf').write_bytes(b'%PDF-1.4\n' + b'0' * 48000)
    (folder / 'groot-archief.zip').write_bytes(b'PK' + b'\0' * 2_500_000)
    (folder / 'tabel.csv').write_text('a,b\n1,2\n')
    (folder / 'een-bestand-met-een-heel-lange-naam-die-niet-past.txt').write_text('x' * 900)
    roots = home / 'KitUiIndex'
    roots.mkdir(exist_ok=True)
    for name in ('alpha.txt', 'beta.txt', 'gamma.txt', 'delta.md'):
        (roots / name).write_text(name)
    h.sleep(3)
    for step in (lambda: alles(h, roots), lambda: explorer(h, folder)):
        try:
            step()
        except Exception as error:   # noqa: BLE001 - one part must not cost the other
            h.check('kit-ui: run', False, error=f'{type(error).__name__}: {error}')
    set_theme(h, 'light')
    h.sleep(2)
    try:
        explorer(h, folder, light=True)
    except Exception as error:   # noqa: BLE001
        h.check('kit-ui: light run', False, error=f'{type(error).__name__}: {error}')
    set_theme(h, 'dark')
