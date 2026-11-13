//! Everything's original AND/OR/NOT, modifiers and bilingual range language.
use super::cache::{self, DEL, DIR, Data, HIDDEN, NOSHOW};
use crate::files::ApiError;
use serde_json::{Value, json};
use std::{
    borrow::Cow,
    ffi::{CStr, CString},
    ptr,
};
/// The parser's errors are API errors with a catalogue key (C2): the worker sends code, message, key and params,
/// and the host renders the message again from key + params in its own language (services/index.rs).
pub type Error = ApiError;
type Result<T> = std::result::Result<T, Error>;
struct Regex(*mut glib::ffi::GRegex);
unsafe impl Send for Regex {}
unsafe impl Sync for Regex {}
impl Drop for Regex {
    fn drop(&mut self) {
        unsafe { glib::ffi::g_regex_unref(self.0) }
    }
}
impl Regex {
    fn new(pattern: &str, case: bool, raw: bool) -> Result<Self> {
        let p = CString::new(pattern).map_err(|_| {
            ApiError::tr(
                "bad_regex",
                crate::i18n::current_language(),
                "index.query.regexNullByte",
                crate::i18n::NO_PARAMS,
            )
        })?;
        let mut err = ptr::null_mut();
        let flags = glib::ffi::G_REGEX_MULTILINE
            | if case { 0 } else { glib::ffi::G_REGEX_CASELESS }
            | if raw { glib::ffi::G_REGEX_RAW } else { 0 };
        let re = unsafe { glib::ffi::g_regex_new(p.as_ptr(), flags, 0, &mut err) };
        if re.is_null() {
            let msg = if err.is_null() {
                crate::i18n::tr(
                    crate::i18n::current_language(),
                    "index.query.invalidPattern",
                )
                .into()
            } else {
                let code = unsafe { (*err).code };
                let msg = unsafe { CStr::from_ptr((*err).message) }
                    .to_string_lossy()
                    .into_owned();
                unsafe { glib::ffi::g_error_free(err) };
                match code {
                    glib::ffi::G_REGEX_ERROR_UNTERMINATED_CHARACTER_CLASS => {
                        "unterminated character set".into()
                    }
                    glib::ffi::G_REGEX_ERROR_NOTHING_TO_REPEAT => "nothing to repeat".into(),
                    glib::ffi::G_REGEX_ERROR_STRAY_BACKSLASH => {
                        "bad escape (end of pattern)".into()
                    }
                    glib::ffi::G_REGEX_ERROR_UNMATCHED_PARENTHESIS => {
                        "missing ), unterminated subpattern".into()
                    }
                    glib::ffi::G_REGEX_ERROR_VARIABLE_LENGTH_LOOKBEHIND => {
                        "look-behind requires fixed-width pattern".into()
                    }
                    _ => msg.rsplit(": ").next().unwrap_or(&msg).to_string(),
                }
            };
            return Err(ApiError::tr(
                "bad_regex",
                crate::i18n::current_language(),
                "index.query.regexInvalid",
                &serde_json::json!({"detail": msg.to_string()}),
            ));
        }
        Ok(Self(re))
    }
    fn matches(&self, text: &[u8]) -> bool {
        unsafe {
            glib::ffi::g_regex_match_full(
                self.0,
                text.as_ptr().cast(),
                text.len() as isize,
                0,
                0,
                ptr::null_mut(),
                ptr::null_mut(),
            ) != 0
        }
    }
}
#[derive(Clone, Copy)]
struct Options {
    case: bool,
    path: bool,
    whole: bool,
    regex: bool,
}
pub struct Compiled {
    root: Node,
    pub highlight: Value,
}
enum Node {
    All,
    And(Vec<Node>),
    Or(Vec<Node>),
    Not(Box<Node>),
    Term {
        bytes: Vec<u8>,
        case: bool,
        path: bool,
        whole: bool,
        anchor: u8,
        glob: Option<Regex>,
        regex: bool,
        text: String,
    },
    Folder(bool),
    Category(u8),
    Ext(Vec<Vec<u8>>),
    Range {
        date: bool,
        lo: Option<i64>,
        hi: Option<i64>,
    },
    Parent {
        path: String,
        recursive: bool,
    },
    Empty,
}
impl Node {
    fn test(&self, data: &Data, g: usize) -> bool {
        let (size, modified, flags, cat) = data.fields(g);
        match self {
            Self::All => true,
            Self::And(items) => items.iter().all(|n| n.test(data, g)),
            Self::Or(items) => items.iter().any(|n| n.test(data, g)),
            Self::Not(n) => !n.test(data, g),
            Self::Folder(want) => (flags & DIR != 0) == *want,
            Self::Category(c) => cat == *c,
            Self::Ext(exts) => {
                flags & DIR == 0 && exts.iter().any(|e| data.name(g, true).ends_with(e))
            }
            Self::Range { date, lo, hi } => {
                let v = if *date { modified } else { size };
                lo.is_none_or(|x| v >= x) && hi.is_none_or(|x| v <= x)
            }
            Self::Parent { path, recursive } => {
                let p = data.parent(g);
                if p < 0 {
                    return false;
                }
                let parent = data.path(p as usize);
                if *recursive {
                    cache::beneath(&parent, path)
                } else {
                    parent == *path
                }
            }
            Self::Empty => flags & DIR != 0 && data.children(g).is_empty(),
            Self::Term {
                bytes,
                case,
                path,
                whole,
                anchor,
                glob,
                regex,
                ..
            } => {
                let mut target = if *path {
                    let p = data.path(g).into_bytes();
                    Cow::Owned(if *case || *regex { p } else { cache::lower(&p) })
                } else {
                    data.name_ref(g, !*case && !*regex)
                };
                if *regex && !*path && g >= data.n && target.contains(&0) {
                    target = Cow::Owned(cache::name_string(&target).into_bytes());
                }
                if let Some(rx) = glob {
                    return rx.matches(&target);
                }
                if bytes.is_empty() {
                    return true;
                }
                let mut from = 0;
                while from + bytes.len() <= target.len() {
                    let Some(i) = target[from..].windows(bytes.len()).position(|p| p == bytes)
                    else {
                        return false;
                    };
                    let at = from + i;
                    let end = at + bytes.len();
                    let anchored = match anchor {
                        1 => at == 0,
                        2 => end == target.len(),
                        3 => at == 0 && end == target.len(),
                        _ => true,
                    };
                    let word = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b >= 128;
                    if anchored
                        && (!*whole
                            || (at == 0 || !word(target[at - 1]))
                                && (end == target.len() || !word(target[end])))
                    {
                        return true;
                    }
                    from = at + 1
                }
                false
            }
        }
    }
    fn highlights(&self, out: &mut Vec<String>) {
        match self {
            Self::And(v) => {
                for n in v {
                    n.highlights(out)
                }
            }
            Self::Or(v) => {
                if !v.iter().any(Self::is_all) {
                    for n in v {
                        n.highlights(out)
                    }
                }
            }

            Self::Term {
                bytes,
                glob,
                regex,
                case,
                text,
                ..
            } if !*regex => {
                if glob.is_some() {
                    let p = if *case {
                        text.clone()
                    } else {
                        cache::name_string(&cache::lower(text.as_bytes()))
                    };
                    out.extend(
                        p.split(['*', '?'])
                            .filter(|s| !s.is_empty() && !s.contains('/'))
                            .map(str::to_string),
                    )
                } else if !bytes.contains(&b'/') {
                    out.push(cache::name_string(bytes))
                }
            }
            _ => {}
        }
    }
    fn is_all(&self) -> bool {
        matches!(self, Self::All) || matches!(self,Self::And(v)if v.len()==1&&v[0].is_all())
    }
    fn first_regex(&self) -> Option<String> {
        match self {
            Self::Term {
                regex: true, text, ..
            } => Some(text.clone()),
            Self::And(v) | Self::Or(v) => v.iter().find_map(Node::first_regex),
            _ => None,
        }
    }
}
fn alias(key: &str) -> Option<&'static str> {
    Some(match key.to_ascii_lowercase().as_str() {
        "ext" | "extensie" | "extension" => "ext",
        "size" | "grootte" => "size",
        "dm" | "datemodified" | "gewijzigd" | "datum" => "dm",
        "folder" | "map" | "dir" | "mappen" | "folders" => "folder",
        "file" | "bestand" | "files" | "bestanden" => "file",
        "path" | "pad" => "path",
        "nopath" => "nopath",
        "parent" | "infolder" | "inmap" => "parent",
        "in" | "under" | "onder" => "in",
        "regex" => "regex",
        "case" => "case",
        "nocase" => "nocase",
        "ww" | "wholeword" | "heelwoord" => "ww",
        "noww" | "nowholeword" => "noww",
        "empty" | "leeg" => "empty",
        "audio" | "muziek" | "music" => "cat:audio",
        "zip" | "compressed" | "archief" => "cat:compressed",
        "doc" | "document" | "documenten" => "cat:document",
        "exe" | "executable" | "programma" => "cat:executable",
        "pic" | "picture" | "image" | "afbeelding" | "afbeeldingen" => "cat:picture",
        "video" | "videos" => "cat:video",
        _ => return None,
    })
}
fn category(s: &str) -> Option<u8> {
    Some(match s {
        "audio" => 1,
        "compressed" => 2,
        "document" => 3,
        "executable" => 4,
        "picture" => 5,
        "video" => 6,
        _ => return None,
    })
}
struct Parser {
    input: Vec<char>,
    i: usize,
    depth: usize,
    opts: Options,
}
impl Parser {
    fn ws(&mut self) {
        while self.i < self.input.len() && " \t\r\n".contains(self.input[self.i]) {
            self.i += 1
        }
    }
    fn and(&mut self) -> Result<Node> {
        let mut items = Vec::new();
        loop {
            self.ws();
            if self.i >= self.input.len() || self.input[self.i] == '>' && self.depth > 0 {
                break;
            }
            if self.input[self.i] == '|' {
                self.i += 1;
                continue;
            }
            if let Some(n) = self.or()? {
                items.push(n)
            }
        }
        Ok(Node::And(items))
    }
    fn or(&mut self) -> Result<Option<Node>> {
        let mut items = Vec::new();
        if let Some(n) = self.unary()? {
            items.push(n)
        }
        loop {
            let mut j = self.i;
            while j < self.input.len() && " \t\r\n".contains(self.input[j]) {
                j += 1
            }
            if j >= self.input.len() || self.input[j] != '|' {
                break;
            }
            self.i = j + 1;
            self.ws();
            if self.i >= self.input.len() || self.input[self.i] == '>' && self.depth > 0 {
                break;
            }
            if let Some(n) = self.unary()? {
                items.push(n)
            }
        }
        Ok(if items.is_empty() {
            None
        } else if items.len() == 1 {
            items.pop()
        } else {
            Some(Node::Or(items))
        })
    }
    fn unary(&mut self) -> Result<Option<Node>> {
        if self.i >= self.input.len() {
            return Ok(None);
        }
        match self.input[self.i] {
            '!' => {
                self.i += 1;
                Ok(self.unary()?.map(|n| Node::Not(Box::new(n))))
            }
            '<' => {
                self.i += 1;
                self.depth += 1;
                if self.depth > 128 {
                    return Err(ApiError::tr(
                        "bad_query",
                        crate::i18n::current_language(),
                        "index.query.tooLong",
                        crate::i18n::NO_PARAMS,
                    ));
                }
                let n = self.and()?;
                if self.i < self.input.len() && self.input[self.i] == '>' {
                    self.i += 1
                }
                self.depth -= 1;
                Ok(if matches!(&n,Node::And(v)if v.is_empty()) {
                    None
                } else {
                    Some(n)
                })
            }
            _ => {
                let start = self.i;
                let mut text = String::new();
                let mut quoted = false;
                let mut inq = false;
                while self.i < self.input.len() {
                    let c = self.input[self.i];
                    if c == '"' {
                        inq = !inq;
                        quoted = true;
                        self.i += 1;
                        continue;
                    }
                    if !inq && (" \t\r\n|".contains(c) || c == '>' && self.depth > 0) {
                        break;
                    }
                    text.push(c);
                    self.i += 1
                }
                if start == self.i {
                    self.i += 1;
                    return Ok(None);
                }
                let raw = self.input[start..self.i].iter().collect::<String>();
                make(&raw, &text, quoted, self.opts)
            }
        }
    }
}
fn make(raw: &str, text: &str, _quoted: bool, mut opts: Options) -> Result<Option<Node>> {
    if let Some((key, rest)) = raw.split_once(':') {
        if key.bytes().all(|b| b.is_ascii_alphabetic()) {
            if let Some(kind) = alias(key) {
                let value = text
                    .get(key.len() + 1..)
                    .unwrap_or_else(|| text.split_once(':').map(|p| p.1).unwrap_or(""));
                if [
                    "folder", "file", "path", "nopath", "regex", "case", "nocase", "ww", "noww",
                ]
                .contains(&kind)
                    || kind.starts_with("cat:")
                {
                    match kind {
                        "case" => opts.case = true,
                        "nocase" => opts.case = false,
                        "path" => opts.path = true,
                        "nopath" => opts.path = false,
                        "ww" => opts.whole = true,
                        "noww" => opts.whole = false,
                        "regex" => opts.regex = true,
                        _ => {}
                    }
                    let inner = if rest.is_empty() {
                        None
                    } else if opts.regex {
                        Some(term(value, opts)?)
                    } else {
                        make(rest, value, rest.contains('"'), opts)?
                    };
                    let modifier = match kind {
                        "folder" => Some(Node::Folder(true)),
                        "file" => Some(Node::Folder(false)),
                        _ => kind
                            .strip_prefix("cat:")
                            .and_then(category)
                            .map(Node::Category),
                    };
                    return Ok(match (modifier, inner) {
                        (Some(a), Some(b)) => Some(Node::And(vec![a, b])),
                        (Some(a), None) | (None, Some(a)) => Some(a),
                        _ => None,
                    });
                }
                return Ok(Some(match kind {
                    "ext" => {
                        let exts = value
                            .split([';', ',', '|'])
                            .map(|s| {
                                s.trim()
                                    .to_lowercase()
                                    .trim_start_matches('*')
                                    .trim_start_matches('.')
                                    .to_string()
                            })
                            .filter(|s| !s.is_empty())
                            .map(|s| format!(".{s}").into_bytes())
                            .collect::<Vec<_>>();
                        if exts.is_empty() {
                            return Err(ApiError::tr(
                                "bad_query",
                                crate::i18n::current_language(),
                                "index.query.extMissing",
                                crate::i18n::NO_PARAMS,
                            ));
                        }
                        Node::Ext(exts)
                    }
                    "size" => {
                        let (lo, hi) = parse_size(value)?;
                        Node::Range {
                            date: false,
                            lo,
                            hi,
                        }
                    }
                    "dm" => {
                        let (lo, hi) = parse_date(value)?;
                        Node::Range {
                            date: true,
                            lo,
                            hi: hi.map(|n| n - 1),
                        }
                    }
                    "parent" | "in" => {
                        if value.trim().is_empty() {
                            return Err(ApiError::tr(
                                "bad_query",
                                crate::i18n::current_language(),
                                "index.query.folderMissing",
                                crate::i18n::NO_PARAMS,
                            ));
                        }
                        Node::Parent {
                            path: cache::normalize(value),
                            recursive: kind == "in",
                        }
                    }
                    "empty" => Node::Empty,
                    _ => {
                        return Err(ApiError::tr(
                            "bad_query",
                            crate::i18n::current_language(),
                            "index.query.unknownFunction",
                            &serde_json::json!({"name": kind.to_string()}),
                        ));
                    }
                }));
            }
        }
    }
    if text.is_empty() {
        Ok(None)
    } else {
        Ok(Some(term(text, opts)?))
    }
}
fn escaped(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        if ".^$*+?{}[]\\|()".contains(c) {
            out.push('\\')
        }
        out.push(c)
    }
    out
}
fn too_complex(p: &str) -> bool {
    if p.chars().count() > 400 {
        return true;
    }
    static NESTED: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    static BACKREF: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    NESTED
        .get_or_init(|| {
            Regex::new(
                r"\((?:[^()\\]|\\.)*(?:[+*]|\{\d*,?\d*\})(?:[^()\\]|\\.)*\)(?:[+*]|\{\d*,?\d*\})",
                true,
                true,
            )
            .expect("fixed regex")
        })
        .matches(p.as_bytes())
        || BACKREF
            .get_or_init(|| Regex::new(r"\\[1-9]|\(\?P=", true, true).expect("fixed regex"))
            .matches(p.as_bytes())
}

