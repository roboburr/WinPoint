"""Settings parity, batch F (notes/claude/settings-f.md): Apps, Geïnstalleerde apps, Geavanceerde opties (the data
page app-advanced), Standaard-apps, Meldingen, Meldingen van app (the data page app-notifications), the notification
sound of amendment 63, Persoonlijke instellingen, Achtergrond and Kleuren in a real GNOME 50 session, with real
pointer clicks, the private keyfile and the private mimeapps.list. Privacy en beveiliging is settings-privacy.py
(with the stand-ins of tests/gnome/lib/standins/f.py).

Run (from G), once per language, under strace (Geavanceerde opties reaches Taakbeheer's process list and the
software service; nothing may run a real pkexec, apt or PackageKit client):
    tools/heavy.sh --min-mb 2560 --tag f -- python3 -B tests/gnome/harness.py launch run --lang nl --source worktree \\
        --scenario tests/gnome/scenarios/settings-apps.py --out tests/gnome/scenarios/settings-apps.out \\
        --strace tests/gnome/scenarios/settings-apps.out/strace.txt
and the same with --lang en --out tests/gnome/scenarios/settings-apps.en.out (--strace …en.out/strace.txt).

Every page: its title and breadcrumb as the registry names them (a data page: the app's name as the last crumb), a
screenshot <page>.<lang> and, in English, no Dutch words (tests/i18n/dutch-words.txt). Every click on a control is a
real pointer click (the page is scrolled with the real wheel first); the last check says whether any fell back to the
dev channel. What it checks besides:
  1. Apps: the links to Geïnstalleerde apps, Standaard-apps and Opstarten
  2. Geïnstalleerde apps: the list; Sorteren op Naam (Z-A) turns it round; the "…" menu of Bestanden (Nautilus) offers
     Geavanceerde opties and Openen in {store}, which names the store that opens this app and is greyed when that
     store is not installed (a system package needs GNOME Software), and Geavanceerde opties opens the app's page
  3. Geavanceerde opties of Bestanden: App-machtigingen has Meldingen and Zoeken and no PermissionStore rows (GNOME
     shows those for Flatpak and Snap apps, the harness has none: the host unit tests cover them); Zoeken bound to
     search-providers `disabled` (a click, an outside write followed), greyed with GNOME's line while
     `disable-external` is written true from outside; the sections in Windows' order (Wordt uitgevoerd bij aanmelden,
     Beëindigen, Verwijderen, then Standaardkoppelingen); Wordt uitgevoerd bij aanmelden writes the
     autostart copy GNOME writes (X-GNOME-Autostart-enabled true, then false); Standaardkoppelingen: Als standaard
     instellen and Type verwijderen write mimeapps.list as GIO does, Opnieuw instellen takes both out again;
     Beëindigen says Bestanden is not running; Meldingen opens the app's notification page (Systeem › Meldingen ›
     Bestanden)
  4. Standaard-apps as Windows 11 (amendment 22): no drop-down above the two searches; the type search finds
     text/html and its chooser sets another browser for it (mimeapps.list); "Standaard-apps per soort" holds GNOME's
     rows and its Teksteditor sets text/plain; Alle standaard-apps opnieuw instellen (with its question) takes both
     out; an app's view (Files & Links) and the file-type view
  5. Meldingen: the lock-screen checkbox inside the Meldingen expander writes show-in-lock-screen; Niet storen is
     show-banners inverted (and the mirror sets notifications.dnd); Bestanden's switch in the app list writes its
     `enable` with application-id and application-children (amendment 6) and follows an outside write; Sorteren op
     Naam sorts the list; WinPoint's own Meldingen and geluiden switches write settings.json
  6. Meldingen van app (Bestanden, opened from its row): each of GNOME's six keys at the app's path by a click and
     back from outside, greyed as GNOME greys them (Inhoud weergeven needs the banners, the lock-screen rows need
     the global lock-screen key, everything needs Meldingen); a write the host refuses (a test app removed after its
     page opened) puts Meldingen back and leaves the other rows usable
  7. amendment 63: a GNOME notification of Bestanden (enable-sound-alerts true) plays exactly one sound while the
     WinPoint toast shows (Sound.play counted in the shell), none with notifications.sounds false, with Niet storen
     on, or with the app's enable-sound-alerts false
  8. Persoonlijke instellingen; Achtergrond (a click on an Ubuntu picture sets wallpaper.path and GNOME's picture-uri;
     Uitrekken writes picture-options 'stretched'; background.add copies a picture, its remove button trashes it;
     GNOME's background keys are WinPoint's: gsettings.set refuses them); Kleuren (a Windows accent swatch sets
     theme.accent and, through the mirror, GNOME's accent-color to the nearest GNOME colour, which the row shows;
     the row follows an outside write; gsettings.set refuses accent-color, owned by theme.accent)
"""
import json


NAUTILUS = 'org.gnome.Nautilus.desktop'
APP_CENTER = 'snap-store_snap-store.desktop'
GNOME_SOFTWARE = 'org.gnome.Software.desktop'
REVERT_APP = 'win11-test-revert.desktop'      # a test app of the run (Meldingen van app: a refused write)
SEARCH = 'org.gnome.desktop.search-providers'
NOTIFY = 'org.gnome.desktop.notifications'
APP_NOTIFY = 'org.gnome.desktop.notifications.application'
NAUTILUS_PATH = '/org/gnome/desktop/notifications/application/org-gnome-nautilus/'
INTERFACE = 'org.gnome.desktop.interface'
BACKGROUND = 'org.gnome.desktop.background'

CLICKS: list[dict] = []        # every control click of the run: its name and how it reached the app

# ---- helpers (the same in settings-privacy.py) ---------------------------------------------------------------------
WINDOW_JS = ("global.get_window_actors().map(a=>a.meta_window).filter(w=>w.get_pid()===%d"
             "&&w.get_window_type()===Meta.WindowType.NORMAL).map(w=>{const r=w.get_frame_rect();"
             "return {title:w.get_title(),x:r.x,y:r.y,w:r.width,h:r.height};})")
# a real wheel: the harness's Clutter virtual pointer (lib/scenario.py POINTER_CLICK_JS) over the page, n notches
SCROLL_JS = ("(async()=>{{const Clutter=(await import('gi://Clutter')).default;const GLib=(await import('gi://GLib')).default;"
             "const wait=ms=>new Promise(r=>GLib.timeout_add(GLib.PRIORITY_DEFAULT,ms,()=>{{r();return GLib.SOURCE_REMOVE;}}));"
             "const seat=Clutter.get_default_backend().get_default_seat();"
             "globalThis.__ge1hPointer??=seat.create_virtual_device(Clutter.InputDeviceType.POINTER_DEVICE);"
             "const d=globalThis.__ge1hPointer,us=()=>GLib.get_monotonic_time();"
             "d.notify_absolute_motion(us(),{x},{y});await wait(60);"
             "for(let i=0;i<{n};i++){{d.notify_discrete_scroll(us(),Clutter.ScrollDirection.{dir},Clutter.ScrollSource.WHEEL);"
             "await wait(40);}}return global.get_pointer().slice(0,2);}})()")


