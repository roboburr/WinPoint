//! Instellingen › Persoonlijke instellingen › Achtergrond (6ebf169f:vendor/ui/js/apps/settings/pages/background.js):
//! a picture or a solid colour, the fit, and the picture that follows light and dark, with GNOME Settings 50's
//! background chooser (settings parity, batch F): "Recente afbeeldingen" are the user's own pictures, copied into
//! `~/.local/share/backgrounds` by "Door foto's bladeren" as GNOME's Add Picture does (`background.add`) and
//! removable there (`background.remove`, to the trash); "Afbeeldingen" are WinPoint's own; "Afbeeldingen van Ubuntu
//! en GNOME" is GNOME's catalogue (`background.catalog`: gnome-background-properties, light and dark pairs, timed
//! slideshows). Choosing a picture sets settings.json `wallpaper.*` (WinPoint's look, which wallpaper.rs writes to
//! GNOME); of a light and dark pair the one of the current mode.
//!
//! Also the pieces the personalisation and Bluetooth pages share: `FillGrid` (CSS `repeat(auto-fill,
//! minmax(<min>, 1fr))`), `Thumb` (a wallpaper picture, cover-scaled in rounded corners) and the per-viewer
//! lists bridge.js kept in localStorage (recent photos and colours), here in settings-app.json next to the
//! settings, as common.rs keeps the recently opened pages.
use super::themes::WALLPAPERS;
use super::{Builder, Page, PageFuture};
use crate::common;
use crate::preview::{DesktopPreview, PreviewOpts};
use crate::widgets::Clamp;
use gtk::subclass::prelude::*;
use gtk::{graphene, gsk};
use kit::controls::{self as c, ColorSwatches, Dropdown, DropdownOption, SettingsCard, SettingsExpander, SettingsGroup, Variant};
use kit::prelude::*;
use kit::t;
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

pub const PAGES: &[(&str, Builder)] = &[("background", background)];
pub const CSS: &str = r#"
button.st-wp {
  min-height: 0; min-width: 0; margin: 0; padding: 0;
  border: 0 solid transparent; border-radius: 6px;
  background-color: transparent; background-image: none;
  box-shadow: 0 0 0 1px var(--stroke-card);
  text-shadow: none; -gtk-icon-shadow: none; outline: 0 solid transparent;
  transition: box-shadow 83ms linear;
}
button.st-wp:hover { box-shadow: 0 0 0 1px var(--stroke-control-strong); }
button.st-wp.w11-selected { box-shadow: 0 0 0 2px var(--accent), 0 0 0 4px var(--bg-solid-base); }
button.st-wp:focus-visible { outline: 2px solid var(--focus-outer); outline-offset: 3px; }
.st-thumb { border-radius: 6px; background-color: var(--fill-control-alt-3); color: var(--text-2); }
.st-wp-check {
  margin: 5px 5px 0 0; border-radius: 999px;
  background-color: var(--accent); color: var(--accent-text);
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.3); opacity: 0;
}
button.st-wp.w11-selected .st-wp-check { opacity: 1; }
.st-color-block-title { margin-bottom: 12px; }
button.st-wp-remove {
  min-height: 22px; min-width: 22px; margin: 4px; padding: 0; border-radius: 999px;
  background-color: var(--bg-solid-base); background-image: none; color: var(--text-1); opacity: 0.85;
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.3);
}
button.st-wp-remove:hover, button.st-wp-remove:focus-visible { opacity: 1; }
.st-wp-badge {
  margin: 0 0 5px 5px; padding: 1px 6px; border-radius: 4px;
  background-color: rgba(0, 0, 0, 0.55); color: #ffffff; font-size: var(--fs-caption);
}
"#;

// --------------------------------------------------------------------------------------------- FillGrid

mod fill_imp {
    use super::*;

    #[derive(Default)]
    pub struct FillGrid {
        pub children: RefCell<Vec<gtk::Widget>>,
        pub min: Cell<i32>,
        pub col_gap: Cell<i32>,
        pub row_gap: Cell<i32>,
        pub max: Cell<i32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for FillGrid {
        const NAME: &'static str = "StSettingsFillGrid";
        type Type = super::FillGrid;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for FillGrid {
        fn dispose(&self) {
            for c in self.children.borrow_mut().drain(..) {
                c.unparent();
            }
        }
    }

    impl FillGrid {
        pub fn visible(&self) -> Vec<gtk::Widget> {
            self.children.borrow().iter().filter(|c| c.is_visible()).cloned().collect()
        }

        /// (columns, column width, row heights) at a width: as many `min`-wide columns as fit, also when
        /// there are fewer children (auto-fill keeps the empty tracks).
        pub fn layout(&self, width: i32) -> (usize, i32, Vec<i32>) {
            let width = if self.max.get() > 0 { width.min(self.max.get()) } else { width };
            let (min, gap) = (self.min.get().max(1), self.col_gap.get());
            let cols = ((width + gap) / (min + gap)).max(1) as usize;
            let cw = ((width - gap * (cols as i32 - 1)) / cols as i32).max(1);
            let heights = self
                .visible()
                .chunks(cols)
                .map(|row| row.iter().map(|c| c.measure(gtk::Orientation::Vertical, cw).1).max().unwrap_or(0))
                .collect();
            (cols, cw, heights)
        }
    }

    impl WidgetImpl for FillGrid {
        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::HeightForWidth
        }

        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            let kids = self.visible();
            if kids.is_empty() {
                return (0, 0, -1, -1);
            }
            match orientation {
                gtk::Orientation::Horizontal => {
                    let min = kids.iter().map(|c| c.measure(orientation, -1).0).max().unwrap_or(0);
                    let n = kids.len() as i32;
                    let all = self.min.get() * n + self.col_gap.get() * (n - 1);
                    let nat = if self.max.get() > 0 { all.min(self.max.get()) } else { all };
                    (min, nat.max(min), -1, -1)
                }
                _ => {
                    let width = if for_size >= 0 { for_size } else { self.min.get() };
                    let (_, _, rows) = self.layout(width);
                    let h = rows.iter().sum::<i32>() + self.row_gap.get() * (rows.len() as i32 - 1).max(0);
                    (h, h, -1, -1)
                }
            }
        }

