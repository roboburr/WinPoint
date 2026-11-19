//! `machine.locales` and `machine.login.*`: the locales of Taal en regio, Regionale notatie and Beheerinstellingen
//! voor taal, as GNOME Settings' Region & Language panel finds and names them.
//!
//! * The generated locales: `localedef --list-archive` through `tools::system_tool` (amendment 59); when that is not
//!   there, glibc's locale archive read directly, as gnome-desktop's gnome-languages.c reads it, and the locale
//!   directories next to it. Names are normalised (`nl_NL.utf8` → `nl_NL.UTF-8`), C and POSIX left out.
//! * Their names: iso-codes' ISO 639 and ISO 3166 names, translated through its own catalogues
//!   (/usr/share/locale/<ll>/LC_MESSAGES/iso_639-2.mo, iso_3166-1.mo), as gnome-desktop names them: "Nederlands
//!   (Nederland)" in Dutch, "Dutch (Netherlands)" in English, and the endonym in the locale's own language.
//! * `{preview: locale}`: GNOME's Formats preview (cc-format-preview.c): %x, %X and %c in that locale, a number with
//!   its separators and grouping, its measurement system and paper size.
//! * The sign-in screen (locale1): `machine.login.get` reads Locale, `machine.login.setLocale {locale?, formats?}`
//!   sends what GNOME sends (cc-region-page.c set_localed_locale): LANG, and with a formats locale LC_TIME,
//!   LC_NUMERIC, LC_MONETARY, LC_MEASUREMENT and LC_PAPER, with `interactive` true (polkit `locale1.set-locale`).
use super::{Buses, LOCALE, dry, languages, unavailable};
use crate::files::{ApiError, ApiResult};
use crate::i18n::{Lang, NO_PARAMS};
use crate::services::users::accounts::valid_locale;
use glib::variant::ToVariant;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// `nl_NL.utf8` → `nl_NL.UTF-8` (gnome-desktop's normalisation); C and POSIX → None.
pub(crate) fn normalize(name: &str) -> Option<String> {
    let name = name.trim();
    if name.is_empty() || name.starts_with("C.") || name == "C" || name == "POSIX" {
        return None;
    }
    let (rest, modifier) = match name.split_once('@') {
        Some((r, m)) => (r, Some(m)),
        None => (name, None),
    };
    let (base, codeset) = match rest.split_once('.') {
        Some((b, c)) => (b, Some(c)),
        None => (rest, None),
    };
    let codeset = codeset.map(|c| {
        if c.eq_ignore_ascii_case("utf8") || c.eq_ignore_ascii_case("utf-8") {
            "UTF-8".to_owned()
        } else {
            c.to_owned()
        }
    });
    let mut out = base.to_owned();
    if let Some(c) = codeset {
        out.push('.');
        out.push_str(&c);
    }
    if let Some(m) = modifier {
        out.push('@');
        out.push_str(m);
    }
    valid_locale(&out).then_some(out)
}

/// The locales of a list (localedef's lines, the archive's names): normalised, a UTF-8 one where there is one
/// (`en_AG` next to `en_AG.UTF-8` is left out), without doubles, sorted.
pub(crate) fn clean_list<'a>(names: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut all: Vec<String> = names.filter_map(normalize).collect();
    all.sort();
    all.dedup();
    let key = |l: &str| {
        let (rest, modifier) = l.split_once('@').map_or((l, ""), |(r, m)| (r, m));
        (
            rest.split('.').next().unwrap_or("").to_owned(),
            modifier.to_owned(),
        )
    };
    let utf8: std::collections::HashSet<(String, String)> = all
        .iter()
        .filter(|l| l.contains(".UTF-8"))
        .map(|l| key(l))
        .collect();
    all.into_iter()
        .filter(|l| l.contains(".UTF-8") || !utf8.contains(&key(l)))
        .collect()
}

