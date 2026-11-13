"""R3-NATIVE: Task Manager's "Always on top" in GNOME (plan.md D-G21; the extension's TaskManagerOnTop).

Run (from G; the step's build and the tree's UI and extension), once per start language:
    tools/heavy.sh --min-mb 2560 --tag r3-native-ontop -- python3 -B tests/gnome/harness.py launch run --lang nl \\
        --build-dir tests/build/native-release --source worktree --scenario tests/gnome/scenarios/r3-ontop.py \\
        --out tests/gnome/scenarios/r3-ontop.out
    (and --lang en --out tests/gnome/scenarios/r3-ontop.en.out)
Each run does the whole cycle in its start language, switches Win11's language and does it again, so a run
covers nl and en; the en run covers the other order.

The cycle, in the language shown (the real switch on Task Manager's Settings page, clicked with the pointer):
  A. Off (the default): the switch is enabled and its description has no "not available" note. Task Manager is
     a normal window: not above, not on all workspaces, and the focused File Explorer is above it.
  B. On: settings.json says taskmgr.alwaysOnTop = true and the extension makes Task Manager's window above and
     sticky (on every workspace). Focusing File Explorer keeps Task Manager on top; on the next workspace Task
     Manager is still there and File Explorer is not.
  C. A Task Manager opened while the switch is on is above and sticky at once (the window-created path).
     Maximised: recorded (GNOME's own rule for above windows that are maximised).
  D. Off again: every window state is as in A (restored, followed live from the settings file).
"""
import json
import time

TM = 'io.github.win11shell.TaskManager'
EXPLORER = 'io.github.win11shell.Explorer'
TITLE = {'nl': 'Altijd op voorgrond', 'en': 'Always on top'}
DESC = {'nl': 'Taakbeheer blijft boven andere vensters en is op elk bureaublad te zien.',
        'en': 'Keeps Task Manager above other windows and on every desktop.'}

# Shell Eval (Main and Meta in scope): the state of Task Manager's window(s) against File Explorer's.
STATE_JS = """(() => {
  const all = global.display.list_all_windows();
  const tms = all.filter(w => w.get_wm_class() === %(tm)s);
  const other = all.find(w => w.get_wm_class() === %(explorer)s);
  const wm = global.workspace_manager;
  const active = wm.get_active_workspace();
  const stack = global.display.sort_windows_by_stacking(all);
  const maximized = w => typeof w.is_maximized === 'function' ? w.is_maximized()
    : w.get_maximized() === Meta.MaximizeFlags.BOTH;
  return {workspace: active.index(), workspaces: wm.get_n_workspaces(),
    focus: global.display.focus_window?.get_wm_class() ?? null,
    explorer: other ? {onActive: other.located_on_workspace(active), stack: stack.indexOf(other)} : null,
    tm: tms.map(w => ({above: w.is_above(), sticky: w.is_on_all_workspaces(), onActive: w.located_on_workspace(active),
      showing: w.showing_on_its_workspace(), maximized: maximized(w), stack: stack.indexOf(w), pid: w.get_pid()}))};
})()""" % {'tm': json.dumps(TM), 'explorer': json.dumps(EXPLORER)}
ACTIVATE_JS = ("(() => {const w = global.display.list_all_windows().find(w => w.get_wm_class() === %s);"
               "if (w) Main.activateWindow(w); return Boolean(w);})()")
WORKSPACE_JS = ("(() => {const wm = global.workspace_manager; while (wm.get_n_workspaces() < %(n)d) "
                "wm.append_new_workspace(false, global.get_current_time());"
                "wm.get_workspace_by_index(%(i)d).activate(global.get_current_time()); return wm.get_active_workspace().index();})()")
MAXIMIZE_JS = ("(() => {const w = global.display.list_all_windows().find(w => w.get_wm_class() === %(tm)s); if (!w) return null;"
               "const m = %(on)s; try { m ? w.maximize() : w.unmaximize(); } catch (e) {"
               "m ? w.maximize(Meta.MaximizeFlags.BOTH) : w.unmaximize(Meta.MaximizeFlags.BOTH); } return true;})()")