        fn size_allocate(&self, width: i32, _height: i32, _baseline: i32) {
            let (cols, cw, rows) = self.layout(width);
            let (cg, rg) = (self.col_gap.get(), self.row_gap.get());
            let mut y = 0;
            for (r, row) in self.visible().chunks(cols).enumerate() {
                for (i, c) in row.iter().enumerate() {
                    let h = if c.valign() == gtk::Align::Fill { rows[r] } else { c.measure(gtk::Orientation::Vertical, cw).1 };
                    c.size_allocate(&gtk::Allocation::new(i as i32 * (cw + cg), y, cw, h), -1);
                }
                y += rows[r] + rg;
            }
        }
    }
}

glib::wrapper! {
    /// CSS `grid-template-columns: repeat(auto-fill, minmax(<min>, 1fr))`: as many equal columns as fit (empty
    /// tracks kept), rows as high as their tallest child; `max` is the grid's max-width (0: none).
    pub struct FillGrid(ObjectSubclass<fill_imp::FillGrid>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl FillGrid {
    pub fn new(min: i32, col_gap: i32, row_gap: i32, max: i32) -> FillGrid {
        let g: FillGrid = glib::Object::new();
        g.imp().min.set(min);
        g.imp().col_gap.set(col_gap);
        g.imp().row_gap.set(row_gap);
        g.imp().max.set(max);
        g
    }

    pub fn append(&self, child: &impl IsA<gtk::Widget>) {
        child.as_ref().set_parent(self);
        self.imp().children.borrow_mut().push(child.as_ref().clone());
        self.queue_resize();
    }

    pub fn remove_all(&self) {
        for c in self.imp().children.borrow_mut().drain(..) {
            c.unparent();
        }
        self.queue_resize();
    }
}

// ------------------------------------------------------------------------------------------------ Thumb

type Painter = Rc<dyn Fn(&gtk::Snapshot, f32, f32)>;

mod thumb_imp {
    use super::*;

    #[derive(Default)]
    pub struct Thumb {
        pub texture: RefCell<Option<gdk::Texture>>,
        pub failed: Cell<bool>,
        pub ratio: Cell<f32>,
        pub height: Cell<i32>,
        pub nat_width: Cell<i32>,
        pub pos_y: Cell<f32>,
        pub radius: Cell<f32>,
        pub color: RefCell<Option<gdk::RGBA>>,
        pub path: RefCell<String>,
        pub painter: RefCell<Option<Painter>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Thumb {
        const NAME: &'static str = "StSettingsThumb";
        type Type = super::Thumb;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for Thumb {}

    impl WidgetImpl for Thumb {
        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::HeightForWidth
        }

        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            match orientation {
                gtk::Orientation::Horizontal => (0, self.nat_width.get(), -1, -1),
                _ => {
                    let r = self.ratio.get();
                    if r > 0.0 {
                        let w = if for_size > 0 { for_size } else { self.nat_width.get() };
                        let h = (w as f32 / r).round() as i32;
                        (h, h, -1, -1)
                    } else {
                        (self.height.get(), self.height.get(), -1, -1)
                    }
                }
            }
        }

        fn snapshot(&self, sn: &gtk::Snapshot) {
            let obj = self.obj();
            let (w, h) = (obj.width() as f32, obj.height() as f32);
            if w < 1.0 || h < 1.0 {
                return;
            }
            let bounds = graphene::Rect::new(0.0, 0.0, w, h);
            sn.push_rounded_clip(&gsk::RoundedRect::from_rect(bounds, self.radius.get().min(w / 2.0).min(h / 2.0)));
            if let Some(c) = self.color.borrow().as_ref() {
                sn.append_color(c, &bounds);
            }
            if let Some(t) = self.texture.borrow().as_ref() {
                let (tw, th) = (t.width() as f32, t.height() as f32);
                if tw > 0.0 && th > 0.0 {
                    // background-size: cover; background-position: center <pos_y>
                    let s = (w / tw).max(h / th);
                    let (dw, dh) = (tw * s, th * s);
                    let rect = graphene::Rect::new((w - dw) / 2.0, (h - dh) * self.pos_y.get(), dw, dh);
                    sn.append_scaled_texture(t, gsk::ScalingFilter::Trilinear, &rect);
                }
            } else if self.failed.get() {
                // the picture could not be loaded: the "image" glyph instead (st-wp__missing)
                let color = obj.color();
                let cr = sn.append_cairo(&bounds);
                kit::icons::draw_glyph(&cr, "image", (w as f64 - 24.0) / 2.0, (h as f64 - 24.0) / 2.0, 24.0, &color);
            }
            if let Some(p) = self.painter.borrow().clone() {
                p(sn, w, h);
            }
            sn.pop();
        }
    }
}

glib::wrapper! {
    /// A wallpaper picture: the texture cover-scaled into rounded corners, `ratio` (width / height) or a fixed
    /// height; a painter can draw over it (the taskbar preview's bar).
    pub struct Thumb(ObjectSubclass<thumb_imp::Thumb>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Thumb {
    /// `ratio` > 0: height from the width; else `height` px high. `nat_width`: the natural width.
    pub fn new(ratio: f32, height: i32, nat_width: i32, radius: f32) -> Thumb {
        let t: Thumb = glib::Object::new();
        t.add_css_class("st-thumb");
        let imp = t.imp();
        imp.ratio.set(ratio);
        imp.height.set(height);
        imp.nat_width.set(nat_width);
        imp.radius.set(radius);
        imp.pos_y.set(0.5);
        t.set_overflow(gtk::Overflow::Hidden);
        t
    }

    /// Show this wallpaper ("" none), loaded about `px` wide.
    pub fn set_path(&self, path: &str, px: i32) {
        if *self.imp().path.borrow() == path {
            return;
        }
        *self.imp().path.borrow_mut() = path.to_owned();
        *self.imp().texture.borrow_mut() = None;
        self.imp().failed.set(false);
        self.queue_draw();
        if path.is_empty() {
            return;
        }
        let weak = self.downgrade();
        let want = path.to_owned();
        common::wallpaper_texture(path, px, move |tex| {
            let Some(me) = weak.upgrade() else { return };
            if *me.imp().path.borrow() != want {
                return;
            }
            me.imp().failed.set(tex.is_none());
            *me.imp().texture.borrow_mut() = tex;
            me.queue_draw();
        });
    }

    /// A colour under the picture (a solid-colour wallpaper).
    pub fn set_color(&self, color: Option<gdk::RGBA>) {
        *self.imp().color.borrow_mut() = color;
        self.queue_draw();
    }

    /// background-position-y as a fraction (0.5: centre).
    pub fn set_pos_y(&self, y: f32) {
        self.imp().pos_y.set(y);
        self.queue_draw();
    }

    /// Draw `f(snapshot, width, height)` over the picture, inside the rounded corners.
    pub fn set_painter(&self, f: impl Fn(&gtk::Snapshot, f32, f32) + 'static) {
        *self.imp().painter.borrow_mut() = Some(Rc::new(f));
        self.queue_draw();
    }
}

// --------------------------------------------------------------------------------- remembered lists

thread_local! {
    static STATE: RefCell<Option<Value>> = const { RefCell::new(None) };
}

fn state_file() -> PathBuf {
    kit::ctx::settings_path().with_file_name("settings-app.json")
}

/// A list kept in settings-app.json ("recentPhotos", "recentColors"; bridge.js localStorage).
pub fn remembered(key: &str) -> Vec<String> {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        let v = s.get_or_insert_with(|| {
            std::fs::read(state_file())
                .ok()
                .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                .filter(Value::is_object)
                .unwrap_or_else(|| json!({}))
        });
        v.get(key)
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect())
            .unwrap_or_default()
    })
}

/// Keep a list in settings-app.json (written off the main thread; the file's other keys stay).
pub fn remember(key: &str, list: Vec<String>) {
    STATE.with(|s| {
        if let Some(v) = s.borrow_mut().as_mut()
            && let Some(o) = v.as_object_mut()
        {
            o.insert(key.to_owned(), json!(list));
        }
    });
    let (path, key) = (state_file(), key.to_owned());
    kit::spawn(
        move || {
            let mut state = std::fs::read(&path)
                .ok()
                .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                .filter(Value::is_object)
                .unwrap_or_else(|| json!({}));
            state[key.as_str()] = json!(list);
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let tmp = path.with_extension("json.lists.tmp");
            if std::fs::write(&tmp, state.to_string()).is_ok() {
                let _ = std::fs::rename(&tmp, &path);
            }
        },
        |_| {},
    );
}

fn recent_photos() -> Vec<String> {
    let mut l = remembered("recentPhotos");
    l.truncate(4);
    l
}

fn push_photo(path: &str) {
    let mut list = vec![path.to_owned()];
    list.extend(recent_photos().into_iter().filter(|x| x != path));
    list.truncate(4);
    remember("recentPhotos", list);
}

// ------------------------------------------------------------------------------------------------- page

const SOLID: &[&str] = &[
    "#FF8C00", "#E81123", "#D13438", "#C30052", "#BF0077", "#9A0089", "#881798", "#744DA9", "#10893E", "#107C10", "#018574", "#2D7D9A",
    "#0063B1", "#6B69D6", "#8E8CD8", "#8764B8", "#038387", "#486860", "#525E54", "#7E735F", "#4C4A48", "#515C6B", "#4A5459", "#000000",
];

/// settings.patch with common.js's error toast (background.js patch).
fn patch(p: &Page, patch: Value) {
    let wait = p.call("settings.patch", json!({ "patch": patch }));
    let pp = p.clone();
    p.spawn(async move {
        if let Err(e) = wait.await {
            pp.error(&e, None);
        }
    });
}

/// "Lotus licht" -> "Lotus" (the picture follows the mode).
fn strip_mode(name: &str) -> String {
    for suffix in ["licht", "donker", "light", "dark"] {
        let Some(cut) = name.len().checked_sub(suffix.len()) else { continue };
        if !name.is_char_boundary(cut) || !name[cut..].eq_ignore_ascii_case(suffix) {
            continue;
        }
        let head = &name[..cut];
        if head.ends_with(char::is_whitespace) {
            return head.trim_end().to_owned();
        }
    }
    name.to_owned()
}

/// The tiles of WinPoint's own pictures: one per light / dark pair while the picture follows the mode.
fn tiles() -> Vec<(String, String)> {
    let s = kit::ctx().settings();
    let follow = s["wallpaper"]["followTheme"] != false;
    let dark = kit::theme::dark();
    let mut list: Vec<(String, String)> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for w in WALLPAPERS {
        let path = common::twin_for(&format!("builtin:{}", w.id), dark, follow);
        if seen.contains(&path) {
            continue;
        }
        seen.push(path.clone());
        let wp = WALLPAPERS.iter().find(|x| format!("builtin:{}", x.id) == path).unwrap_or(w);
        let name = if follow && (w.id.starts_with("lotus-") || w.id.starts_with("flow-")) { strip_mode(wp.name()) } else { wp.name().to_owned() };
        list.push((path, name));
    }
    list
}

/// A selection check in the corner of a picture (`.st-wp__check`, `.st-theme__check`).
fn check_mark(size: i32, class: &str) -> gtk::Box {
    let check = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    check.add_css_class(class);
    check.set_size_request(size, size);
    check.set_halign(gtk::Align::End);
    check.set_valign(gtk::Align::Start);
    check.set_can_target(false);
    let g = kit::icons::glyph_stroke("check", 12, 2.2);
    g.set_hexpand(true);
    g.set_halign(gtk::Align::Center);
    check.append(&g);
    check
}

/// One picture of a grid: the thumbnail (`thumb`), the selection check, an optional badge ("Diavoorstelling").
fn tile_button(thumb_path: &str, name: &str, on: bool, widget: &str, badge: Option<&str>) -> gtk::Button {
    let thumb = Thumb::new(1.6, 0, 104, 6.0);
    thumb.set_path(thumb_path, 320);
    let ov = gtk::Overlay::new();
    ov.set_child(Some(&thumb));
    ov.add_overlay(&check_mark(18, "st-wp-check"));
    if let Some(text) = badge {
        let l = c::label(text, "st-wp-badge", false);
        l.set_halign(gtk::Align::Start);
        l.set_valign(gtk::Align::End);
        l.set_can_target(false);
        ov.add_overlay(&l);
    }
    let b = gtk::Button::new();
    b.add_css_class("st-wp");
    if on {
        b.add_css_class("w11-selected");
    }
    b.set_tooltip_text(Some(name));
    b.set_child(Some(&ov));
    b.set_widget_name(widget);
    b
}

fn paint_grid(p: &Page, grid: &FillGrid) {
    grid.remove_all();
    let s = kit::ctx().settings();
    let dark = kit::theme::dark();
    let follow = s["wallpaper"]["followTheme"] != false;
    let cur = (s["wallpaper"]["type"] == "image").then(|| common::resolve_wallpaper_path(&s, dark));
    for (i, (path, name)) in tiles().into_iter().enumerate() {
        let on = cur.as_deref() == Some(common::twin_for(&path, dark, follow).as_str());
        let widget = match path.strip_prefix("builtin:") {
            Some(id) => format!("wp-{id}"),
            None => format!("wp-photo-{i}"),
        };
        let b = tile_button(&path, &name, on, &widget, None);
        let pp = p.clone();
        b.connect_clicked(move |_| patch(&pp, json!({ "wallpaper": { "type": "image", "path": path, "pathDark": "" } })));
        grid.append(&b);
    }
}

// --------------------------------------------------------------------------- GNOME's catalogue and own pictures

/// GNOME's picture-options as WinPoint's `wallpaper.fit` (None: keep the fit).
pub(super) fn fit_of(options: &str) -> Option<&'static str> {
    Some(match options {
        "zoom" => "fill",
        "scaled" => "fit",
        "stretched" => "stretch",
        "wallpaper" => "tile",
        "centered" => "center",
        "spanned" => "span",
        _ => return None,
    })
}

