//! The "Opties voor Alles" dialog (options.js): which drives and folders to index, excluded folders, hidden
//! files, live updates and "Index opnieuw opbouwen".
use kit::controls::{self, Dialog, DialogResult, InfoBar, Severity, SettingsCard, SettingsGroup, Toast, ToggleSwitch, Variant};
use kit::prelude::*;
use kit::t;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Clone, Debug)]
struct Location {
    path: String,
    label: String,
    total: f64,
    free: f64,
    auto: bool,
    folder: bool,
    removable: bool,
    indexed: bool,
}

fn base_name(p: &str) -> String {
    p.trim_end_matches('/').rsplit('/').next().filter(|s| !s.is_empty()).unwrap_or(p).to_owned()
}

/// index.locations with the drive names of storage.volumes; without it the volumes themselves, and last the
/// roots of the index status (options.js loadLocations).
fn locations(locs: Option<Value>, vols: Option<Value>, status: &Value, roots: &Value) -> Vec<Location> {
    let system = t("Systeemschijf", "System drive");
    let mut names: HashMap<String, String> = HashMap::new();
    if let Some(vols) = vols.as_ref().and_then(Value::as_array) {
        for v in vols {
            let name = v["name"].as_str().filter(|s| !s.is_empty()).or(v["label"].as_str().filter(|s| !s.is_empty()));
            if let (Some(m), Some(name)) = (v["mount"].as_str(), name) {
                names.insert(m.to_owned(), if v["isSystem"] == true { system.to_owned() } else { name.to_owned() });
            }
        }
    }
    if let Some(list) = locs.as_ref().and_then(Value::as_array).filter(|l| !l.is_empty()) {
        return list
            .iter()
            .filter_map(|l| {
                let path = l["path"].as_str()?.to_owned();
                let folder = l["kind"] == "folder";
                let label = match names.get(&path).filter(|_| !folder) {
                    Some(n) => n.clone(),
                    None => l["label"].as_str().map(str::to_owned).unwrap_or_else(|| base_name(&path)),
                };
                Some(Location {
                    label,
                    total: l["total"].as_f64().unwrap_or(0.0),
                    free: l["free"].as_f64().unwrap_or(0.0),
                    auto: l["auto"] == true,
                    folder,
                    removable: l["removable"] == true,
                    indexed: l["indexed"] == true,
                    path,
                })
            })
            .collect();
    }
    if let Some(vols) = vols.as_ref().and_then(Value::as_array).filter(|v| !v.is_empty()) {
        let auto = roots.as_str() == Some("auto") || roots.is_null();
        return vols
            .iter()
            .filter_map(|v| {
                let mount = v["mount"].as_str().filter(|s| !s.is_empty())?.to_owned();
                let label = if v["isSystem"] == true {
                    system.to_owned()
                } else {
                    v["label"].as_str().filter(|s| !s.is_empty()).or(v["name"].as_str().filter(|s| !s.is_empty())).unwrap_or(&mount).to_owned()
                };
                let indexed = auto || roots.as_array().is_some_and(|r| r.iter().any(|x| x.as_str() == Some(mount.as_str())));
                Some(Location {
                    label,
                    total: v["total"].as_f64().unwrap_or(0.0),
                    free: v["free"].as_f64().unwrap_or(0.0),
                    auto: true,
                    folder: false,
                    removable: v["removable"] == true,
                    indexed,
                    path: mount,
                })
            })
            .collect();
    }
    status["roots"]
        .as_array()
        .map(|r| {
            r.iter()
                .filter_map(|r| {
                    let path = r["path"].as_str()?.to_owned();
                    Some(Location {
                        label: base_name(&path),
                        total: 0.0,
                        free: 0.0,
                        auto: true,
                        folder: false,
                        removable: false,
                        indexed: true,
                        path,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

struct State {
    on: HashMap<String, bool>,
    folders: Vec<String>,
    excluded: Vec<String>,
    hidden: bool,
    live: bool,
}

fn string_list(v: &Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect()).unwrap_or_default()
}

/// Ask for a folder: the toolkit's folder chooser (the Win11 Explorer picker through the portal when the user
/// set it up); when there is none, the path typed in a small dialog (options.js pickFolder's fallback).
fn pick_folder(parent: &gtk::Widget, title: &str, done: impl FnOnce(Option<String>) + 'static) {
    let dialog = gtk::FileDialog::builder().title(title).modal(true).build();
    let window = parent.root().and_downcast::<gtk::Window>();
    let p = parent.downgrade();
    let title = title.to_owned();
    dialog.select_folder(window.as_ref(), gio::Cancellable::NONE, move |r| match r {
        Ok(f) => done(f.path().map(|p| p.to_string_lossy().into_owned())),
        Err(e) if e.matches(gtk::DialogError::Dismissed) || e.matches(gtk::DialogError::Cancelled) => done(None),
        Err(e) => {
            eprintln!("win11-gtk-alles: folder chooser: {e}");
            let Some(parent) = p.upgrade() else {
                done(None);
                return;
            };
            prompt_path(&parent, &title, done);
        }
    });
}

fn prompt_path(parent: &gtk::Widget, _title: &str, done: impl FnOnce(Option<String>) + 'static) {
    let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
    body.add_css_class("alles-prompt");
    body.append(&controls::label(t("Typ het volledige pad van de map", "Type the full path of the folder"), "w11-body", true));
    let entry = controls::text_box("/home/robin/…");
    body.append(&entry);
    let e = entry.clone();
    Dialog::new(t("Map kiezen", "Choose a folder"))
        .body(&body)
        .primary(t("Toevoegen", "Add"))
        .close_label(t("Annuleren", "Cancel"))
        .show(parent, move |r| {
            let v = e.text().trim().to_owned();
            done((r == DialogResult::Primary && !v.is_empty()).then_some(v));
        });
}

struct Ui {
    parent: glib::WeakRef<gtk::Widget>,
    drives: Vec<Location>,
    state: RefCell<State>,
    error: InfoBar,
    loc_group: SettingsGroup,
    ex_group: SettingsGroup,
}

fn remove_button(on: impl Fn() + 'static) -> gtk::Button {
    let b = gtk::Button::new();
    b.add_css_class("w11-icon-button");
    b.set_child(Some(&kit::icons::glyph("close", 12)));
    b.set_size_request(32, 32);
    b.set_valign(gtk::Align::Center);
    b.set_tooltip_text(Some(t("Verwijderen uit de lijst", "Remove from the list")));
    b.connect_clicked(move |_| on());
    b
}

impl Ui {
    fn parent(&self) -> Option<gtk::Widget> {
        self.parent.upgrade()
    }

    fn render_locations(self: &Rc<Self>) {
        let g = &self.loc_group;
        g.clear();
        for d in &self.drives {
            let mut desc = Vec::new();
            if d.path != d.label {
                desc.push(d.path.clone());
            }
            if d.total > 0.0 {
                let (free, total) = (kit::format::bytes(d.free), kit::format::bytes(d.total));
                desc.push(if kit::i18n::en() { format!("{free} free of {total}") } else { format!("{free} vrij van {total}") });
            }
            if d.removable {
                desc.push(t("Externe schijf", "External drive").to_owned());
            }
            let on = self.state.borrow().on.get(&d.path).copied().unwrap_or(false);
            let sw = ToggleSwitch::new(on);
            let me = Rc::downgrade(self);
            let path = d.path.clone();
            sw.connect_toggled(move |v| {
                if let Some(me) = me.upgrade() {
                    me.state.borrow_mut().on.insert(path.clone(), v);
                    me.error.set_visible(false);
                }
            });
            let card = SettingsCard::new(if d.removable { "usb" } else { "drive" }, &d.label, &desc.join(" · "));
            card.set_control(sw.widget());
            g.add(card.widget());
        }
        let folders = self.state.borrow().folders.clone();
        for f in folders {
            let me = Rc::downgrade(self);
            let path = f.clone();
            let rm = remove_button(move || {
                if let Some(me) = me.upgrade() {
                    {
                        let mut s = me.state.borrow_mut();
                        s.folders.retain(|x| *x != path);
                        s.on.remove(&path);
                    }
                    me.render_locations();
                }
            });
            let card = SettingsCard::new("folder", &base_name(&f), &f);
            card.set_control(&rm);
            g.add(card.widget());
        }
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 14);
        actions.add_css_class("alles-opt-actions");
        let add = controls::button_with_glyph("plus", t("Map toevoegen", "Add folder"), Variant::Standard);
        add.set_widget_name("alles-opt-add-folder");
        let me = Rc::downgrade(self);
        add.connect_clicked(move |_| {
            let Some(me) = me.upgrade() else { return };
            let Some(parent) = me.parent() else { return };
            let weak = Rc::downgrade(&me);
            pick_folder(&parent, t("Map toevoegen", "Add folder"), move |p| {
                let (Some(me), Some(p)) = (weak.upgrade(), p) else { return };
                let known = me.state.borrow().folders.contains(&p) || me.drives.iter().any(|d| d.path == p);
                if !known {
                    {
                        let mut s = me.state.borrow_mut();
                        s.folders.push(p.clone());
                        s.on.insert(p, true);
                    }
                    me.render_locations();
                }
            });
        });
        actions.append(&add);
        g.add(&actions);
    }

    fn render_excluded(self: &Rc<Self>) {
        let g = &self.ex_group;
        g.clear();
        let excluded = self.state.borrow().excluded.clone();
        if excluded.is_empty() {
            let empty = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            empty.add_css_class("alles-opt-empty");
            empty.append(&kit::icons::glyph("shield", 16));
            empty.append(&controls::label(t("Er zijn geen mappen uitgesloten.", "No folders are excluded."), "", true));
            g.add(&empty);
        }
        for p in excluded {
            let me = Rc::downgrade(self);
            let path = p.clone();
            let rm = remove_button(move || {
                if let Some(me) = me.upgrade() {
                    me.state.borrow_mut().excluded.retain(|x| *x != path);
                    me.render_excluded();
                }
            });
            let card = SettingsCard::new("eye-off", &base_name(&p), &p);
            card.set_control(&rm);
            g.add(card.widget());
        }
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 14);
        actions.add_css_class("alles-opt-actions");
        let add = controls::button_with_glyph("plus", t("Map uitsluiten", "Exclude folder"), Variant::Standard);
        add.set_widget_name("alles-opt-exclude");
        let me = Rc::downgrade(self);
        add.connect_clicked(move |_| {
            let Some(me) = me.upgrade() else { return };
            let Some(parent) = me.parent() else { return };
            let weak = Rc::downgrade(&me);
            pick_folder(&parent, t("Map uitsluiten", "Exclude folder"), move |p| {
                let (Some(me), Some(p)) = (weak.upgrade(), p) else { return };
                let known = me.state.borrow().excluded.contains(&p);
                if !known {
                    me.state.borrow_mut().excluded.push(p);
                    me.render_excluded();
                }
            });
        });
        actions.append(&add);
        let note = controls::label(
            t("Systeemmappen zoals /proc, /sys en /tmp worden altijd overgeslagen.", "System folders such as /proc, /sys and /tmp are always skipped."),
            "alles-opt-note",
            true,
        );
        note.set_hexpand(true);
        actions.append(&note);
        g.add(&actions);
    }
}

/// Open the options; `closed()` runs once the dialog is gone (main.js refreshes the status then).
pub fn open_options(parent: &gtk::Widget, status: Value, closed: impl FnOnce() + 'static) {
    let ctx = kit::ctx();
    let p = parent.downgrade();
    glib::spawn_future_local(async move {
        let (locs, vols) = futures_join(ctx.call_async("index.locations", json!({})), ctx.call_async("storage.volumes", json!({}))).await;
        let Some(parent) = p.upgrade() else { return };
        let ctx = kit::ctx();
        let roots = ctx.setting("alles.roots");
        let locations = locations(locs.ok(), vols.ok(), &status, &roots);
        show(&parent, locations, status, closed);
    });
}

/// Both futures, run together.
async fn futures_join<A, B>(a: impl std::future::Future<Output = A>, b: impl std::future::Future<Output = B>) -> (A, B) {
    let a = std::pin::pin!(a);
    let b = std::pin::pin!(b);
    let mut a = Some(a);
    let mut b = Some(b);
    let mut ra: Option<A> = None;
    let mut rb: Option<B> = None;
    std::future::poll_fn(move |cx| {
        if let Some(f) = a.as_mut()
            && let std::task::Poll::Ready(v) = f.as_mut().poll(cx)
        {
            ra = Some(v);
            a = None;
        }
        if let Some(f) = b.as_mut()
            && let std::task::Poll::Ready(v) = f.as_mut().poll(cx)
        {
            rb = Some(v);
            b = None;
        }
        if ra.is_some() && rb.is_some() {
            match (ra.take(), rb.take()) {
                (Some(x), Some(y)) => std::task::Poll::Ready((x, y)),
                _ => std::task::Poll::Pending,
            }
        } else {
            std::task::Poll::Pending
        }
    })
    .await
}

fn show(parent: &gtk::Widget, locations: Vec<Location>, status: Value, closed: impl FnOnce() + 'static) {
    let ctx = kit::ctx();
    let orig_roots = match ctx.setting("alles.roots") {
        Value::Null => json!("auto"),
        v => v,
    };
    let excluded = string_list(&ctx.setting("alles.excluded"));
    let state = State {
        on: locations.iter().map(|l| (l.path.clone(), l.indexed)).collect(),
        folders: locations.iter().filter(|l| l.folder).map(|l| l.path.clone()).collect(),
        excluded,
        hidden: ctx.setting("alles.showHidden") == true,
        live: ctx.setting("alles.liveUpdate") != false,
    };
    let drives: Vec<Location> = locations.into_iter().filter(|l| !l.folder).collect();
    let error = InfoBar::new(Severity::Error, t("Kies minstens één locatie om te doorzoeken.", "Choose at least one location to search."), "");
    error.set_closable(false);
    error.set_visible(false);
    let ui = Rc::new(Ui {
        parent: parent.downgrade(),
        drives,
        state: RefCell::new(state),
        error: error.clone(),
        loc_group: SettingsGroup::new(t("Locaties", "Locations"), t("Schijven met een schakelaar aan worden doorzocht.", "Drives that are switched on are searched.")),
        ex_group: SettingsGroup::new(
            t("Uitgesloten mappen", "Excluded folders"),
            t("Deze mappen en alles wat erin staat worden overgeslagen.", "These folders and everything in them are skipped."),
        ),
    });
    ui.render_locations();
    ui.render_excluded();

    // ---- view and updates
    let view = SettingsGroup::new(t("Weergave en bijwerken", "View and updates"), "");
    let hidden = ToggleSwitch::new(ui.state.borrow().hidden);
    let w = Rc::downgrade(&ui);
    hidden.connect_toggled(move |v| {
        if let Some(ui) = w.upgrade() {
            ui.state.borrow_mut().hidden = v;
        }
    });
    let card = SettingsCard::new(
        "eye",
        t("Verborgen bestanden en mappen weergeven", "Show hidden files and folders"),
        t(
            "Bestanden waarvan de naam met een punt begint, en systeemmappen van Windows-schijven.",
            "Files whose name starts with a dot, and Windows system folders.",
        ),
    );
    card.set_control(hidden.widget());
    hidden.switch().set_widget_name("alles-opt-hidden");
    view.add(card.widget());
    let live = ToggleSwitch::new(ui.state.borrow().live);
    let w = Rc::downgrade(&ui);
    live.connect_toggled(move |v| {
        if let Some(ui) = w.upgrade() {
            ui.state.borrow_mut().live = v;
        }
    });
    let card = SettingsCard::new(
        "lightning",
        t("Live bijwerken", "Live updates"),
        t(
            "Wijzigingen in je persoonlijke map verschijnen meteen. Andere schijven worden elke 10 minuten bijgewerkt.",
            "Changes in your home folder show up right away. Other drives are updated every 10 minutes.",
        ),
    );
    card.set_control(live.widget());
    live.switch().set_widget_name("alles-opt-live");
    view.add(card.widget());

    // ---- index
    let index = SettingsGroup::new(t("Index", "Index"), "");
    let files = status["files"].as_f64().unwrap_or(0.0);
    let folders = status["folders"].as_f64().unwrap_or(0.0);
    let stats = if files > 0.0 || folders > 0.0 {
        let base = if kit::i18n::en() {
            format!("{} files and {} folders", crate::text::num(files), crate::text::num(folders))
        } else {
            format!("{} bestanden en {} mappen", crate::text::num(files), crate::text::num(folders))
        };
        match status["builtAt"].as_f64() {
            Some(at) if at > 0.0 => format!("{base} · {}", crate::ui::built_at(at)),
            _ => base,
        }
    } else {
        String::new()
    };
    let rebuild = controls::button_with_glyph("refresh", t("Opnieuw opbouwen", "Rebuild"), Variant::Standard);
    rebuild.set_widget_name("alles-opt-rebuild");
    rebuild.connect_clicked(|b| {
        let b = b.clone();
        controls::confirm(
            &b.clone(),
            t("Index opnieuw opbouwen?", "Rebuild the index?"),
            t(
                "Alles leest al je schijven opnieuw in. Dat kan een paar minuten duren en je computer kan ondertussen iets trager zijn. Je kunt gewoon blijven zoeken.",
                "Alles will read all your drives again. This can take a few minutes, and your computer may be a bit slower in the meantime. You can keep searching.",
            ),
            t("Opnieuw opbouwen", "Rebuild"),
            false,
            move |ok| {
                if !ok {
                    return;
                }
                let bw = b.downgrade();
                kit::ctx().call("index.rebuild", json!({}), move |r| {
                    let Some(b) = bw.upgrade() else { return };
                    match r {
                        Ok(_) => {
                            Toast::new(t("De index wordt opnieuw opgebouwd", "The index is being rebuilt")).severity(Severity::Info).timeout(3500).show(&b);
                            b.set_sensitive(false);
                        }
                        Err(e) => {
                            Toast::new(t("Opslaan is niet gelukt", "Couldn’t save")).message(&e.message).severity(Severity::Error).show(&b);
                        }
                    }
                });
            },
        );
    });
    let card = SettingsCard::new(
        "refresh",
        t("Index opnieuw opbouwen", "Rebuild the index"),
        t(
            "Maakt het overzicht helemaal opnieuw. Dit duurt een paar minuten; zoeken blijft gewoon werken.",
            "Rebuilds the overview from scratch. This takes a few minutes; searching keeps working.",
        ),
    );
    card.set_control(&rebuild);
    if !stats.is_empty() {
        add_stats(&card, &stats);
    }
    index.add(card.widget());

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.add_css_class("alles-opt");
    content.append(&controls::label(t("Kies welke schijven en mappen Alles doorzoekt.", "Choose which drives and folders Alles searches."), "alles-opt-intro", true));
    error.widget().add_css_class("alles-opt-error");
    content.append(error.widget());
    content.append(ui.loc_group.widget());
    content.append(ui.ex_group.widget());
    content.append(view.widget());
    content.append(index.widget());
    let scroller = gtk::ScrolledWindow::new();
    scroller.set_hscrollbar_policy(gtk::PolicyType::Never);
    scroller.set_propagate_natural_height(true);
    let height = parent.root().and_downcast::<gtk::Window>().map(|w| w.height()).filter(|h| *h > 0).unwrap_or(680);
    scroller.set_max_content_height((height - 200).max(240));
    scroller.set_child(Some(&content));
    scroller.set_widget_name("alles-options-body");

    let width = crate::actions::parent_width(parent).map(|w| (w - 48).clamp(360, 680)).unwrap_or(680);
    let ui2 = ui.clone();
    let saving = Rc::new(std::cell::Cell::new(false));
    Dialog::new(t("Opties voor Alles", "Alles options"))
        .body(&scroller)
        .primary(t("Opslaan", "Save"))
        .close_label(t("Annuleren", "Cancel"))
        .width(width)
        .on_primary(move |d| {
            if !saving.get() {
                save(&ui2, &orig_roots, d, saving.clone());
            }
            false
        })
        .show(parent, move |_| closed());
}

/// The stats line under the rebuild card's description (.alles-opt__stats).
fn add_stats(card: &SettingsCard, stats: &str) {
    fn find(w: &gtk::Widget) -> Option<gtk::Box> {
        if w.has_css_class("w11-sc-desc") {
            return w.parent().and_downcast::<gtk::Box>();
        }
        let mut c = w.first_child();
        while let Some(child) = c {
            if let Some(f) = find(&child) {
                return Some(f);
            }
            c = child.next_sibling();
        }
        None
    }
    if let Some(text) = find(card.widget()) {
        text.append(&controls::label(stats, "alles-opt-stats", true));
    }
}

fn save(ui: &Rc<Ui>, orig_roots: &Value, dialog: &Dialog, saving: Rc<std::cell::Cell<bool>>) {
    let ctx = kit::ctx();
    let (enabled, roots_auto, excluded, hidden, live) = {
        let s = ui.state.borrow();
        let enabled_drives: Vec<String> = ui.drives.iter().filter(|d| s.on.get(&d.path) == Some(&true)).map(|d| d.path.clone()).collect();
        let mut enabled = enabled_drives.clone();
        enabled.extend(s.folders.iter().filter(|f| s.on.get(*f) != Some(&false)).cloned());
        let all_auto = !ui.drives.is_empty()
            && ui.drives.iter().all(|d| !d.auto || s.on.get(&d.path) == Some(&true))
            && s.folders.is_empty()
            && ui.drives.iter().filter(|d| d.auto).count() == enabled_drives.len();
        (enabled, all_auto, s.excluded.clone(), s.hidden, s.live)
    };
    if enabled.is_empty() {
        ui.error.set_visible(true);
        return;
    }
    let roots = if roots_auto { json!("auto") } else { json!(enabled) };
    let same_roots = roots == *orig_roots;
    let same_ex = excluded == string_list(&ctx.setting("alles.excluded"));
    let hidden_changed = hidden != (ctx.setting("alles.showHidden") == true);
    let live_changed = live != (ctx.setting("alles.liveUpdate") != false);
    saving.set(true);
    dialog.set_busy(true);
    let d = dialog.clone();
    let parent = ui.parent.clone();
    glib::spawn_future_local(async move {
        let ctx = kit::ctx();
        let toast_error = |e: &kit::CallError| {
            if let Some(p) = parent.upgrade() {
                Toast::new(t("Opslaan is niet gelukt", "Couldn’t save")).message(&e.message).severity(Severity::Error).show(&p);
            }
        };
        let mut ok = true;
        if !same_roots || !same_ex {
            match ctx.call_async("index.setRoots", json!({ "roots": roots, "excluded": excluded })).await {
                Ok(r) => {
                    if r["overridden"] == true
                        && let Some(p) = parent.upgrade()
                    {
                        Toast::new(t("Opties opgeslagen", "Options saved"))
                            .message(t(
                                "De locaties worden nu bepaald door de instelling WIN11SHELL_INDEX_ROOTS.",
                                "The locations are currently set by WIN11SHELL_INDEX_ROOTS.",
                            ))
                            .severity(Severity::Info)
                            .show(&p);
                    }
                }
                Err(e) => {
                    toast_error(&e);
                    ok = false;
                }
            }
        }
        for (path, changed, value) in [("alles.showHidden", hidden_changed, hidden), ("alles.liveUpdate", live_changed, live)] {
            if !ok || !changed {
                continue;
            }
            let (tx, rx) = futures_oneshot();
            ctx.set_setting_then(path, json!(value), move |r| tx(r));
            if let Err(e) = rx.await {
                toast_error(&e);
                ok = false;
            }
        }
        saving.set(false);
        d.set_busy(false);
        if ok {
            d.close(DialogResult::Primary);
        }
    });
}

/// A one-shot channel for a callback's result (the kit's call_async does the same for calls).
fn futures_oneshot<T: 'static>() -> (impl FnOnce(T), impl std::future::Future<Output = T>) {
    let slot: Rc<RefCell<(Option<T>, Option<std::task::Waker>)>> = Rc::new(RefCell::new((None, None)));
    let s = slot.clone();
    let send = move |v: T| {
        let waker = {
            let mut s = s.borrow_mut();
            s.0 = Some(v);
            s.1.take()
        };
        if let Some(w) = waker {
            w.wake();
        }
    };
    let recv = std::future::poll_fn(move |cx| {
        let mut s = slot.borrow_mut();
        match s.0.take() {
            Some(v) => std::task::Poll::Ready(v),
            None => {
                s.1 = Some(cx.waker().clone());
                std::task::Poll::Pending
            }
        }
    });
    (send, recv)
}
