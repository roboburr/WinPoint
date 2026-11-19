//! Batch D (display, sound, power, storage, multitasking, screen time): the allow table of `gsettings.*`
//! (notes/claude/settings-parity.md §6.1). Only batch D edits this file.
use super::Allow;
use super::Mode::{Mirrored, Read, Service, Write};

pub(super) const TABLE: &[Allow] = &[
    // Beeldscherm, Nachtlampje
    Allow::new(
        "org.gnome.settings-daemon.plugins.power",
        &[
            "ambient-enabled",
            "power-saver-profile-on-low-battery",
            "power-button-action",
        ],
        Write,
    ),
    // energy.rs stays the only writer of the idle and sleep keys (amendment 50)
    Allow::new(
        "org.gnome.settings-daemon.plugins.power",
        &[
            "idle-dim",
            "sleep-inactive-ac-type",
            "sleep-inactive-ac-timeout",
            "sleep-inactive-battery-type",
            "sleep-inactive-battery-timeout",
        ],
        Service("system.power.settings.set"),
    ),
    Allow::new(
        "org.gnome.desktop.session",
        &["idle-delay"],
        Service("system.power.settings.set"),
    ),
    Allow::new(
        "org.gnome.desktop.privacy",
        &[
            "privacy-screen",
            "remove-old-trash-files",
            "remove-old-temp-files",
            "old-files-age",
        ],
        Write,
    ),
    Allow::new(
        "org.gnome.settings-daemon.plugins.color",
        &[
            "night-light-enabled",
            "night-light-temperature",
            "night-light-schedule-automatic",
            "night-light-schedule-from",
            "night-light-schedule-to",
        ],
        Write,
    ),
    Allow::new(
        "org.gnome.settings-daemon.plugins.color",
        &["night-light-last-coordinates"],
        Read,
    ),
    Allow::new(
        "org.gnome.settings-daemon.peripherals.touchscreen",
        &["orientation-lock"],
        Write,
    ),
    Allow::new(
        "org.gnome.mutter.wayland",
        &["xwayland-scaling-factor"],
        Write,
    ),
    // Geluid (allow-volume-above-100-percent: one owner, also used by Toegankelijkheid › Audio)
    Allow::new(
        "org.gnome.desktop.sound",
        &["allow-volume-above-100-percent", "input-feedback-sounds"],
        Write,
    ),
    Allow::new(
        "org.gnome.desktop.sound",
        &["event-sounds", "theme-name"],
        Service("sound.alerts.set"),
    ),
    Allow::new("org.gnome.shell.ubuntu", &["startup-sound"], Write).optional("ubuntu"),
    // Schermtijd
    Allow::new("org.gnome.desktop.screen-time-limits", &["*"], Write),
    Allow::new(
        "org.gnome.desktop.break-reminders",
        &["selected-breaks"],
        Write,
    ),
    Allow::new("org.gnome.desktop.break-reminders.movement", &["*"], Write),
    Allow::new("org.gnome.desktop.break-reminders.eyesight", &["*"], Write),
    // Energie en batterij: the tray's battery percentage (settings_mirror.rs)
    Allow::new(
        "org.gnome.desktop.interface",
        &["show-battery-percentage"],
        Mirrored("taskbar.batteryPercent"),
    ),
    // Multitasking (amendments 11, 12)
    Allow::new(
        "org.gnome.desktop.interface",
        &["enable-hot-corners"],
        Write,
    ),
    Allow::new(
        "org.gnome.mutter",
        &[
            "edge-tiling",
            "dynamic-workspaces",
            "workspaces-only-on-primary",
        ],
        Write,
    ),
    Allow::new("org.gnome.mutter", &["experimental-features"], Read),
    Allow::new(
        "org.gnome.desktop.wm.preferences",
        &["num-workspaces"],
        Write,
    ),
    // Alt+Tab: settings_mirror.rs pairs App Switching with taskView.allDesktops; the window switcher is written along
    Allow::new(
        "org.gnome.shell.app-switcher",
        &["current-workspace-only"],
        Mirrored("taskView.allDesktops"),
    ),
    Allow::new(
        "org.gnome.shell.window-switcher",
        &["current-workspace-only"],
        Write,
    ),
];