/// The names in glibc's locale archive (locarchive.h: the header's name hash table, every used entry's name).
pub(crate) fn archive_names(bytes: &[u8]) -> Vec<String> {
    let u32_at = |o: usize| -> Option<usize> {
        Some(u32::from_ne_bytes(bytes.get(o..o + 4)?.try_into().ok()?) as usize)
    };
    if u32_at(0) != Some(0xde02_0109) {
        return Vec::new();
    }
    let (Some(offset), Some(size)) = (u32_at(8), u32_at(16)) else {
        return Vec::new();
    };
    (0..size.min(1 << 20))
        .filter_map(|i| {
            let entry = offset + i * 12;
            let (name, record) = (u32_at(entry + 4)?, u32_at(entry + 8)?);
            if record == 0 {
                return None;
            }
            let tail = bytes.get(name..)?;
            let end = tail.iter().position(|b| *b == 0)?;
            std::str::from_utf8(&tail[..end]).ok().map(str::to_owned)
        })
        .collect()
}

/// The generated locales of this computer.
pub(crate) fn generated(b: &Buses) -> Vec<String> {
    let program = b
        .localedef
        .clone()
        .or_else(|| crate::tools::system_tool("localedef"));
    if let Some(program) = program.filter(|p| p.is_file()) {
        let out = crate::services::system::common::run(
            &[program.to_string_lossy().as_ref(), "--list-archive"],
            5,
            true,
        );
        if out.ok && !out.out.trim().is_empty() {
            return clean_list(out.out.lines());
        }
    }
    // gnome-desktop's own way: the archive itself, then the directories of locales outside it
    let root = b.file("/usr/lib/locale");
    let mut names = std::fs::read(root.join("locale-archive"))
        .map(|bytes| archive_names(&bytes))
        .unwrap_or_default();
    names.extend(
        std::fs::read_dir(&root)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().join("LC_IDENTIFICATION").is_file())
            .filter_map(|e| e.file_name().to_str().map(str::to_owned)),
    );
    clean_list(names.iter().map(String::as_str))
}

/// [`generated`], listed again only when the locale archive or its directory changes (users.* resolves every
/// account's language against it, machine.locales lists it).
pub(crate) fn generated_cached(b: &Buses) -> Arc<Vec<String>> {
    type Stamp = Option<std::time::SystemTime>;
    type Key = (Option<PathBuf>, Option<PathBuf>, Stamp, Stamp);
    static CACHE: Mutex<Option<(Key, Arc<Vec<String>>)>> = Mutex::new(None);
    let stamp = |p: PathBuf| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    let root = b.file("/usr/lib/locale");
    let key: Key = (
        b.root.clone(),
        b.localedef.clone(),
        stamp(root.join("locale-archive")),
        stamp(root.clone()),
    );
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((k, list)) = cache.as_ref()
        && *k == key
        && (key.2.is_some() || key.3.is_some())
    {
        return list.clone();
    }
    let list = Arc::new(generated(b));
    *cache = Some((key, list.clone()));
    list
}

// --------------------------------------------------------------------------- the account's language (Ubuntu)

/// What an account's language is resolved against: the generated locales and Ubuntu's main countries
/// (/usr/share/language-tools/main-countries: `nl nl_NL`).
#[derive(Clone, Default)]
pub(crate) struct LocaleData {
    pub generated: Arc<Vec<String>>,
    pub main: Arc<HashMap<String, String>>,
}

/// main-countries' lines (`<language> <locale>`, `#` comments).
pub(crate) fn parse_main_countries(text: &str) -> HashMap<String, String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            let mut f = l.split_whitespace();
            Some((f.next()?.to_owned(), f.next()?.to_owned()))
        })
        .collect()
}

/// The generated locales and main-countries of this computer.
pub(crate) fn locale_data(b: &Buses) -> LocaleData {
    LocaleData {
        generated: generated_cached(b),
        main: Arc::new(parse_main_countries(
            &std::fs::read_to_string(b.file("/usr/share/language-tools/main-countries"))
                .unwrap_or_default(),
        )),
    }
}

