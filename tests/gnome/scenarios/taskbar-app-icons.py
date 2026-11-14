"""Icons of our own apps (13-11): each of the seven native apps shows its own icon on the taskbar, in Start (All
apps and search) and in GNOME's own UI, and Start lists each of them once.

The windows of Task Manager, Alles and Default apps showed the generic program icon on the taskbar (Adwaita's
blue diamond), Disks GNOME Disks' icon and the Installer a parcel; Start showed Explorer as GNOME Files, Disks as
GNOME Disks and the Installer as a parcel, and listed "Win11 Verkenner", "Win11 Standaardapps" and "Win11
App-installatie" next to our own entries. The windows' app ids are io.github.win11shell.<Name>; scripts/install.py
writes a desktop entry per app with that StartupWMClass, several NoDisplay, with an Icon= that is a file of the
payload (the bundled Win11 icons, BUNDLED_ICONS there; Settings and Default apps ui/assets/icons/settings.svg), so
that GNOME finds it under any icon theme. The private session runs the stock icon theme (no
org.gnome.desktop.interface icon-theme of its own: Adwaita), which has none of the theme names except
drive-harddisk. The catalogue (apps.list) gives its own apps (win11shell-<kind>.desktop) those entries' icons and
names the entry (`entry`), and leaves our entries themselves out (host/src/apps/catalog.rs).

Per app this launches the app (the launcher as Start does: win11-app --app <kind>, win11-explorer, ...), waits
for its boot report and a sized window, focuses it and then records:

  entry     the extension's window JSON (appId, desktopId, icon = the desktop entry's Icon= through GNOME's
            window tracker)
  button    the taskbar button the window got (the shell's dev state: tb-app-<key>, its tooltip, its rect)
  shot      a crop of the taskbar around the app buttons (taskbar-<kind>.png), at the end one with all seven
            apps running (taskbar-all.png), Task View over them (taskview.png) and Start (start.png)

and once: the icon theme, the backend's apps.list entries of our own apps (id, entry, icon) and what the backend's
/icon route answers for every icon name involved (status, bytes, the same bytes as the generic fallback or not),
as a contact sheet (icon-route.png: one cell per name, in the order of records.iconRoute).

Checks: per app the focused button, the window's icon (the entry's Icon=, through GNOME's window tracker) and the
icon the button asked for (the shell's dev state: the image's "w11-icon"); the /icon route resolves the theme names
of our apps (the bundled Win11 icons, host images/icons.rs) and is not the generic fallback; the catalogue's own
apps have their entry's icon and our entries are not listed; Start's All apps has each own app once with that icon
(start-allapps.png) and a search for Verkenner finds it once (start-search.png).

Run (from G):
    tools/heavy.sh --min-mb 2560 --tag taskbar-icons -- python3 -B tests/gnome/harness.py launch run --lang nl \\
        --build-dir tests/build/icons-release --source worktree --scenario tests/gnome/scenarios/taskbar-app-icons.py
"""
import hashlib
import http.client
import json
import os
import subprocess
from pathlib import Path

APPS = (
    # kind, argv after the prefix's bin dir, GApplication id (= the desktop entry's name and StartupWMClass)
    ('explorer', ['win11-explorer', '__HOME__'], 'io.github.win11shell.Explorer'),
    ('settings', ['win11-app', '--app', 'settings', '--params', json.dumps({'page': 'home'})], 'io.github.win11shell.Settings'),
    ('taskmgr', ['win11-app', '--app', 'taskmgr'], 'io.github.win11shell.TaskManager'),
    ('alles', ['win11-app', '--app', 'alles'], 'io.github.win11shell.Search'),
    ('disks', ['win11-app', '--app', 'disks'], 'io.github.win11shell.Disks'),
    ('defaultapps', ['win11-default-apps'], 'io.github.win11shell.DefaultApps'),
    ('installer', ['win11-installer'], 'io.github.win11shell.Installer'),
)
# Every name the taskbar, Start or Task View may ask /icon for our apps: the entries' Icon= values, the
# catalogue's own lists (host/src/apps/catalog-data.json) and the generic fallback.
ICON_NAMES = (
    'utilities-system-monitor', 'system-search', 'system-software-install', 'drive-harddisk', 'system-file-manager',
    'preferences-desktop-default-applications', 'org.gnome.DiskUtility', 'package-x-generic', 'win11shell-taskmgr',
    'io.github.win11shell.TaskManager', 'application-x-executable',
)

