//! Instellingen › Apps › Geïnstalleerde apps (6ebf169f:vendor/ui/js/apps/settings/pages/installed.js): a searchable,
//! sortable, filterable list (apps.list) with an "…" menu per app: Openen, Vastmaken aan Start / taakbalk, and
//! Verwijderen with the reviewed plan of core/software.js (software.uninstallInfo, software.prepare,
//! software.execute, software.status), ported in `software` below.
use super::{Builder, Content, Page, PageFuture};
use crate::common;
use kit::controls::{self as c, Dropdown, MenuItem, SearchBox, SettingsCard, SettingsGroup, Toast};
use kit::prelude::*;
use kit::t;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

pub const PAGES: &[(&str, Builder)] = &[("installed", installed)];

/// css/software.css for the review and job dialogs.
pub const CSS: &str = r#"
.st-sw-name { font-size: 18px; font-weight: 600; }
.st-sw-description, .st-sw-note, .st-sw-job-action, .st-sw-job-message, .st-sw-dt { color: var(--text-2); }
.st-sw-note { font-size: 12px; }
"#;

const CAT_NL: &[(&str, &str)] = &[
    ("WebBrowser", "Webbrowser"),
    ("Email", "E-mail"),
    ("FileManager", "Bestandsbeheer"),
    ("TextEditor", "Teksteditor"),
    ("TerminalEmulator", "Terminal"),
    ("Game", "Spel"),
    ("Development", "Ontwikkeling"),
    ("Graphics", "Grafisch"),
    ("AudioVideo", "Audio en video"),
    ("Audio", "Audio"),
    ("Video", "Video"),
    ("Office", "Kantoor"),
    ("Utility", "Hulpmiddel"),
    ("System", "Systeem"),
    ("Settings", "Instellingen"),
    ("Network", "Internet"),
    ("Education", "Onderwijs"),
    ("Science", "Wetenschap"),
];
const CAT_EN: &[(&str, &str)] = &[
    ("WebBrowser", "Web browser"),
    ("Email", "Email"),
    ("FileManager", "File manager"),
    ("TextEditor", "Text editor"),
    ("TerminalEmulator", "Terminal"),
    ("Game", "Game"),
    ("Development", "Development"),
    ("Graphics", "Graphics"),
    ("AudioVideo", "Audio & video"),
    ("Audio", "Audio"),
    ("Video", "Video"),
    ("Office", "Office"),
    ("Utility", "Utility"),
    ("System", "System"),
    ("Settings", "Settings"),
    ("Network", "Internet"),
    ("Education", "Education"),
    ("Science", "Science"),
];
const PRIORITY: &[&str] = &[
    "WebBrowser", "Email", "FileManager", "TextEditor", "TerminalEmulator", "Game", "Office", "Development", "Graphics", "Audio", "Video",
    "AudioVideo", "Network", "Education", "Science", "Utility", "System", "Settings",
];

/// "Webbrowser · Surf op internet": the generic name or the main category, and the comment (installed.js describe).
fn describe(app: &Value) -> String {
    let id = common::value_key(&app["id"]);
    if id.starts_with("win11shell-") {
        return t("Onderdeel van WinPoint", "Part of WinPoint").to_owned();
    }
    let names = if kit::i18n::en() { CAT_EN } else { CAT_NL };
    let cats: Vec<&str> = app["categories"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    let cat = PRIORITY.iter().find(|c| cats.contains(c));
    let name = common::s(&app["name"]);
    let generic = common::s(&app["genericName"]);
    let kind = if !generic.is_empty() && generic != name {
        generic.to_owned()
    } else {
        cat.and_then(|c| names.iter().find(|(k, _)| k == c)).map(|(_, v)| (*v).to_owned()).unwrap_or_default()
    };
    let mut parts: Vec<String> = Vec::new();
    for v in [kind, common::s(&app["comment"]).to_owned()] {
        if !v.is_empty() && v != name && !parts.contains(&v) {
            parts.push(v);
        }
    }
    parts.join(" · ")
}

fn pins(path: &str) -> Vec<String> {
    kit::ctx().setting(path).as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect()).unwrap_or_default()
}

/// Pin or unpin an app id in start.pinned / taskbar.pinned (installed.js togglePin).
fn toggle_pin(p: &Page, path: &str, id: &str, on_msg: &str, off_msg: &str) {
    let mut list = pins(path);
    let had = list.iter().position(|x| x == id);
    match had {
        Some(i) => {
            list.remove(i);
        }
        None => list.push(id.to_owned()),
    }
    let msg = if had.is_some() { off_msg } else { on_msg }.to_owned();
    let anchor = p.anchor();
    common::set_setting(p, path, json!(list), move |ok| {
        if ok {
            common::toast_success(&anchor, &msg, 2000);
        }
    });
}

