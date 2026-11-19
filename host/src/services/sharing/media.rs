//! Media Sharing: GNOME Settings' Sharing › Media Sharing (rygel, the DLNA server) in Windows' "Mediastreaming".
//! The switch and the networks are gnome-settings-daemon's Sharing plugin (`org.gnome.SettingsDaemon.Sharing` on the
//! session bus: EnableService("rygel") turns it on for the current network, DisableService("rygel", uuid) off for
//! one network; gsd keeps the list in its relocatable `org.gnome.settings-daemon.plugins.sharing.service` schema);
//! the folders are rygel's own `~/.config/rygel.conf`, written as cc-media-sharing.c does (the system's
//! /etc/rygel.conf as the start, comments kept, the user's special folders as @MUSIC@, @VIDEOS@, @PICTURES@).
use crate::files::ApiError;
use crate::i18n::{Lang, NO_PARAMS};
use crate::services::system::common::{interactive_error, variant_json};
use glib::variant::ToVariant;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub(crate) const GSD: &str = "org.gnome.SettingsDaemon.Sharing";
pub(crate) const GSD_PATH: &str = "/org/gnome/SettingsDaemon/Sharing";
pub(crate) const SERVICE: &str = "rygel";
pub(crate) const RYGEL: &str = "/usr/bin/rygel";
pub(crate) const SYSTEM_CONF: &str = "/etc/rygel.conf";
/// At most this many folders (GNOME has no limit; a guard for the file).
const MAX_FOLDERS: usize = 64;

/// gsd's SharingStatus.
pub(crate) fn status_name(n: u64) -> &'static str {
    match n {
        0 => "offline",
        1 => "disabled-mobile-broadband",
        2 => "disabled-low-security",
        3 => "available",
        _ => "unknown",
    }
}

fn not_running(lang: Lang) -> ApiError {
    ApiError::tr("unsupported", lang, "sharing.noSharing", NO_PARAMS)
}

fn call(
    bus: &gio::DBusConnection,
    iface: &str,
    method: &str,
    args: Option<&glib::Variant>,
) -> Result<glib::Variant, String> {
    bus.call_sync(
        Some(GSD),
        GSD_PATH,
        iface,
        method,
        args,
        None,
        gio::DBusCallFlags::NONE,
        5000,
        gio::Cancellable::NONE,
    )
    .map_err(|e| e.to_string())
}

/// The networks rygel is shared on: ListNetworks("rygel") → [(uuid, name, carrier type)].
pub(crate) fn networks(bus: &gio::DBusConnection) -> Result<Vec<Value>, String> {
    let v = call(bus, GSD, "ListNetworks", Some(&(SERVICE,).to_variant()))?;
    Ok(variant_json(&v.child_value(0))
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|n| {
            let a = n.as_array()?;
            Some(json!({
                "uuid": a.first()?.as_str()?,
                "name": a.get(1).and_then(Value::as_str).unwrap_or(""),
                "carrier": a.get(2).and_then(Value::as_str).unwrap_or(""),
            }))
        })
        .collect())
}

/// The user's file, the system's, in that order.
pub(crate) fn conf_paths() -> (PathBuf, PathBuf) {
    (
        glib::user_config_dir().join("rygel.conf"),
        PathBuf::from(SYSTEM_CONF),
    )
}

/// rygel.conf as GNOME opens it: the user's, else the system's (comments kept).
pub(crate) fn open_conf(user: &Path, system: &Path) -> glib::KeyFile {
    let file = glib::KeyFile::new();
    let flags = glib::KeyFileFlags::KEEP_COMMENTS | glib::KeyFileFlags::KEEP_TRANSLATIONS;
    if file.load_from_file(user, flags).is_err() {
        let _ = file.load_from_file(system, flags);
    }
    file
}

/// The special folders GNOME writes as rygel's names.
pub(crate) fn specials() -> Vec<(&'static str, Option<PathBuf>)> {
    vec![
        (
            "@MUSIC@",
            glib::user_special_dir(glib::UserDirectory::Music),
        ),
        (
            "@VIDEOS@",
            glib::user_special_dir(glib::UserDirectory::Videos),
        ),
        (
            "@PICTURES@",
            glib::user_special_dir(glib::UserDirectory::Pictures),
        ),
    ]
}

