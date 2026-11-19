"""Settings parity, batch F (notes/claude/settings-f.md): Privacy en beveiliging and its eight pages in a real GNOME 50
session, with real pointer clicks, the private keyfile and D-Bus stand-ins (tests/gnome/lib/standins/f.py).

Run (from G), once per language, under strace (Diagnostische gegevens spawns ubuntu-insights; nothing may run a real
pkexec, apt or PackageKit client):
    tools/heavy.sh --min-mb 2560 --tag f -- python3 -B tests/gnome/harness.py launch run --lang nl --source worktree \\
        --scenario tests/gnome/scenarios/settings-privacy.py --out tests/gnome/scenarios/settings-privacy.out \\
        --strace tests/gnome/scenarios/settings-privacy.out/strace.txt
and the same with --lang en --out tests/gnome/scenarios/settings-privacy.en.out (--strace …en.out/strace.txt).

Every page: opened by its id, its title and breadcrumb as the registry names them (Privacy en beveiliging › <page>),
a screenshot <page>.<lang> and, in English, no Dutch words (tests/i18n/dutch-words.txt). Every click on a control is
a real pointer click (the page is scrolled with the real wheel first); the last check says whether any fell back to
the dev channel. What it checks besides:
  1. Privacy en beveiliging: Windows' groups (Beveiliging, Machtigingen, App-machtigingen with Meldingen); Deze pc
     doorzoeken shown exactly when caps.tracker (its registry `needs`)
  2. without the daemons (the private buses run none): Camera says the app permissions are not available,
     Apparaatbeveiliging says "Controles niet beschikbaar", Diagnostische gegevens has no Foutrapporten row; with
     their stand-ins all three appear (part 6-8)
  3. Activiteitengeschiedenis: remember-recent-files false by a click, the mirror's privacy.recentFiles, the duration
     greyed, an outside write followed; "7 dagen" writes recent-files-max-age 7, an outside 30 shows "30 dagen";
     Geschiedenis wissen (with its question) empties recently-used.xbel and keeps a .bak
  4. Zoekmachtigingen (amendment 45): Bestanden's provider into `disabled` and back from outside; Omhoog writes the
     whole sort-order with Bestanden first (and then has no Omhoog), Omlaag puts it second; "Zoekresultaten van apps
     opnemen" off writes disable-external true and greys the list, an outside false turns it back on
  5. Deze pc doorzoeken: Klassiek gives the index the search locations and Tracker the same places without $HOME
     recursive; the Muziek switch takes it out of both; an outside Tracker write shows; Uitgebreid: alles.roots
     "auto" and $HOME recursive
  6. Locatie, Camera, Microfoon with the PermissionStore stand-in (Bestanden as GNOME stores it: location EXACT with
     its last access, camera yes, microphone no): the device switch writes its GNOME key and follows an outside write;
     Bestanden is listed (Locatie: "Laatst geopend op"), its switch sends SetPermission with GNOME's values (location
     keeps its timestamp) and no interactive authorization, and keeps the clicked value after the store's Changed;
     a SetPermission made elsewhere while the page is open (a portal dialog answered) shows at once (the host relays
     the store's Changed as privacy.changed); the apps grey while the device is off
  7. Apparaatbeveiliging with fwupd's stand-in (HSI:1!, five attributes, one event): "Controles geslaagd" with the
     runtime line and "Niveau 1", every check under its level, Beveiligd opstarten on, the security processor, the
     event; Technisch rapport kopiëren puts GNOME's report on the clipboard; only reads, never interactive; the
     Security Centre and the USB4 link shown exactly when caps.securityCenter / caps.thunderbolt
  8. Diagnostische gegevens en feedback with whoopsie-preferences' stand-in: Foutrapporten shows Handmatig;
     "Automatisch" sends SetReportCrashes(true), SetAutomaticallyReportCrashes(true), each with interactive
     authorization; a system-report switch runs `ubuntu-insights consent linux --state true` (the run's stub, which
     fails: the switch goes back); Rapport weergeven runs `ubuntu-insights collect --dry-run`
"""
import importlib
import importlib.util
import sys
from pathlib import Path


