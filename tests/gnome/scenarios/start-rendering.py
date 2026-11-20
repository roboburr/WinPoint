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

def move(h, x, y):
    return h.eval("(async()=>{const C=(await import('gi://Clutter')).default;const G=(await import('gi://GLib')).default;"
        "globalThis.__visualPointer??=C.get_default_backend().get_default_seat().create_virtual_device(C.InputDeviceType.POINTER_DEVICE);"
        f"globalThis.__visualPointer.notify_absolute_motion(G.get_monotonic_time(),{int(x)},{int(y)});"
        "return global.get_pointer().slice(0,2);})()")

def changed_pixels(h, before, after, region, exclude=None):
    images = [h.s.GdkPixbuf.Pixbuf.new_from_file(str(h.hm.RUN/'shots'/p)) for p in (before, after)]
    a,b = images
    da,db = a.get_pixels(),b.get_pixels()
    x0,y0,w,ht = region
    changed = 0
    for y in range(y0,y0+ht):
        for x in range(x0,x0+w):
            if exclude and exclude[0]-2 <= x < exclude[0]+exclude[2]+2 and exclude[1]-2 <= y < exclude[1]+exclude[3]+2:
                continue
            ia=y*a.get_rowstride()+x*a.get_n_channels()
            ib=y*b.get_rowstride()+x*b.get_n_channels()
            if max(abs(da[ia+c]-db[ib+c]) for c in range(3)) > 2:
                changed += 1
    return changed

def run(h):
    h.click('panel', 'tb-start')
    h.until(lambda: any(n.get('name') == 'start-search' for n in nodes(h)), 'Start opens', 8)
    h.sleep(1)
    h.record('initialBlur', h.eval(f'{BRIDGE}.status()'))
    flags = h.eval(FLAGS)
    h.check('Desktop blur leaves clipped redraws enabled', not bool(flags & 2), flags=flags)
    entries=h.eval(f'{BRIDGE}.status()')['entries']
    h.check('Glass uses isolated underlying window sources', all(e.get('source')=='windows' and e.get('mode')==0 for e in entries if e['effect']), entries=entries)
    h.check('Blur does not enable continuous redraw', not bool(flags & 64), flags=flags)
    before = h.screenshot('pinned-before')
    frame = h.s.window_of('io.github.win11shell.Start', h.native_pid)
    pins = [n for n in nodes(h) if str(n.get('name','')).startswith('start-pin-') and n['rect'][2]>0]
    band = (frame['x']+32, frame['y']+124, 578, 300)
    for key in ('settings','taskmgr','calculator','libreoffice'):
        tile = next((n for n in pins if key in n['name'].lower()), None)
        h.require('Hover target exists: '+key, bool(tile))
        x,y,w,ht = tile['rect'];rect=(frame['x']+x,frame['y']+y,w,ht)
        move(h,rect[0]+w//2,rect[1]+ht//2)
        # Let both the hover transition and its tooltip settle before comparing frames.
        h.sleep(0.6)
        pointer=h.eval('global.get_pointer().slice(0,2)')
        shot=h.screenshot('hover-'+key,settle=0.15)
        inside=changed_pixels(h,before,shot,rect)
        h.check('Real pointer hovers its tile: '+key,inside>10 and pointer==[rect[0]+w//2,rect[1]+ht//2],
                changedInside=inside,pointer=pointer,rect=rect)
        h.eval('global.stage.queue_redraw()')
        reference=h.screenshot('hover-full-frame-'+key,settle=0.15)
        changed=changed_pixels(h,shot,reference,band)
        h.check('Hover matches full-frame reference: '+key,changed<=30,changedPixels=changed)
        move(h,frame['x']+320,frame['y']+500)
        h.sleep(0.3)
    after=h.screenshot('pinned-after-hovers',settle=0.2)
    changed=changed_pixels(h,before,after,band)
    h.check('Pointer leaving removes every hover without trails',changed<=30,changedPixels=changed)
    # Repeated menu changes must not capture the previous frame's foreground into the blur.
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
    h.check('Restored blur leaves redraw flags unchanged', not bool(h.eval(FLAGS) & 2))
    h.sleep(0.6)
    h.eval("(()=>{globalThis.__visualPaints=0; globalThis.__visualPaintSignal=global.stage.connect('after-paint',()=>{globalThis.__visualPaints++;}); return true;})()")
    h.sleep(2)
    paints = h.eval('(()=>{global.stage.disconnect(globalThis.__visualPaintSignal); return globalThis.__visualPaints;})()')
    h.check('Idle blur does not repaint continuously', paints < 10, paintsInTwoSeconds=paints)
