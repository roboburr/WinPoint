//! Helpers (6ebf169f:vendor/ui/js/apps/explorer/util.js): paths, friendly names, type names, sizes, dates, groups,
//! sorting and drives. File items are the backend's JSON objects (files.list / files.find / files.stat rows:
//! {name, displayName, path, isDir, icon, mime, size, modified, hidden, typeName, ...}), shared as `Item`.
use crate::i18n::{f, s};
use kit::format::{self, DateFormat};
use serde_json::Value;
use std::cmp::Ordering;
use std::rc::Rc;

/// A file item as the backend sent it (cheap to clone).
pub type Item = Rc<Value>;

/// A string field of a JSON object ("" when absent).
pub fn st<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

/// A number field of a JSON object (0 when absent).
pub fn num(v: &Value, key: &str) -> f64 {
    v.get(key).and_then(Value::as_f64).unwrap_or(0.0)
}

/// A boolean field (false when absent).
pub fn flag(v: &Value, key: &str) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(false)
}

// ------------------------------------------------------------------------------------------------- paths

/// "/a/./b/../c/" -> "/a/c".
pub fn norm_path(p: &str) -> String {
    if p.is_empty() {
        return String::new();
    }
    let abs = p.starts_with('/');
    let mut out: Vec<&str> = Vec::new();
    for seg in p.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            s => out.push(s),
        }
    }
    let joined = out.join("/");
    let r = if abs { format!("/{joined}") } else { joined };
    if r.is_empty() { "/".into() } else { r }
}

pub fn basename(p: &str) -> String {
    if p.is_empty() {
        return String::new();
    }
    let s = if p.len() > 1 { p.trim_end_matches('/') } else { p };
    if s.is_empty() || s == "/" {
        return "/".into();
    }
    s.rsplit('/').next().unwrap_or(s).to_owned()
}

pub fn dirname(p: &str) -> String {
    if p.is_empty() || p == "/" {
        return "/".into();
    }
    let s = p.trim_end_matches('/');
    match s.rfind('/') {
        Some(i) if i > 0 => s[..i].to_owned(),
        _ => "/".into(),
    }
}

pub fn join_path(dir: &str, name: &str) -> String {
    if dir.ends_with('/') { format!("{dir}{name}") } else { format!("{dir}/{name}") }
}

/// Whether `p` is `dir` or inside it.
pub fn is_under(p: &str, dir: &str) -> bool {
    if p.is_empty() || dir.is_empty() {
        return false;
    }
    p == dir || if dir == "/" { p.starts_with('/') } else { p.starts_with(&format!("{dir}/")) }
}

/// "foto.tar.gz" -> ("foto", ".tar.gz"); ".bashrc" -> (".bashrc", "").
pub fn split_ext(name: &str) -> (String, String) {
    if name.is_empty() || (name.starts_with('.') && !name[1..].contains('.')) {
        return (name.to_owned(), String::new());
    }
    let lower = name.to_lowercase();
    for tar in [".tar.gz", ".tar.bz2", ".tar.xz", ".tar.zst"] {
        if lower.ends_with(tar) && name.len() > tar.len() {
            let at = name.len() - tar.len();
            if name.is_char_boundary(at) {
                return (name[..at].to_owned(), name[at..].to_owned());
            }
        }
    }
    if let Some(i) = name.rfind('.')
        && i > 0
    {
        let ext = &name[i + 1..];
        let n = ext.chars().count();
        if (1..=12).contains(&n) && !ext.chars().any(char::is_whitespace) {
            return (name[..i].to_owned(), name[i..].to_owned());
        }
    }
    (name.to_owned(), String::new())
}

/// The extension without the dot, lower case ("tar.gz", "png", "").
pub fn ext_of(name: &str) -> String {
    split_ext(name).1.trim_start_matches('.').to_lowercase()
}

/// file:// URI of a path.
#[allow(dead_code)] // the API of builders B and C (see main.rs)
pub fn file_uri(p: &str) -> String {
    gtk::glib::filename_to_uri(p, None).map(|u| u.to_string()).unwrap_or_else(|_| format!("file://{p}"))
}