# The Icon= scripts/install.py writes per app, below the payload (prefix/share/win11-gnome)
ENTRY_ICONS = {
    'explorer': 'icons/Win11/apps/scalable/system-file-manager.svg',
    'settings': 'ui/assets/icons/settings.svg',
    'taskmgr': 'icons/Win11/apps/scalable/utilities-system-monitor.svg',
    'alles': 'icons/Win11/apps/scalable/system-search.svg',
    'disks': 'icons/Win11/devices/scalable/drive-harddisk.svg',
    'defaultapps': 'ui/assets/icons/settings.svg',
    'installer': 'icons/Win11/apps/scalable/system-software-install.svg',
}
# /icon must resolve these (no generic fallback): the names of our apps; drive-harddisk is in Adwaita itself
ROUTE_NAMES = ('utilities-system-monitor', 'system-search', 'system-software-install', 'drive-harddisk',
               'system-file-manager')
# our entries that are visible (not NoDisplay): the catalogue used to list them as apps of their own
VISIBLE_ENTRIES = ('io.github.win11shell.Explorer.desktop', 'io.github.win11shell.DefaultApps.desktop',
                   'io.github.win11shell.Installer.desktop')

WINDOWS_JS = ("(()=>{const Meta=imports.gi.Meta;return global.get_window_actors().map(a=>a.meta_window)"
              ".filter(w=>(w.get_wm_class()==='__ID__'||w.get_gtk_application_id()==='__ID__')"
              "&&w.get_window_type()===Meta.WindowType.NORMAL).map(w=>{const r=w.get_frame_rect();"
              "return {pid:w.get_pid(),title:w.get_title(),x:r.x,y:r.y,w:r.width,h:r.height};});})()")
# the extension's own window JSON (what windows.list / windows.changed give the taskbar)
WINDOW_JSON_JS = ("(()=>{const b=Main.extensionManager.lookup('__UUID__').stateObj._panelBridge;"
                  "return b._userWindows(true).filter(w=>w.get_pid()===__PID__).map(w=>{const j=b._windowJSON(w);"
                  "return {id:j.id,appId:j.appId,desktopId:j.desktopId,icon:j.icon,title:j.title};});})()")
ICON_THEME_JS = ("(()=>{const Gio=imports.gi.Gio;const s=new Gio.Settings({schema_id:'org.gnome.desktop.interface'});"
                 "return {iconTheme:s.get_string('icon-theme'),user:!!s.get_user_value('icon-theme'),"
                 "desktop:imports.gi.GLib.getenv('XDG_CURRENT_DESKTOP')};})()")
ACTIVATE_JS = ("(()=>{const ws=global.get_window_actors().map(a=>a.meta_window).filter(w=>w.get_pid()===__PID__);"
               "ws.forEach(w=>{w.unminimize();Main.activateWindow(w);});return ws.length;})()")
CLOSE_JS = ("(()=>{const ws=global.get_window_actors().map(a=>a.meta_window).filter(w=>w.get_pid()===__PID__);"
            "ws.forEach(w=>w.delete(global.get_current_time()));return ws.length;})()")


def gone(pid):
    """Ended: no /proc entry, or a zombie (reaped here: the harness is the session's subreaper)."""
    try:
        os.waitpid(pid, os.WNOHANG)
    except OSError:
        pass
    try:
        stat = Path(f'/proc/{pid}/stat').read_text()
    except OSError:
        return True
    return stat[stat.rindex(')') + 2:].startswith('Z')


def descendants(root):
    parents = {}
    for entry in Path('/proc').iterdir():
        if entry.name.isdigit():
            try:
                stat = (entry / 'stat').read_text()
                parents[int(entry.name)] = int(stat[stat.rindex(')') + 2:].split()[1])
            except (OSError, ValueError):
                pass
    out, todo = [], [root]
    while todo:
        pid = todo.pop()
        for child, parent in parents.items():
            if parent == pid:
                out.append(child)
                todo.append(child)
    return out


def spawn(h, kind, argv):
    bindir = h.hm.RUN / 'prefix' / 'bin'
    argv = [str(bindir / argv[0]), *[str(h.hm.RUN / 'home') if a == '__HOME__' else a for a in argv[1:]]]
    env = dict(os.environ, WAYLAND_DISPLAY=h.hm.WL, GDK_BACKEND='wayland')
    log = open(h.hm.RUN / 'logs' / f'app-{kind}.log', 'a')
    return subprocess.Popen(argv, env=env, stdout=log, stderr=subprocess.STDOUT)


# ---- the shell's backend (win11-panel --http-only, native/shell/src/backend.rs) ------------------------------
def endpoint(h):
    """(port, token) of the native shell's backend: it writes them to $XDG_RUNTIME_DIR/win11-gnome/
    native-backend-<shell pid>.json."""
    pid = h.native_pid
    path = h.hm.RT / 'win11-gnome' / f'native-backend-{pid}.json'
    data = json.loads(path.read_text())
    return int(data['port']), data['token']


