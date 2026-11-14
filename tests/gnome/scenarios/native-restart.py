"""Native restart (lifecycle, 13-11): `shell.restartAll` (Windows Update) restarts every native program with the
new build, each with a new backend, and the windows come back.

The backends of the native programs (win11-shell's `win11-panel --http-only`, each win11-gtk-<kind>'s `win11-app
--app <kind> --http-only`) do not re-exec themselves: once their waits are over they leave `<instance>.restart` and
exit with 75 (host/src/lib.rs run_http_only); the GUI saves its state and execs itself (native/kit/src/app.rs,
native/shell/src/main.rs). This scenario:

  A. starts Explorer (a window with two tabs and a second window with one), Settings (Start page) and Task Manager
     in the private session and records, per GUI and for the shell: pid, /proc/<pid>/exe, the backend child and that
     it answers shell.info
  B. replaces the installed binaries of those programs and their backends by rename, as scripts/install.py does,
     so every running image shows "<path> (deleted)"
  C. calls shell.restartAll {reason:'libraries'} the way Settings' update page does: on Settings' own backend,
     with the token of its instance file
  D. checks per GUI and for the shell: a new dev boot report from the same pid, /proc/<pid>/exe is the installed
     binary again (not deleted), a NEW backend child that answers shell.info (with mainLoopLagMs: the backends run a
     GLib main loop), the window is back; Explorer has both windows again with their tabs (2 and 1: the restart
     file, transfer.rs; window-state.json alone would bring back one window), Settings (the origin) is on Update en
     herstel (its window title: the nav pane shows that name on every page), the shell has its taskbar and GNOME sees
     its Panel window
  E. no process is left behind: no old backend, no zombie, no win11-* process outside the expected trees
  G. a backend killed with SIGKILL (Task Manager's, the shell's): the GUI starts a new one that answers and keeps
     running (no re-exec)
  F. closing the apps ends them and their new backends; no instance, restart note or Explorer restart file is left

Run (from G; a build dir with all 19 binaries):
    tools/heavy.sh --min-mb 2560 --tag native-restart -- python3 -B tests/gnome/harness.py launch run --lang nl \\
        --build-dir tests/build/lifecycle-release --source worktree --scenario tests/gnome/scenarios/native-restart.py
"""
import json
import os
import re
import shutil
import subprocess
import urllib.request
from pathlib import Path

APPS = (
    # kind, argv after the prefix's bin dir, GApplication id, native binary
    ('explorer', ['win11-explorer', '__HOME__'], 'io.github.win11shell.Explorer', 'win11-gtk-explorer'),
    ('settings', ['win11-app', '--app', 'settings', '--params', json.dumps({'page': 'home'})],
     'io.github.win11shell.Settings', 'win11-gtk-settings'),
    ('taskmgr', ['win11-app', '--app', 'taskmgr'], 'io.github.win11shell.TaskManager', 'win11-gtk-taskmgr'),
)
# what the restart replaces: the GUIs and the backends
REPLACED = ('win11-gtk-explorer', 'win11-gtk-settings', 'win11-gtk-taskmgr', 'win11-shell', 'win11-panel', 'win11-app')
PANEL = 'io.github.win11shell.Panel'
WINDOWS_JS = ("(()=>{const Meta=imports.gi.Meta;return global.get_window_actors().map(a=>a.meta_window)"
              ".filter(w=>(w.get_wm_class()==='__ID__'||w.get_gtk_application_id()==='__ID__')"
              "&&w.get_window_type()===Meta.WindowType.NORMAL).map(w=>{const r=w.get_frame_rect();"
              "return {pid:w.get_pid(),title:w.get_title(),x:r.x,y:r.y,w:r.width,h:r.height};});})()")
CLOSE_JS = ("(()=>{const ws=global.get_window_actors().map(a=>a.meta_window).filter(w=>w.get_pid()===__PID__);"
            "ws.forEach(w=>w.delete(global.get_current_time()));return ws.length;})()")
TAB = re.compile(r'^tab-\d+$')
# the origin Settings' window title on the update page (registry.rs "update", kit frame: "<page> – <app>")
UPDATE_TITLE = {'nl': 'Update en herstel – Instellingen', 'en': 'Update & recovery – Settings'}


