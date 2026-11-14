"""Native GTK 4 apps (13-11): every app starts through the host's switch as win11-gtk-<kind>, with no WebKit.

Robin (2026-11-13): "zet de apps ook om naar GTK 4". The launchers stay as they were (win11-explorer, win11-app --app
<kind>, win11-default-apps, win11-installer); the host execs the native app (host/src/lib.rs native_app_switch),
which starts `win11-app --app <kind> --http-only` as its backend (native/APPS.md). Per app this checks:

  boot      the app's dev boot report (native/kit dev.rs, windowId native-<kind>)
  window    a normal window with the app's id, on screen, a sensible size
  process   the window's pid runs win11-gtk-<kind>; its backend child runs win11-app --http-only; no
            WebKitWebProcess or WebKitNetworkProcess anywhere under it
  state     the dev state lists widgets with text (the app rendered something, not an empty frame)
  shot      a screenshot of the app focused over the desktop (dark, then light for a few)
  memory    PSS and anonymous memory of the app and everything under it, after a short settle; then the same
            for the WebKit apps (WIN11_WEBKIT_APPS=1) of Explorer, Settings and Task Manager, for comparison
  use       a short interaction per app through the kit dev channel (native/kit/src/dev.rs: state, click by widget
            name, type, key) and the real pointer, each with a check and a screenshot (after the memory sample):
              explorer     double-click on a folder opens it, Back returns; a click and Enter open it, Alt+Left returns
              settings     two pages from the navigation (Systeem, Persoonlijke instellingen): their title shows
              taskmgr      Processen and Prestaties from the navigation: their table / device list shows
              disks        with a UDisks2 stand-in on the private system bus (one USB disk, one NTFS volume): the
                           volume row shows, Eigenschappen opens its page, Back returns (no stand-in: Vernieuwen)
              defaultapps  a file type row opens "Een app kiezen", Escape closes it
              installer    the empty state: the drop zone and "Nog geen installaties"
              alles        (its own launch with a small index root) a typed query lists the matching files
  procs     Explorer, Settings and Alles running, then Task Manager › Processen: each app is one row of the Apps
            group in its dev state (its backend `win11-app --http-only` is no row of its own), and the backend's
            sysmon.processes groups the window and its backend under one app key
  close     closing the window ends the app and its backend

Run (from G):
    tools/heavy.sh --min-mb 2560 --tag native-gtk-apps -- python3 -B tests/gnome/harness.py launch run --lang nl \\
        --build-dir tests/build/native-release --source worktree --scenario tests/gnome/scenarios/native-gtk-apps.py
"""
import json
import os
import subprocess
import urllib.request
from pathlib import Path

APPS = (
    # kind, argv after the prefix's bin dir, GApplication id
    ('explorer', ['win11-explorer', '__HOME__'], 'io.github.win11shell.Explorer'),
    ('settings', ['win11-app', '--app', 'settings', '--params', json.dumps({'page': 'home'})], 'io.github.win11shell.Settings'),
    ('taskmgr', ['win11-app', '--app', 'taskmgr'], 'io.github.win11shell.TaskManager'),
    ('alles', ['win11-app', '--app', 'alles'], 'io.github.win11shell.Search'),
    ('disks', ['win11-app', '--app', 'disks'], 'io.github.win11shell.Disks'),
    ('defaultapps', ['win11-default-apps'], 'io.github.win11shell.DefaultApps'),
    ('installer', ['win11-installer'], 'io.github.win11shell.Installer'),
)
LIGHT = ('explorer', 'settings', 'taskmgr')

WINDOWS_JS = ("(()=>{const Meta=imports.gi.Meta;return global.get_window_actors().map(a=>a.meta_window)"
              ".filter(w=>(w.get_wm_class()==='__ID__'||w.get_gtk_application_id()==='__ID__')"
              "&&w.get_window_type()===Meta.WindowType.NORMAL).map(w=>{const r=w.get_frame_rect();"
              "return {pid:w.get_pid(),title:w.get_title(),x:r.x,y:r.y,w:r.width,h:r.height,"
              "minimized:w.minimized,focus:w.has_focus()};});})()")
ACTIVATE_JS = ("(()=>{const ws=global.get_window_actors().map(a=>a.meta_window).filter(w=>w.get_pid()===__PID__);"
               "ws.forEach(w=>{w.unminimize();Main.activateWindow(w);});return ws.length;})()")
MINIMIZE_JS = ("(()=>{const ws=global.get_window_actors().map(a=>a.meta_window).filter(w=>w.get_pid()===__PID__);"
               "ws.forEach(w=>w.minimize());return ws.length;})()")
CLOSE_JS = ("(()=>{const ws=global.get_window_actors().map(a=>a.meta_window).filter(w=>w.get_pid()===__PID__);"
            "ws.forEach(w=>w.delete(global.get_current_time()));return ws.length;})()")
