//! Opslag > Opruimen (js/apps/taskmgr/storage/cleanup.js): a Storage Sense-like checklist (storage.cleanup.list)
//! with sizes and plain explanations; items that need admin rights carry a shield and the explanation first. Old
//! downloads are reviewed file by file and go to the Prullenbak. Opruimen asks first, then storage.cleanup.run
//! and the result as an info bar. The footer with the total and the button stays at the bottom of the page while
//! the list scrolls (the page places it, `footer()` / `placeholder()`).
//!
//! Item shape: {id, name, description, size|null, needsAdmin, safe, defaultSelected, available, details}; details
//! may be {count} | {items:[...]} | {total, keepDays} | an array of strings | a string.
use super::layout::{self, Layout};
use crate::app::App;
use crate::hostcall;
use crate::util::{self, fill};
use kit::controls::{Dialog, DialogResult, InfoBar, Severity, Toast, Variant};
use kit::format::{bytes, number};
use kit::prelude::*;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};

fn tr(key: &str) -> &'static str {
    let (nl, en) = match key {
        "intro" => (
            "Maak ruimte vrij door bestanden op te ruimen die je pc niet meer nodig heeft. Je ziet precies wat er weggaat.",
            "Free up space by cleaning up files your PC no longer needs. You\u{2019}ll see exactly what will be removed.",
        ),
        "adminTitle" => ("Hiervoor vraagt Ubuntu je wachtwoord", "Ubuntu will ask for your password"),
        "adminBody" => (
            "Een paar onderdelen staan in systeemmappen. Ubuntu vraagt dan eerst om je wachtwoord \u{2013} dat is normaal en veilig.",
            "A few items are in system folders, so Ubuntu asks for your password first \u{2013} that\u{2019}s normal and safe.",
        ),
        "adminTag" => ("Wachtwoord nodig", "Password needed"),
        "reviewTag" => ("Eerst bekijken", "Review first"),
        "trashTag" => ("Gaat naar de Prullenbak", "Goes to the Recycle Bin"),
        "recommended" => ("Aanbevolen", "Recommended"),
        "other" => ("Ook mogelijk", "Also possible"),
        "selected" => ("Geselecteerd: {size}", "Selected: {size}"),
        "selectedUnknown" => ("Geselecteerd: {size} + onbekend", "Selected: {size} + unknown"),
        "nothing" => ("Niets geselecteerd", "Nothing selected"),
        "clean" => ("Opruimen", "Clean up"),
        "cleaning" => ("Bezig met opruimen\u{2026}", "Cleaning up\u{2026}"),
        "confirmTitle" => ("{size} opruimen?", "Clean up {size}?"),
        "confirmTitleUnknown" => ("Opruimen?", "Clean up?"),
        "confirmBody" => ("Dit wordt opgeruimd:", "This will be cleaned up:"),
        "confirmAdmin" => (
            "Ubuntu vraagt daarna \u{e9}\u{e9}n keer om je wachtwoord voor de onderdelen met een schildje.",
            "Ubuntu will then ask once for your password for the items with a shield.",
        ),
        "confirmPermanent" => (
            "Let op: wat je uit de Prullenbak opruimt, kun je niet meer terugzetten.",
            "Note: what you clean up from the Recycle Bin can\u{2019}t be restored.",
        ),
        "confirmDownloads" => (
            "De gekozen downloads gaan naar de Prullenbak; je kunt ze daar nog terugzetten.",
            "The chosen downloads will be moved to the Recycle Bin; you can still restore them from there.",
        ),
        "cancel" => ("Annuleren", "Cancel"),
        "freed" => ("{size} vrijgemaakt", "{size} freed up"),
        "freedBody" => ("Je pc heeft weer wat meer ademruimte.", "Your PC has a bit more room to breathe."),
        "partly" => ("Niet alles is gelukt", "Not everything worked"),
        "freedPartly" => ("{size} vrijgemaakt \u{2013} niet alles is gelukt", "{size} freed up \u{2013} not everything worked"),
        "movedDownloadsOne" => ("1 download staat nu in de Prullenbak ({size})", "1 download is now in the Recycle Bin ({size})"),
        "movedDownloadsOther" => ("{n} downloads staan nu in de Prullenbak ({size})", "{n} downloads are now in the Recycle Bin ({size})"),
        "skipped" => (
            "Overgeslagen omdat ze nog open zijn: {names}. Sluit ze en probeer het opnieuw.",
            "Skipped because they\u{2019}re still open: {names}. Close them and try again.",
        ),
        "failedItem" => ("{name}: {error}", "{name}: {error}"),
        "nothingToDo" => ("Er valt niets op te ruimen", "Nothing to clean up"),
        "nothingBody" => ("Je pc is al netjes opgeruimd.", "Your PC is already tidy."),
        "loading" => ("Berekenen hoeveel ruimte je kunt vrijmaken\u{2026}", "Working out how much space you can free\u{2026}"),
        "refresh" => ("Opnieuw berekenen", "Calculate again"),
        "more" => ("Wat zit hierin?", "What\u{2019}s in here?"),
        "review" => ("Bestanden bekijken", "View files"),
        "failedLoad" => ("Kon de lijst niet ophalen", "Couldn\u{2019}t load the list"),
        "dryRun" => ("Testmodus: er is niets echt verwijderd", "Test mode: nothing was actually removed"),
        "unknownSize" => ("Onbekend", "Unknown"),
        "itemsOne" => ("1 item", "1 item"),
        "itemsOther" => ("{n} items", "{n} items"),
        "filesOne" => ("1 bestand", "1 file"),
        "filesOther" => ("{n} bestanden", "{n} files"),
        "journalInfo" => ("Logboeken samen {total}; alles ouder dan {days} dagen gaat weg.", "Logs take up {total}; everything older than {days} days will be removed."),
        "running" => ("nog geopend", "still open"),
        "pickAll" => ("Alles kiezen", "Choose all"),
        "pickNone" => ("Niets kiezen", "Choose none"),
        "picked" => ("{n} van {m} gekozen", "{n} of {m} chosen"),
        "notOpened" => ("niet geopend sinds {when}", "not opened since {when}"),
        "andMore" => ("en nog {n}", "and {n} more"),
        "revision" => ("versie {r}", "revision {r}"),
        _ => ("", ""),
    };
    kit::t(nl, en)
}

