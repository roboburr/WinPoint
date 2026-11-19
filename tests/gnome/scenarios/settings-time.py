"""Settings parity, batch E (notes/claude/settings-parity.md §2, §5.6, §5.8, §7, amendments 16-18, 30, 37, 59, 68; plan.json
"testing"): Tijd en taal, Datum en tijd, Regionale notatie, Taal en regio, Beheerinstellingen voor taal, Info and
Systeem in the private GNOME. Run twice, nl and en:

    tools/heavy.sh --min-mb 2560 --tag e -- python3 -B tests/gnome/harness.py launch run --lang nl --source worktree \\
        --scenario tests/gnome/scenarios/settings-time.py
    (and --lang en --out tests/gnome/scenarios/settings-time.en.out)

The daemons are batch E's stand-ins on the private system bus (tests/gnome/lib/standins/e.py, ``start_live``):
timedated, hostnamed and localed change their properties and send PropertiesChanged as the real ones do (the host
makes machine.changed of it), AccountsService and PackageKit; there is no timesync1 (amendment 30). Per page: the
title or breadcrumb, a screenshot (<page>.<lang>), no placeholder, no Dutch word in en. Then:

  time            the clock in the stand-in's zone, the child links
  datetime        Nu synchroniseren is exactly SetNTP(false, true), SetNTP(true, true), interactive, no toast
                  (amendment 68); the four tray-clock switches (real clicks) write GNOME's keys and follow them
                  written from outside, and the mirror carries seconds and the date to WinPoint's settings and the
                  taskbar clock; Tijdzone automatisch instellen waits for location services, then writes
                  automatic-timezone and takes the time zone drop-down away (not while location services are
                  off: GNOME's rule); Tijd automatisch instellen off
                  (SetNTP(false, true)) makes the manual time available: Wijzigen sends SetTime(now, false, true);
                  Tijdzone (real clicks) sends SetTimezone and shows timedated's new zone; NTP back on
  formats         Regionale notatie writes the region key and AccountsService's FormatsLocale (Aanbevolen: the
                  display language's locale, never ''); Eerste dag van de
                  week writes week-start-day and follows it; Korte tijd 12-uurs writes clock-format and the file
                  choosers' copies, Lange tijd and the taskbar clock follow; a write from outside moves it back
  language        Weergavetaal lists the installed languages (SetLanguage); Een taal toevoegen (PackageKit):
                  Resolve, then InstallPackages ONLY_TRUSTED with interactive authorization and the progress card
  language-admin  Taal van het aanmeldingsscherm sends locale1 SetLocale; Instellingen kopiëren sends the own language
  about           hostname1's name, GNOME Shell's version (amendment 37); the rename dialog sends SetPrettyHostname and
                  SetStaticHostname, interactive, and the page shows hostnamed's new name
  system          the device card with the new name and Naam wijzigen
  switch          Taal van WinPoint (the real drop-down) switches Settings to the other language and back

No key of these pages is WinPoint-owned (allow/accounts.rs has no Owned row), so there is no pointer to check.
"""
import importlib.util
import json
import os
import re
import sys
import time
from pathlib import Path


def _load(name, path, package=None):
    """A module by path: the harness runs with python -I and loads tests/gnome/lib by path, never from sys.path."""
    spec = importlib.util.spec_from_file_location(name, path, submodule_search_locations=[str(package)] if package else None)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


_LIB = Path(__file__).resolve().parent.parent / 'lib' / 'standins'
_load('ge2h_standins', _LIB / '__init__.py', _LIB)
e = _load('ge2h_standins.e', _LIB / 'e.py')

INTERFACE = 'org.gnome.desktop.interface'
CALENDAR = 'org.gnome.desktop.calendar'
DATETIME = 'org.gnome.desktop.datetime'
LOCATION = 'org.gnome.system.location'
LOCALE = 'org.gnome.system.locale'


def winpoint_settings(h) -> dict:
    try:
        return json.loads((h.hm.RUN / 'config' / 'win11-gnome' / 'settings.json').read_text())
    except (OSError, ValueError):
        return {}


def tray(h) -> str:
    """The taskbar clock's text (the native shell's tb-clock-time and tb-clock-date labels)."""
    parts = [n.get('label') or '' for name in ('tb-clock-time', 'tb-clock-date') for n in h.find('panel', name)]
    return ' | '.join(parts)