def call(h, method, args=None):
    port, token = endpoint(h)
    body = json.dumps({'args': args or {}})
    conn = http.client.HTTPConnection('127.0.0.1', port, timeout=15)
    try:
        conn.request('POST', f'/api/call/{method}', body, {'Content-Type': 'application/json', 'X-Shell-Token': token,
                                                         'X-Shell-Lang': h.lang})
        return json.loads(conn.getresponse().read() or b'null')
    finally:
        conn.close()


def get(h, path):
    port, token = endpoint(h)
    sep = '&' if '?' in path else '?'
    conn = http.client.HTTPConnection('127.0.0.1', port, timeout=15)
    try:
        conn.request('GET', f'{path}{sep}token={token}')
        response = conn.getresponse()
        return response.status, response.getheader('Content-Type') or '', response.read()
    finally:
        conn.close()


def enc(s):
    from urllib.parse import quote
    return quote(s, safe='-_.~')


def icon_route(h):
    """What /icon answers for each name, with and without the taskbar's generic fallback; one contact sheet."""
    GdkPixbuf, GLib = h.s.GdkPixbuf, h.s.GLib
    rows, cells = [], []
    generic = get(h, '/icon?name=application-x-executable&size=64')[2]
    for name in ICON_NAMES:
        status, kind, data = get(h, f'/icon?name={enc(name)}&size=64')
        fb_status, _, fb_data = get(h, f'/icon?name={enc(name)}&size=64&fallback=application-x-executable')
        rows.append({'name': name, 'status': status, 'type': kind, 'bytes': len(data),
                     'sha': hashlib.sha1(data).hexdigest()[:12] if status == 200 else None,
                     'withFallback': fb_status, 'isGenericFallback': fb_status == 200 and fb_data == generic})
        cells.append(fb_data if fb_status == 200 else b'')
    size, pad = 64, 8
    sheet = GdkPixbuf.Pixbuf.new(GdkPixbuf.Colorspace.RGB, True, 8, (size + pad) * len(cells) + pad, size + 2 * pad)
    sheet.fill(0x303030ff)
    for i, data in enumerate(cells):
        if not data:
            continue
        try:
            loader = GdkPixbuf.PixbufLoader()
            loader.set_size(size, size)
            loader.write(data)
            loader.close()
            pix = loader.get_pixbuf()
            pix = pix.scale_simple(size, size, GdkPixbuf.InterpType.BILINEAR)
            pix.composite(sheet, pad + i * (size + pad), pad, size, size, pad + i * (size + pad), pad, 1, 1,
                          GdkPixbuf.InterpType.BILINEAR, 255)
        except GLib.Error as error:
            rows[i]['decodeError'] = str(error)
    name = 'icon-route.png'
    sheet.savev(str(h.hm.RUN / 'shots' / name), 'png', [], [])
    h.s.shots.append({'file': name, 'group': 'scenario', 'bytes': (h.hm.RUN / 'shots' / name).stat().st_size})
    h.results['shots'].append(name)
    return rows


# ---- the taskbar -------------------------------------------------------------------------------------------
def walk(node, out):
    if isinstance(node, dict):
        out.append(node)
        for child in node.get('children') or []:
            walk(child, out)
    return out


def first_icon(node):
    """The icon of the first image below a dev-state node (native/shell/src/dev.rs image_icon)."""
    for n in walk(node, []):
        if n.get('icon'):
            return n['icon']
    return None


def own_icon(h, kind, icon):
    """Whether `icon` is the Icon= install.py gives `kind`: that file of the payload."""
    return isinstance(icon, str) and icon.endswith(f'/share/win11-gnome/{ENTRY_ICONS[kind]}') and Path(icon).is_file()


def start_nodes(h):
    """The nodes of Start's (visible) window."""
    out = []
    for top in h.native_state():
        if top.get('visible'):
            nodes = walk(top.get('tree'), [])
            if any(n.get('name') == 'start-window' for n in nodes) or any(str(n.get('name') or '').startswith('start-') for n in nodes):
                out += nodes
    return out


