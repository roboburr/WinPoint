"""Settings parity batch B (notes/claude/settings-parity.md §2, §7; plan.json "testing", GNOME HARNESS): Bluetooth en
apparaten, Apparaten, Automatisch afspelen, USB, Printers en scanners, a printer's page and Kleurprofiel in the private
GNOME, with the stand-ins of tests/gnome/lib/standins/b.py and tests/gnome/lib/fake_ipp.py.

Run (from the worktree; with --strace because the Bluetooth page can reach rfkill and, through flight mode, nmcli):
    tools/heavy.sh --min-mb 2560 --tag b -- python3 -B tests/gnome/harness.py launch run --lang nl --source worktree \\
        --scenario tests/gnome/scenarios/settings-devices.py --out tests/gnome/scenarios/settings-devices.out \\
        --strace tests/gnome/scenarios/settings-devices.out/strace.txt
and the same with --lang en --out tests/gnome/scenarios/settings-devices.en.out (--strace inside it); the strace logs
stay local (results.json has their straceConnect and straceExec verdicts).

Every page: opened with h.open('settings:<id>') (the printer's data page by a real click on its card,
h.settings_data_page), a screenshot '<id>.<lang>' (more of a page: '<id>-<what>.<lang>'), the header checked against
registry.rs (the title of a top-level page, else the breadcrumb of the parent chain: crumb-<id> buttons and the current
page or the data page's item name), Settings in the run's language and, in en, no Dutch word on the page.

  1. Bluetooth en apparaten without BlueZ (the private buses have none): the no-Bluetooth state still lists Windows'
     rows of the page (Apparaten, Printers en scanners, Muis, Automatisch afspelen); USB is listed, and found by the
     search, exactly when caps report Thunderbolt (this machine has none, so it is hidden; step 4 opens its deep link).
  2. Automatisch afspelen, every bound control by its st-autoplay-* name with real pointer clicks and GNOME's exact
     value in the run's keyfile: the switch is the inverse of autorun-never; Verwisselbaar station is GNOME's fixed
     "Altijd vragen" (nothing in GNOME 50 reads automount-open, and gsettings.set refuses it); each media row
     (Geheugenkaart, Audio-cd, Dvd-video, Muziekspeler, Software) moves its type into one of the three autorun lists
     (GNOME's autorun_set_preferences); outside writes of autorun-never and autorun-x-content-ignore move the controls
     (gsettings.changed); with AutoPlay off the rows, Verwisselbaar station too, are insensitive; Andere mediatypen
     opens GNOME's Other Media Types dialog. allow/devices.rs has no Owned key (nothing to point to a
     WinPoint page) and the sandbox has no /etc/glib-2.0 locks (README: lockdown is a host unit test).
  3. Bluetooth en apparaten with BlueZ and gsd's Rfkill: the switch on, "Detecteerbaar als “WinPoint-pc”" (BlueZ's
     Alias) and Discoverable set while the page shows (the lease, ended when the page goes and when the window closes
     on the page); the paired devices as
     tiles; the switch off is amendment 57 (gsd's BluetoothAirplaneMode true; the stand-in's adapter does not follow
     gsd, so the host then sets BlueZ's Powered false itself) and on again; flight mode on shows GNOME's notice.
  4. Apparaten with BlueZ and UPower: Windows' groups (the Bluetooth mouse with BlueZ's 70 % and the receiver mouse
     with UPower's 55 % under Muis, toetsenbord en pen; headphones under Audio; the phone under Overige apparaten), the
     ⋯ menu's Verbinding verbreken (Device1.Disconnect), Eigenschappen (GNOME's device dialog, its Bestanden verzenden
     runs bluetooth-sendto --device=<address> --name=<name>, the stub's argv) and Apparaat verwijderen (Windows'
     question, then Adapter1.RemoveDevice(<path>)).
  5. USB with bolt (by its deep link, also where caps report no Thunderbolt): Directe toegang shows AuthMode; the
     pending dock's "Autoriseren en verbinden" (a real click) sends EnrollDevice(uid, 'default', '') with interactive
     authorization, the switch off sends Properties.Set AuthMode 'disabled'.
  6. Printers en scanners with fake_ipp.py as the scheduler and a cups-pk-helper stand-in: both queues, the stopped one
     with GNOME's reason; "Apparaat toevoegen" lists what DevicesGet found (interactive); the driverless printer is
     added as PrinterAdd(name, uri, 'everywhere', info, location), PrinterSetEnabled, PrinterSetAcceptJobs and
     PrinterAddOptionDefault media a4 (amendment 41), each interactive; "Scanner openen" (caps.simpleScan) starts
     Documentscanner (the simple-scan stub).
  7. A printer's page: Testpagina afdrukken (a Print-Job of GNOME's test page as the user), the queue and pausing a job
     (JobSetHoldUntil(id, 'indefinite'), interactive), the PPD's preferences, Hervatten (PrinterSetEnabled and
     PrinterSetAcceptJobs) and Als standaard instellen (PrinterSetDefault, interactive).
  8. Kleurprofiel with colord: the display's expander open with its default profile; the Kleurprofiel drop-down sends
     Device.MakeProfileDefault(<profile>), the switch off Device.SetEnabled(false), both interactive; without a colour
     sensor "Kalibreren…" is not offered (a notice says why), with the colord stand-in that has one it is. This
     GNOME's own Mutter: org.gnome.Mutter.ColorManager's CalibrateMonitor and its session's GammaLutSize,
     SetCrtcGammaLut and Stop have the signatures devices/color.rs calls (the calibration's gamma).

Buttons inside a kit dialog get a real pointer click first and, when that does nothing, the dev channel's click by
name, so the action itself is still checked; one check (known B-dialog-click, note 9 of notes/claude/settings-b.md)
records whether the real clicks reached them.
"""
from __future__ import annotations

import json
import os
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from lib.standins import start  # noqa: E402
from lib.standins.b import (ALIAS, BLUEZ, BOLT, CD_CALIB, CD_CALIB_TITLE, COLORD, COLORD_SENSOR,  # noqa: E402
                            CUPSPKHELPER, DEV, DOCK_UID, GSD_RFKILL, GSD_RFKILL_FLIGHT, HEADPHONES, MOUSE, NAMES, PHONE,
                            UPOWER, start_objects)

