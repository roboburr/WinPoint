"""Settings parity, batch A (notes/claude/settings-parity.md §7; plan.json "testing", GNOME HARNESS): the pages Muis,
Touchpad, Pen en tablet, Typen, Taalopties, Geavanceerde toetsenbordinstellingen and Sneltoetsen against GNOME's own
keys in the run's private keyfile, with polkit stand-ins on the private system bus for the system keyboard.

Run (from G), once per language, with --strace (Taalopties reaches pkexec):
    tools/heavy.sh --min-mb 2560 --tag a -- python3 -B tests/gnome/harness.py launch run --lang nl --source worktree \\
        --scenario tests/gnome/scenarios/settings-input.py --strace tests/gnome/scenarios/settings-input.out/strace.txt
and the same with --lang en --out tests/gnome/scenarios/settings-input.en.out --strace .../settings-input.en.out/strace.txt.

Every page: opened as Start's search opens it (h.open), a screenshot <page>.<lang>, its title and breadcrumb (the
registry's parent chain, in the run's language) and, in English, no Dutch word. Every switch and check box a page
binds is flipped by a real pointer click (the page scrolled there by real wheel steps first): the keyfile then holds
GNOME's exact value, the old value written from outside moves the control back (gsettings.changed), and a last check
per page lists any st-<page>-* switch or box that was not flipped. Dialog controls must also take real pointer
clicks; one check per run verifies the repaired modal layer.

What else it checks (the checks are language-independent; the screenshots carry the language):
  0. caps and the search: nl "muis" / en "mouse" finds Muis before Toegankelijkheid > Muis (amendment 65); Touchpad
     and Pen en tablet are found exactly while caps has a touchpad or a tablet (the harness reads the host's own udev
     database, so here they are hidden; the positive side is the native caps tests); Sneltoetsen (needs gnome) is found
  1. Muis, the plan's reference case, by real pointer clicks on the controls named st-mouse-<key>: Primaire
     muisknop Rechts -> left-handed true; Snelheid van muisaanwijzer at position 15 -> speed 0.5; Nauwkeurigheid van
     aanwijzer verbeteren off -> accel-profile 'flat'; Schuifrichting "Neerwaartse beweging schuift omhoog" ->
     natural-scroll true; Plakken met de middelste muisknop flips gtk-enable-primary-paste; outside writes follow
  2. Eigenschappen voor Muis follows the page (the swap box ticked); Middelste klik nabootsen, the double-click speed
     and locate-pointer write their keys; Instellingen testen opens and a real click ticks Primaire klik
  3. Touchpad (opened directly; no touchpad here): the notice; the switch on the expander's header off writes
     send-events 'disabled' and leaves the expander open; the mouse box writes 'disabled-on-external-mouse'; every box
     of Tikken; Schuiven langs de rand writes both scrolling keys; Cursorsnelheid; Schuifrichting; Opnieuw instellen
     brings every key back to its default; no sensitivity drop-down (amendment 67)
  4. Pen en tablet: "Geen tablet gevonden" without a tablet
  5. Typen: the sub-page links (Sneltoetsen while caps.gnome); Het aanraaktoetsenbord weergeven Altijd ->
     screen-keyboard-enabled true and back; an outside write moves the drop-down
  6. Taalopties: without sources the notice; Een toetsenbord toevoegen (search, the choice) -> sources [('xkb', 'nl')]
     through input.sources.set; with polkit answering "would ask" (a challenge) CheckAuthorization of
     org.gnome.controlcenter.set-keyboard reaches the stand-in with flags 0 and no interactive authorization, and no
     pkexec runs; with polkit answering yes the run's pkexec stub logs GNOME's helper with the exact argv
     (/usr/libexec/gnome-control-center-set-debian-keyboard nl,us ,intl), also after Omhoog (us,nl intl,)
  7. Geavanceerde toetsenbordinstellingen: Standaardinvoermethode overschrijven moves the chosen source first; Voor
     elk app-venster, Toetsen herhalen and the overview's Windows key flipped; Samenstellingstoets Rechter Ctrl and
     Toets voor alternatieve tekens Menutoets -> xkb-options ['compose:rctrl', 'lv3:menu_switch']; the repeat
     sliders; Windows-toets opent Start off -> settings.json start.windowsKey false; a real click on the breadcrumb's
     Typen goes there
  8. Sneltoetsen: the sections; WinPoint's Alt+Tab named on Schakelen tussen apps ("In gebruik door Taakweergave",
     amendment 13); Venster sluiten by real keys Super+Q -> close ['<Super>q']; Super+L is a conflict and nothing is
     written; a custom shortcut; Alles opnieuw instellen
  9. the modes of every key these pages bind: no Owned key (so no page shows a pointer to WinPoint instead of a control)
"""

import importlib.util  # noqa: F401  (settings-a11y.py loads this file's helpers)
import json
import os
import sys
from collections import Counter
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from lib.standins import start as start_standin  # noqa: E402
from lib.standins.a import SPECS  # noqa: E402

MOUSE = 'org.gnome.desktop.peripherals.mouse'
TOUCHPAD = 'org.gnome.desktop.peripherals.touchpad'
IFACE = 'org.gnome.desktop.interface'
KEYBOARD = 'org.gnome.desktop.peripherals.keyboard'
SOURCES = 'org.gnome.desktop.input-sources'
A11Y_APPS = 'org.gnome.desktop.a11y.applications'
WM = 'org.gnome.desktop.wm.keybindings'
MEDIA = 'org.gnome.settings-daemon.plugins.media-keys'
CUSTOM = 'org.gnome.settings-daemon.plugins.media-keys.custom-keybinding'
CUSTOM0 = '/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/custom0/'
MUTTER = 'org.gnome.mutter'
HELPER = '/usr/libexec/gnome-control-center-set-debian-keyboard'
SET_KEYBOARD = 'org.gnome.controlcenter.set-keyboard'

# Real key presses through a Clutter virtual keyboard in the private shell (%s: a JSON list of Clutter KEY_ names),
# pressed in order and released in reverse: Wayland input to the focused window.
KEYS_JS = (
    "(async()=>{const Clutter=(await import('gi://Clutter')).default;const GLib=(await import('gi://GLib')).default;"
    "const wait=ms=>new Promise(r=>GLib.timeout_add(GLib.PRIORITY_DEFAULT,ms,()=>{r();return GLib.SOURCE_REMOVE;}));"
    "const seat=Clutter.get_default_backend().get_default_seat();"
    "globalThis.__a2Keyboard??=seat.create_virtual_device(Clutter.InputDeviceType.KEYBOARD_DEVICE);"
    "const d=globalThis.__a2Keyboard,us=()=>GLib.get_monotonic_time(),keys=%s;"
    "for(const k of keys){d.notify_keyval(us(),Clutter['KEY_'+k],Clutter.KeyState.PRESSED);await wait(60);}"
    "for(const k of keys.slice().reverse()){d.notify_keyval(us(),Clutter['KEY_'+k],Clutter.KeyState.RELEASED);"
    "await wait(40);}return keys;})()")

# ------------------------------------------------------------------------------------------------------------------
# Helpers, shared with settings-a11y.py (which loads this file)
# ------------------------------------------------------------------------------------------------------------------