/// The local path of a file:// URI.
pub fn uri_to_path(u: &str) -> Option<String> {
    let u = u.trim();
    if !u.to_ascii_lowercase().starts_with("file:") {
        return None;
    }
    gtk::glib::filename_from_uri(u).ok().map(|(p, _)| p.to_string_lossy().into_owned())
}

// ------------------------------------------------------------------------------------------------ display

/// The name shown for an item (without extension when extensions are hidden).
pub fn display_name(item: &Value, show_ext: bool) -> String {
    let dn = st(item, "displayName");
    if (flag(item, "isDesktopEntry") || flag(item, "inTrash") || !st(item, "trashName").is_empty()) && !dn.is_empty() {
        return dn.to_owned();
    }
    let name = match st(item, "name") {
        "" => basename(st(item, "path")),
        n => n.to_owned(),
    };
    if show_ext || flag(item, "isDir") {
        return name;
    }
    let (stem, ext) = split_ext(&name);
    if !ext.is_empty() && !stem.is_empty() { stem } else { name }
}

const TYPES: &[(&str, &str, &str)] = &[
    ("txt", "Tekstdocument", "Text Document"),
    ("md", "Markdown-document", "Markdown Document"),
    ("log", "Logbestand", "Log File"),
    ("rtf", "RTF-document", "RTF Document"),
    ("pdf", "PDF-document", "PDF Document"),
    ("doc", "Word-document", "Word Document"),
    ("docx", "Word-document", "Word Document"),
    ("odt", "OpenDocument-tekst", "OpenDocument Text"),
    ("xls", "Excel-werkblad", "Excel Worksheet"),
    ("xlsx", "Excel-werkblad", "Excel Worksheet"),
    ("ods", "OpenDocument-werkblad", "OpenDocument Spreadsheet"),
    ("csv", "CSV-bestand", "CSV File"),
    ("ppt", "PowerPoint-presentatie", "PowerPoint Presentation"),
    ("pptx", "PowerPoint-presentatie", "PowerPoint Presentation"),
    ("odp", "OpenDocument-presentatie", "OpenDocument Presentation"),
    ("jpg", "JPG-bestand", "JPG File"),
    ("jpeg", "JPG-bestand", "JPG File"),
    ("png", "PNG-bestand", "PNG File"),
    ("gif", "GIF-bestand", "GIF File"),
    ("webp", "WebP-afbeelding", "WebP Image"),
    ("svg", "SVG-afbeelding", "SVG Image"),
    ("bmp", "BMP-bestand", "BMP File"),
    ("heic", "HEIC-afbeelding", "HEIC Image"),
    ("tif", "TIFF-afbeelding", "TIFF Image"),
    ("tiff", "TIFF-afbeelding", "TIFF Image"),
    ("ico", "Pictogram", "Icon"),
    ("mp3", "MP3-bestand", "MP3 File"),
    ("wav", "WAV-geluid", "WAV Audio"),
    ("flac", "FLAC-audio", "FLAC Audio"),
    ("ogg", "OGG-audio", "OGG Audio"),
    ("m4a", "MPEG-4-audio", "MPEG-4 Audio"),
    ("opus", "Opus-audio", "Opus Audio"),
    ("mp4", "MP4-video", "MP4 Video"),
    ("mkv", "MKV-video", "MKV Video"),
    ("avi", "AVI-video", "AVI Video"),
    ("mov", "QuickTime-film", "QuickTime Movie"),
    ("webm", "WebM-video", "WebM Video"),
    ("zip", "Gecomprimeerde (gezipte) map", "Compressed (zipped) Folder"),
    ("7z", "7-Zip-archief", "7-Zip Archive"),
    ("rar", "RAR-archief", "RAR Archive"),
    ("tar", "TAR-archief", "TAR Archive"),
    ("tar.gz", "TAR.GZ-archief", "TAR.GZ Archive"),
    ("tar.xz", "TAR.XZ-archief", "TAR.XZ Archive"),
    ("tar.bz2", "TAR.BZ2-archief", "TAR.BZ2 Archive"),
    ("tar.zst", "TAR.ZST-archief", "TAR.ZST Archive"),
    ("gz", "GZ-archief", "GZ Archive"),
    ("xz", "XZ-archief", "XZ Archive"),
    ("deb", "Softwarepakket", "Software Package"),
    ("rpm", "Softwarepakket", "Software Package"),
    ("appimage", "AppImage-programma", "AppImage Program"),
    ("exe", "Windows-programma", "Windows Program"),
    ("msi", "Windows-installatiepakket", "Windows Installer Package"),
    ("sh", "Shellscript", "Shell Script"),
    ("py", "Python-bestand", "Python File"),
    ("js", "JavaScript-bestand", "JavaScript File"),
    ("html", "HTML-document", "HTML Document"),
    ("htm", "HTML-document", "HTML Document"),
    ("css", "CSS-bestand", "CSS File"),
    ("json", "JSON-bestand", "JSON File"),
    ("xml", "XML-document", "XML Document"),
    ("iso", "Schijfkopiebestand", "Disc Image File"),
    ("img", "Schijfkopiebestand", "Disc Image File"),
    ("desktop", "Snelkoppeling", "Shortcut"),
    ("ttf", "TrueType-lettertype", "TrueType Font"),
    ("otf", "OpenType-lettertype", "OpenType Font"),
    ("torrent", "Torrent-bestand", "Torrent File"),
    ("epub", "E-boek", "E-book"),
    ("ini", "Configuratie-instellingen", "Configuration Settings"),
    ("conf", "Configuratiebestand", "Configuration File"),
];

