//! The special pages (last-webkit:vendor/ui/js/apps/explorer/pages.js): Start (Snelle toegang, Favorieten, Recent),
//! Deze pc (folders, devices and drives) and Galerij (the newest pictures by month, justified rows). Their
//! items behave like Explorer items: click, Ctrl+click, double click / Enter, arrows (by position), the item
//! context menu, the middle button for a new tab, drag and drop (dnd.rs).
use crate::actbox::ActBox;
use crate::explorer::{Explorer, big_empty};
use crate::i18n::{f, n, s};
use crate::menus;
use crate::model::Loc;
use crate::util::{self, Item, flag, num, st};
use gtk::prelude::*;
use gtk::{gdk, glib};
use kit::format::{self, DateFormat};
use serde_json::{Value, json};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

const MAX_PHOTOS: usize = 600;
const PHOTO_H: f64 = 172.0;

// ------------------------------------------------------------------------------------------ shared bits

fn page_root(x: &Explorer, body: &gtk::Box) -> gtk::ScrolledWindow {
    body.add_css_class("xp-page");
    body.set_focusable(true);
    let sw = gtk::ScrolledWindow::new();
    sw.set_hscrollbar_policy(gtk::PolicyType::Never);
    sw.set_vexpand(true);
    sw.set_hexpand(true);
    sw.set_child(Some(body));
    sw.add_css_class("xp-page-scroll");
    sw.set_widget_name("explorer-page");
    // a press on empty space clears the selection
    let click = gtk::GestureClick::new();
    click.set_button(gdk::BUTTON_PRIMARY);
    let w = x.weak();
    click.connect_pressed(move |_, _, _, _| {
        if let Some(x) = w.upgrade()
            && x.active_tab().is_some_and(|t| !t.borrow().selection.is_empty())
        {
            x.set_selection(&[], None);
        }
    });
    body.add_controller(click);
    let w = x.weak();
    kit::controls::bind_context_menu(body, move |_, _| {
        let x = w.upgrade()?;
        Some(menus::background_menu(&x))
    });
    sw
}

/// A section with a header that collapses it (remembered per window).
fn section(x: &Explorer, id: &str, title: &str, count: Option<usize>, body: &impl IsA<gtk::Widget>) -> (gtk::Box, gtk::Label) {
    let key = format!("sec:{id}");
    let collapsed = x.is_collapsed(&key);
    let sec = gtk::Box::new(gtk::Orientation::Vertical, 0);
    sec.add_css_class("xp-sec");
    let head = gtk::Button::new();
    head.add_css_class("xp-sec-head");
    head.set_halign(gtk::Align::Start);
    head.set_widget_name(&format!("explorer-sec-{id}"));
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let chev = kit::icons::glyph(if collapsed { "chevron-right" } else { "chevron-down" }, 12);
    chev.add_css_class("xp-sec-chev");
    row.append(&chev);
    row.append(&kit::controls::label(title, "xp-sec-title", false));
    let count_l = gtk::Label::new(count.map(|c| format!("({c})")).as_deref());
    count_l.add_css_class("xp-sec-count");
    count_l.set_visible(count.is_some());
    row.append(&count_l);
    head.set_child(Some(&row));
    let body_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
    body_box.add_css_class("xp-sec-body");
    body_box.append(body.as_ref());
    body_box.set_visible(!collapsed);
    let w = x.weak();
    let bb = body_box.downgrade();
    head.connect_clicked(move |_| {
        let (Some(x), Some(b)) = (w.upgrade(), bb.upgrade()) else { return };
        let now = b.is_visible();
        b.set_visible(!now);
        kit::icons::set_glyph(&chev, if now { "chevron-right" } else { "chevron-down" });
        x.set_collapsed(&key, now);
    });
    sec.append(&head);
    sec.append(&body_box);
    (sec, count_l)
}