# Real wheel steps through the harness's Clutter virtual pointer (tests/gnome/lib/scenario.py POINTER_CLICK_JS's
# device): the pointer moves to (x, y) on the screen, then n discrete steps in a direction (UP or DOWN).
SCROLL_JS = (
    "(async()=>{const Clutter=(await import('gi://Clutter')).default;const GLib=(await import('gi://GLib')).default;"
    "const wait=ms=>new Promise(r=>GLib.timeout_add(GLib.PRIORITY_DEFAULT,ms,()=>{r();return GLib.SOURCE_REMOVE;}));"
    "const seat=Clutter.get_default_backend().get_default_seat();"
    "globalThis.__ge1hPointer??=seat.create_virtual_device(Clutter.InputDeviceType.POINTER_DEVICE);"
    "const d=globalThis.__ge1hPointer,us=()=>GLib.get_monotonic_time();"
    "d.notify_absolute_motion(us(),%d,%d);await wait(80);"
    "for(let i=0;i<%d;i++){d.notify_discrete_scroll(us(),Clutter.ScrollDirection.%s,Clutter.ScrollSource.WHEEL);"
    "await wait(50);}await wait(150);return true;})()")

# The pages' places in the registry (registry.rs parent chains, the top first): the breadcrumb Settings shows.
CHAINS = {
    'mouse': ['bluetooth', 'mouse'], 'touchpad': ['bluetooth', 'touchpad'], 'pen': ['bluetooth', 'pen'],
    'typing': ['time', 'typing'], 'language-options': ['time', 'language', 'language-options'],
    'keyboard-advanced': ['time', 'typing', 'keyboard-advanced'], 'shortcuts': ['time', 'typing', 'shortcuts'],
    'accessibility': ['accessibility'],
    **{p: ['accessibility', p] for p in ('textsize', 'visualeffects', 'contrast', 'mousepointer', 'textcursor',
                                         'magnifier', 'narrator', 'a11y-audio', 'a11y-keyboard', 'a11y-mouse')},
}

# How each click reached its widget in this run ('clutter': real pointer input, 'dev': the dev channel's click)
INPUTS: Counter = Counter()


def until(h, check, label, timeout=10):
    try:
        return h.until(check, label, timeout)
    except TimeoutError:
        return None


def node(h, s, name):
    found = h.find(s, name)
    return found[0] if found else None


def click(h, name=None, n=None, real=True):
    """h.click on Settings, counting the input it took (a widget that is not there is a failed check, never a click
    on whatever comes first)."""
    if name is None and n is None:
        h.check('the widget to click is on the page', False)
        return {}
    done = h.click('settings', name, node=n, real=real)
    INPUTS[done.get('input', '?') if isinstance(done, dict) else '?'] += 1
    return done


def gv(value):
    """A JSON value as the GVariant text GLib's keyfile backend writes for it (bools, strings, numbers, string
    arrays: an empty one is [])."""
    if value is True:
        return 'true'
    if value is False:
        return 'false'
    if isinstance(value, str):
        return "'" + value.replace('\\', '\\\\').replace("'", "\\'") + "'"
    if isinstance(value, list):
        return '[' + ', '.join(gv(v) for v in value) + ']'
    return json.dumps(value)


def wait_key(h, schema, key, want, label, timeout=10, path=None):
    seen = until(h, lambda: h.gsettings_get(schema, key, path) == want or None, label, timeout)
    value = h.gsettings_get(schema, key, path)
    h.check(label, bool(seen) and value == want, want=want, value=value)
    return value == want


def backend_value(h, schema, key, path=None):
    """GSettings' own view of a key through the app's backend (the keyfile keeps a reset key's line)."""
    k = {'schema': schema, 'key': key, **({'path': path} if path else {})}
    answer = h.backend('settings', 'gsettings.get', {'keys': [k]}) or {}
    items = answer.get('items') or [{}]
    return items[0]


def no_dutch(h, en, page):
    if en:
        text = h.page_text('settings')
        h.check(f'{page}: no Dutch words in English', not text.get('dutchWordHits'), hits=text.get('dutchWordHits'))


def crumbs(h):
    """Settings' header as [(page id or None, text)]: a sub-page's breadcrumb (the crumb-<id> buttons, then the
    current page's label) or a top-level page's title."""
    heads = h.find('settings', cls='st-title')
    if not heads:
        return []
    head = heads[0]
    if 'w11-breadcrumb' not in (head.get('classes') or []):
        return [(None, head.get('label'))]
    out = []
    for child in head.get('children') or []:
        name = child.get('name') or ''
        if name.startswith('crumb-'):
            out.append((name[len('crumb-'):], child.get('label')))
        elif 'w11-breadcrumb-current' in (child.get('classes') or []):
            out.append((None, child.get('label')))
    return out


def page_open(h, page, ready=None, shot=True, timeout=15):
    """Open a page as Start's search does, wait for `ready` (a widget name), take <page>.<lang>, and check its title
    and breadcrumb: the registry's parent chain in the run's language, each earlier crumb naming its page."""
    h.open(f'settings:{page}')
    if ready:
        h.check(f'{page}: {ready} is on the page', bool(until(h, lambda: node(h, 'settings', ready), f'{page} ready',
                                                               timeout)))
    h.sleep(0.6)
    chain = CHAINS[page]
    titles = h.session.titles
    want = [titles.get(p, {}).get(h.lang) for p in chain]
    got = crumbs(h)
    h.check(f'{page}: title and breadcrumb "{" > ".join(str(w) for w in want)}"',
            [text for _, text in got] == want and [i for i, _ in got[:-1]] == chain[:-1], got=got, want=want)
    if shot:
        h.screenshot(f'{page}.{h.lang}', surface='settings')


def viewport(h, tops):
    """The page's scrolled area: the largest GtkScrolledWindow right of the navigation pane, as [x, y, w, h]."""
    found = [n for n in h.find('settings', where=lambda n: n.get('type') == 'GtkScrolledWindow' and n.get('rect'),
                               state=tops) if n.get('top', 0) == 0 and n['rect'][0] > 200]
    return max(found, key=lambda n: n['rect'][2] * n['rect'][3])['rect'] if found else None


def reveal(h, name, expander=None):
    """The widget `name`, with its centre inside the page's scrolled area: the page is scrolled there by real wheel
    steps over its left margin (never over a slider). `expander` (its header's name) is opened first. None when the
    widget is not on the page."""
    sign = 1
    last = None
    if expander:
        # a collapsed expander's rows can keep an old rectangle: open it first, whatever the dev state says
        open_expander(h, expander)
    for attempt in range(10):
        tops = h.state('settings')
        found = [n for n in h.find('settings', name, state=tops) if n.get('rect') and n['rect'][3] > 0]
        if not found:
            return None
        n = found[0]
        area = viewport(h, tops)
        if not area:
            return n
        cy = n['rect'][1] + n['rect'][3] / 2
        top, bottom = area[1] + 12, area[1] + area[3] - 12
        if top <= cy <= bottom:
            return n
        if last is not None and ((cy - last) * (1 if want_down else -1)) > 0:
            sign = -sign                         # the wheel went the other way (natural scrolling)
        want_down = cy > bottom
        frame = h._frame(h.surface('settings'), tops[n.get('top', 0)])
        if not frame:
            return n
        steps = max(1, min(8, round(abs(cy - (area[1] + area[3] / 2)) / 60)))
        down = want_down if sign > 0 else not want_down
        h.eval(SCROLL_JS % (frame['x'] + area[0] + 10, frame['y'] + area[1] + area[3] // 2, steps,
                            'DOWN' if down else 'UP'))
        h.sleep(0.3)
        last = cy
    return n


