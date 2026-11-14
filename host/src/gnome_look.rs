//! WinPoint as its own login session (scripts/install-session.py): the GNOME settings WinPoint writes (GTK and icon
//! theme, colour scheme, window buttons, wallpaper) are GNOME's own again in a GNOME or Ubuntu session.
//!
//! Every write of a GNOME key by WinPoint goes through [`note`] first. It keeps, per key, the value GNOME had
//! ("gnome", GVariant text, or null for the key's default) next to WinPoint's last value ("winpoint") in
//! `$XDG_CONFIG_HOME/win11-gnome/gnome-look.json`. A later write keeps "gnome" while the key still holds WinPoint's last
//! value, and takes the current value as the new "gnome" when someone changed it outside WinPoint (in a GNOME session).
//! A key that already held WinPoint's value before this record existed (an install from before the session) gets
//! null: back to GNOME's default.
//!
//! [`restore`] is `win11-app --gnome-look`, run by the autostart entry io.github.win11shell.GnomeLook.desktop in every
//! session but WinPoint's (NotShowIn=WinPoint): each key that still holds WinPoint's value goes back to GNOME's; the
//! GTK CSS blocks (Windows 11 caption buttons, Fluent theme) leave ~/.config/gtk-3.0 and gtk-4.0; the keybindings the
//! extension took (logout, Alt+Tab for Task View) go back as scripts/uninstall.py restores them. The WinPoint session
//! applies its own values again at its start.
use gio::prelude::*;
use serde_json::{Value, json};
use std::path::PathBuf;

const UUID: &str = "win11-gnome@win11shell.local";
const EXTENSION_SCHEMA: &str = "org.gnome.shell.extensions.win11-gnome";
/// One of the four files of scripts/install-session.py: the WinPoint login session is installed.
const SESSION: &str = "/usr/share/wayland-sessions/winpoint.desktop";

fn config_home() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| glib::home_dir().join(".config"))
}

fn record_path() -> PathBuf {
    config_home().join("win11-gnome/gnome-look.json")
}

fn load() -> Value {
    std::fs::read_to_string(record_path())
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .filter(|v| v["keys"].is_object())
        .unwrap_or_else(|| json!({"version":1,"keys":{}}))
}

fn save(record: &Value) {
    let path = record_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let temporary = path.with_extension(format!("json.{}", std::process::id()));
    if std::fs::write(&temporary, format!("{record:#}\n")).is_ok() {
        let _ = std::fs::rename(&temporary, &path);
    }
}

/// Before WinPoint writes `target` to `key` of `settings`: keep what GNOME had (module docs).
pub(crate) fn note(settings: &gio::Settings, key: &str, target: &glib::Variant) {
    let Some(schema) = settings.schema_id() else {
        return;
    };
    let id = format!("{schema} {key}");
    let current = settings.value(key);
    let current_text = current.print(true).to_string();
    let target_text = target.print(true).to_string();
    let mut record = load();
    let previous = record["keys"][&id].clone();
    let gnome = if previous.is_object() {
        if previous["winpoint"].as_str() == Some(current_text.as_str()) {
            previous["gnome"].clone()
        } else {
            json!(current_text)
        }
    } else if current_text != target_text {
        json!(current_text)
    } else {
        Value::Null
    };
    if previous["gnome"] == gnome && previous["winpoint"].as_str() == Some(target_text.as_str()) {
        return;
    }
    record["keys"][&id] = json!({"gnome": gnome, "winpoint": target_text});
    save(&record);
}

fn schema(id: &str) -> Option<gio::Settings> {
    gio::SettingsSchemaSource::default()?
        .lookup(id, true)
        .map(|_| gio::Settings::new(id))
}

