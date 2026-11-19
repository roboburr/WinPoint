"""Native shell smoke (D-G63, D-G70): win11-shell, the GTK 4 taskbar with its flyouts, Start and the desktop in one
process without WebKit. Since D-G70 it is the only shell: the harness waits for its boot report before a scenario
runs (harness.py Session.run), so there is no switch any more.

Robin (2026-11-12): "ultracode, meer bouwen, minder review, minder testen, opschieten"; (2026-11-13) "ik wil de GTK4
versie op github hebben, verwijder de GTK3 variant". win11-panel, started by the extension, execs win11-shell, which
starts its backend (win11-panel --http-only) as a child; extension/frostedBridge.js holds one pid with several
identities.

Run (from G; a build dir with the host binaries plus win11-shell and win11-calendar copied from the native
workspace's target):
    tools/heavy.sh --min-mb 2560 --tag native -- python3 -B tests/gnome/harness.py launch run --lang nl \\
        --build-dir tests/build/native-release --source worktree --scenario tests/gnome/scenarios/native-shell.py

  A. The boot: the harness's native boot row names a pid whose process is win11-shell; its backend win11-panel
     --http-only is its child; no win11-start or win11-desktop process runs and no WebKit process is a descendant of
     win11-shell.
  B. The native state (dev channel {"native":"state"}): a taskbar window, and the widgets tb-start, tb-clock and tb-sys.
     GNOME sees a Panel window of the native pid; the extension holds the native pid with the Panel identity.
  C. Flyouts quick and notifications open and close ({"native":"flyout"}), with screenshots.
  D. Start through a click on tb-start: a Start window of the same pid; the extension holds both identities under
     that pid; a second click closes it.
  N. Memory of the native process tree (win11-shell and its backend), PSS + SwapPss and Anonymous + Swap as
     gtk-native.md measures (recorded; the WebKit comparison of D-G63 is history: 443 / 269 MiB).
"""
import json
import os

BRIDGE = "Main.extensionManager.lookup('win11-gnome@win11shell.local').stateObj._panelBridge"
HOSTS_JS = (f"(()=>{{const f={BRIDGE}._frostedBridge;return Object.fromEntries([...f._hosts].map(([p,ids])=>"
            "[String(p),[...ids]]));})()")
CAPS_JS = f"JSON.stringify({BRIDGE}._frostedBridge.capabilities())"
# The WebKit hosts of before D-G70: none may run
LEGACY_HOSTS = ('win11-start', 'win11-desktop')
PANEL = 'io.github.win11shell.Panel'
START = 'io.github.win11shell.Start'
WIDGETS = ('tb-start', 'tb-clock', 'tb-sys')


# ---- memory -------------------------------------------------------------------------------------------------
def measure(pid):
    v = {}
    with open(f'/proc/{pid}/smaps_rollup') as f:
        for line in f:
            key, _, rest = line.partition(':')
            if key in ('Pss', 'SwapPss', 'Anonymous', 'Swap'):
                v[key] = int(rest.split()[0])
    return {'pssKb': v.get('Pss', 0) + v.get('SwapPss', 0), 'anonKb': v.get('Anonymous', 0) + v.get('Swap', 0)}


def tree_memory(h, roots):
    """Sum over the roots and all their descendants (each pid once), plus the rows per process."""
    table = h.hm.proc_table()
    pids = []
    for root in roots:
        for pid in [root] + h.hm.descendants(root, table):
            if pid not in pids:
                pids.append(pid)
    rows, pss, anon = [], 0, 0
    for pid in pids:
        try:
            m = measure(pid)
        except OSError:
            continue
        rows.append({'pid': pid, 'comm': table.get(pid, {}).get('comm'), **m})
        pss += m['pssKb']
        anon += m['anonKb']
    return {'pssMiB': round(pss / 1024, 1), 'anonMiB': round(anon / 1024, 1), 'processes': rows}


def http_only(pid):
    try:
        with open(f'/proc/{pid}/cmdline', 'rb') as f:
            return b'--http-only' in f.read().split(b'\0')
    except OSError:
        return False


def zombie(pid):
    try:
        with open(f'/proc/{pid}/stat') as f:
            stat = f.read()
        return stat[stat.rindex(')') + 2:].split()[0] in ('Z', 'X')
    except (OSError, ValueError, IndexError):
        return True


def legacy_hosts(h):
    """win11-start or win11-desktop processes among this harness's descendants (D-G70: there must be none)."""
    table = h.hm.proc_table()
    return [{'pid': pid, 'comm': table[pid]['comm']} for pid in h.hm.descendants(os.getpid(), table)
            if table.get(pid, {}).get('comm') in LEGACY_HOSTS and not zombie(pid)]