def open_expander(h, header):
    """Open the SettingsExpander whose header button is `header` (a real click on it) unless it is open."""
    for exp in h.find('settings', cls='w11-settings-expander'):
        if any(n.get('name') == header for n in h.hm.tree_nodes([{'tree': exp}])):
            if 'w11-expanded' in (exp.get('classes') or []):
                return True
            n = reveal(h, header)
            if not n:
                return False
            click(h, n=n)
            h.sleep(0.6)
            return True
    return False


def flip(h, name, schema, key, path=None, pair=None, label=None, expander=None, needs=(), dialog=False,
         outside=True, tested=None):
    """Flip a bound switch or check box by a real pointer click: GNOME's exact value lands in the keyfile and the
    control shows it; then the old value, written from outside, moves the control back. `pair` = the GVariant texts
    of a control mapped onto a non-boolean key (on, off). `needs`: (schema, key, GVariant) written from outside first
    (the control is sensitive only then) and restored afterwards. `dialog`: the control is in a kit Dialog, so the
    named control is clicked with the real pointer."""
    label = label or name
    if tested is not None:
        tested.add(name)
    restore = []
    for s, k, v in needs:
        restore.append((s, k, gv(backend_value(h, s, k).get('value'))))
        h.gsettings_set(s, k, v)
    n = reveal(h, name, expander) if not dialog else node(h, 'settings', name)
    if needs and n is not None:
        n = until(h, lambda: (lambda m: m if m and m.get('sensitive') else None)(node(h, 'settings', name)),
                  f'{label} sensitive', 5) or n
    if n is None:
        h.check(f'{label}: the control is on the page', False)
        return None
    if n.get('sensitive') is False:
        h.check(f'{label}: the control can be used', False, node={k: n.get(k) for k in ('name', 'rect', 'active')})
        return None
    before_active = n.get('active')
    before = backend_value(h, schema, key, path).get('value')
    if isinstance(before, bool) and pair is None:
        want = gv(not before)
    elif pair:
        want = pair[1] if before_active else pair[0]
    else:
        want = None
    old = h.gsettings_get(schema, key, path)
    if dialog:
        click(h, name, real=True)
    else:
        click(h, n=n)
    if want is not None:
        ok = wait_key(h, schema, key, want, f'{label}: a click writes {key} {want}', path=path)
    else:
        seen = until(h, lambda: (lambda v: v if v not in (None, old) else None)(h.gsettings_get(schema, key, path)),
                     f'{label} writes', 10)
        ok = h.check(f'{label}: a click writes {key}', bool(seen), value=seen, before=old)
    after = node(h, 'settings', name) or {}
    if before_active is not None:
        h.check(f'{label}: the control shows the new value', after.get('active') == (not before_active),
                active=after.get('active'))
    if outside and ok:
        h.gsettings_set(schema, key, gv(before), path)
        seen = until(h, lambda: (node(h, 'settings', name) or {}).get('active') == before_active or None,
                     f'{label} follows', 10)
        h.check(f'{label}: {key} {gv(before)} written from outside moves the control back (gsettings.changed)',
                bool(seen))
    for s, k, v in restore:
        h.gsettings_set(s, k, v)
    return ok


def slider_click(h, name, fraction, knob=10, expander=None, dialog=False):
    """A real click on a kit slider's trough at `fraction` (0 left, 1 right; the 20 px thumb's halves excluded)."""
    scale = node(h, 'settings', name) if dialog else reveal(h, name, expander)
    if not (scale and scale.get('rect')):
        h.check(f'{name} has a place on the screen', False)
        return False
    x, y, w, hh = scale['rect']
    px = x + knob + round((w - 2 * knob) * fraction)
    click(h, n={**scale, 'rect': [px - 1, y, 2, hh]})
    h.sleep(1.0)
    return True


def slide(h, name, schema, key, lo, hi, step, at, to_gnome, tol, label=None, expander=None, value=None,
          outside=None, path=None, tested=None):
    """A slider of `lo`..`hi` (step `step`) by a real click at position `at`: the key gets to_gnome(at) within
    `tol`. `value`: the value text's widget name, which an outside write (`outside`, GVariant text) must change."""
    label = label or name
    if tested is not None:
        tested.add(name)
    before = h.gsettings_get(schema, key, path)
    if not slider_click(h, name, (at - lo) / (hi - lo), expander=expander):
        return None
    want = to_gnome(at)
    seen = until(h, lambda: (lambda v: v if v not in (None, before) else None)(h.gsettings_get(schema, key, path)),
                 f'{label} writes', 6)
    try:
        got = float(seen)
    except (TypeError, ValueError):
        got = None
    h.check(f'{label}: a click at {at} writes {key} near {want}', got is not None and abs(got - want) <= tol,
            value=seen, want=want, before=before, exact=got == want)
    if value and outside:
        shown = (node(h, 'settings', value) or {}).get('label')
        h.gsettings_set(schema, key, outside, path)
        seen = until(h, lambda: (lambda n: n if n and n.get('label') != shown else None)(node(h, 'settings', value)),
                     f'{label} value follows', 8)
        h.check(f'{label}: {key} {outside} written from outside changes its value text', bool(seen),
                before=shown, after=(seen or {}).get('label'))
    return got


def choose(h, s, dropdown, option, expander=None):
    """A drop-down by real clicks: the button, then the option's label in its popover (never a switch's Aan/Uit
    text: slice 4's first run clicked Overversterking's "Uit" instead of an option Uit)."""
    n = reveal(h, dropdown, expander) if s == 'settings' else node(h, s, dropdown)
    if n is None:
        h.check(f'{dropdown}: the drop-down is on the page', False)
        return False
    click(h, n=n)
    h.sleep(0.6)
    found = h.find(s, label=option, where=lambda m: 'w11-toggle-state' not in (m.get('classes') or []))
    if not found:
        h.check(f'{dropdown}: the option {option!r} is in its popover', False)
        h.key(s, 'Escape')
        return False
    click(h, n=found[-1])
    h.sleep(0.4)
    return True


def shows(h, name, text):
    """Whether the widget `name` (a drop-down) shows `text` somewhere in its subtree."""
    n = node(h, 'settings', name)
    return bool(n) and any(m.get('label') == text for m in h.hm.tree_nodes([{'tree': n}]))


def unflipped(h, page, tested, skip=()):
    """The page's last check: every st-<page>-* switch and check box on it was flipped (or is in `skip`, with why)."""
    prefix = f'st-{page}-'
    found = {n.get('name') for n in h.find('settings', where=lambda n: str(n.get('name') or '').startswith(prefix)
                                           and 'active' in n and n.get('type') in ('GtkSwitch', 'GtkCheckButton'))}
    left = sorted(found - set(tested) - set(skip))
    h.check(f'{page}: every switch and check box on the page was flipped by a click', not left, left=left,
            flipped=sorted(set(tested) & found), skipped=sorted(set(skip) & found))


