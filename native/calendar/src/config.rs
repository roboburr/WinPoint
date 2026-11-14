//! The user's choices from ~/.config/win11-gnome/settings.json (XDG_CONFIG_HOME respected), the same file the
//! shell and the HTML apps read: theme.mode (dark / light, otherwise GNOME's colour scheme), theme.accent,
//! language ("nl" default, "en"), time.h24 and accessibility.textScale. Also the text helper `t(nl, en)` and
//! the data folder (~/.local/share/win11-gnome-calendar, XDG_DATA_HOME respected).
use gtk::{gio, glib};
use gtk::prelude::*;
use serde_json::Value;
use std::cell::Cell;
use std::path::PathBuf;

#[derive(Clone, PartialEq, Debug)]
pub struct Config {
    pub en: bool,
    pub dark: bool,
    pub accent: String,
    pub h24: bool,
    pub text_scale: f64,
    /// theme.transparency (default on) and accessibility.highContrast: the extension's blur (backdrop.rs).
    pub transparency: bool,
    pub high_contrast: bool,
}

impl Default for Config {
    fn default() -> Config {
        Config { en: false, dark: true, accent: "#0078d4".into(), h24: true, text_scale: 1.0, transparency: true, high_contrast: false }
    }
}

thread_local! {
    static EN: Cell<bool> = const { Cell::new(false) };
}

/// Whether the UI is in English (otherwise Dutch).
pub fn en() -> bool {
    EN.with(|e| e.get())
}

/// Dutch or English text by settings.language.
pub fn t(nl: &'static str, en_text: &'static str) -> &'static str {
    if en() { en_text } else { nl }
}

pub fn settings_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(glib::user_config_dir);
    base.join("win11-gnome").join("settings.json")
}

/// Until 13-11 the folder was win11-gnome/calendar: with the default prefix that is the install payload, which an
/// update swaps and an uninstall removes. The first call moves it here (scripts/install.py `rescue_calendar` does
/// the same before it touches the payload).
pub fn data_dir() -> PathBuf {
    static MOVED: std::sync::Once = std::sync::Once::new();
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(glib::user_data_dir);
    let dir = base.join("win11-gnome-calendar");
    MOVED.call_once(|| {
        let old = base.join("win11-gnome").join("calendar");
        if old.is_dir() && !old.is_symlink() && !dir.exists() {
            let _ = std::fs::rename(&old, &dir);
        }
    });
    dir
}

/// Read settings.json now (missing or broken: the defaults) and make its language current.
pub fn load() -> Config {
    let s: Value = std::fs::read(settings_path())
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(Value::Null);
    let en = s["language"].as_str() == Some("en");
    let dark = match s["theme"]["mode"].as_str() {
        Some("light") => false,
        Some("dark") => true,
        _ => gnome_prefers_dark(),
    };
    let accent = s["theme"]["accent"].as_str().unwrap_or("#0078d4").to_owned();
    let h24 = s["time"]["h24"].as_bool().unwrap_or(!en);
    let ts = s["accessibility"]["textScale"].as_f64().filter(|v| v.is_finite() && *v > 0.0).unwrap_or(100.0);
    let transparency = s["theme"]["transparency"] != false;
    let high_contrast = s["accessibility"]["highContrast"] == true;
    EN.with(|e| e.set(en));
    Config { en, dark, accent, h24, text_scale: ts.clamp(100.0, 225.0) / 100.0, transparency, high_contrast }
}

/// GNOME's own light/dark choice (org.gnome.desktop.interface color-scheme), dark when unknown.
fn gnome_prefers_dark() -> bool {
    let Some(source) = gio::SettingsSchemaSource::default() else { return true };
    let Some(schema) = source.lookup("org.gnome.desktop.interface", true) else { return true };
    if !schema.has_key("color-scheme") {
        return true;
    }
    let settings = gio::Settings::new("org.gnome.desktop.interface");
    settings.string("color-scheme").as_str() == "prefer-dark"
}