def wait(h, fn, label, timeout=20.0, interval=0.3):
    """fn() until it is truthy; None after `timeout` (the check that follows records it)."""
    try:
        return h.until(fn, label, timeout, interval)
    except TimeoutError as error:
        h.log(f'timeout: {error}')
        return None


def node(h, name):
    found = h.find('settings', name)
    return found[0] if found else None


def active(h, name):
    return (node(h, name) or {}).get('active')


def sensitive(h, name):
    return (node(h, name) or {}).get('sensitive')


def labels(h, name):
    """Every label inside the widget `name` (a row, a drop-down: its shown value)."""
    found = node(h, name)
    return [str(n['label']) for n in h.hm.tree_nodes([{'tree': found}]) if n.get('label')] if found else []


def shows(h, text):
    return text in h.text('settings')['text']


def no_dutch(h, page):
    text = h.page_text('settings')
    if h.lang == 'en':
        h.check(f'{page}: no Dutch words in English', not text['dutchWordHits'], hits=text['dutchWordHits'][:20])
    return text


def crumbs(h):
    """Settings' header as it reads: the page title, or each crumb of the breadcrumb (the labels inside st-title)."""
    for head in h.find('settings', cls='st-title'):
        found = [str(n['label']) for n in h.hm.tree_nodes([{'tree': head}]) if n.get('type') == 'GtkLabel' and n.get('label')]
        if found:
            return found
    return []


def header(h, page, chain, last=None):
    """The header shows the registry's titles of `chain` (the parents, top first) and the page's own (or `last`)."""
    titles = h.session.titles
    want = [titles.get(p, {}).get(h.lang) for p in chain] + [last or titles.get(page, {}).get(h.lang)]
    got = wait(h, lambda: crumbs(h) == want, f'{page}: its header', 10)
    h.check(f'{page}: title and breadcrumb "{" › ".join(map(str, want))}"', bool(got), want=want, got=crumbs(h))


def dismiss(h):
    """Close a kit dialog that is still open (Escape), so it does not cover the next page."""
    for _ in range(3):
        if not node(h, 'w11-dialog'):
            return
        h.key('settings', 'Escape')
        wait(h, lambda: not node(h, 'w11-dialog'), 'the dialog closed', 4)
    h.check('a dialog that stayed open is closed', not node(h, 'w11-dialog'))


def visit(h, page, chain, ready=None, shot=True):
    if h.surfaces.get('settings'):
        dismiss(h)
    shown = h.surfaces.get('settings')
    if shown:
        shown.page = None   # a click or a deep link may have left the helper's page behind: always show this one
    h.open(f'settings:{page}')
    if ready:
        wait(h, lambda: node(h, ready), f'{page}: {ready}', 20)
    header(h, page, chain)
    if shot:
        h.screenshot(f'{page}.{h.lang}')
    no_dutch(h, page)


