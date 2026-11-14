//! "Schijven en volumes" (last-webkit:vendor/ui/js/apps/disks/overview.js): a card per disk with the Schijfbeheer bar
//! (partitions and unallocated space side by side, proportional; click selects, double-click or Enter opens,
//! right-click, the Menu key or Shift+F10 for the actions; an MBR extended partition is a frame around its
//! logical drives and the free space in it; one tab stop per bar, arrows move) and the volume list below it.
use crate::actions;
use crate::app::{App, MarkKind};
use crate::art;
use crate::model::{self, Item, arr, bytes, num, seg_id, seg_kind, st, truthy};
use crate::strings::{s, tf, tp};
use crate::widgets::{FlexBar, Segment, hbox, label, usage_bar, vbox};
use kit::controls::{self, MenuItem, Variant};
use kit::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

/// The last part of an object path (widget names for the dev channel).
pub fn short(id: &str) -> String {
    id.rsplit('/').next().unwrap_or(id).to_owned()
}

/// Whether (x, y) in `w` is on a button inside it (a row's own click must leave those alone: stopPropagation in
/// the HTML, `e.target.closest('button')`).
fn on_inner_button(w: &gtk::Widget, x: f64, y: f64) -> bool {
    let Some(mut p) = w.pick(x, y, gtk::PickFlags::DEFAULT) else { return false };
    loop {
        if p == *w {
            return false;
        }
        if p.is::<gtk::Button>() {
            return true;
        }
        match p.parent() {
            Some(q) => p = q,
            None => return false,
        }
    }
}

/// Right-click on `w` (not on a button in it): `f(x, y)` with the pointer position.
pub fn on_right_click(w: &impl IsA<gtk::Widget>, f: impl Fn(f64, f64) + 'static) {
    let g = gtk::GestureClick::new();
    g.set_button(gtk::gdk::BUTTON_SECONDARY);
    g.connect_pressed(move |g, _, x, y| {
        if g.widget().is_some_and(|w| on_inner_button(&w, x, y)) {
            return;
        }
        g.set_state(gtk::EventSequenceState::Claimed);
        f(x, y);
    });
    w.add_controller(g);
}

/// A primary click on `w` (not on a button in it): `f(n_press)` (1, 2 for a double click).
pub fn on_click(w: &impl IsA<gtk::Widget>, f: impl Fn(i32) + 'static) {
    let g = gtk::GestureClick::new();
    g.set_button(gtk::gdk::BUTTON_PRIMARY);
    g.connect_pressed(move |g, n, x, y| {
        if g.widget().is_some_and(|w| on_inner_button(&w, x, y)) {
            return;
        }
        g.set_state(gtk::EventSequenceState::Claimed);
        f(n);
    });
    w.add_controller(g);
}

/// A menu opened from the keyboard: below the widget's top-left corner (contextmenu.js anchor).
pub fn menu_below(app: &App, w: &impl IsA<gtk::Widget>, items: Vec<MenuItem>) {
    let h = w.as_ref().height() as f64;
    app.menu_at(w, 8.0, h, items);
}

fn is_menu_key(key: gtk::gdk::Key, state: gtk::gdk::ModifierType) -> bool {
    key == gtk::gdk::Key::Menu || (key == gtk::gdk::Key::F10 && state.contains(gtk::gdk::ModifierType::SHIFT_MASK))
}

fn health_chip(d: &Value) -> Option<gtk::Widget> {
    let h = model::health_of(d);
    if h == "unknown" && !truthy(&d["smart"]["supported"]) {
        return None;
    }
    let chip = hbox(6, &format!("dk-health is-{h}"));
    let dot = hbox(0, "dk-health-dot");
    dot.set_valign(gtk::Align::Center);
    dot.set_halign(gtk::Align::Center);
    chip.append(&dot);
    chip.append(&label(&s(&format!("h_{h}")), "", false));
    Some(chip.upcast())
}