MEDIA = 'org.gnome.desktop.media-handling'
TITLE = {
    'nl': {'ignore': 'Geen actie ondernemen', 'folder': 'Map openen om bestanden weer te geven', 'ask': 'Altijd vragen',
           'paper': 'Papier op', 'resume': 'Hervatten', 'test': 'Testpagina', 'search': 'Zoekresultaten',
           'discoverable': f'Detecteerbaar als “{ALIAS}”', 'off': 'Bluetooth staat uit',
           'flight': 'Vliegtuigmodus staat aan', 'connected': 'Verbonden'},
    'en': {'ignore': 'Take no action', 'folder': 'Open folder to view files', 'ask': 'Ask me every time',
           'paper': 'Out of paper', 'resume': 'Resume', 'test': 'Test page', 'search': 'Search results',
           'discoverable': f'Discoverable as “{ALIAS}”', 'off': 'Bluetooth is off',
           'flight': 'Flight mode is on', 'connected': 'Connected'},
}
# registry.rs: each page's parent chain (the breadcrumb before the page's own title)
PARENTS = {'bluetooth': [], 'bluetooth-devices': ['bluetooth'], 'autoplay': ['bluetooth'], 'usb': ['bluetooth'],
           'printers': ['bluetooth'], 'printer': ['bluetooth', 'printers'], 'colorprofile': ['system', 'display']}
IPP_PORT = 8631
DIALOG_CLICKS: list[dict] = []


# ---- helpers --------------------------------------------------------------------------------------------------

def fake_cups(h):
    """fake_ipp.py as the scheduler on the sandbox's own loopback (its own network namespace, so the port is free),
    found by libcups through the private $HOME/.cups/client.conf; None (a failed check) when it does not start."""
    home = h.hm.RUN / 'home'
    (home / '.cups').mkdir(parents=True, exist_ok=True)
    (home / '.cups' / 'client.conf').write_text(f'ServerName 127.0.0.1:{IPP_PORT}\n')
    log = h.hm.RUN / 'logs' / 'fake-ipp.jsonl'
    script = Path(__file__).resolve().parent.parent / 'lib' / 'fake_ipp.py'
    h.spawn('fake-ipp', ['python3', '-B', '-I', str(script), '--port', str(IPP_PORT), '--log', str(log)])
    out = h.hm.RUN / 'logs' / 'app-fake-ipp.log'
    ready = wait(h, lambda: out.exists() and 'ready' in out.read_text(), 'fake_ipp.py listens', 10)
    h.check('fake_ipp.py listens on the loopback', bool(ready))
    return log if ready else None


def ipp_requests(log) -> list[dict]:
    try:
        return [json.loads(line) for line in log.read_text().splitlines() if line.strip()]
    except OSError:
        return []


def calls_of(standin, method: str) -> list[dict]:
    return [c for c in standin.calls() if c['method'] == method]


def stub_calls(h, tool: str) -> list[str]:
    """The argv (after the name) of every call of a run/stubs tool (logs/guard-calls.log: time, name, args)."""
    try:
        lines = (h.hm.RUN / 'logs' / 'guard-calls.log').read_text().splitlines()
    except OSError:
        return []
    return [(line.split('\t', 2) + [''])[2] for line in lines if line.split('\t')[1:2] == [tool]]


def texts(node: dict) -> list[str]:
    """Every label below a node of the dev state (a drop-down's shown choice)."""
    out, todo = [], [node]
    while todo:
        n = todo.pop()
        if n.get('label') and n.get('type') == 'GtkLabel':
            out.append(n['label'])
        todo.extend(n.get('children') or [])
    return out


def names_below(node: dict | None) -> set[str]:
    out, todo = set(), [node] if node else []
    while todo:
        n = todo.pop()
        if n.get('name'):
            out.add(n['name'])
        todo.extend(n.get('children') or [])
    return out


def one(h, name: str):
    found = h.find('settings', name)
    return found[0] if found else None


def wait(h, fn, label: str, timeout: float = 10):
    """h.until without the exception: None when it timed out (the check after it fails)."""
    try:
        return h.until(fn, label, timeout)
    except TimeoutError:
        return None


def click(h, name: str, **kw) -> bool:
    """A real click on a Settings widget; a missing widget is a failed check, not the end of the scenario."""
    try:
        h.click('settings', name, **kw)
        return True
    except (LookupError, RuntimeError) as error:
        h.check(f'click {name}', False, error=str(error))
        return False


def goto(h, pid: str, wait_for: str, label: str, timeout: float = 20):
    """Show a page (h.open; when the harness's last page is this one already, another page first so it is built
    again) and wait for one of its widgets."""
    sf = h.surfaces.get('settings')
    if sf and sf.alive() and sf.page == pid:
        h.open('settings:home')
    h.open(f'settings:{pid}')
    node = wait(h, lambda: one(h, wait_for), f'{label}: {wait_for}', timeout)
    h.check(f'{label} shows {wait_for}', bool(node))
    return node


def header(h, pid: str, item: str | None = None, timeout: float = 10) -> bool:
    """Settings' header as a user reads it, against registry.rs: the title of a top-level page, else the breadcrumb of
    its parent chain (crumb-<id> buttons, then the current page or a data page's item name)."""
    titles = h.session.titles
    parents = PARENTS[pid]
    want = [titles[p][h.lang] for p in parents] + [item or titles[pid][h.lang]]
    seen: dict = {}

    def shown():
        heads = h.find('settings', cls='st-title')
        if not heads:
            return None
        nodes = h.hm.tree_nodes([{'tree': heads[0]}])
        crumbs = [n for n in nodes if str(n.get('name') or '').startswith('crumb-')]
        current = [n.get('label') for n in nodes if 'w11-breadcrumb-current' in (n.get('classes') or [])]
        got = [n.get('label') for n in crumbs] + current if crumbs or current else [heads[0].get('label')]
        ids = [n['name'][len('crumb-'):] for n in crumbs]
        last = next((n for n in nodes if 'w11-breadcrumb-current' in (n.get('classes') or [])), None)
        seen.update(shown=got, ids=ids, currentRect=last and last.get('rect'))
        return got == want and ids == parents
    ok = bool(wait(h, shown, f'{pid}: header', timeout))
    h.check(f'{pid}: title and breadcrumb {" › ".join(want)}', ok, want=want, **seen)
    if parents and seen.get('currentRect'):
        # the current crumb must be readable: a label ellipsized to "…" is about 20 px wide; the title font (28 px)
        # needs about 14 px a character, so half of that is a safe floor (note 11 for I: the printer's page)
        width, need = seen['currentRect'][2], 7 * len(want[-1])
        h.check(f'{pid}: the current crumb is shown in full, not as "…"', width >= need, width=width, need=need)
    return ok