def modes(h, page, keys, mirrored=None):
    """The modes of the keys a page binds (gsettings.get): none is Owned (an Owned key shows a pointer to its
    WinPoint page instead of a control; batch A binds none), every Mirrored one names its WinPoint setting."""
    mirrored = mirrored or {}
    answer = h.backend('settings', 'gsettings.get', {'keys': [{'schema': s, 'key': k} for s, k in keys]}) or {}
    items = answer.get('items') or []
    got = {(i.get('schema'), i.get('key')): (i.get('mode'), i.get('setting')) for i in items}
    owned = sorted(f'{s} {k}' for (s, k), (m, _) in got.items() if m == 'owned')
    h.check(f'{page}: no key the page binds is Owned (no pointer to a WinPoint page)', not owned and len(got) == len(keys),
            owned=owned, keys=len(keys), items=len(got))
    for (s, k), setting in mirrored.items():
        h.check(f'{page}: {k} is Mirrored with {setting}', got.get((s, k)) == ('mirrored', setting),
                got=got.get((s, k)))


def search(h, q):
    """Settings' own search as a user runs it (the text, then Enter): the result page's ids, in order."""
    h.type_text('settings', 'settings-search', q)
    h.sleep(0.5)
    h.key('settings', 'Return', name='settings-search')
    lead = until(h, lambda: [n for n in h.find('settings', cls='st-lead') if q in str(n.get('label'))] or None,
                 f'the results for {q!r}', 10)
    h.sleep(0.4)
    ids = [n['name'][len('result-'):] for n in h.find('settings', where=lambda n: str(n.get('name') or '')
                                                     .startswith('result-'))]
    h.type_text('settings', 'settings-search', '')
    h.surface('settings').page = 'search'
    h.record(f'search {q!r}', ids)
    if not lead:
        h.check(f'the search for {q!r} shows its results page', False)
    return ids


def real_keys(h, keys):
    """Press `keys` (Clutter KEY_ names) as a real keyboard would."""
    return h.eval(KEYS_JS % json.dumps(keys))


def guard_calls(h, name):
    """The run's stub calls of `name` (logs/guard-calls.log: time, program, its arguments)."""
    try:
        lines = (h.hm.RUN / 'logs' / 'guard-calls.log').read_text().splitlines()
    except OSError:
        return []
    return [line.split('\t', 2)[2] for line in lines if line.count('\t') >= 2 and line.split('\t')[1] == name]


def private_stubs(h):
    """The common harness establishes private permissions; this scenario only checks them."""
    mode = (h.hm.RUN / 'stubs').stat().st_mode & 0o777
    h.check('the tool seam run/stubs is private (0700)', mode == 0o700, mode=oct(mode))


def dialog_click_check(h, close_name, open_name):
    """Require the repaired kit Dialog to pass a real pointer click to its button."""
    target = node(h, 'settings', close_name)
    if not target:
        return
    click(h, n=target)
    h.sleep(0.8)
    closed = node(h, 'settings', open_name) is None
    h.check('a real pointer click on a kit Dialog button registers', closed)
    if not closed:
        h.key('settings', 'Escape')
        h.sleep(0.5)


# ------------------------------------------------------------------------------------------------------------------