/// overview.js grow: the share of the whole, at least 7 % (2.5 % for an MSR partition).
fn grow(it: &Item, whole: f64) -> f64 {
    let share = if whole > 0.0 { it.size / whole } else { 0.0 };
    let small = it.is_volume() && it.v()["partTypeKey"] == "msr";
    share.max(if small { 0.025 } else { 0.07 })
}

type BarSegs = Rc<RefCell<Vec<(String, glib::WeakRef<Segment>)>>>;

/// The Schijfbeheer bar for one disk.
pub fn disk_bar(app: &Rc<App>, d: &Value, compact: bool) -> gtk::Widget {
    let items = model::layout(d);
    let bar = FlexBar::new(3);
    bar.add_css_class("dk-bar");
    if compact {
        bar.add_css_class("dk-compact");
    }
    bar.update_property(&[gtk::accessible::Property::Label(&format!("{}: {}", st(&d["name"]), st(&d["model"])))]);
    if items.is_empty() {
        let name = if d["media"] == false { "\u{2013}".to_owned() } else { s("unallocated") };
        let n = label(&name, "dk-seg-name-text", false);
        let row = hbox(4, "dk-seg-name");
        row.append(&n);
        let seg = Segment::new(vec![row.upcast()], None);
        seg.add_css_class("dk-seg");
        seg.add_css_class("dk-seg--empty");
        bar.add(&seg, 1.0);
        return bar.upcast();
    }
    let bar_id = app.new_bar();
    let ids: Vec<String> = model::flat_layout(d).iter().filter(|it| !it.is_container()).map(|it| seg_id(d, it)).collect();
    let selected = app.selected();
    let tab_stop = match &selected {
        Some(sel) if ids.contains(sel) => sel.clone(),
        _ => ids.first().cloned().unwrap_or_default(),
    };
    let segs: BarSegs = Rc::new(RefCell::new(Vec::new()));
    for it in &items {
        if it.is_container() {
            bar.add(&group(app, d, it, bar_id, &tab_stop, &segs), grow(it, num(&d["size"])));
        } else {
            bar.add(&segment(app, d, it, bar_id, &tab_stop, &segs), grow(it, num(&d["size"])));
        }
    }
    bar.upcast()
}

/// An MBR extended partition: a frame around its logical drives and the free space in it.
fn group(app: &Rc<App>, d: &Value, it: &Item, bar_id: u32, tab_stop: &str, segs: &BarSegs) -> gtk::Widget {
    let c = it.v().clone();
    let cid = st(&c["id"]);
    let g = vbox(2, "dk-seg-group");
    if app.selected().as_deref() == Some(cid.as_str()) {
        g.add_css_class("dk-selected");
    }
    g.set_widget_name(&format!("dk-group-{}", short(&cid)));
    g.set_tooltip_text(Some(&format!("{}\n{}", st(&c["name"]), bytes(&c["size"]))));
    let head = hbox(6, "dk-seg-group-label");
    head.append(&label(&st(&c["name"]), "", false));
    head.append(&label(&bytes(&c["size"]), "dk-seg-group-size", false));
    g.append(&head);
    let inner = FlexBar::new(3);
    inner.add_css_class("dk-seg-group-inner");
    inner.set_vexpand(true);
    if it.children.is_empty() {
        let seg = Segment::new(vec![], None);
        seg.add_css_class("dk-seg");
        seg.add_css_class("dk-seg--free");
        seg.add_css_class("dk-in-extended");
        inner.add(&seg, 1.0);
    } else {
        for x in &it.children {
            inner.add(&segment(app, d, x, bar_id, tab_stop, segs), grow(x, num(&c["size"])));
        }
    }
    g.append(&inner);
    app.mark(&cid, MarkKind::Group, &g);
    let (w, dd, cc) = (Rc::downgrade(app), d.clone(), c.clone());
    on_click(&g, move |n| {
        let Some(app) = w.upgrade() else { return };
        if n >= 2 {
            app.open_volume(&st(&cc["id"]));
        } else {
            app.select(&st(&cc["id"]), false);
        }
    });
    let (w, gw) = (Rc::downgrade(app), g.downgrade());
    on_right_click(&g, move |x, y| {
        let (Some(app), Some(g)) = (w.upgrade(), gw.upgrade()) else { return };
        app.select(&st(&c["id"]), false);
        app.menu_at(&g, x, y, actions::volume_menu(&app, &dd, &c));
    });
    g.upcast()
}

