"""Integrated Settings tour, in nl and en, on the private GNOME harness only.
Visits every non-data registry page, verifies reset via GSettings' isDefault rather than keyfile text,
checks that an explicitly empty shortcut list stays [], and switches the real WinPoint language dropdown.
"""
import json

MOUSE = 'org.gnome.desktop.peripherals.mouse'
KEYS = 'org.gnome.desktop.wm.keybindings'


def item(h, schema, key):
    answer = h.backend('settings', 'gsettings.get', {'keys': [{'schema': schema, 'key': key}]}) or {}
    return next((v for v in answer.get('items', []) if v.get('schema') == schema and v.get('key') == key), {})


def reset_checks(h):
    h.open('settings:mouse')
    before = item(h, MOUSE, 'speed')
    h.require('mouse speed is available in the private backend', before.get('available') is True)
    try:
        h.backend('settings', 'gsettings.set', {'schema': MOUSE, 'values': {'speed': .25}})
        h.check('a written mouse speed is a user value', item(h, MOUSE, 'speed').get('isDefault') is False)
        h.backend('settings', 'gsettings.reset', {'schema': MOUSE, 'keys': ['speed']})
        after = item(h, MOUSE, 'speed')
        h.check('reset restores GNOME default ownership, regardless of remaining keyfile text',
                after.get('isDefault') is True, value=after.get('value'))
    finally:
        if before.get('isDefault'):
            h.backend('settings', 'gsettings.reset', {'schema': MOUSE, 'keys': ['speed']})
        else:
            h.backend('settings', 'gsettings.set', {'schema': MOUSE, 'values': {'speed': before.get('value')}})
    before = item(h, KEYS, 'close')
    h.require('the close-window shortcut is available', before.get('available') is True)
    try:
        h.backend('settings', 'input.shortcuts.set', {'schema': KEYS, 'key': 'close', 'accels': []})
        empty = item(h, KEYS, 'close')
        h.check('an explicitly disabled shortcut is an empty list, not an absent value',
                empty.get('value') == [] and empty.get('isDefault') is False, item=empty)
        h.backend('settings', 'input.shortcuts.reset', {'schema': KEYS, 'key': 'close'})
        h.check('the shortcut service resets to the effective default', item(h, KEYS, 'close').get('isDefault') is True)
    finally:
        if before.get('isDefault'):
            h.backend('settings', 'input.shortcuts.reset', {'schema': KEYS, 'key': 'close'})
        else:
            h.backend('settings', 'input.shortcuts.set', {'schema': KEYS, 'key': 'close', 'accels': before.get('value', [])})


def choose(h, name, label):
    """Choose an actual control, never an Aan/Uit state label beside a switch."""
    nodes = h.find('settings', name=name) if name else h.find('settings', label=label)
    choices = [n for n in nodes if 'w11-toggle-state' not in n.get('classes', [])
               and n.get('type') in ('GtkButton', 'GtkCheckButton')]
    h.require(f'{label}: an actionable control is present', bool(choices))
    click = h.click('settings', node=choices[0])
    h.check(f'{label}: a real pointer selects the control', click.get('input') == 'clutter')
    return click


def run(h):
    lang = h.lang
    reset_checks(h)
    pages = json.loads((h.hm.SRC / 'native/shell/src/start/settings_pages.json').read_text())
    h.check('Start offers the complete non-data Settings tree', len(pages) >= 85)
    for page in pages:
        pid, expected = page['id'], page['title'][lang]
        h.open(f'settings:{pid}')
        try:
            h.until(lambda: str(h.text('settings').get('title', '')).startswith(expected), f'{pid}: title', 12)
        except TimeoutError:
            pass
        text = h.page_text('settings')
        h.check(f'{pid}: the requested page and language are shown',
                text.get('lang') == lang and str(text.get('title', '')).startswith(expected), title=text.get('title'))
        h.check(f'{pid}: the page is implemented', not h.find('settings', 'st-placeholder'))
        if page.get('parent') is None:
            h.screenshot(f'tour-{pid}.{lang}', 'settings', settle=.2)
    # The endonyms are exact, independent of the system's display language and of nl/en UI translation.
    h.open('settings:language')
    choose(h, 'language-display', 'WinPoint language')
    for label in ('Nederlands', 'English (United Kingdom)'):
        h.check(f'the WinPoint language list offers {label}', bool(h.find('settings', label=label)))
    h.key('settings', 'Escape')
    other = 'en' if lang == 'nl' else 'nl'
    changed = h.set_language(other, via='dropdown')
    h.check('the language dropdown writes the requested language', changed.get('fileLanguageOk') is True)
    back = h.set_language(lang, via='dropdown')
    h.check('the language dropdown restores the scenario language', back.get('fileLanguageOk') is True)