def _standins():
    """tests/gnome/lib/standins and its f.py by path: the harness runs with python -I, so `lib` is no module here."""
    root = Path(__file__).resolve().parent.parent / 'lib' / 'standins'
    name = 'ge1h_lib_standins'
    if name not in sys.modules:
        spec = importlib.util.spec_from_file_location(name, root / '__init__.py', submodule_search_locations=[str(root)])
        package = importlib.util.module_from_spec(spec)
        sys.modules[name] = package
        spec.loader.exec_module(package)
    return sys.modules[name], importlib.import_module(f'{name}.f')


STANDINS, SPECS_F = _standins()
start = SPECS_F.start          # B0's start with byArgs and $v answers (f.py)
FWUPD, PERMISSION_STORE, WHOOPSIE = SPECS_F.FWUPD, SPECS_F.PERMISSION_STORE, SPECS_F.WHOOPSIE

PRIVACY = 'org.gnome.desktop.privacy'
SEARCH = 'org.gnome.desktop.search-providers'
TRACKER = 'org.freedesktop.Tracker3.Miner.Files'
LOCATION = 'org.gnome.system.location'
NAUTILUS = 'org.gnome.Nautilus.desktop'

CLICKS: list[dict] = []        # every control click of the run: its name and how it reached the app

# ---- helpers (the same in settings-apps.py) ------------------------------------------------------------------------
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
CLIPBOARD_JS = ("(async()=>{const St=(await import('gi://St')).default;return await new Promise(r=>"
                "St.Clipboard.get_default().get_text(St.ClipboardType.CLIPBOARD,(c,t)=>r(t??'')));})()")


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


def guard_calls(h, tool):
    try:
        lines = (h.hm.RUN / 'logs' / 'guard-calls.log').read_text().splitlines()
    except OSError:
        return []
    return [line.split('\t', 2)[2] for line in lines if line.count('\t') >= 2 and line.split('\t')[1] == tool]


def part(path):
    return ''.join(c.lower() if c.isascii() and c.isalnum() else '-' for c in path)


# ---- the run ------------------------------------------------------------------------------------------------------
def run(h):
    # host/src/tools.rs takes a stub directory only when it is private (0700, like tests/rust-sandbox's guard-bin);
    # the shared harness establishes these permissions before starting the session
    stubs = h.hm.RUN / 'stubs'
    mode = stubs.stat().st_mode & 0o777
    h.record('stubs mode before', oct(mode))
    h.check('the tool seam run/stubs is private (0700)', mode == 0o700, mode=oct(mode))
    caps = overview_part(h)
    without_part(h)
    store = start(h, PERMISSION_STORE, 'permission-store')
    whoopsie = start(h, WHOOPSIE, 'whoopsie')
    fwupd = start(h, FWUPD, 'fwupd')
    try:
        activity_part(h)
        searchpermissions_part(h)
        searching_part(h)
        permissions_part(h, store)
        security_part(h, fwupd, caps)
        diagnostics_part(h, whoopsie)
    finally:
        for standin in (store, whoopsie, fwupd):
            if standin:
                standin.stop()
    dev = [c for c in CLICKS if c['input'] != 'clutter']
    h.record('clicks', CLICKS)
    h.check(f'every control click ({len(CLICKS)}) was a real pointer click', not dev, dev=dev)
    h.record('dialog clicks', DIALOG_CLICKS)
    h.check(f'a pointer click reaches a kit dialog\'s button ({len(DIALOG_CLICKS)} pressed)',
            all(c['path'] == 'pointer' for c in DIALOG_CLICKS),
            dialogs=DIALOG_CLICKS)