# A Clutter virtual pointer in the private shell (real Wayland input), as kit-ui.py: steps ["move", x, y] |
# ["down", button] | ["up", button] | ["wait", ms]; 30 ms between steps.
POINTER_JS = (
    "(async()=>{const Clutter=(await import('gi://Clutter')).default;const GLib=(await import('gi://GLib')).default;"
    "const wait=ms=>new Promise(r=>GLib.timeout_add(GLib.PRIORITY_DEFAULT,ms,()=>{r();return GLib.SOURCE_REMOVE;}));"
    "const seat=Clutter.get_default_backend().get_default_seat();"
    "globalThis.__ngaPointer??=seat.create_virtual_device(Clutter.InputDeviceType.POINTER_DEVICE);"
    "const d=globalThis.__ngaPointer,us=()=>GLib.get_monotonic_time();"
    "for(const s of __STEPS__){if(s[0]==='move')d.notify_absolute_motion(us(),s[1],s[2]);"
    "else if(s[0]==='down')d.notify_button(us(),s[1],Clutter.ButtonState.PRESSED);"
    "else if(s[0]==='up')d.notify_button(us(),s[1],Clutter.ButtonState.RELEASED);"
    "await wait(s[0]==='wait'?s[1]:30);}"
    "return global.get_pointer().slice(0,2);})()")
# A UDisks2 stand-in for Disks (the private system bus has no UDisks): org.freedesktop.UDisks2 with one USB drive,
# a GPT table and one unmounted NTFS partition (host/src/services/disks/reference-tests.json "normal"), answering
# only ObjectManager.GetManagedObjects; every other call fails, so nothing can be formatted or mounted. It runs
# as a child of this harness on the sandbox's private system bus (allow own="*") and is stopped after Disks.
UDISKS_OBJECTS = {
    '/org/freedesktop/UDisks2/drives/WinPoint_Test': {'org.freedesktop.UDisks2.Drive': {
        'Vendor': 'WinPoint', 'Model': 'Testschijf', 'SortKey': '01winpoint', 'RotationRate': 0, 'ConnectionBus': 'usb',
        'Removable': True, 'Ejectable': True, 'CanPowerOff': True, 'MediaAvailable': True, 'Size': 536870912}},
    '/org/freedesktop/UDisks2/block_devices/sdx': {
        'org.freedesktop.UDisks2.Block': {'Device': '/dev/sdx', 'PreferredDevice': '/dev/sdx',
                                          'Drive': '/org/freedesktop/UDisks2/drives/WinPoint_Test', 'Size': 536870912,
                                          'IdType': '', 'IdUsage': ''},
        'org.freedesktop.UDisks2.PartitionTable': {'Type': 'gpt',
                                                   'Partitions': ['/org/freedesktop/UDisks2/block_devices/sdx1']}},
    '/org/freedesktop/UDisks2/block_devices/sdx1': {
        'org.freedesktop.UDisks2.Block': {'Device': '/dev/sdx1', 'PreferredDevice': '/dev/sdx1',
                                          'Drive': '/org/freedesktop/UDisks2/drives/WinPoint_Test', 'Size': 134217728,
                                          'IdType': 'ntfs', 'IdUsage': 'filesystem', 'IdLabel': 'TESTDATA'},
        'org.freedesktop.UDisks2.Filesystem': {'MountPoints': []},
        'org.freedesktop.UDisks2.Partition': {'Table': '/org/freedesktop/UDisks2/block_devices/sdx', 'Number': 1,
                                              'Offset': 1048576, 'Size': 134217728,
                                              'Type': 'ebd0a0a2-b9e5-4433-87c0-68b6b72699c7',
                                              'UUID': '11111111-2222-3333-4444-555555555555'}},
    '/org/freedesktop/UDisks2/Manager': {'org.freedesktop.UDisks2.Manager': {
        'SupportedFilesystems': ['ntfs', 'exfat', 'vfat', 'ext4', 'btrfs', 'xfs'],
        'SupportedEncryptionTypes': ['luks1', 'luks2'], 'DefaultEncryptionType': 'luks2', 'Version': '2.10.1'}},
}
UDISKS = {}
UDISKS_MOCK = r"""
import json, sys, warnings
warnings.simplefilter('ignore', DeprecationWarning)
from gi.repository import Gio, GLib
OBJECTS = json.loads(sys.argv[1])
SIG = {'Device': 'ay', 'PreferredDevice': 'ay', 'MountPoints': 'aay', 'Drive': 'o', 'Table': 'o', 'Partitions': 'ao',
       'Size': 't', 'Offset': 't', 'Number': 'u', 'RotationRate': 'i'}
def variant(key, value):
    sig = SIG.get(key) or ('b' if isinstance(value, bool) else 't' if isinstance(value, int) else
                           'as' if isinstance(value, list) else 's')
    if sig == 'ay':
        value = value.encode() + b'\0'
    elif sig == 'aay':
        value = [v.encode() + b'\0' for v in value]
    return GLib.Variant(sig, value)
MANAGED = GLib.Variant('(a{oa{sa{sv}}})', ({path: {iface: {k: variant(k, v) for k, v in props.items()}
                                                   for iface, props in ifaces.items()}
                                            for path, ifaces in OBJECTS.items()},))
XML = ('<node><interface name="org.freedesktop.DBus.ObjectManager"><method name="GetManagedObjects">'
       '<arg type="a{oa{sa{sv}}}" direction="out"/></method></interface></node>')
def call(conn, sender, path, iface, method, params, invocation):
    if method == 'GetManagedObjects':
        invocation.return_value(MANAGED)
    else:
        invocation.return_dbus_error('org.freedesktop.UDisks2.Error.NotSupported', 'test stand-in')
bus = Gio.bus_get_sync(Gio.BusType.SYSTEM, None)
bus.register_object('/org/freedesktop/UDisks2', Gio.DBusNodeInfo.new_for_xml(XML).interfaces[0], call, None, None)
loop = GLib.MainLoop()
Gio.bus_own_name_on_connection(bus, 'org.freedesktop.UDisks2', Gio.BusNameOwnerFlags.NONE,
                               lambda *a: print('owned', flush=True), lambda *a: loop.quit())
loop.run()
"""
# the folder Explorer opens (made in the private home by run) and the file inside it
FOLDER, INSIDE = 'Testmap', 'binnen.txt'
# Task Manager's names of the apps the procs step runs (host sysmon/process.rs own_names, nl)
PROC_APPS = (('explorer', 'Verkenner'), ('settings', 'Instellingen'), ('alles', 'Alles'), ('taskmgr', 'Taakbeheer'))


