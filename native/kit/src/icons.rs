//! Icons: the UI's Fluent-style line glyphs (6ebf169f:vendor/ui/js/core/icons.js, every name of `iconNames()`, plus
//! the shell's "start-logo", "start-logo-mono", "task-view", "battery-<0..100>", "battery-charging-<0..100>" and the
//! caption glyphs "cap-min", "cap-max", "cap-restore", "cap-close") drawn with cairo as symbolic paintables, so
//! they take the widget's CSS `color` like a symbolic theme icon; app icons (icon theme, file, or the backend's
//! `/icon`), file icons (theme icon or MIME type), thumbnails through the backend's `/thumb`, and textures.
use crate::glyph_data::{self, Source};
use crate::svg;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{cairo, gdk, gdk_pixbuf, gio, glib, graphene};
use serde_json::Value;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

/// The glyphs' stroke width on their 20 x 20 grid (components.css `--icon-stroke`).
pub const STROKE: f64 = 1.35;

/// A Fluent glyph by name ("search", "folder", "chevron-down", "settings", ...), `px` logical pixels square,
/// in the widget's text colour. Unknown names draw icons.js's "?" template (and warn once).
pub fn glyph(name: &str, px: i32) -> gtk::Image {
    glyph_stroke(name, px, STROKE)
}

/// `glyph` with another stroke width (components.css overrides such as `--icon-stroke: 1.6`).
pub fn glyph_stroke(name: &str, px: i32, stroke: f64) -> gtk::Image {
    let image = gtk::Image::new();
    let paintable = Glyph::new(name, px, stroke, image.upcast_ref());
    image.set_paintable(Some(&paintable));
    image.set_pixel_size(px);
    image.add_css_class("w11-glyph");
    image.set_can_target(false);
    image
}

/// Show another glyph in an image made by `glyph` (any other image becomes a glyph image).
pub fn set_glyph(image: &gtk::Image, name: &str) {
    if let Some(g) = image.paintable().and_downcast::<Glyph>() {
        g.set_name(name);
        return;
    }
    let px = if image.pixel_size() > 0 { image.pixel_size() } else { 16 };
    let paintable = Glyph::new(name, px, STROKE, image.upcast_ref());
    image.set_paintable(Some(&paintable));
    image.set_pixel_size(px);
    image.add_css_class("w11-glyph");
}

/// The glyph an image from `glyph` shows.
pub fn glyph_name(image: &gtk::Image) -> Option<String> {
    image.paintable().and_downcast::<Glyph>().map(|g| g.imp().name.borrow().clone())
}

/// A glyph as a paintable (for gtk::Picture, entry icons, ...); `owner` gives the colour when it is drawn as a
/// plain (not symbolic) paintable.
pub fn glyph_paintable(name: &str, px: i32, owner: &impl IsA<gtk::Widget>) -> gdk::Paintable {
    Glyph::new(name, px, STROKE, owner.upcast_ref()).upcast()
}

/// Whether there is a glyph of this name.
pub fn has_glyph(name: &str) -> bool {
    glyph_data::source(name, true).is_some()
}

/// Every glyph name of icons.js plus the extras (levels as their 100 % form).
pub fn glyph_names() -> Vec<&'static str> {
    let mut names: Vec<&'static str> = glyph_data::NAMES.to_vec();
    names.extend(["start-logo", "start-logo-mono", "task-view", "battery-100", "battery-charging-100"]);
    // the settings-parity pages' glyphs (glyph_data.rs)
    names.extend(["touchpad", "pen", "vpn", "magnifier", "narrator", "text-cursor", "remote-desktop"]);
    names
}

