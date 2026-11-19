"""Calendar app (13-11): win11-calendar opens on a date, and a second launch goes to the same single window.

Robin (2026-11-13): "kunnen we een windows styled agenda app maken? voor win11shell? het moet werken met de agenda
rechtsonder in de taakbalk, zodra ik een datum klik dat het die datum opent". The taskbar calendar (HTML
notifcenter.js and native flyouts/calendar.rs) launches io.github.win11shell.Calendar.desktop through apps.launch
with the URI win11-calendar:YYYY-MM-DD. The private session has no systemd user manager, so the host cannot spawn an
external app there (launch-attention.py); this scenario starts the app the way that launch ends:

  P. The install: win11-calendar in the private prefix, its desktop entry (MimeType, Exec %u, StartupWMClass), GNOME
     knows the app and x-scheme-handler/win11-calendar defaults to it (install.py set_scheme_default).
  A. win11-calendar --date 2026-12-24, with an activation token from the extension (apps.activationToken, as the
     host gets one for every launch): one Calendar window appears. Screenshot.
  B. The URI win11-calendar:2027-01-15 through GNOME's own launch of the desktop entry (Exec %u), else the binary
     with the URI: the second process hands it to the running one and exits; still one window, the same window and
     pid. Screenshot. The window titles of A and B are recorded (a dev hook of the app, when it has one, is not used
     here: the screenshots show the opened date).
  C. A third launch with --view week on 22 December: the week view of the same window, with the seeded events.
     Screenshot.
  D. --view day --new on 24 December: the day view with the event editor open over it. Screenshot.
  F. The frosted backdrop: the app registers with nativeBackdrop.configure {pid, kind: 'calendar'} once its window
     is mapped, and the extension blurs the focused Calendar window (its Mica is then 90 %).

Before A a few events are seeded in the private events.json (December 2026), so the views show chips and blocks.
The taskbar is the native shell's (D-G70: the only one; the harness waits for its boot).

Run (from G; a build dir with the host binaries plus win11-shell and win11-calendar copied from the native
workspace's target):
    tools/heavy.sh --min-mb 2560 --tag calendar-app -- python3 -B tests/gnome/harness.py launch run --lang nl \\
        --build-dir tests/build/native-release --source worktree --scenario tests/gnome/scenarios/calendar-app.py
"""
import json
import os
import signal
import subprocess

APP = 'io.github.win11shell.Calendar'
DESKTOP_ID = f'{APP}.desktop'
SCHEME = 'x-scheme-handler/win11-calendar'
FIRST, SECOND = '2026-12-24', '2027-01-15'
BRIDGE = "Main.extensionManager.lookup('win11-gnome@win11shell.local').stateObj._panelBridge"
KNOWN_JS = f"Boolean(imports.gi.Shell.AppSystem.get_default().lookup_app('{DESKTOP_ID}'))"
SCHEME_JS = "imports.gi.Gio.AppInfo.get_default_for_uri_scheme('win11-calendar')?.get_id()??null"
WINDOWS_JS = ("(()=>{const Meta=imports.gi.Meta;const f=global.display.focus_window;"
              "return global.get_window_actors().map(a=>a.meta_window).filter(w=>(w.get_wm_class()==='" + APP + "'"
              "||w.get_gtk_application_id()==='" + APP + "')&&w.get_window_type()===Meta.WindowType.NORMAL)"
              ".map(w=>{const r=w.get_frame_rect();return {id:String(w.get_id()),pid:w.get_pid(),title:w.get_title(),"
              "focused:f===w,minimized:w.minimized,x:r.x,y:r.y,w:r.width,h:r.height};});})()")
GNOME_LAUNCH_JS = ("(()=>{const app=imports.gi.Shell.AppSystem.get_default().lookup_app('" + DESKTOP_ID + "');"
                   "if(!app)return 'no app';try{return app.get_app_info().launch_uris(['win11-calendar:{date}'],"
                   "global.create_app_launch_context(0,-1));}catch(e){return String(e);}})()")
FROSTED_JS = ("(()=>{try{return " + BRIDGE + "._frostedBridge.capabilities().windows.filter(w=>w.pid===__PID__);}"
              "catch(e){return String(e);}})()")


