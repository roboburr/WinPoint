"""GE1c scenario (review stage, notes/claude/round1/ge1c-review.md): the extension follows Win11's language live
(contracts C1), its failure texts and notifications are in that language (C3, plan-critique F4), and the fallback
St taskbar shows the right language after the panel stops. Keeps every check of the owner's version.

Order:
 1. nl: extension_labels() (labels, accessible names, Ctrl+Alt+Tab group, clock), the GNOME names of the own
    entries (install.py, C6), a bridge failure (in-process Call) and the "app missing" GNOME notification
    (Main.notify). D-G70: the taskbar is the native shell's (no page), so the checks of the HTML panel (its failure
    toast through the host relay, the Win11 toast mirror of the notification, its <html lang>) are gone.
 2. nl -> en through the real Settings dropdown (Settings writes settings.json itself); a probe in the extension
    times its re-label against the file's mtime. Same checks in en.
 3. C6 at runtime, simulated: the harness rewrites Name= of the Explorer entry the way win11-panel will (GE1b;
    the 3b898e6 binaries do not), so the extension's new installed-changed handler is tested on its own.
 4. en -> nl by rename (Settings closed), nl -> en by in-place rewrite.
 5. C1 edge values live (from en): "EN", the object form, settings.json moved away, then back.
 6. kill_panel(): the fallback bar in en; switch to nl and back while it is visible; widths measured only.

Run (from G):
    tools/heavy.sh --min-mb 2560 --tag ge1c-review -- python3 -B tests/gnome/harness.py launch run --lang nl \\
        --build-dir tests/build/native-release --scenario tests/gnome/scenarios/ge1c-extension.py
Results: tests/gnome/scenarios/ge1c-extension.out/ (results.json + PNGs).
"""
import json
import os
import re
import time

UUID = 'win11-gnome@win11shell.local'
NAMES = {'nl': ['Start', 'Zoeken', 'Taakweergave', 'Netwerk, geluid en snelle instellingen'],
         'en': ['Start', 'Search', 'Task view', 'Network, sound and quick settings']}
SEARCH_LABEL = {'nl': 'Zoeken', 'en': 'Search'}
GROUP = {'nl': ['Win11-taakbalk'], 'en': ['Win11 Taskbar']}
CLOCK = {'nl': r'\n\d\d-\d\d-\d{4}$', 'en': r'\n\d\d/\d\d/\d{4}$'}
# en-GB in Windows 11 wording since the GE1c fix round (ge1c-review M1); nl unchanged.
FAILURE = {'nl': 'Het gevraagde app-venster bestaat niet meer.', 'en': 'The requested app window no longer exists.'}
NOTIFY = {'nl': ('Win11 voor GNOME', 'Deze app is nog niet geïnstalleerd.'),
          'en': ('Win11 for GNOME', 'This app isn’t installed yet.')}
EXPLORER_ID = 'io.github.win11shell.Explorer.desktop'
EXPLORER_NAME = {'nl': 'Win11 Verkenner', 'en': 'Win11 File Explorer'}
ENTRY = {'nl': {EXPLORER_ID: 'Win11 Verkenner', 'io.github.win11shell.TaskManager.desktop': 'Win11 Taakbeheer'},
         'en': {EXPLORER_ID: 'Win11 File Explorer', 'io.github.win11shell.TaskManager.desktop': 'Win11 Task Manager'}}
STATE = "Main.extensionManager.lookup('" + UUID + "').stateObj"
WIDTH = ("(()=>{const s=" + STATE + ";const b=s&&s._searchButton;"
         "return b?{button:b.get_preferred_width(-1)[1],label:b.child.get_preferred_width(-1)[1],"
         "compact:b.has_style_class_name('compact'),text:b.child.text,bar:s._bar.width,visible:s._bar.visible}:null;})()")
CALL = ("JSON.parse(" + STATE + "._panelBridge.Call('windows.focus', JSON.stringify({id: 'ge1c-nope'})))")
# The extension's language watcher, wrapped once so every re-label is timed (Date.now(), realtime ms).
ARM = ("(()=>{const s=" + STATE + ";const w=s&&s._language;if(!w)return {error:'no LanguageWatch'};"
       "globalThis.__ge1cRelabels??=[];if(!w.__ge1cWrapped){const orig=w._onChange;"
       "w._onChange=l=>{globalThis.__ge1cRelabels.push({lang:l,ms:Date.now()});return orig(l);};w.__ge1cWrapped=true;}"
       "return {armed:true,language:w.language,dirMonitor:!!w._dirMonitor,parentMonitor:!!w._parentMonitor,"
       "dir:w._dir.get_path()};})()")
