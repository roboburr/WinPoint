"""Settings parity, batch D (notes/claude/settings-parity.md §7, plan "testing"): Systeem's pages in the private GNOME
of tests/gnome/harness.py. Run twice, nl and en, under strace (Energie's Aan/uit can reach systemctl):

    tools/heavy.sh --min-mb 2560 --tag d -- python3 -B tests/gnome/harness.py launch run --lang nl \\
        --source worktree --scenario tests/gnome/scenarios/settings-system.py \\
        --strace tests/gnome/scenarios/settings-system.out/strace.txt
    ... --lang en --out tests/gnome/scenarios/settings-system.en.out --strace .../settings-system.en.out/strace.txt

Every page: opened by its id (data pages by a real click on their parent), the window maximised, a screenshot
(<page>.<lang>, long pages also <page>-more), the title and the breadcrumb chain (registry.rs: crumb-<id> buttons and
the current label; a data page's last crumb is its item), no Dutch word of these pages in en, every bound control
(st-<page>-<key>) flipped with real pointer clicks (the content scrolled with the virtual pointer's wheel first)
and GNOME's exact value read back from the private keyfile, an outside write the control follows
(gsettings.changed), and the hardware rows hidden without their stand-in and shown with it.

  display      the GNOME path (mutter's DisplayConfig is the harness's own virtual monitor): Helderheid shows GNOME's
               "only built-in screens" row (no backlight), no Privacyscherm, no Draaivergrendeling, no automatic
               brightness; Nachtlampje says it isn't available (mutter's NightLightSupported is false on a virtual
               CRTC: GNOME Settings shows the same); Meerdere beeldschermen with Dit mijn hoofdbeeldscherm maken (on
               and insensitive for the only display) and Detecteren ("Geen ander beeldscherm gevonden")
  keep         amendment 64 through the real Schaal drop-down: 125 % applies persistently, exactly one keep question
               (GNOME Shell's until step I routes display.* in panelBridge.js, then Settings'), and without an answer
               mutter's 20 s bring 100 % back, the drop-down following
  nightlight   reached by Nachtlampje's chevron: GNOME's "Night Light Unavailable"
  advanced     Geavanceerd beeldscherm: no VRR and no Aanpassen voor tv on the virtual monitor, one refresh rate;
               Schaal voor apps herstellen (Ubuntu's xwayland-native-scaling is on) writes xwayland-scaling-factor
               GNOME's way (off = 1, on = 0) and follows an outside write
  audio        amendment 62: a private PipeWire (pipewire, wireplumber without hardware monitors, pipewire-pulse)
               with two null sinks and a null source
  sound        both device lists, output pick, Volume, mute, the input volume, system.audio.* on the same client;
               Meer geluidsinstellingen (Opstartgeluid afspelen writes startup-sound; the alert theme)
  device       Eigenschappen (a data page: Systeem > Geluid > <device>): Instellen als standaard, Volume, Rechterkanaal
               (GNOME's balance to the left), Volume boven 100% toestaan (allow-volume-above-100-percent, outside write)
  devices      Alle geluidsapparaten: both directions; a click opens the input's Eigenschappen
  mixer        Volumemixer with a pw-play stream: the output drop-down, master volume, Systeemgeluiden, the app's
               volume and its Uitvoerapparaat drop-down (the stream moves)
  power-bare   Energie without UPower, power-profiles-daemon or logind: no Batterij group (Netstroom instead),
               Energiestand "Niet beschikbaar", no Energiebesparing, automatic sleep insensitive with GNOME's reason;
               Scherm uitschakelen na (idle-delay, outside write), Scherm dimmen (idle-dim), the power button
               (power-button-action, outside write), Slaapstand runs `systemctl suspend` (the run's stub;
               logs/guard-calls.log) and Opnieuw opstarten asks first (Escape: nothing runs)
  kit          evidence for the handoff note: a real click on the switch in Meldingen' expander header (known 'kit')
  hardware     stand-ins on the private system bus (tests/gnome/lib/standins/d.py): UPower with a battery,
               power-profiles-daemon, iio-sensor-proxy; Settings started again for fresh caps
  power        with them: the Batterij group, Energiestand (ActiveProfile through Properties.Set as GNOME), Altijd
               energiebesparing gebruiken, power-saver-profile-on-low-battery, Batterijpercentage (and the mirror to
               settings.json taskbar.batteryPercent; the tray's text is step I's), Batterijgezondheid behouden
               (UPower EnableChargeThreshold(true) with interactive authorization), Batterijniveaus' 7 dagen
               (GetHistory over 604800 s)
  display-hw   with the sensor stand-in: Draaivergrendeling writes orientation-lock and follows; automatic
               brightness still needs a backlight
  storage      Opslaginzicht writes both purge keys and follows; its chevron opens Opslaginzicht
  storagesense Tijdelijke bestanden opschonen, the master switch, the Prullenbak drop-down (14 dagen, Nooit, an
               outside 7 days as its own option), Opslaginzicht nu uitvoeren, Prullenbak nu leegmaken with its
               question (an item in the run's own Prullenbak is gone)
  multitasking Vensters uitlijnen, both Bureaubladen rows (taskbar.allDesktops; Alt+Tab = taskView.allDesktops and
               both GNOME switchers), Vast aantal bureaubladen with Aantal bureaubladen (num-workspaces), desktops on
               all displays, Actieve hoek; outside writes followed
  screentime   wellbeing.history on gnome-shell's own record, the chart or "Screen Time Unavailable", recording off,
               both break reminders, the movement schedule, Geluiden (both play-sound keys), Limiet voor schermtijd,
               Dagelijkse limiet (+1 uur), Grijswaarden; outside writes followed
  owned        no key of these pages is WinPoint-owned (allow/system.rs has no Owned row: no pointer instead of a
               switch); show-battery-percentage is Mirrored to taskbar.batteryPercent
  mirror       plan "testing": Niet storen in Settings changes show-banners and the shell's Snelle instellingen tile,
               the tile changes show-banners, an outside show-banners reaches settings.json and the Settings switch
"""
import json
import os
import re
import time
from pathlib import Path

COLOR = 'org.gnome.settings-daemon.plugins.color'
POWER = 'org.gnome.settings-daemon.plugins.power'
SESSION = 'org.gnome.desktop.session'
INTERFACE = 'org.gnome.desktop.interface'
PRIVACY = 'org.gnome.desktop.privacy'
MUTTER = 'org.gnome.mutter'
WAYLAND = 'org.gnome.mutter.wayland'
WM = 'org.gnome.desktop.wm.preferences'
TOUCHSCREEN = 'org.gnome.settings-daemon.peripherals.touchscreen'
LIMITS = 'org.gnome.desktop.screen-time-limits'
BREAKS = 'org.gnome.desktop.break-reminders'
MOVEMENT = 'org.gnome.desktop.break-reminders.movement'
EYESIGHT = 'org.gnome.desktop.break-reminders.eyesight'
SOUND = 'org.gnome.desktop.sound'
UBUNTU = 'org.gnome.shell.ubuntu'
NOTIFICATIONS = 'org.gnome.desktop.notifications'

# Dutch words of these pages that must never show in en
DUTCH = ('Beeldscherm', 'Helderheid', 'Nachtlampje', 'Schaal', 'indeling', 'Afdrukstand', 'Meerdere', 'plannen',
         'Sterkte', 'Uren instellen', 'Inschakelen', 'vernieuwingsfrequentie', 'Beeldscherminformatie',
         'Uitvoer', 'Invoer', 'Geluid', 'Kies ', 'Geavanceerd', 'geluidsapparaten', 'Volumemixer', 'koppelen',
         'Eigenschappen', 'Linkerkanaal', 'Rechterkanaal', 'Systeemgeluiden', 'Uitvoerapparaat', 'Invoerapparaat',
         'Toestaan', 'standaard', 'Energiestand', 'slaapstand', 'Batterij', 'aan/uit-knop', 'Opslaginzicht', 'Prullenbak',
         'Opslagbeheer', 'tijdelijke', 'Bureaubladen', 'Vensters uitlijnen', 'Actieve hoek', 'Schermtijd',
         'Pauzeherinneringen', 'Herinneringen', 'Limiet', 'Niet beschikbaar', 'Netstroom', 'Draaivergrendeling',
         'Grijswaarden', 'Dagelijkse', 'Energiebesparing', 'Detecteren', 'hoofdbeeldscherm')
# the private PipeWire's null devices (pw-cli create-node, object.linger)
NODES = [
    ('w11-speakers', 'WinPoint Test Speakers', 'Audio/Sink', 1000),
    ('w11-headphones', 'WinPoint Test Headphones', 'Audio/Sink', 900),
    ('w11-mic', 'WinPoint Test Microphone', 'Audio/Source', 1000),
]
# wireplumber 0.5: no hardware monitor at all (the sandbox's /dev has no sound, camera or Bluetooth nodes either)
WIREPLUMBER_CONF = """wireplumber.profiles = {
  main = {
    monitor.alsa = disabled
    monitor.alsa-midi = disabled
    monitor.bluez = disabled
    monitor.bluez-midi = disabled
    monitor.v4l2 = disabled
    monitor.libcamera = disabled
  }
}
"""

DIALOGS_JS = ("(()=>{const g=Main.layoutManager.modalDialogGroup;return g?g.get_children().filter(c=>c.visible)"
              ".map(c=>c.constructor.name):[];})()")
# Review D-1: two DisplayBridges one after the other, as the extension's disable and enable at a screen lock build them:
# each holds GNOME Shell's keep question once and lets it go (display.release); their ids must differ, or Settings
# takes the second change for the first and asks nothing.
BRIDGE_IDS_JS = (
    "(async()=>{{const e=Main.extensionManager.lookup('{uuid}');if(!e)return null;"
    "const m=await import('file://'+e.path+'/displayBridge.js');const ids=[];"
    "for(let i=0;i<2;i++){{const b=new m.DisplayBridge(()=>{{}});b.enable();const r=b.call('display.expectConfirm',{{}});"
    "ids.push(r&&r.id!==undefined?r.id:(r&&r.error?r.error.code:null));if(r&&r.id!==undefined)b.call('display.release',{{id:r.id}});"
    "b.disable();}}return ids;}})()")
# Maximise the app's normal window (Windows' Settings is usually maximised; more of each page per screenshot). Mutter 50
# may take MaximizeFlags or none: both are tried. Answers the frame size.
MAXIMIZE_JS = (
    "(async()=>{{const Meta=(await import('gi://Meta')).default;"
    "const w=global.get_window_actors().map(a=>a.meta_window).find(w=>w.get_pid()==={pid}"
    "&&w.get_window_type()===Meta.WindowType.NORMAL);if(!w)return null;"
    "if(!(w.maximized_horizontally&&w.maximized_vertically)){{try{{w.maximize(Meta.MaximizeFlags.BOTH);}}"
    "catch(e){{try{{w.maximize();}}catch(e2){{return String(e2);}}}}}}"
    "const r=w.get_frame_rect();return [r.width,r.height];}})()")
