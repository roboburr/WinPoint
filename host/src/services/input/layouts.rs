//! `input.layouts` and `input.engines` (batch A, notes/claude/settings-parity.md §5.2): the catalogue of Taalopties'
//! "Een toetsenbord toevoegen", as GNOME Settings' Add Input Source finds it.
//!
//! * Layouts: xkeyboard-config's `evdev.xml` (`/usr/share/X11/xkb/rules`), as gnome-desktop's GnomeXkbInfo reads it
//!   (the extras file only with `show-all-sources`, which has no control): one entry per layout and per variant, the
//!   id `layout` or `layout+variant` (the id of an input source `('xkb', id)`), a variant without languages or
//!   countries of its own inheriting the layout's. Names and short names are translated through the
//!   `xkeyboard-config` catalogue of the UI language when it is installed (gettext's .mo files; Ubuntu's langpacks
//!   first), else they stay xkeyboard-config's English.
//! * Languages and countries: iso-codes (`/usr/share/iso-codes/json`, ISO 639-3 then 639-2 for the language codes
//!   xkeyboard-config and IBus use, ISO 3166-1 for the countries, `common_name` before `name`), translated through
//!   iso-codes' own catalogues.
//! * Engines: IBus's component files (`/usr/share/ibus/component/*.xml`) with their engines listed inline, without
//!   the `xkb:` engines (GNOME Settings lists those as layouts). A component that lists its engines by running a
//!   program (`<engines exec=...>`, ibus-table's Python) is not run: no Python under a WinPoint process; GNOME
//!   Settings asks the IBus daemon instead. `setup` is whether `ibus-setup-<engine before ':'>.desktop` is installed,
//!   which is what extension/launchBridge.js `launch.ibusSetup` starts (cc-input-source-ibus.c).
use crate::i18n::Lang;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

/// A gettext catalogue: msgid → msgstr (the first form of a plural; context entries keep their `ctx\x04` prefix).
#[derive(Debug, Default)]
pub(crate) struct Mo(HashMap<String, String>);

impl Mo {
    /// The translation of `text`, or `text` itself.
    pub(crate) fn get<'a>(&'a self, text: &'a str) -> &'a str {
        self.0.get(text).map_or(text, String::as_str)
    }
}

/// GNU gettext's binary .mo format (either byte order); None for anything else.
pub(crate) fn parse_mo(data: &[u8]) -> Option<Mo> {
    const MAGIC: u32 = 0x9504_12de;
    let head: [u8; 4] = data.get(0..4)?.try_into().ok()?;
    let le = match u32::from_le_bytes(head) {
        MAGIC => true,
        m if m.swap_bytes() == MAGIC => false,
        _ => return None,
    };
    let word = |at: usize| -> Option<usize> {
        let b: [u8; 4] = data.get(at..at.checked_add(4)?)?.try_into().ok()?;
        Some(if le {
            u32::from_le_bytes(b)
        } else {
            u32::from_be_bytes(b)
        } as usize)
    };
    let (count, originals, translations) = (word(8)?, word(12)?, word(16)?);
    if count > 1_000_000 {
        return None;
    }
    let string = |table: usize, i: usize| -> Option<&str> {
        let entry = table.checked_add(i.checked_mul(8)?)?;
        let (len, offset) = (word(entry)?, word(entry + 4)?);
        std::str::from_utf8(data.get(offset..offset.checked_add(len)?)?).ok()
    };
    let mut map = HashMap::with_capacity(count);
    for i in 0..count {
        let (Some(original), Some(translated)) = (string(originals, i), string(translations, i))
        else {
            continue;
        };
        let original = original.split('\0').next().unwrap_or("");
        let translated = translated.split('\0').next().unwrap_or("");
        if !original.is_empty() && !translated.is_empty() {
            map.insert(original.to_owned(), translated.to_owned());
        }
    }
    Some(Mo(map))
}