/// The file of a catalogue item for a mode: the dark twin in dark mode when there is one.
pub(super) fn item_path(item: &Value, dark: bool) -> String {
    match item["dark"].as_str().filter(|d| dark && !d.is_empty()) {
        Some(d) => d.to_owned(),
        None => common::s(&item["path"]).to_owned(),
    }
}

/// The wallpaper keys a catalogue item writes (see [`choose_item`]); every other choice clears `pathDark`.
pub(super) fn choice_of(item: &Value, follow: bool, dark: bool) -> Value {
    let pair = item["dark"].as_str().filter(|d| !d.is_empty());
    match pair {
        Some(dark_file) if follow => json!({ "type": "image", "path": common::s(&item["path"]), "pathDark": dark_file }),
        _ => json!({ "type": "image", "path": item_path(item, dark), "pathDark": "" }),
    }
}

/// Whether a catalogue item is the current wallpaper (either file of a pair).
pub(super) fn item_is(item: &Value, current: &str) -> bool {
    !current.is_empty() && (item["path"] == current || item["dark"] == current)
}

/// "Recente afbeeldingen": the user's own copies (newest first), then the current picture and the photos chosen
/// before the copies existed (settings-app.json), at most five; (path, name, own copy).
pub(super) fn recent_tiles(own: &[Value], current: Option<&str>, legacy: &[String], catalog: &[Value]) -> Vec<(String, String, bool)> {
    let mut out: Vec<(String, String, bool)> = Vec::new();
    let name_of = |p: &str| p.rsplit('/').next().filter(|n| !n.is_empty()).unwrap_or(p).to_owned();
    for o in own {
        let path = common::s(&o["path"]);
        if !path.is_empty() {
            out.push((path.to_owned(), name_of(path), true));
        }
    }
    let known = |p: &str, out: &[(String, String, bool)]| p.starts_with("builtin:") || out.iter().any(|(x, _, _)| x == p) || catalog.iter().any(|i| item_is(i, p));
    for p in current.into_iter().map(str::to_owned).chain(legacy.iter().cloned()) {
        if !p.is_empty() && !known(&p, &out) {
            let name = name_of(&p);
            out.push((p, name, false));
        }
    }
    out.truncate(5);
    out
}