# The virtual pointer's wheel over the content's right margin (never over a slider, which would take the wheel):
# real Wayland scroll events, as a user scrolls to a row.
SCROLL_JS = (
    "(async()=>{{const Clutter=(await import('gi://Clutter')).default;const GLib=(await import('gi://GLib')).default;"
    "const wait=ms=>new Promise(r=>GLib.timeout_add(GLib.PRIORITY_DEFAULT,ms,()=>{{r();return GLib.SOURCE_REMOVE;}}));"
    "const seat=Clutter.get_default_backend().get_default_seat();"
    "globalThis.__ge1hPointer??=seat.create_virtual_device(Clutter.InputDeviceType.POINTER_DEVICE);"
    "const d=globalThis.__ge1hPointer,us=()=>GLib.get_monotonic_time();"
    "d.notify_absolute_motion(us(),{x},{y});await wait(60);"
    "for(let i=0;i<{n};i++){{d.notify_discrete_scroll(us(),Clutter.ScrollDirection.{dir},Clutter.ScrollSource.WHEEL);"
    "await wait(35);}}await wait(200);return true;}})()")


# ------------------------------------------------------------------------------------------------------------ helpers

def tr(h, nl, en):
    return en if h.lang == 'en' else nl


def no_dutch(h, page):
    if h.lang != 'en':
        return
    text = h.page_text('settings')
    blob = json.dumps(text, ensure_ascii=False)
    hits = [w for w in DUTCH if w in blob]
    h.check(f'{page}: no Dutch in en', not hits, hits=hits)


def node_of(h, name=None, label=None):
    nodes = h.find('settings', name, label=label)
    return nodes[0] if nodes else None


def switch_active(h, name):
    node = node_of(h, name)
    return bool(node) and bool(node.get('active'))


def labels_of(h, node):
    return [str(k.get('label') or '') for k in h.hm.tree_nodes([{'tree': node}])]


def shows(h, name, text):
    """Whether the widget `name` (a drop-down) shows a label starting with `text`."""
    node = node_of(h, name)
    return bool(node) and any(label.startswith(text) for label in labels_of(h, node))


def wrote(h, label, test, timeout=10, **detail):
    """Wait until `test()` holds, then the check (details evaluated after the wait)."""
    try:
        h.until(test, label, timeout)
        ok = True
    except TimeoutError:
        ok = False
    return h.check(label, ok, **{k: (v() if callable(v) else v) for k, v in detail.items()})


def number(text):
    try:
        return float(str(text).split()[-1])
    except (TypeError, ValueError, IndexError):
        return None


def settings_json(h):
    try:
        return json.loads((h.hm.RUN / 'config' / 'win11-gnome' / 'settings.json').read_text() or '{}')
    except (OSError, ValueError):
        return {}


def guard_lines(h):
    try:
        return (h.hm.RUN / 'logs' / 'guard-calls.log').read_text().splitlines()
    except OSError:
        return []


REGISTRY = {}


def registry(h):
    """registry.rs: (parent by page id, titles by page id)."""
    if not REGISTRY:
        text = (h.hm.SRC / 'native' / 'apps' / 'settings' / 'src' / 'registry.rs').read_text()
        REGISTRY['parents'] = {m.group(1): m.group(3) for m in
                               re.finditer(r'page!\("([a-z-]+)",\s*(None|Some\("([a-z-]+)"\))', text)}
        REGISTRY['titles'] = h.hm.settings_titles()
    return REGISTRY['parents'], REGISTRY['titles']


def head(h):
    """The header's texts (the st-title breadcrumb or label, separators left out) and its crumb-<id> buttons."""
    heads = h.find('settings', cls='st-title')
    if not heads:
        return [], []
    nodes = h.hm.tree_nodes([{'tree': heads[0]}])
    labels = []
    for n in nodes:
        text = str(n.get('label') or '')
        if any(ch.isalpha() for ch in text) and (not labels or labels[-1] != text):
            labels.append(text)
    crumbs = [n.get('name') for n in nodes if str(n.get('name') or '').startswith('crumb-')]
    return labels, crumbs


def header(h, page, last=None):
    """Title and breadcrumb: Systeem > ... > the page (registry.rs), a data page's item as the last crumb."""
    parents, titles = registry(h)
    chain, p = [], page
    while p:
        chain.insert(0, p)
        p = parents.get(p)
    want = [titles.get(c, {}).get(h.lang) for c in chain]
    if last:
        want[-1] = last
    crumbs = [f'crumb-{c}' for c in chain[:-1]]
    got = (None, None)

    def match():
        nonlocal got
        got = head(h)
        return got[0] == want and got[1] == crumbs
    try:
        h.until(match, f'{page} header', 8, 0.3)
        ok = True
    except TimeoutError:
        ok = False
    h.check(f'{page}: title and breadcrumb {" > ".join(str(w) for w in want)}', ok, want=want, got=got[0],
            crumbs=got[1])


MAXIMISED = set()


def maximize(h):
    sf = h.surface('settings')
    if sf.pid in MAXIMISED:
        return
    try:
        size = h.eval(MAXIMIZE_JS.format(pid=sf.pid))
    except Exception as error:   # noqa: BLE001
        size = str(error)
    h.record(f'maximize.{sf.pid}', size)
    MAXIMISED.add(sf.pid)
    h.sleep(0.8)


def page(h, pid, wait=None, last=None):
    """Open a page by its id (Start's way), maximised, wait for a text of it, check the header."""
    h.open(f'settings:{pid}')
    maximize(h)
    if wait:
        h.wait_text('settings', wait, required=True)
    header(h, pid, last)


def went(h, pid, last=None):
    """After a click that navigated to `pid`: the header, and the harness's idea of the page."""
    h.surface('settings').page = pid
    header(h, pid, last)


def scroll(h, steps, direction):
    sf = h.surface('settings')
    tops = h.state('settings')
    frame = h._frame(sf, tops[0]) if tops else None
    if not frame:
        return False
    try:
        return h.eval(SCROLL_JS.format(x=frame['x'] + frame['w'] - 10, y=frame['y'] + int(frame['h'] * 0.6),
                                       n=steps, dir=direction))
    except Exception as error:   # noqa: BLE001
        h.record('scroll.error', str(error))
        return False


def reveal(h, name=None, label=None):
    """Scroll (the wheel) until the widget's centre is in the window below the fixed header; its node."""
    sf = h.surface('settings')
    node = None
    for _ in range(16):
        tops = h.state('settings')
        nodes = [n for n in h.find('settings', name, label=label, state=tops) if n.get('rect')]
        if not nodes:
            return node
        node = nodes[0]
        frame = h._frame(sf, tops[node.get('top', 0)])
        if not frame:
            return node
        x, y, w, hh = node['rect']
        cy = y + hh / 2
        low, high = 150, frame['h'] - 40
        if low <= cy <= high:
            return node
        steps = max(1, min(10, int(abs(cy - (low + high) / 2) / 70)))
        if not scroll(h, steps, 'DOWN' if cy > high else 'UP'):
            return node
    return node


def press(h, name=None, label=None):
    """A real pointer click on a widget (scrolled into view first); the click's record."""
    node = reveal(h, name, label)
    if node is None:
        raise LookupError(f'settings: no visible widget {name or label!r}')
    result = h.click('settings', name, node=node)
    if result.get('input') == 'dev':
        h.record(f'press.dev.{name or label}', result)
    return result


def expand(h, name):
    press(h, name)
    h.sleep(0.5)


def choose(h, name, label):
    """A drop-down as a user works it: a pointer click opens its list (w11-list-item buttons), a click on the option
    whose label starts with `label`."""
    def option():
        return next((n for n in h.find('settings', cls='w11-list-item')
                     if any(text.startswith(label) for text in labels_of(h, n))), None)
    press(h, name)
    try:
        h.until(option, f'{name} open', 3)
    except TimeoutError:
        h.click('settings', name, real=False)
        h.until(option, f'{name} open (dev click)', 5)
    h.click('settings', node=option())
    h.sleep(0.3)


def spin_up(h, name):
    """A real click on a number box's + button."""
    node = reveal(h, name)
    if node is None:
        raise LookupError(f'{name}: not shown')
    up = next((n for n in h.hm.tree_nodes([{'tree': node}]) if 'up' in (n.get('classes') or []) and n.get('rect')), None)
    if up is None:
        raise LookupError(f'{name}: no + button')
    up['top'] = node.get('top', 0)
    return h.click('settings', node=up)


def flip(h, pg, name, schema, key, label, path=None):
    """A real click on a switch bound to a boolean key: the key takes the other value. A key the keyfile does not hold
    yet gets the switch's own state first (its effective default may be an override the keyfile cannot show)."""
    before = h.gsettings_get(schema, key, path)
    if before not in ('true', 'false'):
        before = 'true' if switch_active(h, name) else 'false'
        h.gsettings_set(schema, key, before, path)
        h.sleep(0.6)
    want = 'false' if before == 'true' else 'true'
    try:
        press(h, name)
    except LookupError as error:
        return h.check(f'{pg}: {label}', False, error=str(error))
    return wrote(h, f'{pg}: {label}', lambda: h.gsettings_get(schema, key, path) == want, before=before,
                 after=lambda: h.gsettings_get(schema, key, path))


def follows(h, pg, name, schema, key, value, active, label=None):
    """An outside write of the key; the switch follows (gsettings.changed)."""
    h.gsettings_set(schema, key, value)
    return wrote(h, f'{pg}: {label or name} follows an outside {key}={value}', lambda: switch_active(h, name) == active)


def shot(h, pid, more=False):
    h.screenshot(f'{pid}.{h.lang}')
    if more:
        scroll(h, 10, 'DOWN')
        h.screenshot(f'{pid}-more.{h.lang}')
        scroll(h, 20, 'UP')


def standins():
    """tests/gnome/lib/standins and its d.py by path (the harness runs with python -I): (package, d)."""
    import importlib
    import importlib.util
    import sys
    name = 'w11_settings_standins'
    if name not in sys.modules:
        base = Path(__file__).resolve().parents[1] / 'lib' / 'standins'
        spec = importlib.util.spec_from_file_location(name, base / '__init__.py', submodule_search_locations=[str(base)])
        module = importlib.util.module_from_spec(spec)
        sys.modules[name] = module
        spec.loader.exec_module(module)
    return sys.modules[name], importlib.import_module(f'{name}.d')


# -------------------------------------------------------------------------------------------- slice 1: display pages

