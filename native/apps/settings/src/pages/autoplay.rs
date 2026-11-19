//! Instellingen › Bluetooth en apparaten › Automatisch afspelen (batch B, notes/claude/settings-parity.md §2):
//! Windows 11's AutoPlay page holding GNOME's Removable Media (cc-removable-media-settings.c), whose keys are
//! org.gnome.desktop.media-handling (allow/devices.rs, mode Write):
//!
//! * "Automatisch afspelen gebruiken voor alle media en apparaten": autorun-never, inverted. While it is off, the
//!   media rows are insensitive, as GNOME greys them.
//! * "Standaardinstellingen voor Automatisch afspelen kiezen": Windows' "Verwisselbaar station", shown as what GNOME
//!   does with a drive whose content has no type ("Altijd vragen": gnome-shell's AutorunDispatcher asks; no key
//!   changes that, automount-open is read by nothing in GNOME 50) and greyed with the other rows while AutoPlay is
//!   off; "Geheugenkaart" (GNOME's Photos, x-content/image-dcf), then GNOME's other rows Audio-cd, Dvd-video,
//!   Muziekspeler and Software, and "Andere mediatypen" (GNOME's Other Media Types dialog, Type + Actie).
//! * Each media row is GNOME's choice: Altijd vragen (in no list), Geen actie ondernemen (autorun-x-content-ignore),
//!   Map openen om bestanden weer te geven (autorun-x-content-open-folder) or an app (autorun-x-content-start-app
//!   plus the app as the type's default, apps.setDefaultForType, as g_app_info_set_as_default_for_type). A choice
//!   takes the type out of all three lists and puts it in its own, in one gsettings.set (GNOME's
//!   autorun_set_preferences); the apps of a type come from apps.forType.
use super::{Builder, Page, PageFuture};
use crate::common;
use crate::gs::{self, GKey, Gs};
use kit::controls::{self as c, Dialog, Dropdown, DropdownOption, SettingsCard, SettingsGroup};
use kit::prelude::*;
use kit::t;
use std::cell::RefCell;
use std::rc::Rc;

pub const PAGES: &[(&str, Builder)] = &[("autoplay", page)];
pub const CSS: &str = r#"
.st-autoplay-other { padding: 4px 0; }
.st-autoplay-other-label { min-width: 80px; }
"#;

const SCHEMA: &str = "org.gnome.desktop.media-handling";
const NEVER: GKey = gs::key(SCHEMA, "autorun-never");
const START: GKey = gs::key(SCHEMA, "autorun-x-content-start-app");
const IGNORE: GKey = gs::key(SCHEMA, "autorun-x-content-ignore");
const OPEN: GKey = gs::key(SCHEMA, "autorun-x-content-open-folder");

/// GNOME's rows (cc-removable-media-settings.c), in this page's order: the memory card first (Windows' row).
const MAIN: &[&str] = &["x-content/image-dcf", "x-content/audio-cdda", "x-content/video-dvd", "x-content/audio-player", "x-content/unix-software"];
/// The types of GNOME's Other Media Types dialog.
const OTHER: &[&str] = &[
    "x-content/audio-dvd",
    "x-content/blank-bd",
    "x-content/blank-cd",
    "x-content/blank-dvd",
    "x-content/blank-hddvd",
    "x-content/video-bluray",
    "x-content/ebook-reader",
    "x-content/video-hddvd",
    "x-content/image-picturecd",
    "x-content/video-svcd",
    "x-content/video-vcd",
    "x-content/win32-software",
];

/// The three lists of GNOME's autorun preferences: (start-app, ignore, open-folder).
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Lists {
    pub start: Vec<String>,
    pub ignore: Vec<String>,
    pub open: Vec<String>,
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array().into_iter().flatten().filter_map(|x| x.as_str()).map(str::to_owned).collect()
}

