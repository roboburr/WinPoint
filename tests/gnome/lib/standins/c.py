"""Batch C's D-Bus stand-ins for its settings-parity scenarios (notes/claude/settings-parity.md §7): `Spec`s for
`lib.standins.start(h, spec, tag)`, and `start_fake` for batch C's stateful daemons. Only batch C edits this file.

* ``GSD_RFKILL``: gnome-settings-daemon's org.gnome.SettingsDaemon.Rfkill on the private session bus, flight mode off,
  AirplaneMode, BluetoothAirplaneMode and WwanAirplaneMode writable: the property GNOME Settings' Flight mode switch
  sets, and amendment 57's ``system.network.setAirplane`` with it (host/src/services/system/network.rs). The builder
  records each Properties.Set as ``Set:<property>`` and answers later reads with the new value.
* ``GSD_RFKILL_HARDWARE``: the same with the hardware switch on (HardwareAirplaneMode): GNOME's "Hardware Airplane
  Mode is On" notice, and a flight mode switch that cannot change it.
* ``start_fake(h, stem, *options)``: a daemon of ``tests/dbus-fakes/fakes/c/<stem>.py`` as a harness stand-in, a child
  of the harness on its private buses, as the batch's bus tests use them (their docstrings list the objects):
  ``networkmanager`` (``--system-only``: NetworkManager with Lab-5G connected on wlan0, eth0 without a cable, an
  OpenVPN and a WireGuard profile, a gsm profile, a modem device), ``modemmanager`` (a modem whose SIM wants its PIN
  1234) and ``sharing`` (systemd's managers, gsd Sharing, gnome-remote-desktop's RDP server and configuration daemon,
  PackageKit; ``--no-user-systemd``, ``--user-systemd-only``, ``--system-only``). The builder of ``lib.standins``
  answers fixed values only; these daemons change their state with the writes (an Update2 is read back, an
  activated profile becomes an ActiveConnection, an installed package brings its units), each refuses an argument
  signature its daemon would refuse, and the system writes of ``sharing`` are refused without the flag that allows
  the password dialog, as polkit's auth_admin_keep does without an agent. Every call is one line of
  ``logs/fake-<tag>.calls.jsonl`` {bus, path, iface, method, signature, args, interactive}; ``StandIn.calls()``
  reads them.
* ``calls_after(standin, before, method, n=1, timeout=20)``: the calls of a method after the first ``before`` calls,
  once there are ``n`` (or what came within the timeout).
"""
from __future__ import annotations

import select
import subprocess
import time
from pathlib import Path

from . import Spec, StandIn

RFKILL = 'org.gnome.SettingsDaemon.Rfkill'
RFKILL_PATH = '/org/gnome/SettingsDaemon/Rfkill'
FAKES = Path(__file__).resolve().parents[3] / 'dbus-fakes' / 'fakes' / 'c'


def gsd_rfkill(airplane: bool = False, hardware: bool = False) -> Spec:
    """gsd's Rfkill plugin as GNOME 50 has it (gsd-rfkill-manager.c's properties)."""
    return Spec('session', RFKILL, {
        RFKILL_PATH: {RFKILL: {
            'props': {'AirplaneMode': ['b', airplane], 'HardwareAirplaneMode': ['b', hardware],
                      'HasAirplaneMode': ['b', True], 'ShouldShowAirplaneMode': ['b', True],
                      'BluetoothAirplaneMode': ['b', airplane], 'BluetoothHardwareAirplaneMode': ['b', hardware],
                      'BluetoothHasAirplaneMode': ['b', True], 'WwanAirplaneMode': ['b', airplane],
                      'WwanHardwareAirplaneMode': ['b', hardware], 'WwanHasAirplaneMode': ['b', False]},
            'writable': ['AirplaneMode', 'BluetoothAirplaneMode', 'WwanAirplaneMode'],
        }},
    })


GSD_RFKILL = gsd_rfkill()
GSD_RFKILL_HARDWARE = gsd_rfkill(hardware=True)

SPECS: dict[str, Spec] = {'gsd-rfkill': GSD_RFKILL, 'gsd-rfkill-hardware': GSD_RFKILL_HARDWARE}


def start_fake(h, stem: str, *options: str, tag: str | None = None, timeout: float = 15) -> StandIn | None:
    """Start ``tests/dbus-fakes/fakes/c/<stem>.py`` with ``options`` as a child of the harness (inside the sandbox, on
    its private buses) and wait until it owns its names ("ready"); records it (``standin.<tag>``), or a failed check
    and None."""
    tag = tag or stem
    logs = h.hm.RUN / 'logs'
    calls = logs / f'fake-{tag}.calls.jsonl'
    calls.write_text('')
    err = open(logs / f'fake-{tag}.log', 'w')
    proc = subprocess.Popen(['python3', '-I', '-B', str(FAKES / f'{stem}.py'), str(calls), *options],
                            stdout=subprocess.PIPE, stderr=err, text=True)
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline and proc.poll() is None:
        if select.select([proc.stdout], [], [], 0.5)[0] and proc.stdout.readline().strip() == 'ready':
            h.record(f'standin.{tag}', {'pid': proc.pid, 'fake': f'fakes/c/{stem}.py', 'options': list(options)})
            return StandIn(proc, calls)
    h.check(f'stand-in {tag} (fakes/c/{stem}.py) owns its names', False, exit=proc.poll())
    proc.kill()
    return None


def calls_after(standin: StandIn | None, before: int, method: str, n: int = 1, timeout: float = 20) -> list[dict]:
    """The calls of ``method`` the stand-in got after its first ``before`` calls, once there are ``n`` (or what came
    within ``timeout`` seconds)."""
    if standin is None:
        return []
    deadline = time.monotonic() + timeout
    found: list[dict] = []
    while time.monotonic() < deadline:
        found = [c for c in standin.calls()[before:] if c.get('method') == method]
        if len(found) >= n:
            break
        time.sleep(0.3)
    return found
