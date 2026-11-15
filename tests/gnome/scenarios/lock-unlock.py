"""The taskbar and Start after the lock screen (14-11).

Run (from G):
    tools/heavy.sh --min-mb 2560 --tag lock-unlock -- python3 -B tests/gnome/harness.py launch run --lang nl \\
        --build-dir tests/build/native-release --source worktree --scenario tests/gnome/scenarios/lock-unlock.py \\
        --out tests/gnome/scenarios/lock-unlock.out

Until 14-11 the taskbar did not come back after an unlock: the extension's disable() closed the panel window (and
GNOME's overview could throw halfway through it), win11-shell let GTK destroy it and ignored the activate that asked
for it again, and the new desktop window, briefly carrying the Panel's app ID, was taken for the taskbar.

GNOME disables extensions without "unlock-dialog" in their session-modes on the lock screen and enables them again on
unlock. The private session has no screen shield, so this takes the session-mode path the lock screen takes: push
'unlock-dialog', then pop it, which disables the extension and enables it again as a lock and unlock do. Checked:
the taskbar is back, covers the monitor (the extension took the right window), and the strip and Start are frosted.
"""
import time

EXT = 'win11-gnome@win11shell.local'
BRIDGE_JS = ("(()=>{const Meta=imports.gi.Meta;const e=Main.extensionManager.lookup('" + EXT + "');"
             "const b=e&&e.stateObj&&e.stateObj._panelBridge;const r=w=>{const f=w.get_frame_rect();return [f.x,f.y,f.width,f.height];};"
             "const wins=global.get_window_actors().map(a=>a.meta_window).map(w=>({wm:w.get_wm_class(),app:w.get_gtk_application_id(),"
             "title:w.get_title(),type:w.get_window_type(),rect:r(w),mapped:w.get_compositor_private()?.mapped??null}));"
             "return {bridge:!!b,panel:b&&b._panel?{title:b._panel.get_title(),type:b._panel.get_window_type(),rect:r(b._panel)}:null,"
             "present:b?b._panelPresent:null,windows:wins};})()")
STATE_JS = ("(()=>{const e=Main.extensionManager.lookup('" + EXT + "');return {state:e?e.state:null,"
            "mode:Main.sessionMode.currentMode,locked:Main.sessionMode.isLocked};})()")


def walk(node, out):
    if isinstance(node, dict):
        out.append(node)
        for child in node.get('children') or []:
            walk(child, out)
    return out


def toplevels(h):
    out = {}
    for top in h.native_state():
        glass = any('w11-glass' in (n.get('classes') or []) for n in walk(top.get('tree'), []))
        out[top.get('title') or '?'] = {'visible': top.get('visible'), 'glass': glass,
                                        'size': [top.get('width'), top.get('height')]}
    return out


def run(h):
    h.sleep(5)
    before = toplevels(h)
    h.record('before', before)
    h.record('bridge before', h.eval(BRIDGE_JS))
    h.check('the taskbar is shown before the lock', any('taak' in t.lower() or 'taskbar' in t.lower() for t, v in before.items()
                                                       if v['visible'] is not False), toplevels=before)
    h.screenshot('before-lock')
    h.record('lock', h.eval("(()=>{Main.sessionMode.pushMode('unlock-dialog');return Main.sessionMode.currentMode;})()"))
    h.sleep(4)
    h.record('locked state', h.eval(STATE_JS))
    h.screenshot('locked')
    h.record('unlock', h.eval("(()=>{Main.sessionMode.popMode('unlock-dialog');return Main.sessionMode.currentMode;})()"))
    h.sleep(8)
    h.record('unlocked state', h.eval(STATE_JS))
    after = toplevels(h)
    h.record('after', after)
    h.record('bridge after', h.eval(BRIDGE_JS))
    h.check('the taskbar is shown after unlock', any('taak' in t.lower() or 'taskbar' in t.lower() for t, v in after.items()
                                                    if v['visible'] is not False), toplevels=after)
    bar = next((v for t, v in after.items() if 'taak' in t.lower() or 'taskbar' in t.lower()), None)
    h.check('the taskbar covers the whole monitor again (the extension took it, not the desktop)',
            bool(bar) and bar['size'][1] == before.get('Taakbalk', before.get('Taskbar', {})).get('size', [0, -1])[1],
            bar=bar)
    h.check('the taskbar strip is frosted after unlock', bool(bar) and bar['glass'], bar=bar)
    h.screenshot('after-unlock')
    h.open('start')
    time.sleep(1.5)
    with_start = toplevels(h)
    h.record('start open after unlock', with_start)
    h.check('Start is frosted after unlock', any(v['glass'] for t, v in with_start.items() if 'start' in t.lower()),
            toplevels=with_start)
    h.screenshot('start-after-unlock')
