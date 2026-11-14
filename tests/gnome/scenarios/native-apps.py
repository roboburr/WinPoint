"""Native shell with our own apps (13-11): their caption buttons and their shadow while dragging.

Robin (2026-11-13), with shell.native on: "nog steeds zie ik schaduw dat blijf hangen, maar alleen bij onze eigen
apps", "ik zie geen windows 3 knoppen rechts boven bij alle apps". The native shell is the only shell since D-G70
(the harness waits for its boot); this opens Settings and Explorer (since 13-11 the native GTK 4 apps), records their
caption buttons from their dev state (frosted-glass.py captions()), drags Settings by its title bar with a virtual
pointer, and lists every frosted backdrop actor in the window group (an orphan would be a shadow that stays behind),
with screenshots before and after.

Run (from G):
    tools/heavy.sh --min-mb 2560 --tag native-apps -- python3 -B tests/gnome/harness.py launch run --lang nl \\
        --build-dir tests/build/native-release --source worktree --scenario tests/gnome/scenarios/native-apps.py
"""
import importlib.util
from pathlib import Path

HERE = Path(__file__).resolve().parent


def load(name):
    spec = importlib.util.spec_from_file_location(name.replace('-', '_') + '_lib', HERE / f'{name}.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


fg = load('frosted-glass')

BACKDROPS_JS = ("(()=>{const w=global.get_window_actors().map(a=>a.meta_window).find(w=>w.get_wm_class()==="
                "'io.github.win11shell.Settings');const a=w?.get_compositor_private();"
                "return {window:a?{x:Math.round(a.x),y:Math.round(a.y),w:Math.round(a.width),h:Math.round(a.height)}:null,"
                "backdrops:global.window_group.get_children().filter(c=>(c.name||'').startsWith('win11-frosted'))"
                ".map(c=>({name:c.name,x:Math.round(c.x),y:Math.round(c.y),w:Math.round(c.width),h:Math.round(c.height),"
                "visible:c.visible,constraints:c.get_constraints().length,style:(c.get_style()||'').slice(0,80)}))};})()")


def switch(h):
    """The native shell's boot row (D-G70: the harness waited for it before the scenario; nothing to switch)."""
    boot = h.s.facts.get('nativeBoot')
    if not boot:
        boot = h.s.native_boot(0, 60)
    return boot


def run(h):
    h.record('boot', switch(h))
    h.sleep(4)
    h.open('settings')
    h.sleep(2)
    caps = fg.captions(h, 'settings')
    h.record('settings.captions', caps)
    h.check('Settings shows its three caption buttons', caps['visible'] == 3, caps=caps)
    h.record('before', h.eval(BACKDROPS_JS))
    h.screenshot('a-settings-before-drag')
    real = h.eval(fg.REAL_DRAG_JS)
    h.record('drag', real)
    h.sleep(1.5)
    after = h.eval(BACKDROPS_JS)
    h.record('after', after)
    h.screenshot('b-settings-after-drag')
    win = after.get('window') or {}
    stray = [b for b in after.get('backdrops', []) if b['name'] == 'win11-frosted-own-window' and b['visible']
             and win and (abs(b['x'] - win['x']) > 2 or abs(b['y'] - win['y']) > 2)]
    h.check('no own-window backdrop away from its window after the drag', not stray, stray=stray, after=after)
    h.check('the drag moved Settings', isinstance(real, dict) and real.get('moved', {}).get('dx', 0) >= 60, real=real)
    h.open('explorer')
    h.sleep(2)
    ecaps = fg.captions(h, 'explorer')
    h.record('explorer.captions', ecaps)
    h.check('Explorer shows its three caption buttons', ecaps['visible'] == 3, caps=ecaps)
    h.screenshot('c-explorer')
    # D-G65: the panel host (here the native shell's backend) imports the Windows 11 caption buttons for every
    # other GTK app from the user's gtk.css, as the setting theme.captionButtons says (on by default).
    gtk = {v: (h.hm.RUN / 'config' / f'gtk-{v}.0' / 'gtk.css') for v in (3, 4)}
    text = {v: (p.read_text() if p.is_file() else '') for v, p in gtk.items()}
    h.record('gtk.css', {v: t[-400:] for v, t in text.items()})
    for v in (3, 4):
        h.check(f'GTK {v} apps import the Windows 11 caption buttons', f'caption-buttons-gtk{v}.css' in text[v],
                tail=text[v][-300:])