/// MediaExport's uris as folders (the special names expanded; an unknown special name is left out).
pub(crate) fn folders(file: &glib::KeyFile, specials: &[(&str, Option<PathBuf>)]) -> Vec<String> {
    file.string_list("MediaExport", "uris")
        .map(|l| {
            l.iter()
                .filter_map(|u| {
                    let u = u.as_str();
                    match specials.iter().find(|(name, _)| *name == u) {
                        Some((_, dir)) => dir.as_ref().map(|d| d.to_string_lossy().into_owned()),
                        None => Some(u.to_owned()),
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

/// GKeyFile's escaping of one string-list item (g_key_file_set_string_list: `\`, newlines, tabs, a leading blank and
/// the list separator).
pub(crate) fn list_item(item: &str) -> String {
    let mut out = String::new();
    for (i, c) in item.chars().enumerate() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            ';' => out.push_str("\\;"),
            ' ' if i == 0 => out.push_str("\\s"),
            c => out.push(c),
        }
    }
    out
}

/// A folder GNOME's chooser could give: an absolute path of a directory name, no control character.
pub(crate) fn folder_ok(f: &str) -> bool {
    f.starts_with('/') && f.len() <= 4096 && !f.chars().any(char::is_control)
}

/// The file's text with GNOME's preferences and `folders` (special folders back to their rygel names).
pub(crate) fn with_folders(
    file: &glib::KeyFile,
    folders: &[String],
    specials: &[(&str, Option<PathBuf>)],
) -> String {
    file.set_boolean("general", "upnp-enabled", true);
    file.set_boolean("Tracker", "enabled", false);
    file.set_boolean("Tracker3", "enabled", false);
    file.set_boolean("MediaExport", "enabled", true);
    let items: Vec<String> = folders
        .iter()
        .map(|f| {
            let name = specials
                .iter()
                .find(|(_, dir)| {
                    dir.as_ref()
                        .is_some_and(|d| d.to_string_lossy() == f.as_str())
                })
                .map(|(name, _)| (*name).to_owned())
                .unwrap_or_else(|| f.clone());
            format!("{};", list_item(&name))
        })
        .collect();
    file.set_value("MediaExport", "uris", &items.concat());
    file.to_data().to_string()
}

/// The checked folder list of sharing.media.folders.set.
pub(crate) fn folders_arg(v: &Value, lang: Lang) -> Result<Vec<String>, ApiError> {
    let bad = || ApiError::tr("bad_args", lang, "sharing.badFolder", NO_PARAMS);
    let list = v
        .as_array()
        .filter(|a| a.len() <= MAX_FOLDERS)
        .ok_or_else(bad)?;
    let mut out: Vec<String> = Vec::new();
    for f in list {
        let f = f.as_str().filter(|f| folder_ok(f)).ok_or_else(bad)?;
        if !out.iter().any(|o| o == f) {
            out.push(f.to_owned());
        }
    }
    Ok(out)
}

/// sharing.media.get: rygel installed, gsd's state (status, current network), whether rygel is shared on the current
/// network, every network it is shared on, and the folders.
pub(crate) fn get(bus: Option<&gio::DBusConnection>) -> Value {
    let (user, system) = conf_paths();
    let file = open_conf(&user, &system);
    let mut out = json!({
        "available": Path::new(RYGEL).exists(),
        "running": false,
        "folders": folders(&file, &specials()),
    });
    let Some(bus) = bus else { return out };
    let props = match call(
        bus,
        "org.freedesktop.DBus.Properties",
        "GetAll",
        Some(&(GSD,).to_variant()),
    ) {
        Ok(v) => variant_json(&v.child_value(0)),
        Err(_) => return out,
    };
    let current = props["CurrentNetwork"].as_str().unwrap_or("").to_owned();
    let list = networks(bus).unwrap_or_default();
    out["running"] = json!(true);
    out["status"] = json!(status_name(props["SharingStatus"].as_u64().unwrap_or(9)));
    out["currentNetwork"] = json!(current);
    out["currentNetworkName"] = props["CurrentNetworkName"].clone();
    out["carrierType"] = props["CarrierType"].clone();
    out["enabled"] = json!(!current.is_empty() && list.iter().any(|n| n["uuid"] == json!(current)));
    out["networks"] = json!(list);
    out
}

/// sharing.media.set {on}: EnableService("rygel") / DisableService("rygel", the current network).
pub(crate) fn set(bus: &gio::DBusConnection, on: bool, lang: Lang) -> Result<(), ApiError> {
    if on {
        call(bus, GSD, "EnableService", Some(&(SERVICE,).to_variant()))
    } else {
        let current = get(Some(bus))["currentNetwork"]
            .as_str()
            .unwrap_or("")
            .to_owned();
        if current.is_empty() {
            return Err(not_running(lang));
        }
        call(
            bus,
            GSD,
            "DisableService",
            Some(&(SERVICE, current.as_str()).to_variant()),
        )
    }
    .map(|_| ())
    .map_err(|e| {
        if e.contains("ServiceUnknown") || e.contains("NameHasNoOwner") {
            not_running(lang)
        } else {
            interactive_error(&e, lang)
        }
    })
}

/// sharing.media.networks.remove {uuid}: DisableService("rygel", uuid).
pub(crate) fn remove_network(
    bus: &gio::DBusConnection,
    uuid: &str,
    lang: Lang,
) -> Result<(), ApiError> {
    call(
        bus,
        GSD,
        "DisableService",
        Some(&(SERVICE, uuid).to_variant()),
    )
    .map(|_| ())
    .map_err(|e| {
        if e.contains("ServiceUnknown") || e.contains("NameHasNoOwner") {
            not_running(lang)
        } else {
            interactive_error(&e, lang)
        }
    })
}

/// sharing.media.folders.set {folders}: rygel.conf written as GNOME writes it.
pub(crate) fn set_folders(folders: &[String], lang: Lang) -> Result<(), ApiError> {
    let (user, system) = conf_paths();
    let file = open_conf(&user, &system);
    let text = with_folders(&file, folders, &specials());
    if let Some(dir) = user.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    super::tls::write_file(&user, &text, 0o644).map_err(|e| {
        ApiError::tr(
            "failed",
            lang,
            "sharing.saveFailed",
            &json!({ "error": e.to_string() }),
        )
    })
}