fn term(text: &str, opts: Options) -> Result<Node> {
    let bytes = if opts.case {
        text.as_bytes().to_vec()
    } else {
        cache::lower(text.as_bytes())
    };
    if opts.regex {
        if too_complex(text) {
            return Err(ApiError::tr(
                "regex_too_complex",
                crate::i18n::current_language(),
                "index.query.regexTooComplex",
                crate::i18n::NO_PARAMS,
            ));
        }
        return Ok(Node::Term {
            bytes: Vec::new(),
            case: opts.case,
            path: opts.path,
            whole: false,
            anchor: 0,
            glob: Some(Regex::new(text, opts.case, text.is_ascii())?),
            regex: true,
            text: text.into(),
        });
    }
    if text.contains(['*', '?']) {
        if text.chars().all(|c| c == '*') {
            return Ok(Node::All);
        }
        let core = bytes.strip_prefix(b"*").unwrap_or(&bytes);
        let core = core.strip_suffix(b"*").unwrap_or(core);
        if !bytes.contains(&b'?')
            && !opts.whole
            && !core.contains(&b'*')
            && !(opts.path && core.contains(&b'/'))
        {
            return Ok(Node::Term {
                bytes: core.to_vec(),
                case: opts.case,
                path: opts.path,
                whole: false,
                anchor: match (bytes.starts_with(b"*"), bytes.ends_with(b"*")) {
                    (true, true) => 0,
                    (true, false) => 2,
                    _ => 1,
                },
                glob: None,
                regex: false,
                text: text.into(),
            });
        }
        let lowtext = String::from_utf8_lossy(&bytes);
        let mut pattern = String::from("\\A(?:");
        for c in lowtext.chars() {
            match c {
                '*' => pattern.push_str("[\\s\\S]*"),
                '?' => pattern.push_str("(?:[^\\n\\x80-\\xff]|[\\xc0-\\xff][\\x80-\\xbf]+)"),
                _ => pattern.push_str(&escaped(&c.to_string())),
            }
        }
        pattern.push_str(")\\z");
        return Ok(Node::Term {
            bytes,
            case: opts.case,
            path: opts.path,
            whole: false,
            anchor: 0,
            glob: Some(Regex::new(&pattern, true, true)?),
            regex: false,
            text: text.into(),
        });
    }
    Ok(Node::Term {
        bytes,
        case: opts.case,
        path: opts.path,
        whole: opts.whole,
        anchor: 0,
        glob: None,
        regex: false,
        text: text.into(),
    })
}
impl Compiled {
    pub fn new(query: &str, args: &Value) -> Result<Self> {
        let query = query.trim();
        if query.chars().count() > 4000 {
            return Err(ApiError::tr(
                "bad_query",
                crate::i18n::current_language(),
                "index.query.tooLong",
                crate::i18n::NO_PARAMS,
            ));
        }
        let opts = Options {
            case: truth(&args["matchCase"]),
            path: truth(&args["matchPath"]),
            whole: truth(&args["wholeWord"]),
            regex: truth(&args["regex"]),
        };
        let root = if opts.regex && !query.is_empty() {
            term(query, opts)?
        } else {
            Parser {
                input: query.chars().collect(),
                i: 0,
                depth: 0,
                opts,
            }
            .and()?
        };
        let mut hl = Vec::new();
        root.highlights(&mut hl);
        hl.sort_by_key(|s| std::cmp::Reverse(s.chars().count()));
        let mut seen = std::collections::HashSet::new();
        hl.retain(|s| {
            !s.is_empty()
                && seen.insert(if opts.case {
                    s.clone()
                } else {
                    s.to_lowercase()
                })
        });
        let regex = root.first_regex();
        let filter = args["filter"]
            .as_str()
            .filter(|s| !s.is_empty())
            .unwrap_or("all");
        let root = if filter == "all" {
            root
        } else {
            let n = if filter == "folder" {
                Node::Folder(true)
            } else if let Some(c) = category(filter) {
                Node::Category(c)
            } else {
                return Err(ApiError::tr(
                    "bad_query",
                    crate::i18n::current_language(),
                    "index.query.unknownFilter",
                    crate::i18n::NO_PARAMS,
                ));
            };
            Node::And(vec![root, n])
        };
        Ok(Self {
            root,
            highlight: json!({"terms":hl,"regex":regex,"case":opts.case}),
        })
    }
    pub fn test(&self, data: &Data, g: usize, hidden: bool) -> bool {
        let f = data.fields(g).2;
        f & (DEL | NOSHOW | if hidden { 0 } else { HIDDEN }) == 0 && self.root.test(data, g)
    }
}
pub fn truth(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|x| x != 0.),
        Value::String(s) => !s.is_empty(),
        Value::Array(v) => !v.is_empty(),
        Value::Object(v) => !v.is_empty(),
    }
}
pub fn integer(v: &Value, default: i64) -> Result<i64> {
    match v {
        Value::Null => Ok(default),
        Value::Bool(b) => Ok(i64::from(*b)),
        Value::Number(n) => n.as_f64().map(|n| n as i64).ok_or_else(|| {
            ApiError::tr(
                "bad_args",
                crate::i18n::current_language(),
                "index.badArgs",
                crate::i18n::NO_PARAMS,
            )
        }),
        Value::String(s) => s.trim().parse::<i64>().map_err(|_| {
            ApiError::tr(
                "bad_args",
                crate::i18n::current_language(),
                "index.badArgs",
                crate::i18n::NO_PARAMS,
            )
        }),
        _ => Err(ApiError::tr(
            "bad_args",
            crate::i18n::current_language(),
            "index.badArgs",
            crate::i18n::NO_PARAMS,
        )),
    }
}
fn size_value(s: &str) -> Result<i64> {
    let text = s.trim().to_lowercase();
    let split = text
        .find(|c: char| !c.is_ascii_digit() && c != '.' && c != ',')
        .unwrap_or(text.len());
    let number = text[..split]
        .parse::<f64>()
        .or_else(|_| text[..split].replace(',', ".").parse::<f64>());
    let unit = text[split..].trim();
    let mul = match unit {
        "" | "b" | "byte" | "bytes" => Some(1u64),
        "k" | "kb" | "kib" => Some(1 << 10),
        "m" | "mb" | "mib" => Some(1 << 20),
        "g" | "gb" | "gib" => Some(1 << 30),
        "t" | "tb" | "tib" => Some(1 << 40),
        _ => None,
    };
    match (number, mul) {
        (Ok(n), Some(m)) if n >= 0. => Ok((n * m as f64) as i64),
        _ => Err(ApiError::tr(
            "bad_query",
            crate::i18n::current_language(),
            "index.query.sizeValue",
            &serde_json::json!({"value": s.to_string()}),
        )),
    }
}
fn parse_size(v: &str) -> Result<(Option<i64>, Option<i64>)> {
    let v = v.trim().to_lowercase();
    if v.is_empty() {
        return Err(ApiError::tr(
            "bad_query",
            crate::i18n::current_language(),
            "index.query.sizeMissing",
            crate::i18n::NO_PARAMS,
        ));
    }
    let word = match v.as_str() {
        "empty" | "leeg" => Some((Some(0), Some(0))),
        "tiny" | "piepklein" => Some((Some(1), Some(10 << 10))),
        "small" | "klein" => Some((Some((10 << 10) + 1), Some(100 << 10))),
        "medium" | "middel" | "gemiddeld" => Some((Some((100 << 10) + 1), Some(1 << 20))),
        "large" | "groot" => Some((Some((1 << 20) + 1), Some(16 << 20))),
        "huge" | "enorm" => Some((Some((16 << 20) + 1), Some(128 << 20))),
        "gigantic" | "gigantisch" => Some((Some((128 << 20) + 1), None)),
        _ => None,
    };
    if let Some(v) = word {
        return Ok(v);
    }
    for sep in ["..", "-"] {
        if let Some((a, b)) = v.split_once(sep) {
            if !a.is_empty() {
                return Ok((Some(size_value(a)?), Some(size_value(b)?)));
            }
        }
    }
    for op in [">=", "<=", ">", "<", "="] {
        if let Some(rest) = v.strip_prefix(op) {
            let n = size_value(rest)?;
            return Ok(match op {
                ">=" => (Some(n), None),
                "<=" => (None, Some(n)),
                ">" => (Some(n.saturating_add(1)), None),
                "<" => (None, Some(n.saturating_sub(1))),
                _ => (Some(n), Some(n)),
            });
        }
    }
    let n = size_value(&v)?;
    Ok((Some(n), Some(n)))
}
fn local() -> libc::tm {
    let t = (cache::now() / 1000.) as libc::time_t;
    let mut tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&t, &mut tm) };
    tm
}
fn stamp(y: i32, m: i32, d: i32) -> i64 {
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    tm.tm_year = y - 1900;
    tm.tm_mon = m - 1;
    tm.tm_mday = d;
    tm.tm_isdst = -1;
    unsafe { libc::mktime(&mut tm) as i64 * 1000 }
}
fn date_word(w: &str) -> Option<(i64, i64)> {
    let t = local();
    let (y, m, d) = (t.tm_year + 1900, t.tm_mon + 1, t.tm_mday);
    let today = stamp(y, m, d);
    let tomorrow = stamp(y, m, d + 1);
    let now = cache::now() as i64;
    let next = now + 86400000;
    Some(match w {
        "today" | "vandaag" => (today, tomorrow),
        "yesterday" | "gisteren" => (stamp(y, m, d - 1), today),
        "thisweek" | "dezeweek" => (stamp(y, m, d - (t.tm_wday + 6) % 7), tomorrow),
        "lastweek" | "vorigeweek" => {
            let day = d - (t.tm_wday + 6) % 7;
            (stamp(y, m, day - 7), stamp(y, m, day))
        }
        "thismonth" | "dezemaand" => (stamp(y, m, 1), next),
        "lastmonth" | "vorigemaand" => (stamp(y, m - 1, 1), stamp(y, m, 1)),
        "thisyear" | "ditjaar" | "dit-jaar" => (stamp(y, 1, 1), next),
        "lastyear" | "vorigjaar" | "vorig-jaar" => (stamp(y - 1, 1, 1), stamp(y, 1, 1)),
        _ => {
            let s = w
                .strip_prefix("last")
                .or_else(|| w.strip_prefix("afgelopen"))
                .unwrap_or(w);
            let pos = s.find(|c: char| !c.is_ascii_digit())?;
            let n = s[..pos].parse::<i64>().ok()?;
            let unit = &s[pos..];
            let delta = match unit {
                "hour" | "hours" | "uur" => n * 3600000,
                "day" | "days" | "dagen" => return Some((stamp(y, m, d - (n - 1) as i32), next)),
                "week" | "weeks" | "weken" => n * 7 * 86400000,
                "month" | "months" | "maanden" => (n as f64 * 30.44 * 86400000.) as i64,
                "year" | "years" | "jaar" | "jaren" => (n as f64 * 365.25 * 86400000.) as i64,
                _ => return None,
            };
            (now - delta, next)
        }
    })
}
fn date_point(s: &str) -> Result<(i64, i64)> {
    let s = s.trim();
    if let Some(v) = date_word(&s.to_lowercase()) {
        return Ok(v);
    }
    let parts = s
        .split(['-', '/', '.'])
        .map(str::parse::<i32>)
        .collect::<std::result::Result<Vec<_>, _>>();
    if let Ok(v) = parts {
        let (y, m, d) = match v.as_slice() {
            [y] if s.len() == 4 => return Ok((stamp(*y, 1, 1), stamp(*y + 1, 1, 1))),
            [y, m] if s.starts_with(&format!("{y:04}")) => {
                return Ok((stamp(*y, *m, 1), stamp(*y, *m + 1, 1)));
            }
            [y, m, d]
                if s.split(['-', '/', '.'])
                    .next()
                    .is_some_and(|s| s.len() == 4) =>
            {
                (*y, *m, *d)
            }
            [d, m, y] => (*y, *m, *d),
            _ => {
                return Err(ApiError::tr(
                    "bad_query",
                    crate::i18n::current_language(),
                    "index.query.dateValueHint",
                    &serde_json::json!({"value": s.to_string()}),
                ));
            }
        };
        if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
            return Err(ApiError::tr(
                "bad_query",
                crate::i18n::current_language(),
                "index.query.dateValue",
                &serde_json::json!({"value": s.to_string()}),
            ));
        }
        return Ok((stamp(y, m, d), stamp(y, m, d + 1)));
    }
    Err(ApiError::tr(
        "bad_query",
        crate::i18n::current_language(),
        "index.query.dateValueHint",
        &serde_json::json!({"value": s.to_string()}),
    ))
}
fn parse_date(v: &str) -> Result<(Option<i64>, Option<i64>)> {
    let v = v.trim().to_lowercase();
    if v.is_empty() {
        return Err(ApiError::tr(
            "bad_query",
            crate::i18n::current_language(),
            "index.query.dateMissing",
            crate::i18n::NO_PARAMS,
        ));
    }
    if let Some((a, b)) = date_word(&v) {
        return Ok((Some(a), Some(b)));
    }
    if let Some((a, b)) = v.split_once("..") {
        return Ok((
            if a.is_empty() {
                None
            } else {
                Some(date_point(a)?.0)
            },
            if b.is_empty() {
                None
            } else {
                Some(date_point(b)?.1)
            },
        ));
    }
    for op in [">=", "<=", ">", "<", "="] {
        if let Some(rest) = v.strip_prefix(op) {
            let (a, b) = date_point(rest)?;
            return Ok(match op {
                ">=" => (Some(a), None),
                ">" => (Some(b), None),
                "<=" => (None, Some(b)),
                "<" => (None, Some(a)),
                _ => (Some(a), Some(b)),
            });
        }
    }
    let (a, b) = date_point(&v)?;
    Ok((Some(a), Some(b)))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ranges_and_bilingual() {
        assert_eq!(parse_size(">10mb").unwrap(), (Some((10 << 20) + 1), None));
        assert_eq!(
            parse_size("klein").unwrap(),
            (Some((10 << 10) + 1), Some(100 << 10))
        );
        assert!(parse_date("gisteren").unwrap().0 < parse_date("vandaag").unwrap().0);
    }
    #[test]
    fn grammar_and_highlights() {
        let c = Compiled::new("map:foo | bar !regex:baz ext:txt;md", &json!({})).unwrap();
        assert_eq!(c.highlight["terms"], json!(["foo", "bar"]));
        assert!(c.highlight["regex"].is_null());
        assert!(Compiled::new("(a+)+", &json!({"regex":true})).is_err());
    }
    // R2-B: the examples the (translated) messages give are input the parser accepts in both languages (C9)
    #[test]
    fn the_examples_of_the_messages_parse() {
        for q in [
            "dm:vandaag",
            "dm:today",
            "dm:2026-10-01..2026-10-25",
            "size:>10mb",
            "ext:jpg",
            "ext:mp3;flac",
            "parent:/home/robin/Downloads",
        ] {
            assert!(Compiled::new(q, &json!({})).is_ok(), "{q}");
        }
        let e = Compiled::new("dm:", &json!({})).err().unwrap();
        assert_eq!(e.key.as_deref(), Some("index.query.dateMissing"));
        assert_eq!(
            crate::i18n::tr(crate::i18n::Lang::En, "index.query.dateMissing"),
            "Enter a date, for example dm:today."
        );
    }
    #[test]
    fn unicode_preserving_lower() {
        assert_eq!(cache::lower("İΣStraße".as_bytes()), "İσstraße".as_bytes());
    }
}