def display_page(h):
    page(h, 'display', tr(h, 'Helderheid en kleur', 'Brightness & colour'))
    state = h.backend('settings', 'display.state')
    h.record('display.state', state)
    h.require('display.state reads Mutter DisplayConfig',
              isinstance(state, dict) and state.get('backend') == 'mutter' and state.get('monitors'))
    h.wait_text('settings', tr(h, 'Schaal en indeling', 'Scale & layout'))
    shot(h, 'display', more=True)
    for name in ('st-display-scale', 'st-display-resolution', 'st-display-orientation', 'st-display-multiple'):
        h.check(f'display: {name}', bool(h.find('settings', name)))
    h.check('display: no placeholder', not h.find('settings', 'st-placeholder'))
    h.check('display: Geavanceerd beeldscherm under Gerelateerde instellingen', bool(h.find('settings', 'link-display-advanced')))
    h.check('display: Kleurprofiel is a row of Helderheid en kleur (amendment 19)', bool(h.find('settings', 'link-colorprofile')))
    no_dutch(h, 'display')
    # the rows of hardware this virtual monitor does not have
    bright = h.backend('settings', 'system.brightness.get') or {}
    h.record('system.brightness.get', bright)
    h.check('display: Helderheid without a backlight: the "only built-in screens" row, no slider',
            bright.get('available') is not True and not h.find('settings', 'st-display-brightness')
            and bool(h.find('settings', label=tr(h, 'Alleen beschikbaar voor ingebouwde schermen, zoals van een laptop',
                                                 'Only available for built-in screens, such as a laptop’s'))),
            brightness=bright)
    h.check('display: no Privacyscherm without a privacy-screen panel', not h.find('settings', 'st-display-privacy-screen'))
    h.check('display: no Draaivergrendeling without an accelerometer (caps)', not h.find('settings', 'st-display-orientation-lock'))
    h.check('display: no automatic brightness without a light sensor', not h.find('settings', 'st-display-ambient-enabled'))
    night = h.backend('settings', 'display.nightLight.status')
    h.record('display.nightLight.status', night)
    sw = 'st-display-night-light-enabled'
    if night.get('supported') is False:
        h.check('display: Nachtlampje says it is not available (mutter: NightLightSupported false), no switch',
                not h.find('settings', sw) and bool(h.find('settings', label=tr(
                    h, 'Nachtlampje is op deze computer niet beschikbaar.', 'Night light isn’t available on this computer.'))))
    else:
        flip(h, 'display', sw, COLOR, 'night-light-enabled', 'the Nachtlampje switch writes night-light-enabled')
        follows(h, 'display', sw, COLOR, 'night-light-enabled', 'false', False)
    # Meerdere beeldschermen closes Schaal en indeling (amendment 19)
    expand(h, 'st-display-multiple')
    main = node_of(h, 'st-display-primary')
    h.check('display: Meerdere beeldschermen holds Dit mijn hoofdbeeldscherm maken (on, insensitive: the only display) '
            'and Detecteren', bool(main) and main.get('active') is True and main.get('sensitive') is False
            and bool(h.find('settings', 'st-display-detect')), main=main and {k: main.get(k) for k in ('active', 'sensitive')})
    press(h, 'st-display-detect')
    h.wait_text('settings', tr(h, 'Geen ander beeldscherm gevonden', 'Didn’t detect another display'), timeout=8)
    h.sleep(0.5)
    # review D-5: Detecteren reloads the page's state and builds Schaal en indeling again; the expander stays open
    h.check('display: Meerdere beeldschermen stays open after Detecteren (the reload builds it again)',
            bool(h.find('settings', cls='w11-expanded', where=lambda n: any(
                k.get('name') == 'st-display-multiple' for k in h.hm.tree_nodes([{'tree': n}])))))
    h.screenshot(f'display-multiple.{h.lang}')
    return state


def bridge_ids(h):
    """Review D-1: keep-question ids stay unique when the extension builds a new DisplayBridge (screen lock)."""
    ids = h.eval(BRIDGE_IDS_JS.format(uuid=h.hm.UUID))
    h.record('displayBridge.ids', ids)
    h.check('display: a new DisplayBridge (after a screen lock) never gives a keep-question id again',
            isinstance(ids, list) and len(ids) == 2 and all(isinstance(i, int) for i in ids) and ids[1] > ids[0],
            ids=ids)


def keep_question(h, state):
    """Amendment 64 through the real Schaal drop-down: one keep question; without an answer the previous scale."""
    logical = state.get('logical') or []
    monitors = {m['connector']: m for m in state.get('monitors') or []}
    if not logical:
        h.check('keep: a layout to change', False, known='harness')
        return
    first = logical[0]
    before = first['scale']
    mon = monitors.get(first['monitors'][0]) or {}
    mode = next((m for m in mon.get('modes', []) if m['id'] == mon.get('current')), None) or {}
    target = next((s for s in (1.25, 1.5) if abs(s - before) > 1e-3
                   and any(abs(s - x) < 1e-3 for x in mode.get('scales') or [])), None)
    if target is None:
        h.check('keep: the virtual monitor offers 125 % or 150 %', False, known='harness', scales=mode.get('scales'))
        return
    h.open('settings:display')
    try:
        choose(h, 'st-display-scale', f'{round(target * 100)}%')
    except Exception as error:   # noqa: BLE001
        h.check(f'keep: the Schaal drop-down offers {round(target * 100)}%', False, error=str(error))
        return
    ours_label = tr(h, 'Deze beeldscherminstellingen behouden?', 'Keep these display settings?')

    def asked():
        return (h.eval(DIALOGS_JS) or []) or h.find('settings', label=ours_label)
    try:
        h.until(asked, 'a keep question', 10, 0.3)
    except TimeoutError:
        pass
    shell = h.eval(DIALOGS_JS) or []
    ours = h.find('settings', label=ours_label)
    now = h.backend('settings', 'display.state') or {}
    applied = (now.get('logical') or [{}])[0].get('scale')
    h.screenshot(f'display-keep.{h.lang}')
    h.record('keep.dialogs', {'shell': shell, 'settings': len(ours), 'applied': applied})
    h.check(f'keep: the Schaal drop-down applies {round(target * 100)}% persistently (display.apply)',
            applied is not None and abs(applied - target) < 1e-3, applied=applied)
    h.check('keep: Settings asks, GNOME Shell does not', bool(ours) and not shell, settings=len(ours), shell=shell)
    # nobody answers: mutter restores the previous layout after its 20 s; the drop-down follows (display.changed)
    wrote(h, 'keep: without an answer the previous scale comes back',
          lambda: abs(((h.backend('settings', 'display.state') or {}).get('logical') or [{}])[0].get('scale', 0) - before) < 1e-3,
          timeout=40)
    wrote(h, f'keep: the Schaal drop-down shows {round(before * 100)}% again',
          lambda: shows(h, 'st-display-scale', f'{round(before * 100)}%'), timeout=10)


def nightlight_page(h):
    """Nachtlampje through its chevron on Beeldscherm."""
    h.open('settings:display')
    h.wait_text('settings', tr(h, 'Helderheid en kleur', 'Brightness & colour'))
    try:
        press(h, 'link-nightlight')
    except LookupError as error:
        h.check('nightlight: the chevron on Beeldscherm', False, error=str(error))
        h.open('settings:nightlight')
    went(h, 'nightlight')
    status = h.backend('settings', 'display.nightLight.status')
    if status.get('supported') is False:
        h.wait_text('settings', tr(h, 'Nachtlampje is niet beschikbaar', 'Night light isn’t available'))
        h.screenshot(f'nightlight.{h.lang}')
        no_dutch(h, 'nightlight')
        h.check('nightlight: Night Light Unavailable (mutter NightLightSupported false: GNOME Settings shows the same)',
                bool(h.find('settings', 'st-nightlight-unavailable')))
        return
    h.wait_text('settings', tr(h, 'Nachtlampje plannen', 'Schedule night light'), required=True)
    h.screenshot(f'nightlight.{h.lang}')
    no_dutch(h, 'nightlight')
    h.check('nightlight: the status row', bool(h.find('settings', 'st-nightlight-status')))
    h.gsettings_set(COLOR, 'night-light-enabled', 'false')
    h.sleep(0.5)
    flip(h, 'nightlight', 'st-nightlight-night-light-enabled', COLOR, 'night-light-enabled',
         'Nachtlampje plannen writes night-light-enabled')
    try:
        press(h, label=tr(h, 'Uren instellen', 'Set hours'))
        wrote(h, 'nightlight: Uren instellen writes night-light-schedule-automatic false',
              lambda: h.gsettings_get(COLOR, 'night-light-schedule-automatic') == 'false')
        h.check('nightlight: the time pickers show', bool(h.find('settings', 'st-nightlight-night-light-schedule-from'))
                and bool(h.find('settings', 'st-nightlight-night-light-schedule-to')))
    except LookupError:
        h.check('nightlight: the Uren instellen radio', False)
    h.gsettings_set(COLOR, 'night-light-temperature', '4700')
    h.sleep(0.5)
    press(h, 'st-nightlight-night-light-temperature')
    wrote(h, 'nightlight: Sterkte writes night-light-temperature in GNOME\'s range',
          lambda: number(h.gsettings_get(COLOR, 'night-light-temperature')) not in (None, 4700.0))


def advanced_page(h):
    page(h, 'display-advanced', tr(h, 'Beeldscherminformatie', 'Display information'))
    shot(h, 'display-advanced', more=True)
    no_dutch(h, 'display-advanced')
    for name in ('st-display-advanced-select', 'st-display-advanced-info', 'st-display-advanced-refresh'):
        h.check(f'display-advanced: {name}', bool(h.find('settings', name)))
    h.check('display-advanced: no Dynamische vernieuwingsfrequentie (the virtual monitor has no variable mode)',
            not h.find('settings', 'st-display-advanced-vrr'))
    h.check('display-advanced: no Aanpassen voor tv (mutter reports no underscanning)',
            not h.find('settings', 'st-display-advanced-underscanning'))
    rate = node_of(h, 'st-display-advanced-refresh')
    h.check('display-advanced: one refresh rate (60 Hz), its drop-down insensitive',
            bool(rate) and rate.get('sensitive') is False and any(text.startswith('60') for text in labels_of(h, rate)),
            labels=rate and labels_of(h, rate))
    name = 'st-display-advanced-xwayland-scaling-factor'
    if not h.find('settings', name):
        h.check('display-advanced: Schaal voor apps herstellen shows (Ubuntu turns xwayland-native-scaling on)', False,
                features=h.gsettings_get(MUTTER, 'experimental-features'))
        return
    on = switch_active(h, name)
    press(h, name)
    want = 1.0 if on else 0.0
    wrote(h, 'display-advanced: Schaal voor apps herstellen writes xwayland-scaling-factor GNOME\'s way (on 0, off 1)',
          lambda: number(h.gsettings_get(WAYLAND, 'xwayland-scaling-factor')) == want, was_on=on,
          value=lambda: h.gsettings_get(WAYLAND, 'xwayland-scaling-factor'))
    h.wait_text('settings', tr(h, 'Start apps opnieuw om de nieuwe schaal toe te passen',
                               'Restart apps for the new scaling to take effect'), timeout=4)
    follows(h, 'display-advanced', name, WAYLAND, 'xwayland-scaling-factor', '0.0', True, 'Schaal voor apps herstellen')


# ---------------------------------------------------------------------------------------------------- slice 2: sound

def start_audio(h):
    """Amendment 62: pipewire, wireplumber and pipewire-pulse in the sandbox, with null devices; True when
    pipewire-pulse's socket is there."""
    runtime = Path(os.environ['XDG_RUNTIME_DIR'])
    conf = Path(os.environ['XDG_CONFIG_HOME']) / 'wireplumber' / 'wireplumber.conf.d'
    conf.mkdir(parents=True, exist_ok=True)
    (conf / '90-winpoint-no-hardware.conf').write_text(WIREPLUMBER_CONF)
    h.spawn('pipewire', ['/usr/bin/pipewire'])
    try:
        h.until(lambda: (runtime / 'pipewire-0').exists(), 'pipewire socket', 15)
    except TimeoutError:
        h.check('audio: pipewire started', False, known='harness')
        return False
    h.spawn('wireplumber', ['/usr/bin/wireplumber'])
    h.spawn('pipewire-pulse', ['/usr/bin/pipewire-pulse'])
    for name, desc, cls, prio in NODES:
        props = (f'{{ factory.name=support.null-audio-sink node.name={name} node.description="{desc}" '
                 f'media.class={cls} object.linger=true audio.position=[FL FR] priority.session={prio} '
                 f'priority.driver={prio} }}')
        proc = h.spawn(f'pw-cli-{name}', ['/usr/bin/pw-cli', 'create-node', 'adapter', props])
        try:
            proc.wait(10)
        except Exception:
            proc.kill()
    try:
        h.until(lambda: (runtime / 'pulse' / 'native').exists(), 'pipewire-pulse socket', 15)
    except TimeoutError:
        h.check('audio: pipewire-pulse started', False, known='harness')
        return False
    return True