def words(h, pid: str) -> None:
    """Settings shows the run's language; in en no Dutch word (tests/i18n/dutch-words.txt) is on the page."""
    text = h.page_text('settings')
    h.check(f'{pid}: Settings is in {h.lang}', text['lang'] == h.lang, lang=text['lang'])
    if h.lang == 'en':
        h.check(f'{pid}: no Dutch words in en', not text['dutchWordHits'], hits=text['dutchWordHits'])


def shot(h, pid: str, what: str = '') -> None:
    h.screenshot(f'{pid}{"-" + what if what else ""}.{h.lang}', 'settings')


def check_page(h, pid: str, item: str | None = None, what: str = '') -> None:
    header(h, pid, item)
    words(h, pid)
    shot(h, pid, what)


def option(h, label: str):
    """The open drop-down list's option with that label (a w11-list-item button of the popover)."""
    return next((n for n in h.find('settings', cls='w11-list-item')
                 if label in [k.get('label') for k in h.hm.tree_nodes([{'tree': n}])]), None)


def choose(h, name: str, label: str, done=None, timeout: float = 8) -> bool:
    """A kit drop-down by real pointer clicks: its button, then the option with that label in its list; True when the
    option was clicked and ``done()`` held (when given)."""
    if not click(h, name):
        return False
    opt = wait(h, lambda: option(h, label), f'{name}: option {label!r}', 5)
    if not opt:
        h.check(f'{name} lists {label!r}', False, texts=texts(one(h, name) or {}))
        h.key('settings', 'Escape')
        return False
    h.click('settings', node=opt)
    return bool(wait(h, done, f'{name}: {label!r} chosen', timeout)) if done else True


def dialog_click(h, name: str, done, label: str) -> bool:
    """A button inside a kit dialog: a real pointer click, and when ``done()`` stays false the dev channel's click by
    name (note 9 of notes/claude/settings-b.md: real clicks never reach a kit dialog's buttons), so the action itself
    is checked either way; each attempt is listed in the B-dialog-click check at the end."""
    try:
        real = h.click('settings', name)
    except (LookupError, RuntimeError) as error:
        real = {'error': str(error)}
    ok = wait(h, done, f'{label} (real click)', 4)
    path = 'real'
    if not ok:
        path = 'dev'
        try:
            h.click('settings', name, real=False)
        except (LookupError, RuntimeError) as error:
            h.check(f'dev click {name}', False, error=str(error))
        ok = wait(h, done, f'{label} (dev click)', 10)
    DIALOG_CLICKS.append({'button': name, 'action': label, 'path': path, 'done': bool(ok),
                          'pointer': {k: real.get(k) for k in ('x', 'y', 'rect', 'window', 'error') if k in real}})
    return bool(ok)


def strv(value: str | None) -> list[str] | None:
    """A keyfile string list (GVariant text, '@as []' or "['a', 'b']") as a list; None for GNOME's default."""
    if value is None:
        return None
    text = value.removeprefix('@as').strip()
    return [] if text == '[]' else [part.strip().strip("'") for part in text.strip('[]').split(',')]


def schema_default(schema: str, key: str) -> str | None:
    """The key's default as GVariant text, from the installed schema files (Ubuntu overrides some of GNOME's: here
    autorun-never is true and start-app lists two types); no dconf, no bus."""
    try:
        from gi.repository import Gio
        source = Gio.SettingsSchemaSource.get_default()
        found = source.lookup(schema, True) if source else None
        return found.get_key(key).get_default_value().print_(False) if found and found.has_key(key) else None
    except Exception:   # noqa: BLE001 - a missing schema is a failed check later
        return None


def media(h, key: str) -> str | None:
    """The key's value in effect: the run's keyfile, else the schema's default."""
    value = h.gsettings_get(MEDIA, key)
    return value if value is not None else schema_default(MEDIA, key)


# ---- the steps ------------------------------------------------------------------------------------------------

def bluetooth_without_bluez(h, T) -> dict:
    goto(h, 'bluetooth', 'link-autoplay', 'Bluetooth en apparaten')
    check_page(h, 'bluetooth', what='nobluez')
    caps = h.backend('settings', 'caps.get') or {}
    h.record('caps', {k: caps.get(k) for k in ('session', 'gnome', 'thunderbolt', 'colorSensor', 'simpleScan')})
    links = {n: bool(one(h, n)) for n in ('link-bluetooth-devices', 'link-printers', 'link-mouse', 'link-autoplay')}
    h.record('bluetooth.links', links)
    h.check('without Bluetooth the page still lists its sub-pages', all(links.values()), links=links)
    h.check('without BlueZ the page has no Bluetooth switch', not one(h, 'bt-toggle'))
    thunderbolt = caps.get('thunderbolt') is True
    h.check('USB is listed exactly when caps report Thunderbolt', bool(one(h, 'link-usb')) == thunderbolt,
            thunderbolt=caps.get('thunderbolt'))
    # the search leaves a page out too while its caps key is false
    h.type_text('settings', 'settings-search', 'usb')
    h.key('settings', 'Return', name='settings-search')
    shown = wait(h, lambda: any(n.get('label') == T['search'] for n in h.find('settings', cls='st-title')),
                 'the search results', 10)
    results = sorted(n['name'] for n in h.find('settings', where=lambda n: str(n.get('name') or '').startswith('result-')))
    h.record('search.usb', results)
    h.check('the search for "usb" finds the USB page exactly when caps report Thunderbolt',
            bool(shown) and ('result-usb' in results) == thunderbolt, results=results, thunderbolt=thunderbolt)
    h.screenshot(f'search-usb.{h.lang}', 'settings')
    h.type_text('settings', 'settings-search', '')
    return caps