/// Draw a glyph with cairo into a square at (x, y) of `size`, in `color` (custom drawing: charts, cells).
pub fn draw_glyph(cr: &cairo::Context, name: &str, x: f64, y: f64, size: f64, color: &gdk::RGBA) {
    let Some((tree, src)) = parsed(name, crate::theme::dark()) else { return };
    cr.save().ok();
    cr.translate(x, y);
    cr.scale(size / src.view, size / src.view);
    svg::render(
        cr,
        &tree,
        (color.red() as f64, color.green() as f64, color.blue() as f64, color.alpha() as f64),
        src.outline,
        src.fixed_stroke.unwrap_or(STROKE),
    );
    cr.restore().ok();
}

/// Wi-Fi glyph for a signal strength 0..100 (icons.js wifiIcon); None is plain "wifi".
pub fn wifi_icon(strength: Option<f64>) -> &'static str {
    match strength {
        None => "wifi",
        Some(s) if s >= 67.0 => "wifi-3",
        Some(s) if s >= 34.0 => "wifi-2",
        Some(_) => "wifi-1",
    }
}

/// Speaker glyph for a volume 0..100 and mute (icons.js volumeIcon).
pub fn volume_icon(volume: f64, muted: bool) -> &'static str {
    if muted || volume <= 0.0 {
        "volume-mute"
    } else if volume < 34.0 {
        "volume-1"
    } else if volume < 67.0 {
        "volume-2"
    } else {
        "volume"
    }
}

/// The battery glyph for a charge 0..100 (taskbar.js batteryGlyph).
pub fn battery_icon(percent: f64, charging: bool) -> String {
    let p = if percent.is_finite() { percent.round().clamp(0.0, 100.0) as i64 } else { 0 };
    if charging { format!("battery-charging-{p}") } else { format!("battery-{p}") }
}

/// Redraw every live glyph (the theme does this after a light/dark switch).
pub fn invalidate() {
    LIVE.with(|live| {
        let mut live = live.borrow_mut();
        live.retain(|w| w.upgrade().is_some());
        for g in live.iter().filter_map(|w| w.upgrade()) {
            g.invalidate_contents();
        }
    });
}

// ------------------------------------------------------------------------------------- parsed glyph cache

type Parsed = (Rc<svg::Node>, Rc<Source>);

thread_local! {
    static PARSED: RefCell<HashMap<(String, bool), Option<Parsed>>> = RefCell::new(HashMap::new());
    static LIVE: RefCell<Vec<glib::WeakRef<Glyph>>> = const { RefCell::new(Vec::new()) };
    static WARNED: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
}

fn parsed(name: &str, dark: bool) -> Option<Parsed> {
    let key = (name.to_owned(), dark);
    if let Some(hit) = PARSED.with(|p| p.borrow().get(&key).cloned()) {
        return hit;
    }
    let src = glyph_data::source(name, dark).or_else(|| {
        WARNED.with(|w| {
            if w.borrow_mut().insert(name.to_owned()) {
                eprintln!("win11-kit: unknown glyph \"{name}\"");
            }
        });
        Some(glyph_data::unknown())
    });
    let entry = src.map(|s| (Rc::new(svg::parse(&s.body)), Rc::new(s)));
    PARSED.with(|p| {
        let mut p = p.borrow_mut();
        if p.len() > 800 {
            p.clear();
        }
        p.insert(key, entry.clone());
    });
    entry
}

