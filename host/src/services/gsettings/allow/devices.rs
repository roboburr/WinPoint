//! Batch B (Bluetooth, printers and colour): the allow table of `gsettings.*` (notes/claude/settings-parity.md §6.1).
//! Only batch B edits this file.
use super::Allow;
use super::Mode::Write;

pub(super) const TABLE: &[Allow] = &[
    // Bluetooth en apparaten › Automatisch afspelen: GNOME's Removable Media keys. automount is left out (no page
    // of the plan has a control for it) and so is automount-open: gnome-shell 50 and Nautilus no longer read it (a
    // drive without a content type always asks), and GNOME Settings never writes it
    Allow::new(
        "org.gnome.desktop.media-handling",
        &[
            "autorun-never",
            "autorun-x-content-start-app",
            "autorun-x-content-ignore",
            "autorun-x-content-open-folder",
        ],
        Write,
    ),
];
