//! Per-MIME defaults for the separate Default Apps program. Reading never changes associations.
use super::{
    catalog::{Catalog, compare_names},
    defaults,
};
use crate::files::{ApiError, ApiResult};
use gio::prelude::*;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};
const MAX_TYPES: usize = 20000;
#[derive(Clone)]
struct Kind {
    mime: String,
    name: String,
    extensions: Vec<String>,
}
struct Cache {
    at: Instant,
    language: String,
    roots: Vec<PathBuf>,
    items: Vec<Kind>,
}
fn valid(mime: &str) -> bool {
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
fn selected(args: &Value, lang: &str) -> Result<String, ApiError> {
    args["mime"]
        .as_str()
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .filter(|s| valid(s))
        .ok_or_else(|| ApiError::tr("bad_args", lang, "apps.invalidFileType", crate::i18n::NO_PARAMS))
}
fn roots() -> Vec<PathBuf> {
    let mut roots = vec![glib::user_data_dir().join("mime")];
    roots.extend(
        std::env::var("XDG_DATA_DIRS")
            .unwrap_or("/usr/local/share:/usr/share".into())
            .split(':')
            .filter(|s| Path::new(s).is_absolute())
            .map(|s| Path::new(s).join("mime")),
    );
    let mut seen = HashSet::new();
    roots.retain(|p| seen.insert(p.clone()));
    roots
}
fn bounded(path: &Path, max: u64) -> Option<String> {
    use std::io::Read;
    let f = std::fs::File::open(path).ok()?;
    let mut data = Vec::new();
    f.take(max + 1).read_to_end(&mut data).ok()?;
    if data.len() as u64 > max {
        return None;
    }
    Some(String::from_utf8_lossy(&data).into_owned())
}
fn label(mime: &str, lang: &str, roots: &[PathBuf]) -> String {
    for root in roots {
        let Some(text) = bounded(&root.join(format!("{mime}.xml")), 512 << 10) else {
            continue;
        };
        let Ok(doc) = roxmltree::Document::parse(&text) else {
            continue;
        };
        let mut fallback = None;
        for c in doc.descendants().filter(|n| n.has_tag_name("comment")) {
            let language = c.attribute(("http://www.w3.org/XML/1998/namespace", "lang"));
            if language == Some(lang) {
                if let Some(s) = c.text() {
                    return s.to_string();
                }
            }
            if language.is_none() {
                fallback = c.text()
            }
        }
        if let Some(s) = fallback {
            return s.to_string();
        }
    }
    if mime.starts_with("x-scheme-handler/") {
        let s = mime.split_once('/').unwrap().1;
        return crate::i18n::trf(lang, "apps.schemeLinks", &serde_json::json!({"scheme": s.to_string()}));
    }
    gio::content_type_get_description(mime).to_string()
}
fn kinds(lang: &str) -> Vec<Kind> {
    static CACHE: OnceLock<Mutex<Option<Cache>>> = OnceLock::new();
    let mut cache = CACHE
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let roots = roots();
    if let Some(c) = cache.as_ref()
        && c.language == lang
        && c.roots == roots
        && c.at.elapsed() < Duration::from_secs(30)
    {
        return c.items.clone();
    }
    let mut types = BTreeMap::<String, BTreeSet<String>>::new();
    for root in &roots {
        if let Some(text) = bounded(&root.join("types"), 4 << 20) {
            for line in text.lines().map(str::trim).filter(|s| valid(s)) {
                if types.len() < MAX_TYPES || types.contains_key(line) {
                    types.entry(line.to_ascii_lowercase()).or_default();
                }
            }
        }
        if let Some(text) = bounded(&root.join("globs2"), 8 << 20) {
            for line in text.lines().filter(|s| !s.starts_with('#')) {
                let mut parts = line.split(':');
                let _weight = parts.next();
                let (Some(mime), Some(glob)) = (parts.next(), parts.next()) else {
                    continue;
                };
                if !valid(mime) {
                    continue;
                }
                let Some(ext) = glob.strip_prefix("*.") else {
                    continue;
                };
                if ext.is_empty()
                    || ext.len() > 80
                    || ext
                        .chars()
                        .any(|c| c.is_control() || "*?[]{}\\/".contains(c))
                {
                    continue;
                }
                if types.len() < MAX_TYPES || types.contains_key(mime) {
                    let entries = types.entry(mime.to_ascii_lowercase()).or_default();
                    if entries.len() < 64 {
                        entries.insert(format!(".{ext}"));
                    }
                }
            }
        }
    }
    // GIO contributes user-defined MIME types and registered link schemes not present in types/globs2.
    for t in gio::content_types_get_registered() {
        if types.len() >= MAX_TYPES {
            break;
        }
        let t = t.to_ascii_lowercase();
        if valid(&t) {
            types.entry(t).or_default();
        }
    }
    for info in gio::AppInfo::all() {
        for t in info.supported_types() {
            let t = t.to_ascii_lowercase();
            if valid(&t) && (types.len() < MAX_TYPES || types.contains_key(&t)) {
                types.entry(t).or_default();
            }
        }
    }
    let items = types
        .into_iter()
        .map(|(mime, e)| Kind {
            name: label(&mime, lang, &roots),
            mime,
            extensions: e.into_iter().collect(),
        })
        .collect::<Vec<_>>();
    *cache = Some(Cache {
        at: Instant::now(),
        language: lang.into(),
        roots,
        items: items.clone(),
    });
    items
}
fn record(kind: &Kind, catalog: &Catalog, lang: &str) -> Value {
    let default =
        gio::AppInfo::default_for_type(&kind.mime, false).and_then(|a| catalog.brief(&a, lang));
    json!({"mime":kind.mime,"name":kind.name,"extensions":kind.extensions,"default":default})
}
pub(super) fn call(method: &str, args: &Value, catalog: &Catalog, lang: &str) -> Option<ApiResult> {
    match method {
        "apps.mimeTypes" => Some(list(args, catalog, lang)),
        "apps.forType" => Some(for_type(args, catalog, lang)),
        "apps.setDefaultForType" => Some(set(args, lang)),
        _ => None,
    }
}
fn integer(
    args: &Value,
    key: &str,
    default: u64,
    max: u64,
    lang: &str,
) -> Result<usize, ApiError> {
    match &args[key] {
        Value::Null => Ok(default as usize),
        v => v
            .as_u64()
            .filter(|n| *n <= max)
            .map(|n| n as usize)
            .ok_or_else(|| ApiError::tr("bad_args", lang, "apps.invalidInput", crate::i18n::NO_PARAMS)),
    }
}
fn list(args: &Value, catalog: &Catalog, lang: &str) -> ApiResult {
    let query = match &args["query"] {
        Value::Null => String::new(),
        Value::String(s) if s.chars().count() <= 200 => s.trim().to_lowercase(),
        _ => return Err(ApiError::tr("bad_args", lang, "apps.invalidSearch", crate::i18n::NO_PARAMS)),
    };
    let offset = integer(args, "offset", 0, MAX_TYPES as u64, lang)?;
    let limit = integer(args, "limit", 100, 200, lang)?.max(1);
    let terms = query.split_whitespace().collect::<Vec<_>>();
    let mut items = kinds(lang)
        .into_iter()
        .filter(|k| {
            let search = format!("{} {} {}", k.mime, k.name, k.extensions.join(" ")).to_lowercase();
            terms.iter().all(|q| search.contains(q))
        })
        .collect::<Vec<_>>();
    items.sort_by(|a, b| a.mime.cmp(&b.mime));
    let total = items.len();
    let items = items
        .iter()
        .skip(offset)
        .take(limit)
        .map(|k| record(k, catalog, lang))
        .collect::<Vec<_>>();
    Ok(json!({"items":items,"total":total,"offset":offset,"limit":limit}))
}
fn for_type(args: &Value, catalog: &Catalog, lang: &str) -> ApiResult {
    let mime = selected(args, lang)?;
    let kind = kinds(lang)
        .into_iter()
        .find(|k| k.mime == mime)
        .unwrap_or_else(|| Kind {
            name: label(&mime, lang, &roots()),
            mime: mime.clone(),
            extensions: vec![],
        });
    let mut result = record(&kind, catalog, lang);
    let default = result["default"]["id"].as_str().map(String::from);
    let mut infos = gio::AppInfo::all_for_type(&mime);
    if let Some(a) = gio::AppInfo::default_for_type(&mime, false) {
        infos.insert(0, a)
    }
    let mut seen = HashSet::new();
    let mut apps = infos
        .iter()
        .filter_map(|a| {
            let id = a.id()?;
            let is_default = Some(id.as_str()) == default.as_deref();
            if !seen.insert(id.to_string()) || !is_default && !a.should_show() {
                return None;
            }
            let mut brief = catalog.brief(a, lang)?;
            brief["isDefault"] = json!(is_default);
            Some(brief)
        })
        .collect::<Vec<_>>();
    apps.sort_by(|a, b| {
        b["isDefault"]
            .as_bool()
            .cmp(&a["isDefault"].as_bool())
            .then_with(|| {
                compare_names(
                    a["name"].as_str().unwrap_or(""),
                    b["name"].as_str().unwrap_or(""),
                )
            })
    });
    result["apps"] = json!(apps);
    Ok(result)
}
fn set(args: &Value, lang: &str) -> ApiResult {
    let mime = selected(args, lang)?;
    let id = args["id"]
        .as_str()
        .filter(|s| {
            !s.is_empty() && s.len() <= 300 && !s.contains('/') && !s.chars().any(char::is_control)
        })
        .ok_or_else(|| ApiError::tr("bad_args", lang, "apps.invalidApp", crate::i18n::NO_PARAMS))?;
    let info = gio::DesktopAppInfo::new(id)
        .ok_or_else(|| ApiError::tr("not_found", lang, "apps.appNotFound", crate::i18n::NO_PARAMS))?;
    if crate::backend::dry_run() {
        return Ok(json!({"ok":true,"dryRun":true,"types":[mime]}));
    }
    let config = glib::user_config_dir();
    let _lock = defaults::ConfigLock::acquire(&config)
        .map_err(|_| ApiError::tr("failed", lang, "apps.defaultNotChanged", crate::i18n::NO_PARAMS))?;
    info.set_as_default_for_type(&mime)
        .map_err(|_| ApiError::tr("failed", lang, "apps.defaultNotChanged", crate::i18n::NO_PARAMS))?;
    let current = gio::AppInfo::default_for_type(&mime, false)
        .and_then(|a| a.id())
        .map(|s| s.to_string());
    let effective = current.as_deref() == Some(id);
    let mut result = json!({"types":[mime],"effective":effective,"current":current});
    if !effective {
        result["overriddenBy"] = json!(defaults::desktop_mimeapps(
            &config,
            &std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default()
        ));
    }
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_link_type_without_a_description_is_named_in_the_settings_language() {
        // The former inline pair: nl is the 3b898e6 "{s}-koppelingen", en its English half.
        assert_eq!(label("x-scheme-handler/mailto", "nl", &[]), "mailto-koppelingen");
        assert_eq!(label("x-scheme-handler/mailto", "en", &[]), "mailto links");
    }
}
