//! One installed app as GNOME Settings' Apps panel shows it (privacy.app): what it is (Flatpak, Snap or a system
//! app) and its portal app id, its version and disk use, the PermissionStore rows, Run in Background and Autostart,
//! its search provider, its global shortcuts, the sandbox's static permissions or the snap's interfaces, and its
//! file and link types. Also the Autostart write (`~/.config/autostart/<desktop id>`, as GNOME: a copy of the app's
//! desktop file with `X-GNOME-Autostart-enabled`).
use super::{flatpak, handlers, snapd};
use gio::prelude::*;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// A desktop id as apps.list gives it (`org.gnome.Calculator.desktop`, `firefox_firefox.desktop`).
pub(super) fn desktop_id_ok(id: &str) -> bool {
    id.ends_with(".desktop")
        && id.len() > 8
        && id.len() <= 300
        && !id.starts_with('.')
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._@+-".contains(&b))
}

/// What kind of app it is.
#[derive(Debug, PartialEq)]
pub(super) enum Kind {
    Flatpak(String),
    Snap(String),
    Host,
}

impl Kind {
    pub(super) fn of(info: &gio::DesktopAppInfo) -> Kind {
        if let Some(id) = info
            .string("X-Flatpak")
            .map(|s| s.to_string())
            .filter(|s| flatpak::id_ok(s))
        {
            return Kind::Flatpak(id);
        }
        if let Some(name) = info
            .string("X-SnapInstanceName")
            .map(|s| s.to_string())
            .filter(|s| snapd::name_ok(s))
        {
            return Kind::Snap(name);
        }
        Kind::Host
    }

    /// The app id the portals know it by (cc-applications-panel.c get_portal_app_id).
    pub(super) fn portal_id(&self) -> Option<String> {
        match self {
            Kind::Flatpak(id) => Some(id.clone()),
            Kind::Snap(name) => Some(format!("snap.{name}")),
            Kind::Host => None,
        }
    }

    fn name(&self) -> &'static str {
        match self {
            Kind::Flatpak(_) => "flatpak",
            Kind::Snap(_) => "snap",
            Kind::Host => "host",
        }
    }
}

pub(super) fn icon_string(icon: Option<gio::Icon>) -> String {
    let Some(icon) = icon else {
        return String::new();
    };
    if let Some(themed) = icon.downcast_ref::<gio::ThemedIcon>() {
        return themed
            .names()
            .first()
            .map(ToString::to_string)
            .unwrap_or_default();
    }
    if let Some(file) = icon.downcast_ref::<gio::FileIcon>() {
        return file
            .file()
            .path()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
    }
    String::new()
}

// ---------------------------------------------------------------------------------------------- search provider

/// The search provider an app ships (`<data dir>/gnome-shell/search-providers/*.ini` whose DesktopId is the app):
/// whether it is off unless enabled (DefaultDisabled), which decides between the `disabled` and `enabled` lists of
/// org.gnome.desktop.search-providers.
pub(super) fn search_provider(id: &str, dirs: &[PathBuf]) -> Option<Value> {
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir.join("gnome-shell").join("search-providers"))
        else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "ini")
                || std::fs::metadata(&path).map_or(true, |m| m.len() > 65536)
            {
                continue;
            }
            let kf = glib::KeyFile::new();
            if kf.load_from_file(&path, glib::KeyFileFlags::NONE).is_err() {
                continue;
            }
            let group = "Shell Search Provider";
            if kf
                .string(group, "DesktopId")
                .is_ok_and(|d| d.as_str() == id)
            {
                return Some(
                    json!({ "desktopId": id, "defaultDisabled": kf.boolean(group, "DefaultDisabled").unwrap_or(false) }),
                );
            }
        }
    }
    None
}

pub(super) fn data_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![glib::user_data_dir()];
    dirs.extend(glib::system_data_dirs());
    dirs
}

// --------------------------------------------------------------------------------------------------- autostart