def time_page(h):
    text = e.page(h, 'time')
    h.check('time: the clock shows the stand-in’s zone', '(UTC+01:00) Amsterdam' in text['text'], text=text['text'][:400])
    links = [f'link-{p}' for p in ('datetime', 'language', 'typing')]
    h.check('time: Datum en tijd, Taal en regio and Typen are linked', all(e.node(h, n) for n in links),
            missing=[n for n in links if not e.node(h, n)])


def sync_now(h, td):
    seen = len(td.calls())
    click = e.press(h, 'st-datetime-sync')
    first = e.call_of(td, 'SetNTP', seen)
    if first:
        e.call_of(td, 'SetNTP', td.calls().index(first) + 1)
    h.sleep(1.5)
    got = e.writes(td, seen)
    h.check('datetime: Nu synchroniseren (a real click) is SetNTP(false) then SetNTP(true), interactive, nothing else',
            got == [('SetNTP', '(false, true)', True), ('SetNTP', '(true, true)', True)] and click.get('input') == 'clutter',
            writes=got, input=click.get('input'))
    h.check('datetime: no toast after Nu synchroniseren (amendment 68)', not h.find('settings', cls='w11-toast'))
    h.screenshot(f'datetime-synced.{h.lang}', 'settings')


def tray_clock(h):
    e.flip(h, 'datetime', 'st-datetime-clock-show-date', INTERFACE, 'clock-show-date', 'true')
    e.flip(h, 'datetime', 'st-datetime-show-weekdate', CALENDAR, 'show-weekdate', 'false')
    # seconds and the weekday by hand: the mirror (B0, settings_mirror.rs) in between
    name = 'st-datetime-clock-show-seconds'
    before = h.gsettings_get(INTERFACE, 'clock-show-seconds') or 'false'
    click = e.press(h, name)
    got = e.until(h, lambda: h.gsettings_get(INTERFACE, 'clock-show-seconds') == 'true' or None, 'clock-show-seconds true')
    h.check('datetime: a real click on Seconden weergeven writes clock-show-seconds true',
            bool(got) and click.get('input') == 'clutter', before=before, after=h.gsettings_get(INTERFACE, 'clock-show-seconds'))
    mirrored = e.until(h, lambda: (lambda s: s if s.get('time', {}).get('showSeconds') is True else None)(winpoint_settings(h)),
                       'time.showSeconds', 10)
    h.check('datetime: the mirror sets WinPoint’s time.showSeconds and taskbar.showSeconds',
            bool(mirrored) and mirrored.get('taskbar', {}).get('showSeconds') is True,
            time=(mirrored or winpoint_settings(h)).get('time'))
    seconds = e.until(h, lambda: re.search(r'\d{1,2}:\d{2}:\d{2}', tray(h)) or None, 'the taskbar clock with seconds', 10)
    h.check('datetime: the taskbar clock shows the seconds', bool(seconds), tray=tray(h))
    h.screenshot(f'datetime-tray-seconds.{h.lang}', 'panel')
    h.gsettings_set(INTERFACE, 'clock-show-seconds', 'false')
    follows = e.until(h, lambda: e.active(h, name) is False or None, 'the seconds switch follows', 10)
    h.check('datetime: Seconden weergeven follows clock-show-seconds written from outside', bool(follows))
    h.check('datetime: and the taskbar clock drops the seconds', e.until(
        h, lambda: not re.search(r'\d{1,2}:\d{2}:\d{2}', tray(h)) or None, 'no seconds', 10) is not None, tray=tray(h))

    name = 'st-datetime-clock-show-weekday'
    click = e.press(h, name)
    got = e.until(h, lambda: h.gsettings_get(INTERFACE, 'clock-show-weekday') == 'true' or None, 'clock-show-weekday true')
    h.check('datetime: a real click on Weekdag weergeven writes clock-show-weekday true',
            bool(got) and click.get('input') == 'clutter', after=h.gsettings_get(INTERFACE, 'clock-show-weekday'))
    mirrored = e.until(h, lambda: winpoint_settings(h).get('time', {}).get('showWeekday') is True or None, 'time.showWeekday', 10)
    h.check('datetime: the mirror sets WinPoint’s time.showWeekday', bool(mirrored))
    days = ('ma', 'di', 'wo', 'do', 'vr', 'za', 'zo') if h.lang != 'en' else ('Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun')
    weekday = e.until(h, lambda: any(re.search(rf'\b{d}\b', tray(h)) for d in days) or None, 'the weekday in the taskbar clock', 6)
    h.check('datetime: the taskbar clock shows the weekday (contract §8 note 2, for I)', bool(weekday), tray=tray(h))
    h.gsettings_set(INTERFACE, 'clock-show-weekday', 'false')
    follows = e.until(h, lambda: e.active(h, name) is False or None, 'the weekday switch follows', 10)
    h.check('datetime: Weekdag weergeven follows clock-show-weekday written from outside', bool(follows))