fn icon_of(id: &str) -> &'static str {
    match id {
        "trash" => "trash",
        "thumbnails" => "image",
        "user-cache-safe" => "app",
        "browser-cache" => "globe",
        "old-downloads" => "download",
        "flatpak-unused" => "apps",
        "crash-reports" => "warning",
        "journal" => "document",
        "apt-cache" => "update",
        "old-snaps" => "history",
        _ => "clean",
    }
}

fn known(it: &Value) -> bool {
    it["size"].is_number()
}

fn selectable(it: &Value) -> bool {
    it["available"] != false && (it["size"].is_null() || util::num(&it["size"]) > 0.0)
}

/// "{n} items" with the number formatted.
fn count_text(n: f64, one: &str, other: &str) -> String {
    fill(if n.round() as i64 == 1 { one } else { other }, &[("n", &number(n, 0))])
}

struct Picker {
    head: gtk::Label,
    boxes: Vec<(gtk::CheckButton, String)>,
}

struct Details {
    toggle: gtk::Button,
    content: gtk::Widget,
    chev: gtk::Image,
    picker: Option<Picker>,
}

impl Details {
    fn set_open(&self, open: bool) {
        self.content.set_visible(open);
        kit::icons::set_glyph(&self.chev, if open { "chevron-up" } else { "chevron-down" });
        util::set_class(&self.toggle, "taskmgr-is-open", open);
    }
}

struct CleanRow {
    id: String,
    item: Value,
    row: gtk::Box,
    check: gtk::CheckButton,
    size: gtk::Label,
    details: Option<Rc<Details>>,
}

pub struct Cleanup {
    me: Weak<Cleanup>,
    app: Rc<App>,
    on_changed: Box<dyn Fn()>,
    items: RefCell<Vec<Value>>,
    selected: RefCell<HashSet<String>>,
    download_paths: RefCell<HashSet<String>>,
    loaded: Cell<bool>,
    busy: Cell<bool>,
    syncing: Cell<bool>,
    clamp: Layout,
    result_bar: gtk::Box,
    admin_bar: InfoBar,
    list: gtk::Box,
    total_label: gtk::Label,
    clean_btn: gtk::Button,
    footer: gtk::Box,
    placeholder: gtk::Box,
    rows: RefCell<Vec<CleanRow>>,
}

