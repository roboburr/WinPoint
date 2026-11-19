//! A PPD's options as GNOME's Printing Options dialog shows them (pp-options-dialog.c, pp-ppd-option-widget.c): the
//! groups (`*OpenGroup`, CUPS's "General" for options outside one) with their PickOne, PickMany and Boolean options
//! (`*OpenUI` / `*JCLOpenUI`), each with its choices, the PPD's `*Default…` and the translations of the UI language
//! (`*nl.Translation Keyword/Text`, `*nl.Keyword Choice/Text`, CUPS's localisation). Pure; tested on fixtures.
use serde_json::{Value, json};

/// One choice of an option.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Choice {
    pub value: String,
    pub text: String,
}

/// One option (`*OpenUI *Keyword/Text: PickOne`).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Opt {
    pub keyword: String,
    pub text: String,
    /// "pickone", "pickmany" or "boolean".
    pub ui: String,
    pub default: String,
    pub choices: Vec<Choice>,
}

/// One group (`*OpenGroup: Name/Text`).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Group {
    pub name: String,
    pub text: String,
    pub options: Vec<Opt>,
}

/// Options g-c-c never shows: PageRegion follows PageSize (CUPS sets it itself).
const HIDDEN: &[&str] = &["PageRegion"];

/// `Keyword/Text` (the text may be missing; then the keyword is the text).
fn split_text(s: &str) -> (String, String) {
    match s.split_once('/') {
        Some((k, t)) => (k.trim().to_owned(), decode_hex(t.trim())),
        None => (s.trim().to_owned(), s.trim().to_owned()),
    }
}

/// A PPD text with `<hex>` bytes (translation strings: "Qualit<E9>"): the bytes as UTF-8, else ISO 8859-1. PPD
/// text is untrusted (a driver's, a user's, a printer's): `<...>` is decoded only when it holds an even number of hex
/// digits once its spaces are gone, else its '<' stays as it is.
pub(crate) fn decode_hex(s: &str) -> String {
    if !s.contains('<') {
        return s.to_owned();
    }
    let mut bytes = Vec::new();
    let mut rest = s;
    while let Some(start) = rest.find('<') {
        bytes.extend_from_slice(rest[..start].as_bytes());
        let after = &rest[start + 1..];
        let decoded = after.find('>').and_then(|end| {
            let digits: Vec<u8> = after[..end].bytes().filter(|b| *b != b' ').collect();
            let hex = !digits.is_empty()
                && digits.len() % 2 == 0
                && digits.iter().all(u8::is_ascii_hexdigit);
            hex.then_some((digits, end))
        });
        match decoded {
            Some((digits, end)) => {
                for pair in digits.chunks(2) {
                    let digit = |b: u8| (b as char).to_digit(16).unwrap_or(0) as u8;
                    bytes.push(digit(pair[0]) * 16 + digit(pair[1]));
                }
                rest = &after[end + 1..];
            }
            None => {
                bytes.push(b'<');
                rest = after;
            }
        }
    }
    bytes.extend_from_slice(rest.as_bytes());
    match String::from_utf8(bytes.clone()) {
        Ok(s) => s,
        Err(_) => bytes.iter().map(|b| *b as char).collect(),
    }
}

/// Whether a `*lang.` prefix is the UI language's ("nl", "nl_NL", "nl-BE" for nl; "en_GB" / "en" for en).
fn lang_matches(prefix: &str, lang: &str) -> bool {
    let p = prefix.to_ascii_lowercase().replace('-', "_");
    p == lang || p.starts_with(&format!("{lang}_"))
}