def proc_comm(pid):
    try:
        return Path(f'/proc/{pid}/comm').read_text().strip()
    except OSError:
        return None


def proc_cmdline(pid):
    try:
        return Path(f'/proc/{pid}/cmdline').read_bytes().split(b'\0')
    except OSError:
        return []


def gone(pid):
    """Ended: no /proc entry, or a zombie. A backend outlives its app by milliseconds and is then reparented to
    this harness process (the session's subreaper), which reaps it here; /proc/<pid>/comm stays readable for a
    zombie, so comm alone would call it alive."""
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
    """Every process under ``root`` (by /proc/*/stat parent links)."""
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


def memory(pids):
    """Summed PSS and anonymous + swap in MiB of these processes (smaps_rollup)."""
    pss = anon = 0
    for pid in pids:
        try:
            for line in Path(f'/proc/{pid}/smaps_rollup').read_text().splitlines():
                key, _, rest = line.partition(':')
                kib = int(rest.split()[0]) if rest.split() and rest.split()[0].isdigit() else 0
                if key == 'Pss':
                    pss += kib
                elif key in ('Anonymous', 'Swap'):
                    anon += kib
        except OSError:
            pass
    return {'pss': round(pss / 1024, 1), 'anonSwap': round(anon / 1024, 1), 'processes': len(pids)}


def walk(node, out):
    if isinstance(node, dict):
        out.append(node)
        for child in node.get('children') or []:
            walk(child, out)
    return out


def texts(state):
    found = []
    for top in state if isinstance(state, list) else []:
        for node in walk(top.get('tree'), []):
            text = node.get('label') or node.get('text')
            if text and node.get('visible', True):
                found.append(text)
    return found


def set_theme(h, mode):
    path = h.hm.RUN / 'config' / 'win11-gnome' / 'settings.json'
    try:
        data = json.loads(path.read_text() or '{}')
    except (OSError, ValueError):
        data = {}
    data.setdefault('theme', {})['mode'] = mode
    path.write_text(json.dumps(data, indent=2))


def spawn(h, kind, argv, webkit=False, extra=None):
    bindir = h.hm.RUN / 'prefix' / 'bin'
    argv = [str(bindir / argv[0]), *[str(h.hm.RUN / 'home') if a == '__HOME__' else a for a in argv[1:]]]
    env = dict(os.environ, WAYLAND_DISPLAY=h.hm.WL, GDK_BACKEND='wayland', **(extra or {}))
    if webkit:
        env['WIN11_WEBKIT_APPS'] = '1'
    log = open(h.hm.RUN / 'logs' / f'app-{kind}.log', 'a')
    return subprocess.Popen(argv, env=env, stdout=log, stderr=subprocess.STDOUT)


def shot(h, name, settle=1.5):
    try:
        return h.screenshot(name, settle=settle)
    except Exception as error:   # noqa: BLE001 - one bad shot must not cost the others
        h.check(f'screenshot {name}', False, error=str(error))
        return None