impl Lists {
    fn of(gs: &Gs) -> Lists {
        Lists { start: strings(&gs.value(&START)), ignore: strings(&gs.value(&IGNORE)), open: strings(&gs.value(&OPEN)) }
    }

    /// GNOME's choice for a type (prepare_combo_box): "ask" when it is in no list, else "ignore", "folder", "app".
    pub(super) fn action(&self, mime: &str) -> &'static str {
        let has = |l: &[String]| l.iter().any(|m| m == mime);
        if has(&self.ignore) {
            "ignore"
        } else if has(&self.open) {
            "folder"
        } else if has(&self.start) {
            "app"
        } else {
            "ask"
        }
    }

    /// GNOME's autorun_set_preferences: the type out of every list, then into the one of `action` ("ask": none).
    pub(super) fn with(&self, mime: &str, action: &str) -> Lists {
        let mut out = self.clone();
        for l in [&mut out.start, &mut out.ignore, &mut out.open] {
            l.retain(|m| m != mime);
        }
        match action {
            "app" => out.start.push(mime.to_owned()),
            "ignore" => out.ignore.push(mime.to_owned()),
            "folder" => out.open.push(mime.to_owned()),
            _ => {}
        }
        out
    }
}

/// A row's choice as a drop-down value: "ask", "ignore", "folder" or "app:<desktop id>".
pub(super) fn choice_key(action: &str, default_app: Option<&str>) -> String {
    match (action, default_app) {
        ("app", Some(id)) => format!("app:{id}"),
        ("app", None) => "ask".to_owned(),
        (a, _) => a.to_owned(),
    }
}

/// The apps.forType answer of a type: its name and (default app id, apps).
#[derive(Clone, Default)]
struct TypeInfo {
    name: String,
    default: Option<String>,
    apps: Vec<(String, String)>,
}

impl TypeInfo {
    fn of(mime: &str, v: Option<&Value>) -> TypeInfo {
        let Some(v) = v else { return TypeInfo { name: mime.to_owned(), ..TypeInfo::default() } };
        let apps = v["apps"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|a| Some((a["id"].as_str()?.to_owned(), a["name"].as_str().unwrap_or("").to_owned())))
            .filter(|(id, _)| !id.is_empty())
            .collect();
        TypeInfo {
            name: v["name"].as_str().filter(|n| !n.is_empty()).unwrap_or(mime).to_owned(),
            default: v["default"]["id"].as_str().map(str::to_owned),
            apps,
        }
    }

    fn options(&self) -> Vec<DropdownOption> {
        let mut out = vec![
            DropdownOption::new("ask", t("Altijd vragen", "Ask me every time")),
            DropdownOption::new("ignore", t("Geen actie ondernemen", "Take no action")),
            DropdownOption::new("folder", t("Map openen om bestanden weer te geven", "Open folder to view files")),
        ];
        for (id, name) in &self.apps {
            out.push(DropdownOption::new(&format!("app:{id}"), if name.is_empty() { id } else { name }));
        }
        out
    }
}

/// A row of the page: a type and its drop-down.
struct Row {
    /// The type (the Other Media Types row changes it with its Type drop-down).
    mime: RefCell<String>,
    dd: Dropdown,
    info: RefCell<TypeInfo>,
}

struct AutoPlay {
    p: Page,
    gs: Gs,
    lists: RefCell<Lists>,
    rows: RefCell<Vec<Rc<Row>>>,
    cards: RefCell<Vec<gtk::Widget>>,
}

impl AutoPlay {
    fn writable(&self) -> bool {
        [&START, &IGNORE, &OPEN].iter().all(|k| self.gs.writable(k))
    }

