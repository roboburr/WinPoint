//! The details and preview pane (vendor/ui/js/apps/explorer/detailspane.js): Detailvenster (a big icon or the
//! thumbnail, a video's poster, the name, type, size, dates and location; several items as a stack of icons
//! with the total size; without a selection the location itself with its item count) and Voorbeeldvenster
//! (Alt+P: a large picture of the selected image, else Windows' messages).
//!
//! The core makes one pane per window (`DetailsPane::new`), puts `widget()` right of the content, shows or
//! hides it, sets the mode, calls `update()` after selection changes (debounced 60 ms here) and `render()`
//! after layout or language changes. Answers that arrive after the selection moved on are dropped (`seq`).
use crate::explorer::{Explorer, WeakExplorer};
use crate::i18n::{contains_val, n, s};
use crate::ops::Timer;
use crate::util::{self, Item, flag, num, st};
use gtk::gdk;
use gtk::prelude::*;
use serde_json::Value;
use std::cell::Cell;
use std::rc::{Rc, Weak};

const DASH: &str = "\u{2013}";
const VIDEO_EXT: &[&str] = &[".mp4", ".m4v", ".mov", ".mkv", ".webm", ".avi", ".mpg", ".mpeg", ".ogv", ".wmv", ".flv", ".mts", ".m2ts"];
const IMAGE_EXT: &[&str] = &[".png", ".jpg", ".jpeg", ".gif", ".webp", ".bmp", ".svg", ".avif", ".ico", ".tif", ".tiff"];

fn name_or_path(item: &Value) -> String {
    match st(item, "name") {
        "" => st(item, "path").to_lowercase(),
        n => n.to_lowercase(),
    }
}

fn is_volume(item: &Value) -> bool {
    item.get("volume").is_some_and(Value::is_object)
}

fn is_video(item: &Value) -> bool {
    !flag(item, "isDir") && !is_volume(item) && (st(item, "mime").starts_with("video/") || VIDEO_EXT.iter().any(|e| name_or_path(item).ends_with(e)))
}

fn is_image(item: &Value) -> bool {
    st(item, "mime").starts_with("image/") || IMAGE_EXT.iter().any(|e| name_or_path(item).ends_with(e))
}

/// A box that is exactly `w` x `h` (or, with -1, takes the size it is given) whatever its child's natural
/// size: a gtk::Picture asks for its texture's size, which would widen the pane.
fn sized(child: &impl IsA<gtk::Widget>, w: i32, h: i32) -> gtk::ScrolledWindow {
    let sw = gtk::ScrolledWindow::new();
    sw.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Never);
    sw.set_propagate_natural_width(false);
    sw.set_propagate_natural_height(false);
    sw.set_size_request(w, h);
    sw.set_child(Some(child.as_ref()));
    sw
}

fn picture(tex: &gdk::Texture) -> gtk::Picture {
    let pic = gtk::Picture::for_paintable(tex);
    pic.set_can_shrink(true);
    pic.set_content_fit(gtk::ContentFit::Contain);
    pic
}

/// A picture of a texture (fetched at twice the logical size) fitted in `max_w` x `max_h`, never above its
/// own logical size; `class` goes on the frame (radius, shadow).
fn fit_picture(tex: &gdk::Texture, max_w: i32, max_h: i32, class: &str) -> gtk::ScrolledWindow {
    let (tw, th) = ((tex.width() as f64 / 2.0).max(1.0), (tex.height() as f64 / 2.0).max(1.0));
    let k = (max_w as f64 / tw).min(max_h as f64 / th).min(1.0);
    let frame = sized(&picture(tex), (tw * k).round().max(1.0) as i32, (th * k).round().max(1.0) as i32);
    frame.set_halign(gtk::Align::Center);
    frame.set_valign(gtk::Align::Center);
    frame.set_overflow(gtk::Overflow::Hidden);
    frame.add_css_class(class);
    frame
}

/// Which pane is showing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaneMode {
    Details,
    Preview,
}

struct Inner {
    x: WeakExplorer,
    root: gtk::Box,
    mode: Cell<PaneMode>,
    /// Bumped by every render; late answers for an older one are dropped.
    seq: Cell<u64>,
    soon: Timer,
}

/// The details / preview pane; a cheap handle.
#[derive(Clone)]
pub struct DetailsPane {
    inner: Rc<Inner>,
}

