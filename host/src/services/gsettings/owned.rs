//! The GNOME values WinPoint itself holds (notes/claude/settings-parity.md §6.1, amendment 13).
//!
//! * WinPoint's look (`gnome_look::note`: color-scheme, gtk-theme, icon-theme, accent-color, button-layout and every
//!   org.gnome.desktop.background key) is `Owned` in the allow tables; [`LOOK_KEYS`] is the list the audit checks.
//! * The extension holds two accelerators (extension/shortcutBridge.js): `<Alt>Tab` (WinPoint's Task View; it strips
//!   it from every org.gnome.desktop.wm.keybindings key and keeps the user's value in `task-view-restore`) and
//!   Ctrl+Alt+Delete (Task Manager; it strips it from media-keys `logout` and keeps `logout-restore`). The check is
//!   per accelerator: a keybinding write without them is fine, one with them is refused ([`held_accel`]).
use gio::prelude::*;
use std::path::{Path, PathBuf};

/// The extension's schema and UUID (extension/schemas, metadata.json).
pub(crate) const EXTENSION_SCHEMA: &str = "org.gnome.shell.extensions.win11-gnome";
pub(crate) const UUID: &str = "win11-gnome@win11shell.local";
const WM_KEYBINDINGS: &str = "org.gnome.desktop.wm.keybindings";
const MEDIA_KEYS: &str = "org.gnome.settings-daemon.plugins.media-keys";

/// Every GNOME key WinPoint's look writes through `gnome_look::note` (settings_effects.rs `theme_targets` and
/// `sync_button_layout`, the mirror's accent pair, wallpaper.rs): (schema, key, the WinPoint setting); `*` = every
/// key of the schema.
#[cfg_attr(not(test), allow(dead_code))] // the allowlist audit
pub(crate) const LOOK_KEYS: &[(&str, &str, &str)] = &[
    ("org.gnome.desktop.interface", "color-scheme", "theme.mode"),
    (
        "org.gnome.desktop.interface",
        "gtk-theme",
        "theme.gtkFamily",
    ),
    ("org.gnome.desktop.interface", "icon-theme", "theme.mode"),
    (
        "org.gnome.desktop.interface",
        "accent-color",
        "theme.accent",
    ),
    (
        "org.gnome.desktop.wm.preferences",
        "button-layout",
        "theme.captionButtons",
    ),
    ("org.gnome.desktop.background", "*", "wallpaper"),
];

/// The extension's current bindings (empty when its schema is not installed: then nothing is held).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Held {
    pub task_view: Vec<String>,
    pub task_manager: Vec<String>,
}

fn parts(accel: &str) -> Vec<String> {
    let lower = accel.to_ascii_lowercase();
    let mut out = Vec::new();
    let mut rest = lower.as_str();
    while let Some(start) = rest.find('<') {
        if start > 0 {
            out.push(rest[..start].to_owned());
        }
        let Some(end) = rest[start..].find('>') else {
            break;
        };
        out.push(rest[start..start + end + 1].to_owned());
        rest = &rest[start + end + 1..];
    }
    if !rest.is_empty() {
        out.push(rest.to_owned());
    }
    out
}

/// `<Alt>Tab` or `<Mod1>Tab`, any case (shortcutBridge.js `_takeTaskView`).
pub(crate) fn is_alt_tab(accel: &str) -> bool {
    matches!(parts(accel).as_slice(), [m, k] if (m == "<alt>" || m == "<mod1>") && k == "tab")
}

/// Ctrl+Alt+Delete in any spelling (shortcutBridge.js `isCtrlAltDelete`).
pub(crate) fn is_ctrl_alt_delete(accel: &str) -> bool {
    let mut p = parts(accel);
    if p.pop().as_deref() != Some("delete") || p.len() != 2 {
        return false;
    }
    p.iter().any(|m| m == "<alt>")
        && p.iter()
            .any(|m| matches!(m.as_str(), "<control>" | "<ctrl>" | "<primary>"))
}

/// The accelerator of `value` that the extension holds for (schema, key), with the extension key that holds it
/// ("task-view" or "task-manager"); None when the write takes nothing from WinPoint.
pub(crate) fn held_accel(
    schema: &str,
    key: &str,
    value: &[String],
    held: &Held,
) -> Option<(String, &'static str)> {
    if schema == WM_KEYBINDINGS && held.task_view.iter().any(|a| is_alt_tab(a)) {
        return value
            .iter()
            .find(|a| is_alt_tab(a))
            .map(|a| (a.clone(), "task-view"));
    }
    if schema == MEDIA_KEYS
        && key == "logout"
        && held.task_manager.iter().any(|a| is_ctrl_alt_delete(a))
    {
        return value
            .iter()
            .find(|a| is_ctrl_alt_delete(a))
            .map(|a| (a.clone(), "task-manager"));
    }
    None
}

/// Where the extension's compiled schema may be: the user's install (scripts/install.py), a system install.
pub(crate) fn extension_schema_dirs() -> Vec<PathBuf> {
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| glib::home_dir().join(".local/share"));
    vec![
        data.join(format!("gnome-shell/extensions/{UUID}/schemas")),
        PathBuf::from(format!("/usr/share/gnome-shell/extensions/{UUID}/schemas")),
    ]
}

/// The extension's schema source: the first of `dirs` with a gschemas.compiled that has it.
pub(crate) fn extension_source(dirs: &[PathBuf]) -> Option<gio::SettingsSchemaSource> {
    dirs.iter()
        .filter(|dir| Path::new(dir).join("gschemas.compiled").is_file())
        .find_map(|dir| {
            let source = gio::SettingsSchemaSource::from_directory(dir, None, false).ok()?;
            source.lookup(EXTENSION_SCHEMA, false).map(|_| source)
        })
}

/// The extension's current bindings, from its schema in `source` (or the default source) on `backend`.
pub(crate) fn held(
    source: Option<&gio::SettingsSchemaSource>,
    backend: Option<&gio::SettingsBackend>,
) -> Held {
    let schema = source
        .and_then(|s| s.lookup(EXTENSION_SCHEMA, false))
        .or_else(|| gio::SettingsSchemaSource::default()?.lookup(EXTENSION_SCHEMA, true));
    let Some(schema) = schema else {
        return Held::default();
    };
    let settings = gio::Settings::new_full(&schema, backend, None);
    let strv = |key: &str| -> Vec<String> {
        if schema.has_key(key) && schema.key(key).value_type().as_str() == "as" {
            settings.strv(key).iter().map(|s| s.to_string()).collect()
        } else {
            Vec::new()
        }
    };
    Held {
        task_view: strv("task-view"),
        task_manager: strv("task-manager"),
    }
}
