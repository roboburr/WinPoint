"""Taskbar editing -> Start results, only in the private GNOME harness.

All ordinary editing uses real Clutter pointer/Wayland keyboard input. The dev
channel only observes fields, plus one deliberate same-callback lifecycle race.
"""
import json

BRIDGE = "Main.extensionManager.lookup('win11-gnome@win11shell.local').stateObj._panelBridge"


def walk(node):
    return [node] + [n for child in node.get('children', []) for n in walk(child)] if node else []


def widget(top, name):
    return next((n for n in walk((top or {}).get('tree')) if n.get('name') == name), {})


def snapshot(h):
    tops = [top for top in h.native_state() if top.get('visible')]
    return {'panel': next((t for t in tops if widget(t, 'tb-strip')), None),
            'start': next((t for t in tops if widget(t, 'start-search')), None)}


def searching(top):
    return any('is-searching' in n.get('classes', []) for n in walk((top or {}).get('tree')))


def keys(h, names, chord=False):
    code = "(async()=>{const C=(await import('gi://Clutter')).default;const G=(await import('gi://GLib')).default;" \
           "const wait=ms=>new Promise(r=>G.timeout_add(G.PRIORITY_DEFAULT,ms,()=>{r();return G.SOURCE_REMOVE;}));" \
           "if(!globalThis.__taskbarKeys){globalThis.__taskbarKeys=C.get_default_backend().get_default_seat()" \
           ".create_virtual_device(C.InputDeviceType.KEYBOARD_DEVICE);await wait(100);}" \
           "const d=globalThis.__taskbarKeys,ns=" + json.dumps(names) + ";"
    if chord:
        code += "for(const n of ns){d.notify_keyval(G.get_monotonic_time(),C['KEY_'+n],C.KeyState.PRESSED);await wait(30);}" \
                "for(const n of ns.slice().reverse()){d.notify_keyval(G.get_monotonic_time(),C['KEY_'+n],C.KeyState.RELEASED);await wait(30);}"
    else:
        code += "for(const n of ns){d.notify_keyval(G.get_monotonic_time(),C['KEY_'+n],C.KeyState.PRESSED);await wait(30);" \
                "d.notify_keyval(G.get_monotonic_time(),C['KEY_'+n],C.KeyState.RELEASED);await wait(30);}"
    return h.eval(code + "return true;})()")


def click(h, surface, name):
    answer = h.click(surface, name, real=True)
    h.require('Real pointer: ' + name, answer.get('input') == 'clutter', input=answer)


def pair(h, text, label, focus='panel'):
    last = {}
    def ready():
        state = snapshot(h)
        last['state'] = state
        top = state[focus]
        field = 'tb-search-input' if focus == 'panel' else 'start-search'
        expected_wm = 'io.github.win11shell.Panel' if focus == 'panel' else 'io.github.win11shell.Start'
        if (top and state['start'] and (focus == 'panel' or top.get('active')) and top.get('focus') == field
                and widget(state['panel'], 'tb-search-input').get('text') == text
                and widget(state['start'], 'start-search').get('text') == text
                and searching(state['start'])
                and h.eval('global.display.focus_window?.get_wm_class()') == expected_wm):
            return state
        return None
    try:
        state = h.until(ready, label, 8, 0.05)
    except TimeoutError:
        def compact(top):
            return {**{key: (top or {}).get(key) for key in ('title', 'visible', 'mapped', 'active', 'focus', 'width', 'height')},
                    'widgets': [{key: n.get(key) for key in ('name', 'text', 'position', 'hasFocus', 'rect')}
                                for name in ('tb-search', 'tb-search-input', 'start-search', 'start-search-empty')
                                if (n := widget(top, name))]}
        h.record('Search timeout: ' + label, {'expectedText': text, 'expectedFocus': focus,
                 'panel': compact(last.get('state', {}).get('panel')),
                 'start': compact(last.get('state', {}).get('start')),
                 'bridge': h.bridge('shell.info'),
                 'meta': h.eval("(()=>{const f=global.display.focus_window;const b=" + BRIDGE + ";"
                                "return {focus:f?{id:f.get_id(),pid:f.get_pid(),wm:f.get_wm_class(),type:f.get_window_type()}:null,"
                                "search:b._panelSearch,exclusive:b._panelExclusive,windows:global.get_window_actors().map(a=>"
                                "({wm:a.meta_window.get_wm_class(),pid:a.meta_window.get_pid(),focus:a.meta_window.has_focus(),"
                                "mapped:a.mapped,visible:a.visible}))};})()")})
        raise
    h.check(label + ': isolated search keyboard role', h.eval(f'{BRIDGE}._panelSearch') is True)
    return state


def close(h):
    keys(h, ['Escape'])
    h.until(lambda: snapshot(h)['start'] is None and not h.eval(f'{BRIDGE}._panelSearch'),
            'Escape closes results and releases taskbar search', 8, 0.05)
    h.check('Closing clears taskbar text', widget(snapshot(h)['panel'], 'tb-search-input').get('text') == '')
    h.sleep(0.4)


