"""Launch attention (D-G61): an app launched from our Start or taskbar comes to the front, no "“…” is ready".

Robin (2026-11-12): "soms krijg ik een notificatie dat een venster klaar staat in plaats van dat het venster gewoon
direct opent". Apps that run once (Claude, WhatsApp, Steam, often Firefox) hand a launch to their running window,
which then asks for the focus without the activation token the launch carried; GNOME's WindowAttentionHandler shows
a notification instead. extension/panelBridge.js now lets a window of an app launched through apps.activationToken
take the focus within 10 s, once, and leaves every other request to GNOME's handler.

tests/gnome/scenarios/attention-helper.py plays such an app (org.example.AttentionTest, a private desktop entry). The
private session has no systemd user manager, so the host cannot spawn an external app there (its process scope);
the scenario asks the extension for the launch token (apps.activationToken, as the host does for every launch) and
starts the helper with it in XDG_ACTIVATION_TOKEN, as GIO does.

Run (from G):
    tools/heavy.sh --min-mb 2560 --tag launch-attention -- python3 -B tests/gnome/harness.py launch run --lang nl \\
        --build-dir tests/build/<step>-release --source worktree --scenario tests/gnome/scenarios/launch-attention.py

  A. Launched with a token: the helper's window opens with the focus.
  B. Settings focused, then the running helper presents itself without a launch: GNOME's notification, no focus
     (the reproduction: this is what Robin sees).
  C. Settings focused, then the helper launched again with a token (the second process hands over): its window
     comes to the front and there is no notification.
  D. Once more without a launch: GNOME's notification again (the grant was used once; GNOME's handler still works).
"""
import json
import os
import signal
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
APP = 'org.example.AttentionTest'
BRIDGE = "Main.extensionManager.lookup('win11-gnome@win11shell.local').stateObj._panelBridge"
TOKEN_JS = f"JSON.parse({BRIDGE}.Call('apps.activationToken', JSON.stringify({{desktopId:'{APP}.desktop'}}))).token"
STATE_JS = ("(()=>{const Shell=imports.gi.Shell;"
            "const w=global.get_window_actors().map(a=>a.meta_window).find(w=>w.get_wm_class()==='" + APP + "'"
            "||w.get_gtk_application_id()==='" + APP + "');const f=global.display.focus_window;"
            "const sources=Main.messageTray.getSources().filter(s=>s.title==='AttentionTest');"
            "return {exists:!!w,focused:!!w&&f===w,demands:!!w&&w.demands_attention,focus:f?.get_wm_class()??null,"
            "app:w?Shell.WindowTracker.get_default().get_window_app(w)?.get_id()??null:null,notified:sources.length};})()")
KNOWN_JS = f"Boolean(imports.gi.Shell.AppSystem.get_default().lookup_app('{APP}.desktop'))"
ACTIVATE_JS = ("(()=>{const w=global.get_window_actors().map(a=>a.meta_window).find(w=>w.get_wm_class()==='" + APP + "'"
               "||w.get_gtk_application_id()==='" + APP + "');if(w)Main.activateWindow(w,global.display.get_current_time_roundtrip());"
               "return !!w;})()")


def state(h):
    return h.eval(STATE_JS)


def wait(h, label, test, timeout=6.0):
    try:
        return h.until(lambda: (lambda s: s if test(s) else None)(state(h)), label, timeout, 0.1)
    except TimeoutError:
        return state(h)


def launch(h, run_dir):
    """What the host does for a launch: a token from the extension, then the app with it (GIO's environment)."""
    token = h.eval(TOKEN_JS)
    env = dict(os.environ, WAYLAND_DISPLAY=h.hm.WL, GDK_BACKEND='wayland', XDG_ACTIVATION_TOKEN=token,
               DESKTOP_STARTUP_ID=token)
    log = open(h.hm.RUN / 'logs' / 'app-attention-helper.log', 'a')
    subprocess.Popen([sys.executable, '-I', str(HERE / 'attention-helper.py'), str(run_dir)], env=env, stdout=log,
                     stderr=subprocess.STDOUT)
    return {'token': bool(token)}


def focus_settings(h):
    h.open('settings')
    return wait(h, 'Settings focused', lambda s: s['focus'] == 'io.github.win11shell.Settings')


def run(h):
    run_dir = h.hm.RUN / 'tmp' / 'attention'
    run_dir.mkdir(parents=True, exist_ok=True)
    trigger = run_dir / 'attention-helper.present'
    apps = h.hm.RUN / 'data' / 'applications'
    apps.mkdir(parents=True, exist_ok=True)
    (apps / f'{APP}.desktop').write_text(
        '[Desktop Entry]\nType=Application\nName=AttentionTest\n'
        f'Exec=/usr/bin/python3 -I {HERE / "attention-helper.py"} {run_dir}\n'
        f'StartupNotify=true\nStartupWMClass={APP}\n')
    h.until(lambda: h.eval(KNOWN_JS) or None, 'GNOME knows the test app', 20, 0.2)

    # A. First launch.
    h.record('A.launch', launch(h, run_dir))
    s = wait(h, 'helper window focused', lambda s: s['exists'] and s['focused'], 15)
    h.record('A.state', s)
    h.check('A: the launched helper opens with the focus', s['exists'] and s['focused'], state=s)
    h.check('A: GNOME maps its window to the test app', s['app'] == f'{APP}.desktop', state=s)

    # B. Without a launch: the request GNOME refuses (what Robin sees).
    focus_settings(h)
    trigger.touch()
    s = wait(h, 'GNOME notification without a launch', lambda s: s['notified'] > 0, 6)
    h.record('B.state', s)
    h.check('B: without a launch GNOME shows "is ready" and keeps the focus', s['notified'] > 0 and not s['focused'],
            state=s)
    h.screenshot('b-gnome-notification')
    h.eval(ACTIVATE_JS)                            # the notification goes with the focus
    wait(h, 'notification gone', lambda s: s['notified'] == 0 and s['focused'])

    # C. Launched again: the second process hands over; the running window may come to the front.
    focus_settings(h)
    h.record('C.launch', launch(h, run_dir))
    s = wait(h, 'helper to the front after a launch', lambda s: s['focused'], 8)
    h.sleep(0.5)
    s = state(h)
    h.record('C.state', s)
    h.check('C: after a launch the window comes to the front', s['focused'], state=s)
    h.check('C: no "is ready" notification', s['notified'] == 0, state=s)
    h.screenshot('c-launched-front')

    # D. The grant was used once: GNOME's own handler takes the next request again.
    focus_settings(h)
    trigger.touch()
    s = wait(h, 'GNOME notification again', lambda s: s['notified'] > 0, 6)
    h.record('D.state', s)
    h.check('D: a later request without a launch goes to GNOME again', s['notified'] > 0 and not s['focused'], state=s)

    try:
        os.kill(int((run_dir / 'attention-helper.pid').read_text()), signal.SIGTERM)
    except (OSError, ValueError):
        pass
    failed = [c['check'] for c in h.results['checks'] if not c['ok']]
    h.require('every check passed', not failed, failed=failed)
