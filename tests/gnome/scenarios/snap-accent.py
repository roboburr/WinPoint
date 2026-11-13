"""Snap preview accent (D-G69): GNOME's and Tiling Assistant's .tile-preview follows the Win11 accent.

Robin (2026-11-13): "de snap preview pane is nu nog oranje, kan het meegaan met accent kleur?". The extension loads
a generated stylesheet with theme.accent into the shell theme. Checks the computed background of a .tile-preview
actor for the accent in the settings, then after theme.accent changes to #E81123.
"""
PREVIEW_JS = ("(()=>{const St=imports.gi.St;const w=new St.Widget({style_class:'tile-preview',width:10,height:10});"
              "Main.uiGroup.add_child(w);const c=w.get_theme_node().get_background_color();const b=w.get_theme_node()"
              ".get_border_color(0);w.destroy();return {bg:[c.red,c.green,c.blue,c.alpha],border:[b.red,b.green,b.blue]};})()")


def run(h):
    first = h.eval(PREVIEW_JS)
    h.record('default', first)
    accent = str(h.settings_file().get('theme', {}).get('accent') or '#0067C0')
    want = [int(accent[i:i + 2], 16) for i in (1, 3, 5)]
    h.check('the snap preview has the accent of the settings', first['bg'][:3] == want and first['bg'][3] < 128,
            preview=first, accent=accent)
    h.hm.write_settings({'theme': {'accent': '#E81123'}})
    try:
        second = h.until(lambda: (lambda v: v if v['bg'][:3] == [232, 17, 35] else None)(h.eval(PREVIEW_JS)),
                         'snap preview follows the accent', 8, 0.3)
    except TimeoutError:
        second = h.eval(PREVIEW_JS)
    h.record('changed', second)
    h.check('the snap preview follows a new accent', second['bg'][:3] == [232, 17, 35], preview=second)