def buttons(h):
    """The taskbar's app buttons: key (from the widget name tb-app-<key>), tooltip, classes, screen rect and the
    icon the button asked for."""
    out = []
    for top in h.native_state():
        if not top.get('visible'):
            continue
        nodes = walk(top.get('tree'), [])
        found = [n for n in nodes if str(n.get('name') or '').startswith('tb-app-')]
        if not found:
            continue
        # the panel window is bottom-aligned: its own coordinates plus the room above it
        dy = h.hm.HEIGHT - (top.get('height') or h.hm.HEIGHT)
        for n in found:
            x, y, w, ht = n.get('rect') or [0, 0, 0, 0]
            out.append({'key': n['name'][len('tb-app-'):], 'tooltip': n.get('label'),
                        'classes': [c for c in n.get('classes') or [] if c.startswith('is-')],
                        'rect': [x, y + dy, w, ht], 'icon': first_icon(n)})
    return out


def taskbar_shot(h, name, rows):
    """A crop of the taskbar around its app buttons (the whole strip when none is known)."""
    top = h.hm.HEIGHT - h.hm.TASKBAR
    if rows:
        x0 = max(0, min(r['rect'][0] for r in rows) - 120)
        x1 = min(h.hm.WIDTH, max(r['rect'][0] + r['rect'][2] for r in rows) + 24)
        area = (x0, top, x1 - x0, h.hm.TASKBAR)
    else:
        area = (0, top, h.hm.WIDTH, h.hm.TASKBAR)
    try:
        return h.screenshot(name, area=area, settle=0.5)
    except Exception as error:   # noqa: BLE001 - one bad shot must not cost the others
        h.check(f'screenshot {name}', False, error=str(error))
        return None


def one(h, kind, argv, app_id, running):
    rows_before = len(h.s.rows())
    proc = spawn(h, kind, argv)
    try:
        h.s.native_app_boot(kind, rows_before, 45)
    except TimeoutError as error:
        h.check(f'{kind}: boot report', False, error=str(error), exit=proc.poll())
    try:
        wins = h.until(lambda: [w for w in h.eval(WINDOWS_JS.replace('__ID__', app_id)) or [] if w['w'] > 0] or None,
                       f'{kind} window', 30, 0.3)
    except TimeoutError as error:
        h.check(f'{kind}: window', False, error=str(error))
        proc.kill()
        return None
    pid = wins[0]['pid']
    running.append({'kind': kind, 'pid': pid, 'proc': proc})
    h.eval(ACTIVATE_JS.replace('__PID__', str(pid)))
    h.sleep(2.5)   # windows.changed, the catalogue match and the icon fetch
    entry = h.eval(WINDOW_JSON_JS.replace('__UUID__', h.hm.UUID).replace('__PID__', str(pid)))
    h.record(f'{kind}.window', entry)
    rows = buttons(h)
    h.record(f'{kind}.buttons', rows)
    focused = [r for r in rows if 'is-focused' in r['classes']]
    h.check(f'{kind}: a focused taskbar button', len(focused) == 1, focused=focused)
    # GNOME's icon for the window (Alt+Tab, the overview): the entry's Icon=, a file of the payload
    h.check(f'{kind}: the window has its entry\'s icon', bool(entry) and all(own_icon(h, kind, e.get('icon')) for e in entry),
            entry=entry, want=ENTRY_ICONS[kind])
    # what the shell chose (panel/taskbar/apps.rs: the window's entry icon for our apps)
    h.check(f'{kind}: the taskbar button asks for that icon', len(focused) == 1 and own_icon(h, kind, focused[0].get('icon')),
            icon=focused[0].get('icon') if focused else None, want=ENTRY_ICONS[kind])
    taskbar_shot(h, f'taskbar-{kind}', rows)
    return pid


def start_lists(h):
    """Start's All apps and a search: each own app once, with its entry's icon; none of our entries itself."""
    h.native({'native': 'click', 'name': 'start-all-apps'})
    h.sleep(1.5)
    nodes = start_nodes(h)
    rows = {str(n['name'])[len('start-app-'):]: n for n in nodes if str(n.get('name') or '').startswith('start-app-')}
    h.record('start.allApps', sorted(rows))
    h.screenshot('start-allapps', settle=0.5)
    for kind, _, _ in APPS:
        row = rows.get(f'win11shell-{kind}.desktop')
        h.check(f'start: {kind} in All apps with its entry\'s icon', bool(row) and own_icon(h, kind, first_icon(row)),
                icon=first_icon(row) if row else None, want=ENTRY_ICONS[kind])
    h.check('start: none of our entries in All apps as an app of its own',
            not [i for i in rows if i.startswith('io.github.win11shell.') and i != 'io.github.win11shell.Calendar.desktop'],
            listed=sorted(i for i in rows if 'win11shell' in i))
    h.native({'native': 'click', 'name': 'start-all-back'})
    h.sleep(0.8)
    query = 'verkenner' if h.lang == 'nl' else 'file explorer'
    name = 'Verkenner' if h.lang == 'nl' else 'File Explorer'
    typed = h.native({'native': 'type', 'name': 'start-search', 'text': query})
    h.sleep(2)
    results = []
    for n in start_nodes(h):
        if str(n.get('name') or '').startswith('start-result-') and 'is-app' in (n.get('classes') or []):
            titles = [c.get('label') for c in walk(n, []) if 'w11-sr-title' in (c.get('classes') or [])]
            results.append({'name': n['name'], 'title': titles[0] if titles else None, 'icon': first_icon(n)})
    h.record('start.search', {'query': query, 'typed': typed, 'apps': results})
    h.screenshot('start-search', settle=0.5)
    found = [r for r in results if r['title'] == name]
    h.check(f'start: a search for {query} finds {name} once, with its entry\'s icon',
            len(found) == 1 and own_icon(h, 'explorer', found[0]['icon'])
            and not [r for r in results if (r['title'] or '').startswith('Win11 ')], results=results)
    h.native({'native': 'type', 'name': 'start-search', 'text': ''})
    h.sleep(0.5)