def run(h):
    en = h.lang == 'en'
    t = (lambda nl_text, en_text: en_text if en else nl_text)
    INPUTS.clear()
    private_stubs(h)

    # ---- 0. caps and the search
    h.open('settings:mouse')
    h.wait_text('settings', t('Primaire muisknop', 'Primary mouse button'), required=True)
    caps = h.backend('settings', 'caps.get') or {}
    h.record('caps', caps)
    hits = search(h, t('muis', 'mouse'))
    h.check('the search finds Muis first, before Toegankelijkheid > Muis (amendment 65)',
            hits[:1] == ['mouse'] and 'a11y-mouse' in hits and hits.index('a11y-mouse') > 0, hits=hits)
    h.screenshot(f'search-mouse.{h.lang}', surface='settings')
    hits = search(h, 'touchpad')
    h.check('Touchpad is in the search exactly while caps has a touchpad', ('touchpad' in hits) == (caps.get('touchpad') is True),
            hits=hits, touchpad=caps.get('touchpad'))
    hits = search(h, t('pen', 'pen'))
    h.check('Pen en tablet is in the search exactly while caps has a tablet', ('pen' in hits) == (caps.get('tablet') is True),
            hits=hits, tablet=caps.get('tablet'))
    hits = search(h, t('sneltoetsen', 'keyboard shortcuts'))
    h.check('Sneltoetsen (needs gnome) is in the search while caps.gnome', ('shortcuts' in hits) == (caps.get('gnome') is True),
            hits=hits, gnome=caps.get('gnome'))

    # ---- 1. Muis
    page_open(h, 'mouse', 'st-mouse-left-handed')
    for name in ('st-mouse-left-handed', 'st-mouse-mouse-speed', 'st-mouse-mouse-accel-profile', 'st-mouse-natural-scroll',
                 'st-mouse-gtk-enable-primary-paste', 'st-mouse-more', 'st-mouse-test'):
        h.check(f'Muis has {name}', node(h, 'settings', name) is not None)
    h.check('the pointing-stick group shows exactly while caps has one',
            (node(h, 'settings', 'st-mouse-pointingstick-speed') is not None) == (caps.get('pointingstick') is True))
    scroll_fixed = node(h, 'settings', 'st-mouse-scroll-inactive') or {}
    h.check('Inactieve vensters schuiven is on and fixed (GNOME always scrolls the window under the pointer)',
            scroll_fixed.get('active') is True and scroll_fixed.get('sensitive') is False)
    for key, default in (('left-handed', 'false'), ('speed', '0.0'), ('accel-profile', "'default'"), ('natural-scroll', 'false')):
        h.record(f'mouse {key} before', h.gsettings_get(MOUSE, key) or default)
    tested = set()

    choose(h, 'settings', 'st-mouse-left-handed', t('Rechts', 'Right'))
    wait_key(h, MOUSE, 'left-handed', 'true', 'Primaire muisknop Rechts writes left-handed true')
    tested.add('st-mouse-left-handed')

    scale = reveal(h, 'st-mouse-mouse-speed')
    if scale and scale.get('rect'):
        x, y, w, hh = scale['rect']
        # the trough runs between the thumb's half widths (20 px thumb): position 15 of 1..20
        knob = 10
        px = x + knob + round((w - 2 * knob) * (15 - 1) / (20 - 1))
        click(h, n={**scale, 'rect': [px - 1, y, 2, hh]})
        h.sleep(1.0)
        value = h.gsettings_get(MOUSE, 'speed')
        h.record('mouse speed after a click at position 15', value)
        try:
            speed = float(value)
        except (TypeError, ValueError):
            speed = None
        h.check('Snelheid van muisaanwijzer at position 15 writes speed 0.5 ((n - 10) / 10)',
                speed is not None and abs(speed - 0.5) < 0.11, value=value,
                exact=speed is not None and abs(speed - 0.5) < 1e-9)
    else:
        h.check('the speed slider has a place on the screen', False)

    click(h, 'st-mouse-mouse-accel-profile')
    wait_key(h, MOUSE, 'accel-profile', "'flat'", 'Nauwkeurigheid van aanwijzer verbeteren off writes accel-profile flat')
    tested.add('st-mouse-mouse-accel-profile')
    choose(h, 'settings', 'st-mouse-natural-scroll', t('Neerwaartse beweging schuift omhoog', 'Down motion scrolls up'))
    wait_key(h, MOUSE, 'natural-scroll', 'true', 'Schuifrichting "down motion scrolls up" writes natural-scroll true')
    flip(h, 'st-mouse-gtk-enable-primary-paste', IFACE, 'gtk-enable-primary-paste', label='Plakken met de middelste muisknop',
         tested=tested)

    # a change from elsewhere: the precision switch and the drop-downs follow
    h.gsettings_set(MOUSE, 'accel-profile', "'default'")
    seen = until(h, lambda: (node(h, 'settings', 'st-mouse-mouse-accel-profile') or {}).get('active') is True or None,
                 'the switch follows an outside write')
    h.check('an outside write of accel-profile moves the switch (gsettings.changed)', bool(seen))
    h.gsettings_set(MOUSE, 'accel-profile', "'flat'")
    until(h, lambda: (node(h, 'settings', 'st-mouse-mouse-accel-profile') or {}).get('active') is False or None, 'back', 5)
    h.gsettings_set(MOUSE, 'left-handed', 'false')
    seen = until(h, lambda: shows(h, 'st-mouse-left-handed', t('Links', 'Left')) or None, 'the drop-down follows', 8)
    h.check('left-handed false written from outside shows Primaire muisknop Links', bool(seen))
    h.gsettings_set(MOUSE, 'left-handed', 'true')
    until(h, lambda: shows(h, 'st-mouse-left-handed', t('Rechts', 'Right')) or None, 'Rechts again', 8)
    h.screenshot(f'mouse-set.{h.lang}', surface='settings')
    unflipped(h, 'mouse', tested, skip=('st-mouse-scroll-inactive',))
    no_dutch(h, en, 'Muis')

    # ---- 2. Eigenschappen voor Muis, Instellingen testen
    click(h, n=reveal(h, 'st-mouse-more'))
    seen = until(h, lambda: node(h, 'settings', 'st-mouse-props-left-handed'), 'Eigenschappen voor Muis')
    h.check('Aanvullende muisinstellingen opens Eigenschappen voor Muis', bool(seen))
    if seen:
        h.check('its swap box follows the page (left-handed true)', seen.get('active') is True, node=seen)
        h.screenshot(f'mouse-properties.{h.lang}', surface='settings')
        dtested = set()
        flip(h, 'st-mouse-props-middle-click-emulation', MOUSE, 'middle-click-emulation', dialog=True,
             label='Middelste klik nabootsen', tested=dtested)
        # the double-click speed: kit sliders take a press of their own (a gesture), which the dialog layer lets through
        before = h.gsettings_get(MOUSE, 'double-click')
        if slider_click(h, 'st-mouse-props-double-click', (4 - 1) / (10 - 1), dialog=True):
            value = until(h, lambda: (lambda v: v if v not in (None, before) else None)(h.gsettings_get(MOUSE, 'double-click')),
                          'double-click', 5)
            h.record('double-click after a real click at step 4', value)
            h.check('Dubbelkliksnelheid at step 4 writes double-click 700 ms (1000 - 100 (n - 1))',
                    value is not None and abs(int(value) - 700) <= 100, value=value, before=before)
        # kit::controls::Pivot names its tabs tab-<id>
        tab = node(h, 'settings', 'tab-options')
        h.check('Eigenschappen voor Muis has the tab Opties voor aanwijzer', bool(tab))
        if tab:
            click(h, 'tab-options', real=True)
            h.sleep(0.5)
            selected = node(h, 'settings', 'tab-options') or {}
            h.check('the tab Opties voor aanwijzer is selected', 'w11-selected' in (selected.get('classes') or []))
        flip(h, 'st-mouse-props-locate-pointer', IFACE, 'locate-pointer', dialog=True,
             label='Locatie van aanwijzer weergeven', tested=dtested)
        flip(h, 'st-mouse-props-accel-profile', MOUSE, 'accel-profile', pair=("'default'", "'flat'"), dialog=True,
             label='Nauwkeurigheid van aanwijzer verbeteren (Eigenschappen)', tested=dtested)
        h.screenshot(f'mouse-properties-options.{h.lang}', surface='settings')
        dialog_click_check(h, 'w11-dialog-close', 'st-mouse-props-tabs')
        if node(h, 'settings', 'st-mouse-props-tabs'):
            h.key('settings', 'Escape')
            h.sleep(0.5)
    click(h, n=reveal(h, 'st-mouse-test'))
    seen = until(h, lambda: node(h, 'settings', 'st-input-test-area'), 'Instellingen testen')
    h.check('Instellingen testen opens the test dialog', bool(seen))
    if seen:
        click(h, 'st-input-test-area')
        h.sleep(0.4)
        primary = node(h, 'settings', 'st-input-test-primary')
        h.check('a click in the test area ticks Primaire klik', bool(primary) and 'st-done' in (primary.get('classes') or []))
        h.screenshot(f'mouse-test.{h.lang}', surface='settings')
        h.key('settings', 'Escape')
        h.sleep(0.5)
    modes(h, 'Muis', [(MOUSE, k) for k in ('left-handed', 'speed', 'accel-profile', 'natural-scroll', 'middle-click-emulation',
                                           'double-click')] + [(IFACE, 'locate-pointer'), (IFACE, 'gtk-enable-primary-paste')])

    # ---- 3. Touchpad
    page_open(h, 'touchpad', 'st-touchpad-expander')
    h.check('without a touchpad the page says so', bool(h.find('settings', label=t(
        'Er is geen touchpad gevonden. Deze instellingen gelden zodra er een is aangesloten.',
        'No touchpad was found. These settings apply once one is connected.'))) == (caps.get('touchpad') is not True))
    tested = set()
    if node(h, 'settings', 'st-touchpad-send-events'):
        tested.add('st-touchpad-send-events')
        click(h, n=reveal(h, 'st-touchpad-send-events'))
        wait_key(h, TOUCHPAD, 'send-events', "'disabled'", 'the Touchpad switch off writes send-events disabled')
        opened = [n for n in h.find('settings', cls='w11-expanded') if 'st-touchpad-expander' in json.dumps(n)]
        h.check('a click on the header switch leaves the expander open (w11-expanded)', bool(opened))
        mouse_box = node(h, 'settings', 'st-touchpad-send-events-mouse') or {}
        h.check('... and makes the mouse box insensitive', mouse_box.get('sensitive') is False)
        h.gsettings_set(TOUCHPAD, 'send-events', "'enabled'")
        seen = until(h, lambda: (node(h, 'settings', 'st-touchpad-send-events') or {}).get('active') is True or None,
                     'the switch follows', 8)
        h.check("send-events 'enabled' written from outside turns the switch on", bool(seen))
        flip(h, 'st-touchpad-send-events-mouse', TOUCHPAD, 'send-events', pair=("'enabled'", "'disabled-on-external-mouse'"),
             label='Touchpad ingeschakeld laten wanneer er een muis is aangesloten', tested=tested)
    if slide(h, 'st-touchpad-speed', TOUCHPAD, 'speed', 1, 10, 1, 8, lambda n: (n - 5) / 5, 0.2,
             label='Cursorsnelheid', tested=tested) is None:
        h.check('Touchpad has Cursorsnelheid', False)
    open_expander(h, 'st-touchpad-taps')
    h.check('Tikken has no sensitivity drop-down (amendment 67)',
            not h.find('settings', label=t('Gevoeligheid van touchpad', 'Touchpad sensitivity')))
    for name, key, pair, label in (
            ('st-touchpad-disable-while-typing', 'disable-while-typing', None, 'Touchpad uitschakelen tijdens het typen'),
            ('st-touchpad-tap-to-click', 'tap-to-click', None, 'Met één vinger tikken'),
            ('st-touchpad-click-method', 'click-method', ("'areas'", "'fingers'"), 'Rechterbenedenhoek')):
        flip(h, name, TOUCHPAD, key, pair=pair, label=label, expander='st-touchpad-taps', tested=tested)
    flip(h, 'st-touchpad-tap-and-drag', TOUCHPAD, 'tap-and-drag', label='Twee keer tikken en slepen',
         expander='st-touchpad-taps', needs=[(TOUCHPAD, 'tap-to-click', 'true')], tested=tested)
    h.gsettings_set(TOUCHPAD, 'tap-to-click', 'false')
    seen = until(h, lambda: (lambda n: n if n and n.get('sensitive') is False else None)(node(h, 'settings', 'st-touchpad-tap-and-drag')),
                 'tap-and-drag insensitive', 6)
    two = node(h, 'settings', 'st-touchpad-tap-two-fingers') or {}
    h.check('without tap-to-click, tapping and dragging is insensitive and the fixed two-finger box unticked',
            bool(seen) and two.get('active') is False and two.get('sensitive') is False)
    h.gsettings_set(TOUCHPAD, 'tap-to-click', 'true')
    seen = until(h, lambda: (node(h, 'settings', 'st-touchpad-tap-two-fingers') or {}).get('active') is True or None,
                 'the two-finger box follows tapping', 6)
    h.check('with tap-to-click the fixed two-finger box is ticked', bool(seen))
    h.screenshot(f'touchpad-taps.{h.lang}', surface='settings')
    open_expander(h, 'st-touchpad-scroll')
    edge = reveal(h, 'st-touchpad-edge-scrolling-enabled', 'st-touchpad-scroll')
    if edge:
        click(h, n=edge)
        wait_key(h, TOUCHPAD, 'edge-scrolling-enabled', 'true', 'Schuiven langs de rand writes edge-scrolling-enabled true')
        wait_key(h, TOUCHPAD, 'two-finger-scrolling-enabled', 'false', '... and two-finger-scrolling-enabled false in the same write')
        tested.add('st-touchpad-edge-scrolling-enabled')
    else:
        h.check('Schuiven en zoomen has Schuiven langs de rand', False)
    two_name = 'st-touchpad-two-finger-scrolling-enabled-two'
    two = reveal(h, two_name, 'st-touchpad-scroll')
    if two:
        click(h, n=two)
        wait_key(h, TOUCHPAD, 'two-finger-scrolling-enabled', 'true', 'Two-finger scrolling writes its key true')
        wait_key(h, TOUCHPAD, 'edge-scrolling-enabled', 'false', '... and clears edge scrolling in the same write')
        tested.add(two_name)
    else:
        h.check('Schuiven en zoomen has the two-finger scrolling choice', False)
    natural = backend_value(h, TOUCHPAD, 'natural-scroll').get('value')
    option = t('Neerwaartse beweging schuift omlaag', 'Down motion scrolls down') if natural else \
        t('Neerwaartse beweging schuift omhoog', 'Down motion scrolls up')
    if choose(h, 'settings', 'st-touchpad-natural-scroll', option, 'st-touchpad-scroll'):
        wait_key(h, TOUCHPAD, 'natural-scroll', gv(not natural), f'Schuifrichting {option!r} writes natural-scroll {gv(not natural)}')
    h.screenshot(f'touchpad-open.{h.lang}', surface='settings')
    open_expander(h, 'st-touchpad-three-finger')
    tap3 = reveal(h, 'st-touchpad-three-finger-tap', 'st-touchpad-three-finger')
    h.check('Bewegingen met drie vingers says what a three-finger tap does (tap-button-map)', bool(tap3 and tap3.get('label')),
            label=(tap3 or {}).get('label'))
    h.screenshot(f'touchpad-gestures.{h.lang}', surface='settings')
    unflipped(h, 'touchpad', tested, skip=('st-touchpad-tap-two-fingers', 'st-touchpad-pinch'))
    reset = reveal(h, 'st-touchpad-reset')
    if reset:
        click(h, n=reset)
        h.sleep(1.5)
        h.screenshot(f'touchpad-reset.{h.lang}', surface='settings')
        # GSettings' own view (the app's backend): GLib's keyfile backend leaves a reset key's line in the file
        keys = ['send-events', 'edge-scrolling-enabled', 'two-finger-scrolling-enabled', 'speed', 'tap-to-click',
                'disable-while-typing', 'click-method', 'natural-scroll']
        answer = h.backend('settings', 'gsettings.get', {'keys': [{'schema': TOUCHPAD, 'key': k} for k in keys]}) or {}
        items = {i.get('key'): i for i in answer.get('items') or []}
        for key in keys:
            item = items.get(key) or {}
            h.check(f'Opnieuw instellen resets {key}', item.get('isDefault') is True, value=item.get('value'),
                    keyfile=h.gsettings_get(TOUCHPAD, key))
        sw = node(h, 'settings', 'st-touchpad-send-events') or {}
        h.check('after Opnieuw instellen the Touchpad switch is on again', sw.get('active') is True)
    else:
        h.check('Touchpad has Opnieuw instellen', False)
    modes(h, 'Touchpad', [(TOUCHPAD, k) for k in ('send-events', 'speed', 'disable-while-typing', 'tap-to-click', 'tap-and-drag',
                                                  'tap-button-map', 'click-method', 'two-finger-scrolling-enabled',
                                                  'edge-scrolling-enabled', 'natural-scroll')])
    no_dutch(h, en, 'Touchpad')

    # ---- 4. Pen en tablet
    page_open(h, 'pen', shot=False)
    seen = until(h, lambda: node(h, 'settings', 'st-pen-none') or node(h, 'settings', 'st-pen-tablet'), 'the Pen page', 15)
    h.check('Pen en tablet without a tablet says so', bool(seen) and seen.get('name') == 'st-pen-none', node=seen)
    h.screenshot(f'pen.{h.lang}', surface='settings')
    no_dutch(h, en, 'Pen')
    run_keyboard(h, en, t, caps)
    h.record('click inputs', dict(INPUTS))
    h.close('settings')