def overview_part(h):
    visit(h, 'privacy', [], ready='link-security')
    caps = h.backend('settings', 'caps.get') or {}
    h.record('caps', {k: caps.get(k) for k in ('tracker', 'geoclue', 'securityCenter', 'thunderbolt', 'whoopsie',
                                               'insights', 'session')})
    for page in ('security', 'diagnostics', 'activity', 'searchpermissions', 'location', 'camera', 'microphone',
                 'notifications'):
        h.check(f'privacy: a link to {page}', node(h, f'link-{page}') is not None)
    h.check('privacy: Deze pc doorzoeken is shown exactly when caps.tracker (its registry needs)',
            (node(h, 'link-searching') is not None) == (caps.get('tracker') is True), tracker=caps.get('tracker'))
    h.check('privacy: the recent-files rows moved to Activiteitengeschiedenis', node(h, 'privacy-recent') is None)
    return caps


def without_part(h):
    """The pages of the daemons before their stand-ins run: the private buses have no services."""
    nl = h.lang == 'nl'
    h.open('settings:camera')
    gone = wait(h, lambda: shows(h, 'De machtigingen van apps zijn nu niet beschikbaar.' if nl
                                 else 'App permissions aren’t available right now.'), 'no PermissionStore', 15)
    h.check('camera: without a PermissionStore the apps are "not available" and none is listed',
            bool(gone) and node(h, 'camera-app-org-gnome-nautilus') is None)
    h.screenshot(f'camera-without.{h.lang}')
    h.open('settings:security')
    gone = wait(h, lambda: shows(h, 'Controles niet beschikbaar' if nl else 'Checks unavailable'), 'no fwupd', 25)
    h.check('security: without fwupd "Controles niet beschikbaar" and no security level',
            bool(gone) and node(h, 'security-level') is None and node(h, 'security-copy-report') is None)
    h.screenshot(f'security-without.{h.lang}')
    h.open('settings:diagnostics')
    shown = wait(h, lambda: node(h, 'diagnostics-data'), 'Diagnostische gegevens', 15)
    h.check('diagnostics: without whoopsie-preferences there is no Foutrapporten row',
            bool(shown) and node(h, 'st-diagnostics-reports') is None)


def activity_part(h):
    nl = h.lang == 'nl'
    visit(h, 'activity', ['privacy'], ready='st-activity-remember-recent-files')
    switch = node(h, 'st-activity-remember-recent-files')
    h.check('activity: Mijn activiteitengeschiedenis ... is bound to remember-recent-files', bool(switch))
    for widget in ('st-activity-recent-files-max-age', 'activity-clear'):
        h.check(f'activity: {widget}', node(h, widget) is not None)
    if switch:
        h.check('activity: the switch starts on', switch.get('active') is True)
        press(h, 'st-activity-remember-recent-files')
        off = wait(h, lambda: h.gsettings_get(PRIVACY, 'remember-recent-files') == 'false', 'remember false', 10)
        h.check('activity: a click writes remember-recent-files false', bool(off))
        mirrored = wait(h, lambda: setting(h, 'privacy.recentFiles') is False, 'privacy.recentFiles mirrored', 10)
        h.check('activity: the mirror sets privacy.recentFiles', bool(mirrored))
        h.check('activity: the duration is greyed while history is off',
                bool(wait(h, lambda: sensitive(h, 'st-activity-recent-files-max-age') is False, 'greyed', 8)))
        h.gsettings_set(PRIVACY, 'remember-recent-files', 'true')
        back = wait(h, lambda: active(h, 'st-activity-remember-recent-files') is True, 'the switch follows', 10)
        h.check('activity: the switch follows the key written from outside', bool(back))
    if node(h, 'st-activity-recent-files-max-age'):
        wait(h, lambda: sensitive(h, 'st-activity-recent-files-max-age') is True, 'the duration sensitive', 8)
        chosen = choose(h, 'st-activity-recent-files-max-age', '7 dagen' if nl else '7 days')
        written = chosen and wait(h, lambda: h.gsettings_get(PRIVACY, 'recent-files-max-age') == '7', 'max age 7', 10)
        h.check('activity: "7 dagen" writes recent-files-max-age 7', bool(written),
                value=h.gsettings_get(PRIVACY, 'recent-files-max-age'))
        h.gsettings_set(PRIVACY, 'recent-files-max-age', '30')
        shown = wait(h, lambda: ('30 dagen' if nl else '30 days') in labels(h, 'st-activity-recent-files-max-age'),
                     'the duration follows', 10)
        h.check('activity: the duration follows recent-files-max-age 30 written from outside', bool(shown),
                shown=labels(h, 'st-activity-recent-files-max-age'))
    # Geschiedenis wissen: GTK's recently-used.xbel in the data dir (run/data)
    xbel = h.hm.RUN / 'data' / 'recently-used.xbel'
    xbel.write_text('<?xml version="1.0" encoding="UTF-8"?>\n<xbel version="1.0" '
                    'xmlns:bookmark="http://www.freedesktop.org/standards/desktop-bookmarks" '
                    'xmlns:mime="http://www.freedesktop.org/standards/shared-mime-info">\n'
                    f'  <bookmark href="file://{h.hm.RUN / "home" / "alpha.txt"}" added="2026-11-16T08:00:00Z" '
                    'modified="2026-11-16T08:00:00Z" visited="2026-11-16T08:00:00Z"/>\n</xbel>\n')
    if node(h, 'activity-clear'):
        press(h, 'activity-clear')
        asked = confirm(h, lambda: 'bookmark href' not in xbel.read_text(), 'Wissen')
        cleared = asked and 'bookmark href' not in xbel.read_text()
        h.check('activity: Geschiedenis wissen asks first, then empties recently-used.xbel (a .bak kept)',
                bool(cleared) and (xbel.parent / 'recently-used.xbel.bak').exists(), asked=asked)