def webkit_memory(h, kind, argv, app_id):
    """The WebKit app of this kind (WIN11_WEBKIT_APPS=1), measured the same way, then closed."""
    proc = spawn(h, kind, argv, webkit=True)
    try:
        wins = h.until(lambda: h.eval(WINDOWS_JS.replace('__ID__', app_id)) or None, f'{kind} WebKit window', 40, 0.3)
    except TimeoutError as error:
        h.check(f'{kind} (WebKit): window', False, error=str(error))
        proc.kill()
        return
    pid = wins[0]['pid']
    h.sleep(8)
    pids = [pid, *descendants(pid)]
    used = memory(pids)
    h.record(f'memory.webkit.{kind}', {**used, 'comms': sorted({proc_comm(k) for k in pids} - {None})})
    h.eval(CLOSE_JS.replace('__PID__', str(pid)))
    try:
        h.until(lambda: gone(pid), f'{kind} WebKit exits', 15, 0.3)
    except TimeoutError:
        for k in pids:
            try:
                os.kill(k, 9)
            except OSError:
                pass


# ---- interaction (the "use" step) ------------------------------------------------------------------------------
def pointer(h, steps):
    return h.eval(POINTER_JS.replace('__STEPS__', json.dumps(steps)))


def nodes(state):
    out = []
    for top in state if isinstance(state, list) else []:
        walk(top.get('tree'), out)
    return out


def subtree(state, name):
    """The nodes under (and including) the first widget called ``name``."""
    for n in nodes(state):
        if n.get('name') == name:
            return walk(n, [])
    return []


def dev(h, kind, command, timeout=20):
    return h.s.native_app(kind, command, timeout)


def view(h, kind):
    return dev(h, kind, {'native': 'state'})


def wait_view(h, kind, test, label, timeout=15):
    """Poll the app's dev state until ``test(state)`` is truthy; returns (value, last state)."""
    last = {'state': None}

    def probe():
        last['state'] = view(h, kind)
        return test(last['state']) or None
    try:
        return h.until(probe, label, timeout, 0.8), last['state']
    except TimeoutError:
        return None, last['state']


def labelled(state, text, cls=None):
    """Visible nodes whose label is ``text`` (and that carry class ``cls``)."""
    return [n for n in nodes(state) if n.get('label') == text and n.get('visible', True)
            and (cls is None or cls in (n.get('classes') or []))]


def item(state, text):
    rows = [n for n in labelled(state, text, 'xp-item-name') if n.get('rect')]
    return rows[0] if rows else None


def centre(win, node):
    x, y, w, hh = node['rect']
    return win['x'] + x + w // 2, win['y'] + y + hh // 2


def use_explorer(h, pid, win):
    """A folder opens by double-click and Back returns; a click and Enter open it again and Alt+Left returns."""
    found, state = wait_view(h, 'explorer', lambda s: item(s, FOLDER), f'{FOLDER} in the list')
    h.check(f'explorer use: {FOLDER} is listed', bool(found), labels=texts(state)[:40])
    if not found:
        return
    x, y = centre(win, found)
    pointer(h, [['move', x, y], ['wait', 200], ['down', 1], ['wait', 40], ['up', 1], ['wait', 60],
                ['down', 1], ['wait', 40], ['up', 1], ['wait', 300]])
    opened, state = wait_view(h, 'explorer', lambda s: item(s, INSIDE) and not item(s, FOLDER),
                              f'{FOLDER} opened by double-click')
    h.check(f'explorer use: a double-click opens {FOLDER}', bool(opened),
            crumbs=[n.get('label') for n in nodes(state) if str(n.get('name') or '').startswith('crumb-')])
    shot(h, 'explorer-use-open', settle=0.8)
    home = lambda s: item(s, FOLDER) if not item(s, INSIDE) else None   # noqa: E731 - the folder's node, at home
    answer = dev(h, 'explorer', {'native': 'click', 'name': 'explorer-back'})
    back, _ = wait_view(h, 'explorer', home, 'Back to the home folder')
    h.check('explorer use: Back returns to the home folder', bool(back), answer=answer)
    shot(h, 'explorer-use-back', settle=0.8)
    if not back:
        return
    # select with one click, then Enter (main.js onKey: Enter opens the selection)
    x, y = centre(win, back)
    pointer(h, [['move', x, y], ['wait', 200], ['down', 1], ['wait', 40], ['up', 1], ['wait', 600]])
    pressed = dev(h, 'explorer', {'native': 'key', 'name': 'explorer-items', 'key': 'Return'})
    entered, _ = wait_view(h, 'explorer', lambda s: item(s, INSIDE) and not item(s, FOLDER), f'{FOLDER} opened by Enter')
    h.check(f'explorer use: Enter opens the selected {FOLDER}', bool(entered), key=pressed)
    shot(h, 'explorer-use-enter', settle=0.8)
    pressed = dev(h, 'explorer', {'native': 'key', 'name': 'explorer-items', 'key': '<Alt>Left'})
    left, _ = wait_view(h, 'explorer', home, 'Alt+Left to the home folder')
    h.check('explorer use: Alt+Left returns to the home folder', bool(left), key=pressed)