RELABELS = "globalThis.__ge1cRelabels||[]"
# The fallback bar's "app missing" path (_launch of an id that is not installed) -> Main.notify; read back at once,
# because Main.notify notifications are transient.
MISSING = ("(async()=>{const MT=await import('resource:///org/gnome/shell/ui/messageTray.js');const s=" + STATE + ";"
           "s._launch('io.github.win11shell.GE1cMissing.desktop');const src=MT.getSystemSource();"
           "return {source:src.title,notifications:(src.notifications||[]).map(n=>({title:n.title,body:n.body}))};})()")
CLEAR = ("(async()=>{const MT=await import('resource:///org/gnome/shell/ui/messageTray.js');const src=MT.getSystemSource();"
         "let n=0;for(const x of [...(src.notifications||[])])if(/Win11 (voor|for) GNOME/.test(x.title||'')){x.destroy();n++;}"
         "return n;})()")
def own_names(value):
    """The extension's own buttons: Start, Search, Task view (first three) and the system button (last); the app
    buttons between them carry GNOME's app names (.desktop Name=, the system locale for other apps)."""
    names = value.get('accessibleNames', []) if isinstance(value, dict) else []
    return names[:3] + names[-1:]


def labels_ok(value, lang):
    return (isinstance(value, dict) and own_names(value) == NAMES[lang] and value.get('ctrlAltTab') == GROUP[lang]
            and bool(re.search(CLOCK[lang], value.get('clock') or '')))


def wait_labels(h, lang, label, timeout=5.0):
    """Poll extension_labels() until all own texts are in `lang`; returns (value, seconds or None)."""
    started = time.monotonic()
    value = None
    try:
        value = h.until(lambda: (lambda v: v if labels_ok(v, lang) else None)(h.extension_labels()),
                        f'{label}: extension labels in {lang}', timeout, 0.1)
        seconds = round(time.monotonic() - started, 2)
    except TimeoutError:
        seconds = None
    return value or h.extension_labels(), seconds


def labels_in(h, lang, label, wait=0.0, panel_alive=True):
    """The extension's own texts in `lang`; with wait > 0, poll until they are (a live switch)."""
    if wait:
        value, seconds = wait_labels(h, lang, label, wait)
        h.record(f'{label}.pollSeconds', seconds)
    else:
        value = h.extension_labels()
    h.check(f'{label}: accessible names {lang}', own_names(value) == NAMES[lang], seen=value.get('accessibleNames'))
    h.check(f'{label}: Ctrl+Alt+Tab group {lang} (exactly one entry for the bar)', value.get('ctrlAltTab') == GROUP[lang],
            seen=value.get('ctrlAltTab'), all=value.get('ctrlAltTabAll'))
    h.check(f'{label}: clock date format {lang}', bool(re.search(CLOCK[lang], value.get('clock') or '')),
            seen=value.get('clock'))
    h.check(f'{label}: search label {lang}', any(x.endswith(SEARCH_LABEL[lang]) or x == '⌕' for x in value.get('labels', [])),
            labels=value.get('labels'))
    if lang == 'en':
        own = [*value.get('labels', []), *own_names(value), *value.get('ctrlAltTab', [])]
        own = [x for x in own if x not in (EXPLORER_NAME['nl'],)]     # GNOME's app name, checked separately
        h.check(f'{label}: no Dutch word in the extension\'s own texts', not h._hits(*own), hits=h._hits(*own))
    reply = h.eval(CALL)
    h.check(f'{label}: bridge failure text {lang} (in-process Call)', reply.get('error', {}).get('message') == FAILURE[lang],
            code=reply.get('error', {}).get('code'), seen=reply.get('error', {}).get('message'))
    missing_notification(h, lang, label, panel_alive)
    h.record(f'{label}.extension', value)
    h.record(f'{label}.searchWidth', h.eval(WIDTH))      # measured only (no layout change in round 1)
    return value


