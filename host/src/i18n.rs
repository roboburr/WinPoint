//! Host-side language runtime (notes/claude/contracts.md C1-C3; how to use it: notes/claude/round2-gate.md).
//!
//! ```text
//! use crate::i18n::{self, Lang};
//! let lang = ctx.lang();                                       // services: the ServiceContext's settings file
//! let lang = i18n::current_language();                          // anywhere else (picker, gnome, launch, …)
//! Err(ApiError::tr("not_found", lang, "files.gone", &json!({"name": name})))
//! i18n::tr(lang, "core.badArgs")                                // plain text
//! i18n::trf(lang, "files.gone", &json!({"name": name}))          // {name} placeholders
//! i18n::trn(lang, "files.trashFailed", n, &json!({}))           // .one / .other (/ .zero), {n} filled with n
//! i18n::decimal(lang, "1.5")  i18n::number(lang, 1.5, 1)         // "1,5" in nl, "1.5" in en
//! i18n::join(lang, &["A", "B", "C"])                            // "A, B en C" / "A, B and C"
//! ```
//!
//! * **Language** (C1): only Win11's own `settings.language`. `"en"` is English, anything else (no file, no key,
//!   another string, the object form) is Dutch. [`current_language`] and [`language_in`] re-parse settings.json
//!   only when its (device, inode, size, mtime, ctime) changed, so a call costs one `stat`.
//! * **Catalogues** (C3): `host/i18n/<stem>.tsv`, `key<TAB>nl<TAB>en`, escapes `\n`, `\t`, `\\`, `{name}`
//!   placeholders, plurals `key.one` / `key.other` (optional `key.zero`), the first key segment is the stem. All
//!   of them are embedded ([`STEMS`]), so there is no missing-file path at run time. Lines starting with `#` are
//!   comments; `#@` comment lines record which 3b898e6 literal a row replaced (tools/i18n/extract_rs.py writes
//!   them, tools/i18n/drift_rust.py reads them).
//! * **Fallback**: the language's text → nl → the key itself; each miss is logged once per process.
//! * **Never** call these from a `static`/`const`/`OnceLock` initialiser or cache their result across calls without
//!   the language in the cache key: the text would freeze in the language of that moment.
// The API is frozen for round 2 (round2-gate.md); parts of it get their first callers there.
#![allow(dead_code)]
use serde_json::Value;
use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
    fs,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

/// Every catalogue, one per stem. Round 2 fills the files; nobody edits this table (round2-gate.md).
pub(crate) const STEMS: &[(&str, &str)] = &[
    ("core", include_str!("../i18n/core.tsv")),
    ("window", include_str!("../i18n/window.tsv")),
    ("update", include_str!("../i18n/update.tsv")),
    ("system", include_str!("../i18n/system.tsv")),
    ("display", include_str!("../i18n/display.tsv")),
    ("media", include_str!("../i18n/media.tsv")),
    ("notifications", include_str!("../i18n/notifications.tsv")),
    ("shell", include_str!("../i18n/shell.tsv")),
    ("clipboard", include_str!("../i18n/clipboard.tsv")),
    ("filemanager1", include_str!("../i18n/filemanager1.tsv")),
    ("tray", include_str!("../i18n/tray.tsv")),
    ("settings", include_str!("../i18n/settings.tsv")),
    ("sysmon", include_str!("../i18n/sysmon.tsv")),
    ("storage", include_str!("../i18n/storage.tsv")),
    ("disks", include_str!("../i18n/disks.tsv")),
    ("index", include_str!("../i18n/index.tsv")),
    ("files", include_str!("../i18n/files.tsv")),
    ("files_launch", include_str!("../i18n/files_launch.tsv")),
    ("apps", include_str!("../i18n/apps.tsv")),
    ("picker", include_str!("../i18n/picker.tsv")),
    ("transfers", include_str!("../i18n/transfers.tsv")),
    ("launch", include_str!("../i18n/launch.tsv")),
    ("gnome", include_str!("../i18n/gnome.tsv")),
    ("wallpaper", include_str!("../i18n/wallpaper.tsv")),
];

// ------------------------------------------------------------------------------------------------ language

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Lang {
    Nl,
    En,
}

impl Lang {
    pub(crate) const ALL: [Lang; 2] = [Lang::Nl, Lang::En];

    /// `"nl"` or `"en"`, the value `settings.language` holds.
    pub(crate) fn code(self) -> &'static str {
        match self {
            Lang::Nl => "nl",
            Lang::En => "en",
        }
    }

    /// C1: exactly `"en"` is English; everything else is Dutch.
    pub(crate) fn parse(value: &str) -> Lang {
        if value == "en" { Lang::En } else { Lang::Nl }
    }

    /// The language of a `settings.language` JSON value (a string; anything else is Dutch).
    pub(crate) fn of(value: &Value) -> Lang {
        value.as_str().map(Lang::parse).unwrap_or(Lang::Nl)
    }

    /// The language of a whole settings document.
    pub(crate) fn of_settings(settings: &Value) -> Lang {
        Lang::of(&settings["language"])
    }

    pub(crate) fn from_u8(value: u8) -> Lang {
        if value == 1 { Lang::En } else { Lang::Nl }
    }

    pub(crate) fn as_u8(self) -> u8 {
        match self {
            Lang::Nl => 0,
            Lang::En => 1,
        }
    }
}