struct State {
    p: Page,
    apps: RefCell<Vec<Value>>,
    query: RefCell<String>,
    sort: RefCell<String>,
    filter: RefCell<String>,
    list: gtk::Box,
    count: gtk::Label,
    cards: RefCell<HashMap<String, gtk::Widget>>,
}

fn visible(apps: Vec<Value>) -> Vec<Value> {
    apps.into_iter().filter(|a| a["noDisplay"] != true).collect()
}

fn matches(app: &Value, q: &str) -> bool {
    if q.is_empty() {
        return true;
    }
    common::s(&app["name"]).to_lowercase().contains(q)
        || common::s(&app["genericName"]).to_lowercase().contains(q)
        || app["keywords"].as_array().is_some_and(|k| k.iter().any(|k| common::value_key(k).to_lowercase().contains(q)))
}

fn paint(st: &Rc<State>) {
    c::clear(&st.list);
    let q = st.query.borrow().trim().to_lowercase();
    let filter = st.filter.borrow().clone();
    let (pins_s, pins_t) = (pins("start.pinned"), pins("taskbar.pinned"));
    let mut items: Vec<Value> = st.apps.borrow().iter().filter(|a| matches(a, &q)).cloned().collect();
    let id_of = |a: &Value| common::value_key(&a["id"]);
    if filter == "start" {
        items.retain(|a| pins_s.contains(&id_of(a)));
    }
    if filter == "taskbar" {
        items.retain(|a| pins_t.contains(&id_of(a)));
    }
    let az = st.sort.borrow().as_str() == "az";
    items.sort_by(|a, b| {
        let o = kit::format::compare_natural(common::s(&a["name"]), common::s(&b["name"]));
        if az { o } else { o.reverse() }
    });
    st.count.set_label(&match items.len() {
        0 => t("Geen apps gevonden", "No apps found").to_owned(),
        n => common::plural(n as i64, t("1 app gevonden", "1 app found"), t("{n} apps gevonden", "{n} apps found")),
    });
    if items.is_empty() {
        st.list.append(&c::empty_state("apps", t("Geen apps gevonden", "No apps found"), t("Probeer een andere naam.", "Try another name."), None));
        return;
    }
    for app in &items {
        let id = id_of(app);
        let cached = st.cards.borrow().get(&id).cloned();
        let card = match cached {
            Some(w) => w,
            None => {
                let w = app_card(st, app);
                st.cards.borrow_mut().insert(id, w.clone());
                w
            }
        };
        st.list.append(&card);
    }
}

/// One row: the 32 px app icon, the name, what it is, and the "…" button (also right-click).
fn app_card(st: &Rc<State>, app: &Value) -> gtk::Widget {
    let more = c::icon_button("more", t("Meer opties", "More options"), 32);
    more.set_widget_name(&format!("installed-more-{}", common::value_key(&app["id"])));
    let card = SettingsCard::new("", common::s(&app["name"]), &describe(app));
    let icon = app["icon"].as_str().filter(|s| !s.is_empty()).map(str::to_owned).unwrap_or_else(|| common::value_key(&app["id"]));
    card.set_icon_widget(&kit::icons::app_icon(&icon, 32));
    card.set_control(&more);
    card.widget().add_css_class("st-app-row");
    let weak = Rc::downgrade(st);
    let a = app.clone();
    more.connect_clicked(move |b| {
        if let Some(st) = weak.upgrade() {
            menu(&st, &a, b);
        }
    });
    let weak = Rc::downgrade(st);
    let (a, mw) = (app.clone(), more.downgrade());
    c::bind_context_menu(card.widget(), move |_, _| {
        if let (Some(st), Some(more)) = (weak.upgrade(), mw.upgrade()) {
            menu(&st, &a, &more);
        }
        None
    });
    card.widget().clone()
}