/// A locale id without its codeset: `nl_NL.UTF-8` → `nl_NL`, `sr_RS.UTF-8@latin` → `sr_RS@latin`.
fn without_codeset(locale: &str) -> String {
    let (rest, modifier) = match locale.split_once('@') {
        Some((r, m)) => (r, Some(m)),
        None => (locale, None),
    };
    let base = rest.split('.').next().unwrap_or("");
    match modifier {
        Some(m) => format!("{base}@{m}"),
        None => base.to_owned(),
    }
}

/// A language (AccountsService's Language, a session's LANG) as one of the generated UTF-8 locales, the way Ubuntu's
/// /usr/share/language-tools/language2locale finds it: the codeset dropped, then the exact `ll_CC[@mod]`; for a
/// bare language its main country's locale, else the first locale of that language (not for Chinese). Ubuntu's
/// accounts-daemon stores `nl_NL` (or `en`) after language-validate, never the `nl_NL.UTF-8` it was given, so the
/// pages compare and look up this id, which is machine.locales' own. None when no generated locale fits.
pub(crate) fn language_locale(raw: &str, data: &LocaleData) -> Option<String> {
    let first = raw.split(':').next().unwrap_or("").trim();
    if first.is_empty() || first == "C" || first.starts_with("C.") || first == "POSIX" {
        return None;
    }
    let lang = without_codeset(first);
    let code = lang.split(['_', '@']).next().unwrap_or("").to_owned();
    let modifier = lang.split_once('@').map(|(_, m)| m.to_owned());
    let candidates: Vec<(&String, String)> = data
        .generated
        .iter()
        .filter(|l| l.contains(".UTF-8"))
        .map(|l| (l, without_codeset(l)))
        .filter(|(_, short)| short.split(['_', '@']).next() == Some(code.as_str()))
        .collect();
    if let Some((id, _)) = candidates.iter().find(|(_, short)| *short == lang) {
        return Some((*id).clone());
    }
    if lang.split('@').next().unwrap_or("").contains('_') {
        return None;
    }
    if let Some(main) = data.main.get(&code) {
        let main = match &modifier {
            Some(m) => format!("{main}@{m}"),
            None => main.clone(),
        };
        if let Some((id, _)) = candidates.iter().find(|(_, short)| *short == main) {
            return Some((*id).clone());
        }
    }
    if code == "zh" {
        return None;
    }
    candidates.first().map(|(id, _)| (*id).clone())
}

/// The language users.* reports for an account: `raw` (or `fallback` when the account has none: GNOME's Region page
/// shows the session's language then) as a generated locale id; the normalised value itself when no generated
/// locale fits; "" for none.
pub(crate) fn account_language(raw: &str, fallback: &str, data: &LocaleData) -> String {
    let raw = if raw.trim().is_empty() { fallback } else { raw };
    if raw.trim().is_empty() {
        return String::new();
    }
    language_locale(raw, data)
        .or_else(|| normalize(raw.split(':').next().unwrap_or("")))
        .unwrap_or_default()
}

/// The session's display language (LANG) as a generated locale id.
pub(crate) fn session_language(data: &LocaleData) -> String {
    let lang = std::env::var("LANG").unwrap_or_default();
    account_language(&lang, "", data)
}

// ------------------------------------------------------------------------------------------- iso-codes' names

