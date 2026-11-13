//! Icons: the UI's Fluent glyphs (vendor/ui/js/core/icons.js, ported 1:1 to SVG strings below) drawn as
//! symbolic paintables, so they take the widget's CSS `color` like a symbolic theme icon (GTK hands the colour
//! to `snapshot_symbolic`, and redraws on a colour change), plus app icons from the icon theme, a file or the
//! backend's `/icon` service.
//!
//! `glyph` names are the ones of icons.js, plus:
//!   "start-logo"            the Start logo of icons.js `startLogo` (blue gradient disc)
//!   "start-logo-mono"       the same in the text colour (`startLogo(.., {monochrome: true})`)
//!   "task-view"             the taskbar's Task View glyph (taskbar.js `taskViewGlyph`)
//!   "battery-<0..100>"      the tray's battery with its charge drawn in (taskbar.js `batteryGlyph`)
//!   "battery-charging-<0..100>"  the same with the charging bolt
//! `wifi_icon`, `volume_icon` and `battery_icon` give the names for a level, as icons.js/taskbar.js do.
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, gdk_pixbuf, glib, graphene};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::f64::consts::PI;

/// The glyphs' stroke width on their 20 x 20 grid (components.css `--icon-stroke`).
pub const STROKE: f64 = 1.35;
/// The taskbar's and tray's thinner stroke (shell.css `.sh-tb-ico .fl-icon { --icon-stroke: 1.3 }`).
pub const STROKE_TASKBAR: f64 = 1.3;
/// The search box glyph's stroke (shell.css `.sh-tb-searchbox__icon .fl-icon { --icon-stroke: 1.6 }`).
pub const STROKE_SEARCH: f64 = 1.6;

/// A Fluent glyph by name ("search", "wifi-3", "volume-2", "battery-80", "bell", "chevron-up", ...), `px`
/// logical pixels square, in the widget's text colour.
pub fn glyph(name: &str, px: i32) -> gtk::Image {
    glyph_stroke(name, px, STROKE)
}

/// `glyph` with another stroke width (STROKE_TASKBAR, STROKE_SEARCH, ...).
pub fn glyph_stroke(name: &str, px: i32, stroke: f64) -> gtk::Image {
    let image = gtk::Image::new();
    let paintable = Glyph::new(name, px, stroke, image.upcast_ref());
    image.set_paintable(Some(&paintable));
    image.set_pixel_size(px);
    image.add_css_class("w11-glyph");
    image
}

/// Change the glyph an image from `glyph` shows (e.g. the tray's volume level); any other image gets one.
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

/// The name of the glyph an image from `glyph` shows.
pub fn glyph_name(image: &gtk::Image) -> Option<String> {
    image.paintable().and_downcast::<Glyph>().map(|g| g.imp().name.borrow().clone())
}

/// A glyph as a paintable, for widgets that take one (gtk::Picture, ...). `owner` is the widget whose colour
/// and scale it follows when drawn as a plain paintable.
pub fn glyph_paintable(name: &str, px: i32, owner: &impl IsA<gtk::Widget>) -> gdk::Paintable {
    Glyph::new(name, px, STROKE, owner.upcast_ref()).upcast()
}

/// Whether icons.js has this glyph (or it is one of the extra names above).
pub fn has_glyph(name: &str) -> bool {
    source(name, false).is_some()
}

/// Wi-Fi glyph for a signal strength 0..100 (icons.js wifiIcon); None (unknown) is plain "wifi".
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

/// The tray's battery glyph name for a charge 0..100 (taskbar.js batteryGlyph).
pub fn battery_icon(percent: f64, charging: bool) -> String {
    let p = if percent.is_finite() { percent.round().clamp(0.0, 100.0) as i64 } else { 0 };
    if charging { format!("battery-charging-{p}") } else { format!("battery-{p}") }
}

/// Every glyph name of icons.js (iconNames) plus the extras above (levels as their 100 % form).
pub fn glyph_names() -> Vec<&'static str> {
    let mut names: Vec<&'static str> = NAMES.to_vec();
    names.extend(["start-logo", "start-logo-mono", "task-view", "battery-100", "battery-charging-100"]);
    names
}

/// Redraw every live glyph (theme.rs after a light/dark or accent change; GTK already redraws on a colour
/// change of the widget itself).
pub fn invalidate() {
    LIVE.with(|live| {
        let mut live = live.borrow_mut();
        live.retain(|w| w.upgrade().is_some());
        for g in live.iter().filter_map(|w| w.upgrade()) {
            g.invalidate_contents();
        }
    });
}

// ------------------------------------------------------------------------------------------------ app icons

/// An app icon: a theme icon name (with or without ".desktop", snap "name_name" folded), an absolute path or
/// file:// URI (also a file without an extension), a bare name found in the pixmaps folders, and otherwise the
/// backend's `/icon` service (which knows the host's own apps); "application-x-executable" until then.
pub fn app_icon(icon: &str, px: i32) -> gtk::Image {
    let image = gtk::Image::new();
    image.set_pixel_size(px);
    image.add_css_class("w11-app-icon");
    let name = icon.trim();
    match resolve_local(name) {
        Some(Local::Name(n)) => image.set_icon_name(Some(&n)),
        Some(Local::File(path)) => image.set_from_file(Some(&path)),
        None => {
            image.set_icon_name(Some("application-x-executable"));
            if !name.is_empty() {
                fetch_remote(&image, name, px);
            }
        }
    }
    image
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

/// "firefox_firefox" (snap) is "firefox" (taskbar.js guessIcon).
fn fold_snap(name: &str) -> &str {
    if let Some((a, b)) = name.split_once('_')
        && a == b
    {
        return a;
    }
    name
}

thread_local! {
    static REMOTE: RefCell<HashMap<String, Option<gdk::Texture>>> = RefCell::new(HashMap::new());
    static PENDING: RefCell<HashMap<String, Vec<glib::WeakRef<gtk::Image>>>> = RefCell::new(HashMap::new());
}

fn fetch_remote(image: &gtk::Image, name: &str, px: i32) {
    let size = (px.max(16) * 2).clamp(32, 256);
    let key = format!("{name}@{size}");
    if let Some(known) = REMOTE.with(|r| r.borrow().get(&key).cloned()) {
        if let Some(t) = known {
            image.set_paintable(Some(&t));
        }
        return;
    }
    let first = PENDING.with(|p| {
        let mut p = p.borrow_mut();
        let waiting = p.entry(key.clone()).or_default();
        waiting.push(image.downgrade());
        waiting.len() == 1
    });
    if !first {
        return;
    }
    let backend = crate::core::shell().backend.clone();
    let query = format!("/icon?name={}&size={size}&fallback=application-x-executable", encode(name));
    glib::spawn_future_local(async move {
        let bytes = gtk::gio::spawn_blocking(move || backend.get(&query)).await.ok().and_then(|r| r.ok());
        let tex = bytes.as_deref().and_then(texture);
        REMOTE.with(|r| {
            let mut r = r.borrow_mut();
            if r.len() > 400 {
                r.clear();
            }
            r.insert(key.clone(), tex.clone());
        });
        let waiting = PENDING.with(|p| p.borrow_mut().remove(&key)).unwrap_or_default();
        if let Some(t) = tex {
            for img in waiting.iter().filter_map(|w| w.upgrade()) {
                img.set_paintable(Some(&t));
            }
        }
    });
}

fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~' | b'/') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

// --------------------------------------------------------------------------------------- the paintable

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
        const NAME: &'static str = "W11Glyph";
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
        // empty, and panics in an extern "C" frame (abort). Draw it ourselves.
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
    pub struct Glyph(ObjectSubclass<imp::Glyph>) @implements gdk::Paintable, gtk::SymbolicPaintable;
}