impl DetailsPane {
    /// Made while the window is built: do not upgrade `x` here.
    pub fn new(x: WeakExplorer) -> DetailsPane {
        crate::dialogs::install_css();
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("xp-details");
        root.set_hexpand(false);
        root.set_widget_name("explorer-details");
        root.update_property(&[gtk::accessible::Property::Label(s("detailsPane"))]);
        DetailsPane { inner: Rc::new(Inner { x, root, mode: Cell::new(PaneMode::Details), seq: Cell::new(0), soon: Timer::default() }) }
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.inner.root
    }

    pub fn set_mode(&self, m: PaneMode) {
        if self.inner.mode.replace(m) == m {
            return;
        }
        let root = &self.inner.root;
        if m == PaneMode::Preview {
            root.add_css_class("is-preview");
        } else {
            root.remove_css_class("is-preview");
        }
        root.update_property(&[gtk::accessible::Property::Label(s(if m == PaneMode::Preview { "previewPane" } else { "detailsPane" }))]);
    }

    pub fn mode(&self) -> PaneMode {
        self.inner.mode.get()
    }

    /// The selection changed: render in 60 ms (detailspane.js debounce).
    pub fn update(&self) {
        let weak: Weak<Inner> = Rc::downgrade(&self.inner);
        self.inner.soon.start(60, move || {
            if let Some(inner) = weak.upgrade() {
                DetailsPane { inner }.render();
            }
        });
    }

    /// Draw the pane for the current selection.
    pub fn render(&self) {
        self.inner.soon.cancel();
        let my = self.inner.seq.get() + 1;
        self.inner.seq.set(my);
        let root = &self.inner.root;
        kit::controls::clear(root);
        let Some(x) = self.inner.x.upgrade() else { return };
        root.update_property(&[gtk::accessible::Property::Label(s(if self.mode() == PaneMode::Preview { "previewPane" } else { "detailsPane" }))]);
        if x.active_tab().is_none() {
            return;
        }
        if self.mode() == PaneMode::Preview {
            self.render_preview(&x, my);
        } else {
            self.render_details(&x, my);
        }
    }

    fn current(&self, my: u64) -> bool {
        self.inner.seq.get() == my
    }

    fn weak(&self) -> Weak<Inner> {
        Rc::downgrade(&self.inner)
    }

    /// The width the pane's content has (300 px pane, 240 px below 860 px; padding 20 on both sides).
    fn content_width(&self, x: &Explorer) -> i32 {
        let w = self.inner.root.width();
        let pane = if w > 0 { w } else if x.anchor().has_css_class("xp-narrow-860") { 240 } else { 300 };
        (pane - 40).max(80)
    }

    // ------------------------------------------------------------------------------------- preview

    fn message(&self, text: &str) {
        let l = kit::controls::label(text, "xp-preview-msg", true);
        l.set_justify(gtk::Justification::Center);
        l.set_xalign(0.5);
        l.set_halign(gtk::Align::Center);
        l.set_valign(gtk::Align::Center);
        l.set_max_width_chars(30);
        l.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        // the pane keeps its width whatever the text asks for
        let frame = sized(&l, -1, -1);
        frame.set_vexpand(true);
        self.inner.root.append(&frame);
    }

    /// Voorbeeldvenster: a large picture of the selected image (thumbnail service), else Windows' messages.
    fn render_preview(&self, x: &Explorer, my: u64) {
        let sel = x.selected_items();
        let item = match sel.as_slice() {
            [one] if !flag(one, "isDir") && !is_volume(one) => one.clone(),
            _ => {
                self.message(s("previewHint"));
                return;
            }
        };
        if !is_image(&item) && !kit::icons::can_thumb(&item) {
            self.message(s("noPreview"));
            return;
        }
        let holder = gtk::Box::new(gtk::Orientation::Vertical, 0);
        holder.add_css_class("xp-preview");
        holder.set_vexpand(true);
        self.inner.root.append(&holder);
        let width = self.inner.root.width().max(300) as f64;
        let px = (width * 2.0 * self.inner.root.scale_factor().max(1) as f64).clamp(320.0, 1024.0).round() as i64;
        let query = format!("/thumb?path={}&size={px}", kit::backend::encode(st(&item, "path")));
        let weak = self.weak();
        kit::icons::fetch(query, move |tex| {
            let Some(inner) = weak.upgrade() else { return };
            let me = DetailsPane { inner };
            if !me.current(my) {
                return;
            }
            match tex {
                Some(tex) => {
                    let pic = picture(&tex);
                    pic.add_css_class("xp-preview-img");
                    pic.update_property(&[gtk::accessible::Property::Label(&me.inner.x.upgrade().map(|x| x.item_name(&item)).unwrap_or_default())]);
                    let frame = sized(&pic, -1, -1);
                    frame.set_vexpand(true);
                    frame.set_hexpand(true);
                    holder.append(&frame);
                }
                None => {
                    if let Some(parent) = holder.parent().and_downcast::<gtk::Box>() {
                        parent.remove(&holder);
                    }
                    me.message(s("noPreview"));
                }
            }
        });
    }