/// Choose a picture of GNOME's catalogue, with its fit: a light and dark pair as both files (`path` the light one,
/// `pathDark` the dark one), so the wallpaper follows the mode as in GNOME; while the picture does not follow the
/// mode, the file of the current mode.
fn choose_item(p: &Page, item: &Value) {
    let follow = kit::ctx().settings()["wallpaper"]["followTheme"] != false;
    let mut wallpaper = choice_of(item, follow, kit::theme::dark());
    if let Some(fit) = item["options"].as_str().and_then(fit_of) {
        wallpaper["fit"] = json!(fit);
    }
    patch(p, json!({ "wallpaper": wallpaper }));
}

fn paint_catalog(p: &Page, grid: &FillGrid, items: &[Value]) {
    grid.remove_all();
    let s = kit::ctx().settings();
    let cur = if s["wallpaper"]["type"] == "image" { s["wallpaper"]["path"].as_str().unwrap_or("").to_owned() } else { String::new() };
    let dark = kit::theme::dark();
    for (i, item) in items.iter().enumerate() {
        let slideshow = item["slideshow"] == true;
        let thumb = if slideshow { common::s(&item["preview"]).to_owned() } else { item_path(item, dark) };
        let badge = slideshow.then(|| t("Diavoorstelling", "Slideshow"));
        let b = tile_button(&thumb, common::s(&item["name"]), item_is(item, &cur), &format!("wp-gnome-{i}"), badge);
        let (pp, item) = (p.clone(), item.clone());
        b.connect_clicked(move |_| choose_item(&pp, &item));
        grid.append(&b);
    }
}