fn segment(app: &Rc<App>, d: &Value, it: &Item, bar_id: u32, tab_stop: &str, segs: &BarSegs) -> gtk::Widget {
    let id = seg_id(d, it);
    let v = it.v().clone();
    let f = it.f().clone();
    let free = !it.is_volume();
    let in_ext = free && truthy(&f["logical"]);
    let kind = if free { "free" } else { seg_kind(&v) };
    let name = if free {
        if truthy(&d["partitionTable"]) {
            if in_ext { s("freeSpace") } else { s("unallocated") }
        } else {
            s("noTable")
        }
    } else {
        model::title_with_mount(&v)
    };
    let line2 = if free {
        model::bytes(&f["size"])
    } else {
        [bytes(&v["size"]), st(&v["fsName"])].iter().filter(|x| !x.is_empty()).cloned().collect::<Vec<_>>().join(" ")
    };
    let line3 = if free {
        String::new()
    } else if truthy(&v["busy"]) {
        model::busy_text(&v["busy"])
    } else {
        model::volume_status(&v)
    };
    let name_row = hbox(4, "dk-seg-name");
    if !free && truthy(&v["encrypted"]) {
        let g = kit::icons::glyph(if truthy(&v["encrypted"]["unlocked"]) { "lock-open" } else { "lock" }, 12);
        g.add_css_class("dk-seg-lock");
        name_row.append(&g);
    }
    let nl = label(&name, "dk-seg-name-text", false);
    nl.set_hexpand(true);
    name_row.append(&nl);
    let mut parts: Vec<gtk::Widget> = vec![name_row.upcast(), label(&line2, "dk-seg-line", false).upcast()];
    if !line3.is_empty() {
        parts.push(label(&line3, "dk-seg-line dk-seg-status", false).upcast());
    }
    let progress = if free { None } else { v["busy"]["progress"].as_f64() };
    let seg = Segment::new(parts, progress);
    seg.add_css_class("dk-seg");
    seg.add_css_class(&format!("dk-seg--{kind}"));
    if in_ext {
        seg.add_css_class("dk-in-extended");
    }
    if app.selected().as_deref() == Some(id.as_str()) {
        seg.add_css_class("dk-selected");
    }
    seg.set_widget_name(&format!("dk-seg-{}", short(&id)));
    seg.set_tooltip_text(Some(&[name.as_str(), line2.as_str(), line3.as_str()].iter().filter(|x| !x.is_empty()).cloned().collect::<Vec<_>>().join("\n")));
    seg.set_focusable(id == tab_stop);
    segs.borrow_mut().push((id.clone(), seg.downgrade()));
    app.mark(&id, MarkKind::Seg(bar_id), &seg);

    // what the segment does: open (a volume's page, or a free region's first action) and its menu
    let menu: Rc<dyn Fn(&Rc<App>) -> Vec<MenuItem>> = {
        let (d, v, f) = (d.clone(), v.clone(), f.clone());
        Rc::new(move |app: &Rc<App>| if free { actions::free_menu(app, &d, &f) } else { actions::volume_menu(app, &d, &v) })
    };
    let open: Rc<dyn Fn(&Rc<App>, &Segment, Option<(f64, f64)>)> = {
        let (d, v, f) = (d.clone(), v.clone(), f.clone());
        Rc::new(move |app: &Rc<App>, seg: &Segment, at: Option<(f64, f64)>| {
            if free {
                actions::free_default(app, &d, &f, seg.upcast_ref(), at);
            } else {
                app.open_volume(&st(&v["id"]));
            }
        })
    };
    let (w, sw, idc, op) = (Rc::downgrade(app), seg.downgrade(), id.clone(), open.clone());
    let click = gtk::GestureClick::new();
    click.set_button(gtk::gdk::BUTTON_PRIMARY);
    click.connect_pressed(move |g, n, x, y| {
        g.set_state(gtk::EventSequenceState::Claimed);
        let (Some(app), Some(seg)) = (w.upgrade(), sw.upgrade()) else { return };
        app.select(&idc, false);
        seg.set_focusable(true);
        seg.grab_focus();
        if n >= 2 {
            op(&app, &seg, Some((x, y)));
        }
    });
    seg.add_controller(click);
    let (w, sw, idc, m) = (Rc::downgrade(app), seg.downgrade(), id.clone(), menu.clone());
    on_right_click(&seg, move |x, y| {
        let (Some(app), Some(seg)) = (w.upgrade(), sw.upgrade()) else { return };
        app.select(&idc, false);
        app.menu_at(&seg, x, y, m(&app));
    });
    let keys = gtk::EventControllerKey::new();
    let (w, sw, idc, list) = (Rc::downgrade(app), seg.downgrade(), id.clone(), segs.clone());
    keys.connect_key_pressed(move |_, key, _, state| {
        use gtk::gdk::Key;
        let (Some(app), Some(seg)) = (w.upgrade(), sw.upgrade()) else { return glib::Propagation::Proceed };
        match key {
            Key::Right | Key::Left | Key::Home | Key::End | Key::KP_Right | Key::KP_Left => {
                let ids: Vec<String> = list.borrow().iter().map(|(i, _)| i.clone()).collect();
                let k = ids.iter().position(|i| *i == idc).unwrap_or(0) as i64;
                let last = ids.len() as i64 - 1;
                let to = match key {
                    Key::Home => 0,
                    Key::End => last,
                    Key::Right | Key::KP_Right => k + 1,
                    _ => k - 1,
                };
                if let Some(next) = ids.get(to.clamp(0, last.max(0)) as usize) {
                    app.select(next, true);
                }
                glib::Propagation::Stop
            }
            Key::Return | Key::KP_Enter | Key::ISO_Enter => {
                open(&app, &seg, None);
                glib::Propagation::Stop
            }
            k if is_menu_key(k, state) => {
                menu_below(&app, &seg, menu(&app));
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        }
    });
    seg.add_controller(keys);
    let (w, idc) = (Rc::downgrade(app), id);
    seg.connect_activate(move || {
        if let Some(app) = w.upgrade() {
            app.select(&idc, false);
        }
    });
    seg.upcast()
}

/// The glyph of a volume row.
fn volume_icon(v: &Value, d: &Value) -> gtk::Image {
    let enc = &v["encrypted"];
    let name = if truthy(enc) {
        if truthy(&enc["unlocked"]) { "lock-open" } else { "lock" }
    } else if d["kind"] == "usb" || d["kind"] == "sd" {
        "usb"
    } else {
        "drive"
    };
    kit::icons::glyph(name, 20)
}

/// The shared body of a volume or free-space row: icon, title and description, usage, the button at the end.
fn row_shell(app: &Rc<App>, classes: &str, icon: gtk::Image, icon_class: &str, title: &str, desc: &str, usage: gtk::Widget, end: gtk::Widget) -> gtk::Box {
    let row = hbox(16, &format!("dk-vol {classes}"));
    row.set_focusable(true);
    let ib = hbox(0, &format!("dk-vol-icon {icon_class}"));
    ib.set_size_request(24, -1);
    icon.set_halign(gtk::Align::Center);
    icon.set_hexpand(true);
    ib.append(&icon);
    ib.set_hexpand(false);
    ib.set_valign(gtk::Align::Center);
    row.append(&ib);
    let text = vbox(0, "dk-vol-text");
    text.set_hexpand(true);
    text.set_valign(gtk::Align::Center);
    text.append(&label(title, "dk-vol-title", false));
    text.append(&label(desc, "dk-vol-desc", false));
    row.append(&text);
    if !app.narrow() {
        usage.set_size_request(240, -1);
        usage.set_valign(gtk::Align::Center);
        row.append(&usage);
    }
    let eb = hbox(0, "dk-vol-end");
    eb.set_halign(gtk::Align::End);
    eb.set_valign(gtk::Align::Center);
    if !app.narrow() {
        eb.set_size_request(172, -1);
    }
    end.set_halign(gtk::Align::End);
    end.set_hexpand(true);
    eb.append(&end);
    eb.set_hexpand(false);
    usage.set_hexpand(false);
    row.append(&eb);
    row
}

/// One row of the volume list (Windows 11 Settings style).
pub fn volume_row(app: &Rc<App>, d: &Value, v: &Value) -> gtk::Widget {
    let id = st(&v["id"]);
    let container = truthy(&v["isContainer"]);
    let desc = if container {
        let (o, sz) = (num(&v["offset"]), num(&v["size"]));
        let n = arr(&d["volumes"]).iter().filter(|x| truthy(&x["isContained"]) && num(&x["offset"]) >= o && num(&x["offset"]) < o + sz).count();
        tp("containerDesc", n as f64, &[("n", &n.to_string())])
    } else {
        let fs_name = st(&v["fsName"]);
        let mut parts = Vec::new();
        if fs_name != st(&v["name"]) && !fs_name.is_empty() {
            parts.push(fs_name);
        }
        parts.push(model::volume_status(v));
        if !truthy(&v["mounted"]) && truthy(&v["fsType"]) && !truthy(&v["encrypted"]) && v["role"] == "data" && !truthy(&v["isSwap"]) {
            parts.push(s("notMounted"));
        }
        parts.into_iter().filter(|p| !p.is_empty()).collect::<Vec<_>>().join(" \u{b7} ")
    };
    let desc = if desc.is_empty() { st(&v["partTypeName"]) } else { desc };
    let usage = vbox(6, "dk-vol-usage");
    if truthy(&v["total"]) {
        usage.append(&usage_bar(num(&v["used"]), num(&v["total"]), false));
        usage.append(&label(&tf("freeOf", &[("f", &bytes(&v["free"])), ("t", &bytes(&v["total"]))]), "dk-vol-usage-text", false));
    } else {
        usage.append(&label(&bytes(&v["size"]), "dk-vol-usage-text", false));
    }
    let props = controls::button(&s("properties"), Variant::Standard);
    props.set_widget_name(&format!("dk-props-{}", short(&id)));
    let (w, idc) = (Rc::downgrade(app), id.clone());
    props.connect_clicked(move |_| {
        if let Some(app) = w.upgrade() {
            app.open_volume(&idc);
        }
    });
    let icon = if container { kit::icons::glyph("storage", 20) } else { volume_icon(v, d) };
    let mut classes = String::new();
    if truthy(&v["isContained"]) {
        classes.push_str(" dk-contained");
    }
    if truthy(&v["busy"]) {
        classes.push_str(" dk-busy");
    }
    if app.selected().as_deref() == Some(id.as_str()) {
        classes.push_str(" dk-selected");
    }
    let row = row_shell(app, &classes, icon, &format!("is-{}", seg_kind(v)), &model::title_with_mount(v), &desc, usage.upcast(), props.upcast());
    row.set_widget_name(&format!("dk-vol-{}", short(&id)));
    row.set_tooltip_text(None);
    app.mark(&id, MarkKind::Row, &row);
    let (w, idc, rw) = (Rc::downgrade(app), id.clone(), row.downgrade());
    on_click(&row, move |n| {
        let Some(app) = w.upgrade() else { return };
        if n >= 2 {
            app.open_volume(&idc);
        } else {
            app.select(&idc, false);
            if let Some(r) = rw.upgrade() {
                r.grab_focus();
            }
        }
    });
    let keys = gtk::EventControllerKey::new();
    let (w, rw, idc) = (Rc::downgrade(app), row.downgrade(), id.clone());
    let (dd, vv) = (d.clone(), v.clone());
    keys.connect_key_pressed(move |_, key, _, state| {
        let (Some(app), Some(row)) = (w.upgrade(), rw.upgrade()) else { return glib::Propagation::Proceed };
        if !row.has_focus() {
            return glib::Propagation::Proceed;
        }
        if matches!(key, gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter | gtk::gdk::Key::ISO_Enter) {
            app.open_volume(&idc);
            return glib::Propagation::Stop;
        }
        if is_menu_key(key, state) {
            app.select(&idc, false);
            menu_below(&app, &row, actions::volume_menu(&app, &dd, &vv));
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    row.add_controller(keys);
    let (w, rw, idc, dd, vv) = (Rc::downgrade(app), row.downgrade(), id, d.clone(), v.clone());
    on_right_click(&row, move |x, y| {
        let (Some(app), Some(row)) = (w.upgrade(), rw.upgrade()) else { return };
        app.select(&idc, false);
        app.menu_at(&row, x, y, actions::volume_menu(&app, &dd, &vv));
    });
    row.upcast()
}

pub fn free_row(app: &Rc<App>, d: &Value, f: &Value) -> gtk::Widget {
    let id = model::free_id(d, f);
    let init = !truthy(&d["partitionTable"]);
    let logical = truthy(&f["logical"]);
    let btn = if init {
        let b = controls::button(&format!("{}\u{2026}", s("initialize")), Variant::Standard);
        b.set_sensitive(actions::can_init(d));
        let (w, dd) = (Rc::downgrade(app), d.clone());
        b.connect_clicked(move |_| {
            if let Some(app) = w.upgrade() {
                actions::initialize(&app, &dd);
            }
        });
        b
    } else {
        let b = controls::button_with_glyph("plus", &s("newVolumeDots"), Variant::Standard);
        b.set_sensitive(actions::can_create_in(d, f));
        let (w, dd, ff) = (Rc::downgrade(app), d.clone(), f.clone());
        b.connect_clicked(move |_| {
            if let Some(app) = w.upgrade() {
                actions::new_volume(&app, &dd, &ff);
            }
        });
        b
    };
    btn.set_widget_name(&format!("dk-free-action-{}", short(&id)));
    let title = if init { s("noTable") } else if logical { s("freeSpace") } else { s("unallocated") };
    let size = bytes(&f["size"]);
    let desc = if init { s("initializeDesc") } else { tf(if logical { "freeSpaceDesc" } else { "newVolumeDesc" }, &[("s", &size)]) };
    let usage = vbox(6, "dk-vol-usage");
    usage.append(&label(&size, "dk-vol-usage-text", false));
    let mut classes = " dk-vol--free".to_owned();
    if logical {
        classes.push_str(" dk-contained");
    }
    if app.selected().as_deref() == Some(id.as_str()) {
        classes.push_str(" dk-selected");
    }
    let icon = kit::icons::glyph(if init { "disk" } else { "storage" }, 20);
    let row = row_shell(app, &classes, icon, "is-free", &title, &desc, usage.upcast(), btn.upcast());
    row.set_widget_name(&format!("dk-vol-{}", short(&id)));
    app.mark(&id, MarkKind::Row, &row);
    let (w, idc, rw) = (Rc::downgrade(app), id.clone(), row.downgrade());
    on_click(&row, move |_| {
        if let Some(app) = w.upgrade() {
            app.select(&idc, false);
        }
        if let Some(r) = rw.upgrade() {
            r.grab_focus();
        }
    });
    let keys = gtk::EventControllerKey::new();
    let (w, rw, idc, dd, ff) = (Rc::downgrade(app), row.downgrade(), id.clone(), d.clone(), f.clone());
    keys.connect_key_pressed(move |_, key, _, state| {
        let (Some(app), Some(row)) = (w.upgrade(), rw.upgrade()) else { return glib::Propagation::Proceed };
        if row.has_focus() && is_menu_key(key, state) {
            app.select(&idc, false);
            menu_below(&app, &row, actions::free_menu(&app, &dd, &ff));
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    row.add_controller(keys);
    let (w, rw, idc, dd, ff) = (Rc::downgrade(app), row.downgrade(), id, d.clone(), f.clone());
    on_right_click(&row, move |x, y| {
        let (Some(app), Some(row)) = (w.upgrade(), rw.upgrade()) else { return };
        app.select(&idc, false);
        app.menu_at(&row, x, y, actions::free_menu(&app, &dd, &ff));
    });
    row.upcast()
}

/// Volume and free-space rows of a disk in layout order (MSR partitions hidden, like Windows); `true` marks a
/// free-space row.
pub fn disk_rows(app: &Rc<App>, d: &Value) -> Vec<(gtk::Widget, bool)> {
    let mut rows = Vec::new();
    for it in model::flat_layout(d) {
        if it.is_volume() {
            if it.v()["partTypeKey"] != "msr" {
                rows.push((volume_row(app, d, it.v()), false));
            }
        } else {
            rows.push((free_row(app, d, it.f()), true));
        }
    }
    rows
}

/// A list of rows (`.dk-vols`), with dividers between them.
pub fn rows_box(rows: Vec<gtk::Widget>, standalone: bool) -> gtk::Box {
    let b = vbox(0, if standalone { "dk-vols dk-standalone" } else { "dk-vols" });
    for r in rows {
        b.append(&r);
    }
    b.set_overflow(gtk::Overflow::Hidden);
    b
}

pub fn disk_card(app: &Rc<App>, d: &Value) -> gtk::Widget {
    let id = st(&d["id"]);
    let meta = [model::kind_label(d), bytes(&d["size"]), model::table_label(d), if truthy(&d["dynamic"]) { s("dynamic") } else { String::new() }]
        .into_iter()
        .filter(|x| !x.is_empty())
        .collect::<Vec<_>>()
        .join(" \u{b7} ");
    let busy = model::disk_busy(d);
    let actions_box = hbox(8, "dk-disk-actions");
    actions_box.set_valign(gtk::Align::Center);
    if truthy(&d["removable"]) && truthy(&d["canEject"]) {
        let b = controls::button_with_glyph("usb", &s("safeRemove"), Variant::Standard);
        b.set_sensitive(!busy);
        if busy {
            b.set_tooltip_text(Some(&s("busyDisk")));
        }
        b.set_widget_name(&format!("dk-saferemove-{}", short(&id)));
        let (w, dd) = (Rc::downgrade(app), d.clone());
        b.connect_clicked(move |_| {
            if let Some(app) = w.upgrade() {
                actions::safe_remove(&app, &dd);
            }
        });
        actions_box.append(&b);
    }
    let props = controls::button(&s("properties"), Variant::Standard);
    props.set_widget_name(&format!("dk-disk-props-{}", short(&id)));
    let (w, idc) = (Rc::downgrade(app), id.clone());
    props.connect_clicked(move |_| {
        if let Some(app) = w.upgrade() {
            app.open_disk(&idc);
        }
    });
    actions_box.append(&props);
    let more = controls::icon_button("more", &s("more"), 32);
    more.set_widget_name(&format!("dk-disk-more-{}", short(&id)));
    let (w, dd) = (Rc::downgrade(app), d.clone());
    more.connect_clicked(move |b| {
        if let Some(app) = w.upgrade() {
            app.menu_for(b, actions::disk_menu(&app, &dd));
        }
    });
    actions_box.append(&more);

    let art_box = hbox(0, "dk-disk-art");
    art_box.set_size_request(44, 44);
    art_box.set_valign(gtk::Align::Center);
    let a = art::drive_art(&st(&d["kind"]), 40, truthy(&d["isSystem"]));
    a.set_hexpand(true);
    art_box.append(&a);
    art_box.set_hexpand(false);
    let text = vbox(2, "dk-disk-text");
    text.set_hexpand(true);
    text.set_valign(gtk::Align::Center);
    let title = hbox(10, "dk-disk-title");
    let name = label(&st(&d["name"]), "dk-disk-name", false);
    name.set_ellipsize(gtk::pango::EllipsizeMode::None);
    name.set_valign(gtk::Align::Baseline);
    let model_l = label(&st(&d["model"]), "dk-disk-model", false);
    model_l.set_valign(gtk::Align::Baseline);
    model_l.set_tooltip_text(Some(&st(&d["model"])));
    title.append(&name);
    title.append(&model_l);
    text.append(&title);
    let meta_row = hbox(12, "dk-disk-meta");
    meta_row.append(&label(&meta, "", false));
    if let Some(chip) = health_chip(d) {
        meta_row.append(&chip);
    }
    if truthy(&d["busy"]) {
        meta_row.append(&label(&model::busy_text(&d["busy"]), "dk-busy-text", false));
    }
    text.append(&meta_row);

    let head: gtk::Box = if app.narrow() {
        // @media (max-width: 760px): the buttons wrap to a row of their own, at the end
        let h = vbox(12, "dk-disk-head");
        let top = hbox(16, "");
        top.append(&art_box);
        top.append(&text);
        h.append(&top);
        actions_box.set_halign(gtk::Align::End);
        h.append(&actions_box);
        h
    } else {
        let h = hbox(16, "dk-disk-head");
        h.append(&art_box);
        h.append(&text);
        h.append(&actions_box);
        h
    };
    let (w, hw, dd) = (Rc::downgrade(app), head.downgrade(), d.clone());
    on_right_click(&head, move |x, y| {
        let (Some(app), Some(head)) = (w.upgrade(), hw.upgrade()) else { return };
        app.menu_at(&head, x, y, actions::disk_menu(&app, &dd));
    });
    let card = vbox(0, if truthy(&d["isSystem"]) { "dk-disk dk-system" } else { "dk-disk" });
    card.set_widget_name(&format!("dk-disk-{}", short(&id)));
    card.set_overflow(gtk::Overflow::Hidden);
    card.append(&head);
    let bar_wrap = vbox(0, "dk-disk-bar");
    bar_wrap.append(&disk_bar(app, d, false));
    card.append(&bar_wrap);
    let rows: Vec<gtk::Widget> = disk_rows(app, d).into_iter().map(|(w, _)| w).collect();
    if !rows.is_empty() {
        card.append(&rows_box(rows, false));
    }
    card.upcast()
}

pub fn legend(snap: &Value) -> gtk::Widget {
    let b = gtk::FlowBox::new();
    b.add_css_class("dk-legend");
    b.set_selection_mode(gtk::SelectionMode::None);
    b.set_column_spacing(18);
    b.set_row_spacing(6);
    b.set_homogeneous(false);
    b.set_max_children_per_line(12);
    b.set_focusable(false);
    b.set_can_target(false);
    b.update_property(&[gtk::accessible::Property::Label(&s("legend"))]);
    let ext = arr(&snap["disks"]).iter().any(|d| arr(&d["volumes"]).iter().any(|v| truthy(&v["isContainer"])));
    let mut items = vec![("data", "lg_data"), ("system", "lg_system")];
    if ext {
        items.push(("container", "lg_container"));
    }
    items.extend([("swap", "lg_swap"), ("encrypted", "lg_encrypted"), ("free", "lg_free")]);
    for (cls, key) in items {
        let it = hbox(6, "dk-legend-item");
        let sw = hbox(0, &format!("dk-legend-sw dk-sw-{cls}"));
        sw.set_valign(gtk::Align::Center);
        it.append(&sw);
        it.append(&label(&s(key), "", false));
        let child = gtk::FlowBoxChild::new();
        child.set_child(Some(&it));
        child.set_focusable(false);
        b.insert(&child, -1);
    }
    b.upcast()
}