def automatic_zone(h):
    name = 'st-datetime-automatic-timezone'
    sw = e.node(h, name)
    h.record('caps.geoclue', (h.backend('settings', 'caps.get') or {}).get('geoclue'))
    if not sw:
        h.check('datetime: Tijdzone automatisch instellen where geoclue is installed', False)
        return
    located = h.gsettings_get(LOCATION, 'enabled')
    h.record('location enabled at start', located)
    h.record('automatic time zone switch at start', {'sensitive': sw.get('sensitive'), 'active': sw.get('active')})
    if sw.get('sensitive') is False:
        h.check('datetime: Tijdzone automatisch instellen waits for location services (insensitive, with the hint)',
                located != 'true' and bool(h.wait_text('settings', e.tr(
                    h, 'Hiervoor moeten locatievoorzieningen zijn ingeschakeld.', 'Location services need to be turned on for this.'), timeout=3)),
                located=located)
        h.gsettings_set(LOCATION, 'enabled', 'true')
        ok = e.until(h, lambda: (e.node(h, name) or {}).get('sensitive') is not False or None, 'the switch follows location', 10)
        h.check('datetime: it follows location services turned on from outside', bool(ok))
    e.flip(h, 'datetime', name, DATETIME, 'automatic-timezone', 'false')
    h.gsettings_set(DATETIME, 'automatic-timezone', 'true')
    zone = e.until(h, lambda: (e.node(h, 'st-datetime-timezone') or {}).get('sensitive') is False or None, 'the zone drop-down off', 8)
    h.check('datetime: with the automatic time zone on, the time zone can’t be chosen', bool(zone))
    h.screenshot(f'datetime-autozone.{h.lang}', 'settings')
    # GNOME's rule (cc-datetime-page.c): the zone stays choosable while location services are off, even with the
    # automatic time zone on (review of batch E, finding 6)
    h.gsettings_set(LOCATION, 'enabled', 'false')
    free = e.until(h, lambda: (e.node(h, 'st-datetime-timezone') or {}).get('sensitive') is not False or None,
                   'the zone drop-down with location off', 8)
    h.check('datetime: with location services off the time zone can be chosen, the automatic time zone on or not',
            free is not None)
    h.gsettings_set(LOCATION, 'enabled', 'true')
    h.check('datetime: and with location services on again it can’t', e.until(
        h, lambda: (e.node(h, 'st-datetime-timezone') or {}).get('sensitive') is False or None,
        'the zone drop-down off again', 8) is not None)
    h.gsettings_set(DATETIME, 'automatic-timezone', 'false')
    h.check('datetime: and with it off again, the drop-down is there again', e.until(
        h, lambda: (e.node(h, 'st-datetime-timezone') or {}).get('sensitive') is not False or None, 'the zone drop-down on', 8) is not None)
    h.gsettings_set(LOCATION, 'enabled', located or 'false')


