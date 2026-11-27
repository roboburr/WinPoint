"""Fresh WinPoint boot with stale native tiling keys and real inactive TA state 6.

Run via heavy.sh and harness.py launch run --session-mode winpoint --native-snap-fixture
--source worktree --build-dir tests/build/<reviewed-19-binaries> --scenario tests/gnome/scenarios/native-snap.py.
The fixture is seeded before Shell starts. No live session, dconf, compositor or installed extension is touched.
Uses real virtual-keyboard events, not windows.snap, and requires geometry plus restoration/custom-profile checks.
"""
import json
import os
import subprocess
import sys
from pathlib import Path

UUID = 'win11-gnome@win11shell.local'
TA = 'tiling-assistant@ubuntu.com'
KEYS = ('org.gnome.mutter edge-tiling', 'org.gnome.mutter.keybindings toggle-tiled-left',
        'org.gnome.mutter.keybindings toggle-tiled-right')
PROFILE_JS = """(()=>{const G=imports.gi.Gio,m=new G.Settings({schema_id:'org.gnome.mutter'}),
b=new G.Settings({schema_id:'org.gnome.mutter.keybindings'});return {edge:m.get_boolean('edge-tiling'),
left:b.get_strv('toggle-tiled-left'),right:b.get_strv('toggle-tiled-right')};})()"""
KEY_JS = """(async()=>{const C=(await import('gi://Clutter')).default,G=(await import('gi://GLib')).default;
const wait=ms=>new Promise(r=>G.timeout_add(G.PRIORITY_DEFAULT,ms,()=>{r();return G.SOURCE_REMOVE;}));
if(!globalThis.__snapKeys){globalThis.__snapKeys=C.get_default_backend().get_default_seat()
.create_virtual_device(C.InputDeviceType.KEYBOARD_DEVICE);await wait(150);}
const d=globalThis.__snapKeys,keys=KEYS;
for(const k of keys){d.notify_keyval(G.get_monotonic_time(),C['KEY_'+k],C.KeyState.PRESSED);await wait(60);}
for(const k of keys.slice().reverse()){d.notify_keyval(G.get_monotonic_time(),C['KEY_'+k],C.KeyState.RELEASED);await wait(60);}
return true;})()"""

def repaired(h):
    expected = {'edge': True, 'left': ['<Super>Left'], 'right': ['<Super>Right']}
    value = h.until(lambda: (p if (p := h.eval(PROFILE_JS)) == expected else None), 'native snap repaired', 8)
    h.require('all three native snap settings repaired', value == expected, profile=value)

def restore(h):
    result = subprocess.run([str(h.hm.RUN / 'prefix/bin/win11-app'), '--gnome-look'],
                            env=dict(os.environ), capture_output=True, text=True, timeout=30)
    h.record('restore.last', {'exit': result.returncode, 'stderr': result.stderr[-400:]})
    h.require('GNOME preference restoration succeeds', result.returncode == 0)
    h.sleep(0.5)

def geometry(h, pid):
    return h.eval("(()=>{const w=global.get_window_actors().map(a=>a.meta_window).find(w=>w.get_pid()==="
                  + str(pid) + "&&w.get_window_type()===Meta.WindowType.NORMAL);if(!w)return null;"
                  "const r=w.get_frame_rect(),a=w.get_work_area_for_monitor(w.get_monitor());"
                  "return {x:r.x,y:r.y,w:r.width,h:r.height,focus:w.has_focus(),"
                  "work:{x:a.x,y:a.y,w:a.width,h:a.height}};})()")

