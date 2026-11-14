//! Installed application metadata. Only owned JSON/string data survives a build.
use crate::files::ApiResult;
use gio::prelude::*;
use glib::Unichar;
use gtk::{gio, glib};
use serde_json::{Value, json};
use std::{
    cmp::Ordering,
    collections::HashMap,
    fs::{self, File, OpenOptions},
    io::Write,
    os::{
        fd::AsRawFd,
        unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicU64, Ordering as AtomicOrdering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

const OWN_IDS: [&str; 7] = [
    "settings",
    "taskmgr",
    "explorer",
    "alles",
    "disks",
    "defaultapps",
    "installer",
];
const LOCALE_ROOTS: [&str; 3] = [
    "/usr/share/locale-langpack",
    "/usr/share/locale",
    "/usr/local/share/locale",
];

pub(super) struct Catalog {
    config: PathBuf,
    state: Mutex<State>,
}
struct State {
    dirty: bool,
    language: String,
    apps: Vec<Value>,
    by_id: HashMap<String, Value>,
    translator: Option<Translator>,
}
impl Catalog {
    pub(super) fn new(config: PathBuf) -> Self {
        Self {
            config,
            state: Mutex::new(State {
                dirty: true,
                language: String::new(),
                apps: Vec::new(),
                by_id: HashMap::new(),
                translator: None,
            }),
        }
    }
    pub(super) fn invalidate(&self) {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).dirty = true;
    }
    pub(super) fn list(&self, lang: &str) -> ApiResult {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        self.ensure(&mut state, lang);
        Ok(Value::Array(state.apps.clone()))
    }
    pub(super) fn brief_id(&self, id: &str, lang: &str) -> Option<Value> {
        if id.is_empty() {
            return None;
        }
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        self.ensure(&mut state, lang);
        if let Some(value) = state.by_id.get(id) {
            return Some(brief_value(value));
        }
        let info = gio::DesktopAppInfo::new(id)?;
        let name = info.name();
        Some(
            json!({"id":id,"name":if name.is_empty(){id}else{name.as_str()},"icon":fallback_icon(icon_string(info.icon()))}),
        )
    }
    pub(super) fn brief(&self, info: &gio::AppInfo, lang: &str) -> Option<Value> {
        let id = info.id();
        if let Some(value) = id.as_deref().and_then(|id| self.brief_id(id, lang)) {
            return Some(value);
        }
        Some(
            json!({"id":id.as_deref(),"name":info.name().as_str(),"icon":fallback_icon(icon_string(info.icon()))}),
        )
    }
    fn ensure(&self, state: &mut State, lang: &str) {
        let lang = if lang == "en" { "en" } else { "nl" };
        if !state.dirty && state.language == lang {
            return;
        }
        let translate = lang != system_language();
        if translate
            && !state
                .translator
                .as_ref()
                .is_some_and(|tr| tr.language == lang)
        {
            state.translator = Some(Translator::new(
                lang,
                LOCALE_ROOTS.iter().map(PathBuf::from).collect(),
            ));
        }
        let mut records = Vec::new();
        // GIO instances are created, read and dropped on this calling thread.
        for info in gio::AppInfo::all() {
            if let Some(record) = Record::from_info(&info) {
                records.push(record);
            }
        }
        let (apps, by_id) = build(
            &self.config,
            lang,
            records,
            if translate {
                state.translator.as_mut()
            } else {
                None
            },
            now_ms(),
        );
        state.apps = apps;
        state.by_id = by_id;
        state.language = lang.into();
        state.dirty = false;
    }
}

fn brief_value(value: &Value) -> Value {
    json!({"id":value["id"],"name":value["name"],"icon":fallback_icon(value["icon"].as_str().unwrap_or("").to_owned())})
}
fn fallback_icon(icon: String) -> String {
    if icon.is_empty() {
        "application-x-executable".into()
    } else {
        icon
    }
}
pub(super) fn icon_string(icon: Option<gio::Icon>) -> String {
    let Some(icon) = icon else {
        return String::new();
    };
    if let Some(icon) = icon.downcast_ref::<gio::ThemedIcon>() {
        return icon
            .names()
            .first()
            .map(ToString::to_string)
            .unwrap_or_default();
    }
    if let Some(icon) = icon.downcast_ref::<gio::FileIcon>() {
        return icon
            .file()
            .path()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
    }
    gio::prelude::IconExt::to_string(&icon)
        .filter(|s| !s.starts_with(". "))
        .map(|s| s.to_string())
        .unwrap_or_default()
}
fn data() -> &'static Value {
    static DATA: OnceLock<Value> = OnceLock::new();
    DATA.get_or_init(|| {
        serde_json::from_str(include_str!("catalog-data.json")).expect("reference app metadata")
    })
}
/// Verkenner's table of GNOME app names (files/launcher-names.json): `NL_NAMES` (the Dutch name when the entry has
/// none of its own) and `SAME_NAME_AS_OURS` (a GNOME app that shares a name with one of ours, per language). The
/// app catalogue reads the same table instead of keeping a copy (plan-critique H). Data in both languages, never a
/// rendered text, so it may live in a static.
fn launcher_names() -> &'static Value {
    static NAMES: OnceLock<Value> = OnceLock::new();
    NAMES.get_or_init(|| {
        serde_json::from_str(include_str!("../files/launcher-names.json"))
            .expect("bundled launcher names")
    })
}
/// F38: the English text of a desktop-entry key, for an English catalogue on a non-English system (GIO's own
/// texts are in the system locale, so Dutch on a Dutch machine). In this order, each only when it is not empty:
/// `key[en_GB]`; then `translator.get` in English, i.e. `key[en]` when it differs from `key=`, or the `en` gettext
/// translation of `key=`; then the untranslated `key=`. None when the entry has no English text for the key.
fn english(translator: &mut Translator, file: Option<&glib::KeyFile>, key: &str) -> Option<String> {
    let file = file?;
    let exact = |name: &str| {
        file.string("Desktop Entry", name)
            .ok()
            .map(|value| value.to_string())
            .filter(|value| !value.is_empty())
    };
    exact(&format!("{key}[en_GB]"))
        .or_else(|| Some(translator.get(Some(file), key, "")).filter(|value| !value.is_empty()))
        .or_else(|| exact(key))
}
pub(super) fn own_icon(app: &str) -> String {
    static CACHE: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    let mut cache = CACHE
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    cache
        .entry(app.into())
        .or_insert_with(|| {
            let icons = data()["ownApps"][app]["icons"]
                .as_array()
                .expect("known own app");
            icons
                .iter()
                .filter_map(Value::as_str)
                .find(|icon| crate::files::icon_exists(icon))
                .unwrap_or_else(|| icons[1].as_str().unwrap())
                .to_owned()
        })
        .clone()
}
/// The desktop entry scripts/install.py writes for an own app: `<GApplication id>.desktop` (launch.rs OWN_APP_IDS).
fn own_entry(app: &str) -> Option<String> {
    crate::launch::app_id(app).map(|id| format!("{id}.desktop"))
}
/// The own app a desktop entry is: the entry install.py writes for it (io.github.win11shell.<Name>.desktop), or a
/// `win11shell-<app>.desktop` of Win11Shell's own naming (the catalogue's ids).
fn own_of_entry(id: &str) -> Option<&'static str> {
    OWN_IDS
        .iter()
        .copied()
        .find(|app| own_entry(app).as_deref() == Some(id) || id == format!("win11shell-{app}.desktop"))
}
/// An own app as the catalogue lists it: id `win11shell-<app>.desktop`, our name and texts, the icon of its
/// installed entry (`entry`: the Icon= scripts/install.py gives it, a file of the bundled Win11 icons) when that
/// resolves, else the first of catalog-data.json's names that does.
fn own_value(app: &str, lang: &str, installed: Option<&Record>) -> Value {
    let meta = &data()["ownApps"][app];
    let icon = installed
        .map(|record| record.icon.as_str())
        .filter(|icon| !icon.is_empty() && crate::files::icon_exists(icon))
        .map(str::to_owned)
        .unwrap_or_else(|| own_icon(app));
    json!({"id":format!("win11shell-{app}.desktop"),"entry":own_entry(app),"name":meta["name"][lang],"genericName":meta["generic"][lang],"comment":meta["comment"][lang],"icon":icon,"categories":meta["categories"],"keywords":meta["keywords"],"noDisplay":false,"own":true,"added":0,"installedAt":0})
}

