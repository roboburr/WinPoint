//! Drag and drop of files (vendor/ui/js/apps/explorer/dnd.js and the drag code of fileview.js, pages.js,
//! navpane.js, toolbar.js and tabs.js) with GTK 4's DragSource and DropTarget.
//!
//! - Dragging out: items of the file view (not in the Prullenbak) and of Start and Galerij. The content is a
//!   gdk::FileList (text/uri-list and the portal types for other apps), the paths as JSON
//!   (application/x-win11shell-paths) and as text/plain; the drag icon is the item's icon or thumbnail with a
//!   count badge (dnd.js dragImage). Moves into other apps are theirs to carry out: nothing is deleted here.
//! - Drop targets: folders in the view and the view itself (a folder listing), breadcrumb segments, navigation
//!   rows (a collapsed row opens after 800 ms hovering), tabs (a tab becomes active after 600 ms) and page
//!   items that are folders or drives; the Prullenbak row takes drags from Explorer and moves them to the
//!   Prullenbak.
//! - main.js dropInfo decides: nothing into the folder a drag came from (without Ctrl) or into a dragged
//!   folder; copy with Ctrl or to another drive, move with Shift or on the same drive. Drops from other apps
//!   are copied. The "Verplaatsen naar ..." / "Kopiëren naar ..." hint follows the pointer in the window
//!   (dnd.js showHint). The work itself is `ops::drop_to` / `ops::trash_paths`.
//!
//! A drag that started in any Explorer window of this process is "internal" (`CURRENT`); drags of tabs
//! between windows (transfer.rs, application/x-win11-explorer-tab+json) are left alone here.
use crate::explorer::Explorer;
use crate::fileview::FileView;
use crate::i18n::{f, s};
use crate::model::Loc;
use crate::ops::{self, Timer};
use crate::util::{self, Item, st};
use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use serde_json::Value;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// Our own type: the dragged paths as a JSON array.
pub const DND_MIME: &str = "application/x-win11shell-paths";
/// Tabs dragged between windows (transfer.rs): not files.
const TAB_MIME: &str = "application/x-win11-explorer-tab+json";

/// The drag that started in this process (dnd.js `dnd.current`).
#[derive(Clone)]
struct Current {
    paths: Vec<String>,
    src_dir: Option<String>,
}

thread_local! {
    static CURRENT: RefCell<Option<Current>> = const { RefCell::new(None) };
    static HINTS: RefCell<Vec<Hint>> = const { RefCell::new(Vec::new()) };
}

fn start_drag(paths: Vec<String>, src_dir: Option<String>) {
    CURRENT.with(|c| *c.borrow_mut() = Some(Current { paths, src_dir }));
}

fn end_drag() {
    CURRENT.with(|c| c.borrow_mut().take());
    hide_hints();
}

fn current() -> Option<Current> {
    CURRENT.with(|c| c.borrow().clone())
}

/// What a drop target sees of a drag.
struct Pointer {
    /// From Explorer (this process).
    internal: bool,
    ctrl: bool,
    shift: bool,
}

fn pointer(t: &gtk::DropTarget) -> Pointer {
    let mut m = t.current_event_state();
    if let Some(kb) = gdk::Display::default().and_then(|d| d.default_seat()).and_then(|s| s.keyboard()) {
        m |= kb.modifier_state();
    }
    let ours = t.current_drop().is_some_and(|d| d.drag().is_some());
    Pointer {
        internal: ours && current().is_some(),
        ctrl: m.contains(gdk::ModifierType::CONTROL_MASK),
        shift: m.contains(gdk::ModifierType::SHIFT_MASK),
    }
}

fn is_tab_drag(t: &gtk::DropTarget) -> bool {
    t.current_drop().is_some_and(|d| d.formats().contain_mime_type(TAB_MIME))
}

/// Where a drop goes and what it does (main.js dropInfo).
pub struct DropInfo {
    pub dest: String,
    pub copy: bool,
    pub label: String,
    /// To the Prullenbak (the navigation row).
    pub trash: bool,
}