def autoplay(h, T) -> None:
    before = media(h, 'autorun-never')
    sw = goto(h, 'autoplay', 'st-autoplay-autorun-never', 'Automatisch afspelen')
    check_page(h, 'autoplay')
    h.record('autoplay.before', {'key': h.gsettings_get(MEDIA, 'autorun-never'), 'effective': before,
                                 'switch': sw and sw.get('active')})
    h.check('the switch shows the inverse of autorun-never', bool(sw) and sw.get('active') is (before == 'false'),
            key=before, switch=sw and sw.get('active'))
    # the switch: a real click writes autorun-never; AutoPlay on for the rows below
    click(h, 'st-autoplay-autorun-never')
    want = 'false' if before == 'true' else 'true'
    after = wait(h, lambda: (lambda v: v if v == want else None)(h.gsettings_get(MEDIA, 'autorun-never')),
                 f'autorun-never={want} in the keyfile', 10)
    h.check('a click on the switch writes autorun-never', after == want, before=before, after=after)
    if after == 'true':
        click(h, 'st-autoplay-autorun-never')
        wait(h, lambda: h.gsettings_get(MEDIA, 'autorun-never') == 'false', 'autorun-never=false', 10)
    dcf = wait(h, lambda: (lambda n: n if n and n.get('sensitive') else None)(one(h, 'st-autoplay-image-dcf')),
               'the Geheugenkaart row is sensitive with AutoPlay on', 10)
    h.check('with AutoPlay on the media rows are sensitive', bool(dcf))
    # outside writes of autorun-never move the switch (gsettings.changed)
    h.gsettings_set(MEDIA, 'autorun-never', 'true')
    off = wait(h, lambda: (lambda n: n if n and n.get('active') is False else None)(one(h, 'st-autoplay-autorun-never')),
               'the switch follows autorun-never=true', 10)
    h.gsettings_set(MEDIA, 'autorun-never', 'false')
    on = wait(h, lambda: (lambda n: n if n and n.get('active') is True else None)(one(h, 'st-autoplay-autorun-never')),
              'the switch follows autorun-never=false', 10)
    h.check('outside writes of autorun-never move the switch', bool(off) and bool(on))

    # Verwisselbaar station (review B finding 7): gnome-shell 50 asks for a drive without a content type whatever
    # automount-open says (nothing reads that key any more), so the row shows GNOME's fixed "Altijd vragen", no control
    # is bound to the key and the host refuses to write it
    removable = one(h, 'st-autoplay-removable')
    h.check('Verwisselbaar station shows GNOME\'s fixed choice, Altijd vragen', bool(removable)
            and T['ask'] in texts(removable), texts=removable and texts(removable))
    h.check('no control is bound to automount-open', not one(h, 'st-autoplay-automount-open'))
    before_key = h.gsettings_get(MEDIA, 'automount-open')
    refused = h.backend('settings', 'gsettings.set', {'schema': MEDIA, 'values': {'automount-open': False}})
    h.check('gsettings.set refuses automount-open (in no writing mode of allow/devices.rs)',
            isinstance(refused, dict) and bool(refused.get('error'))
            and h.gsettings_get(MEDIA, 'automount-open') == before_key, answer=refused)

    # an outside write of the ignore list moves Geheugenkaart to Geen actie ondernemen
    h.gsettings_set(MEDIA, 'autorun-x-content-ignore', "['x-content/image-dcf']")
    shown = wait(h, lambda: T['ignore'] in texts(one(h, 'st-autoplay-image-dcf') or {}),
                 'Geheugenkaart follows the outside write', 10)
    h.check('an outside write moves the Geheugenkaart drop-down to Geen actie ondernemen', bool(shown),
            texts=texts(one(h, 'st-autoplay-image-dcf') or {}))

    # every media row by a real click: GNOME's autorun_set_preferences, the type out of all three lists and into one
    lists = {key: strv(media(h, key)) or [] for key in ('autorun-x-content-start-app', 'autorun-x-content-ignore',
                                                         'autorun-x-content-open-folder')}
    h.record('autoplay.lists', lists)
    into = {'folder': 'autorun-x-content-open-folder', 'ignore': 'autorun-x-content-ignore'}
    # each row to a choice other than the one it shows (a drop-down sends nothing for its current option)
    for mime, action in (('image-dcf', 'folder'), ('audio-cdda', 'ignore'), ('video-dvd', 'ignore'),
                         ('audio-player', 'folder'), ('unix-software', 'ignore')):
        full = f'x-content/{mime}'
        for key in lists:
            lists[key] = [m for m in lists[key] if m != full]
        if action in into:
            lists[into[action]].append(full)
        expect = {k: list(v) for k, v in lists.items()}
        got: dict = {}

        def written():
            got.update({k: strv(h.gsettings_get(MEDIA, k)) for k in expect})
            return got == expect
        ok = choose(h, f'st-autoplay-{mime}', T[action], written)
        h.check(f'{mime} › {T[action]} writes GNOME\'s three lists', ok, want=expect, got=dict(got))
    shot(h, 'autoplay', 'rows')

    # Andere mediatypen: GNOME's Other Media Types dialog (Type and Actie)
    click(h, 'st-autoplay-other')
    other = wait(h, lambda: one(h, 'st-autoplay-other-action') and one(h, 'st-autoplay-other-type'),
                 'the Other Media Types dialog', 10)
    h.check('Andere mediatypen opens the dialog with Type and Actie', bool(other))
    shot(h, 'autoplay', 'other')
    h.key('settings', 'Escape')
    wait(h, lambda: not one(h, 'st-autoplay-other-action'), 'the dialog closes', 5)

    # AutoPlay off: the rows grey out, as GNOME's
    click(h, 'st-autoplay-autorun-never')
    off = wait(h, lambda: (lambda n: n if n and not n.get('sensitive') else None)(one(h, 'st-autoplay-image-dcf')),
               'the rows are insensitive with AutoPlay off', 10)
    h.check('with AutoPlay off the media rows are insensitive (as GNOME greys them)', bool(off),
            key=h.gsettings_get(MEDIA, 'autorun-never'))
    removable = one(h, 'st-autoplay-removable')
    h.check('with AutoPlay off Verwisselbaar station is insensitive too (as Windows greys it)',
            bool(removable) and removable.get('sensitive') is False, node=removable)
    h.check('the switch off writes autorun-never true', h.gsettings_get(MEDIA, 'autorun-never') == 'true')