/// Set `key` in a desktop file's [Desktop Entry] group, keeping everything else.
pub(super) fn set_desktop_key(text: &str, key: &str, value: &str) -> String {
    let mut out = Vec::new();
    let (mut in_entry, mut done) = (false, false);
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') && t.ends_with(']') {
            if in_entry && !done {
                out.push(format!("{key}={value}"));
                done = true;
            }
            in_entry = t == "[Desktop Entry]";
        } else if in_entry && t.split_once('=').is_some_and(|(k, _)| k.trim() == key) {
            if !done {
                out.push(format!("{key}={value}"));
                done = true;
            }
            continue;
        }
        out.push(line.to_owned());
    }
    if !done {
        if !in_entry {
            out.push("[Desktop Entry]".into());
        }
        out.push(format!("{key}={value}"));
    }
    out.join("\n") + "\n"
}

/// Whether a desktop file's text starts its app at log-in (GNOME: not Hidden, X-GNOME-Autostart-enabled not false).
pub(super) fn starts(text: &str) -> bool {
    let kf = glib::KeyFile::new();
    if kf.load_from_data(text, glib::KeyFileFlags::NONE).is_err() {
        return false;
    }
    let g = "Desktop Entry";
    !kf.boolean(g, "Hidden").unwrap_or(false)
        && kf.boolean(g, "X-GNOME-Autostart-enabled").unwrap_or(true)
}

fn system_autostart_dirs() -> Vec<PathBuf> {
    glib::system_config_dirs()
        .into_iter()
        .map(|d| d.join("autostart"))
        .collect()
}

/// The autostart file in effect for `id`: the user's, else the first system one.
fn autostart_file(id: &str, user: &Path) -> Option<PathBuf> {
    std::iter::once(user.join(id))
        .chain(system_autostart_dirs().into_iter().map(|d| d.join(id)))
        .find(|p| p.is_file())
}

/// privacy.app's `autostart`: whether it starts at log-in.
pub(super) fn autostart(id: &str, user: &Path) -> Value {
    let enabled = autostart_file(id, user)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .is_some_and(|t| starts(&t));
    json!({ "enabled": enabled })
}

/// The new text of the user's autostart file: the file in effect (or the app's own desktop file) with the flag set.
pub(super) fn autostart_text(base: &str, on: bool, generated: bool) -> String {
    let mut text = set_desktop_key(
        base,
        "X-GNOME-Autostart-enabled",
        if on { "true" } else { "false" },
    );
    if on {
        text = set_desktop_key(&text, "Hidden", "false");
    }
    if generated {
        text = set_desktop_key(&text, "X-GNOME-Settings-Autostart-Generated", "true");
    }
    text
}

/// Write the user's autostart file for `id` (atomically, 0644).
pub(super) fn write_autostart(user: &Path, id: &str, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::create_dir_all(user)?;
    let target = user.join(id);
    if std::fs::symlink_metadata(&target).is_ok_and(|m| m.is_symlink()) {
        std::fs::remove_file(&target)?;
    }
    let temp = user.join(format!(".{id}.{}.new", crate::events::nonce()));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o644)
        .open(&temp)?;
    let result = file
        .write_all(text.as_bytes())
        .and_then(|_| file.sync_all())
        .and_then(|_| std::fs::rename(&temp, &target));
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

/// The base text for a new autostart file: the file in effect, else the app's desktop file (and whether it is new).
pub(super) fn autostart_base(
    info: &gio::DesktopAppInfo,
    id: &str,
    user: &Path,
) -> Option<(String, bool)> {
    if let Some(text) = autostart_file(id, user).and_then(|p| std::fs::read_to_string(p).ok()) {
        return Some((text, false));
    }
    let path = info.filename()?;
    (std::fs::metadata(&path).ok()?.len() <= 1 << 20)
        .then(|| std::fs::read_to_string(&path).ok().map(|t| (t, true)))?
}

// ------------------------------------------------------------------------------------------------------ storage