# ---- processes -------------------------------------------------------------------------------------------------
def stat(pid):
    try:
        text = Path(f'/proc/{pid}/stat').read_text()
    except OSError:
        return None
    fields = text[text.rindex(')') + 2:].split()
    return {'comm': text[text.index('(') + 1:text.rindex(')')], 'state': fields[0], 'ppid': int(fields[1])}


def gone(pid):
    """Ended: no /proc entry, or a zombie (reaped here if it is ours: the harness is the session's subreaper)."""
    try:
        os.waitpid(pid, os.WNOHANG)
    except OSError:
        pass
    row = stat(pid)
    return row is None or row['state'] in ('Z', 'X')


def exe(pid):
    try:
        return os.readlink(f'/proc/{pid}/exe')
    except OSError:
        return None


def cmdline(pid):
    try:
        return Path(f'/proc/{pid}/cmdline').read_bytes().split(b'\0')
    except OSError:
        return []


def table():
    out = {}
    for entry in Path('/proc').iterdir():
        if entry.name.isdigit():
            row = stat(int(entry.name))
            if row:
                out[int(entry.name)] = row
    return out


def backends(pid):
    """The live --http-only children of a GUI."""
    return [p for p, row in table().items() if row['ppid'] == pid and row['state'] not in ('Z', 'X')
            and b'--http-only' in cmdline(p)]


# ---- the backend API -------------------------------------------------------------------------------------------
def instance_file(h, kind, pid):
    name = 'native-backend' if kind == 'shell' else f'native-{kind}'
    return h.hm.RT / 'win11-gnome' / f'{name}-{pid}.json'


def api(h, kind, pid, method, args=None, timeout=10):
    data = json.loads(instance_file(h, kind, pid).read_text())
    request = urllib.request.Request(f'http://127.0.0.1:{data["port"]}/api/call/{method}',
                                     data=json.dumps({'args': args or {}}).encode(), method='POST',
                                     headers={'X-Shell-Token': data['token'], 'Content-Type': 'application/json'})
    with urllib.request.urlopen(request, timeout=timeout) as reply:
        return data['pid'], json.loads(reply.read())


def answers(h, kind, pid):
    """The backend named by the GUI's instance file: its pid and its shell.info (or the error)."""
    try:
        backend, reply = api(h, kind, pid, 'shell.info')
        info = reply.get('result') or {}
        return {'pid': backend, 'ok': reply.get('ok') is True, 'mainLoopLagMs': info.get('mainLoopLagMs')}
    except Exception as error:   # noqa: BLE001
        return {'pid': None, 'ok': False, 'error': f'{type(error).__name__}: {error}'}


# ---- the native dev channel ------------------------------------------------------------------------------------
def walk(node, out):
    if isinstance(node, dict):
        out.append(node)
        for child in node.get('children') or []:
            walk(child, out)
    return out


def nodes(state):
    return [n for top in state if isinstance(state, list) for n in walk(top.get('tree'), [])]


def tabs(h):
    state = h.s.native_app('explorer', {'native': 'state'}, 20)
    return sorted(n['name'] for n in nodes(state) if TAB.match(str(n.get('name') or '')) and n.get('visible', True))


def window_tabs(h):
    """The visible tabs per Explorer window (the dev state has one top per window), sorted."""
    state = h.s.native_app('explorer', {'native': 'state'}, 20)
    counts = [len([n for n in walk(top.get('tree'), []) if TAB.match(str(n.get('name') or '')) and n.get('visible', True)])
              for top in state if isinstance(state, list) and top.get('visible', True)]
    return sorted(counts)


def labels(h, kind):
    state = h.s.native_app(kind, {'native': 'state'}, 20)
    return [n.get('label') for n in nodes(state) if n.get('label') and n.get('visible', True)]


def spawn(h, kind, argv):
    bindir = h.hm.RUN / 'prefix' / 'bin'
    argv = [str(bindir / argv[0]), *[str(h.hm.RUN / 'home') if a == '__HOME__' else a for a in argv[1:]]]
    env = dict(os.environ, WAYLAND_DISPLAY=h.hm.WL, GDK_BACKEND='wayland')
    log = open(h.hm.RUN / 'logs' / f'app-{kind}.log', 'a')
    return subprocess.Popen(argv, env=env, stdout=log, stderr=subprocess.STDOUT)