/// Make a widget behave like an Explorer item.
fn selectable(x: &Explorer, w: &impl IsA<gtk::Widget>, item: Item, drag: bool, drop: bool) {
    let w = w.as_ref();
    w.set_focusable(true);
    w.add_css_class("xp-pitem");
    x.register_page_item(item.clone(), w);
    let path = st(&item, "path").to_owned();
    let click = gtk::GestureClick::new();
    click.set_button(0);
    let wx = x.weak();
    let it = item.clone();
    let p = path.clone();
    let ww = w.downgrade();
    click.connect_pressed(move |g, n_press, px, py| {
        let (Some(x), Some(w)) = (wx.upgrade(), ww.upgrade()) else { return };
        let button = g.current_button();
        let state = g.current_event_state();
        g.set_state(gtk::EventSequenceState::Claimed);
        if button == gdk::BUTTON_MIDDLE {
            if flag(&it, "isDir") {
                x.open_tab(Loc::Folder(p.clone()), false);
            }
            return;
        }
        if button != gdk::BUTTON_PRIMARY && button != gdk::BUTTON_SECONDARY {
            return;
        }
        let sel = x.active_tab().map(|t| t.borrow().selection.clone()).unwrap_or_default();
        if button == gdk::BUTTON_PRIMARY && state.contains(gdk::ModifierType::CONTROL_MASK) {
            let mut next: Vec<String> = sel.iter().cloned().collect();
            if let Some(i) = next.iter().position(|q| *q == p) {
                next.remove(i);
            } else {
                next.push(p.clone());
            }
            x.set_selection(&next, Some(&p));
        } else if !sel.contains(&p) {
            x.set_selection(std::slice::from_ref(&p), Some(&p));
        }
        w.grab_focus();
        if button == gdk::BUTTON_PRIMARY && n_press == 2 {
            x.open_item(&it, false);
        }
        if button == gdk::BUTTON_SECONDARY {
            let items = x.selected_items();
            let ww = w.clone();
            menus::item_menu(&x, items, move |top, list| menus::show(&ww, px, py, top, list));
        }
    });
    w.add_controller(click);
    let keys = gtk::EventControllerKey::new();
    let wx = x.weak();
    let it = item.clone();
    let ww = w.downgrade();
    keys.connect_key_pressed(move |_, key, _, state| {
        let (Some(x), Some(w)) = (wx.upgrade(), ww.upgrade()) else { return glib::Propagation::Proceed };
        match key {
            gdk::Key::Return | gdk::Key::KP_Enter if !state.contains(gdk::ModifierType::ALT_MASK) => {
                x.open_item(&it, false);
                glib::Propagation::Stop
            }
            gdk::Key::Menu => {
                let ww = w.clone();
                menus::item_menu(&x, x.selected_items(), move |top, list| menus::show(&ww, 24.0, 28.0, top, list));
                glib::Propagation::Stop
            }
            gdk::Key::F10 if state.contains(gdk::ModifierType::SHIFT_MASK) => {
                let ww = w.clone();
                menus::item_menu(&x, x.selected_items(), move |top, list| menus::show(&ww, 24.0, 28.0, top, list));
                glib::Propagation::Stop
            }
            gdk::Key::Left | gdk::Key::Right | gdk::Key::Up | gdk::Key::Down => {
                arrow(&x, &w, key);
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        }
    });
    w.add_controller(keys);
    if let Some(ab) = w.downcast_ref::<ActBox>() {
        let wx = x.weak();
        let it = item.clone();
        ab.connect_activated(move |_| {
            if let Some(x) = wx.upgrade() {
                x.open_item(&it, false);
            }
        });
    }
    crate::dnd::attach_page_item(x, w, &item, drag, drop);
}

/// Arrow keys on a page: Left / Right in page order, Up / Down to the nearest item in the row above or below.
fn arrow(x: &Explorer, from: &gtk::Widget, key: gdk::Key) {
    let Some(page) = from.ancestor(gtk::ScrolledWindow::static_type()) else { return };
    let items: Vec<(Item, gtk::Widget)> = x.page_items().into_iter().filter(|(_, w)| w.is_mapped() && w.is_drawable()).collect();
    let pos = |w: &gtk::Widget| w.compute_bounds(&page).map(|b| (b.x() as f64, b.y() as f64)).unwrap_or((0.0, 0.0));
    let Some(i) = items.iter().position(|(_, w)| w == from) else { return };
    let (x0, y0) = pos(from);
    let target = match key {
        gdk::Key::Right => items.get(i + 1),
        gdk::Key::Left => i.checked_sub(1).and_then(|j| items.get(j)),
        _ => {
            let down = key == gdk::Key::Down;
            let mut c: Vec<&(Item, gtk::Widget)> = items
                .iter()
                .filter(|(_, w)| {
                    let (_, y) = pos(w);
                    if down { y > y0 + 4.0 } else { y < y0 - 4.0 }
                })
                .collect();
            c.sort_by(|a, b| {
                let (ax, ay) = pos(&a.1);
                let (bx, by) = pos(&b.1);
                let da = (ay - y0).abs().round();
                let db = (by - y0).abs().round();
                da.partial_cmp(&db)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then((ax - x0).abs().partial_cmp(&(bx - x0).abs()).unwrap_or(std::cmp::Ordering::Equal))
            });
            c.first().copied()
        }
    };
    if let Some((it, w)) = target {
        let p = st(it, "path").to_owned();
        x.set_selection(std::slice::from_ref(&p), Some(&p));
        w.grab_focus();
    }
}

fn folder_tile(x: &Explorer, path: &str, label: &str, icon: &str, sub: &str, pinned: bool) -> ActBox {
    let tile = ActBox::new(gtk::Orientation::Horizontal, 12);
    tile.add_css_class("xp-ftile");
    tile.set_tooltip_text(Some(path));
    tile.set_size_request(248, 68);
    let ib = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    ib.add_css_class("xp-ftile-icon");
    ib.set_valign(gtk::Align::Center);
    ib.append(&kit::icons::file_icon(icon, 48));
    tile.append(&ib);
    let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
    text.set_valign(gtk::Align::Center);
    text.set_hexpand(true);
    let name = kit::controls::label(label, "xp-ftile-name", false);
    name.set_max_width_chars(24);
    text.append(&name);
    let subrow = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let sl = kit::controls::label(sub, "xp-ftile-sub", false);
    sl.set_max_width_chars(24);
    subrow.append(&sl);
    if pinned {
        let pin = kit::icons::glyph("pin", 12);
        pin.add_css_class("xp-ftile-pin");
        pin.set_tooltip_text(Some(s("pinned")));
        subrow.append(&pin);
    }
    text.append(&subrow);
    tile.append(&text);
    tile.set_widget_name(&format!("explorer-tile-{}", util::basename(path)));
    let item = Rc::new(json!({ "path": path, "name": label, "displayName": label, "isDir": true, "icon": icon, "pinned": pinned }));
    selectable(x, &tile, item, true, true);
    tile
}

fn tiles_box() -> gtk::FlowBox {
    let fb = gtk::FlowBox::new();
    fb.add_css_class("xp-ftiles");
    fb.set_selection_mode(gtk::SelectionMode::None);
    fb.set_homogeneous(true);
    fb.set_max_children_per_line(24);
    fb.set_min_children_per_line(1);
    fb.set_row_spacing(4);
    fb.set_column_spacing(4);
    fb.set_halign(gtk::Align::Start);
    fb.set_can_focus(false);
    fb.set_activate_on_single_click(false);
    fb
}

fn add_tile(fb: &gtk::FlowBox, tile: &impl IsA<gtk::Widget>) {
    fb.append(tile);
    if let Some(child) = tile.as_ref().parent() {
        child.set_focusable(false);
        child.set_can_focus(false);
    }
}

fn page_message(text: &str) -> gtk::Label {
    kit::controls::label(text, "xp-page-empty", true)
}

fn page_loading(size: i32) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 0);
    b.add_css_class("xp-page-loading");
    let r = kit::controls::ProgressRing::new(size, None);
    r.widget().set_halign(gtk::Align::Center);
    b.append(r.widget());
    b
}

