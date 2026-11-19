"""Start rendering regression, private GNOME at 100% scale, Dutch language.

Run through tests/gnome/harness.py launch run --lang nl --source worktree.
Measures allocated GTK rectangles (not minimum-size properties), opens/closes
power dropdowns with real Escape input, checks full-frame blur redraw ownership,
and counts actual idle paint signals. Never targets the running desktop.
"""
BRIDGE = "Main.extensionManager.lookup('win11-gnome@win11shell.local').stateObj._panelBridge._frostedBridge"
FLAGS = "(await import('gi://Clutter')).default.get_debug_flags()[1]"
ESCAPE = """(async()=>{const C=(await import('gi://Clutter')).default;const G=(await import('gi://GLib')).default;
globalThis.__visualKeyboard??=C.get_default_backend().get_default_seat().create_virtual_device(C.InputDeviceType.KEYBOARD_DEVICE);
const d=globalThis.__visualKeyboard;d.notify_keyval(G.get_monotonic_time(),C.KEY_Escape,C.KeyState.PRESSED);
await new Promise(r=>G.timeout_add(G.PRIORITY_DEFAULT,40,()=>{r();return G.SOURCE_REMOVE;}));
d.notify_keyval(G.get_monotonic_time(),C.KEY_Escape,C.KeyState.RELEASED);return true;})()"""

def walk(node):
    if not node:
        return []
    return [node] + [n for c in node.get('children', []) for n in walk(c)]

def nodes(h):
    return [n for t in h.native_state() if t.get('visible')
            for n in walk(t.get('tree')) if n.get('rect')]

def run(h):
    h.click('panel', 'tb-start')
    h.until(lambda: any(n.get('name') == 'start-search' for n in nodes(h)), 'Start opens', 8)
    h.sleep(1)
    h.record('initialBlur', h.eval(f'{BRIDGE}.status()'))
    flags = h.eval(FLAGS)
    h.check('Active background blur disables clipped redraws', bool(flags & 2), flags=flags)
    h.check('Blur does not enable continuous redraw', not bool(flags & 64), flags=flags)
    h.screenshot('pinned-before')
    # A tiny pointer/hover update must not capture the previous frame's icons into the blur.
    for i in range(5):
        h.native({'native': 'click', 'name': 'start-power'})
        h.sleep(0.2)
        h.eval(ESCAPE)
        h.sleep(0.2)
        h.require('Dropdown close keeps Start open '+str(i), any(n.get('name') == 'start-search' for n in nodes(h)))
    h.screenshot('pinned-after-dropdowns')
    h.eval('global.stage.queue_redraw(); true')
    h.screenshot('pinned-full-redraw')
    for query in ('zzwinpointvisualprobezz', 'instellingen', 'verkenner'):
        h.native({'native': 'type', 'name': 'start-search', 'text': query})
        h.sleep(1.3)
        ns = nodes(h)
        tiles = [n for n in ns if 'w11-glyph-tile' in n.get('classes', [])]
        rows = [n for n in ns if 'w11-sr-row' in n.get('classes', [])]
        data = {'tiles': [{'rect': n['rect'], 'classes': n['classes']} for n in tiles],
                'rows': [{'rect': n['rect'], 'classes': n['classes']} for n in rows]}
        h.record(query, data)
        if query.startswith('zz'):
            h.check('Web-only search has glyph tiles', len(tiles) >= 2, count=len(tiles))
        h.check(query+' tiles stay square', all(abs(n['rect'][2]-n['rect'][3]) < 1 for n in tiles), **data)
        h.check(query+' result rows stay compact', bool(rows) and all(n['rect'][3] <= 80 for n in rows), **data)
        h.screenshot('search-'+query)
    h.native({'native': 'type', 'name': 'start-search', 'text': ''})
    h.sleep(0.4)
    h.record('finalBlur', h.eval(f'{BRIDGE}.status()'))
    # Verify release of only our redraw flag; respect an already-set flag from another extension.
    options = h.eval(f'({{enabled: true, ...{BRIDGE}._options}})')
    import json
    h.eval(f'{BRIDGE}.configure({{enabled:false}}); true')
    h.check('No backdrops restores clipped redraws', not bool(h.eval(FLAGS) & 2))
    h.eval("(async()=>{const C=(await import('gi://Clutter')).default; C.add_debug_flags(0,C.DrawDebugFlag.DISABLE_CLIPPED_REDRAWS,0); return true;})()")
    h.eval(f'{BRIDGE}.configure({json.dumps(options)}); true')
    h.eval(f'{BRIDGE}.configure({{enabled:false}}); true')
    h.check('Existing external redraw flag is preserved', bool(h.eval(FLAGS) & 2))
    h.eval("(async()=>{const C=(await import('gi://Clutter')).default; C.remove_debug_flags(0,C.DrawDebugFlag.DISABLE_CLIPPED_REDRAWS,0); return true;})()")
    h.eval(f'{BRIDGE}.configure({json.dumps(options)}); true')
    h.check('Restored blur reacquires full redraws', bool(h.eval(FLAGS) & 2))
    h.sleep(0.6)
    h.eval("(()=>{globalThis.__visualPaints=0; globalThis.__visualPaintSignal=global.stage.connect('after-paint',()=>{globalThis.__visualPaints++;}); return true;})()")
    h.sleep(2)
    paints = h.eval('(()=>{global.stage.disconnect(globalThis.__visualPaintSignal); return globalThis.__visualPaints;})()')
    h.check('Idle blur does not repaint continuously', paints < 10, paintsInTwoSeconds=paints)