def windows(h, app_id, pid=None):
    return [w for w in h.eval(WINDOWS_JS.replace('__ID__', app_id)) or [] if w['w'] > 0
            and (pid is None or w['pid'] == pid)]


def window(h, app_id, pid=None):
    wins = windows(h, app_id, pid)
    return wins[0] if wins else None


def snapshot(h, kind, pid):
    kids = backends(pid)
    return {'pid': pid, 'exe': exe(pid), 'backends': kids, 'answers': answers(h, kind, pid)}


def leftovers(h, expected):
    """win11-* processes that are not one of `expected` or a descendant of one, and zombies of ours."""
    rows = table()
    keep = set(expected)
    for pid in list(expected):
        keep.update(h.hm.descendants(pid, {p: {'pid': p, 'ppid': r['ppid'], 'comm': r['comm']} for p, r in rows.items()}))
    stray = [{'pid': p, **r, 'cmd': b' '.join(cmdline(p)).decode(errors='replace')[:160]} for p, r in rows.items()
             if r['comm'].startswith('win11-') and p not in keep and r['state'] not in ('Z', 'X')]
    zombies = [{'pid': p, **r} for p, r in rows.items() if r['state'] == 'Z' and r['comm'].startswith('win11-')]
    return stray, zombies


def run(h):
    bindir = h.hm.RUN / 'prefix' / 'bin'
    shell = h.s.facts.get('nativeBoot') or {}
    h.require('the native shell is up', bool(shell.get('pid')))
    shell_pid = shell['pid']
    h.sleep(3)

    # A. the apps, and what runs before the restart
    gui = {}
    for kind, argv, app_id, binary in APPS:
        rows = len(h.s.rows())
        proc = spawn(h, kind, argv)
        try:
            h.s.native_app_boot(kind, rows, 45)
            win = h.until(lambda: window(h, app_id), f'{kind} window', 30, 0.3)
        except TimeoutError as error:
            proc.kill()
            h.require(f'A: {kind} started', False, error=str(error))
        pid = win['pid']
        h.until(lambda: answers(h, kind, pid)['ok'] or None, f'{kind} backend answers', 30, 0.3)
        gui[kind] = {'pid': pid, 'app_id': app_id, 'binary': binary, 'proc': proc}
    h.s.native_app('explorer', {'native': 'click', 'name': 'explorer-new-tab'}, 15)
    try:
        before_tabs = h.until(lambda: (lambda t: t if len(t) == 2 else None)(tabs(h)), 'Explorer has two tabs', 15, 0.3)
    except TimeoutError:
        before_tabs = tabs(h)
    h.check('A: Explorer has two tabs', len(before_tabs) == 2, tabs=before_tabs)
    # a second Explorer window (one tab): a second launch, handed to the running Explorer
    second = h.hm.RUN / 'home' / 'Tweede venster'
    second.mkdir(exist_ok=True)
    launcher = spawn(h, 'explorer-2', ['win11-explorer', str(second)])
    try:
        launcher.wait(timeout=20)
    except subprocess.TimeoutExpired:
        launcher.kill()
        launcher.wait()
    try:
        h.until(lambda: len(windows(h, gui['explorer']['app_id'], gui['explorer']['pid'])) == 2 or None,
                'a second Explorer window', 20, 0.3)
        before_windows = h.until(lambda: (lambda c: c if c == [1, 2] else None)(window_tabs(h)),
                                 'both Explorer windows with their tabs', 15, 0.3)
    except TimeoutError:
        before_windows = window_tabs(h)
    h.check('A: Explorer has two windows (1 and 2 tabs) in one process',
            before_windows == [1, 2] and len(windows(h, gui['explorer']['app_id'], gui['explorer']['pid'])) == 2
            and launcher.returncode == 0,
            tabs=before_windows, launcher=launcher.returncode)
    before = {kind: snapshot(h, kind, g['pid']) for kind, g in gui.items()}
    before['shell'] = snapshot(h, 'shell', shell_pid)
    h.record('A.before', before)
    for kind, snap in before.items():
        h.check(f'A: {kind} has one backend child that answers',
                len(snap['backends']) == 1 and snap['answers']['ok'] and snap['answers']['pid'] in snap['backends'],
                **snap)
        h.check(f'A: {kind} backend reports mainLoopLagMs', isinstance(snap['answers'].get('mainLoopLagMs'), int),
                answers=snap['answers'])
    stray, zombies = leftovers(h, [shell_pid, *[g['pid'] for g in gui.values()]])
    h.record('A.leftovers', {'stray': stray, 'zombies': zombies})
    h.screenshot('a-before', settle=1.0)

    # B. a new build: every binary replaced by rename (the running images show "<path> (deleted)")
    for name in REPLACED:
        target = bindir / name
        temp = bindir / f'.{name}.new'
        shutil.copy2(target, temp)
        os.replace(temp, target)
    deleted = {kind: exe(snap['pid']) for kind, snap in before.items()}
    h.check('B: the running images are the replaced ones',
            all((path or '').endswith(' (deleted)') for path in deleted.values()), exe=deleted)

    # C. shell.restartAll, as Settings' update page calls it: on Settings' own backend
    rows = len(h.s.rows())
    origin, reply = api(h, 'settings', gui['settings']['pid'], 'shell.restartAll', {'reason': 'libraries'})
    h.record('C.restartAll', {'backend': origin, 'reply': reply})
    h.require('C: shell.restartAll is accepted', reply.get('ok') is True and reply.get('result', {}).get('ok') is True,
              reply=reply)

    # D. every program again, same pid, new build, new backend
    boots = {}
    for kind in gui:
        try:
            boots[kind] = h.s.native_app_boot(kind, rows, 60)
        except TimeoutError as error:
            boots[kind] = {'error': str(error)}
    try:
        boots['shell'] = h.s.native_boot(rows, 60)
    except TimeoutError as error:
        boots['shell'] = {'error': str(error)}
    h.record('D.boots', boots)
    after = {}
    for kind in [*gui, 'shell']:
        old = before[kind]
        pid = old['pid']
        h.check(f'D: {kind} re-exec\'d (a new boot report from the same pid)', boots[kind].get('pid') == pid,
                boot=boots[kind], pid=pid)
        try:
            h.until(lambda: (lambda a: a['ok'] and a['pid'] not in old['backends'] or None)(answers(h, kind, pid)),
                    f'{kind} new backend answers', 45, 0.3)
        except TimeoutError:
            pass
        snap = after[kind] = snapshot(h, kind, pid)
        binary = gui[kind]['binary'] if kind in gui else 'win11-shell'
        h.check(f'D: {kind} runs the installed binary', snap['exe'] == str(bindir / binary), exe=snap['exe'])
        h.check(f'D: {kind} has a NEW backend child that answers',
                len(snap['backends']) == 1 and not set(snap['backends']) & set(old['backends'])
                and snap['answers']['ok'] and snap['answers']['pid'] in snap['backends'], before=old, after=snap)
        h.check(f'D: {kind} backend runs the installed win11-{"panel" if kind == "shell" else "app"}',
                all(exe(b) == str(bindir / ('win11-panel' if kind == 'shell' else 'win11-app')) for b in snap['backends']),
                exe=[exe(b) for b in snap['backends']])
        h.check(f'D: {kind} backend reports mainLoopLagMs', isinstance(snap['answers'].get('mainLoopLagMs'), int),
                answers=snap['answers'])
        h.check(f'D: the old {kind} backend is gone', all(gone(b) for b in old['backends']), old=old['backends'])
    h.record('D.after', after)
    for kind, g in gui.items():
        try:
            win = h.until(lambda: window(h, g['app_id'], g['pid']), f'{kind} window again', 30, 0.3)
        except TimeoutError:
            win = None
        h.check(f'D: the {kind} window is back', bool(win), window=win)
    # both Explorer windows with their own tabs: the restart file (--restart-sessions), not window-state.json's one
    try:
        after_windows = h.until(lambda: (lambda c: c if c == before_windows else None)(window_tabs(h)),
                                'Explorer windows again', 20, 0.3)
    except TimeoutError:
        after_windows = window_tabs(h)
    explorers = windows(h, gui['explorer']['app_id'], gui['explorer']['pid'])
    h.check('D: Explorer reopened both windows with their tabs', after_windows == before_windows == [1, 2]
            and len(explorers) == 2, before=before_windows, after=after_windows, windows=explorers)
    # the origin Settings on its update page: the window title (the nav pane lists "Update en herstel" on every page)
    want = UPDATE_TITLE.get(h.lang, UPDATE_TITLE['nl'])
    try:
        page = h.until(lambda: (lambda w: w if w and w['title'] == want else None)(
            window(h, gui['settings']['app_id'], gui['settings']['pid'])), 'Settings on the update page', 20, 0.5)
    except TimeoutError:
        page = window(h, gui['settings']['app_id'], gui['settings']['pid'])
    h.check('D: the origin Settings reopened on Update en herstel', bool(page) and page['title'] == want,
            window=page, want=want)
    try:
        bar = h.until(lambda: (lambda s: s if any(n.get('name') == 'tb-start' for n in nodes(s)) else None)(
            h.s.native({'native': 'state'})), 'the taskbar again', 30, 0.5)
    except TimeoutError:
        bar = []
    h.check('D: the shell has its taskbar again', bool(bar))
    try:
        panel = h.until(lambda: next((w for w in h.s.windows() if w['wm'] == PANEL and w['pid'] == shell_pid), None),
                        'GNOME sees the Panel window', 20, 0.3)
    except TimeoutError:
        panel = None
    h.check('D: GNOME sees the Panel window of the same pid', bool(panel), window=panel)
    h.screenshot('d-after', settle=1.5)

    # E. nothing left behind
    h.sleep(2)
    stray, zombies = leftovers(h, [shell_pid, *[g['pid'] for g in gui.values()]])
    h.check('E: no process left behind', not stray and not zombies, stray=stray, zombies=zombies)

    # G. a backend that ends unexpectedly: the GUI starts a new one and stays (no re-exec, no new boot report)
    for kind in ('taskmgr', 'shell'):
        pid = gui[kind]['pid'] if kind in gui else shell_pid
        old = backends(pid)
        rows = len(h.s.rows())
        for b in old:
            os.kill(b, 9)
        try:
            h.until(lambda: (lambda a: a['ok'] and a['pid'] not in old or None)(answers(h, kind, pid)),
                    f'{kind} new backend after a crash', 30, 0.3)
        except TimeoutError:
            pass
        snap = snapshot(h, kind, pid)
        rebooted = [r for r in h.s.rows()[rows:] if r.get('event') == 'test' and (r.get('data') or {}).get('tag') == 'boot'
                    and (r.get('data') or {}).get('pid') == pid]
        h.check(f'G: {kind} has a new backend after its backend crashed',
                len(snap['backends']) == 1 and not set(snap['backends']) & set(old) and snap['answers']['ok']
                and snap['answers']['pid'] in snap['backends'], before=old, after=snap)
        h.check(f'G: {kind} kept running (no re-exec)', not rebooted and not gone(pid), boots=rebooted)
    h.check('G: the taskbar stayed', 'tb-start' in {n.get('name') for n in nodes(h.s.native({'native': 'state'}))})

    # F. the apps close with their new backends
    for kind, g in gui.items():
        kids = backends(g['pid'])
        h.eval(CLOSE_JS.replace('__PID__', str(g['pid'])))
        try:
            h.until(lambda: gone(g['pid']) and all(gone(k) for k in kids) or None, f'{kind} closes', 20, 0.3)
            h.check(f'F: {kind} and its backend end on close', True)
        except TimeoutError:
            h.check(f'F: {kind} and its backend end on close', False, left=[p for p in [g['pid'], *kids] if not gone(p)])
            for p in [g['pid'], *kids]:
                try:
                    os.kill(p, 9)
                except OSError:
                    pass
    instances = sorted(p.name for p in (h.hm.RT / 'win11-gnome').glob('native-*.json*')
                       if not p.name.startswith(f'native-backend-{shell_pid}'))
    h.check('F: no instance or restart file of an ended app is left', not instances, files=instances)
    # the new Explorer image read (and removed) the restart file of the old one (transfer.rs take_restart_sessions)
    restart_files = sorted(p.name for p in (h.hm.RT / 'win11-gnome').glob('explorer-restart-*.json*'))
    h.check('F: no Explorer restart file is left', not restart_files, files=restart_files)

    failed = [c['check'] for c in h.results['checks'] if not c['ok']]
    h.require('every check passed', not failed, failed=failed)