impl Cleanup {
    pub fn new(app: &Rc<App>, on_changed: impl Fn() + 'static) -> Rc<Cleanup> {
        let intro = layout::wrap(tr("intro"), "taskmgr-clean-intro taskmgr-text-2");
        intro.set_max_width_chars(96);
        intro.set_halign(gtk::Align::Start);
        let result_bar = layout::vbox(8, "taskmgr-clean-result");
        result_bar.set_visible(false);
        let admin_bar = InfoBar::new(Severity::Info, tr("adminTitle"), tr("adminBody"));
        admin_bar.set_closable(false);
        admin_bar.set_visible(false);
        let list = layout::vbox(0, "taskmgr-clean-list");
        list.set_widget_name("taskmgr-cleanup-list");
        // where the footer sits when the list is short (the page floats the real one above it)
        let placeholder = layout::vbox(0, "taskmgr-clean-placeholder");
        placeholder.set_size_request(-1, 64);
        let root = layout::vbox(12, "taskmgr-clean");
        root.append(&intro);
        root.append(&result_bar);
        root.append(admin_bar.widget());
        root.append(&list);
        root.append(&placeholder);
        let clamp = Layout::clamp(980);
        clamp.append(&root);

        let total_label = layout::label(tr("nothing"), "taskmgr-clean-total");
        let totalwrap = layout::hbox(10, "");
        totalwrap.set_hexpand(true);
        totalwrap.set_valign(gtk::Align::Center);
        totalwrap.append(&kit::icons::glyph("storage", 20));
        totalwrap.append(&total_label);
        let clean_btn = kit::controls::button_with_glyph("broom", tr("clean"), Variant::Accent);
        clean_btn.add_css_class("w11-large");
        clean_btn.set_widget_name("taskmgr-cleanup-run");
        clean_btn.set_sensitive(false);
        clean_btn.set_valign(gtk::Align::Center);
        let footer = layout::hbox(16, "taskmgr-clean-footer");
        footer.append(&totalwrap);
        footer.append(&clean_btn);

        let c = Rc::new_cyclic(|me: &Weak<Cleanup>| Cleanup {
            me: me.clone(),
            app: app.clone(),
            on_changed: Box::new(on_changed),
            items: RefCell::new(Vec::new()),
            selected: RefCell::new(HashSet::new()),
            download_paths: RefCell::new(HashSet::new()),
            loaded: Cell::new(false),
            busy: Cell::new(false),
            syncing: Cell::new(false),
            clamp,
            result_bar,
            admin_bar,
            list,
            total_label,
            clean_btn,
            footer,
            placeholder,
            rows: RefCell::new(Vec::new()),
        });
        let w = Rc::downgrade(&c);
        c.clean_btn.connect_clicked(move |_| {
            if let Some(c) = w.upgrade() {
                c.run();
            }
        });
        c
    }

    pub fn widget(&self) -> gtk::Widget {
        self.clamp.clone().upcast()
    }

    /// The footer (total and Opruimen); the page shows it above the list, at the bottom of the view.
    pub fn footer(&self) -> &gtk::Box {
        &self.footer
    }

    /// The footer's place at the end of the list.
    pub fn placeholder(&self) -> &gtk::Box {
        &self.placeholder
    }

    /// Shown: load once, else bring the totals up to date.
    pub fn show(&self) {
        if self.loaded.get() {
            self.update();
        } else {
            self.load(None);
        }
    }

    pub fn refresh(&self) {
        self.load(None);
    }

    fn dl_items(&self) -> Vec<Value> {
        self.items
            .borrow()
            .iter()
            .find(|i| i["id"] == "old-downloads")
            .and_then(|i| i["details"]["items"].as_array().cloned())
            .unwrap_or_default()
    }

    /// The size of an item for the current selection (old downloads: the ticked files).
    fn size_of(&self, it: &Value) -> f64 {
        if it["id"] == "old-downloads" {
            let dl = self.dl_items();
            if !dl.is_empty() {
                let paths = self.download_paths.borrow();
                return dl.iter().filter(|f| paths.contains(&util::text(&f["path"]))).map(|f| util::num(&f["size"])).sum();
            }
        }
        util::num(&it["size"])
    }

    fn load(&self, then: Option<Box<dyn FnOnce()>>) {
        kit::controls::clear(&self.list);
        self.rows.borrow_mut().clear();
        self.list.append(&layout::ring_note(28, tr("loading")));
        let w = self.me.clone();
        hostcall::call("storage.cleanup.list", json!({}), move |r| {
            let Some(c) = w.upgrade() else { return };
            match r {
                Err(e) => {
                    kit::controls::clear(&c.list);
                    let bar = InfoBar::new(Severity::Error, tr("failedLoad"), &e.message);
                    bar.set_closable(false);
                    let w = c.me.clone();
                    bar.set_action(Some((
                        tr("refresh"),
                        Rc::new(move || {
                            if let Some(c) = w.upgrade() {
                                c.load(None);
                            }
                        }),
                    )));
                    c.list.append(bar.widget());
                }
                Ok(items) => {
                    let items: Vec<Value> = items.as_array().into_iter().flatten().filter(|i| i["available"] != false).cloned().collect();
                    *c.items.borrow_mut() = items.clone();
                    if !c.loaded.get() {
                        let defaults: HashSet<String> =
                            items.iter().filter(|it| it["defaultSelected"] == true && selectable(it)).map(|it| util::text(&it["id"])).collect();
                        *c.selected.borrow_mut() = defaults;
                        c.download_paths.borrow_mut().clear();
                    } else {
                        c.selected.borrow_mut().retain(|id| items.iter().any(|i| util::text(&i["id"]) == *id && selectable(i)));
                        let valid: HashSet<String> = c.dl_items().iter().map(|f| util::text(&f["path"])).collect();
                        c.download_paths.borrow_mut().retain(|p| valid.contains(p));
                    }
                    c.loaded.set(true);
                    c.render();
                }
            }
            if let Some(f) = then {
                f();
            }
        });
    }