    // ------------------------------------------------------------------------------------- details

    fn render_details(&self, x: &Explorer, my: u64) {
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.add_css_class("xp-details-content");
        let scroller = gtk::ScrolledWindow::new();
        scroller.set_hscrollbar_policy(gtk::PolicyType::Never);
        scroller.set_propagate_natural_width(false);
        scroller.set_vexpand(true);
        scroller.set_child(Some(&content));
        self.inner.root.append(&scroller);
        let sel = x.selected_items();
        if sel.is_empty() {
            self.render_location(x, &content);
            return;
        }
        if sel.len() > 1 {
            self.render_several(&sel, &content);
            return;
        }
        self.render_one(x, &sel[0], &content, my);
    }

    fn preview_box(thumb: bool) -> gtk::Box {
        let b = gtk::Box::new(gtk::Orientation::Vertical, 0);
        b.add_css_class("xp-details-preview");
        b.add_css_class(if thumb { "is-thumb" } else { "is-icon" });
        b.set_size_request(-1, 140);
        b
    }

    fn centred(w: &impl IsA<gtk::Widget>) {
        let w = w.as_ref();
        w.set_halign(gtk::Align::Center);
        w.set_valign(gtk::Align::Center);
        w.set_vexpand(true);
    }

    fn name_label(text: &str) -> gtk::Label {
        let l = kit::controls::label(text, "xp-details-name", true);
        l.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        l.set_selectable(true);
        l
    }

    fn prop(label: &str, value: &str) -> (gtk::Box, gtk::Label) {
        let b = gtk::Box::new(gtk::Orientation::Vertical, 1);
        b.add_css_class("xp-dprop");
        b.append(&kit::controls::label(label, "xp-dprop-label", true));
        let v = kit::controls::label(if value.is_empty() { DASH } else { value }, "xp-dprop-value", true);
        v.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        v.set_selectable(true);
        b.append(&v);
        (b, v)
    }

    /// No selection: the location, its item count, and the hint outside folders and search results.
    fn render_location(&self, x: &Explorer, content: &gtk::Box) {
        let loc = x.loc();
        let pv = Self::preview_box(false);
        let icon = x.loc_icon(&loc, 96);
        Self::centred(&icon);
        pv.append(&icon);
        content.append(&pv);
        content.append(&Self::name_label(&x.loc_label(&loc, false)));
        if let Some(count) = x.visible_count() {
            content.append(&kit::controls::label(&n("itemsInFolder", count, &[]), "xp-details-type", true));
        }
        if !loc.is_fileish() {
            content.append(&kit::controls::label(s("noSelection"), "xp-details-hint", true));
        }
    }

    /// Several items: a stack of three icons and the total size of the files.
    fn render_several(&self, sel: &[Item], content: &gtk::Box) {
        let pv = Self::preview_box(false);
        let stack = gtk::Fixed::new();
        stack.add_css_class("xp-details-stack");
        stack.set_size_request(120, 110);
        Self::centred(&stack);
        const SPOTS: [(f64, f64, f64); 3] = [(0.0, 0.0, 0.55), (18.0, 12.0, 0.8), (36.0, 24.0, 1.0)];
        let shown: Vec<&Item> = sel.iter().take(3).rev().collect();
        for (it, (left, top, opacity)) in shown.into_iter().zip(SPOTS) {
            let icon = kit::icons::file_item_icon(it, 72, true);
            icon.set_opacity(opacity);
            stack.put(&icon, left, top);
        }
        pv.append(&stack);
        content.append(&pv);
        content.append(&Self::name_label(&n("selected", sel.len(), &[])));
        let props = gtk::Box::new(gtk::Orientation::Vertical, 10);
        props.add_css_class("xp-details-props");
        let total: f64 = sel.iter().filter(|i| !flag(i, "isDir")).map(|i| num(i, "size")).sum();
        if total > 0.0 {
            props.append(&Self::prop(s("totalSize"), &util::size_text_n(total)).0);
        }
        content.append(&props);
    }