def use_settings(h, pid, win):
    """Two pages from the navigation pane: the page title follows."""
    for nav, title, name in (('nav-system', 'Systeem', 'settings-use-system'),
                             ('nav-personalization', 'Persoonlijke instellingen', 'settings-use-personalization')):
        answer = dev(h, 'settings', {'native': 'click', 'name': nav})
        shown, state = wait_view(h, 'settings', lambda s, t=title: labelled(s, t, 'st-title'), f'Settings page {title}')
        h.check(f'settings use: {nav} opens {title}', bool(shown), answer=answer,
                titles=[n.get('label') for n in nodes(state) if 'st-title' in (n.get('classes') or [])])
        shot(h, name, settle=1.0)


def use_taskmgr(h, pid, win):
    """Processen and Prestaties from the navigation pane."""
    for nav, widget, name in (('nav-processes', 'taskmgr-processes', 'taskmgr-use-processes'),
                              ('nav-performance', 'taskmgr-perf-list', 'taskmgr-use-performance')):
        answer = dev(h, 'taskmgr', {'native': 'click', 'name': nav})
        shown, state = wait_view(h, 'taskmgr', lambda s, w=widget: len(subtree(s, w)) > 3, f'Task Manager {widget}')
        h.check(f'taskmgr use: {nav} shows {widget}', bool(shown), answer=answer,
                labels=[n.get('label') for n in subtree(state, widget) if n.get('label')][:30])
        h.sleep(1)
        shot(h, name, settle=1.0)


def use_disks(h, pid, win):
    """With the UDisks stand-in: the disk card lists its volume; Eigenschappen opens the volume page, Back returns.
    Without it (it did not start): Vernieuwen brings the "UDisks is niet beschikbaar" state back."""
    named = lambda s, n: [k for k in nodes(s) if k.get('name') == n and k.get('visible', True)]   # noqa: E731
    row, state = wait_view(h, 'disks', lambda s: named(s, 'dk-vol-sdx1'), 'the stand-in volume row', 20)
    if not UDISKS.get('proc') or not row:
        h.check('disks use: the stand-in disk is listed', False, mock=bool(UDISKS.get('proc')), labels=texts(state)[:20])
        answer = dev(h, 'disks', {'native': 'click', 'name': 'disks-refresh'})
        shown, state = wait_view(h, 'disks', lambda s: [t for t in texts(s) if 'UDisks' in t], 'Disks state after Vernieuwen')
        h.check('disks use: Vernieuwen shows the UDisks state again', bool(shown), answer=answer, after=texts(state)[:12])
        shot(h, 'disks-use-refresh', settle=0.8)
        return
    h.check('disks use: the stand-in disk is listed', True,
            labels=[t for t in texts(state) if 'Testschijf' in t or 'TESTDATA' in t or 'Schijf' in t][:10])
    shot(h, 'disks-use-overview', settle=0.8)
    answer = dev(h, 'disks', {'native': 'click', 'name': 'dk-props-sdx1'})
    page, state = wait_view(h, 'disks', lambda s: not named(s, 'dk-vol-sdx1') and [t for t in texts(s) if 'TESTDATA' in t],
                            'the volume page')
    h.check('disks use: Eigenschappen opens the volume page', bool(page), answer=answer, labels=texts(state)[:30])
    shot(h, 'disks-use-volume', settle=0.8)
    answer = dev(h, 'disks', {'native': 'click', 'name': 'disks-back'})
    back, _ = wait_view(h, 'disks', lambda s: named(s, 'dk-vol-sdx1'), 'back to the overview')
    h.check('disks use: Back returns to the overview', bool(back), answer=answer)


def start_udisks(h):
    """The UDisks2 stand-in on the private system bus (a child of this harness), once it owns its name."""
    import select
    import time
    log = open(h.hm.RUN / 'logs' / 'udisks-standin.log', 'w')
    proc = subprocess.Popen(['python3', '-I', '-c', UDISKS_MOCK, json.dumps(UDISKS_OBJECTS)], stdout=subprocess.PIPE,
                            stderr=log, text=True)
    deadline = time.monotonic() + 10
    line = ''
    while time.monotonic() < deadline and proc.poll() is None:
        if not select.select([proc.stdout], [], [], 0.5)[0]:
            continue
        line = proc.stdout.readline().strip()
        if line == 'owned':
            UDISKS['proc'] = proc
            h.record('udisks.standin', {'pid': proc.pid, 'objects': sorted(UDISKS_OBJECTS)})
            return proc
    h.check('udisks stand-in owns org.freedesktop.UDisks2', False, exit=proc.poll(), line=line)
    proc.kill()
    return None