def manual_time(h, td):
    seen = len(td.calls())
    click = e.press(h, 'st-datetime-ntp')
    c = e.call_of(td, 'SetNTP', seen)
    h.check('datetime: Tijd automatisch instellen off (a real click) sends SetNTP(false, true), interactive',
            bool(c) and c['args'] == '(false, true)' and c['interactive'] and click.get('input') == 'clutter', call=c)
    manual = e.until(h, lambda: (e.node(h, 'st-datetime-manual') or {}).get('sensitive') is True or None,
                     'Wijzigen once timedated has NTP off (machine.changed)', 10)
    h.check('datetime: the page follows timedated: De datum en tijd handmatig instellen can be used, Nu synchroniseren not',
            bool(manual) and (e.node(h, 'st-datetime-sync') or {}).get('sensitive') is False)
    if manual:
        seen = len(td.calls())
        e.press(h, 'st-datetime-manual')
        if e.dialog_open(h):
            h.sleep(0.4)
            h.screenshot(f'datetime-manual.{h.lang}', 'settings')
            now = time.time()
            e.dialog_button(h)
            e.dialog_closed(h, 5)
            c = e.call_of(td, 'SetTime', seen)
            m = re.search(r'int64 (-?\d+)', c['args']) if c else None
            usec = int(m.group(1)) if m else None
            h.check('datetime: Wijzigen sends SetTime(the picked local time in µs, false, true), interactive',
                    bool(c) and c['interactive'] and c['args'].endswith(', false, true)')
                    and usec is not None and abs(usec / 1e6 - now) < 120, call=c, now=now)


def time_zone(h, td):
    zones = (h.backend('settings', 'machine.time.zones') or {}).get('zones') or []
    by_label = {z.get('label'): z.get('id') for z in zones}
    h.record('machine.time.zones', len(zones))
    name = 'st-datetime-timezone'
    seen = len(td.calls())
    # the list opens at its top (no scroll to the current zone): one of its first rows the pointer reaches
    click = e.choose(h, name, accept=lambda t: t in by_label and 'Amsterdam' not in t)
    if not click:
        return
    label = click['text']
    c = e.call_of(td, 'SetTimezone', seen)
    h.check(f'datetime: Tijdzone {label} (real clicks) sends SetTimezone({by_label[label]}, true), interactive',
            bool(c) and c['args'] == f"('{by_label[label]}', true)" and c['interactive'] and click.get('input') == 'clutter', call=c)
    shown = e.until(h, lambda: label in (h.text('settings').get('text') or '') or None, 'the new zone', 10)
    h.check('datetime: the page shows timedated’s new zone', bool(shown))
    h.screenshot(f'datetime-zone.{h.lang}', 'settings')
    # back to Amsterdam through the backend (the long list would need a search)
    h.backend('settings', 'machine.time.setTimezone', {'zone': 'Europe/Amsterdam'})


def datetime_page(h, td):
    e.page(h, 'datetime')
    ntp = e.node(h, 'st-datetime-ntp')
    h.check('datetime: Tijd automatisch instellen follows timedated’s NTP', bool(ntp) and ntp.get('active') is True)
    manual = e.node(h, 'st-datetime-manual')
    h.check('datetime: the manual time only while NTP is off', bool(manual) and manual.get('sensitive') is False)
    h.check('datetime: the time zone in Windows’ label', '(UTC+01:00) Amsterdam' in e.dropdown_label(h, 'st-datetime-timezone'),
            label=e.dropdown_label(h, 'st-datetime-timezone'))
    dst = e.node(h, 'st-datetime-dst')
    h.check('datetime: Automatisch aanpassen aan zomertijd on and fixed (tzdata does it)', bool(dst)
            and dst.get('active') is True and dst.get('sensitive') is False)
    h.check('datetime: no 24-hour row here (amendment 16)', not e.node(h, 'st-datetime-clock-format'))
    if td:
        sync_now(h, td)
    tray_clock(h)
    automatic_zone(h)
    if td:
        manual_time(h, td)
        time_zone(h, td)
        seen = len(td.calls())
        e.press(h, 'st-datetime-ntp')
        c = e.call_of(td, 'SetNTP', seen)
        h.check('datetime: Tijd automatisch instellen back on sends SetNTP(true, true)', bool(c) and c['args'] == '(true, true)', call=c)