def reveal(h, name=None, label=None, tries=14):
    """Scroll Settings' page with the real wheel until the widget's centre (by name or label) is inside the page's
    scroll view, so the click that follows is a pointer click (h.click falls back to the dev channel for a widget
    outside the window)."""
    for _ in range(tries):
        tops = h.state('settings')
        found = [n for n in h.find('settings', name, label=label, state=tops) if n.get('rect')]
        views = [n for n in h.find('settings', cls='st-scroll', state=tops) if n.get('rect')]
        if not found or not views:
            return bool(found)
        widget = found[0]
        view = next((v for v in views if v['top'] == widget['top']), views[0])
        _, y, _, height = widget['rect']
        vx, vy, vw, vh = view['rect']
        middle, centre = y + height // 2, vy + vh // 2
        if vy + 6 <= middle <= vy + vh - 6:
            return True
        wins = h.eval(WINDOW_JS % h.surface('settings').pid) or []
        title = tops[widget['top']].get('title')
        win = next((w for w in wins if w['title'] == title), wins[0] if wins else None)
        if not win:
            return False
        notches = max(1, min(8, abs(middle - centre) // 70))
        h.eval(SCROLL_JS.format(x=win['x'] + vx + vw // 2, y=win['y'] + vy + vh // 2, n=notches,
                                dir='DOWN' if middle > centre else 'UP'))
        h.sleep(0.3)
    return False


def press(h, name=None, found=None, scroll=True, label=None):
    """A real pointer click on a control (by name, by label or a node from h.find; scrolled into view first);
    recorded for the last check."""
    if scroll and (name or label):
        reveal(h, name, label)
    if label and not found:
        found = next((n for n in h.find('settings', label=label) if n.get('rect')), None)
    how = h.click('settings', name, node=found) if found else h.click('settings', name)
    CLICKS.append({'name': name or (found or {}).get('name') or (found or {}).get('label'), 'input': how.get('input')})
    return how


def choose(h, name, label):
    """Pick `label` in the kit drop-down `name`: a pointer click opens its list, a second one chooses."""
    def option():
        return next((n for n in h.find('settings', cls='w11-list-item')
                     if any(str(k.get('label') or '') == label for k in h.hm.tree_nodes([{'tree': n}]))), None)
    press(h, name)
    found = wait(h, option, f'the option {label}', 5)
    if not found:
        h.click('settings', name, real=False)
        CLICKS.append({'name': name, 'input': 'dev-fallback'})
        found = wait(h, option, f'the option {label} (dev click)', 5)
    if found:
        press(h, found=found, scroll=False)
    return bool(found)


DIALOG_CLICKS: list[dict] = []   # every button pressed inside a kit dialog, and whether the pointer reached it


def confirm(h, done, label):
    """The primary button of the kit dialog that is open (c::confirm, Dialog::primary): a real pointer click, and when
    done() stays false the dev channel's click by name (notes/claude/settings-b.md note 9: the dialog's smoke layer
    claims every press, so a pointer click never reaches a kit dialog's buttons); the action is checked either way.
    False when no dialog asked."""
    found = wait(h, lambda: [n for n in h.find('settings', 'w11-dialog-primary') if n.get('rect')], 'the dialog', 8)
    if not found:
        dismiss(h)
        return False
    how = h.click('settings', node=found[-1])   # the dialog on top (the last one in the tree)
    ok, path = wait(h, done, f'{label} (pointer)', 4), 'pointer'
    if not ok:
        path = 'dev'
        h.click('settings', 'w11-dialog-primary', real=False)
        ok = wait(h, done, f'{label} (dev click)', 10)
    DIALOG_CLICKS.append({'button': 'w11-dialog-primary', 'action': label, 'path': path, 'done': bool(ok),
                          'pointer': {k: how.get(k) for k in ('input', 'x', 'y', 'rect') if k in how}})
    wait(h, lambda: not node(h, 'w11-dialog'), 'the dialog closed', 5)
    dismiss(h)
    return True


def setting(h, path):
    """A WinPoint setting as the Settings app's host reads it (settings.get)."""
    value = h.backend('settings', 'settings.get') or {}
    for part in path.split('.'):
        value = value.get(part) if isinstance(value, dict) else None
    return value


def set_setting(h, path, value):
    return h.backend('settings', 'settings.set', {'path': path, 'value': value})


def deep_link(h, params):
    """Show a page with parameters in the running Settings, as a deep link does (win11-app --params)."""
    argv = [str(h.hm.RUN / 'prefix' / 'bin' / 'win11-app'), '--app', 'settings', '--params', json.dumps(params)]
    proc = h.spawn(f'relay-{params["page"]}', argv)
    try:
        proc.wait(timeout=20)
    except Exception:  # noqa: BLE001 (a relay that stays is stopped at the end of the run)
        pass


def mime_entry(h, group, mime):
    """A key of the private mimeapps.list (run/config, GIO's user file), or None."""
    try:
        lines = (h.hm.RUN / 'config' / 'mimeapps.list').read_text().splitlines()
    except OSError:
        return None
    current = None
    for line in lines:
        line = line.strip()
        if line.startswith('[') and line.endswith(']'):
            current = line[1:-1]
        elif current == group and line.split('=', 1)[0].strip() == mime:
            return line.split('=', 1)[1].strip()
    return None


def safe(mime):
    return ''.join(c if c.isascii() and c.isalnum() else '-' for c in mime)


def guard_calls(h, tool):
    try:
        lines = (h.hm.RUN / 'logs' / 'guard-calls.log').read_text().splitlines()
    except OSError:
        return []
    return [line.split('\t', 2)[2] for line in lines if line.count('\t') >= 2 and line.split('\t')[1] == tool]


def app_name(h, desktop_id):
    apps = h.backend('settings', 'apps.list') or []
    return next((a.get('name') for a in apps if isinstance(a, dict) and a.get('id') == desktop_id), None)


# ---- the run ------------------------------------------------------------------------------------------------------
def run(h):
    apps_part(h)
    advanced_part(h)
    defaultapps_part(h)
    notifications_part(h)
    app_notifications_part(h)
    sound_part(h)
    personalization_part(h)
    dev = [c for c in CLICKS if c['input'] != 'clutter']
    h.record('clicks', CLICKS)
    h.check(f'every control click ({len(CLICKS)}) was a real pointer click', not dev, dev=dev)
    h.record('dialog clicks', DIALOG_CLICKS)
    h.check(f'a pointer click reaches a kit dialog\'s button ({len(DIALOG_CLICKS)} pressed)',
            all(c['path'] == 'pointer' for c in DIALOG_CLICKS),
            dialogs=DIALOG_CLICKS)
    h.record('guard calls', {tool: guard_calls(h, tool) for tool in ('pkexec', 'systemctl', 'apt', 'apt-get', 'pkcon')})
    h.check('no pkexec, apt or pkcon was called by a page of this run',
            not any(guard_calls(h, tool) for tool in ('pkexec', 'apt', 'apt-get', 'pkcon')))


def rows_of(h, prefix):
    return [n['name'] for n in h.find('settings', where=lambda n: str(n.get('name') or '').startswith(prefix))]


def apps_part(h):
    nl = h.lang == 'nl'
    # ---- 1. Apps
    visit(h, 'apps', [], ready='link-installed')
    for page in ('installed', 'defaultapps', 'startup'):
        h.check(f'apps: a link to {page}', node(h, f'link-{page}') is not None)

    # ---- 2. Geïnstalleerde apps
    visit(h, 'installed', ['apps'], ready='installed-list')
    apps = h.backend('settings', 'apps.list') or []
    ids = [a.get('id') for a in apps if isinstance(a, dict)]
    h.record('apps', len(ids))
    h.check('installed: Bestanden (Nautilus) is installed in the session', NAUTILUS in ids)
    az = rows_of(h, 'installed-more-')
    if len(az) > 1 and choose(h, 'installed-sort', 'Naam (Z-A)' if nl else 'Name (Z-A)'):
        za = wait(h, lambda: (lambda r: r if r and r[0] != az[0] else None)(rows_of(h, 'installed-more-')), 'Z-A', 8)
        h.check('installed: Sorteren op Naam (Z-A) turns the list round', bool(za) and za[0] == az[-1] and za[-1] == az[0],
                first=(za or [None])[0], last=az[-1])
        choose(h, 'installed-sort', 'Naam (A-Z)' if nl else 'Name (A-Z)')
    if NAUTILUS not in ids:
        return
    h.type_text('settings', 'installed-search', 'Bestanden' if nl else 'Files')
    more = wait(h, lambda: node(h, f'installed-more-{NAUTILUS}'), 'the Nautilus row', 10)
    item = None
    if more:
        press(h, f'installed-more-{NAUTILUS}')
        item = wait(h, lambda: node(h, 'installed-advanced'), 'the menu item Geavanceerde opties', 8)
        h.check('installed: the "…" menu has Geavanceerde opties and Openen in App Center', bool(item)
                and node(h, 'installed-store') is not None)
        # Openen in {store} is the store that opens this app (privacy.app's kind): App Center only for a snap,
        # GNOME Software for a Flatpak or a system package; greyed when that store is not installed
        info = h.backend('settings', 'privacy.app', {'id': NAUTILUS, 'sizes': False}) or {}
        kind = info.get('kind')
        target = APP_CENTER if kind == 'snap' else GNOME_SOFTWARE if kind in ('flatpak', 'host') else None
        opens = target in ids
        store = 'GNOME Software' if target == GNOME_SOFTWARE else 'App Center'
        shown = wait(h, lambda: sensitive(h, 'installed-store') is opens
                     and (not opens or any(store in x for x in labels(h, 'installed-store'))), 'the store item', 8)
        h.record('store item', {'kind': kind, 'target': target, 'installed': target in ids,
                                'sensitive': sensitive(h, 'installed-store'), 'labels': labels(h, 'installed-store')})
        h.check(f'installed: Openen in … for Bestanden ({kind}) follows its store: '
                f'{"enabled, " + store if opens else "greyed (" + str(target) + " is not installed)"}', bool(shown))
        h.screenshot(f'installed-menu.{h.lang}')
        if item:
            press(h, 'installed-advanced', scroll=False)
    if not item:
        deep_link(h, {'page': 'app-advanced', 'id': NAUTILUS})


def advanced_part(h):
    nl = h.lang == 'nl'
    page = wait(h, lambda: node(h, 'st-app-advanced-terminate'), 'the app page', 25)
    if not page:
        deep_link(h, {'page': 'app-advanced', 'id': NAUTILUS})
        page = wait(h, lambda: node(h, 'st-app-advanced-terminate'), 'the app page', 25)
    h.check('app-advanced: the page of Bestanden is shown', bool(page))
    if not page:
        return
    name = app_name(h, NAUTILUS)
    header(h, 'app-advanced', ['apps', 'installed'], last=name)
    for widget in ('st-app-advanced-open', 'st-app-advanced-uninstall', 'st-app-advanced-notifications',
                   'st-app-advanced-handlers-reset', 'st-app-advanced-autostart'):
        h.check(f'app-advanced: {widget}', node(h, widget) is not None)
    h.screenshot(f'app-advanced.{h.lang}')
    no_dutch(h, 'app-advanced')

    # App-machtigingen: a desktop app has no portal id, so GNOME shows no PermissionStore rows for it
    h.check('app-advanced: App-machtigingen of a desktop app: Meldingen and Zoeken, no PermissionStore rows (GNOME)',
            not any(node(h, f'st-app-advanced-{k}') for k in ('location', 'camera', 'microphone', 'screenshot',
                                                                'wallpaper', 'shortcutsInhibit'))
            and node(h, 'st-app-advanced-background') is None)

    # Zoeken: search-providers disabled
    switch = node(h, 'st-app-advanced-disabled')
    h.check('app-advanced: Zoeken is bound to search-providers disabled', switch is not None)
    if switch:
        h.check('app-advanced: Zoeken starts on (Nautilus is not in disabled)', switch.get('active') is True)
        press(h, 'st-app-advanced-disabled')
        written = wait(h, lambda: NAUTILUS in (h.gsettings_get(SEARCH, 'disabled') or ''), 'disabled written', 10)
        h.check('app-advanced: a click writes org.gnome.Nautilus.desktop into disabled', bool(written),
                value=h.gsettings_get(SEARCH, 'disabled'))
        h.gsettings_set(SEARCH, 'disabled', '@as []')
        back = wait(h, lambda: active(h, 'st-app-advanced-disabled') is True, 'the switch follows an outside write', 10)
        h.check('app-advanced: the switch follows the key written from outside', bool(back))
        # app search off for every app (Zoekmachtigingen, or anywhere else): greyed with GNOME's line, and back
        off_text = 'Zoeken in apps is uitgeschakeld' if nl else 'Searching in apps is turned off'
        h.gsettings_set(SEARCH, 'disable-external', 'true')
        grey = wait(h, lambda: sensitive(h, 'st-app-advanced-disabled') is False and shows(h, off_text),
                    'Zoeken greyed', 10)
        h.check('app-advanced: Zoeken greys (with its line) when disable-external is written from outside', bool(grey))
        h.gsettings_set(SEARCH, 'disable-external', 'false')
        again = wait(h, lambda: sensitive(h, 'st-app-advanced-disabled') is True and not shows(h, off_text),
                     'Zoeken back', 10)
        h.check('app-advanced: and usable again when disable-external is false', bool(again))

    # Windows' order: Wordt uitgevoerd bij aanmelden, Beëindigen, (Opnieuw instellen,) Verwijderen, then WinPoint's
    # additions from GNOME (Standaardkoppelingen)
    def top(name):
        rect = (node(h, name) or {}).get('rect')
        return rect[1] if rect else None
    order = ['st-app-advanced-notifications', 'st-app-advanced-autostart', 'st-app-advanced-terminate',
             'st-app-advanced-uninstall', 'st-app-advanced-handlers-reset']
    tops = [top(n) for n in order]
    h.record('section order', dict(zip(order, tops)))
    h.check('app-advanced: the sections in Windows 11\'s order, WinPoint\'s additions after Verwijderen',
            None not in tops and tops == sorted(tops), tops=tops)
    h.check('app-advanced: no Opnieuw instellen for a system package (no cache the host can empty)',
            node(h, 'st-app-advanced-clear-cache') is None)

    # Wordt uitgevoerd bij aanmelden: GNOME's autostart copy
    autostart = h.hm.RUN / 'config' / 'autostart' / NAUTILUS
    if node(h, 'st-app-advanced-autostart'):
        press(h, 'st-app-advanced-autostart')
        on = wait(h, lambda: autostart.is_file() and 'X-GNOME-Autostart-enabled=true' in autostart.read_text(),
                  'the autostart copy', 10)
        h.check('app-advanced: Wordt uitgevoerd bij aanmelden writes ~/.config/autostart/org.gnome.Nautilus.desktop '
                '(X-GNOME-Autostart-enabled=true)', bool(on))
        press(h, 'st-app-advanced-autostart')
        off = wait(h, lambda: autostart.is_file() and 'X-GNOME-Autostart-enabled=false' in autostart.read_text(),
                   'autostart off', 10)
        h.check('app-advanced: and off again X-GNOME-Autostart-enabled=false', bool(off))

    # Standaardkoppelingen (GNOME's Files & Links) in mimeapps.list
    info = h.backend('settings', 'privacy.app', {'id': NAUTILUS, 'sizes': False}) or {}
    handlers = [x for x in info.get('handlers') or [] if isinstance(x, dict)]
    h.record('handlers', [(x.get('type'), x.get('isDefault')) for x in handlers])
    other = next((x['type'] for x in handlers if not x.get('isDefault')), None)
    removed = next((x['type'] for x in reversed(handlers) if x.get('type') != other), None)
    if other:
        press(h, f'st-app-advanced-default-{safe(other)}')
        made = wait(h, lambda: NAUTILUS in (mime_entry(h, 'Default Applications', other) or ''), 'set as default', 10)
        h.check(f'app-advanced: Als standaard instellen writes {other}={NAUTILUS} under Default Applications', bool(made),
                value=mime_entry(h, 'Default Applications', other))
    if removed:
        wait(h, lambda: node(h, f'st-app-advanced-remove-{safe(removed)}'), 'the page again', 10)
        press(h, f'st-app-advanced-remove-{safe(removed)}')
        gone = wait(h, lambda: NAUTILUS in (mime_entry(h, 'Removed Associations', removed) or ''), 'type removed', 10)
        h.check(f'app-advanced: Type verwijderen writes {removed} under Removed Associations', bool(gone),
                value=mime_entry(h, 'Removed Associations', removed))
    if handlers and wait(h, lambda: node(h, 'st-app-advanced-handlers-reset'), 'Opnieuw instellen', 10):
        press(h, 'st-app-advanced-handlers-reset')
        reset = wait(h, lambda: not (other and mime_entry(h, 'Default Applications', other))
                     and not (removed and mime_entry(h, 'Removed Associations', removed)), 'reset', 10)
        h.check('app-advanced: Opnieuw instellen takes both out of mimeapps.list', bool(reset))

    # Beëindigen and Verwijderen
    if wait(h, lambda: node(h, 'st-app-advanced-terminate'), 'Beëindigen', 10):
        press(h, 'st-app-advanced-terminate')
        said = wait(h, lambda: shows(h, f'{name} wordt niet uitgevoerd.' if nl else f'{name} isn’t running.'), 'the toast', 8)
        h.check('app-advanced: Beëindigen says the app is not running (sysmon.processes has no Bestanden)', bool(said))
    h.record('uninstall', {'sensitive': sensitive(h, 'st-app-advanced-uninstall')})

    # Meldingen opens the app's own notification page
    if wait(h, lambda: node(h, 'st-app-advanced-notifications'), 'Meldingen', 10):
        press(h, 'st-app-advanced-notifications')
        shown = wait(h, lambda: node(h, 'st-app-notifications-enable'), 'the app notification page', 15)
        h.check('app-advanced: Meldingen opens Meldingen van app for Bestanden', bool(shown))
        header(h, 'app-notifications', ['system', 'notifications'], last=name)


def chosen_radio(h):
    """The label of the chooser's radio that is on (a GtkCheckButton's text is its GtkLabel child)."""
    group = node(h, 'defaultapps-type-apps')
    for n in h.hm.tree_nodes([{'tree': group}]) if group else []:
        if n.get('type') == 'GtkCheckButton' and n.get('active'):
            return next((str(k['label']) for k in h.hm.tree_nodes([{'tree': n}]) if k.get('type') == 'GtkLabel'), None)
    return None


def defaultapps_part(h):
    nl = h.lang == 'nl'
    visit(h, 'defaultapps', ['apps'], ready='defaultapps-type-search')
    h.check('defaultapps: the two Windows searches are there',
            bool(node(h, 'defaultapps-type-search')) and node(h, 'defaultapps-app-search') is not None)
    browser, apps_search = node(h, 'defaultapps-browser'), node(h, 'defaultapps-app-search')
    below = (browser is None or not browser.get('rect')
             or bool(apps_search and apps_search.get('rect') and browser['rect'][1] > apps_search['rect'][1]))
    h.check('defaultapps: no drop-down above the searches (amendment 22)', below,
            browser=browser and browser.get('rect'), search=apps_search and apps_search.get('rect'))
    # the type search and its chooser
    h.type_text('settings', 'defaultapps-type-search', 'html')
    found = wait(h, lambda: node(h, 'defaultapps-type-text-html'), 'text/html found', 10)
    h.check('defaultapps: the type search finds text/html', bool(found))
    h.screenshot(f'defaultapps-types.{h.lang}')
    html = h.backend('settings', 'apps.forType', {'mime': 'text/html'}) or {}
    pick = next((a for a in html.get('apps') or [] if not a.get('isDefault')), None)
    h.record('text/html apps', [(a.get('id'), a.get('isDefault')) for a in html.get('apps') or []])
    if found and pick:
        press(h, 'defaultapps-type-text-html')
        radio = wait(h, lambda: next((n for n in h.hm.tree_nodes([{'tree': node(h, 'defaultapps-type-apps')}])
                                      if n.get('type') == 'GtkLabel' and n.get('label') == pick['name']), None)
                     if node(h, 'defaultapps-type-apps') else None, 'the chooser', 10)
        h.check('defaultapps: the chooser lists the apps of text/html', bool(radio))
        h.screenshot(f'defaultapps-chooser.{h.lang}')
        if radio:
            # Require the repaired dialog to select another app through a real pointer click.
            h.click('settings', node={**radio, 'top': (node(h, 'defaultapps-type-apps') or {}).get('top', 0)})
            picked = wait(h, lambda: chosen_radio(h) == pick['name'], 'the radio of another app', 3)
            h.check('defaultapps: a pointer click selects the other application in the dialog', bool(picked))
            target = pick
            h.record('chooser radio', {'pointer': bool(picked), 'target': target.get('id')})
            done = confirm(h, lambda: target['id'] in (mime_entry(h, 'Default Applications', 'text/html') or ''),
                           'Standaard instellen')
            h.check(f'defaultapps: Standaard instellen writes text/html={target["id"]} into mimeapps.list',
                    done and target['id'] in (mime_entry(h, 'Default Applications', 'text/html') or ''),
                    value=mime_entry(h, 'Default Applications', 'text/html'))
        dismiss(h)
    # GNOME's Default Apps rows: Teksteditor
    press(h, 'defaultapps-kinds')
    kinds = wait(h, lambda: node(h, 'defaultapps-browser') and node(h, 'defaultapps-calendar'), 'the kinds', 8)
    h.check('defaultapps: Standaard-apps per soort holds Webbrowser and Agenda', bool(kinds))
    defs = h.backend('settings', 'apps.defaults', {'candidates': True}) or {}
    current = (defs.get('text') or {}).get('id') if isinstance(defs.get('text'), dict) else None
    editor = next((a for a in (defs.get('candidates') or {}).get('text') or [] if a.get('id') != current), None)
    h.record('text candidates', [a.get('id') for a in (defs.get('candidates') or {}).get('text') or []])
    if kinds and editor:
        chosen = choose(h, 'defaultapps-text', editor['name'])
        made = chosen and wait(h, lambda: editor['id'] in (mime_entry(h, 'Default Applications', 'text/plain') or ''),
                               'text/plain set', 10)
        h.check(f'defaultapps: Teksteditor writes text/plain={editor["id"]}', bool(made),
                value=mime_entry(h, 'Default Applications', 'text/plain'))
    for widget in ('defaultapps-view-file', 'defaultapps-view-link', 'defaultapps-reset-all'):
        h.check(f'defaultapps: {widget}', node(h, widget) is not None)
    h.screenshot(f'defaultapps-kinds.{h.lang}')
    if node(h, 'defaultapps-reset-all'):
        press(h, 'defaultapps-reset-all')
        asked = confirm(h, lambda: not mime_entry(h, 'Default Applications', 'text/html')
                        and not mime_entry(h, 'Default Applications', 'text/plain'), 'Opnieuw instellen')
        reset = asked and not mime_entry(h, 'Default Applications', 'text/html') \
            and not mime_entry(h, 'Default Applications', 'text/plain')
        h.check('defaultapps: Alle standaard-apps opnieuw instellen asks, then takes text/html and text/plain out',
                bool(reset), html=mime_entry(h, 'Default Applications', 'text/html'),
                plain=mime_entry(h, 'Default Applications', 'text/plain'))
    # the file-type view and an app's view
    if wait(h, lambda: node(h, 'defaultapps-view-file'), 'the related views', 10):
        press(h, 'defaultapps-view-file')
        listed = wait(h, lambda: len(rows_of(h, 'defaultapps-type-')) > 5, 'the file types', 15)
        h.check('defaultapps: Standaardwaarden kiezen op bestandstype lists file types', bool(listed))
        header(h, 'defaultapps', ['apps'])
        h.screenshot(f'defaultapps-files.{h.lang}')
    deep_link(h, {'page': 'defaultapps', 'id': NAUTILUS})
    view = wait(h, lambda: node(h, 'st-app-advanced-handlers-reset') and node(h, 'defaultapps-advanced'),
                'the app view', 20)
    h.check('defaultapps: an app view shows its Files & Links and links to its Geavanceerde opties', bool(view))
    header(h, 'defaultapps', ['apps'])
    h.screenshot(f'defaultapps-app.{h.lang}')
    no_dutch(h, 'defaultapps app view')


def notifications_part(h):
    nl = h.lang == 'nl'
    visit(h, 'notifications', ['system'], ready='notifications-toggle')
    for widget in ('st-notifications-show-in-lock-screen', 'notifications-sounds', 'st-notifications-show-banners',
                   'notifications-sort', 'notifications-test'):
        h.check(f'notifications: {widget}', node(h, widget) is not None)
    # the lock screen (amendment 5: inside the Meldingen expander)
    if node(h, 'st-notifications-show-in-lock-screen'):
        press(h, 'st-notifications-show-in-lock-screen')
        written = wait(h, lambda: h.gsettings_get(NOTIFY, 'show-in-lock-screen') == 'false', 'lock screen written', 10)
        h.check('notifications: a click writes show-in-lock-screen false', bool(written))
        h.gsettings_set(NOTIFY, 'show-in-lock-screen', 'true')
        back = wait(h, lambda: active(h, 'st-notifications-show-in-lock-screen') is True, 'the checkbox follows', 10)
        h.check('notifications: the checkbox follows the key written from outside', bool(back))
    # Niet storen: GNOME's show-banners inverted, the mirror keeps notifications.dnd equal
    if node(h, 'st-notifications-show-banners'):
        h.check('notifications: Niet storen starts off', active(h, 'st-notifications-show-banners') is False)
        press(h, 'st-notifications-show-banners')
        on = wait(h, lambda: h.gsettings_get(NOTIFY, 'show-banners') == 'false', 'show-banners false', 10)
        h.check('notifications: Niet storen writes show-banners false', bool(on))
        mirrored = wait(h, lambda: setting(h, 'notifications.dnd') is True, 'notifications.dnd mirrored', 10)
        h.check('notifications: the mirror sets notifications.dnd', bool(mirrored))
        h.screenshot(f'notifications-dnd.{h.lang}')
        press(h, 'st-notifications-show-banners')
        off = wait(h, lambda: h.gsettings_get(NOTIFY, 'show-banners') == 'true'
                   and setting(h, 'notifications.dnd') is False, 'Niet storen off again', 10)
        h.check('notifications: Niet storen off again on both sides', bool(off))
    # the apps: Bestanden uses notifications (X-GNOME-UsesNotifications)
    row = wait(h, lambda: node(h, 'notifications-app-org-gnome-nautilus'), 'the Bestanden row', 10)
    h.check('notifications: Bestanden is in the app list', bool(row))
    enable = 'st-notifications-org-gnome-nautilus-enable'
    h.check('notifications: its switch is bound to the app\'s enable', node(h, enable) is not None)
    if node(h, enable):
        press(h, enable)
        written = wait(h, lambda: h.gsettings_get(APP_NOTIFY, 'enable', NAUTILUS_PATH) == 'false', 'enable false', 10)
        h.check('notifications: the switch writes enable false at the app\'s path', bool(written))
        h.check('notifications: the first write records application-id (amendment 6)',
                h.gsettings_get(APP_NOTIFY, 'application-id', NAUTILUS_PATH) == "'org.gnome.Nautilus.desktop'",
                value=h.gsettings_get(APP_NOTIFY, 'application-id', NAUTILUS_PATH))
        h.check('notifications: and the app in application-children',
                'org-gnome-nautilus' in (h.gsettings_get(NOTIFY, 'application-children') or ''),
                value=h.gsettings_get(NOTIFY, 'application-children'))
        h.gsettings_set(APP_NOTIFY, 'enable', 'true', NAUTILUS_PATH)
        back = wait(h, lambda: active(h, enable) is True, 'the app switch follows', 10)
        h.check('notifications: the app switch follows the key written from outside', bool(back))
    # Sorteren op Naam
    if node(h, 'notifications-sort') and choose(h, 'notifications-sort', 'Naam' if nl else 'Name'):
        def names():
            out = []
            for n in h.find('settings', where=lambda n: str(n.get('name') or '').startswith('notifications-app-')):
                first = next((str(k['label']) for k in h.hm.tree_nodes([{'tree': n}])
                              if k.get('type') == 'GtkLabel' and k.get('label')), '')
                out.append(first)
            return out
        listed = names()
        h.check('notifications: Sorteren op Naam sorts the apps by name', len(listed) > 1
                and listed == sorted(listed, key=str.lower), names=listed[:12])
        choose(h, 'notifications-sort', 'Meest recent' if nl else 'Most recent')
    # WinPoint's own switches: settings.json (the Meldingen expander's switch and "geluiden afspelen")
    if node(h, 'notifications-sounds'):
        press(h, 'notifications-sounds')
        off = wait(h, lambda: setting(h, 'notifications.sounds') is False, 'sounds off', 10)
        press(h, 'notifications-sounds')
        on = wait(h, lambda: setting(h, 'notifications.sounds') is True, 'sounds on', 10)
        h.check('notifications: Toestaan dat meldingen geluiden afspelen writes notifications.sounds (off, on)',
                bool(off) and bool(on))
    if node(h, 'notifications-toggle'):
        press(h, 'notifications-toggle')
        off = wait(h, lambda: setting(h, 'notifications.showToasts') is False, 'toasts off', 10)
        press(h, 'notifications-toggle')
        on = wait(h, lambda: setting(h, 'notifications.showToasts') is True, 'toasts on', 10)
        h.check('notifications: the Meldingen switch writes notifications.showToasts (off, on)', bool(off) and bool(on))


APP_KEYS = ('enable', 'show-banners', 'force-expanded', 'show-in-lock-screen', 'details-in-lock-screen',
            'enable-sound-alerts')


def app_notifications_part(h):
    name = app_name(h, NAUTILUS)
    if h.surfaces.get('settings'):
        h.surfaces['settings'].page = None
    h.open('settings:notifications')
    if not wait(h, lambda: node(h, 'notifications-app-org-gnome-nautilus'), 'the Bestanden row', 10):
        h.check('app-notifications: the Bestanden row on Meldingen', False)
        return
    press(h, 'notifications-app-org-gnome-nautilus')
    page = wait(h, lambda: node(h, 'st-app-notifications-enable-sound-alerts'), 'the app page', 20)
    h.check('app-notifications: the page of Bestanden is shown', bool(page))
    if not page:
        return
    header(h, 'app-notifications', ['system', 'notifications'], last=name)
    for key in APP_KEYS:
        h.check(f'app-notifications: st-app-notifications-{key}', node(h, f'st-app-notifications-{key}') is not None)
    h.check('app-notifications: the related link to Geavanceerde opties', node(h, 'st-app-notifications-advanced') is not None)
    h.screenshot(f'app-notifications.{h.lang}')
    no_dutch(h, 'app-notifications')

    def flip(key, start, written, outside, greys=()):
        """A real click on the control of `key` (shown `start`), GNOME's value `written` at the app's path, the
        controls of `greys` insensitive meanwhile; `outside` written from outside brings it back."""
        widget = f'st-app-notifications-{key}'
        h.check(f'app-notifications: {key} starts {"on" if start else "off"}', active(h, widget) is start)
        press(h, widget)
        done = wait(h, lambda: h.gsettings_get(APP_NOTIFY, key, NAUTILUS_PATH) == written, f'{key} {written}', 10)
        h.check(f'app-notifications: a click writes {key} {written} at the app\'s path', bool(done),
                value=h.gsettings_get(APP_NOTIFY, key, NAUTILUS_PATH))
        if greys:
            grey = wait(h, lambda: all(sensitive(h, f'st-app-notifications-{g}') is False for g in greys), 'greyed', 8)
            h.check(f'app-notifications: {", ".join(greys)} greyed meanwhile (GNOME)', bool(grey))
        h.gsettings_set(APP_NOTIFY, key, outside, NAUTILUS_PATH)
        back = wait(h, lambda: active(h, widget) is start and all(sensitive(h, f'st-app-notifications-{g}') is not False
                                                                 for g in greys), f'{key} follows', 15)
        h.check(f'app-notifications: {key} follows {outside} written from outside', bool(back))

    flip('show-banners', True, 'false', 'true', greys=('force-expanded',))
    flip('force-expanded', False, 'true', 'false')
    flip('show-in-lock-screen', True, 'false', 'true', greys=('details-in-lock-screen',))
    # "Inhoud verbergen ..." is details-in-lock-screen inverted
    flip('details-in-lock-screen', True, 'true', 'false')
    flip('enable-sound-alerts', True, 'false', 'true')
    flip('enable', True, 'false', 'true', greys=('show-banners', 'show-in-lock-screen', 'enable-sound-alerts'))
    # the lock-screen rows need Meldingen's own lock-screen key
    h.gsettings_set(NOTIFY, 'show-in-lock-screen', 'false')
    grey = wait(h, lambda: sensitive(h, 'st-app-notifications-show-in-lock-screen') is False
                and sensitive(h, 'st-app-notifications-details-in-lock-screen') is False, 'lock rows greyed', 10)
    h.check('app-notifications: the lock-screen rows grey while show-in-lock-screen is off for all apps', bool(grey))
    h.gsettings_set(NOTIFY, 'show-in-lock-screen', 'true')
    wait(h, lambda: sensitive(h, 'st-app-notifications-show-in-lock-screen') is True, 'lock rows back', 10)
    refused_write(h)


def refused_write(h):
    """A write notifications.app.set refuses (the app was removed after its page opened: not_found) puts the switch
    back and leaves the other rows usable: the page's own copy of the values gets the old value too."""
    apps_dir = h.hm.RUN / 'data' / 'applications'
    apps_dir.mkdir(parents=True, exist_ok=True)
    desktop = apps_dir / REVERT_APP
    desktop.write_text('[Desktop Entry]\nType=Application\nName=WinPoint Revert Test\nExec=true\n', encoding='utf-8')
    listed = wait(h, lambda: app_name(h, REVERT_APP), 'the test app in apps.list', 20)
    h.check('app-notifications (refused write): the test app is installed in the session', bool(listed))
    if not listed:
        desktop.unlink(missing_ok=True)
        return
    deep_link(h, {'page': 'app-notifications', 'id': REVERT_APP})
    page = wait(h, lambda: shows(h, 'WinPoint Revert Test') and node(h, 'st-app-notifications-enable'), 'its page', 20)
    h.check('app-notifications (refused write): the page of the test app is shown', bool(page))
    desktop.unlink()
    gone = wait(h, lambda: app_name(h, REVERT_APP) is None, 'the test app gone', 20)
    if not page or not gone:
        h.check('app-notifications (refused write): the test app was removed', bool(gone))
        return
    path = '/org/gnome/desktop/notifications/application/win11-test-revert/'
    start = active(h, 'st-app-notifications-enable')
    press(h, 'st-app-notifications-enable')
    back = wait(h, lambda: active(h, 'st-app-notifications-enable') is start, 'the switch back', 10)
    h.check('app-notifications (refused write): Meldingen is put back after the refusal', bool(back) and start is True)
    usable = wait(h, lambda: all(sensitive(h, f'st-app-notifications-{k}') is True
                                 for k in ('show-banners', 'show-in-lock-screen', 'enable-sound-alerts')), 'usable', 8)
    h.check('app-notifications (refused write): the other rows stay usable (the old value counts again)', bool(usable),
            sensitive={k: sensitive(h, f'st-app-notifications-{k}') for k in APP_KEYS})
    h.check('app-notifications (refused write): nothing was written at the app\'s path',
            h.gsettings_get(APP_NOTIFY, 'enable', path) is None)


SOUND_JS = r"""(async () => {
  const MT = await import('resource:///org/gnome/shell/ui/messageTray.js');
  const Shell = (await import('gi://Shell')).default;
  if (!globalThis.__wpSoundLog) {
    globalThis.__wpSoundLog = [];
    MT.Sound.prototype.play = function (title) { globalThis.__wpSoundLog.push(String(title ?? '')); };
  }
  const app = Shell.AppSystem.get_default().lookup_app('org.gnome.Nautilus.desktop');
  if (!app) return {error: 'no Nautilus'};
  const source = new MT.Source({title: app.get_name(), policy: MT.NotificationPolicy.newForApp(app)});
  Main.messageTray.add(source);
  const n = new MT.Notification({source, title: TITLE, body: 'WinPoint settings parity F',
    sound: new MT.Sound(null, 'message-new-instant')});
  source.addNotification(n);
  return {ok: true, enableSound: source.policy.enableSound};
})()"""

COUNT_JS = "(globalThis.__wpSoundLog || []).filter(t => t === TITLE).length"


def toast_shown(h, title):
    try:
        return any(n.get('label') == title for n in h.hm.tree_nodes(h.native_state()))
    except Exception:  # noqa: BLE001 (a state read during a redraw)
        return False


def one_notification(h, case, expect_sound, expect_toast):
    title = f'WinPoint sound test {case}'
    sent = h.eval(SOUND_JS.replace('TITLE', json.dumps(title)))
    h.record(f'sound-{case}', sent)
    toast = wait(h, lambda: toast_shown(h, title), f'the toast of {case}', 8) if expect_toast else None
    h.sleep(2.5)
    count = h.eval(COUNT_JS.replace('TITLE', json.dumps(title)))
    if expect_toast:
        h.check(f'sound ({case}): the WinPoint toast shows', bool(toast))
    h.check(f'sound ({case}): {"exactly one sound" if expect_sound else "no sound"}', count == (1 if expect_sound else 0),
            count=count, sent=sent)


def dnd(h, on):
    """Niet storen through settings.json, then wait for the mirror's show-banners: the next outside keyfile write must
    not race the panel host's own write (gsettings_set rewrites the whole file)."""
    set_setting(h, 'notifications.dnd', on)
    wait(h, lambda: h.gsettings_get(NOTIFY, 'show-banners') == ('false' if on else 'true'), 'the mirror\'s show-banners', 10)


def sound_part(h):
    """Amendment 63 (the test of amendment 4)."""
    h.gsettings_set(APP_NOTIFY, 'enable-sound-alerts', 'true', NAUTILUS_PATH)
    set_setting(h, 'notifications.sounds', True)
    dnd(h, False)
    h.sleep(2.5)   # the consumer renews its lease every 2 s with the new values
    one_notification(h, 'on', True, True)
    set_setting(h, 'notifications.sounds', False)
    h.sleep(2.5)
    one_notification(h, 'sounds-off', False, True)
    set_setting(h, 'notifications.sounds', True)
    dnd(h, True)
    h.sleep(2.5)
    one_notification(h, 'dnd', False, False)
    dnd(h, False)
    h.gsettings_set(APP_NOTIFY, 'enable-sound-alerts', 'false', NAUTILUS_PATH)
    h.sleep(2.5)
    one_notification(h, 'app-sound-off', False, True)
    h.gsettings_set(APP_NOTIFY, 'enable-sound-alerts', 'true', NAUTILUS_PATH)


def owned(h, schema, key, setting_path, value):
    """GNOME's look key is WinPoint's: gsettings.get reports it owned by its setting, gsettings.set refuses it."""
    item = ((h.backend('settings', 'gsettings.get', {'keys': [{'schema': schema, 'key': key}]}) or {}).get('items')
            or [{}])[0]
    h.check(f'{schema} {key} is owned by WinPoint\'s {setting_path}', item.get('mode') == 'owned'
            and item.get('setting') == setting_path, mode=item.get('mode'), setting=item.get('setting'))
    refused = h.backend('settings', 'gsettings.set', {'schema': schema, 'values': {key: value}}) or {}
    error = refused.get('error') if isinstance(refused.get('error'), dict) else {}
    h.check(f'gsettings.set refuses {key} (denied gsettings.ownedByWinPoint)', error.get('code') == 'denied'
            and setting_path in json.dumps(error), error=error)


def personalization_part(h):
    nl = h.lang == 'nl'
    visit(h, 'personalization', [], shot=False)
    h.wait_text('settings', 'Achtergrond' if nl else 'Background', timeout=15)
    h.screenshot(f'personalization.{h.lang}')
    # ---- Achtergrond: GNOME's catalogue
    visit(h, 'background', ['personalization'], ready='wp-gnome-0')
    grid = node(h, 'wp-gnome-0')
    h.check('background: GNOME\'s catalogue is shown', bool(grid))
    catalog = h.backend('settings', 'background.catalog') or {}
    items = catalog.get('items') or []
    h.record('catalog', len(items))
    if grid and items:
        first = items[0]
        press(h, 'wp-gnome-0')
        chosen = wait(h, lambda: setting(h, 'wallpaper.path') in (first.get('path'), first.get('dark')),
                      'wallpaper.path set', 10)
        h.check('background: a click on an Ubuntu picture sets wallpaper.path', bool(chosen),
                path=setting(h, 'wallpaper.path'))
        uri = wait(h, lambda: any(p and p in (h.gsettings_get(BACKGROUND, 'picture-uri') or '')
                                  for p in (first.get('path'), first.get('dark'))), 'picture-uri written', 10)
        h.check('background: WinPoint\'s look writes GNOME\'s picture-uri', bool(uri),
                uri=h.gsettings_get(BACKGROUND, 'picture-uri'))
    if node(h, 'background-fit') and choose(h, 'background-fit', 'Uitrekken' if nl else 'Stretch'):
        fit = wait(h, lambda: setting(h, 'wallpaper.fit') == 'stretch'
                   and h.gsettings_get(BACKGROUND, 'picture-options') == "'stretched'", 'stretched', 10)
        h.check('background: Uitrekken sets wallpaper.fit and GNOME\'s picture-options \'stretched\'', bool(fit),
                fit=setting(h, 'wallpaper.fit'), options=h.gsettings_get(BACKGROUND, 'picture-options'))
        choose(h, 'background-fit', 'Opvullen' if nl else 'Fill')
        wait(h, lambda: h.gsettings_get(BACKGROUND, 'picture-options') == "'zoom'", 'zoom', 10)
    owned(h, BACKGROUND, 'picture-uri', 'wallpaper', 'file:///usr/share/backgrounds/warty-final-ubuntu.png')
    # GNOME's Add Picture and remove (the file dialog is the portal's; the host method is what Browse calls)
    added = h.backend('settings', 'background.add', {'path': '/usr/share/backgrounds/warty-final-ubuntu.png'}) or {}
    # the harness's XDG_DATA_HOME is run/data: the copy lands in its backgrounds folder
    h.check('background: background.add copies into the data dir\'s backgrounds folder',
            added.get('copied') is True and '/backgrounds/' in str(added.get('path')), answer=added)
    tile = wait(h, lambda: node(h, 'background-remove-0'), 'the own picture with its remove button', 10)
    h.check('background: the copy is a recent image with a remove button', bool(tile))
    if tile:
        reveal(h, 'background-remove-0')
    h.screenshot(f'background-own.{h.lang}')
    if tile:
        press(h, 'background-remove-0')
        gone = wait(h, lambda: not (h.backend('settings', 'background.catalog') or {}).get('own'), 'the copy removed', 10)
        h.check('background: the remove click moves the copy to the trash', bool(gone))
    # ---- Kleuren: a Windows accent swatch reaches GNOME's accent-color through the mirror
    visit(h, 'colors', ['personalization'], ready='st-colors-accent-color', shot=False)
    accent = node(h, 'st-colors-accent-color')
    h.check('colors: GNOME\'s accent-color is shown', bool(accent and accent.get('label')),
            label=accent and accent.get('label'), key=h.gsettings_get(INTERFACE, 'accent-color'))
    swatch = f'{"Kleur" if nl else "Colour"} #E81123'
    red = wait(h, lambda: next(iter(h.find('settings', label=swatch)), None), 'the red swatch', 8)
    if red:
        press(h, label=swatch)
        took = wait(h, lambda: str(setting(h, 'theme.accent') or '').lower() == '#e81123', 'theme.accent red', 10)
        mirrored = wait(h, lambda: h.gsettings_get(INTERFACE, 'accent-color') == "'red'", 'accent-color red', 10)
        shown = wait(h, lambda: (node(h, 'st-colors-accent-color') or {}).get('label') == ('Rood' if nl else 'Red'),
                     'the row says red', 10)
        h.check('colors: the swatch #E81123 sets theme.accent, the mirror GNOME\'s accent-color \'red\', the row shows it',
                bool(took) and bool(mirrored) and bool(shown), accent=setting(h, 'theme.accent'),
                gnome=h.gsettings_get(INTERFACE, 'accent-color'))
    else:
        h.check('colors: the Windows accent swatch #E81123', False)
    h.screenshot(f'colors.{h.lang}')
    no_dutch(h, 'colors')
    h.gsettings_set(INTERFACE, 'accent-color', "'green'")
    follows = wait(h, lambda: (node(h, 'st-colors-accent-color') or {}).get('label') == ('Groen' if nl else 'Green'),
                   'the row follows', 10)
    h.check('colors: the row follows accent-color written from outside', bool(follows))
    owned(h, INTERFACE, 'accent-color', 'theme.accent', 'teal')
    set_setting(h, 'theme.accent', '#0067c0')
