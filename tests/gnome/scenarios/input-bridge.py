"""Integrated A/I proof: asynchronous shell capture and live taskbar input sources, private GNOME only."""
import json
import concurrent.futures
import importlib.util
from pathlib import Path

spec = importlib.util.spec_from_file_location('input_helpers', Path(__file__).with_name('settings-input.py'))
A = importlib.util.module_from_spec(spec)
spec.loader.exec_module(A)
SCHEMA = 'org.gnome.desktop.input-sources'


def run(h):
    h.open('settings:typing')
    before = h.gsettings_get(SCHEMA, 'sources')
    try:
        h.gsettings_set(SCHEMA, 'sources', "[('xkb', 'us'), ('xkb', 'nl')]")
        current = h.until(lambda: (lambda v: v if len(v.get('sources', [])) == 2 else None)(
            h.backend('settings', 'input.sources.current') or {}), 'two GNOME input sources', 15)
        h.record('inputSources', current)
        h.check('input.sources.current reaches the GNOME input bridge',
                {s['id'] for s in current['sources']} == {'us', 'nl'})
        h.until(lambda: [n for n in h.find('panel', 'tb-input') if n.get('visible', True) and n.get('rect')],
                'the live tray indicator', 12)
        target = next(s for s in current['sources'] if s['index'] != current['current'])
        answer = h.backend('settings', 'input.sources.activate', {'index': target['index']}) or {}
        h.check('input.sources.activate reaches GNOME', answer.get('current') == target['index'], answer=answer)
        h.until(lambda: (h.backend('settings', 'input.sources.current') or {}).get('current') == target['index'],
                'GNOME current source changes', 10)
        def indicator_matches():
            nodes = h.find('panel', 'tb-input')
            text = str(nodes)
            return nodes and target['short'].upper() in text
        h.check('the sourceChanged event updates the taskbar label', bool(h.until(indicator_matches, 'tray source label', 10)))
        h.screenshot(f'input-indicator.{h.lang}', area=(1100, 840, 340, 60), settle=.3)
        # A modal shell capture must leave D-Bus responsive while its Promise is pending.
        with concurrent.futures.ThreadPoolExecutor(max_workers=1) as pool:
            capture = pool.submit(h.backend, 'settings', 'input.capture', {'timeout': 10000}, 20)
            try:
                h.sleep(.7)
                h.check('CallAsync waits for the key instead of returning a Promise object', not capture.done())
                h.check('GNOME stays responsive during the asynchronous capture', h.eval('1+1') == 2)
                A.real_keys(h, ['Control_L', 'Alt_L', 'q'])
                result = capture.result(timeout=15) or {}
                h.check('real captured keys return through CallAsync to the host',
                        result.get('accel') == '<Control><Alt>q', answer=result)
            finally:
                h.backend('settings', 'input.capture.cancel')
        h.gsettings_set(SCHEMA, 'sources', "[('xkb', 'us')]")
        h.until(lambda: len((h.backend('settings', 'input.sources.current') or {}).get('sources', [])) == 1,
                'one remaining source', 12)
        h.check('the tray indicator hides with one source',
                not [n for n in h.find('panel', 'tb-input') if n.get('visible', True) and n.get('rect')])
        # No physical tablet exists in this test session: prove routing, not an actual pad OSD.
        from gi.repository import Gio, GLib
        bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
        reply = bus.call_sync('io.github.win11shell.GnomeBridge', '/io/github/win11shell/GnomeBridge',
                              'io.github.win11shell.GnomeBridge', 'Call',
                              GLib.Variant('(ss)', ('input.padOsd', json.dumps({'vidpid': 'ffff:ffff'}))),
                              GLib.VariantType.new('(s)'), Gio.DBusCallFlags.NO_AUTO_START, 3000, None)
        pad = json.loads(reply.unpack()[0])
        error = pad.get('error') or {}
        h.check('padOsd reaches the bridge and reports missing hardware', error.get('code') == 'not_found', answer=pad)
    finally:
        if before is not None:
            h.gsettings_set(SCHEMA, 'sources', before)
