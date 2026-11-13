//! The original explorer_args grouping, limits and local-only URI rules.
use super::*;
use gio::prelude::*;
use std::collections::{BTreeMap, BTreeSet};
pub(super) struct Env {
    pub cwd: String,
    pub home: String,
    pub hostname: String,
    pub dirs: Option<BTreeSet<String>>,
    pub exists: BTreeSet<String>,
    pub remote: BTreeMap<String, String>,
    pub gio: bool,
}
impl Env {
    pub fn actual() -> Self {
        let mut hostname = [0i8; 256];
        unsafe {
            libc::gethostname(hostname.as_mut_ptr(), 255);
        }
        Self {
            cwd: std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("/"))
                .to_string_lossy()
                .into_owned(),
            home: glib::home_dir().to_string_lossy().into_owned(),
            hostname: unsafe { std::ffi::CStr::from_ptr(hostname.as_ptr()) }
                .to_string_lossy()
                .to_lowercase(),
            dirs: None,
            exists: BTreeSet::new(),
            remote: BTreeMap::new(),
            gio: true,
        }
    }
    fn isdir(&self, path: &str) -> bool {
        self.dirs
            .as_ref()
            .map(|d| d.contains(path))
            .unwrap_or_else(|| std::path::Path::new(path).is_dir())
    }
    fn exists(&self, path: &str) -> bool {
        if let Some(dirs) = self.dirs.as_ref() {
            dirs.contains(path) || self.exists.contains(path)
        } else {
            std::fs::symlink_metadata(path).is_ok()
        }
    }
}
#[derive(PartialEq)]
enum Kind {
    Path,
    Folder,
    Place,
}
fn pytrim(s: &str) -> &str {
    s.trim_matches(|c: char| c.is_whitespace() || matches!(c, '\u{1c}'..='\u{1f}'))
}
fn normalize(path: &str) -> String {
    let prefix = if path.starts_with("//") && !path.starts_with("///") {
        "//"
    } else {
        "/"
    };
    let mut parts = vec![];
    for p in path.split('/') {
        match p {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(p),
        }
    }
    format!("{prefix}{}", parts.join("/"))
}
fn parent(path: &str) -> String {
    let p = path.trim_end_matches('/');
    let p = p.rsplit_once('/').map(|(p, _)| p).unwrap_or("");
    if p.is_empty() { "/".into() } else { p.into() }
}
fn nearest(mut path: String, env: &Env) -> String {
    loop {
        if env.isdir(&path) {
            return path;
        }
        let next = parent(&path);
        if next == path {
            return "/".into();
        }
        path = next;
    }
}
fn scheme(s: &str) -> Option<(&str, &str)> {
    let (k, v) = s.split_once(':')?;
    let b = k.as_bytes();
    (!b.is_empty()
        && b[0].is_ascii_alphabetic()
        && b[1..]
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || [b'+', b'-', b'.'].contains(b)))
    .then_some((k, v))
}
fn file_path(tail: &str, env: &Env) -> Option<(Kind, String)> {
    let path = if let Some(tail) = tail.strip_prefix("//") {
        let end = tail.find(['/', '?', '#']).unwrap_or(tail.len());
        let authority = &tail[..end];
        let host = authority.rsplit('@').next().unwrap_or("");
        let host = if let Some(host) = host.strip_prefix('[') {
            host.split_once(']')?.0
        } else {
            host.split(':').next().unwrap_or("")
        }
        .to_lowercase();
        if !["", "localhost", "localhost.localdomain", "127.0.0.1", "::1"].contains(&host.as_str())
            && host != env.hostname
        {
            return None;
        }
        &tail[end..]
    } else {
        tail
    };
    let path = path.split(['?', '#']).next().unwrap_or("");
    let path = if path.is_empty() { "/" } else { path };
    if !path.starts_with('/') {
        return None;
    }
    let bytes = path.as_bytes();
    let mut raw = vec![];
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(a), Some(b)) = (
                (bytes[i + 1] as char).to_digit(16),
                (bytes[i + 2] as char).to_digit(16),
            ) {
                raw.push((a * 16 + b) as u8);
                i += 3;
                continue;
            }
        }
        raw.push(bytes[i]);
        i += 1;
    }
    let prefix = if raw.starts_with(b"//") && !raw.starts_with(b"///") {
        b"//".as_slice()
    } else {
        b"/".as_slice()
    };
    let mut segments: Vec<&[u8]> = vec![];
    for part in raw.split(|c| *c == b'/') {
        match part {
            b"" | b"." => {}
            b".." => {
                segments.pop();
            }
            _ => segments.push(part),
        }
    }
    let mut out = prefix.to_vec();
    for (i, p) in segments.iter().enumerate() {
        if i > 0 {
            out.push(b'/');
        }
        out.extend_from_slice(p);
    }
    match String::from_utf8(out) {
        Ok(p) => Some((Kind::Path, p)),
        Err(_) => {
            let mut valid = vec![];
            for p in segments {
                let Ok(p) = std::str::from_utf8(p) else {
                    break;
                };
                valid.push(p);
            }
            Some((Kind::Folder, format!("/{}", valid.join("/"))))
        }
    }
}
fn classify(raw: &str, env: &Env) -> Option<(Kind, String)> {
    let s = if raw.starts_with('/') {
        raw
    } else {
        pytrim(raw)
    };
    if s.is_empty() || s.chars().count() > 8192 || s.chars().any(|c| c < ' ') {
        return None;
    }
    if s.starts_with('/') {
        return Some((Kind::Path, normalize(s)));
    }
    if s == "~" || s.starts_with("~/") {
        return Some((Kind::Path, normalize(&format!("{}{}", env.home, &s[1..]))));
    }
    if let Some((key, tail)) = scheme(s) {
        let key = key.to_ascii_lowercase();
        let place = match key.as_str() {
            "trash" => Some("trash:///"),
            "computer" => Some("computer:///"),
            "recent" => Some("recent:///"),
            _ => None,
        };
        let forbidden = [
            "http",
            "https",
            "mailto",
            "ftp",
            "javascript",
            "data",
            "about",
            "tel",
            "sms",
            "magnet",
            "news",
            "irc",
            "callto",
            "x-scheme-handler",
        ]
        .contains(&key.as_str());
        if tail.starts_with("//") || place.is_some() || key == "file" || forbidden {
            if key == "file" {
                return file_path(tail, env);
            }
            if let Some(place) = place {
                return Some((Kind::Place, place.into()));
            }
            if forbidden {
                return None;
            }
            let local = env.remote.get(s).cloned().or_else(|| {
                if env.gio {
                    gio::File::for_uri(s)
                        .path()
                        .map(|p| p.to_string_lossy().into_owned())
                } else {
                    None
                }
            })?;
            return local
                .starts_with('/')
                .then(|| (Kind::Path, normalize(&local)));
        }
    }
    let p = normalize(&format!("{}/{s}", env.cwd));
    let b = s.as_bytes();
    let word = (1..=32).contains(&b.len())
        && b[0].is_ascii_alphabetic()
        && b[1..]
            .iter()
            .all(|b| b.is_ascii_alphabetic() || [b'\'', b'_', b'-'].contains(b));
    Some(if !env.exists(&p) && word {
        (Kind::Place, s.into())
    } else {
        (Kind::Path, p)
    })
}
pub(super) fn explorer(uris: &[String], items: bool, properties: bool, env: &Env) -> Option<Value> {
    let mut entries: Vec<(String, Vec<String>)> = vec![];
    let mut by_folder = BTreeMap::new();
    for raw in uris.iter().take(2000) {
        let Some((kind, val)) = classify(raw, env) else {
            continue;
        };
        let (folder, selected) = if kind == Kind::Place {
            (val, None)
        } else if kind == Kind::Folder {
            (
                if env.isdir(&val) {
                    val
                } else {
                    nearest(val, env)
                },
                None,
            )
        } else if !items && env.isdir(&val) {
            (val, None)
        } else if !items && !env.exists(&val) {
            (nearest(parent(&val), env), None)
        } else {
            let p = if val == "/" { "/".into() } else { parent(&val) };
            let folder = if env.isdir(&p) {
                p.clone()
            } else {
                nearest(p.clone(), env)
            };
            let selected = (val != "/" && folder == p).then_some(val);
            (folder, selected)
        };
        let index = *by_folder.entry(folder.clone()).or_insert_with(|| {
            let i = entries.len();
            entries.push((folder, vec![]));
            i
        });
        if let Some(sel) = selected {
            if !entries[index].1.contains(&sel) {
                entries[index].1.push(sel);
            }
        }
    }
    if entries.is_empty() {
        return None;
    }
    entries.truncate(20);
    for (_, sel) in &mut entries {
        sel.truncate(500);
    }
    let mut params = build(&entries, properties);
    while json_len(&params) > 48 * 1024 {
        if entries.len() > 1 {
            entries.pop();
        } else {
            let sel = &mut entries[0].1;
            if sel.is_empty() {
                break;
            }
            sel.truncate(sel.len() / 2);
        }
        params = build(&entries, properties);
    }
    Some(params)
}
fn build(entries: &[(String, Vec<String>)], properties: bool) -> Value {
    let one = |(path, select): &(String, Vec<String>)| {
        let mut v = json!({"path":path});
        if !select.is_empty() {
            v["select"] = json!(select);
        }
        v
    };
    let mut out = one(&entries[0]);
    if entries.len() > 1 {
        out["tabs"] = json!(entries.iter().skip(1).map(one).collect::<Vec<_>>());
    }
    if properties {
        out["properties"] = json!(true);
    }
    out
}
fn json_len(v: &Value) -> usize {
    match v {
        Value::Array(v) => {
            2 + v.iter().map(json_len).sum::<usize>() + v.len().saturating_sub(1) * 2
        }
        Value::Object(v) => {
            2 + v
                .iter()
                .map(|(k, v)| serde_json::to_string(k).unwrap().len() + 2 + json_len(v))
                .sum::<usize>()
                + v.len().saturating_sub(1) * 2
        }
        _ => v.to_string().len(),
    }
}