/// The catalogue of gettext domain `domain` for the UI language (none for English: the sources are English).
pub(crate) fn catalogue(root: &Path, domain: &str, lang: Lang) -> Mo {
    if lang == Lang::En {
        return Mo::default();
    }
    let code = lang.code();
    for base in ["usr/share/locale-langpack", "usr/share/locale"] {
        let path = root.join(format!("{base}/{code}/LC_MESSAGES/{domain}.mo"));
        if let Some(mo) = std::fs::read(&path).ok().and_then(|d| parse_mo(&d)) {
            return mo;
        }
    }
    Mo::default()
}

/// One keyboard layout (or variant) of evdev.xml, untranslated.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Layout {
    pub id: String,
    pub name: String,
    pub short: String,
    pub languages: Vec<String>,
    pub countries: Vec<String>,
}

/// roxmltree with the DTD xkeyboard-config's files declare (`<!DOCTYPE xkbConfigRegistry SYSTEM "xkb.dtd">`).
pub(crate) fn parse_xml(xml: &str) -> Option<roxmltree::Document<'_>> {
    let options = roxmltree::ParsingOptions {
        allow_dtd: true,
        ..Default::default()
    };
    roxmltree::Document::parse_with_options(xml, options).ok()
}

fn child<'a, 'i>(node: roxmltree::Node<'a, 'i>, tag: &str) -> Option<roxmltree::Node<'a, 'i>> {
    node.children().find(|n| n.has_tag_name(tag))
}

fn text_of(node: roxmltree::Node, tag: &str) -> String {
    child(node, tag)
        .and_then(|n| n.text())
        .map(|t| t.trim().to_owned())
        .unwrap_or_default()
}