def run_keyboard(h, en, t, caps):
    # ---- 5. Typen
    page_open(h, 'typing', 'st-typing-screen-keyboard-enabled')
    h.check('Typen links to Geavanceerde toetsenbordinstellingen', node(h, 'settings', 'link-keyboard-advanced') is not None)
    h.check('Typen links to Sneltoetsen while caps.gnome', (node(h, 'settings', 'link-shortcuts') is not None) == (caps.get('gnome') is True))
    if node(h, 'settings', 'st-typing-screen-keyboard-enabled'):
        choose(h, 'settings', 'st-typing-screen-keyboard-enabled', t('Altijd', 'Always'))
        wait_key(h, A11Y_APPS, 'screen-keyboard-enabled', 'true', 'Aanraaktoetsenbord Altijd writes screen-keyboard-enabled true')
        # back, so the screen keyboard does not cover the rest of the run
        choose(h, 'settings', 'st-typing-screen-keyboard-enabled', t('Als er geen toetsenbord is aangesloten',
                                                                     'When no keyboard attached'))
        wait_key(h, A11Y_APPS, 'screen-keyboard-enabled', 'false', '... and its other choice writes false')
        h.gsettings_set(A11Y_APPS, 'screen-keyboard-enabled', 'true')
        seen = until(h, lambda: shows(h, 'st-typing-screen-keyboard-enabled', t('Altijd', 'Always')) or None, 'Altijd', 8)
        h.check('screen-keyboard-enabled true written from outside shows Altijd', bool(seen))
        h.gsettings_set(A11Y_APPS, 'screen-keyboard-enabled', 'false')
    else:
        h.check('Typen has the touch keyboard drop-down', False)
    no_dutch(h, en, 'Typen')

    # ---- 6. Taalopties, with polkit on the private system bus
    page_open(h, 'language-options', 'st-language-options-add')
    h.check('without sources it says the system layout is used',
            bool(h.find('settings', label=t('Geen toetsenborden ingesteld: de indeling van het systeem wordt gebruikt.',
                                            'No keyboards set: the system layout is used.'))))
    challenge = start_standin(h, SPECS['polkit-challenge'], 'polkit-challenge')
    add = reveal(h, 'st-language-options-add')
    if add:
        click(h, n=add)
        seen = until(h, lambda: node(h, 'settings', 'st-language-options-search'), 'Een toetsenbord toevoegen')
        h.check('Een toetsenbord toevoegen opens its dialog with the search', bool(seen))
        if seen:
            h.type_text('settings', 'st-language-options-search', t('Nederlands', 'Dutch'))
            h.sleep(0.8)
            h.screenshot(f'language-options-add.{h.lang}', surface='settings')
            choice = until(h, lambda: node(h, 'settings', 'st-language-options-choice-xkb-nl'), 'the Dutch layout')
            h.check('the search finds the Dutch layout', bool(choice))
            if choice:
                # A row of a kit Dialog must take a real pointer click.
                click(h, 'st-language-options-choice-xkb-nl', real=True)
                wait_key(h, SOURCES, 'sources', "[('xkb', 'nl')]", 'choosing Dutch writes sources [(xkb, nl)] (input.sources.set)')
            else:
                h.key('settings', 'Escape')
    calls = until(h, lambda: [c for c in challenge.calls() if c.get('method') == 'CheckAuthorization'] or None
                  if challenge else None, 'CheckAuthorization', 10) or []
    h.record('polkit (challenge) calls', challenge.calls() if challenge else None)
    h.check(f'the keyboard write asks polkit for {SET_KEYBOARD} once', len(calls) == 1 and SET_KEYBOARD in calls[0].get('args', ''),
            calls=calls)
    h.check('... for this process (unix-process), with flags 0 and no interactive authorization (GNOME Settings never '
            'prompts for the system keyboard)', bool(calls) and "'unix-process'" in calls[0]['args']
            and 'uint32 0' in calls[0]['args'] and calls[0].get('interactive') is False, call=calls[:1])
    h.sleep(1.0)
    h.check('polkit would ask: no pkexec runs (the session setting only)', not guard_calls(h, 'pkexec'),
            pkexec=guard_calls(h, 'pkexec'))
    if challenge:
        challenge.stop()
    row = until(h, lambda: node(h, 'settings', 'st-language-options-0-more'), 'the source on the page')
    h.check('the page shows the new keyboard', bool(row))
    polkit = start_standin(h, SPECS['polkit'], 'polkit')
    h.backend('settings', 'input.sources.set', {'sources': [['xkb', 'nl'], ['xkb', 'us+intl']]})
    lines = until(h, lambda: guard_calls(h, 'pkexec') or None, 'pkexec', 10) or []
    h.check(f'polkit says yes: pkexec runs GNOME\'s helper with the sources\' layouts and variants ({HELPER} nl,us ,intl)',
            lines[:1] == [f'{HELPER} nl,us ,intl'], pkexec=lines)
    second = until(h, lambda: node(h, 'settings', 'st-language-options-1-more'), 'the second source')
    if second:
        click(h, n=reveal(h, 'st-language-options-1-more'))
        h.sleep(0.6)
        up = node(h, 'settings', 'st-language-options-up')
        if up:
            click(h, n=up)
            h.sleep(0.6)
            if h.gsettings_get(SOURCES, 'sources') != "[('xkb', 'us+intl'), ('xkb', 'nl')]" and node(h, 'settings', 'st-language-options-up'):
                click(h, 'st-language-options-up', real=True)
        h.check('the ⋯ menu has Omhoog', bool(up))
        wait_key(h, SOURCES, 'sources', "[('xkb', 'us+intl'), ('xkb', 'nl')]", 'Omhoog moves the second keyboard first')
        lines = until(h, lambda: (lambda ls: ls if len(ls) >= 2 else None)(guard_calls(h, 'pkexec')), 'pkexec again', 10) or []
        h.check('... and the system keyboard follows the new order (us,nl intl,)', lines[1:2] == [f'{HELPER} us,nl intl,'],
                pkexec=lines)
    if polkit:
        calls = [c for c in polkit.calls() if c.get('method') == 'CheckAuthorization']
        h.record('polkit calls', polkit.calls())
        h.check('every keyboard write asked polkit without interactive authorization',
                len(calls) >= 2 and all(c.get('interactive') is False and 'uint32 0' in c.get('args', '') for c in calls),
                calls=len(calls))
        polkit.stop()
    h.screenshot(f'language-options-two.{h.lang}', surface='settings')
    modes(h, 'Taalopties', [(SOURCES, 'sources')])
    no_dutch(h, en, 'Taalopties')

    # ---- 7. Geavanceerde toetsenbordinstellingen
    page_open(h, 'keyboard-advanced', 'st-keyboard-advanced-default')
    for name in ('st-keyboard-advanced-per-window', 'st-keyboard-advanced-lv3', 'st-keyboard-advanced-compose',
                 'st-keyboard-advanced-repeat', 'st-keyboard-advanced-delay', 'st-keyboard-advanced-repeat-interval',
                 'st-keyboard-advanced-windows-key', 'st-keyboard-advanced-overlay-key', 'st-keyboard-advanced-hotkeys'):
        h.check(f'the advanced page has {name}', node(h, 'settings', name) is not None)
    tested = set()
    if choose(h, 'settings', 'st-keyboard-advanced-default', 'Dutch'):
        wait_key(h, SOURCES, 'sources', "[('xkb', 'nl'), ('xkb', 'us+intl')]",
                 'Standaardinvoermethode overschrijven moves Dutch first')
    until(h, lambda: node(h, 'settings', 'st-keyboard-advanced-per-window'), 'the page again', 10)
    flip(h, 'st-keyboard-advanced-per-window', SOURCES, 'per-window', label='Voor elk app-venster een andere invoermethode',
         tested=tested)
    choose(h, 'settings', 'st-keyboard-advanced-compose', t('Rechter Ctrl', 'Right Ctrl'))
    wait_key(h, SOURCES, 'xkb-options', "['compose:rctrl']", 'Samenstellingstoets Rechter Ctrl writes compose:rctrl')
    choose(h, 'settings', 'st-keyboard-advanced-lv3', t('Menutoets', 'Menu key'))
    wait_key(h, SOURCES, 'xkb-options', "['compose:rctrl', 'lv3:menu_switch']",
             'Toets voor alternatieve tekens Menutoets adds lv3:menu_switch and keeps compose:rctrl')
    flip(h, 'st-keyboard-advanced-repeat', KEYBOARD, 'repeat', label='Toetsen herhalen', tested=tested)
    slide(h, 'st-keyboard-advanced-delay', KEYBOARD, 'delay', 1, 20, 1, 15, lambda n: 2100 - 100 * n, 100,
          label='Vertraging voor herhalen')
    slide(h, 'st-keyboard-advanced-repeat-interval', KEYBOARD, 'repeat-interval', 1, 20, 1, 5, lambda n: 115 - 5 * n, 5,
          label='Herhalingssnelheid')
    flip(h, 'st-keyboard-advanced-overlay-key', MUTTER, 'overlay-key', pair=("'Super'", "''"),
         label='Windows-toets opent het activiteitenoverzicht', tested=tested)
    tested.add('st-keyboard-advanced-windows-key')
    click(h, n=reveal(h, 'st-keyboard-advanced-windows-key'))
    h.sleep(1.0)
    settings = h.backend('settings', 'settings.get', {}) or {}
    h.check('Windows-toets opent Start off writes start.windowsKey false (amendment 58)',
            (settings.get('start') or {}).get('windowsKey') is False, start=settings.get('start'))
    click(h, n=reveal(h, 'st-keyboard-advanced-windows-key'))
    h.sleep(0.6)
    settings = h.backend('settings', 'settings.get', {}) or {}
    h.check('... and on again true', (settings.get('start') or {}).get('windowsKey') is True, start=settings.get('start'))
    h.screenshot(f'keyboard-advanced-set.{h.lang}', surface='settings')
    unflipped(h, 'keyboard-advanced', tested)
    modes(h, 'Geavanceerde toetsenbordinstellingen', [(SOURCES, 'sources'), (SOURCES, 'xkb-options'), (SOURCES, 'per-window'),
                                                     (KEYBOARD, 'repeat'), (KEYBOARD, 'delay'), (KEYBOARD, 'repeat-interval'),
                                                     (MUTTER, 'overlay-key')])
    no_dutch(h, en, 'Geavanceerde toetsenbordinstellingen')
    crumb = node(h, 'settings', 'crumb-typing')
    if crumb:
        click(h, n=crumb)
        seen = until(h, lambda: [x for _, x in crumbs(h)][-1:] == [h.session.titles.get('typing', {}).get(h.lang)] or None, 'Typen', 8)
        h.check('a real click on the breadcrumb\'s Typen shows Typen', bool(seen), crumbs=crumbs(h))
        h.surface('settings').page = 'typing'
    else:
        h.check('the breadcrumb has Typen', False)

    # ---- 8. Sneltoetsen
    page_open(h, 'shortcuts', 'st-shortcuts-section-windows')
    for section in ('winpoint', 'accessibility', 'launchers', 'navigation', 'screenshots', 'media', 'system', 'typing',
                    'windows', 'custom'):
        h.check(f'Sneltoetsen has the section {section}', node(h, 'settings', f'st-shortcuts-section-{section}') is not None)
    click(h, n=reveal(h, 'st-shortcuts-section-navigation'))
    h.sleep(0.6)
    text = json.dumps(h.page_text('settings'), ensure_ascii=False)
    h.check('Schakelen tussen apps names the Alt+Tab WinPoint holds (amendment 13)',
            t('In gebruik door Taakweergave', 'In use by Task View') in text)
    click(h, n=reveal(h, 'st-shortcuts-section-windows'))
    h.sleep(0.6)
    close = reveal(h, 'st-shortcuts-close')
    if close:
        click(h, n=close)
        h.sleep(1.5)
        h.screenshot(f'shortcuts-capture.{h.lang}', surface='settings')
        h.record('keys Super+Q', real_keys(h, ['Super_L', 'q']))
        wait_key(h, WM, 'close', "['<Super>q']", 'Venster sluiten by real keys Super+Q writes close [<Super>q]')
        # a combination another shortcut has: as a real key it would reach that shortcut first (the capture
        # is now intercepted by the shell's modal input.capture), so verify conflict handling through the backend
        answer = h.backend('settings', 'input.shortcuts.set', {'schema': WM, 'key': 'close', 'accels': ['<Super>l']}) or {}
        h.check('Super+L, the lock screen\'s, is a conflict and nothing is written',
                (answer.get('conflict') or {}).get('key') == 'screensaver' and h.gsettings_get(WM, 'close') == "['<Super>q']",
                answer=answer, close=h.gsettings_get(WM, 'close'))
        # GNOME Settings' rows follow their keys: written from outside, the row shows the new keys (Alt F4)
        keys_of = (lambda: [m.get('label') for m in h.hm.tree_nodes([{'tree': node(h, 'settings', 'st-shortcuts-close') or {}}])
                            if m.get('type') == 'GtkLabel'])
        # the page is built again 300 ms after a write: first the row with Super+Q
        shown = until(h, lambda: (lambda now: now if now and 'F4' not in now else None)(keys_of()), 'Windows Q', 8)
        h.gsettings_set(WM, 'close', "['<Alt>F4']")
        seen = until(h, lambda: (lambda now: now if now and now != shown and 'F4' in now else None)(keys_of()),
                     'the row follows', 8)
        h.check('close [<Alt>F4] written from outside shows on Venster sluiten (gsettings.changed)', bool(shown) and bool(seen),
                before=shown, after=seen)
    else:
        h.check('Vensters has Venster sluiten', False)
    # a custom shortcut
    until(h, lambda: node(h, 'settings', 'st-shortcuts-section-custom'), 'the page again', 10)
    click(h, n=reveal(h, 'st-shortcuts-section-custom'))
    h.sleep(0.6)
    add = reveal(h, 'st-shortcuts-custom-add')
    if add:
        click(h, n=add)
        seen = until(h, lambda: node(h, 'settings', 'st-shortcuts-custom-name'), 'the custom shortcut dialog')
        if seen:
            h.type_text('settings', 'st-shortcuts-custom-name', 'Terminal')
            h.type_text('settings', 'st-shortcuts-custom-command', 'kgx')
            h.screenshot(f'shortcuts-custom.{h.lang}', surface='settings')
            # The dialog's Toevoegen must take a real pointer click.
            click(h, 'w11-dialog-primary', real=True)
            wait_key(h, MEDIA, 'custom-keybindings', f"['{CUSTOM0}']", 'Toevoegen lists the custom shortcut')
            item = backend_value(h, CUSTOM, 'command', CUSTOM0)
            h.check('... with its command at custom0', item.get('value') == 'kgx', item=item)
        else:
            h.check('Sneltoets toevoegen opens its dialog', False)
    else:
        h.check('Aangepaste sneltoetsen has Sneltoets toevoegen', False)
    # Alles opnieuw instellen
    until(h, lambda: node(h, 'settings', 'st-shortcuts-reset-all'), 'the page again', 10)
    reset = reveal(h, 'st-shortcuts-reset-all')
    click(h, n=reset) if reset else click(h, 'st-shortcuts-reset-all', real=True)
    h.sleep(0.8)
    confirm = node(h, 'settings', 'w11-dialog-primary')
    h.check('Alles opnieuw instellen asks first',
            bool(confirm) and bool(h.find('settings', label=t('Alle sneltoetsen opnieuw instellen?', 'Reset all shortcuts?'))))
    h.screenshot(f'shortcuts-reset.{h.lang}', surface='settings')
    if confirm:
        click(h, 'w11-dialog-primary', real=True)
        h.sleep(1.5)
        item = backend_value(h, WM, 'close')
        h.check('Alles opnieuw instellen resets Venster sluiten', item.get('isDefault') is True, item=item)
        custom = backend_value(h, MEDIA, 'custom-keybindings')
        h.check('... and keeps the custom shortcut', CUSTOM0 in (custom.get('value') or []), item=custom)
    no_dutch(h, en, 'Sneltoetsen')