// ================================================================================================= Start

/// Start: Snelle toegang, Favorieten, Recent.
pub fn home_page(x: &Explorer) -> gtk::Widget {
    let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let sw = page_root(x, &body);
    let qa = tiles_box();
    for (path, label, icon) in x.pinned_list() {
        let sub = x.stored_on(&path);
        add_tile(&qa, &folder_tile(x, &path, &label, &icon, &sub, true));
    }
    body.append(&section(x, "qa", s("quickAccess"), None, &qa).0);
    let fav_body = gtk::Box::new(gtk::Orientation::Vertical, 0);
    fav_body.add_css_class("xp-reclist");
    body.append(&section(x, "fav", s("favorites"), None, &fav_body).0);
    let rec_body = gtk::Box::new(gtk::Orientation::Vertical, 0);
    rec_body.add_css_class("xp-reclist");
    rec_body.append(&page_loading(24));
    body.append(&section(x, "recent", s("recent"), None, &rec_body).0);

    // favourites (settings), their details from files.properties
    let favs = x.favorite_paths();
    let placeholder: Vec<Value> = favs.iter().map(|p| json!({ "path": p, "name": util::basename(p) })).collect();
    fav_body.append(&rec_table(x, &placeholder, s("favEmpty"), false));
    if !favs.is_empty() {
        let ctx = x.ctx();
        let w = x.weak();
        let fb = fav_body.downgrade();
        glib::spawn_future_local(async move {
            let mut list = Vec::new();
            for p in favs {
                if let Ok(pr) = ctx.call_async("files.properties", json!({ "path": p })).await {
                    let name = match st(&pr, "name") {
                        "" => util::basename(&p),
                        n => n.to_owned(),
                    };
                    let is_dir = st(&pr, "type") == "folder" || pr.get("contains").is_some_and(|c| !c.is_null());
                    list.push(json!({ "path": p, "name": name, "mime": pr["mime"], "modified": pr["modified"], "isDir": is_dir }));
                }
            }
            let (Some(x), Some(fb)) = (w.upgrade(), fb.upgrade()) else { return };
            if fb.root().is_none() {
                return;
            }
            kit::controls::clear(&fb);
            fb.append(&rec_table(&x, &list, s("favEmpty"), false));
            x.repaint_selection();
        });
    }
    // recent files
    let enabled = x.privacy_recent();
    if enabled {
        let w = x.weak();
        let rb = rec_body.downgrade();
        x.call("apps.recent", json!({ "limit": 20 }), move |_, r| {
            let (Some(x), Some(rb)) = (w.upgrade(), rb.upgrade()) else { return };
            if rb.root().is_none() {
                return;
            }
            let list = r.ok().and_then(|v| v.as_array().cloned()).unwrap_or_default();
            kit::controls::clear(&rb);
            rb.append(&rec_table(&x, &list, s("recentEmpty"), true));
            x.repaint_selection();
        });
    } else {
        kit::controls::clear(&rec_body);
        rec_body.append(&rec_table(x, &[], s("recentOff"), true));
    }
    sw.upcast()
}