    // ------------------------------------------------------------------------------------------- details

    fn details_node(&self, it: &Value) -> Option<Rc<Details>> {
        let d = &it["details"];
        let id = util::text(&it["id"]);
        let (content, picker): (gtk::Widget, Option<Picker>) = match d {
            Value::Null => return None,
            Value::String(s) => {
                if s.trim().is_empty() {
                    return None;
                }
                let l = layout::wrap(s, "taskmgr-clean-details");
                l.set_selectable(true);
                (l.upcast(), None)
            }
            Value::Array(a) => {
                if a.is_empty() {
                    return None;
                }
                let b = layout::vbox(0, "taskmgr-clean-details");
                for x in a {
                    let t = if x.is_string() {
                        util::text(x)
                    } else {
                        [&x["name"], &x["path"]].into_iter().map(util::text).find(|s| !s.is_empty()).unwrap_or_default()
                    };
                    b.append(&layout::wrap(&t, ""));
                }
                (b.upcast(), None)
            }
            _ if id == "old-downloads" && d["items"].is_array() => {
                let (w, p) = self.downloads_picker();
                (w, Some(p))
            }
            _ => {
                let b = layout::vbox(0, "taskmgr-clean-details");
                let mut any = false;
                if let Some(items) = d["items"].as_array() {
                    let shown: Vec<&Value> = items.iter().filter(|x| x["size"].as_f64().is_none_or(|s| s > 0.0)).take(8).collect();
                    for x in &shown {
                        let name = [&x["name"], &x["browser"]].into_iter().map(util::text).find(|s| !s.is_empty()).unwrap_or_else(|| util::pretty_path(&util::text(&x["path"])));
                        let rev = util::text(&x["revision"]);
                        let parts = [
                            name,
                            if rev.is_empty() { String::new() } else { fill(tr("revision"), &[("r", &rev)]) },
                            if x["running"] == true { format!("({})", tr("running")) } else { String::new() },
                        ];
                        let label = parts.into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ");
                        let li = layout::hbox(12, "taskmgr-clean-li");
                        let l = layout::label(&label, "");
                        l.set_hexpand(true);
                        li.append(&l);
                        if let Some(s) = x["size"].as_f64() {
                            li.append(&layout::label(&bytes(s), "taskmgr-tabular"));
                        }
                        b.append(&li);
                        any = true;
                    }
                    if items.len() > shown.len() {
                        b.append(&layout::label(&fill(tr("andMore"), &[("n", &(items.len() - shown.len()).to_string())]), "taskmgr-text-3"));
                    }
                }
                if let Some(n) = d["count"].as_f64()
                    && !d["items"].is_array()
                {
                    b.append(&layout::label(&count_text(n, tr("itemsOne"), tr("itemsOther")), ""));
                    any = true;
                }
                if let (Some(total), Some(days)) = (d["total"].as_f64(), d.get("keepDays").filter(|v| !v.is_null())) {
                    b.append(&layout::wrap(&fill(tr("journalInfo"), &[("total", &bytes(total)), ("days", &util::text(days))]), ""));
                    any = true;
                }
                if !any {
                    return None;
                }
                (b.upcast(), None)
            }
        };
        let is_picker = picker.is_some();
        let chev = kit::icons::glyph("chevron-down", 12);
        let inner = layout::hbox(6, "");
        inner.append(&chev);
        inner.append(&gtk::Label::new(Some(tr(if is_picker { "review" } else { "more" }))));
        let toggle = gtk::Button::new();
        toggle.add_css_class("w11-link");
        toggle.add_css_class("taskmgr-clean-more");
        toggle.set_child(Some(&inner));
        toggle.set_halign(gtk::Align::Start);
        toggle.set_widget_name(&format!("taskmgr-cleanup-more-{id}"));
        content.set_visible(false);
        let det = Rc::new(Details { toggle: toggle.clone(), content, chev, picker });
        let dw = Rc::downgrade(&det);
        toggle.connect_clicked(move |_| {
            if let Some(d) = dw.upgrade() {
                d.set_open(!d.content.is_visible());
            }
        });
        Some(det)
    }