// --------------------------------------------------------------------------------------------- the paintable

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct Glyph {
        pub name: RefCell<String>,
        pub size: Cell<i32>,
        pub stroke: Cell<f64>,
        pub owner: glib::WeakRef<gtk::Widget>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Glyph {
        const NAME: &'static str = "W11KitGlyph";
        type Type = super::Glyph;
        type Interfaces = (gdk::Paintable, gtk::SymbolicPaintable);
    }

    impl ObjectImpl for Glyph {}

    impl PaintableImpl for Glyph {
        fn intrinsic_width(&self) -> i32 {
            self.size.get()
        }
        fn intrinsic_height(&self) -> i32 {
            self.size.get()
        }
        fn flags(&self) -> gdk::PaintableFlags {
            gdk::PaintableFlags::STATIC_SIZE
        }
        fn snapshot(&self, snapshot: &gdk::Snapshot, width: f64, height: f64) {
            let color = self.owner.upgrade().map(|w| w.color()).unwrap_or(gdk::RGBA::WHITE);
            self.obj().draw(snapshot, width, height, &color);
        }
    }

    impl SymbolicPaintableImpl for Glyph {
        fn snapshot_symbolic(&self, snapshot: &gdk::Snapshot, width: f64, height: f64, colors: &[gdk::RGBA]) {
            let color = colors
                .first()
                .copied()
                .or_else(|| self.owner.upgrade().map(|w| w.color()))
                .unwrap_or(gdk::RGBA::WHITE);
            self.obj().draw(snapshot, width, height, &color);
        }

        // GTK 4.22 calls snapshot_with_weight; gtk4-rs's default chains to the parent interface, whose slot is
        // empty, and panics in an extern "C" frame (abort). Draw it ourselves (native/shell/src/icons.rs).
        fn snapshot_with_weight(
            &self,
            snapshot: &gdk::Snapshot,
            width: f64,
            height: f64,
            colors: &[gdk::RGBA],
            _weight: f64,
        ) {
            self.snapshot_symbolic(snapshot, width, height, colors);
        }
    }
}

glib::wrapper! {
    /// A glyph paintable (see `glyph`).
    pub struct Glyph(ObjectSubclass<imp::Glyph>) @implements gdk::Paintable, gtk::SymbolicPaintable;
}

impl Glyph {
    fn new(name: &str, px: i32, stroke: f64, owner: &gtk::Widget) -> Glyph {
        let g: Glyph = glib::Object::new();
        let imp = g.imp();
        *imp.name.borrow_mut() = name.to_owned();
        imp.size.set(px.max(1));
        imp.stroke.set(if stroke > 0.0 { stroke } else { STROKE });
        imp.owner.set(Some(owner));
        let _ = parsed(name, crate::theme::dark());
        LIVE.with(|live| {
            let mut live = live.borrow_mut();
            if live.len() > 512 && live.len().is_power_of_two() {
                live.retain(|w| w.upgrade().is_some());
            }
            live.push(g.downgrade());
        });
        g
    }

    fn set_name(&self, name: &str) {
        if *self.imp().name.borrow() == name {
            return;
        }
        *self.imp().name.borrow_mut() = name.to_owned();
        self.invalidate_contents();
    }

    fn draw(&self, snapshot: &gdk::Snapshot, width: f64, height: f64, color: &gdk::RGBA) {
        let Some(snapshot) = snapshot.downcast_ref::<gtk::Snapshot>() else { return };
        let imp = self.imp();
        let side = width.min(height);
        if side <= 0.0 {
            return;
        }
        let name = imp.name.borrow().clone();
        let Some((tree, src)) = parsed(&name, crate::theme::dark()) else { return };
        let rect = graphene::Rect::new(0.0, 0.0, width as f32, height as f32);
        let cr = snapshot.append_cairo(&rect);
        cr.translate((width - side) / 2.0, (height - side) / 2.0);
        cr.scale(side / src.view, side / src.view);
        let stroke = src.fixed_stroke.unwrap_or(imp.stroke.get());
        let c = (color.red() as f64, color.green() as f64, color.blue() as f64, color.alpha() as f64);
        let c = if src.tinted { c } else { (c.0, c.1, c.2, 1.0) };
        svg::render(&cr, &tree, c, src.outline, stroke);
    }
}

// ------------------------------------------------------------------------------------------------ app icons