def missing_notification(h, lang, label, panel_alive):
    """The extension's own failure notification (Main.notify) in `lang` (D-G70: the native taskbar shows no Win11
    toast mirror yet, gtk-native.md "toasts"; while it runs, a screenshot of its corner is kept for the look)."""
    seen = h.eval(MISSING)
    rows = seen.get('notifications', []) if isinstance(seen, dict) else []
    title, body = NOTIFY[lang]
    h.check(f'{label}: "app missing" notification {lang}', any(r.get('title') == title and r.get('body') == body for r in rows),
            seen=rows, source=seen.get('source') if isinstance(seen, dict) else None)
    if panel_alive:
        h.screenshot(f'{label}-missing-notification', area=(1440 - 420, 900 - 48 - 260, 420, 260), settle=1.0)
    h.record(f'{label}.cleared', h.eval(CLEAR))


def relabel_latency(h, label, before_count):
    """ms from the settings.json mtime to the extension's re-label (the probe in ARM)."""
    rows = h.eval(RELABELS) or []
    new = rows[before_count:]
    mtime = h.settings_file().get('mtime')
    latency = [round(r['ms'] - mtime * 1000) for r in new] if mtime else None
    h.record(f'{label}.relabels', {'rows': new, 'msAfterFileMtime': latency})
    return len(rows)


def rewrite_name(h, entry_id, name):
    """What win11-panel will do (C6, GE1b): replace only the Name= line, atomically (temp file + rename)."""
    path = h.hm.RUN / 'data' / 'applications' / entry_id
    text = path.read_text()
    new = re.sub(r'(?m)^Name=.*$', 'Name=' + name, text, count=1)
    temporary = path.with_name('.' + entry_id + '.ge1c-review')
    temporary.write_text(new)
    os.chmod(temporary, 0o644)
    os.replace(temporary, path)
    return {'changedLines': sum(1 for a, b in zip(text.splitlines(), new.splitlines()) if a != b)}


def explorer_button(h, name, label, timeout=8.0):
    started = time.monotonic()
    try:
        h.until(lambda: name in (h.extension_labels().get('accessibleNames') or []), f'{label}: pinned button {name!r}',
                timeout, 0.2)
        seconds = round(time.monotonic() - started, 2)
    except TimeoutError:
        seconds = None
    h.check(f'{label}: pinned File Explorer button shows {name!r} after the Name= rewrite (installed-changed)',
            seconds is not None, seconds=seconds, gnome=h.app_names([EXPLORER_ID]))


def c6_runtime(h, label, name, timeout=12.0):
    """C6 end to end: after a switch win11-panel rewrites Name= (GE1b) and GNOME shows it on the pinned button.
    GNOME's desktop-dir monitor and Shell.AppSystem take about 5 s (ge1c-review m9), hence 12 s. The check is a
    hard one with a panel that has the rewriter (any --build-dir but the 3b898e6 one); with the 3b898e6 panel it is
    the known C6-runtime gap."""
    started = time.monotonic()
    try:
        h.until(lambda: name in (h.extension_labels().get('accessibleNames') or []), f'{label}: C6 {name!r}', timeout, 0.25)
        seconds = round(time.monotonic() - started, 2)
    except TimeoutError:
        seconds = None
    baseline = h.hm.BUILD.name == '3b898e6-debug'
    h.check(f'{label}: pinned File Explorer button follows the .desktop Name= rewritten by win11-panel (C6)',
            seconds is not None, known='C6-runtime' if baseline else None, seconds=seconds, build=str(h.hm.BUILD),
            gnome=h.app_names([EXPLORER_ID]))


