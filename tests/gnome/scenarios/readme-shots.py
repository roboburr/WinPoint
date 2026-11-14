"""README screenshots (13-11): the GUI in English, dark theme, on the private 1440x900 monitor.

Robin (2026-11-13): "de hele readme moet engels". Takes the pictures docs/screenshots/ holds, each with nothing else in
the way (no leftover flyouts, no test windows, the pointer parked on the right edge, animations settled):

  desktop         the desktop with wallpaper, icons and the taskbar, nothing open
  start           Start open (full screen, the taskbar under it)
  quick-settings  the quick settings flyout (full screen; cropped bottom right when copied)
  winx            the Win+X menu, opened with a real right-click on the Start button (dev flyout as fallback)
  notifications   the notification centre with a Calendar notification and the product's test notification
  explorer        Explorer (the home folder) focused over the desktop
  settings        Settings > Personalisation focused over the desktop
  task-manager-summary, task-manager
                  Task Manager's Summary (it lists the host's busiest apps) and its Performance page
  calendar        the Calendar app's month view with a few seeded events (events.json in the private XDG_DATA_HOME)
  task-view       Task View with Settings, Task Manager and the Calendar (Explorer's window icon is a theme name
                  that does not resolve in Task View, so it would show the generic icon)
  light-start, light-explorer
                  theme.mode light (settings.json), Start and Explorer

Run (from G):
    tools/heavy.sh --min-mb 2560 --tag readme-shots -- python3 -B tests/gnome/harness.py launch run --lang en \\
        --build-dir tests/build/native-release --source worktree --scenario tests/gnome/scenarios/readme-shots.py \\
        --out tests/gnome/scenarios/readme-shots.out
"""
import datetime
import json
import os
import signal
import subprocess

PANEL = 'io.github.win11shell.Panel'
START = 'io.github.win11shell.Start'
CAL_APP = 'io.github.win11shell.Calendar'
CAL_DESKTOP = f'{CAL_APP}.desktop'
BRIDGE = "Main.extensionManager.lookup('win11-gnome@win11shell.local').stateObj._panelBridge"
TOKEN_JS = ("(()=>{try{return JSON.parse(" + BRIDGE + ".Call('apps.activationToken',JSON.stringify({desktopId:'"
            + CAL_DESKTOP + "'}))).token||null;}catch(e){return null;}})()")
CAL_WINDOWS_JS = ("(()=>{const Meta=imports.gi.Meta;return global.get_window_actors().map(a=>a.meta_window)"
                  ".filter(w=>(w.get_wm_class()==='" + CAL_APP + "'||w.get_gtk_application_id()==='" + CAL_APP + "')"
                  "&&w.get_window_type()===Meta.WindowType.NORMAL).map(w=>{const r=w.get_frame_rect();"
                  "return {pid:w.get_pid(),title:w.get_title(),x:r.x,y:r.y,w:r.width,h:r.height};});})()")

POINTER_JS = (
    "(async()=>{{const Clutter=(await import('gi://Clutter')).default;const GLib=(await import('gi://GLib')).default;"
    "const wait=ms=>new Promise(r=>GLib.timeout_add(GLib.PRIORITY_DEFAULT,ms,()=>{{r();return GLib.SOURCE_REMOVE;}}));"
    "const seat=Clutter.get_default_backend().get_default_seat();"
    "globalThis.__ge1hPointer??=seat.create_virtual_device(Clutter.InputDeviceType.POINTER_DEVICE);"
    "const d=globalThis.__ge1hPointer,us=()=>GLib.get_monotonic_time();"
    "d.notify_absolute_motion(us(),{x},{y});await wait(120);"
    "if({button}){{d.notify_button(us(),{button},Clutter.ButtonState.PRESSED);await wait(60);"
    "d.notify_button(us(),{button},Clutter.ButtonState.RELEASED);await wait(40);}}"
    "return global.get_pointer().slice(0,2);}})()")

WINDOWS_OF_JS = ("global.get_window_actors().map(a=>a.meta_window).filter(w=>w.get_pid()===__PID__"
                 "&&w.get_window_type()===Meta.WindowType.NORMAL)")
MINIMIZE_JS = "(()=>{const ws=" + WINDOWS_OF_JS + ";ws.forEach(w=>w.minimize());return ws.length;})()"
RESTORE_JS = ("(()=>{const ws=" + WINDOWS_OF_JS + ";ws.forEach(w=>{w.unminimize();Main.activateWindow(w);});"
              "return ws.length;})()")