def formats_page(h, acc):
    e.page(h, 'formats')
    for name in ('st-formats-region', 'st-formats-week-start-day', 'st-formats-interface-clock-format', 'st-formats-preview'):
        h.check(f'formats: {name}', bool(e.node(h, name)))
    locales = (h.backend('settings', 'machine.locales') or {}).get('locales') or []
    h.record('machine.locales', [loc.get('id') for loc in locales][:40])
    names = {loc.get('name'): loc.get('id') for loc in locales if loc.get('id') not in (None, 'C.UTF-8')}
    if names and acc:
        seen = len(acc.calls())
        before = h.gsettings_get(LOCALE, 'region')
        # a generated locale the pointer reaches in the list (English (United Kingdom) when it is in view)
        click = e.choose(h, 'st-formats-region', accept=lambda t: names.get(t) not in (None, e.LANGUAGE))
        want = names.get((click or {}).get('text'))
        got = e.written(h, LOCALE, 'region', before) if want else None
        c = e.call_of(acc, 'SetFormatsLocale', seen) if want else None
        h.check(f'formats: Regionale notatie {(click or {}).get("text")} (real clicks) writes region and AccountsService’s '
                'FormatsLocale, interactive', bool(want) and got == f"'{want}'" and bool(c) and c['args'] == f"('{want}',)"
                and c['interactive'] and click.get('input') == 'clutter', got=got, call=c, want=want)
        banner = e.until(h, lambda: e.node(h, 'st-signout'), 'the sign-out banner', 8)
        h.check('formats: Windows’ sign-out banner: the session keeps its formats until the next sign-in', bool(banner))
        h.screenshot(f'formats-region.{h.lang}', 'settings')
        seen = len(acc.calls())
        e.choose(h, 'st-formats-region', e.tr(h, 'Aanbevolen', 'Recommended'))
        back = e.until(h, lambda: h.gsettings_get(LOCALE, 'region') == "''" or None, 'region back to the recommended', 10)
        h.check('formats: Aanbevolen writes region \'\' (the display language’s formats)', bool(back),
                region=h.gsettings_get(LOCALE, 'region'))
        # GNOME passes a real locale: the display language's (Ubuntu's save-to-pam-env refuses an empty one; review
        # of batch E, finding 7); the account's stored "en_US" is the generated en_US.UTF-8 (finding 5)
        c = e.call_of(acc, 'SetFormatsLocale', seen)
        h.check(f'formats: and FormatsLocale gets the display language’s locale ({e.LANGUAGE}), not \'\'',
                bool(c) and c['args'] == f"('{e.LANGUAGE}',)" and c['interactive'], call=c)
        h.check('formats: and the banner goes', e.until(h, lambda: not e.node(h, 'st-signout') or None, 'no banner', 8) is not None)
    else:
        h.check('formats: a generated locale to choose', False, locales=len(locales))

    name = 'st-formats-week-start-day'
    before = h.gsettings_get(CALENDAR, 'week-start-day')
    click = e.choose(h, name, e.tr(h, 'Maandag', 'Monday'), exact=True)
    seen, end = [], time.monotonic() + 3
    while time.monotonic() < end:
        v = (h.gsettings_get(CALENDAR, 'week-start-day'), winpoint_settings(h).get('time', {}).get('firstDayOfWeek'))
        if not seen or seen[-1] != v:
            seen.append(v)
        time.sleep(0.1)
    h.record('formats: week-start-day and time.firstDayOfWeek after the click', seen)
    h.record('formats: GSettings’ week-start-day after the click', h.backend(
        'settings', 'gsettings.get', {'keys': [{'schema': CALENDAR, 'key': 'week-start-day'}]}))
    h.screenshot(f'formats-weekstart.{h.lang}', 'settings')
    got = e.written(h, CALENDAR, 'week-start-day', before)
    h.check('formats: Eerste dag van de week Maandag (real clicks) writes week-start-day monday',
            got == "'monday'" and bool(click) and click.get('input') == 'clutter', got=got, click=click,
            label=e.dropdown_label(h, name), toast=[n.get('label') for n in h.find('settings', cls='w11-toast')][:3])
    if got != "'monday'":
        # what the host says to the drop-down's own write (Gs::set)
        h.record('formats: gsettings.set week-start-day through the backend', h.backend(
            'settings', 'gsettings.set', {'schema': CALENDAR, 'values': {'week-start-day': 'monday'}}))
        h.record('formats: week-start-day after that', h.gsettings_get(CALENDAR, 'week-start-day'))
    h.gsettings_set(CALENDAR, 'week-start-day', "'sunday'")
    sunday = e.tr(h, 'Zondag', 'Sunday')
    h.check('formats: the drop-down follows week-start-day written from outside', e.until(
        h, lambda: sunday in e.dropdown_label(h, name) or None, sunday, 10) is not None, label=e.dropdown_label(h, name))
    h.gsettings_set(CALENDAR, 'week-start-day', "'default'")

    name = 'st-formats-interface-clock-format'
    click = e.choose(h, name, '2:35 PM')
    got = e.written(h, INTERFACE, 'clock-format', "'24h'")
    h.check('formats: Korte tijd 12-uurs (real clicks) writes clock-format 12h', got == "'12h'" and bool(click)
            and click.get('input') == 'clutter', got=got)
    h.check('formats: and the file choosers’ copies (GTK 3 and GTK 4), in the same write',
            h.gsettings_get('org.gtk.gtk4.Settings.FileChooser', 'clock-format') == "'12h'"
            and h.gsettings_get('org.gtk.Settings.FileChooser', 'clock-format') == "'12h'",
            gtk4=h.gsettings_get('org.gtk.gtk4.Settings.FileChooser', 'clock-format'),
            gtk3=h.gsettings_get('org.gtk.Settings.FileChooser', 'clock-format'))
    h.check('formats: Lange tijd follows (2:35:12 PM)', e.until(
        h, lambda: (e.node(h, 'st-formats-long-time') or {}).get('label') == '2:35:12 PM' or None, 'Lange tijd', 8) is not None)
    ampm = e.until(h, lambda: re.search(r'\b(AM|PM|a\.m\.|p\.m\.)', tray(h), re.I) or None, 'the taskbar clock in 12 hours', 10)
    h.check('formats: the mirror takes the 12-hour clock to the taskbar clock (time.h24)', bool(ampm)
            and winpoint_settings(h).get('time', {}).get('h24') is False, tray=tray(h), time=winpoint_settings(h).get('time'))
    h.screenshot(f'formats-12h.{h.lang}', 'settings')
    h.gsettings_set(INTERFACE, 'clock-format', "'24h'")
    h.check('formats: Korte tijd follows clock-format written from outside', e.until(
        h, lambda: '14:35' in e.dropdown_label(h, name) or None, '24-hour', 10) is not None, label=e.dropdown_label(h, name))