/// A GNU gettext catalogue (.mo): msgid → msgstr (the first form of a plural, no header).
pub(crate) fn parse_mo(bytes: &[u8]) -> HashMap<String, String> {
    let little = match bytes.get(0..4) {
        Some([0xde, 0x12, 0x04, 0x95]) => true,
        Some([0x95, 0x04, 0x12, 0xde]) => false,
        _ => return HashMap::new(),
    };
    let u32_at = |o: usize| -> Option<usize> {
        let raw: [u8; 4] = bytes.get(o..o + 4)?.try_into().ok()?;
        Some(if little {
            u32::from_le_bytes(raw)
        } else {
            u32::from_be_bytes(raw)
        } as usize)
    };
    let text = |table: usize, i: usize| -> Option<&str> {
        let (len, off) = (u32_at(table + i * 8)?, u32_at(table + i * 8 + 4)?);
        let raw = bytes.get(off..off + len)?;
        let first = raw.split(|b| *b == 0).next()?;
        std::str::from_utf8(first).ok()
    };
    let (Some(n), Some(originals), Some(translations)) = (u32_at(8), u32_at(12), u32_at(16)) else {
        return HashMap::new();
    };
    (0..n.min(1 << 20))
        .filter_map(|i| {
            let id = text(originals, i)?;
            let s = text(translations, i)?;
            (!id.is_empty() && !s.is_empty()).then(|| (id.to_owned(), s.to_owned()))
        })
        .collect()
}

type Catalogue = Arc<HashMap<String, String>>;

/// The catalogue of an iso-codes domain in a language (`nl`, `pt_BR` then `pt`), cached per (domain, language).
fn catalogue(b: &Buses, domain: &str, language: &str) -> Catalogue {
    type Key = (Option<PathBuf>, String, String);
    static CACHE: Mutex<Option<HashMap<Key, Catalogue>>> = Mutex::new(None);
    let key = (b.root.clone(), domain.to_owned(), language.to_owned());
    if let Some(hit) = CACHE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .and_then(|c| c.get(&key))
    {
        return hit.clone();
    }
    let short = language.split(['_', '.', '@']).next().unwrap_or("");
    let found = [language, short]
        .iter()
        .filter(|l| !l.is_empty())
        .find_map(|l| {
            std::fs::read(b.file(&format!("/usr/share/locale/{l}/LC_MESSAGES/{domain}.mo"))).ok()
        })
        .map(|bytes| parse_mo(&bytes))
        .unwrap_or_default();
    let found = Arc::new(found);
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let map = cache.get_or_insert_with(HashMap::new);
    if map.len() > 256 {
        map.clear();
    }
    map.insert(key, found.clone());
    found
}

/// iso-codes' English names: ISO 639-2 and 639-3 languages by code, ISO 3166-1 countries by alpha-2.
struct Iso {
    languages: HashMap<String, String>,
    countries: HashMap<String, String>,
}

/// The English names of an iso-codes JSON file: `code_key` → `common_name` or else `name`.
pub(crate) fn iso_names(text: &str, code_keys: &[&str]) -> HashMap<String, String> {
    let v: Value = serde_json::from_str(text).unwrap_or(Value::Null);
    let mut out = HashMap::new();
    for list in v.as_object().into_iter().flat_map(|o| o.values()) {
        for e in list.as_array().into_iter().flatten() {
            let Some(name) = e["common_name"].as_str().or(e["name"].as_str()) else {
                continue;
            };
            for k in code_keys {
                if let Some(code) = e[*k].as_str() {
                    out.entry(code.to_owned())
                        .or_insert_with(|| name.to_owned());
                }
            }
        }
    }
    out
}

/// The names of the root's iso-codes, read once per root.
fn iso(b: &Buses) -> Arc<Iso> {
    static CACHE: Mutex<Option<HashMap<Option<PathBuf>, Arc<Iso>>>> = Mutex::new(None);
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let map = cache.get_or_insert_with(HashMap::new);
    map.entry(b.root.clone())
        .or_insert_with(|| {
            let read = |f: &str| {
                std::fs::read_to_string(b.file(&format!("/usr/share/iso-codes/json/{f}")))
                    .unwrap_or_default()
            };
            let mut languages = iso_names(&read("iso_639-3.json"), &["alpha_3"]);
            // 639-2's codes and names win: the catalogues of iso_639-2 translate those
            languages.extend(iso_names(&read("iso_639-2.json"), &["alpha_2", "alpha_3"]));
            Arc::new(Iso {
                languages,
                countries: iso_names(&read("iso_3166-1.json"), &["alpha_2"]),
            })
        })
        .clone()
}