def stop_udisks(h):
    proc = UDISKS.pop('proc', None)
    if proc:
        proc.terminate()
        try:
            proc.wait(5)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait(5)


def use_defaultapps(h, pid, win):
    """A file type row opens "Een app kiezen"; Escape closes it."""
    def first_row(s):
        # a type with an app that can open it (the chooser then lists it), else the first type
        rows = [n for n in nodes(s) if str(n.get('name') or '').startswith('defaultapps-type-') and n.get('rect')]
        with_app = [n for n in rows if any('Firefox' in str(k.get('label') or '') for k in walk(n, []))]
        return (with_app or rows or [None])[0]
    def chooser(s):
        """The kit dialog layer ("w11-dialog") with the title "Een app kiezen voor <type>"."""
        titles = [n['label'] for n in nodes(s) if str(n.get('label') or '').startswith('Een app kiezen')
                  and n.get('visible', True)]
        return titles if titles and any(n.get('name') == 'w11-dialog' for n in nodes(s)) else None
    row, _ = wait_view(h, 'defaultapps', first_row, 'a file type row')
    h.check('defaultapps use: file types are listed', bool(row))
    if not row:
        return
    answer = dev(h, 'defaultapps', {'native': 'click', 'name': row['name']})
    opened, state = wait_view(h, 'defaultapps', chooser, 'the app chooser dialog')
    h.check('defaultapps use: a row opens "Een app kiezen"', bool(opened), row=row['name'], answer=answer, titles=opened,
            choices=[n.get('name') for n in nodes(state) if str(n.get('name') or '').startswith('defaultapps-choice-')][:10])
    shot(h, 'defaultapps-use-dialog', settle=1.0)
    pressed = dev(h, 'defaultapps', {'native': 'key', 'key': 'Escape'})
    closed, _ = wait_view(h, 'defaultapps', lambda s: s is not None and not chooser(s), 'the dialog closed')
    h.check('defaultapps use: Escape closes the dialog', bool(closed), key=pressed)
    shot(h, 'defaultapps-use-closed', settle=0.8)


def use_installer(h, pid, win):
    """The empty state: the drop zone, its title and "Nog geen installaties"; no app card or job view."""
    state = view(h, 'installer')
    drop = [n for n in nodes(state) if n.get('name') == 'installer-drop' and n.get('visible', True)]
    detail = [n for n in nodes(state) if n.get('name') in ('installer-detail', 'installer-progress')]
    h.check('installer use: the empty state shows', bool(drop) and bool(labelled(state, 'Sleep een installatiebestand hierheen'))
            and bool(labelled(state, 'Nog geen installaties')) and not detail,
            detail=[n.get('name') for n in detail], labels=texts(state)[:20])
    shot(h, 'installer-use-empty', settle=0.5)


USE = {'explorer': use_explorer, 'settings': use_settings, 'taskmgr': use_taskmgr, 'disks': use_disks,
       'defaultapps': use_defaultapps, 'installer': use_installer}


def window_of(h, app_id, pids=()):
    return h.until(lambda: [w for w in h.eval(WINDOWS_JS.replace('__ID__', app_id)) or [] if w['w'] > 0
                            and w['pid'] not in pids] or None, f'{app_id} window', 30, 0.3)[0]


def close_all(h, pids):
    pointer(h, [['move', h.hm.WIDTH - 3, 330], ['wait', 200]])
    everything = [k for pid in pids for k in [pid, *descendants(pid)]]
    for pid in pids:
        h.eval(CLOSE_JS.replace('__PID__', str(pid)))
    try:
        h.until(lambda: all(gone(k) for k in everything), 'the apps and their backends exit', 20, 0.3)
        return True
    except TimeoutError:
        for k in everything:
            try:
                os.kill(k, 9)
            except OSError:
                pass
        return False


def use_alles(h):
    """Alles with a small index root (WIN11SHELL_INDEX_ROOTS, as kit-ui.py): a typed query lists its files."""
    roots = h.hm.RUN / 'home' / 'AllesIndex'
    roots.mkdir(parents=True, exist_ok=True)
    for name in ('alpha-notes.txt', 'alpha-plan.md', 'beta.txt'):
        (roots / name).write_text(name)
    rows = len(h.s.rows())
    proc = spawn(h, 'alles-use', ['win11-app', '--app', 'alles'], extra={'WIN11SHELL_INDEX_ROOTS': str(roots)})
    try:
        h.s.native_app_boot('alles', rows, 45)
        win = window_of(h, 'io.github.win11shell.Search')
    except TimeoutError as error:
        h.check('alles use: window', False, error=str(error))
        proc.kill()
        return
    h.sleep(3)
    h.eval(ACTIVATE_JS.replace('__PID__', str(win['pid'])))
    h.sleep(1)
    answer = dev(h, 'alles', {'native': 'type', 'name': 'alles-search', 'text': 'alpha'})

    def hits(s):
        names = [t for t in (n.get('label') for n in subtree(s, 'alles-results')) if t and 'alpha' in t]
        return names if any('alpha-notes' in t for t in names) and any('alpha-plan' in t for t in names) else None
    found, state = wait_view(h, 'alles', hits, 'Alles results for alpha', 30)
    h.check('alles use: a typed query lists the matching files', bool(found), answer=answer, hits=found,
            beta=[t for t in texts(state) if 'beta.txt' in t],
            status=[n.get('label') for n in subtree(state, 'alles-status') if n.get('label')])
    shot(h, 'alles-use-query', settle=1.0)
    h.check('alles use: closes', close_all(h, [win['pid']]))