    /// The rows' selection and sensitivity after a change of the lists or of autorun-never.
    fn refresh(&self) {
        let lists = self.lists.borrow().clone();
        let on = self.gs.value(&NEVER) != true;
        let writable = self.writable();
        for row in self.rows.borrow().iter() {
            let info = row.info.borrow();
            row.dd.set_selected(Some(&choice_key(lists.action(&row.mime.borrow()), info.default.as_deref())));
            row.dd.widget().set_sensitive(on && writable);
            row.dd.widget().set_tooltip_text(if writable { None } else { Some(gs::locked_text()) });
        }
        for card in self.cards.borrow().iter() {
            card.set_sensitive(on);
        }
    }

    /// The user picked `choice` for `row`: the three lists in one write, then the app as the type's default.
    fn choose(self: &Rc<Self>, row: &Rc<Row>, choice: &str) {
        let (action, app) = match choice.strip_prefix("app:") {
            Some(id) => ("app", Some(id.to_owned())),
            None => (choice, None),
        };
        let mime = row.mime.borrow().clone();
        let before = self.lists.borrow().clone();
        let lists = before.with(&mime, action);
        *self.lists.borrow_mut() = lists.clone();
        let mut values = serde_json::Map::new();
        values.insert(START.key.to_owned(), json!(lists.start));
        values.insert(IGNORE.key.to_owned(), json!(lists.ignore));
        values.insert(OPEN.key.to_owned(), json!(lists.open));
        let write = self.p.run_with("gsettings.set", json!({ "schema": SCHEMA, "values": values }), None, false);
        let set_app = app.clone().map(|id| self.p.run_with("apps.setDefaultForType", json!({ "mime": mime, "id": id }), None, true));
        let (me, row) = (self.clone(), row.clone());
        self.p.spawn(async move {
            let ok = write.await.is_some();
            if !ok {
                *me.lists.borrow_mut() = before;
            }
            if let (true, Some(wait), Some(id)) = (ok, set_app, app)
                && wait.await.is_some()
            {
                row.info.borrow_mut().default = Some(id);
            }
            if me.p.alive() {
                me.refresh();
            }
        });
    }

    fn row(self: &Rc<Self>, mime: &str, info: TypeInfo) -> Rc<Row> {
        let dd = Dropdown::new(info.options(), None);
        dd.widget().set_widget_name(&format!("st-autoplay-{}", mime.trim_start_matches("x-content/")));
        let row = Rc::new(Row { mime: RefCell::new(mime.to_owned()), dd: dd.clone(), info: RefCell::new(info) });
        let (me, r) = (Rc::downgrade(self), Rc::downgrade(&row));
        dd.connect_changed(move |choice| {
            if let (Some(me), Some(r)) = (me.upgrade(), r.upgrade()) {
                me.choose(&r, choice);
            }
        });
        self.rows.borrow_mut().push(row.clone());
        row
    }

    /// GNOME's Other Media Types dialog: a type and its action.
    fn other_types(self: &Rc<Self>, infos: Vec<TypeInfo>) {
        let types = Dropdown::new(OTHER.iter().zip(&infos).map(|(m, i)| DropdownOption::new(m, &i.name)).collect(), Some(OTHER[0]));
        types.widget().set_widget_name("st-autoplay-other-type");
        let first = infos.first().cloned().unwrap_or_default();
        let row = self.row(OTHER[0], first);
        row.dd.widget().set_widget_name("st-autoplay-other-action");
        self.refresh();
        {
            let (r, infos) = (Rc::downgrade(&row), infos.clone());
            let me = Rc::downgrade(self);
            types.connect_changed(move |mime| {
                let (Some(r), Some(me)) = (r.upgrade(), me.upgrade()) else { return };
                let Some(i) = OTHER.iter().position(|m| *m == mime) else { return };
                // the row now stands for another type: its mime, its apps and its choice
                *r.mime.borrow_mut() = mime.to_owned();
                *r.info.borrow_mut() = infos[i].clone();
                r.dd.set_options(infos[i].options());
                me.refresh();
            });
        }
        let line = |label: &str, w: &gtk::Widget| {
            let b = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            b.add_css_class("st-autoplay-other");
            let l = c::label(label, "st-autoplay-other-label", false);
            l.set_valign(gtk::Align::Center);
            b.append(&l);
            w.set_hexpand(true);
            b.append(w);
            b
        };
        let body = gtk::Box::new(gtk::Orientation::Vertical, 8);
        body.append(&line(t("Type", "Type"), &types.widget().clone().upcast()));
        body.append(&line(t("Actie", "Action"), &row.dd.widget().clone().upcast()));
        let me = self.clone();
        let keep = (types, row.clone());
        Dialog::new(t("Andere mediatypen", "Other media types")).body(&body).close_label(t("Sluiten", "Close")).show(&self.p.anchor(), move |_| {
            me.rows.borrow_mut().retain(|x| !OTHER.contains(&x.mime.borrow().as_str()));
            drop(keep);
        });
    }
}