def sound_state(h):
    return h.backend('settings', 'sound.state') or {}


def wait_state(h, test, label, timeout=10):
    try:
        return h.until(lambda: (lambda st: st if test(st) else None)(sound_state(h)), label, timeout, 0.3)
    except TimeoutError:
        return None


def sink_of(state, name):
    return next((d for d in state.get('sinks') or [] if d.get('name') == name), None)


def source_of(state, name):
    return next((d for d in state.get('sources') or [] if d.get('name') == name), None)


def sound_page(h):
    state = wait_state(h, lambda st: st.get('available') and len(st.get('outputs') or []) >= 2
                       and len(st.get('inputs') or []) >= 1, 'sound.state sees the private PipeWire', 25)
    h.record('sound.state', state)
    h.require('sound.state reads pipewire-pulse through libpulse', bool(state))
    page(h, 'sound', tr(h, 'Kies waar je geluid wilt afspelen', 'Choose where to play sound'))
    shot(h, 'sound', more=True)
    for name in ('st-sound-outputs', 'st-sound-inputs', 'st-sound-volume', 'st-sound-volume-mute', 'st-sound-input-volume',
                 'st-sound-input-level', 'st-sound-out-0', 'st-sound-out-1', 'st-sound-out-0-props', 'st-sound-in-0',
                 'st-sound-pair-output', 'st-sound-pair-input', 'st-sound-more'):
        h.check(f'sound: {name}', bool(h.find('settings', name)))
    h.check('sound: no placeholder and not the wpctl page', not h.find('settings', 'st-placeholder')
            and not h.find('settings', 'sound-volume'))
    for link in ('link-sound-devices', 'link-volume-mixer'):
        h.check(f'sound: {link} under Geavanceerd', bool(h.find('settings', link)))
    no_dutch(h, 'sound')
    outputs = state['outputs']
    other = outputs[1]['device'] if outputs[0]['default'] else outputs[0]['device']
    index = 1 if outputs[0]['default'] else 0
    press(h, f'st-sound-out-{index}')
    after = wait_state(h, lambda st: st.get('defaultSink') == other, 'default sink moved')
    h.check('sound: picking an output makes it the default', after is not None, wanted=other)
    # the volume slider (a click in its middle) and the mute button reach the default sink; 90 % first, so the click
    # in the middle is a change whatever the null sink started with
    h.backend('settings', 'sound.setVolume', {'kind': 'sink', 'name': other, 'volume': 90})
    wait_state(h, lambda st: abs(((sink_of(st, other) or {}).get('volume') or 0) - 90) <= 1, 'volume 90')
    h.sleep(1.0)
    before = sink_of(sound_state(h), other) or {}
    press(h, 'st-sound-volume')
    moved = wait_state(h, lambda st: abs(((sink_of(st, other) or {}).get('volume') or 0) - (before.get('volume') or 0)) >= 5,
                       'volume written')
    h.check('sound: the volume slider writes the sink volume', moved is not None, before=before.get('volume'),
            after=(sink_of(moved or {}, other) or {}).get('volume'))
    press(h, 'st-sound-volume-mute')
    muted = wait_state(h, lambda st: (sink_of(st, other) or {}).get('mute') is True, 'muted')
    h.check('sound: the mute button mutes the sink', muted is not None)
    press(h, 'st-sound-volume-mute')
    wait_state(h, lambda st: (sink_of(st, other) or {}).get('mute') is False, 'unmuted')
    # the input's volume: the default source
    src = (sound_state(h).get('defaultSource')) or 'w11-mic'
    h.backend('settings', 'sound.setVolume', {'kind': 'source', 'name': src, 'volume': 90})
    wait_state(h, lambda st: abs(((source_of(st, src) or {}).get('volume') or 0) - 90) <= 1, 'input volume 90')
    h.sleep(1.0)
    press(h, 'st-sound-input-volume')
    moved = wait_state(h, lambda st: abs(((source_of(st, src) or {}).get('volume') or 90) - 90) >= 5, 'input volume written')
    h.check('sound: the input volume slider writes the source volume', moved is not None, source=src,
            after=(source_of(moved or sound_state(h), src) or {}).get('volume'))
    # system.audio.* (taskbar, Snelle instellingen) answers from the same client
    legacy = h.backend('settings', 'system.audio.get') or {}
    st = sound_state(h)
    ids = sorted(d.get('objectId') for d in st.get('sinks') or [])
    h.record('system.audio.get', legacy)
    h.check('sound: system.audio.get is the pulse client with PipeWire object ids', legacy.get('available') is True
            and sorted(s.get('id') for s in legacy.get('sinks') or []) == ids
            and next((s for s in legacy['sinks'] if s.get('default')), {}).get('name') == (sink_of(st, other) or {}).get('description'),
            legacy=legacy, ids=ids)
    return st


def alerts(h):
    press(h, 'st-sound-more')
    try:
        h.until(lambda: bool(h.find('settings', 'st-sound-alert')), 'Meer geluidsinstellingen', 10)
        ok = True
    except TimeoutError:
        ok = False
    h.check('alerts: Meer geluidsinstellingen opens the Geluiden tab', ok)
    if ok:
        h.screenshot(f'sound-alerts.{h.lang}')
        h.check('alerts: Opstartgeluid afspelen (Ubuntu)', bool(h.find('settings', 'st-sound-startup-sound')))
        h.gsettings_set(UBUNTU, 'startup-sound', "''")
        h.sleep(0.5)
        # the dev channel's toggle (the dialog's layer gets no real pointer click in the headless session)
        h.click('settings', 'st-sound-startup-sound', real=False)
        wrote(h, 'alerts: Opstartgeluid afspelen writes startup-sound warty-startup',
              lambda: h.gsettings_get(UBUNTU, 'startup-sound') == "'warty-startup'",
              value=lambda: h.gsettings_get(UBUNTU, 'startup-sound'))
        h.key('settings', 'Escape')
    answer = h.backend('settings', 'sound.alerts.set', {'alert': 'click'})
    h.record('sound.alerts.set click', answer)
    link = Path(os.environ['XDG_DATA_HOME']) / 'sounds' / '__custom' / 'bell-terminal.ogg'
    target = os.readlink(link) if link.is_symlink() else None
    h.check('alerts: Klik writes GNOME\'s __custom theme', (answer or {}).get('alert') == 'click'
            and h.gsettings_get(SOUND, 'theme-name') == "'__custom'" and (target or '').endswith('/click.ogg'),
            theme=h.gsettings_get(SOUND, 'theme-name'), link=target)
    answer = h.backend('settings', 'sound.alerts.set', {'alert': 'default'})
    h.check('alerts: Standaard puts the theme back', (answer or {}).get('alert') == 'default'
            and h.gsettings_get(SOUND, 'theme-name') != "'__custom'", theme=h.gsettings_get(SOUND, 'theme-name'))


def device_page(h, state):
    first = (state.get('outputs') or [{}])[0]
    sink = first.get('device')
    reveal(h, 'st-sound-out-0-props')   # below the fixed header, where the pointer reaches it
    h.settings_data_page('sound', 'st-sound-out-0-props', first.get('title'))
    maximize(h)
    went(h, 'sound-device', first.get('title'))
    shot(h, 'sound-device', more=True)
    for name in ('st-sound-device-default', 'st-sound-device-volume', 'st-sound-device-left', 'st-sound-device-right',
                 'st-sound-device-allow-volume-above-100-percent'):
        h.check(f'device: {name}', bool(h.find('settings', name)))
    no_dutch(h, 'sound-device')
    # Instellen als standaard: the first output is not the default since the Geluid page picked the other one
    if (sound_state(h).get('defaultSink')) != sink:
        press(h, 'st-sound-device-default')
        h.check('device: Instellen als standaard makes it the default',
                wait_state(h, lambda st: st.get('defaultSink') == sink, 'default sink') is not None, sink=sink)
    h.backend('settings', 'sound.setVolume', {'kind': 'sink', 'name': sink, 'volume': 90})
    wait_state(h, lambda st: abs(((sink_of(st, sink) or {}).get('volume') or 0) - 90) <= 1, 'device volume 90')
    h.sleep(1.0)
    press(h, 'st-sound-device-volume')
    h.check('device: Volume writes the sink volume',
            wait_state(h, lambda st: abs(((sink_of(st, sink) or {}).get('volume') or 90) - 90) >= 5, 'device volume') is not None)
    # Windows' Rechterkanaal at half: GNOME's balance towards the left (pa_cvolume_get_balance -0.5). From both
    # channels at 100: 1 s first, so the Volume slider's own commit has landed and does not take the sink back to its
    # value after the reset (the page writes the other channel as the server has it, review D-4)
    h.sleep(1.0)
    h.backend('settings', 'sound.setBalance', {'sink': sink, 'left': 100, 'right': 100})
    wait_state(h, lambda st: abs(((sink_of(st, sink) or {}).get('balance') or 0)) < 0.02
               and min((sink_of(st, sink) or {}).get('left') or 0, (sink_of(st, sink) or {}).get('right') or 0) >= 98,
               'both channels at 100')
    h.sleep(1.0)
    press(h, 'st-sound-device-right')
    st = wait_state(h, lambda st: ((sink_of(st, sink) or {}).get('balance') or 0) < -0.3
                    and ((sink_of(st, sink) or {}).get('right') or 100) < 70, 'balance written')
    h.check('device: Rechterkanaal at half reaches the sink as GNOME\'s balance towards the left', st is not None,
            sink=sink_of(sound_state(h), sink))
    # review D-4: the volume goes down elsewhere (both channels follow), then one channel moves: the other one keeps
    # the new volume instead of the page's first value (1 s first: a slider just moved ignores the server for 700 ms)
    h.sleep(1.0)
    h.backend('settings', 'sound.setVolume', {'kind': 'sink', 'name': sink, 'volume': 20})
    low = wait_state(h, lambda st: ((sink_of(st, sink) or {}).get('left') or 100) <= 22, 'volume 20')
    h.sleep(1.5)
    left = (sink_of(low or sound_state(h), sink) or {}).get('left')
    press(h, 'st-sound-device-right')
    after = wait_state(h, lambda st: ((sink_of(st, sink) or {}).get('right') or 0) > 30, 'right channel written')
    dev = sink_of(after or sound_state(h), sink) or {}
    h.check('device: after a volume change Rechterkanaal leaves Linkerkanaal at the new volume',
            after is not None and left is not None and abs((dev.get('left') or 0) - left) <= 2,
            before=left, sink={k: dev.get(k) for k in ('left', 'right', 'volume', 'balance')})
    flip(h, 'device', 'st-sound-device-allow-volume-above-100-percent', SOUND, 'allow-volume-above-100-percent',
         'Volume boven 100% toestaan writes allow-volume-above-100-percent')
    follows(h, 'device', 'st-sound-device-allow-volume-above-100-percent', SOUND, 'allow-volume-above-100-percent',
            'false', False, 'Volume boven 100% toestaan')
    page(h, 'sound-devices', tr(h, 'Uitvoerapparaten', 'Output devices'))
    h.screenshot(f'sound-devices.{h.lang}')
    h.check('devices: Alle geluidsapparaten lists outputs and inputs',
            bool(h.find('settings', 'st-sound-devices-out-0')) and bool(h.find('settings', 'st-sound-devices-in-0')))
    no_dutch(h, 'sound-devices')
    mic = (state.get('inputs') or [{}])[0].get('title')
    press(h, 'st-sound-devices-in-0')
    h.wait_text('settings', tr(h, 'Invoerinstellingen', 'Input settings'), timeout=10)
    went(h, 'sound-device', mic)
    h.check('devices: the input\'s Eigenschappen has Je microfoon testen', bool(h.find('settings', 'st-sound-device-mic-test')))