fn rec_table(x: &Explorer, items: &[Value], empty: &str, opened: bool) -> gtk::Widget {
    if items.is_empty() {
        return page_message(empty).upcast();
    }
    let table = gtk::Box::new(gtk::Orientation::Vertical, 0);
    table.add_css_class("xp-rectable");
    let cells = |a: gtk::Widget, b: gtk::Widget, c: gtk::Widget, class: &str| {
        let row = ActBox::new(gtk::Orientation::Horizontal, 0);
        row.add_css_class("xp-rec");
        if !class.is_empty() {
            row.add_css_class(class);
        }
        a.set_hexpand(true);
        a.set_size_request(180, -1);
        b.set_size_request(150, -1);
        c.set_hexpand(true);
        c.set_size_request(120, -1);
        for w in [&a, &b, &c] {
            w.add_css_class("xp-rec-cell");
            row.append(w);
        }
        row
    };
    let head = cells(
        kit::controls::label(s("colName"), "", false).upcast(),
        kit::controls::label(if opened { s("colOpened") } else { s("colModified") }, "", false).upcast(),
        kit::controls::label(s("colLocation"), "", false).upcast(),
        "xp-rec-head",
    );
    table.append(&head);
    for it in items {
        let path = st(it, "path").to_owned();
        let name = match st(it, "name") {
            "" => util::basename(&path),
            n => n.to_owned(),
        };
        let item = Rc::new(json!({
            "path": path, "name": name, "isDir": flag(it, "isDir"), "mime": it["mime"], "icon": it["icon"],
            "modified": it["modified"], "size": it["size"],
        }));
        let name_cell = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        name_cell.append(&kit::icons::file_item_icon(&item, 16, false));
        name_cell.append(&kit::controls::label(&x.item_name(&item), "xp-rec-text", false));
        let row = cells(
            name_cell.upcast(),
            kit::controls::label(&util::date_time(num(it, "modified")), "xp-rec-dim", false).upcast(),
            kit::controls::label(&x.friendly_dir(&util::dirname(&path)), "xp-rec-dim", false).upcast(),
            "",
        );
        row.set_tooltip_text(Some(&path));
        selectable(x, &row, item, true, false);
        table.append(&row);
    }
    table.upcast()
}