def language_page(h, acc, pk):
    text = e.page(h, 'language')
    # The formats link belongs to the collapsed regional samples expander. Reveal it before inspecting children.
    h.click('settings', 'st-language-region-samples')
    e.until(h, lambda: e.node(h, 'st-language-change-formats'), 'regional samples expanded', 5)
    for name in ('st-language-display', 'language-display', 'st-language-add', 'st-language-region-samples',
                 'st-language-region', 'st-language-change-formats'):
        h.check(f'language: {name}', bool(e.node(h, name)))
    offered = e.options(h, 'st-language-display')
    h.record('language: display languages', offered)
    h.check('language: Weergavetaal offers only languages that are installed (by their own names)', bool(offered))
    current = e.dropdown_label(h, 'st-language-display')
    h.check('language: Weergavetaal shows the account’s language by its own name', 'English (United States)' in current,
            label=current)
    h.check('language: no sign-out banner while the account’s language is the session’s', not e.node(h, 'st-signout'))
    other = next((o for o in offered if o and o not in current), None) if len(offered) > 1 else None
    if other and acc:
        seen = len(acc.calls())
        click = e.choose(h, 'st-language-display', accept=lambda t: bool(t) and t != current)
        c = e.call_of(acc, 'SetLanguage', seen)
        h.check(f'language: Weergavetaal {(click or {}).get("text")} (real clicks) sends SetLanguage, interactive',
                bool(c) and c['interactive'] and bool(click) and click.get('input') == 'clutter', call=c)
        banner = e.until(h, lambda: e.node(h, 'st-signout'), 'the sign-out banner', 10)
        h.check('language: Windows’ sign-out banner for the new display language', bool(banner))
        h.screenshot(f'language-signout.{h.lang}', 'settings')
        # the own language back (Beheerinstellingen voor taal copies it later)
        h.backend('settings', 'users.setLanguage', {'language': e.STORED_LANGUAGE})
    else:
        h.record('language: no second display language to choose in this sandbox', offered)

    if not pk:
        return
    add = e.node(h, 'st-language-add')
    h.check('language: Een taal toevoegen can be clicked with PackageKit', bool(add) and add.get('sensitive') is not False, add=add)
    if not add or add.get('sensitive') is False:
        return
    seen = len(pk.calls())
    e.press(h, 'st-language-add')
    if not e.dialog_open(h):
        return
    rows = e.until(h, lambda: h.find('settings', where=lambda n: str(n.get('name', '')).startswith('st-language-add-')),
                   'the languages to install', 8) or []
    h.record('language: languages to add', [r['name'].removeprefix('st-language-add-') for r in rows][:60])
    row = next((r for r in rows if r['name'] == 'st-language-add-de'), rows[0] if rows else None)
    if not row:
        h.check('language: languages to install', False)
        e.dialog_closed(h, 1)
        return
    code = row['name'].removeprefix('st-language-add-')
    if row.get('rect'):
        h.click('settings', node=row)
    if (e.node(h, 'w11-dialog-primary') or {}).get('sensitive') is False:
        h.click('settings', row['name'], real=False)
    h.sleep(0.4)
    h.screenshot(f'language-add.{h.lang}', 'settings')
    e.dialog_button(h)
    e.dialog_closed(h, 3)
    progress = e.until(h, lambda: (lambda n: n if n and n.get('visible', True) else None)(e.node(h, 'st-language-progress')),
                       'the progress card', 10)
    h.check('language: the install shows its progress card', bool(progress))
    h.screenshot(f'language-installing.{h.lang}', 'settings')
    e.until(h, lambda: any(m == 'InstallPackages' for m, _, _ in e.writes(pk, seen)) or None, 'InstallPackages', 15)
    h.sleep(3.5)
    got = e.writes(pk, seen)
    h.record('language: PackageKit calls', got)
    methods = [m for m, _, _ in got]
    resolve = next((w for w in got if w[0] == 'Resolve'), None)
    install = next((w for w in got if w[0] == 'InstallPackages'), None)
    # NOT_INSTALLED | NEWEST | ARCH (327688, gnome-software's filter), and the stand-in's i386 build of a package is
    # never installed (review of batch E, finding 2)
    h.check(f'language: installing {code} is Resolve (not installed, newest, native architecture) of its packages, '
            'then InstallPackages ONLY_TRUSTED of what PackageKit found for this architecture, with interactive '
            'authorization',
            methods.count('CreateTransaction') == 2 and bool(resolve) and resolve[1].startswith('(uint64 327688, [')
            and resolve[2] is False and bool(install) and install[2] is True
            and install[1] == f"(uint64 2, ['{e.PK_PACKAGE}'])", calls=got)
    hints = [a for m, a, _ in got if m == 'SetHints']
    h.check('language: the install transaction carries PackageKit’s interactive hint',
            any("'interactive=true'" in a for a in hints) and any("'interactive=false'" in a for a in hints), hints=hints)
    done = e.until(h, lambda: (lambda n: None if n and n.get('visible', True) else True)(e.node(h, 'st-language-progress')),
                   'the page after the install', 10)
    h.check('language: the page is read again after the install (no error)', bool(done) and not h.find('settings', 'w11-dialog'))