def bluetooth_with_bluez(h, T, bluez) -> None:
    # Bring both collaborating services up before opening the page; a late rfkill arrival refreshes its controls.
    rfkill = start(h, GSD_RFKILL, 'gsd-rfkill')
    sw = goto(h, 'bluetooth', 'bt-toggle', 'Bluetooth en apparaten with BlueZ')
    h.check('with BlueZ the switch shows the adapter powered', bool(sw) and sw.get('active') is True)
    h.wait_text('settings', T['discoverable'])
    disc = wait(h, lambda: [c for c in calls_of(bluez, 'Set:Discoverable') if c['args'] == 'true'],
                'Discoverable while the page shows', 10)
    h.check('the PC is discoverable while the page shows (Set Discoverable true)', bool(disc),
            calls=calls_of(bluez, 'Set:Discoverable'))
    tiles = {addr: one(h, f'bt-dev-{addr}') for addr in (MOUSE, HEADPHONES, PHONE)}
    h.check('the paired devices are tiles', all(tiles.values()), tiles={a: bool(n) for a, n in tiles.items()})
    h.check("the mouse's tile shows BlueZ's battery", bool(tiles[MOUSE]) and any('70%' in s for s in texts(tiles[MOUSE])),
            texts=tiles[MOUSE] and texts(tiles[MOUSE]))
    check_page(h, 'bluetooth')

    if rfkill:
        h.require('Bluetooth is ready for the power click', bool(wait(
            h, lambda: (lambda n: n and n.get('active') is True and n.get('sensitive') is not False)(one(h, 'bt-toggle')),
            'the powered, sensitive Bluetooth switch', 10)))
        click(h, 'bt-toggle')
        gsd = wait(h, lambda: calls_of(rfkill, 'Set:BluetoothAirplaneMode'), 'gsd BluetoothAirplaneMode', 10)
        h.check("the switch off goes through gsd's Rfkill: BluetoothAirplaneMode true (amendment 57)",
                bool(gsd) and gsd[0]['args'] == 'true', calls=gsd)
        power = wait(h, lambda: [c for c in calls_of(bluez, 'Set:Powered') if c['args'] == 'false'],
                     'BlueZ Powered false after gsd', 15)
        h.check("the adapter did not follow gsd: the host sets BlueZ's Powered false", bool(power))
        off = wait(h, lambda: (lambda n: n if n and n.get('active') is False else None)(one(h, 'bt-toggle')),
                   'the switch shows off', 10)
        h.check('the page shows Bluetooth off', bool(off) and bool(h.wait_text('settings', T['off'], timeout=5)))
        shot(h, 'bluetooth', 'off')
        click(h, 'bt-toggle')
        back = wait(h, lambda: [c for c in calls_of(bluez, 'Set:Powered') if c['args'] == 'true'],
                    'BlueZ Powered true', 15)
        gsd = calls_of(rfkill, 'Set:BluetoothAirplaneMode')
        h.check('the switch on: BluetoothAirplaneMode false, then the adapter powered',
                [c['args'] for c in gsd] == ['true', 'false'] and bool(back), gsd=gsd, powered=back)
        on = wait(h, lambda: (lambda n: n if n and n.get('active') is True else None)(one(h, 'bt-toggle')),
                  'the switch shows on', 10)
        h.check('the page shows Bluetooth on again', bool(on))
        rfkill.stop()
    flight = start(h, GSD_RFKILL_FLIGHT, 'gsd-rfkill-flight')
    if flight:
        notice = goto(h, 'bluetooth', 'bt-airplane', 'Bluetooth en apparaten in flight mode')
        h.check("flight mode on shows GNOME's notice", bool(notice) and any(T['flight'] in s for s in texts(notice)),
                texts=notice and texts(notice))
        shot(h, 'bluetooth', 'flightmode')
        flight.stop()


def window_closes_on_bluetooth(h, bluez) -> None:
    """Review B finding 1: the page's cleanups run only when another page is shown, and Settings ends (its backend
    with it, PR_SET_PDEATHSIG) right after its window, so the window's close-request ends the lease itself."""
    def last(want):
        disc = [c['args'] for c in calls_of(bluez, 'Set:Discoverable')]
        return disc if disc[-1:] == [want] else None
    goto(h, 'bluetooth', 'bt-toggle', 'Bluetooth en apparaten before the window closes')
    asked = wait(h, lambda: last('true'), 'Discoverable true again', 10)
    h.close('settings')
    ended = wait(h, lambda: last('false'), 'Discoverable false after the window closed', 10)
    h.check('closing the window on the Bluetooth page ends the lease: Discoverable false', bool(asked) and bool(ended),
            calls=[c['args'] for c in calls_of(bluez, 'Set:Discoverable')])


def devices_page(h, T, bluez) -> None:
    upower = start(h, UPOWER, 'upower')
    card = goto(h, 'bluetooth-devices', f'btd-dev-{MOUSE}', 'Apparaten')
    groups = [names_below(one(h, f'btd-group-{i}')) for i in range(3)]
    h.record('devices.groups', [sorted(n for n in g if n.startswith('btd-dev-') or n.startswith('btd-battery-'))
                                for g in groups])
    h.check("Windows' groups: mice under Muis, toetsenbord en pen, headphones under Audio, the phone under Overige",
            {f'btd-dev-{MOUSE}', 'btd-battery-mouse_hidpp_battery_0'} <= groups[0]
            and f'btd-dev-{HEADPHONES}' in groups[1] and f'btd-dev-{PHONE}' in groups[2])
    h.check("the Bluetooth mouse shows BlueZ's battery, the receiver mouse UPower's",
            bool(card) and any('70%' in s for s in texts(card))
            and any('55%' in s for s in texts(one(h, 'btd-battery-mouse_hidpp_battery_0') or {})))
    h.check("the computer's own battery is not listed", not one(h, 'btd-battery-battery_BAT0'))
    h.check('with BlueZ there is no no-Bluetooth notice', not one(h, 'btd-no-bluetooth'))
    check_page(h, 'bluetooth-devices')

    # ⋯ › Verbinding verbreken (the headphones)
    if click(h, f'btd-more-{HEADPHONES}') and wait(h, lambda: one(h, 'btd-toggle'), 'the ⋯ menu', 5):
        click(h, 'btd-toggle')
    gone = wait(h, lambda: [c for c in calls_of(bluez, 'Disconnect') if c['path'] == DEV[HEADPHONES]], 'Disconnect', 15)
    h.check('⋯ › Verbinding verbreken sends Device1.Disconnect to the headphones', bool(gone))
    # ⋯ › Eigenschappen (the phone): GNOME's device dialog with Bestanden verzenden
    props = None
    if click(h, f'btd-more-{PHONE}') and wait(h, lambda: one(h, 'btd-properties-item'), 'the ⋯ menu', 5):
        click(h, 'btd-properties-item')
        props = wait(h, lambda: one(h, 'btd-properties'), 'the device dialog', 10)
    h.check("⋯ › Eigenschappen opens GNOME's device dialog with the address", bool(props) and PHONE in texts(props),
            texts=props and texts(props))
    shot(h, 'bluetooth-devices', 'properties')
    if props:
        sent = dialog_click(h, 'btd-send', lambda: stub_calls(h, 'bluetooth-sendto'), 'Bestanden verzenden')
        argv = stub_calls(h, 'bluetooth-sendto')
        h.check('Bestanden verzenden runs bluetooth-sendto --device=<address> --name=<name>',
                sent and argv[:1] == [f'--device={PHONE} --name={NAMES[PHONE]}'], argv=argv)
        h.key('settings', 'Escape')
        wait(h, lambda: not one(h, 'btd-properties'), 'the device dialog closes', 5)
    # ⋯ › Apparaat verwijderen (the phone): Windows' question, then RemoveDevice
    question = None
    if click(h, f'btd-more-{PHONE}') and wait(h, lambda: one(h, 'btd-remove'), 'the ⋯ menu', 5):
        click(h, 'btd-remove')
        question = wait(h, lambda: one(h, 'w11-dialog-primary'), "Windows' question", 10)
    h.check('Apparaat verwijderen asks first', bool(question))
    shot(h, 'bluetooth-devices', 'remove')
    if question:
        removed = dialog_click(h, 'w11-dialog-primary', lambda: calls_of(bluez, 'RemoveDevice'), 'Ja (remove)')
        calls = calls_of(bluez, 'RemoveDevice')
        h.check('Ja sends Adapter1.RemoveDevice with the phone',
                removed and calls[0]['args'] == f"(objectpath '{DEV[PHONE]}',)", calls=calls)
        h.check('the removed phone leaves the list',
                bool(wait(h, lambda: not one(h, f'btd-dev-{PHONE}'), 'the phone card goes', 10)))
    if upower:
        upower.stop()


