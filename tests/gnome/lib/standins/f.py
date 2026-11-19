"""Batch F's D-Bus stand-ins for its settings-parity scenarios (notes/claude/settings-parity.md §7): `Spec`s for
``start(h, spec, tag)``. Only batch F edits this file.

The shared ``lib.standins.start`` builder supports these reply forms:

* a method may answer per call: ``byArgs`` maps the call's arguments as GVariant text (``"('devices', 'camera')"``)
  to the value it answers then (else ``value``), so one PermissionStore answers each table as GNOME stores it;
* a value may hold ``{"$v": [signature, value]}`` for a ``v`` (GLib.Variant(signature, value)), so fwupd's
  ``aa{sv}`` attributes and events can be served (handoff note slice 3, note 5: a plain JSON spec cannot);
* a method with ``"permissionStore": true`` is the PermissionStore's ``SetPermission (s table, b create, s id,
  s app, as values)``: it stores the app's values in ``Lookup``'s answer for (table, id) and emits ``Changed (s, s,
  b, v, a{sas})`` with the whole entry, as xdg-permission-store does, whoever called it (the page through the host,
  or the scenario as a portal dialog answered elsewhere).

The stand-ins:

* ``whoopsie``: Ubuntu's whoopsie-preferences on the private system bus (Diagnostische gegevens en feedback,
  privacy.reports.*): Handmatig (ReportCrashes true, AutomaticallyReportCrashes false); its Set methods record what
  the page sent, with the interactive flag (amendment 52, call_interactive).
* ``fwupd``: fwupd on the private system bus (Apparaatbeveiliging, privacy.security): HSI:1 with a runtime issue
  (``HSI:1!``), five attributes as fwupd 2.0 sends them (UEFI Secure Boot enabled and TPM 2.0 found at level 1, IOMMU
  not found at level 2, kernel lockdown disabled at level 3, a tainted kernel while running with the runtime flag)
  and one event (Secure Boot went from not enabled to enabled).
* ``permission-store``: xdg-desktop-portal's PermissionStore on the private session bus (Locatie, Camera, Microfoon,
  Geavanceerde opties): Bestanden (``org.gnome.Nautilus``, a desktop app GNOME Shell stores by its desktop id) as GNOME
  stores it per table: location EXACT with the time of its last access, camera ``yes``, microphone ``no``; every other
  table has no entry for it (an empty entry, as a store that never asked). SetPermission keeps what it is given and
  emits ``Changed``.
"""
from __future__ import annotations

from . import SERVER, Spec, StandIn, start

# The common builder supports variants, byArgs and permissionStore for every batch.
SERVER_F = SERVER


WHOOPSIE = Spec('system', 'com.ubuntu.WhoopsiePreferences', {
    '/com/ubuntu/WhoopsiePreferences': {'com.ubuntu.WhoopsiePreferences': {
        'props': {'ReportCrashes': ('b', True), 'AutomaticallyReportCrashes': ('b', False), 'ReportMetrics': ('b', False)},
        'methods': {'SetReportCrashes': {'in': 'b'}, 'SetAutomaticallyReportCrashes': {'in': 'b'},
                    'SetReportMetrics': {'in': 'b'}, 'GetIdentifier': {'out': 's', 'value': ['0123456789abcdef']}},
    }}})


def _attr(appstream: str, level: int, result: int, flags: int, plugin: str, created: int = 1763280000) -> dict:
    """One fwupd security attribute (fu_security_attr_to_variant's keys); flags: 1 success, 1<<10 runtime issue."""
    return {'AppstreamId': {'$v': ['s', appstream]}, 'HsiLevel': {'$v': ['u', level]},
            'HsiResult': {'$v': ['u', result]}, 'Flags': {'$v': ['t', flags]}, 'Plugin': {'$v': ['s', plugin]},
            'Created': {'$v': ['t', created]}}


# FwupdSecurityAttrResult: 1 enabled, 2 not enabled, 9 tainted, 11 found, 12 not found
FWUPD_ATTRS = [
    _attr('org.fwupd.hsi.Uefi.SecureBoot', 1, 1, 1, 'uefi_capsule'),
    _attr('org.fwupd.hsi.Tpm.Version20', 1, 11, 1, 'tpm'),
    _attr('org.fwupd.hsi.Iommu', 2, 12, 0, 'iommu'),
    _attr('org.fwupd.hsi.Kernel.Lockdown', 3, 2, 0, 'linux_lockdown'),
    _attr('org.fwupd.hsi.Kernel.Tainted', 0, 9, 1 << 10, 'linux_tainted'),
]
FWUPD_EVENTS = [{**_attr('org.fwupd.hsi.Uefi.SecureBoot', 1, 1, 1, 'uefi_capsule', 1763200000),
                 'HsiResultFallback': {'$v': ['u', 2]}}]

FWUPD = Spec('system', 'org.freedesktop.fwupd', {
    '/': {'org.freedesktop.fwupd': {
        'props': {'HostSecurityId': ('s', 'HSI:1! (v2.0.13)'), 'DaemonVersion': ('s', '2.0.13'),
                  'HostVendor': ('s', 'WinPoint test'), 'HostProduct': ('s', 'Harness PC')},
        'methods': {'GetHostSecurityAttrs': {'out': 'aa{sv}', 'value': [FWUPD_ATTRS]},
                    'GetHostSecurityEvents': {'in': 'u', 'out': 'aa{sv}', 'value': [FWUPD_EVENTS]}},
    }}})

# Bestanden's last location access: 16-11-2025 08:00 UTC (cc-location-panel.c shows it as "Last accessed")
LOCATION_STAMP = '1763280000'

PERMISSION_STORE = Spec('session', 'org.freedesktop.impl.portal.PermissionStore', {
    '/org/freedesktop/impl/portal/PermissionStore': {'org.freedesktop.impl.portal.PermissionStore': {
        # the real Lookup also returns a `v` (the entry's data), which the host does not read
        'methods': {'Lookup': {'in': 'ss', 'out': 'a{sas}', 'value': [{}], 'byArgs': {
                        "('location', 'location')": [{'org.gnome.Nautilus': ['EXACT', LOCATION_STAMP]}],
                        "('devices', 'camera')": [{'org.gnome.Nautilus': ['yes']}],
                        "('devices', 'microphone')": [{'org.gnome.Nautilus': ['no']}]}},
                    'SetPermission': {'in': 'sbssas', 'permissionStore': True}, 'DeletePermission': {'in': 'sss'}},
        'signals': {'Changed': 'ssbva{sas}'},
    }}})

SPECS: dict[str, "Spec"] = {'whoopsie': WHOOPSIE, 'fwupd': FWUPD, 'permission-store': PERMISSION_STORE}
