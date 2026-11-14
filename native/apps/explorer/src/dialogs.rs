//! Dialogs (vendor/ui/js/apps/explorer/dialogs.js): Eigenschappen of a file or folder (Algemeen / Details,
//! the name box, Kenmerken, OK · Annuleren · Toepassen), of several items, of a drive (donut, Ruimte
//! vrijmaken) and of the Prullenbak; Delen; Mapopties. Also the sheet of builder B's widgets (css/ops.css:
//! these dialogs, the details pane, the progress cards and the drag and drop hint).
use crate::explorer::{Explorer, ToastOpts, WeakExplorer};
use crate::i18n::{contains_val, f, n, s};
use crate::model::Loc;
use crate::ops::{self, Attr};
use crate::util::{self, Item, flag, num, st};
use gtk::prelude::*;
use gtk::gdk;
use kit::controls::{Dialog, DialogResult, Pivot, Variant};
use kit::format::{self, DateFormat};
use serde_json::{Value, json};
use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::rc::{Rc, Weak};

const DASH: &str = "\u{2013}";
const CSS: &str = include_str!("../css/ops.css");

thread_local! {
    static CSS_DONE: Cell<bool> = const { Cell::new(false) };
}

/// The sheet of the dialogs, the details pane, the progress cards and the drop hint, once per process
/// (above the kit's and the Explorer's, which install_app_css puts at APPLICATION + 2).
pub fn install_css() {
    if CSS_DONE.with(|d| d.replace(true)) {
        return;
    }
    let Some(display) = gdk::Display::default() else { return };
    let p = gtk::CssProvider::new();
    p.connect_parsing_error(|_, section, error| {
        eprintln!("win11-gtk-explorer: ops css {}: {}", section.to_str(), error);
    });
    p.load_from_string(CSS);
    gtk::style_context_add_provider_for_display(&display, &p, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 3);
}

// ============================================================================================ pieces

/// "zaterdag 25 oktober 2026, 14:05:09" ("–" without a time).
fn long_date(ms: f64) -> String {
    if ms > 0.0 && ms.is_finite() { format!("{}, {}", format::date(ms, DateFormat::Long), format::time(ms, true, true)) } else { DASH.into() }
}

/// `bytes_full` of an optional number ("–" when absent).
fn bytes_opt(v: Option<f64>) -> String {
    match v {
        Some(n) if n.is_finite() => util::bytes_full(n),
        _ => DASH.into(),
    }
}

fn name_of(item: &Value) -> String {
    match st(item, "name") {
        "" => util::basename(st(item, "path")),
        n => n.to_owned(),
    }
}

/// A value of a property row (selectable, wrapping).
fn value_label(text: &str) -> gtk::Label {
    let l = kit::controls::label(text, "xp-prop-value", true);
    l.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    l.set_selectable(true);
    l.set_max_width_chars(36);
    l.set_hexpand(true);
    l.set_valign(gtk::Align::Start);
    l
}

/// `.xp-prop`: a 136 px label column and the value.
fn row_w(label: &impl IsA<gtk::Widget>, value: &impl IsA<gtk::Widget>) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    b.add_css_class("xp-prop");
    let l = label.as_ref();
    l.set_size_request(136, -1);
    l.set_hexpand(false);
    l.set_valign(gtk::Align::Start);
    b.append(l);
    let v = value.as_ref();
    v.set_hexpand(true);
    b.append(v);
    b
}

fn prop_label(text: &str) -> gtk::Label {
    let l = kit::controls::label(text, "xp-prop-label", true);
    l.set_max_width_chars(1);
    l.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    l
}

fn row(label: &str, value: &impl IsA<gtk::Widget>) -> gtk::Box {
    row_w(&prop_label(label), value)
}

fn row_text(label: &str, text: &str) -> gtk::Box {
    row(label, &value_label(if text.is_empty() { DASH } else { text }))
}

