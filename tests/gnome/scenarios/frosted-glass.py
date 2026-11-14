"""Frosted glass (D-G59): GNOME's own blur behind the taskbar, the focused own app window and the open Start menu.

Robin (2026-11-12): "is frosted blur mogelijk? net als de python versie? maar dan in rust", "let wel op ram gebruik",
"blur aan", and for the taskbar "de blur is al goed maar mag doorzichtiger". The reference host's in-page blur
(WebKit on the GPU) froze the taskbar on NVIDIA; this uses extension/frostedBridge.js (Shell.BlurEffect in
BACKGROUND mode) under a tint: the taskbar 70 %, the rest 90 % (D-G60, "alles behalve de taakbalk mag op 90%
opacity").

D-G60 (Robin, live after installing D-G59): "de start menu heeft nu geen blur", windows "glitchy bij het bewegen" and
Start "hapert een beetje" when opening. Resident Start's window is unmanaged on every close, which dropped its
registration, so only the first open had glass (D2 reopens it); a drag destroyed and rebuilt the window blur on every
step (F); Start waited for the blur call while opening (now sent without waiting).

D-G70: the taskbar and Start are the native shell's (win11-shell, GTK 4): their tint is GTK CSS (shell/css), not a
page, so A, C, D and D2 read the extension's side (frostedBridge _hosts and capabilities(), as native-shell.py does).
13-11: Settings is the native app (win11-gtk-settings): its Mica over the blur is the kit's (native/kit/css/frame.css,
window.w11-glass, 90 %), read from its dev state (native/kit/src/dev.rs) with its caption buttons and width.

Run (from G; a build of this tree and the tree's UI and extension):
    tools/heavy.sh --min-mb 2560 --tag frosted -- python3 -B tests/gnome/harness.py launch run --lang nl \\
        --build-dir tests/build/native-release --source worktree --scenario tests/gnome/scenarios/frosted-glass.py

  A. The taskbar: one blur entry for the 48 px strip; the extension holds the native pid as Panel and lists its
     taskbar with blur in capabilities().
  B. Settings focused: one window blur (the rim inset 3 px, not 8); the window wears the kit's glass (w11-glass). That
     its Mica is 90 % over the blur is the kit's unit test (native/kit/src/theme.rs, tokens.css and frame.css).
  C. Start open: a Start blur, Settings' blur gone (it lost the focus), so one window blur in all; the extension
     holds Panel and Start under the native pid and lists a Start with blur.
  D. Start closed: its blur is released at once (not only when the window hides); Settings has its blur again;
     the extension keeps the hidden Start's registration. D2: reopened, Start has its blur again, and closes it again.
  F. Settings dragged in 12 steps: its blur stays (no entry rebuilt), follows the drawn window, and sits on the frame
     once the drag ends.
  G. A real drag with a virtual pointer (Robin, 12-11, after the first D-G60 build: windows could not be dragged,
     content stretched on resize, the caption buttons were gone): the title bar moves the window, the right edge
     resizes it, and afterwards the content is not stretched (window actor = buffer, the app's own window width =
     frame width) and the three caption buttons are inside the window.
  E. Memory: gnome-shell PSS with Settings open and the window blurs switched off, versus Settings focused with its
     blur and Start open, each after forced paints; the extra must stay below 64 MiB (earlier 1080p: 19.5 MiB).
"""
import json
import os

BRIDGE = "Main.extensionManager.lookup('win11-gnome@win11shell.local').stateObj._panelBridge"
STATUS_JS = f"JSON.stringify({BRIDGE}._frostedBridge.status())"
CONFIGURE_JS = (f"JSON.stringify({BRIDGE}._frostedBridge.configure({{enabled: true, blur: true, windowBlur: %s, "
                "shadows: true, radius: 14}))")
PAINT_JS = "(()=>{global.stage.queue_redraw();return true;})()"
# Settings moved 12 times by 15 px, 40 ms apart; after each step: does it have its blur, does the blur sit under the
# drawn window (its actor, which can be a frame behind its frame), and how many entries were created meanwhile
# (status().created). Then back; SETTLED_JS: once the drawn window is on its frame again, so is the blur (headless,
# the client can take a while before Mutter draws the window at its new place).
DRAG_JS = (f"(async()=>{{const GLib=(await import('gi://GLib')).default;const f={BRIDGE}._frostedBridge;"
           "const w=global.get_window_actors().map(a=>a.meta_window).find(w=>w.get_wm_class()==='io.github.win11shell.Settings');"
           "if(!w)return {error:'no Settings window'};const start=f.status().created;const steps=[];"
           "const r0=w.get_frame_rect();"
           "for(let i=1;i<=12;i++){w.move_frame(true,r0.x+15*i,r0.y+(i%2)*6);"
           "await new Promise(r=>GLib.timeout_add(GLib.PRIORITY_DEFAULT,40,()=>{r();return GLib.SOURCE_REMOVE;}));"
           "const [ax,ay]=w.get_compositor_private().get_transformed_position();"
           "const e=f.status().entries.find(x=>x.kind==='window');"
           "steps.push({x:Math.round(ax),y:Math.round(ay),blur:Boolean(e&&e.effect),"
           "follows:Boolean(e&&e.rect&&Math.abs(e.rect.x-ax)<=2&&Math.abs(e.rect.y-ay)<=2)});}"
           "const rebuilt=f.status().created-start;w.move_frame(true,r0.x,r0.y);return {steps,rebuilt,back:[r0.x,r0.y]};})()")