def run(h):
    h.sleep(3)
    try:
        h.record('iconTheme', h.eval(ICON_THEME_JS))
    except Exception as error:   # noqa: BLE001
        h.record('iconTheme', {'error': str(error)})
    try:
        listed = call(h, 'apps.list')
        own = [{'id': a.get('id'), 'entry': a.get('entry'), 'icon': a.get('icon'), 'own': a.get('own'),
                'name': a.get('name')} for a in (listed.get('result') or []) if 'win11shell' in str(a.get('id'))]
        h.record('catalogue.own', own)
        by_id = {a['id']: a for a in own}
        for kind, argv, app_id in APPS:
            item = by_id.get(f'win11shell-{kind}.desktop') or {}
            h.check(f'catalogue: {kind} has its entry and the entry\'s icon',
                    item.get('own') is True and item.get('entry') == f'{app_id}.desktop' and own_icon(h, kind, item.get('icon')),
                    item=item, want=ENTRY_ICONS[kind])
        ours = sorted(i for i in by_id if i.startswith('io.github.win11shell.'))
        h.check('catalogue: our desktop entries are not listed as apps of their own',
                not set(ours) & set(VISIBLE_ENTRIES) and all(i == 'io.github.win11shell.Calendar.desktop' for i in ours),
                listed=ours)
    except Exception as error:   # noqa: BLE001
        h.check('catalogue', False, error=f'{type(error).__name__}: {error}')
    try:
        rows = icon_route(h)
        h.record('iconRoute', rows)
        by_name = {r['name']: r for r in rows}
        for name in ROUTE_NAMES:
            row = by_name.get(name) or {}
            h.check(f'icon route: {name}', row.get('status') == 200 and not row.get('isGenericFallback'), row=row)
    except Exception as error:   # noqa: BLE001
        h.check('icon route', False, error=f'{type(error).__name__}: {error}')
    taskbar_shot(h, 'taskbar-before', buttons(h))
    running = []
    try:
        for kind, argv, app_id in APPS:
            try:
                one(h, kind, argv, app_id, running)
            except Exception as error:   # noqa: BLE001 - one app must not cost the others
                h.check(f'{kind}: run', False, error=f'{type(error).__name__}: {error}')
        h.sleep(1)
        rows = buttons(h)
        h.record('all.buttons', rows)
        taskbar_shot(h, 'taskbar-all', rows)
        try:
            h.native({'native': 'flyout', 'name': 'taskview'})
            h.sleep(2)
            h.screenshot('taskview', settle=0.5)
            h.native({'native': 'flyout', 'name': 'taskview'})
            h.sleep(1)
        except Exception as error:   # noqa: BLE001
            h.check('taskview shot', False, error=str(error))
        try:
            h.open('start')
            h.sleep(2)
            h.screenshot('start', settle=0.5)
            start_lists(h)
            h.close('start')
        except Exception as error:   # noqa: BLE001
            h.check('start', False, error=f'{type(error).__name__}: {error}')
    finally:
        for app in running:
            pid = app['pid']
            kids = descendants(pid)
            try:
                h.eval(CLOSE_JS.replace('__PID__', str(pid)))
                h.until(lambda: gone(pid) and all(gone(k) for k in kids), f'{app["kind"]} exits', 15, 0.3)
            except Exception:   # noqa: BLE001 - only processes this scenario started
                for k in [pid, *kids]:
                    if not gone(k):
                        try:
                            os.kill(k, 9)
                        except OSError:
                            pass
            if app['proc'].poll() is None:
                app['proc'].kill()