def run(h):
    h.require('session starts in Dutch', h.lang == 'nl', lang=h.lang)
    h.record('arm', h.eval(ARM))
    count = 0

    # 1. Dutch -----------------------------------------------------------------------------------------------
    labels_in(h, 'nl', 'nl')
    names = h.app_names()
    h.record('nl.appNames', names)
    h.check('nl: GNOME names of the own entries (install.py, C6)', all(names.get(k) == v for k, v in ENTRY['nl'].items()),
            seen=names)

    # 2. nl -> en through the real Settings dropdown -----------------------------------------------------------
    switched = h.set_language('en', via='dropdown')
    h.record('switch.en.dropdown', switched)
    h.check('dropdown: Settings wrote language=en by rename', switched.get('fileLanguageOk') and not switched.get('sameInode'),
            input=switched.get('input'), seconds=switched.get('seconds'))
    count = relabel_latency(h, 'en-dropdown', count)
    labels_in(h, 'en', 'en-dropdown', wait=5)
    c6_runtime(h, 'en-dropdown', EXPLORER_NAME['en'])
    h.screenshot('01-en-panel-after-dropdown', area=(0, 900 - 48, 1440, 48))
    h.close('settings')

    # 3. C6 at runtime, simulated: the extension's installed-changed handler on its own ---------------------------
    h.record('c6.rewrite.en', rewrite_name(h, EXPLORER_ID, EXPLORER_NAME['en']))
    explorer_button(h, EXPLORER_NAME['en'], 'c6-en')
    h.record('c6.rewrite.nl', rewrite_name(h, EXPLORER_ID, EXPLORER_NAME['nl']))
    explorer_button(h, EXPLORER_NAME['nl'], 'c6-nl')

    # 4. rename and in-place rewrite ---------------------------------------------------------------------------
    switched = h.set_language('nl', via='rename')
    h.record('switch.nl.rename', switched)
    count = relabel_latency(h, 'nl-rename', count)
    labels_in(h, 'nl', 'nl-rename', wait=5)
    switched = h.set_language('en', via='rewrite')
    h.record('switch.en.rewrite', switched)
    count = relabel_latency(h, 'en-rewrite', count)
    labels_in(h, 'en', 'en-rewrite', wait=5)

    # 5. C1 edge values, live, from en ----------------------------------------------------------------------------
    for value in ('EN', {'custom': 'ge1c'}):
        h.hm.rewrite_settings_in_place({'language': value})
        lab = 'c1-' + ('EN' if value == 'EN' else 'object')
        got, seconds = wait_labels(h, 'nl', lab, 5)
        h.check(f'{lab}: language {json.dumps(value)} means nl in the extension (C1)', labels_ok(got, 'nl'),
                seconds=seconds, seen=own_names(got))
        count = relabel_latency(h, lab, count)
        h.hm.write_settings({'language': 'en'})
        wait_labels(h, 'en', lab + '-back', 5)
    config = h.hm.RUN / 'config' / 'win11-gnome'
    os.rename(config / 'settings.json', config / 'settings.json.ge1c-away')
    got, seconds = wait_labels(h, 'nl', 'c1-missing', 5)
    h.check('c1-missing: settings.json moved away means nl (C1)', labels_ok(got, 'nl'), seconds=seconds, seen=own_names(got))
    os.rename(config / 'settings.json.ge1c-away', config / 'settings.json')
    got, seconds = wait_labels(h, 'en', 'c1-back', 5)
    h.check('c1-back: settings.json moved back gives en again', labels_ok(got, 'en'), seconds=seconds, seen=own_names(got))
    count = relabel_latency(h, 'c1-moves', count)

    # 6. fallback bar --------------------------------------------------------------------------------------------
    killed = h.kill_panel()
    h.record('fallback.kill', {k: v for k, v in killed.items() if k != 'before'})
    h.check('fallback bar shown after the panel stopped', killed.get('fallbackShownSeconds') is not None,
            seconds=killed.get('fallbackShownSeconds'))
    after = killed.get('after') or {}
    h.check('fallback bar labels English (the language at kill time)',
            any(x.endswith('Search') or x == '⌕' for x in after.get('labels', [])) and labels_ok(after, 'en'),
            labels=after.get('labels'), names=own_names(after))
    labels_in(h, 'en', 'en-fallback', panel_alive=False)
    h.screenshot('02-en-fallback-bar', area=(0, 900 - 48, 1440, 48))
    switched = h.set_language('nl', via='rename', skip=('panel', 'start'))
    h.record('switch.nl.fallback', switched)
    labels_in(h, 'nl', 'nl-fallback', wait=5, panel_alive=False)
    h.screenshot('03-nl-fallback-bar', area=(0, 900 - 48, 1440, 48))
    switched = h.set_language('en', via='rename', skip=('panel', 'start'))
    h.record('switch.en.fallback', switched)
    labels_in(h, 'en', 'en-fallback-2', wait=5, panel_alive=False)
    relabel_latency(h, 'fallback', count)
    h.record('notifications', h.notifications())
    h.log('ge1c review scenario done')
