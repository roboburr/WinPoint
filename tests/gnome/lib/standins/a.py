"""Batch A's D-Bus stand-ins for its settings-parity scenarios (notes/claude/settings-parity.md §7): `Spec`s for
`lib.standins.start(h, spec, tag)`. Only batch A edits this file."""
from __future__ import annotations

from . import Spec


def _polkit(authorized: bool, challenge: bool) -> Spec:
    """polkit's Authority on the private system bus, answering every CheckAuthorization with (authorized,
    challenge, {}). Taalopties' keyboard write asks it, as Ubuntu's GNOME Settings does, whether
    org.gnome.controlcenter.set-keyboard is allowed without a prompt (flags 0, no interactive authorization:
    host/src/services/input/sources.rs allowed_without_prompt), and runs GNOME's helper through pkexec only then."""
    return Spec('system', 'org.freedesktop.PolicyKit1', {
        '/org/freedesktop/PolicyKit1/Authority': {'org.freedesktop.PolicyKit1.Authority': {
            'methods': {
                'CheckAuthorization': {'in': '(sa{sv})sa{ss}us', 'out': '(bba{ss})',
                                       'value': [[authorized, challenge, {}]]},
            },
            'props': {'BackendName': ('s', 'js'), 'BackendVersion': ('s', '126'), 'BackendFeatures': ('u', 1)},
        }},
    })


SPECS: dict[str, "Spec"] = {
    # Orca 50's remote controller (orca/dbus_service.py): Verteller's Instellingen voor Verteller openen calls
    # ShowPreferences on the running Orca (amendment 36), a plain call without interactive authorization.
    'orca': Spec('session', 'org.gnome.Orca.Service', {
        '/org/gnome/Orca/Service': {'org.gnome.Orca.Service': {
            'methods': {
                'ShowPreferences': {'in': '', 'out': 'b', 'value': [True]},
                'GetVersion': {'in': '', 'out': 's', 'value': ['50.3']},
            },
        }},
    }),
    # Ubuntu's rule for a local active sudo user (com.ubuntu.desktop.rules: controlcenter.set-keyboard YES): the
    # system keyboard follows, through pkexec /usr/libexec/gnome-control-center-set-debian-keyboard (the run's stub)
    'polkit': _polkit(True, False),
    # anyone else: polkit would ask (a challenge), so GNOME Settings keeps the session setting only, without a prompt
    'polkit-challenge': _polkit(False, True),
}