def language_admin_page(h, lc):
    e.page(h, 'language-admin')
    if not lc:
        return
    offered = e.options(h, 'st-language-admin-language')
    h.record('language-admin: sign-in screen languages', offered[:20])
    current = e.dropdown_label(h, 'st-language-admin-language')
    if len(offered) > 1:
        seen = len(lc.calls())
        click = e.choose(h, 'st-language-admin-language', accept=lambda t: bool(t) and t not in current)
        c = e.call_of(lc, 'SetLocale', seen)
        h.check(f'language-admin: Taal van het aanmeldingsscherm {(click or {}).get("text")} (real clicks) sends locale1 '
                'SetLocale, interactive', bool(c) and c['interactive'] and c['args'].startswith("(['LANG=")
                and bool(click) and click.get('input') == 'clutter', call=c)
    seen = len(lc.calls())
    e.press(h, 'st-language-admin-copy')
    c = e.call_of(lc, 'SetLocale', seen)
    h.check('language-admin: Instellingen kopiëren sends the own language to locale1, interactive',
            bool(c) and c['interactive'] and f"'LANG={e.LANGUAGE}'" in c['args'], call=c)
    h.screenshot(f'language-admin-copied.{h.lang}', 'settings')


def about_page(h, hn):
    text = e.page(h, 'about')
    h.check('about: the name from hostname1', 'WinPoint Test' in text['text'], text=text['text'][:300])
    h.check('about: GNOME’s version from GNOME Shell', e.tr(h, 'GNOME-versie', 'GNOME version') in text['text'])
    h.check('about: Apparaatspecificaties and the OS specifications', bool(e.node(h, 'st-about-device')) and bool(e.node(h, 'st-about-os')))
    if not hn:
        return
    seen = len(hn.calls())
    e.press(h, 'about-rename')
    if e.dialog_open(h):
        h.sleep(0.4)
        h.screenshot(f'about-rename.{h.lang}', 'settings')
        h.type_text('settings', 'st-system-rename-entry', 'Robins Laptop')
        e.dialog_button(h)
        e.dialog_closed(h)
    pretty = e.call_of(hn, 'SetPrettyHostname', seen)
    fixed = e.call_of(hn, 'SetStaticHostname', seen)
    h.check('about: the new name goes to hostname1 as GNOME sends it, interactive',
            bool(pretty) and bool(fixed) and pretty['args'] == "('Robins Laptop', true)"
            and fixed['args'] == "('robins-laptop', true)" and pretty['interactive'] and fixed['interactive'],
            pretty=pretty, fixed=fixed)
    shown = e.until(h, lambda: (e.node(h, 'st-about-name') or {}).get('label') == 'Robins Laptop' or None, 'the new name', 10)
    h.check('about: the page shows hostnamed’s new name (machine.changed)', bool(shown),
            label=(e.node(h, 'st-about-name') or {}).get('label'))


