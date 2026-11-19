"""Batch B0's D-Bus stand-ins for its settings-parity scenarios (notes/claude/settings-parity.md §7): `Spec`s for
`lib.standins.start(h, spec, tag)`. Only batch B0 edits this file."""
from __future__ import annotations

from . import Spec  # noqa: F401

SPECS: dict[str, "Spec"] = {}