/// main.js dropInfo: None where a drop makes no sense (a dragged item itself, inside a dragged folder, the
/// folder the drag came from without Ctrl); copy with Ctrl or to another drive, else move.
fn drop_info(x: &Explorer, dest: &str, p: &Pointer) -> Option<DropInfo> {
    if dest.is_empty() {
        return None;
    }
    let cur = if p.internal { current() } else { None };
    if let Some(c) = &cur {
        if c.paths.iter().any(|q| q == dest || util::is_under(dest, q)) {
            return None;
        }
        if c.src_dir.as_deref() == Some(dest) && !p.ctrl {
            return None;
        }
    }
    let same_disk = cur.as_ref().is_some_and(|c| {
        let s = x.state();
        let mount = |path: &str| util::volume_for(path, &s.volumes).map(|v| st(v, "mount").to_owned()).unwrap_or_else(|| "/".into());
        c.paths.first().is_some_and(|first| mount(first) == mount(dest))
    });
    let copy = p.ctrl || (!p.shift && !same_disk);
    let label = f(if copy { "copyTo" } else { "moveTo" }, &[("name", &x.folder_label(dest))]);
    Some(DropInfo { dest: dest.to_owned(), copy, label, trash: false })
}

/// The paths of a drop: the internal drag's, else the files of the FileList (local files only).
fn drop_paths(value: &glib::Value, p: &Pointer) -> (Vec<String>, bool) {
    if p.internal
        && let Some(c) = current()
    {
        return (c.paths, false);
    }
    let paths = value
        .get::<gdk::FileList>()
        .map(|l| l.files().iter().filter_map(|f| f.path()).map(|p| p.to_string_lossy().into_owned()).collect())
        .unwrap_or_default();
    (paths, true)
}

fn set_class(w: &impl IsA<gtk::Widget>, class: &str, on: bool) {
    if on {
        w.as_ref().add_css_class(class);
    } else {
        w.as_ref().remove_css_class(class);
    }
}

// ============================================================================================ the hint

/// "Verplaatsen naar Documenten" next to the pointer (dnd.js showHint), one per window, in its layer.
#[derive(Clone)]
struct Hint {
    layer: glib::WeakRef<gtk::Overlay>,
    fixed: gtk::Fixed,
    root: gtk::Box,
    icon: gtk::Box,
    label: gtk::Label,
    kind: Rc<RefCell<String>>,
}

fn hint_for(widget: &gtk::Widget) -> Option<Hint> {
    let win = kit::frame::AppWindow::of(widget)?;
    let layer = win.layer().clone();
    let known = HINTS.with(|h| {
        let mut h = h.borrow_mut();
        h.retain(|u| u.layer.upgrade().is_some());
        h.iter().find(|u| u.layer.upgrade().as_ref() == Some(&layer)).cloned()
    });
    if known.is_some() {
        return known;
    }
    let fixed = gtk::Fixed::new();
    fixed.set_can_target(false);
    fixed.set_can_focus(false);
    let root = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    root.add_css_class("xp-drophint");
    root.set_can_target(false);
    root.set_visible(false);
    let icon = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    icon.add_css_class("xp-drophint-icon");
    icon.set_size_request(16, 16);
    icon.set_valign(gtk::Align::Center);
    let label = gtk::Label::new(None);
    label.add_css_class("xp-drophint-text");
    root.append(&icon);
    root.append(&label);
    fixed.put(&root, 0.0, 0.0);
    layer.add_overlay(&fixed);
    let hint = Hint { layer: layer.downgrade(), fixed, root, icon, label, kind: Rc::new(RefCell::new(String::new())) };
    HINTS.with(|h| h.borrow_mut().push(hint.clone()));
    Some(hint)
}

/// Show the hint at (x, y) of `widget`: kind "move", "copy" or "trash".
fn show_hint(widget: &gtk::Widget, x: f64, y: f64, text: &str, kind: &str) {
    let Some(h) = hint_for(widget) else { return };
    let Some(layer) = h.layer.upgrade() else { return };
    let Some(pt) = widget.compute_point(&layer, &gtk::graphene::Point::new(x as f32, y as f32)) else { return };
    // the hints of other windows go
    HINTS.with(|all| {
        for o in all.borrow().iter() {
            if o.root != h.root {
                o.root.set_visible(false);
            }
        }
    });
    if *h.kind.borrow() != kind {
        kit::controls::clear(&h.icon);
        let g = kit::icons::glyph(
            match kind {
                "copy" => "plus",
                "trash" => "delete",
                _ => "arrow-right",
            },
            12,
        );
        g.set_hexpand(true);
        h.icon.append(&g);
        *h.kind.borrow_mut() = kind.to_owned();
    }
    if h.label.label() != text {
        h.label.set_label(text);
    }
    h.root.set_visible(true);
    let (_, nat) = h.root.preferred_size();
    let (w, hh) = (nat.width() as f64, nat.height() as f64);
    let left = (layer.width() as f64 - w - 4.0).min(pt.x() as f64 + 14.0).max(0.0);
    let top = (layer.height() as f64 - hh - 4.0).min(pt.y() as f64 + 22.0).max(0.0);
    h.fixed.move_(&h.root, left.round(), top.round());
}

