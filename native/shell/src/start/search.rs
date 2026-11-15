//! Start's search view (search.js createSearchPanel, embedded): typing in Start's box turns it into this panel in
//! place. Results: best match, Apps (fuzzy, prefix and word-start boosted, Dutch and English keywords and
//! synonyms), Instellingen (the Settings app's page registry), Acties, Bestanden (index.search, falling back to
//! recent files), Zoeken op internet and "Meer resultaten in Alles"; the right pane shows the selected result.
use super::data::{self, App, FileItem};
use super::menu::{Anchor, Item as MItem};
use crate::core::Shell;
use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use serde_json::{Value, json};
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;
use std::time::Duration;

/* ================================================================ matching */

fn is_split(c: char) -> bool {
    c.is_whitespace() || "-_.:/()'’,+".contains(c)
}

fn words(text: &str) -> Vec<&str> {
    text.split(is_split).filter(|w| !w.is_empty()).collect()
}

/// Optimal string alignment distance, stopping above `max`.
fn edit_distance(a: &[char], b: &[char], max: usize) -> usize {
    if a.len().abs_diff(b.len()) > max {
        return max + 1;
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut prev2: Option<Vec<usize>> = None;
    for i in 1..=a.len() {
        let mut cur = vec![i];
        let mut row_min = i;
        for j in 1..=b.len() {
            let mut v = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + usize::from(a[i - 1] != b[j - 1]));
            if let Some(p2) = &prev2
                && i > 1
                && j > 1
                && a[i - 1] == b[j - 2]
                && a[i - 2] == b[j - 1]
            {
                v = v.min(p2[j - 2] + 1);
            }
            cur.push(v);
            row_min = row_min.min(v);
        }
        if row_min > max {
            return max + 1;
        }
        prev2 = Some(prev);
        prev = cur;
    }
    prev[b.len()]
}

fn fuzzy(q: &str, text: &str) -> f64 {
    let t: Vec<char> = text.chars().collect();
    let (mut from, mut gaps, mut first, mut last) = (0usize, 0usize, None::<usize>, None::<usize>);
    for ch in q.chars() {
        if ch == ' ' {
            continue;
        }
        let Some(j) = t[from.min(t.len())..].iter().position(|&c| c == ch).map(|p| p + from) else { return 0.0 };
        if first.is_none() {
            first = Some(j);
        }
        if let Some(l) = last
            && j > l + 1
        {
            gaps += j - l - 1;
        }
        last = Some(j);
        from = j + 1;
    }
    let s = 46.0 - gaps as f64 * 2.0 - first.unwrap_or(0).min(10) as f64;
    if s > 18.0 { s } else { 0.0 }
}

/// search.js scoreText(normalizedQuery, normalizedText, loose) 0..100: exact > prefix > word start > all words >
/// acronym > substring > (loose only) one typo > fuzzy subsequence.
pub fn score_text(q: &str, text: &str, loose: bool) -> f64 {
    if q.is_empty() || text.is_empty() {
        return 0.0;
    }
    if text == q {
        return 100.0;
    }
    let qlen = q.chars().count();
    let tlen = text.chars().count();
    if text.starts_with(q) {
        return 92.0 - (tlen.saturating_sub(qlen) as f64 / 3.0).min(8.0);
    }
    let ws = words(text);
    for (i, w) in ws.iter().enumerate() {
        if w.starts_with(q) {
            return 84.0 - ((i * 2) as f64).min(6.0);
        }
    }
    let qw: Vec<&str> = q.split_whitespace().collect();
    if qw.len() > 1 && qw.iter().all(|x| ws.iter().any(|w| w.starts_with(x))) {
        return 78.0;
    }
    if qlen >= 2 && !q.contains(' ') {
        let acr: String = ws.iter().filter_map(|w| w.chars().next()).collect();
        if acr.chars().count() > 1 && acr.starts_with(q) {
            return 72.0;
        }
    }
    if let Some(idx) = text.find(q) {
        return 64.0 - text[..idx].chars().count().min(14) as f64;
    }
    if !loose {
        return 0.0;
    }
    if qlen >= 6 && !q.contains(' ') {
        let max = if qlen >= 9 { 2 } else { 1 };
        let qc: Vec<char> = q.chars().collect();
        for w in &ws {
            let wc: Vec<char> = w.chars().collect();
            if wc.len() + 1 >= qlen && edit_distance(&qc, &wc[..qlen.min(wc.len())], max) <= max {
                return 52.0;
            }
        }
    }
    if qlen >= 3 {
        return fuzzy(q, text);
    }
    0.0
}

/// Synonyms (Dutch and English) so people find apps by what they do or by their Windows name. The patterns are
/// case-insensitive substrings of "<id> <categories>", as the HTML's regular expressions.
const SYNONYMS: &[(&[&str], &[&str])] = &[
    (&["webbrowser", "firefox", "chrome", "chromium", "brave", "epiphany"], &["browser", "internet", "webbrowser", "surfen", "web", "edge", "internet explorer"]),
    (&["filemanager", "nautilus", "win11shell-explorer", "dolphin", "thunar"], &["verkenner", "explorer", "bestanden", "files", "mappen", "bestandsbeheer", "file explorer", "deze pc"]),
    (&["terminalemulator", "ptyxis", "gnome-terminal", "console", "konsole"], &["terminal", "opdrachtprompt", "cmd", "command prompt", "powershell", "console", "shell", "opdrachtregel"]),
    (&["texteditor", "gedit", "mousepad", "kate"], &["kladblok", "notepad", "teksteditor", "text editor", "tekst", "notities"]),
    (&["calculator"], &["rekenmachine", "calculator", "rekenen", "calc"]),
    (&["packagemanager", "snap-store", "gnome.software", "discover"], &["store", "winkel", "microsoft store", "app center", "software", "apps installeren", "programma installeren"]),
    (&["win11shell-settings"], &["configuratiescherm", "control panel", "instellingen", "settings", "opties"]),
    (&["win11shell-taskmgr", "systemmonitor", "resources"], &["taakbeheer", "task manager", "processen", "prestaties", "ctrl alt delete"]),
    (&["email", "thunderbird", "evolution", "geary"], &["mail", "e-mail", "email", "outlook", "post"]),
    (&["calendar"], &["agenda", "calendar", "kalender", "afspraken"]),
    (&["loupe", "eog", "imageviewer", "shotwell", "gthumb"], &["foto's", "fotos", "photos", "afbeeldingen", "images", "fotoweergave", "plaatjes"]),
    (&["player", "video", "vlc", "showtime", "totem", "celluloid", "mpv"], &["video", "films", "media player", "mediaspeler", "filmpjes", "windows media player"]),
    (&["audio", "music", "rhythmbox", "spotify", "lollypop"], &["muziek", "music", "liedjes", "media player"]),
    (&["wordprocessor", "writer"], &["word", "tekstverwerker", "brief", "document schrijven"]),
    (&["spreadsheet", "libreoffice-calc"], &["excel", "spreadsheet", "rekenblad", "werkblad", "tabel"]),
    (&["presentation", "impress"], &["powerpoint", "presentatie", "dia", "slides"]),
    (&["rastergraphics", "gimp", "krita", "pinta"], &["paint", "tekenen", "foto bewerken", "photoshop"]),
    (&["game", "steam", "lutris", "heroic"], &["games", "spellen", "spelletjes", "xbox"]),
    (&["instantmessaging", "chat", "discord", "telegram", "whatsapp", "signal"], &["chat", "berichten", "teams", "bellen", "messenger"]),
    (&["snapshot", "cheese", "camera"], &["camera", "webcam"]),
    (&["clocks"], &["klok", "wekker", "alarm", "timer", "stopwatch", "alarmen en klok"]),
    (&["weather"], &["weer", "weather", "weerbericht"]),
    (&["win11shell-alles"], &["everything", "zoeken", "bestanden zoeken", "alles", "find"]),
    (&["diskutility", "baobab"], &["schijfbeheer", "schijven", "disk", "opslag"]),
    (
        &["win11shell-disks"],
        &["schijfbeheer", "disk management", "diskmgmt", "partities", "partitie", "formatteren", "format", "usb-stick", "harde schijf", "ssd", "volume", "stations", "gnome-disks", "schijven en volumes"],
    ),
    (&["archiving", "file-roller", "fileroller", "ark"], &["zip", "winzip", "7zip", "winrar", "uitpakken"]),
    (&["characters"], &["emoji", "speciale tekens", "tekenkaart"]),
];