    fn downloads_picker(&self) -> (gtk::Widget, Picker) {
        let files = self.dl_items();
        let head = layout::label("", "taskmgr-clean-pickhead");
        head.set_hexpand(true);
        let ul = layout::vbox(0, "taskmgr-clean-picks");
        let mut boxes = Vec::new();
        for f in files.iter().take(300) {
            let path = util::text(&f["path"]);
            let name = util::text(&f["name"]);
            let when = f["accessed"].as_f64().or(f["modified"].as_f64()).filter(|t| *t > 0.0);
            let desc = [
                util::pretty_path(&util::dir_name(&path)),
                when.map(|t| fill(tr("notOpened"), &[("when", &kit::format::rel_time(t, kit::format::now_ms()))])).unwrap_or_default(),
            ]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" \u{b7} ");
            let cb = kit::controls::checkbox_desc(if name.is_empty() { &path } else { &name }, &desc);
            cb.set_active(self.download_paths.borrow().contains(&path));
            cb.set_hexpand(true);
            let w = self.me.clone();
            let p = path.clone();
            cb.connect_toggled(move |b| {
                let Some(c) = w.upgrade() else { return };
                if c.syncing.get() {
                    return;
                }
                if b.is_active() {
                    c.download_paths.borrow_mut().insert(p.clone());
                } else {
                    c.download_paths.borrow_mut().remove(&p);
                }
                c.sync_downloads();
            });
            let li = layout::hbox(12, "taskmgr-clean-pick");
            li.append(&cb);
            let sl = layout::label(&bytes(util::num(&f["size"])), "taskmgr-tabular");
            sl.set_valign(gtk::Align::Center);
            li.append(&sl);
            ul.append(&li);
            boxes.push((cb, path));
        }
        let all = kit::controls::hyperlink(tr("pickAll"), None);
        all.add_css_class("taskmgr-clean-more");
        let w = self.me.clone();
        all.connect_clicked(move |_| {
            if let Some(c) = w.upgrade() {
                let paths: Vec<String> = c.dl_items().iter().map(|f| util::text(&f["path"])).collect();
                c.download_paths.borrow_mut().extend(paths);
                c.sync_downloads();
            }
        });
        let none = kit::controls::hyperlink(tr("pickNone"), None);
        none.add_css_class("taskmgr-clean-more");
        let w = self.me.clone();
        none.connect_clicked(move |_| {
            if let Some(c) = w.upgrade() {
                c.download_paths.borrow_mut().clear();
                c.sync_downloads();
            }
        });
        let bar = layout::hbox(16, "taskmgr-clean-pickbar");
        bar.append(&head);
        bar.append(&all);
        bar.append(&none);
        let wrap = layout::vbox(4, "taskmgr-clean-pickwrap");
        wrap.append(&bar);
        let scroll = layout::max_scroll(&ul, 280);
        scroll.add_css_class("taskmgr-clean-details");
        wrap.append(&scroll);
        (wrap.upcast(), Picker { head, boxes })
    }

    fn sync_downloads(&self) {
        if self.download_paths.borrow().is_empty() {
            self.selected.borrow_mut().remove("old-downloads");
        } else {
            self.selected.borrow_mut().insert("old-downloads".into());
        }
        self.update();
    }

    // ---------------------------------------------------------------------------------------------- rows

    fn item_row(&self, it: &Value) -> CleanRow {
        let ok = selectable(it);
        let id = util::text(&it["id"]);
        let name = util::text(&it["name"]);
        let check = kit::controls::checkbox("");
        check.set_active(self.selected.borrow().contains(&id) && ok);
        check.set_sensitive(ok && !self.busy.get());
        check.set_tooltip_text(Some(&name));
        check.set_valign(gtk::Align::Start);
        check.set_size_request(28, -1);
        check.set_widget_name(&format!("taskmgr-cleanup-{id}"));
        let w = self.me.clone();
        let item = it.clone();
        check.connect_toggled(move |b| {
            let Some(c) = w.upgrade() else { return };
            if c.syncing.get() {
                return;
            }
            c.toggle(&item, b.is_active());
        });
        let tags = layout::hbox(6, "taskmgr-clean-tags");
        let pill = |glyph: Option<&str>, text: &str, class: &str| {
            let p = layout::hbox(4, &format!("taskmgr-pill {class}"));
            if let Some(g) = glyph {
                p.append(&kit::icons::glyph(g, 12));
            }
            let l = layout::label(text, "");
            l.set_ellipsize(gtk::pango::EllipsizeMode::None);
            p.append(&l);
            p.set_valign(gtk::Align::Center);
            p
        };
        if it["needsAdmin"] == true {
            let p = pill(Some("shield"), tr("adminTag"), "taskmgr-is-accent");
            p.set_tooltip_text(Some(tr("adminBody")));
            tags.append(&p);
        }
        if id == "old-downloads" {
            tags.append(&pill(Some("trash"), tr("trashTag"), ""));
        } else if it["safe"] == false && id != "trash" {
            tags.append(&pill(None, tr("reviewTag"), "taskmgr-is-warning"));
        }
        let name_row = layout::Layout::wrap(8, 4);
        let nl = layout::wrap(&name, "taskmgr-clean-name");
        name_row.append(&nl);
        if tags.first_child().is_some() {
            name_row.append(&tags);
        }
        let text = layout::vbox(2, "");
        text.set_hexpand(true);
        text.append(&name_row);
        let desc = util::text(&it["description"]);
        if !desc.is_empty() {
            let dl = layout::wrap(&desc, "taskmgr-clean-desc");
            dl.set_max_width_chars(100);
            dl.set_halign(gtk::Align::Start);
            text.append(&dl);
        }
        let details = self.details_node(it);
        if let Some(d) = &details {
            let wrap = layout::vbox(4, "taskmgr-clean-detailwrap");
            wrap.append(&d.toggle);
            wrap.append(&d.content);
            text.append(&wrap);
        }
        let icon = kit::icons::glyph(if it["needsAdmin"] == true { "shield" } else { icon_of(&id) }, 20);
        let icon_box = layout::hbox(0, "taskmgr-clean-icon");
        icon_box.set_size_request(28, 20);
        icon_box.set_hexpand(false);
        icon_box.set_valign(gtk::Align::Start);
        icon.set_hexpand(true);
        icon_box.append(&icon);
        let size = layout::label("", "taskmgr-clean-size");
        size.set_ellipsize(gtk::pango::EllipsizeMode::None);
        size.set_valign(gtk::Align::Start);
        let row = layout::hbox(12, "taskmgr-clean-row");
        if !ok {
            row.add_css_class("taskmgr-is-empty");
        }
        row.append(&check);
        row.append(&icon_box);
        row.append(&text);
        row.append(&size);
        // a click on the row ticks it (not on the check box, a button or the details)
        let click = gtk::GestureClick::new();
        let w = self.me.clone();
        let (item, rw) = (it.clone(), row.downgrade());
        let content = details.as_ref().map(|d| d.content.downgrade());
        click.connect_released(move |_, _, x, y| {
            let (Some(c), Some(row)) = (w.upgrade(), rw.upgrade()) else { return };
            if !selectable(&item) || c.busy.get() {
                return;
            }
            if let Some(t) = row.pick(x, y, gtk::PickFlags::DEFAULT) {
                let in_details = content.as_ref().and_then(|d| d.upgrade()).is_some_and(|d| t.is_ancestor(&d) || t == d);
                if in_details || t.ancestor(gtk::CheckButton::static_type()).is_some() || t.ancestor(gtk::Button::static_type()).is_some() {
                    return;
                }
            }
            let on = !c.selected.borrow().contains(&util::text(&item["id"]));
            c.toggle(&item, on);
        });
        row.add_controller(click);
        CleanRow { id, item: it.clone(), row, check, size, details }
    }

    fn toggle(&self, it: &Value, on: bool) {
        if it["id"] == "old-downloads" {
            let dl = self.dl_items();
            if !dl.is_empty() {
                // ticking the row selects every old download; the list opens so you can review it
                if on {
                    self.download_paths.borrow_mut().extend(dl.iter().map(|f| util::text(&f["path"])));
                } else {
                    self.download_paths.borrow_mut().clear();
                }
                if let Some(d) = self.rows.borrow().iter().find(|r| r.id == "old-downloads").and_then(|r| r.details.clone()) {
                    d.set_open(true);
                }
                self.sync_downloads();
                return;
            }
        }
        let id = util::text(&it["id"]);
        if on {
            self.selected.borrow_mut().insert(id);
        } else {
            self.selected.borrow_mut().remove(&id);
        }
        self.update();
    }

    fn render(&self) {
        kit::controls::clear(&self.list);
        self.rows.borrow_mut().clear();
        let items = self.items.borrow().clone();
        if !items.iter().any(selectable) {
            self.list.append(&kit::controls::empty_state("success", tr("nothingToDo"), tr("nothingBody"), None));
        } else {
            for (title, safe) in [(tr("recommended"), true), (tr("other"), false)] {
                let group: Vec<&Value> = items.iter().filter(|i| (i["safe"] != false) == safe).collect();
                if group.is_empty() {
                    continue;
                }
                let t = layout::section_title(title);
                t.add_css_class("taskmgr-clean-group");
                self.list.append(&t);
                let card = layout::vbox(0, "taskmgr-card taskmgr-clean-card");
                card.set_overflow(gtk::Overflow::Hidden);
                for it in group {
                    let r = self.item_row(it);
                    card.append(&r.row);
                    self.rows.borrow_mut().push(r);
                }
                self.list.append(&card);
            }
        }
        self.update();
    }

    fn selection(&self) -> Vec<Value> {
        let sel = self.selected.borrow();
        self.items.borrow().iter().filter(|i| sel.contains(&util::text(&i["id"])) && selectable(i)).cloned().collect()
    }

    fn update(&self) {
        let sel = self.selection();
        let total: f64 = sel.iter().map(|i| self.size_of(i)).sum();
        let unknown = sel.iter().any(|i| !known(i));
        self.total_label.set_label(&if sel.is_empty() {
            tr("nothing").to_owned()
        } else {
            fill(tr(if unknown { "selectedUnknown" } else { "selected" }), &[("size", &bytes(total))])
        });
        self.clean_btn.set_sensitive(!sel.is_empty() && !self.busy.get());
        self.admin_bar.set_visible(sel.iter().any(|i| i["needsAdmin"] == true));
        self.syncing.set(true);
        let selected = self.selected.borrow().clone();
        let paths = self.download_paths.borrow().clone();
        let dl = self.dl_items();
        for r in self.rows.borrow().iter() {
            let on = selected.contains(&r.id);
            util::set_class(&r.row, "taskmgr-is-on", on);
            if r.check.is_active() != on && selectable(&r.item) {
                r.check.set_active(on);
            }
            if r.id == "old-downloads" && !dl.is_empty() {
                r.check.set_inconsistent(!paths.is_empty() && paths.len() < dl.len());
                let full = util::num(&r.item["size"]);
                r.size.set_label(&if !paths.is_empty() && paths.len() < dl.len() {
                    format!("{} / {}", bytes(self.size_of(&r.item)), bytes(full))
                } else {
                    bytes(full)
                });
                if let Some(p) = r.details.as_ref().and_then(|d| d.picker.as_ref()) {
                    p.head.set_label(&fill(tr("picked"), &[("n", &paths.len().to_string()), ("m", &dl.len().to_string())]));
                    for (b, path) in &p.boxes {
                        let on = paths.contains(path);
                        if b.is_active() != on {
                            b.set_active(on);
                        }
                    }
                }
            } else if known(&r.item) {
                let s = util::num(&r.item["size"]);
                r.size.set_label(&if s > 0.0 { bytes(s) } else { "0 bytes".to_owned() });
            } else {
                r.size.set_label(tr("unknownSize"));
            }
        }
        self.syncing.set(false);
    }

    // -------------------------------------------------------------------------------------------- clean up

    fn run(&self) {
        let sel = self.selection();
        if sel.is_empty() {
            return;
        }
        let total: f64 = sel.iter().map(|i| self.size_of(i)).sum();
        let any_admin = sel.iter().any(|i| i["needsAdmin"] == true);
        let dl = self.dl_items();
        let n_paths = self.download_paths.borrow().len();
        let content = layout::vbox(16, "taskmgr-dlg");
        content.append(&layout::wrap(tr("confirmBody"), ""));
        let list = layout::vbox(2, "taskmgr-dlg-list");
        for i in &sel {
            let id = util::text(&i["id"]);
            let li = layout::hbox(10, "taskmgr-dlg-item");
            li.append(&kit::icons::glyph(if i["needsAdmin"] == true { "shield" } else { icon_of(&id) }, 14));
            let name = util::text(&i["name"]);
            let label = if id == "old-downloads" && !dl.is_empty() {
                format!("{name} \u{2013} {}", count_text(n_paths as f64, tr("filesOne"), tr("filesOther")))
            } else {
                name
            };
            let nl = layout::label(&label, "");
            nl.set_hexpand(true);
            li.append(&nl);
            li.append(&layout::label(&if known(i) { bytes(self.size_of(i)) } else { tr("unknownSize").to_owned() }, "taskmgr-text-2 taskmgr-tabular"));
            list.append(&li);
        }
        let scroll = layout::max_scroll(&list, 208);
        scroll.add_css_class("taskmgr-dlg-listwrap");
        content.append(&scroll);
        let hint = |key: &str| layout::wrap(tr(key), "taskmgr-dlg-hint");
        if sel.iter().any(|i| i["id"] == "trash") {
            content.append(&hint("confirmPermanent"));
        }
        if sel.iter().any(|i| i["id"] == "old-downloads") {
            content.append(&hint("confirmDownloads"));
        }
        if any_admin {
            content.append(&hint("confirmAdmin"));
        }
        let title = if total > 0.0 { fill(tr("confirmTitle"), &[("size", &bytes(total))]) } else { tr("confirmTitleUnknown").to_owned() };
        let w = self.me.clone();
        Dialog::new(&title).body(&content).primary(tr("clean")).close_label(tr("cancel")).width(480).show(&self.app.parent(), move |r| {
            if r != DialogResult::Primary {
                return;
            }
            if let Some(c) = w.upgrade() {
                c.run_confirmed(sel);
            }
        });
    }

    fn run_confirmed(&self, sel: Vec<Value>) {
        self.busy.set(true);
        self.clean_btn.set_sensitive(false);
        layout::relabel(&self.clean_btn, tr("cleaning"));
        kit::controls::clear(&self.result_bar);
        self.result_bar.set_visible(false);
        let mut args = json!({ "ids": sel.iter().map(|i| util::text(&i["id"])).collect::<Vec<_>>() });
        if sel.iter().any(|i| i["id"] == "old-downloads") && !self.dl_items().is_empty() {
            args["paths"] = json!(self.download_paths.borrow().iter().cloned().collect::<Vec<_>>());
        }
        let w = self.me.clone();
        hostcall::call("storage.cleanup.run", args, move |r| {
            let Some(c) = w.upgrade() else { return };
            c.busy.set(false);
            layout::relabel(&c.clean_btn, tr("clean"));
            match r {
                Err(e) => {
                    let bar = InfoBar::new(Severity::Error, tr("partly"), &e.message);
                    c.result_bar.append(bar.widget());
                    c.result_bar.set_visible(true);
                    c.update();
                }
                Ok(res) => {
                    c.show_result(&res);
                    c.selected.borrow_mut().clear();
                    c.download_paths.borrow_mut().clear();
                    c.loaded.set(false);
                    let w = c.me.clone();
                    c.load(Some(Box::new(move || {
                        if let Some(c) = w.upgrade() {
                            (c.on_changed)();
                        }
                    })));
                }
            }
        });
    }

    fn show_result(&self, res: &Value) {
        let results: Vec<Value> = res["results"].as_array().cloned().unwrap_or_default();
        let failed: Vec<&Value> = results.iter().filter(|r| r["ok"] != true).collect();
        let freed = res["freed"].as_f64().unwrap_or_else(|| results.iter().map(|r| util::num(&r["freed"])).sum());
        let bar = if res["dryRun"] == true {
            InfoBar::new(Severity::Info, tr("dryRun"), "")
        } else {
            let names: HashMap<String, String> = self.items.borrow().iter().map(|i| (util::text(&i["id"]), util::text(&i["name"]))).collect();
            let mut lines = Vec::new();
            if let Some(dl) = results.iter().find(|r| r["id"] == "old-downloads" && util::num(&r["movedToTrash"]) > 0.0) {
                let n = util::num(&dl["count"]);
                lines.push(fill(&count_text(n, tr("movedDownloadsOne"), tr("movedDownloadsOther")), &[("size", &bytes(util::num(&dl["movedToTrash"])))]));
            }
            let skipped: Vec<String> = results.iter().flat_map(|r| r["skipped"].as_array().cloned().unwrap_or_default()).map(|s| util::text(&s)).collect();
            if !skipped.is_empty() {
                lines.push(fill(tr("skipped"), &[("names", &skipped.join(", "))]));
            }
            for f in &failed {
                let id = util::text(&f["id"]);
                let name = names.get(&id).cloned().filter(|n| !n.is_empty()).unwrap_or(id);
                lines.push(fill(tr("failedItem"), &[("name", &name), ("error", &util::text(&f["error"]))]));
            }
            let title = fill(tr("freed"), &[("size", &bytes(freed))]);
            if failed.is_empty() {
                let mut msg = vec![tr("freedBody").to_owned()];
                msg.extend(lines);
                self.app.toast(Toast::new(&title).severity(Severity::Success).timeout(4000));
                InfoBar::new(Severity::Success, &title, &msg.join(" "))
            } else {
                InfoBar::new(
                    if freed > 0.0 { Severity::Warning } else { Severity::Error },
                    &if freed > 0.0 { fill(tr("freedPartly"), &[("size", &bytes(freed))]) } else { tr("partly").to_owned() },
                    &lines.join(" \u{b7} "),
                )
            }
        };
        let rb = self.result_bar.downgrade();
        bar.connect_closed(move || {
            if let Some(rb) = rb.upgrade() {
                rb.set_visible(false);
            }
        });
        self.result_bar.append(bar.widget());
        self.result_bar.set_visible(true);
    }
}