def mixer_page(h, state):
    play = h.spawn('pw-play', ['/usr/bin/pw-play', '--target', 'w11-speakers', '--raw', '--rate', '48000',
                               '--channels', '2', '--format', 's16', '/dev/zero'])
    st = wait_state(h, lambda st: any(s.get('kind') == 'output' for s in st.get('streams') or []), 'a playback stream', 15)
    page(h, 'volume-mixer', tr(h, 'Systeemgeluiden', 'System sounds'))
    h.sleep(1.0)
    shot(h, 'volume-mixer', more=True)
    for name in ('st-volume-mixer-output', 'st-volume-mixer-input', 'st-volume-mixer-volume', 'st-volume-mixer-event'):
        h.check(f'mixer: {name}', bool(h.find('settings', name)))
    no_dutch(h, 'volume-mixer')
    # Uitvoerapparaat (the default) through the real drop-down
    default = sound_state(h).get('defaultSink')
    other = 'w11-headphones' if default != 'w11-headphones' else 'w11-speakers'
    title = next(desc for name, desc, *_ in NODES if name == other)
    try:
        choose(h, 'st-volume-mixer-output', title)
    except Exception as error:   # noqa: BLE001
        h.record('mixer.output', str(error))
    h.check('mixer: the Uitvoerapparaat drop-down sets the default output',
            wait_state(h, lambda st: st.get('defaultSink') == other, 'default via the mixer') is not None, wanted=other)
    # the master volume (the default output)
    h.backend('settings', 'sound.setVolume', {'kind': 'sink', 'name': other, 'volume': 90})
    wait_state(h, lambda st: abs(((sink_of(st, other) or {}).get('volume') or 0) - 90) <= 1, 'master 90')
    h.sleep(1.0)
    press(h, 'st-volume-mixer-volume')
    h.check('mixer: the master volume writes the default output',
            wait_state(h, lambda st: abs(((sink_of(st, other) or {}).get('volume') or 90) - 90) >= 5, 'master written') is not None)
    # Systeemgeluiden: the event role (ext-stream-restore)
    h.backend('settings', 'sound.setEventVolume', {'volume': 90})
    wait_state(h, lambda st: st.get('eventVolume') == 90, 'event 90')
    h.sleep(1.0)
    press(h, 'st-volume-mixer-event')
    ev = wait_state(h, lambda st: abs((st.get('eventVolume') or 90) - 90) >= 5, 'event volume')
    h.check('mixer: Systeemgeluiden writes the event role (ext-stream-restore)', ev is not None,
            now=sound_state(h).get('eventVolume'))
    stream = next((s for s in (st or {}).get('streams') or [] if s.get('kind') == 'output'), None)
    h.record('mixer.stream', stream)
    if stream is None:
        h.check('mixer: pw-play shows as an app', False, known='harness')
    else:
        row = f'st-volume-mixer-app-{stream["index"]}'
        h.check('mixer: the app row with its volume slider and its Uitvoerapparaat',
                bool(h.find('settings', f'{row}-volume')) and bool(h.find('settings', f'{row}-device')))
        h.backend('settings', 'sound.setVolume', {'kind': 'sinkInput', 'index': stream['index'], 'volume': 90})
        h.sleep(1.0)
        press(h, f'{row}-volume')

        def app_volume(st):
            return next((s.get('volume') for s in st.get('streams') or [] if s.get('index') == stream['index']), None)
        h.check('mixer: the app\'s volume slider writes its stream', wait_state(
            h, lambda st: app_volume(st) is not None and abs(app_volume(st) - 90) >= 5, 'app volume') is not None,
            now=app_volume(sound_state(h)))
        h.screenshot(f'volume-mixer-app.{h.lang}')
        try:
            choose(h, f'{row}-device', 'WinPoint Test Headphones')
        except Exception as error:   # noqa: BLE001
            h.record('mixer.appDevice', str(error))
        moved = wait_state(h, lambda st: any(s.get('index') == stream['index'] and s.get('device') == 'w11-headphones'
                                             for s in st.get('streams') or []), 'stream moved')
        h.check('mixer: the app\'s Uitvoerapparaat drop-down moves its stream (moveStream)', moved is not None)
        # review D-6: a sink that comes while the app plays (headphones connected) is in the app's drop-down at once
        props = ('{ factory.name=support.null-audio-sink node.name=w11-usb node.description="WinPoint Test USB" '
                 'media.class=Audio/Sink object.linger=true audio.position=[FL FR] priority.session=100 '
                 'priority.driver=100 }')
        proc = h.spawn('pw-cli-w11-usb', ['/usr/bin/pw-cli', 'create-node', 'adapter', props])
        try:
            proc.wait(10)
        except Exception:   # noqa: BLE001
            proc.kill()
        came = wait_state(h, lambda st: sink_of(st, 'w11-usb') is not None, 'a new sink', 15)
        h.sleep(1.0)
        try:
            choose(h, f'{row}-device', 'WinPoint Test USB')
        except Exception as error:   # noqa: BLE001
            h.record('mixer.newDevice', str(error))
        moved = wait_state(h, lambda st: any(s.get('index') == stream['index'] and s.get('device') == 'w11-usb'
                                             for s in st.get('streams') or []), 'stream moved to the new sink')
        h.check('mixer: a sink that comes while the app plays is in its Uitvoerapparaat drop-down', came is not None
                and moved is not None, streams=(sound_state(h).get('streams') or []))
    play.terminate()


def sound_pages(h):
    state = sound_page(h)
    alerts(h)
    device_page(h, state)
    mixer_page(h, state)


# ------------------------------------------------------------------------------------- slice 3: power ... screentime

def power_bare(h):
    """Energie en batterij without UPower, power-profiles-daemon or logind (the harness's empty system bus)."""
    page(h, 'power', tr(h, 'Besturingselementen voor deksel en aan/uit-knop', 'Lid & power button controls'))
    h.sleep(1.0)
    shot(h, 'power', more=True)
    no_dutch(h, 'power')
    h.check('power: no placeholder', not h.find('settings', 'st-placeholder'))
    for name in ('st-power-screen-sleep', 'st-power-lid-button', 'power-mode'):
        h.check(f'power: {name}', bool(h.find('settings', name)))
    h.check('power: without UPower no Batterij group, the Netstroom card instead', not h.find('settings', 'st-power-battery')
            and bool(h.find('settings', label=tr(h, 'Netstroom', 'Mains power'))))
    mode = node_of(h, 'power-mode')
    h.check('power: without power-profiles-daemon Energiestand is "Niet beschikbaar", insensitive',
            bool(mode) and mode.get('sensitive') is False and tr(h, 'Niet beschikbaar', 'Not available') in labels_of(h, mode),
            labels=mode and labels_of(h, mode))
    h.check('power: no Energiebesparing without a power-saver profile', not h.find('settings', 'st-power-energy-saver'))
    # Scherm en slaapstand
    expand(h, 'st-power-screen-sleep')
    idle = h.gsettings_get(SESSION, 'idle-delay')
    try:
        choose(h, 'power-screen', tr(h, '1 uur', '1 hour'))
    except Exception as error:   # noqa: BLE001
        h.record('power.screen', str(error))
    wrote(h, 'power: Scherm uitschakelen na 1 uur writes idle-delay 3600',
          lambda: number(h.gsettings_get(SESSION, 'idle-delay')) == 3600, value=lambda: h.gsettings_get(SESSION, 'idle-delay'))
    h.gsettings_set(SESSION, 'idle-delay', 'uint32 600')
    wrote(h, 'power: Scherm uitschakelen na follows an outside idle-delay (10 minuten)',
          lambda: shows(h, 'power-screen', tr(h, '10 minuten', '10 minutes')))
    h.gsettings_set(SESSION, 'idle-delay', idle or 'uint32 0')
    if h.find('settings', 'power-dim'):
        flip(h, 'power', 'power-dim', POWER, 'idle-dim', 'Scherm dimmen bij inactiviteit writes idle-dim')
    sleep = node_of(h, 'power-sleep-ac')
    h.check('power: without logind automatic sleep is insensitive, with GNOME\'s reason',
            bool(sleep) and sleep.get('sensitive') is False and bool(h.find('settings', label=tr(
                h, 'Deze computer ondersteunt automatisch slapen momenteel niet.',
                'This computer doesn’t support automatic sleep right now.'))))
    h.check('power: no "Als je automatisch slapen uitschakelt" warning where the pc cannot sleep (GNOME hides that row)',
            not h.find('settings', 'st-power-suspend-notice'))
    h.screenshot(f'power-sleep.{h.lang}')
    # Besturingselementen voor deksel en aan/uit-knop
    expand(h, 'st-power-lid-button')
    h.gsettings_set(POWER, 'power-button-action', "'interactive'")
    h.sleep(0.5)
    try:
        choose(h, 'st-power-power-button-action', tr(h, 'Niets doen', 'Do nothing'))
    except Exception as error:   # noqa: BLE001
        h.record('power.choose', str(error))
    wrote(h, 'power: Niets doen writes power-button-action nothing',
          lambda: h.gsettings_get(POWER, 'power-button-action') == "'nothing'",
          value=lambda: h.gsettings_get(POWER, 'power-button-action'))
    h.gsettings_set(POWER, 'power-button-action', "'suspend'")
    wrote(h, 'power: the power-button drop-down follows an outside power-button-action (Slaapstand)',
          lambda: shows(h, 'st-power-power-button-action', tr(h, 'Slaapstand', 'Sleep')))
    # Aan/uit: Slaapstand runs at once, Opnieuw opstarten asks first
    press(h, 'power-suspend')
    wrote(h, 'power: Slaapstand runs systemctl suspend (the run\'s stub, logs/guard-calls.log)',
          lambda: any(line.split('\t')[1:] == ['systemctl', 'suspend'] for line in guard_lines(h)))
    press(h, 'power-reboot')
    asked = h.wait_text('settings', tr(h, 'Opnieuw opstarten?', 'Restart?'), timeout=5)
    h.screenshot(f'power-restart.{h.lang}')
    h.key('settings', 'Escape')
    h.sleep(1.5)
    h.check('power: Opnieuw opstarten asks first; Escape runs nothing', bool(asked)
            and not any('reboot' in line for line in guard_lines(h)), guard=guard_lines(h)[-5:])