/// `win11-app --gnome-look`: GNOME's own look in a session that is not WinPoint's (module docs). Exit code 0, or 1
/// when something could not be put back (the rest still is).
pub fn restore() -> i32 {
    let mut ok = true;
    // The WinPoint session turns the extension on through its shell mode; GNOME and Ubuntu do not run it (an install
    // from before the session had it in enabled-extensions for every session).
    if std::path::Path::new(SESSION).is_file()
        && let Some(shell) = schema("org.gnome.shell")
    {
        let enabled: Vec<String> = shell
            .strv("enabled-extensions")
            .iter()
            .map(|s| s.to_string())
            .collect();
        if enabled.iter().any(|uuid| uuid == UUID) {
            let rest: Vec<&str> = enabled
                .iter()
                .map(String::as_str)
                .filter(|uuid| *uuid != UUID)
                .collect();
            ok &= shell
                .set_strv("enabled-extensions", rest.as_slice())
                .is_ok();
        }
    }
    let record = load();
    if let Some(keys) = record["keys"].as_object() {
        for (id, entry) in keys {
            let Some((schema_id, key)) = id.split_once(' ') else {
                continue;
            };
            let Some(settings) = schema(schema_id) else {
                continue;
            };
            if !settings
                .settings_schema()
                .is_some_and(|schema| schema.has_key(key))
            {
                continue;
            }
            if entry["winpoint"].as_str() != Some(settings.value(key).print(true).as_str()) {
                continue; // changed since WinPoint wrote it: the user's value stays
            }
            match entry["gnome"].as_str() {
                None => settings.reset(key),
                Some(text) => match glib::Variant::parse(None, text) {
                    Ok(value) => ok &= settings.set_value(key, &value).is_ok(),
                    Err(_) => ok = false,
                },
            }
        }
    }
    let config = config_home();
    let captions =
        crate::settings_runtime::effects::apply_captions(false, &PathBuf::new(), &config);
    ok &= captions["ok"] == true;
    ok &= crate::settings_runtime::effects::gtk4_remove(&config.join("gtk-4.0/gtk.css")).is_ok();
    ok &= restore_keybindings();
    gio::Settings::sync();
    if ok { 0 } else { 1 }
}

/// The extension's own record of the keybindings it took (logout, Task View): as scripts/uninstall.py, only values
/// still equal to what the extension set go back.
fn restore_keybindings() -> bool {
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| glib::home_dir().join(".local/share"));
    let dir = data.join(format!("gnome-shell/extensions/{UUID}/schemas"));
    if !dir.join("gschemas.compiled").is_file() {
        return true;
    }
    let Ok(source) = gio::SettingsSchemaSource::from_directory(
        &dir,
        gio::SettingsSchemaSource::default().as_ref(),
        false,
    ) else {
        return false;
    };
    let Some(schema) = source.lookup(EXTENSION_SCHEMA, false) else {
        return true;
    };
    let own = gio::Settings::new_full(&schema, gio::SettingsBackend::NONE, None);
    let list = |value: &Value| -> Option<Vec<String>> {
        let items = value.as_array()?;
        (items.len() <= 100)
            .then(|| {
                items
                    .iter()
                    .map(|v| v.as_str().map(str::to_owned))
                    .collect()
            })
            .flatten()
    };
    let put_back = |target: &gio::Settings, key: &str, entry: &Value| {
        let (Some(owned), user) = (list(&entry["owned"]), &entry["user"]) else {
            return;
        };
        if target
            .strv(key)
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
            != owned
        {
            return;
        }
        if user.is_null() {
            target.reset(key);
        } else if let Some(user) = list(user) {
            let refs: Vec<&str> = user.iter().map(String::as_str).collect();
            let _ = target.set_strv(key, refs.as_slice());
        }
    };
    if schema.has_key("logout-restore")
        && let Ok(saved) = serde_json::from_str::<Value>(&own.string("logout-restore"))
        && let Some(media) = self::schema("org.gnome.settings-daemon.plugins.media-keys")
    {
        put_back(&media, "logout", &saved);
        own.reset("logout-restore");
    }
    if schema.has_key("task-view-restore")
        && let Ok(saved) = serde_json::from_str::<Value>(&own.string("task-view-restore"))
        && let Some(entries) = saved.as_object()
        && let Some(wm) = self::schema("org.gnome.desktop.wm.keybindings")
    {
        let keys = wm
            .settings_schema()
            .map(|s| s.list_keys())
            .unwrap_or_default();
        for (key, entry) in entries {
            if keys.iter().any(|k| k.as_str() == key) {
                put_back(&wm, key, entry);
            }
        }
        own.reset("task-view-restore");
    }
    true
}
