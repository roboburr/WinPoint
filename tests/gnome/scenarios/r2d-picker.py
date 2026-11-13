"""R2-D scenario: the file picker's own window identity (GE1b observation o2) in the real product (WebKitGTK inside
the private GNOME Shell; UI and extension from --source, binaries from --build-dir).

GE1b's review saw both pickers of Settings with the WM class io.github.win11shell.Settings. picker.rs asked GTK for
io.github.win11shell.FilePicker in a realize hook, but GTK3 creates the Wayland xdg_toplevel only when the window is
mapped, so that call did nothing and the toplevel took g_get_prgname(), the caller's app id. R2-D moves the call to
the map signal (picker.rs configure()). Fix round (review m6): with only that, a non-modal picker got a running
taskbar button of its own ("GNOME-instellingen": the extension's windows.list resolved the FilePicker role through
GNOME's WindowTracker), so the picker now also keeps its caller's GTK application id (gtk_shell1 D-Bus properties),
as frostedBridge.js windowIdentity() describes: the extension's windows.list reads that id first.

1. Settings opens one picker the way its Background page does (shell.pickFile with a filter; modal, Win11's own
   caller), and a second, non-modal one ({modal: false}, as a file-portal caller may ask).
2. Per picker window: Meta's WM class, GTK application id, window type, the transient parent's WM class, the app
   GNOME's WindowTracker gives it, skip_taskbar, and the extension's frosted-backdrop entries (frostedBridge.js
   gives own-app windows a shadow; its windowIdentity() keeps a FilePicker out of that on purpose). A screenshot of
   each picker with a 40 px margin, for the look. The Win11 taskbar before the pickers and with each one open: the
   extension's windows.list entries of the Settings process, grouped by app as the taskbar groups its buttons
   (D-G70: the native taskbar has no page to read its buttons from; it draws them from this same list).
3. Checks: the WM class is io.github.win11shell.FilePicker and the GTK application id is Settings's (a build without
   the fix is a reference run: these checks are marked known there), GNOME still groups the picker under Settings
   (transient parent), no picker adds a taskbar button and the non-modal one joins the Settings button (2 windows),
   and both shell.pickFile calls settle as cancelled when the pickers are closed.

Run (from G; since D-G70 a build dir with win11-shell, so the round-1 reference run, r1-debug, cannot run any more):
    tools/heavy.sh --min-mb 2560 --tag r2d-picker -- python3 -B tests/gnome/harness.py launch run --lang nl \\
        --build-dir tests/build/native-release --source worktree --scenario tests/gnome/scenarios/r2d-picker.py \\
        --out tests/gnome/scenarios/r2d-picker.out/native
Output r2d-old is the build of the first R2-D pass (FilePicker role only), run with the taskbar checks of the fix
round: it shows the separate taskbar button.
In a worktree (.wt/r2-*) the harness cannot start its private system bus: RT/system-bus is 102 bytes there and
dbus-daemon allows 99. R2-D ran a byte-identical copy of harness.py and lib/ from W/t/g (two levels below W, so the
harness's G is still W) and copied the output here; see notes/claude/round2/r2d.md §6.
"""
import json
import os

FILE_PICKER = 'io.github.win11shell.FilePicker'
SETTINGS = 'io.github.win11shell.Settings'
TITLE = 'Openen'

PICK_JS = (
    "(async()=>{const {bridge}=await import(new URL('/ui/js/core/bridge.js',location.href).href);"
    "window.__r2dPicks=window.__r2dPicks||[];"
    "window.__r2dPicks.push(bridge.call('shell.pickFile',%s).then(v=>({result:v}),e=>({code:e.code,message:e.message})));"
    "return window.__r2dPicks.length;})()")
PICK_DONE_JS = ("(async()=>Promise.race([Promise.all(window.__r2dPicks||[]),"
                "new Promise(r=>setTimeout(()=>r('still open after 5 s'),5000))]))()")
