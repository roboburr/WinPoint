"""Settings parity, batch A (notes/claude/settings-parity.md §7; plan.json "testing", GNOME HARNESS): the pages
Toegankelijkheid, Tekstgrootte, Visuele effecten, Contrastthema's, Muisaanwijzer en aanraken, Tekstcursor, Vergrootglas,
Verteller, Audio, Toetsenbord and Muis against GNOME's own keys in the run's private keyfile, the rendering pairs of
the mirror (host/src/settings_mirror/render.rs), the Toegankelijkheid tile of Snelle instellingen and Orca's
preferences through a stand-in of org.gnome.Orca.Service (amendment 36).

Run (from G), once per language:
    tools/heavy.sh --min-mb 2560 --tag a -- python3 -B tests/gnome/harness.py launch run --lang nl --source worktree \\
        --scenario tests/gnome/scenarios/settings-a11y.py
and the same with --lang en --out tests/gnome/scenarios/settings-a11y.en.out. (No page here reaches pkexec,
systemctl, passwd, nmcli or grdctl, so no --strace; guard-calls.log shows what ran.)

Every page as settings-input.py does it (its helpers, loaded from there): the screenshot <page>.<lang>, title and
breadcrumb, no Dutch word in English, every bound switch and check box flipped by a real pointer click with GNOME's
exact value in the keyfile and the old value written from outside moving it back, a last check that none was left,
and the modes of the keys the page binds (no Owned key; the Mirrored ones name their WinPoint setting).

What else it checks (language-independent checks; the screenshots carry the language):
  1. Toegankelijkheid: the Zicht, Gehoor and Interactie links, Vergrootglas and Verteller (needs gnome) exactly while
     caps.gnome; Toegankelijkheidsmenu altijd weergeven -> always-show-universal-access-status true; Snelle
     instellingen then shows the Toegankelijkheid tile, its page lists GNOME's functions and Plaktoetsen there writes
     stickykeys-enable true (and back); Animatie-effecten
  2. Tekstgrootte, the plan's risk "text scaling may double between GTK and the kit": text-scaling-factor 1.5 written
     from outside makes the page's text 1.5 times as high (GNOME's factor once, never 1.5 x 1.5) and the slider follow
     (150 %); a real click on the slider at 125 % and Toepassen write text-scaling-factor 1.25
  3. Visuele effecten: Schuifbalken altijd weergeven; Animatie-effecten off -> reduced-motion 'reduce' and
     enable-animations false, and the panel host's mirror brings accessibility.reduceMotion true to settings.json;
     reduced-motion written from outside moves the switch back; Vormen voor aan/uit-status -> show-status-shapes true
     and the switches show their I and O images; Transparantie-effecten -> settings.json theme.transparency
  4. Contrastthema's: Hoog contrast + Toepassen -> high-contrast true, settings.json accessibility.highContrast true
     (the mirror), the Settings window's w11-hc and the shell's w11-high-contrast; Geen + Toepassen back; an outside
     write moves the drop-down; Transparantie uitzetten
  5. Muisaanwijzer en aanraken: Grootte at step 3 -> cursor-size 48; a size written from outside shows (96 px);
     Muisaanwijzerindicator; the Aanraken rows exactly while caps has a touchscreen
  6. Tekstcursor: Knipperende tekstcursor off -> cursor-blink false and the speed slider insensitive; Knippersnelheid
     at 500 ms by a real click
  7. Vergrootglas: the switch on its expander header -> screen-magnifier-enabled true (then off from outside, so no
     later click lands in a zoomed screen); + -> mag-factor 2.25 and 225%; mag-factor 3.0 from outside shows 300%;
     Weergave Lens -> lens-mode true with screen-position top-half, Volledig scherm -> lens-mode false and full-screen;
     every box and switch (edges, tracking, crosshairs); Helderheid writes its three keys together; Kleur, Dikte,
     Dekking; a shortcut row opens Sneltoetsen at Toegankelijkheid
  8. Verteller: Instellingen voor Verteller openen is insensitive while the screen reader is off; the switch ->
     screen-reader-enabled true; the button then calls ShowPreferences on the Orca stand-in, once, without
     interactive authorization, and no orca program is started by WinPoint
  9. Audio: Het hele scherm laten knipperen -> visual-bell true and visual-bell-type fullscreen-flash, Het actieve
     venster -> frame-flash, Uit -> visual-bell false; Testen; Overversterking
 10. Toetsenbord: every switch and box of Plaktoetsen, Filtertoetsen and Wisseltoetsen; Sneltoets voor Plaktoetsen ->
     enable true, and the same row in Filtertoetsen follows (amendment 24); amendment 14's note; Filtertoetsen' switch
     -> slowkeys-enable true, off -> slowkeys-enable and bouncekeys-enable false; the acceptance delays; Print Screen
     off -> show-screenshot-ui [] and on -> Print; Schermtoetsenbord; Meldingsvoorkeuren
 11. Muis: Muistoetsen with its two sliders; Een venster activeren ... -> focus-mode sloppy; Dubbelkliksnelheid;
     Gesimuleerde secundaire klik and Klikken door te zweven with their delays and threshold
"""

import importlib.util
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from lib.standins import start as start_standin  # noqa: E402
from lib.standins.a import SPECS  # noqa: E402

_SPEC = importlib.util.spec_from_file_location('settings_input', Path(__file__).with_name('settings-input.py'))
A = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(A)
until, node, click, gv, wait_key, backend_value = A.until, A.node, A.click, A.gv, A.wait_key, A.backend_value
no_dutch, page_open, reveal, open_expander, flip = A.no_dutch, A.page_open, A.reveal, A.open_expander, A.flip
slider_click, slide, choose, shows, unflipped, modes = A.slider_click, A.slide, A.choose, A.shows, A.unflipped, A.modes

IFACE = 'org.gnome.desktop.interface'
A11Y = 'org.gnome.desktop.a11y'
A11Y_IFACE = 'org.gnome.desktop.a11y.interface'
A11Y_KEYBOARD = 'org.gnome.desktop.a11y.keyboard'
A11Y_APPS = 'org.gnome.desktop.a11y.applications'
MAG = 'org.gnome.desktop.a11y.magnifier'
A11Y_MOUSE = 'org.gnome.desktop.a11y.mouse'
WM_PREFS = 'org.gnome.desktop.wm.preferences'
SOUND = 'org.gnome.desktop.sound'
MEDIA = 'org.gnome.settings-daemon.plugins.media-keys'
SHELL_KEYS = 'org.gnome.shell.keybindings'
MOUSE = 'org.gnome.desktop.peripherals.mouse'