CLEAR_NOTIFICATIONS_JS = ("(()=>{let n=0;Main.messageTray.getSources().forEach(s=>(s.notifications||[]).slice()"
                          ".forEach(x=>{x.destroy();n++;}));return n;})()")


# ---- helpers ------------------------------------------------------------------------------------------------
def walk(node, out):
    if isinstance(node, dict):
        out.append(node)
        for child in node.get('children') or []:
            walk(child, out)
    return out


def nodes(h):
    out = []
    for top in h.s.native_state():
        walk(top.get('tree'), out)
    return out


def open_flyouts(h):
    """Names of the flyouts that are shown (the dev state lists only visible widgets; a frame has w11-flyout-<name>)."""
    found = []
    for node in nodes(h):
        classes = node.get('classes') or []
        if 'w11-flyout' in classes:
            found += [c[len('w11-flyout-'):] for c in classes if c.startswith('w11-flyout-')]
    return found


def park(h, x=None, y=None):
    """The pointer on the right edge of the screen, where no window, icon or flyout is."""
    x = h.hm.WIDTH - 3 if x is None else x
    y = 330 if y is None else y
    return h.eval(POINTER_JS.format(x=x, y=y, button=0))


def clean(h):
    """No flyout, no Start, pointer parked."""
    pid = h.native_pid
    for name in open_flyouts(h):
        h.native({'native': 'flyout', 'name': name})
    h.s.start_flyout(False, pid)
    park(h)
    try:
        h.until(lambda: not open_flyouts(h), 'no flyout open', 5, 0.3)
    except TimeoutError:
        h.log(f'flyouts still open: {open_flyouts(h)}')
    h.sleep(0.6)


def shot(h, name, settle=1.2, **kw):
    try:
        return h.screenshot(name, settle=settle, **kw)
    except Exception as error:   # noqa: BLE001 - one bad shot must not cost the others
        h.check(f'screenshot {name}', False, error=str(error))
        return None


def step(h, label, fn):
    """Run one shot's preparation; record a failure and carry on."""
    try:
        fn()
        h.check(label, True)
    except Exception as error:   # noqa: BLE001
        h.check(label, False, error=f'{type(error).__name__}: {error}')
        try:
            clean(h)
        except Exception:   # noqa: BLE001
            pass


def flyout(h, name):
    value = h.native({'native': 'flyout', 'name': name})
    if not (isinstance(value, dict) and value.get('open') == name):
        raise RuntimeError(f'flyout {name} did not open: {value}')
    return value


def minimize(h, pid):
    return h.eval(MINIMIZE_JS.replace('__PID__', str(pid)))


def restore(h, pid):
    return h.eval(RESTORE_JS.replace('__PID__', str(pid)))


def notify(h, desktop_id, app, summary, body):
    """A notification from an app's source in the private shell's message tray (the extension forwards it to the
    native centre as it does every GNOME notification)."""
    js = ("(async()=>{const MT=await import('resource:///org/gnome/shell/ui/messageTray.js');"
          "const Shell=(await import('gi://Shell')).default;const a=Shell.AppSystem.get_default().lookup_app(__ID__);"
          "const props={title:__APP__};if(a)props.icon=a.get_icon();else props.iconName='preferences-system-notifications';"
          "const source=new MT.Source(props);Main.messageTray.add(source);"
          "const n=new MT.Notification({source,title:__TITLE__,body:__BODY__});source.addNotification(n);return Boolean(a);})()")
    for key, value in (('__ID__', desktop_id), ('__APP__', app), ('__TITLE__', summary), ('__BODY__', body)):
        js = js.replace(key, json.dumps(value))
    return h.eval(js)


def test_notification(h):
    """The product's own test notification (Settings > Notifications' button): notifications.test of the bridge."""
    return h.eval("(()=>{try{return JSON.parse(" + BRIDGE + ".Call('notifications.test',JSON.stringify({language:'en'})))"
                  ".notification?.summary||true;}catch(e){return String(e);}})()")


def set_theme(h, mode):
    path = h.hm.RUN / 'config' / 'win11-gnome' / 'settings.json'
    data = json.loads(path.read_text() or '{}')
    theme = dict(data.get('theme') or {})
    theme['mode'] = mode
    h.hm.write_settings({'theme': theme})
    return theme