/// What GNOME Shell does with a drive whose content has no x-content type (AutorunDispatcher.addMount: ASK): the
/// "Verwisselbaar station" row's fixed value, the same text as the media rows' choice.
pub(super) fn removable_choice() -> &'static str {
    t("Altijd vragen", "Ask me every time")
}

fn type_title(mime: &str) -> (&'static str, &'static str, &'static str) {
    match mime {
        "x-content/image-dcf" => ("image", t("Geheugenkaart", "Memory card"), t("Foto’s", "Photos")),
        "x-content/audio-cdda" => ("music", t("Audio-cd", "Audio CD"), ""),
        "x-content/video-dvd" => ("video", t("Dvd-video", "DVD video"), ""),
        "x-content/audio-player" => ("headphones", t("Muziekspeler", "Music player"), ""),
        _ => ("app", t("Software", "Software"), ""),
    }
}

fn page(p: Page) -> PageFuture {
    Box::pin(async move {
        let gs = Gs::load(&p, &[NEVER, START, IGNORE, OPEN]).await;
        let infos = p.all(MAIN.iter().map(|m| ("apps.forType", json!({ "mime": m }))).collect()).await;
        if !p.alive() {
            return Vec::new();
        }
        if !gs.available(&NEVER) {
            let card = common::unavailable_card("play", t("Automatisch afspelen", "AutoPlay"), None);
            return vec![card.widget().clone()];
        }
        let page = Rc::new(AutoPlay {
            p: p.clone(),
            gs: gs.clone(),
            lists: RefCell::new(Lists::of(&gs)),
            rows: RefCell::new(Vec::new()),
            cards: RefCell::new(Vec::new()),
        });
        let mut out: Vec<gtk::Widget> = Vec::new();
        let main = SettingsGroup::new("", "");
        if let Some(card) = gs.toggle_card(&NEVER, "play", t("Automatisch afspelen gebruiken voor alle media en apparaten", "Use AutoPlay for all media and devices"), "", true) {
            main.add(card.widget());
        }
        out.push(main.widget().clone().upcast());

        let group = SettingsGroup::new(t("Standaardinstellingen voor Automatisch afspelen kiezen", "Choose AutoPlay defaults"), "");
        // Windows' first row: GNOME always asks for a drive without a content type (no key decides it)
        let removable = SettingsCard::new("drive", t("Verwisselbaar station", "Removable drive"), "");
        let fixed = common::value_label(removable_choice());
        fixed.set_widget_name("st-autoplay-removable");
        removable.set_control(&fixed);
        page.cards.borrow_mut().push(removable.widget().clone());
        group.add(removable.widget());
        for (i, mime) in MAIN.iter().enumerate() {
            let info = TypeInfo::of(mime, infos.get(i).and_then(|r| r.as_ref().ok()));
            let row = page.row(mime, info);
            let (glyph, title, desc) = type_title(mime);
            let card = SettingsCard::new(glyph, title, desc);
            card.set_control(row.dd.widget());
            page.cards.borrow_mut().push(card.widget().clone());
            group.add(card.widget());
        }
        let other = SettingsCard::clickable("apps", t("Andere mediatypen", "Other media types"), t("Lege schijven, Blu-ray, e-boeklezers en meer", "Blank discs, Blu-ray, e-book readers and more"), c::Chevron::Right);
        if let Some(b) = other.button() {
            b.set_widget_name("st-autoplay-other");
        }
        {
            let pg = page.clone();
            other.connect_clicked(move || {
                let pg = pg.clone();
                let wait = pg.p.all(OTHER.iter().map(|m| ("apps.forType", json!({ "mime": m }))).collect());
                pg.p.clone().spawn(async move {
                    let got = wait.await;
                    if !pg.p.alive() {
                        return;
                    }
                    let infos = OTHER.iter().enumerate().map(|(i, m)| TypeInfo::of(m, got.get(i).and_then(|r| r.as_ref().ok()))).collect();
                    pg.other_types(infos);
                });
            });
        }
        page.cards.borrow_mut().push(other.widget().clone());
        group.add(other.widget());
        out.push(group.widget().clone().upcast());

        for k in [&START, &IGNORE, &OPEN, &NEVER] {
            let pg = Rc::downgrade(&page);
            gs.follow(k, move |_| {
                if let Some(pg) = pg.upgrade() {
                    *pg.lists.borrow_mut() = Lists::of(&pg.gs);
                    pg.refresh();
                }
            });
        }
        page.refresh();

        let related = SettingsGroup::new(common::ct::related(), "");
        related.add(common::link_card(&p, "defaultapps").widget());
        out.push(related.widget().clone().upcast());
        out
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lists(start: &[&str], ignore: &[&str], open: &[&str]) -> Lists {
        let v = |l: &[&str]| l.iter().map(|s| s.to_string()).collect();
        Lists { start: v(start), ignore: v(ignore), open: v(open) }
    }

    #[test]
    fn a_type_reads_as_gnomes_choice() {
        // Ubuntu's defaults: software and ostree repositories start their app
        let l = lists(&["x-content/unix-software", "x-content/ostree-repository"], &[], &[]);
        assert_eq!(l.action("x-content/unix-software"), "app");
        assert_eq!(l.action("x-content/image-dcf"), "ask");
        assert_eq!(lists(&[], &["x-content/audio-cdda"], &[]).action("x-content/audio-cdda"), "ignore");
        assert_eq!(lists(&[], &[], &["x-content/video-dvd"]).action("x-content/video-dvd"), "folder");
        assert_eq!(choice_key("app", Some("org.gnome.Rhythmbox3.desktop")), "app:org.gnome.Rhythmbox3.desktop");
        assert_eq!(choice_key("app", None), "ask", "no default app: GNOME asks");
    }

    #[test]
    fn a_removable_drive_is_always_asked_about() {
        // review B finding 7: no control for automount-open (nothing reads it); the row shows GNOME's behaviour in
        // the words of the media rows' "ask" choice
        let ask = TypeInfo::default().options().into_iter().find(|o| o.value == "ask").map(|o| o.label);
        assert_eq!(ask.as_deref(), Some(removable_choice()));
    }

    #[test]
    fn a_choice_moves_the_type_into_one_list() {
        let l = lists(&["x-content/unix-software", "x-content/image-dcf"], &["x-content/image-dcf"], &[]);
        let folder = l.with("x-content/image-dcf", "folder");
        assert_eq!(folder, lists(&["x-content/unix-software"], &[], &["x-content/image-dcf"]));
        let ask = folder.with("x-content/image-dcf", "ask");
        assert_eq!(ask, lists(&["x-content/unix-software"], &[], &[]));
        let app = ask.with("x-content/audio-cdda", "app");
        assert_eq!(app.start, ["x-content/unix-software", "x-content/audio-cdda"]);
        assert_eq!(app.with("x-content/audio-cdda", "app"), app, "the same choice again changes nothing");
    }
}