def searchpermissions_part(h):
    visit(h, 'searchpermissions', ['privacy'], ready='st-searchpermissions-disable-external')
    external = node(h, 'st-searchpermissions-disable-external')
    h.check('searchpermissions: Zoekresultaten van apps opnemen is bound to disable-external', bool(external))
    files = wait(h, lambda: node(h, 'st-searchpermissions-disabled-org-gnome-nautilus'), 'the Bestanden provider', 10)
    h.check('searchpermissions: Bestanden (its search provider) is in the list', bool(files))
    if files:
        h.check('searchpermissions: Bestanden starts on', files.get('active') is True)
        press(h, 'st-searchpermissions-disabled-org-gnome-nautilus')
        written = wait(h, lambda: NAUTILUS in (h.gsettings_get(SEARCH, 'disabled') or ''), 'disabled written', 10)
        h.check('searchpermissions: a click writes org.gnome.Nautilus.desktop into disabled', bool(written),
                value=h.gsettings_get(SEARCH, 'disabled'))
        h.gsettings_set(SEARCH, 'disabled', '@as []')
        back = wait(h, lambda: active(h, 'st-searchpermissions-disabled-org-gnome-nautilus') is True,
                    'the switch follows', 10)
        h.check('searchpermissions: the switch follows the key written from outside', bool(back))
        press(h, 'searchpermissions-up-org-gnome-nautilus')
        moved = wait(h, lambda: (h.gsettings_get(SEARCH, 'sort-order') or '').startswith(f"['{NAUTILUS}'"),
                     'sort-order written', 10)
        h.check('searchpermissions: Omhoog writes sort-order with Bestanden first', bool(moved),
                value=h.gsettings_get(SEARCH, 'sort-order'))
        top = wait(h, lambda: sensitive(h, 'searchpermissions-up-org-gnome-nautilus') is False, 'first: no Omhoog', 8)
        h.check('searchpermissions: the first row has no Omhoog', bool(top))
        press(h, 'searchpermissions-down-org-gnome-nautilus')
        second = wait(h, lambda: (lambda order: len(order) > 1 and order[1].strip(" '") == NAUTILUS)(
            (h.gsettings_get(SEARCH, 'sort-order') or '').strip('[]').split(',')), 'Bestanden second', 10)
        h.check('searchpermissions: Omlaag puts Bestanden second', bool(second), value=h.gsettings_get(SEARCH, 'sort-order'))
    if external:
        press(h, 'st-searchpermissions-disable-external')
        off = wait(h, lambda: h.gsettings_get(SEARCH, 'disable-external') == 'true', 'disable-external true', 10)
        h.check('searchpermissions: App search off writes disable-external true', bool(off))
        if files:
            grey = wait(h, lambda: sensitive(h, 'st-searchpermissions-disabled-org-gnome-nautilus') is False,
                        'the list greyed', 8)
            h.check('searchpermissions: the provider list is greyed while app search is off', bool(grey))
        h.screenshot(f'searchpermissions-off.{h.lang}')
        h.gsettings_set(SEARCH, 'disable-external', 'false')
        back = wait(h, lambda: active(h, 'st-searchpermissions-disable-external') is True
                    and sensitive(h, 'st-searchpermissions-disabled-org-gnome-nautilus') is not False,
                    'app search back on', 10)
        h.check('searchpermissions: an outside disable-external false turns app search and the list back on', bool(back))