/// An app icon: a theme icon name (with or without ".desktop", snap "name_name" folded, reverse-DNS tail), an
/// absolute path or file:// URI, a name in the pixmaps folders, and otherwise the backend's `/icon` (which knows
/// the host's own apps); "application-x-executable" until then.
pub fn app_icon(icon: &str, px: i32) -> gtk::Image {
    let image = gtk::Image::new();
    image.set_pixel_size(px);
    image.add_css_class("w11-app-icon");
    let name = icon.trim();
    match resolve_local(name) {
        Some(Local::Name(n)) => set_theme_icon(&image, &n),
        Some(Local::File(path)) => image.set_from_file(Some(&path)),
        None => {
            image.set_icon_name(Some("application-x-executable"));
            if !name.is_empty() {
                fetch_into(&image, format!("/icon?name={}&size={}&fallback=application-x-executable", crate::backend::encode(name), fetch_size(px)));
            }
        }
    }
    image
}

// ------------------------------------------------------------------------------------------------ theme icons

/// One image [`set_theme_icon`] filled: shown again when the icon theme changes, as long as it still shows that.
struct Themed {
    image: glib::WeakRef<gtk::Image>,
    name: String,
    file: Option<gio::File>,
}

thread_local! {
    /// [`set_theme_icon`]'s images by address (pruned of dead ones when the map doubles).
    static THEMED: RefCell<(HashMap<usize, Themed>, usize)> = RefCell::new((HashMap::new(), 64));
    /// name → the theme's scalable SVG (None: GTK's own choice), until the icon theme changes.
    static SCALABLE: RefCell<HashMap<String, Option<gio::File>>> = RefCell::new(HashMap::new());
    static WATCHING: Cell<bool> = const { Cell::new(false) };
}

/// Show the theme icon `name` the way the host's `/icon` picks one (host/src/images/icons.rs `score`): the theme's
/// scalable SVG before its fixed sizes. Win11 draws its fixed 16-24 px places and devices icons as monochrome line
/// icons and its coloured ones from 32 px on (places/scalable and devices/scalable have MinSize=32), so at 16 px
/// GTK's own pick for a folder or a drive is the line icon; the WebKit apps showed the coloured one through
/// `/icon`, and so does this (GTK renders the SVG at the image's size and scale). A name without a scalable SVG keeps
/// GTK's pick. The image follows a change of the icon theme (Win11 / Win11-dark with the colour scheme).
pub fn set_theme_icon(image: &gtk::Image, name: &str) {
    let file = show_theme_icon(image, name);
    THEMED.with(|t| {
        let (map, prune_at) = &mut *t.borrow_mut();
        if map.len() >= *prune_at {
            map.retain(|_, e| e.image.upgrade().is_some());
            *prune_at = (map.len() * 2).max(64);
        }
        map.insert(image.as_ptr() as usize, Themed { image: image.downgrade(), name: name.to_owned(), file });
    });
}

fn show_theme_icon(image: &gtk::Image, name: &str) -> Option<gio::File> {
    let file = scalable(name);
    match &file {
        Some(file) => image.set_from_gicon(&gio::FileIcon::new(file)),
        None => image.set_icon_name(Some(name)),
    }
    file
}

/// The theme's scalable SVG for `name`: what GTK picks for a large size, when that is an SVG in a "scalable" folder.
fn scalable(name: &str) -> Option<gio::File> {
    if let Some(hit) = SCALABLE.with(|s| s.borrow().get(name).cloned()) {
        return hit;
    }
    let theme = gtk::IconTheme::for_display(&gdk::Display::default()?);
    watch_theme(&theme);
    let file = theme
        .lookup_icon(name, &[], 512, 1, gtk::TextDirection::None, gtk::IconLookupFlags::empty())
        .file()
        .filter(|f| {
            f.path().is_some_and(|p| {
                p.extension().is_some_and(|e| e == "svg") && p.parent().and_then(|d| d.file_name()).is_some_and(|d| d == "scalable")
            })
        });
    SCALABLE.with(|s| s.borrow_mut().insert(name.to_owned(), file.clone()));
    file
}