def webkit_children(h, pid):
    """WebKit processes below win11-shell (D-G70: none; the shell has no WebView)."""
    table = h.hm.proc_table()
    return [{'pid': p, 'comm': table[p]['comm']} for p in h.hm.descendants(pid, table)
            if table.get(p, {}).get('comm', '').startswith('WebKit') and not zombie(p)]


def settle(h, seconds=4.0):
    h.sleep(seconds)


# ---- the native dev channel ---------------------------------------------------------------------------------
def native(h, command, timeout=15):
    """One dev command for win11-shell ({"native": ...}); its report row's value (harness Session.native)."""
    return h.s.native(command, timeout)


def names(node, out=None):
    out = set() if out is None else out
    if isinstance(node, dict):
        if node.get('name'):
            out.add(node['name'])
        for child in node.get('children') or []:
            names(child, out)
    return out


def state(h):
    value = native(h, {'native': 'state'})
    return value if isinstance(value, list) else []


def gnome_window(h, wm, pid):
    return next((w for w in h.s.windows() if w['wm'] == wm and w['pid'] == pid), None)


def start_shown(h, pid):
    w = gnome_window(h, START, pid)
    return bool(w and w['visible'] and not w['minimized'])


def run(h):
    # A. The boot the harness waited for: win11-shell with its backend, no WebKit shell host.
    settle(h)
    boot = h.record('A.boot', h.s.facts.get('nativeBoot'))
    h.require('A: win11-shell reported its boot', bool(boot and boot.get('pid')))
    pid = boot['pid']
    table = h.hm.proc_table()
    h.check('A: the panel process is win11-shell', table.get(pid, {}).get('comm') == 'win11-shell',
            pid=pid, comm=table.get(pid, {}).get('comm'))
    backend = [p for p in h.hm.descendants(pid, table) if table[p]['comm'] == 'win11-panel' and http_only(p)]
    h.record('A.backend', backend)
    h.check('A: its backend win11-panel --http-only is a child of win11-shell', bool(backend), backend=backend)
    legacy = legacy_hosts(h)
    h.check('A: no win11-start or win11-desktop process runs', not legacy, legacy=legacy)
    webkit = webkit_children(h, pid)
    h.check('A: no WebKit process below win11-shell', not webkit, webkit=webkit)

    # B. Taskbar window and widgets.
    try:
        tops = h.until(lambda: (lambda s: s if any(names(w.get('tree')) >= set(WIDGETS) for w in s) else None)(state(h)),
                       'native taskbar widgets', 30, 0.5)
    except TimeoutError:
        tops = state(h)
    h.record('B.toplevels', [{k: w.get(k) for k in ('title', 'visible', 'width', 'height')} for w in tops])
    bar = next((w for w in tops if set(WIDGETS) <= names(w.get('tree'))), None)
    h.check('B: a visible taskbar window', bool(bar and bar.get('visible')),
            titles=[w.get('title') for w in tops])
    found = names(bar.get('tree')) if bar else set()
    for widget in WIDGETS:
        h.check(f'B: the taskbar has {widget}', widget in found)
    try:
        panel_window = h.until(lambda: gnome_window(h, PANEL, pid), 'GNOME sees the native Panel window', 15, 0.3)
    except TimeoutError:
        panel_window = None
    h.check('B: GNOME sees a Panel window of the native pid', bool(panel_window), window=panel_window)
    try:
        hosts = h.until(lambda: (lambda v: v if PANEL in v.get(str(pid), []) else None)(h.eval(HOSTS_JS)),
                        'frosted registration of the panel', 10, 0.3)
    except TimeoutError:
        hosts = h.eval(HOSTS_JS)
    h.check('B: the extension registered the native pid as Panel', PANEL in hosts.get(str(pid), []), hosts=hosts)
    h.screenshot('b-native-taskbar', area=(0, h.hm.HEIGHT - 120, h.hm.WIDTH, 120))
    h.screenshot('b-native-desktop')

    # C. Flyouts.
    for flyout in ('quick', 'notifications'):
        opened = native(h, {'native': 'flyout', 'name': flyout})
        h.record(f'C.{flyout}.open', opened)
        h.check(f'C: the {flyout} flyout opens', isinstance(opened, dict) and opened.get('open') == flyout,
                result=opened)
        h.screenshot(f'c-{flyout}-open', settle=0.8)
        closed = native(h, {'native': 'flyout', 'name': flyout})
        h.record(f'C.{flyout}.close', closed)
        h.check(f'C: the {flyout} flyout closes again', isinstance(closed, dict) and closed.get('open') is None,
                result=closed)

    # D. Start through the taskbar button.
    clicked = native(h, {'native': 'click', 'name': 'tb-start'})
    h.record('D.click', clicked)
    try:
        shown = h.until(lambda: start_shown(h, pid), 'native Start shown', 10, 0.2)
    except TimeoutError:
        shown = False
    h.check('D: tb-start opens a Start window of the same pid', shown, click=clicked)
    hosts = h.eval(HOSTS_JS)
    h.check('D: the extension holds Panel and Start under one pid',
            {PANEL, START} <= set(hosts.get(str(pid), [])), hosts=hosts)
    caps = json.loads(h.eval(CAPS_JS))
    kinds = sorted({w['kind'] for w in caps.get('windows', []) if w.get('pid') == pid})
    h.record('D.capabilities', caps)
    h.check('D: the backdrop capabilities list the taskbar and Start of that pid', {'taskbar', 'start'} <= set(kinds),
            kinds=kinds)
    h.screenshot('d-native-start-open', settle=0.8)
    h.record('D.visibleToplevels', [{k: w.get(k) for k in ('title', 'width', 'height')} for w in state(h)
                                    if w.get('visible')])
    native(h, {'native': 'click', 'name': 'tb-start'})
    try:
        hidden = h.until(lambda: not start_shown(h, pid), 'native Start hidden', 10, 0.2)
    except TimeoutError:
        hidden = False
    h.check('D: a second click closes Start', hidden)
    h.screenshot('d-native-start-closed')

    # N. Memory of the native tree (recorded, no comparison: there is no WebKit shell any more).
    settle(h)
    mem = h.record('N.native', {'root': pid, **tree_memory(h, [pid])})
    h.check('N: the native tree was measured', mem['pssMiB'] > 0 and bool(mem['processes']),
            pssMiB=mem['pssMiB'], anonMiB=mem['anonMiB'])

    integrated_parity(h)

    failed = [c['check'] for c in h.results['checks'] if not c['ok']]
    h.require('every check passed', not failed, failed=failed)



