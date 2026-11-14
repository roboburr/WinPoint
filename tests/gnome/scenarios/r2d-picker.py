"""R2-D scenario: the file picker's own window identity (GE1b observation o2) in the real product.

History: GE1b's review saw both pickers of the WebKit Settings with the WM class io.github.win11shell.Settings
(GTK 3 took the caller's app id); R2-D gave the picker window io.github.win11shell.FilePicker, and its fix round
(review m6) kept the caller's GTK application id so the extension's windows.list did not give a non-modal picker a
taskbar button of its own. Those pickers were windows of the caller's own process (shell.pickFile).

13-11 (WebKit removal): the picker is the native GTK 4 Explorer in picker mode (`win11-gtk-explorer
--picker-stdio`, native/apps/explorer/src/picker.rs), a child process of the file portal (win11-file-portal,
host/src/portal.rs and picker.rs show_native), and shell.pickFile is gone. A caller reaches it as every app does:
GTK's file dialog asks xdg-desktop-portal's FileChooser, which the installed portal configuration
(run/config/xdg-desktop-portal/gnome-portals.conf, scripts/file_portal.py) gives to Win11's backend. The private
session bus activates nothing, so this starts the two portal processes itself. An unsandboxed GTK 4 app shows its
own GtkFileChooserDialog unless GDK_DEBUG=portals (a first run of this port saw exactly that, a dialog of the
Settings process), while a Flatpak app always asks the portal: since 14-11 the native apps set it for themselves
(native/kit app.rs own_environment), so Settings starts without it and the helper gets it.

1. The portal stack: win11-file-portal owns org.freedesktop.impl.portal.desktop.win11shell (idle exit 300 s here),
   then xdg-desktop-portal owns org.freedesktop.portal.Desktop.
2. Two callers: the native Settings on Achtergrond, whose "Foto's zoeken" (background-browse, clicked with the
   pointer) opens a modal picker for its window; then a GTK 4 app of somebody else (picker-helper.py,
   org.example.PickerTest), which opens a non-modal one for its window.
3. Per picker window: Meta's WM class, GTK application id, window type, title, the transient parent's WM class,
   the app GNOME's WindowTracker gives it, skip_taskbar, its process (and that process's parent), and the
   extension's frosted-backdrop entry for its frame (frostedBridge.js windowIdentity() keeps a FilePicker out of
   the own-app shadow on purpose); a screenshot of each with a 40 px margin. The Win11 taskbar before and with
   each picker open: the extension's windows.list (D-G70: the native taskbar draws its buttons from this list),
   grouped by app as the taskbar groups its buttons.
4. Checks: the WM class is io.github.win11shell.FilePicker; the window is the portal's picker process; it is
   transient for the caller's window and GNOME groups it under the caller (WindowTracker, via the transient
   parent); the modal one is a modal dialog of its caller and the other is not; the title is the caller's;
   it has the focus (both callers are focused when they ask; GNOME's "is ready" banners are recorded, then dropped
   before the screenshots); skip_taskbar, so no picker is in windows.list and the taskbar's buttons stay as they
   were; a window-manager close ends each picker, and its caller hears "cancelled" (the helper writes path null;
   Settings keeps its wallpaper). The non-modal picker is a normal window with a parent, which Mutter keeps in the
   task list (13-11: known picker-nonmodal-taskbar, on its caller's button); since 14-11 the extension hides a
   picker with a parent from the window list (panelBridge.js _considerPicker), as Windows keeps an owned window off
   the taskbar.

Run (from G), in nl and in en:
    tools/heavy.sh --min-mb 2560 --tag r2d-picker -- python3 -B tests/gnome/harness.py launch run --lang nl \\
        --build-dir tests/build/native-release --source worktree --scenario tests/gnome/scenarios/r2d-picker.py \\
        --out tests/gnome/scenarios/r2d-picker.out/nl
"""
import json
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
FILE_PICKER = 'io.github.win11shell.FilePicker'
SETTINGS = 'io.github.win11shell.Settings'
HELPER = 'org.example.PickerTest'
BACKEND = 'org.freedesktop.impl.portal.desktop.win11shell'
FRONTEND = 'org.freedesktop.portal.Desktop'
# the dialog titles the callers give (native/apps/settings/src/pages/background.rs; picker-helper.py)
TITLE = {SETTINGS: {'nl': 'Een foto kiezen voor je bureaublad', 'en': 'Choose a photo for your desktop'},
         HELPER: {'nl': 'Openen', 'en': 'Openen'}}