    /// One item: the thumbnail (a video's poster) or the icon, name, type, size, dates and location.
    fn render_one(&self, x: &Explorer, item: &Item, content: &gtk::Box, my: u64) {
        let video = is_video(item);
        let big = kit::icons::can_thumb(item) && !video;
        let pv = Self::preview_box(big);
        if big {
            let size = 240.min(self.content_width(x));
            let slot = gtk::Box::new(gtk::Orientation::Vertical, 0);
            slot.set_size_request(size, size);
            Self::centred(&slot);
            pv.append(&slot);
            let (weak, it) = (self.weak(), item.clone());
            kit::icons::thumbnail(st(item, "path"), 240, move |tex| {
                let Some(inner) = weak.upgrade() else { return };
                if inner.seq.get() != my {
                    return;
                }
                let w: gtk::Widget = match tex {
                    Some(tex) => fit_picture(&tex, size, 240, "xp-details-thumb").upcast(),
                    None => kit::icons::file_item_icon(&it, size, false).upcast(),
                };
                Self::centred(&w);
                kit::controls::clear(&slot);
                slot.append(&w);
            });
        } else {
            let icon = kit::icons::file_item_icon(item, 96, false);
            Self::centred(&icon);
            pv.append(&icon);
        }
        if video && self.inner.root.is_visible() {
            // keep the file icon while the bounded poster worker runs; no video player starts
            let px = (240.0 * (self.inner.root.scale_factor() as f64).clamp(1.0, 3.0)).min(1024.0).round() as i64;
            let query = format!(
                "/thumb?path={}&size={px}&video=1&modified={}",
                kit::backend::encode(st(item, "path")),
                kit::backend::encode(&(num(item, "modified") as i64).to_string())
            );
            let (weak, pvw) = (self.weak(), pv.downgrade());
            kit::icons::fetch(query, move |tex| {
                let (Some(inner), Some(pv), Some(tex)) = (weak.upgrade(), pvw.upgrade(), tex) else { return };
                if inner.seq.get() != my {
                    return;
                }
                let pic = fit_picture(&tex, 240, 240, "xp-details-thumb");
                Self::centred(&pic);
                kit::controls::clear(&pv);
                pv.append(&pic);
                pv.remove_css_class("is-icon");
                pv.add_css_class("is-thumb");
            });
        }
        content.append(&pv);
        content.append(&Self::name_label(&x.item_name(item)));
        let type_text = if is_volume(item) { s("localDisk").to_owned() } else { util::type_name(item) };
        content.append(&kit::controls::label(&type_text, "xp-details-type", true));
        let props = gtk::Box::new(gtk::Orientation::Vertical, 10);
        props.add_css_class("xp-details-props");
        if !flag(item, "isDir") {
            props.append(&Self::prop(s("size"), &util::size_text(item)).0);
        }
        props.append(&Self::prop(s("modified"), &util::date_time(num(item, "modified"))).0);
        let (created_row, created) = Self::prop(s("created"), "\u{2026}");
        props.append(&created_row);
        let in_trash = flag(item, "inTrash") || !st(item, "trashName").is_empty();
        let dir = if in_trash { Explorer::orig_dir(item) } else { util::dirname(st(item, "path")) };
        props.append(&Self::prop(s("location"), &x.friendly_dir(&dir)).0);
        content.append(&props);
        if is_volume(item) {
            props.remove(&created_row);
            return;
        }
        let weak = self.weak();
        let is_dir = flag(item, "isDir");
        x.call("files.properties", serde_json::json!({ "path": st(item, "path") }), move |_, r| {
            let Some(inner) = weak.upgrade() else { return };
            if inner.seq.get() != my {
                return;
            }
            match r {
                Ok(pr) => {
                    let c = util::date_time(num(&pr, "created"));
                    created.set_label(if c.is_empty() { DASH } else { &c });
                    if is_dir && pr["contains"].is_object() {
                        let k = &pr["contains"];
                        let text = contains_val(&(num(k, "files") as u64).to_string(), &(num(k, "folders") as u64).to_string());
                        props.prepend(&Self::prop(s("contains"), &text).0);
                        if let Some(size) = pr["size"].as_f64() {
                            props.prepend(&Self::prop(s("size"), &util::size_text_n(size)).0);
                        }
                    }
                }
                Err(_) => created.set_label(DASH),
            }
        });
    }
}