def integrated_parity(h):
    """Explicit S2/S4/S6 practical acceptance from the integration handoff, all in the private session."""
    import cairo
    h.open('settings:home')
    old = h.backend('settings', 'settings.get')
    def put(path, value):
        reply = h.backend('settings', 'settings.set', {'path': path, 'value': value})
        h.require(f'parity: {path} accepted', not (isinstance(reply, dict) and reply.get('error')), reply=reply)
    def task_buttons():
        return sorted({n['name'] for n in h.hm.tree_nodes(h.native_state())
                       if str(n.get('name', '')).startswith('tb-app-')
                       and 'taskmgr' in str(n['name']).lower()})
    manager = None
    app_path = '/org/gnome/desktop/notifications/application/org-gnome-nautilus/'
    notify_schema = 'org.gnome.desktop.notifications.application'
    expanded_before = h.gsettings_get(notify_schema, 'force-expanded', app_path)
    base_workspace = h.eval('global.workspace_manager.get_active_workspace_index()')
    try:
        # S2: an unpinned window really moves between desktops; no synthetic window list is substituted.
        put('taskbar.allDesktops', False)
        manager = h.open('taskmgr')
        shown = h.until(task_buttons, 'task manager on the current desktop', 15)
        h.check('S2: an unpinned Task Manager appears on the current desktop taskbar', bool(shown), buttons=shown)
        target = base_workspace + 1
        moved = h.eval('(()=>{const m=global.workspace_manager;'
                       f'while(m.n_workspaces<={target})m.append_new_workspace(false,global.get_current_time());'
                       f'const w=global.get_window_actors().map(a=>a.meta_window).find(w=>w.get_pid()==={manager.pid});'
                       f'if(!w)return false;w.change_workspace(m.get_workspace_by_index({target}));'
                       f'm.get_workspace_by_index({base_workspace}).activate(global.get_current_time());'
                       'return w.get_workspace().index();})()')
        h.check('S2: the real Task Manager window moved to the other desktop', moved == target, workspace=moved)
        h.until(lambda: not task_buttons(), 'other desktop window absent from taskbar', 15)
        h.check('S2: current-desktop mode hides the other desktop window', not task_buttons())
        put('taskbar.allDesktops', True)
        h.until(task_buttons, 'all-desktops mode shows the window', 15)
        h.check('S2: all-desktops mode shows the other desktop window', bool(task_buttons()))
        put('taskbar.allDesktops', False)
        h.until(lambda: not task_buttons(), 'current-desktop mode hides it again', 15)
        h.eval(f'global.workspace_manager.get_workspace_by_index({target}).activate(global.get_current_time())||true')
        h.until(task_buttons, 'taskbar follows the active desktop', 15)
        h.check('S2: changing to that desktop shows its Task Manager button', bool(task_buttons()))
        h.screenshot(f's2-current-desktop.{h.lang}')
        h.eval(f'global.workspace_manager.get_workspace_by_index({base_workspace}).activate(global.get_current_time())||true')
        h.close('taskmgr')
        manager = None

        # S4: GNOME policy -> expanded payload -> actual GTK body allocation beyond three lines.
        h.gsettings_set(notify_schema, 'force-expanded', 'true', app_path)
        h.sleep(.5)
        body = '\n'.join(f'Expanded notification line {i}' for i in range(1, 9))
        code = """(async()=>{const MT=await import('resource:///org/gnome/shell/ui/messageTray.js');
const Shell=(await import('gi://Shell')).default;
const app=Shell.AppSystem.get_default().lookup_app('org.gnome.Nautilus.desktop');
if(!app)return {error:'missing private fixture app'};
const source=new MT.Source({title:app.get_name(),policy:MT.NotificationPolicy.newForApp(app)});
Main.messageTray.add(source);
const n=new MT.Notification({source,title:'Expanded integration test',body:BODY});source.addNotification(n);
return {expanded:source.policy.forceExpanded};})()"""
        sent = h.eval(code.replace('BODY', json.dumps(body)))
        h.require('S4: GNOME app policy requests expanded banners', sent.get('expanded') is True, sent=sent)
        def body_node():
            return next((n for n in h.hm.tree_nodes([top for top in h.native_state() if top.get('visible')])
                         if n.get('type') == 'GtkLabel' and n.get('visible', True) and n.get('label') == body and n.get('rect', [0,0,0,0])[3] > 0), None)
        label = h.until(body_node, 'the expanded toast body', 8)
        h.check('S4: expanded toast actually allocates more than three lines', label['rect'][3] >= 90, rect=label['rect'])
        h.screenshot(f's4-expanded-toast.{h.lang}', settle=.2)

        # S6: inspect the actual private desktop pixels, not only the resolver's unit result.
        light = h.hm.RUN / 'home' / 'parity-light.png'
        dark = h.hm.RUN / 'home' / 'parity-dark.png'
        for path, level in ((light, .8), (dark, .12)):
            surface = cairo.ImageSurface(cairo.FORMAT_RGB24, 32, 32)
            cr = cairo.Context(surface); cr.set_source_rgb(level, level, level); cr.paint()
            surface.write_to_png(str(path))
        put('wallpaper', {'type':'image','path':str(light),'pathDark':str(dark),
                          'fit':'fill','followTheme':True,'color':'#000000'})
        settings_pid = h.surface('settings').pid
        h.eval(f'global.get_window_actors().map(a=>a.meta_window).filter(w=>w.get_pid()==={settings_pid}).forEach(w=>w.minimize())||true')
        for mode, expected_light in (('light', True), ('dark', False)):
            put('theme.mode', mode)
            h.sleep(2)
            shot = h.screenshot(f's6-wallpaper-{mode}.{h.lang}', settle=.3)
            pixels = h.s.pixel_stats(shot, (h.hm.WIDTH//2, h.hm.HEIGHT//3, 20, 20))
            h.check(f'S6: {mode} mode paints the matching image of the pair',
                    190 <= pixels['meanLevel'] <= 220 if expected_light else 20 <= pixels['meanLevel'] <= 45, pixels=pixels)
        # S3: both the native toplevel and Mutter's actual title follow a live language change.
        for language in (('en' if h.lang == 'nl' else 'nl'), h.lang):
            put('language', language)
            expected_title = 'Taskbar' if language == 'en' else 'Taakbalk'
            def titles_match():
                bar = next((top for top in h.native_state() if set(WIDGETS) <= names(top.get('tree'))), {})
                wm = gnome_window(h, PANEL, h.s.facts['nativeBoot']['pid']) or {}
                return bar.get('title') == expected_title and wm.get('title') == expected_title
            h.until(titles_match, 'taskbar title follows live language', 15)
            h.check(f'S3: native and compositor taskbar titles follow live {language}', titles_match())
    finally:
        h.eval(f'global.workspace_manager.get_workspace_by_index({base_workspace}).activate(global.get_current_time())||true')
        if manager:
            h.close('taskmgr')
        h.gsettings_set(notify_schema, 'force-expanded', expanded_before or 'false', app_path)
        put('taskbar.allDesktops', old['taskbar']['allDesktops'])
        put('wallpaper', old['wallpaper'])
        put('theme.mode', old['theme']['mode'])
        put('language', old['language'])
        h.close('settings')