def settings_json(h):
    try:
        return json.loads((h.hm.RUN / 'config' / 'win11-gnome' / 'settings.json').read_text() or '{}')
    except (OSError, ValueError):
        return {}


def setting(h, path):
    value = settings_json(h)
    for part in path.split('.'):
        value = value.get(part) if isinstance(value, dict) else None
    return value


def wait_setting(h, path, want, label, timeout=10):
    seen = until(h, lambda: setting(h, path) == want or None, label, timeout)
    h.check(label, bool(seen), want=want, value=setting(h, path))


def flip_setting(h, name, path, label, tested):
    """A switch bound to a WinPoint setting (settings.json): a real click flips it, a second one puts it back."""
    tested.add(name)
    before = setting(h, path)
    if before is None:
        before = backend_setting(h, path)
    n = reveal(h, name)
    if not n:
        h.check(f'{label}: the switch is on the page', False)
        return
    click(h, n=n)
    wait_setting(h, path, not before, f'{label}: a click writes settings.json {path} {str(not before).lower()}')
    click(h, n=reveal(h, name))
    wait_setting(h, path, before, f'{label}: a second click puts {path} back')


def backend_setting(h, path):
    value = h.backend('settings', 'settings.get', {}) or {}
    for part in path.split('.'):
        value = value.get(part) if isinstance(value, dict) else None
    return value


def active(h, s, name):
    found = node(h, s, name)
    return found.get('active') if found else None


def shell_node(h, name):
    found = h.find('panel', name) if 'panel' in h.surfaces else []
    return found[0] if found else None