fn paint_recent(p: &Page, grid: &FillGrid, catalog: &Value, reload: Rc<dyn Fn()>) {
    grid.remove_all();
    let s = kit::ctx().settings();
    let image = s["wallpaper"]["type"] == "image";
    let cur = s["wallpaper"]["path"].as_str().filter(|_| image).unwrap_or("").to_owned();
    let own: Vec<Value> = catalog["own"].as_array().cloned().unwrap_or_default();
    let items: Vec<Value> = catalog["items"].as_array().cloned().unwrap_or_default();
    for (i, (path, name, mine)) in recent_tiles(&own, Some(cur.as_str()), &recent_photos(), &items).into_iter().enumerate() {
        let b = tile_button(&path, &name, path == cur, &format!("wp-own-{i}"), None);
        {
            let (pp, path) = (p.clone(), path.clone());
            b.connect_clicked(move |_| patch(&pp, json!({ "wallpaper": { "type": "image", "path": path, "pathDark": "" } })));
        }
        if !mine {
            grid.append(&b);
            continue;
        }
        // GNOME's remove button on an own picture (it goes to the trash)
        let tile = gtk::Overlay::new();
        tile.add_css_class("st-wp-tile");
        tile.set_child(Some(&b));
        let remove = gtk::Button::new();
        remove.add_css_class("st-wp-remove");
        remove.set_child(Some(&kit::icons::glyph("close", 10)));
        remove.set_halign(gtk::Align::End);
        remove.set_valign(gtk::Align::Start);
        remove.set_tooltip_text(Some(t("Verwijderen", "Remove")));
        remove.set_widget_name(&format!("background-remove-{i}"));
        let (pp, path2, reload2, current) = (p.clone(), path.clone(), reload.clone(), path == cur);
        remove.connect_clicked(move |_| {
            let fut = pp.run("background.remove", json!({ "path": path2 }));
            let (p2, reload3) = (pp.clone(), reload2.clone());
            pp.spawn(async move {
                if let Some(r) = fut.await
                    && r["dryRun"] != true
                {
                    // the wallpaper was this picture: WinPoint's own default again
                    if current {
                        patch(&p2, json!({ "wallpaper": { "type": "image", "path": "builtin:lotus-dark", "pathDark": "" } }));
                    }
                    reload3();
                }
            });
        });
        tile.add_overlay(&remove);
        grid.append(&tile);
    }
}

