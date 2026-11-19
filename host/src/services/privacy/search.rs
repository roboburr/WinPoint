//! GNOME's search providers (privacy.searchProviders): the `gnome-shell/search-providers/*.ini` files of the system
//! data dirs, the same files and order GNOME Shell (remoteSearch.js) and GNOME Settings' Search panel
//! (cc-search-panel.c) read; a provider whose DesktopId names no installed app is left out, as GNOME Settings does.
//! Whether each one shows results and in which order is org.gnome.desktop.search-providers (`disabled`, `enabled`,
//! `sort-order`, `disable-external`), which Settings binds itself.
use gio::prelude::*;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

const GROUP: &str = "Shell Search Provider";
/// More providers than any system ships; the answer stays bounded.
const MAX: usize = 100;

/// One provider file: its DesktopId and DefaultDisabled (None for a file that is no provider).
pub(super) fn parse(path: &Path) -> Option<(String, bool)> {
    if path.extension().is_none_or(|e| e != "ini")
        || std::fs::metadata(path).map_or(true, |m| !m.is_file() || m.len() > 65536)
    {
        return None;
    }
    let kf = glib::KeyFile::new();
    kf.load_from_file(path, glib::KeyFileFlags::NONE).ok()?;
    let id = kf.string(GROUP, "DesktopId").ok()?.to_string();
    if !super::app::desktop_id_ok(&id) {
        return None;
    }
    Some((id, kf.boolean(GROUP, "DefaultDisabled").unwrap_or(false)))
}

/// The providers below `dirs` (each dir's files by name, the first dir that names an app wins).
pub(super) fn files(dirs: &[PathBuf]) -> Vec<(String, bool)> {
    let mut out: Vec<(String, bool)> = Vec::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir.join("gnome-shell").join("search-providers"))
        else {
            continue;
        };
        let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        paths.sort();
        for path in paths {
            if let Some((id, off)) = parse(&path)
                && !out.iter().any(|(o, _)| *o == id)
            {
                out.push((id, off));
            }
            if out.len() >= MAX {
                return out;
            }
        }
    }
    out
}

/// `{providers: [{desktopId, name, icon, defaultDisabled}]}`, by name (Settings orders them by `sort-order`).
pub(super) fn providers(lang: crate::i18n::Lang) -> Value {
    let dirs: Vec<PathBuf> = glib::system_data_dirs();
    let mut list: Vec<Value> = files(&dirs)
        .into_iter()
        .filter_map(|(id, off)| {
            let info = gio::DesktopAppInfo::new(&id)?;
            Some(json!({
                "desktopId": id,
                "name": crate::services::notifications::app_settings::name_in(&info, lang),
                "icon": super::app::icon_string(info.icon()),
                "defaultDisabled": off,
            }))
        })
        .collect();
    list.sort_by_key(|p| p["name"].as_str().unwrap_or("").to_lowercase());
    json!({ "providers": list })
}