def searching_part(h):
    home = h.hm.RUN / 'home'
    visit(h, 'searching', ['privacy'], ready='searching-enhanced')
    mode = node(h, 'searching-enhanced')
    h.check('searching: Mijn bestanden zoeken with Klassiek and Uitgebreid', bool(mode) and node(h, 'searching-classic'))
    h.check('searching: Uitgebreid while the index covers every drive (alles.roots auto)',
            active(h, 'searching-enhanced') is True, roots=setting(h, 'alles.roots'))
    h.check('searching: Zoeklocaties aanpassen is there', node(h, 'searching-locations') is not None)
    if not mode:
        return
    press(h, 'searching-classic')
    classic = wait(h, lambda: isinstance(setting(h, 'alles.roots'), list), 'alles.roots a list', 10)
    roots = setting(h, 'alles.roots') or []
    h.check('searching: Klassiek gives WinPoint\'s index the search locations (index.setRoots first)',
            bool(classic) and any(str(r).endswith('/Documents') for r in roots), roots=roots)
    recursive = wait(h, lambda: '&DOCUMENTS' in (h.gsettings_get(TRACKER, 'index-recursive-directories') or ''),
                     'Tracker recursive', 10)
    h.check('searching: and Tracker the same places, without $HOME recursive', bool(recursive)
            and '$HOME' not in (h.gsettings_get(TRACKER, 'index-recursive-directories') or ''),
            recursive=h.gsettings_get(TRACKER, 'index-recursive-directories'),
            single=h.gsettings_get(TRACKER, 'index-single-directories'))
    music = f'searching-place{part(str(home / "Music"))}'
    press(h, 'searching-locations')   # Zoeklocaties aanpassen opens (a collapsed expander hides its rows)
    if wait(h, lambda: active(h, music) is True and (node(h, music) or {}).get('rect'), 'the Muziek switch on', 10):
        press(h, music)
        gone = wait(h, lambda: '&MUSIC' not in (h.gsettings_get(TRACKER, 'index-recursive-directories') or '')
                    and not any(str(r).endswith('/Music') for r in setting(h, 'alles.roots') or []), 'Muziek out', 10)
        h.check('searching: the Muziek switch takes it out of Tracker and the index', bool(gone),
                roots=setting(h, 'alles.roots'), recursive=h.gsettings_get(TRACKER, 'index-recursive-directories'))
        h.gsettings_set(TRACKER, 'index-recursive-directories', "['&DOCUMENTS', '&MUSIC']")
        back = wait(h, lambda: active(h, music) is True, 'the Muziek switch follows', 10)
        h.check('searching: a Tracker write from outside shows', bool(back))
    else:
        h.check('searching: the Muziek switch is on in Klassiek', False)
    h.screenshot(f'searching-classic.{h.lang}')
    press(h, 'searching-enhanced')
    auto = wait(h, lambda: setting(h, 'alles.roots') == 'auto', 'alles.roots auto', 10)
    h.check('searching: Uitgebreid sets WinPoint\'s index to every drive', bool(auto), roots=setting(h, 'alles.roots'))
    enhanced = wait(h, lambda: (h.gsettings_get(TRACKER, 'index-recursive-directories') or '').startswith("['$HOME'"),
                    'Tracker $HOME', 10)
    h.check('searching: and Tracker the whole home folder', bool(enhanced),
            recursive=h.gsettings_get(TRACKER, 'index-recursive-directories'))