def run(h):
    en = h.lang == 'en'
    t = (lambda nl_text, en_text: en_text if en else nl_text)
    A.INPUTS.clear()
    A.private_stubs(h)

    # ---- 1. Toegankelijkheid and the quick settings tile
    page_open(h, 'accessibility', 'st-accessibility-reduced-motion')
    caps = h.backend('settings', 'caps.get') or {}
    h.record('caps', caps)
    for name in ('link-textsize', 'link-visualeffects', 'link-mousepointer', 'link-textcursor', 'link-contrast',
                 'link-a11y-audio', 'link-a11y-keyboard', 'link-a11y-mouse',
                 'st-accessibility-reduced-motion', 'st-accessibility-always-show-universal-access-status'):
        h.check(f'Toegankelijkheid has {name}', node(h, 'settings', name) is not None)
    for page in ('magnifier', 'narrator'):
        h.check(f'Toegankelijkheid links to {page} (needs gnome) exactly while caps.gnome',
                (node(h, 'settings', f'link-{page}') is not None) == (caps.get('gnome') is True), gnome=caps.get('gnome'))
    tested = {'st-accessibility-always-show-universal-access-status'}
    click(h, 'st-accessibility-always-show-universal-access-status')
    wait_key(h, A11Y, 'always-show-universal-access-status', 'true',
             'Toegankelijkheidsmenu altijd weergeven writes always-show-universal-access-status true')
    no_dutch(h, en, 'Toegankelijkheid')
    opened = h.native({'native': 'flyout', 'name': 'quick'})
    h.check('Snelle instellingen opens', isinstance(opened, dict) and opened.get('open') == 'quick', result=opened)
    tile = until(h, lambda: shell_node(h, 'qs-tile-accessibility'), 'the Toegankelijkheid tile', 10)
    h.check('Snelle instellingen shows the Toegankelijkheid tile while the menu is on', tile is not None)
    if tile:
        h.native({'native': 'click', 'name': 'qs-tile-accessibility-main'})
        row = until(h, lambda: shell_node(h, 'qs-a11y-stickykeys-enable'), 'the accessibility page', 10)
        h.check('its page lists GNOME\'s functions (Plaktoetsen)', row is not None)
        for key in ('screen-magnifier-enabled', 'high-contrast', 'screen-reader-enabled', 'mousekeys-enable'):
            h.check(f'... and {key}', shell_node(h, f'qs-a11y-{key}') is not None)
        h.screenshot(f'quick-accessibility.{h.lang}', settle=0.8)
        if row:
            h.native({'native': 'click', 'name': 'qs-a11y-stickykeys-enable'})
            wait_key(h, A11Y_KEYBOARD, 'stickykeys-enable', 'true', 'Plaktoetsen in Snelle instellingen writes stickykeys-enable true')
            h.native({'native': 'click', 'name': 'qs-a11y-stickykeys-enable'})
            wait_key(h, A11Y_KEYBOARD, 'stickykeys-enable', 'false', '... and off again')
    h.native({'native': 'flyout', 'name': 'quick'})
    click(h, 'st-accessibility-always-show-universal-access-status')
    wait_key(h, A11Y, 'always-show-universal-access-status', 'false', 'the menu off again')
    h.gsettings_set(A11Y, 'always-show-universal-access-status', 'true')
    seen = until(h, lambda: active(h, 'settings', 'st-accessibility-always-show-universal-access-status') is True or None,
                 'the menu switch follows', 8)
    h.check('always-show-universal-access-status true written from outside moves its switch (gsettings.changed)', bool(seen))
    h.gsettings_set(A11Y, 'always-show-universal-access-status', 'false')
    until(h, lambda: active(h, 'settings', 'st-accessibility-always-show-universal-access-status') is False or None, 'off', 5)
    flip(h, 'st-accessibility-reduced-motion', A11Y_IFACE, 'reduced-motion', pair=("'no-preference'", "'reduce'"),
         label='Animatie-effecten (Toegankelijkheid)', tested=tested)
    wait_key(h, IFACE, 'enable-animations', 'true', '... and the mirror makes enable-animations agree again')
    unflipped(h, 'accessibility', tested)
    modes(h, 'Toegankelijkheid', [(A11Y, 'always-show-universal-access-status'), (A11Y_IFACE, 'reduced-motion'),
                                  (IFACE, 'enable-animations')],
          mirrored={(A11Y_IFACE, 'reduced-motion'): 'accessibility.reduceMotion',
                    (IFACE, 'enable-animations'): 'accessibility.reduceMotion'})

    # ---- 2. Tekstgrootte: GNOME's factor once
    page_open(h, 'textsize', 'st-textsize-value', shot=False)
    h.sleep(0.6)
    h.screenshot(f'textsize.{h.lang}', surface='settings')
    shown = node(h, 'settings', 'st-textsize-value')
    h.check('Tekstgrootte shows 100 %', bool(shown) and shown.get('label') == '100%', node=shown)
    # the card's title: a label at its natural height (in the card's vertical text box), in the kit's px ramp
    title = t('Tekstgrootte', 'Text size')
    before = (h.find('settings', label=title, cls='w11-sc-title') or [None])[0]
    h.gsettings_set(IFACE, 'text-scaling-factor', '1.5')
    after = until(h, lambda: (lambda n: n if n and n.get('label') == '150%' else None)(node(h, 'settings', 'st-textsize-value')),
                  'the slider follows 1.5', 10)
    h.check('text-scaling-factor 1.5 from outside: the page follows (150%)', after is not None, node=after)
    h.sleep(1.0)
    after = (h.find('settings', label=title, cls='w11-sc-title') or [None])[0]
    if before and after and before.get('rect') and after.get('rect') and before['rect'][3]:
        ratio = after['rect'][3] / before['rect'][3]
        h.record('text height 100% -> 150%', [before['rect'][3], after['rect'][3], round(ratio, 3)])
        h.check('the kit scales its text by GNOME\'s factor once (x1.5, never x2.25)', 1.3 <= ratio <= 1.7, ratio=ratio)
    else:
        h.check('the card title has a size before and after', False, before=before, after=after)
    h.screenshot(f'textsize-150.{h.lang}', surface='settings')
    h.gsettings_set(IFACE, 'text-scaling-factor', '1.0')
    until(h, lambda: (lambda n: n if n and n.get('label') == '100%' else None)(node(h, 'settings', 'st-textsize-value')),
          'back at 100 %', 10)
    h.sleep(1.0)
    if slider_click(h, 'st-textsize-text-scaling-factor', (125 - 100) / (225 - 100)):
        shown = node(h, 'settings', 'st-textsize-value')
        h.record('text size after the click at 125', shown.get('label') if shown else None)
        click(h, n=reveal(h, 'st-textsize-apply'))
        h.sleep(1.0)
        value = h.gsettings_get(IFACE, 'text-scaling-factor')
        h.record('text-scaling-factor after Toepassen', value)
        try:
            factor = float(value)
        except (TypeError, ValueError):
            factor = None
        h.check('Tekstgrootte at 125 % and Toepassen write text-scaling-factor 1.25',
                factor is not None and abs(factor - 1.25) < 0.021, value=value,
                exact=factor is not None and abs(factor - 1.25) < 1e-9)
    no_dutch(h, en, 'Tekstgrootte')
    h.gsettings_set(IFACE, 'text-scaling-factor', '1.0')
    modes(h, 'Tekstgrootte', [(IFACE, 'text-scaling-factor')],
          mirrored={(IFACE, 'text-scaling-factor'): 'accessibility.textScale'})

    # ---- 3. Visuele effecten
    page_open(h, 'visualeffects', 'st-visualeffects-overlay-scrolling')
    tested = set()
    flip(h, 'st-visualeffects-overlay-scrolling', IFACE, 'overlay-scrolling', label='Schuifbalken altijd weergeven',
         tested=tested)
    tested.add('st-visualeffects-reduced-motion')
    click(h, n=reveal(h, 'st-visualeffects-reduced-motion'))
    wait_key(h, A11Y_IFACE, 'reduced-motion', "'reduce'", 'Animatie-effecten off writes reduced-motion reduce')
    wait_key(h, IFACE, 'enable-animations', 'false', '... and enable-animations false')
    wait_setting(h, 'accessibility.reduceMotion', True, 'the mirror brings accessibility.reduceMotion true')
    h.gsettings_set(A11Y_IFACE, 'reduced-motion', "'no-preference'")
    seen = until(h, lambda: active(h, 'settings', 'st-visualeffects-reduced-motion') is True or None,
                 'the switch follows reduced-motion', 10)
    h.check('reduced-motion written from outside moves Animatie-effecten (gsettings.changed)', bool(seen))
    wait_setting(h, 'accessibility.reduceMotion', False, '... and the mirror brings accessibility.reduceMotion false')
    wait_key(h, IFACE, 'enable-animations', 'true', '... and makes enable-animations agree')
    tested.add('st-visualeffects-show-status-shapes')
    click(h, n=reveal(h, 'st-visualeffects-show-status-shapes'))
    wait_key(h, A11Y_IFACE, 'show-status-shapes', 'true', 'Vormen voor aan/uit-status writes show-status-shapes true')
    h.sleep(1.0)
    switch = node(h, 'settings', 'st-visualeffects-show-status-shapes') or {}
    images = [c for c in switch.get('children') or [] if c.get('type') == 'GtkImage']
    h.check('the kit\'s switches show GtkSwitch\'s I and O shapes', len(images) >= 1, images=len(images))
    h.screenshot(f'visualeffects-shapes.{h.lang}', surface='settings')
    h.gsettings_set(A11Y_IFACE, 'show-status-shapes', 'false')
    seen = until(h, lambda: active(h, 'settings', 'st-visualeffects-show-status-shapes') is False or None, 'shapes off', 8)
    h.check('show-status-shapes false written from outside moves its switch back', bool(seen))
    flip_setting(h, 'st-visualeffects-transparency', 'theme.transparency', 'Transparantie-effecten', tested)
    unflipped(h, 'visualeffects', tested)
    modes(h, 'Visuele effecten', [(IFACE, 'overlay-scrolling'), (A11Y_IFACE, 'reduced-motion'), (IFACE, 'enable-animations'),
                                  (A11Y_IFACE, 'show-status-shapes')])
    no_dutch(h, en, 'Visuele effecten')

    # ---- 4. Contrastthema's
    page_open(h, 'contrast', 'st-contrast-high-contrast')
    tested = set()
    choose(h, 'settings', 'st-contrast-high-contrast', t('Hoog contrast', 'High contrast'))
    h.check('Hoog contrast is not written before Toepassen', h.gsettings_get(A11Y_IFACE, 'high-contrast') in (None, 'false'))
    click(h, n=reveal(h, 'st-contrast-apply'))
    wait_key(h, A11Y_IFACE, 'high-contrast', 'true', 'Hoog contrast and Toepassen write high-contrast true')
    wait_setting(h, 'accessibility.highContrast', True, 'the mirror brings accessibility.highContrast true')
    seen = until(h, lambda: h.find('settings', cls='w11-hc') or None, 'the Settings window in high contrast', 10)
    h.check('the Settings window gets w11-hc', bool(seen))
    shell_hc = until(h, lambda: h.find('panel', cls='w11-high-contrast') or None, 'the shell in high contrast', 10) \
        if 'panel' in h.surfaces else None
    h.check('the shell gets w11-high-contrast', bool(shell_hc))
    h.screenshot(f'contrast-high.{h.lang}', surface='settings')
    choose(h, 'settings', 'st-contrast-high-contrast', t('Geen', 'None'))
    click(h, n=reveal(h, 'st-contrast-apply'))
    wait_key(h, A11Y_IFACE, 'high-contrast', 'false', 'Geen and Toepassen write high-contrast false')
    wait_setting(h, 'accessibility.highContrast', False, '... and the mirror brings it back to settings.json')
    h.gsettings_set(A11Y_IFACE, 'high-contrast', 'true')
    seen = until(h, lambda: shows(h, 'st-contrast-high-contrast', t('Hoog contrast', 'High contrast')) or None,
                 'the drop-down follows', 10)
    h.check('high-contrast true written from outside shows Hoog contrast', bool(seen))
    h.gsettings_set(A11Y_IFACE, 'high-contrast', 'false')
    seen = until(h, lambda: shows(h, 'st-contrast-high-contrast', t('Geen', 'None')) or None, 'Geen again', 10)
    h.check('... and false shows Geen', bool(seen))
    until(h, lambda: not h.find('settings', cls='w11-hc') or None, 'out of high contrast', 10)
    flip_setting(h, 'st-contrast-transparency', 'theme.transparency', 'Transparantie uitzetten', tested)
    unflipped(h, 'contrast', tested)
    modes(h, 'Contrastthema\'s', [(A11Y_IFACE, 'high-contrast'), (MEDIA, 'toggle-contrast')],
          mirrored={(A11Y_IFACE, 'high-contrast'): 'accessibility.highContrast'})
    no_dutch(h, en, 'Contrastthema\'s')

    # ---- 5. Muisaanwijzer en aanraken
    page_open(h, 'mousepointer', 'st-mousepointer-cursor-size')
    tested = set()
    # the size label shows the step before the key is read; a second click only where the first one missed (found by
    # slice 3's first runs: the page was built again under the pointer when a touchscreen came and went in caps, and
    # the size label's width moved the slider under the pointer; both fixed in a11y_vision.rs)
    labels = []
    for _ in range(2):
        if not slider_click(h, 'st-mousepointer-cursor-size', (3 - 1) / (5 - 1)):
            break
        shown = node(h, 'settings', 'st-mousepointer-size-value')
        labels.append(shown.get('label') if shown else None)
        if labels[-1] == '48 px':
            break
    h.record('size label after each click', labels)
    h.screenshot(f'mousepointer-size.{h.lang}', surface='settings')
    wait_key(h, IFACE, 'cursor-size', '48', 'Grootte at step 3 writes cursor-size 48')
    h.gsettings_set(IFACE, 'cursor-size', '96')
    seen = until(h, lambda: (lambda n: n if n and n.get('label') == '96 px' else None)(node(h, 'settings', 'st-mousepointer-size-value')),
                 'the size follows', 10)
    h.check('cursor-size written from outside shows (96 px)', seen is not None)
    h.gsettings_set(IFACE, 'cursor-size', '24')
    flip(h, 'st-mousepointer-locate-pointer', IFACE, 'locate-pointer', label='Muisaanwijzerindicator', tested=tested)
    # Aanraken: pen.rs' touchscreen rows exactly while caps has a touchscreen (the harness reads the host's udev data)
    caps = h.backend('settings', 'caps.get') or {}
    has_touch = caps.get('touchscreen') is True
    seen = until(h, lambda: (node(h, 'settings', 'st-mousepointer-output') is not None) == has_touch or None,
                 'the Aanraken rows follow caps', 5)
    h.record('touchscreen in caps', has_touch)
    h.check('Aanraakscherm toewijzen aan beeldscherm is there exactly while caps has a touchscreen', bool(seen),
            touchscreen=has_touch)
    unflipped(h, 'mousepointer', tested)
    modes(h, 'Muisaanwijzer en aanraken', [(IFACE, 'cursor-size'), (IFACE, 'locate-pointer')])
    no_dutch(h, en, 'Muisaanwijzer en aanraken')

    # ---- 6. Tekstcursor
    page_open(h, 'textcursor', 'st-textcursor-cursor-blink')
    tested = {'st-textcursor-cursor-blink'}
    click(h, n=reveal(h, 'st-textcursor-cursor-blink'))
    wait_key(h, IFACE, 'cursor-blink', 'false', 'Knipperende tekstcursor off writes cursor-blink false')
    slider = until(h, lambda: (lambda n: n if n and n.get('sensitive') is False else None)(node(h, 'settings', 'st-textcursor-cursor-blink-time')),
                   'the speed slider insensitive', 5)
    h.check('Knippersnelheid is insensitive while the cursor does not blink', slider is not None)
    h.gsettings_set(IFACE, 'cursor-blink', 'true')
    seen = until(h, lambda: active(h, 'settings', 'st-textcursor-cursor-blink') is True or None, 'blink follows', 8)
    h.check('cursor-blink true written from outside turns its switch on again', bool(seen))
    h.sleep(0.6)
    if slider_click(h, 'st-textcursor-cursor-blink-time', (2100 - 100) / (2500 - 100)):
        value = h.gsettings_get(IFACE, 'cursor-blink-time')
        h.record('cursor-blink-time after the click', value)
        try:
            ms = int(value)
        except (TypeError, ValueError):
            ms = None
        h.check('Knippersnelheid near 500 ms by a click at its place (Snel is right: 2600 - position)',
                ms is not None and abs(ms - 500) <= 60, value=value, exact=ms == 500)
    unflipped(h, 'textcursor', tested)
    modes(h, 'Tekstcursor', [(IFACE, 'cursor-blink'), (IFACE, 'cursor-blink-time')])
    no_dutch(h, en, 'Tekstcursor')

    run_magnifier(h, en, t)
    run_interaction(h, en, t)
    h.record('click inputs', dict(A.INPUTS))
    h.close('settings')