def procs(h):
    """Explorer, Settings and Alles running, then Task Manager › Processen: each app is one row of the Apps group in
    its dev state, and the backend's sysmon.processes groups each window with its backend under one app key."""
    running = {}
    for kind, argv, app_id in APPS:
        if kind not in dict(PROC_APPS):
            continue
        rows = len(h.s.rows())
        proc = spawn(h, f'{kind}-procs', argv)
        try:
            h.s.native_app_boot(kind, rows, 45)
            running[kind] = window_of(h, app_id)['pid']
        except TimeoutError as error:
            h.check(f'procs: {kind} window', False, error=str(error))
            proc.kill()
    h.sleep(4)
    pid = running.get('taskmgr')
    if pid is None:
        close_all(h, list(running.values()))
        return
    h.eval(ACTIVATE_JS.replace('__PID__', str(pid)))
    dev(h, 'taskmgr', {'native': 'click', 'name': 'nav-processes'})
    names = [name for _, name in PROC_APPS]

    def name_labels(s):
        # the GtkLabels only: a row's tooltip (its window title, "Alles" for Alles) is no row of its own
        return [n.get('label') for n in subtree(s, 'taskmgr-processes') if n.get('label') and n.get('type') == 'GtkLabel']

    def counted(s):
        labels = name_labels(s)
        counts = {name: labels.count(name) for name in names}
        return counts if all(counts.values()) else None
    h.sleep(5)
    counts, state = wait_view(h, 'taskmgr', counted, 'every app in the Processen table', 20)
    labels = name_labels(state)
    h.check('procs: each running native app is one row in Processen',
            bool(counts) and all(c == 1 for c in counts.values()),
            counts={name: labels.count(name) for name in names}, labels=labels[:60])
    backend_rows = [k for k in labels if k and ('--http-only' in k or k == 'win11-app')]
    h.check('procs: no backend row of its own in the visible table', not backend_rows, rows=backend_rows)
    shot(h, 'taskmgr-procs', settle=1.0)
    # the same through Task Manager's backend (its instance file: port and token)
    try:
        data = json.loads((h.hm.RT / 'win11-gnome' / f'native-taskmgr-{pid}.json').read_text())
        request = urllib.request.Request(f'http://127.0.0.1:{data["port"]}/api/call/sysmon.processes',
                                         data=json.dumps({'args': {}}).encode(), method='POST',
                                         headers={'X-Shell-Token': data['token'], 'Content-Type': 'application/json'})
        with urllib.request.urlopen(request, timeout=10) as reply:
            result = json.loads(reply.read()).get('result') or {}
        apps = result.get('apps') or []
        host = [g for g in result.get('background') or [] if g.get('key') == 'win11shell']
        for kind, name in PROC_APPS:
            key = f'win11shell-{kind}.desktop'
            groups = [g for g in apps if g.get('key') == key]
            gui = running.get(kind)
            backend = [k for k in descendants(gui) if b'--http-only' in proc_cmdline(k)] if gui else []
            pids = set(groups[0].get('pids') or []) if groups else set()
            if kind == 'taskmgr':
                # this backend answers sysmon.processes itself: as the host it is the background row "Win11 Shell"
                # (process.rs: the host's own pid and its children), not part of Taakbeheer
                inside = bool(backend) and bool(host) and set(backend) <= set(host[0].get('pids') or [])
                what = 'its window, its backend as the host row'
            else:
                inside = bool(backend) and set(backend) <= pids
                what = 'its window and backend'
            h.check(f'procs: sysmon.processes has {kind} once, with {what}',
                    len(groups) == 1 and gui in pids and inside, groups=len(groups),
                    shown=groups[0].get('name') if groups else None, window=gui, backend=backend, pids=sorted(pids),
                    host=host[0].get('pids') if host else None)
        h.record('procs.apps', [{'key': g.get('key'), 'name': g.get('name'), 'pids': g.get('pids')} for g in apps])
    except Exception as error:   # noqa: BLE001
        h.check('procs: sysmon.processes', False, error=f'{type(error).__name__}: {error}')
    h.check('procs: the apps close', close_all(h, list(running.values())))