def usb_page(h) -> None:
    bolt = start(h, BOLT, 'bolt')
    if not bolt:
        return
    sw = goto(h, 'usb', 'usb-direct-access', 'USB')
    h.check('Directe toegang shows AuthMode enabled', bool(sw) and sw.get('active') is True, switch=sw)
    dock = wait(h, lambda: one(h, f'usb-dev-{DOCK_UID}'), 'the dock', 10)
    h.check('the pending dock is listed', bool(dock))
    button = wait(h, lambda: one(h, 'usb-authorize'), 'Autoriseren en verbinden', 10)
    h.check('the pending dock shows Autoriseren en verbinden', bool(button) and button.get('sensitive'))
    check_page(h, 'usb')
    click(h, 'usb-authorize')
    enroll = wait(h, lambda: calls_of(bolt, 'EnrollDevice'), 'EnrollDevice', 15)
    h.record('usb.enroll', enroll)
    h.check('Autoriseren en verbinden enrolls the dock with the default policy',
            bool(enroll) and enroll[0]['args'] == f"('{DOCK_UID}', 'default', '')", calls=enroll)
    h.check('the enrolment allows the password dialog', bool(enroll) and enroll[0]['interactive'] is True)
    click(h, 'usb-direct-access')
    setm = wait(h, lambda: calls_of(bolt, 'Set:AuthMode'), 'Set AuthMode', 15)
    h.check('the switch off writes AuthMode disabled', bool(setm) and "'disabled'" in setm[-1]['args'], calls=setm)
    shot(h, 'usb', 'direct-access-off')
    bolt.stop()


def printers_page(h, T, log, helper, caps) -> None:
    hp = goto(h, 'printers', 'printer-HP_LaserJet_M404', 'Printers en scanners', 25)
    epson = one(h, 'printer-Epson_ET_2850')
    h.check('both queues of the scheduler are listed', bool(hp) and bool(epson))
    h.check("the stopped printer shows GNOME's reason", bool(epson) and any(T['paper'] in t for t in texts(epson)),
            texts=epson and texts(epson))
    check_page(h, 'printers')
    if caps.get('simpleScan') is True:
        click(h, 'printers-scan')
        scan = wait(h, lambda: stub_calls(h, 'simple-scan'), 'simple-scan started', 15)
        probe = None
        if not scan:
            # why: the same apps.launch the button sends, through the app's own backend
            try:
                probe = h.backend('settings', 'apps.launch', {'id': 'simple-scan.desktop'}, timeout=20)
            except Exception as error:   # noqa: BLE001 - recorded for the handoff note
                probe = {'error': str(error)}
            scan = wait(h, lambda: stub_calls(h, 'simple-scan'), 'simple-scan started (backend call)', 5)
        # files/actions.rs launch_gio binds its ack socket in $TMPDIR (run/tmp), whose path in a deep checkout is
        # longer than a unix socket allows: known B-sun-len (note 12 for I), not the printer page's
        long_path = 'SUN_LEN' in json.dumps(probe or {})
        h.check('Scanner openen starts Documentscanner (simple-scan)', bool(scan), known='B-sun-len' if long_path else None,
                argv=scan, backend=probe, tmpdir=len(str(h.hm.RUN / 'tmp')))
    else:
        h.check('without simple-scan there is no Scanner openen', not one(h, 'printers-scan'))
    if not helper:
        return
    click(h, 'printers-add')
    found = wait(h, lambda: one(h, 'printers-found-add-1'), 'the devices DevicesGet found', 30)
    h.check('Apparaat toevoegen lists the devices the scheduler found', bool(found))
    get = calls_of(helper, 'DevicesGet')
    h.check('DevicesGet allows the password dialog', bool(get) and get[0]['interactive'] is True, calls=get)
    shot(h, 'printers', 'found')
    click(h, 'printers-found-add-1')
    wait(h, lambda: calls_of(helper, 'PrinterAddOptionDefault'), 'the new queue is set up', 20)
    writes = [c for c in helper.calls() if c['method'].startswith('Printer')]
    h.record('printers.add', writes)
    h.check('the driverless printer is added with IPP Everywhere, enabled, accepting jobs and A4',
            [c['method'] for c in writes][:4] == ['PrinterAdd', 'PrinterSetEnabled', 'PrinterSetAcceptJobs',
                                                  'PrinterAddOptionDefault']
            and writes[0]['args'] == "('Fake-LaserJet', 'ipp://127.0.0.1:8631/ipp/print', 'everywhere', 'Fake LaserJet', 'Lab')"
            and writes[3]['args'] == "('Fake-LaserJet', 'media', ['a4'])", calls=writes)
    h.check('every printer write allows the password dialog', bool(writes) and all(c['interactive'] for c in writes))