REAL_DRAG_JS = (
    "(async()=>{const Clutter=(await import('gi://Clutter')).default;const GLib=(await import('gi://GLib')).default;"
    "const wait=ms=>new Promise(r=>GLib.timeout_add(GLib.PRIORITY_DEFAULT,ms,()=>{r();return GLib.SOURCE_REMOVE;}));"
    "const seat=Clutter.get_default_backend().get_default_seat();"
    "globalThis.__w11DragPointer??=seat.create_virtual_device(Clutter.InputDeviceType.POINTER_DEVICE);"
    "const p=globalThis.__w11DragPointer;const us=()=>GLib.get_monotonic_time();"
    "const w=global.get_window_actors().map(a=>a.meta_window).find(w=>w.get_wm_class()==='io.github.win11shell.Settings');"
    "if(!w)return {error:'no Settings window'};Main.activateWindow(w,global.display.get_current_time_roundtrip());await wait(400);"
    "const f=w.get_frame_rect();const x0=f.x+Math.round(f.width*0.6),y0=f.y+14;"
    "p.notify_absolute_motion(us(),x0,y0);await wait(100);"
    "p.notify_button(us(),Clutter.BUTTON_PRIMARY,Clutter.ButtonState.PRESSED);await wait(100);"
    "for(let i=1;i<=12;i++){p.notify_absolute_motion(us(),x0+8*i,y0+5*i);await wait(30);}"
    "p.notify_button(us(),Clutter.BUTTON_PRIMARY,Clutter.ButtonState.RELEASED);await wait(600);"
    "const m=w.get_frame_rect();const ex=m.x+m.width-2,ey=m.y+Math.round(m.height/2);"
    "p.notify_absolute_motion(us(),ex,ey);await wait(100);"
    "p.notify_button(us(),Clutter.BUTTON_PRIMARY,Clutter.ButtonState.PRESSED);await wait(100);"
    "for(let i=1;i<=10;i++){p.notify_absolute_motion(us(),ex+8*i,ey);await wait(30);}"
    "p.notify_button(us(),Clutter.BUTTON_PRIMARY,Clutter.ButtonState.RELEASED);await wait(1500);"
    "const r=w.get_frame_rect(),b=w.get_buffer_rect(),a=w.get_compositor_private();"
    "return {moved:{dx:m.x-f.x,dy:m.y-f.y},resized:{dw:r.width-m.width},frame:{w:r.width,h:r.height},"
    "buffer:{w:b.width,h:b.height},actor:{w:Math.round(a.width),h:Math.round(a.height)}};})()")
SETTLED_JS = (f"(()=>{{const f={BRIDGE}._frostedBridge;"
              "const w=global.get_window_actors().map(a=>a.meta_window).find(w=>w.get_wm_class()==='io.github.win11shell.Settings');"
              "const r=w.get_frame_rect();const [ax,ay]=w.get_compositor_private().get_transformed_position();"
              "const e=f.status().entries.find(x=>x.kind==='window');"
              "return {frame:[r.x,r.y],actor:[Math.round(ax),Math.round(ay)],blur:e?[e.rect.x,e.rect.y]:null,"
              "drawn:Math.abs(ax-r.x)<=2&&Math.abs(ay-r.y)<=2,ok:Boolean(e&&e.effect&&e.rect.x===r.x&&e.rect.y===r.y)};})()")
HOSTS_JS = (f"(()=>{{const f={BRIDGE}._frostedBridge;return Object.fromEntries([...f._hosts].map(([p,ids])=>"
            "[String(p),[...ids]]));})()")
CAPS_JS = f"JSON.stringify({BRIDGE}._frostedBridge.capabilities())"
PANEL, START = 'io.github.win11shell.Panel', 'io.github.win11shell.Start'


def status(h):
    return json.loads(h.eval(STATUS_JS))


def entries(h, kind):
    return [e for e in status(h)['entries'] if e['kind'] == kind]


