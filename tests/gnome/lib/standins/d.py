"""Batch D's D-Bus stand-ins for its settings-parity scenarios (notes/claude/settings-parity.md §7): `Spec`s for
`lib.standins.start(h, spec, tag)`, and `start_text` for a stand-in whose properties need GVariant text. Only batch D
edits this file.

``upower(now)``: UPower 1.91 on the private system bus with a laptop battery (76 %, discharging, a charge limit of
75-80 % supported and off, a day of history) and a mains adapter, for Energie en batterij (system.battery.get,
battery.charging.*, battery.history). Its properties do not change: a call is checked in ``standin.calls()``.

``sensor_proxy()``: iio-sensor-proxy (net.hadess.SensorProxy) with an accelerometer and a light sensor, so caps.get
reports `accelerometer` and `ambientLight` (host/src/services/caps.rs) and Beeldscherm shows Draaivergrendeling.

``power_profiles()``: power-profiles-daemon 0.30 with writable ActiveProfile. Its aa{sv} properties use the
shared server's nested $v format. start_text remains a compatibility alias for the common start function.
"""
from __future__ import annotations

from . import Spec, start

UPOWER_PATH = '/org/freedesktop/UPower'
BATTERY = '/org/freedesktop/UPower/devices/battery_BAT0'
ADAPTER = '/org/freedesktop/UPower/devices/line_power_AC'
DISPLAY = '/org/freedesktop/UPower/devices/DisplayDevice'
DEVICE = 'org.freedesktop.UPower.Device'
PROFILES_NAME = 'org.freedesktop.UPower.PowerProfiles'
PROFILES_PATH = '/org/freedesktop/UPower/PowerProfiles'


def _battery_props() -> dict:
    return {
        'Type': ('u', 2), 'PowerSupply': ('b', True), 'IsPresent': ('b', True), 'HasHistory': ('b', True),
        'Percentage': ('d', 76.0), 'State': ('u', 2), 'TimeToEmpty': ('x', 9000), 'TimeToFull': ('x', 0),
        'EnergyRate': ('d', 7.5), 'EnergyFull': ('d', 50.0), 'EnergyFullDesign': ('d', 57.0), 'ChargeCycles': ('i', 120),
        'IconName': ('s', 'battery-good-symbolic'), 'Model': ('s', 'WinPoint Test Battery'),
        'ChargeThresholdSupported': ('b', True), 'ChargeThresholdEnabled': ('b', False),
        'ChargeStartThreshold': ('u', 75), 'ChargeEndThreshold': ('u', 80),
    }


def upower(now: int) -> Spec:
    """The stand-in, with a history of the last 24 hours ending at `now` (seconds)."""
    history = [[now - 3600 * k, float(100 - 2 * k), 2 if k else 1] for k in range(24, -1, -1)]
    return Spec('system', 'org.freedesktop.UPower', {
        UPOWER_PATH: {'org.freedesktop.UPower': {
            'props': {'OnBattery': ('b', True), 'DaemonVersion': ('s', '1.91.1'), 'LidIsPresent': ('b', True)},
            'methods': {'EnumerateDevices': {'in': '', 'out': 'ao', 'value': [[ADAPTER, BATTERY]]},
                        'GetDisplayDevice': {'in': '', 'out': 'o', 'value': [DISPLAY]}},
        }},
        DISPLAY: {DEVICE: {'props': _battery_props()}},
        BATTERY: {DEVICE: {
            'props': _battery_props(),
            'methods': {'EnableChargeThreshold': {'in': 'b', 'out': ''},
                        'GetHistory': {'in': 'suu', 'out': 'a(udu)', 'value': [history]}},
        }},
        ADAPTER: {DEVICE: {'props': {'Type': ('u', 1), 'PowerSupply': ('b', True), 'Online': ('b', False)}}},
    })


def sensor_proxy() -> Spec:
    """iio-sensor-proxy 3.x with an accelerometer (normal orientation) and a light sensor (300 lux)."""
    nothing = {'in': '', 'out': ''}
    return Spec('system', 'net.hadess.SensorProxy', {
        '/net/hadess/SensorProxy': {'net.hadess.SensorProxy': {
            'props': {'HasAccelerometer': ('b', True), 'AccelerometerOrientation': ('s', 'normal'),
                      'HasAmbientLight': ('b', True), 'LightLevelUnit': ('s', 'lux'), 'LightLevel': ('d', 300.0),
                      'HasProximity': ('b', False), 'ProximityNear': ('b', False)},
            'methods': {name: nothing for name in ('ClaimAccelerometer', 'ReleaseAccelerometer', 'ClaimLight',
                                                   'ReleaseLight', 'ClaimProximity', 'ReleaseProximity')},
        }}})


def power_profiles(active: str = 'balanced') -> Spec:
    """power-profiles-daemon with GNOME's three profiles; uses the shared typed-variant builder."""
    profiles = [{'Profile': {'$v': ['s', p]}, 'Driver': {'$v': ['s', 'placeholder']},
                 'PlatformDriver': {'$v': ['s', 'placeholder']}}
                for p in ('power-saver', 'balanced', 'performance')]
    return Spec('system', PROFILES_NAME, {
        PROFILES_PATH: {PROFILES_NAME: {
            'props': {'ActiveProfile': ['s', active], 'Profiles': ('aa{sv}', profiles),
                      'PerformanceInhibited': ('s', ''), 'PerformanceDegraded': ('s', ''), 'Actions': ('as', []),
                      'ActiveProfileHolds': ('aa{sv}', []), 'Version': ('s', '0.30')},
            'writable': ['ActiveProfile'],
            'methods': {'HoldProfile': {'in': 'sss', 'out': 'u', 'value': [1]}, 'ReleaseProfile': {'in': 'u', 'out': ''}},
        }}})


# Older callers use this name; there is only one server implementation.
start_text = start


SPECS: dict[str, "Spec"] = {}