// =============================================================================================== Deze pc

/// Deze pc: the folders and the drives.
pub fn this_pc_page(x: &Explorer) -> gtk::Widget {
    let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let sw = page_root(x, &body);
    let folders = x.default_folders();
    let fgrid = tiles_box();
    for (path, label, icon) in &folders {
        let sub = x.stored_on(path);
        add_tile(&fgrid, &folder_tile(x, path, label, icon, &sub, false));
    }
    body.append(&section(x, "folders", s("folders"), Some(folders.len()), &fgrid).0);
    let dgrid = tiles_box();
    dgrid.add_css_class("xp-dtiles");
    let holder = gtk::Box::new(gtk::Orientation::Vertical, 0);
    holder.append(&dgrid);
    let (sec, count) = section(x, "devices", s("devices"), Some(0), &holder);
    body.append(&sec);
    render_drives(x, &dgrid, &holder, &count);
    let w = x.weak();
    let (dg, hb, cl) = (dgrid.downgrade(), holder.downgrade(), count.downgrade());
    x.load_volumes(move |_| {
        if let (Some(x), Some(dg), Some(hb), Some(cl)) = (w.upgrade(), dg.upgrade(), hb.upgrade(), cl.upgrade())
            && dg.root().is_some()
        {
            render_drives(&x, &dg, &hb, &cl);
        }
    });
    sw.upcast()
}