def run(h):
    fixture = json.loads((h.hm.RUN / 'logs/native-snap-fixture.json').read_text())
    h.record('firstBoot.fixture', fixture)
    h.require('stale keys were seeded before first Shell start', fixture == {
        'beforeFirstShellStart': True, 'edge': False, 'left': [], 'right': [], 'ta': TA, 'expectedState': 6})
    mode = h.eval("(()=>({mode:Main.sessionMode.currentMode,user:global.settings.get_strv('enabled-extensions'),"
                  f"own:Main.extensionManager.lookup('{UUID}')?.state,ta:Main.extensionManager.lookup('{TA}')?.state}}))()")
    h.record('firstBoot.mode', mode)
    h.require('fresh WinPoint mode owns the shell; TA is installed but inactive',
              mode['mode'] == 'winpoint' and mode['user'] == [] and mode['own'] == 1 and mode['ta'] == 6, mode=mode)
    # The private bus has no activation servicedirs. The real host must query Shell directly, not launch a proxy.
    from gi.repository import Gio, GLib
    info = h.s.bus.call_sync('org.gnome.Shell', '/org/gnome/Shell', 'org.gnome.Shell.Extensions', 'GetExtensionInfo',
        GLib.Variant('(s)', (TA,)), GLib.VariantType('(a{sv})'), Gio.DBusCallFlags.NO_AUTO_START, 500, None).unpack()[0]
    h.require('real GetExtensionInfo reports state 6', info.get('state') == 6, state=info.get('state'))
    repaired(h)
    record = json.loads((h.hm.RUN / 'config/win11-gnome/gnome-look.json').read_text())['keys']
    h.record('snap.restore.entries', {key: record.get(key) for key in KEYS})
    h.require('all native snap keys retain the stale user preferences for restoration',
              all(key in record for key in KEYS) and record[KEYS[0]]['gnome'] == 'false'
              and all(record[key]['gnome'] == '@as []' for key in KEYS[1:]))

    proc = h.spawn('native-snap-window', [sys.executable, '-B', str(Path(__file__).with_name('native-snap-window.py'))])
    h.until(lambda: geometry(h, proc.pid), 'small resizable GTK window', 15)
    h.eval("(()=>{const w=global.get_window_actors().map(a=>a.meta_window).find(w=>w.get_pid()==="
           + str(proc.pid) + ");Main.activateWindow(w,global.get_current_time());return true;})()")
    h.until(lambda: (g if (g := geometry(h, proc.pid)) and g['focus'] else None), 'snap fixture focused', 5)
    for direction in ('Left', 'Right'):
        h.eval(KEY_JS.replace('KEYS', json.dumps(['Super_L', direction])))
        def aligned():
            g = geometry(h, proc.pid)
            if not g: return None
            a = g['work']; half = a['w'] // 2
            expected = (a['x'] if direction == 'Left' else a['x'] + half, a['y'],
                        half if direction == 'Left' else a['w'] - half, a['h'])
            return g if all(abs(g[k] - value) <= 2 for k, value in zip(('x', 'y', 'w', 'h'), expected)) else None
        actual = h.until(aligned, 'real Super+' + direction + ' tiles the window', 8, 0.1)
        h.record('geometry.' + direction.lower(), actual)
        h.require('real Super+' + direction + ' produces half-workarea geometry', bool(actual), geometry=actual)
        h.screenshot('native-snap-' + direction.lower(), settle=0.3)

    restore(h)
    h.require('all three original stale preferences restored outside WinPoint effects',
              h.eval(PROFILE_JS) == {'edge': False, 'left': [], 'right': []}, profile=h.eval(PROFILE_JS))
    # A second private panel start re-applies only the broken fingerprint; then a user's change must win restoration.
    h.kill_panel(); h.restart_panel(); repaired(h)
    h.gsettings_set('org.gnome.mutter.keybindings', 'toggle-tiled-left', "['<Alt>Left']")
    h.until(lambda: h.eval(PROFILE_JS)['left'] == ['<Alt>Left'], 'custom user shortcut committed', 5)
    restore(h)
    profile = h.eval(PROFILE_JS)
    h.require('restoration preserves a later custom shortcut and restores only owned keys',
              profile == {'edge': False, 'left': ['<Alt>Left'], 'right': []}, profile=profile)