fn hide_hints() {
    HINTS.with(|all| {
        for h in all.borrow().iter() {
            h.root.set_visible(false);
        }
    });
}

// ============================================================================================ drag sources

/// The data of a drag: the files (text/uri-list for other apps), our paths type and plain text.
fn provider(paths: &[String]) -> gdk::ContentProvider {
    let files: Vec<gio::File> = paths.iter().map(gio::File::for_path).collect();
    let list = gdk::FileList::from_array(&files);
    let json = serde_json::to_string(paths).unwrap_or_default();
    gdk::ContentProvider::new_union(&[
        gdk::ContentProvider::for_value(&list.to_value()),
        gdk::ContentProvider::for_bytes(DND_MIME, &glib::Bytes::from_owned(json.into_bytes())),
        gdk::ContentProvider::for_bytes("text/plain;charset=utf-8", &glib::Bytes::from_owned(paths.join("\n").into_bytes())),
    ])
}

/// The drag icon (dnd.js dragImage): the icon or thumbnail at 64 px in an 80 px square, a count badge for
/// several items, the pointer in the middle.
fn set_icon(drag: &gdk::Drag, item: &Value, count: usize) {
    let ov = gtk::Overlay::new();
    ov.set_size_request(80, 80);
    let icon = kit::icons::file_item_icon(item, 64, true);
    icon.set_halign(gtk::Align::Center);
    icon.set_valign(gtk::Align::Center);
    ov.set_child(Some(&icon));
    if count > 1 {
        // the drag icon is its own surface without the window's CSS tokens: the badge draws itself
        let accent = kit::theme::token("accent");
        let fg = kit::theme::token("accent-text");
        let area = gtk::DrawingArea::new();
        area.set_size_request(20, 20);
        area.set_draw_func(move |_, cr, w, h| {
            let (w, h) = (w as f64, h as f64);
            let r = (h / 2.0).min(10.0);
            cr.new_sub_path();
            cr.arc(w - r, r, r, -std::f64::consts::FRAC_PI_2, 0.0);
            cr.arc(w - r, h - r, r, 0.0, std::f64::consts::FRAC_PI_2);
            cr.arc(r, h - r, r, std::f64::consts::FRAC_PI_2, std::f64::consts::PI);
            cr.arc(r, r, r, std::f64::consts::PI, 1.5 * std::f64::consts::PI);
            cr.close_path();
            cr.set_source_rgba(accent.red() as f64, accent.green() as f64, accent.blue() as f64, accent.alpha() as f64);
            let _ = cr.fill();
        });
        let label = gtk::Label::new(Some(&count.to_string()));
        let attrs = gtk::pango::AttrList::new();
        let c = |v: f32| (v.clamp(0.0, 1.0) * 65535.0).round() as u16;
        attrs.insert(gtk::pango::AttrColor::new_foreground(c(fg.red()), c(fg.green()), c(fg.blue())));
        attrs.insert(gtk::pango::AttrInt::new_weight(gtk::pango::Weight::Semibold));
        attrs.insert(gtk::pango::AttrSize::new_size_absolute(12 * gtk::pango::SCALE));
        label.set_attributes(Some(&attrs));
        label.set_margin_start(6);
        label.set_margin_end(6);
        let badge = gtk::Overlay::new();
        badge.set_child(Some(&area));
        badge.add_overlay(&label);
        badge.set_measure_overlay(&label, true);
        badge.set_halign(gtk::Align::End);
        badge.set_valign(gtk::Align::End);
        badge.set_margin_end(4);
        badge.set_margin_bottom(4);
        ov.add_overlay(&badge);
    }
    if let Ok(di) = gtk::DragIcon::for_drag(drag).downcast::<gtk::DragIcon>() {
        di.set_child(Some(&ov));
    }
    drag.set_hotspot(40, 40);
}