# ---- the calendar -------------------------------------------------------------------------------------------
def seed_events(h, today):
    y, m = today.year, today.month
    day = lambda d: datetime.date(y, m, d).isoformat()
    seed = [
        ('r1', 'Team stand-up', 'work', False, f'{day(3)}T09:30', f'{day(3)}T10:00', ''),
        ('r2', 'Running club', 'personal', False, f'{day(5)}T07:30', f'{day(5)}T08:30', 'Park'),
        ('r3', 'Dentist', 'personal', False, f'{day(10)}T14:00', f'{day(10)}T14:45', ''),
        ('r4', 'Lunch with Sam', 'personal', False, f'{day(today.day)}T12:00', f'{day(today.day)}T13:00', 'Cafe'),
        ('r5', 'Sprint review', 'work', False, f'{day(today.day)}T15:00', f'{day(today.day)}T16:00', ''),
        ('r6', 'Release 1.0', 'work', True, day(18), day(18), ''),
        ('r7', "Mum's birthday", 'birthdays', True, day(21), day(21), ''),
        ('r8', 'Weekend away', 'family', True, day(26), day(28), ''),
        ('r9', 'Book club', 'family', False, f'{day(24)}T19:30', f'{day(24)}T21:00', ''),
    ]
    folder = h.hm.RUN / 'data' / 'win11-gnome-calendar'
    folder.mkdir(parents=True, exist_ok=True)
    (folder / 'events.json').write_text(json.dumps({'version': 1, 'events': [
        {'id': i, 'title': title, 'calendar': cal, 'allDay': all_day, 'start': start, 'end': end,
         'location': where, 'notes': ''} for i, title, cal, all_day, start, end, where in seed]}, indent=1))


def spawn_calendar(h, args):
    binary = h.hm.RUN / 'prefix' / 'bin' / 'win11-calendar'
    env = dict(os.environ, WAYLAND_DISPLAY=h.hm.WL, GDK_BACKEND='wayland')
    token = h.eval(TOKEN_JS)
    if token:
        env.update(XDG_ACTIVATION_TOKEN=token, DESKTOP_STARTUP_ID=token)
    log = open(h.hm.RUN / 'logs' / 'app-calendar.log', 'a')
    return subprocess.Popen([str(binary), *args], env=env, stdout=log, stderr=subprocess.STDOUT)


