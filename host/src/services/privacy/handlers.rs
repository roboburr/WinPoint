//! An app's file and link types, GNOME Settings' "Bestanden en koppelingen" (cc-applications-panel.c
//! update_handler_sections): the types of its `MimeType=` for which GIO recommends it, whether it is the default,
//! removing one (`g_app_info_remove_supports_type`, which records it under [Removed Associations] of
//! `~/.config/mimeapps.list`) and Reset (`g_app_info_reset_type_associations` for each of its types).
use crate::i18n::Lang;
use gio::prelude::*;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};

/// A MIME type or `x-scheme-handler/<scheme>` (apps/pertype.rs `valid`).
pub(super) fn type_ok(mime: &str) -> bool {
    mime.len() <= 200
        && mime.split_once('/').is_some_and(|(a, b)| {
            a.as_bytes().first().is_some_and(u8::is_ascii_alphanumeric)
                && b.as_bytes().first().is_some_and(u8::is_ascii_alphanumeric)
                && !b.contains('/')
                && mime
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"!#$&^_.+-/".contains(&b))
        })
}

/// The content types of an app's `MimeType=`, normalised and without repeats, in their order.
pub(super) fn supported(info: &gio::DesktopAppInfo) -> Vec<String> {
    let mut seen = HashSet::new();
    info.supported_types()
        .iter()
        .map(|t| {
            gio::content_type_from_mime_type(t)
                .map(|c| c.to_string())
                .unwrap_or_else(|| t.to_string())
        })
        .filter(|t| type_ok(t) && seen.insert(t.clone()))
        .collect()
}

fn mime_roots() -> Vec<PathBuf> {
    let mut roots = vec![glib::user_data_dir().join("mime")];
    roots.extend(
        std::env::var("XDG_DATA_DIRS")
            .unwrap_or_else(|_| "/usr/local/share:/usr/share".into())
            .split(':')
            .filter(|s| Path::new(s).is_absolute())
            .map(|s| Path::new(s).join("mime")),
    );
    roots
}

/// The `*.ext` globs of shared-mime-info's `globs2` texts: MIME type → ".ext" (at most 3 each, the heaviest first).
pub(super) fn extensions(texts: &[String]) -> BTreeMap<String, Vec<String>> {
    let mut weighted: BTreeMap<String, Vec<(u32, String)>> = BTreeMap::new();
    for text in texts {
        for line in text.lines().filter(|l| !l.starts_with('#')) {
            let mut parts = line.split(':');
            let (Some(weight), Some(mime), Some(glob)) = (parts.next(), parts.next(), parts.next())
            else {
                continue;
            };
            let Some(ext) = glob.strip_prefix("*.") else {
                continue;
            };
            if ext.is_empty()
                || ext.len() > 20
                || ext
                    .chars()
                    .any(|c| c.is_control() || "*?[]{}\\/".contains(c))
            {
                continue;
            }
            let list = weighted.entry(mime.to_ascii_lowercase()).or_default();
            let ext = format!(".{ext}");
            if !list.iter().any(|(_, e)| *e == ext) {
                list.push((weight.parse().unwrap_or(50), ext));
            }
        }
    }
    weighted
        .into_iter()
        .map(|(mime, mut list)| {
            list.sort_by(|a, b| b.0.cmp(&a.0));
            (mime, list.into_iter().take(3).map(|(_, e)| e).collect())
        })
        .collect()
}

fn globs() -> BTreeMap<String, Vec<String>> {
    let texts: Vec<String> = mime_roots()
        .iter()
        .filter_map(|r| {
            let p = r.join("globs2");
            (std::fs::metadata(&p).ok()?.len() <= 8 << 20)
                .then(|| std::fs::read_to_string(&p).ok())?
        })
        .collect();
    extensions(&texts)
}

