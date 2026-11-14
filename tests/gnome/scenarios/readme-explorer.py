"""Only the README's File Explorer picture (docs/screenshots/explorer.png), with the Win11 icon theme (14-11).

Robin (2026-11-14): "kan je een nieuwe foto maken van File Explorer ... maar met de nieuwe icons". The explorer step of
readme-shots.py on its own: Explorer on the home folder, focused over the desktop, the pointer parked, English, dark.

Run (from G):
    tools/heavy.sh --min-mb 2560 --tag readme-explorer -- python3 -B tests/gnome/harness.py launch run --lang en \\
        --build-dir tests/build/native-release --source worktree --scenario tests/gnome/scenarios/readme-explorer.py \\
        --out tests/gnome/scenarios/readme-explorer.out
"""
import importlib.util
from pathlib import Path

_spec = importlib.util.spec_from_file_location('readme_shots', Path(__file__).with_name('readme-shots.py'))
shots = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(shots)


def run(h):
    h.record('boot', h.s.facts.get('nativeBoot'))
    shots.win11_icons(h)
    h.sleep(4)
    shots.clean(h)
    sf = h.open('explorer')
    h.sleep(2.5)
    shots.clean(h)
    shots.restore(h, sf.pid)
    shots.park(h)
    shots.shot(h, 'explorer', settle=1.5)
    h.close('explorer')
    failed = [c['check'] for c in h.results['checks'] if not c['ok']]
    h.record('failed', failed)
