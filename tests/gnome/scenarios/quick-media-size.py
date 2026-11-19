"""Quick settings stays card-sized with large MPRIS artwork (the 17-11 oversized-panel regression).
Run in the private GNOME harness in nl and en. Uses generated local pictures and a private fake player only.
"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from lib.standins import Spec, start


def player(path, case):
    return Spec('session', f'org.mpris.MediaPlayer2.WinPointTest{case.title()}', {
        '/org/mpris/MediaPlayer2': {
            'org.mpris.MediaPlayer2': {'props': {
                'Identity': ['s', f'WinPoint test {case}'], 'DesktopEntry': ['s', ''],
                'HasTrackList': ['b', False], 'CanQuit': ['b', False], 'CanRaise': ['b', False],
                'SupportedUriSchemes': ['as', []], 'SupportedMimeTypes': ['as', []]}},
            'org.mpris.MediaPlayer2.Player': {
                'props': {'PlaybackStatus': ['s', 'Playing'], 'LoopStatus': ['s', 'None'],
                          'Rate': ['d', 1.0], 'Volume': ['d', 1.0], 'Position': ['x', 0],
                          'Shuffle': ['b', False], 'CanControl': ['b', True], 'CanGoNext': ['b', True],
                          'CanGoPrevious': ['b', True], 'CanPlay': ['b', True], 'CanPause': ['b', True],
                          'CanSeek': ['b', False], 'Metadata': ['a{sv}', {
                              'xesam:title': {'$v': ['s', f'Large media test {case}']},
                              'xesam:artist': {'$v': ['as', ['WinPoint test']]},
                              'mpris:artUrl': {'$v': ['s', path.as_uri()]}}]},
                'methods': {name: {} for name in ('Next', 'Previous', 'PlayPause', 'Play', 'Pause')}}}})


def first(h, cls):
    nodes = h.find('panel', cls=cls)
    return nodes[0] if nodes else None


def run(h):
    import cairo
    for case, width, height in (('landscape', 1920, 1080), ('portrait', 1080, 1920)):
        path = h.hm.RUN / 'home' / f'media-{case}.png'
        image = cairo.ImageSurface(cairo.FORMAT_RGB24, width, height)
        cr = cairo.Context(image)
        cr.set_source_rgb(.12, .35, .7)
        cr.paint()
        cr.set_source_rgb(.8, .5, .15)
        cr.rectangle(width / 4, height / 4, width / 2, height / 2)
        cr.fill()
        image.write_to_png(str(path))
        fake = start(h, player(path, case), f'media-{case}')
        h.require(f'{case}: the private media player starts', fake is not None)
        try:
            h.native({'native': 'flyout', 'name': 'quick'})
            h.until(lambda: h.find('panel', label=f'Large media test {case}'), f'{case}: player shown', 15)
            art = h.until(lambda: first(h, 'fo-media__art'), f'{case}: picture loaded', 15)
            wrap = first(h, 'fo-qs-wrap')
            h.require(f'{case}: quick settings has geometry', bool(wrap and wrap.get('rect')))
            x, y, w, hh = wrap['rect']
            h.record(f'{case}.geometry', {'quick': wrap['rect'], 'art': art.get('rect'), 'source': [width, height]})
            h.check(f'{case}: a large source does not widen the panel', 300 <= w <= 440, width=w)
            h.check(f'{case}: the panel stays within the work area', x >= 0 and y >= 0 and x + w <= 1440
                    and y + hh <= 852, rect=wrap['rect'])
            h.check(f'{case}: artwork stays at card height', art.get('rect', [0, 0, 0, 9999])[3] <= 190,
                    rect=art.get('rect'))
            button = first(h, 'fo-media__next')
            h.require(f'{case}: next-track control is present', button is not None)
            click = h.click('panel', node=button)
            h.until(lambda: any(c['method'] == 'Next' for c in fake.calls()), f'{case}: next-track click', 8)
            h.check(f'{case}: a real pointer click reaches the media control', click.get('input') == 'clutter')
            h.screenshot(f'quick-media-{case}.{h.lang}', area=tuple(wrap['rect']), settle=.5)
            h.native({'native': 'flyout', 'name': 'quick'})
        finally:
            fake.stop()