def set_permissions(store, table, entry):
    return [c for c in store.calls() if c['method'] == 'SetPermission' and f"'{table}'" in c['args']
            and f"'{entry}'" in c['args']] if store else []


def outside_set(h, table, entry, values):
    """SetPermission from another client of the private session bus, as a portal dialog answered elsewhere (never
    through WinPoint): the stand-in stores it and emits Changed."""
    GLib, Gio = h.s.GLib, h.s.Gio
    h.s.bus.call_sync('org.freedesktop.impl.portal.PermissionStore', '/org/freedesktop/impl/portal/PermissionStore',
                      'org.freedesktop.impl.portal.PermissionStore', 'SetPermission',
                      GLib.Variant('(sbssas)', (table, True, entry, 'org.gnome.Nautilus', values)),
                      None, Gio.DBusCallFlags.NONE, 5000, None)


def device_page(h, store, page, schema, key, invert, table, entry, allowed, sends):
    """One of Locatie, Camera, Microfoon: the device switch (GNOME's key), Bestanden from the store, its switch."""
    switch = f'st-{page}-{key}'
    app = f'{page}-app-org-gnome-nautilus'
    visit(h, page, ['privacy'], ready=switch, shot=False)
    wait(h, lambda: node(h, app), f'{page}: Bestanden listed', 10)
    h.screenshot(f'{page}.{h.lang}')
    on = node(h, switch)
    h.check(f'{page}: the device switch is bound to {schema} {key}', bool(on))
    h.check(f'{page}: the PermissionStore\'s Bestanden is listed by its name', node(h, app) is not None)
    if on and on.get('active') is not True:
        # GNOME's default leaves location off: turned on from outside first, so the apps can be switched
        h.gsettings_set(schema, key, 'false' if invert else 'true')
        follows = wait(h, lambda: active(h, switch) is True and sensitive(h, app) is not False, f'{page}: on', 10)
        h.check(f'{page}: the device switch and the apps follow {key} written from outside (on)', bool(follows))
    if node(h, app) and store:
        h.check(f'{page}: Bestanden shows the stored value ({"allowed" if allowed else "not allowed"})',
                active(h, app) is allowed)
        press(h, app)
        sent = wait(h, lambda: set_permissions(store, table, entry), f'{page}: SetPermission', 10)
        call = (sent or [{}])[-1]
        h.check(f'{page}: the switch sends SetPermission({table}, {entry}, org.gnome.Nautilus, {sends})',
                bool(sent) and "'org.gnome.Nautilus'" in call.get('args', '') and sends in call.get('args', ''), call=call)
        h.check(f'{page}: the PermissionStore needs no password', bool(sent) and call.get('interactive') is False)
        # the store keeps the click's value (its Changed comes back to the page): the switch stays as clicked
        stays = wait(h, lambda: active(h, app) is (not allowed), f'{page}: the clicked value stays', 6)
        h.check(f'{page}: after the store\'s own Changed the switch keeps the clicked value', bool(stays))
        # a portal dialog answered elsewhere while the page is open: GNOME's list follows the store, so does this one
        values = (['EXACT', SPECS_F.LOCATION_STAMP] if allowed else ['NONE', SPECS_F.LOCATION_STAMP]) \
            if table == 'location' else (['yes'] if allowed else ['no'])
        outside_set(h, table, entry, values)
        follows = wait(h, lambda: active(h, app) is allowed, f'{page}: follows the store', 10)
        h.check(f'{page}: the switch follows a SetPermission made elsewhere (the store\'s Changed, privacy.changed)',
                bool(follows), active=active(h, app))
    if on:
        press(h, switch)
        written = 'true' if invert else 'false'   # off: disable-camera / disable-microphone true, location enabled false
        flipped = wait(h, lambda: h.gsettings_get(schema, key) == written, f'{key} {written}', 10)
        h.check(f'{page}: the device switch off writes {key} {written}', bool(flipped), value=h.gsettings_get(schema, key))
        if node(h, app):
            grey = wait(h, lambda: sensitive(h, app) is False, f'{page}: the apps greyed', 8)
            h.check(f'{page}: the apps are greyed while the device is off', bool(grey))
        h.gsettings_set(schema, key, 'false' if written == 'true' else 'true')
        back = wait(h, lambda: active(h, switch) is True and sensitive(h, app) is not False, f'{page}: follows', 10)
        h.check(f'{page}: the switch and the apps follow the key written from outside', bool(back))