def put(h, path, value):
    answer = h.backend('settings', 'settings.set', {'path': path, 'value': value})
    h.require('Private setting accepted: ' + path, not (isinstance(answer, dict) and answer.get('error')), answer=answer)


def run(h):
    # Wayland clients bind wl_keyboard only after the private seat gains this
    # device. Initialize before opening/clicking anything; never lose key zero.
    keys(h, [])
    h.open('settings:home')
    h.until(lambda: h.eval('global.display.focus_window?.get_wm_class()') == 'io.github.win11shell.Settings',
            'Private Settings window is initially focused', 10, 0.05)
    previous = h.eval('global.display.focus_window.get_id()')
    click(h, 'panel', 'tb-search-input')
    empty = pair(h, '', 'Taskbar edit opens empty Start results without moving the caret')
    width = widget(empty['panel'], 'tb-search').get('rect', [0, 0, 0, 0])[2]
    h.check('Search pill retains its compact width', 190 <= width <= 260, width=width)
    h.check('Empty results retain categories and prompt', widget(empty['start'], 'start-tab-apps')
            and widget(empty['start'], 'start-search-empty'))
    keys(h, list('zzsearchfocus'))
    typed = pair(h, 'zzsearchfocus', 'Real typing stays in taskbar and mirrors into Start')
    h.check('Taskbar caret follows native editing', widget(typed['panel'], 'tb-search-input').get('position') == 13)
    keys(h, ['BackSpace'])
    pair(h, 'zzsearchfocu', 'Backspace edits the taskbar field')
    keys(h, ['Down'])
    pair(h, 'zzsearchfocu', 'Arrow selection keeps taskbar text focus')
    click(h, 'panel', 'tb-search-input')
    caret = pair(h, 'zzsearchfocu', 'Text click keeps native caret placement')
    h.check('Text click places the caret inside the query instead of reopening it',
            0 < widget(caret['panel'], 'tb-search-input').get('position', 0) < len('zzsearchfocu'))
    click(h, 'panel', 'tb-search-clear')
    pair(h, '', 'Taskbar clear keeps the empty result view open')
    keys(h, list('abcdefghijklmnopqrstuvwxyzzzzzz'))
    long = pair(h, 'abcdefghijklmnopqrstuvwxyzzzzzz', 'Long taskbar query')
    h.check('Long text and clear button do not move taskbar icons',
            widget(long['panel'], 'tb-search').get('rect', [0, 0, 0, 0])[2] == width)
    keys(h, ['Control_L', 'a'], chord=True)
    keys(h, ['x'])
    pair(h, 'x', 'Ctrl+A replacement is native GtkText editing')
    click(h, 'start', 'start-search')
    keys(h, ['Control_L', 'a'], chord=True)
    keys(h, list('zzfromstart'))
    pair(h, 'zzfromstart', 'Explicit Start-field click allows reverse query mirroring', focus='start')
    click(h, 'panel', 'tb-search-input')
    keys(h, ['End', 'y'])
    pair(h, 'zzfromstarty', 'Returning to taskbar keeps the same query')
    h.screenshot('taskbar-input-start-results')
    close(h)
    h.until(lambda: h.eval('global.display.focus_window?.get_id()') == previous,
            'Escape safely restores the previous application', 8, 0.05)

    # A native toplevel remembers its last focus widget even while another app
    # is active. Reactivating the dock for Start must not reopen Search first.
    h.record('Before unrelated panel activation', {
        'panelFocus': (snapshot(h)['panel'] or {}).get('focus'), 'bridge': h.bridge('shell.info')})
    h.eval(f'(()=>{{const p={BRIDGE}._panel;if(p)Main.activateWindow(p,global.display.get_current_time_roundtrip());return !!p;}})()')
    h.sleep(0.3)
    activated = snapshot(h)
    h.record('After unrelated panel activation before any pointer click', {
        'panelFocus': (activated['panel'] or {}).get('focus'), 'startVisible': activated['start'] is not None,
        'bridge': h.bridge('shell.info'), 'metaFocus': h.eval('global.display.focus_window?.get_wm_class()')})
    h.require('Restoring dock focus alone does not open Search', activated['start'] is None
              and not h.eval(f'{BRIDGE}._panelSearch'))
    click(h, 'panel', 'tb-start')
    h.until(lambda: (s := snapshot(h)['start']) and not searching(s) and s.get('focus') == 'start-search',
            'Real Start click after taskbar Search opens Home', 8, 0.05)
    close(h)

    # Clearing remembered focus must preserve deliberate keyboard navigation.
    h.eval(f'(()=>{{const p={BRIDGE}._panel;if(p)Main.activateWindow(p,global.display.get_current_time_roundtrip());return !!p;}})()')
    h.sleep(0.1)
    for _ in range(2):
        if (snapshot(h)['panel'] or {}).get('focus') == 'tb-search-input':
            break
        keys(h, ['Tab'])
    pair(h, '', 'Explicit keyboard Tab into the editable still opens Search')
    close(h)

    click(h, 'panel', 'tb-search-input')
    pair(h, '', 'Reopening keeps typing on taskbar')
    h.eval(f"{BRIDGE}._startAction('start-search-dismiss'); true")
    h.sleep(0.3)
    pair(h, '', 'A delayed old dismiss action cannot close the current valid search')
    keys(h, list('zzoutside'))
    pair(h, 'zzoutside', 'Outside-dismiss fixture')
    frame = h.s.window_of('io.github.win11shell.Settings', h.surface('settings').pid)
    # Real click on the exposed left part of Settings titlebar, outside the Start rectangle.
    h.eval("(async()=>{const C=(await import('gi://Clutter')).default;const G=(await import('gi://GLib')).default;"
           "globalThis.__outsideSearchPointer??=C.get_default_backend().get_default_seat().create_virtual_device(C.InputDeviceType.POINTER_DEVICE);"
           f"const d=globalThis.__outsideSearchPointer;d.notify_absolute_motion(G.get_monotonic_time(),{frame['x']+70},{frame['y']+20});"
           "await new Promise(r=>G.timeout_add(G.PRIORITY_DEFAULT,60,()=>{r();return G.SOURCE_REMOVE;}));"
           "d.notify_button(G.get_monotonic_time(),C.BUTTON_PRIMARY,C.ButtonState.PRESSED);"
           "d.notify_button(G.get_monotonic_time(),C.BUTTON_PRIMARY,C.ButtonState.RELEASED);return true;})()")
    h.until(lambda: snapshot(h)['start'] is None and not h.eval(f'{BRIDGE}._panelSearch'),
            'A real outside click closes results without trapping application input', 8, 0.05)
    h.check('Outside click preserves the chosen application focus',
            h.eval('global.display.focus_window?.get_wm_class()') == 'io.github.win11shell.Settings')

    click(h, 'panel', 'tb-search-input')
    pair(h, '', 'Before activating a real Settings result from the taskbar')
    language = h.settings_file().get('language', 'nl')
    query = 'settings' if language == 'en' else 'instellingen'
    keys(h, list(query))
    pair(h, query, 'Settings result is selected from a real taskbar query')
    keys(h, ['Return'])
    h.until(lambda: snapshot(h)['start'] is None and not h.eval(f'{BRIDGE}._panelSearch')
            and h.eval('global.display.focus_window?.get_wm_class()') == 'io.github.win11shell.Settings',
            'Taskbar Enter opens Settings and closes its result/search role', 10, 0.05)
    h.check('Result launch clears the taskbar text',
            widget(snapshot(h)['panel'], 'tb-search-input').get('text') == '')
    h.sleep(0.3)
    h.check('No late search focus return interferes with the launched Settings result',
            h.eval('global.display.focus_window?.get_wm_class()') == 'io.github.win11shell.Settings'
            and not h.eval(f'{BRIDGE}._panelSearch'))

    click(h, 'panel', 'tb-search-input')
    pair(h, '', 'Before hiding the search entry')
    put(h, 'taskbar.search', 'hidden')
    h.until(lambda: not widget(snapshot(h)['panel'], 'tb-search-input') and snapshot(h)['start'] is None
            and not h.eval(f'{BRIDGE}._panelSearch'), 'Removing the editable releases its keyboard role', 8, 0.05)
    put(h, 'taskbar.search', 'box')
    h.until(lambda: widget(snapshot(h)['panel'], 'tb-search-input'), 'Editable search returns', 8, 0.05)

    # Two real search activations in one GTK callback, before the first paint.
    answer = h.native({'native': 'click', 'name': 'tb-search', 'repeat': 2})
    h.require('Before-paint double activation arranged', answer.get('clicked') == 'tb-search')
    pair(h, '', 'Double activation preserves taskbar focus')
    h.until(lambda: h.eval(f'{BRIDGE}._frostedBridge._startPhase.get({h.native_pid})') == 'open',
            'Double activation preserves the initial glass callback', 8, 0.05)
    close(h)

    click(h, 'panel', 'tb-start')
    h.until(lambda: (s := snapshot(h)['start']) and not searching(s) and s.get('focus') == 'start-search',
            'Normal Start still opens Home with its own field focused', 8, 0.05)
    keys(h, list('zzinline'))
    h.until(lambda: widget(snapshot(h)['start'], 'start-search').get('text') == 'zzinline',
            'Normal Start retains type-to-search', 8, 0.05)
    click(h, 'start', 'start-search-clear')
    h.until(lambda: (s := snapshot(h)['start']) and not searching(s),
            'Clearing normal inline search returns to Home', 8, 0.05)
    close(h)
    h.record('finalNativePid', h.native_pid)
