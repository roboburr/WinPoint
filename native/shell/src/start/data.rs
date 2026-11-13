//! Start's data: the app catalogue (apps.list), the pins (start.pinned, taskbar.pinned, shell.js defaultStart),
//! recent files and small text helpers (letters, "when" labels, sizes), ported from startmenu.js and shell.js.
use crate::core::Shell;
use gtk::glib;
use serde_json::{Value, json};

#[derive(Clone, Debug, Default)]
pub struct App {
    pub id: String,
    pub name: String,
    pub generic: String,
    pub comment: String,
    pub icon: String,
    pub categories: Vec<String>,
    pub keywords: Vec<String>,
    pub installed_at: i64,
}

impl App {
    pub fn from_json(v: &Value) -> Option<App> {
        let id = v["id"].as_str()?.to_owned();
        if id.is_empty() || v["noDisplay"] == true {
            return None;
        }
        let name = match v["name"].as_str() {
            Some(n) if !n.trim().is_empty() => n.to_owned(),
            _ => id.trim_end_matches(".desktop").to_owned(),
        };
        Some(App {
            name,
            generic: text(v, "genericName"),
            comment: text(v, "comment"),
            icon: text(v, "icon"),
            categories: str_list(&v["categories"]),
            keywords: str_list(&v["keywords"]),
            installed_at: num(&v["installedAt"]),
            id,
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct FileItem {
    pub path: String,
    pub name: String,
    pub dir: String,
    pub is_dir: bool,
    pub size: Option<u64>,
    pub modified: i64,
    pub mime: String,
    pub icon: String,
}

impl FileItem {
    pub fn from_json(v: &Value) -> Option<FileItem> {
        let path = v["path"].as_str()?.to_owned();
        if path.is_empty() {
            return None;
        }
        let name = v["name"].as_str().filter(|n| !n.is_empty()).map(str::to_owned).unwrap_or_else(|| leaf(&path));
        let dir = v["dir"].as_str().filter(|d| !d.is_empty()).map(str::to_owned).unwrap_or_else(|| parent_dir(&path));
        Some(FileItem {
            is_dir: v["isDir"] == true,
            size: v["size"].as_u64().or_else(|| v["size"].as_f64().map(|f| f as u64)),
            modified: num(&v["modified"]),
            mime: text(v, "mime"),
            icon: text(v, "icon"),
            path,
            name,
            dir,
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct User {
    pub name: String,
    pub real_name: String,
    pub avatar: Option<String>,
}

impl User {
    pub fn display(&self) -> String {
        if !self.real_name.trim().is_empty() {
            self.real_name.clone()
        } else {
            self.name.clone()
        }
    }
}

#[derive(Clone, Debug)]
pub enum RecKind {
    App(App),
    File(FileItem),
}

#[derive(Clone, Debug)]
pub struct RecItem {
    pub kind: RecKind,
    pub key: String,
    pub title: String,
    pub time: i64,
}

pub fn text(v: &Value, key: &str) -> String {
    v[key].as_str().unwrap_or("").to_owned()
}

pub fn num(v: &Value) -> i64 {
    v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)).unwrap_or(0)
}

pub fn str_list(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect())
        .unwrap_or_default()
}

/// The folder of a path, as the HTML's `path.replace(/\/[^/]*$/, '') || '/'`.
pub fn parent_dir(path: &str) -> String {
    match path.rfind('/') {
        Some(0) | None => "/".into(),
        Some(i) => path[..i].to_owned(),
    }
}

pub fn leaf(path: &str) -> String {
    path.trim_end_matches('/').rsplit('/').next().filter(|s| !s.is_empty()).unwrap_or("/").to_owned()
}

/// Our own HTML apps start through shell.openApp (shell.js OWN_APP_RE).
pub fn own_app(id: &str) -> Option<&'static str> {
    let base = id.strip_suffix(".desktop").unwrap_or(id);
    match base.strip_prefix("win11shell-")? {
        "settings" => Some("settings"),
        "taskmgr" => Some("taskmgr"),
        "explorer" => Some("explorer"),
        "alles" => Some("alles"),
        "disks" => Some("disks"),
        _ => None,
    }
}

/// ctx.apps.launch: own apps via shell.openApp, the rest via apps.launch.
pub fn launch(shell: &Shell, id: &str) {
    if let Some(own) = own_app(id) {
        shell.fire("shell.openApp", json!({"app": own, "params": {}}));
    } else {
        shell.fire("apps.launch", json!({"id": id}));
    }
}

pub fn open_app(shell: &Shell, app: &str, params: Value) {
    shell.fire("shell.openApp", json!({"app": app, "params": params}));
}

/* ------------------------------------------------------------------ pins */

pub fn dedupe(ids: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for id in ids {
        if !out.contains(&id) {
            out.push(id);
        }
    }
    out
}

/// ctx.pins.start.list(): start.pinned, or the default list while it is empty.
pub fn start_pins(shell: &Shell, apps: &[App], browser: Option<&str>) -> Vec<String> {
    let v = str_list(&shell.setting("start.pinned"));
    if v.is_empty() { default_start(apps, browser) } else { v }
}

pub fn set_start_pins(shell: &Shell, ids: Vec<String>) {
    shell.set_setting("start.pinned", json!(dedupe(ids)));
}

pub fn taskbar_pins(shell: &Shell) -> Vec<String> {
    str_list(&shell.setting("taskbar.pinned"))
}

pub fn set_taskbar_pins(shell: &Shell, ids: Vec<String>) {
    shell.set_setting("taskbar.pinned", json!(dedupe(ids)));
}

enum M {
    /// id starts with (case-insensitive)
    P(&'static str),
    /// id contains (case-insensitive)
    C(&'static str),
}

/// shell.js START_DEFAULTS: one slot per entry, the first app that matches one of its patterns.
const START_DEFAULTS: &[&[M]] = &[
    &[M::P("firefox"), M::P("google-chrome"), M::P("chromium"), M::P("brave"), M::P("microsoft-edge"), M::P("org.gnome.epiphany")],
    &[M::P("win11shell-explorer")],
    &[M::P("win11shell-settings")],
    &[M::P("win11shell-taskmgr")],
    &[M::P("win11shell-alles")],
    &[M::C("ptyxis"), M::C("gnome-terminal"), M::C("org.gnome.console"), M::C("konsole"), M::C("terminal")],
    &[M::C("texteditor"), M::C("gedit"), M::C("kate."), M::C("mousepad"), M::C("xed")],
    &[M::C("snap-store"), M::C("gnome.software"), M::C("discover"), M::C("appcenter"), M::C("app-center")],
    &[M::C("calculator")],
    &[M::C("thunderbird"), M::C("evolution"), M::C("geary")],
    &[M::C("libreoffice-writer")],
    &[M::C("libreoffice-calc")],
    &[M::C("loupe"), M::C("eog"), M::C("shotwell"), M::C("gthumb")],
    &[M::C("showtime"), M::C("totem"), M::C("celluloid"), M::C("vlc")],
    &[M::C("rhythmbox"), M::C("spotify"), M::C("lollypop")],
    &[M::C("calendar")],
    &[M::C("gimp")],
    &[M::C("steam")],
    &[M::C("discord")],
    &[M::C("telegram")],
    &[M::C("whatsapp")],
    &[M::C("code_code"), M::C("vscode"), M::C("codium")],
    &[M::C("clocks")],
];

pub fn default_start(apps: &[App], browser: Option<&str>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (i, alts) in START_DEFAULTS.iter().enumerate() {
        if i == 0
            && let Some(b) = browser
        {
            out.push(b.to_owned());
            continue;
        }
        for m in alts.iter() {
            let found = apps.iter().find(|a| {
                let id = a.id.to_lowercase();
                let hit = match m {
                    M::P(p) => id.starts_with(p),
                    M::C(c) => id.contains(c),
                };
                hit && !out.contains(&a.id)
            });
            if let Some(a) = found {
                out.push(a.id.clone());
                break;
            }
        }
        if out.len() >= 24 {
            break;
        }
    }
    out
}

/* --------------------------------------------------------------- letters */

/// Lower case without accents (search.js normalize).
pub fn normalize(s: &str) -> String {
    strip_marks(&s.to_lowercase()).trim().to_owned()
}

pub fn strip_marks(s: &str) -> String {
    glib::normalize(s, glib::NormalizeMode::Default)
        .chars()
        .filter(|c| !('\u{300}'..='\u{36f}').contains(c))
        .collect()
}

/// startmenu.js letterOf: A-Z, '#' for digits, '&' for the rest.
pub fn letter_of(name: &str) -> char {
    let c = strip_marks(name.trim()).chars().next().map(|c| c.to_ascii_uppercase()).unwrap_or('&');
    if c.is_ascii_uppercase() {
        c
    } else if c.is_ascii_digit() {
        '#'
    } else {
        '&'
    }
}

pub fn sort_key(name: &str) -> glib::FilenameCollationKey {
    glib::FilenameCollationKey::from(normalize(name))
}

/* ------------------------------------------------------------------ time */

pub fn now_ms() -> i64 {
    glib::real_time() / 1000
}

fn local(ms: i64) -> Option<glib::DateTime> {
    glib::DateTime::from_unix_local(ms.div_euclid(1000)).ok()
}

pub fn fmt_time(ms: i64) -> String {
    local(ms).and_then(|d| d.format("%H:%M").ok()).map(|s| s.to_string()).unwrap_or_default()
}

pub fn fmt_date_short(ms: i64) -> String {
    local(ms).and_then(|d| d.format("%d-%m-%Y").ok()).map(|s| s.to_string()).unwrap_or_default()
}

const MONTHS_NL: [&str; 12] =
    ["januari", "februari", "maart", "april", "mei", "juni", "juli", "augustus", "september", "oktober", "november", "december"];
const MONTHS_EN: [&str; 12] =
    ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const MON_NL: [&str; 12] = ["jan", "feb", "mrt", "apr", "mei", "jun", "jul", "aug", "sep", "okt", "nov", "dec"];
const MON_EN: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

fn rel_time(shell: &Shell, ms: i64, now: i64) -> String {
    let abs = ((now - ms).abs() as f64) / 1000.0;
    let nl = shell.lang() != "en";
    if abs < 45.0 {
        return if nl { "zojuist".into() } else { "just now".into() };
    }
    if abs < 3600.0 {
        let n = ((abs / 60.0).round() as i64).max(1);
        return match (nl, n) {
            (true, 1) => "1 minuut geleden".into(),
            (true, n) => format!("{n} minuten geleden"),
            (false, 1) => "1 minute ago".into(),
            (false, n) => format!("{n} minutes ago"),
        };
    }
    let n = ((abs / 3600.0).round() as i64).max(1);
    match (nl, n) {
        (true, n) => format!("{n} uur geleden"),
        (false, 1) => "1 hour ago".into(),
        (false, n) => format!("{n} hours ago"),
    }
}

/// startmenu.js whenLabel: "Zojuist", "5 minuten geleden", "Vandaag om 09:12", "Gisteren om 18:40", "3 november".
pub fn when_label(shell: &Shell, ms: i64) -> String {
    if ms <= 0 {
        return String::new();
    }
    let now = now_ms();
    let diff = now - ms;
    if diff < 60_000 {
        return shell.t("Zojuist", "Just now");
    }
    if diff < 3_600_000 {
        return rel_time(shell, ms, now);
    }
    let (Some(d), Some(today)) = (local(ms), local(now)) else { return String::new() };
    let same = |a: &glib::DateTime, b: &glib::DateTime| a.year() == b.year() && a.day_of_year() == b.day_of_year();
    if same(&d, &today) {
        return if diff < 6 * 3_600_000 {
            rel_time(shell, ms, now)
        } else {
            format!("{} {}", shell.t("Vandaag om", "Today at"), fmt_time(ms))
        };
    }
    if let Ok(y) = today.add_days(-1)
        && same(&d, &y)
    {
        return format!("{} {}", shell.t("Gisteren om", "Yesterday at"), fmt_time(ms));
    }
    let m = (d.month().clamp(1, 12) - 1) as usize;
    let nl = shell.lang() != "en";
    if d.year() == today.year() {
        format!("{} {}", d.day_of_month(), if nl { MONTHS_NL[m] } else { MONTHS_EN[m] })
    } else {
        format!("{} {} {}", d.day_of_month(), if nl { MON_NL[m] } else { MON_EN[m] }, d.year())
    }
}

/// format.js bytes: "1,4 MB" / "1.4 MB".
pub fn bytes(shell: &Shell, n: u64) -> String {
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < units.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    let s = if i == 0 || v >= 100.0 { format!("{}", v.round() as u64) } else { format!("{v:.1}") };
    let s = if shell.lang() == "en" { s } else { s.replace('.', ",") };
    format!("{s} {}", units[i])
}

pub fn home_dir() -> String {
    glib::home_dir().to_string_lossy().trim_end_matches('/').to_owned()
}

/// search.js prettyDir: "Documenten › Werk" under the home folder, the path elsewhere.
pub fn pretty_dir(shell: &Shell, dir: &str) -> String {
    let home = home_dir();
    if !home.is_empty() && (dir == home || dir.starts_with(&format!("{home}/"))) {
        let rel = dir[home.len()..].trim_start_matches('/');
        return if rel.is_empty() { shell.t("Persoonlijke map", "Home") } else { rel.split('/').collect::<Vec<_>>().join(" › ") };
    }
    if dir.is_empty() { "/".into() } else { dir.to_owned() }
}
