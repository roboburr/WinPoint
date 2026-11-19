//! Batch F (apps, notifications, privacy, background): the allow table of `gsettings.*`
//! (notes/claude/settings-parity.md §6.1). Only batch F edits this file. The look rows (`Owned`) are B0's rule:
//! the audit requires every key of owned.rs LOOK_KEYS to stay `Owned` here.
use super::Allow;
use super::Mode::{Mirrored, Owned, Read, Service, Write};

pub(super) const TABLE: &[Allow] = &[
    // Systeem › Meldingen (amendments 5, 6)
    Allow::new(
        "org.gnome.desktop.notifications",
        &["show-banners"],
        Mirrored("notifications.dnd"),
    ),
    Allow::new(
        "org.gnome.desktop.notifications",
        &["show-in-lock-screen"],
        Write,
    ),
    Allow::new(
        "org.gnome.desktop.notifications",
        &["application-children"],
        Read,
    ),
    Allow::at(
        "org.gnome.desktop.notifications.application",
        "/org/gnome/desktop/notifications/application/{appid}/",
        &["*"],
        Service("notifications.app.set"),
    ),
    // Privacy en beveiliging
    Allow::new(
        "org.gnome.desktop.privacy",
        &["remember-recent-files"],
        Mirrored("privacy.recentFiles"),
    ),
    Allow::new(
        "org.gnome.desktop.privacy",
        &[
            "recent-files-max-age",
            "disable-camera",
            "disable-microphone",
        ],
        Write,
    ),
    Allow::new("org.gnome.desktop.search-providers", &["*"], Write),
    Allow::new(
        "org.freedesktop.Tracker3.Miner.Files",
        &["index-recursive-directories", "index-single-directories"],
        Write,
    )
    .optional("tracker"),
    Allow::new("org.gnome.system.location", &["enabled"], Write),
    // GNOME Settings 50 has no control for the accuracy cap (gnome-a.json: "'exact', no UI"), so neither does
    // WinPoint: readable, never written (§6.1, a key a source names only to say it has no control)
    Allow::new("org.gnome.system.location", &["max-accuracy-level"], Read),
    // Persoonlijke instellingen: WinPoint's look (gnome_look::note; restored by `win11-app --gnome-look`)
    Allow::new(
        "org.gnome.desktop.interface",
        &["color-scheme"],
        Owned("theme.mode"),
    ),
    Allow::new(
        "org.gnome.desktop.interface",
        &["gtk-theme"],
        Owned("theme.gtkFamily"),
    ),
    Allow::new(
        "org.gnome.desktop.interface",
        &["icon-theme"],
        Owned("theme.mode"),
    ),
    Allow::new(
        "org.gnome.desktop.interface",
        &["accent-color"],
        Owned("theme.accent"),
    ),
    Allow::new(
        "org.gnome.desktop.wm.preferences",
        &["button-layout"],
        Owned("theme.captionButtons"),
    ),
    Allow::new("org.gnome.desktop.background", &["*"], Owned("wallpaper")),
];