/// A titled block inside the expander (`.st-color-block`).
pub fn color_block(title: &str, content: &impl IsA<gtk::Widget>) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 0);
    b.add_css_class("st-color-block");
    b.append(&c::label(title, "st-color-block-title", true));
    b.append(content.as_ref());
    b
}

fn background(p: Page) -> PageFuture {
    Box::pin(async move {
        let catalog = p.get("background.catalog", json!({})).await.unwrap_or_else(|| json!({}));
        if !p.alive() {
            return Vec::new();
        }
        let catalog = Rc::new(RefCell::new(catalog));
        let prev = DesktopPreview::new(&PreviewOpts::default(), true);
        let s0 = kit::ctx().settings();
        let is_color = |s: &Value| s["wallpaper"]["type"] == "color";

        let type_dd = Dropdown::new(
            vec![DropdownOption::new("image", t("Afbeelding", "Picture")), DropdownOption::new("color", t("Effen kleur", "Solid colour"))],
            Some(if is_color(&s0) { "color" } else { "image" }),
        );
        type_dd.widget().set_widget_name("background-type");
        {
            let pp = p.clone();
            type_dd.connect_changed(move |v| patch(&pp, json!({ "wallpaper": { "type": v } })));
        }

        // ---- the pictures
        let grid = FillGrid::new(104, 8, 8, 760);
        grid.add_css_class("st-wp-grid");
        grid.set_widget_name("background-pictures");
        let recent_grid = FillGrid::new(104, 8, 8, 760);
        recent_grid.add_css_class("st-wp-grid");
        recent_grid.set_widget_name("background-recent");
        let gnome_grid = FillGrid::new(104, 8, 8, 760);
        gnome_grid.add_css_class("st-wp-grid");
        gnome_grid.set_widget_name("background-gnome");
        // the catalogue again (an own picture added or removed, here or by GNOME Settings) and the grids with it
        let repaint: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
        let reload: Rc<dyn Fn()> = {
            let (pp, catalog, repaint) = (p.clone(), catalog.clone(), repaint.clone());
            Rc::new(move || {
                let fut = pp.get("background.catalog", json!({}));
                let (catalog, repaint) = (catalog.clone(), repaint.clone());
                pp.spawn(async move {
                    if let Some(v) = fut.await {
                        *catalog.borrow_mut() = v;
                        if let Some(f) = repaint.borrow().clone() {
                            f();
                        }
                    }
                });
            })
        };
        {
            let reload = reload.clone();
            p.on("background.changed", move |_| reload());
        }
        let browse = c::button(t("Door foto’s bladeren", "Browse photos"), Variant::Standard);
        browse.set_widget_name("background-browse");
        {
            let (pp, reload) = (p.clone(), reload.clone());
            // GTK's file dialog (the desktop's FileChooser portal: Win11's picker when it is chosen).
            browse.connect_clicked(move |_| {
                let filter = gtk::FileFilter::new();
                filter.set_name(Some(t("Afbeeldingen", "Pictures")));
                for pattern in ["*.jpg", "*.jpeg", "*.png", "*.webp", "*.gif", "*.bmp", "*.svg"] {
                    filter.add_pattern(pattern);
                }
                filter.add_mime_type("image/*");
                let filters = gio::ListStore::new::<gtk::FileFilter>();
                filters.append(&filter);
                let dialog = gtk::FileDialog::new();
                dialog.set_title(t("Een foto kiezen voor je bureaublad", "Choose a photo for your desktop"));
                dialog.set_modal(true);
                dialog.set_filters(Some(&filters));
                dialog.set_default_filter(Some(&filter));
                let (p2, reload) = (pp.clone(), reload.clone());
                dialog.open(Some(pp.window().window()), gio::Cancellable::NONE, move |result| match result {
                    Ok(file) => {
                        if let Some(path) = file.path().and_then(|p| p.to_str().map(str::to_owned)) {
                            // GNOME's Add Picture: a copy in ~/.local/share/backgrounds becomes the wallpaper
                            let fut = p2.call("background.add", json!({ "path": path }));
                            let (p3, reload) = (p2.clone(), reload.clone());
                            p2.spawn(async move {
                                match fut.await {
                                    Ok(r) if r["dryRun"] == true => common::note_dry_run(&p3.anchor(), &r),
                                    Ok(r) => {
                                        let copy = r["path"].as_str().unwrap_or(&path).to_owned();
                                        patch(&p3, json!({ "wallpaper": { "type": "image", "path": copy, "pathDark": "" } }));
                                        reload();
                                    }
                                    // an older host without background.*: the photo itself, as before
                                    Err(e) if e.code == "unknown_method" => {
                                        push_photo(&path);
                                        patch(&p3, json!({ "wallpaper": { "type": "image", "path": path, "pathDark": "" } }));
                                    }
                                    Err(e) => p3.error(&e, None),
                                }
                            });
                        }
                    }
                    Err(e) if e.matches(gtk::DialogError::Dismissed) || e.matches(gtk::DialogError::Cancelled) => {}
                    Err(e) => p2.error(
                        &kit::CallError::new("picker", e.message()),
                        None,
                    ),
                });
            });
        }

        // ---- the solid colours
        let colors = ColorSwatches::new(Some(SOLID), s0["wallpaper"]["color"].as_str(), 8, 40);
        colors.widget().set_widget_name("background-colors");
        {
            let pp = p.clone();
            colors.connect_selected(move |c| patch(&pp, json!({ "wallpaper": { "type": "color", "color": c } })));
        }
        let custom = c::button(t("Kleuren weergeven", "View colours"), Variant::Standard);
        custom.set_widget_name("background-custom");
        {
            let pp = p.clone();
            custom.connect_clicked(move |_| {
                let cur = common::setting_str("wallpaper.color").unwrap_or_else(|| "#0b2a4a".into());
                let p2 = pp.clone();
                crate::colorpicker::pick_color(&pp.anchor(), &cur, None, false, move |hex| {
                    if let Some(n) = hex.as_deref().and_then(kit::theme::normalize_hex) {
                        patch(&p2, json!({ "wallpaper": { "type": "color", "color": n } }));
                    }
                });
            });
        }

        // Windows' own en-GB wording: "Recent images" (tools/i18n/glossary-context.tsv)
        let recent_block = color_block(t("Recente afbeeldingen", "Recent images"), &recent_grid);
        let pictures_block = color_block(t("Afbeeldingen", "Pictures"), &grid);
        let gnome_block = color_block(t("Afbeeldingen van Ubuntu en GNOME", "Ubuntu and GNOME pictures"), &gnome_grid);
        let choose_card = SettingsCard::new("", t("Een foto kiezen", "Choose a photo"), "");
        choose_card.set_control(&browse);
        let colors_block = color_block(t("Kies je achtergrondkleur", "Choose your background colour"), colors.widget());
        let custom_card = SettingsCard::new("", t("Aangepaste kleuren", "Custom colours"), "");
        custom_card.set_control(&custom);
        let fit_dd = common::setting_dropdown(
            &p,
            "wallpaper.fit",
            vec![
                (json!("fill"), t("Opvullen", "Fill").into()),
                (json!("fit"), t("Passend maken", "Fit").into()),
                (json!("stretch"), t("Uitrekken", "Stretch").into()),
                (json!("tile"), t("Tegel", "Tile").into()),
                (json!("center"), t("Centreren", "Centre").into()),
                (json!("span"), t("Uitspreiden", "Span").into()),
            ],
            None,
        );
        fit_dd.widget().set_widget_name("background-fit");
        // Windows 11: the fit is a row of "Je achtergrond aanpassen", after Een foto kiezen
        let fit_card = SettingsCard::new("", t("Kies hoe de afbeelding het scherm vult", "Choose a fit for your desktop image"), "");
        fit_card.set_control(&common::sized(fit_dd.widget(), 180));
        let exp = SettingsExpander::new(
            "image",
            t("Je achtergrond aanpassen", "Personalise your background"),
            t("Een afbeelding of een effen kleur op je bureaublad", "A picture or a solid colour on your desktop"),
        );
        exp.header().set_control(&common::sized(type_dd.widget(), 180));
        exp.set_expanded(true);
        exp.add(&recent_block);
        exp.add(choose_card.widget());
        exp.add(fit_card.widget());
        exp.add(&pictures_block);
        exp.add(&gnome_block);
        exp.add(&colors_block);
        exp.add(custom_card.widget());

        let (follow_card, follow_sw) = common::toggle_card(
            &p,
            "theme",
            t("Afbeelding aanpassen aan licht en donker", "Match the picture to light and dark"),
            t(
                "Lotus en Stroming hebben een lichte en een donkere versie die meewisselt met je modus",
                "Lotus and Flow have a light and a dark version that switches with your mode",
            ),
            "wallpaper.followTheme",
            false,
        );
        follow_sw.switch().set_widget_name("background-follow");

        let sync = {
            let (pp, grid, prev, type_dd, colors) = (p.clone(), grid.clone(), prev.clone(), type_dd.clone(), colors.clone());
            let (recent_grid, gnome_grid, catalog, reload) = (recent_grid.clone(), gnome_grid.clone(), catalog.clone(), reload.clone());
            let (recent_w, gnome_w): (gtk::Widget, gtk::Widget) = (recent_block.clone().upcast(), gnome_block.clone().upcast());
            let image_parts: Vec<gtk::Widget> = vec![pictures_block.upcast(), choose_card.widget().clone()];
            let color_parts: Vec<gtk::Widget> = vec![colors_block.upcast(), custom_card.widget().clone()];
            let (fit_w, follow_w) = (fit_card.widget().clone(), follow_card.widget().clone());
            move || {
                let s = kit::ctx().settings();
                let color = is_color(&s);
                type_dd.set_selected(Some(if color { "color" } else { "image" }));
                for w in &image_parts {
                    w.set_visible(!color);
                }
                for w in &color_parts {
                    w.set_visible(color);
                }
                fit_w.set_visible(!color);
                follow_w.set_visible(!color);
                colors.set_selected(s["wallpaper"]["color"].as_str().unwrap_or(""));
                prev.update(&PreviewOpts::default());
                paint_grid(&pp, &grid);
                let cat = catalog.borrow().clone();
                paint_recent(&pp, &recent_grid, &cat, reload.clone());
                recent_w.set_visible(!color && recent_grid.first_child().is_some());
                let items: Vec<Value> = cat["items"].as_array().cloned().unwrap_or_default();
                paint_catalog(&pp, &gnome_grid, &items);
                gnome_w.set_visible(!color && !items.is_empty());
            }
        };
        let sync: Rc<dyn Fn()> = Rc::new(sync);
        *repaint.borrow_mut() = Some(sync.clone());
        sync();
        p.on_settings(move |_, _| sync());

        let main = SettingsGroup::new("", "");
        main.add(exp.widget());
        main.add(follow_card.widget());
        let related = SettingsGroup::new(common::ct::related(), "");
        related.add(common::link_card(&p, "colors").widget());
        related.add(common::link_card(&p, "themes").widget());
        vec![Clamp::new(&prev, 384).upcast(), main.widget().clone().upcast(), related.widget().clone().upcast()]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gnome_pictures_choose_as_gnome_settings() {
        for (options, fit) in [("zoom", Some("fill")), ("scaled", Some("fit")), ("wallpaper", Some("tile")), ("spanned", Some("span")), ("none", None), ("", None)] {
            assert_eq!(fit_of(options), fit, "{options}");
        }
        let pair = json!({"path": "/usr/share/backgrounds/l.png", "dark": "/usr/share/backgrounds/d.png"});
        assert_eq!(item_path(&pair, false), "/usr/share/backgrounds/l.png");
        assert_eq!(item_path(&pair, true), "/usr/share/backgrounds/d.png", "the dark twin in dark mode");
        let single = json!({"path": "/usr/share/backgrounds/x.jpg", "dark": null});
        assert_eq!(item_path(&single, true), "/usr/share/backgrounds/x.jpg");
        assert!(item_is(&pair, "/usr/share/backgrounds/d.png") && item_is(&pair, "/usr/share/backgrounds/l.png"));
        assert!(!item_is(&pair, "") && !item_is(&single, "/usr/share/backgrounds/l.png"));
        // a pair writes both files; a single picture, or a pair while the picture does not follow the mode, clears
        // pathDark
        assert_eq!(
            choice_of(&pair, true, false),
            json!({"type": "image", "path": "/usr/share/backgrounds/l.png", "pathDark": "/usr/share/backgrounds/d.png"})
        );
        assert_eq!(choice_of(&pair, true, true), choice_of(&pair, true, false), "the same in either mode");
        assert_eq!(choice_of(&pair, false, true), json!({"type": "image", "path": "/usr/share/backgrounds/d.png", "pathDark": ""}));
        assert_eq!(choice_of(&single, true, true), json!({"type": "image", "path": "/usr/share/backgrounds/x.jpg", "pathDark": ""}));
        // the page's own resolver takes the dark file in dark mode (host/src/wallpaper.rs dark_file)
        let s = json!({"wallpaper": {"type": "image", "path": "/l.png", "pathDark": "/d.png"}});
        assert_eq!(common::resolve_wallpaper_path(&s, true), "/d.png");
        assert_eq!(common::resolve_wallpaper_path(&s, false), "/l.png");
        let fixed = json!({"wallpaper": {"type": "image", "path": "/l.png", "pathDark": "/d.png", "followTheme": false}});
        assert_eq!(common::resolve_wallpaper_path(&fixed, true), "/l.png");
        let builtin = json!({"wallpaper": {"type": "image", "path": "builtin:lotus-light", "pathDark": "/d.png"}});
        assert_eq!(common::resolve_wallpaper_path(&builtin, true), "builtin:lotus-dark");
        let old = json!({"wallpaper": {"type": "image", "path": "/l.png"}});
        assert_eq!(common::resolve_wallpaper_path(&old, true), "/l.png");
    }

    #[test]
    fn recent_images_are_the_own_copies_first() {
        let own = vec![json!({"path": "/h/.local/share/backgrounds/2026-11-16-10-00-00-b.png"}), json!({"path": "/h/.local/share/backgrounds/2026-11-15-10-00-00-a.png"})];
        let catalog = vec![json!({"path": "/usr/share/backgrounds/l.png", "dark": "/usr/share/backgrounds/d.png"})];
        let legacy = vec!["/h/Pictures/old.jpg".to_owned(), "/h/.local/share/backgrounds/2026-11-15-10-00-00-a.png".to_owned()];
        let tiles = recent_tiles(&own, Some("/h/Pictures/now.jpg"), &legacy, &catalog);
        let paths: Vec<&str> = tiles.iter().map(|(p, _, _)| p.as_str()).collect();
        assert_eq!(
            paths,
            ["/h/.local/share/backgrounds/2026-11-16-10-00-00-b.png", "/h/.local/share/backgrounds/2026-11-15-10-00-00-a.png", "/h/Pictures/now.jpg", "/h/Pictures/old.jpg"]
        );
        assert_eq!(tiles.iter().map(|t| t.2).collect::<Vec<_>>(), [true, true, false, false], "only the copies can be removed");
        assert_eq!(tiles[2].1, "now.jpg");
        // a built-in or a catalogue picture is no recent image; at most five
        assert!(recent_tiles(&[], Some("builtin:lotus-dark"), &[], &catalog).is_empty());
        assert!(recent_tiles(&[], Some("/usr/share/backgrounds/d.png"), &[], &catalog).is_empty());
        let many: Vec<Value> = (0..9).map(|i| json!({"path": format!("/b/{i}.png")})).collect();
        assert_eq!(recent_tiles(&many, None, &[], &[]).len(), 5);
    }
}