/// The Type column ("Bestandsmap", "PNG-bestand", the backend's typeName, "XYZ-bestand").
pub fn type_name(item: &Value) -> String {
    if flag(item, "isDir") {
        return s("folder").to_owned();
    }
    let tn = st(item, "typeName");
    if !tn.is_empty() {
        return tn.to_owned();
    }
    if flag(item, "isDesktopEntry") {
        return s("shortcut").to_owned();
    }
    let name = match st(item, "name") {
        "" => basename(st(item, "path")),
        n => n.to_owned(),
    };
    let ext = ext_of(&name);
    if let Some((_, nl, en)) = TYPES.iter().find(|(e, _, _)| *e == ext) {
        return kit::t(nl, en).to_owned();
    }
    if !ext.is_empty() {
        return f("extFile", &[("ext", &ext.to_uppercase())]);
    }
    if st(item, "mime") == "text/plain" {
        return kit::t("Tekstdocument", "Text Document").to_owned();
    }
    s("file").to_owned()
}

/// The Size column: "" for folders, "0 kB", "12 kB", "4,2 MB" (Windows style).
pub fn size_text(item: &Value) -> String {
    if flag(item, "isDir") {
        return String::new();
    }
    match item.get("size").and_then(Value::as_f64) {
        Some(n) if n.is_finite() => size_text_n(n),
        _ => String::new(),
    }
}

/// `size_text` of a number of bytes.
pub fn size_text_n(n: f64) -> String {
    if n <= 0.0 {
        return "0 kB".into();
    }
    let kb = (n / 1024.0).ceil();
    if kb < 1000.0 { format!("{} kB", format::number(kb, 0)) } else { format::bytes(n) }
}

/// "4,2 MB (4.404.019 bytes)".
pub fn bytes_full(n: f64) -> String {
    if !n.is_finite() {
        return "\u{2013}".into();
    }
    f("bytesFull", &[("h", &format::bytes(n)), ("b", &format::number(n, 0))])
}

/// "25-10-2026 14:03" ("" without a time).
pub fn date_time(ms: f64) -> String {
    if ms == 0.0 || !ms.is_finite() {
        return String::new();
    }
    format::date(ms, DateFormat::DateTime)
}

// ------------------------------------------------------------------------------------------------- groups

/// A group of the "Groeperen op" view.
#[derive(Clone, Debug)]
pub struct GroupKey {
    pub key: String,
    pub label: String,
    pub order: i32,
}

fn gk(key: &str, label_key: &str, order: i32) -> GroupKey {
    GroupKey { key: key.into(), label: s(label_key).into(), order }
}

/// Local midnight of the day of `ms`, in ms.
fn start_of_day(ms: f64) -> (f64, i32, i32, i32, i32) {
    let dt = gtk::glib::DateTime::from_unix_local((ms / 1000.0).floor() as i64).ok();
    match dt {
        Some(d) => {
            let midnight = gtk::glib::DateTime::from_local(d.year(), d.month(), d.day_of_month(), 0, 0, 0.0)
                .map(|m| m.to_unix() as f64 * 1000.0)
                .unwrap_or(ms);
            (midnight, d.year(), d.month(), d.day_of_month(), d.day_of_week())
        }
        None => (ms, 1970, 1, 1, 1),
    }
}

