"""Private GTK desktop regression: really drag Trash to the final cell, then restart.

Use harness.py launch run --session-mode winpoint --source worktree --build-dir <19-bin-stage>
--scenario tests/gnome/scenarios/desktop-grid-edge.py --out tests/gnome/scenarios/desktop-grid-edge.out.
The 1/1.5/2 matrix is GNOME TEXT scaling, not fractional monitor scaling. Actual monitor
geometry is recorded; the current harness has no monitor-scale option. No live desktop is used.
"""
import importlib.util
import json
import math
from pathlib import Path

_spec = importlib.util.spec_from_file_location('desktop_grid_mm', Path(__file__).with_name('multi-monitor.py'))
MM = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(MM)

TRASH = 'desktop-item-::trash'
KEY = '::trash'
ORIGIN = 3
BAR = 48
DIMS = {'small': (76, 74), 'medium': (76, 100), 'large': (112, 148)}


def settings(h):
    return json.loads((h.hm.RUN / 'config/win11-gnome/settings.json').read_text())


def settings_set(h, path, value):
    data = settings(h)
    parts = path.split('.')
    parent = data
    for part in parts[:-1]:
        parent = parent.setdefault(part, {})
    parent[parts[-1]] = value
    h.hm.write_settings({parts[0]: data[parts[0]]})
    h.record('private-settings.' + path, value)


def trash(h):
    top = MM.desk_top(h)
    node = MM.node_named(top, TRASH) if top else None
    return (top, node) if node and node.get('rect') and node['rect'][2] > 0 else None


def saved(h):
    return settings(h).get('desktop', {}).get('positions', {}).get(KEY)


def position_is(h, expected):
    p = saved(h)
    return p if p and all(abs(p[k] - expected[k]) < 0.035 for k in ('x', 'y')) else None


def restore_text_scale(h, value):
    if value is not None:
        h.gsettings_set('org.gnome.desktop.interface', 'text-scaling-factor', value)
        return
    # None means the schema default, not the invalid GVariant string "None".
    path, group = h._keyfile(), h._group('org.gnome.desktop.interface', None)
    lines, current = [], None
    for line in path.read_text().splitlines():
        if line.startswith('[') and line.rstrip().endswith(']'):
            current = line.strip()[1:-1]
        if current != group or line.split('=', 1)[0].strip() != 'text-scaling-factor':
            lines.append(line)
    with path.open('r+') as fh:
        fh.write('\n'.join(lines) + '\n')
        fh.truncate()


def axis(span, cell):
    count = max(1, math.floor((span - ORIGIN * 2) / cell))
    return count, (span - ORIGIN * 2 - cell) / (count - 1) if count > 1 else cell


def metrics(h, top, size, text_scale, auto_hide):
    cw, ch = DIMS[size]
    cw, ch = round(cw + (text_scale - 1) * 24), round(ch + (text_scale - 1) * 32)
    frame = MM.desktop_windows(h)['io.github.win11shell.Desktop']['frame']
    monitor = next(v for v in h.monitors() if v['primary'])
    monitor_height = monitor['h'] * top['height'] / frame[3]
    width, height = top['width'], min(top['height'], monitor_height - (0 if auto_hide else BAR))
    cols, dx = axis(width, cw)
    rows, dy = axis(height, ch)
    return {'width': width, 'height': height, 'cw': cw, 'ch': ch,
            'cols': cols, 'rows': rows, 'dx': dx, 'dy': dy, 'autoHide': auto_hide}


def spot(m, p):
    # Rust rounds positive .5 upward; Python's round uses ties-to-even.
    return [math.floor(ORIGIN + p['x'] * m['dx'] + 1.5),
            math.floor(ORIGIN + p['y'] * m['dy'] + 0.5)]


def bounds(h, label, m, p, edge=True):
    top, tile = h.until(lambda: trash(h), label + ': GTK tile', 10)
    x, y = spot(m, p)
    r = tile['rect']
    h.require(label + ': final grid origin', abs(r[0] - x) <= 2 and abs(r[1] - y) <= 2,
              actual=r, expected=[x, y], metrics=m)
    rects = [n['rect'] for n in MM.subtree(tile) if n.get('rect') and n['rect'][2] > 0 and n['rect'][3] > 0]
    h.require(label + ': full GTK tile/image/label are inside usable desktop', bool(rects) and all(
        a >= 0 and b >= 0 and a + w <= m['width'] + 1 and b + ht <= m['height'] + 1
        for a, b, w, ht in rects), rects=rects, usable=[m['width'], m['height']])
    if edge:
        # The entire reserved final cell reaches the edge; actual short labels need not fill its height.
        h.require(label + ': last cell reaches right and bottom without a partial extra cell',
                  abs((x - 1 + m['cw']) - (m['width'] - ORIGIN)) <= 1
                  and abs((y + m['ch']) - (m['height'] - ORIGIN)) <= 1, rect=r, metrics=m)
        # Compare with the independently measured taskbar strip and Mutter work area too:
        # copying the production height-minus-BAR formula alone could miss a double reservation.
        frames = h.eval("(()=>{const r=w=>{const f=w?.get_frame_rect();return f?[f.x,f.y,f.width,f.height]:null;};"
                        "const all=global.get_window_actors().map(a=>a.meta_window);"
                        "return Object.fromEntries(['Desktop','Panel'].map(k=>[k,r(all.find(w=>"
                        "w.get_wm_class()==='io.github.win11shell.'+k))]));})()")
        frame = frames['Desktop']
        ratio = frame[3] / top['height']
        cell_bottom = frame[1] + (y + m['ch']) * ratio
        monitor = next(v for v in h.monitors() if v['primary'])
        if m['autoHide']:
            # The preference alone need not remove Mutter's strut. Icons cannot extend
            # beyond the actual desktop surface while it is still work-area constrained.
            boundary = min(monitor['y'] + monitor['h'], frame[1] + frame[3])
        else:
            panel = next(t for t in h.native_state() if MM.node_named(t, 'tb-strip'))
            strip = MM.node_named(panel, 'tb-strip')['rect']
            boundary = frames['Panel'][1] + strip[1] * frames['Panel'][3] / panel['height']
            work = h.work_area(monitor['index'])
            h.require(label + ': real taskbar matches Mutter work-area boundary',
                      abs(boundary - (work['y'] + work['h'])) <= 2, taskbarTop=boundary, workArea=work)
        h.require(label + ': final cell reaches the actual screen/taskbar boundary without a second reservation',
                  abs(cell_bottom - (boundary - ORIGIN * ratio)) <= 2,
                  cellBottom=cell_bottom, boundary=boundary, desktopFrame=frame,
                  gtkHeight=top['height'], ratio=ratio)
    return top, tile