def captions(h, surface):
    """The app's three caption buttons (native/kit/src/frame.rs: <kind>-minimize, -maximize and -close) that lie inside
    its window, and that window's own width and height (its dev state)."""
    kind = h.surface(surface).kind
    tops = [t for t in h.state(surface) if t.get('visible')]
    top = tops[0] if tops else {}
    names = [f'{kind}-{b}' for b in ('minimize', 'maximize', 'close')]
    rects = [n['rect'] for n in h.find(surface, state=[top], where=lambda n: n.get('name') in names) if n.get('rect')]
    inside = [r for r in rects if r[2] > 0 and r[0] >= 0 and r[0] + r[2] <= (top.get('width') or 0) + 0.5 and r[1] >= 0]
    return {'visible': len(inside), 'width': top.get('width'), 'height': top.get('height'), 'rects': rects}


def window_classes(h, surface):
    """The CSS classes of the app window itself (the root of its first visible toplevel)."""
    tops = [t for t in h.state(surface) if t.get('visible')]
    return (tops[0].get('tree') or {}).get('classes') or [] if tops else []


def shell_pss(h):
    """gnome-shell's PSS in MiB (smaps_rollup), the harness's own descendant."""
    for pid in h.hm.descendants(os.getpid()):
        try:
            with open(f'/proc/{pid}/comm') as f:
                if f.read().strip() != 'gnome-shell':
                    continue
            with open(f'/proc/{pid}/smaps_rollup') as f:
                for line in f:
                    if line.startswith('Pss:'):
                        return round(int(line.split()[1]) / 1024, 1)
        except OSError:
            continue
    return None


def settled_pss(h):
    for _ in range(3):
        h.eval(PAINT_JS)
        h.sleep(0.6)
    return shell_pss(h)


def hosts(h, pid):
    """The identities the extension holds for the native shell's pid (frostedBridge _hosts)."""
    return (h.eval(HOSTS_JS) or {}).get(str(pid), [])


def caps(h, pid, kind):
    """capabilities() windows of that pid and kind (taskbar, start)."""
    return [w for w in json.loads(h.eval(CAPS_JS)).get('windows', []) if w.get('pid') == pid and w.get('kind') == kind]


def wait_entries(h, kind, count, label, timeout=6.0):
    try:
        return h.until(lambda: (lambda e: e if len([x for x in e if x['effect']]) == count else None)(entries(h, kind)),
                       label, timeout, 0.1)
    except TimeoutError:
        return entries(h, kind)