def system_page(h):
    text = e.page(h, 'system')
    h.check('system: the device card with Naam wijzigen', bool(e.node(h, 'hero-rename')))
    # the card reads system.info, whose pretty name is /etc/machine-info's (hostnamed writes it; the stand-in cannot)
    pretty = (h.backend('settings', 'system.info') or {}).get('prettyHostname')
    h.check('system: the device card shows system.info’s name (the /etc/machine-info hostnamed writes)',
            bool(pretty) and pretty in text['text'], pretty=pretty)
    e.press(h, 'hero-rename')
    opened = e.dialog_open(h)
    h.check('system: Naam wijzigen opens Windows’ rename dialog', opened and bool(e.node(h, 'st-system-rename-entry')))
    if opened:
        h.key('settings', 'Escape')
        e.dialog_closed(h, 3)


def switch_language(h):
    other = 'en' if h.lang != 'en' else 'nl'
    back = h.lang
    went = h.set_language(other, via='dropdown')
    h.record('switch to the other language', went)
    h.check(f'switch: Taal van WinPoint {other} switches Settings', h.wait_lang('settings', other, timeout=15) is not None)
    h.check('switch: the page title in the other language', bool(h.wait_text('settings', e.title('language', other), timeout=10)))
    h.screenshot(f'language-switched.{h.lang}', 'settings')
    came = h.set_language(back, via='dropdown')
    h.record('switch back', came)
    h.check(f'switch: and back to {back}', h.wait_lang('settings', back, timeout=15) is not None)


def run(h):
    uid = os.getuid()
    started = time.monotonic()
    td = e.start_live(h, e.timedate1(), 'timedate1')
    hn = e.start_live(h, e.hostname1(), 'hostname1')
    lc = e.start_live(h, e.locale1(), 'locale1')
    acc = e.start_live(h, e.accounts(uid), 'accounts')
    pk = e.start_live(h, e.packagekit(), 'packagekit')

    time_page(h)
    datetime_page(h, td)
    formats_page(h, acc)
    language_page(h, acc, pk)
    language_admin_page(h, lc)
    about_page(h, hn)
    system_page(h)
    switch_language(h)

    h.record('scenario seconds', round(time.monotonic() - started, 1))
    for tag, s in (('timedate1', td), ('hostname1', hn), ('locale1', lc), ('accounts', acc), ('packagekit', pk)):
        e.record_calls(h, tag, s)
        if s:
            s.stop()