/// Whether `image` still shows what [`set_theme_icon`] gave it (no thumbnail, fetched or other icon since).
fn still_shows(image: &gtk::Image, e: &Themed) -> bool {
    match &e.file {
        Some(file) => image
            .gicon()
            .and_then(|g| g.downcast::<gio::FileIcon>().ok())
            .is_some_and(|g| g.file().equal(file)),
        None => image.storage_type() == gtk::ImageType::IconName && image.icon_name().as_deref() == Some(e.name.as_str()),
    }
}

fn watch_theme(theme: &gtk::IconTheme) {
    if WATCHING.replace(true) {
        return;
    }
    theme.connect_changed(|_| {
        SCALABLE.with(|s| s.borrow_mut().clear());
        let live: Vec<(usize, gtk::Image, String)> = THEMED.with(|t| {
            let (map, _) = &mut *t.borrow_mut();
            map.retain(|_, e| e.image.upgrade().is_some_and(|image| still_shows(&image, e)));
            map.iter().filter_map(|(k, e)| Some((*k, e.image.upgrade()?, e.name.clone()))).collect()
        });
        for (key, image, name) in live {
            let file = show_theme_icon(&image, &name);
            THEMED.with(|t| {
                if let Some(e) = t.borrow_mut().0.get_mut(&key) {
                    e.file = file;
                }
            });
        }
    });
}

/// A file icon: a theme icon name ("folder", "user-home", "drive-harddisk", through [`set_theme_icon`]) or a MIME type
/// ("image/png"), from the icon theme, else the backend's `/icon`; the "file" glyph when nothing is found.
pub fn file_icon(icon_or_mime: &str, px: i32) -> gtk::Image {
    let image = gtk::Image::new();
    image.set_pixel_size(px);
    image.add_css_class("w11-file-icon");
    let spec = icon_or_mime.trim();
    if spec.is_empty() {
        set_glyph(&image, "file");
        return image;
    }
    let theme = gdk::Display::default().map(|d| gtk::IconTheme::for_display(&d));
    if spec.contains('/') && !spec.starts_with('/') {
        let gicon = gio::content_type_get_icon(spec);
        if theme.as_ref().is_some_and(|t| t.has_gicon(&gicon)) {
            image.set_from_gicon(&gicon);
        } else {
            image.set_icon_name(Some(if spec.starts_with("inode/directory") { "folder" } else { "text-x-generic" }));
            fetch_into(&image, format!("/icon?mime={}&size={}", crate::backend::encode(spec), fetch_size(px)));
        }
        return image;
    }
    match resolve_local(spec) {
        Some(Local::Name(n)) => set_theme_icon(&image, &n),
        Some(Local::File(p)) => image.set_from_file(Some(&p)),
        None => {
            image.set_icon_name(Some("text-x-generic"));
            fetch_into(&image, format!("/icon?name={}&size={}&fallback=text-x-generic", crate::backend::encode(spec), fetch_size(px)));
        }
    }
    image
}

/// Whether a file item (Explorer / Alles row: {path, name, mime, isDir}) gets a thumbnail (explorer/util.js
/// canThumb).
pub fn can_thumb(item: &Value) -> bool {
    if item["isDir"] == true {
        return false;
    }
    let mime = item["mime"].as_str().unwrap_or("");
    if mime.starts_with("image/") || mime.starts_with("video/") || mime == "application/pdf" {
        return true;
    }
    let name = item["name"].as_str().or(item["path"].as_str()).unwrap_or("").to_lowercase();
    const EXT: &[&str] = &[
        ".png", ".jpg", ".jpeg", ".gif", ".webp", ".bmp", ".svg", ".avif", ".heic", ".tif", ".tiff", ".mp4", ".mkv",
        ".webm", ".mov", ".avi", ".pdf",
    ];
    EXT.iter().any(|e| name.ends_with(e))
}