fn sizes_json(app: Option<u64>, data: Option<u64>, cache: Option<u64>, truncated: bool) -> Value {
    let total = app.unwrap_or(0) + data.unwrap_or(0) + cache.unwrap_or(0);
    json!({ "app": app, "data": data, "cache": cache, "total": total, "truncated": truncated })
}

/// A Flatpak app's version, disk use and static permissions.
fn flatpak_info(id: &str, home: &Path, sizes: bool, out: &mut Value) {
    let deploy = flatpak::deploy(&flatpak::installations(), id);
    if let Some(deploy) = &deploy {
        out["version"] = json!(flatpak::version(deploy, id));
        let metadata = std::fs::read_to_string(deploy.join("metadata")).unwrap_or_default();
        out["required"] =
            json!({ "kind": "flatpak", "static": flatpak::static_permissions(&metadata) });
    }
    let dirs = flatpak::data_dirs(home, id);
    out["canClearCache"] = json!(dirs.as_ref().is_some_and(|(_, cache)| cache.is_dir()));
    if sizes {
        let mut walk = flatpak::Walk::new();
        let app = deploy.as_ref().map(|d| walk.usage(&d.join("files"), None));
        let (data, cache) = match &dirs {
            Some((data, cache)) => (
                Some(walk.usage(data, Some(cache))),
                Some(walk.usage(cache, None)),
            ),
            None => (None, None),
        };
        out["storage"] = sizes_json(app, data, cache, walk.truncated);
    }
}

/// A snap's version, size, data use and interfaces (snapd unreachable: none of them).
fn snap_info(name: &str, home: &Path, sizes: bool, socket: &Path, out: &mut Value) {
    let wait = Duration::from_secs(10);
    let snap = snapd::request(socket, "GET", &format!("/v2/snaps/{name}"), None, wait).ok();
    if let Some(snap) = &snap {
        out["version"] = snap["result"]["version"].clone();
    }
    if let Ok(connections) = snapd::request(socket, "GET", "/v2/connections?select=all", None, wait)
    {
        out["required"] =
            json!({ "kind": "snap", "plugs": snapd::plugs(&connections["result"], name) });
    }
    if sizes {
        let mut walk = flatpak::Walk::new();
        let app = snap
            .as_ref()
            .and_then(|s| s["result"]["installed-size"].as_u64());
        let data_dir = home.join("snap").join(name);
        let data = std::fs::symlink_metadata(&data_dir)
            .is_ok_and(|m| m.is_dir())
            .then(|| walk.usage(&data_dir, None));
        out["storage"] = sizes_json(app, data, None, walk.truncated);
    }
}

/// Everything privacy.app answers but the PermissionStore rows and the global shortcuts (privacy.rs adds them).
pub(super) fn describe(
    info: &gio::DesktopAppInfo,
    id: &str,
    lang: crate::i18n::Lang,
    sizes: bool,
    socket: &Path,
) -> Value {
    let kind = Kind::of(info);
    let home = glib::home_dir();
    let user_autostart = glib::user_config_dir().join("autostart");
    let mut out = json!({
        "id": id,
        "name": info.name().as_str(),
        "icon": icon_string(info.icon()),
        "kind": kind.name(),
        "portalId": kind.portal_id(),
        "sandboxed": kind != Kind::Host,
        "version": null,
        "storage": null,
        "canClearCache": false,
        "required": null,
        "autostart": autostart(id, &user_autostart),
        "searchProvider": search_provider(id, &data_dirs()),
        "handlers": handlers::list(info, lang),
    });
    match &kind {
        Kind::Flatpak(fid) => {
            out["flatpakId"] = json!(fid);
            flatpak_info(fid, &home, sizes, &mut out);
        }
        Kind::Snap(name) => {
            out["snapName"] = json!(name);
            snap_info(name, &home, sizes, socket, &mut out);
        }
        Kind::Host => {}
    }
    out
}