fn local_ms(y: i32, m: i32, d: i32) -> f64 {
    gtk::glib::DateTime::from_local(y, m, d, 0, 0, 0.0).map(|t| t.to_unix() as f64 * 1000.0).unwrap_or(0.0)
}

/// The date group of a time (Vandaag, Gisteren, Eerder deze week, ...).
pub fn date_group(ms: f64, now: f64) -> GroupKey {
    if ms == 0.0 || !ms.is_finite() {
        return gk("unknown", "gUnknown", 99);
    }
    const DAY: f64 = 86_400_000.0;
    let (today, y, m, _, dow) = start_of_day(now);
    if ms >= today + DAY {
        return gk("future", "gFuture", 0);
    }
    if ms >= today {
        return gk("today", "gToday", 1);
    }
    if ms >= today - DAY {
        return gk("yesterday", "gYesterday", 2);
    }
    // glib: Monday = 1 .. Sunday = 7
    let week_start = today - (dow - 1) as f64 * DAY;
    if ms >= week_start {
        return gk("thisWeek", "gThisWeek", 3);
    }
    if ms >= week_start - 7.0 * DAY {
        return gk("lastWeek", "gLastWeek", 4);
    }
    if ms >= local_ms(y, m, 1) {
        return gk("thisMonth", "gThisMonth", 5);
    }
    let (py, pm) = if m == 1 { (y - 1, 12) } else { (y, m - 1) };
    if ms >= local_ms(py, pm, 1) {
        return gk("lastMonth", "gLastMonth", 6);
    }
    if ms >= local_ms(y, 1, 1) {
        return gk("thisYear", "gThisYear", 7);
    }
    gk("older", "gOlder", 8)
}

/// The size group of an item.
pub fn size_group(item: &Value) -> GroupKey {
    if flag(item, "isDir") {
        return gk("folders", "gFolders", 0);
    }
    let n = num(item, "size");
    const K: f64 = 1024.0;
    const M: f64 = K * K;
    const G: f64 = M * K;
    if n == 0.0 {
        gk("empty", "gEmpty", 1)
    } else if n < 16.0 * K {
        gk("tiny", "gTiny", 2)
    } else if n < M {
        gk("small", "gSmall", 3)
    } else if n < 128.0 * M {
        gk("medium", "gMedium", 4)
    } else if n < G {
        gk("large", "gLarge", 5)
    } else if n < 4.0 * G {
        gk("huge", "gHuge", 6)
    } else {
        gk("gigantic", "gGigantic", 7)
    }
}

// ------------------------------------------------------------------------------------------------ sorting

/// Natural, case-insensitive name order.
pub fn compare_names(a: &str, b: &str) -> Ordering {
    format::compare_natural(a, b)
}

/// A sort value for `sort_items`' own getters.
pub enum SortVal {
    Num(f64),
    Text(String),
}

fn name_of(x: &Value) -> &str {
    match st(x, "displayName") {
        "" => st(x, "name"),
        n => n,
    }
}

/// Windows order: folders first, numeric-aware names; `desc` reverses everything. `getter` gives the value
/// for keys the default getters don't know (orig, deleted, location).
pub fn sort_items(items: &mut [Item], key: &str, desc: bool, getter: Option<&dyn Fn(&Value) -> SortVal>) {
    let val = |x: &Value| -> SortVal {
        if let Some(g) = getter {
            return g(x);
        }
        match key {
            "modified" => SortVal::Num(num(x, "modified")),
            "type" => SortVal::Text(type_name(x)),
            "size" => SortVal::Num(if flag(x, "isDir") { -1.0 } else { num(x, "size") }),
            _ => SortVal::Text(name_of(x).to_owned()),
        }
    };
    items.sort_by(|a, b| {
        let (ad, bd) = (flag(a, "isDir"), flag(b, "isDir"));
        if ad != bd {
            return if ad { Ordering::Less } else { Ordering::Greater };
        }
        let r = match (val(a), val(b)) {
            (SortVal::Num(x), SortVal::Num(y)) => x.partial_cmp(&y).unwrap_or(Ordering::Equal),
            (SortVal::Text(x), SortVal::Text(y)) => compare_names(&x, &y),
            (SortVal::Num(x), SortVal::Text(y)) => compare_names(&x.to_string(), &y),
            (SortVal::Text(x), SortVal::Num(y)) => compare_names(&x, &y.to_string()),
        };
        if r == Ordering::Equal && key != "name" { compare_names(name_of(a), name_of(b)) } else { r }
    });
    if desc {
        items.reverse();
    }
}