/// The "…" menu (installed.js menu): Openen, the pins, then Verwijderen once software.uninstallInfo answered.
fn menu(st: &Rc<State>, app: &Value, anchor: &gtk::Button) {
    let id = common::value_key(&app["id"]);
    let name = common::s(&app["name"]).to_owned();
    let in_start = pins("start.pinned").contains(&id);
    let in_bar = pins("taskbar.pinned").contains(&id);
    let p = st.p.clone();
    let (pp, id2, name2) = (p.clone(), id.clone(), name.clone());
    let open = MenuItem::new(t("Openen", "Open"), move || {
        let fut = pp.run("apps.launch", json!({ "id": id2 }));
        let (p, name) = (pp.clone(), name2.clone());
        pp.spawn(async move {
            if fut.await.is_some() {
                p.toast(Toast::new(&common::fill(t("{name} wordt geopend", "Opening {name}"), &[("name", &name)])).timeout(1800));
            }
        });
    })
    .glyph("open")
    .name("installed-open");
    let (pp, id2) = (p.clone(), id.clone());
    let start = MenuItem::new(if in_start { t("Losmaken van Start", "Unpin from Start") } else { t("Vastmaken aan Start", "Pin to Start") }, move || {
        toggle_pin(&pp, "start.pinned", &id2, t("Vastgemaakt aan Start", "Pinned to Start"), t("Losgemaakt van Start", "Unpinned from Start"));
    })
    .glyph(if in_start { "unpin" } else { "pin" })
    .name("installed-pin-start");
    let (pp, id2) = (p.clone(), id.clone());
    let bar = MenuItem::new(if in_bar { t("Losmaken van taakbalk", "Unpin from taskbar") } else { t("Vastmaken aan taakbalk", "Pin to taskbar") }, move || {
        toggle_pin(
            &pp,
            "taskbar.pinned",
            &id2,
            t("Vastgemaakt aan de taakbalk", "Pinned to the taskbar"),
            t("Losgemaakt van de taakbalk", "Unpinned from the taskbar"),
        );
    })
    .glyph(if in_bar { "unpin" } else { "taskbar" })
    .name("installed-pin-taskbar");
    // Verwijderen: a placeholder until software.uninstallInfo answers, then filled in place (software.js
    // populateSoftwareRemoval: no reopening, no focus moved)
    let remove_action: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    let ra = remove_action.clone();
    let remove = MenuItem::new(&software::sw("uninstall"), move || {
        let f = ra.borrow().clone();
        if let Some(f) = f {
            f();
        }
    })
    .glyph("delete")
    .description(&software::sw("checking"))
    .enabled(false)
    .name("installed-uninstall");
    let items = vec![open, MenuItem::separator(), start, bar, MenuItem::separator(), remove];
    let popover = c::show_menu_for(anchor, items);
    popover.set_halign(gtk::Align::End);
    let pw = popover.downgrade();
    let anchor_w: gtk::Widget = p.anchor();
    kit::ctx().call("software.uninstallInfo", json!({ "id": id }), move |r| {
        let (label, enabled, desc) = match r {
            Ok(info) => {
                let supported = info["supported"] == true;
                let label = software::action_label(&json!(info["action"].as_str().filter(|s| !s.is_empty()).unwrap_or("uninstall")), None);
                let desc = if supported {
                    String::new()
                } else {
                    info["reason"].as_str().filter(|s| !s.is_empty()).map(str::to_owned).unwrap_or_else(|| software::sw("unavailable"))
                };
                if supported {
                    let anchor = anchor_w.clone();
                    let id = id.clone();
                    *remove_action.borrow_mut() = Some(Rc::new(move || {
                        software::review_and_track(&anchor, json!({ "action": "uninstall", "id": id }));
                    }));
                }
                (label, supported, desc)
            }
            Err(e) => (software::sw("uninstall"), false, if e.message.is_empty() { software::sw("unavailable") } else { e.message }),
        };
        let Some(popover) = pw.upgrade().filter(|p| p.is_visible()) else { return };
        let Some(item) = find_named(popover.upcast_ref(), "installed-uninstall") else { return };
        if let Some(l) = find_class(&item, "w11-menu-label").and_downcast::<gtk::Label>() {
            l.set_label(&label);
        }
        if let Some(d) = find_class(&item, "w11-menu-desc").and_downcast::<gtk::Label>() {
            d.set_label(&desc);
            d.set_visible(!desc.is_empty());
        }
        item.set_sensitive(enabled);
    });
}

fn find_named(w: &gtk::Widget, name: &str) -> Option<gtk::Widget> {
    if w.widget_name() == name {
        return Some(w.clone());
    }
    let mut c = w.first_child();
    while let Some(child) = c {
        if let Some(f) = find_named(&child, name) {
            return Some(f);
        }
        c = child.next_sibling();
    }
    None
}

fn find_class(w: &gtk::Widget, class: &str) -> Option<gtk::Widget> {
    if w.has_css_class(class) {
        return Some(w.clone());
    }
    let mut c = w.first_child();
    while let Some(child) = c {
        if let Some(f) = find_class(&child, class) {
            return Some(f);
        }
        c = child.next_sibling();
    }
    None
}