fn render_drives(x: &Explorer, grid: &gtk::FlowBox, holder: &gtk::Box, count: &gtk::Label) {
    grid.remove_all();
    let mut c = holder.first_child();
    while let Some(w) = c {
        c = w.next_sibling();
        if w.has_css_class("xp-page-empty") {
            holder.remove(&w);
        }
    }
    let vols = x.state().volumes.clone();
    count.set_label(&format!("({})", vols.len()));
    if vols.is_empty() {
        holder.append(&page_message(s("noDrives")));
        return;
    }
    for v in vols {
        let pct = util::volume_percent(&v);
        let full = pct >= 90.0;
        let tile = ActBox::new(gtk::Orientation::Horizontal, 12);
        tile.add_css_class("xp-dtile");
        if full {
            tile.add_css_class("is-full");
        }
        tile.set_size_request(280, 76);
        let mount = st(&v, "mount").to_owned();
        tile.set_tooltip_text(Some(&mount));
        let ib = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        ib.set_valign(gtk::Align::Center);
        ib.append(&kit::icons::file_icon(util::volume_icon(&v), 48));
        tile.append(&ib);
        let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
        text.set_valign(gtk::Align::Center);
        text.set_hexpand(true);
        let label = util::volume_label(&v);
        let name = kit::controls::label(&label, "xp-dtile-name", false);
        name.set_max_width_chars(26);
        text.append(&name);
        let bar = gtk::DrawingArea::new();
        bar.add_css_class("xp-dtile-bar");
        bar.set_content_height(6);
        bar.set_hexpand(true);
        bar.set_draw_func(move |_, cr, w, h| {
            let (w, h) = (w as f64, h as f64);
            let r = h / 2.0;
            let rounded = |cr: &gtk::cairo::Context, width: f64| {
                cr.new_sub_path();
                cr.arc(width - r, r, r, -std::f64::consts::FRAC_PI_2, std::f64::consts::FRAC_PI_2);
                cr.arc(r, r, r, std::f64::consts::FRAC_PI_2, 3.0 * std::f64::consts::FRAC_PI_2);
                cr.close_path();
            };
            let track = kit::theme::token("chart-track");
            cr.set_source_rgba(track.red() as f64, track.green() as f64, track.blue() as f64, track.alpha() as f64);
            rounded(cr, w);
            let _ = cr.fill();
            let fw = (w * pct / 100.0).max(if pct > 0.0 { h } else { 0.0 });
            if fw > 0.0 {
                let c = kit::theme::token(if full { "sys-critical" } else { "accent" });
                cr.set_source_rgba(c.red() as f64, c.green() as f64, c.blue() as f64, c.alpha() as f64);
                rounded(cr, fw);
                let _ = cr.fill();
            }
        });
        kit::theme::track(&bar);
        bar.update_property(&[gtk::accessible::Property::ValueNow(pct.round())]);
        if full {
            bar.set_tooltip_text(Some(s("almostFull")));
        }
        text.append(&bar);
        let sub = f("freeOf", &[("free", &format::bytes(num(&v, "free"))), ("total", &format::bytes(num(&v, "total")))]);
        text.append(&kit::controls::label(&sub, "xp-dtile-sub", false));
        tile.append(&text);
        tile.set_widget_name(&format!("explorer-drive-{}", util::basename(&mount)));
        let item = Rc::new(json!({
            "path": mount, "name": label, "displayName": label, "isDir": true, "volume": v, "icon": util::volume_icon(&v),
        }));
        selectable(x, &tile, item, false, true);
        add_tile(grid, &tile);
    }
    x.repaint_selection();
}

// =============================================================================================== Galerij

struct Photo {
    item: Item,
    tile: ActBox,
    /// The overlay's main child: its size request is the tile's size (the picture does not count).
    sizer: gtk::Box,
    overlay: gtk::Overlay,
    picture: gtk::Picture,
    ar: Cell<f64>,
    asked: Cell<bool>,
}

struct Month {
    photos: Vec<Rc<Photo>>,
    rows: gtk::Box,
}

struct Gallery {
    sw: gtk::ScrolledWindow,
    months: RefCell<Vec<Month>>,
    width: Cell<i32>,
    pending: RefCell<Option<glib::SourceId>>,
}