// -------------------------------------------------------------------------------------------------- drives

/// Friendly drive name: "Systeemschijf (/)", "BACKUP", "Virtuele machines (/srv/vm)".
pub fn volume_label(v: &Value) -> String {
    let mount = st(v, "mount");
    let name = [st(v, "name"), st(v, "label")]
        .into_iter()
        .find(|s| !s.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| {
            let b = basename(mount);
            if b.is_empty() { mount.to_owned() } else { b }
        });
    if mount == "/" {
        return format!("{name} (/)");
    }
    let removable_root = mount.starts_with("/media/") || mount.starts_with("/run/media/") || mount.starts_with("/mnt/");
    if !removable_root && !name.contains(mount) {
        return format!("{name} ({mount})");
    }
    name
}

/// How full a drive is, 0..100 (the host's df-style percent).
pub fn volume_percent(v: &Value) -> f64 {
    let p = match v.get("percent").and_then(Value::as_f64) {
        Some(p) if p.is_finite() => p,
        _ => {
            let (u, fr) = (num(v, "used"), num(v, "free"));
            if u + fr > 0.0 { u / (u + fr) * 100.0 } else { 0.0 }
        }
    };
    p.clamp(0.0, 100.0)
}

pub fn volume_icon(v: &Value) -> &'static str {
    if flag(v, "removable") || st(v, "kind") == "usb" {
        "drive-removable-media"
    } else if flag(v, "isSystem") || st(v, "mount") == "/" {
        "drive-harddisk-system"
    } else {
        "drive-harddisk"
    }
}

/// The drive a path lives on (longest mount prefix).
pub fn volume_for<'a>(path: &str, volumes: &'a [Value]) -> Option<&'a Value> {
    volumes
        .iter()
        .filter(|v| {
            let m = st(v, "mount");
            !m.is_empty() && is_under(path, m)
        })
        .max_by_key(|v| st(v, "mount").len())
}

/// Archives "Alles uitpakken" offers to extract.
pub fn is_archive(name: &str) -> bool {
    let l = name.to_lowercase();
    [".zip", ".tar", ".tar.gz", ".tar.bz2", ".tar.xz", ".tar.zst", ".tgz", ".tbz2", ".txz", ".gz", ".bz2", ".xz"]
        .iter()
        .any(|e| l.ends_with(e))
}

/// Pictures for the Gallery.
pub fn is_image(item: &Value) -> bool {
    if st(item, "mime").starts_with("image/") {
        return true;
    }
    let l = st(item, "name").to_lowercase();
    [".png", ".jpg", ".jpeg", ".gif", ".webp", ".bmp", ".svg", ".avif", ".heic", ".tif", ".tiff"].iter().any(|e| l.ends_with(e))
}

/// Copy text to the system clipboard.
pub fn copy_text(widget: &impl gtk::prelude::IsA<gtk::Widget>, text: &str) -> bool {
    use gtk::prelude::*;
    widget.as_ref().clipboard().set_text(text);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths() {
        assert_eq!(norm_path("/a/./b/../c/"), "/a/c");
        assert_eq!(norm_path("/"), "/");
        assert_eq!(basename("/a/b/"), "b");
        assert_eq!(dirname("/a/b"), "/a");
        assert_eq!(dirname("/a"), "/");
        assert!(is_under("/a/b", "/a"));
        assert!(!is_under("/ab", "/a"));
        assert_eq!(split_ext("foto.tar.gz"), ("foto".into(), ".tar.gz".into()));
        assert_eq!(split_ext(".bashrc"), (".bashrc".into(), "".into()));
        assert_eq!(split_ext("a.b c"), ("a.b c".into(), "".into()));
    }
}
