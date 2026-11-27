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
    let mut record = load();
    if note_in(&mut record, settings, key, target) {
        save(&record);
    }
}

/// [`note`] on a record in memory; whether it changed.
pub(crate) fn note_in(
    record: &mut Value,
    settings: &gio::Settings,
    key: &str,
    target: &glib::Variant,
) -> bool {
    let Some(schema) = settings.schema_id() else {
        return false;
    };
    let id = format!("{schema} {key}");
    let current = settings.value(key);
    let current_text = current.print(true).to_string();
    let target_text = target.print(true).to_string();
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
        return false;
    }
    record["keys"][&id] = json!({"gnome": gnome, "winpoint": target_text});
    true
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
    ok &= restore_keys(&load(), schema);
    let config = config_home();
    let captions =
        crate::settings_runtime::effects::apply_captions(false, &PathBuf::new(), &config);
    ok &= captions["ok"] == true;
    ok &= crate::settings_runtime::effects::gtk4_remove(&config.join("gtk-4.0/gtk.css")).is_ok();
    ok &= restore_keybindings();
    gio::Settings::sync();
    if ok { 0 } else { 1 }
}

/// Each recorded key that still holds WinPoint's value goes back to GNOME's (`open` gives a schema's settings);
/// false when a value could not be written.
pub(crate) fn restore_keys(record: &Value, open: impl Fn(&str) -> Option<gio::Settings>) -> bool {
    let mut ok = true;
    if let Some(keys) = record["keys"].as_object() {
        for (id, entry) in keys {
            let Some((schema_id, key)) = id.split_once(' ') else {
                continue;
            };
            let Some(settings) = open(schema_id) else {
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
    ok
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

// Ubuntu's Tiling Assistant disables these native keys. The WinPoint mode does not
// enable that extension: repair only its complete disabled fingerprint, at panel start.
fn snap_context(kind: &str, dry_run: bool, session: &str, desktop: &str) -> bool {
    kind == "panel"
        && !dry_run
        && session == "gnome"
        && desktop
            .split(':')
            .any(|part| part.eq_ignore_ascii_case("winpoint"))
}

fn snap_fingerprint(edge: bool, left: &[String], right: &[String], ta_inactive: bool) -> bool {
    ta_inactive && !edge && left.is_empty() && right.is_empty()
}

fn snap_profile(mutter: &gio::Settings, bindings: &gio::Settings) -> bool {
    if ![
        (mutter, "edge-tiling"),
        (bindings, "toggle-tiled-left"),
        (bindings, "toggle-tiled-right"),
    ]
    .iter()
    .all(|(s, key)| {
        s.settings_schema()
            .is_some_and(|schema| schema.has_key(key))
            && s.is_writable(key)
    }) {
        return false;
    }
    let left: Vec<_> = bindings
        .strv("toggle-tiled-left")
        .iter()
        .map(|s| s.to_string())
        .collect();
    let right: Vec<_> = bindings
        .strv("toggle-tiled-right")
        .iter()
        .map(|s| s.to_string())
        .collect();
    snap_fingerprint(mutter.boolean("edge-tiling"), &left, &right, true)
}

fn checked_note(settings: &gio::Settings, key: &str, target: &glib::Variant) -> bool {
    let mut expected = load();
    if note_in(&mut expected, settings, key, target) {
        save(&expected);
    }
    load() == expected
}

fn ta_info_inactive(info: &glib::Variant) -> bool {
    if info.type_().as_str() != "a{sv}" {
        return false;
    }
    if info.n_children() == 0 {
        return true;
    }
    let Some(state) = glib::VariantDict::new(Some(info)).lookup_value("state", None) else {
        return false;
    };
    let state = state
        .get::<f64>()
        .or_else(|| state.get::<u32>().map(f64::from))
        .or_else(|| state.get::<i32>().map(f64::from));
    state == Some(2.0) || state == Some(6.0) // INACTIVE or INITIALIZED: installed, never enabled.
}

fn ta_inactive() -> bool {
    let Ok(bus) = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE) else {
        return false;
    };
    let started = std::time::Instant::now();
    for uuid in [
        "tiling-assistant@ubuntu.com",
        "tiling-assistant@leleat-on-github",
    ] {
        let Some(remaining) = std::time::Duration::from_millis(500).checked_sub(started.elapsed())
        else {
            return false;
        };
        let Ok(reply) = bus.call_sync(
            Some("org.gnome.Shell"),
            "/org/gnome/Shell",
            "org.gnome.Shell.Extensions",
            "GetExtensionInfo",
            Some(&(uuid,).to_variant()),
            Some(glib::VariantTy::new("(a{sv})").unwrap()),
            gio::DBusCallFlags::NO_AUTO_START,
            remaining.as_millis().max(1) as i32,
            gio::Cancellable::NONE,
        ) else {
            return false;
        };
        if !ta_info_inactive(&reply.child_value(0)) {
            return false;
        }
    }
    true
}

fn repair_snap(
    mutter: &gio::Settings,
    bindings: &gio::Settings,
    ta_inactive: bool,
    mut remember: impl FnMut(&gio::Settings, &str, &glib::Variant) -> bool,
    mut write: impl FnMut(&gio::Settings, &str, &glib::Variant) -> bool,
) -> bool {
    let updates = [
        (mutter, "edge-tiling", true.to_variant()),
        (
            bindings,
            "toggle-tiled-left",
            vec!["<Super>Left"].to_variant(),
        ),
        (
            bindings,
            "toggle-tiled-right",
            vec!["<Super>Right"].to_variant(),
        ),
    ];
    if !snap_profile(mutter, bindings) || !ta_inactive {
        return false;
    }
    let before: Vec<_> = updates
        .iter()
        .map(|(s, key, _)| s.user_value(key))
        .collect();
    for (s, key, value) in &updates {
        if !remember(s, key, value) {
            return false;
        }
    }
    for (index, (s, key, value)) in updates.iter().enumerate() {
        if !write(s, key, value) {
            for ((previous, name, owned), user) in updates[..index].iter().zip(&before).rev() {
                if previous.value(name) != *owned {
                    continue;
                }
                if let Some(user) = user {
                    let _ = previous.set_value(name, user);
                } else {
                    previous.reset(name);
                }
            }
            return false;
        }
    }
    true
}

pub(crate) fn repair_native_snap_at_start(kind: &str, dry_run: bool) {
    if !snap_context(
        kind,
        dry_run,
        crate::services::caps::session(),
        &std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default(),
    ) {
        return;
    }
    let (Some(mutter), Some(bindings)) = (
        schema("org.gnome.mutter"),
        schema("org.gnome.mutter.keybindings"),
    ) else {
        return;
    };
    if !snap_profile(&mutter, &bindings) {
        return;
    }
    if repair_snap(
        &mutter,
        &bindings,
        ta_inactive(),
        checked_note,
        |s, key, value| s.set_value(key, value).is_ok(),
    ) {
        gio::Settings::sync();
    }
}

#[cfg(test)]
mod snap_tests {
    use super::*;
    fn memory() -> (gio::Settings, gio::Settings) {
        let source = gio::SettingsSchemaSource::default().unwrap();
        let backend = gio::memory_settings_backend_new();
        let settings =
            |id| gio::Settings::new_full(&source.lookup(id, true).unwrap(), Some(&backend), None);
        let mutter = settings("org.gnome.mutter");
        let bindings = settings("org.gnome.mutter.keybindings");
        mutter.set_boolean("edge-tiling", false).unwrap();
        for key in ["toggle-tiled-left", "toggle-tiled-right"] {
            bindings.set_strv(key, &[] as &[&str]).unwrap();
        }
        (mutter, bindings)
    }
    #[test]
    fn snap_guard_and_complete_fingerprint_only() {
        assert!(snap_context("panel", false, "gnome", "WinPoint:GNOME"));
        for args in [
            ("settings", false, "gnome", "WinPoint"),
            ("panel", true, "gnome", "WinPoint"),
            ("panel", false, "hyprland", "WinPoint"),
            ("panel", false, "gnome", "ubuntu:GNOME"),
        ] {
            assert!(!snap_context(args.0, args.1, args.2, args.3));
        }
        assert!(snap_fingerprint(false, &[], &[], true));
        assert!(!snap_fingerprint(false, &[], &[], false));
        assert!(!snap_fingerprint(true, &[], &[], true));
        assert!(!snap_fingerprint(false, &["<Alt>Left".into()], &[], true));
        assert!(!snap_fingerprint(
            false,
            &[],
            &["<Super>Right".into()],
            true
        ));
    }
    #[test]
    fn ta_requires_absent_inactive_or_never_started() {
        let empty = std::collections::HashMap::<String, glib::Variant>::new().to_variant();
        assert!(ta_info_inactive(&empty));
        for state in [1.0, 2.0, 3.0, 6.0, 7.0, 8.0, 9.0, 99.0, 2.1, f64::NAN] {
            let info = std::collections::HashMap::from([("state".to_string(), state.to_variant())])
                .to_variant();
            assert_eq!(ta_info_inactive(&info), state == 2.0 || state == 6.0);
        }
        assert!(!ta_info_inactive(&"unknown".to_variant()));
    }
    #[test]
    fn snap_repair_records_and_restores_all_three_values() {
        let (mutter, bindings) = memory();
        let mut record = json!({"version":1,"keys":{}});
        assert!(repair_snap(
            &mutter,
            &bindings,
            true,
            |s, k, v| {
                note_in(&mut record, s, k, v);
                true
            },
            |s, k, v| s.set_value(k, v).is_ok()
        ));
        assert!(mutter.boolean("edge-tiling"));
        assert_eq!(
            bindings.strv("toggle-tiled-left").as_slice(),
            ["<Super>Left"]
        );
        assert_eq!(
            bindings.strv("toggle-tiled-right").as_slice(),
            ["<Super>Right"]
        );
        assert_eq!(record["keys"].as_object().unwrap().len(), 3);
        assert!(!repair_snap(
            &mutter,
            &bindings,
            true,
            |_, _, _| panic!("already repaired"),
            |_, _, _| false
        ));
        assert!(restore_keys(&record, |id| Some(
            if id == "org.gnome.mutter" {
                mutter.clone()
            } else {
                bindings.clone()
            }
        )));
        assert!(!mutter.boolean("edge-tiling"));
        assert!(
            bindings.strv("toggle-tiled-left").is_empty()
                && bindings.strv("toggle-tiled-right").is_empty()
        );
    }
    #[test]
    fn partial_failure_rolls_back_and_preserves_custom_profiles() {
        let (mutter, bindings) = memory();
        let mut writes = 0;
        assert!(!repair_snap(
            &mutter,
            &bindings,
            true,
            |_, _, _| true,
            |s, k, v| {
                writes += 1;
                writes < 3 && s.set_value(k, v).is_ok()
            }
        ));
        assert!(!mutter.boolean("edge-tiling"));
        assert!(
            bindings.strv("toggle-tiled-left").is_empty()
                && bindings.strv("toggle-tiled-right").is_empty()
        );
        bindings
            .set_strv("toggle-tiled-left", ["<Alt>Left"])
            .unwrap();
        assert!(!repair_snap(
            &mutter,
            &bindings,
            true,
            |_, _, _| panic!("custom profile"),
            |_, _, _| false
        ));
        assert_eq!(bindings.strv("toggle-tiled-left").as_slice(), ["<Alt>Left"]);
    }
    #[test]
    fn unconfirmed_backup_prevents_every_write() {
        let (mutter, bindings) = memory();
        let mut writes = 0;
        assert!(!repair_snap(
            &mutter,
            &bindings,
            true,
            |_, _, _| false,
            |_, _, _| {
                writes += 1;
                true
            }
        ));
        assert_eq!(writes, 0);
        assert!(!mutter.boolean("edge-tiling"));
        assert!(
            bindings.strv("toggle-tiled-left").is_empty()
                && bindings.strv("toggle-tiled-right").is_empty()
        );
    }
}