/// The first item of a "Dutch; Flemish" list, its first letter upper case (gnome-languages.c).
pub(crate) fn first_item(name: &str) -> String {
    let first = name.split(';').next().unwrap_or("").trim();
    let mut chars = first.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// The name of a language code in `language` ("nl" in "nl" → "Nederlands"), None when iso-codes has no name.
pub(crate) fn language_name(b: &Buses, code: &str, language: &str) -> Option<String> {
    let names = iso(b);
    let english = names.languages.get(code)?;
    let domain = if code.len() == 2 {
        "iso_639-2"
    } else {
        "iso_639-3"
    };
    let translated = if language.starts_with("en") {
        None
    } else {
        catalogue(b, domain, language)
            .get(english)
            .cloned()
            .or_else(|| catalogue(b, "iso_639-2", language).get(english).cloned())
    };
    Some(first_item(translated.as_deref().unwrap_or(english)))
}

/// The name of a country in `language` ("NL" in "nl" → "Nederland").
pub(crate) fn country_name(b: &Buses, code: &str, language: &str) -> Option<String> {
    let names = iso(b);
    let english = names.countries.get(code)?;
    let translated = if language.starts_with("en") {
        None
    } else {
        catalogue(b, "iso_3166-1", language).get(english).cloned()
    };
    Some(translated.unwrap_or_else(|| english.clone()))
}

/// (language, country, modifier) of a locale id.
pub(crate) fn parts(locale: &str) -> (String, Option<String>, Option<String>) {
    let (rest, modifier) = match locale.split_once('@') {
        Some((r, m)) => (r, Some(m.to_owned())),
        None => (locale, None),
    };
    let base = rest.split('.').next().unwrap_or("");
    match base.split_once('_') {
        Some((l, c)) => (l.to_owned(), Some(c.to_owned()), modifier),
        None => (base.to_owned(), None, modifier),
    }
}

/// A locale's name in `language`: "Nederlands (Nederland)", "Serbian (Serbia, latin)"; the id when iso-codes has none.
pub(crate) fn display_name(b: &Buses, locale: &str, language: &str) -> String {
    let (code, country, modifier) = parts(locale);
    let Some(name) = language_name(b, &code, language) else {
        return locale.to_owned();
    };
    let mut extra: Vec<String> = Vec::new();
    if let Some(c) = country
        .as_deref()
        .and_then(|c| country_name(b, c, language))
    {
        extra.push(c);
    }
    if let Some(m) = modifier.filter(|m| m != "euro") {
        extra.push(m);
    }
    if extra.is_empty() {
        name
    } else {
        format!("{name} ({})", extra.join(", "))
    }
}

// -------------------------------------------------------------------------------------------- the preview

const MEASUREMENT: libc::nl_item = 0xB0000; // _NL_MEASUREMENT_MEASUREMENT: _NL_ITEM (LC_MEASUREMENT = 11, 0)
const PAPER_HEIGHT: libc::nl_item = 0x70000; // _NL_PAPER_HEIGHT: _NL_ITEM (LC_PAPER = 7, 0)
const PAPER_WIDTH: libc::nl_item = 0x70001; // _NL_PAPER_WIDTH
const GROUPING: libc::nl_item = 0x10002; // GROUPING: _NL_ITEM (LC_NUMERIC = 1, 2)

/// `digits` with the locale's thousands separator in the places its grouping says (glibc: each byte a group size
/// from the right, the last one repeating, CHAR_MAX: no more groups).
pub(crate) fn group(digits: &str, separator: &str, grouping: &[u8]) -> String {
    let chars: Vec<char> = digits.chars().collect();
    if separator.is_empty() || grouping.is_empty() || grouping[0] == 0 || grouping[0] >= 127 {
        return digits.to_owned();
    }
    let mut groups: Vec<String> = Vec::new();
    let (mut end, mut size, mut i) = (chars.len(), grouping[0] as usize, 0);
    while end > size {
        groups.push(chars[end - size..end].iter().collect());
        end -= size;
        i += 1;
        match grouping.get(i) {
            Some(g) if *g >= 127 => size = end,
            Some(g) if *g > 0 => size = *g as usize,
            _ => {}
        }
    }
    groups.push(chars[..end].iter().collect());
    groups.reverse();
    groups.join(separator)
}

/// GNOME's Formats preview of a locale at `at` (seconds since the epoch), None when the locale is not generated.
pub(crate) fn preview(locale: &str, at: i64) -> Option<Value> {
    let c = std::ffi::CString::new(locale).ok()?;
    unsafe {
        let loc = libc::newlocale(libc::LC_ALL_MASK, c.as_ptr(), std::ptr::null_mut());
        if loc.is_null() {
            return None;
        }
        let mut tm: libc::tm = std::mem::zeroed();
        let t = at as libc::time_t;
        libc::localtime_r(&t, &mut tm);
        let format = |f: &std::ffi::CStr| {
            let mut buf = [0u8; 256];
            let n = libc::strftime_l(buf.as_mut_ptr().cast(), buf.len(), f.as_ptr(), &tm, loc);
            String::from_utf8_lossy(&buf[..n]).trim().to_owned()
        };
        let info = |item: libc::nl_item| {
            let p = libc::nl_langinfo_l(item, loc);
            if p.is_null() {
                Vec::new()
            } else {
                std::ffi::CStr::from_ptr(p).to_bytes().to_vec()
            }
        };
        let word =
            |item: libc::nl_item| (libc::nl_langinfo_l(item, loc) as usize & 0xffff_ffff) as u32;
        let radix = String::from_utf8_lossy(&info(libc::RADIXCHAR)).into_owned();
        let thousands = String::from_utf8_lossy(&info(libc::THOUSEP)).into_owned();
        let number = format!(
            "{}{}00",
            group("123456789", &thousands, &info(GROUPING)),
            if radix.is_empty() { "." } else { &radix }
        );
        let measurement = info(MEASUREMENT).first().copied();
        let (width, height) = (word(PAPER_WIDTH), word(PAPER_HEIGHT));
        let out = json!({
            "date": format(c"%x"),
            "time": format(c"%X"),
            "dateTime": format(c"%c"),
            "number": number,
            "measurement": if measurement == Some(2) { "imperial" } else { "metric" },
            "paper": paper_name(width, height),
            "paperSize": [width, height],
        });
        libc::freelocale(loc);
        Some(out)
    }
}

/// GTK's names of the common sizes in mm; None for another size.
pub(crate) fn paper_name(width: u32, height: u32) -> Option<&'static str> {
    match (width, height) {
        (210, 297) => Some("A4"),
        (216, 279) => Some("US Letter"),
        (216, 356) => Some("US Legal"),
        (148, 210) => Some("A5"),
        _ => None,
    }
}