fn divider() -> gtk::Separator {
    let d = gtk::Separator::new(gtk::Orientation::Horizontal);
    d.add_css_class("w11-divider");
    d.add_css_class("xp-props-sep");
    d
}

fn props_box() -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 0);
    b.add_css_class("xp-props");
    b
}

/// `.xp-props__head`: an icon and a strong text (drives, the Prullenbak, several items).
fn head(icon: &impl IsA<gtk::Widget>, text: &str) -> gtk::Box {
    let h = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    h.add_css_class("xp-props-head");
    icon.as_ref().set_valign(gtk::Align::Center);
    h.append(icon.as_ref());
    let l = kit::controls::label(text, "w11-body-strong", true);
    l.set_hexpand(true);
    h.append(&l);
    h
}

fn dialog_root(child: &impl IsA<gtk::Widget>) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 0);
    b.add_css_class("xp-propsdlg");
    b.append(child.as_ref());
    b
}

fn user_name(x: &Explorer) -> String {
    x.state().user["name"].as_str().unwrap_or("").to_owned()
}

// ============================================================================================ file / folder

/// Eigenschappen of one or more items (a drive item goes to `show_drive_properties`).
pub fn show_properties(x: &Explorer, items: Vec<Item>) {
    if items.is_empty() {
        return;
    }
    if items.len() == 1
        && let Some(v) = items[0].get("volume").filter(|v| v.is_object())
    {
        show_drive_properties(x, v);
        return;
    }
    if items.len() > 1 {
        show_multi(x, items);
        return;
    }
    show_single(x, items[0].clone());
}

/// The state of an open Eigenschappen dialog of one item.
struct Props {
    x: WeakExplorer,
    /// The name it had when the dialog opened.
    name: String,
    /// Where it is now (after an Apply that renamed it).
    cur_path: RefCell<String>,
    entry: gtk::Entry,
    error: gtk::Label,
    read_only: gtk::CheckButton,
    hidden: gtk::CheckButton,
    /// Set while the dialog itself changes a check box.
    silent: Cell<bool>,
    dirty: Cell<bool>,
    ro_dirty: Cell<bool>,
    hidden_dirty: Cell<bool>,
    working: Cell<bool>,
    apply: RefCell<Option<gtk::Button>>,
}

impl Props {
    fn mark_dirty(&self) {
        self.dirty.set(true);
        if let Some(b) = self.apply.borrow().as_ref() {
            b.set_sensitive(true);
        }
    }

    fn set_error(&self, msg: &str) {
        self.error.set_label(msg);
        self.error.set_visible(!msg.is_empty());
        if msg.is_empty() {
            self.entry.remove_css_class("xp-props-bad");
        } else {
            self.entry.add_css_class("xp-props-bad");
        }
    }

    fn set_check(&self, c: &gtk::CheckButton, on: bool) {
        self.silent.set(true);
        c.set_active(on);
        self.silent.set(false);
    }
}

/// dialogs.js apply(): validate the name, the attributes first (a leading dot when the host has no hidden
/// attribute), then the rename; `done(true)` when everything was applied.
fn apply(p: &Rc<Props>, done: impl FnOnce(bool) + 'static) {
    let Some(x) = p.x.upgrade() else {
        done(false);
        return;
    };
    if !p.dirty.get() {
        done(true);
        return;
    }
    let base = p.entry.text().trim().to_owned();
    if base.is_empty() {
        p.set_error(s("nameEmpty"));
        done(false);
        return;
    }
    if base.contains('/') {
        p.set_error(s("nameSlash"));
        done(false);
        return;
    }
    p.working.set(true);
    let pw = p.clone();
    let done: Box<dyn FnOnce(bool)> = Box::new(move |ok| {
        pw.working.set(false);
        done(ok);
    });
    if !(p.ro_dirty.get() || p.hidden_dirty.get()) {
        apply_name(&x, p, base, done);
        return;
    }
    let mut args = json!({ "path": p.cur_path.borrow().clone() });
    if p.ro_dirty.get() {
        args["readOnly"] = json!(p.read_only.is_active());
    }
    if p.hidden_dirty.get() && !p.name.starts_with('.') {
        args["hidden"] = json!(p.hidden.is_active());
    }
    let pp = p.clone();
    ops::set_attributes(&x, args, p.hidden_dirty.get(), move |x, r| {
        let mut base = base;
        match r {
            Ok(Attr::RenameHidden) => {
                if pp.hidden.is_active() && !base.starts_with('.') {
                    base = format!(".{base}");
                }
            }
            Ok(Attr::Done) => {}
            Err(m) => {
                pp.set_error(&m);
                done(false);
                return;
            }
        }
        if pp.hidden_dirty.get() && !pp.hidden.is_active() && pp.name.starts_with('.') && base.starts_with('.') {
            base = base.trim_start_matches('.').to_owned();
        }
        pp.ro_dirty.set(false);
        pp.hidden_dirty.set(false);
        apply_name(x, &pp, base, done);
    });
}