/// Galerij: the newest pictures of Afbeeldingen, by month.
pub fn gallery_page(x: &Explorer) -> gtk::Widget {
    let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
    body.add_css_class("xp-gallery");
    let sw = page_root(x, &body);
    body.add_css_class("xp-page-gallery");
    body.append(&page_loading(32));
    let root = x.place_path("pictures");
    let g = Rc::new(Gallery { sw: sw.clone(), months: RefCell::new(Vec::new()), width: Cell::new(0), pending: RefCell::new(None) });
    let probe = gtk::DrawingArea::new();
    probe.set_can_target(false);
    let ov = gtk::Overlay::new();
    ov.set_child(Some(&sw));
    ov.add_overlay(&probe);
    let gw = Rc::downgrade(&g);
    probe.connect_resize(move |_, w, _| {
        if let Some(g) = gw.upgrade() {
            g.width.set(w);
            g.relayout_soon();
        }
    });
    let gw = Rc::downgrade(&g);
    sw.vadjustment().connect_value_changed(move |_| {
        if let Some(g) = gw.upgrade() {
            g.load_visible();
        }
    });
    let bw = body.downgrade();
    let g2 = g.clone();
    let pictures = root.clone();
    let done = move |x: &Explorer, photos: Vec<Item>| {
        let root = pictures;
        let Some(body) = bw.upgrade() else { return };
        if body.root().is_none() {
            return;
        }
        kit::controls::clear(&body);
        if photos.is_empty() {
            let action = if root.is_empty() {
                None
            } else {
                let b = kit::controls::button(s("openFolder"), kit::controls::Variant::Standard);
                b.set_halign(gtk::Align::Center);
                let (w, r) = (x.weak(), root.clone());
                b.connect_clicked(move |_| {
                    if let Some(x) = w.upgrade() {
                        x.go_to(Loc::Folder(r.clone()));
                    }
                });
                Some(b.upcast())
            };
            body.append(&big_empty(kit::icons::glyph("image", 40).upcast(), s("galleryEmpty"), s("galleryEmptyMsg"), action));
            return;
        }
        let more = photos.len() > MAX_PHOTOS;
        let mut cur = String::new();
        let mut months: Vec<Month> = Vec::new();
        for p in photos.into_iter().take(MAX_PHOTOS) {
            let m = format::date(num(&p, "modified"), DateFormat::Month);
            if m != cur {
                cur = m.clone();
                let mut title: String = m.chars().take(1).flat_map(char::to_uppercase).collect();
                title.extend(m.chars().skip(1));
                body.append(&kit::controls::label(&title, "xp-gallery-month", false));
                let rows = gtk::Box::new(gtk::Orientation::Vertical, 4);
                rows.add_css_class("xp-gallery-grid");
                body.append(&rows);
                months.push(Month { photos: Vec::new(), rows });
            }
            let tile = ActBox::new(gtk::Orientation::Horizontal, 0);
            tile.add_css_class("xp-photo");
            // the picture fills the tile; the tile keeps the width the row gives it
            tile.set_hexpand(false);
            tile.set_overflow(gtk::Overflow::Hidden);
            tile.set_tooltip_text(Some(st(&p, "name")));
            let picture = gtk::Picture::new();
            picture.set_content_fit(gtk::ContentFit::Cover);
            picture.set_can_shrink(true);
            let sizer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            let overlay = gtk::Overlay::new();
            overlay.set_child(Some(&sizer));
            overlay.add_overlay(&picture);
            tile.append(&overlay);
            selectable(x, &tile, p.clone(), true, false);
            if let Some(m) = months.last_mut() {
                m.photos.push(Rc::new(Photo { item: p, tile, sizer, overlay, picture, ar: Cell::new(1.5), asked: Cell::new(false) }));
            }
        }
        if more {
            body.append(&page_message(&n("galleryMore", MAX_PHOTOS, &[])));
        }
        *g2.months.borrow_mut() = months;
        g2.relayout();
        x.repaint_selection();
    };
    if root.is_empty() {
        done(x, Vec::new());
    } else {
        x.collect_images(&root, MAX_PHOTOS + 1, done);
    }
    // the gallery's state lives as long as its widget
    let keep = g.clone();
    ov.connect_destroy(move |_| {
        let _ = &keep;
    });
    ov.upcast()
}