struct AppEntry {
    app: App,
    n: String,
    g: String,
    k: Vec<String>,
    c: String,
    id: String,
}

fn app_entry(app: &App) -> AppEntry {
    let hay = format!("{} {}", app.id, app.categories.join(" ")).to_lowercase();
    let mut k: Vec<String> = app.keywords.iter().map(|w| data::normalize(w)).filter(|w| !w.is_empty()).collect();
    for (pats, syn) in SYNONYMS {
        if pats.iter().any(|p| hay.contains(p)) {
            k.extend(syn.iter().map(|w| data::normalize(w)));
        }
    }
    let id: String = app
        .id
        .trim_end_matches(".desktop")
        .chars()
        .map(|c| if matches!(c, '.' | '_' | '-') { ' ' } else { c })
        .collect();
    AppEntry {
        app: app.clone(),
        n: data::normalize(&app.name),
        g: data::normalize(&app.generic),
        k,
        c: data::normalize(&app.comment),
        id: data::normalize(&id.split_whitespace().collect::<Vec<_>>().join(" ")),
    }
}

/* ======================================================= settings registry */

pub struct SettingPage {
    pub id: String,
    pub parent: Option<String>,
    pub icon: String,
    title: [String; 2],
    desc: [String; 2],
    kw: [Vec<String>; 2],
}

impl SettingPage {
    pub fn title(&self, shell: &Shell) -> String {
        self.title[usize::from(shell.lang() == "en")].clone()
    }
    fn desc(&self, shell: &Shell) -> String {
        self.desc[usize::from(shell.lang() == "en")].clone()
    }
}