def card_js(title: str, what: str) -> str:
    """A JS expression on Task Manager's page: the always-on-top card's toggle (what='track') or its state."""
    card = ("[...document.querySelectorAll('.fl-settings-card')].find(c => "
            f"c.querySelector('.fl-sc__title')?.textContent === {json.dumps(title)})")
    if what == 'track':
        return f"({card})?.querySelector('.fl-toggle__track')"
    return (f"(() => {{const c = {card}; const t = c?.querySelector('.fl-toggle__track');"
            "return c ? {desc: c.querySelector('.fl-sc__desc')?.textContent ?? '', checked: t?.getAttribute('aria-checked'),"
            "disabled: Boolean(t?.disabled), cardDisabled: c.classList.contains('is-disabled')} : null;})()")


def state(h):
    return h.eval(STATE_JS)


def on_top_in_file(h):
    """taskmgr.alwaysOnTop in the private settings.json (None when missing or unreadable)."""
    try:
        data = json.loads((h.hm.RUN / 'config' / 'win11-gnome' / 'settings.json').read_text() or '{}')
    except (OSError, ValueError):
        return None
    return (data.get('taskmgr') or {}).get('alwaysOnTop')


def settings_page(h, lang):
    """Task Manager on its Settings page (the nav footer item, a real click), then the card."""
    h.click('taskmgr', selector='.fl-nav__item[data-id="settings"]')
    return h.until(lambda: h.js('taskmgr', card_js(TITLE[lang], 'state')), f'Task Manager settings card ({lang})',
                   timeout=15)


def flip(h, lang, want: bool):
    """Click the real switch; wait for the file and for the extension to follow."""
    clicked = h.click('taskmgr', find=card_js(TITLE[lang], 'track'))
    file_ok = h.until(lambda: on_top_in_file(h) is want, f'settings.json taskmgr.alwaysOnTop={want}', timeout=10)
    try:
        st = h.until(lambda: (lambda s: s if s['tm'] and all(t['above'] is want and t['sticky'] is want for t in s['tm'])
                              else None)(state(h)), f'Task Manager above/sticky = {want}', timeout=10)
    except TimeoutError:
        st = state(h)
    return {'click': {k: clicked.get(k) for k in ('input', 'x', 'y', 'element')}, 'file': file_ok, 'state': st}


def explorer_focused(h):
    h.eval(ACTIVATE_JS % json.dumps(EXPLORER))
    try:
        return h.until(lambda: (lambda s: s if s['focus'] == EXPLORER else None)(state(h)), 'File Explorer focused',
                       timeout=10)
    except TimeoutError:
        return state(h)


def on_workspace(h, index):
    h.eval(WORKSPACE_JS % {'n': index + 1, 'i': index})
    time.sleep(0.8)
    return state(h)