/// A file type's name in the settings language: the `<comment xml:lang>` of shared-mime-info's `<type>.xml` (as
/// apps.mimeTypes names it), the untranslated comment, else GIO's description (in the process's locale).
pub(super) fn label(mime: &str, lang: Lang, roots: &[PathBuf]) -> String {
    let wanted: &[&str] = if lang.code() == "en" {
        &["en_GB", "en"]
    } else {
        &["nl", "nl_NL"]
    };
    for root in roots {
        let path = root.join(format!("{mime}.xml"));
        let Some(text) = std::fs::metadata(&path)
            .ok()
            .filter(|m| m.len() <= 512 << 10)
            .and_then(|_| std::fs::read_to_string(&path).ok())
        else {
            continue;
        };
        let Ok(doc) = roxmltree::Document::parse(&text) else {
            continue;
        };
        let comments: Vec<(Option<&str>, &str)> = doc
            .descendants()
            .filter(|n| n.has_tag_name("comment"))
            .filter_map(|n| {
                Some((
                    n.attribute(("http://www.w3.org/XML/1998/namespace", "lang")),
                    n.text()?,
                ))
            })
            .collect();
        let found = wanted
            .iter()
            .find_map(|w| comments.iter().find(|(l, _)| *l == Some(*w)))
            .or_else(|| comments.iter().find(|(l, _)| l.is_none()));
        if let Some((_, text)) = found {
            return (*text).to_owned();
        }
    }
    gio::content_type_get_description(mime).to_string()
}

fn default_id(content: &str) -> Option<String> {
    let info = match content.strip_prefix("x-scheme-handler/") {
        Some(scheme) => gio::AppInfo::default_for_uri_scheme(scheme),
        None => gio::AppInfo::default_for_type(content, false),
    };
    info.and_then(|i| i.id()).map(|id| id.to_string())
}

/// privacy.app's `handlers`: the file types first, then the link types, each by name.
pub(super) fn list(info: &gio::DesktopAppInfo, lang: Lang) -> Vec<Value> {
    let id = info.id().map(|s| s.to_string()).unwrap_or_default();
    let globs = globs();
    let roots = mime_roots();
    let mut out: Vec<Value> = supported(info)
        .into_iter()
        .filter(|t| {
            gio::AppInfo::recommended_for_type(t)
                .iter()
                .any(|a| a.id().as_deref() == Some(id.as_str()))
        })
        .map(|t| {
            let link = t.strip_prefix("x-scheme-handler/");
            let name = match link {
                Some(scheme) => crate::i18n::trf(
                    lang.code(),
                    "apps.schemeLinks",
                    &json!({ "scheme": scheme }),
                ),
                None => label(&t, lang, &roots),
            };
            json!({
                "type": t,
                "kind": if link.is_some() { "link" } else { "file" },
                "name": name,
                "extensions": globs.get(&t).cloned().unwrap_or_default(),
                "isDefault": default_id(&t).as_deref() == Some(id.as_str()),
            })
        })
        .collect();
    out.sort_by(|a, b| {
        (a["kind"] == "link")
            .cmp(&(b["kind"] == "link"))
            .then_with(|| {
                a["name"]
                    .as_str()
                    .unwrap_or("")
                    .to_lowercase()
                    .cmp(&b["name"].as_str().unwrap_or("").to_lowercase())
            })
    });
    out
}

/// Every type the user's `mimeapps.list` names in [Default Applications], [Added Associations] or [Removed
/// Associations] ("Alle standaard-apps opnieuw instellen" resets each of them).
pub(super) fn user_types(mimeapps: &str) -> Vec<String> {
    let kf = glib::KeyFile::new();
    if kf
        .load_from_data(mimeapps, glib::KeyFileFlags::NONE)
        .is_err()
    {
        return Vec::new();
    }
    let mut out = BTreeSet::new();
    for group in [
        "Default Applications",
        "Added Associations",
        "Removed Associations",
    ] {
        if let Ok(keys) = kf.keys(group) {
            out.extend(keys.iter().map(|k| k.to_string()).filter(|k| type_ok(k)));
        }
    }
    out.into_iter().collect()
}

/// Reset the associations of `types` (GNOME's Reset): the user's own choices for them go.
pub(super) fn reset(types: &[String]) {
    for t in types {
        gio::AppInfo::reset_type_associations(t);
    }
}