def drag(h, label, m, p):
    top, tile = trash(h)
    frame = MM.desktop_windows(h).get('io.github.win11shell.Desktop', {}).get('frame')
    h.require(label + ': mapped desktop geometry available', bool(frame) and top['width'] > 0, frame=frame)
    scale_x, scale_y = frame[2] / top['width'], frame[3] / top['height']
    x, y, w, ht = tile['rect']
    ox, oy = min(16, w / 2), min(16, ht / 2)
    target = spot(m, p)
    start = [frame[0] + (x + ox) * scale_x, frame[1] + (y + oy) * scale_y]
    end = [frame[0] + (target[0] + ox) * scale_x, frame[1] + (target[1] + oy) * scale_y]
    steps = [['move', *map(round, start)], ['wait', 140], ['down', 1], ['wait', 100]]
    steps += [['move', round(start[0] + (end[0] - start[0]) * i / 16),
               round(start[1] + (end[1] - start[1]) * i / 16)] for i in range(1, 17)]
    steps += [['wait', 100], ['up', 1], ['wait', 250]]
    MM.pointer(h, steps)
    actual = h.until(lambda: position_is(h, p), label + ': drag persisted', 10)
    h.require(label + ': real drag saved the requested cell', bool(actual), position=actual)
    bounds(h, label, m, p, edge=p['x'] == m['cols'] - 1 and p['y'] == m['rows'] - 1)


def configure(h, size, text_scale, auto_hide=False, aligned=True):
    h.gsettings_set('org.gnome.desktop.interface', 'text-scaling-factor', str(float(text_scale)))
    settings_set(h, 'taskbar.autoHide', auto_hide)
    settings_set(h, 'desktop', {'showIcons': True, 'showTrash': True, 'showThisPC': True,
        'showHome': False, 'autoArrange': False, 'alignToGrid': aligned, 'iconSize': size, 'positions': {}})
    width = DIMS[size][0] + (text_scale - 1) * 24 - 2
    # Theme/settings notifications are asynchronous. A fresh origin and actual GTK allocation
    # prevent accidentally testing the previous size/scale's frame.
    def ready():
        result = trash(h)
        if result and result[1]['rect'][0] <= 5 and abs(result[1]['rect'][2] - width) <= 2:
            return result
        return None
    top, _tile = h.until(ready, 'desktop size/text-scale applied', 15)
    h.sleep(0.5)
    return metrics(h, top, size, text_scale, auto_hide)


def run(h):
    original = settings(h)
    old_text = h.gsettings_get('org.gnome.desktop.interface', 'text-scaling-factor')
    h.record('desktop-grid.monitors', h.monitors())
    h.record('desktop-grid.monitor-scale', h.eval(
        'Main.layoutManager.monitors.map(m=>({index:m.index,scale:global.display.get_monitor_scale(m.index)}))'))
    try:
        for text_scale in (1.0, 1.5, 2.0):
            for size in ('small', 'medium', 'large'):
                label = f'{size}-text-{text_scale:g}'
                m = configure(h, size, text_scale)
                p = {'x': m['cols'] - 1, 'y': m['rows'] - 1}
                drag(h, label, m, p)
                if size == 'large':
                    h.screenshot(label + '-trash-bottom-right', surface='desktop', settle=0.2)

        m = configure(h, 'large', 1.0, auto_hide=True)
        drag(h, 'large-autohide', m, {'x': m['cols'] - 1, 'y': m['rows'] - 1})
        m = configure(h, 'medium', 1.0, aligned=False)
        drag(h, 'medium-unaligned', m, {'x': m['cols'] - 1.35, 'y': m['rows'] - 1.4})

        m = configure(h, 'large', 1.0)
        p = {'x': m['cols'] - 1, 'y': m['rows'] - 1}
        drag(h, 'large-before-restart', m, p)
        before = saved(h)
        h.kill_panel()
        restarted = h.restart_panel()
        h.require('private native panel restarted', restarted.get('panelMapped'), restarted=restarted)
        h.until(lambda: trash(h), 'desktop after native restart', 15)
        h.require('last-cell coordinates survive a native process restart', saved(h) == before, position=saved(h))
        bounds(h, 'large-after-restart', m, p)
        h.screenshot('large-trash-bottom-right-restored', surface='desktop', settle=0.2)
    finally:
        settings_set(h, 'desktop', original.get('desktop', {}))
        settings_set(h, 'taskbar.autoHide', original.get('taskbar', {}).get('autoHide', False))
        restore_text_scale(h, old_text)