def printer_page(h, T, log, helper) -> None:
    sf = h.surfaces.get('settings')
    if sf and sf.page == 'printers':
        h.open('settings:home')
    h.settings_data_page('printers', 'printer-HP_LaserJet_M404', title='HP LaserJet M404')
    queue = wait(h, lambda: one(h, 'printer-queue'), "the printer's page", 15)
    h.check("the printer's page shows its settings", bool(queue))
    check_page(h, 'printer', item='HP LaserJet M404')
    click(h, 'printer-testpage')
    jobs = wait(h, lambda: [r for r in ipp_requests(log) if r.get('op') == 'Print-Job'], 'Print-Job', 15) or []
    h.record('printers.testPage', jobs[:1])
    h.check("Testpagina afdrukken prints GNOME's test page as the user's job",
            bool(jobs) and jobs[0].get('docHead', '').startswith('#PDF-BANNER')
            and (jobs[0]['attrs'].get('job-name') or [''])[0] == T['test'], jobs=jobs[:1])
    click(h, 'printer-queue')
    job = wait(h, lambda: one(h, 'printer-job-41-hold'), 'the queue lists job 41', 15)
    h.check("the queue lists the scheduler's jobs", bool(job) and bool(one(h, 'printer-job-42')))
    shot(h, 'printer', 'queue')
    if helper and job:
        dialog_click(h, 'printer-job-41-hold', lambda: calls_of(helper, 'JobSetHoldUntil'), 'pause job 41')
        hold = calls_of(helper, 'JobSetHoldUntil')
        h.check('pausing a job sends JobSetHoldUntil(id, indefinite) with the password dialog allowed',
                bool(hold) and hold[0]['args'] == "(41, 'indefinite')" and hold[0]['interactive'] is True, calls=hold)
    if one(h, 'w11-dialog-close'):
        dialog_click(h, 'w11-dialog-close', lambda: not one(h, 'w11-dialog'), "the queue's Sluiten")
    if one(h, 'w11-dialog'):
        h.key('settings', 'Escape')
    time.sleep(0.5)
    click(h, 'printer-preferences')
    size = wait(h, lambda: one(h, 'printer-opt-PageSize'), 'the PPD options', 15)
    h.check("the preferences show the PPD's PageSize with the queue's default", bool(size) and 'A4' in texts(size),
            texts=size and texts(size))
    h.check('and the other PPD options', bool(one(h, 'printer-opt-Duplex')) and bool(one(h, 'printer-opt-ColorModel')))
    shot(h, 'printer', 'preferences')
    h.key('settings', 'Escape')
    time.sleep(0.5)
    h.open('settings:bluetooth')
    h.settings_data_page('printers', 'printer-Epson_ET_2850', title='Epson ET-2850')
    paused = wait(h, lambda: one(h, 'printer-paused'), 'the paused notice', 15)
    h.check('the stopped printer offers Hervatten', bool(paused))
    header(h, 'printer', item='Epson ET-2850')
    shot(h, 'printer', 'paused')
    if helper and paused:
        resume = h.find('settings', label=T['resume'])
        if resume:
            h.click('settings', node=resume[0])
        enabled = wait(h, lambda: [c for c in calls_of(helper, 'PrinterSetAcceptJobs') if 'Epson' in c['args']],
                       'PrinterSetAcceptJobs Epson', 15)
        h.check("Hervatten is GNOME's restart: enabled and accepting jobs",
                bool(enabled) and any(c['args'] == "('Epson_ET_2850', true)" for c in calls_of(helper, 'PrinterSetEnabled')),
                calls=[c for c in helper.calls() if 'Epson' in c['args']])
        click(h, 'printer-default')
        default = wait(h, lambda: calls_of(helper, 'PrinterSetDefault'), 'PrinterSetDefault', 15)
        h.check('Als standaard instellen sends PrinterSetDefault',
                bool(default) and default[0]['args'] == "('Epson_ET_2850',)" and default[0]['interactive'] is True,
                calls=default)


def colour_page(h) -> None:
    colord = start(h, COLORD, 'colord')
    if colord:
        sw = goto(h, 'colorprofile', 'colorprofile-0-enabled', 'Kleurprofiel')
        h.check('the display is listed open, colour management on', bool(sw) and sw.get('active') is True, switch=sw)
        dd = one(h, 'colorprofile-0-default')
        h.check('its default profile is the EDID profile', bool(dd) and 'Dell U2720Q' in texts(dd), texts=dd and texts(dd))
        h.check('without a colour sensor Kalibreren… is not offered, a notice says why',
                not one(h, 'colorprofile-0-calibrate') and bool(one(h, 'colorprofile-no-sensor')))
        check_page(h, 'colorprofile')
        choose(h, 'colorprofile-0-default', CD_CALIB_TITLE, lambda: calls_of(colord, 'MakeProfileDefault'))
        made = calls_of(colord, 'MakeProfileDefault')
        h.check('the Kleurprofiel drop-down sends Device.MakeProfileDefault with the password dialog allowed',
                bool(made) and made[0]['args'] == f"(objectpath '{CD_CALIB}',)" and made[0]['interactive'] is True,
                calls=made)
        time.sleep(1)
        click(h, 'colorprofile-0-enabled')
        off = wait(h, lambda: calls_of(colord, 'SetEnabled'), 'SetEnabled', 15)
        h.check('switching colour management off sends Device.SetEnabled(false) with the password dialog allowed',
                bool(off) and off[0]['args'] == '(false,)' and off[0]['interactive'] is True, calls=off)
        colord.stop()
    sensor = start(h, COLORD_SENSOR, 'colord-sensor')
    if sensor:
        cal = goto(h, 'colorprofile', 'colorprofile-0-calibrate', 'Kleurprofiel with a colour sensor')
        h.check('with a colour sensor the display offers Kalibreren… and no notice',
                bool(cal) and not one(h, 'colorprofile-no-sensor'))
        shot(h, 'colorprofile', 'sensor')
        sensor.stop()