def one(h, kind, argv, app_id, light=False):
    rows_before = len(h.s.rows())
    proc = spawn(h, kind, argv)
    label = f'{kind}{" (light)" if light else ""}'
    try:
        boot = h.s.native_app_boot(kind, rows_before, 45)
        h.check(f'{label}: boot report', bool(boot), boot=boot)
    except TimeoutError as error:
        h.check(f'{label}: boot report', False, error=str(error), exit=proc.poll())
        boot = None
    try:
        # a window is listed before mutter has given it a size: wait for a sized one
        wins = h.until(lambda: [w for w in h.eval(WINDOWS_JS.replace('__ID__', app_id)) or [] if w['w'] > 0] or None,
                       f'{kind} window', 30, 0.3)
    except TimeoutError as error:
        h.check(f'{label}: window', False, error=str(error))
        proc.kill()
        return
    win = wins[0]
    h.check(f'{label}: window', win['w'] >= 400 and win['h'] >= 300, window=win)
    pid = win['pid']
    comm = proc_comm(pid)
    h.check(f'{label}: native process', comm == f'win11-gtk-{kind}'[:15], comm=comm, pid=pid)
    h.sleep(2.5)
    kids = descendants(pid)
    kid_rows = [{'pid': k, 'comm': proc_comm(k), 'http': b'--http-only' in proc_cmdline(k)} for k in kids]
    h.check(f'{label}: backend child', any(k['http'] for k in kid_rows), children=kid_rows)
    webkit = [k for k in kid_rows if (k['comm'] or '').startswith('WebKit')]
    h.check(f'{label}: no WebKit', not webkit, webkit=webkit)
    try:
        state = h.s.native_app(kind, {'native': 'state'}, 20)
        found = texts(state)
        h.check(f'{label}: rendered', len(found) >= 5, texts=found[:60], windows=len(state or []))
        names = sorted({n.get('name') for top in state or [] for n in walk(top.get('tree'), [])
                        if n.get('name') and not str(n.get('name')).startswith('Gtk')})
        h.record(f'{label}.names', names[:200])
    except Exception as error:   # noqa: BLE001
        h.check(f'{label}: rendered', False, error=str(error))
    h.eval(ACTIVATE_JS.replace('__PID__', str(pid)))
    h.sleep(1.5)
    shot(h, f'{kind}{"-light" if light else ""}')
    if not light:
        h.sleep(4)
        h.record(f'memory.native.{kind}', memory([pid, *descendants(pid)]))
        if kind in USE:
            try:
                USE[kind](h, pid, win)
            except Exception as error:   # noqa: BLE001 - the close below must still run
                h.check(f'{kind} use: run', False, error=f'{type(error).__name__}: {error}')
            # the pointer off the window before it closes (kit-ui.py: a hover while the window goes)
            pointer(h, [['move', h.hm.WIDTH - 3, 330], ['wait', 200]])
        kids = descendants(pid)
    h.eval(CLOSE_JS.replace('__PID__', str(pid)))
    try:
        h.until(lambda: gone(pid), f'{kind} exits after close', 15, 0.3)
        h.until(lambda: all(gone(k) for k in kids), f'{kind} backend exits', 15, 0.3)
        h.check(f'{label}: closes', True)
    except TimeoutError as error:
        h.check(f'{label}: closes', False, error=str(error), left=[k for k in kids if not gone(k)])
        for k in [pid, *kids]:
            try:
                os.kill(k, 9)
            except OSError:
                pass


def run(h):
    h.record('boot', h.s.facts.get('nativeBoot'))
    folder = h.hm.RUN / 'home' / FOLDER
    folder.mkdir(parents=True, exist_ok=True)
    (folder / INSIDE).write_text('hallo\n')
    h.sleep(3)
    for kind, argv, app_id in APPS:
        try:
            if kind == 'disks':
                start_udisks(h)
            one(h, kind, argv, app_id)
        except Exception as error:   # noqa: BLE001 - one app must not cost the others
            h.check(f'{kind}: run', False, error=f'{type(error).__name__}: {error}')
        finally:
            stop_udisks(h)
    for step in (use_alles, procs):
        try:
            step(h)
        except Exception as error:   # noqa: BLE001
            h.check(f'{step.__name__}: run', False, error=f'{type(error).__name__}: {error}')
    set_theme(h, 'light')
    h.sleep(2)
    for kind, argv, app_id in APPS:
        if kind in LIGHT:
            try:
                one(h, kind, argv, app_id, light=True)
            except Exception as error:   # noqa: BLE001
                h.check(f'{kind} (light): run', False, error=f'{type(error).__name__}: {error}')
    set_theme(h, 'dark')
    h.sleep(2)
    for kind, argv, app_id in APPS:
        if kind in LIGHT:
            try:
                webkit_memory(h, kind, argv, app_id)
            except Exception as error:   # noqa: BLE001
                h.check(f'{kind} (WebKit): run', False, error=f'{type(error).__name__}: {error}')