# Every window of the Settings process titled like a picker, oldest first, with its identity as GNOME sees it.
PICKERS_JS = (
    "(async()=>{const Shell=(await import('gi://Shell')).default;const T=Shell.WindowTracker.get_default();"
    "const type=t=>Object.keys(Meta.WindowType).find(k=>Meta.WindowType[k]===t)||String(t);"
    "return global.get_window_actors().map(a=>a.meta_window).filter(w=>(%d<=0||w.get_pid()===%d)&&w.get_title()===%s)"
    ".sort((a,b)=>a.get_stable_sequence()-b.get_stable_sequence()).map(w=>{const p=w.get_transient_for();"
    "const app=T.get_window_app(w);const r=w.get_frame_rect();return {seq:w.get_stable_sequence(),title:w.get_title(),"
    "wmClass:w.get_wm_class(),wmClassInstance:w.get_wm_class_instance(),gtkApplicationId:w.get_gtk_application_id(),"
    "type:type(w.get_window_type()),parent:p?p.get_wm_class():null,app:app?app.get_id():null,"
    "skipTaskbar:w.skip_taskbar,x:r.x,y:r.y,w:r.width,h:r.height};});})()")
FROSTED_JS = ("(()=>{const b=Main.extensionManager.lookup('%s')?.stateObj?._panelBridge?._frostedBridge;"
              "if(!b)return null;const c=b.capabilities();return {status:b.status(),"
              "windows:c.windows};})()")
CLOSE_JS = ("(()=>{const w=global.get_window_actors().map(a=>a.meta_window).find(w=>w.get_stable_sequence()===%d);"
            "if(!w)return false;w.delete(global.get_current_time());return true;})()")
# review m6: what the Win11 taskbar is told about each window of the Settings process (panelBridge windows.list:
# _userWindows() + _windowJSON(), the same code the panel's windows.list reads) ...
BRIDGE_WINDOWS_JS = ("(()=>{const b=Main.extensionManager.lookup('%s')?.stateObj?._panelBridge;if(!b)return null;"
                     "return b._userWindows().filter(w=>w.get_pid()===%d).map(w=>{const j=b._windowJSON(w);"
                     "return {seq:w.get_stable_sequence(),title:j.title,appId:j.appId,desktopId:j.desktopId};});})()")


def buttons(rows):
    """The taskbar's running buttons for these windows.list rows: one per app group (desktopId, else appId), as the
    taskbar groups them; ``multi`` when a group holds more than one window."""
    groups: dict = {}
    for row in rows or []:
        key = row.get('desktopId') or row.get('appId') or f"window-{row.get('seq')}"
        groups.setdefault(key, []).append(row)
    return [{'key': key, 'windows': len(items), 'running': True, 'multi': len(items) > 1}
            for key, items in sorted(groups.items())]