// ------------------------------------------------------------------------------------------------ the methods

/// The session's own locales: LANG (the display language) and LC_TIME (the formats), from this process's environment,
/// as generated locale ids (the ids users.* reports for an account's language).
fn session(data: &LocaleData) -> Value {
    let language = session_language(data);
    let formats = std::env::var("LC_TIME")
        .ok()
        .map(|l| account_language(&l, "", data))
        .filter(|l| !l.is_empty())
        .unwrap_or_else(|| language.clone());
    json!({"language": language, "formats": formats})
}

/// The account's own language as AccountsService stores it (its Language), None without AccountsService or a
/// language; unit tests never ask the system bus.
fn account_raw_language() -> Option<String> {
    if cfg!(test) {
        return None;
    }
    use crate::services::users::accounts;
    let daemon = accounts::Daemon::system()?;
    let path = daemon.find(accounts::self_uid(), Lang::En).ok()?;
    let props = daemon.props(&path, Lang::En).ok()?;
    props["Language"]
        .as_str()
        .filter(|l| !l.trim().is_empty())
        .map(str::to_owned)
}

/// The entries of `ids` that are not generated, named from the language and territory tables (as
/// gnome_get_language_from_locale names any locale), so the account's and the session's language never show their
/// id: `{id, name, endonym, language, country, supported: false, generated: false}`. An id without a language name
/// (C, POSIX) is left out.
pub(crate) fn named_extras(
    b: &Buses,
    ids: &[String],
    generated: &[String],
    ui: &str,
) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    for id in ids {
        if id.is_empty() || generated.contains(id) || out.iter().any(|e| e["id"] == id.as_str()) {
            continue;
        }
        let (code, country, _) = parts(id);
        if language_name(b, &code, ui).is_none() {
            continue;
        }
        let own = id.split(['.', '@']).next().unwrap_or("").to_owned();
        out.push(json!({
            "id": id,
            "name": display_name(b, id, ui),
            "endonym": display_name(b, id, &own),
            "language": code,
            "country": country,
            "supported": false,
            "generated": false,
        }));
    }
    out
}

