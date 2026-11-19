//! Batch A (input and accessibility): the allow table of `gsettings.*` (notes/claude/settings-parity.md §6.1). Only
//! batch A edits this file.
use super::Allow;
use super::Mode::{Mirrored, Read, Service, Write};

pub(super) const TABLE: &[Allow] = &[
    // Bluetooth en apparaten › Muis, Touchpad (amendment 10: no disable-while-typing-timeout, mutter 50 ignores it)
    Allow::new(
        "org.gnome.desktop.peripherals.mouse",
        &[
            "left-handed",
            "speed",
            "accel-profile",
            "natural-scroll",
            "middle-click-emulation",
            "double-click",
        ],
        Write,
    ),
    Allow::new(
        "org.gnome.desktop.peripherals.pointingstick",
        &["speed", "accel-profile", "scroll-method"],
        Write,
    ),
    Allow::new(
        "org.gnome.desktop.peripherals.touchpad",
        &[
            "send-events",
            "speed",
            "accel-profile",
            "disable-while-typing",
            "tap-to-click",
            "tap-button-map",
            "tap-and-drag",
            "tap-and-drag-lock",
            "click-method",
            "two-finger-scrolling-enabled",
            "edge-scrolling-enabled",
            "natural-scroll",
            "left-handed",
            "middle-click-emulation",
        ],
        Write,
    ),
    // Pen en tablet (relocatable, libmutter-18's paths; amendments 8 and 9)
    Allow::at(
        "org.gnome.desktop.peripherals.tablet",
        "/org/gnome/desktop/peripherals/tablets/{vidpid}/",
        &["left-handed", "mapping", "output", "keep-aspect", "area"],
        Write,
    ),
    Allow::at(
        "org.gnome.desktop.peripherals.tablet.stylus",
        "/org/gnome/desktop/peripherals/stylus/{serial}/",
        &["*"],
        Write,
    ),
    Allow::at(
        "org.gnome.desktop.peripherals.tablet.pad-button",
        "/org/gnome/desktop/peripherals/tablets/{vidpid}/{padctl}/",
        &["action", "keybinding"],
        Write,
    ),
    Allow::at(
        "org.gnome.desktop.peripherals.touchscreen",
        "/org/gnome/desktop/peripherals/touchscreens/{vidpid}/",
        &["output"],
        Write,
    ),
    // Tijd en taal › Typen, Taalopties, Geavanceerde toetsenbordinstellingen
    Allow::new(
        "org.gnome.desktop.peripherals.keyboard",
        &["repeat", "delay", "repeat-interval"],
        Write,
    ),
    Allow::new(
        "org.gnome.desktop.input-sources",
        &["sources"],
        Service("input.sources.set"),
    ),
    Allow::new(
        "org.gnome.desktop.input-sources",
        &["xkb-options"],
        Service("input.options.set"),
    ),
    // show-all-sources is left out: GNOME Settings has no control for it (gnome-b.json)
    Allow::new("org.gnome.desktop.input-sources", &["per-window"], Write),
    Allow::new(
        "org.gnome.desktop.input-sources",
        &["current", "mru-sources", "xkb-model"],
        Read,
    ),
    // amendment 58: GNOME's own key, a separate row marked GNOME-only (WinPoint's is start.windowsKey)
    Allow::new("org.gnome.mutter", &["overlay-key"], Write),
    // Sneltoetsen: written by input.shortcuts.* (conflict detection, the held accelerators of amendment 13)
    Allow::new(
        "org.gnome.desktop.wm.keybindings",
        &["*"],
        Service("input.shortcuts.set"),
    ),
    Allow::new(
        "org.gnome.mutter.keybindings",
        &["*"],
        Service("input.shortcuts.set"),
    ),
    Allow::new(
        "org.gnome.mutter.wayland.keybindings",
        &["*"],
        Service("input.shortcuts.set"),
    ),
    Allow::new(
        "org.gnome.shell.keybindings",
        &["*"],
        Service("input.shortcuts.set"),
    ),
    Allow::new(
        "org.gnome.settings-daemon.plugins.media-keys",
        &["*"],
        Service("input.shortcuts.set"),
    ),
    Allow::at(
        "org.gnome.settings-daemon.plugins.media-keys.custom-keybinding",
        "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/{custom}/",
        &["name", "command", "binding", "enable-in-lockscreen"],
        Service("input.shortcuts.custom.update"),
    ),
    Allow::new(
        "org.gnome.shell.extensions.win11-gnome",
        &["task-manager", "task-view"],
        Service("input.shortcuts.set"),
    )
    .optional("winpoint-extension"),
    Allow::new(
        "org.gnome.shell.extensions.win11-gnome",
        &["task-view-restore", "logout-restore"],
        Read,
    )
    .optional("winpoint-extension"),
    // Toegankelijkheid (always-show-text-caret, toolkit-accessibility and audible-bell are left out: no page of the
    // plan has a control for them; toolkit-accessibility is gsd's a11y-settings plugin's)
    Allow::new(
        "org.gnome.desktop.a11y",
        &["always-show-universal-access-status"],
        Write,
    ),
    Allow::new(
        "org.gnome.desktop.a11y.applications",
        &[
            "screen-keyboard-enabled",
            "screen-magnifier-enabled",
            "screen-reader-enabled",
        ],
        Write,
    ),
    Allow::new(
        "org.gnome.desktop.a11y.interface",
        &["show-status-shapes"],
        Write,
    ),
    // the rendering pairs of batch A (host/src/settings_mirror/render.rs, amendment 49)
    Allow::new(
        "org.gnome.desktop.a11y.interface",
        &["high-contrast"],
        Mirrored("accessibility.highContrast"),
    ),
    Allow::new(
        "org.gnome.desktop.a11y.interface",
        &["reduced-motion"],
        Mirrored("accessibility.reduceMotion"),
    ),
    Allow::new(
        "org.gnome.desktop.interface",
        &["enable-animations"],
        Mirrored("accessibility.reduceMotion"),
    ),
    Allow::new(
        "org.gnome.desktop.interface",
        &["text-scaling-factor"],
        Mirrored("accessibility.textScale"),
    ),
    // gtk-enable-primary-paste: Muis › "Plakken met de middelste muisknop" (windows.json's WinPoint addition)
    Allow::new(
        "org.gnome.desktop.interface",
        &[
            "locate-pointer",
            "gtk-enable-primary-paste",
            "cursor-size",
            "cursor-blink",
            "cursor-blink-time",
            "overlay-scrolling",
        ],
        Write,
    ),
    Allow::new(
        "org.gnome.desktop.a11y.magnifier",
        &[
            "mag-factor",
            "screen-position",
            "lens-mode",
            "scroll-at-edges",
            "mouse-tracking",
            "focus-tracking",
            "caret-tracking",
            "invert-lightness",
            "brightness-red",
            "brightness-green",
            "brightness-blue",
            "contrast-red",
            "contrast-green",
            "contrast-blue",
            "color-saturation",
            "show-cross-hairs",
            "cross-hairs-clip",
            "cross-hairs-thickness",
            "cross-hairs-length",
            "cross-hairs-color",
            "cross-hairs-opacity",
        ],
        Write,
    ),
    Allow::new("org.gnome.desktop.a11y.keyboard", &["*"], Write),
    Allow::new("org.gnome.desktop.a11y.mouse", &["*"], Write),
    Allow::new(
        "org.gnome.desktop.wm.preferences",
        &["visual-bell", "visual-bell-type", "focus-mode"],
        Write,
    ),
];