def introspect(h, name: str, path: str):
    GLib, Gio = h.s.GLib, h.s.Gio
    reply = h.s.bus.call_sync(name, path, 'org.freedesktop.DBus.Introspectable', 'Introspect', None,
                              GLib.VariantType('(s)'), Gio.DBusCallFlags.NONE, 5000, None)
    return Gio.DBusNodeInfo.new_for_xml(reply.unpack()[0])


def method_signature(node, iface: str, method: str):
    info = node.lookup_interface(iface) if node else None
    m = info.lookup_method(method) if info else None
    return (''.join(a.signature for a in m.in_args), ''.join(a.signature for a in m.out_args)) if m else None


MUTTER_CM = ('org.gnome.Mutter.ColorManager', '/org/gnome/Mutter/ColorManager')
MUTTER_CALIBRATION = 'org.gnome.Mutter.ColorManager.Calibration'


def mutter_calibration(h) -> None:
    """Review B finding 10: the calibration's gamma (host/src/services/devices/color.rs) against this GNOME's own
    Mutter, read on the private session bus (no colorimeter here, so colord-session never asks): CalibrateMonitor(s
    connector) -> (o), and on a session of the harness's monitor GammaLutSize (u), SetCrtcGammaLut(aq, aq, aq) with a
    linear ramp, and Stop."""
    GLib, Gio = h.s.GLib, h.s.Gio
    try:
        node = introspect(h, *MUTTER_CM)
    except Exception as error:   # noqa: BLE001
        h.check("Mutter's colour manager answers on the session bus", False, error=str(error))
        return
    sig = method_signature(node, MUTTER_CM[0], 'CalibrateMonitor')
    h.check("Mutter's CalibrateMonitor takes a connector and gives a session path, as the host calls it",
            sig == ('s', 'o'), signature=sig)
    connector, path = None, None
    try:
        state = h.s.bus.call_sync('org.gnome.Mutter.DisplayConfig', '/org/gnome/Mutter/DisplayConfig',
                                  'org.gnome.Mutter.DisplayConfig', 'GetCurrentState', None, None,
                                  Gio.DBusCallFlags.NONE, 5000, None).unpack()
        connector = state[1][0][0][0]
        path = h.s.bus.call_sync(*MUTTER_CM, MUTTER_CM[0], 'CalibrateMonitor', GLib.Variant('(s)', (connector,)),
                                 GLib.VariantType('(o)'), Gio.DBusCallFlags.NONE, 5000, None).unpack()[0]
    except Exception as error:   # noqa: BLE001 - a virtual monitor may have no gamma LUT to calibrate
        h.record('mutterCalibrateMonitor', {'connector': connector, 'error': str(error)})
        h.check("CalibrateMonitor gives the harness's monitor a session", False, known='B-mutter-virtual',
                connector=connector, error=str(error))
        return
    try:
        session = introspect(h, MUTTER_CM[0], path)
        info = session.lookup_interface(MUTTER_CALIBRATION)
        prop = info.lookup_property('GammaLutSize') if info else None
        sigs = {m: method_signature(session, MUTTER_CALIBRATION, m) for m in ('SetCrtcGammaLut', 'Stop')}
        h.check('the session has GammaLutSize (u), SetCrtcGammaLut(aq, aq, aq) and Stop, as the host calls them',
                bool(prop) and prop.signature == 'u' and sigs == {'SetCrtcGammaLut': ('aqaqaq', ''), 'Stop': ('', '')},
                gammaLutSize=prop and prop.signature, methods=sigs)
        size = h.s.bus.call_sync(MUTTER_CM[0], path, 'org.freedesktop.DBus.Properties', 'Get',
                                 GLib.Variant('(ss)', (MUTTER_CALIBRATION, 'GammaLutSize')), None,
                                 Gio.DBusCallFlags.NONE, 5000, None).unpack()[0]
        h.record('mutterCalibration', {'connector': connector, 'path': path, 'gammaLutSize': size})
        if size:
            ramp = [round(i * 65535 / max(size - 1, 1)) for i in range(size)]
            h.s.bus.call_sync(MUTTER_CM[0], path, MUTTER_CALIBRATION, 'SetCrtcGammaLut',
                              GLib.Variant('(aqaqaq)', (ramp, ramp, ramp)), None, Gio.DBusCallFlags.NONE, 5000, None)
            h.check('SetCrtcGammaLut takes a linear ramp of GammaLutSize points', True, size=size)
    except Exception as error:   # noqa: BLE001
        h.check('the calibration session answers as the host expects', False, error=str(error))
    finally:
        try:
            h.s.bus.call_sync(MUTTER_CM[0], path, MUTTER_CALIBRATION, 'Stop', None, None, Gio.DBusCallFlags.NONE,
                              5000, None)
        except Exception as error:   # noqa: BLE001
            h.record('mutterCalibrationStop', str(error))


def private_stubs(h):
    """The common harness establishes private permissions; this scenario only checks them."""
    mode = (h.hm.RUN / 'stubs').stat().st_mode & 0o777
    h.check('the tool seam run/stubs is private (0700)', mode == 0o700, mode=oct(mode))


def run(h):
    L = h.lang
    T = TITLE[L]
    private_stubs(h)
    caps = bluetooth_without_bluez(h, T)
    autoplay(h, T)
    bluez = start_objects(h, BLUEZ, 'bluez')
    if bluez:
        bluetooth_with_bluez(h, T, bluez)
        devices_page(h, T, bluez)
        # leaving Bluetooth en apparaten ended the lease: Discoverable false again (only the host's own)
        disc = [c['args'] for c in calls_of(bluez, 'Set:Discoverable')]
        h.check('the lease ends when the page goes: Discoverable false', disc[-1:] == ['false'], calls=disc)
        window_closes_on_bluetooth(h, bluez)
        bluez.stop()
    usb_page(h)
    log = fake_cups(h)
    helper = start(h, CUPSPKHELPER, 'cupspkhelper') if log else None
    if log:
        printers_page(h, T, log, helper, caps)
        printer_page(h, T, log, helper)
    if helper:
        helper.stop()
    colour_page(h)
    mutter_calibration(h)
    h.record('dialogClicks', DIALOG_CLICKS)
    h.check('real pointer clicks reach the buttons of kit dialogs', bool(DIALOG_CLICKS)
            and all(d['path'] == 'real' for d in DIALOG_CLICKS), clicks=DIALOG_CLICKS)
    h.close('settings')