def kit_header_control(h):
    """A real pointer click on the independent expander-header control toggles notifications."""
    h.open('settings:notifications')
    h.sleep(0.8)
    before = settings_json(h).get('notifications', {}).get('showToasts')
    if not h.find('settings', 'notifications-toggle'):
        h.record('kit.headerControl', 'no notifications-toggle')
        return
    click = press(h, 'notifications-toggle')
    h.sleep(1.5)
    after = settings_json(h).get('notifications', {}).get('showToasts')
    taken = before != after
    h.record('kit.headerControl', {'click': click, 'before': before, 'after': after})
    h.check('kit: a switch in an expander header takes a real pointer click', taken,
            before=before, after=after)
    if taken:
        h.click('settings', 'notifications-toggle', real=False)


def start_hardware(h):
    """The stand-ins of the hardware pages, then Settings again (caps.get is read once at its start)."""
    package, specs = standins()
    upower = package.start(h, specs.upower(int(time.time())), 'upower')
    profiles = specs.start_text(h, specs.power_profiles(), 'power-profiles')
    sensors = package.start(h, specs.sensor_proxy(), 'sensor-proxy')
    h.close('settings')
    h.sleep(1.0)
    return upower, profiles, sensors


def power_page(h, upower, profiles):
    page(h, 'power', tr(h, 'Batterij opladen', 'Battery charging'))
    h.sleep(1.0)
    battery = h.backend('settings', 'system.battery.get') or {}
    charging = h.backend('settings', 'battery.charging.get') or {}
    energy = h.backend('settings', 'system.power.settings.get') or {}
    h.record('power.battery', {'battery': battery, 'charging': charging, 'profiles': energy.get('profiles')})
    h.check('power: system.battery.get and battery.charging.get read the UPower stand-in',
            battery.get('present') is True and charging.get('supported') is True, battery=battery, charging=charging)
    shot(h, 'power-battery', more=True)
    h.check('power: with UPower the battery hero shows (76 %)', bool(h.find('settings', cls='st-battery-hero')))
    for name in ('st-power-battery', 'st-power-energy-saver', 'st-power-show-battery-percentage', 'st-power-charging',
                 'st-power-levels'):
        h.check(f'power: with UPower and power-profiles-daemon {name} shows', bool(h.find('settings', name)))
    h.check('power: no Netstroom card with a battery', not h.find('settings', label=tr(h, 'Netstroom', 'Mains power')))
    # Energiestand: power-profiles-daemon's ActiveProfile through Properties.Set (GNOME's way: no interactive flag)
    mode = node_of(h, 'power-mode')
    h.check('power: Energiestand with power-profiles-daemon (Gebalanceerd)', bool(mode) and mode.get('sensitive') is True
            and any(text.startswith(tr(h, 'Gebalanceerd', 'Balanced')) for text in labels_of(h, mode)),
            labels=mode and labels_of(h, mode), profiles=energy.get('profiles'))
    if profiles is not None:
        try:
            choose(h, 'power-mode', tr(h, 'Beste prestaties', 'Best performance'))
        except Exception as error:   # noqa: BLE001
            h.record('power.mode', str(error))
        wrote(h, 'power: Beste prestaties sets ActiveProfile performance (Properties.Set)',
              lambda: any(c['method'] == 'Set:ActiveProfile' and 'performance' in c['args'] for c in profiles.calls()),
              calls=lambda: [c for c in profiles.calls() if c['method'].startswith('Set:')])
        expand(h, 'st-power-energy-saver')
        try:
            press(h, 'st-power-saver')
        except LookupError as error:
            h.record('power.saver', str(error))
        wrote(h, 'power: Altijd energiebesparing gebruiken sets ActiveProfile power-saver',
              lambda: any(c['method'] == 'Set:ActiveProfile' and 'power-saver' in c['args'] for c in profiles.calls()),
              calls=lambda: [c for c in profiles.calls() if c['method'].startswith('Set:')])
        wrote(h, 'power: Energiestand follows: Beste energie-efficiëntie',
              lambda: shows(h, 'power-mode', tr(h, 'Beste energie-efficiëntie', 'Best power efficiency')))
        flip(h, 'power', 'st-power-power-saver-profile-on-low-battery', POWER, 'power-saver-profile-on-low-battery',
             'Energiebesparing automatisch inschakelen bij een laag batterijniveau writes power-saver-profile-on-low-battery')
        h.screenshot(f'power-saver.{h.lang}')
    # Batterijpercentage, Mirrored to taskbar.batteryPercent (the mirror runs in the panel host)
    flip(h, 'power', 'st-power-show-battery-percentage', INTERFACE, 'show-battery-percentage',
         'Batterijpercentage writes show-battery-percentage')
    shown = h.gsettings_get(INTERFACE, 'show-battery-percentage') == 'true'
    wrote(h, 'mirror: show-battery-percentage reaches settings.json taskbar.batteryPercent',
          lambda: settings_json(h).get('taskbar', {}).get('batteryPercent') is shown,
          setting=lambda: settings_json(h).get('taskbar', {}).get('batteryPercent'))
    if shown:
        texts = [str(n.get('label') or '') for n in h.hm.tree_nodes(h.native_state())]
        tray = any(re.fullmatch(r'76\s?%', t) for t in texts)
        h.check('mirror: the tray shows 76% next to the battery (taskbar/system.rs, section 8.2: step I)', tray)
    follows(h, 'power', 'st-power-show-battery-percentage', INTERFACE, 'show-battery-percentage', 'false', False,
            'Batterijpercentage')
    # Batterij opladen: UPower's EnableChargeThreshold with interactive authorization (polkit, section 5.8)
    expand(h, 'st-power-charging')
    if h.find('settings', 'st-power-charging-preserve') and upower is not None:
        press(h, 'st-power-charging-preserve')
        wrote(h, 'power: Batterijgezondheid behouden sends EnableChargeThreshold(true), interactive',
              lambda: any(c['method'] == 'EnableChargeThreshold' and 'true' in c['args'] and c['interactive']
                          for c in upower.calls()),
              calls=lambda: [c for c in upower.calls() if c['method'] == 'EnableChargeThreshold'])
    else:
        h.check('power: the charging radios', False)
    # Batterijniveaus
    expand(h, 'st-power-levels')
    h.sleep(1.0)
    history = h.backend('settings', 'battery.history', {'type': 'charge', 'timespan': 86400})
    h.check('power: battery.history reads GetHistory, oldest first', isinstance(history, dict)
            and history.get('available') is True and len(history.get('points') or []) == 25
            and history['points'][0][0] < history['points'][-1][0], points=len((history or {}).get('points') or []))
    h.check('power: Batterijniveaus draws the chart', bool(h.find('settings', 'st-power-levels-chart')))
    if upower is not None:
        try:
            choose(h, 'st-power-levels-span', tr(h, 'Afgelopen 7 dagen', 'Last 7 days'))
        except Exception as error:   # noqa: BLE001
            h.record('power.span', str(error))
        wrote(h, 'power: Afgelopen 7 dagen asks UPower GetHistory over 604800 s',
              lambda: any(c['method'] == 'GetHistory' and '604800' in c['args'] for c in upower.calls()),
              calls=lambda: [c['args'] for c in upower.calls() if c['method'] == 'GetHistory'])
    h.screenshot(f'power-levels.{h.lang}')


def display_hardware(h):
    caps = h.backend('settings', 'caps.get') or {}
    if caps.get('accelerometer') is not True:
        # the host keeps the bus probes 30 s (caps.rs BUS_CACHE): once more after that, with Settings started again
        h.sleep(31)
        h.close('settings')
        h.open('settings:display')
        caps = h.backend('settings', 'caps.get') or {}
    h.record('caps.withStandIns', caps)
    h.check('caps: the sensor stand-in gives accelerometer and ambientLight',
            caps.get('accelerometer') is True and caps.get('ambientLight') is True, caps=caps)
    page(h, 'display', tr(h, 'Helderheid en kleur', 'Brightness & colour'))
    name = 'st-display-orientation-lock'
    h.check('display: Draaivergrendeling shows with an accelerometer', bool(h.find('settings', name)))
    if h.find('settings', name):
        flip(h, 'display', name, TOUCHSCREEN, 'orientation-lock', 'Draaivergrendeling writes orientation-lock')
        follows(h, 'display', name, TOUCHSCREEN, 'orientation-lock', 'false', False, 'Draaivergrendeling')
        reveal(h, name)
        h.screenshot(f'display-rotation.{h.lang}')
    h.check('display: automatic brightness also needs a backlight (the virtual monitor has none)',
            not h.find('settings', 'st-display-ambient-enabled'))