def cycle(h, lang):
    label = f'{lang}:'
    card = h.record(f'{lang}.card', settings_page(h, lang))
    h.check(f'{label} the always-on-top switch is enabled', card and not card['disabled'] and not card['cardDisabled'],
            card=card)
    h.check(f'{label} its description is the D-G21 text, without a note', card and card['desc'] == DESC[lang],
            desc=card and card['desc'])
    h.check(f'{label} the switch starts off', card and card['checked'] == 'false' and on_top_in_file(h) is not True,
            checked=card and card['checked'], file=on_top_in_file(h))

    # A. off: a normal window
    off = h.record(f'{lang}.A.off', explorer_focused(h))
    tm = off['tm'][0] if off['tm'] else {}
    h.check(f'{label} off: Task Manager is not above and not on all workspaces', tm.get('above') is False
            and tm.get('sticky') is False, state=off)
    h.check(f'{label} off: the focused File Explorer is above Task Manager',
            off['explorer'] and off['explorer']['stack'] > tm.get('stack', 1e9), state=off)
    h.screenshot(f'{lang}-a-off-explorer-in-front')

    # B. on
    on = h.record(f'{lang}.B.on', flip(h, lang, True))
    h.check(f'{label} on: settings.json taskmgr.alwaysOnTop is true', on['file'] is True)
    tm = on['state']['tm'][0] if on['state']['tm'] else {}
    h.check(f'{label} on: Task Manager is above and on all workspaces', tm.get('above') is True
            and tm.get('sticky') is True, state=on['state'])
    focused = h.record(f'{lang}.B.explorerFocused', explorer_focused(h))
    tm = focused['tm'][0] if focused['tm'] else {}
    h.check(f'{label} on: Task Manager stays above the focused File Explorer',
            focused['focus'] == EXPLORER and focused['explorer'] and tm.get('stack', -1) > focused['explorer']['stack'],
            state=focused)
    h.screenshot(f'{lang}-b-on-task-manager-on-top')
    other = h.record(f'{lang}.B.workspace1', on_workspace(h, 1))
    tm = other['tm'][0] if other['tm'] else {}
    h.check(f'{label} on: Task Manager is on the next workspace too, File Explorer is not',
            other['workspace'] == 1 and tm.get('onActive') is True and tm.get('showing') is True
            and other['explorer'] and other['explorer']['onActive'] is False, state=other)
    h.screenshot(f'{lang}-b-on-workspace-2')
    on_workspace(h, 0)

    # C. a Task Manager opened while the switch is on; maximised
    h.close('taskmgr')
    h.open('taskmgr')
    reopened = h.until(lambda: (lambda s: s if s['tm'] else None)(state(h)), 'Task Manager window', timeout=15)
    try:
        reopened = h.until(lambda: (lambda s: s if s['tm'] and s['tm'][0]['above'] and s['tm'][0]['sticky'] else None)(
            state(h)), 'reopened Task Manager above and sticky', timeout=5)
    except TimeoutError:
        pass
    h.record(f'{lang}.C.reopened', reopened)
    tm = reopened['tm'][0] if reopened['tm'] else {}
    h.check(f'{label} on: a Task Manager opened now is above and on all workspaces at once',
            tm.get('above') is True and tm.get('sticky') is True, state=reopened)
    h.eval(MAXIMIZE_JS % {'tm': json.dumps(TM), 'on': 'true'})
    time.sleep(1.0)
    maxed = explorer_focused(h)
    tm = maxed['tm'][0] if maxed['tm'] else {}
    h.record(f'{lang}.C.maximised', {'state': maxed, 'aboveExplorer': bool(maxed['explorer'])
                                     and tm.get('stack', -1) > maxed['explorer']['stack']})
    h.screenshot(f'{lang}-c-on-maximised')
    h.eval(MAXIMIZE_JS % {'tm': json.dumps(TM), 'on': 'false'})
    time.sleep(1.0)

    # D. off again: restored
    settings_page(h, lang)
    off2 = h.record(f'{lang}.D.off', flip(h, lang, False))
    h.check(f'{label} off again: settings.json taskmgr.alwaysOnTop is false', off2['file'] is True)
    focused = h.record(f'{lang}.D.explorerFocused', explorer_focused(h))
    tm = focused['tm'][0] if focused['tm'] else {}
    h.check(f'{label} off again: Task Manager is a normal window (not above, not on all workspaces)',
            tm.get('above') is False and tm.get('sticky') is False, state=focused)
    h.check(f'{label} off again: the focused File Explorer is above Task Manager',
            focused['explorer'] and focused['explorer']['stack'] > tm.get('stack', 1e9), state=focused)
    other = on_workspace(h, 1)
    tm = other['tm'][0] if other['tm'] else {}
    h.check(f'{label} off again: Task Manager is not on the next workspace', tm.get('onActive') is False, state=other)
    on_workspace(h, 0)
    h.screenshot(f'{lang}-d-off-again')


def run(h):
    start = h.lang
    other = 'en' if start == 'nl' else 'nl'
    h.require('settings.json starts with always-on-top off', on_top_in_file(h) is not True, file=on_top_in_file(h))
    h.open('explorer')
    h.open('taskmgr')
    cycle(h, start)
    switch = h.record('switch', h.set_language(other, via='rename'))
    h.check(f'the language switched to {other}', switch['fileLanguageOk'] and switch['seconds'].get('taskmgr') is not None,
            seconds=switch['seconds'])
    cycle(h, other)
    failed = [c['check'] for c in h.results['checks'] if not c['ok']]
    h.require('every check passed', not failed, failed=failed)