struct Record {
    id: String,
    name: String,
    icon: String,
    visible: bool,
    path: Option<PathBuf>,
    generic: String,
    comment: String,
    keywords: Vec<String>,
    categories: Vec<String>,
}
impl Record {
    fn from_info(info: &gio::AppInfo) -> Option<Self> {
        let id = info.id()?.to_string();
        if id.is_empty() {
            return None;
        }
        let desktop = info.downcast_ref::<gio::DesktopAppInfo>();
        let mut name = info.name().to_string();
        if name.is_empty() {
            name = id.strip_suffix(".desktop").unwrap_or(&id).into();
        }
        Some(Self {
            id,
            name,
            icon: icon_string(info.icon()),
            visible: info.should_show(),
            path: desktop.and_then(|v| v.filename()),
            generic: desktop
                .and_then(|v| v.generic_name())
                .map(|s| s.to_string())
                .unwrap_or_default(),
            comment: info
                .description()
                .map(|s| s.to_string())
                .unwrap_or_default(),
            keywords: desktop
                .map(|v| v.keywords().iter().map(ToString::to_string).collect())
                .unwrap_or_default(),
            categories: desktop
                .and_then(|v| v.categories())
                .map(|v| {
                    v.split(';')
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
        })
    }
}

fn build(
    config: &Path,
    lang: &str,
    records: Vec<Record>,
    mut translator: Option<&mut Translator>,
    now: i64,
) -> (Vec<Value>, HashMap<String, Value>) {
    let _lock = SeenLock::new(config).ok();
    let path = config.join("apps-seen.json");
    let raw = fs::read(&path)
        .ok()
        .and_then(|v| serde_json::from_slice::<Value>(&v).ok());
    let first_run = !raw.as_ref().is_some_and(Value::is_object);
    let mut seen = raw.and_then(|v| v.as_object().cloned()).unwrap_or_default();
    seen.retain(|_, value| value.is_i64() || value.is_u64() || value.is_boolean());
    let mut seen_dirty = first_run;
    let mut apps = Vec::new();
    let mut by_id = HashMap::new();
    let mut installed = HashMap::new();
    for record in records {
        // Our own entries are the catalogue's own apps (below), not apps of their own: listed, the visible ones
        // (Explorer, Default apps, the Installer) would come twice in All apps and search. Their Icon= is the own
        // app's icon; a lookup by the entry's id (brief_id: a default handler, Open with) still finds it.
        if let Some(own) = own_of_entry(&record.id) {
            by_id.insert(
                record.id.clone(),
                json!({"id":record.id,"name":record.name,"icon":record.icon,"visible":false}),
            );
            if own_entry(own).as_deref() == Some(record.id.as_str()) || !installed.contains_key(own) {
                installed.insert(own, record);
            }
            continue;
        }
        if !record.visible {
            // A hidden entry can still be a default handler (Open with, Default Apps). F38: in English on a
            // non-English system its GIO name is in the system language; Dutch keeps GIO's name as before.
            let name = match translator.as_deref_mut() {
                Some(translator) if lang != "nl" => {
                    let file = record.path.as_deref().and_then(load_keyfile);
                    english(translator, file.as_ref(), "Name").unwrap_or_else(|| record.name.clone())
                }
                _ => record.name.clone(),
            };
            by_id.insert(
                record.id.clone(),
                json!({"id":record.id,"name":name,"icon":record.icon,"visible":false}),
            );
            continue;
        }
        let mut name = record.name.clone();
        let mut generic = record.generic.clone();
        let mut comment = record.comment.clone();
        let mut keywords = record.keywords.clone();
        if let Some(translator) = translator.as_deref_mut() {
            let file = record.path.as_deref().and_then(load_keyfile);
            if lang == "nl" {
                let translated = translator.get(file.as_ref(), "Name", "");
                name = if !translated.is_empty() {
                    translated
                } else {
                    launcher_names()["NL_NAMES"][&record.id]
                        .as_str()
                        .filter(|s| !s.is_empty())
                        .unwrap_or(&record.name)
                        .into()
                };
                let generic_tr = translator.get(file.as_ref(), "GenericName", "");
                let comment_tr = translator.get(file.as_ref(), "Comment", "");
                generic = generic_tr;
                comment = if !comment_tr.is_empty() {
                    comment_tr
                } else {
                    data()["nlComments"][&record.id]
                        .as_str()
                        .unwrap_or("")
                        .into()
                };
            } else {
                // F38: a translator in English means a non-English system, where GIO's name, generic name and
                // description are in that system's language (Dutch on a Dutch machine). Take the entry's own
                // English texts (english()); GIO's name only when the entry has none, and no generic name or
                // description rather than one in the system language (as the Dutch branch does).
                name = english(translator, file.as_ref(), "Name").unwrap_or_else(|| record.name.clone());
                generic = english(translator, file.as_ref(), "GenericName").unwrap_or_default();
                comment = english(translator, file.as_ref(), "Comment").unwrap_or_default();
            }
            let extra = translator.keywords(file.as_ref());
            keywords = extra
                .iter()
                .cloned()
                .chain(
                    keywords
                        .into_iter()
                        .filter(|keyword| !extra.contains(keyword)),
                )
                .collect();
            for keyword in [&record.name, &record.generic] {
                if !keyword.is_empty() && keyword != &name && !keywords.contains(keyword) {
                    keywords.push(keyword.clone());
                }
            }
        }
        if let Some(twin) = launcher_names()["SAME_NAME_AS_OURS"][&record.id][lang].as_str() {
            for keyword in [&name, &record.name] {
                if !keyword.is_empty() && keyword != twin && !keywords.contains(keyword) {
                    keywords.push(keyword.clone());
                }
            }
            name = twin.into();
        }
        let modified = record
            .path
            .as_deref()
            .and_then(|path| fs::metadata(path).ok())
            .map(|m| ((m.mtime() as f64 + m.mtime_nsec() as f64 * 1e-9) * 1000.0) as i64)
            .unwrap_or(0);
        let added = seen
            .entry(record.id.clone())
            .or_insert_with(|| {
                seen_dirty = true;
                json!(if first_run { 0 } else { now })
            })
            .clone();
        let value = json!({"id":record.id,"name":name,"genericName":generic,"comment":comment,"icon":fallback_icon(record.icon),"categories":record.categories,"keywords":keywords,"noDisplay":false,"own":false,"added":added,"installedAt":added,"modified":modified});
        let mut indexed = value.clone();
        indexed["visible"] = json!(true);
        by_id.insert(record.id, indexed);
        apps.push(value);
    }
    for app in OWN_IDS {
        let value = own_value(app, lang, installed.get(app));
        let mut indexed = value.clone();
        indexed["visible"] = json!(true);
        by_id.insert(value["id"].as_str().unwrap().into(), indexed);
        apps.push(value);
    }
    apps.sort_by(|a, b| {
        compare_names(
            a["name"].as_str().unwrap_or(""),
            b["name"].as_str().unwrap_or(""),
        )
    });
    if seen_dirty {
        let _ = write_seen(&path, &Value::Object(seen));
    }
    (apps, by_id)
}

fn system_language() -> String {
    for language in glib::language_names() {
        let language = language
            .split('.')
            .next()
            .unwrap_or("")
            .split('@')
            .next()
            .unwrap_or("");
        if !language.is_empty() && language != "C" {
            return language.split('_').next().unwrap_or("en").to_lowercase();
        }
    }
    "en".into()
}
fn load_keyfile(path: &Path) -> Option<glib::KeyFile> {
    let file = glib::KeyFile::new();
    file.load_from_file(path, glib::KeyFileFlags::KEEP_TRANSLATIONS)
        .ok()?;
    Some(file)
}

struct Translator {
    language: String,
    roots: Vec<PathBuf>,
    domains: HashMap<String, Option<Vec<HashMap<String, String>>>>,
}
impl Translator {
    fn new(language: &str, roots: Vec<PathBuf>) -> Self {
        Self {
            language: language.into(),
            roots,
            domains: HashMap::new(),
        }
    }
    fn domain(&mut self, name: &str) -> Option<&Vec<HashMap<String, String>>> {
        if !self.domains.contains_key(name) {
            let territory = if self.language == "nl" { "NL" } else { "US" };
            let variants = [
                format!("{}_{territory}.ISO8859-1", self.language),
                format!("{}_{territory}", self.language),
                format!("{}.ISO8859-1", self.language),
                self.language.clone(),
            ];
            let mut found = None;
            for root in &self.roots {
                let files = variants
                    .iter()
                    .map(|language| {
                        root.join(language)
                            .join("LC_MESSAGES")
                            .join(format!("{name}.mo"))
                    })
                    .filter(|path| path.is_file())
                    .collect::<Vec<_>>();
                if files.is_empty() {
                    continue;
                }
                let catalogs = files
                    .iter()
                    .map(|path| fs::read(path).ok().and_then(|bytes| parse_mo(&bytes)))
                    .collect::<Option<Vec<_>>>();
                if catalogs.is_some() {
                    found = catalogs;
                    break;
                }
            }
            self.domains.insert(name.into(), found);
        }
        self.domains.get(name).and_then(Option::as_ref)
    }
    fn get(&mut self, file: Option<&glib::KeyFile>, key: &str, fallback: &str) -> String {
        let Some(file) = file else {
            return fallback.into();
        };
        let Ok(raw) = file.string("Desktop Entry", key) else {
            return fallback.into();
        };
        if let Ok(local) = file.locale_string("Desktop Entry", key, Some(&self.language)) {
            if !local.is_empty() && local != raw {
                return local.into();
            }
        }
        for domain_key in [
            "X-GNOME-Gettext-Domain",
            "X-Ubuntu-Gettext-Domain",
            "X-Gettext-Domain",
        ] {
            let Ok(domain) = file.string("Desktop Entry", domain_key) else {
                continue;
            };
            if domain.is_empty() {
                continue;
            }
            if let Some(translated) = self.domain(&domain).and_then(|catalogs| {
                catalogs
                    .iter()
                    .find_map(|catalog| catalog.get(raw.as_str()))
            }) {
                if !translated.is_empty() && translated != raw.as_str() {
                    return translated.clone();
                }
            }
        }
        fallback.into()
    }
    fn keywords(&self, file: Option<&glib::KeyFile>) -> Vec<String> {
        file.and_then(|file| {
            file.locale_string_list("Desktop Entry", "Keywords", Some(&self.language))
                .ok()
        })
        .map(|values| {
            values
                .iter()
                .filter(|s| !s.is_empty())
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default()
    }
}

fn parse_mo(bytes: &[u8]) -> Option<HashMap<String, String>> {
    let little = match bytes.get(..4)? {
        [0xde, 0x12, 0x04, 0x95] => true,
        [0x95, 0x04, 0x12, 0xde] => false,
        _ => return None,
    };
    let word = |offset: usize| -> Option<usize> {
        let raw: [u8; 4] = bytes.get(offset..offset.checked_add(4)?)?.try_into().ok()?;
        Some(if little {
            u32::from_le_bytes(raw)
        } else {
            u32::from_be_bytes(raw)
        } as usize)
    };
    if word(4)? >> 16 > 1 {
        return None;
    }
    let count = word(8)?;
    let originals = word(12)?;
    let translations = word(16)?;
    if count > bytes.len() / 8 {
        return None;
    }
    let mut charset = "ASCII".to_owned();
    let mut output = HashMap::new();
    for index in 0..count {
        let slice = |table: usize| -> Option<&[u8]> {
            let entry = table.checked_add(index.checked_mul(8)?)?;
            let length = word(entry)?;
            let offset = word(entry + 4)?;
            let end = offset.checked_add(length)?;
            if end >= bytes.len() {
                return None;
            }
            bytes.get(offset..end)
        };
        let original = slice(originals)?;
        let translated = slice(translations)?;
        if original.is_empty() {
            for line in std::str::from_utf8(translated).ok()?.lines() {
                if let Some((key, value)) = line.split_once(':') {
                    if key.trim().eq_ignore_ascii_case("content-type") {
                        if let Some((_, value)) = value.split_once("charset=") {
                            charset = value.trim().into();
                        }
                    }
                }
            }
        }
        let decode = |raw: &[u8]| -> Option<String> {
            let (converted, _) = glib::convert(raw, "UTF-8", &charset).ok()?;
            String::from_utf8(converted.to_vec()).ok()
        };
        let original = original.split(|b| *b == 0).next()?;
        let translated = translated.split(|b| *b == 0).next()?;
        output.insert(decode(original)?, decode(translated)?);
    }
    Some(output)
}

#[derive(Eq, PartialEq)]
enum Part {
    Number(String),
    Text(String),
}
fn natural_key(name: &str) -> Vec<Part> {
    let normalized = glib::normalize(name, glib::NormalizeMode::All);
    let folded = glib::casefold(normalized);
    let mut groups = Vec::<(bool, String)>::new();
    for character in folded.chars().filter(|c| c.combining_class() == 0) {
        let digit = unsafe { glib::ffi::g_unichar_digit_value(character as u32) };
        let numeric = digit >= 0;
        let character = if numeric {
            char::from(b'0' + digit as u8)
        } else {
            character
        };
        if groups.last().is_some_and(|(kind, _)| *kind == numeric) {
            groups.last_mut().unwrap().1.push(character);
        } else {
            groups.push((numeric, character.to_string()));
        }
    }
    groups
        .into_iter()
        .map(|(numeric, value)| {
            if numeric {
                Part::Number(value.trim_start_matches('0').to_owned())
            } else {
                Part::Text(value)
            }
        })
        .collect()
}
pub(super) fn compare_names(first: &str, second: &str) -> Ordering {
    let first = natural_key(first);
    let second = natural_key(second);
    for (first, second) in first.iter().zip(&second) {
        let order = match (first, second) {
            (Part::Number(a), Part::Number(b)) => a.len().cmp(&b.len()).then_with(|| a.cmp(b)),
            (Part::Text(a), Part::Text(b)) => a.cmp(b),
            (Part::Number(_), _) => Ordering::Less,
            (_, Part::Number(_)) => Ordering::Greater,
        };
        if order != Ordering::Equal {
            return order;
        }
    }
    first.len().cmp(&second.len())
}

struct SeenLock(File);
impl SeenLock {
    fn new(config: &Path) -> std::io::Result<Self> {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(config)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(config.join(".apps-seen.lock"))?;
        loop {
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } == 0 {
                return Ok(Self(file));
            }
            let error = std::io::Error::last_os_error();
            if error.kind() != std::io::ErrorKind::Interrupted {
                return Err(error);
            }
        }
    }
}
impl Drop for SeenLock {
    fn drop(&mut self) {
        unsafe {
            libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
        }
    }
}
fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}
fn write_seen(path: &Path, value: &Value) -> std::io::Result<()> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let temporary = path.with_file_name(format!(
        ".apps-seen-{}-{}-{}.tmp",
        std::process::id(),
        now_ms(),
        NEXT.fetch_add(1, AtomicOrdering::Relaxed)
    ));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(&temporary)?;
        let mut bytes = serde_json::to_vec(value)?;
        bytes.push(b'\n');
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "win11-catalog-{}-{}-{}",
                std::process::id(),
                now_ms(),
                NEXT.fetch_add(1, AtomicOrdering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn desktop(&self, name: &str, text: &str) -> PathBuf {
            let path = self.0.join(name);
            fs::write(&path, text).unwrap();
            path
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn record(id: &str, name: &str) -> Record {
        Record {
            id: id.into(),
            name: name.into(),
            icon: String::new(),
            visible: true,
            path: None,
            generic: "Generic English".into(),
            comment: "English description".into(),
            keywords: vec!["original".into()],
            categories: vec!["Utility".into()],
        }
    }
    fn item<'a>(values: &'a [Value], id: &str) -> &'a Value {
        values.iter().find(|value| value["id"] == id).unwrap()
    }

    fn mo(entries: &[(&[u8], &[u8])], little: bool) -> Vec<u8> {
        let count = entries.len();
        let originals = 28;
        let translations = originals + count * 8;
        let mut output = vec![0; translations + count * 8];
        let word = |output: &mut Vec<u8>, offset: usize, value: u32| {
            output[offset..offset + 4].copy_from_slice(&if little {
                value.to_le_bytes()
            } else {
                value.to_be_bytes()
            });
        };
        word(&mut output, 0, 0x950412de);
        word(&mut output, 4, 0);
        word(&mut output, 8, count as u32);
        word(&mut output, 12, originals as u32);
        word(&mut output, 16, translations as u32);
        for (index, (original, translated)) in entries.iter().enumerate() {
            let offset = output.len();
            word(&mut output, originals + index * 8, original.len() as u32);
            word(&mut output, originals + index * 8 + 4, offset as u32);
            output.extend_from_slice(original);
            output.push(0);
            let offset = output.len();
            word(
                &mut output,
                translations + index * 8,
                translated.len() as u32,
            );
            word(&mut output, translations + index * 8 + 4, offset as u32);
            output.extend_from_slice(translated);
            output.push(0);
        }
        output
    }

    #[test]
    fn mo_endianness_charset_and_corrupt_offsets_are_handled_natively() {
        for little in [true, false] {
            let bytes = mo(
                &[
                    (b"", b"Content-Type: text/plain; charset=ISO-8859-1\n"),
                    (b"Coffee", b"Caf\xe9"),
                    (b"file\0files", b"bestand\0bestanden"),
                ],
                little,
            );
            let catalog = parse_mo(&bytes).unwrap();
            assert_eq!(catalog["Coffee"], "Café");
            assert_eq!(catalog["file"], "bestand");
            assert!(parse_mo(&bytes[..bytes.len() - 2]).is_none());
            let mut corrupt = bytes.clone();
            corrupt[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
            assert!(parse_mo(&corrupt).is_none());
        }
        assert!(parse_mo(b"not an mo file").is_none());
        assert!(parse_mo(&mo(&[(b"Name", b"\xff")], true)).is_none());
    }

    #[test]
    fn inline_locale_gettext_domain_order_and_keyword_duplicates_match_source() {
        let fixture = Fixture::new();
        let locale = fixture.0.join("locale/nl/LC_MESSAGES");
        fs::create_dir_all(&locale).unwrap();
        let catalog = mo(
            &[
                (b"", b"Content-Type: text/plain; charset=UTF-8\n"),
                (b"Raw name", b"Gettextnaam"),
                (b"Comment", b"Beschrijving"),
            ],
            true,
        );
        fs::write(locale.join("test.mo"), catalog).unwrap();
        let path=fixture.desktop("local.desktop","[Desktop Entry]\nName=Raw name\nName[nl]=Inline naam\nGenericName=Generic\nComment=Comment\nX-GNOME-Gettext-Domain=missing\nX-Ubuntu-Gettext-Domain=test\nKeywords=raw;original;\nKeywords[nl]=lokaal;lokaal;\n");
        let keyfile = load_keyfile(&path).unwrap();
        let mut tr = Translator::new("nl", vec![fixture.0.join("locale")]);
        assert_eq!(tr.get(Some(&keyfile), "Name", "fallback"), "Inline naam");
        assert_eq!(
            tr.get(Some(&keyfile), "Comment", "fallback"),
            "Beschrijving"
        );
        assert_eq!(
            tr.get(Some(&keyfile), "GenericName", "fallback"),
            "fallback"
        );
        assert_eq!(tr.keywords(Some(&keyfile)), vec!["lokaal", "lokaal"]);
        let mut rec = record("local.desktop", "Raw name");
        rec.path = Some(path);
        rec.keywords = vec!["original".into(), "lokaal".into()];
        let (apps, _) = build(
            &fixture.0.join("config"),
            "nl",
            vec![rec],
            Some(&mut tr),
            123,
        );
        let value = item(&apps, "local.desktop");
        assert_eq!(value["name"], "Inline naam");
        assert_eq!(value["genericName"], "");
        assert_eq!(value["comment"], "Beschrijving");
        assert_eq!(
            value["keywords"],
            json!([
                "lokaal",
                "lokaal",
                "original",
                "Raw name",
                "Generic English"
            ])
        );
    }

    #[test]
    fn gettext_territory_fallback_and_first_directory_precedence_are_preserved() {
        let fixture = Fixture::new();
        for (relative, entries) in [
            (
                "one/nl_NL/LC_MESSAGES/domain.mo",
                vec![(b"Name".as_slice(), b"Territorium".as_slice())],
            ),
            (
                "one/nl/LC_MESSAGES/domain.mo",
                vec![
                    (b"Name".as_slice(), b"Taal".as_slice()),
                    (b"Comment".as_slice(), b"Opmerking".as_slice()),
                ],
            ),
            (
                "two/nl/LC_MESSAGES/domain.mo",
                vec![(b"Name".as_slice(), b"Tweede map".as_slice())],
            ),
        ] {
            let path = fixture.0.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, mo(&entries, true)).unwrap();
        }
        let path = fixture.desktop(
            "app.desktop",
            "[Desktop Entry]\nName=Name\nComment=Comment\nX-Gettext-Domain=domain\n",
        );
        let file = load_keyfile(&path).unwrap();
        let mut tr = Translator::new("nl", vec![fixture.0.join("one"), fixture.0.join("two")]);
        assert_eq!(tr.get(Some(&file), "Name", ""), "Territorium");
        assert_eq!(tr.get(Some(&file), "Comment", ""), "Opmerking");
    }

    #[test]
    fn dutch_fallback_hidden_brief_and_gnome_disks_twin_are_exact() {
        let fixture = Fixture::new();
        let mut tr = Translator::new("nl", vec![]);
        let mut hidden = record("hidden.desktop", "Raw hidden name");
        hidden.visible = false;
        hidden.icon = "hidden-icon".into();
        let (apps, index) = build(
            &fixture.0,
            "nl",
            vec![
                record("org.gnome.Nautilus.desktop", "Files"),
                record("org.gnome.DiskUtility.desktop", "Disks"),
                hidden,
            ],
            Some(&mut tr),
            123,
        );
        assert_eq!(apps.len(), OWN_IDS.len() + 2);
        assert_eq!(
            item(&apps, "org.gnome.Nautilus.desktop")["name"],
            launcher_names()["NL_NAMES"]["org.gnome.Nautilus.desktop"]
        );
        let disks = item(&apps, "org.gnome.DiskUtility.desktop");
        assert_eq!(disks["name"], "Schijven (GNOME)");
        assert!(
            disks["keywords"]
                .as_array()
                .unwrap()
                .contains(&json!("Disks"))
        );
        assert_eq!(
            brief_value(&index["hidden.desktop"]),
            json!({"id":"hidden.desktop","name":"Raw hidden name","icon":"hidden-icon"})
        );
        assert!(!apps.iter().any(|value| value["id"] == "hidden.desktop"));
        for app in OWN_IDS {
            let own = item(&apps, &format!("win11shell-{app}.desktop"));
            assert_eq!(own["own"], true);
            assert_eq!(own["added"], 0);
            assert!(own.get("modified").is_none());
        }
        let (english, _) = build(
            &fixture.0,
            "en",
            vec![record("org.gnome.DiskUtility.desktop", "Disks")],
            None,
            456,
        );
        assert_eq!(
            item(&english, "org.gnome.DiskUtility.desktop")["name"],
            "Disks (GNOME)"
        );
    }

    #[test]
    fn our_installed_entries_give_the_own_apps_their_icon_and_are_not_listed_twice() {
        // scripts/install.py: io.github.win11shell.<Name>.desktop, Explorer, Default apps and the Installer visible,
        // Icon= a file of the bundled Win11 icons; the Calendar is no own app of the catalogue and stays an app
        let fixture = Fixture::new();
        let svg = fixture.desktop("system-file-manager.svg", "<svg xmlns='http://www.w3.org/2000/svg'/>\n");
        let mut explorer = record("io.github.win11shell.Explorer.desktop", "Win11 Verkenner");
        explorer.icon = svg.to_string_lossy().into_owned();
        let mut installer = record("io.github.win11shell.Installer.desktop", "Win11 App-installatie");
        installer.icon = fixture.0.join("missing.svg").to_string_lossy().into_owned();
        let mut taskmgr = record("io.github.win11shell.TaskManager.desktop", "Win11 Taakbeheer");
        taskmgr.visible = false;
        let mut panel = record("io.github.win11shell.Panel.desktop", "Win11 Taakbalk");
        panel.visible = false;
        let (apps, index) = build(
            &fixture.0,
            "nl",
            vec![
                explorer,
                installer,
                taskmgr,
                panel,
                record("io.github.win11shell.Calendar.desktop", "Agenda"),
                record("org.gnome.Nautilus.desktop", "Files"),
            ],
            None,
            123,
        );
        let ids: Vec<_> = apps.iter().filter_map(|a| a["id"].as_str()).collect();
        assert!(
            !ids.iter().any(|id| id.starts_with("io.github.win11shell.") && *id != "io.github.win11shell.Calendar.desktop"),
            "{ids:?}"
        );
        assert!(ids.contains(&"io.github.win11shell.Calendar.desktop"));
        assert_eq!(apps.len(), OWN_IDS.len() + 2);
        let own = item(&apps, "win11shell-explorer.desktop");
        assert_eq!(own["icon"], json!(svg.to_string_lossy()), "the installed entry's icon");
        assert_eq!(own["entry"], "io.github.win11shell.Explorer.desktop");
        assert_eq!(own["name"], "Verkenner");
        // an icon that does not resolve: the catalogue's own list
        assert_eq!(item(&apps, "win11shell-installer.desktop")["icon"], json!(own_icon("installer")));
        assert_eq!(item(&apps, "win11shell-alles.desktop")["entry"], "io.github.win11shell.Search.desktop");
        for app in OWN_IDS {
            assert_eq!(item(&apps, &format!("win11shell-{app}.desktop"))["entry"], json!(own_entry(app)), "{app}");
        }
        // looked up by its own id (a default handler, Open with) the entry is still there
        assert_eq!(
            brief_value(&index["io.github.win11shell.Explorer.desktop"]),
            json!({"id":"io.github.win11shell.Explorer.desktop","name":"Win11 Verkenner","icon":svg.to_string_lossy()})
        );
        assert_eq!(index["io.github.win11shell.Panel.desktop"]["visible"], false);
    }

    #[test]
    fn first_seen_registry_retains_original_types_and_marks_only_new_visible_apps() {
        let fixture = Fixture::new();
        let (first, _) = build(
            &fixture.0,
            "en",
            vec![record("old.desktop", "Old")],
            None,
            123,
        );
        assert_eq!(item(&first, "old.desktop")["added"], 0);
        let mut hidden = record("hidden.desktop", "Hidden");
        hidden.visible = false;
        let (second, _) = build(
            &fixture.0,
            "en",
            vec![
                record("old.desktop", "Old"),
                record("new.desktop", "New"),
                hidden,
            ],
            None,
            456,
        );
        assert_eq!(item(&second, "new.desktop")["added"], 456);
        assert_eq!(item(&second, "old.desktop")["installedAt"], 0);
        let seen: Value =
            serde_json::from_slice(&fs::read(fixture.0.join("apps-seen.json")).unwrap()).unwrap();
        assert!(seen.get("hidden.desktop").is_none());
        assert_eq!(seen.as_object().unwrap().len(), 2);
        fs::write(
            fixture.0.join("apps-seen.json"),
            b"{\"old.desktop\":true,\"bad\":2.5}",
        )
        .unwrap();
        let (third, _) = build(
            &fixture.0,
            "en",
            vec![record("old.desktop", "Old")],
            None,
            789,
        );
        assert_eq!(item(&third, "old.desktop")["added"], true);
        assert_eq!(
            fs::read_to_string(fixture.0.join("apps-seen.json")).unwrap(),
            "{\"old.desktop\":true,\"bad\":2.5}"
        );
    }

    #[test]
    fn concurrent_catalog_builds_merge_seen_entries_without_lost_updates() {
        let fixture = Fixture::new();
        fs::write(fixture.0.join("apps-seen.json"), "{}").unwrap();
        let mut threads = Vec::new();
        for index in 0..8 {
            let config = fixture.0.clone();
            threads.push(std::thread::spawn(move || {
                build(
                    &config,
                    "en",
                    vec![record(&format!("app-{index}.desktop"), "App")],
                    None,
                    1000 + index,
                );
            }));
        }
        for thread in threads {
            thread.join().unwrap();
        }
        let seen: Value =
            serde_json::from_slice(&fs::read(fixture.0.join("apps-seen.json")).unwrap()).unwrap();
        assert_eq!(seen.as_object().unwrap().len(), 8);
        for index in 0..8 {
            assert_eq!(seen[format!("app-{index}.desktop")], 1000 + index);
        }
        assert_eq!(
            fs::metadata(fixture.0.join("apps-seen.json"))
                .unwrap()
                .mode()
                & 0o777,
            0o600
        );
    }

    #[test]
    fn natural_order_is_stable_accent_insensitive_and_numeric_beyond_u64() {
        assert_eq!(compare_names("École ٢", "ecole 2"), Ordering::Equal);
        assert_eq!(compare_names("file 0002", "file 10"), Ordering::Less);
        assert_eq!(
            compare_names(
                "App 999999999999999999999999",
                "App 1000000000000000000000000"
            ),
            Ordering::Less
        );
        assert_eq!(compare_names("Straße", "STRASSE"), Ordering::Equal);
        let mut names = ["File 10", "file 2", "Fíle 02", "2 files", "File"];
        names.sort_by(|first, second| compare_names(first, second));
        assert_eq!(names, ["2 files", "File", "file 2", "Fíle 02", "File 10"]);
    }

    // F38 fixtures: three entries with English and Dutch texts, and the records GIO makes of them in the system
    // locale. `ensure()` passes a translator exactly when Win11's language differs from the system's.
    struct Locales {
        fixture: Fixture,
        files: PathBuf,
        picker: PathBuf,
        hidden: PathBuf,
    }
    impl Locales {
        fn new() -> Self {
            let fixture = Fixture::new();
            let files = fixture.desktop(
                "org.example.Files.desktop",
                "[Desktop Entry]\nType=Application\nName=Files\nName[nl]=Bestanden\n\
                 GenericName=File Manager\nGenericName[nl]=Bestandsbeheerder\n\
                 Comment=Access and organise files\nComment[nl]=Bestanden openen en ordenen\n\
                 Keywords=folder;\nKeywords[nl]=map;\nExec=true\n",
            );
            let picker = fixture.desktop(
                "org.example.Picker.desktop",
                "[Desktop Entry]\nType=Application\nName=Color Picker\nName[en_GB]=Colour Picker\n\
                 Name[nl]=Kleurkiezer\nExec=true\n",
            );
            let hidden = fixture.desktop(
                "org.example.Handler.desktop",
                "[Desktop Entry]\nType=Application\nName=Link Handler\nName[nl]=Koppelingafhandelaar\n\
                 NoDisplay=true\nExec=true\n",
            );
            Self {
                fixture,
                files,
                picker,
                hidden,
            }
        }
        /// The records as GIO reports them on a system whose locale is `system` ("nl" or "en").
        fn records(&self, system: &str) -> Vec<Record> {
            let nl = system == "nl";
            let pick = |nl_text: &str, en_text: &str| if nl { nl_text } else { en_text }.to_owned();
            let mut files = record("org.example.Files.desktop", &pick("Bestanden", "Files"));
            files.path = Some(self.files.clone());
            files.generic = pick("Bestandsbeheerder", "File Manager");
            files.comment = pick("Bestanden openen en ordenen", "Access and organise files");
            files.keywords = vec![pick("map", "folder")];
            let mut picker = record("org.example.Picker.desktop", &pick("Kleurkiezer", "Color Picker"));
            picker.path = Some(self.picker.clone());
            let mut hidden = record(
                "org.example.Handler.desktop",
                &pick("Koppelingafhandelaar", "Link Handler"),
            );
            hidden.path = Some(self.hidden.clone());
            hidden.visible = false;
            // no desktop file to read: GIO's name is all there is
            let bare = record("org.example.Bare.desktop", &pick("Kaal", "Bare"));
            vec![files, picker, hidden, bare]
        }
        fn build(&self, lang: &str, system: &str) -> (Vec<Value>, HashMap<String, Value>) {
            let mut translator = Translator::new(lang, vec![]);
            build(
                &self.fixture.0.join(format!("config-{lang}-{system}")),
                lang,
                self.records(system),
                (lang != system).then_some(&mut translator),
                123,
            )
        }
    }

    #[test]
    fn english_on_a_dutch_system_takes_each_entrys_english_texts_not_gios_dutch_ones() {
        let locales = Locales::new();
        let (apps, index) = locales.build("en", "nl");
        let files = item(&apps, "org.example.Files.desktop");
        assert_eq!(files["name"], "Files");
        assert_eq!(files["genericName"], "File Manager");
        assert_eq!(files["comment"], "Access and organise files");
        // GIO's Dutch name stays findable
        assert!(files["keywords"].as_array().unwrap().contains(&json!("Bestanden")));
        // Name[en_GB] before the untranslated Name=
        assert_eq!(item(&apps, "org.example.Picker.desktop")["name"], "Colour Picker");
        // a hidden handler (Open with, Default Apps) too
        assert_eq!(
            brief_value(&index["org.example.Handler.desktop"])["name"],
            "Link Handler"
        );
        // no desktop file: GIO's name is kept
        assert_eq!(item(&apps, "org.example.Bare.desktop")["name"], "Kaal");
    }

    #[test]
    fn english_texts_follow_en_gb_then_en_then_the_untranslated_key_and_never_the_system_language() {
        // review m5: an empty value is skipped; a generic name or description that the entry has only in the
        // system language (no untranslated key) is left out in English instead of showing GIO's Dutch text
        let fixture = Fixture::new();
        let entries = [
            (
                "org.example.Paint.desktop",
                "[Desktop Entry]\nType=Application\nName=Paint\nName[en]=Drawing\nName[en_GB]=Colour Drawing\n\
                 Name[nl]=Tekenen\nExec=true\n",
                ["Tekenen", "", ""],
            ),
            (
                "org.example.Notes.desktop",
                "[Desktop Entry]\nType=Application\nName=Notes\nName[en_GB]=\nName[en]=Notebook\n\
                 Name[nl]=Notities\nGenericName[nl]=Notitieblok\nComment[nl]=Notities maken\nExec=true\n",
                ["Notities", "Notitieblok", "Notities maken"],
            ),
            (
                "org.example.Clock.desktop",
                "[Desktop Entry]\nType=Application\nName=Clock\nName[en_GB]=\nName[en]=\nName[nl]=Klok\n\
                 GenericName=World Clock\nGenericName[en_GB]=\nGenericName[nl]=Wereldklok\nExec=true\n",
                ["Klok", "Wereldklok", ""],
            ),
        ];
        // the records as GIO reports them on a Dutch system (the Dutch catalogue below reads only the entries)
        let records = || {
            entries
                .iter()
                .map(|(id, text, [name, generic, comment])| {
                    let mut item = record(id, name);
                    item.path = Some(fixture.desktop(id, text));
                    item.generic = (*generic).into();
                    item.comment = (*comment).into();
                    item
                })
                .collect::<Vec<_>>()
        };
        let mut translator = Translator::new("en", vec![]);
        let (apps, _) = build(&fixture.0.join("en"), "en", records(), Some(&mut translator), 1);
        let paint = item(&apps, "org.example.Paint.desktop");
        assert_eq!(paint["name"], "Colour Drawing", "Name[en_GB] before Name[en]");
        let notes = item(&apps, "org.example.Notes.desktop");
        assert_eq!(notes["name"], "Notebook", "an empty Name[en_GB] falls back to Name[en]");
        assert_eq!(notes["genericName"], "", "GenericName only in Dutch");
        assert_eq!(notes["comment"], "", "Comment only in Dutch");
        let clock = item(&apps, "org.example.Clock.desktop");
        assert_eq!(clock["name"], "Clock", "empty Name[en_GB] and Name[en] fall back to Name=");
        assert_eq!(clock["genericName"], "World Clock");
        // Dutch on an English system: the entries' Dutch names, as before
        let mut translator = Translator::new("nl", vec![]);
        let (apps, _) = build(&fixture.0.join("nl"), "nl", records(), Some(&mut translator), 1);
        for (id, name) in [
            ("org.example.Paint.desktop", "Tekenen"),
            ("org.example.Notes.desktop", "Notities"),
            ("org.example.Clock.desktop", "Klok"),
        ] {
            assert_eq!(item(&apps, id)["name"], name);
        }
        assert_eq!(item(&apps, "org.example.Clock.desktop")["genericName"], "Wereldklok");
    }

    #[test]
    fn dutch_and_same_language_catalogues_keep_todays_names() {
        let locales = Locales::new();
        // Dutch on this English machine (en_US): the entry's Dutch texts, a hidden entry keeps GIO's name
        let (apps, index) = locales.build("nl", "en");
        let files = item(&apps, "org.example.Files.desktop");
        assert_eq!(files["name"], "Bestanden");
        assert_eq!(files["genericName"], "Bestandsbeheerder");
        assert_eq!(files["comment"], "Bestanden openen en ordenen");
        assert_eq!(item(&apps, "org.example.Picker.desktop")["name"], "Kleurkiezer");
        assert_eq!(
            brief_value(&index["org.example.Handler.desktop"])["name"],
            "Link Handler"
        );
        assert_eq!(item(&apps, "org.example.Bare.desktop")["name"], "Bare");
        // Dutch on a Dutch machine and English on an English one: no translator, GIO's own texts
        for (lang, names) in [
            ("nl", ["Bestanden", "Kleurkiezer", "Koppelingafhandelaar", "Kaal"]),
            ("en", ["Files", "Color Picker", "Link Handler", "Bare"]),
        ] {
            let (apps, index) = locales.build(lang, lang);
            assert_eq!(item(&apps, "org.example.Files.desktop")["name"], names[0]);
            assert_eq!(item(&apps, "org.example.Picker.desktop")["name"], names[1]);
            assert_eq!(
                brief_value(&index["org.example.Handler.desktop"])["name"],
                names[2]
            );
            assert_eq!(item(&apps, "org.example.Bare.desktop")["name"], names[3]);
        }
    }

    #[test]
    fn gnome_names_come_from_verkenners_launcher_table() {
        // plan-critique H: one table for Verkenner's launchers and the app catalogue (files/launcher-names.json)
        assert_eq!(
            launcher_names()["NL_NAMES"]["org.gnome.Nautilus.desktop"],
            "Bestanden"
        );
        assert_eq!(
            launcher_names()["NL_NAMES"].as_object().unwrap().len(),
            57
        );
        assert_eq!(
            launcher_names()["SAME_NAME_AS_OURS"]["org.gnome.DiskUtility.desktop"],
            json!({"nl": "Schijven (GNOME)", "en": "Disks (GNOME)"})
        );
        assert!(data().get("nlNames").is_none() && data().get("twins").is_none());
        let fixture = Fixture::new();
        let mut tr = Translator::new("nl", vec![]);
        let (apps, _) = build(
            &fixture.0,
            "nl",
            vec![record("org.gnome.Calculator.desktop", "Calculator")],
            Some(&mut tr),
            1,
        );
        assert_eq!(item(&apps, "org.gnome.Calculator.desktop")["name"], "Rekenmachine");
    }
}