thread_local! {
    static LIVE: RefCell<Vec<glib::WeakRef<Glyph>>> = const { RefCell::new(Vec::new()) };
    static TEXTURES: RefCell<HashMap<String, (Option<gdk::Texture>, bool)>> = RefCell::new(HashMap::new());
    static WARNED: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
}

impl Glyph {
    fn new(name: &str, px: i32, stroke: f64, owner: &gtk::Widget) -> Glyph {
        let g: Glyph = glib::Object::new();
        let imp = g.imp();
        *imp.name.borrow_mut() = name.to_owned();
        imp.size.set(px.max(1));
        imp.stroke.set(if stroke > 0.0 { stroke } else { STROKE });
        imp.owner.set(Some(owner));
        warn_unknown(name);
        LIVE.with(|live| {
            let mut live = live.borrow_mut();
            if live.len() > 256 {
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
        warn_unknown(name);
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
        let scale = imp
            .owner
            .upgrade()
            .and_then(|w| w.native())
            .and_then(|n| n.surface())
            .map(|s| s.scale())
            .unwrap_or(1.0)
            .max(1.0);
        let phys = ((side * scale).ceil() as u32).clamp(1, 1024);
        let (r, g, b) = (channel(color.red()), channel(color.green()), channel(color.blue()));
        let fg = format!("#{r:02x}{g:02x}{b:02x}");
        // A light text colour means a dark theme (task view pane, charging bolt).
        let dark = luminance(r, g, b) > 0.5;
        let name = imp.name.borrow().clone();
        let stroke = imp.stroke.get();
        let key = format!("{name}|{phys}|{fg}|{stroke:.2}");
        let cached = TEXTURES.with(|t| t.borrow().get(&key).cloned());
        let (texture, tinted) = match cached {
            Some(hit) => hit,
            None => {
                let src = source(&name, dark).unwrap_or_else(unknown);
                let entry = (texture(render(&src, phys, &fg, stroke).as_bytes()), src.tinted);
                TEXTURES.with(|c| {
                    let mut c = c.borrow_mut();
                    if c.len() > 600 {
                        c.clear();
                    }
                    c.insert(key, entry.clone());
                });
                entry
            }
        };
        let Some(texture) = texture else { return };
        let rect = graphene::Rect::new(
            ((width - side) / 2.0) as f32,
            ((height - side) / 2.0) as f32,
            side as f32,
            side as f32,
        );
        let alpha = color.alpha() as f64;
        if tinted && alpha < 0.999 {
            snapshot.push_opacity(alpha.max(0.0));
            snapshot.append_texture(&texture, &rect);
            snapshot.pop();
        } else {
            snapshot.append_texture(&texture, &rect);
        }
    }
}

fn warn_unknown(name: &str) {
    if source(name, false).is_some() {
        return;
    }
    WARNED.with(|w| {
        if w.borrow_mut().insert(name.to_owned()) {
            eprintln!("win11-shell: unknown icon \"{name}\"");
        }
    });
}

fn channel(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn luminance(r: u8, g: u8, b: u8) -> f64 {
    let lin = |c: u8| {
        let c = c as f64 / 255.0;
        if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
}

/// A glyph's SVG source: its grid, its body, whether the outline defaults apply (fill none, stroke
/// currentColor, round caps and joins) and whether it follows the text colour.
struct Source {
    view: u32,
    body: String,
    outline: bool,
    tinted: bool,
}

fn render(src: &Source, phys: u32, fg: &str, stroke: f64) -> String {
    let defaults = if src.outline {
        format!(
            r#" fill="none" stroke="currentColor" stroke-width="{stroke}" stroke-linecap="round" stroke-linejoin="round""#
        )
    } else {
        String::new()
    };
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{phys}" height="{phys}" viewBox="0 0 {v} {v}"{defaults}>{body}</svg>"#,
        v = src.view,
        body = src.body
    );
    svg.replace("currentColor", fg)
}

/// icons.js template(): the fallback for an unknown name.
fn unknown() -> Source {
    outline(r(4.0, 4.0, 12.0, 12.0, 2.0) + &p("M8 8.25a2 2 0 1 1 2.75 1.85c-.45.2-.75.6-.75 1.1v.3") + &dot(10.0, 13.75, 0.75))
}

fn outline(body: String) -> Source {
    Source { view: 20, body, outline: true, tinted: true }
}

// ------------------------------------------------------------------------------- builders (icons.js)

const FILL: &str = r#" fill="currentColor" stroke="none""#;
const FAINT: &str = r#" opacity=".3""#;

fn p(d: &str) -> String {
    format!(r#"<path d="{d}"/>"#)
}
fn pe(d: &str, extra: &str) -> String {
    format!(r#"<path d="{d}"{extra}/>"#)
}
fn c(cx: f64, cy: f64, rad: f64) -> String {
    format!(r#"<circle cx="{cx}" cy="{cy}" r="{rad}"/>"#)
}
fn ce(cx: f64, cy: f64, rad: f64, extra: &str) -> String {
    format!(r#"<circle cx="{cx}" cy="{cy}" r="{rad}"{extra}/>"#)
}
fn r(x: f64, y: f64, w: f64, h: f64, rx: f64) -> String {
    format!(r#"<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="{rx}"/>"#)
}
fn re(x: f64, y: f64, w: f64, h: f64, rx: f64, extra: &str) -> String {
    format!(r#"<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="{rx}"{extra}/>"#)
}
fn dot(cx: f64, cy: f64, rad: f64) -> String {
    ce(cx, cy, rad, FILL)
}

fn gear(cx: f64, cy: f64, r1: f64, r2: f64, teeth: u32) -> String {
    let step = PI * 2.0 / teeth as f64;
    let mut d = String::new();
    for i in 0..teeth {
        let a = i as f64 * step - PI / 2.0;
        let pts = [(r1, a - step * 0.17), (r1, a + step * 0.17), (r2, a + step * 0.33), (r2, a + step * 0.67)];
        for (j, (rad, ang)) in pts.iter().enumerate() {
            let cmd = if i == 0 && j == 0 { "M" } else { "L" };
            d.push_str(&format!("{cmd}{} {}", fixed(cx + rad * ang.cos()), fixed(cy + rad * ang.sin())));
        }
    }
    d + "Z"
}

fn rays(cx: f64, cy: f64, r1: f64, r2: f64, n: u32) -> String {
    let mut d = String::new();
    for i in 0..n {
        let a = i as f64 * PI * 2.0 / n as f64;
        d.push_str(&format!(
            "M{} {}L{} {}",
            fixed(cx + r1 * a.cos()),
            fixed(cy + r1 * a.sin()),
            fixed(cx + r2 * a.cos()),
            fixed(cy + r2 * a.sin())
        ));
    }
    d
}

fn star4(cx: f64, cy: f64, s: f64) -> String {
    let f = fixed;
    format!(
        "M{cx} {}c{} {} {} {} {s} {s}c{} {} {} {} {} {s}c{} {} {} {} {} {}c{} {} {} {} {s} {}Z",
        cy - s,
        f(s * 0.18),
        f(s * 0.62),
        f(s * 0.38),
        f(s * 0.82),
        f(-s * 0.62),
        f(s * 0.18),
        f(-s * 0.82),
        f(s * 0.38),
        -s,
        f(-s * 0.18),
        f(-s * 0.62),
        f(-s * 0.38),
        f(-s * 0.82),
        -s,
        -s,
        f(s * 0.62),
        f(-s * 0.18),
        f(s * 0.82),
        f(-s * 0.38),
        -s
    )
}

/// JS toFixed(2).
fn fixed(v: f64) -> String {
    let s = format!("{v:.2}");
    if s == "-0.00" { "0.00".into() } else { s }
}

const SPEAKER: &str = "M3.5 7.25h2.4l3.7-2.95a.6.6 0 0 1 .98.47v10.46a.6.6 0 0 1-.98.47l-3.7-2.95H3.5a1 1 0 0 1-1-1v-3.5a1 1 0 0 1 1-1Z";
const WAVE1: &str = "M12.32 7.54a3 3 0 0 1 0 4.92";
const WAVE2: &str = "M13.81 5.41a5.6 5.6 0 0 1 0 9.18";
const WAVE3: &str = "M15.3 3.28a8.2 8.2 0 0 1 0 13.44";
const WIFI_A1: &str = "M7.45 13.2a3.6 3.6 0 0 1 5.1 0";
const WIFI_A2: &str = "M4.91 10.66a7.2 7.2 0 0 1 10.18 0";
const WIFI_A3: &str = "M2.36 8.11a10.8 10.8 0 0 1 15.28 0";
const BELL: &str = "M10 2.75a5 5 0 0 0-5 5v2.9l-1.3 2.9a.8.8 0 0 0 .73 1.12h11.14a.8.8 0 0 0 .73-1.12L15 10.65v-2.9a5 5 0 0 0-5-5Z";
const PAGE: &str = "M6 2.5h5.1c.4 0 .78.16 1.06.44l3.4 3.4c.28.28.44.66.44 1.06V16c0 .83-.67 1.5-1.5 1.5H6c-.83 0-1.5-.67-1.5-1.5V4c0-.83.67-1.5 1.5-1.5Z";
const PAGE_FOLD: &str = "M11 2.75V6c0 .55.45 1 1 1h3.25";
const FOLDER: &str = "M2.5 5.75c0-1.1.9-2 2-2h3.08c.53 0 1.04.21 1.41.59L10.5 5.75h5c1.1 0 2 .9 2 2v6.5c0 1.1-.9 2-2 2h-11c-1.1 0-2-.9-2-2Z";
const FOLDER_TAB: &str = "M2.5 8.25h5.4c.4 0 .78-.16 1.06-.44l1.56-1.56";
const TRASH_CAN: &str = "M5 5.25l.85 10.4c.07.9.82 1.6 1.73 1.6h4.84c.9 0 1.66-.7 1.73-1.6L15 5.25";
const PIN: &str = r#"<g transform="rotate(45 10 10)"><path d="M7.25 2.5h5.5M8.25 2.5v4.25L5.75 10h8.5l-2.5-3.25V2.5"/><path d="M10 10v7.5"/></g>"#;

fn wifi_dot() -> String {
    dot(10.0, 15.75, 1.2)
}
fn monitor() -> String {
    r(2.5, 3.5, 15.0, 10.5, 1.75)
}
fn stand() -> String {
    p("M7.25 16.75h5.5M10 14v2.75")
}
fn battery_body() -> String {
    r(2.0, 5.5, 14.0, 9.0, 2.0) + &p("M18 8.5v3")
}
fn circle() -> String {
    c(10.0, 10.0, 7.25)
}
fn slash() -> String {
    p("M3 3l14 14")
}
fn clock_hands() -> String {
    p("M10 5.75V10l2.75 1.75")
}
fn trash_lid() -> String {
    p("M3.5 5.25h13") + &p("M8 5.25V4c0-.69.56-1.25 1.25-1.25h1.5c.69 0 1.25.56 1.25 1.25v1.25")
}
fn eye_shape() -> String {
    p("M2.25 10S5.05 4.75 10 4.75 17.75 10 17.75 10 14.95 15.25 10 15.25 2.25 10 2.25 10Z") + &c(10.0, 10.0, 2.5)
}
fn moon() -> String {
    p("M16.2 12.6A6.75 6.75 0 0 1 7.4 3.8a6.75 6.75 0 1 0 8.8 8.8Z")
}
fn restart() -> String {
    p("M16.25 10A6.25 6.25 0 1 1 14.42 5.58") + &p("M14.42 2.08V5.58H10.92")
}
fn back() -> String {
    p("M16.5 10H3.75M9 4.75 3.75 10 9 15.25")
}
fn forward() -> String {
    p("M3.5 10h12.75M11 4.75 16.25 10 11 15.25")
}
fn drive() -> String {
    r(3.0, 4.5, 14.0, 11.0, 2.0) + &p("M3 11h14") + &dot(14.0, 13.25, 0.8)
}
fn sparkle() -> String {
    p(&star4(9.0, 8.0, 5.25)) + &p(&star4(14.75, 13.75, 2.5))
}

/// Every name of icons.js ICONS, in its order.
const NAMES: &[&str] = &[
    "start", "search", "taskview", "widgets", "apps", "start-menu", "taskbar", "window", "snap", "desktop",
    "display", "pc", "wifi", "wifi-3", "wifi-2", "wifi-1", "wifi-off", "ethernet", "network", "globe",
    "bluetooth", "airplane", "hotspot", "cast", "usb", "drive", "disk", "storage", "cloud", "keyboard", "mouse",
    "cpu", "memory", "gpu", "phone", "devices", "headphones", "mic", "camera", "print", "volume", "volume-2",
    "volume-1", "volume-mute", "sound", "brightness", "sun", "moon", "sleep", "night-light", "battery",
    "battery-charging", "battery-saver", "lightning", "power", "restart", "refresh", "update", "lock",
    "lock-open", "key", "signout", "shield", "user", "people", "accessibility", "heart", "emoji",
    "chevron-right", "chevron-left", "chevron-down", "chevron-up", "arrow-left", "arrow-right", "arrow-up",
    "arrow-down", "back", "forward", "close", "minimize", "maximize", "restore", "more", "more-vertical", "menu",
    "plus", "minus", "check", "external", "open", "link", "zoom-in", "zoom-out", "undo", "redo", "folder",
    "folder-open", "file", "document", "image", "music", "video", "download", "upload", "home", "trash",
    "delete", "trash-full", "cut", "copy", "paste", "clipboard", "rename", "share", "properties", "info", "help",
    "sort", "filter", "view", "grid", "list", "new", "edit", "save", "tag", "star", "history", "pin", "unpin",
    "terminal", "code", "mail", "eye", "eye-off", "location", "settings", "personalize", "brush", "color",
    "theme", "clock", "timer", "calendar", "language", "notification", "notification-off", "focus", "gamepad",
    "bulb", "flag", "app", "process", "startup", "services", "summary", "chart", "clean", "sparkle", "broom",
    "warning", "error", "success", "play", "pause", "next", "previous",
];

/// The SVG source of a glyph; `dark` says the theme is dark (for the few with a theme colour of their own).
fn source(name: &str, dark: bool) -> Option<Source> {
    if let Some(s) = special(name, dark) {
        return Some(s);
    }
    let name = match name {
        // Names the HTML modules use for the same glyphs (bell in the clock, task view button)
        "bell" => "notification",
        "bell-off" => "notification-off",
        other => other,
    };
    icon_body(name).map(outline)
}

fn special(name: &str, dark: bool) -> Option<Source> {
    const GRADIENT: &str = r##"<defs><linearGradient id="g" x1="0" y1="0" x2="24" y2="24" gradientUnits="userSpaceOnUse"><stop offset="0" stop-color="#5ad1ff"/><stop offset=".48" stop-color="#1a9df5"/><stop offset="1" stop-color="#0063d1"/></linearGradient><linearGradient id="h" x1="0" y1="0" x2="0" y2="24" gradientUnits="userSpaceOnUse"><stop offset="0" stop-color="#fff" stop-opacity=".22"/><stop offset=".5" stop-color="#fff" stop-opacity="0"/></linearGradient></defs>"##;
    match name {
        "start-logo" => Some(Source {
            view: 24,
            body: format!(
                r#"{GRADIENT}<circle cx="12" cy="12" r="10.5" fill="url(#g)"/><circle cx="12" cy="12" r="10.5" fill="url(#h)"/>"#
            ),
            outline: false,
            tinted: false,
        }),
        "start-logo-mono" => Some(Source {
            view: 24,
            body: r#"<circle cx="12" cy="12" r="10.5" fill="currentColor"/>"#.into(),
            outline: false,
            tinted: true,
        }),
        "task-view" | "taskview-glyph" => {
            let front = if dark { "#2b2b2b" } else { "#f6f6f6" };
            Some(Source {
                view: 24,
                body: format!(
                    r#"<rect x="8.5" y="3.5" width="12" height="11" rx="2" fill="currentColor" fill-opacity="0.18" stroke="currentColor" stroke-width="1.4"/><rect x="3.5" y="8.5" width="12" height="11" rx="2" fill="{front}" stroke="currentColor" stroke-width="1.4"/>"#
                ),
                outline: false,
                tinted: true,
            })
        }
        _ => {
            let (charging, rest) = if let Some(rest) = name.strip_prefix("battery-charging-") {
                (true, rest)
            } else if let Some(rest) = name.strip_prefix("battery-") {
                (false, rest)
            } else {
                return None;
            };
            let percent = rest.parse::<u32>().ok()?.min(100) as f64;
            Some(battery_level(percent, charging, dark))
        }
    }
}

/// taskbar.js batteryGlyph: the outline with the charge level drawn in, red at 10 % or less (not charging),
/// and the charging bolt over it.
fn battery_level(percent: f64, charging: bool, dark: bool) -> Source {
    let w = (((10.5 * percent.clamp(0.0, 100.0) / 100.0) * 10.0).round() / 10.0).max(1.0);
    let low = percent <= 10.0 && !charging;
    let fill = if low { "#e81123" } else { "currentColor" };
    // the bolt is cut out in the taskbar's own colour (shell.css --sh-tb-solid)
    let bolt = if dark { "#1c1c1c" } else { "#eeeeee" };
    let mut body = String::from(r#"<rect x="2" y="5.5" width="14" height="9" rx="2" fill="none" stroke="currentColor"/>"#);
    body.push_str(r#"<path d="M18 8.5v3" stroke="currentColor" stroke-linecap="round"/>"#);
    body.push_str(&format!(r#"<rect x="3.75" y="7.25" width="{w}" height="5.5" rx="1" fill="{fill}" stroke="none"/>"#));
    if charging {
        body.push_str(&format!(
            r#"<path d="M9.8 6.2 7.6 10.3h2.6l-1 3.5 2.9-4.4H9.5l1-3.2Z" fill="{bolt}" stroke="currentColor" stroke-width=".8" stroke-linejoin="round"/>"#
        ));
    }
    Source { view: 20, body, outline: true, tinted: true }
}

/// icons.js ICONS.
fn icon_body(name: &str) -> Option<String> {
    let s = match name {
        // shell
        "start" => re(3.0, 3.0, 6.25, 6.25, 1.1, FILL)
            + &re(10.75, 3.0, 6.25, 6.25, 1.1, FILL)
            + &re(3.0, 10.75, 6.25, 6.25, 1.1, FILL)
            + &re(10.75, 10.75, 6.25, 6.25, 1.1, FILL),
        "search" => c(8.5, 8.5, 5.5) + &p("M12.6 12.6 17 17"),
        "taskview" => r(2.75, 6.25, 11.0, 10.0, 1.75) + &p("M6.25 3.25h9.25c.97 0 1.75.78 1.75 1.75v8.25"),
        "widgets" => {
            r(3.0, 3.0, 6.0, 8.0, 1.5) + &r(11.0, 3.0, 6.0, 4.5, 1.5) + &r(11.0, 9.5, 6.0, 7.5, 1.5) + &r(3.0, 13.0, 6.0, 4.0, 1.5)
        }
        "apps" => {
            r(3.0, 3.0, 5.5, 5.5, 1.4)
                + &r(3.0, 11.5, 5.5, 5.5, 1.4)
                + &r(11.5, 11.5, 5.5, 5.5, 1.4)
                + &p("M14.25 2.6l3.15 3.15-3.15 3.15-3.15-3.15Z")
        }
        "start-menu" => {
            r(4.0, 2.75, 12.0, 14.5, 2.0)
                + &r(6.5, 5.25, 3.0, 3.0, 0.75)
                + &r(10.5, 5.25, 3.0, 3.0, 0.75)
                + &p("M6.5 11.5h7M6.5 14h4.5")
        }
        "taskbar" => {
            r(2.5, 3.5, 15.0, 13.0, 2.0)
                + &p("M2.5 13h15")
                + &dot(7.0, 14.85, 0.75)
                + &dot(10.0, 14.85, 0.75)
                + &dot(13.0, 14.85, 0.75)
        }
        "window" => r(2.5, 3.5, 15.0, 13.0, 2.0) + &p("M2.5 7h15"),
        "snap" => r(2.5, 3.5, 15.0, 13.0, 2.0) + &p("M10 3.5v13M10 10h7.5"),
        "desktop" => monitor() + &p("M2.5 11.25h15") + &stand(),
        "display" => monitor() + &stand(),
        "pc" => {
            r(2.5, 4.5, 8.5, 7.0, 1.25)
                + &p("M5.25 14.5h3M6.75 11.5v3")
                + &r(13.0, 3.5, 4.5, 13.0, 1.25)
                + &p("M14.5 6.25h1.5")
                + &dot(15.25, 13.5, 0.7)
        }

        // network & devices
        "wifi" | "wifi-3" => wifi_dot() + &p(WIFI_A1) + &p(WIFI_A2) + &p(WIFI_A3),
        "wifi-2" => wifi_dot() + &p(WIFI_A1) + &p(WIFI_A2) + &pe(WIFI_A3, FAINT),
        "wifi-1" => wifi_dot() + &p(WIFI_A1) + &pe(WIFI_A2, FAINT) + &pe(WIFI_A3, FAINT),
        "wifi-off" => {
            wifi_dot() + &pe(WIFI_A1, FAINT) + &pe(WIFI_A2, FAINT) + &pe(WIFI_A3, FAINT) + &p("M3.25 3.25l13.5 13.5")
        }
        "ethernet" => {
            p("M2.75 5.25c0-.97.78-1.75 1.75-1.75h11c.97 0 1.75.78 1.75 1.75v7.5c0 .97-.78 1.75-1.75 1.75H13.5v1.75c0 .28-.22.5-.5.5H7a.5.5 0 0 1-.5-.5V14.5H4.5c-.97 0-1.75-.78-1.75-1.75Z")
                + &p("M7 7.25v3M10 7.25v3M13 7.25v3")
        }
        "network" => {
            r(7.0, 2.5, 6.0, 4.5, 1.25)
                + &r(2.5, 13.0, 6.0, 4.5, 1.25)
                + &r(11.5, 13.0, 6.0, 4.5, 1.25)
                + &p("M10 7v3M5.5 13v-3h9v3")
        }
        "globe" => circle() + r#"<ellipse cx="10" cy="10" rx="3" ry="7.25"/>"# + &p("M2.75 10h14.5"),
        "bluetooth" => p("M6 6.5l8 7L10 17V3l4 3.5-8 7"),
        "airplane" => p("M10 2.25c.83 0 1.5.67 1.5 1.5v3.9l5.2 3.05c.19.11.3.31.3.53v1.02c0 .3-.29.52-.58.44L11.5 11.3v3.45l1.75 1.3c.16.12.25.3.25.5v.68c0 .26-.25.45-.5.38L10 16.9l-3 .71c-.25.07-.5-.12-.5-.38v-.68c0-.2.09-.38.25-.5l1.75-1.3V11.3l-4.92 1.39c-.29.08-.58-.14-.58-.44v-1.02c0-.22.11-.42.3-.53l5.2-3.05v-3.9c0-.83.67-1.5 1.5-1.5Z"),
        "hotspot" => {
            dot(10.0, 10.0, 1.4)
                + &p("M7.17 12.83a4 4 0 0 1 0-5.66M12.83 7.17a4 4 0 0 1 0 5.66M4.7 15.3a7.5 7.5 0 0 1 0-10.6M15.3 4.7a7.5 7.5 0 0 1 0 10.6")
        }
        "cast" => {
            p("M2.75 7V5.5c0-.97.78-1.75 1.75-1.75h11c.97 0 1.75.78 1.75 1.75v9c0 .97-.78 1.75-1.75 1.75H11")
                + &p("M2.75 10.5a5.75 5.75 0 0 1 5.75 5.75M2.75 13.5a2.75 2.75 0 0 1 2.75 2.75")
                + &dot(3.25, 16.0, 0.9)
        }
        "usb" => {
            p("M6.5 8h7v8c0 .83-.67 1.5-1.5 1.5H8c-.83 0-1.5-.67-1.5-1.5Z")
                + &p("M8 8V3.5a1 1 0 0 1 1-1h2a1 1 0 0 1 1 1V8")
                + &p("M9.1 5h.01M10.9 5h.01")
        }
        "drive" | "disk" => drive(),
        "storage" => {
            r(3.0, 3.5, 14.0, 5.5, 1.5) + &r(3.0, 11.0, 14.0, 5.5, 1.5) + &dot(13.75, 6.25, 0.8) + &dot(13.75, 13.75, 0.8)
        }
        "cloud" => p("M6 15.5a3.5 3.5 0 0 1-.3-7 4.5 4.5 0 0 1 8.7-1 3.75 3.75 0 0 1-.15 8Z"),
        "keyboard" => {
            r(2.0, 5.0, 16.0, 10.0, 2.0)
                + &dot(5.5, 8.0, 0.8)
                + &dot(8.5, 8.0, 0.8)
                + &dot(11.5, 8.0, 0.8)
                + &dot(14.5, 8.0, 0.8)
                + &dot(7.0, 10.25, 0.8)
                + &dot(10.0, 10.25, 0.8)
                + &dot(13.0, 10.25, 0.8)
                + &p("M6.5 12.5h7")
        }
        "mouse" => r(5.25, 2.5, 9.5, 15.0, 4.75) + &p("M10 2.75v4.5"),
        "cpu" => {
            r(5.5, 5.5, 9.0, 9.0, 1.5)
                + &r(8.0, 8.0, 4.0, 4.0, 0.6)
                + &p("M8 2.75V5.5M12 2.75V5.5M8 14.5v2.75M12 14.5v2.75M2.75 8H5.5M2.75 12H5.5M14.5 8h2.75M14.5 12h2.75")
        }
        "memory" => {
            r(2.5, 5.5, 15.0, 8.0, 1.5)
                + &p("M5.5 13.5v2.75M10 13.5v2.75M14.5 13.5v2.75")
                + &r(5.25, 8.0, 3.5, 3.0, 0.6)
                + &r(11.25, 8.0, 3.5, 3.0, 0.6)
        }
        "gpu" => r(2.5, 5.0, 15.0, 9.0, 1.5) + &c(8.0, 9.5, 2.5) + &p("M12 8h3M12 11h3M2.5 14v2.5"),
        "phone" => r(5.5, 2.5, 9.0, 15.0, 2.0) + &p("M9 15h2"),
        "devices" => r(2.5, 4.0, 10.5, 8.0, 1.5) + &p("M5 15h5.5M7.75 12v3") + &r(14.0, 6.5, 3.5, 9.0, 1.0),
        "headphones" => {
            p("M3.5 13v-2.5a6.5 6.5 0 0 1 13 0V13") + &r(3.0, 11.5, 3.5, 5.5, 1.5) + &r(13.5, 11.5, 3.5, 5.5, 1.5)
        }
        "mic" => r(7.25, 2.5, 5.5, 9.5, 2.75) + &p("M4.5 9.5a5.5 5.5 0 0 0 11 0M10 15v2.5"),
        "camera" => {
            p("M2.5 7c0-.97.78-1.75 1.75-1.75h1.9l1.1-1.6c.24-.34.62-.65 1.04-.65h3.42c.42 0 .8.31 1.04.65l1.1 1.6h1.9c.97 0 1.75.78 1.75 1.75v7.5c0 .97-.78 1.75-1.75 1.75H4.25c-.97 0-1.75-.78-1.75-1.75Z")
                + &c(10.0, 10.5, 3.0)
        }
        "print" => {
            p("M5.5 7V3.5a1 1 0 0 1 1-1h7a1 1 0 0 1 1 1V7")
                + &p("M5.5 14H4a1.5 1.5 0 0 1-1.5-1.5v-4A1.5 1.5 0 0 1 4 7h12a1.5 1.5 0 0 1 1.5 1.5v4A1.5 1.5 0 0 1 16 14h-1.5")
                + &r(5.5, 11.0, 9.0, 6.5, 1.0)
        }

        // volume / brightness / power
        "volume" => p(SPEAKER) + &p(WAVE1) + &p(WAVE2) + &p(WAVE3),
        "volume-2" | "sound" => p(SPEAKER) + &p(WAVE1) + &p(WAVE2),
        "volume-1" => p(SPEAKER) + &p(WAVE1),
        "volume-mute" => p(SPEAKER) + &p("M13 8l4 4M17 8l-4 4"),
        "brightness" => {
            c(10.0, 10.0, 3.25) + &pe("M10 6.75a3.25 3.25 0 0 0 0 6.5Z", FILL) + &p(&rays(10.0, 10.0, 5.5, 7.0, 8))
        }
        "sun" => c(10.0, 10.0, 3.25) + &p(&rays(10.0, 10.0, 5.6, 7.6, 8)),
        "moon" | "sleep" => moon(),
        "night-light" => {
            p("M14.75 13.4A6.25 6.25 0 0 1 6.6 5.25a6.25 6.25 0 1 0 8.15 8.15Z") + &p(&star4(14.25, 5.75, 2.25))
        }
        "battery" => battery_body(),
        "battery-charging" => battery_body() + &p("M9.75 7.25 7.75 10h2.75l-2 2.75"),
        "battery-saver" => {
            battery_body() + &p("M6 12c0-2.6 1.9-4.25 5.25-4.25 0 3-1.7 4.25-3.75 4.25M6 12l2.25-1.85")
        }
        "lightning" => p("M11 2.5 4.75 11h5l-1 6.5L15.25 9h-5Z"),
        "power" => p("M10 2.75v6.5") + &p("M6.4 4.75a6.25 6.25 0 1 0 7.2 0"),
        "restart" | "refresh" => restart(),
        "update" => {
            p("M15.75 7.75A6.25 6.25 0 0 0 4.35 6.5")
                + &p("M15.75 3.75v4h-4")
                + &p("M4.25 12.25a6.25 6.25 0 0 0 11.4 1.25")
                + &p("M4.25 16.25v-4h4")
        }
        "lock" => r(4.5, 8.75, 11.0, 8.5, 2.0) + &p("M7 8.75V6.5a3 3 0 0 1 6 0v2.25") + &dot(10.0, 13.0, 1.0),
        "lock-open" => r(4.5, 8.75, 11.0, 8.5, 2.0) + &p("M7 8.75V6.5a3 3 0 0 1 5.85-.95") + &dot(10.0, 13.0, 1.0),
        "key" => c(6.5, 13.5, 3.5) + &p("M9 11l7.5-7.5M14 6l2 2M12 8l1.5 1.5"),
        "signout" => p("M8.5 3.25H5.75C4.78 3.25 4 4.03 4 5v10c0 .97.78 1.75 1.75 1.75H8.5") + &p("M9 10h8M14 7l3 3-3 3"),
        "shield" => p("M10 2.5 4.25 4.6c-.3.11-.5.4-.5.72v3.93c0 4.16 2.6 6.84 5.94 8.15.2.08.42.08.62 0 3.34-1.31 5.94-3.99 5.94-8.15V5.32c0-.32-.2-.61-.5-.72Z"),

        // people
        "user" => {
            c(10.0, 6.25, 3.25)
                + &p("M5.25 11.5h9.5c.97 0 1.75.78 1.75 1.75 0 2.4-2.6 4-6.5 4s-6.5-1.6-6.5-4c0-.97.78-1.75 1.75-1.75Z")
        }
        "people" => {
            c(7.5, 6.5, 2.75)
                + &c(14.0, 7.25, 2.25)
                + &p("M2.5 15.5c0-2.3 2-3.75 5-3.75s5 1.45 5 3.75")
                + &p("M13.25 11.8c2.4-.1 4.25 1.1 4.25 3.2")
        }
        "accessibility" => {
            c(10.0, 4.25, 1.5) + &p("M4 7.25c2 .65 4 .95 6 .95s4-.3 6-.95M10 8.2v3.8M10 12l-2.75 5.25M10 12l2.75 5.25")
        }
        "heart" => p("M10 16.5S3 12.6 3 7.6A3.6 3.6 0 0 1 10 6a3.6 3.6 0 0 1 7 1.6c0 5-7 8.9-7 8.9Z"),
        "emoji" => {
            circle()
                + &p("M7 11.75c.7 1.1 1.75 1.75 3 1.75s2.3-.65 3-1.75")
                + &dot(7.5, 8.0, 1.0)
                + &dot(12.5, 8.0, 1.0)
        }

        // navigation & chrome
        "chevron-right" => p("M7.5 4.25 13.25 10 7.5 15.75"),
        "chevron-left" => p("M12.5 4.25 6.75 10l5.75 5.75"),
        "chevron-down" => p("M4.25 7.25 10 13l5.75-5.75"),
        "chevron-up" => p("M4.25 12.75 10 7l5.75 5.75"),
        "arrow-left" | "back" => back(),
        "arrow-right" | "forward" => forward(),
        "arrow-up" => p("M10 16.5V3.75M4.75 9 10 3.75 15.25 9"),
        "arrow-down" => p("M10 3.5v12.75M4.75 11 10 16.25 15.25 11"),
        "close" => p("M4.75 4.75l10.5 10.5M15.25 4.75 4.75 15.25"),
        "minimize" => p("M4 10h12"),
        "maximize" => r(4.0, 4.0, 12.0, 12.0, 1.75),
        "restore" => r(4.0, 6.5, 9.5, 9.5, 1.5) + &p("M6.5 4h7.75c.97 0 1.75.78 1.75 1.75v7.75"),
        "more" => dot(4.5, 10.0, 1.25) + &dot(10.0, 10.0, 1.25) + &dot(15.5, 10.0, 1.25),
        "more-vertical" => dot(10.0, 4.5, 1.25) + &dot(10.0, 10.0, 1.25) + &dot(10.0, 15.5, 1.25),
        "menu" => p("M3 5.5h14M3 10h14M3 14.5h14"),
        "plus" => p("M10 3.75v12.5M3.75 10h12.5"),
        "minus" => p("M3.75 10h12.5"),
        "check" => p("M4 10.5l3.75 3.75L16 6"),
        "external" => {
            p("M8.5 4.5H5.5C4.67 4.5 4 5.17 4 6v8.5c0 .83.67 1.5 1.5 1.5H14c.83 0 1.5-.67 1.5-1.5v-3")
                + &p("M11.5 3.5h5v5M16.5 3.5l-7 7")
        }
        "open" => {
            p("M4 10V5.5C4 4.67 4.67 4 5.5 4h9c.83 0 1.5.67 1.5 1.5v9c0 .83-.67 1.5-1.5 1.5H10")
                + &p("M3.5 16.5l6-6M5.5 10.5h4v4")
        }
        "link" => {
            p("M8.5 11.5a3 3 0 0 0 4.24 0l2.5-2.5a3 3 0 0 0-4.24-4.24l-.75.75")
                + &p("M11.5 8.5a3 3 0 0 0-4.24 0l-2.5 2.5a3 3 0 0 0 4.24 4.24l.75-.75")
        }
        "zoom-in" => c(8.5, 8.5, 5.5) + &p("M12.6 12.6 17 17M8.5 6.25v4.5M6.25 8.5h4.5"),
        "zoom-out" => c(8.5, 8.5, 5.5) + &p("M12.6 12.6 17 17M6.25 8.5h4.5"),
        "undo" => p("M4.5 7.5h7.75a4.25 4.25 0 0 1 0 8.5H8") + &p("M7.5 4.5l-3 3 3 3"),
        "redo" => p("M15.5 7.5H7.75a4.25 4.25 0 0 0 0 8.5H12") + &p("M12.5 4.5l3 3-3 3"),

        // files
        "folder" => p(FOLDER) + &p(FOLDER_TAB),
        "folder-open" => {
            p("M2.75 14.5V5.75c0-1.1.9-2 2-2h2.83c.53 0 1.04.21 1.41.59L10.25 5.6h4.5c1.1 0 2 .9 2 2V9")
                + &p("M2.85 15.1 5 10.2c.24-.55.78-.9 1.38-.9h10.4c.72 0 1.2.74.92 1.4l-1.95 4.5c-.24.55-.78.9-1.38.9H3.8c-.72 0-1.2-.74-.95-1.4Z")
        }
        "file" => p(PAGE) + &p(PAGE_FOLD),
        "document" => p(PAGE) + &p(PAGE_FOLD) + &p("M7.5 10.5h5M7.5 13.5h5"),
        "image" => {
            r(2.75, 3.75, 14.5, 12.5, 2.25) + &c(13.0, 7.5, 1.5) + &p("M3 14.5l4.4-4.4a1.25 1.25 0 0 1 1.77 0l6.1 6.1")
        }
        "music" => p("M8 15V4.75l8.5-2v10.25M8 7.75l8.5-2") + &c(6.0, 15.0, 2.0) + &c(14.5, 12.75, 2.0),
        "video" => {
            r(2.5, 5.0, 10.5, 10.0, 2.0)
                + &p("M13 8.6l3.72-2.24a.5.5 0 0 1 .78.43v6.42a.5.5 0 0 1-.78.43L13 11.4")
        }
        "download" => p("M10 3.25v9.5M6 9l4 4 4-4M4.25 16.5h11.5"),
        "upload" => p("M10 13.5V4M6 7.75l4-4 4 4M4.25 16.5h11.5"),
        "home" => p("M3.25 8.6 9.2 3.5a1.25 1.25 0 0 1 1.6 0l5.95 5.1c.16.14.25.34.25.55v6.6c0 .69-.56 1.25-1.25 1.25H12.5v-4.5a1 1 0 0 0-1-1h-3a1 1 0 0 0-1 1V17H4.25C3.56 17 3 16.44 3 15.75V9.15c0-.21.09-.41.25-.55Z"),
        "trash" | "delete" => trash_lid() + &p(TRASH_CAN) + &p("M8.5 8.5v5.5M11.5 8.5v5.5"),
        "trash-full" => {
            p("M3.5 5.25h13")
                + &p("M6.25 5.25 7.4 2.6l2.6 1.3 2.3-1.5 1.45 2.85")
                + &p(TRASH_CAN)
                + &p("M8.5 8.5v5.5M11.5 8.5v5.5")
        }
        "cut" => c(5.75, 14.25, 2.5) + &c(14.25, 14.25, 2.5) + &p("M7.5 12.4 14.5 2.75M12.5 12.4 5.5 2.75"),
        "copy" => r(6.5, 5.5, 9.5, 12.0, 1.75) + &p("M4 14V4.25C4 3.28 4.78 2.5 5.75 2.5H12"),
        "paste" => {
            p("M7 3.75H5.5C4.67 3.75 4 4.42 4 5.25v10.5c0 .83.67 1.5 1.5 1.5H8.5")
                + &r(7.0, 2.25, 6.0, 3.0, 1.0)
                + &p("M13 3.75h1.5c.83 0 1.5.67 1.5 1.5V8")
                + &r(10.25, 9.5, 7.0, 8.0, 1.5)
        }
        "clipboard" => r(4.5, 3.5, 11.0, 14.0, 1.75) + &r(7.25, 2.25, 5.5, 2.75, 1.0) + &p("M7.5 9h5M7.5 12h5M7.5 15h3"),
        "rename" => {
            p("M10.5 6.25H4.25c-.97 0-1.75.78-1.75 1.75v4c0 .97.78 1.75 1.75 1.75h6.25")
                + &p("M15.5 6.25h.25c.97 0 1.75.78 1.75 1.75v4c0 .97-.78 1.75-1.75 1.75h-.25")
                + &p("M13 3.5v13M11.25 3.5h3.5M11.25 16.5h3.5")
        }
        "share" => {
            p("M12.75 3.5l4 4-4 4")
                + &p("M16.5 7.5h-4a6 6 0 0 0-6 6v.25")
                + &p("M8 4.25H5.25c-.97 0-1.75.78-1.75 1.75v8.75c0 .97.78 1.75 1.75 1.75H14c.97 0 1.75-.78 1.75-1.75V13")
        }
        "properties" => {
            r(3.0, 3.0, 14.0, 14.0, 2.0) + &p("M6.5 7h1.25M9.75 7h3.75M6.5 10h1.25M9.75 10h3.75M6.5 13h1.25M9.75 13h3.75")
        }
        "info" => circle() + &p("M10 9v4.75") + &dot(10.0, 6.5, 1.0),
        "help" => {
            circle() + &p("M7.75 7.75a2.25 2.25 0 1 1 3.4 1.95c-.65.4-1.15.95-1.15 1.7v.35") + &dot(10.0, 14.0, 1.0)
        }
        "sort" => p("M6.25 3.5v13M3.25 6.5l3-3 3 3M13.75 16.5v-13M10.75 13.5l3 3 3-3"),
        "filter" => p("M3.5 4.5h13l-5 6v5l-3 1.5v-6.5Z"),
        "view" => r(2.75, 3.75, 14.5, 12.5, 2.0) + &p("M2.75 7.75h14.5M7.25 7.75v8.5"),
        "grid" => {
            r(3.0, 3.0, 6.0, 6.0, 1.5) + &r(11.0, 3.0, 6.0, 6.0, 1.5) + &r(3.0, 11.0, 6.0, 6.0, 1.5) + &r(11.0, 11.0, 6.0, 6.0, 1.5)
        }
        "list" => dot(4.5, 5.5, 1.0) + &dot(4.5, 10.0, 1.0) + &dot(4.5, 14.5, 1.0) + &p("M8 5.5h8.5M8 10h8.5M8 14.5h8.5"),
        "new" => circle() + &p("M10 6.75v6.5M6.75 10h6.5"),
        "edit" => {
            p("M13.2 3.6a1.8 1.8 0 0 1 2.55 0l.65.65a1.8 1.8 0 0 1 0 2.55L7.6 15.6l-3.6.9.9-3.6Z") + &p("M12 4.8l3.2 3.2")
        }
        "save" => {
            p("M4.5 3h9l3 3v9.5a1.5 1.5 0 0 1-1.5 1.5H4.5A1.5 1.5 0 0 1 3 15.5v-11A1.5 1.5 0 0 1 4.5 3Z")
                + &p("M6.5 3v3.5h6V3M6 17v-5h8v5")
        }
        "tag" => {
            p("M3 4.5A1.5 1.5 0 0 1 4.5 3h5.38c.4 0 .78.16 1.06.44l5.62 5.62a1.5 1.5 0 0 1 0 2.12l-5.38 5.38a1.5 1.5 0 0 1-2.12 0L3.44 10.94A1.5 1.5 0 0 1 3 9.88Z")
                + &dot(6.75, 6.75, 1.1)
        }
        "star" => p("M10 2.8l2.2 4.5 4.95.7-3.58 3.5.85 4.95L10 14.1l-4.42 2.35.85-4.95L2.85 8l4.95-.7Z"),
        "history" => p("M3.75 10a6.25 6.25 0 1 0 1.83-4.42") + &p("M3.75 3.25v3.5h3.5") + &p("M10 6.75V10l2.5 1.5"),
        "pin" => PIN.to_owned(),
        "unpin" => PIN.to_owned() + &p("M2.5 2.5l15 15"),
        "terminal" => r(2.5, 3.5, 15.0, 13.0, 2.0) + &p("M6 8l2.5 2.25L6 12.5M10.5 12.5h3.5"),
        "code" => p("M7 5.5 2.75 10 7 14.5M13 5.5l4.25 4.5L13 14.5"),
        "mail" => r(2.5, 4.5, 15.0, 11.0, 2.0) + &p("M3 5.5l7 5 7-5"),
        "eye" => eye_shape(),
        "eye-off" => eye_shape() + &slash(),
        "location" => p("M10 17.5s-5.5-4.9-5.5-9.25a5.5 5.5 0 0 1 11 0C15.5 12.6 10 17.5 10 17.5Z") + &c(10.0, 8.25, 2.0),

        // settings & personalisation
        "settings" => p(&gear(10.0, 10.0, 7.6, 5.7, 8)) + &c(10.0, 10.0, 2.6),
        "personalize" => {
            p("M10 2.5a7.5 7.5 0 0 0 0 15c1.1 0 1.8-.8 1.8-1.7 0-.9-.7-1.3-.7-2.1 0-.8.6-1.4 1.4-1.4h1.7a3.3 3.3 0 0 0 3.3-3.3C17.5 5.5 14.1 2.5 10 2.5Z")
                + &dot(6.5, 9.0, 1.0)
                + &dot(8.75, 5.9, 1.0)
                + &dot(12.6, 6.2, 1.0)
        }
        "brush" => {
            p("M16.4 3.6a1.4 1.4 0 0 0-2 0L8.5 9.5l2 2 5.9-5.9a1.4 1.4 0 0 0 0-2Z")
                + &p("M8 10.5l1.5 1.5c.5 2-1 4.5-5.5 4.5 1-1 .8-2.6 1.5-4 .6-1.2 1.6-2 2.5-2Z")
        }
        "color" => c(10.0, 7.25, 4.0) + &c(7.25, 12.0, 4.0) + &c(12.75, 12.0, 4.0),
        "theme" => c(10.0, 10.0, 7.0) + &pe("M10 3a7 7 0 0 1 0 14Z", FILL),
        "clock" => circle() + &clock_hands(),
        "timer" => c(10.0, 11.0, 6.5) + &p("M10 7.75V11l2 1.25M8 2.5h4M15.25 5.25l1 1"),
        "calendar" => {
            r(3.0, 3.75, 14.0, 13.5, 2.0)
                + &p("M3 7.75h14M7 2.5v2.5M13 2.5v2.5")
                + &dot(7.0, 11.0, 0.85)
                + &dot(10.0, 11.0, 0.85)
                + &dot(13.0, 11.0, 0.85)
                + &dot(7.0, 14.0, 0.85)
                + &dot(10.0, 14.0, 0.85)
        }
        "language" => {
            p("M2.75 16.5 6.25 7l3.5 9.5M3.9 13.25h4.7")
                + &p("M11 4h6M14 2.75V4M12.25 7.75c1.4 1.9 3 3 4.75 3.5M16.1 4c-.6 3-2.3 5.3-4.7 6.7")
        }
        "notification" => p(BELL) + &p("M8 15a2 2 0 0 0 4 0"),
        "notification-off" => p(BELL) + &p("M8 15a2 2 0 0 0 4 0") + &slash(),
        "focus" => circle() + &c(10.0, 10.0, 3.5),
        "gamepad" => {
            p("M6.25 5.5h7.5a4.25 4.25 0 0 1 4.2 4.9l-.5 3.15a2.25 2.25 0 0 1-3.85 1.25L12 13H8l-1.6 1.8a2.25 2.25 0 0 1-3.85-1.25l-.5-3.15A4.25 4.25 0 0 1 6.25 5.5Z")
                + &p("M6.5 8.25v2.5M5.25 9.5h2.5")
                + &dot(13.0, 8.75, 0.85)
                + &dot(14.5, 10.5, 0.85)
        }
        "bulb" => {
            p("M7 14.5v-1.6c0-.8-.45-1.5-1.05-2.05a5.25 5.25 0 1 1 8.1 0c-.6.55-1.05 1.25-1.05 2.05v1.6Z")
                + &p("M7.75 17.25h4.5")
        }
        "flag" => p("M4.5 17.5V3.5M4.5 4h10.25l-2.25 3.75 2.25 3.75H4.5"),
        "app" => r(3.0, 3.0, 14.0, 14.0, 3.0) + &r(6.5, 6.5, 7.0, 7.0, 1.5),

        // task manager
        "process" => r(2.75, 2.75, 14.5, 14.5, 2.0) + &p("M2.75 7.25h14.5M6.25 10.5h7.5M6.25 13.5h5"),
        "startup" => p("M3.5 14.5a6.5 6.5 0 1 1 13 0") + &p("M10 14.5l3-4.25") + &dot(10.0, 14.5, 1.1),
        "services" => p("M8 3.5H5A1.5 1.5 0 0 0 3.5 5v3h1a1.75 1.75 0 0 1 0 3.5h-1v3A1.5 1.5 0 0 0 5 16h3v-1a1.75 1.75 0 0 1 3.5 0v1h3a1.5 1.5 0 0 0 1.5-1.5v-3h-1a1.75 1.75 0 0 1 0-3.5h1V5A1.5 1.5 0 0 0 14.5 3.5h-3v1a1.75 1.75 0 0 1-3.5 0Z"),
        "summary" => p("M8.75 4.15A6.25 6.25 0 1 0 15.85 11.25H8.75Z") + &p("M11.25 2.75v6h6A6.25 6.25 0 0 0 11.25 2.75Z"),
        "chart" => {
            p("M3.5 16.5h13") + &r(4.75, 10.0, 2.5, 4.0, 0.75) + &r(8.75, 5.0, 2.5, 9.0, 0.75) + &r(12.75, 8.0, 2.5, 6.0, 0.75)
        }
        "clean" | "sparkle" => sparkle(),
        "broom" => r#"<g transform="rotate(40 10 10)"><path d="M10 1.75v7"/><path d="M7 8.75h6l1.25 8.5h-8.5Z"/><path d="M8.75 12.5l-.25 4.75M11.25 12.5l.25 4.75"/></g>"#.to_owned(),

        // status
        "warning" => {
            p("M8.7 3.3a1.5 1.5 0 0 1 2.6 0l6.3 11.1a1.5 1.5 0 0 1-1.3 2.2H3.7a1.5 1.5 0 0 1-1.3-2.2Z")
                + &p("M10 7.5v4")
                + &dot(10.0, 14.0, 0.95)
        }
        "error" => circle() + &p("M7.5 7.5l5 5M12.5 7.5l-5 5"),
        "success" => circle() + &p("M6.8 10.2l2.2 2.2 4.2-4.6"),

        // media
        "play" => p("M6 3.8v12.4a.8.8 0 0 0 1.2.7l10-6.2a.8.8 0 0 0 0-1.4l-10-6.2A.8.8 0 0 0 6 3.8Z"),
        "pause" => r(5.0, 3.5, 3.5, 13.0, 1.0) + &r(11.5, 3.5, 3.5, 13.0, 1.0),
        "next" => p("M4.5 4.6v10.8a.7.7 0 0 0 1.1.6l7.6-5.4a.7.7 0 0 0 0-1.2L5.6 4a.7.7 0 0 0-1.1.6Z") + &p("M15.75 4v12"),
        "previous" => {
            p("M15.5 4.6v10.8a.7.7 0 0 1-1.1.6l-7.6-5.4a.7.7 0 0 1 0-1.2L14.4 4a.7.7 0 0 1 1.1.6Z") + &p("M4.25 4v12")
        }
        _ => return None,
    };
    Some(s)
}
