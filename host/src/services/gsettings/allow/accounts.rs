//! Batch E (accounts, time and region, About): the allow table of `gsettings.*` (notes/claude/settings-parity.md
//! §6.1). Only batch E edits this file.
use super::Allow;
use super::Mode::{Mirrored, Read, Write};

pub(super) const TABLE: &[Allow] = &[
    // Accounts › Aanmeldingsopties: "Als je weg bent geweest, wanneer moet WinPoint vereisen dat je je opnieuw aanmeldt?"
    // (lock-enabled + lock-delay, amendment 23) and "Ook vergrendelen wanneer de pc in slaapstand gaat"
    Allow::new(
        "org.gnome.desktop.screensaver",
        &["lock-enabled", "lock-delay", "ubuntu-lock-on-suspend"],
        Write,
    ),
    // Nieuwe USB-apparaten blokkeren (shown only with usbguard, as GNOME); the level has no control in GNOME either
    Allow::new("org.gnome.desktop.privacy", &["usb-protection"], Write),
    Allow::new("org.gnome.desktop.privacy", &["usb-protection-level"], Read),
    // disable-lock-screen and user-administration-disabled make the Accounts pages' controls insensitive, as in GNOME
    Allow::new("org.gnome.desktop.lockdown", &["*"], Read),
    // Tijd en taal › Datum en tijd, Regionale notatie (the tray clock pairs of settings_mirror.rs)
    Allow::new("org.gnome.desktop.datetime", &["automatic-timezone"], Write),
    Allow::new(
        "org.gnome.desktop.interface",
        &["clock-format"],
        Mirrored("time.h24"),
    ),
    Allow::new(
        "org.gnome.desktop.interface",
        &["clock-show-seconds"],
        Mirrored("time.showSeconds"),
    ),
    Allow::new(
        "org.gnome.desktop.interface",
        &["clock-show-date"],
        Mirrored("time.showDate"),
    ),
    Allow::new(
        "org.gnome.desktop.interface",
        &["clock-show-weekday"],
        Mirrored("time.showWeekday"),
    ),
    Allow::new(
        "org.gnome.desktop.calendar",
        &["show-weekdate"],
        Mirrored("time.weekNumbers"),
    ),
    Allow::new(
        "org.gnome.desktop.calendar",
        &["week-start-day"],
        Mirrored("time.firstDayOfWeek"),
    ),
    // GNOME writes the time format to the file choosers' own keys as well (cc-datetime-panel.c
    // change_clock_settings): Regionale notatie › Korte tijd does the same
    Allow::new("org.gtk.Settings.FileChooser", &["clock-format"], Write),
    Allow::new(
        "org.gtk.gtk4.Settings.FileChooser",
        &["clock-format"],
        Write,
    ),
    // Taal en regio › Regionale notatie (with users.setFormats, as GNOME's Formats dialog)
    Allow::new("org.gnome.system.locale", &["region"], Write),
];