fn apply_name(x: &Explorer, p: &Rc<Props>, base: String, done: Box<dyn FnOnce(bool)>) {
    let cur = p.cur_path.borrow().clone();
    let cur_name = util::basename(&cur);
    let finish = |p: &Props| {
        p.set_error("");
        p.dirty.set(false);
    };
    if base == cur_name {
        finish(p);
        done(true);
        return;
    }
    let pp = p.clone();
    ops::rename_path(x, cur.clone(), cur_name, base.clone(), move |_, r| match r {
        Ok(_) => {
            *pp.cur_path.borrow_mut() = util::join_path(&util::dirname(&cur), &base);
            finish(&pp);
            done(true);
        }
        Err(m) => {
            pp.set_error(&m);
            done(false);
        }
    });
}

fn show_single(x: &Explorer, item: Item) {
    let name = name_of(&item);
    let path = st(&item, "path").to_owned();
    let is_trash = x.loc() == Loc::Trash;
    let is_dir = flag(&item, "isDir");
    let general = props_box();
    let details = props_box();
    details.set_visible(false);
    let tabs = Pivot::new(&[("general", s("general"), ""), ("details", s("details"), "")], Some("general"));
    tabs.widget().add_css_class("xp-props-tabs");
    let (g, d) = (general.downgrade(), details.downgrade());
    tabs.connect_selected(move |id| {
        if let (Some(g), Some(d)) = (g.upgrade(), d.upgrade()) {
            g.set_visible(id == "general");
            d.set_visible(id == "details");
        }
    });

    // the head: icon and name box (with the error under it)
    let entry = kit::controls::text_box("");
    entry.set_text(&name);
    entry.set_hexpand(true);
    entry.set_editable(!is_trash);
    entry.set_widget_name("explorer-props-name");
    entry.update_property(&[gtk::accessible::Property::Label(s("colName"))]);
    let error = kit::controls::label("", "xp-props-error", true);
    error.set_visible(false);
    let name_col = gtk::Box::new(gtk::Orientation::Vertical, 4);
    name_col.set_hexpand(true);
    name_col.set_valign(gtk::Align::Center);
    name_col.append(&entry);
    name_col.append(&error);
    let top = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    top.add_css_class("xp-props-head");
    let icon = kit::icons::file_item_icon(&item, 32, true);
    icon.set_valign(gtk::Align::Center);
    top.append(&icon);
    top.append(&name_col);

    let size = item.get("size").and_then(Value::as_f64);
    let calculating = s("calculating");
    let size_val = value_label(&if is_dir { calculating.to_owned() } else { bytes_opt(size) });
    let disk = size.map(|n| if n > 0.0 { (n / 4096.0).ceil() * 4096.0 } else { 0.0 }).unwrap_or(0.0);
    let disk_val = value_label(&if is_dir { calculating.to_owned() } else { util::bytes_full(disk) });
    let contains = value_label(calculating);
    let created = value_label("\u{2026}");
    let modified = value_label(&long_date(num(&item, "modified")));
    let accessed = value_label("\u{2026}");
    let read_only = kit::controls::checkbox(s("readOnly"));
    read_only.set_sensitive(false);
    read_only.set_widget_name("explorer-props-readonly");
    let hidden = kit::controls::checkbox(s("hiddenAttr"));
    hidden.set_active(name.starts_with('.') || flag(&item, "hidden"));
    hidden.set_sensitive(!is_trash);
    hidden.set_widget_name("explorer-props-hidden");
    let opens = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    opens.add_css_class("xp-prop-app");
    opens.set_hexpand(true);
    opens.append(&value_label(DASH));
    let (_, ext) = util::split_ext(&name);
    let type_text = if !ext.is_empty() && !is_dir { format!("{} ({ext})", util::type_name(&item)) } else { util::type_name(&item) };

    general.append(&top);
    general.append(&divider());
    general.append(&row_text(s("type"), &type_text));
    if !is_dir {
        general.append(&row(s("opensWith"), &opens));
    }
    general.append(&divider());
    general.append(&row_text(s("location"), &if is_trash { Explorer::orig_dir(&item) } else { util::dirname(&path) }));
    general.append(&row(s("size"), &size_val));
    if !is_dir {
        general.append(&row(s("sizeOnDisk"), &disk_val));
    } else {
        general.append(&row(s("contains"), &contains));
    }
    general.append(&divider());
    general.append(&row(s("created"), &created));
    general.append(&row(s("modified"), &modified));
    general.append(&row(s("accessed"), &accessed));
    general.append(&divider());
    let checks = gtk::Box::new(gtk::Orientation::Horizontal, 20);
    checks.add_css_class("xp-prop-checks");
    checks.append(&read_only);
    checks.append(&hidden);
    general.append(&row(s("attributes"), &checks));

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.add_css_class("xp-propsdlg");
    content.append(tabs.widget());
    content.append(&general);
    content.append(&details);

    let props = Rc::new(Props {
        x: x.weak(),
        name: name.clone(),
        cur_path: RefCell::new(path.clone()),
        entry: entry.clone(),
        error,
        read_only: read_only.clone(),
        hidden: hidden.clone(),
        silent: Cell::new(false),
        dirty: Cell::new(false),
        ro_dirty: Cell::new(false),
        hidden_dirty: Cell::new(false),
        working: Cell::new(false),
        apply: RefCell::new(None),
    });
    let weak: Weak<Props> = Rc::downgrade(&props);
    entry.connect_changed(move |_| {
        if let Some(p) = weak.upgrade() {
            p.mark_dirty();
        }
    });
    let weak: Weak<Props> = Rc::downgrade(&props);
    read_only.connect_toggled(move |_| {
        if let Some(p) = weak.upgrade()
            && !p.silent.get()
        {
            p.ro_dirty.set(true);
            p.mark_dirty();
        }
    });
    let weak: Weak<Props> = Rc::downgrade(&props);
    hidden.connect_toggled(move |_| {
        if let Some(p) = weak.upgrade()
            && !p.silent.get()
        {
            p.hidden_dirty.set(true);
            p.mark_dirty();
        }
    });

    let pp = props.clone();
    let dlg = Dialog::new(&f("propsTitle", &[("name", &name)]))
        .body(&content)
        .primary(s("ok"))
        .close_label(s("cancel"))
        .width(460)
        .on_primary(move |d| {
            if !pp.dirty.get() {
                return true;
            }
            if pp.working.get() {
                return false;
            }
            let d = d.clone();
            d.set_busy(true);
            apply(&pp, move |ok| {
                d.set_busy(false);
                if ok {
                    d.close(DialogResult::Primary);
                }
            });
            false
        });
    let keep = props.clone();
    dlg.show(&x.anchor(), move |_| drop(keep));
    // Windows order: OK · Annuleren · Toepassen; Toepassen applies and keeps the dialog open
    if !is_trash
        && let Some(footer) = content.parent().and_then(|b| b.parent()).and_then(|d| d.last_child()).and_downcast::<gtk::Box>()
    {
        let b = kit::controls::button(s("apply"), Variant::Standard);
        b.set_hexpand(true);
        b.set_sensitive(false);
        b.set_widget_name("explorer-props-apply");
        let weak: Weak<Props> = Rc::downgrade(&props);
        b.connect_clicked(move |b| {
            let Some(p) = weak.upgrade() else { return };
            if p.working.get() {
                return;
            }
            let bw = b.downgrade();
            apply(&p, move |ok| {
                if ok && let Some(b) = bw.upgrade() {
                    b.set_sensitive(false);
                }
            });
        });
        footer.append(&b);
        *props.apply.borrow_mut() = Some(b);
    }

    // what the host knows
    let pp = props.clone();
    let user = user_name(x);
    let item2 = item.clone();
    x.call("files.properties", json!({ "path": path }), move |_, r| match r {
        Ok(pr) if pr.is_object() => {
            if let Some(n) = pr["size"].as_f64() {
                size_val.set_label(&util::bytes_full(n));
                disk_val.set_label(&util::bytes_full((n / 4096.0).ceil() * 4096.0));
            }
            if pr["contains"].is_object() {
                let c = &pr["contains"];
                contains.set_label(&contains_val(&format::number(num(c, "files"), 0), &format::number(num(c, "folders"), 0)));
            } else {
                contains.set_label(DASH);
            }
            created.set_label(&long_date(num(&pr, "created")));
            let m = num(&pr, "modified");
            modified.set_label(&long_date(if m > 0.0 { m } else { num(&item2, "modified") }));
            accessed.set_label(&long_date(num(&pr, "accessed")));
            pp.set_check(&pp.read_only, pr["readOnly"] == true);
            let owner = st(&pr, "owner");
            if !is_trash && !owner.is_empty() && !user.is_empty() && owner == user {
                pp.read_only.set_sensitive(true);
            }
            if !pr["hidden"].is_null() {
                pp.set_check(&pp.hidden, pr["hidden"] == true);
            }
            let dn = [st(&pr, "displayName"), st(&pr, "name")].into_iter().find(|t| !t.is_empty()).map(str::to_owned).unwrap_or_else(|| name_of(&item2));
            let ty = match st(&pr, "type") {
                "" => util::type_name(&item2),
                t => t.to_owned(),
            };
            let mime = [st(&pr, "mime"), st(&item2, "mime")].into_iter().find(|t| !t.is_empty()).unwrap_or(DASH).to_owned();
            let full = match st(&pr, "path") {
                "" => st(&item2, "path").to_owned(),
                p => p.to_owned(),
            };
            for r in [
                row_text(s("colName"), &dn),
                row_text(s("type"), &ty),
                row_text(s("mime"), &mime),
                row_text(s("fullPath"), &full),
                row_text(s("size"), &bytes_opt(pr["size"].as_f64())),
                row_text(s("owner"), owner),
                row_text(s("permissions"), st(&pr, "permissions")),
                row_text(s("created"), &long_date(num(&pr, "created"))),
                row_text(s("modified"), &long_date(num(&pr, "modified"))),
                row_text(s("accessed"), &long_date(num(&pr, "accessed"))),
            ] {
                details.append(&r);
            }
        }
        other => {
            if is_dir {
                size_val.set_label(DASH);
            }
            contains.set_label(DASH);
            created.set_label(DASH);
            accessed.set_label(DASH);
            details.append(&row_text(s("fullPath"), st(&item2, "path")));
            if let Err(e) = other {
                eprintln!("win11-gtk-explorer: properties failed: {e}");
            }
        }
    });
    if !is_dir && !is_trash {
        x.call("apps.forFile", json!({ "path": st(&item, "path") }), move |_, r| {
            let apps = r.ok().and_then(|v| v.as_array().cloned()).unwrap_or_default();
            let def = apps.iter().find(|a| a["isDefault"] == true).or_else(|| apps.first());
            if let Some(a) = def {
                kit::controls::clear(&opens);
                opens.append(&kit::icons::app_icon(st(a, "icon"), 16));
                let l = kit::controls::label(st(a, "name"), "xp-prop-value", false);
                l.set_hexpand(true);
                opens.append(&l);
            }
        });
    }
}