fn new_source() -> gtk::DragSource {
    let src = gtk::DragSource::new();
    src.set_actions(gdk::DragAction::COPY | gdk::DragAction::MOVE);
    src.set_button(gdk::BUTTON_PRIMARY);
    src
}

/// The current file view if it is still the one of this canvas.
fn view_of(x: &Explorer, canvas: &gtk::Widget) -> Option<FileView> {
    x.view().filter(|v| v.canvas() == *canvas)
}

// ============================================================================================ drop targets

type InfoFn = dyn Fn(&Explorer, f64, f64, &Pointer) -> Option<DropInfo>;
type MarkFn = dyn Fn(Option<&DropInfo>, f64, f64);

/// How one drop target behaves.
struct Wiring {
    info: Box<InfoFn>,
    /// Mark the target (Some) or clear the mark (None).
    mark: Box<MarkFn>,
    /// A drag came in (tabs: switch after a moment).
    enter: Option<Box<dyn Fn(&Explorer)>>,
    /// Each motion, with whether a drop is possible (navigation rows: open after a moment).
    hover: Option<Box<dyn Fn(&Explorer, bool)>>,
    /// The drag left or dropped.
    leave: Option<Box<dyn Fn()>>,
}

fn wire(x: &Explorer, widget: &gtk::Widget, wiring: Wiring) {
    let target = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY | gdk::DragAction::MOVE);
    let wiring = Rc::new(wiring);
    let (wx, ww) = (x.weak(), widget.downgrade());
    let motion = {
        let wiring = wiring.clone();
        Rc::new(move |t: &gtk::DropTarget, px: f64, py: f64, entering: bool| -> gdk::DragAction {
            let (Some(x), Some(widget)) = (wx.upgrade(), ww.upgrade()) else { return gdk::DragAction::empty() };
            if is_tab_drag(t) {
                return gdk::DragAction::empty();
            }
            if entering && let Some(e) = &wiring.enter {
                e(&x);
            }
            let p = pointer(t);
            let info = (wiring.info)(&x, px, py, &p);
            (wiring.mark)(info.as_ref(), px, py);
            if let Some(h) = &wiring.hover {
                h(&x, info.is_some());
            }
            match &info {
                None => {
                    hide_hints();
                    gdk::DragAction::empty()
                }
                Some(i) => {
                    show_hint(&widget, px, py, &i.label, if i.trash { "trash" } else if i.copy { "copy" } else { "move" });
                    // a drop from another app is copied, so say copy to it
                    if i.copy || !p.internal { gdk::DragAction::COPY } else { gdk::DragAction::MOVE }
                }
            }
        })
    };
    let m = motion.clone();
    target.connect_enter(move |t, px, py| m(t, px, py, true));
    let m = motion.clone();
    target.connect_motion(move |t, px, py| m(t, px, py, false));
    let w = wiring.clone();
    target.connect_leave(move |_| {
        (w.mark)(None, 0.0, 0.0);
        hide_hints();
        if let Some(l) = &w.leave {
            l();
        }
    });
    let w = wiring;
    let wx = x.weak();
    target.connect_drop(move |t, value, px, py| {
        (w.mark)(None, 0.0, 0.0);
        hide_hints();
        if let Some(l) = &w.leave {
            l();
        }
        let Some(x) = wx.upgrade() else { return false };
        if is_tab_drag(t) {
            return false;
        }
        let p = pointer(t);
        let Some(info) = (w.info)(&x, px, py, &p) else { return false };
        let (paths, external) = drop_paths(value, &p);
        end_drag();
        if paths.is_empty() {
            return false;
        }
        if info.trash {
            ops::trash_paths(&x, paths);
        } else {
            ops::drop_to(&x, paths, info.dest, info.copy || external);
        }
        true
    });
    widget.add_controller(target);
}

/// A widget marked with `.is-drop` while it is the target.
fn class_mark(widget: &gtk::Widget) -> Box<MarkFn> {
    let w = widget.downgrade();
    Box::new(move |i, _, _| {
        if let Some(w) = w.upgrade() {
            set_class(&w, "is-drop", i.is_some());
        }
    })
}

/// A target with one destination folder and a class mark (crumbs, page items).
fn folder_target(x: &Explorer, widget: &gtk::Widget, dest: String) {
    wire(
        x,
        widget,
        Wiring {
            info: Box::new(move |x, _, _, p| drop_info(x, &dest, p)),
            mark: class_mark(widget),
            enter: None,
            hover: None,
            leave: None,
        },
    );
}