def windows(h):
    return h.eval(WINDOWS_JS) or []


def spawn(h, binary, args, token=None):
    """The app as GIO starts it: the private Wayland display and, with a launch, the activation token."""
    env = dict(os.environ, WAYLAND_DISPLAY=h.hm.WL, GDK_BACKEND='wayland')
    if token:
        env.update(XDG_ACTIVATION_TOKEN=token, DESKTOP_STARTUP_ID=token)
    log = open(h.hm.RUN / 'logs' / 'app-calendar.log', 'a')
    return subprocess.Popen([str(binary), *args], env=env, stdout=log, stderr=subprocess.STDOUT)


def run(h):
    prefix, data, config = h.hm.RUN / 'prefix', h.hm.RUN / 'data', h.hm.RUN / 'config'
    binary = prefix / 'bin' / 'win11-calendar'
    entry = data / 'applications' / DESKTOP_ID

    # P. The install.
    h.require('P: win11-calendar is installed in the private prefix', binary.is_file() and os.access(binary, os.X_OK),
              binary=str(binary))
    text = entry.read_text() if entry.is_file() else ''
    h.record('P.entry', text)
    h.check('P: the Calendar desktop entry handles win11-calendar: and starts the private binary',
            f'MimeType={SCHEME};' in text and str(binary) in text and ' %u' in text
            and f'StartupWMClass={APP}' in text and 'Categories=Office;Calendar;' in text)
    mimeapps = config / 'mimeapps.list'
    h.record('P.mimeapps', mimeapps.read_text() if mimeapps.is_file() else None)
    try:
        known = h.until(lambda: h.eval(KNOWN_JS) or None, 'GNOME knows the Calendar app', 20, 0.2)
    except TimeoutError:
        known = False
    h.check('P: GNOME knows io.github.win11shell.Calendar.desktop', known)
    handler = h.eval(SCHEME_JS)
    h.check('P: win11-calendar: opens the Calendar app (the scheme default)', handler == DESKTOP_ID, handler=handler)

    h.record('native.boot', h.s.facts.get('nativeBoot'))

    # Seeded events (store.rs format), so the month, week and day views have something to draw.
    events_dir = data / 'win11-gnome-calendar'
    events_dir.mkdir(parents=True, exist_ok=True)
    seed = [
        ('e1', 'Teamoverleg', 'work', False, '2026-12-21T09:30', '2026-12-21T10:30', 'Kantoor'),
        ('e2', 'Tandarts', 'personal', False, '2026-12-22T14:00', '2026-12-22T14:45', ''),
        ('e3', 'Lunch met Sanne', 'personal', False, '2026-12-22T12:00', '2026-12-22T13:00', 'Centrum'),
        ('e4', 'Sprint review', 'work', False, '2026-12-22T12:30', '2026-12-22T13:30', ''),
        ('e5', 'Kerstavond', 'family', True, '2026-12-24', '2026-12-24', ''),
        ('e6', 'Kerstdiner', 'family', False, '2026-12-24T18:00', '2026-12-24T21:30', 'Thuis'),
        ('e7', 'Verjaardag oma', 'birthdays', True, '2026-12-27', '2026-12-27', ''),
        ('e8', 'Vakantie', 'personal', True, '2026-12-28', '2026-12-31', ''),
        ('e9', 'Hardlopen', 'personal', False, '2026-12-24T08:00', '2026-12-24T09:00', ''),
        ('e10', 'Inpakken', 'family', False, '2026-12-24T10:00', '2026-12-24T11:00', ''),
    ]
    (events_dir / 'events.json').write_text(json.dumps({'version': 1, 'events': [
        {'id': i, 'title': title, 'calendar': cal, 'allDay': all_day, 'start': start, 'end': end,
         'location': where, 'notes': ''} for i, title, cal, all_day, start, end, where in seed]}, indent=1))

    started = []
    # A. The first launch, on 24 December 2026.
    token = (h.bridge('apps.activationToken', {'desktopId': DESKTOP_ID}) or {}).get('token')
    h.check('async bridge supplies Calendar an activation token', isinstance(token, str) and bool(token))
    first = spawn(h, binary, ['--date', FIRST], token)
    started.append(first)
    try:
        a = h.until(lambda: windows(h) or None, 'a Calendar window', 30, 0.2)
    except TimeoutError:
        a = windows(h)
    h.sleep(1.5)
    a = windows(h) or a
    h.record('A.token', bool(token))
    h.record('A.windows', a)
    h.require('A: win11-calendar --date opens one Calendar window', len(a) == 1, windows=a, exit=first.poll())
    h.check('A: the window belongs to the launched process', a[0]['pid'] == first.pid, window=a[0], pid=first.pid)
    h.check('A: the launched window has the focus', a[0]['focused'], window=a[0])
    h.screenshot(f'a-calendar-{FIRST}')

    # B. A second launch with the URI, on 15 January 2027: the running Calendar takes it over.
    how = h.eval(GNOME_LAUNCH_JS.replace('{date}', SECOND))
    second = None
    if how is not True:
        second = spawn(h, binary, [f'win11-calendar:{SECOND}'], (h.bridge('apps.activationToken', {'desktopId': DESKTOP_ID}) or {}).get('token'))
        started.append(second)
    h.record('B.launch', {'gnome': how, 'spawned': bool(second)})
    if second:
        try:
            code = second.wait(15)
        except subprocess.TimeoutExpired:
            code = None
        h.check('B: the second process hands the date over and exits', code == 0, exit=code)
    h.sleep(3)
    b = windows(h)
    h.record('B.windows', b)
    h.check('B: still one Calendar window (single instance)', len(b) == 1, windows=b)
    h.check('B: the same window and process as A', len(b) == 1 and b[0]['id'] == a[0]['id']
            and b[0]['pid'] == a[0]['pid'], before=a[0], after=b)
    h.record('B.titleChanged', bool(b) and b[0]['title'] != a[0]['title'])
    h.screenshot(f'b-calendar-{SECOND}')

    # F. The app's backdrop as the extension sees it (blur and shadow once the app registers kind 'calendar').
    frosted = h.eval(FROSTED_JS.replace('__PID__', str(a[0]['pid'])))
    h.record('F.frosted', frosted)
    h.check('F: the extension blurs the focused Calendar window (kind calendar registered)',
            isinstance(frosted, list) and any(w.get('blur') and w.get('focused') for w in frosted), frosted=frosted)

    # C. The week view through a third launch (forwarded to the running window).
    third = spawn(h, binary, ['--date', '2026-12-22', '--view', 'week'], (h.bridge('apps.activationToken', {'desktopId': DESKTOP_ID}) or {}).get('token'))
    started.append(third)
    try:
        code = third.wait(15)
    except subprocess.TimeoutExpired:
        code = None
    h.check('C: the --view week launch is handed over and exits', code == 0, exit=code)
    h.sleep(2)
    c = windows(h)
    h.check('C: still the one Calendar window', len(c) == 1 and c[0]['id'] == a[0]['id'], windows=c)
    h.screenshot('c-calendar-week-2026-12-22')

    # D. The day view with a new event's editor over it.
    fourth = spawn(h, binary, ['--date', FIRST, '--view', 'day', '--new'], (h.bridge('apps.activationToken', {'desktopId': DESKTOP_ID}) or {}).get('token'))
    started.append(fourth)
    try:
        code = fourth.wait(15)
    except subprocess.TimeoutExpired:
        code = None
    h.check('D: the --view day --new launch is handed over and exits', code == 0, exit=code)
    h.sleep(2)
    h.screenshot('d-calendar-day-new-2026-12-24')

    for pid in {w['pid'] for w in windows(h)} | {p.pid for p in started if p.poll() is None}:
        try:
            os.kill(pid, signal.SIGTERM)
        except OSError:
            pass
    for proc in started:
        try:
            proc.wait(5)
        except subprocess.TimeoutExpired:
            proc.kill()
    failed = [c['check'] for c in h.results['checks'] if not c['ok']]
    h.record('failed', failed)
    h.require('every check passed', not failed, failed=failed)