fn installed(p: Page) -> PageFuture {
    Box::pin(async move {
        let r = p.call("apps.list", json!({})).await;
        if !p.alive() {
            return Content::new();
        }
        let Some(apps) = r.ok().and_then(|v| v.as_array().cloned()).map(visible) else {
            return vec![common::notice(t("De lijst met apps is niet beschikbaar.", "The list of apps isn’t available.")).widget().clone().upcast()];
        };
        let list = gtk::Box::new(gtk::Orientation::Vertical, 4);
        list.add_css_class("st-app-list");
        list.set_widget_name("installed-list");
        let count = c::label("", "st-count", false);
        let st = Rc::new(State {
            p: p.clone(),
            apps: RefCell::new(apps),
            query: RefCell::new(String::new()),
            sort: RefCell::new("az".into()),
            filter: RefCell::new("all".into()),
            list,
            count,
            cards: RefCell::new(HashMap::new()),
        });

        let search = SearchBox::new(Some(t("Apps zoeken", "Search apps")));
        search.widget().set_widget_name("installed-search");
        search.widget().set_size_request(300, -1);
        for submit in [false, true] {
            let weak = Rc::downgrade(&st);
            let f = move |v: &str| {
                if let Some(st) = weak.upgrade() {
                    *st.query.borrow_mut() = v.to_owned();
                    paint(&st);
                }
            };
            if submit {
                search.connect_submit(f);
            } else {
                search.connect_changed(f);
            }
        }
        let sort = Dropdown::simple(&[("az", t("Naam (A-Z)", "Name (A-Z)")), ("za", t("Naam (Z-A)", "Name (Z-A)"))], Some("az"));
        sort.widget().set_widget_name("installed-sort");
        let weak = Rc::downgrade(&st);
        sort.connect_changed(move |v| {
            if let Some(st) = weak.upgrade() {
                *st.sort.borrow_mut() = v.to_owned();
                paint(&st);
            }
        });
        let filter = Dropdown::simple(
            &[
                ("all", t("Alle apps", "All apps")),
                ("start", t("Vastgemaakt aan Start", "Pinned to Start")),
                ("taskbar", t("Vastgemaakt aan taakbalk", "Pinned to taskbar")),
            ],
            Some("all"),
        );
        filter.widget().set_widget_name("installed-filter");
        let weak = Rc::downgrade(&st);
        filter.connect_changed(move |v| {
            if let Some(st) = weak.upgrade() {
                *st.filter.borrow_mut() = v.to_owned();
                paint(&st);
            }
        });
        paint(&st);

        {
            let weak = Rc::downgrade(&st);
            p.on("apps.changed", move |_| {
                let Some(st) = weak.upgrade() else { return };
                let fut = st.p.call("apps.list", json!({}));
                let w2 = Rc::downgrade(&st);
                st.p.spawn(async move {
                    let r = fut.await;
                    let Some(st) = w2.upgrade().filter(|s| s.p.alive()) else { return };
                    if let Some(list) = r.ok().and_then(|v| v.as_array().cloned()) {
                        *st.apps.borrow_mut() = visible(list);
                        // names and icons may have changed: the rows are built again
                        st.cards.borrow_mut().clear();
                        paint(&st);
                    }
                });
            });
            let weak = Rc::downgrade(&st);
            p.on_settings(move |_, path| {
                if let Some(st) = weak.upgrade()
                    && st.filter.borrow().as_str() != "all"
                    && path.is_none_or(|p| p.contains("pinned"))
                {
                    paint(&st);
                }
            });
        }

        let toolbar = crate::widgets::Wrap::new(20, 12);
        toolbar.add_css_class("st-toolbar");
        toolbar.append(search.widget());
        toolbar.append(&super::apps::toolbar_field(t("Sorteren op", "Sort by"), sort.widget()));
        toolbar.append(&super::apps::toolbar_field(t("Filteren op", "Filter by"), filter.widget()));
        let g = SettingsGroup::new("", "");
        g.add(&st.list);
        let count: gtk::Widget = st.count.clone().upcast();
        p.on_cleanup(move || drop(st));
        vec![toolbar.upcast(), count, g.widget().clone().upcast()]
    })
}

/// 6ebf169f:vendor/ui/js/core/software.js: the review of a plan (software.prepare; software.execute only from the
/// reviewed plan's own button) and the job view that follows software.status until the job is finished.
mod software {
    use kit::controls::{self as c, Dialog, DialogResult, InfoBar, ProgressState, Severity, Toast, Variant};
    use kit::prelude::*;
    use kit::t;
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use std::time::Duration;