fn ids(item: roxmltree::Node, list: &str, tag: &str) -> Vec<String> {
    child(item, list)
        .map(|l| {
            l.children()
                .filter(|n| n.has_tag_name(tag))
                .filter_map(|n| n.text().map(|t| t.trim().to_owned()))
                .filter(|t| !t.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// A layout or variant name as xkeyboard-config writes them: letters, digits, `_` and `-`.
fn xkb_name_ok(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// The layouts and variants of an xkeyboard-config rules file (GnomeXkbInfo's parse: a variant inherits the
/// layout's languages and countries when it lists none).
pub(crate) fn parse_rules(xml: &str) -> Vec<Layout> {
    let Some(doc) = parse_xml(xml) else {
        return Vec::new();
    };
    let Some(list) = child(doc.root_element(), "layoutList") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for layout in list.children().filter(|n| n.has_tag_name("layout")) {
        let Some(item) = child(layout, "configItem") else {
            continue;
        };
        let name = text_of(item, "name");
        if !xkb_name_ok(&name) {
            continue;
        }
        let base = Layout {
            id: name.clone(),
            name: text_of(item, "description"),
            short: text_of(item, "shortDescription"),
            languages: ids(item, "languageList", "iso639Id"),
            countries: ids(item, "countryList", "iso3166Id"),
        };
        let variants: Vec<Layout> = child(layout, "variantList")
            .into_iter()
            .flat_map(|v| v.children().filter(|n| n.has_tag_name("variant")))
            .filter_map(|v| child(v, "configItem"))
            .filter_map(|vi| {
                let variant = text_of(vi, "name");
                if !xkb_name_ok(&variant) {
                    return None;
                }
                let short = text_of(vi, "shortDescription");
                let languages = ids(vi, "languageList", "iso639Id");
                let countries = ids(vi, "countryList", "iso3166Id");
                Some(Layout {
                    id: format!("{name}+{variant}"),
                    name: text_of(vi, "description"),
                    short: if short.is_empty() {
                        base.short.clone()
                    } else {
                        short
                    },
                    languages: if languages.is_empty() {
                        base.languages.clone()
                    } else {
                        languages
                    },
                    countries: if countries.is_empty() {
                        base.countries.clone()
                    } else {
                        countries
                    },
                })
            })
            .collect();
        out.push(base);
        out.extend(variants);
    }
    out
}

/// Language and country names from iso-codes, translated for the UI language.
#[derive(Default)]
pub(crate) struct IsoNames {
    languages: HashMap<String, String>,
    countries: HashMap<String, String>,
}

impl IsoNames {
    pub(crate) fn load(root: &Path, lang: Lang) -> IsoNames {
        let json = |file: &str| -> Value {
            std::fs::read(root.join("usr/share/iso-codes/json").join(file))
                .ok()
                .and_then(|d| serde_json::from_slice(&d).ok())
                .unwrap_or(Value::Null)
        };
        let mut names = IsoNames::default();
        // 639-3 first (its "Dutch" before 639-2's "Dutch; Flemish"), 639-2 for the collective codes (ber, ...)
        for (file, table, domain) in [
            ("iso_639-3.json", "639-3", "iso_639-3"),
            ("iso_639-2.json", "639-2", "iso_639-2"),
        ] {
            let mo = catalogue(root, domain, lang);
            for entry in json(file)[table].as_array().into_iter().flatten() {
                let Some(name) = entry["name"].as_str() else {
                    continue;
                };
                for field in ["alpha_3", "bibliographic", "alpha_2"] {
                    if let Some(code) = entry[field].as_str() {
                        names
                            .languages
                            .entry(code.to_owned())
                            .or_insert_with(|| mo.get(name).to_owned());
                    }
                }
            }
        }
        let mo = catalogue(root, "iso_3166-1", lang);
        for entry in json("iso_3166-1.json")["3166-1"]
            .as_array()
            .into_iter()
            .flatten()
        {
            let (Some(code), Some(name)) = (
                entry["alpha_2"].as_str(),
                entry["common_name"]
                    .as_str()
                    .or_else(|| entry["name"].as_str()),
            ) else {
                continue;
            };
            names
                .countries
                .insert(code.to_owned(), mo.get(name).to_owned());
        }
        names
    }

    /// A language's name by its ISO 639 code (`nld`, `dut`, `nl`; IBus's `zh_CN` by its language part).
    pub(crate) fn language(&self, code: &str) -> Option<&str> {
        let code = code.split(['_', '-', '.', '@']).next().unwrap_or("");
        self.languages.get(code).map(String::as_str)
    }

    pub(crate) fn country(&self, code: &str) -> Option<&str> {
        self.countries.get(code).map(String::as_str)
    }
}

fn sort_by_name(list: &mut [Value]) {
    list.sort_by_key(|v| v["name"].as_str().unwrap_or("").to_lowercase());
}

/// `input.layouts {}`: `{layouts: [{id, name, short, languages, countries}], languages: [{code, name}],
/// countries: [{code, name}]}`, the names in the UI language where a catalogue has them, sorted by name.
pub(crate) fn layouts(root: &Path, lang: Lang) -> Value {
    let xml =
        std::fs::read_to_string(root.join("usr/share/X11/xkb/rules/evdev.xml")).unwrap_or_default();
    let mo = catalogue(root, "xkeyboard-config", lang);
    let iso = IsoNames::load(root, lang);
    let mut languages: BTreeMap<String, String> = BTreeMap::new();
    let mut countries: BTreeMap<String, String> = BTreeMap::new();
    let mut list: Vec<Value> = parse_rules(&xml)
        .into_iter()
        .map(|l| {
            for code in &l.languages {
                if let Some(name) = iso.language(code) {
                    languages.insert(code.clone(), name.to_owned());
                }
            }
            for code in &l.countries {
                if let Some(name) = iso.country(code) {
                    countries.insert(code.clone(), name.to_owned());
                }
            }
            json!({
                "id": l.id,
                "name": mo.get(&l.name),
                "short": mo.get(&l.short),
                "languages": l.languages,
                "countries": l.countries,
            })
        })
        .collect();
    sort_by_name(&mut list);
    let named = |map: BTreeMap<String, String>| -> Vec<Value> {
        let mut v: Vec<Value> = map
            .into_iter()
            .map(|(code, name)| json!({"code": code, "name": name}))
            .collect();
        sort_by_name(&mut v);
        v
    };
    json!({"layouts": list, "languages": named(languages), "countries": named(countries)})
}

/// One IBus engine of a component file.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Engine {
    pub id: String,
    pub name: String,
    pub language: String,
    /// The layout the engine types on (`layout` or `layout+variant`), for "Indeling weergeven".
    pub layout: Option<String>,
    pub textdomain: String,
}

/// An IBus engine name as launchBridge.js takes it: `[\w.+-]` parts joined by up to two `:` (§6.6).
pub(crate) fn engine_id_ok(id: &str) -> bool {
    let parts: Vec<&str> = id.split(':').collect();
    !id.is_empty()
        && id.len() <= 128
        && parts.len() <= 3
        && parts.iter().all(|p| {
            !p.is_empty()
                && p.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'+' | b'-'))
        })
}

/// The engines a component file lists inline (not `xkb:`, not a component that runs a program to list them).
pub(crate) fn parse_component(xml: &str) -> Vec<Engine> {
    let Some(doc) = parse_xml(xml) else {
        return Vec::new();
    };
    let component = doc.root_element();
    if !component.has_tag_name("component") {
        return Vec::new();
    }
    let textdomain = text_of(component, "textdomain");
    child(component, "engines")
        .into_iter()
        .flat_map(|e| e.children().filter(|n| n.has_tag_name("engine")))
        .filter_map(|e| {
            let id = text_of(e, "name");
            if id.starts_with("xkb:") || !engine_id_ok(&id) {
                return None;
            }
            let longname = text_of(e, "longname");
            let layout = text_of(e, "layout");
            let variant = text_of(e, "layout_variant");
            let layout = (xkb_name_ok(&layout) && layout != "default").then(|| {
                if xkb_name_ok(&variant) {
                    format!("{layout}+{variant}")
                } else {
                    layout
                }
            });
            Some(Engine {
                name: if longname.is_empty() {
                    id.clone()
                } else {
                    longname
                },
                id,
                language: text_of(e, "language"),
                layout,
                textdomain: textdomain.clone(),
            })
        })
        .collect()
}

/// Whether the engine has a setup program GNOME Shell can start (`ibus-setup-<part before ':'>.desktop`).
pub(crate) fn has_setup(root: &Path, id: &str) -> bool {
    let prefix = id.split(':').next().unwrap_or(id);
    let file = format!("ibus-setup-{prefix}.desktop");
    ["usr/share/applications", "usr/local/share/applications"]
        .iter()
        .any(|dir| root.join(dir).join(&file).is_file())
}

/// `input.engines {}`: `{engines: [{id, name, language, languageName, layout, setup}]}`, sorted by name.
pub(crate) fn engines(root: &Path, lang: Lang) -> Value {
    let iso = IsoNames::load(root, lang);
    let mut seen = std::collections::HashSet::new();
    let mut files: Vec<_> = std::fs::read_dir(root.join("usr/share/ibus/component"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "xml"))
        .collect();
    files.sort();
    let mut domains: HashMap<String, Mo> = HashMap::new();
    let mut list: Vec<Value> = Vec::new();
    for file in files {
        let Ok(xml) = std::fs::read_to_string(&file) else {
            continue;
        };
        for engine in parse_component(&xml) {
            if !seen.insert(engine.id.clone()) {
                continue;
            }
            let mo = domains.entry(engine.textdomain.clone()).or_insert_with(|| {
                if engine.textdomain.is_empty() {
                    Mo::default()
                } else {
                    catalogue(root, &engine.textdomain, lang)
                }
            });
            list.push(json!({
                "id": engine.id,
                "name": mo.get(&engine.name),
                "language": engine.language,
                "languageName": iso.language(&engine.language),
                "layout": engine.layout,
                "setup": has_setup(root, &engine.id),
            }));
        }
    }
    sort_by_name(&mut list);
    json!({ "engines": list })
}
