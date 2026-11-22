"""Start teardown regression: Ubuntu 26.04 / Mutter 50.1, meta_window_get_workspaces abort.

Mutter emits workspace-changed after removing a closing window's workspace, before
unmanaged. A pending placement change must never move/resize that dying window.
An unmanaging callback switches this private bridge's alignment provider to make
that stale geometry path deterministic. Unfixed 0.9.2 aborts GNOME on the first
close; the lifecycle guard must preserve the compositor across button, Escape and
outside-click closes, including closes during the opening animation.

Run in the normal private GNOME harness with the release binaries. No real user
setting is changed, and all injected input belongs to that private compositor.
"""
POINTER_CLICK_JS = (
    "(async()=>{{const Clutter=(await import('gi://Clutter')).default;const GLib=(await import('gi://GLib')).default;"
    "const wait=ms=>new Promise(r=>GLib.timeout_add(GLib.PRIORITY_DEFAULT,ms,()=>{{r();return GLib.SOURCE_REMOVE;}}));"
    "const seat=Clutter.get_default_backend().get_default_seat();"
    "globalThis.__ge1hPointer??=seat.create_virtual_device(Clutter.InputDeviceType.POINTER_DEVICE);"
    "const d=globalThis.__ge1hPointer,us=()=>GLib.get_monotonic_time();"
    "d.notify_absolute_motion(us(),{x},{y});await wait(80);"
    "d.notify_button(us(),Clutter.BUTTON_PRIMARY,Clutter.ButtonState.PRESSED);await wait(50);"
    "d.notify_button(us(),Clutter.BUTTON_PRIMARY,Clutter.ButtonState.RELEASED);await wait(30);"
    "return global.get_pointer().slice(0,2);}})()")

BRIDGE = "Main.extensionManager.lookup('win11-gnome@win11shell.local').stateObj._panelBridge._frostedBridge"
KEYBOARD_JS = (
    "(async()=>{const Clutter=(await import('gi://Clutter')).default;const GLib=(await import('gi://GLib')).default;"
    "const seat=Clutter.get_default_backend().get_default_seat();"
    "globalThis.__minipcKeyboard??=seat.create_virtual_device(Clutter.InputDeviceType.KEYBOARD_DEVICE);"
    "ACTION;return true;})()")
ESCAPE_JS = KEYBOARD_JS.replace('ACTION',
    "const d=globalThis.__minipcKeyboard;"
    "d.notify_keyval(GLib.get_monotonic_time(),Clutter.KEY_Escape,Clutter.KeyState.PRESSED);"
    "await new Promise(r=>GLib.timeout_add(GLib.PRIORITY_DEFAULT,60,()=>{r();return GLib.SOURCE_REMOVE;}));"
    "d.notify_keyval(GLib.get_monotonic_time(),Clutter.KEY_Escape,Clutter.KeyState.RELEASED)")

def shown(h, pid):
    return any(w['pid'] == pid and w['wm'] == 'io.github.win11shell.Start'
               and w['visible'] and not w['minimized'] for w in h.s.windows())

def run(h):
    pid = h.native_pid
    h.require('Published native shell is running', bool(pid), pid=pid)
    h.require('Start blur is enabled in the private session', h.eval(f'{BRIDGE}._startEligible()'))
    token = h.eval("globalThis.__minipcStartCloseToken = 'same-private-session'")
    h.eval(KEYBOARD_JS.replace('ACTION', ''))
    h.sleep(0.4)
    h.record('initialBackdrop', h.eval(f'{BRIDGE}.status()'))
    h.eval(f"globalThis.__startCloseOriginalAlignment = {BRIDGE.replace('._frostedBridge', '')}._getStartAlignment; true")
    cycles = []
    for delay in (0.0, 0.06, 0.35):
        for method in ('button', 'escape', 'outside'):
            for repeat in range(2):
                label = f'{len(cycles)+1}: {method}, delay {delay}'
                opened = h.click('panel', 'tb-start')
                try:
                    h.until(lambda: shown(h, pid), label+' opens', 8, 0.05)
                except TimeoutError:
                    tops = h.native_state()
                    h.record(label+' normal-open timeout', {
                        'input': opened,
                        'native': [{key: top.get(key) for key in ('title','visible','mapped','active','focus','width','height')}
                                   for top in tops],
                        'bridge': h.bridge('shell.info'),
                        'meta': h.eval("(()=>{const b=" + BRIDGE.replace('._frostedBridge', '') + ";"
                            "const f=global.display.focus_window;return {focus:f?{wm:f.get_wm_class(),pid:f.get_pid()}:null,"
                            "search:b._panelSearch,exclusive:b._panelExclusive,phase:b._frostedBridge._startPhase.get("+str(pid)+"),"
                            "windows:global.get_window_actors().map(a=>({wm:a.meta_window.get_wm_class(),"
                            "gtkApp:a.meta_window.get_gtk_application_id(),pid:a.meta_window.get_pid(),"
                            "type:a.meta_window.get_window_type(),mapped:a.mapped,visible:a.visible}))};})()"),
                    })
                    raise
                h.until(lambda: any(e['kind'] == 'start' and e['effect']
                                    for e in h.eval(f'{BRIDGE}.status()')['entries']),
                        label+' has blur', 6, 0.05)
                armed = h.eval("""(()=>{
                    const b=Main.extensionManager.lookup('win11-gnome@win11shell.local').stateObj._panelBridge;
                    const w=global.get_window_actors().map(a=>a.meta_window)
                        .find(w=>w.get_wm_class()==='io.github.win11shell.Start');
                    if(!w)return false;
                    w.connect('unmanaging',()=>{
                        const old=b._getStartAlignment();
                        b._getStartAlignment=()=>old==='left'?'center':'left';
                    });
                    return true;
                })()""")
                h.require(label+' has a pending placement change', armed)
                h.sleep(delay)
                if method == 'button':
                    closed = h.click('panel', 'tb-start')
                elif method == 'escape':
                    h.until(lambda: h.eval("global.display.focus_window?.get_wm_class()") == 'io.github.win11shell.Start',
                            label+' has keyboard focus', 6, 0.05)
                    closed = h.eval(ESCAPE_JS)
                else:
                    closed = h.eval(POINTER_CLICK_JS.format(x=h.hm.WIDTH-20, y=20))
                h.until(lambda: not shown(h, pid), label+' closes', 8, 0.05)
                h.require(label+' keeps the native process', h.native_pid == pid)
                h.require(label+' keeps the same compositor session',
                          h.eval('globalThis.__minipcStartCloseToken') == token)
                backdrop = h.eval(f'{BRIDGE}.status()')
                h.check(label+' releases the Start backdrop',
                        not any(e['kind'] == 'start' for e in backdrop['entries']))
                cycles.append({'method': method, 'delay': delay, 'repeat': repeat,
                               'openInput': opened, 'closeInput': closed})
                h.sleep(0.4)
    h.record('cycles', cycles)
    h.record('finalBackdrop', h.eval(f'{BRIDGE}.status()'))
    h.eval(f"{BRIDGE.replace('._frostedBridge', '')}._getStartAlignment = globalThis.__startCloseOriginalAlignment; delete globalThis.__startCloseOriginalAlignment; true")
    h.screenshot('after-18-close-cycles', settle=0.3)