impl Gallery {
    fn relayout_soon(self: &Rc<Self>) {
        if let Some(id) = self.pending.borrow_mut().take() {
            id.remove();
        }
        let w = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(Duration::from_millis(60), move || {
            if let Some(g) = w.upgrade() {
                g.pending.borrow_mut().take();
                g.relayout();
            }
        });
        *self.pending.borrow_mut() = Some(id);
    }

    /// Justified rows: as many photos (172 px high, their own aspect) as fit, stretched to the width; the
    /// last row of a month keeps its natural size.
    fn relayout(self: &Rc<Self>) {
        let avail = (self.width.get() as f64 - 32.0).max(200.0);
        let gap = 4.0;
        for m in self.months.borrow().iter() {
            kit::controls::clear(&m.rows);
            let mut row: Vec<Rc<Photo>> = Vec::new();
            let mut sum = 0.0;
            let flush = |row: &mut Vec<Rc<Photo>>, sum: f64, last: bool| {
                if row.is_empty() {
                    return;
                }
                let line = gtk::Box::new(gtk::Orientation::Horizontal, gap as i32);
                line.set_halign(gtk::Align::Start);
                let gaps = gap * (row.len() as f64 - 1.0);
                let ars: f64 = row.iter().map(|p| p.ar.get()).sum();
                let extra = if last { 0.0 } else { (avail - sum - gaps).max(0.0) };
                for p in row.iter() {
                    let base = PHOTO_H * p.ar.get();
                    let w = (base + extra * p.ar.get() / ars).min(avail).round() as i32;
                    p.sizer.set_size_request(w.max(40), PHOTO_H as i32);
                    if let Some(parent) = p.tile.parent().and_downcast::<gtk::Box>() {
                        parent.remove(&p.tile);
                    }
                    line.append(&p.tile);
                }
                m.rows.append(&line);
                row.clear();
            };
            for p in &m.photos {
                let base = PHOTO_H * p.ar.get();
                let next = sum + base + if row.is_empty() { 0.0 } else { gap };
                if !row.is_empty() && next > avail {
                    flush(&mut row, sum, false);
                    sum = 0.0;
                }
                sum += base + if row.is_empty() { 0.0 } else { gap };
                row.push(p.clone());
            }
            let total = sum;
            flush(&mut row, total, true);
        }
        let w = Rc::downgrade(self);
        glib::idle_add_local_once(move || {
            if let Some(g) = w.upgrade() {
                g.load_visible();
            }
        });
    }

    /// Thumbnails for the photos in (or near) view, once each.
    fn load_visible(self: &Rc<Self>) {
        let h = self.sw.height() as f64;
        for m in self.months.borrow().iter() {
            for p in &m.photos {
                if p.asked.get() {
                    continue;
                }
                let Some(b) = p.tile.compute_bounds(&self.sw) else { continue };
                let (y, bh) = (b.y() as f64, b.height() as f64);
                if y + bh < -600.0 || y > h + 600.0 {
                    continue;
                }
                p.asked.set(true);
                let w = Rc::downgrade(self);
                let pw = Rc::downgrade(p);
                kit::icons::thumbnail(st(&p.item, "path"), 180, move |tex| {
                    let (Some(g), Some(p)) = (w.upgrade(), pw.upgrade()) else { return };
                    match tex {
                        Some(t) => {
                            let ar = t.width() as f64 / t.height().max(1) as f64;
                            p.picture.set_paintable(Some(&t));
                            if (ar - p.ar.get()).abs() > 0.01 && ar.is_finite() && ar > 0.0 {
                                p.ar.set(ar.clamp(0.2, 5.0));
                                g.relayout_soon();
                            }
                        }
                        None => {
                            p.tile.add_css_class("is-broken");
                            let icon = kit::icons::file_item_icon(&p.item, 48, false);
                            icon.set_halign(gtk::Align::Center);
                            icon.set_valign(gtk::Align::Center);
                            p.picture.set_visible(false);
                            p.overlay.add_overlay(&icon);
                        }
                    }
                });
            }
        }
    }
}