fn show_multi(x: &Explorer, items: Vec<Item>) {
    let types: BTreeSet<String> = items.iter().map(|i| util::type_name(i)).collect();
    let dirs: BTreeSet<String> = items.iter().map(|i| util::dirname(st(i, "path"))).collect();
    let files = items.iter().filter(|i| !flag(i, "isDir")).count();
    let folders = items.len() - files;
    let file_total: f64 = items.iter().filter(|i| !flag(i, "isDir")).map(|i| num(i, "size")).sum();
    let size_val = value_label(&if folders > 0 { s("calculating").to_owned() } else { util::bytes_full(file_total) });
    let body = props_box();
    let icon = kit::icons::file_item_icon(&items[0], 32, false);
    body.append(&head(&icon, &contains_val(&format::number(files as f64, 0), &format::number(folders as f64, 0))));
    body.append(&divider());
    let type_text = match types.iter().next() {
        Some(t) if types.len() == 1 => f("allOfType", &[("type", t)]),
        _ => s("multiTypes").to_owned(),
    };
    body.append(&row_text(s("type"), &type_text));
    let loc_text = match dirs.iter().next() {
        Some(d) if dirs.len() == 1 => f("allIn", &[("dir", d)]),
        _ => s("variousFolders").to_owned(),
    };
    body.append(&row_text(s("location"), &loc_text));
    body.append(&row(s("size"), &size_val));
    let title = f("propsTitle", &[("name", &n("multiName", items.len(), &[]))]);
    Dialog::new(&title).body(&dialog_root(&body)).close_label(s("ok")).width(460).show(&x.anchor(), |_| {});
    if folders == 0 {
        return;
    }
    // the host can total a whole selection in one call; otherwise ask per folder (at most 50)
    let paths: Vec<String> = items.iter().map(|i| st(i, "path").to_owned()).collect();
    let dir_paths: Vec<String> = items.iter().filter(|i| flag(i, "isDir")).take(50).map(|i| st(i, "path").to_owned()).collect();
    x.call("files.properties", json!({ "paths": paths }), move |x, r| {
        if let Ok(m) = &r
            && let Some(total) = m["size"].as_f64()
            && num(m, "count") > 0.0
        {
            size_val.set_label(&util::bytes_full(total));
            return;
        }
        let left = Rc::new(Cell::new(dir_paths.len()));
        let sum = Rc::new(Cell::new(file_total));
        for p in dir_paths {
            let (left, sum, label) = (left.clone(), sum.clone(), size_val.clone());
            x.call("files.properties", json!({ "path": p }), move |_, r| {
                if let Ok(v) = r {
                    sum.set(sum.get() + num(&v, "size"));
                }
                left.set(left.get().saturating_sub(1));
                if left.get() == 0 {
                    label.set_label(&util::bytes_full(sum.get()));
                }
            });
        }
    });
}