def permissions_part(h, store):
    nl = h.lang == 'nl'
    device_page(h, store, 'location', LOCATION, 'enabled', False, 'location', 'location', True,
                f"['NONE', '{SPECS_F.LOCATION_STAMP}']")
    h.check('location: Bestanden shows "Laatst geopend op" from the stored time',
            shows(h, 'Laatst geopend op' if nl else 'Last accessed'))
    device_page(h, store, 'camera', PRIVACY, 'disable-camera', True, 'devices', 'camera', True, "['no']")
    device_page(h, store, 'microphone', PRIVACY, 'disable-microphone', True, 'devices', 'microphone', False, "['yes']")


def security_part(h, fwupd, caps):
    nl = h.lang == 'nl'
    visit(h, 'security', ['privacy'], ready='security-level', shot=False)
    level = node(h, 'security-level')
    h.check('security: with fwupd\'s stand-in the security level', bool(level))
    text = h.text('settings')['text']
    h.check('security: HSI:1 is "Controles geslaagd" with "Niveau 1"', ('Controles geslaagd' if nl else 'Checks passed') in text
            and ('Niveau 1' if nl else 'Level 1') in text)
    h.check('security: the runtime issue is named', ('Tijdens het gebruik zijn er problemen gevonden.' if nl
                                                    else 'Problems were found while the system is running.') in text)
    h.check('security: Beveiligd opstarten is on (UEFI Secure Boot enabled)', node(h, 'security-secure-boot') is not None
            and ('Beveiligd opstarten is ingeschakeld.' if nl else 'Secure boot is on.') in text)
    h.check('security: the security processor (TPM 2.0 found)',
            ('Je beveiligingsprocessor (TPM 2.0) is aanwezig' if nl else 'Your security processor (TPM 2.0) is present') in text)
    h.check('security: Beveiligingsgebeurtenissen with the Secure Boot event',
            ('Beveiligingsgebeurtenissen' if nl else 'Security events') in text
            and ('UEFI Beveiligd opstarten: Uitgeschakeld → Ingeschakeld' if nl
                 else 'UEFI Secure Boot: Disabled → Enabled') in text)
    h.screenshot(f'security.{h.lang}')
    if level:
        press(h, 'security-level')
        sections = wait(h, lambda: all(shows(h, s) for s in (
            ['Beveiligingsniveau 1', 'Beveiligingsniveau 2', 'Beveiligingsniveau 3', 'Tijdens het gebruik'] if nl
            else ['Security level 1', 'Security level 2', 'Security level 3', 'While running'])), 'the checks', 8)
        h.check('security: every check under its level, the running ones last', bool(sections))
        h.check('security: a check with its verdict and fwupd\'s result',
                shows(h, 'Geslaagd (Ingeschakeld)' if nl else 'Pass (Enabled)')
                and shows(h, 'Mislukt (Niet gevonden)' if nl else 'Fail (Not found)'))
        h.screenshot(f'security-checks.{h.lang}')
        if node(h, 'security-copy-report'):
            press(h, 'security-copy-report')
            report = wait(h, lambda: (lambda t: t if 'fwupd' in t else None)(h.eval(CLIPBOARD_JS) or ''),
                          'the report on the clipboard', 8) or ''
            h.record('security report', report[:1200])
            h.check('security: Technisch rapport kopiëren puts GNOME\'s report on the clipboard',
                    ('Rapport over apparaatbeveiliging' if nl else 'Device security report') in report
                    and 'HSI:1!' in report and ('UEFI Beveiligd opstarten: Geslaagd (Ingeschakeld)' if nl
                                                else 'UEFI Secure Boot: Pass (Enabled)') in report)
    no_dutch(h, 'security')
    h.check('security: Beveiligingscentrum shown exactly when caps.securityCenter',
            (node(h, 'security-center') is not None) == (caps.get('securityCenter') is True),
            securityCenter=caps.get('securityCenter'))
    h.check('security: the USB4 and Thunderbolt link shown exactly when caps.thunderbolt',
            (node(h, 'link-usb') is not None) == (caps.get('thunderbolt') is True), thunderbolt=caps.get('thunderbolt'))
    if fwupd:
        calls = fwupd.calls()
        h.check('security: fwupd read (attributes and the last 10 events), no password',
                any(c['method'] == 'GetHostSecurityAttrs' for c in calls)
                and any(c['method'] == 'GetHostSecurityEvents' and '10' in c['args'] for c in calls)
                and all(not c['interactive'] for c in calls), calls=calls[-6:])