/// The icon of a file item (explorer/util.js fileIcon): with `thumb` and a thumbnailable file the backend's
/// thumbnail, else item.icon, the MIME icon, "folder" for directories, and the "folder" / "file" glyph last.
pub fn file_item_icon(item: &Value, px: i32, thumb: bool) -> gtk::Image {
    let is_dir = item["isDir"] == true;
    let image = match item["icon"].as_str().filter(|s| !s.is_empty()) {
        Some(icon) => file_icon(icon, px),
        None if is_dir => file_icon("folder", px),
        None => file_icon(item["mime"].as_str().unwrap_or("application/octet-stream"), px),
    };
    if thumb && can_thumb(item)
        && let Some(path) = item["path"].as_str()
    {
        let weak = image.downgrade();
        thumbnail(path, px.max(64), move |tex| {
            if let (Some(img), Some(tex)) = (weak.upgrade(), tex) {
                img.set_paintable(Some(&tex));
                img.add_css_class("w11-thumb");
            }
        });
    }
    image
}

/// A thumbnail of a local file through the backend (`/thumb?path=..&size=..`, cached on disk by the host),
/// `px` logical pixels (fetched at twice that for HiDPI); `done` runs on the main thread, None when there is
/// none. Recent results are cached in memory.
pub fn thumbnail(path: &str, px: i32, done: impl FnOnce(Option<gdk::Texture>) + 'static) {
    let size = (px * 2).clamp(32, 1024);
    let query = format!("/thumb?path={}&size={size}", crate::backend::encode(path));
    fetch(query, done);
}

/// GET an image from the backend (any path and query) as a texture, cached; `done` on the main thread.
pub fn fetch(query: String, done: impl FnOnce(Option<gdk::Texture>) + 'static) {
    if let Some(known) = REMOTE.with(|r| r.borrow().get(&query).cloned()) {
        done(known);
        return;
    }
    let first = PENDING.with(|p| {
        let mut p = p.borrow_mut();
        let waiting = p.entry(query.clone()).or_default();
        waiting.push(Box::new(done));
        waiting.len() == 1
    });
    if !first {
        return;
    }
    let Some(ctx) = crate::ctx::try_ctx() else {
        let waiting = PENDING.with(|p| p.borrow_mut().remove(&query)).unwrap_or_default();
        for f in waiting {
            f(None);
        }
        return;
    };
    let backend = ctx.backend();
    glib::spawn_future_local(async move {
        let q = query.clone();
        let bytes = gio::spawn_blocking(move || backend.get(&q)).await.ok().and_then(|r| r.ok());
        let tex = bytes.as_deref().and_then(texture);
        REMOTE.with(|r| {
            let mut r = r.borrow_mut();
            if r.len() > 600 {
                r.clear();
            }
            r.insert(query.clone(), tex.clone());
        });
        let waiting = PENDING.with(|p| p.borrow_mut().remove(&query)).unwrap_or_default();
        for f in waiting {
            f(tex.clone());
        }
    });
}

fn fetch_into(image: &gtk::Image, query: String) {
    let weak = image.downgrade();
    fetch(query, move |tex| {
        if let (Some(img), Some(tex)) = (weak.upgrade(), tex) {
            img.set_paintable(Some(&tex));
        }
    });
}

fn fetch_size(px: i32) -> i32 {
    (px.max(16) * 2).clamp(32, 256)
}

type Waiting = Vec<Box<dyn FnOnce(Option<gdk::Texture>)>>;

thread_local! {
    static REMOTE: RefCell<HashMap<String, Option<gdk::Texture>>> = RefCell::new(HashMap::new());
    static PENDING: RefCell<HashMap<String, Waiting>> = RefCell::new(HashMap::new());
}

/// Encoded image bytes (PNG, JPEG, SVG, ICO, ...) as a texture.
pub fn texture(bytes: &[u8]) -> Option<gdk::Texture> {
    if bytes.is_empty() {
        return None;
    }
    if let Ok(t) = gdk::Texture::from_bytes(&glib::Bytes::from(bytes)) {
        return Some(t);
    }
    let loader = gdk_pixbuf::PixbufLoader::new();
    if loader.write(bytes).is_err() {
        let _ = loader.close();
        return None;
    }
    loader.close().ok()?;
    let pixbuf = loader.pixbuf()?;
    #[allow(deprecated)]
    Some(gdk::Texture::for_pixbuf(&pixbuf))
}