// ============================================================================================ drives

/// A 12 px colour square of a token (`.xp-swatch`, radius 2), drawn: an empty box paints no CSS background.
fn swatch(token: &'static str) -> gtk::DrawingArea {
    let a = gtk::DrawingArea::new();
    a.add_css_class("xp-swatch");
    a.set_content_width(12);
    a.set_content_height(12);
    a.set_valign(gtk::Align::Center);
    a.set_draw_func(move |_, cr, w, h| {
        let (w, h, r) = (w as f64, h as f64, 2.0);
        let c = kit::theme::token(token);
        cr.new_sub_path();
        cr.arc(w - r, r, r, -std::f64::consts::FRAC_PI_2, 0.0);
        cr.arc(w - r, h - r, r, 0.0, std::f64::consts::FRAC_PI_2);
        cr.arc(r, h - r, r, std::f64::consts::FRAC_PI_2, std::f64::consts::PI);
        cr.arc(r, r, r, std::f64::consts::PI, 1.5 * std::f64::consts::PI);
        cr.close_path();
        cr.set_source_rgba(c.red() as f64, c.green() as f64, c.blue() as f64, c.alpha() as f64);
        let _ = cr.fill();
    });
    kit::theme::track(&a);
    a
}

fn swatch_label(token: &'static str, text: &str) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    b.append(&swatch(token));
    b.append(&kit::controls::label(text, "xp-prop-label", false));
    b
}