// ============================================================================================ the hooks

/// The file view of a folder, the Prullenbak or search results (made by every render): items drag out (not in
/// the Prullenbak); a folder listing takes drops on its folders and on itself.
pub fn attach_view(x: &Explorer, view: &FileView) {
    let canvas = view.canvas();
    if view.draggable() {
        let src = new_source();
        let ghost: Rc<RefCell<Option<(Item, usize)>>> = Rc::new(RefCell::new(None));
        let (wx, cw, g) = (x.weak(), canvas.downgrade(), ghost.clone());
        src.connect_prepare(move |_, px, py| {
            let (x, c) = (wx.upgrade()?, cw.upgrade()?);
            let v = view_of(&x, &c)?;
            let item = v.item_at(px, py)?;
            let paths = v.drag_paths_at(px, py);
            if paths.is_empty() {
                return None;
            }
            let src_dir = v.src_dir().unwrap_or_else(|| util::dirname(st(&item, "path")));
            start_drag(paths.clone(), Some(src_dir));
            *g.borrow_mut() = Some((item, paths.len()));
            Some(provider(&paths))
        });
        let (wx, cw, g) = (x.weak(), canvas.downgrade(), ghost);
        src.connect_drag_begin(move |_, drag| {
            let shown = g.borrow_mut().take();
            if let Some((item, count)) = shown {
                set_icon(drag, &item, count);
            }
            if let (Some(x), Some(c), Some(cur)) = (wx.upgrade(), cw.upgrade(), current())
                && let Some(v) = view_of(&x, &c)
            {
                v.set_dragging(&cur.paths);
            }
        });
        let (wx, cw) = (x.weak(), canvas.downgrade());
        src.connect_drag_end(move |_, _, _| {
            if let (Some(x), Some(c)) = (wx.upgrade(), cw.upgrade())
                && let Some(v) = view_of(&x, &c)
            {
                v.set_dragging(&[]);
            }
            end_drag();
        });
        canvas.add_controller(src);
    }
    // drops only into a folder listing (not the Prullenbak or search results)
    let Some(folder) = view.src_dir() else { return };
    let last: Rc<RefCell<Option<(Option<String>, bool)>>> = Rc::new(RefCell::new(None));
    let (wx, cw) = (x.weak(), canvas.downgrade());
    let target_at = move |px: f64, py: f64| -> Option<String> {
        let (x, c) = (wx.upgrade()?, cw.upgrade()?);
        let v = view_of(&x, &c)?;
        v.item_at(px, py).filter(|it| util::flag(it, "isDir")).map(|it| st(&it, "path").to_owned())
    };
    let target_at = Rc::new(target_at);
    let ta = target_at.clone();
    let (wx, cw) = (x.weak(), canvas.downgrade());
    wire(
        x,
        &canvas,
        Wiring {
            info: Box::new(move |x, px, py, p| {
                let dest = ta(px, py).unwrap_or_else(|| folder.clone());
                drop_info(x, &dest, p)
            }),
            mark: Box::new(move |i, px, py| {
                let want = match i {
                    Some(_) => {
                        let t = target_at(px, py);
                        let whole = t.is_none();
                        (t, whole)
                    }
                    None => (None, false),
                };
                if last.borrow().as_ref() == Some(&want) {
                    return;
                }
                *last.borrow_mut() = Some(want.clone());
                if let (Some(x), Some(c)) = (wx.upgrade(), cw.upgrade())
                    && let Some(v) = view_of(&x, &c)
                {
                    v.set_drop_target(want.0.as_deref(), want.1);
                }
            }),
            enter: None,
            hover: None,
            leave: None,
        },
    );
}

/// Whether a navigation row can open and is closed (its chevron points right).
fn row_collapsed(row: &gtk::Widget) -> bool {
    fn find(w: &gtk::Widget) -> Option<gtk::Widget> {
        if w.has_css_class("xp-node-chev") {
            return Some(w.clone());
        }
        let mut c = w.first_child();
        while let Some(child) = c {
            if let Some(hit) = find(&child) {
                return Some(hit);
            }
            c = child.next_sibling();
        }
        None
    }
    let Some(chev) = find(row) else { return false };
    if chev.has_css_class("is-none") {
        return false;
    }
    chev.first_child().and_downcast::<gtk::Image>().and_then(|i| kit::icons::glyph_name(&i)).as_deref() == Some("chevron-right")
}