def storage_pages(h):
    h.gsettings_set(PRIVACY, 'remove-old-trash-files', 'false')
    h.gsettings_set(PRIVACY, 'remove-old-temp-files', 'false')
    page(h, 'storage', tr(h, 'Opslagbeheer', 'Storage management'))
    shot(h, 'storage', more=True)
    no_dutch(h, 'storage')
    h.check('storage: Opslaginzicht with its chevron', bool(h.find('settings', 'link-storagesense')))
    press(h, 'st-storage-storage-sense')
    wrote(h, 'storage: the Opslaginzicht switch turns both purges on',
          lambda: h.gsettings_get(PRIVACY, 'remove-old-trash-files') == 'true'
          and h.gsettings_get(PRIVACY, 'remove-old-temp-files') == 'true',
          trash=lambda: h.gsettings_get(PRIVACY, 'remove-old-trash-files'),
          temp=lambda: h.gsettings_get(PRIVACY, 'remove-old-temp-files'))
    h.gsettings_set(PRIVACY, 'remove-old-trash-files', 'false')
    h.gsettings_set(PRIVACY, 'remove-old-temp-files', 'false')
    wrote(h, 'storage: the switch follows an outside write', lambda: not switch_active(h, 'st-storage-storage-sense'))

    press(h, 'link-storagesense')
    h.wait_text('settings', tr(h, 'Opschoonschema’s configureren', 'Configure cleanup schedules'), required=True)
    went(h, 'storagesense')
    shot(h, 'storagesense', more=True)
    no_dutch(h, 'storagesense')
    h.check('storagesense: no placeholder', not h.find('settings', 'st-placeholder'))
    for name in ('st-storagesense-remove-old-temp-files', 'st-storagesense-storage-sense', 'st-storagesense-old-files-age',
                 'st-storagesense-run', 'st-storagesense-empty-trash', 'st-storagesense-delete-temp'):
        h.check(f'storagesense: {name}', bool(h.find('settings', name)))
    run = node_of(h, 'st-storagesense-run')
    h.check('storagesense: Opslaginzicht nu uitvoeren is off while both purges are off', bool(run) and run.get('sensitive') is False)
    flip(h, 'storagesense', 'st-storagesense-remove-old-temp-files', PRIVACY, 'remove-old-temp-files',
         'Tijdelijke bestanden opschonen writes remove-old-temp-files')
    follows(h, 'storagesense', 'st-storagesense-remove-old-temp-files', PRIVACY, 'remove-old-temp-files', 'false', False,
            'Tijdelijke bestanden opschonen')
    press(h, 'st-storagesense-storage-sense')
    wrote(h, 'storagesense: the Opslaginzicht switch on its page turns both purges on',
          lambda: h.gsettings_get(PRIVACY, 'remove-old-trash-files') == 'true'
          and h.gsettings_get(PRIVACY, 'remove-old-temp-files') == 'true')
    try:
        choose(h, 'st-storagesense-old-files-age', tr(h, '14 dagen', '14 days'))
    except Exception as error:   # noqa: BLE001
        h.record('storagesense.choose', str(error))
    wrote(h, 'storagesense: 14 dagen writes remove-old-trash-files true and old-files-age 14',
          lambda: h.gsettings_get(PRIVACY, 'remove-old-trash-files') == 'true'
          and number(h.gsettings_get(PRIVACY, 'old-files-age')) == 14,
          age=lambda: h.gsettings_get(PRIVACY, 'old-files-age'))
    h.gsettings_set(PRIVACY, 'old-files-age', 'uint32 7')
    wrote(h, 'storagesense: an outside age of 7 days is an option of its own, selected',
          lambda: shows(h, 'st-storagesense-old-files-age', tr(h, '7 dagen', '7 days')))
    press(h, 'st-storagesense-run')
    h.wait_text('settings', tr(h, 'Opslaginzicht is uitgevoerd', 'Storage Sense has run'), timeout=8)
    answer = h.backend('settings', 'storage.cleanTemp', {'sense': True})
    h.record('storage.cleanTemp sense', answer)
    h.check('storagesense: Opslaginzicht nu uitvoeren runs gsd\'s purge with the keys (nothing old enough here)',
            isinstance(answer, dict) and answer.get('ok') is True and answer.get('age') == 7
            and isinstance(answer.get('temp'), dict) and isinstance(answer.get('trash'), dict), answer=answer)
    try:
        choose(h, 'st-storagesense-old-files-age', tr(h, 'Nooit', 'Never'))
    except Exception as error:   # noqa: BLE001
        h.record('storagesense.never', str(error))
    wrote(h, 'storagesense: Nooit writes remove-old-trash-files false (the age stays)',
          lambda: h.gsettings_get(PRIVACY, 'remove-old-trash-files') == 'false'
          and number(h.gsettings_get(PRIVACY, 'old-files-age')) == 7)
    # Prullenbak nu leegmaken: an item in the run's own Prullenbak, the question, Leegmaken
    trash = Path(os.environ['XDG_DATA_HOME']) / 'Trash'
    (trash / 'files').mkdir(parents=True, exist_ok=True)
    (trash / 'info').mkdir(parents=True, exist_ok=True)
    item = trash / 'files' / 'w11-old-note.txt'
    item.write_text('old\n')
    (trash / 'info' / 'w11-old-note.txt.trashinfo').write_text(
        '[Trash Info]\nPath=/tmp/w11-old-note.txt\nDeletionDate=2026-01-01T10:00:00\n')
    press(h, 'st-storagesense-empty-trash')
    try:
        h.until(lambda: bool(h.find('settings', 'w11-dialog-primary')), 'the Prullenbak question', 5)
        asked = True
    except TimeoutError:
        asked = False
    h.screenshot(f'storagesense-empty.{h.lang}')
    if asked:
        h.sleep(0.8)   # the dialog's entry transition
        click = h.click('settings', 'w11-dialog-primary')
        h.record('storagesense.emptyDialog.click', click)
        try:
            h.until(lambda: not item.exists(), 'the Prullenbak emptied', 5)
            real = True
        except TimeoutError:
            real = False
        # kit dialog.rs: the smoke layer's GestureClick claims every press in the bubble phase, which cancels the
        # button's own gesture (handoff note, Notes for I); Enter and the dev channel's click still work
        h.check('kit: a real pointer click on a kit dialog\'s button (Leegmaken)', real,
                click=click)
        if not real and h.find('settings', 'w11-dialog-primary'):
            h.record('storagesense.emptyDialog', 'the real click did not reach the button; dev click')
            h.click('settings', 'w11-dialog-primary', real=False)
    wrote(h, 'storagesense: Prullenbak nu leegmaken asks first, then empties the Prullenbak', lambda: not item.exists(),
          asked=asked)


def switchers_match(h, current_only):
    """Read effective values: a keyfile can omit an unchanged schema default (window-switcher defaults true)."""
    schemas = ('org.gnome.shell.app-switcher', 'org.gnome.shell.window-switcher')
    value = h.backend('settings', 'gsettings.get',
                      {'keys': [{'schema': schema, 'key': 'current-workspace-only'} for schema in schemas]}) or {}
    items = value.get('items', [])
    return len(items) == 2 and all(item.get('available') is True and item.get('value') is current_only for item in items)


def multitasking_page(h):
    # GNOME's App Switching at "current workspace only" (the row then shows Alleen op het bureaublad dat ik gebruik)
    h.gsettings_set('org.gnome.shell.app-switcher', 'current-workspace-only', 'true')
    page(h, 'multitasking', tr(h, 'Vensters uitlijnen', 'Snap windows'))
    h.check('multitasking: no placeholder', not h.find('settings', 'st-placeholder'))
    expand(h, 'st-multitasking-desktops')
    shot(h, 'multitasking', more=True)
    no_dutch(h, 'multitasking')
    for name in ('st-multitasking-edge-tiling', 'st-multitasking-taskbar-all-desktops',
                 'st-multitasking-app-switcher-current-workspace-only', 'st-multitasking-dynamic-workspaces',
                 'st-multitasking-workspaces-only-on-primary', 'st-multitasking-enable-hot-corners'):
        h.check(f'multitasking: {name}', bool(h.find('settings', name)))
    flip(h, 'multitasking', 'st-multitasking-edge-tiling', MUTTER, 'edge-tiling', 'Vensters uitlijnen writes edge-tiling')
    follows(h, 'multitasking', 'st-multitasking-edge-tiling', MUTTER, 'edge-tiling', 'true', True, 'Vensters uitlijnen')
    try:
        choose(h, 'st-multitasking-taskbar-all-desktops', tr(h, 'Op alle bureaubladen', 'On all desktops'))
    except Exception as error:   # noqa: BLE001
        h.record('multitasking.taskbar', str(error))
    wrote(h, 'multitasking: Op de taakbalk ... Op alle bureaubladen writes taskbar.allDesktops (amendment 12)',
          lambda: settings_json(h).get('taskbar', {}).get('allDesktops') is True,
          setting=lambda: settings_json(h).get('taskbar'))
    # GNOME's initial preference wins (H12); selecting its already-active default emits no change.
    # Establish the opposite choice first, then prove both switcher keys are written in each direction.
    choose(h, 'st-multitasking-app-switcher-current-workspace-only',
           tr(h, 'Alleen op het bureaublad dat ik gebruik', 'Only on the desktop I’m using'))
    wrote(h, 'multitasking: current desktop writes taskView and both GNOME switchers',
          lambda: settings_json(h).get('taskView', {}).get('allDesktops') is False
          and switchers_match(h, True))
    try:
        choose(h, 'st-multitasking-app-switcher-current-workspace-only', tr(h, 'Op alle bureaubladen', 'On all desktops'))
    except Exception as error:   # noqa: BLE001
        h.record('multitasking.choose', str(error))
    wrote(h, 'multitasking: Alt+Tab on all desktops writes taskView.allDesktops and both GNOME switchers (amendment 12)',
          lambda: settings_json(h).get('taskView', {}).get('allDesktops') is True
          and switchers_match(h, False),
          setting=lambda: settings_json(h).get('taskView'),
          apps=lambda: h.gsettings_get('org.gnome.shell.app-switcher', 'current-workspace-only'),
          windows=lambda: h.gsettings_get('org.gnome.shell.window-switcher', 'current-workspace-only'))
    # review D-7: an outside change of GNOME's App Switching (gsettings, GNOME Settings in the Ubuntu session)
    alt = 'st-multitasking-app-switcher-current-workspace-only'
    only = tr(h, 'Alleen op het bureaublad dat ik gebruik', 'Only on the desktop I’m using')
    for value, all_desktops, label in (('true', False, only), ('false', True, tr(h, 'Op alle bureaubladen', 'On all desktops'))):
        h.gsettings_set('org.gnome.shell.app-switcher', 'current-workspace-only', value)
        wrote(h, f'multitasking: an outside current-workspace-only {value} shows "{label}" and sets taskView.allDesktops '
              f'{str(all_desktops).lower()}',
              lambda: shows(h, alt, label) and settings_json(h).get('taskView', {}).get('allDesktops') is all_desktops,
              setting=lambda: settings_json(h).get('taskView'))
    h.gsettings_set(MUTTER, 'dynamic-workspaces', 'true')
    h.sleep(0.5)
    try:
        press(h, label=tr(h, 'Vast aantal bureaubladen', 'Fixed number of desktops'))
        wrote(h, 'multitasking: Vast aantal bureaubladen writes dynamic-workspaces false',
              lambda: h.gsettings_get(MUTTER, 'dynamic-workspaces') == 'false')
        wrote(h, 'multitasking: Aantal bureaubladen shows for a fixed number',
              lambda: bool(h.find('settings', 'st-multitasking-num-workspaces')))
        count = number(h.gsettings_get(WM, 'num-workspaces')) or 4.0
        spin_up(h, 'st-multitasking-num-workspaces')
        wrote(h, f'multitasking: Aantal bureaubladen + writes num-workspaces {int(count) + 1}',
              lambda: number(h.gsettings_get(WM, 'num-workspaces')) == count + 1,
              value=lambda: h.gsettings_get(WM, 'num-workspaces'))
        h.gsettings_set(MUTTER, 'dynamic-workspaces', 'true')
        wrote(h, 'multitasking: an outside dynamic-workspaces true selects Lege bureaubladen automatisch verwijderen '
              'and hides the number', lambda: not h.find('settings', 'st-multitasking-num-workspaces'))
    except LookupError as error:
        h.check('multitasking: the Vast aantal bureaubladen radio and the number box', False, error=str(error))
    h.gsettings_set(MUTTER, 'workspaces-only-on-primary', 'true')
    h.sleep(0.5)
    try:
        press(h, label=tr(h, 'Bureaubladen op alle beeldschermen', 'Desktops on all displays'))
        wrote(h, 'multitasking: Bureaubladen op alle beeldschermen writes workspaces-only-on-primary false',
              lambda: h.gsettings_get(MUTTER, 'workspaces-only-on-primary') == 'false')
    except LookupError as error:
        h.check('multitasking: the Bureaubladen op alle beeldschermen radio', False, error=str(error))
    flip(h, 'multitasking', 'st-multitasking-enable-hot-corners', INTERFACE, 'enable-hot-corners',
         'Actieve hoek writes enable-hot-corners')
    follows(h, 'multitasking', 'st-multitasking-enable-hot-corners', INTERFACE, 'enable-hot-corners', 'true', True,
            'Actieve hoek')