/// Eigenschappen of a drive (a storage.volumes row): type, file system, mount point, used and free space,
/// capacity, the donut (red from 90 %) and Ruimte vrijmaken (Settings > Opslag).
pub fn show_drive_properties(x: &Explorer, v: &Value) {
    let pct = util::volume_percent(v);
    let critical = pct >= 90.0;
    let label = util::volume_label(v);
    let body = props_box();
    body.append(&head(&kit::icons::file_icon(util::volume_icon(v), 32), &label));
    body.append(&divider());
    body.append(&row_text(s("type"), s(if flag(v, "removable") { "removable" } else { "localDisk" })));
    let fs = match st(v, "fsType") {
        "" => DASH.to_owned(),
        t => t.to_uppercase(),
    };
    body.append(&row_text(s("fsType"), &fs));
    body.append(&row_text(s("mount"), st(v, "mount")));
    body.append(&divider());
    let used_token = if critical { "sys-critical" } else { "accent" };
    body.append(&row_w(&swatch_label(used_token, s("used")), &value_label(&util::bytes_full(num(v, "used")))));
    body.append(&row_w(&swatch_label("chart-track", s("free")), &value_label(&util::bytes_full(num(v, "free")))));
    body.append(&divider());
    body.append(&row_text(s("capacity"), &util::bytes_full(num(v, "total"))));
    let donut = kit::chart::Donut::new(132, 18.0);
    donut.update(
        vec![
            kit::chart::Segment::new(num(v, "used"), used_token, s("used")),
            kit::chart::Segment::new(num(v, "free"), "chart-track", s("free")),
        ],
        None,
        &format!("{}%", pct.round() as i64),
        &s("used").to_lowercase(),
    );
    donut.widget().set_halign(gtk::Align::Start);
    let foot = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    foot.add_css_class("xp-props-donut");
    foot.append(donut.widget());
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    foot.append(&spacer);
    let free_up = kit::controls::button_with_glyph("broom", s("freeUp"), Variant::Standard);
    free_up.set_valign(gtk::Align::Center);
    free_up.set_widget_name("explorer-free-up");
    foot.append(&free_up);
    body.append(&foot);
    let dlg = Dialog::new(&f("driveTitle", &[("name", &label)])).body(&dialog_root(&body)).close_label(s("ok")).width(460);
    // the button closes the dialog: hold it only while it is open
    let slot: Rc<RefCell<Option<Dialog>>> = Rc::new(RefCell::new(Some(dlg.clone())));
    let w = x.weak();
    let s2 = slot.clone();
    free_up.connect_clicked(move |_| {
        let d = s2.borrow_mut().take();
        if let Some(d) = d {
            d.close(DialogResult::Close);
        }
        if let Some(x) = w.upgrade() {
            x.call("shell.openApp", json!({ "app": "settings", "params": { "page": "storage" } }), |_, _| {});
        }
    });
    dlg.show(&x.anchor(), move |_| drop(slot.borrow_mut().take()));
}