/// A navigation pane row; `expand` opens it (after 800 ms hovering with a drag that can drop there).
pub fn attach_nav_row(x: &Explorer, row: &gtk::Widget, loc: &Loc, expand: Rc<dyn Fn()>) {
    let (dest, trash) = match loc {
        Loc::Folder(p) => (p.clone(), false),
        Loc::Trash => (String::new(), true),
        _ => return,
    };
    let collapsed = row_collapsed(row);
    let timer = Timer::default();
    let armed = Rc::new(Cell::new(false));
    let (t, a) = (timer.clone(), armed.clone());
    wire(
        x,
        row,
        Wiring {
            info: Box::new(move |x, _, _, p| {
                if trash {
                    p.internal.then(|| DropInfo { dest: String::new(), copy: false, label: s("trashTo").to_owned(), trash: true })
                } else {
                    drop_info(x, &dest, p)
                }
            }),
            mark: class_mark(row),
            enter: None,
            hover: Some(Box::new(move |_, ok| {
                if ok && collapsed && !a.replace(true) {
                    let e = expand.clone();
                    t.start(800, move || e());
                }
            })),
            leave: Some(Box::new(move || {
                armed.set(false);
                timer.cancel();
            })),
        },
    );
}

/// A breadcrumb segment of the address bar (folders only).
pub fn attach_crumb(x: &Explorer, crumb: &gtk::Widget, loc: &Loc) {
    if let Loc::Folder(dest) = loc {
        folder_target(x, crumb, dest.clone());
    }
}

/// A tab of the strip: files dragged over it make it active after 600 ms; a drop goes into its folder.
pub fn attach_tab(x: &Explorer, tab: &gtk::Widget, id: u64) {
    let timer = Timer::default();
    let t = timer.clone();
    wire(
        x,
        tab,
        Wiring {
            info: Box::new(move |x, _, _, p| {
                let dest = x.tab(id).and_then(|t| match &t.borrow().loc {
                    Loc::Folder(p) => Some(p.clone()),
                    _ => None,
                })?;
                drop_info(x, &dest, p)
            }),
            mark: class_mark(tab),
            enter: Some(Box::new(move |x| {
                let w = x.weak();
                t.start(600, move || {
                    if let Some(x) = w.upgrade()
                        && x.active_id() != id
                    {
                        x.activate_tab(id);
                    }
                });
            })),
            hover: None,
            leave: Some(Box::new(move || timer.cancel())),
        },
    );
}

/// An item of Start, Deze pc or Galerij (`drag`: may be dragged; `drop`: a folder or drive to drop on).
pub fn attach_page_item(x: &Explorer, w: &gtk::Widget, item: &Item, drag: bool, drop: bool) {
    let path = st(item, "path").to_owned();
    if drag && !path.is_empty() {
        let src = new_source();
        let ghost: Rc<RefCell<Option<usize>>> = Rc::new(RefCell::new(None));
        let (wx, p, g) = (x.weak(), path.clone(), ghost.clone());
        src.connect_prepare(move |_, _, _| {
            let x = wx.upgrade()?;
            let selected = x.active_tab().is_some_and(|t| t.borrow().selection.contains(&p));
            if !selected {
                x.set_selection(std::slice::from_ref(&p), Some(&p));
            }
            let paths: Vec<String> = x.selected_items().iter().map(|i| st(i, "path").to_owned()).filter(|p| !p.is_empty()).collect();
            if paths.is_empty() {
                return None;
            }
            *g.borrow_mut() = Some(paths.len());
            start_drag(paths.clone(), Some(util::dirname(&p)));
            Some(provider(&paths))
        });
        let it = item.clone();
        src.connect_drag_begin(move |_, drag| {
            let count = ghost.borrow_mut().take().unwrap_or(1);
            set_icon(drag, &it, count);
        });
        src.connect_drag_end(|_, _, _| end_drag());
        w.add_controller(src.clone());
        // the item's click claims the press (pages.rs); in one group the drag can still start
        let list = w.observe_controllers();
        for i in 0..list.n_items() {
            if let Some(click) = list.item(i).and_downcast::<gtk::GestureClick>() {
                click.group_with(&src);
                break;
            }
        }
    }
    if drop && !path.is_empty() && util::flag(item, "isDir") {
        folder_target(x, w, path);
    }
}