def run_magnifier(h, en, t):
    # ---- 7. Vergrootglas
    page_open(h, 'magnifier', 'st-magnifier-screen-magnifier-enabled', shot=False)
    h.sleep(0.6)
    h.screenshot(f'magnifier.{h.lang}', surface='settings')
    tested = {'st-magnifier-screen-magnifier-enabled'}
    click(h, n=reveal(h, 'st-magnifier-screen-magnifier-enabled'))
    wait_key(h, A11Y_APPS, 'screen-magnifier-enabled', 'true', 'the Vergrootglas switch on its header writes screen-magnifier-enabled true')
    # off from outside at once: every later real click would land in a magnified screen
    h.gsettings_set(A11Y_APPS, 'screen-magnifier-enabled', 'false')
    seen = until(h, lambda: active(h, 'settings', 'st-magnifier-screen-magnifier-enabled') is False or None, 'the switch follows', 10)
    h.check('screen-magnifier-enabled written from outside moves the switch back (gsettings.changed)', bool(seen))
    h.sleep(1.0)
    click(h, n=reveal(h, 'st-magnifier-zoom-in'))
    wait_key(h, MAG, 'mag-factor', '2.25', '+ writes mag-factor 2.25 (GNOME Settings\' quarter steps)')
    shown = node(h, 'settings', 'st-magnifier-mag-factor')
    h.check('... and shows 225%', bool(shown) and shown.get('label') == '225%', node=shown)
    h.gsettings_set(MAG, 'mag-factor', '3.0')
    seen = until(h, lambda: (lambda n: n if n and n.get('label') == '300%' else None)(node(h, 'settings', 'st-magnifier-mag-factor')),
                 'the zoom level follows', 10)
    h.check('mag-factor 3.0 written from outside shows 300%', seen is not None)
    click(h, n=reveal(h, 'st-magnifier-zoom-out'))
    wait_key(h, MAG, 'mag-factor', '2.75', '- writes mag-factor 2.75')
    choose(h, 'settings', 'st-magnifier-view', t('Lens', 'Lens'))
    wait_key(h, MAG, 'lens-mode', 'true', 'Weergave Lens writes lens-mode true')
    wait_key(h, MAG, 'screen-position', "'top-half'", '... with screen-position top-half (a lens over the full screen does nothing)')
    area = node(h, 'settings', 'st-magnifier-screen-position') or {}
    h.check('Schermgebied can be chosen for the lens', area.get('sensitive') is True)
    choose(h, 'settings', 'st-magnifier-view', t('Volledig scherm', 'Full screen'))
    wait_key(h, MAG, 'lens-mode', 'false', 'Volledig scherm writes lens-mode false')
    wait_key(h, MAG, 'screen-position', "'full-screen'", '... and screen-position full-screen')
    flip(h, 'st-magnifier-invert-lightness', MAG, 'invert-lightness', label='Kleuren omkeren', tested=tested)
    flip(h, 'st-magnifier-scroll-at-edges', MAG, 'scroll-at-edges', label='Buiten de schermranden doorgaan', tested=tested)
    open_expander(h, 'st-magnifier-follow')
    flip(h, 'st-magnifier-mouse-tracking-follow', MAG, 'mouse-tracking', label='Vergrootglas laten volgen: Muisaanwijzer',
         expander='st-magnifier-follow', tested=tested)
    flip(h, 'st-magnifier-focus-tracking', MAG, 'focus-tracking', pair=("'proportional'", "'none'"),
         label='Vergrootglas laten volgen: Toetsenbordfocus', expander='st-magnifier-follow', tested=tested)
    flip(h, 'st-magnifier-caret-tracking', MAG, 'caret-tracking', pair=("'centered'", "'none'"),
         label='Vergrootglas laten volgen: Tekstcursor', expander='st-magnifier-follow', tested=tested)
    open_expander(h, 'st-magnifier-keep-pointer')
    # Radio buttons now have stable names too: exercise every choice rather than only the old anonymous Push row.
    for value in ('centered', 'proportional', 'push'):
        name = f'st-magnifier-mouse-tracking-{value}'
        option = reveal(h, name, 'st-magnifier-keep-pointer')
        if option:
            click(h, n=option)
            wait_key(h, MAG, 'mouse-tracking', f"'{value}'", f'Pointer tracking {value} writes its GNOME value')
            tested.add(name)
        else:
            h.check(f'Pointer tracking offers {value}', False)
    h.screenshot(f'magnifier-follow.{h.lang}', surface='settings')
    open_expander(h, 'st-magnifier-colour')
    if slider_click(h, 'st-magnifier-brightness', (0.4 + 0.8) / 1.6, expander='st-magnifier-colour'):
        values = [h.gsettings_get(MAG, f'brightness-{c}') for c in ('red', 'green', 'blue')]
        try:
            nums = [float(v) for v in values]
        except (TypeError, ValueError):
            nums = []
        h.check('Helderheid writes brightness-red, -green and -blue together (near 0.4)',
                len(nums) == 3 and len(set(nums)) == 1 and abs(nums[0] - 0.4) <= 0.05, values=values)
    slide(h, 'st-magnifier-color-saturation', MAG, 'color-saturation', 0, 1, 0.01, 0.4, lambda n: n, 0.04,
          label='Kleur (verzadiging)', expander='st-magnifier-colour')
    h.screenshot(f'magnifier-colour.{h.lang}', surface='settings')
    tested.add('st-magnifier-show-cross-hairs')
    click(h, n=reveal(h, 'st-magnifier-show-cross-hairs'))
    wait_key(h, MAG, 'show-cross-hairs', 'true', 'the Dradenkruis switch on its header writes show-cross-hairs true')
    open_expander(h, 'st-magnifier-show-cross-hairs-expander')
    flip(h, 'st-magnifier-cross-hairs-clip', MAG, 'cross-hairs-clip', label='Over muisaanwijzer heen',
         expander='st-magnifier-show-cross-hairs-expander', tested=tested)
    slide(h, 'st-magnifier-cross-hairs-thickness', MAG, 'cross-hairs-thickness', 1, 100, 1, 26, lambda n: n, 3,
          label='Dikte', expander='st-magnifier-show-cross-hairs-expander',
          value='st-magnifier-cross-hairs-thickness-value', outside='12')
    slide(h, 'st-magnifier-cross-hairs-opacity', MAG, 'cross-hairs-opacity', 0, 100, 1, 30, lambda n: n / 100, 0.03,
          label='Dekking', expander='st-magnifier-show-cross-hairs-expander',
          value='st-magnifier-cross-hairs-opacity-value', outside='0.9')
    h.screenshot(f'magnifier-set.{h.lang}', surface='settings')
    h.gsettings_set(MAG, 'show-cross-hairs', 'false')
    seen = until(h, lambda: active(h, 'settings', 'st-magnifier-show-cross-hairs') is False or None, 'crosshairs off', 8)
    h.check('show-cross-hairs false written from outside moves the Dradenkruis switch back', bool(seen))
    unflipped(h, 'magnifier', tested)
    modes(h, 'Vergrootglas', [(A11Y_APPS, 'screen-magnifier-enabled')] + [(MAG, k) for k in (
        'mag-factor', 'screen-position', 'lens-mode', 'scroll-at-edges', 'mouse-tracking', 'focus-tracking', 'caret-tracking',
        'invert-lightness', 'brightness-red', 'contrast-red', 'color-saturation', 'show-cross-hairs', 'cross-hairs-clip',
        'cross-hairs-thickness', 'cross-hairs-length', 'cross-hairs-color', 'cross-hairs-opacity')])
    no_dutch(h, en, 'Vergrootglas')
    # a shortcut row opens Sneltoetsen at Toegankelijkheid, where it is set
    row = reveal(h, 'st-magnifier-magnifier-zoom-in', 'st-magnifier-screen-magnifier-enabled-expander')
    if row:
        h.record('the shortcut row clicked', {k: row.get(k) for k in ('name', 'rect', 'label')})
        click(h, n=row)
        seen = until(h, lambda: node(h, 'settings', 'st-shortcuts-section-accessibility'), 'Sneltoetsen', 10)
        h.screenshot(f'magnifier-shortcut.{h.lang}', surface='settings')
        opened = [n for n in h.find('settings', cls='w11-expanded')
                  if 'st-shortcuts-section-accessibility' in json.dumps(n)]
        h.check('a shortcut row of Vergrootglas opens Sneltoetsen with Toegankelijkheid open', bool(seen) and bool(opened))
        h.surface('settings').page = 'shortcuts'
    else:
        h.check('Vergrootglas shows its shortcut rows', False)

    # ---- 8. Verteller: Orca's preferences through D-Bus (amendment 36)
    orca = start_standin(h, SPECS['orca'], 'orca')
    page_open(h, 'narrator', 'st-narrator-preferences', shot=False)
    h.sleep(0.6)
    h.screenshot(f'narrator.{h.lang}', surface='settings')
    tested = {'st-narrator-screen-reader-enabled'}
    button = node(h, 'settings', 'st-narrator-preferences')
    h.check('Instellingen voor Verteller openen is insensitive while the screen reader is off',
            bool(button) and button.get('sensitive') is False, node=button)
    click(h, n=reveal(h, 'st-narrator-screen-reader-enabled'))
    wait_key(h, A11Y_APPS, 'screen-reader-enabled', 'true', 'the Verteller switch writes screen-reader-enabled true')
    caps = h.backend('settings', 'caps.get') or {}
    h.record('orca in caps', caps.get('orca'))
    if caps.get('orca') is True:
        until(h, lambda: (lambda n: n if n and n.get('sensitive') else None)(node(h, 'settings', 'st-narrator-preferences')),
              'the button sensitive', 5)
        click(h, n=reveal(h, 'st-narrator-preferences'))
        calls = until(h, lambda: [c for c in orca.calls() if c.get('method') == 'ShowPreferences'] or None
                      if orca else None, 'ShowPreferences on Orca', 10) or []
        h.record('orca calls', orca.calls() if orca else None)
        h.check('Instellingen voor Verteller openen calls ShowPreferences on the running Orca, once', len(calls) == 1, calls=calls)
        h.check('... a plain call, without interactive authorization', all(c.get('interactive') is False for c in calls))
        h.check('... and WinPoint starts no orca program itself', not A.guard_calls(h, 'orca'), orca=A.guard_calls(h, 'orca'))
    else:
        h.check('caps has orca (the harness installs the stub)', False, caps=caps.get('orca'))
    h.gsettings_set(A11Y_APPS, 'screen-reader-enabled', 'false')
    seen = until(h, lambda: active(h, 'settings', 'st-narrator-screen-reader-enabled') is False or None, 'reader off', 8)
    h.check('screen-reader-enabled false written from outside moves the Verteller switch back', bool(seen))
    seen = until(h, lambda: (lambda n: n if n and n.get('sensitive') is False else None)(node(h, 'settings', 'st-narrator-preferences')),
                 'the button insensitive again', 5)
    h.check('... and Instellingen voor Verteller openen is insensitive again', bool(seen))
    if orca:
        orca.stop()
    unflipped(h, 'narrator', tested)
    modes(h, 'Verteller', [(A11Y_APPS, 'screen-reader-enabled'), (MEDIA, 'screenreader')])
    no_dutch(h, en, 'Verteller')