/// Eigenschappen van Prullenbak (files.trashInfo: the number of items and their size).
pub fn show_trash_properties(x: &Explorer) {
    x.call("files.trashInfo", json!({}), |x, r| {
        let info = r.ok().filter(Value::is_object);
        let count = info.as_ref().map(|i| num(i, "count").max(0.0) as usize);
        let body = props_box();
        let icon = kit::icons::file_icon(if count.unwrap_or(0) > 0 { "user-trash-full" } else { "user-trash" }, 32);
        body.append(&head(&icon, s("trash")));
        body.append(&divider());
        body.append(&row_text(s("trashSize"), &count.map(|c| n("items", c, &[])).unwrap_or_else(|| DASH.into())));
        body.append(&row_text(s("size"), &info.as_ref().map(|i| util::bytes_full(num(i, "size"))).unwrap_or_else(|| DASH.into())));
        Dialog::new(s("trashTitle")).body(&dialog_root(&body)).close_label(s("ok")).width(420).show(&x.anchor(), |_| {});
    });
}

// ============================================================================================ share

/// Delen: the message, the first six items, Pad kopiëren.
pub fn show_share(x: &Explorer, items: Vec<Item>) {
    if items.is_empty() {
        return;
    }
    let paths: Vec<String> = items.iter().map(|i| st(i, "path").to_owned()).collect();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.add_css_class("xp-share");
    let msg = kit::controls::label(s("shareMsg"), "w11-text-2", true);
    msg.set_max_width_chars(56);
    content.append(&msg);
    let list = gtk::Box::new(gtk::Orientation::Vertical, 4);
    list.add_css_class("xp-share-list");
    for it in items.iter().take(6) {
        let r = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        r.add_css_class("xp-share-item");
        r.append(&kit::icons::file_item_icon(it, 20, false));
        let l = kit::controls::label(&name_of(it), "", false);
        l.set_hexpand(true);
        r.append(&l);
        list.append(&r);
    }
    content.append(&list);
    let w = x.weak();
    Dialog::new(s("shareTitle")).body(&content).primary(s("copyPathBtn")).close_label(s("close")).width(440).show(&x.anchor(), move |r| {
        let Some(x) = w.upgrade() else { return };
        if r != DialogResult::Primary {
            return;
        }
        let text = paths.iter().map(|p| format!("\"{p}\"")).collect::<Vec<_>>().join("\n");
        if util::copy_text(&x.anchor(), &text) {
            x.toast(&n("pathCopied", paths.len(), &[]), ToastOpts::default());
        }
    });
}