def screentime_page(h):
    history = h.backend('settings', 'wellbeing.history') or {}
    h.record('wellbeing.history', history)
    h.check('screentime: wellbeing.history reads gnome-shell\'s record', history.get('available') is True
            and isinstance(history.get('days'), list), history=history)
    h.gsettings_set(LIMITS, 'history-enabled', 'true')
    page(h, 'screentime', tr(h, 'Pauzeherinneringen', 'Break reminders'))
    h.sleep(0.5)
    shot(h, 'screentime', more=True)
    no_dutch(h, 'screentime')
    h.check('screentime: no placeholder', not h.find('settings', 'st-placeholder'))
    has_data = any((d.get('seconds') or 0) > 0 for d in history.get('days') or [])
    h.check('screentime: the chart with gnome-shell\'s record, else Screen Time Unavailable',
            bool(h.find('settings', 'st-screentime-chart' if has_data else 'st-screentime-unavailable')), data=has_data)
    flip(h, 'screentime', 'st-screentime-history-enabled', LIMITS, 'history-enabled',
         'Schermtijd vastleggen writes history-enabled')
    wrote(h, 'screentime: recording off shows GNOME\'s Screen Time Recording Disabled',
          lambda: bool(h.find('settings', 'st-screentime-disabled')))
    follows(h, 'screentime', 'st-screentime-history-enabled', LIMITS, 'history-enabled', 'true', True,
            'Schermtijd vastleggen')
    h.gsettings_set(BREAKS, 'selected-breaks', '@as []')
    h.sleep(0.5)
    press(h, 'st-screentime-selected-breaks-eyesight')
    wrote(h, 'screentime: Herinneringen voor je ogen adds eyesight to selected-breaks',
          lambda: "'eyesight'" in (h.gsettings_get(BREAKS, 'selected-breaks') or ''),
          value=lambda: h.gsettings_get(BREAKS, 'selected-breaks'))
    press(h, 'st-screentime-selected-breaks-movement')
    wrote(h, 'screentime: Herinneringen om te bewegen adds movement to selected-breaks',
          lambda: "'movement'" in (h.gsettings_get(BREAKS, 'selected-breaks') or ''),
          value=lambda: h.gsettings_get(BREAKS, 'selected-breaks'))
    h.sleep(0.4)
    try:
        choose(h, 'st-screentime-interval-seconds', tr(h, '3 minuten elke 30 minuten', '3 minutes every 30 minutes'))
    except Exception as error:   # noqa: BLE001
        h.record('screentime.choose', str(error))
    wrote(h, 'screentime: the movement schedule writes 180 s every 1800 s',
          lambda: number(h.gsettings_get(MOVEMENT, 'duration-seconds')) == 180
          and number(h.gsettings_get(MOVEMENT, 'interval-seconds')) == 1800,
          duration=lambda: h.gsettings_get(MOVEMENT, 'duration-seconds'))
    # Geluiden: play-sound of both breaks in one write
    on = switch_active(h, 'st-screentime-movement-play-sound')
    want = 'false' if on else 'true'
    press(h, 'st-screentime-movement-play-sound')
    wrote(h, 'screentime: Geluiden writes play-sound of both breaks',
          lambda: h.gsettings_get(MOVEMENT, 'play-sound') == want and h.gsettings_get(EYESIGHT, 'play-sound') == want,
          movement=lambda: h.gsettings_get(MOVEMENT, 'play-sound'), eyesight=lambda: h.gsettings_get(EYESIGHT, 'play-sound'))
    h.screenshot(f'screentime-breaks.{h.lang}')
    # Schermlimieten
    h.gsettings_set(LIMITS, 'daily-limit-enabled', 'false')
    h.sleep(0.5)
    flip(h, 'screentime', 'st-screentime-daily-limit-enabled', LIMITS, 'daily-limit-enabled',
         'Limiet voor schermtijd writes daily-limit-enabled')
    limit = number(h.gsettings_get(LIMITS, 'daily-limit-seconds')) or 28800.0
    try:
        spin_up(h, 'st-screentime-daily-limit-seconds-hours')
        wrote(h, 'screentime: Dagelijkse limiet + 1 uur writes daily-limit-seconds (0.5 s after the last change)',
              lambda: number(h.gsettings_get(LIMITS, 'daily-limit-seconds')) == limit + 3600,
              value=lambda: h.gsettings_get(LIMITS, 'daily-limit-seconds'))
    except LookupError as error:
        h.check('screentime: the hours box of Dagelijkse limiet', False, error=str(error))
    flip(h, 'screentime', 'st-screentime-grayscale', LIMITS, 'grayscale', 'Grijswaarden writes grayscale')
    h.screenshot(f'screentime-limits.{h.lang}')
    follows(h, 'screentime', 'st-screentime-daily-limit-enabled', LIMITS, 'daily-limit-enabled', 'false', False,
            'Limiet voor schermtijd')
    gray = node_of(h, 'st-screentime-grayscale')
    h.check('screentime: Grijswaarden is insensitive while the limit is off', bool(gray) and gray.get('sensitive') is False,
            node=gray and {k: gray.get(k) for k in ('active', 'sensitive')})
    h.gsettings_set(LIMITS, 'grayscale', 'false')


# ---------------------------------------------------------------------------------------------------- owned, mirror

BOUND = [
    (COLOR, 'night-light-enabled'), (COLOR, 'night-light-temperature'), (COLOR, 'night-light-schedule-automatic'),
    (COLOR, 'night-light-schedule-from'), (COLOR, 'night-light-schedule-to'), (POWER, 'ambient-enabled'),
    (PRIVACY, 'privacy-screen'), (TOUCHSCREEN, 'orientation-lock'), (WAYLAND, 'xwayland-scaling-factor'),
    (SOUND, 'allow-volume-above-100-percent'), (POWER, 'power-button-action'),
    (POWER, 'power-saver-profile-on-low-battery'), (INTERFACE, 'show-battery-percentage'),
    (PRIVACY, 'remove-old-trash-files'), (PRIVACY, 'remove-old-temp-files'), (PRIVACY, 'old-files-age'),
    (MUTTER, 'edge-tiling'), (MUTTER, 'dynamic-workspaces'), (MUTTER, 'workspaces-only-on-primary'),
    (WM, 'num-workspaces'), (INTERFACE, 'enable-hot-corners'), ('org.gnome.shell.app-switcher', 'current-workspace-only'),
    ('org.gnome.shell.window-switcher', 'current-workspace-only'), (LIMITS, 'history-enabled'),
    (LIMITS, 'daily-limit-enabled'), (LIMITS, 'daily-limit-seconds'), (LIMITS, 'grayscale'), (BREAKS, 'selected-breaks'),
    (MOVEMENT, 'duration-seconds'), (MOVEMENT, 'interval-seconds'), (MOVEMENT, 'play-sound'), (EYESIGHT, 'play-sound'),
]


def owned_keys(h):
    answer = h.backend('settings', 'gsettings.get', {'keys': [{'schema': s, 'key': k} for s, k in BOUND]}) or {}
    items = answer.get('items') or []
    h.record('owned.items', [{k: i.get(k) for k in ('schema', 'key', 'mode', 'setting', 'available', 'writable')}
                             for i in items])
    owned = [f"{i.get('schema')} {i.get('key')}" for i in items if i.get('mode') == 'owned']
    h.check('owned: no key of Systeem\'s pages is WinPoint-owned (no pointer card instead of a control)',
            len(items) == len(BOUND) and not owned, owned=owned, count=len(items))
    missing = [f"{i.get('schema')} {i.get('key')}" for i in items if i.get('available') is not True or i.get('writable') is not True]
    h.check('owned: every bound key is available and writable here (allow/system.rs)', not missing, missing=missing)
    battery = next((i for i in items if i.get('key') == 'show-battery-percentage'), {})
    h.check('owned: show-battery-percentage is Mirrored to taskbar.batteryPercent', battery.get('mode') == 'mirrored'
            and battery.get('setting') == 'taskbar.batteryPercent', item=battery)


def dnd_tile(h):
    """The Snelle instellingen Niet storen tile: its node (fo-tile with the label) and its main button."""
    label = tr(h, 'Niet storen', 'Do not disturb')
    for node in h.find('panel', cls='fo-tile'):
        nodes = h.hm.tree_nodes([{'tree': node}])
        if any(n.get('label') == label for n in nodes):
            main = next((n for n in nodes if 'fo-tile__main' in (n.get('classes') or [])), None)
            if main is not None:
                main['top'] = node.get('top', 0)
            return node, main
    return None, None


def mirror_dnd(h):
    """Plan "testing": DND in Settings changes show-banners and the shell flyout's state; the flyout's DND changes
    show-banners; an outside show-banners reaches settings.json and the Settings switch (B0's mirror, panel host)."""
    h.gsettings_set(NOTIFICATIONS, 'show-banners', 'true')
    page(h, 'notifications')
    wrote(h, 'mirror: show-banners true is notifications.dnd false',
          lambda: settings_json(h).get('notifications', {}).get('dnd') is not True)
    if not h.find('settings', 'st-notifications-show-banners'):
        h.check('mirror: Niet storen on Meldingen', False)
        return
    press(h, 'st-notifications-show-banners')
    wrote(h, 'mirror: Niet storen in Settings sets show-banners false',
          lambda: h.gsettings_get(NOTIFICATIONS, 'show-banners') == 'false'
          and settings_json(h).get('notifications', {}).get('dnd') is True,
          banners=lambda: h.gsettings_get(NOTIFICATIONS, 'show-banners'))
    h.open('panel')
    opened = h.native({'native': 'flyout', 'name': 'quick'})
    h.record('mirror.quick', opened)
    h.sleep(0.8)
    tile, main = dnd_tile(h)
    h.check('mirror: the Snelle instellingen tile Niet storen is on', bool(tile) and 'is-on' in (tile.get('classes') or []),
            classes=tile and tile.get('classes'))
    h.screenshot(f'mirror-quick.{h.lang}')
    if main is not None:
        try:
            click = h.click('panel', node=main)
        except LookupError as error:
            click = {'error': str(error)}
        h.record('mirror.tileClick', click)
        wrote(h, 'mirror: the tile in Snelle instellingen sets show-banners true again',
              lambda: h.gsettings_get(NOTIFICATIONS, 'show-banners') == 'true',
              banners=lambda: h.gsettings_get(NOTIFICATIONS, 'show-banners'), input=click.get('input'))
    else:
        h.check('mirror: the Niet storen tile\'s button', False)
    if dnd_tile(h)[0] is not None:
        h.native({'native': 'flyout', 'name': 'quick'})
    h.sleep(0.5)
    h.gsettings_set(NOTIFICATIONS, 'show-banners', 'false')
    wrote(h, 'mirror: an outside show-banners false reaches settings.json notifications.dnd',
          lambda: settings_json(h).get('notifications', {}).get('dnd') is True)
    wrote(h, 'mirror: the Settings switch Niet storen follows', lambda: switch_active(h, 'st-notifications-show-banners'))
    h.gsettings_set(NOTIFICATIONS, 'show-banners', 'true')


def run(h):
    audio = start_audio(h)
    state = display_page(h)
    bridge_ids(h)
    keep_question(h, state)
    nightlight_page(h)
    advanced_page(h)
    if audio:
        sound_pages(h)
    power_bare(h)
    kit_header_control(h)
    upower, profiles, sensors = start_hardware(h)
    power_page(h, upower, profiles)
    display_hardware(h)
    storage_pages(h)
    multitasking_page(h)
    screentime_page(h)
    owned_keys(h)
    mirror_dnd(h)
    for standin in (upower, profiles, sensors):
        if standin is not None:
            standin.stop()
    h.close('settings')