/// `machine.locales {preview?}` → `{locales: [{id, name, endonym, language, country, supported, generated}], session,
/// preview?}`: the generated locales, and the account's and the session's language when the locale archive lacks
/// them (`generated: false`, never `supported`).
pub(crate) fn locales(b: &Buses, args: &Value, lang: Lang) -> ApiResult {
    let ui = lang.code();
    let supported = languages::installed_packs(b);
    let data = locale_data(b);
    let mut list: Vec<Value> = data
        .generated
        .iter()
        .cloned()
        .map(|id| {
            let (code, country, _) = parts(&id);
            let own = id.split(['.', '@']).next().unwrap_or("").to_owned();
            json!({
                "id": id,
                "name": display_name(b, &id, ui),
                "endonym": display_name(b, &id, &own),
                "language": code,
                "country": country,
                "supported": code == "en" || supported.contains(&languages::pack_code(b, &id)),
                "generated": true,
            })
        })
        .collect();
    let session = session(&data);
    let mut own: Vec<String> = account_raw_language()
        .map(|raw| account_language(&raw, "", &data))
        .into_iter()
        .collect();
    for key in ["language", "formats"] {
        own.push(session[key].as_str().unwrap_or("").to_owned());
    }
    list.extend(named_extras(b, &own, &data.generated, ui));
    list.sort_by_key(|l| l["name"].as_str().unwrap_or("").to_lowercase());
    let mut out = json!({"locales": list, "session": session});
    match &args["preview"] {
        Value::Null => {}
        Value::String(p) if valid_locale(p) && !p.is_empty() => {
            out["preview"] = preview(p, crate::services::system::common::now_ms() as i64 / 1000)
                .unwrap_or(Value::Null);
        }
        _ => return Err(super::bad(lang)),
    }
    Ok(out)
}

/// locale1's Locale list (`LANG=…`, `LC_TIME=…`) as a map.
pub(crate) fn parse_locale_list(list: &Value) -> BTreeMap<String, String> {
    list.as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| e.as_str()?.split_once('='))
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect()
}

/// The categories GNOME sends for the sign-in screen's formats.
pub(crate) const FORMAT_KEYS: [&str; 5] = [
    "LC_TIME",
    "LC_NUMERIC",
    "LC_MONETARY",
    "LC_MEASUREMENT",
    "LC_PAPER",
];

/// What SetLocale gets: `LANG=<language>`, then with a formats locale the five LC_ entries (an empty one: none, the
/// formats follow the language). Every other entry goes, as with GNOME.
pub(crate) fn compose(language: &str, formats: Option<&str>) -> Vec<String> {
    let mut out = vec![format!("LANG={language}")];
    if let Some(f) = formats.filter(|f| !f.is_empty()) {
        out.extend(FORMAT_KEYS.iter().map(|k| format!("{k}={f}")));
    }
    out
}