    /// softwareT by key, in the UI language.
    fn text(key: &str) -> Option<&'static str> {
        Some(match key {
            "install" => t("Installeren", "Install"),
            "uninstall" => t("Verwijderen", "Uninstall"),
            "removeShortcut" => t("Snelkoppeling verwijderen", "Remove shortcut"),
            "installName" => t("‘{name}’ installeren", "Install “{name}”"),
            "uninstallName" => t("‘{name}’ verwijderen", "Uninstall “{name}”"),
            "removeShortcutName" => t("Snelkoppeling ‘{name}’ verwijderen", "Remove shortcut “{name}”"),
            "review" => t("Installatie controleren", "Review installation"),
            "reviewRemove" => t("Verwijderen controleren", "Review removal"),
            "provider" => t("Pakketbeheerder", "Package manager"),
            "scope" => t("Installatie voor", "Install for"),
            "user" => t("Alleen mijzelf", "Just me"),
            "system" => t("Alle gebruikers", "All users"),
            "version" => t("Versie", "Version"),
            "architecture" => t("Architectuur", "Architecture"),
            "size" => t("Grootte", "Size"),
            "source" => t("Bestand", "File"),
            "action" => t("Actie", "Action"),
            "auth" => t("Er kan om je wachtwoord worden gevraagd.", "You may be asked for your password."),
            "cancel" => t("Annuleren", "Cancel"),
            "close" => t("Sluiten", "Close"),
            "checking" => t("Beschikbaarheid controleren…", "Checking availability…"),
            "unavailable" => t("Deze actie is niet beschikbaar.", "This action isn’t available."),
            "failed" => t("De actie kon niet worden gestart", "The operation couldn’t be started"),
            "removeOnly" => t(
                "Alleen de snelkoppeling wordt verwijderd. Het programma en het oorspronkelijke bestand blijven behouden.",
                "Only the shortcut will be removed. The program and the original file will be kept.",
            ),
            "queued" => t("Wachten op starten…", "Waiting to start…"),
            "running" => t("Bezig…", "Working…"),
            "authenticating" => t("Wachten op toestemming…", "Waiting for authorisation…"),
            "completed" => t("Voltooid", "Completed"),
            "failedJob" => t("Mislukt", "Failed"),
            "cancelled" => t("Geannuleerd", "Cancelled"),
            "interrupted" => t("Uitkomst onbekend", "Outcome unknown"),
            "interruptedBody" => t(
                "Controleer eerst of de app is geïnstalleerd of verwijderd voordat je het opnieuw probeert.",
                "Check whether the app was installed or removed before trying again.",
            ),
            "preview" => t("Voorbeeld voltooid", "Preview completed"),
            "previewBody" => t("Er is niets geïnstalleerd of verwijderd.", "Nothing was installed or removed."),
            "cancelRequested" => t("Annulering aangevraagd. Wachten op de pakketbeheerder…", "Cancellation requested. Waiting for the package manager…"),
            "cancelFailed" => t("Annuleren is nu niet mogelijk", "Can’t cancel right now"),
            "statusFailed" => t("De voortgang is tijdelijk niet beschikbaar. Opnieuw verbinden…", "Progress is temporarily unavailable. Reconnecting…"),
            "shortcutProvider" => t("Snelkoppeling", "Shortcut"),
            "retryReview" => t(
                "Controleer Activiteit voordat je opnieuw probeert. Controleer de actie opnieuw voor een nieuwe poging.",
                "Check Activity and review the operation once more before you try again.",
            ),
            _ => return None,
        })
    }

    /// softwareT(key): the text, or the key itself when there is none.
    pub fn sw(key: &str) -> String {
        text(key).map(str::to_owned).unwrap_or_else(|| key.to_owned())
    }

    fn truthy(v: &Value) -> bool {
        match v {
            Value::Null => false,
            Value::Bool(b) => *b,
            Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
            Value::String(s) => !s.is_empty(),
            Value::Array(_) | Value::Object(_) => true,
        }
    }

    fn value_text(v: &Value) -> String {
        crate::common::value_key(v)
    }

    fn finished(job: &Value) -> bool {
        matches!(job["state"].as_str(), Some("completed" | "failed" | "cancelled" | "interrupted"))
    }

    fn state_key(job: &Value) -> String {
        let state = job["state"].as_str().unwrap_or("");
        if truthy(&job["dryRun"]) && state == "completed" {
            "preview".into()
        } else if state == "failed" {
            "failedJob".into()
        } else {
            value_text(&job["state"])
        }
    }

    /// softwareActionLabel(action, name): "Verwijderen" or "‘Naam’ verwijderen".
    pub fn action_label(action: &Value, name: Option<&Value>) -> String {
        let action = value_text(action);
        match name.map(value_text).filter(|n| !n.is_empty()) {
            Some(n) => crate::common::fill(&sw(&format!("{action}Name")), &[("name", &n)]),
            None => sw(&action),
        }
    }

    fn provider(p: &Value) -> String {
        match p.as_str() {
            Some("shortcut") => sw("shortcutProvider"),
            Some("appimage") => "AppImage".into(),
            Some("packagekit") => "PackageKit".into(),
            Some("flatpak") => "Flatpak".into(),
            Some("snapd") => "Snap".into(),
            _ if truthy(p) => value_text(p),
            _ => String::new(),
        }
    }

    fn info_bar(severity: Severity, title: &str, message: &str) -> gtk::Box {
        let bar = InfoBar::new(severity, title, message);
        bar.set_closable(false);
        bar.widget().clone()
    }

    fn note(text: &str) -> gtk::Label {
        c::label(text, "st-sw-note", true)
    }

    /// softwareDetails(plan, {plan: true}) as a two-column list (1fr : 2fr).
    fn details(info: &Value) -> gtk::Grid {
        let grid = gtk::Grid::new();
        grid.set_column_spacing(16);
        grid.set_row_spacing(8);
        grid.set_column_homogeneous(true);
        let mut row = 0;
        let mut add = |key: &str, value: String| {
            if value.is_empty() {
                return;
            }
            let dt = c::label(&sw(key), "st-sw-dt", true);
            dt.set_valign(gtk::Align::Start);
            let dd = c::label(&value, "st-sw-dd", true);
            dd.set_valign(gtk::Align::Start);
            dd.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            grid.attach(&dt, 0, row, 1, 1);
            grid.attach(&dd, 1, row, 2, 1);
            row += 1;
        };
        add("action", action_label(&info["action"], None));
        add("provider", provider(&info["provider"]));
        add("scope", sw(if info["scope"] == "system" { "system" } else { "user" }));
        let meta = if truthy(&info["info"]) { &info["info"] } else { info };
        add("version", value_text(&meta["version"]));
        add("architecture", value_text(&meta["architecture"]));
        if let Some(size) = meta["size"].as_f64().filter(|s| s.is_finite()) {
            add("size", format!("{} MB", kit::format::number(size / 1e6, 2)));
        }
        let source = if truthy(&info["sourcePath"]) { &info["sourcePath"] } else { &meta["sourcePath"] };
        add("source", value_text(source));
        grid
    }

    thread_local! {
        static REVIEWING: Cell<bool> = const { Cell::new(false) };
    }

    /// reviewAndTrackSoftware(args): the review, then the started job in a dialog of its own until it is closed.
    pub fn review_and_track(parent: &gtk::Widget, args: Value) {
        let p2 = parent.clone();
        review(parent, args, move |job| {
            let Some(job) = job else { return };
            let title = action_label(&job["action"], Some(&job["name"]));
            let view = Rc::new(JobView::new(job));
            let v2 = view.clone();
            Dialog::new(&title)
                .body(&crate::widgets::Clamp::new(view.widget(), 510))
                .close_label(&sw("close"))
                .width(560)
                .show(&p2, move |_| v2.dispose());
        });
    }

    /// reviewSoftware(args): software.prepare, then the review dialog; `done(job)` with the job software.execute
    /// started from the reviewed plan's button, None when nothing ran (or a review is already open).
    fn review(parent: &gtk::Widget, args: Value, done: impl FnOnce(Option<Value>) + 'static) {
        if REVIEWING.with(|r| r.replace(true)) {
            done(None);
            return;
        }
        let parent = parent.clone();
        kit::ctx().call("software.prepare", args, move |result| match result {
            Ok(plan) => show_review(&parent, plan, Box::new(done)),
            Err(e) => {
                REVIEWING.with(|r| r.set(false));
                let _ = Toast::new(&sw("failed")).message(&e.message).severity(Severity::Error).show(&parent);
                done(None);
            }
        });
    }

    fn show_review(parent: &gtk::Widget, plan: Value, done: Box<dyn FnOnce(Option<Value>)>) {
        let action = plan["action"].as_str().unwrap_or("").to_owned();
        let content = gtk::Box::new(gtk::Orientation::Vertical, 16);
        content.set_widget_name("installed-review-plan");
        content.append(&c::label(&value_text(&plan["name"]), "st-sw-name", true));
        if truthy(&plan["description"]) {
            content.append(&c::label(&value_text(&plan["description"]), "st-sw-description", true));
        }
        content.append(&details(&plan));
        if action == "removeShortcut" {
            content.append(&info_bar(Severity::Info, "", &sw("removeOnly")));
        }
        for n in plan["notes"].as_array().into_iter().flatten() {
            content.append(&note(&value_text(n)));
        }
        if truthy(&plan["requiresAuth"]) {
            content.append(&info_bar(Severity::Info, "", &sw("auth")));
        }
        if !truthy(&plan["canExecute"]) {
            let reason = if truthy(&plan["reason"]) { value_text(&plan["reason"]) } else { sw("unavailable") };
            content.append(&info_bar(Severity::Warning, "", &reason));
        }
        let status = gtk::Box::new(gtk::Orientation::Vertical, 16);
        status.set_visible(false);
        content.append(&status);

        let plan_id = plan["planId"].as_str().filter(|s| !s.is_empty()).map(str::to_owned);
        let executable = plan["canExecute"] == true && plan_id.is_some();
        let mut dialog = Dialog::new(&sw(if action == "install" { "review" } else { "reviewRemove" }))
            .body(&crate::widgets::Clamp::new(&content, 510))
            .width(560)
            .close_label(&sw("cancel"))
            .default_button(DialogResult::Close);
        if action != "install" {
            dialog = dialog.danger();
        }
        if executable {
            dialog = dialog.primary(&action_label(&plan["action"], Some(&plan["name"])));
        }
        let job: Rc<RefCell<Option<Value>>> = Rc::default();
        let attempted = Rc::new(Cell::new(false));
        let busy = Rc::new(Cell::new(false));
        {
            let (job, busy) = (job.clone(), busy.clone());
            dialog = dialog.on_primary(move |d| {
                let Some(plan_id) = plan_id.clone().filter(|_| executable && !attempted.get()) else { return false };
                attempted.set(true);
                c::clear(&status);
                status.set_visible(false);
                d.set_busy(true);
                busy.set(true);
                let (d, status, job, busy) = (d.clone(), status.clone(), job.clone(), busy.clone());
                kit::ctx().call("software.execute", json!({ "planId": plan_id }), move |result| {
                    busy.set(false);
                    match result {
                        Ok(started) => {
                            *job.borrow_mut() = Some(started);
                            d.close(DialogResult::Primary);
                        }
                        Err(e) => {
                            d.set_busy(false);
                            status.append(&info_bar(Severity::Error, &sw("failed"), &e.message));
                            status.append(&note(&sw("retryReview")));
                            status.set_visible(true);
                            // a failed request may have used up the single-use plan: another review is needed
                            d.set_primary_enabled(false);
                        }
                    }
                });
                false
            });
        }
        // a language switch closes the review unless it is busy (software.js onLangChange)
        let open: Rc<RefCell<Option<Dialog>>> = Rc::default();
        {
            let (open, busy, armed) = (open.clone(), busy.clone(), Cell::new(false));
            kit::i18n::bind(&content, move |_| {
                if !armed.replace(true) || busy.get() {
                    return;
                }
                let d = open.borrow().clone();
                if let Some(d) = d {
                    d.close(DialogResult::Close);
                }
            });
        }
        let closing = open.clone();
        dialog.show(parent, move |_| {
            closing.borrow_mut().take();
            REVIEWING.with(|r| r.set(false));
            let started = job.borrow_mut().take();
            done(started);
        });
        if dialog.is_open() {
            *open.borrow_mut() = Some(dialog);
        }
    }

    struct JobInner {
        el: gtk::Box,
        job: RefCell<Value>,
        disposed: Cell<bool>,
        timer: RefCell<Option<glib::SourceId>>,
        pending: Cell<bool>,
        failure: RefCell<String>,
        cancelling: Cell<bool>,
        revision: Cell<u64>,
        listener: Cell<Option<kit::ListenerId>>,
    }

    /// createSoftwareJobView: name, action, state, progress, messages and Annuleren; it polls software.status
    /// every 500 ms (and on software.changed) until the job is finished, since a job may run in another host.
    struct JobView {
        inner: Rc<JobInner>,
    }

    impl JobView {
        fn new(job: Value) -> JobView {
            let el = gtk::Box::new(gtk::Orientation::Vertical, 16);
            el.set_widget_name("installed-job-view");
            let inner = Rc::new(JobInner {
                el,
                job: RefCell::new(job),
                disposed: Cell::new(false),
                timer: RefCell::new(None),
                pending: Cell::new(false),
                failure: RefCell::new(String::new()),
                cancelling: Cell::new(false),
                revision: Cell::new(0),
                listener: Cell::new(None),
            });
            let weak = Rc::downgrade(&inner);
            let id = kit::ctx().on("software.changed", move |_, _| {
                if let Some(me) = weak.upgrade()
                    && !finished(&me.job.borrow())
                {
                    me.poll();
                }
            });
            inner.listener.set(Some(id));
            let weak = Rc::downgrade(&inner);
            // renders now and again after a language switch
            kit::i18n::bind(&inner.el, move |_| {
                if let Some(me) = weak.upgrade()
                    && !me.disposed.get()
                {
                    me.render();
                }
            });
            if !finished(&inner.job.borrow()) {
                inner.poll();
            }
            JobView { inner }
        }

        fn widget(&self) -> &gtk::Box {
            &self.inner.el
        }

        fn dispose(&self) {
            self.inner.disposed.set(true);
            self.inner.clear_timer();
            if let Some(id) = self.inner.listener.take() {
                kit::ctx().off(id);
            }
        }
    }

    impl JobInner {
        fn clear_timer(&self) {
            let timer = self.timer.borrow_mut().take();
            if let Some(id) = timer {
                id.remove();
            }
        }

        fn render(self: &Rc<Self>) {
            let el = &self.el;
            c::clear(el);
            let job = self.job.borrow().clone();
            let terminal = finished(&job);
            let name = value_text(&job["name"]);
            el.append(&c::label(&name, "st-sw-name", true));
            el.append(&c::label(&format!("{} · {}", action_label(&job["action"], None), provider(&job["provider"])), "st-sw-job-action", true));
            el.append(&c::label(&sw(&state_key(&job)), "st-sw-job-state", true));
            let bar = c::ProgressBar::new();
            let progress = job["progress"].as_f64().filter(|p| p.is_finite());
            bar.set_fraction(progress.unwrap_or(0.0) / 100.0);
            bar.set_indeterminate(!terminal && progress.is_none());
            if job["state"] == "failed" {
                bar.set_state(ProgressState::Error);
            }
            bar.widget().update_property(&[gtk::accessible::Property::Label(&name)]);
            el.append(bar.widget());
            if truthy(&job["message"]) {
                el.append(&c::label(&value_text(&job["message"]), "st-sw-job-message", true));
            }
            if truthy(&job["dryRun"]) && job["state"] == "completed" {
                el.append(&info_bar(Severity::Info, "", &sw("previewBody")));
            }
            if job["state"] == "interrupted" {
                el.append(&info_bar(Severity::Warning, "", &sw("interruptedBody")));
            }
            if truthy(&job["error"]["message"]) {
                el.append(&info_bar(Severity::Error, "", &value_text(&job["error"]["message"])));
            }
            if truthy(&job["cancelRequested"]) && !terminal {
                el.append(&note(&sw("cancelRequested")));
            }
            let failure = self.failure.borrow().clone();
            if !failure.is_empty() {
                el.append(&info_bar(Severity::Warning, "", &failure));
            }
            if !terminal {
                let cancel = c::button(&sw("cancel"), Variant::Standard);
                cancel.set_widget_name("installed-job-cancel");
                cancel.set_halign(gtk::Align::Start);
                cancel.set_sensitive(!self.cancelling.get() && !truthy(&job["cancelRequested"]) && truthy(&job["cancellable"]));
                let weak = Rc::downgrade(self);
                cancel.connect_clicked(move |_| {
                    if let Some(me) = weak.upgrade() {
                        me.cancel();
                    }
                });
                el.append(&cancel);
            }
        }

        fn cancel(self: &Rc<Self>) {
            let job = self.job.borrow().clone();
            if self.cancelling.get() || truthy(&job["cancelRequested"]) || !truthy(&job["cancellable"]) {
                return;
            }
            self.cancelling.set(true);
            self.revision.set(self.revision.get() + 1);
            self.render();
            let weak = Rc::downgrade(self);
            let job_id = job["jobId"].clone();
            kit::ctx().call("software.cancel", json!({ "jobId": job_id }), move |result| {
                let Some(me) = weak.upgrade() else { return };
                match result {
                    Ok(value) => {
                        if value["jobId"] == job_id {
                            *me.job.borrow_mut() = value;
                        }
                        me.failure.borrow_mut().clear();
                    }
                    Err(e) => *me.failure.borrow_mut() = if e.message.is_empty() { sw("cancelFailed") } else { e.message },
                }
                me.cancelling.set(false);
                me.revision.set(me.revision.get() + 1);
                if !me.disposed.get() {
                    me.render();
                    me.poll();
                }
            });
        }

        fn poll(self: &Rc<Self>) {
            if self.disposed.get() || self.pending.get() {
                return;
            }
            self.pending.set(true);
            self.clear_timer();
            let revision = self.revision.get();
            let job_id = self.job.borrow()["jobId"].clone();
            let weak = Rc::downgrade(self);
            kit::ctx().call("software.status", json!({ "jobId": job_id }), move |result| {
                let Some(me) = weak.upgrade() else { return };
                if !me.disposed.get() && revision == me.revision.get() {
                    match result {
                        Ok(value) => {
                            *me.job.borrow_mut() = value;
                            me.failure.borrow_mut().clear();
                            me.render();
                        }
                        Err(_) => {
                            *me.failure.borrow_mut() = sw("statusFailed");
                            me.render();
                        }
                    }
                }
                me.pending.set(false);
                me.schedule();
            });
        }

        fn schedule(self: &Rc<Self>) {
            self.clear_timer();
            if self.disposed.get() || finished(&self.job.borrow()) {
                return;
            }
            let weak = Rc::downgrade(self);
            let id = glib::timeout_add_local_once(Duration::from_millis(500), move || {
                if let Some(me) = weak.upgrade() {
                    // this source has run: forget it before polling (which would remove it otherwise)
                    me.timer.borrow_mut().take();
                    me.poll();
                }
            });
            *self.timer.borrow_mut() = Some(id);
        }
    }
}