def run_interaction(h, en, t):
    # ---- 9. Audio
    page_open(h, 'a11y-audio', 'st-a11y-audio-visual-bell')
    tested = set()
    choose(h, 'settings', 'st-a11y-audio-visual-bell', t('Het hele scherm laten knipperen', 'Flash the entire screen'))
    wait_key(h, WM_PREFS, 'visual-bell', 'true', 'Het hele scherm laten knipperen writes visual-bell true')
    wait_key(h, WM_PREFS, 'visual-bell-type', "'fullscreen-flash'", '... and visual-bell-type fullscreen-flash')
    choose(h, 'settings', 'st-a11y-audio-visual-bell', t('Het actieve venster laten knipperen', 'Flash the active window'))
    wait_key(h, WM_PREFS, 'visual-bell-type', "'frame-flash'", 'Het actieve venster laten knipperen writes frame-flash')
    click(h, n=reveal(h, 'st-a11y-audio-test'))
    h.sleep(0.8)
    h.check('Testen leaves the page running', node(h, 'settings', 'st-a11y-audio-test') is not None)
    choose(h, 'settings', 'st-a11y-audio-visual-bell', t('Uit', 'Off'))
    wait_key(h, WM_PREFS, 'visual-bell', 'false', 'Uit writes visual-bell false')
    h.gsettings_set(WM_PREFS, 'visual-bell', 'true')
    seen = until(h, lambda: shows(h, 'st-a11y-audio-visual-bell', t('Het actieve venster laten knipperen',
                                                                     'Flash the active window')) or None, 'the bell follows', 8)
    h.check('visual-bell true written from outside shows the window flash (frame-flash)', bool(seen))
    h.gsettings_set(WM_PREFS, 'visual-bell', 'false')
    flip(h, 'st-a11y-audio-allow-volume-above-100-percent', SOUND, 'allow-volume-above-100-percent', label='Overversterking',
         tested=tested)
    unflipped(h, 'a11y-audio', tested)
    modes(h, 'Audio', [(WM_PREFS, 'visual-bell'), (WM_PREFS, 'visual-bell-type'), (SOUND, 'allow-volume-above-100-percent')])
    no_dutch(h, en, 'Audio')

    # ---- 10. Toetsenbord
    page_open(h, 'a11y-keyboard', 'st-a11y-keyboard-stickykeys-enable')
    tested = set()
    sticky = 'st-a11y-keyboard-stickykeys-enable-expander'
    flip(h, 'st-a11y-keyboard-stickykeys-enable', A11Y_KEYBOARD, 'stickykeys-enable', label='Plaktoetsen', tested=tested)
    open_expander(h, sticky)
    h.check('amendment 14\'s note is in Plaktoetsen', bool(h.find('settings', label=t(
        'Met Plaktoetsen aan opent de Windows-toets Start niet', 'With Sticky Keys on, the Windows key doesn’t open Start'))))
    for key, label in (('stickykeys-two-key-off', 'Plaktoetsen uitschakelen wanneer twee toetsen tegelijk worden ingedrukt'),
                       ('stickykeys-modifier-beep', 'Plaktoetsen: een geluid bij de sneltoetsen')):
        flip(h, f'st-a11y-keyboard-{key}', A11Y_KEYBOARD, key, label=label, expander=sticky, tested=tested)
    tested.add('st-a11y-keyboard-enable-sticky')
    click(h, n=reveal(h, 'st-a11y-keyboard-enable-sticky', sticky))
    wait_key(h, A11Y_KEYBOARD, 'enable', 'true', 'Sneltoets voor Plaktoetsen writes enable true')
    h.screenshot(f'a11y-keyboard-sticky.{h.lang}', surface='settings')
    click(h, n=reveal(h, sticky))
    h.sleep(0.5)
    filt = 'st-a11y-keyboard-filterkeys-expander'
    open_expander(h, filt)
    seen = until(h, lambda: active(h, 'settings', 'st-a11y-keyboard-enable-filter') is True or None, 'the shared row', 5)
    h.check('amendment 24: Sneltoets voor Filtertoetsen is the same key and follows', bool(seen))
    flip(h, 'st-a11y-keyboard-enable-filter', A11Y_KEYBOARD, 'enable', label='Sneltoets voor Filtertoetsen', expander=filt,
         tested=tested)
    h.gsettings_set(A11Y_KEYBOARD, 'enable', 'false')
    h.screenshot(f'a11y-keyboard-filter.{h.lang}', surface='settings')
    tested.add('st-a11y-keyboard-filterkeys')
    click(h, n=reveal(h, 'st-a11y-keyboard-filterkeys'))
    wait_key(h, A11Y_KEYBOARD, 'slowkeys-enable', 'true', 'Filtertoetsen\' switch writes slowkeys-enable true')
    click(h, n=reveal(h, 'st-a11y-keyboard-filterkeys'))
    wait_key(h, A11Y_KEYBOARD, 'slowkeys-enable', 'false', 'Filtertoetsen off writes slowkeys-enable false')
    wait_key(h, A11Y_KEYBOARD, 'bouncekeys-enable', 'false', '... and bouncekeys-enable false')
    for key, label in (('slowkeys-enable', 'Korte toetsaanslagen negeren (Trage toetsen)'),
                       ('bouncekeys-enable', 'Onbedoelde toetsaanslagen negeren (Stuitertoetsen)')):
        flip(h, f'st-a11y-keyboard-{key}', A11Y_KEYBOARD, key, label=label, expander=filt, tested=tested)
    slow_on = [(A11Y_KEYBOARD, 'slowkeys-enable', 'true')]
    for key, label in (('slowkeys-beep-press', 'Trage toetsen: pieptoon bij indrukken'),
                       ('slowkeys-beep-accept', 'Trage toetsen: pieptoon bij accepteren'),
                       ('slowkeys-beep-reject', 'Trage toetsen: pieptoon bij weigeren')):
        flip(h, f'st-a11y-keyboard-{key}', A11Y_KEYBOARD, key, label=label, expander=filt, needs=slow_on, tested=tested)
    flip(h, 'st-a11y-keyboard-bouncekeys-beep-reject', A11Y_KEYBOARD, 'bouncekeys-beep-reject',
         label='Stuitertoetsen: pieptoon bij weigeren', expander=filt,
         needs=[(A11Y_KEYBOARD, 'bouncekeys-enable', 'true')], tested=tested)
    h.gsettings_set(A11Y_KEYBOARD, 'slowkeys-enable', 'true')
    h.gsettings_set(A11Y_KEYBOARD, 'bouncekeys-enable', 'true')
    until(h, lambda: (lambda n: n if n and n.get('sensitive') else None)(node(h, 'settings', 'st-a11y-keyboard-slowkeys-delay')),
          'the delays sensitive', 5)
    slide(h, 'st-a11y-keyboard-slowkeys-delay', A11Y_KEYBOARD, 'slowkeys-delay', 0, 500, 10, 300, lambda n: n, 20,
          label='Trage toetsen: Acceptatievertraging', expander=filt, value='st-a11y-keyboard-slowkeys-delay-value',
          outside='100')
    slide(h, 'st-a11y-keyboard-bouncekeys-delay', A11Y_KEYBOARD, 'bouncekeys-delay', 0, 900, 10, 450, lambda n: n, 20,
          label='Stuitertoetsen: Acceptatievertraging', expander=filt, value='st-a11y-keyboard-bouncekeys-delay-value',
          outside='200')
    h.gsettings_set(A11Y_KEYBOARD, 'slowkeys-enable', 'false')
    h.gsettings_set(A11Y_KEYBOARD, 'bouncekeys-enable', 'false')
    click(h, n=reveal(h, filt))
    h.sleep(0.5)
    toggle = 'st-a11y-keyboard-togglekeys-enable-expander'
    flip(h, 'st-a11y-keyboard-togglekeys-enable', A11Y_KEYBOARD, 'togglekeys-enable', label='Wisseltoetsen', tested=tested)
    open_expander(h, toggle)
    flip(h, 'st-a11y-keyboard-enable-toggle', A11Y_KEYBOARD, 'enable', label='Sneltoets voor Wisseltoetsen', expander=toggle,
         tested=tested)
    click(h, n=reveal(h, toggle))
    h.sleep(0.5)
    flip(h, 'st-a11y-keyboard-screen-keyboard-enabled', A11Y_APPS, 'screen-keyboard-enabled', label='Schermtoetsenbord',
         tested=tested)
    tested.add('st-a11y-keyboard-show-screenshot-ui')
    click(h, n=reveal(h, 'st-a11y-keyboard-show-screenshot-ui'))
    # GLib's keyfile backend writes an empty string array as [] (slice 4's first run expected @as [])
    wait_key(h, SHELL_KEYS, 'show-screenshot-ui', '[]', 'Print Screen off takes Print from show-screenshot-ui')
    click(h, n=reveal(h, 'st-a11y-keyboard-show-screenshot-ui'))
    wait_key(h, SHELL_KEYS, 'show-screenshot-ui', "['Print']", '... and on puts it back')
    flip(h, 'st-a11y-keyboard-feature-state-change-beep', A11Y_KEYBOARD, 'feature-state-change-beep',
         label='Een geluid wanneer een functie aan of uit gaat', tested=tested)
    unflipped(h, 'a11y-keyboard', tested)
    modes(h, 'Toetsenbord', [(A11Y_KEYBOARD, k) for k in (
        'stickykeys-enable', 'stickykeys-two-key-off', 'stickykeys-modifier-beep', 'slowkeys-enable', 'slowkeys-delay',
        'slowkeys-beep-press', 'slowkeys-beep-accept', 'slowkeys-beep-reject', 'bouncekeys-enable', 'bouncekeys-delay',
        'bouncekeys-beep-reject', 'togglekeys-enable', 'enable', 'feature-state-change-beep')]
        + [(A11Y_APPS, 'screen-keyboard-enabled'), (SHELL_KEYS, 'show-screenshot-ui')])
    no_dutch(h, en, 'Toetsenbord')

    # ---- 11. Muis
    page_open(h, 'a11y-mouse', 'st-a11y-mouse-mousekeys-enable')
    tested = set()
    mk = 'st-a11y-mouse-mousekeys-enable-expander'
    flip(h, 'st-a11y-mouse-mousekeys-enable', A11Y_KEYBOARD, 'mousekeys-enable', label='Muistoetsen', tested=tested)
    open_expander(h, mk)
    slide(h, 'st-a11y-mouse-mousekeys-max-speed', A11Y_KEYBOARD, 'mousekeys-max-speed', 10, 1000, 10, 500, lambda n: n, 20,
          label='Snelheid van muistoetsen', expander=mk, value='st-a11y-mouse-mousekeys-max-speed-value', outside='100')
    slide(h, 'st-a11y-mouse-mousekeys-accel-time', A11Y_KEYBOARD, 'mousekeys-accel-time', 100, 3000, 100, 800,
          lambda n: 3100 - n, 150, label='Versnelling van muistoetsen', expander=mk,
          value='st-a11y-mouse-mousekeys-accel-time-value', outside='1000')
    h.screenshot(f'a11y-mouse-keys.{h.lang}', surface='settings')
    click(h, n=reveal(h, mk))
    h.sleep(0.5)
    flip(h, 'st-a11y-mouse-focus-mode', WM_PREFS, 'focus-mode', pair=("'sloppy'", "'click'"),
         label='Een venster activeren door de muisaanwijzer erop te plaatsen', tested=tested)
    slide(h, 'st-a11y-mouse-double-click', MOUSE, 'double-click', 1, 10, 1, 6, lambda n: 1000 - 100 * (n - 1), 100,
          label='Dubbelkliksnelheid', value='st-a11y-mouse-double-click-value', outside='400')
    sec = 'st-a11y-mouse-secondary-click-enabled-expander'
    flip(h, 'st-a11y-mouse-secondary-click-enabled', A11Y_MOUSE, 'secondary-click-enabled', label='Gesimuleerde secundaire klik',
         tested=tested)
    open_expander(h, sec)
    slide(h, 'st-a11y-mouse-secondary-click-time', A11Y_MOUSE, 'secondary-click-time', 0.5, 3.0, 0.1, 1.5, lambda n: n, 0.15,
          label='Gesimuleerde secundaire klik: Acceptatievertraging', expander=sec,
          value='st-a11y-mouse-secondary-click-time-value', outside='2.0')
    click(h, n=reveal(h, sec))
    h.sleep(0.5)
    dwell = 'st-a11y-mouse-dwell-click-enabled-expander'
    # Klikken door te zweven clicks by itself where the pointer rests: the longest dwell time while it is on (the old
    # value written from outside turns it off well before that)
    flip(h, 'st-a11y-mouse-dwell-click-enabled', A11Y_MOUSE, 'dwell-click-enabled', label='Klikken door te zweven',
         needs=[(A11Y_MOUSE, 'dwell-time', '3.0')], tested=tested)
    open_expander(h, dwell)
    slide(h, 'st-a11y-mouse-dwell-time', A11Y_MOUSE, 'dwell-time', 0.2, 3.0, 0.1, 1.6, lambda n: n, 0.15,
          label='Klikken door te zweven: Vertraging', expander=dwell, value='st-a11y-mouse-dwell-time-value', outside='1.2')
    slide(h, 'st-a11y-mouse-dwell-threshold', A11Y_MOUSE, 'dwell-threshold', 0, 30, 1, 15, lambda n: n, 2,
          label='Bewegingsdrempel', expander=dwell, value='st-a11y-mouse-dwell-threshold-value', outside='10')
    h.screenshot(f'a11y-mouse-assist.{h.lang}', surface='settings')
    h.check('Klikken door te zweven is off at the end', h.gsettings_get(A11Y_MOUSE, 'dwell-click-enabled') in (None, 'false'))
    unflipped(h, 'a11y-mouse', tested)
    modes(h, 'Muis (Toegankelijkheid)', [(A11Y_KEYBOARD, 'mousekeys-enable'), (A11Y_KEYBOARD, 'mousekeys-max-speed'),
                                         (A11Y_KEYBOARD, 'mousekeys-accel-time'), (WM_PREFS, 'focus-mode'), (MOUSE, 'double-click')]
          + [(A11Y_MOUSE, k) for k in ('secondary-click-enabled', 'secondary-click-time', 'dwell-click-enabled', 'dwell-time',
                                       'dwell-threshold')])
    no_dutch(h, en, 'Muis')