/// `machine.login.get {}` → `{available, language, formats, entries}`.
pub(crate) fn login_get(b: &Buses) -> Value {
    let (_, path, iface) = LOCALE;
    let props = b.locale().and_then(|p| p.props(path, iface).ok());
    let entries = props
        .as_ref()
        .map(|p| parse_locale_list(&p["Locale"]))
        .unwrap_or_default();
    let language = entries.get("LANG").cloned().unwrap_or_default();
    let formats = entries.get("LC_TIME").cloned().unwrap_or_default();
    json!({"available": props.is_some(), "language": language, "formats": formats, "entries": entries})
}

/// `machine.login.setLocale {locale?, formats?}`: the sign-in screen's language and formats (at least one given; a
/// missing one keeps what locale1 has; `formats: ""` lets the formats follow the language).
pub(crate) fn login_set(b: &Buses, args: &Value, lang: Lang, dry_run: bool) -> ApiResult {
    let arg = |k: &str| -> Result<Option<&str>, ApiError> {
        match &args[k] {
            Value::Null => Ok(None),
            Value::String(s) if valid_locale(s) => Ok(Some(s.as_str())),
            _ => Err(ApiError::tr(
                "bad_args",
                lang,
                "machine.badLocale",
                NO_PARAMS,
            )),
        }
    };
    let (language, formats) = (arg("locale")?, arg("formats")?);
    if (language.is_none() && formats.is_none()) || language == Some("") {
        return Err(ApiError::tr(
            "bad_args",
            lang,
            "machine.badLocale",
            NO_PARAMS,
        ));
    }
    if dry_run {
        return dry();
    }
    let peer = b
        .locale()
        .ok_or_else(|| unavailable(lang, "machine.localeUnavailable"))?;
    let (_, path, iface) = LOCALE;
    let current = peer
        .props(path, iface)
        .map(|p| parse_locale_list(&p["Locale"]))
        .map_err(|_| unavailable(lang, "machine.localeUnavailable"))?;
    let language = language
        .map(str::to_owned)
        .or_else(|| current.get("LANG").cloned())
        .unwrap_or_else(|| "C.UTF-8".into());
    let formats = formats
        .map(str::to_owned)
        .or_else(|| current.get("LC_TIME").cloned());
    let list = compose(&language, formats.as_deref());
    peer.interactive(
        path,
        iface,
        "SetLocale",
        &(list.clone(), true).to_variant(),
        lang,
    )?;
    Ok(
        json!({"ok": true, "language": language, "formats": formats.unwrap_or_default(), "locale": list}),
    )
}

#[cfg(test)]
mod named_tests {
    use super::*;

    #[test]
    fn a_language_that_is_not_generated_is_named_in_both_languages() {
        let b = Buses::default();
        if !std::path::Path::new("/usr/share/iso-codes/json/iso_639-2.json").is_file() {
            eprintln!("skipped: no iso-codes");
            return;
        }
        let generated = vec!["en_GB.UTF-8".to_owned()];
        let ids = |list: &[&str]| list.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        let en = named_extras(
            &b,
            &ids(&["nl_BE.UTF-8", "en_GB.UTF-8", "", "C.UTF-8", "nl_BE.UTF-8"]),
            &generated,
            "en",
        );
        assert_eq!(
            en.len(),
            1,
            "a generated one, an empty one, C and a repeat are left out: {en:?}"
        );
        assert_eq!(en[0]["id"], "nl_BE.UTF-8");
        assert_eq!(en[0]["name"], "Dutch (Belgium)");
        assert_eq!(en[0]["language"], "nl");
        assert_eq!(en[0]["country"], "BE");
        assert_eq!(
            (en[0]["supported"].clone(), en[0]["generated"].clone()),
            (json!(false), json!(false))
        );
        if std::path::Path::new("/usr/share/locale/nl/LC_MESSAGES/iso_639-2.mo").is_file() {
            let nl = named_extras(&b, &ids(&["nl_BE.UTF-8"]), &generated, "nl");
            assert_eq!(nl[0]["name"], "Nederlands (België)");
            assert_eq!(nl[0]["endonym"], "Nederlands (België)");
        }
    }
}