# GTK 4 outside a sandbox only asks the portal for a file dialog with this (gdk_display_should_use_portal)
PORTALS = {'GDK_DEBUG': 'portals'}

# Every window with its identity as GNOME sees it, oldest first.
WINDOWS_JS = (
    "(async()=>{const Shell=(await import('gi://Shell')).default;const T=Shell.WindowTracker.get_default();"
    "const type=t=>Object.keys(Meta.WindowType).find(k=>Meta.WindowType[k]===t)||String(t);"
    "return global.get_window_actors().map(a=>a.meta_window)"
    ".sort((a,b)=>a.get_stable_sequence()-b.get_stable_sequence()).map(w=>{const p=w.get_transient_for();"
    "const app=T.get_window_app(w);const r=w.get_frame_rect();return {seq:w.get_stable_sequence(),pid:w.get_pid(),"
    "title:w.get_title(),wmClass:w.get_wm_class(),gtkApplicationId:w.get_gtk_application_id(),"
    "type:type(w.get_window_type()),parent:p?p.get_wm_class():null,parentSeq:p?p.get_stable_sequence():null,"
    "attached:w.is_attached_dialog(),app:app?app.get_id():null,skipTaskbar:w.skip_taskbar,focus:w.has_focus(),"
    "x:r.x,y:r.y,w:r.width,h:r.height};});})()")
# GNOME's "“…” is ready" banners (a window that asked for the focus without getting it): read, then dropped so they
# do not cover the pickers in the screenshots
READY_JS = ("Main.messageTray.getSources().flatMap(s=>(s.notifications||[]).filter(n=>/ is ready$/.test(n.body||''))"
            ".map(n=>({source:s.title,title:n.title,body:n.body})))")
DROP_READY_JS = ("Main.messageTray.getSources().forEach(s=>(s.notifications||[]).filter(n=>/ is ready$/.test(n.body||''))"
                 ".forEach(n=>n.destroy()))||true")

FROSTED_JS = ("(()=>{const b=Main.extensionManager.lookup('%s')?.stateObj?._panelBridge?._frostedBridge;"
              "if(!b)return null;const c=b.capabilities();return {status:b.status(),windows:c.windows};})()")
CLOSE_JS = ("(()=>{const w=global.get_window_actors().map(a=>a.meta_window).find(w=>w.get_stable_sequence()===%d);"
            "if(!w)return false;w.delete(global.get_current_time());return true;})()")
# review m6: what the Win11 taskbar is told about every window (panelBridge windows.list: _userWindows() +
# _windowJSON(), the same code the native taskbar's windows.list reads)
BRIDGE_WINDOWS_JS = ("(()=>{const b=Main.extensionManager.lookup('%s')?.stateObj?._panelBridge;if(!b)return null;"
                     "return b._userWindows().map(w=>{const j=b._windowJSON(w);"
                     "return {seq:w.get_stable_sequence(),pid:w.get_pid(),title:j.title,appId:j.appId,"
                     "desktopId:j.desktopId};});})()")


def buttons(rows):
    """The taskbar's running buttons for these windows.list rows: one per app group (desktopId, else appId), as the
    taskbar groups them, with how many windows each holds."""
    groups: dict = {}
    for row in rows or []:
        key = row.get('desktopId') or row.get('appId') or f"window-{row.get('seq')}"
        groups.setdefault(key, []).append(row['seq'])
    return {key: len(seqs) for key, seqs in sorted(groups.items())}


def has_owner(h, name):
    GLib, Gio = h.s.GLib, h.s.Gio
    return h.s.bus.call_sync('org.freedesktop.DBus', '/org/freedesktop/DBus', 'org.freedesktop.DBus', 'NameHasOwner',
                             GLib.Variant('(s)', (name,)), None, Gio.DBusCallFlags.NONE, 5000, None).unpack()[0]


def process(h, pid):
    """The comm of a process and of its parent (the harness's proc_table)."""
    table = h.hm.proc_table()
    row = table.get(pid) or {}
    return {'comm': row.get('comm'), 'parent': (table.get(row.get('ppid')) or {}).get('comm')}


def wallpaper(h):
    try:
        data = json.loads((h.hm.RUN / 'config' / 'win11-gnome' / 'settings.json').read_text() or '{}')
    except (OSError, ValueError):
        return None
    return data.get('wallpaper')