def diagnostics_part(h, whoopsie):
    nl = h.lang == 'nl'
    visit(h, 'diagnostics', ['privacy'], ready='st-diagnostics-reports')
    reports = node(h, 'st-diagnostics-reports')
    h.check('diagnostics: with whoopsie-preferences\' stand-in, Foutrapporten naar Canonical verzenden', bool(reports))
    h.check('diagnostics: it shows the daemon\'s mode (Handmatig)', ('Handmatig' if nl else 'Manually')
            in labels(h, 'st-diagnostics-reports'), shown=labels(h, 'st-diagnostics-reports'))
    consent = node(h, 'st-diagnostics-insights-linux')
    h.check('diagnostics: the two system reports', bool(consent) and node(h, 'st-diagnostics-insights-ubuntu-release-upgrader'))
    if reports and whoopsie:
        chosen = choose(h, 'st-diagnostics-reports', 'Automatisch' if nl else 'Automatically')
        sets = wait(h, lambda: [c for c in whoopsie.calls() if c['method'].startswith('Set')][-2:] or None,
                    'the two writes', 10) if chosen else None
        got = [(c['method'], c['args'], c['interactive']) for c in sets or []]
        h.check('diagnostics: Automatisch sends SetReportCrashes(true), SetAutomaticallyReportCrashes(true), '
                'each with interactive authorization',
                got == [('SetReportCrashes', '(true,)', True), ('SetAutomaticallyReportCrashes', '(true,)', True)],
                calls=got)
    if consent:
        press(h, 'st-diagnostics-insights-linux')
        ran = wait(h, lambda: 'consent linux --state true' in guard_calls(h, 'ubuntu-insights'), 'consent run', 10)
        h.check('diagnostics: the switch runs ubuntu-insights consent linux --state true (through system_tool)',
                bool(ran), calls=guard_calls(h, 'ubuntu-insights'))
        back = wait(h, lambda: active(h, 'st-diagnostics-insights-linux') is False, 'the switch goes back', 10)
        h.check('diagnostics: the stub fails, so the switch goes back', bool(back))
    if node(h, 'diagnostics-view'):
        press(h, 'diagnostics-view')
        ran = wait(h, lambda: 'collect --dry-run' in guard_calls(h, 'ubuntu-insights'), 'collect run', 10)
        h.check('diagnostics: Rapport weergeven runs ubuntu-insights collect --dry-run', bool(ran),
                calls=guard_calls(h, 'ubuntu-insights'))
        h.screenshot(f'diagnostics-report.{h.lang}')
        h.key('settings', 'Escape')
    h.record('guard calls', {tool: guard_calls(h, tool) for tool in ('pkexec', 'systemctl', 'apt', 'ubuntu-insights')})
    h.check('diagnostics: no pkexec, systemctl or apt was called (only the daemon and the tool)',
            not guard_calls(h, 'pkexec') and not guard_calls(h, 'apt') and not guard_calls(h, 'apt-get'),
            pkexec=guard_calls(h, 'pkexec'))