/// Anything that names a language: [`Lang`], or the `"nl"`/`"en"` strings existing code passes around
/// (`ctx.language()`, `files::Context::language`, the `lang: &str` parameters of the services).
pub(crate) trait AsLang {
    fn lang(&self) -> Lang;
}
impl AsLang for Lang {
    fn lang(&self) -> Lang {
        *self
    }
}
impl AsLang for str {
    fn lang(&self) -> Lang {
        Lang::parse(self)
    }
}
impl AsLang for String {
    fn lang(&self) -> Lang {
        Lang::parse(self)
    }
}
impl<T: AsLang + ?Sized> AsLang for &T {
    fn lang(&self) -> Lang {
        (**self).lang()
    }
}

/// The directory of Win11's settings.json, by the same rule as `runtime.rs` and `settings_runtime.rs`:
/// `$WIN11_GNOME_CONFIG_DIR`, else `$XDG_CONFIG_HOME/win11-gnome` (GLib's user config dir).
pub(crate) fn config_dir() -> PathBuf {
    std::env::var_os("WIN11_GNOME_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| glib::user_config_dir().join("win11-gnome"))
}

/// The language of this process's settings file. For code without a `ServiceContext` (C1).
pub(crate) fn current_language() -> Lang {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    language_in(DIR.get_or_init(config_dir))
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Stamp {
    dev: u64,
    ino: u64,
    size: u64,
    mtime: (i64, i64),
    ctime: (i64, i64),
}
impl Stamp {
    fn of(meta: &fs::Metadata) -> Self {
        Self {
            dev: meta.dev(),
            ino: meta.ino(),
            size: meta.size(),
            mtime: (meta.mtime(), meta.mtime_nsec()),
            ctime: (meta.ctime(), meta.ctime_nsec()),
        }
    }
}
struct Cached {
    stamp: Stamp,
    lang: Lang,
    // A file changed within RACY of our read may still be rewritten inside the same timestamp tick (coarse
    // file-system clocks): such an entry is not trusted and the next call reads the file again.
    trusted: bool,
}
const RACY: Duration = Duration::from_millis(50);

/// Whether a parse of a file with `stamp`, read at `now`, may be reused while the stamp stays the same. The test
/// is on ctime, the time of the file's last change: the kernel sets it on every write, rename and timestamp
/// change, and no writer can choose it (an mtime can be anything: a restored backup, `touch -d`, clock skew).
/// A change that lies RACY or more before `now` is safe, and so is one RACY or more after it (the clock went
/// back): any later change gets a ctime near the clock, which differs from the stamp.
fn trusted_at(stamp: &Stamp, now: SystemTime) -> bool {
    let (seconds, nanos) = stamp.ctime;
    let changed = if seconds >= 0 {
        UNIX_EPOCH + Duration::new(seconds as u64, nanos.clamp(0, 999_999_999) as u32)
    } else {
        UNIX_EPOCH
    };
    match now.duration_since(changed) {
        Ok(age) => age >= RACY,
        Err(ahead) => ahead.duration() >= RACY,
    }
}
static LANGUAGE_CACHE: Mutex<Option<HashMap<PathBuf, Cached>>> = Mutex::new(None);
static PARSES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The language of `<config_dir>/settings.json`, cached by the file's identity (C1): one `stat` per call, a full
/// parse only after the file changed. A missing or unreadable file is Dutch.
pub(crate) fn language_in(config_dir: &Path) -> Lang {
    let path = config_dir.join("settings.json");
    let Ok(meta) = fs::metadata(&path) else {
        return Lang::Nl;
    };
    let stamp = Stamp::of(&meta);
    {
        let cache = LANGUAGE_CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(hit) = cache.as_ref().and_then(|c| c.get(&path))
            && hit.stamp == stamp
            && hit.trusted
        {
            return hit.lang;
        }
    }
    PARSES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let lang = fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .map(|settings| Lang::of_settings(&settings))
        .unwrap_or(Lang::Nl);
    let trusted = trusted_at(&stamp, SystemTime::now());
    let mut cache = LANGUAGE_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let map = cache.get_or_insert_with(HashMap::new);
    if map.len() >= 64 && !map.contains_key(&path) {
        // Tests create a settings directory each; a process has one or two.
        map.clear();
    }
    map.insert(
        path,
        Cached {
            stamp,
            lang,
            trusted,
        },
    );
    lang
}

/// How often this process parsed a settings file for its language (tests and the round-2 performance check).
pub(crate) fn language_parses() -> u64 {
    PARSES.load(std::sync::atomic::Ordering::Relaxed)
}

// ------------------------------------------------------------------------------------------------ catalogues

struct Row {
    nl: Cow<'static, str>,
    en: Option<Cow<'static, str>>,
}

fn unescape(field: &'static str) -> Cow<'static, str> {
    if !field.contains('\\') {
        return Cow::Borrowed(field);
    }
    let mut out = String::with_capacity(field.len());
    let mut chars = field.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('\\') => out.push('\\'),
            // tools/i18n/check_rust.py rejects every other escape; at run time it stays as written.
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    Cow::Owned(out)
}

/// The lenient run-time parse of one catalogue: rows, and the problems tools/i18n/check_rust.py would report.
fn parse(stem: &str, text: &'static str) -> (Vec<(&'static str, Row)>, Vec<String>) {
    let mut rows = Vec::new();
    let mut problems = Vec::new();
    for (number, line) in text.split('\n').enumerate() {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&'static str> = line.split('\t').collect();
        if fields.len() != 3 {
            problems.push(format!(
                "{stem}.tsv:{}: expected key<TAB>nl<TAB>en, found {} fields",
                number + 1,
                fields.len()
            ));
        }
        let key = fields[0];
        let Some(nl) = fields.get(1).filter(|nl| !nl.is_empty()) else {
            problems.push(format!("{stem}.tsv:{}: no Dutch text", number + 1));
            continue;
        };
        if key.split('.').next() != Some(stem) || !key.contains('.') {
            problems.push(format!(
                "{stem}.tsv:{}: key {key} does not start with {stem}.",
                number + 1
            ));
            continue;
        }
        let en = fields
            .get(2)
            .filter(|en| !en.is_empty())
            .map(|en| unescape(en));
        rows.push((
            key,
            Row {
                nl: unescape(nl),
                en,
            },
        ));
    }
    (rows, problems)
}

fn catalogue() -> &'static HashMap<&'static str, Row> {
    static CATALOGUE: OnceLock<HashMap<&'static str, Row>> = OnceLock::new();
    CATALOGUE.get_or_init(|| {
        let mut all = HashMap::new();
        for (stem, text) in STEMS {
            let (rows, problems) = parse(stem, text);
            for problem in problems {
                log_once(format!("row:{problem}"), || {
                    format!("i18n: ongeldige regel (invalid row) {problem}")
                });
            }
            for (key, row) in rows {
                if all.insert(key, row).is_some() {
                    log_once(format!("dup:{key}"), || {
                        format!("i18n: dubbele sleutel (duplicate key) {key}")
                    });
                }
            }
        }
        all
    })
}

fn log_once(tag: String, message: impl FnOnce() -> String) {
    static SEEN: Mutex<Option<HashSet<String>>> = Mutex::new(None);
    let fresh = SEEN
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_insert_with(HashSet::new)
        .insert(tag);
    if fresh {
        eprintln!("{}", message());
    }
}

/// Whether the catalogues hold `key` (a plain row; for a plural, ask for `key.other`).
pub(crate) fn has(key: &str) -> bool {
    catalogue().contains_key(key)
}

fn text(lang: Lang, key: &str) -> Option<&'static str> {
    let row = catalogue().get(key)?;
    if lang == Lang::En {
        if let Some(en) = &row.en {
            return Some(en.as_ref());
        }
        log_once(format!("en:{key}"), || {
            format!("i18n: geen Engelse tekst (no English text) voor {key}")
        });
    }
    Some(row.nl.as_ref())
}

fn missing(key: &str) -> String {
    log_once(format!("key:{key}"), || {
        format!("i18n: sleutel ontbreekt (missing key) {key}")
    });
    key.to_owned()
}

/// A parameter as text: strings as they are, numbers as `format!("{}")` prints them, booleans as true/false.
fn param_text(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(if let Some(i) = n.as_i64() {
            i.to_string()
        } else if let Some(u) = n.as_u64() {
            u.to_string()
        } else {
            n.as_f64()
                .map(|f| f.to_string())
                .unwrap_or_else(|| n.to_string())
        }),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// Fills `{name}` (ASCII letters, digits, `_`) from `params`; a placeholder without a value stays as written,
/// like the UI's `fill()` (no `str.format`-style escapes: a lone brace is just a brace).
pub(crate) fn fill(text: &str, params: &Value) -> String {
    if !text.contains('{') || !params.is_object() {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len() + 16);
    let mut rest = text;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let name_len = after
            .bytes()
            .take_while(|b| b.is_ascii_alphanumeric() || *b == b'_')
            .count();
        if name_len > 0 && after.as_bytes().get(name_len) == Some(&b'}') {
            let name = &after[..name_len];
            match params.get(name).and_then(param_text) {
                Some(value) => out.push_str(&value),
                None => {
                    out.push('{');
                    out.push_str(name);
                    out.push('}');
                }
            }
            rest = &after[name_len + 1..];
        } else {
            out.push('{');
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

/// "No parameters" for [`trf`] and `ApiError::tr`.
pub(crate) const NO_PARAMS: &Value = &Value::Null;

/// The text of `key` in `lang` (→ nl → the key).
pub(crate) fn tr(lang: impl AsLang, key: &str) -> String {
    match text(lang.lang(), key) {
        Some(text) => text.to_owned(),
        None => missing(key),
    }
}

/// The text of `key` in `lang` with its `{name}` placeholders filled from the object `params`
/// (`&json!({"name": name})`; strings or numbers).
pub(crate) fn trf(lang: impl AsLang, key: &str, params: &Value) -> String {
    match text(lang.lang(), key) {
        Some(text) => fill(text, params),
        None => fill(&missing(key), params),
    }
}

/// A count for [`trn`]: its value picks the form, its text fills `{n}`.
pub(crate) trait Count {
    fn count_value(&self) -> f64;
    fn count_text(&self) -> String;
}
macro_rules! count_int {
    ($($t:ty),*) => {$(
        impl Count for $t {
            fn count_value(&self) -> f64 { *self as f64 }
            fn count_text(&self) -> String { self.to_string() }
        }
    )*};
}
count_int!(u8, u16, u32, u64, usize, i8, i16, i32, i64, isize);
impl Count for f64 {
    fn count_value(&self) -> f64 {
        *self
    }
    fn count_text(&self) -> String {
        self.to_string()
    }
}
impl Count for f32 {
    fn count_value(&self) -> f64 {
        *self as f64
    }
    fn count_text(&self) -> String {
        self.to_string()
    }
}
impl<T: Count + ?Sized> Count for &T {
    fn count_value(&self) -> f64 {
        (**self).count_value()
    }
    fn count_text(&self) -> String {
        (**self).count_text()
    }
}

/// `params` plus `n` (unless `params` already has an `n`, e.g. a formatted "1.000").
pub(crate) fn count_params(n: &impl Count, params: &Value) -> Value {
    let mut all = match params {
        Value::Object(map) => Value::Object(map.clone()),
        _ => Value::Object(Default::default()),
    };
    if all.get("n").is_none() {
        all["n"] = Value::String(n.count_text());
    }
    all
}

/// The plural form to use for a count of `value`: `zero` (only when n == 0 and that row exists), `one` when
/// n == 1, else `other`; a missing form falls back to the other one, then to a plain `key` row.
fn plural<'a>(lookup: impl Fn(&str) -> Option<&'a str>, key: &str, value: f64) -> Option<&'a str> {
    let mut forms: Vec<&str> = Vec::with_capacity(3);
    if value == 0.0 {
        forms.push("zero");
    }
    if value == 1.0 {
        forms.extend(["one", "other"]);
    } else {
        forms.extend(["other", "one"]);
    }
    forms
        .into_iter()
        .find_map(|form| lookup(&format!("{key}.{form}")))
        .or_else(|| lookup(key))
}

/// Plural: `key.zero` when n == 0 and that row exists, `key.one` when n == 1, else `key.other` (then the other
/// form, then a plain `key` row, when the chosen one is missing). `{n}` is filled with n unless `params` has one.
pub(crate) fn trn(lang: impl AsLang, key: &str, n: impl Count, params: &Value) -> String {
    let lang = lang.lang();
    let params = count_params(&n, params);
    match plural(|k| text(lang, k), key, n.count_value()) {
        Some(text) => fill(text, &params),
        None => fill(&missing(&format!("{key}.other")), &params),
    }
}

/// The half of an inline `{nl, en}` pair for `lang` (bilingual data tables that stay inline, round2-gate.md).
pub(crate) fn pick<'a>(lang: impl AsLang, nl: &'a str, en: &'a str) -> &'a str {
    if lang.lang() == Lang::En && !en.is_empty() {
        en
    } else {
        nl
    }
}

// ------------------------------------------------------------------------------------------------ formatting

/// The language's decimal separator in an already formatted number: "1.5" → nl "1,5" / en "1.5". Each caller
/// keeps its own rounding and units; no digit grouping.
pub(crate) fn decimal(lang: impl AsLang, formatted: &str) -> String {
    if lang.lang() == Lang::En {
        formatted.to_owned()
    } else {
        formatted.replace('.', ",")
    }
}

/// `format!("{x:.digits$}")` with the language's decimal separator.
pub(crate) fn number(lang: impl AsLang, x: f64, digits: usize) -> String {
    decimal(lang, &format!("{x:.digits$}"))
}

fn join_with(items: &[impl AsRef<str>], last: &str) -> String {
    match items {
        [] => String::new(),
        [one] => one.as_ref().to_owned(),
        [init @ .., tail] => {
            let mut out = init
                .iter()
                .map(|s| s.as_ref())
                .collect::<Vec<_>>()
                .join(", ");
            out.push_str(last);
            out.push_str(tail.as_ref());
            out
        }
    }
}

/// "A, B en C" / "A, B and C" (no serial comma, as `Intl.ListFormat` nl and en-GB).
pub(crate) fn join(lang: impl AsLang, items: &[impl AsRef<str>]) -> String {
    join_with(
        items,
        if lang.lang() == Lang::En {
            " and "
        } else {
            " en "
        },
    )
}

/// "A, B of C" / "A, B or C".
pub(crate) fn join_or(lang: impl AsLang, items: &[impl AsRef<str>]) -> String {
    join_with(
        items,
        if lang.lang() == Lang::En {
            " or "
        } else {
            " of "
        },
    )
}

// ------------------------------------------------------------------------------------------------ tests

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::BTreeSet;

    fn placeholders(text: &str) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        let mut rest = text;
        while let Some(open) = rest.find('{') {
            let after = &rest[open + 1..];
            let n = after
                .bytes()
                .take_while(|b| b.is_ascii_alphanumeric() || *b == b'_')
                .count();
            if n > 0 && after.as_bytes().get(n) == Some(&b'}') {
                out.insert(after[..n].to_owned());
            }
            rest = after;
        }
        out
    }

    // No crate::events here: a binary crate may include this file by #[path] (round2-gate.md).
    fn temp_dir(tag: &str) -> PathBuf {
        static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let unique = format!(
            "{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
            SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        );
        let dir = std::env::temp_dir().join(format!("win11-i18n-{tag}-{unique}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// tests/i18n/rust-frozen-all.tsv as {(path, ordinal): text}; None outside the repository (source tarball).
    fn frozen_literals() -> Option<HashMap<(String, u32), String>> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/i18n/rust-frozen-all.tsv");
        let text = fs::read_to_string(path).ok()?;
        let unesc = |s: &str| {
            let mut out = String::new();
            let mut chars = s.chars();
            while let Some(c) = chars.next() {
                if c == '\\' {
                    match chars.next() {
                        Some('n') => out.push('\n'),
                        Some('t') => out.push('\t'),
                        Some('r') => out.push('\r'),
                        Some('\\') => out.push('\\'),
                        Some(o) => {
                            out.push('\\');
                            out.push(o)
                        }
                        None => out.push('\\'),
                    }
                } else {
                    out.push(c);
                }
            }
            out
        };
        let mut out = HashMap::new();
        for line in text
            .lines()
            .filter(|l| !l.starts_with('#') && !l.starts_with("path\t"))
        {
            let f: Vec<&str> = line.split('\t').collect();
            if f.len() == 9
                && let Ok(ordinal) = f[1].parse()
            {
                out.insert((f[0].to_owned(), ordinal), unesc(f[8]));
            }
        }
        Some(out)
    }

    // plan-critique F12: for every converted site ("#@" line), the Dutch the program renders with sample values is
    // byte for byte what the 3b898e6 literal printed with the same values (format! semantics for "fmt" sites);
    // for "en"/"enfmt" lines (the English half of a former inline pair) the same holds for the English.
    #[test]
    fn every_converted_site_renders_its_3b898e6_dutch() {
        let Some(frozen) = frozen_literals() else {
            eprintln!("tests/i18n/rust-frozen-all.tsv not found: skipped");
            return;
        };
        let mut checked = 0;
        for (stem, text) in STEMS {
            let mut maps: Vec<&str> = vec![];
            for line in text.lines() {
                if let Some(rest) = line.strip_prefix("#@ ") {
                    maps.push(rest);
                    continue;
                }
                if line.starts_with('#') || line.is_empty() {
                    maps.clear();
                    continue;
                }
                let fields: Vec<&str> = line.split('\t').collect();
                let key = fields[0];
                for map in maps.drain(..) {
                    let parts: Vec<&str> = map.split(' ').collect();
                    if parts[0] == "new" {
                        continue;
                    }
                    let (path, ordinal, kind) = (parts[0], parts[1], parts[3]);
                    let Some(literal) = ordinal
                        .parse()
                        .ok()
                        .and_then(|o: u32| frozen.get(&(path.to_owned(), o)))
                    else {
                        panic!("{stem}: {key}: {path}#{ordinal} is not in rust-frozen-all.tsv");
                    };
                    // sample values, one per placeholder name
                    let specs: Vec<(&str, &str)> = parts[4..]
                        .iter()
                        .filter_map(|s| s.split_once('='))
                        .collect();
                    let mut params = serde_json::Map::new();
                    for (_, name) in &specs {
                        params.insert((*name).to_owned(), Value::String(format!("⟨{name}⟩")));
                    }
                    // en / enfmt: the 3b898e6 literal was the English half of an inline pair
                    let english = kind == "en" || kind == "enfmt";
                    let (lang, text) = if english {
                        (Lang::En, unescape(fields[2]))
                    } else {
                        (Lang::Nl, unescape(fields[1]))
                    };
                    let rendered = fill(&text, &Value::Object(params));
                    let expected = if kind == "plain" || kind == "en" {
                        literal.clone()
                    } else {
                        // format!(literal, samples): k-th placeholder of the literal gets the k-th map entry's value
                        let mut out = String::new();
                        let mut k = 0;
                        let mut chars = literal.chars().peekable();
                        while let Some(c) = chars.next() {
                            match c {
                                '{' if chars.peek() == Some(&'{') => {
                                    chars.next();
                                    out.push('{');
                                }
                                '}' if chars.peek() == Some(&'}') => {
                                    chars.next();
                                    out.push('}');
                                }
                                '{' => {
                                    for d in chars.by_ref() {
                                        if d == '}' {
                                            break;
                                        }
                                    }
                                    out.push_str(&format!("⟨{}⟩", specs[k].1));
                                    k += 1;
                                }
                                c => out.push(c),
                            }
                        }
                        assert_eq!(k, specs.len(), "{key}: placeholder map of {path}#{ordinal}");
                        out
                    };
                    assert_eq!(rendered, expected, "{key} ({path}#{ordinal})");
                    if (kind == "plain" || kind == "en")
                        && !key.ends_with(".one")
                        && !key.ends_with(".zero")
                    {
                        assert_eq!(tr(lang, key), expected, "{key}: lookup");
                    }
                    checked += 1;
                }
            }
        }
        assert!(checked >= 17, "only {checked} converted sites found");
    }

    #[test]
    fn every_catalogue_is_well_formed_complete_and_consistent() {
        let mut stems = BTreeSet::new();
        let mut keys = HashMap::new();
        for (stem, text) in STEMS {
            assert!(stems.insert(*stem), "stem {stem} listed twice");
            assert!(
                text.starts_with(&format!("# host/i18n/{stem}.tsv")),
                "{stem}.tsv lacks its header"
            );
            let (rows, problems) = parse(stem, text);
            assert!(problems.is_empty(), "{problems:?}");
            for (number, line) in text.lines().enumerate() {
                if line.starts_with('#') || line.is_empty() {
                    continue;
                }
                assert!(!line.contains('\r'), "{stem}.tsv:{}: CR", number + 1);
                let fields: Vec<_> = line.split('\t').collect();
                assert_eq!(fields.len(), 3, "{stem}.tsv:{}", number + 1);
                assert!(
                    !fields[2].is_empty(),
                    "{stem}.tsv:{}: no English",
                    number + 1
                );
                for field in &fields[1..] {
                    let mut chars = field.chars();
                    while let Some(c) = chars.next() {
                        if c == '\\' {
                            assert!(
                                matches!(chars.next(), Some('n' | 't' | '\\')),
                                "{stem}.tsv:{}: bad escape",
                                number + 1
                            );
                        }
                    }
                }
            }
            for (key, row) in rows {
                assert!(keys.insert(key, *stem).is_none(), "duplicate key {key}");
                let en = row.en.as_deref().unwrap();
                assert_eq!(
                    placeholders(&row.nl),
                    placeholders(en),
                    "{key}: placeholders differ between nl and en"
                );
            }
        }
        // plural completeness: a .one/.other/.zero row needs both .one and .other
        for key in keys.keys() {
            for form in [".one", ".other", ".zero"] {
                if let Some(base) = key.strip_suffix(form) {
                    assert!(
                        keys.contains_key(format!("{base}.one").as_str()),
                        "{base}.one missing"
                    );
                    assert!(
                        keys.contains_key(format!("{base}.other").as_str()),
                        "{base}.other missing"
                    );
                }
            }
        }
        // the embedded table is exactly the files in host/i18n
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("i18n");
        let on_disk: BTreeSet<String> = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter_map(|e| {
                e.file_name()
                    .to_str()
                    .and_then(|n| n.strip_suffix(".tsv"))
                    .map(str::to_owned)
            })
            .collect();
        let listed: BTreeSet<String> = stems.iter().map(|s| s.to_string()).collect();
        assert_eq!(on_disk, listed);
    }

    #[test]
    fn lookups_fall_back_to_dutch_then_to_the_key() {
        assert_eq!(tr(Lang::Nl, "core.badArgs"), "Ongeldige invoer.");
        assert_eq!(tr(Lang::En, "core.badArgs"), "Invalid input.");
        assert_eq!(tr("en", "core.badArgs"), "Invalid input.");
        assert_eq!(tr(&String::from("nl"), "core.badArgs"), "Ongeldige invoer.");
        assert_eq!(
            tr("EN", "core.badArgs"),
            "Ongeldige invoer.",
            "only exactly en is English (C1)"
        );
        assert_eq!(
            tr(Lang::En, "core.noSuchKeyForTests"),
            "core.noSuchKeyForTests"
        );
        assert_eq!(
            trf(Lang::En, "core.noSuchKeyForTests", &json!({"x": 1})),
            "core.noSuchKeyForTests"
        );
        assert!(has("core.badArgs") && !has("core.noSuchKeyForTests"));
    }

    #[test]
    fn placeholders_fill_by_name_and_keep_what_has_no_value() {
        let p = json!({"name": "a.txt", "n": 3, "f": 1.5, "whole": 2.0, "yes": true, "nil": null});
        assert_eq!(
            fill("‘{name}’ ({n}, {f}, {whole}, {yes})", &p),
            "‘a.txt’ (3, 1.5, 2, true)"
        );
        assert_eq!(
            fill("{nil} {missing} {} {a-b} {{name}} {", &p),
            "{nil} {missing} {} {a-b} {a.txt} {"
        );
        assert_eq!(fill("{name}", &json!(null)), "{name}");
        assert_eq!(fill("geen", &p), "geen");
        // the same output as format!("{}") for every number a converted call site passes as text
        for x in [0.1f64, 2.0, 1e21, -3.25] {
            assert_eq!(fill("{x}", &json!({"x": x})), format!("{x}"));
        }
    }

    #[test]
    fn plurals_pick_zero_one_other_with_n_equal_to_one() {
        // core has no plural rows: run the same selection on a parsed fixture catalogue
        let text: &'static str = "# host/i18n/fx.tsv\nfx.items.zero\tgeen items\tno items\nfx.items.one\t{n} item\t{n} item\nfx.items.other\t{n} items\t{n} items\nfx.only.other\t{n} keer\t{n} times\nfx.plain\t{n} x\t{n} x\n";
        let (rows, problems) = parse("fx", text);
        assert!(problems.is_empty(), "{problems:?}");
        let rows: HashMap<_, _> = rows.into_iter().collect();
        let render = |key: &str, n: f64| -> String {
            let found = plural(|k| rows.get(k).map(|r| r.nl.as_ref()), key, n).unwrap_or("");
            fill(found, &count_params(&n, &json!({})))
        };
        assert_eq!(render("fx.items", 0.0), "geen items");
        assert_eq!(render("fx.items", 1.0), "1 item");
        assert_eq!(render("fx.items", 2.0), "2 items");
        assert_eq!(render("fx.items", 1.5), "1.5 items");
        assert_eq!(render("fx.only", 0.0), "0 keer", "no .zero row: .other");
        assert_eq!(
            render("fx.only", 1.0),
            "1 keer",
            "a missing .one falls back to .other"
        );
        assert_eq!(
            render("fx.plain", 1.0),
            "1 x",
            "a plain row as the last fallback"
        );
        assert_eq!(count_params(&5u64, &json!({"n": "1.000"}))["n"], "1.000");
        assert_eq!(count_params(&5u64, &json!({}))["n"], "5");
        assert_eq!(count_params(&5u64, &json!({"x": "y"}))["x"], "y");
        assert_eq!(
            trn(Lang::Nl, "core.noSuchPlural", 2, &json!({})),
            "core.noSuchPlural.other"
        );
    }

    #[test]
    fn escapes_are_decoded() {
        let (rows, _) = parse("fx", "fx.a\tx\\ny\\tz\\\\w\tq\n");
        assert_eq!(rows[0].1.nl, "x\ny\tz\\w");
        let (rows, problems) = parse("fx", "fx.b\tnl\nother.c\tnl\ten\nfx.d\t\ten\n");
        assert_eq!(rows.len(), 1, "a row without en still gives nl");
        assert!(rows[0].1.en.is_none());
        assert_eq!(problems.len(), 3);
    }

    #[test]
    fn formatting_follows_the_language() {
        assert_eq!(decimal(Lang::Nl, "1.5 GB"), "1,5 GB");
        assert_eq!(decimal(Lang::En, "1.5 GB"), "1.5 GB");
        assert_eq!(number("nl", 1.25, 1), "1,2");
        assert_eq!(number("en", 2.0, 2), "2.00");
        assert_eq!(join(Lang::Nl, &["A", "B", "C"]), "A, B en C");
        assert_eq!(join(Lang::En, &["A", "B", "C"]), "A, B and C");
        assert_eq!(join(Lang::En, &["A, x", "B"]), "A, x and B");
        assert_eq!(join_or(Lang::Nl, &["A", "B"]), "A of B");
        assert_eq!(join(Lang::Nl, &["A"]), "A");
        assert_eq!(join(Lang::Nl, &[] as &[&str]), "");
        assert_eq!(pick(Lang::En, "Map", "Folder"), "Folder");
        assert_eq!(pick(Lang::En, "Map", ""), "Map");
    }

    #[test]
    fn the_language_comes_only_from_the_settings_file_and_is_cached_by_its_identity() {
        let dir = temp_dir("lang");
        assert_eq!(language_in(&dir), Lang::Nl, "no file");
        let file = dir.join("settings.json");
        let write = |text: &str| {
            let tmp = dir.join(".settings.tmp");
            fs::write(&tmp, text).unwrap();
            fs::rename(&tmp, &file).unwrap();
        };
        write(r#"{"language":"en","theme":{"mode":"dark"}}"#);
        assert_eq!(language_in(&dir), Lang::En);
        write(r#"{"language":"nl"}"#);
        assert_eq!(language_in(&dir), Lang::Nl);
        for (text, lang) in [
            (r#"{"language":{"custom":"future"}}"#, Lang::Nl),
            (r#"{"language":"EN"}"#, Lang::Nl),
            (r#"{"language":"de"}"#, Lang::Nl),
            (r#"{"theme":{}}"#, Lang::Nl),
            ("not json", Lang::Nl),
            ("[\"en\"]", Lang::Nl),
            (r#"{"language":"en"}"#, Lang::En),
        ] {
            write(text);
            assert_eq!(language_in(&dir), lang, "{text}");
        }
        // an in-place rewrite of the same size is seen as well (same inode, new mtime/ctime)
        fs::write(&file, r#"{"language":"nl"}"#).unwrap();
        assert_eq!(language_in(&dir), Lang::Nl);
        // once the last change is older than the racy window, repeated calls do not parse it again; that holds
        // for an mtime in the past and in the future alike (a restored backup, clock skew: review m6), because
        // the test is on ctime, which setting the mtime moves to now
        for mtime in [
            SystemTime::now() - Duration::from_secs(5),
            SystemTime::now() + Duration::from_secs(3600),
        ] {
            fs::File::options()
                .write(true)
                .open(&file)
                .unwrap()
                .set_modified(mtime)
                .unwrap();
            std::thread::sleep(RACY + Duration::from_millis(30));
            let _ = language_in(&dir);
            let before = language_parses();
            for _ in 0..1000 {
                assert_eq!(language_in(&dir), Lang::Nl);
            }
            let parses = language_parses() - before;
            // other tests in this process may parse their own files meanwhile; ours adds none
            assert!(parses < 100, "{parses} parses for 1000 unchanged lookups");
        }
        write(r#"{"language":"en"}"#);
        assert_eq!(language_in(&dir), Lang::En);
        fs::remove_file(&file).unwrap();
        assert_eq!(language_in(&dir), Lang::Nl, "removed file");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_parse_is_trusted_only_outside_the_racy_window_of_the_last_change() {
        let now = SystemTime::now();
        let at = |t: SystemTime| {
            let d = t.duration_since(UNIX_EPOCH).unwrap();
            (d.as_secs() as i64, d.subsec_nanos() as i64)
        };
        let stamp = |ctime: SystemTime, mtime: SystemTime| Stamp {
            dev: 1,
            ino: 2,
            size: 3,
            mtime: at(mtime),
            ctime: at(ctime),
        };
        let ms = Duration::from_millis;
        let far = Duration::from_secs(3600);
        for (ctime, mtime, trusted) in [
            (now - ms(10), now - ms(10), false), // just written: may change again within its tick
            (now + ms(10), now + ms(10), false), // a few ms ahead (coarse or skewed clock): same
            (now - ms(200), now - ms(200), true),
            (now - ms(200), now + far, true), // mtime in the future (review m6): ctime decides
            (now - ms(200), now - far, true),
            (now - ms(10), now - far, false), // touch -d long ago, just now: still racy
            (now + far, now + far, true),     // the clock went back: any later change differs
        ] {
            assert_eq!(
                trusted_at(&stamp(ctime, mtime), now),
                trusted,
                "{ctime:?} {mtime:?}"
            );
        }
        let before_epoch = Stamp {
            ctime: (-5, 0),
            ..stamp(now, now)
        };
        assert!(trusted_at(&before_epoch, now));
    }

    #[test]
    fn lang_values_parse_as_contract_c1_says() {
        assert_eq!(Lang::of(&json!("en")), Lang::En);
        assert_eq!(Lang::of(&json!("nl")), Lang::Nl);
        assert_eq!(Lang::of(&json!({"custom": "en"})), Lang::Nl);
        assert_eq!(Lang::of(&Value::Null), Lang::Nl);
        assert_eq!(Lang::of_settings(&json!({"language": "en"})), Lang::En);
        for lang in Lang::ALL {
            assert_eq!(Lang::parse(lang.code()), lang);
            assert_eq!(Lang::from_u8(lang.as_u8()), lang);
        }
    }
}