def run(h):
    lang = h.lang
    windows = lambda: h.eval(WINDOWS_JS) or []                                  # noqa: E731
    taskbar = lambda label: (lambda rows: {'label': label, 'rows': rows, 'buttons': buttons(rows)})(  # noqa: E731
        h.eval(BRIDGE_WINDOWS_JS % h.hm.UUID))

    # 1. the portal stack
    bindir = h.hm.RUN / 'prefix' / 'bin'
    h.spawn('file-portal', [str(bindir / 'win11-file-portal')], {'WIN11_FILE_PORTAL_IDLE': '300',
                                                                 'WIN11_FILE_PORTAL_TRACE': '1'})
    h.require('the Win11 FileChooser backend is on the bus',
              h.until(lambda: has_owner(h, BACKEND), BACKEND, 20, 0.2), bus=BACKEND)
    h.spawn('xdg-desktop-portal', ['/usr/libexec/xdg-desktop-portal', '--verbose'])
    h.require('xdg-desktop-portal is on the bus', h.until(lambda: has_owner(h, FRONTEND), FRONTEND, 20, 0.2),
              bus=FRONTEND)

    # 2a. Settings on Achtergrond, its picker (modal)
    settings = h.open('settings:background')
    h.sleep(1.5)                                # the extension's window list follows the Settings window
    before_wallpaper = wallpaper(h)
    bars = [taskbar('Settings only')]
    seen = {w['seq'] for w in windows()}
    h.record('browse.click', h.click('settings', 'background-browse'))
    pickers, ready = [], []

    def picker_after(seen, label, shot):
        found = h.until(lambda: [w for w in windows() if w['seq'] not in seen and w['w'] > 0] or None, label, 30, 0.3)
        h.sleep(1.5)                            # let the extension's frosted sync and GNOME's tracker settle
        row = [w for w in windows() if w['seq'] in {x['seq'] for x in found}][-1]
        ready.extend({'after': f'picker {len(pickers) + 1}', **b} for b in h.eval(READY_JS) or [])
        h.eval(DROP_READY_JS)
        margin = 40
        h.screenshot(shot, area=(max(0, row['x'] - margin), max(0, row['y'] - margin), row['w'] + 2 * margin,
                                 row['h'] + 2 * margin))
        return row

    pickers.append({'caller': SETTINGS, 'callerPid': settings.pid, 'modal': True,
                    'row': picker_after(seen, 'picker 1 (Settings) mapped', 'picker-1-modal')})
    bars.append(taskbar('picker 1 open'))

    # 2b. a GTK 4 app of somebody else, its picker (not modal)
    state_dir = h.hm.RUN / 'tmp' / 'picker'
    state_dir.mkdir(parents=True, exist_ok=True)
    answers = state_dir / 'picker-helper.jsonl'
    seen = {w['seq'] for w in windows()}
    helper = h.spawn('picker-helper', [sys.executable, '-I', str(HERE / 'picker-helper.py'), str(state_dir), 'nonmodal'],
                     PORTALS)
    main = h.until(lambda: next((w for w in windows() if w['wmClass'] == HELPER and w['seq'] not in seen
                                 and w['type'] == 'NORMAL' and w['w'] > 0), None), 'the helper window', 30, 0.3)
    h.s.focus(main['pid'], 'picker-helper')    # as a launch from Start would; then its dialog, like a click in it
    (state_dir / 'picker-helper.open').touch()
    pickers.append({'caller': HELPER, 'callerPid': helper.pid, 'modal': False,
                    'row': picker_after(seen | {main['seq']}, 'picker 2 (the helper) mapped', 'picker-2-nonmodal')})
    bars.append(taskbar('helper and picker 2 open'))
    h.record('helperWindow', main)
    h.record('readyBanners', ready)

    for p in pickers:
        p['process'] = process(h, p['row']['pid'])
    frosted = h.record('frosted', h.eval(FROSTED_JS % h.hm.UUID))
    entries = ((frosted or {}).get('status') or {}).get('entries') or []
    h.record('pickers', pickers)
    h.record('pickerBackdrops', [{'seq': p['row']['seq'], 'entry': next(
        (e for e in entries if e.get('rect') == {'x': p['row']['x'], 'y': p['row']['y'], 'w': p['row']['w'],
                                                 'h': p['row']['h']}), None)} for p in pickers])
    caller_rows = {w['wmClass']: w for w in windows() if w['type'] == 'NORMAL' and w['wmClass'] in (SETTINGS, HELPER)}
    for n, p in enumerate(pickers, 1):
        row, caller = p['row'], caller_rows.get(p['caller']) or {}
        h.check(f'picker {n} ({row["type"]}): Meta WM class is {FILE_PICKER} (o2)', row['wmClass'] == FILE_PICKER, row=row)
        h.check(f'picker {n}: the portal\'s picker process (win11-gtk-explorer under win11-file-portal)',
                p['process'] == {'comm': 'win11-gtk-explo', 'parent': 'win11-file-port'}, process=p['process'],
                pid=row['pid'])
        h.check(f'picker {n}: transient for the caller\'s window ({p["caller"]})',
                row['parent'] == p['caller'] and row['parentSeq'] == caller.get('seq'), row=row, caller=caller)
        h.check(f'picker {n}: GNOME groups it under the caller (WindowTracker, via the transient parent)',
                row['app'] is not None and row['app'] == caller.get('app'), row=row, caller=caller)
        if p['modal']:
            h.check(f'picker {n}: a modal dialog of its caller', row['type'] == 'MODAL_DIALOG', row=row)
        else:
            h.check(f'picker {n}: not modal (modal: false)', row['type'] != 'MODAL_DIALOG', row=row)
        h.check(f'picker {n}: the caller\'s title', row['title'] == TITLE[p['caller']][lang], row=row)
        h.check(f'picker {n}: it has the focus (opened from its focused caller), no "is ready" banner',
                row['focus'] and not [b for b in ready if b['title'] == FILE_PICKER and b['after'] == f'picker {n}'],
                row=row, banners=ready)
        h.check(f'picker {n}: skip-taskbar', row['skipTaskbar'] is True, row=row)
    h.check('two pickers recorded', len(pickers) == 2, pickers=pickers)
    # review m6: no picker on the taskbar, modal or not (it is skip-taskbar): not in windows.list, and the running
    # buttons are the callers' alone
    h.record('taskbar', bars)
    picker_seqs = {p['row']['seq'] for p in pickers}
    alone = bars[0]['buttons']
    for n, bar in enumerate(bars[1:], 1):
        listed = [r for r in bar['rows'] or [] if r['seq'] in picker_seqs]
        h.check(f'taskbar: picker {n} open: no picker in windows.list', not listed, listed=listed, bar=bar)
    h.check('taskbar: picker 1 open: no button of its own, Settings\'s button as before', bars[1]['buttons'] == alone,
            bar=bars[1])
    helper_keys = {r.get('desktopId') or r.get('appId') or f"window-{r['seq']}" for r in bars[2]['rows'] or []
                   if r['pid'] == helper.pid}
    h.check('taskbar: picker 2 open: no button of its own (the buttons are Settings\'s and the helper\'s)',
            set(bars[2]['buttons']) == set(alone) | helper_keys and len(helper_keys) == 1, bar=bars[2],
            helperKeys=sorted(helper_keys))
    h.check('taskbar: picker 2 open: Settings\'s button as before, the helper\'s holds its one window',
            {k: v for k, v in bars[2]['buttons'].items() if k not in helper_keys} == alone
            and all(bars[2]['buttons'][k] == 1 for k in helper_keys), bar=bars[2], helperKeys=sorted(helper_keys))

    # close the modal picker first, then the other one (a window-manager close request: the picker cancels)
    for p in sorted(pickers, key=lambda p: not p['modal']):
        seq, pid = p['row']['seq'], p['row']['pid']
        h.eval(CLOSE_JS % seq)
        try:
            h.until(lambda: seq not in {w['seq'] for w in windows()}, f'picker {seq} closed', 8, 0.2)
            closed = True
        except TimeoutError:
            closed = False
        h.check(f'picker of {p["caller"]}: a window-manager close request closes it', closed, seq=seq)
        try:
            h.until(lambda: pid not in h.hm.proc_table(), f'picker process {pid} ended', 8, 0.2)
            ended = True
        except TimeoutError:
            ended = False
        h.check(f'picker of {p["caller"]}: its process ends', ended, pid=pid)
    try:
        answer = h.until(lambda: answers.read_text().strip() or None, 'the helper\'s answer', 10, 0.2)
        rows = [json.loads(line) for line in answer.splitlines()]
    except (TimeoutError, ValueError) as error:
        rows = [{'error': str(error)}]
    h.record('helperAnswers', rows)
    h.check('the helper hears "cancelled" (path null)', len(rows) == 1 and 'path' in rows[0] and rows[0]['path'] is None,
            answers=rows)
    h.check('Settings keeps its wallpaper (nothing was chosen)', wallpaper(h) == before_wallpaper,
            before=before_wallpaper, after=wallpaper(h))
    helper.terminate()
    h.close('settings')
    failed = [c['check'] for c in h.results['checks'] if not c['ok'] and not c.get('known')]
    h.require('every check passed', not failed, failed=failed, frostedWindows=(frosted or {}).get('windows'))