enum Local {
    Name(String),
    File(String),
}

fn resolve_local(icon: &str) -> Option<Local> {
    if icon.is_empty() {
        return None;
    }
    let path = icon.strip_prefix("file://").unwrap_or(icon);
    if path.starts_with('/') {
        return std::path::Path::new(path).is_file().then(|| Local::File(path.to_owned()));
    }
    let theme = gdk::Display::default().map(|d| gtk::IconTheme::for_display(&d));
    let base = fold_snap(icon.trim_end_matches(".desktop"));
    let mut candidates = vec![base.to_owned()];
    let lower = base.to_lowercase();
    if lower != base {
        candidates.push(lower);
    }
    if let Some(last) = base.rsplit('.').next().filter(|l| *l != base && !l.is_empty()) {
        candidates.push(last.to_owned());
        candidates.push(last.to_lowercase());
    }
    if let Some(theme) = &theme {
        for c in &candidates {
            if theme.has_icon(c) {
                return Some(Local::Name(c.clone()));
            }
        }
    }
    if base.contains('/') {
        return None;
    }
    for dir in ["/usr/share/pixmaps", "/usr/local/share/pixmaps"] {
        for c in &candidates {
            for ext in ["", ".png", ".svg", ".xpm"] {
                let p = format!("{dir}/{c}{ext}");
                if std::path::Path::new(&p).is_file() {
                    return Some(Local::File(p));
                }
            }
        }
    }
    None
}

/// "firefox_firefox" (snap) is "firefox".
fn fold_snap(name: &str) -> &str {
    if let Some((a, b)) = name.split_once('_')
        && a == b
    {
        return a;
    }
    name
}

/// A severity disc (components.js severityIcon: info, success, warning, error) of `px`, drawn in the theme's
/// --sys-* colour with the --sys-icon-fg glyph; it follows theme changes.
pub fn severity_icon(severity: &str, px: i32) -> gtk::DrawingArea {
    let sev = match severity {
        "success" | "warning" | "error" => severity.to_owned(),
        _ => "info".to_owned(),
    };
    let area = gtk::DrawingArea::new();
    area.set_content_width(px);
    area.set_content_height(px);
    area.set_halign(gtk::Align::Center);
    area.set_valign(gtk::Align::Center);
    area.set_can_target(false);
    area.add_css_class("w11-sev-icon");
    area.set_draw_func(move |_, cr, w, h| {
        let s = (w.min(h) as f64) / 16.0;
        let disc = crate::theme::token(match sev.as_str() {
            "success" => "sys-success",
            "warning" => "sys-caution",
            "error" => "sys-critical",
            _ => "accent",
        });
        let fg = crate::theme::token("sys-icon-fg");
        cr.scale(s, s);
        cr.set_source_rgba(disc.red() as f64, disc.green() as f64, disc.blue() as f64, disc.alpha() as f64);
        cr.arc(8.0, 8.0, 8.0, 0.0, 2.0 * std::f64::consts::PI);
        let _ = cr.fill();
        let d = match sev.as_str() {
            "success" => "M5.1 8.2 7 10.1l3.9-4.2",
            "warning" => "M8 4.5v4.25M8 11.1v.1",
            "error" => "M5.75 5.75l4.5 4.5M10.25 5.75l-4.5 4.5",
            _ => "M8 7v4.25M8 4.85v.1",
        };
        let node = svg::parse(&format!(r#"<path d="{d}"/>"#));
        svg::render(cr, &node, (fg.red() as f64, fg.green() as f64, fg.blue() as f64, fg.alpha() as f64), true, 1.6);
    });
    crate::theme::track(&area);
    area
}
