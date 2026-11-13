//! Freedesktop theme lookup, matching the original resolver without GTK main-thread state.
use super::mime;
use gio::prelude::*;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};
const EXTS: [&str; 3] = ["svg", "png", "xpm"];
const GLYPHS: [&str; 6] = [
    "actions",
    "animations",
    "emblems",
    "emotes",
    "intl",
    "international",
];
fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}
fn data() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".local/share"))
}
fn ini(path: &Path) -> HashMap<String, HashMap<String, String>> {
    let mut out = HashMap::<String, HashMap<String, String>>::new();
    let mut section = None;
    for raw in String::from_utf8_lossy(&std::fs::read(path).unwrap_or_default()).lines() {
        let l = raw.trim();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        if l.starts_with('[') && l.ends_with(']') {
            let s = l[1..l.len() - 1].trim().to_string();
            out.entry(s.clone()).or_default();
            section = Some(s)
        } else if let (Some(s), Some((k, v))) = (&section, l.split_once('=')) {
            out.entry(s.clone())
                .or_default()
                .entry(k.trim().into())
                .or_insert_with(|| v.trim().into());
        }
    }
    out
}
fn num(p: &HashMap<String, String>, k: &str, d: i64) -> i64 {
    p.get(k).and_then(|x| x.trim().parse().ok()).unwrap_or(d)
}
struct Dir {
    rel: String,
    size: i64,
    scale: i64,
    typ: String,
    min: i64,
    max: i64,
    glyph: bool,
}
impl Dir {
    fn new(rel: String, p: &HashMap<String, String>) -> Self {
        let context = p
            .get("Context")
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| {
                rel.to_lowercase()
                    .split('/')
                    .find(|s| GLYPHS.contains(s))
                    .unwrap_or("")
                    .into()
            });
        let size = num(p, "Size", 48).max(1);
        Self {
            rel,
            size,
            scale: num(p, "Scale", 1).max(1),
            typ: p
                .get("Type")
                .map(|s| s.trim().to_lowercase())
                .unwrap_or("threshold".into()),
            min: num(p, "MinSize", size),
            max: num(p, "MaxSize", size),
            glyph: GLYPHS.contains(&context.as_str()),
        }
    }
    fn score(&self, ext: &str, size: i64) -> (i64, i64, i64, i64) {
        let hi = i64::from(self.scale != 1);
        let eff = self.size.saturating_mul(self.scale);
        if ext == "svg" {
            if self.typ == "scalable" {
                return (
                    0,
                    i64::from(
                        !(self.min.saturating_mul(self.scale) <= size
                            && size <= self.max.saturating_mul(self.scale)),
                    ),
                    hi,
                    (eff - size).abs(),
                );
            }
            return (1, (eff - size).abs(), hi, 0);
        }
        let tier = if ext == "png" { 2 } else { 4 };
        if eff >= size {
            (tier, eff - size, hi, 0)
        } else {
            (tier + 1, size - eff, hi, 0)
        }
    }
}
struct Theme {
    name: String,
    bases: Vec<PathBuf>,
    dirs: Vec<Dir>,
    parents: Vec<String>,
    valid: bool,
    index: Option<HashMap<String, Vec<(usize, usize, usize)>>>,
    mtimes: Vec<(PathBuf, std::time::SystemTime)>,
    checked: Instant,
}
impl Theme {
    fn new(name: &str, bases: &[PathBuf]) -> Self {
        let bases = bases
            .iter()
            .filter(|b| b.join(name).is_dir())
            .cloned()
            .collect::<Vec<_>>();
        let index = bases
            .iter()
            .map(|b| b.join(name).join("index.theme"))
            .find(|p| p.is_file());
        let mut dirs = Vec::new();
        let mut parents = Vec::new();
        if let Some(p) = &index {
            let sections = ini(p);
            if let Some(h) = sections.get("Icon Theme") {
                parents = h
                    .get("Inherits")
                    .into_iter()
                    .flat_map(|v| v.split(','))
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(String::from)
                    .collect();
                let mut seen = HashSet::new();
                for key in ["Directories", "ScaledDirectories"] {
                    for rel in h
                        .get(key)
                        .into_iter()
                        .flat_map(|s| s.split(','))
                        .map(str::trim)
                    {
                        if !rel.is_empty()
                            && seen.insert(rel.to_string())
                            && let Some(props) = sections.get(rel)
                        {
                            dirs.push(Dir::new(rel.into(), props));
                        }
                    }
                }
            }
        }
        Self {
            name: name.into(),
            bases,
            dirs,
            parents,
            valid: index.is_some(),
            index: None,
            mtimes: Vec::new(),
            checked: Instant::now(),
        }
    }
    fn build(&mut self) {
        let mut index = HashMap::<String, Vec<(usize, usize, usize)>>::new();
        self.mtimes.clear();
        for (di, d) in self.dirs.iter().enumerate() {
            for (bi, b) in self.bases.iter().enumerate() {
                let p = b.join(&self.name).join(&d.rel);
                if let Ok(mt) = std::fs::metadata(&p).and_then(|m| m.modified()) {
                    self.mtimes.push((p.clone(), mt))
                }
                if let Ok(entries) = std::fs::read_dir(p) {
                    for e in entries.flatten() {
                        let n = e.file_name().to_string_lossy().into_owned();
                        if let Some((stem, ext)) = n.rsplit_once('.')
                            && !stem.is_empty()
                            && let Some(ei) = EXTS.iter().position(|s| *s == ext.to_lowercase())
                        {
                            index.entry(stem.into()).or_default().push((di, bi, ei));
                        }
                    }
                }
            }
        }
        self.index = Some(index);
        self.checked = Instant::now();
    }
    fn lookup(&mut self, name: &str, size: i64, refresh: bool, glyph: bool) -> Option<PathBuf> {
        if !self.valid {
            return None;
        }
        let rebuild = self.index.is_none()
            || (refresh && self.checked.elapsed() > Duration::from_secs(30) && {
                self.checked = Instant::now();
                self.mtimes.iter().any(|(p, t)| {
                    std::fs::metadata(p)
                        .and_then(|m| m.modified())
                        .ok()
                        .as_ref()
                        != Some(t)
                })
            });
        if rebuild {
            self.build()
        }
        let mut ranked = Vec::new();
        for &(di, bi, ei) in self.index.as_ref()?.get(name)? {
            let d = &self.dirs[di];
            if d.glyph != glyph {
                continue;
            }
            ranked.push((
                d.score(EXTS[ei], size),
                self.bases[bi]
                    .join(&self.name)
                    .join(&d.rel)
                    .join(format!("{name}.{}", EXTS[ei])),
            ));
        }
        ranked.sort_by_key(|x| x.0);
        ranked.into_iter().map(|x| x.1).find(|p| p.is_file())
    }
}
#[derive(Default)]
struct Lookup {
    themes: HashMap<String, Theme>,
    bases: Option<(Instant, Vec<PathBuf>)>,
    theme: Option<(Instant, String)>,
    cache: VecDeque<(String, Option<PathBuf>, Instant)>,
}
impl Lookup {
    fn bases(&mut self) -> Vec<PathBuf> {
        if let Some((t, v)) = &self.bases
            && t.elapsed() < Duration::from_secs(60)
        {
            return v.clone();
        }
        let mut c = vec![data().join("icons"), home().join(".icons")];
        c.extend(
            std::env::var("XDG_DATA_DIRS")
                .unwrap_or("/usr/local/share:/usr/share".into())
                .split(':')
                .filter(|d| Path::new(d).is_absolute())
                .map(|d| Path::new(d).join("icons")),
        );
        c.extend([
            PathBuf::from("/usr/share/icons"),
            PathBuf::from("/var/lib/flatpak/exports/share/icons"),
            data().join("flatpak/exports/share/icons"),
            PathBuf::from("/var/lib/snapd/desktop/icons"),
        ]);
        let mut seen = HashSet::new();
        c.retain(|p| {
            p.canonicalize()
                .ok()
                .is_some_and(|r| r.is_dir() && seen.insert(r))
        });
        self.bases = Some((Instant::now(), c.clone()));
        c
    }
    fn current(&mut self) -> String {
        if let Ok(t) = std::env::var("WIN11SHELL_ICON_THEME")
            && !t.is_empty()
        {
            return t;
        }
        if let Some((time, name)) = &self.theme
            && time.elapsed() < Duration::from_secs(5)
        {
            return name.clone();
        }
        let mut name = None;
        if let Some(schema) = gio::SettingsSchemaSource::default()
            .and_then(|s| s.lookup("org.gnome.desktop.interface", true))
        {
            let settings = gio::Settings::new_full(&schema, None::<&gio::SettingsBackend>, None);
            let s = settings.string("icon-theme");
            if !s.is_empty() {
                name = Some(s.to_string())
            }
        }
        let name = name
            .or_else(|| {
                let p = std::env::var_os("XDG_CONFIG_HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| home().join(".config"));
                ini(&p.join("gtk-3.0/settings.ini"))
                    .get("Settings")
                    .and_then(|s| s.get("gtk-icon-theme-name"))
                    .cloned()
            })
            .unwrap_or("hicolor".into());
        self.theme = Some((Instant::now(), name.clone()));
        name
    }
    fn chain_add(
        &mut self,
        name: &str,
        depth: usize,
        seen: &mut HashSet<String>,
        out: &mut Vec<String>,
    ) {
        if depth > 10 || !seen.insert(name.into()) {
            return;
        }
        if !self.themes.contains_key(name) {
            let bases = self.bases();
            self.themes.insert(name.into(), Theme::new(name, &bases));
        }
        let t = &self.themes[name];
        if !t.valid {
            return;
        }
        out.push(name.into());
        let parents = t.parents.clone();
        for p in parents {
            if p != "hicolor" {
                self.chain_add(&p, depth + 1, seen, out)
            }
        }
    }
    fn flat(&mut self, name: &str) -> Option<PathBuf> {
        let mut bases = self.bases();
        if Path::new("/usr/share/pixmaps").is_dir() {
            bases.push("/usr/share/pixmaps".into())
        }
        for d in bases {
            for e in EXTS {
                let p = d.join(format!("{name}.{e}"));
                if p.is_file() {
                    return Some(p);
                }
            }
        }
        None
    }
    fn one(&mut self, name: &str, size: i64, chain: &[String], refresh: bool) -> Option<PathBuf> {
        for t in chain {
            if let Some(p) = self.themes.get_mut(t)?.lookup(name, size, refresh, false) {
                return Some(p);
            }
        }
        if let Some(p) = self.flat(name) {
            return Some(p);
        }
        for t in chain {
            if let Some(p) = self.themes.get_mut(t)?.lookup(name, size, refresh, true) {
                return Some(p);
            }
        }
        None
    }
    fn lookup(&mut self, names: &[String], size: i64, generic: bool) -> Option<PathBuf> {
        let clean = names
            .iter()
            .filter_map(|s| {
                let mut s = s.trim().to_string();
                let low = s.to_lowercase();
                for e in EXTS {
                    if low.ends_with(&format!(".{e}")) {
                        s.truncate(s.len() - e.len() - 1);
                        break;
                    }
                }
                if !s.is_empty()
                    && s.len() <= 255
                    && !s.contains("..")
                    && s.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"_.+@ -".contains(&b))
                {
                    Some(s)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        if clean.is_empty() {
            return None;
        }
        let current = self.current();
        let key = format!("{current}\0{size}\0{generic}\0{}", clean.join("\0"));
        if let Some(i) = self.cache.iter().position(|x| x.0 == key) {
            let item = self.cache.remove(i).unwrap();
            let hit = if item.1.is_some() || item.2.elapsed() < Duration::from_secs(60) {
                item.1.as_ref().is_none_or(|p| p.is_file())
            } else {
                false
            };
            let result = item.1.clone();
            self.cache.push_back(item);
            if hit {
                return result;
            }
        }
        let mut chain = Vec::new();
        let mut seen = HashSet::new();
        self.chain_add(&current, 0, &mut seen, &mut chain);
        self.chain_add("Adwaita", 0, &mut seen, &mut chain);
        self.chain_add("hicolor", 0, &mut seen, &mut chain);
        let mut result = None;
        for refresh in [false, true] {
            for n in &clean {
                result = self.one(n, size, &chain, refresh);
                if result.is_some() {
                    break;
                }
            }
            if result.is_some() {
                break;
            }
        }
        if result.is_none() && generic {
            for n in &clean {
                let mut s = n.as_str();
                while let Some((p, _)) = s.rsplit_once('-') {
                    s = p;
                    result = self.one(s, size, &chain, false);
                    if result.is_some() {
                        break;
                    }
                }
                if result.is_some() {
                    break;
                }
            }
        }
        self.cache.retain(|x| x.0 != key);
        self.cache.push_back((key, result.clone(), Instant::now()));
        while self.cache.len() > 4096 {
            self.cache.pop_front();
        }
        result
    }
}
fn lookup() -> &'static Mutex<Lookup> {
    static L: OnceLock<Mutex<Lookup>> = OnceLock::new();
    L.get_or_init(|| Mutex::new(Lookup::default()))
}
pub(super) fn find(name: &str, size: i64) -> Option<PathBuf> {
    let name = name.trim();
    if name.starts_with('/') || name.starts_with("file://") {
        let p = if name.starts_with("file://") {
            super::uri_path(name)?
        } else {
            PathBuf::from(name)
        };
        return (p.is_absolute() && p.is_file() && mime::image(&p)).then_some(p);
    }
    lookup()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .lookup(&[name.into()], size, true)
}
pub(super) fn find_mime(mime: &str, size: i64) -> Option<PathBuf> {
    let mime = mime.trim().to_lowercase();
    if mime.len() > 200 || !mime.contains('/') {
        return None;
    }
    let mut names = Vec::<String>::new();
    if let Ok(icon) = gio::content_type_get_icon(&mime).downcast::<gio::ThemedIcon>() {
        names.extend(icon.names().iter().map(|s| s.to_string()));
    }
    if let Some(n) = gio::content_type_get_generic_icon_name(&mime) {
        names.push(n.to_string())
    }
    let major = mime.split('/').next().unwrap_or("");
    names.extend([mime.replace('/', "-"), format!("{major}-x-generic")]);
    if mime == "inode/directory" {
        names.insert(0, "folder".into())
    }
    names.push(
        if major == "text" {
            "text-x-generic"
        } else {
            "application-x-generic"
        }
        .into(),
    );
    names.push("unknown".into());
    names.sort_by_key(|n| n.ends_with("-symbolic"));
    let mut seen = HashSet::new();
    names.retain(|n| seen.insert(n.clone()));
    lookup()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .lookup(&names, size, false)
}