fn load_pages() -> Vec<SettingPage> {
    let v: Value = serde_json::from_str(include_str!("settings_pages.json")).unwrap_or(Value::Null);
    let pair = |x: &Value| [data::text(x, "nl"), data::text(x, "en")];
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|p| {
                    Some(SettingPage {
                        id: p["id"].as_str()?.to_owned(),
                        parent: p["parent"].as_str().map(str::to_owned),
                        icon: data::text(p, "icon"),
                        title: pair(&p["title"]),
                        desc: pair(&p["description"]),
                        kw: [data::str_list(&p["keywords"]["nl"]), data::str_list(&p["keywords"]["en"])],
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/* ================================================================= actions */

struct Act {
    id: &'static str,
    glyph: &'static str,
    k: &'static str,
}

const ACTIONS: &[Act] = &[
    Act { id: "dark", glyph: "moon", k: "donker donkere modus dark mode thema nachtmodus zwart" },
    Act { id: "light", glyph: "sun", k: "licht lichte modus light mode thema wit" },
    Act { id: "taskmgr", glyph: "chart", k: "taakbeheer task manager processen vastgelopen reageert niet ctrl alt delete" },
    Act { id: "trash", glyph: "trash", k: "prullenbak leegmaken empty recycle bin trash verwijderen leeg" },
    Act { id: "lock", glyph: "lock", k: "vergrendelen lock scherm op slot" },
    Act { id: "logout", glyph: "signout", k: "afmelden uitloggen sign out log out" },
    Act { id: "shutdown", glyph: "power", k: "afsluiten uitzetten shut down power off computer uit" },
    Act { id: "reboot", glyph: "restart", k: "opnieuw opstarten herstarten restart reboot" },
    Act { id: "suspend", glyph: "sleep", k: "slaapstand sleep suspend slapen" },
    Act { id: "settings", glyph: "settings", k: "instellingen settings configuratiescherm control panel" },
    Act { id: "cleanup", glyph: "broom", k: "ruimte vrijmaken opruimen schijfopruiming disk cleanup free up space schijf vol" },
    Act { id: "storage", glyph: "storage", k: "schijfruimte opslag disk space storage schijven" },
    Act { id: "clipboard", glyph: "clipboard", k: "klembord klembordgeschiedenis clipboard history kopieren plakken" },
    Act { id: "alles", glyph: "search", k: "alles zoeken everything bestanden zoeken find files" },
    Act { id: "quick", glyph: "settings", k: "snelle instellingen quick settings wifi geluid helderheid" },
    Act { id: "notif", glyph: "notification", k: "meldingen notifications agenda kalender berichten" },
    Act { id: "dnd", glyph: "notification-off", k: "niet storen do not disturb stil focus meldingen uit" },
    Act { id: "trans", glyph: "theme", k: "transparantie transparency doorzichtig blur effecten" },
    Act { id: "taskview", glyph: "taskview", k: "taakweergave task view vensters alt tab" },
    Act { id: "desktop", glyph: "desktop", k: "bureaublad weergeven show desktop minimaliseren" },
    Act { id: "align", glyph: "taskbar", k: "taakbalk uitlijnen links centreren taskbar alignment left center" },
    Act { id: "explorer", glyph: "folder", k: "verkenner openen file explorer bestanden mappen" },
];

fn act(id: &str) -> Option<&'static Act> {
    ACTIONS.iter().find(|a| a.id == id)
}

fn dnd_on(shell: &Shell) -> bool {
    shell.setting("notifications.dnd") == true
}

fn transparency_on(shell: &Shell) -> bool {
    shell.setting("theme.transparency") != false
}

fn align_left(shell: &Shell) -> bool {
    shell.setting("taskbar.alignment").as_str() == Some("left")
}

fn act_title(shell: &Shell, id: &str) -> String {
    let (nl, en) = match id {
        "dark" => ("Donkere modus inschakelen", "Turn on dark mode"),
        "light" => ("Lichte modus inschakelen", "Turn on light mode"),
        "taskmgr" => ("Taakbeheer openen", "Open Task Manager"),
        "trash" => ("Prullenbak leegmaken", "Empty Recycle Bin"),
        "lock" => ("Vergrendelen", "Lock"),
        "logout" => ("Afmelden", "Sign out"),
        "shutdown" => ("Afsluiten", "Shut down"),
        "reboot" => ("Opnieuw opstarten", "Restart"),
        "suspend" => ("Slaapstand", "Sleep"),
        "settings" => ("Instellingen", "Settings"),
        "cleanup" => ("Ruimte vrijmaken", "Free up space"),
        "storage" => ("Schijfruimte bekijken", "View disk space"),
        "clipboard" => ("Klembordgeschiedenis", "Clipboard history"),
        "alles" => ("Alles zoeken", "Search with Alles"),
        "quick" => ("Snelle instellingen", "Quick settings"),
        "notif" => ("Meldingen weergeven", "Show notifications"),
        "dnd" if dnd_on(shell) => ("Niet storen uitschakelen", "Turn off do not disturb"),
        "dnd" => ("Niet storen inschakelen", "Turn on do not disturb"),
        "trans" if transparency_on(shell) => ("Transparantie-effecten uitschakelen", "Turn off transparency effects"),
        "trans" => ("Transparantie-effecten inschakelen", "Turn on transparency effects"),
        "taskview" => ("Taakweergave", "Task view"),
        "desktop" => ("Bureaublad weergeven", "Show desktop"),
        "align" if align_left(shell) => ("Taakbalk centreren", "Centre taskbar"),
        "align" => ("Taakbalk links uitlijnen", "Align taskbar left"),
        "explorer" => ("Verkenner openen", "Open File Explorer"),
        _ => ("", ""),
    };
    shell.t(nl, en)
}

fn act_desc(shell: &Shell, id: &str) -> String {
    let (nl, en) = match id {
        "dark" => ("Donkere kleuren voor Windows en apps", "Dark colours for Windows and apps"),
        "light" => ("Lichte kleuren voor Windows en apps", "Light colours for Windows and apps"),
        "taskmgr" => ("Zie welke apps je computer bezighouden", "See which apps keep your computer busy"),
        "trash" => ("Maakt ruimte vrij; vraagt eerst om bevestiging", "Frees up space; asks first"),
        "lock" => ("Je scherm vergrendelen", "Lock your screen"),
        "logout" => ("Je sessie afsluiten", "End your session"),
        "shutdown" => ("De computer uitzetten", "Turn the computer off"),
        "reboot" => ("De computer opnieuw opstarten", "Restart the computer"),
        "suspend" => ("De computer laten slapen", "Put the computer to sleep"),
        "settings" => ("Instellingen openen", "Open Settings"),
        "cleanup" => ("Ruim tijdelijke bestanden en de Prullenbak op", "Clean temporary files and the Recycle Bin"),
        "storage" => ("Zie wat ruimte inneemt op je schijven", "See what takes up space on your drives"),
        "clipboard" => ("Eerder gekopieerde tekst en afbeeldingen", "Text and images you copied earlier"),
        "alles" => ("Elk bestand op je computer vinden", "Find any file on your computer"),
        "quick" => ("Wifi, geluid, helderheid en meer", "Wi-Fi, sound, brightness and more"),
        "notif" => ("Meldingen en agenda", "Notifications and calendar"),
        "dnd" => ("Meldingen tijdelijk stil", "Silence notifications for now"),
        "trans" => ("Doorschijnende vensters en menu’s", "See-through windows and menus"),
        "taskview" => ("Alle open vensters bekijken", "See all open windows"),
        "desktop" => ("Alle vensters minimaliseren", "Minimise all windows"),
        "align" => ("Plaats van Start en de apps op de taakbalk", "Where Start and apps sit on the taskbar"),
        "explorer" => ("Je bestanden en mappen", "Your files and folders"),
        _ => ("", ""),
    };
    shell.t(nl, en)
}

fn act_when(shell: &Shell, id: &str) -> bool {
    match id {
        "dark" => !shell.dark(),
        "light" => shell.dark(),
        _ => true,
    }
}

fn open_flyout(name: &'static str) {
    let shell = crate::core::shell();
    let host = shell.flyouts.borrow().clone();
    if let Some(host) = host {
        host.open(name);
    }
}

fn run_action(shell: &Rc<Shell>, id: &str, q: &str) {
    let power = |action: &str| crate::flyouts::power::confirm(shell, action);
    match id {
        "dark" => shell.set_setting("theme.mode", json!("dark")),
        "light" => shell.set_setting("theme.mode", json!("light")),
        "taskmgr" => data::open_app(shell, "taskmgr", json!({})),
        "trash" => empty_trash(shell),
        "lock" | "logout" | "shutdown" | "reboot" | "suspend" => power(id),
        "settings" => data::open_app(shell, "settings", json!({})),
        "cleanup" => data::open_app(shell, "taskmgr", json!({"page": "storage", "tab": "cleanup"})),
        "storage" => data::open_app(shell, "taskmgr", json!({"page": "storage"})),
        // no clipboard flyout of its own: the Settings page Klembord (its history and Super+V)
        "clipboard" => data::open_app(shell, "settings", json!({"page": "clipboard"})),
        "alles" => data::open_app(shell, "alles", if q.is_empty() { json!({}) } else { json!({"query": q}) }),
        "quick" => open_flyout("quick"),
        "notif" => open_flyout("notifications"),
        "dnd" => shell.set_setting("notifications.dnd", json!(!dnd_on(shell))),
        "trans" => shell.set_setting("theme.transparency", json!(!transparency_on(shell))),
        "taskview" => open_flyout("taskview"),
        "desktop" => shell.gnome.send("windows.showDesktop", json!({})),
        "align" => shell.set_setting("taskbar.alignment", json!(if align_left(shell) { "center" } else { "left" })),
        "explorer" => data::open_app(shell, "explorer", json!({})),
        _ => {}
    }
}

/// "Prullenbak leegmaken": asks first, as the HTML's Dialog.confirm.
fn empty_trash(shell: &Rc<Shell>) {
    let shell2 = shell.clone();
    shell.call("files.trashInfo", json!({}), move |info| {
        let shell = shell2;
        let info = info.ok();
        let count = info.as_ref().map(|i| data::num(&i["count"]));
        if count == Some(0) {
            let d = gtk::AlertDialog::builder()
                .message(shell.t("De Prullenbak is al leeg.", "The Recycle Bin is already empty."))
                .buttons([shell.t("OK", "OK")])
                .build();
            d.choose(None::<&gtk::Window>, None::<&gio::Cancellable>, |_| {});
            return;
        }
        let extra = match (&info, count) {
            (Some(i), Some(n)) => {
                let size = data::bytes(&shell, i["size"].as_u64().unwrap_or(0));
                if n == 1 {
                    shell.t(&format!("Er staat 1 item ({size}) in de Prullenbak. "), &format!("There is 1 item ({size}) in the Recycle Bin. "))
                } else {
                    shell.t(&format!("Er staan {n} items ({size}) in de Prullenbak. "), &format!("There are {n} items ({size}) in the Recycle Bin. "))
                }
            }
            _ => String::new(),
        };
        let detail = extra
            + &shell.t(
                "Alles in de Prullenbak wordt definitief verwijderd. Dit kun je niet ongedaan maken.",
                "Everything in the Recycle Bin will be permanently deleted. This can’t be undone.",
            );
        let d = gtk::AlertDialog::builder()
            .message(shell.t("Prullenbak leegmaken?", "Empty Recycle Bin?"))
            .detail(detail)
            .buttons([shell.t("Annuleren", "Cancel"), shell.t("Prullenbak leegmaken", "Empty Recycle Bin")])
            .cancel_button(0)
            .default_button(0)
            .build();
        let s = shell.clone();
        d.choose(None::<&gtk::Window>, None::<&gio::Cancellable>, move |answer| {
            if answer == Ok(1) {
                s.fire("files.emptyTrash", json!({}));
            }
        });
    });
}

/* =================================================================== hits */

#[derive(Clone)]
pub enum Kind {
    App(App),
    Setting(usize),
    Action(&'static str),
    File(FileItem),
    Web(String),
    Url(String),
    Path { path: String, is_dir: bool, name: String },
    Alles(String),
}

#[derive(Clone)]
pub struct Hit {
    s: f64,
    kind: Kind,
    key: String,
}

impl Hit {
    fn kind_class(&self) -> &'static str {
        match self.kind {
            Kind::App(_) => "is-app",
            Kind::Setting(_) => "is-setting",
            Kind::Action(_) => "is-action",
            Kind::File(_) => "is-file",
            Kind::Web(_) => "is-web",
            Kind::Url(_) => "is-url",
            Kind::Path { .. } => "is-path",
            Kind::Alles(_) => "is-alles",
        }
    }
}

const TABS: [&str; 6] = ["all", "apps", "files", "settings", "actions", "web"];

#[derive(Default)]
struct Files {
    q: Option<String>,
    items: Vec<FileItem>,
    loading: bool,
}

pub struct Search {
    shell: Rc<Shell>,
    pub root: gtk::Box,
    tabs: gtk::Box,
    list: gtk::Box,
    scroll: gtk::ScrolledWindow,
    detail: gtk::Box,
    query: RefCell<String>,
    qn: RefCell<String>,
    tab: Cell<usize>,
    flat: RefCell<Vec<Hit>>,
    rows: RefCell<Vec<gtk::Box>>,
    sel: Cell<usize>,
    user_moved: Cell<bool>,
    files: RefCell<Files>,
    file_seq: Cell<u64>,
    pending: RefCell<Option<(String, glib::SourceId)>>,
    detail_key: RefCell<Option<String>>,
    index: RefCell<Vec<AppEntry>>,
    pub default_ids: RefCell<HashSet<String>>,
    pages: Vec<SettingPage>,
    recent: RefCell<Option<Vec<FileItem>>>,
}

impl Search {
    pub fn new(shell: &Rc<Shell>) -> Rc<Search> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("w11-search");
        let tabs = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        tabs.add_css_class("w11-search-tabs");
        root.append(&tabs);
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        content.add_css_class("w11-search-content");
        content.set_vexpand(true);
        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        list.add_css_class("w11-sr-list");
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scroll.set_child(Some(&list));
        scroll.set_size_request(280, -1);
        scroll.set_propagate_natural_width(false);
        scroll.add_css_class("w11-sr-scroll");
        let detail = gtk::Box::new(gtk::Orientation::Vertical, 0);
        detail.add_css_class("w11-sr-detail");
        detail.set_hexpand(true);
        detail.set_size_request(290, -1);
        content.append(&scroll);
        content.append(&detail);
        root.append(&content);
        let search = Rc::new(Search {
            shell: shell.clone(),
            root,
            tabs,
            list,
            scroll,
            detail,
            query: RefCell::new(String::new()),
            qn: RefCell::new(String::new()),
            tab: Cell::new(0),
            flat: RefCell::new(Vec::new()),
            rows: RefCell::new(Vec::new()),
            sel: Cell::new(0),
            user_moved: Cell::new(false),
            files: RefCell::new(Files::default()),
            file_seq: Cell::new(0),
            pending: RefCell::new(None),
            detail_key: RefCell::new(None),
            index: RefCell::new(Vec::new()),
            default_ids: RefCell::new(HashSet::new()),
            pages: load_pages(),
            recent: RefCell::new(None),
        });
        search.render_tabs();
        search
    }

    pub fn set_apps(&self, apps: &[App]) {
        *self.index.borrow_mut() = apps.iter().map(app_entry).collect();
    }

    /* ------------------------------------------------------------ query */

    pub fn set_query(self: &Rc<Self>, q: &str) {
        self.cancel_pending();
        *self.query.borrow_mut() = q.to_owned();
        *self.qn.borrow_mut() = data::normalize(q);
        self.sel.set(0);
        self.user_moved.set(false);
        self.update(false);
        let qn = self.qn.borrow().clone();
        if !qn.is_empty() {
            self.load_files(qn);
        }
    }

    /// Back to tab "Alle" and an empty query (Start closed).
    pub fn reset(self: &Rc<Self>) {
        self.cancel_pending();
        self.tab.set(0);
        self.render_tabs();
        *self.recent.borrow_mut() = None;
        self.set_query("");
    }

    /// Settings, language or apps changed: labels, results and the detail pane again.
    pub fn refresh(self: &Rc<Self>) {
        self.render_tabs();
        if !self.qn.borrow().is_empty() {
            *self.detail_key.borrow_mut() = None;
            self.update(true);
        }
    }

    fn set_tab(self: &Rc<Self>, i: usize) {
        self.cancel_pending();
        self.tab.set(i % TABS.len());
        self.sel.set(0);
        self.user_moved.set(false);
        self.render_tabs();
        self.update(false);
    }

    fn render_tabs(self: &Rc<Self>) {
        clear(&self.tabs);
        for (i, id) in TABS.iter().enumerate() {
            let label = match *id {
                "all" => self.shell.t("Alle", "All"),
                "apps" => "Apps".to_owned(),
                "files" => self.shell.t("Bestanden", "Files"),
                "settings" => self.shell.t("Instellingen", "Settings"),
                "actions" => self.shell.t("Acties", "Actions"),
                _ => "Web".to_owned(),
            };
            let b = gtk::Button::new();
            b.add_css_class("w11-tab");
            b.set_widget_name(&format!("start-tab-{id}"));
            b.set_focus_on_click(false);
            let o = gtk::Overlay::new();
            let l = gtk::Label::new(Some(&label));
            o.set_child(Some(&l));
            let ind = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            ind.add_css_class("w11-tab-ind");
            ind.set_halign(gtk::Align::Center);
            ind.set_valign(gtk::Align::End);
            ind.set_size_request(16, 3);
            o.add_overlay(&ind);
            b.set_child(Some(&o));
            if i == self.tab.get() {
                b.add_css_class("is-active");
            }
            let weak = Rc::downgrade(self);
            b.connect_clicked(move |_| {
                if let Some(s) = weak.upgrade() {
                    s.set_tab(i);
                }
            });
            self.tabs.append(&b);
        }
    }

    /* ------------------------------------------------------------ files */

    fn files_pending(&self) -> bool {
        let qn = self.qn.borrow();
        let f = self.files.borrow();
        !qn.is_empty() && (f.loading || f.q.as_deref() != Some(qn.as_str()))
    }

    fn load_files(self: &Rc<Self>, q: String) {
        let seq = self.file_seq.get() + 1;
        self.file_seq.set(seq);
        let weak = Rc::downgrade(self);
        glib::timeout_add_local_once(Duration::from_millis(110), move || {
            let Some(me) = weak.upgrade() else { return };
            if me.file_seq.get() != seq {
                return;
            }
            {
                let mut f = me.files.borrow_mut();
                let keep = f.q.as_deref() == Some(q.as_str());
                let items = if keep { std::mem::take(&mut f.items) } else { Vec::new() };
                *f = Files { q: Some(q.clone()), items, loading: true };
            }
            me.update(true);
            let limit = if TABS[me.tab.get()] == "files" { 30 } else { 8 };
            let weak = Rc::downgrade(&me);
            me.shell.call(
                "index.search",
                json!({"query": q, "limit": limit, "sort": "name", "channel": "start"}),
                move |result| {
                    let Some(me) = weak.upgrade() else { return };
                    if me.file_seq.get() != seq {
                        return;
                    }
                    match result {
                        Ok(v) => {
                            let items = v["items"].as_array().map(|a| a.iter().filter_map(FileItem::from_json).collect()).unwrap_or_default();
                            me.files_done(&q, items);
                        }
                        Err(e) if e.to_lowercase().contains("cancel") => {
                            if *me.qn.borrow() == q {
                                *me.files.borrow_mut() = Files { q: Some(q.clone()), items: Vec::new(), loading: false };
                                me.update(true);
                            }
                        }
                        Err(_) => me.files_fallback(q, seq),
                    }
                },
            );
        });
    }

    /// No index: the recent files that match.
    fn files_fallback(self: &Rc<Self>, q: String, seq: u64) {
        let cached = self.recent.borrow().clone();
        let filter = move |me: &Rc<Search>, recent: Vec<FileItem>| {
            let mut hits: Vec<(f64, FileItem)> =
                recent.into_iter().map(|f| (score_text(&q, &data::normalize(&f.name), true), f)).filter(|(s, _)| *s >= 40.0).collect();
            hits.sort_by(|a, b| b.0.total_cmp(&a.0));
            let items = hits.into_iter().take(6).map(|(_, f)| f).collect();
            me.files_done(&q, items);
        };
        if let Some(recent) = cached {
            filter(self, recent);
            return;
        }
        let weak = Rc::downgrade(self);
        self.shell.call("apps.recent", json!({"limit": 60}), move |result| {
            let Some(me) = weak.upgrade() else { return };
            let recent: Vec<FileItem> =
                result.ok().and_then(|v| v.as_array().map(|a| a.iter().filter_map(FileItem::from_json).collect())).unwrap_or_default();
            *me.recent.borrow_mut() = Some(recent.clone());
            if me.file_seq.get() == seq {
                filter(&me, recent);
            }
        });
    }

    fn files_done(self: &Rc<Self>, q: &str, items: Vec<FileItem>) {
        if *self.qn.borrow() != q {
            return;
        }
        *self.files.borrow_mut() = Files { q: Some(q.to_owned()), items, loading: false };
        self.update(true);
        let run = self.pending.borrow().as_ref().is_some_and(|(pq, _)| pq == q);
        if run {
            self.cancel_pending();
            let item = self.flat.borrow().get(self.sel.get()).cloned();
            self.run_item(item);
        }
    }

    pub fn cancel_pending(&self) {
        if let Some((_, id)) = self.pending.borrow_mut().take() {
            id.remove();
        }
    }

    /// Enter: open the selected result, or wait a moment for file hits when the web search would win now.
    pub fn activate(self: &Rc<Self>) {
        let item = self.flat.borrow().get(self.sel.get()).cloned();
        let Some(item) = item else { return };
        let qn = self.qn.borrow().clone();
        let stale_file = matches!(item.kind, Kind::File(_)) && self.files.borrow().q.as_deref() != Some(qn.as_str());
        if (matches!(item.kind, Kind::Web(_)) || stale_file) && !self.user_moved.get() && self.files_pending() {
            self.cancel_pending();
            let weak = Rc::downgrade(self);
            let q = qn.clone();
            let id = glib::timeout_add_local_once(Duration::from_millis(900), move || {
                let Some(me) = weak.upgrade() else { return };
                let due = me.pending.borrow().as_ref().is_some_and(|(pq, _)| *pq == q);
                if due {
                    // The source has run: forget it without removing it.
                    let _ = me.pending.borrow_mut().take().map(|(_, id)| std::mem::forget(id));
                    let item = me.flat.borrow().get(me.sel.get()).cloned();
                    me.run_item(item);
                }
            });
            *self.pending.borrow_mut() = Some((qn, id));
            return;
        }
        self.run_item(Some(item));
    }

    /* ---------------------------------------------------------- matching */

    fn match_apps(&self, limit: usize) -> Vec<Hit> {
        let qn = self.qn.borrow();
        let defaults = self.default_ids.borrow();
        let mut out: Vec<Hit> = Vec::new();
        for e in self.index.borrow().iter() {
            let mut s = score_text(&qn, &e.n, true);
            if s < 100.0 {
                s = s.max(score_text(&qn, &e.g, true) * 0.86).max(score_text(&qn, &e.id, false) * 0.7);
                for k in &e.k {
                    s = s.max(score_text(&qn, k, false) * 0.84);
                }
                if qn.chars().count() >= 3 && e.c.contains(qn.as_str()) {
                    s = s.max(40.0);
                }
            }
            if s >= 40.0 {
                let bonus = if defaults.contains(&e.app.id) { 1.5 } else { 0.0 };
                out.push(Hit { s: s + 3.0 + bonus, key: format!("app:{}", e.app.id), kind: Kind::App(e.app.clone()) });
            }
        }
        out.sort_by(|a, b| {
            b.s.total_cmp(&a.s).then_with(|| match (&a.kind, &b.kind) {
                (Kind::App(x), Kind::App(y)) => data::sort_key(&x.name).cmp(&data::sort_key(&y.name)),
                _ => std::cmp::Ordering::Equal,
            })
        });
        out.truncate(limit);
        out
    }

    /// registry.js searchSettings, then search.js matchSettings's per-word score.
    fn match_settings(&self, limit: usize) -> Vec<Hit> {
        let q = data::normalize(self.query.borrow().trim());
        if q.is_empty() {
            return Vec::new();
        }
        let ws: Vec<&str> = q.split_whitespace().collect();
        let lang = usize::from(self.shell.lang() == "en");
        let other = 1 - lang;
        let mut scored: Vec<(usize, f64)> = Vec::new();
        for (i, page) in self.pages.iter().enumerate() {
            let title = data::normalize(&page.title[lang]);
            let mut total = 0.0;
            for w in &ws {
                let mut best: f64 = if title == *w {
                    100.0
                } else if title.starts_with(w) {
                    80.0
                } else if title.split([' ', '-']).any(|x| x.starts_with(w)) {
                    65.0
                } else if title.contains(w) {
                    50.0
                } else {
                    0.0
                };
                for k in &page.kw[lang] {
                    let nk = data::normalize(k);
                    if nk == *w {
                        best = best.max(60.0);
                    } else if nk.starts_with(w) {
                        best = best.max(45.0);
                    } else if nk.contains(w) && w.chars().count() > 2 {
                        best = best.max(30.0);
                    }
                }
                if best == 0.0 && w.chars().count() > 2 && data::normalize(&page.desc[lang]).contains(w) {
                    best = 20.0;
                }
                if best == 0.0 {
                    if data::normalize(&page.title[other]).starts_with(w) {
                        best = 25.0;
                    } else if page.kw[other].iter().any(|k| data::normalize(k).starts_with(w)) {
                        best = 15.0;
                    }
                }
                if best == 0.0 {
                    total = 0.0;
                    break;
                }
                total += best;
            }
            if total > 0.0 {
                scored.push((i, total + if page.parent.is_none() { 2.0 } else { 0.0 }));
            }
        }
        scored.sort_by(|a, b| {
            b.1.total_cmp(&a.1)
                .then_with(|| data::sort_key(&self.pages[a.0].title[lang]).cmp(&data::sort_key(&self.pages[b.0].title[lang])))
        });
        let words = ws.len().max(1) as f64;
        scored
            .into_iter()
            .take(limit)
            .map(|(i, score)| (i, (score / words).min(96.0)))
            .filter(|(_, s)| *s >= 20.0)
            .map(|(i, s)| Hit { s, key: format!("set:{}", self.pages[i].id), kind: Kind::Setting(i) })
            .collect()
    }

    fn match_actions(&self, limit: usize) -> Vec<Hit> {
        let qn = self.qn.borrow();
        let mut out: Vec<Hit> = Vec::new();
        for a in ACTIONS {
            if !act_when(&self.shell, a.id) {
                continue;
            }
            let kn = data::normalize(a.k);
            let mut s = score_text(&qn, &data::normalize(&act_title(&self.shell, a.id)), true);
            for k in kn.split(' ') {
                s = s.max(score_text(&qn, k, false) * 0.8);
            }
            if qn.contains(' ') && kn.contains(qn.as_str()) {
                s = s.max(70.0);
            }
            if s >= 45.0 {
                out.push(Hit { s, key: format!("act:{}", a.id), kind: Kind::Action(a.id) });
            }
        }
        out.sort_by(|a, b| b.s.total_cmp(&a.s));
        out.truncate(limit);
        out
    }

    fn file_results(&self, limit: usize) -> Vec<Hit> {
        self.files
            .borrow()
            .items
            .iter()
            .take(limit)
            .map(|f| Hit { s: 40.0, key: format!("file:{}", f.path), kind: Kind::File(f.clone()) })
            .collect()
    }

    fn web_result(&self) -> Hit {
        let q = self.query.borrow().trim().to_owned();
        Hit { s: 1.0, key: format!("web:{q}"), kind: Kind::Web(q) }
    }

    fn alles_result(&self) -> Hit {
        let q = self.query.borrow().trim().to_owned();
        Hit { s: 0.0, key: format!("alles:{q}"), kind: Kind::Alles(q) }
    }

    /// Win+R style: a typed folder or file path or web address opens directly.
    fn direct_result(&self) -> Option<Hit> {
        let raw = self.query.borrow().trim().to_owned();
        let lower = raw.to_lowercase();
        if (lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("www."))
            && !raw.contains(char::is_whitespace)
            && raw.len() > 4
        {
            let url = if lower.starts_with("www.") { format!("https://{raw}") } else { raw.clone() };
            return Some(Hit { s: 99.0, key: format!("url:{raw}"), kind: Kind::Url(url) });
        }
        if raw.starts_with("~/") || raw == "~" || raw.starts_with('/') || raw.starts_with("file://") {
            let home = data::home_dir();
            let mut path = raw.strip_prefix("file://").unwrap_or(&raw).to_owned();
            if path == "~" || path.starts_with("~/") {
                path = format!("{home}{}", &path[1..]);
            }
            if let Some(p) = glib::Uri::unescape_string(&path, None::<&str>) {
                path = p.to_string();
            }
            let leaf = data::leaf(&path);
            let has_ext = leaf.rfind('.').is_some_and(|i| {
                let ext = &leaf[i + 1..];
                (1..=5).contains(&ext.len()) && ext.chars().all(|c| c.is_ascii_alphanumeric())
            });
            let is_dir = path.ends_with('/') || !has_ext;
            return Some(Hit { s: 99.0, key: format!("path:{path}"), kind: Kind::Path { path, is_dir, name: leaf } });
        }
        None
    }

    /* ------------------------------------------------------------ render */

    pub fn update(self: &Rc<Self>, keep_sel: bool) {
        let qn = self.qn.borrow().clone();
        if qn.is_empty() {
            clear(&self.list);
            clear(&self.detail);
            self.rows.borrow_mut().clear();
            self.flat.borrow_mut().clear();
            *self.detail_key.borrow_mut() = None;
            self.tabs.set_visible(false);
            return;
        }
        self.tabs.set_visible(true);
        let prev_key = if keep_sel && self.user_moved.get() {
            self.flat.borrow().get(self.sel.get()).map(|h| h.key.clone())
        } else {
            None
        };
        let loading = self.files.borrow().loading;
        let t = |nl: &str, en: &str| self.shell.t(nl, en);
        let mut sections: Vec<(bool, String, Vec<Hit>, bool)> = Vec::new();
        match TABS[self.tab.get()] {
            "all" => {
                let (apps, sets, acts, files) = (self.match_apps(8), self.match_settings(5), self.match_actions(4), self.file_results(6));
                let mut cands: Vec<Hit> = [apps.first(), sets.first(), acts.first()].into_iter().flatten().cloned().collect();
                cands.sort_by(|a, b| b.s.total_cmp(&a.s));
                let best = self
                    .direct_result()
                    .or_else(|| cands.first().filter(|c| c.s >= 50.0).cloned())
                    .or_else(|| files.first().cloned())
                    .unwrap_or_else(|| self.web_result());
                let rest = |v: Vec<Hit>| -> Vec<Hit> { v.into_iter().filter(|x| x.key != best.key).collect() };
                let best_web = matches!(best.kind, Kind::Web(_));
                sections.push((true, t("Beste overeenkomst", "Best match"), vec![best.clone()], false));
                let a: Vec<Hit> = rest(apps).into_iter().take(5).collect();
                if !a.is_empty() {
                    sections.push((false, "Apps".into(), a, false));
                }
                let s: Vec<Hit> = rest(sets).into_iter().take(4).collect();
                if !s.is_empty() {
                    sections.push((false, t("Instellingen", "Settings"), s, false));
                }
                let ac: Vec<Hit> = rest(acts).into_iter().take(3).collect();
                if !ac.is_empty() {
                    sections.push((false, t("Acties", "Actions"), ac, false));
                }
                let f = rest(files);
                if !f.is_empty() || loading {
                    let l = loading && f.is_empty();
                    sections.push((false, t("Bestanden", "Files"), f, l));
                }
                if !best_web {
                    sections.push((false, t("Zoeken op internet", "Search the web"), vec![self.web_result()], false));
                }
            }
            "apps" => sections.push((false, "Apps".into(), self.match_apps(40), false)),
            "settings" => sections.push((false, t("Instellingen", "Settings"), self.match_settings(30), false)),
            "actions" => sections.push((false, t("Acties", "Actions"), self.match_actions(30), false)),
            "files" => sections.push((false, t("Bestanden", "Files"), self.file_results(30), loading)),
            _ => sections.push((false, t("Zoeken op internet", "Search the web"), vec![self.web_result()], false)),
        }

        clear(&self.list);
        let mut flat: Vec<Hit> = Vec::new();
        let mut rows: Vec<gtk::Box> = Vec::new();
        for (best, title, items, sec_loading) in sections {
            if items.is_empty() && !sec_loading {
                continue;
            }
            let head = gtk::Label::new(Some(&title));
            head.add_css_class("w11-sr-head");
            head.set_xalign(0.0);
            if flat.is_empty() && rows.is_empty() {
                head.add_css_class("is-first");
            }
            self.list.append(&head);
            for item in items {
                let i = flat.len();
                let row = self.row(&item, i, best);
                self.list.append(&row);
                rows.push(row);
                flat.push(item);
            }
            if sec_loading {
                let l = gtk::Box::new(gtk::Orientation::Horizontal, 10);
                l.add_css_class("w11-sr-loading");
                let sp = gtk::Spinner::new();
                sp.start();
                l.append(&sp);
                l.append(&gtk::Label::new(Some(&t("Bestanden zoeken…", "Searching files…"))));
                self.list.append(&l);
            }
        }
        if flat.is_empty() && !loading {
            let empty = gtk::Box::new(gtk::Orientation::Vertical, 4);
            empty.add_css_class("w11-sr-empty");
            let q = self.query.borrow().trim().to_owned();
            let title = gtk::Label::new(Some(&t(&format!("Geen resultaten voor “{q}”"), &format!("No results for “{q}”"))));
            title.add_css_class("w11-sr-empty-title");
            title.set_xalign(0.0);
            title.set_wrap(true);
            title.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            let hint = gtk::Label::new(Some(&t(
                "Controleer de spelling of probeer andere woorden. Je kunt ook op internet of in Alles zoeken.",
                "Check the spelling or try other words. You can also search the web or Alles.",
            )));
            hint.add_css_class("w11-sr-empty-hint");
            hint.set_xalign(0.0);
            hint.set_wrap(true);
            empty.append(&title);
            empty.append(&hint);
            self.list.append(&empty);
            let web = self.web_result();
            let row = self.row(&web, 0, false);
            self.list.append(&row);
            rows.push(row);
            flat.push(web);
        }
        if matches!(TABS[self.tab.get()], "all" | "files") {
            let al = self.alles_result();
            let row = self.row(&al, flat.len(), false);
            self.list.append(&row);
            rows.push(row);
            flat.push(al);
        }
        let mut sel = self.sel.get();
        if let Some(k) = prev_key {
            sel = flat.iter().position(|h| h.key == k).unwrap_or(0);
        }
        sel = sel.min(flat.len().saturating_sub(1));
        *self.flat.borrow_mut() = flat;
        *self.rows.borrow_mut() = rows;
        self.select(sel, false);
    }

    fn row(self: &Rc<Self>, item: &Hit, i: usize, best: bool) -> gtk::Box {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        row.add_css_class("w11-sr-row");
        row.add_css_class(item.kind_class());
        row.set_widget_name(&format!("start-result-{i}"));
        if best {
            row.add_css_class("is-best");
        }
        let ind = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        ind.add_css_class("w11-sr-ind");
        ind.set_size_request(3, 16);
        ind.set_valign(gtk::Align::Center);
        row.append(&ind);
        let icon = self.item_icon(item, if best { 32 } else { 24 });
        icon.set_margin_start(9);
        icon.set_valign(gtk::Align::Center);
        row.append(&icon);
        let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
        text.set_valign(gtk::Align::Center);
        text.set_hexpand(true);
        text.set_margin_start(if best { 14 } else { 12 });
        let title = label(&self.title_of(item), "w11-sr-title");
        text.append(&title);
        let sub = self.sub_of(item, best);
        if !sub.is_empty() {
            text.append(&label(&sub, "w11-sr-sub"));
        }
        row.append(&text);
        if matches!(item.kind, Kind::Web(_) | Kind::Alles(_)) {
            let chev = crate::icons::glyph("chevron-right", 12);
            chev.add_css_class("w11-sr-chev");
            chev.set_margin_start(8);
            row.append(&chev);
        }
        let click = gtk::GestureClick::new();
        click.set_button(0);
        let weak = Rc::downgrade(self);
        let r = row.downgrade();
        click.connect_pressed(move |g, _, x, y| {
            if g.current_button() == 3
                && let Some(me) = weak.upgrade()
                && let Some(r) = r.upgrade()
            {
                me.select(i, false);
                me.context_menu(i, Anchor::At(r.upcast(), x, y));
            }
        });
        let weak = Rc::downgrade(self);
        click.connect_released(move |g, _, _, _| {
            if g.current_button() == 1
                && let Some(me) = weak.upgrade()
            {
                me.select(i, false);
                let item = me.flat.borrow().get(i).cloned();
                me.run_item(item);
            }
        });
        row.add_controller(click);
        row
    }

    fn select(self: &Rc<Self>, i: usize, scroll: bool) {
        self.sel.set(i);
        let rows = self.rows.borrow().clone();
        for (j, r) in rows.iter().enumerate() {
            if j == i { r.add_css_class("is-selected") } else { r.remove_css_class("is-selected") }
        }
        if scroll && let Some(r) = rows.get(i) {
            self.scroll_to(r);
        }
        let item = self.flat.borrow().get(i).cloned();
        let key = item.as_ref().map(|h| h.key.clone());
        if *self.detail_key.borrow() != key || self.detail.first_child().is_none() {
            self.render_detail(item.as_ref());
        }
    }

    fn scroll_to(&self, row: &gtk::Box) {
        let Some(b) = row.compute_bounds(&self.list) else { return };
        let adj = self.scroll.vadjustment();
        let (y, h) = (b.y() as f64, b.height() as f64);
        if y < adj.value() {
            adj.set_value(y);
        } else if y + h > adj.value() + adj.page_size() {
            adj.set_value(y + h - adj.page_size());
        }
    }

    fn title_of(&self, item: &Hit) -> String {
        match &item.kind {
            Kind::App(a) => a.name.clone(),
            Kind::Setting(i) => self.pages[*i].title(&self.shell),
            Kind::Action(id) => act_title(&self.shell, id),
            Kind::File(f) => f.name.clone(),
            Kind::Web(q) => q.clone(),
            Kind::Url(u) => u.clone(),
            Kind::Path { path, .. } => path.clone(),
            Kind::Alles(_) => self.shell.t("Meer resultaten in Alles", "More results in Alles"),
        }
    }

    fn type_of(&self, item: &Hit) -> String {
        let t = |nl: &str, en: &str| self.shell.t(nl, en);
        match &item.kind {
            Kind::App(_) => "App".into(),
            Kind::Setting(i) => {
                let page = &self.pages[*i];
                match page.parent.as_ref().and_then(|p| self.pages.iter().find(|x| &x.id == p)) {
                    Some(parent) => format!("{} · {}", t("Instelling", "Setting"), parent.title(&self.shell)),
                    None => t("Instelling", "Setting"),
                }
            }
            Kind::Action(_) => t("Actie", "Action"),
            Kind::File(f) => {
                if f.is_dir {
                    t("Map", "Folder")
                } else {
                    t("Bestand", "File")
                }
            }
            Kind::Web(_) => t("Zoekresultaten bekijken", "See search results"),
            Kind::Url(_) => t("Openen in de browser", "Open in browser"),
            Kind::Path { is_dir, .. } => {
                if *is_dir {
                    t("Map openen in Verkenner", "Open folder in File Explorer")
                } else {
                    t("Bestand openen", "Open file")
                }
            }
            Kind::Alles(_) => String::new(),
        }
    }

    fn sub_of(&self, item: &Hit, best: bool) -> String {
        match &item.kind {
            Kind::File(f) => data::pretty_dir(&self.shell, &f.dir),
            _ if best => self.type_of(item),
            Kind::Web(_) => self.shell.t("Zoekresultaten bekijken", "See search results"),
            _ => String::new(),
        }
    }

    fn item_icon(&self, item: &Hit, size: i32) -> gtk::Widget {
        let tile_px = (size as f64 * if size > 32 { 0.5 } else { 0.66 }).round() as i32;
        match &item.kind {
            Kind::App(a) => crate::icons::app_icon(&a.icon, size).upcast(),
            Kind::Setting(i) => {
                let icon = self.pages[*i].icon.clone();
                glyph_tile(if icon.is_empty() { "settings" } else { &icon }, size, tile_px, "is-setting")
            }
            Kind::Action(id) => glyph_tile(act(id).map(|a| a.glyph).unwrap_or("settings"), size, tile_px, "is-action"),
            Kind::Web(_) => glyph_tile("globe", size, tile_px, "is-web"),
            Kind::Url(_) => glyph_tile("link", size, tile_px, "is-web"),
            Kind::Path { path, is_dir, name } => {
                if *is_dir {
                    crate::icons::app_icon("folder", size).upcast()
                } else {
                    file_icon(&FileItem { path: path.clone(), name: name.clone(), ..Default::default() }, size)
                }
            }
            Kind::Alles(_) => {
                let has = self.index.borrow().iter().any(|e| e.app.id.starts_with("win11shell-alles"));
                let icon = self
                    .index
                    .borrow()
                    .iter()
                    .find(|e| e.app.id.starts_with("win11shell-alles"))
                    .map(|e| e.app.icon.clone())
                    .unwrap_or_default();
                if has && !icon.is_empty() {
                    crate::icons::app_icon(&icon, size).upcast()
                } else {
                    glyph_tile("search", size, tile_px, "")
                }
            }
            Kind::File(f) => file_icon(f, size),
        }
    }

    /* ------------------------------------------------------------ detail */

    fn render_detail(self: &Rc<Self>, item: Option<&Hit>) {
        *self.detail_key.borrow_mut() = item.map(|h| h.key.clone());
        clear(&self.detail);
        let Some(item) = item else { return };
        let t = |nl: &str, en: &str| self.shell.t(nl, en);
        let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
        card.add_css_class("w11-detail");
        card.set_vexpand(true);
        let hero = gtk::Box::new(gtk::Orientation::Vertical, 2);
        hero.add_css_class("w11-detail-hero");
        let icon = self.item_icon(item, 64);
        icon.set_halign(gtk::Align::Center);
        icon.set_margin_bottom(12);
        hero.append(&icon);
        let name = gtk::Label::new(Some(&self.title_of(item)));
        name.add_css_class("w11-detail-name");
        name.set_wrap(true);
        name.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        name.set_justify(gtk::Justification::Center);
        name.set_max_width_chars(24);
        hero.append(&name);
        let ty = if matches!(item.kind, Kind::Web(_)) { t("Zoeken op internet", "Search the web") } else { self.type_of(item) };
        if !ty.is_empty() {
            let l = gtk::Label::new(Some(&ty));
            l.add_css_class("w11-detail-type");
            l.set_wrap(true);
            l.set_justify(gtk::Justification::Center);
            hero.append(&l);
        }
        let desc = match &item.kind {
            Kind::App(a) => {
                if a.comment.is_empty() {
                    a.generic.clone()
                } else {
                    a.comment.clone()
                }
            }
            Kind::Setting(i) => self.pages[*i].desc(&self.shell),
            Kind::Action(id) => act_desc(&self.shell, id),
            Kind::Alles(_) => t("Elk bestand op je computer vinden", "Find any file on your computer"),
            _ => String::new(),
        };
        if !desc.is_empty() {
            let l = gtk::Label::new(Some(&desc));
            l.add_css_class("w11-detail-desc");
            l.set_wrap(true);
            l.set_justify(gtk::Justification::Center);
            l.set_max_width_chars(36);
            hero.append(&l);
        }
        card.append(&hero);
        if let Kind::File(f) = &item.kind {
            let info = gtk::Box::new(gtk::Orientation::Vertical, 6);
            info.add_css_class("w11-detail-info");
            let mut rows = vec![(t("Locatie", "Location"), data::pretty_dir(&self.shell, &f.dir))];
            if f.modified > 0 {
                rows.push((t("Gewijzigd", "Modified"), format!("{} {}", data::fmt_date_short(f.modified), data::fmt_time(f.modified))));
            }
            if !f.is_dir
                && let Some(size) = f.size
            {
                rows.push((t("Grootte", "Size"), data::bytes(&self.shell, size)));
            }
            for (k, v) in rows {
                let r = gtk::Box::new(gtk::Orientation::Horizontal, 12);
                let kl = gtk::Label::new(Some(&k));
                kl.add_css_class("w11-detail-k");
                kl.set_xalign(0.0);
                kl.set_size_request(76, -1);
                let vl = gtk::Label::new(Some(&v));
                vl.set_xalign(0.0);
                vl.set_hexpand(true);
                vl.set_wrap(true);
                vl.set_wrap_mode(gtk::pango::WrapMode::WordChar);
                r.append(&kl);
                r.append(&vl);
                info.append(&r);
            }
            card.append(&info);
        }
        let acts = gtk::Box::new(gtk::Orientation::Vertical, 2);
        acts.add_css_class("w11-detail-actions");
        for (i, (label_text, glyph, run)) in self.actions_for(item).into_iter().enumerate() {
            let b = gtk::Button::new();
            b.add_css_class("w11-detail-action");
            b.set_widget_name(&format!("start-detail-action-{i}"));
            let r = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            r.append(&crate::icons::glyph(glyph, 16));
            let l = gtk::Label::new(Some(&label_text));
            l.set_xalign(0.0);
            l.set_hexpand(true);
            l.set_ellipsize(gtk::pango::EllipsizeMode::End);
            r.append(&l);
            b.set_child(Some(&r));
            b.connect_clicked(move |_| run());
            acts.append(&b);
        }
        card.append(&acts);
        self.detail.append(&card);
    }

    fn first_detail_action(&self) -> Option<gtk::Widget> {
        let mut stack = vec![self.detail.first_child()?];
        while let Some(w) = stack.pop() {
            if w.has_css_class("w11-detail-action") {
                return Some(w);
            }
            if let Some(n) = w.next_sibling() {
                stack.push(n);
            }
            if let Some(c) = w.first_child() {
                stack.push(c);
            }
        }
        None
    }

    /// search.js actionsFor: (label, glyph, run).
    fn actions_for(self: &Rc<Self>, item: &Hit) -> Vec<(String, &'static str, Rc<dyn Fn()>)> {
        let t = |nl: &str, en: &str| self.shell.t(nl, en);
        let mut out: Vec<(String, &'static str, Rc<dyn Fn()>)> = Vec::new();
        let weak = Rc::downgrade(self);
        let it = item.clone();
        let open: Rc<dyn Fn()> = Rc::new(move || {
            if let Some(me) = weak.upgrade() {
                me.run_item(Some(it.clone()));
            }
        });
        let shell = self.shell.clone();
        match &item.kind {
            Kind::App(a) => {
                out.push((t("Openen", "Open"), "open", open));
                let id = a.id.clone();
                let on_start = super::start_pins().contains(&id);
                let i2 = id.clone();
                out.push(if on_start {
                    (t("Losmaken van Start", "Unpin from Start"), "unpin", Rc::new(move || super::unpin_start(&i2)) as Rc<dyn Fn()>)
                } else {
                    (t("Vastmaken aan Start", "Pin to Start"), "pin", Rc::new(move || super::pin_start(&i2)) as Rc<dyn Fn()>)
                });
                let on_tb = data::taskbar_pins(&shell).contains(&id);
                let s = shell.clone();
                out.push(if on_tb {
                    (
                        t("Losmaken van taakbalk", "Unpin from taskbar"),
                        "unpin",
                        Rc::new(move || data::set_taskbar_pins(&s, data::taskbar_pins(&s).into_iter().filter(|x| *x != id).collect()))
                            as Rc<dyn Fn()>,
                    )
                } else {
                    (
                        t("Vastmaken aan taakbalk", "Pin to taskbar"),
                        "pin",
                        Rc::new(move || {
                            let mut l: Vec<String> = data::taskbar_pins(&s).into_iter().filter(|x| *x != id).collect();
                            l.push(id.clone());
                            data::set_taskbar_pins(&s, l);
                        }) as Rc<dyn Fn()>,
                    )
                });
            }
            Kind::Setting(_) | Kind::Alles(_) => out.push((t("Openen", "Open"), "open", open)),
            Kind::Action(_) => out.push((t("Uitvoeren", "Run"), "play", open)),
            Kind::File(f) => {
                out.push((t("Openen", "Open"), "open", open));
                let (dir, path) = (f.dir.clone(), f.path.clone());
                let s = shell.clone();
                out.push((
                    t("Bestandslocatie openen", "Open file location"),
                    "folder-open",
                    Rc::new(move || {
                        super::dismiss();
                        data::open_app(&s, "explorer", json!({"path": dir, "select": path}));
                    }),
                ));
                let path = f.path.clone();
                let root = self.root.clone();
                out.push((t("Pad kopiëren", "Copy path"), "copy", Rc::new(move || root.clipboard().set_text(&path))));
            }
            Kind::Web(_) | Kind::Url(_) => out.push((t("Openen in de browser", "Open in browser"), "external", open)),
            Kind::Path { path, .. } => {
                out.push((t("Openen", "Open"), "open", open));
                let path = path.clone();
                let root = self.root.clone();
                out.push((t("Pad kopiëren", "Copy path"), "copy", Rc::new(move || root.clipboard().set_text(&path))));
            }
        }
        out
    }

    fn context_menu(self: &Rc<Self>, i: usize, anchor: Anchor) {
        let Some(item) = self.flat.borrow().get(i).cloned() else { return };
        let items: Vec<MItem> = self
            .actions_for(&item)
            .into_iter()
            .enumerate()
            .map(|(n, (label, glyph, run))| MItem::action(label, glyph, &format!("start-result-menu-{n}"), move || run()))
            .collect();
        if items.is_empty() {
            return;
        }
        if let Some(start) = super::get() {
            start.menu.show(items, anchor);
        }
    }

    /* --------------------------------------------------------------- run */

    fn run_item(self: &Rc<Self>, item: Option<Hit>) {
        self.cancel_pending();
        let Some(item) = item else { return };
        let shell = self.shell.clone();
        let q = self.query.borrow().trim().to_owned();
        super::dismiss();
        match item.kind {
            Kind::App(a) => data::launch(&shell, &a.id),
            Kind::Setting(i) => data::open_app(&shell, "settings", json!({"page": self.pages[i].id})),
            Kind::Action(id) => run_action(&shell, id, &q),
            Kind::File(f) => {
                if f.is_dir {
                    data::open_app(&shell, "explorer", json!({"path": f.path}));
                } else {
                    shell.fire("files.open", json!({"path": f.path}));
                }
            }
            Kind::Url(url) => shell.fire("apps.openUri", json!({"uri": url})),
            Kind::Path { path, is_dir, .. } => {
                if is_dir {
                    data::open_app(&shell, "explorer", json!({"path": path}));
                } else {
                    shell.fire("files.open", json!({"path": path}));
                }
            }
            Kind::Web(q) => {
                let mut engine = shell.setting("search.webEngine").as_str().unwrap_or("").to_owned();
                let lower = engine.to_lowercase();
                if !(lower.starts_with("http://") || lower.starts_with("https://")) {
                    engine = "https://duckduckgo.com/?q=".into();
                }
                let uri = format!("{engine}{}", glib::Uri::escape_string(&q, None, false));
                shell.fire("apps.openUri", json!({"uri": uri}));
            }
            Kind::Alles(q) => data::open_app(&shell, "alles", if q.is_empty() { json!({}) } else { json!({"query": q}) }),
        }
    }

    /* ----------------------------------------------------------- keyboard */

    /// Keys while the search view shows (search.js handleKey); true when handled.
    pub fn handle_key(self: &Rc<Self>, key: gdk::Key, state: gdk::ModifierType, in_entry: bool) -> bool {
        if state.contains(gdk::ModifierType::CONTROL_MASK) && (key == gdk::Key::Tab || key == gdk::Key::ISO_Left_Tab) {
            let n = TABS.len();
            let back = state.contains(gdk::ModifierType::SHIFT_MASK) || key == gdk::Key::ISO_Left_Tab;
            self.set_tab((self.tab.get() + if back { n - 1 } else { 1 }) % n);
            return true;
        }
        let len = self.flat.borrow().len();
        if self.qn.borrow().is_empty() || len == 0 {
            return false;
        }
        let sel = self.sel.get();
        let moved = |me: &Rc<Search>, to: usize| {
            me.cancel_pending();
            me.user_moved.set(true);
            me.select(to, true);
        };
        match key {
            gdk::Key::Down | gdk::Key::KP_Down => moved(self, (sel + 1).min(len - 1)),
            gdk::Key::Up | gdk::Key::KP_Up => moved(self, sel.saturating_sub(1)),
            gdk::Key::Page_Down => moved(self, (sel + 5).min(len - 1)),
            gdk::Key::Page_Up => moved(self, sel.saturating_sub(5)),
            gdk::Key::Return | gdk::Key::KP_Enter => self.activate(),
            gdk::Key::Tab if in_entry && !state.contains(gdk::ModifierType::SHIFT_MASK) => {
                let Some(first) = self.first_detail_action() else { return false };
                first.grab_focus();
            }
            gdk::Key::Menu => {
                let row = self.rows.borrow().get(sel).cloned();
                if let Some(row) = row {
                    self.context_menu(sel, Anchor::Over(row.upcast()));
                }
            }
            _ => return false,
        }
        true
    }
}

/* ================================================================ helpers */

pub fn clear(b: &impl IsA<gtk::Widget>) {
    let w = b.as_ref();
    while let Some(c) = w.first_child() {
        c.unparent();
    }
}

fn label(text: &str, class: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.add_css_class(class);
    l.set_xalign(0.0);
    l.set_ellipsize(gtk::pango::EllipsizeMode::End);
    l.set_max_width_chars(1);
    l.set_hexpand(true);
    l
}

pub fn glyph_tile(name: &str, size: i32, px: i32, class: &str) -> gtk::Widget {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 0);
    b.add_css_class("w11-glyph-tile");
    if !class.is_empty() {
        b.add_css_class(class);
    }
    b.set_size_request(size, size);
    b.set_halign(gtk::Align::Center);
    let g = crate::icons::glyph(name, px);
    g.set_valign(gtk::Align::Center);
    g.set_halign(gtk::Align::Center);
    g.set_vexpand(true);
    b.append(&g);
    b.upcast()
}

/// A file's icon: its theme icon (apps.recent), else its content type's icon.
pub fn file_icon(f: &FileItem, size: i32) -> gtk::Widget {
    if f.is_dir {
        return crate::icons::app_icon("folder", size).upcast();
    }
    if !f.icon.is_empty() && f.icon != "text-x-generic" {
        return crate::icons::app_icon(&f.icon, size).upcast();
    }
    let mime = if f.mime.is_empty() { gio::content_type_guess(Some(f.name.as_str()), None).0.to_string() } else { f.mime.clone() };
    let image = gtk::Image::from_gicon(&gio::content_type_get_icon(&mime));
    image.set_pixel_size(size);
    image.upcast()
}