# ---- the run ------------------------------------------------------------------------------------------------
def run(h):
    W, H = h.hm.WIDTH, h.hm.HEIGHT
    h.record('boot', h.s.facts.get('nativeBoot'))
    h.sleep(4)
    clean(h)
    h.record('toplevels', [{k: t.get(k) for k in ('title', 'visible', 'width', 'height')} for t in h.s.native_state()])

    # 1. The desktop, nothing open.
    step(h, 'desktop', lambda: shot(h, 'desktop', settle=1.5))

    # 2. Start.
    def start():
        h.open('start')
        park(h)
        shot(h, 'start', settle=1.5)
        clean(h)
    step(h, 'start', start)

    # 3a. Quick settings.
    def quick():
        flyout(h, 'quick')
        park(h)
        shot(h, 'quick-settings')
        clean(h)
    step(h, 'quick-settings', quick)

    # 4a. Win+X: a real right-click on the Start button (the menu then sits above it), else the dev flyout.
    def winx():
        btn = next((n for n in nodes(h) if n.get('name') == 'tb-start' and n.get('rect')), None)
        how = 'dev'
        if btn:
            x, y, w, hh = btn['rect']
            h.record('winx.startButton', btn['rect'])
            h.eval(POINTER_JS.format(x=x + w // 2, y=y + hh // 2, button=3))
            try:
                h.until(lambda: 'winx' in open_flyouts(h), 'Win+X open after the right-click', 4, 0.3)
                how = 'right-click'
            except TimeoutError:
                pass
        if how == 'dev':
            flyout(h, 'winx')
        h.record('winx.how', how)
        park(h)
        shot(h, 'winx')
        clean(h)
    step(h, 'winx', winx)

    # 3b. Notifications: one from the Calendar's source and the product's own test notification, the toasts allowed
    # to time out first.
    def notifications():
        posted = [notify(h, CAL_DESKTOP, 'Calendar', 'Sprint review', 'Today, 15:00 – 16:00'),
                  test_notification(h)]
        h.record('notifications.posted', posted)
        h.sleep(8)
        clean(h)
        flyout(h, 'notifications')
        park(h)
        shot(h, 'notifications', settle=1.5)
        clean(h)
        h.record('notifications.cleared', h.eval(CLEAR_NOTIFICATIONS_JS))
        h.sleep(1.5)
    step(h, 'notifications', notifications)

    # 5. The apps, each focused over the desktop (the others minimised for its picture).
    apps = {}

    def app(name, spec, png):
        def go():
            for other in apps.values():
                minimize(h, other.pid)
            sf = h.open(spec)
            apps[name] = sf
            h.sleep(2.5)
            clean(h)
            restore(h, sf.pid)
            park(h)
            shot(h, png, settle=1.5)
        return go
    step(h, 'explorer', app('explorer', 'explorer', 'explorer'))
    step(h, 'settings', app('settings', 'settings:personalization', 'settings'))
    step(h, 'task-manager-summary', app('taskmgr', 'taskmgr', 'task-manager-summary'))

    def performance():
        expr = ("[...document.querySelectorAll('button,a,li,[role=tab],[role=option],[role=treeitem],[role=menuitem]')]"
                ".find(e=>(e.textContent||'').trim()==='Performance')")
        try:
            h.record('taskmgr.click', h.click('taskmgr', find=expr))
        except Exception as error:   # noqa: BLE001
            h.record('taskmgr.click', f'pointer failed: {error}')
            h.click('taskmgr', find=expr, real=False)
        h.sleep(30)         # half a minute of samples for the 60 s graph
        park(h)
        shot(h, 'task-manager', settle=1.0)
    step(h, 'task-manager', performance)
    # Explorer's window icon is a theme name GTK does not resolve here (Task View then shows the generic one):
    # Task View gets Settings, Task Manager and the Calendar.
    try:
        h.close('explorer')
    except Exception as error:   # noqa: BLE001
        h.log(f'close explorer: {error}')
    apps.pop('explorer', None)

    # 6. The Calendar app, month view of this month with seeded events.
    started = []

    def calendar():
        for other in apps.values():
            minimize(h, other.pid)
        today = datetime.date.today()
        seed_events(h, today)
        proc = spawn_calendar(h, ['--date', today.isoformat(), '--view', 'month'])
        started.append(proc)
        found = h.until(lambda: h.eval(CAL_WINDOWS_JS) or None, 'a Calendar window', 30, 0.3)
        h.record('calendar.windows', found)
        h.sleep(2)
        clean(h)
        restore(h, found[0]['pid'])
        park(h)
        shot(h, 'calendar', settle=1.5)
    step(h, 'calendar', calendar)

    # 4b. Task View with three windows.
    def task_view():
        for name in ('settings', 'taskmgr'):
            if name in apps:
                restore(h, apps[name].pid)
                h.sleep(0.4)
        cal = h.eval(CAL_WINDOWS_JS) or []
        if cal:
            restore(h, cal[0]['pid'])
        h.sleep(1)
        clean(h)
        flyout(h, 'taskview')
        park(h)
        shot(h, 'task-view', settle=1.5)
        clean(h)
    step(h, 'task-view', task_view)
    for name in list(apps):
        try:
            h.close(name)
        except Exception as error:   # noqa: BLE001
            h.log(f'close {name}: {error}')
    apps.clear()
    for proc in started:
        if proc.poll() is None:
            try:
                os.kill(proc.pid, signal.SIGTERM)
                proc.wait(5)
            except (OSError, subprocess.TimeoutExpired):
                proc.kill()
    h.sleep(1.5)

    # 7. Light theme: Start, then Explorer.
    def light():
        h.record('theme', set_theme(h, 'light'))
        h.sleep(3)
        clean(h)
        h.open('start')
        park(h)
        shot(h, 'light-start', settle=1.5)
        clean(h)
        h.open('explorer')
        h.sleep(2.5)
        clean(h)
        park(h)
        shot(h, 'light-explorer', settle=1.5)
        h.close('explorer')
    step(h, 'light', light)
    set_theme(h, 'dark')

    failed = [c['check'] for c in h.results['checks'] if not c['ok']]
    h.record('failed', failed)