/// Parse a PPD into its groups. `lang` is "nl" or "en": its translations replace the texts.
pub(crate) fn parse(ppd: &str, lang: &str) -> Vec<Group> {
    let mut groups: Vec<Group> = Vec::new();
    let mut group_stack: Vec<usize> = Vec::new();
    let mut open: Option<(usize, usize)> = None; // (group index, option index)
    let mut defaults: Vec<(String, String)> = Vec::new();
    let mut option_tr: Vec<(String, String)> = Vec::new(); // keyword → text (also groups)
    let mut choice_tr: Vec<(String, String, String)> = Vec::new(); // keyword, choice → text
    let mut in_quote = false;

    let general = |groups: &mut Vec<Group>| -> usize {
        if let Some(i) = groups.iter().position(|g| g.name == "General") {
            i
        } else {
            groups.push(Group {
                name: "General".into(),
                text: "General".into(),
                options: Vec::new(),
            });
            groups.len() - 1
        }
    };

    for raw in ppd.lines() {
        let line = raw.trim_end_matches('\r');
        if in_quote {
            // the rest of a value that spans lines
            if line.contains('"') {
                in_quote = false;
            }
            continue;
        }
        let Some(body) = line.strip_prefix('*') else {
            continue;
        };
        if body.starts_with('%') || body.starts_with('?') || body.starts_with("End") {
            continue;
        }
        // "*Main Option/Text: value": the main keyword, the option part and the value
        let (head, value) = match body.split_once(':') {
            Some((h, v)) => (h.trim(), v.trim()),
            None => (body.trim(), ""),
        };
        if value.starts_with('"') && value[1..].find('"').is_none() {
            in_quote = true;
        }
        let (main, option) = match head.split_once(char::is_whitespace) {
            Some((m, o)) => (m, o.trim()),
            None => (head, ""),
        };
        match main {
            "OpenGroup" => {
                let (name, text) = split_text(value);
                let i = match groups.iter().position(|g| g.name == name) {
                    Some(i) => i,
                    None => {
                        groups.push(Group {
                            name,
                            text,
                            options: Vec::new(),
                        });
                        groups.len() - 1
                    }
                };
                group_stack.push(i);
            }
            "CloseGroup" => {
                group_stack.pop();
            }
            "OpenUI" | "JCLOpenUI" => {
                let (keyword, text) = split_text(option.trim_start_matches('*'));
                let ui = match value.to_ascii_lowercase().as_str() {
                    "pickmany" => "pickmany",
                    "boolean" => "boolean",
                    _ => "pickone",
                };
                let g = match group_stack.last() {
                    Some(i) => *i,
                    None => general(&mut groups),
                };
                groups[g].options.push(Opt {
                    keyword,
                    text,
                    ui: ui.into(),
                    default: String::new(),
                    choices: Vec::new(),
                });
                open = Some((g, groups[g].options.len() - 1));
            }
            "CloseUI" | "JCLCloseUI" => open = None,
            _ if main.starts_with("Default") && main.len() > "Default".len() => {
                defaults.push((main["Default".len()..].to_owned(), value.to_owned()));
            }
            _ => {
                if let Some((prefix, rest)) = main.split_once('.')
                    && !prefix.is_empty()
                    && prefix
                        .chars()
                        .all(|c| c.is_ascii_alphabetic() || c == '_' || c == '-')
                {
                    // a translation: "*nl.Translation PageSize/Text" or "*nl.PageSize A4/Text"
                    if lang_matches(prefix, lang) {
                        let (key, text) = split_text(option);
                        if rest == "Translation" {
                            option_tr.push((key, text));
                        } else if !key.is_empty() {
                            choice_tr.push((rest.to_owned(), key, text));
                        }
                    }
                    continue;
                }
                // a choice of the open option: "*PageSize A4/A4: ..."
                if let Some((g, o)) = open
                    && groups[g].options[o].keyword == main
                    && !option.is_empty()
                {
                    let (value, text) = split_text(option);
                    let opt = &mut groups[g].options[o];
                    if !opt.choices.iter().any(|c| c.value == value) {
                        opt.choices.push(Choice { value, text });
                    }
                }
            }
        }
    }
    for group in &mut groups {
        if let Some((_, t)) = option_tr.iter().find(|(k, _)| *k == group.name) {
            group.text = t.clone();
        }
        for opt in &mut group.options {
            if let Some((_, d)) = defaults.iter().find(|(k, _)| *k == opt.keyword) {
                opt.default = d.clone();
            }
            if let Some((_, t)) = option_tr.iter().find(|(k, _)| *k == opt.keyword) {
                opt.text = t.clone();
            }
            for choice in &mut opt.choices {
                if let Some((_, _, t)) = choice_tr
                    .iter()
                    .find(|(k, c, _)| *k == opt.keyword && *c == choice.value)
                {
                    choice.text = t.clone();
                }
            }
        }
        group
            .options
            .retain(|o| !o.choices.is_empty() && !HIDDEN.contains(&o.keyword.as_str()));
    }
    groups.retain(|g| !g.options.is_empty());
    groups
}

/// The groups as JSON for `printers.get`, each option with its current value: the queue's `<Keyword>-default`
/// (what PrinterAddOptionDefault set, from `defaults`) when it names a choice, else the PPD's default.
pub(crate) fn to_json(groups: &[Group], defaults: &dyn Fn(&str) -> Option<String>) -> Value {
    Value::Array(
        groups
            .iter()
            .map(|g| {
                json!({
                    "name": g.name,
                    "text": g.text,
                    "options": g.options.iter().map(|o| {
                        let value = defaults(&o.keyword)
                            .filter(|v| o.choices.iter().any(|c| &c.value == v))
                            .unwrap_or_else(|| o.default.clone());
                        json!({
                            "name": o.keyword,
                            "text": o.text,
                            "ui": o.ui,
                            "value": value,
                            "choices": o.choices.iter().map(|c| json!({"value": c.value, "text": c.text})).collect::<Vec<_>>(),
                        })
                    }).collect::<Vec<_>>(),
                })
            })
            .collect(),
    )
}

/// Whether the PPD has an option (GNOME shows IPP's "sides" only when the PPD has no Duplex).
pub(crate) fn has_option(groups: &[Group], keyword: &str) -> bool {
    groups
        .iter()
        .any(|g| g.options.iter().any(|o| o.keyword == keyword))
}