def run(h):
    build = os.environ.get('WIN11_BUILD_DIR', '')
    fixed = 'r1-debug' not in build             # every build since R2-D has the fix; r1-debug was the reference
    h.record('buildDir', build)
    h.require('session starts in Dutch', h.lang == 'nl', lang=h.lang)
    settings = h.open('settings')
    pid = settings.pid
    h.record('settingsPid', pid)

    def pickers():
        return h.eval(PICKERS_JS % (pid or 0, pid or 0, json.dumps(TITLE))) or []

    def taskbar(label):
        bridge = h.eval(BRIDGE_WINDOWS_JS % (h.hm.UUID, pid or 0))
        groups = buttons(bridge)
        return {'label': label, 'bridge': bridge, 'buttons': groups,
                'running': sorted(b['key'] for b in groups if b['running'])}

    h.sleep(1.5)                                # the extension's window list follows the Settings window
    bars = [taskbar('Settings only')]
    rows = []
    for n, options in enumerate(({'filters': [{'name': 'Afbeeldingen', 'patterns': ['*.png']}]}, {'modal': False}), 1):
        h.js('settings', PICK_JS % json.dumps(options))
        rows = h.until(lambda: (lambda r: r if len(r) >= n else None)(pickers()), f'picker {n} mapped', 30, 0.3)
        h.sleep(1.5)                            # let the extension's frosted sync and GNOME's tracker settle
        bars.append(taskbar(f'picker {n} open'))
        rows = pickers()
        last = rows[-1]
        margin = 40
        area = (max(0, last['x'] - margin), max(0, last['y'] - margin), last['w'] + 2 * margin, last['h'] + 2 * margin)
        h.screenshot(f'picker-{n}-{"modal" if n == 1 else "nonmodal"}', area=area)
    h.record('pickers', rows)
    frosted = h.record('frosted', h.eval(FROSTED_JS % h.hm.UUID))
    entries = ((frosted or {}).get('status') or {}).get('entries') or []
    # the extension's own-window backdrop/shadow entry whose rectangle is a picker's frame (None: no entry)
    h.record('pickerBackdrops', [{'seq': r['seq'], 'type': r['type'], 'entry': next(
        (e for e in entries if e.get('rect') == {'x': r['x'], 'y': r['y'], 'w': r['w'], 'h': r['h']}), None)}
        for r in rows])
    known = None if fixed else 'o2 (build without the R2-D fix: reference run)'
    for n, row in enumerate(rows, 1):
        h.check(f'picker {n} ({row["type"]}): Meta WM class is {FILE_PICKER} (o2)', row['wmClass'] == FILE_PICKER,
                known=known, row=row)
        h.check(f'picker {n}: GTK application id is the caller\'s, {SETTINGS} (review m6)',
                row['gtkApplicationId'] == SETTINGS, known=known, row=row)
        h.check(f'picker {n}: transient for the Settings window', row['parent'] == SETTINGS, row=row)
        h.check(f'picker {n}: GNOME groups it under Settings (WindowTracker, via the transient parent)',
                row['app'] in ('win11shell-settings.desktop', 'io.github.win11shell.Settings.desktop'), row=row)
    h.check('two pickers recorded', len(rows) == 2, rows=rows)
    # review m6: a picker adds no taskbar button of its own; the non-modal one (in the taskbar, skipTaskbar false)
    # joins the Settings button (two windows), as with the round-1 build
    h.record('taskbar', bars)
    alone = bars[0]['running']
    settings_button = lambda bar: next((b for b in bar['buttons'] if b['key'] in alone), None)  # noqa: E731
    h.check('taskbar: Settings has one running button before the pickers', len(alone) == 1, bar=bars[0])
    for n, bar in enumerate(bars[1:], 1):
        h.check(f'taskbar: picker {n} open: no button of its own (running buttons unchanged)',
                bar['running'] == alone, bar=bar)
    h.check('taskbar: the non-modal picker is grouped on the Settings button (2 windows)',
            bool(settings_button(bars[-1])) and settings_button(bars[-1])['multi'], bar=bars[-1])
    # close the modal picker first: while it holds GTK's grab, GTK drops a close request to any other window of the
    # process (ge1b o1; seen in the first r2d run, when the non-modal one was closed first)
    for row in sorted(rows, key=lambda r: r['type'] != 'MODAL_DIALOG'):
        h.eval(CLOSE_JS % row['seq'])
        try:
            h.until(lambda: row['seq'] not in [r['seq'] for r in pickers()], f'picker {row["seq"]} closed', 8, 0.2)
        except TimeoutError:
            h.check(f'picker {row["seq"]} closed on a window-manager close request', False)
    results = h.record('results', h.js('settings', PICK_DONE_JS, timeout=10))
    h.check('both shell.pickFile calls settle as cancelled (path null)',
            isinstance(results, list) and len(results) == 2
            and all((r.get('result') or {}).get('path', 'x') is None for r in results), results=results)
    h.close('settings')
    failed = [c['check'] for c in h.results['checks'] if not c['ok'] and not c.get('known')]
    h.require('every check passed', not failed, failed=failed, frostedWindows=(frosted or {}).get('windows'))