// ============================================================================================ options

/// Mapopties: the view options for all folders and Recent on Start (privacy.recentFiles).
pub fn show_options(x: &Explorer) {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 10);
    content.add_css_class("xp-options");
    content.append(&kit::controls::label(s("optHint"), "w11-text-2", true));
    for (label, key, def) in [
        ("hidden", "showHidden", false),
        ("extensions", "showExtensions", true),
        ("checkboxes", "checkboxes", false),
        ("compact", "compact", false),
        ("navPane", "navPane", true),
        ("detailsPane", "detailsPane", false),
    ] {
        let c = kit::controls::checkbox(s(label));
        c.set_active(x.cfg_bool(key, def));
        c.set_widget_name(&format!("explorer-opt-{key}"));
        let w = x.weak();
        c.connect_toggled(move |c| {
            if let Some(x) = w.upgrade() {
                x.set_cfg(key, json!(c.is_active()));
            }
        });
        content.append(&c);
    }
    let recent = kit::controls::checkbox(s("optRecent"));
    recent.set_active(x.privacy_recent());
    recent.set_widget_name("explorer-opt-recent");
    let w = x.weak();
    recent.connect_toggled(move |c| {
        if let Some(x) = w.upgrade() {
            x.set_setting("privacy.recentFiles", json!(c.is_active()));
        }
    });
    content.append(&recent);
    Dialog::new(s("optionsTitle")).body(&content).close_label(s("close")).width(420).show(&x.anchor(), |_| {});
}