def run(h):
    # A. Taskbar (the native shell's window; D-G70).
    pid = h.native_pid
    h.require('A: the native shell runs', bool(pid), pid=pid)
    bar = wait_entries(h, 'taskbar', 1, 'taskbar blur')
    h.check('A: one taskbar blur for the 48 px strip', len(bar) == 1 and bar[0]['effect'] and bar[0]['rect']['h'] == 48,
            entries=bar)
    h.check('A: the extension holds the native pid as Panel', PANEL in hosts(h, pid), hosts=h.eval(HOSTS_JS))
    tb = caps(h, pid, 'taskbar')
    h.check('A: capabilities() list the native taskbar with blur', len(tb) == 1 and tb[0]['blur'], windows=tb)
    h.screenshot('a-taskbar', area=(0, h.hm.HEIGHT - 120, h.hm.WIDTH, 120))

    # B. Settings focused; first without window blur for the memory baseline (E), then with it.
    h.open('settings')
    h.record('E.configureOff', h.eval(CONFIGURE_JS % 'false'))
    wait_entries(h, 'window', 0, 'window blur off')
    without = settled_pss(h)
    h.record('E.configureOn', h.eval(CONFIGURE_JS % 'true'))
    win = wait_entries(h, 'window', 1, 'Settings blur')
    blurred = [e for e in win if e['effect']]
    h.check('B: the focused Settings window has one blur', len(blurred) == 1, entries=win)
    rim = blurred and (blurred[0]['blurRect']['x'], blurred[0]['blurRect']['y'])
    h.check('B: the unblurred rim is 3 px (corner on the 8 px arc)', rim == (3, 3), rim=rim)
    try:
        classes = h.until(lambda: (lambda c: c if 'w11-glass' in c else None)(window_classes(h, 'settings')),
                          'Settings window glass', 6, 0.1)
    except TimeoutError:
        classes = window_classes(h, 'settings')
    # the 90 % itself is the kit's unit test (native/kit/src/theme.rs tests); the window's colour is not read here
    h.check('B: the window wears the kit\'s glass class (w11-glass)', 'w11-glass' in classes, classes=classes)
    h.screenshot('b-settings-focused')
    with_window = settled_pss(h)

    # C. Start open.
    h.open('start')
    st = wait_entries(h, 'start', 1, 'Start blur')
    h.check('C: open Start has one blur', len([e for e in st if e['effect']]) == 1, entries=st)
    win = entries(h, 'window')
    h.check('C: Settings lost its blur with the focus', not any(e['effect'] for e in win), entries=win)
    h.check('C: one window blur in all (Start)',
            len([e for e in status(h)['entries'] if e['effect'] and e['kind'] != 'taskbar']) == 1, status=status(h))
    h.check('C: the extension holds Panel and Start under the native pid', {PANEL, START} <= set(hosts(h, pid)),
            hosts=h.eval(HOSTS_JS))
    sc = caps(h, pid, 'start')
    h.check('C: capabilities() list the open Start with blur', len(sc) == 1 and sc[0]['blur'], windows=sc)
    h.screenshot('c-start-open')
    with_start = settled_pss(h)

    # D. Start closed.
    h.close('start')
    gone = h.until(lambda: not entries(h, 'start') or None, 'Start blur released', 3, 0.05)
    h.check('D: closing Start releases its blur', bool(gone))
    win = wait_entries(h, 'window', 1, 'Settings blur back')
    h.check('D: Settings has its blur again', len([e for e in win if e['effect']]) == 1, entries=win)
    h.sleep(1.0)
    h.check('D: the extension keeps the hidden Start registration of the native pid', START in hosts(h, pid),
            hosts=h.eval(HOSTS_JS))

    # D2. Start again: before D-G60 its registration went with the hidden window.
    h.open('start')
    st = wait_entries(h, 'start', 1, 'Start blur on reopen')
    h.check('D2: reopened Start has its blur again', len([e for e in st if e['effect']]) == 1, entries=st)
    sc = caps(h, pid, 'start')
    h.check('D2: capabilities() list the reopened Start with blur', len(sc) == 1 and sc[0]['blur'], windows=sc)
    h.screenshot('d2-start-reopened')
    h.close('start')
    h.until(lambda: not entries(h, 'start') or None, 'Start blur released again', 3, 0.05)

    # F. A drag: the backdrop follows, it is not rebuilt per step.
    wait_entries(h, 'window', 1, 'Settings blur before the drag')
    drag = h.eval(DRAG_JS)
    h.record('F.drag', drag)
    steps = drag.get('steps', []) if isinstance(drag, dict) else []
    h.check('F: the blur stays during the whole drag', bool(steps) and all(x['blur'] for x in steps), drag=drag)
    h.check('F: no blur entry was rebuilt during the drag', isinstance(drag, dict) and drag.get('rebuilt') == 0, drag=drag)
    h.check('F: the backdrop follows the drawn window', bool(steps) and all(x['follows'] for x in steps), drag=drag)
    try:
        settled = h.until(lambda: (lambda v: v if v['drawn'] and v['ok'] else None)(h.eval(SETTLED_JS)),
                          'window drawn on its frame', 5, 0.1)
    except TimeoutError:
        settled = h.eval(SETTLED_JS)
    h.check('F: after the drag the blur is on the frame again', bool(settled.get('ok')), settled=settled)

    # G. Real pointer: move by the title bar, resize by the right edge.
    real = h.eval(REAL_DRAG_JS)
    h.record('G.real', real)
    ok = isinstance(real, dict) and 'error' not in real
    h.check('G: the title bar drag moves the window', ok and real['moved']['dx'] >= 60 and real['moved']['dy'] >= 30,
            real=real)
    h.check('G: the right edge resizes the window', ok and real['resized']['dw'] >= 40, real=real)
    h.check('G: the drawn window is its buffer (no stretch)', ok and abs(real['actor']['w'] - real['buffer']['w']) <= 1
            and abs(real['actor']['h'] - real['buffer']['h']) <= 1, real=real)
    try:
        buttons = h.until(lambda: (lambda c: c if ok and abs((c['width'] or 0) - real['frame']['w']) <= 2 else None)(
            captions(h, 'settings')), 'window at the new width', 5, 0.2)
    except TimeoutError:
        buttons = captions(h, 'settings')
    h.record('G.captions', buttons)
    h.check('G: the app\'s window has the new width', ok and abs((buttons['width'] or 0) - real['frame']['w']) <= 2,
            caps=buttons, real=real)
    h.check('G: the three caption buttons are inside the window', buttons['visible'] == 3, caps=buttons)
    h.screenshot('g-after-real-drag')

    # E. Memory.
    memory = {'withoutWindowBlurMiB': without, 'settingsFocusedMiB': with_window, 'startOpenMiB': with_start}
    extra = max(v for v in (with_window, with_start) if v is not None) - without if without else None
    memory['extraMiB'] = extra
    h.record('E.memory', memory)
    h.check('E: the window blur costs gnome-shell less than 64 MiB PSS', extra is not None and extra < 64, **memory)

    failed = [c['check'] for c in h.results['checks'] if not c['ok']]
    h.require('every check passed', not failed, failed=failed)
