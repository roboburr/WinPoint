//! The wallpaper (`git show 418ea594:vendor/ui/js/desktop/wallpaper.js` with theme.js resolveWallpaper): a picture
//! (fill, fit, stretch, center, tile, span) or a plain colour, cross-faded in 450 ms when it changes. Pictures are
//! decoded off the main thread at the size they are shown, so a 4K photo costs what the monitor shows. A picture that
//! cannot be loaded leaves a calm gradient in the wallpaper colour instead of a black screen.
//!
//! A light and dark pair (GNOME's chooser, wallpaper.pathDark) shows its dark picture in dark mode; a GNOME slideshow
//! (`.xml`, gnome-desktop's gnome-bg-slide-show.c) shows the picture that is current when it loads.
use crate::backend::Backend;
use crate::core::Shell;
use gtk::prelude::*;
use gtk::{cairo, gdk, gio, glib};
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Fit {
    Fill,
    Fit,
    Stretch,
    Center,
    Tile,
    Span,
}

pub struct Spec {
    /// "builtin:<id>", an absolute path or a file:// URI; empty for a plain colour.
    pub image: String,
    pub fit: Fit,
    pub color: String,
}

/// Where one monitor lies in the bounding box of all monitors (stage pixels), for "span" (Uitspreiden): as in Windows
/// 11 the picture covers that box and each monitor shows its own part of it. Parts of the box outside every monitor
/// are simply never shown.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Span {
    /// The monitor's place in the box and its size.
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    /// The size of the box.
    pub box_w: i32,
    pub box_h: i32,
}

/// The place of every monitor in desktop.geometry's `monitors` (stage rectangles) for "span", by monitor index; none
/// with fewer than two monitors (one monitor shows the picture as Opvullen, as in Windows).
pub fn spans(geo: &serde_json::Value) -> std::collections::HashMap<i64, Span> {
    let list: Vec<(i64, i32, i32, i32, i32)> = geo["monitors"]
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|m| {
                    let get = |k: &str| m[k].as_i64().map(|v| v as i32);
                    let (w, h) = (get("w")?, get("h")?);
                    (w > 0 && h > 0).then_some((m["index"].as_i64()?, get("x")?, get("y")?, w, h))
                })
                .collect()
        })
        .unwrap_or_default();
    if list.len() < 2 {
        return Default::default();
    }
    let left = list.iter().map(|m| m.1).min().unwrap_or(0);
    let top = list.iter().map(|m| m.2).min().unwrap_or(0);
    let right = list.iter().map(|m| m.1 + m.3).max().unwrap_or(0);
    let bottom = list.iter().map(|m| m.2 + m.4).max().unwrap_or(0);
    list.iter()
        .map(|&(index, x, y, w, h)| (index, Span { x: x - left, y: y - top, w, h, box_w: right - left, box_h: bottom - top }))
        .collect()
}

/// The part of a picture of `image` pixels that a desktop window shows for "span": the picture covers the box
/// (centred, as Opvullen does on one monitor) and the window, `own` device pixels from its monitor's top left corner,
/// shows the part at its monitor's place. `device` is the window's device pixels per stage pixel. (x, y, w, h) in the
/// picture's pixels, inside it.
pub fn span_crop(image: (i32, i32), span: Span, own: (i32, i32), device: f64) -> (i32, i32, i32, i32) {
    let (iw, ih) = (image.0.max(1) as f64, image.1.max(1) as f64);
    let (bw, bh) = (span.box_w.max(1) as f64 * device, span.box_h.max(1) as f64 * device);
    let cover = (bw / iw).max(bh / ih);
    let (ox, oy) = ((iw * cover - bw) / 2.0, (ih * cover - bh) / 2.0);
    let x = ((span.x as f64 * device + ox) / cover).floor().clamp(0.0, iw - 1.0);
    let y = ((span.y as f64 * device + oy) / cover).floor().clamp(0.0, ih - 1.0);
    let w = (own.0.max(1) as f64 / cover).round().clamp(1.0, iw - x);
    let h = (own.1.max(1) as f64 / cover).round().clamp(1.0, ih - y);
    (x as i32, y as i32, w as i32, h as i32)
}

pub struct Wallpaper {
    pub widget: gtk::Stack,
    key: RefCell<String>,
    ticket: Cell<u64>,
    /// The device-pixel size the current picture was decoded for.
    decoded: Cell<(i32, i32)>,
    /// Its monitor's place among all monitors, with two or more (for "span").
    span: Cell<Option<Span>>,
}

impl Wallpaper {
    pub fn new() -> Rc<Wallpaper> {
        let stack = gtk::Stack::new();
        stack.set_transition_type(gtk::StackTransitionType::Crossfade);
        stack.set_transition_duration(450);
        stack.add_css_class("dt-wall");
        stack.set_can_target(false);
        stack.set_hexpand(true);
        stack.set_vexpand(true);
        Rc::new(Wallpaper {
            widget: stack,
            key: RefCell::new(String::new()),
            ticket: Cell::new(0),
            decoded: Cell::new((0, 0)),
            span: Cell::new(None),
        })
    }

    /// Its monitor's place among all monitors (`spans`); true when it changed. Only "span" shows it (the next apply).
    pub fn set_span(&self, span: Option<Span>) -> bool {
        self.span.replace(span) != span
    }

    /// Show the wallpaper of the current settings. `size` is the logical size of the desktop, `scale` its device
    /// scale; nothing happens when it is already shown, unless the desktop grew beyond what it was decoded for.
    pub fn apply(self: &Rc<Self>, shell: &Rc<Shell>, size: (i32, i32), scale: i32, instant: bool) {
        let spec = spec(shell);
        let span = if spec.fit == Fit::Span { self.span.get() } else { None };
        let mut key = format!("{}|{:?}|{}", spec.image, spec.fit, spec.color);
        if let Some(span) = span {
            // the part of the picture depends on the monitor's place and on the window's size
            key.push_str(&format!("|{span:?}|{size:?}|{scale}"));
        }
        let device = (size.0.max(1) * scale.max(1), size.1.max(1) * scale.max(1));
        let (dw, dh) = self.decoded.get();
        let grew = !spec.image.is_empty() && (device.0 > dw || device.1 > dh);
        if *self.key.borrow() == key && !grew {
            return;
        }
        let same = *self.key.borrow() == key;
        *self.key.borrow_mut() = key;
        let ticket = self.ticket.get() + 1;
        self.ticket.set(ticket);
        let color = gdk::RGBA::parse(&spec.color).unwrap_or(gdk::RGBA::new(0.043, 0.165, 0.29, 1.0));
        if spec.image.is_empty() {
            self.decoded.set((0, 0));
            self.show(page(color, None, spec.fit, false), instant);
            return;
        }
        let backend = shell.backend.clone();
        let image = spec.image.clone();
        let fit = spec.fit;
        let me = self.clone();
        glib::spawn_future_local(async move {
            let texture = gio::spawn_blocking(move || load(&backend, &image, fit, device, span)).await.ok().flatten();
            if me.ticket.get() != ticket {
                return; // a newer wallpaper won
            }
            let failed = texture.is_none();
            if failed {
                eprintln!("win11-shell: desktop: the wallpaper could not be loaded");
            }
            me.decoded.set(if failed { (0, 0) } else { device });
            me.show(page(color, texture, fit, failed), instant || same);
        });
    }

    fn show(&self, page: gtk::Widget, instant: bool) {
        let mut old = Vec::new();
        let mut child = self.widget.first_child();
        while let Some(c) = child {
            child = c.next_sibling();
            old.push(c);
        }
        self.widget.add_child(&page);
        let instant = instant || old.is_empty();
        if instant {
            self.widget.set_transition_type(gtk::StackTransitionType::None);
        }
        self.widget.set_visible_child(&page);
        self.widget.set_transition_type(gtk::StackTransitionType::Crossfade);
        let stack = self.widget.downgrade();
        let remove = move || {
            if let Some(stack) = stack.upgrade() {
                for c in &old {
                    if c.parent().as_ref() == Some(stack.upcast_ref::<gtk::Widget>()) {
                        stack.remove(c);
                    }
                }
            }
        };
        if instant {
            remove();
        } else {
            glib::timeout_add_local_once(Duration::from_millis(520), remove);
        }
    }
}

/// The wallpaper of the settings (wallpaper.type/path/color/fit/followTheme), like wallpaper.js spec().
pub fn spec(shell: &Shell) -> Spec {
    let settings = shell.settings();
    let w = &settings["wallpaper"];
    let fit = match w["fit"].as_str() {
        Some("fit") => Fit::Fit,
        Some("stretch") => Fit::Stretch,
        Some("center") => Fit::Center,
        Some("tile") => Fit::Tile,
        Some("span") => Fit::Span,
        _ => Fit::Fill,
    };
    let color = w["color"]
        .as_str()
        .filter(|c| c.starts_with('#') && (4..=9).contains(&c.len()) && c[1..].chars().all(|x| x.is_ascii_hexdigit()))
        .unwrap_or("#0b2a4a")
        .to_owned();
    let image = if w["type"] == "color" { String::new() } else { resolve_path(w, shell.dark()) };
    Spec { image, fit, color }
}

/// Built-in wallpapers that were renamed (old id, new id): settings saved before the rename still name the old id.
const ALIASES: &[(&str, &str)] = &[("bloom-light", "lotus-light"), ("bloom-dark", "lotus-dark")];

/// "builtin:bloom-dark" becomes "builtin:lotus-dark" (an old built-in id); every other path stays as it is.
fn builtin_alias(path: &str) -> String {
    if let Some(id) = path.strip_prefix("builtin:")
        && let Some((_, new)) = ALIASES.iter().find(|(old, _)| *old == id)
    {
        return format!("builtin:{new}");
    }
    path.to_owned()
}

/// theme.js resolveWallpaperPath: with followTheme, the built-in lotus and flow pictures use the twin of the
/// current light/dark mode, and a light and dark pair of GNOME's chooser its dark picture (wallpaper.pathDark) in
/// dark mode. An old builtin:bloom-... id means the lotus picture; a settings file without pathDark has none.
fn resolve_path(w: &serde_json::Value, dark: bool) -> String {
    let follow = w["followTheme"] != false;
    let builtin = w["path"].as_str().is_some_and(|p| p.starts_with("builtin:"));
    if dark
        && follow
        && !builtin
        && let Some(path_dark) = w["pathDark"].as_str().filter(|p| !p.trim().is_empty())
    {
        return builtin_alias(path_dark);
    }
    let mut path = builtin_alias(w["path"].as_str().filter(|p| !p.is_empty()).unwrap_or("builtin:lotus-dark"));
    if follow && let Some(id) = path.strip_prefix("builtin:") {
        for base in ["lotus", "flow"] {
            if id == format!("{base}-light") || id == format!("{base}-dark") {
                path = format!("builtin:{base}-{}", if dark { "dark" } else { "light" });
                break;
            }
        }
    }
    path
}

/// Where the UI's own files live (built-in wallpapers): as host/src/lib.rs ui_root, plus the user's install.
fn ui_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(dir) = std::env::var_os("WIN11_UI_DIR") {
        roots.push(PathBuf::from(dir));
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        roots.push(dir.join("../share/win11-gnome/ui"));
    }
    roots.push(glib::user_data_dir().join("win11-gnome/ui"));
    roots.push(PathBuf::from("/usr/share/win11-gnome/ui"));
    roots
}

/// The bytes of a wallpaper picture; for a GNOME slideshow (`.xml`) those of its picture that is current now, the
/// size of it that suits `device` best.
fn read(backend: &Backend, image: &str, device: (i32, i32)) -> Option<Vec<u8>> {
    let bytes = read_file(backend, image)?;
    if !is_slideshow(image) {
        return Some(bytes);
    }
    let show = parse_slideshow(&String::from_utf8_lossy(&bytes))?;
    let now = glib::DateTime::now_local().ok().map(|t| t.to_unix() as f64);
    let file = show.current(now.zip(show.start_unix()), device)?;
    // a slideshow names pictures, not other slideshows
    if is_slideshow(&file) {
        return None;
    }
    read_file(backend, &file)
}

/// Whether a wallpaper path is a GNOME slideshow (gnome-bg.c: the name ends in ".xml").
fn is_slideshow(image: &str) -> bool {
    image.to_ascii_lowercase().ends_with(".xml")
}

fn read_file(backend: &Backend, image: &str) -> Option<Vec<u8>> {
    let image = builtin_alias(image);
    let image = image.as_str();
    if let Some(id) = image.strip_prefix("builtin:") {
        if id.is_empty() || id.contains('/') || id.contains("..") {
            return None;
        }
        for root in ui_roots() {
            if let Ok(bytes) = std::fs::read(root.join("assets/wallpapers").join(format!("{id}.svg"))) {
                return Some(bytes);
            }
        }
        return backend.get(&format!("/ui/assets/wallpapers/{}.svg", super::enc(id))).ok();
    }
    let path = if image.starts_with("file://") {
        gio::File::for_uri(image).path()?
    } else if image.starts_with('/') {
        PathBuf::from(image)
    } else {
        return None; // http(s) wallpapers are not fetched by the desktop
    };
    std::fs::read(&path)
        .ok()
        .or_else(|| backend.get(&format!("/file?path={}", super::enc(&path.to_string_lossy()))).ok())
}

fn load(backend: &Backend, image: &str, fit: Fit, device: (i32, i32), span: Option<Span>) -> Option<gdk::Texture> {
    let bytes = read(backend, image, device)?;
    if let Some(span) = span {
        // decoded to cover the whole box (a raster picture only shrinks), then this monitor's part of it: shown with
        // "cover" below it fills the window exactly
        let per = device.0.max(1) as f64 / span.w.max(1) as f64;
        let whole = ((span.box_w as f64 * per).round() as i32, (span.box_h as f64 * per).round() as i32);
        let pixbuf = super::decode_pixbuf(&bytes, Some((whole, true)))?;
        let (x, y, w, h) = span_crop((pixbuf.width(), pixbuf.height()), span, device, per);
        return pixbuf.new_subpixbuf(x, y, w, h).copy().map(|part| super::texture_of(&part));
    }
    let target = match fit {
        Fit::Center | Fit::Tile => None,
        Fit::Fit => Some((device, false)),
        _ => Some((device, true)),
    };
    super::decode(&bytes, target)
}

/* ------------------------------------------------------------- slideshows */

/// One picture of a slideshow slide: a `<file>` with its path, or one `<size width height>` of it (-1: not given).
#[derive(Clone, Debug, PartialEq)]
struct SlideFile {
    width: i32,
    height: i32,
    path: String,
}

/// A `<static>` slide (`to` empty) or a `<transition>` from one picture to the next.
#[derive(Clone, Debug, Default, PartialEq)]
struct Slide {
    duration: f64,
    from: Vec<SlideFile>,
    to: Vec<SlideFile>,
}

/// A GNOME slideshow as gnome-desktop's gnome-bg-slide-show.c reads it: `<background>` with an optional
/// `<starttime>` and its `<static>` and `<transition>` slides in order.
#[derive(Debug, Default, PartialEq)]
struct Slideshow {
    start: [i32; 6],
    slides: Vec<Slide>,
}

/// The text of XML character data with the predefined entities and character references resolved.
fn xml_text(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(end) = rest.find(';').filter(|e| *e <= 10) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let name = &rest[1..end];
        let ch = match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => name
                .strip_prefix("#x")
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| name.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// An attribute of a start tag's text (`size width="1920" height="1080"`).
fn xml_attr(tag: &str, name: &str) -> Option<String> {
    let mut rest = tag;
    while let Some(i) = rest.find(name) {
        let before = rest[..i].chars().next_back();
        let after = rest[i + name.len()..].trim_start();
        rest = &rest[i + name.len()..];
        if !before.is_some_and(char::is_whitespace) {
            continue;
        }
        let Some(value) = after.strip_prefix('=').map(str::trim_start) else { continue };
        let quote = value.chars().next().filter(|q| *q == '"' || *q == '\'')?;
        let value = &value[1..];
        return value.find(quote).map(|end| xml_text(&value[..end]));
    }
    None
}

/// The slideshow of a GNOME background `.xml` (None: not one, or not well-formed).
fn parse_slideshow(text: &str) -> Option<Slideshow> {
    let mut show = Slideshow::default();
    let mut stack: Vec<(String, String, String)> = Vec::new(); // (name, start tag, character data)
    let mut root = None;
    let mut rest = text;
    loop {
        let Some(lt) = rest.find('<') else {
            break;
        };
        if let Some((_, _, data)) = stack.last_mut() {
            data.push_str(&xml_text(&rest[..lt]));
        }
        rest = &rest[lt..];
        if let Some(body) = rest.strip_prefix("<!--") {
            rest = &body[body.find("-->")? + 3..];
            continue;
        }
        if let Some(body) = rest.strip_prefix("<![CDATA[") {
            let end = body.find("]]>")?;
            if let Some((_, _, data)) = stack.last_mut() {
                data.push_str(&body[..end]);
            }
            rest = &body[end + 3..];
            continue;
        }
        let end = rest.find('>')?;
        let tag = &rest[1..end];
        rest = &rest[end + 1..];
        if tag.starts_with('?') || tag.starts_with('!') {
            continue;
        }
        if let Some(name) = tag.strip_prefix('/') {
            let (open, start, data) = stack.pop()?;
            if open != name.trim() {
                return None;
            }
            slideshow_element(&mut show, &stack, &open, &start, &data);
            continue;
        }
        let empty = tag.ends_with('/');
        let tag = tag.trim_end_matches('/');
        let name = tag.split(|c: char| c.is_whitespace()).next().unwrap_or("").to_owned();
        if name.is_empty() {
            return None;
        }
        if stack.is_empty() {
            if root.is_some() {
                return None;
            }
            root = Some(name.clone());
        }
        if stack.len() == 1 && (name == "static" || name == "transition") {
            show.slides.push(Slide::default());
        }
        if empty {
            slideshow_element(&mut show, &stack, &name, tag, "");
        } else {
            stack.push((name, tag.to_owned(), String::new()));
        }
    }
    (stack.is_empty() && root.as_deref() == Some("background") && !show.slides.is_empty()).then_some(show)
}

/// One closed element of a slideshow (`parents` the open ones above it), as gnome-bg-slide-show.c's handle_text.
fn slideshow_element(show: &mut Slideshow, parents: &[(String, String, String)], name: &str, tag: &str, data: &str) {
    let path: Vec<&str> = parents.iter().map(|(n, _, _)| n.as_str()).chain([name]).collect();
    let text = data.trim();
    let file = |width: i32, height: i32| SlideFile { width, height, path: text.to_owned() };
    match path.as_slice() {
        ["background", "starttime", field] => {
            let i = ["year", "month", "day", "hour", "minute", "second"].iter().position(|f| f == field);
            if let (Some(i), Ok(v)) = (i, text.parse::<i32>()) {
                show.start[i] = v;
            }
        }
        ["background", "static" | "transition", "duration"] => {
            if let (Some(slide), Ok(d)) = (show.slides.last_mut(), text.parse::<f64>()) {
                slide.duration = d;
            }
        }
        ["background", "static", "file"] | ["background", "transition", "from"] if !text.is_empty() => {
            if let Some(slide) = show.slides.last_mut() {
                slide.from.push(file(-1, -1));
            }
        }
        ["background", "static", "file", "size"] if !text.is_empty() => {
            let size = |k: &str| xml_attr(tag, k).and_then(|v| v.trim().parse::<i32>().ok()).unwrap_or(-1);
            if let Some(slide) = show.slides.last_mut() {
                slide.from.push(file(size("width"), size("height")));
            }
        }
        ["background", "transition", "to"] if !text.is_empty() => {
            if let Some(slide) = show.slides.last_mut() {
                slide.to.push(file(-1, -1));
            }
        }
        _ => {}
    }
}

/// gnome-bg-slide-show.c find_best_size: the size closest to the monitor's aspect among those at least as large,
/// else among all; on a tie the one closest in width. A `<file>` with only a path is that path.
fn best_size(files: &[SlideFile], want: (i32, i32)) -> Option<&SlideFile> {
    if files.len() <= 1 || want.0 <= 0 || want.1 <= 0 {
        return files.first();
    }
    let aspect = want.0 as f64 / want.1 as f64;
    for large_only in [true, false] {
        let mut best: Option<&SlideFile> = None;
        let mut distance = 10000.0;
        for f in files {
            if large_only && (f.width < want.0 || f.height < want.1) {
                continue;
            }
            let d = (aspect - f.width as f64 / f.height.max(1) as f64).abs();
            if d < distance {
                distance = d;
                best = Some(f);
            } else if d == distance && best.is_some_and(|b| (f.width - want.0).abs() < (b.width - want.0).abs()) {
                best = Some(f);
            }
        }
        if best.is_some() {
            return best;
        }
    }
    files.first()
}

impl Slideshow {
    /// When it starts (its `<starttime>` in local time, as gnome-bg-slide-show.c's mktime), in seconds since the
    /// epoch; the epoch without a start time.
    fn start_unix(&self) -> Option<f64> {
        if self.start == [0; 6] {
            return Some(0.0);
        }
        let [y, mo, d, h, mi, s] = self.start;
        glib::DateTime::from_local(y, mo, d, h, mi, s as f64).ok().map(|t| t.to_unix() as f64)
    }

    /// The picture to show: with `clock` (now, start; seconds since the epoch) the slide that is current, as
    /// gnome-bg-slide-show.c (the time since the start modulo the total of the durations; a transition shows the
    /// picture it is closer to), else the first slide's. Only absolute paths count.
    fn current(&self, clock: Option<(f64, f64)>, want: (i32, i32)) -> Option<String> {
        let total: f64 = self.slides.iter().map(|s| s.duration.max(0.0)).sum();
        let mut pick: Option<(&Slide, f64)> = self.slides.first().map(|s| (s, 0.0));
        if let Some((now, start)) = clock.filter(|_| total > 0.0) {
            let delta = (now - start).rem_euclid(total);
            let mut elapsed = 0.0;
            for slide in &self.slides {
                let duration = slide.duration.max(0.0);
                if elapsed + duration > delta {
                    let progress = if duration > 0.0 { (delta - elapsed) / duration } else { 0.0 };
                    pick = Some((slide, progress));
                    break;
                }
                elapsed += duration;
            }
        }
        let (slide, progress) = pick?;
        let files = if !slide.to.is_empty() && progress >= 0.5 { &slide.to } else { &slide.from };
        best_size(files, want).map(|f| f.path.clone()).filter(|p| p.starts_with('/'))
    }
}

/// A texture as a cairo surface (center and tile are painted, not scaled).
fn surface(texture: &gdk::Texture) -> Option<cairo::ImageSurface> {
    let (w, h) = (texture.width(), texture.height());
    if w <= 0 || h <= 0 {
        return None;
    }
    let stride = w as usize * 4;
    let mut data = vec![0u8; stride * h as usize];
    // GDK's default memory format is premultiplied BGRA on little endian: cairo's ARGB32.
    texture.download(&mut data, stride);
    cairo::ImageSurface::create_for_data(data, cairo::Format::ARgb32, w, h, stride as i32).ok()
}

fn page(color: gdk::RGBA, texture: Option<gdk::Texture>, fit: Fit, failed: bool) -> gtk::Widget {
    let overlay = gtk::Overlay::new();
    overlay.set_hexpand(true);
    overlay.set_vexpand(true);
    let base = gtk::DrawingArea::new();
    base.set_hexpand(true);
    base.set_vexpand(true);
    let painted = match (&texture, fit) {
        (Some(t), Fit::Center | Fit::Tile) => surface(t),
        _ => None,
    };
    let (r, g, b) = (color.red() as f64, color.green() as f64, color.blue() as f64);
    base.set_draw_func(move |_, cr, w, h| {
        cr.set_source_rgb(r, g, b);
        let _ = cr.paint();
        if failed {
            // radial-gradient(120% 90% at 30% 20%, mix(colour 60%, #4a8fe0), colour 60%, #050b16)
            let (w, h) = (w as f64, h as f64);
            let gradient = cairo::RadialGradient::new(0.3 * w, 0.2 * h, 0.0, 0.3 * w, 0.2 * h, 1.2 * w.max(h));
            gradient.add_color_stop_rgb(0.0, r * 0.6 + 0.29 * 0.4, g * 0.6 + 0.561 * 0.4, b * 0.6 + 0.878 * 0.4);
            gradient.add_color_stop_rgb(0.5, r, g, b);
            gradient.add_color_stop_rgb(1.0, 0.02, 0.043, 0.086);
            let _ = cr.set_source(&gradient);
            let _ = cr.paint();
        }
        if let Some(s) = &painted {
            if fit == Fit::Tile {
                let pattern = cairo::SurfacePattern::create(s);
                pattern.set_extend(cairo::Extend::Repeat);
                let _ = cr.set_source(&pattern);
            } else {
                let x = ((w - s.width()) / 2) as f64;
                let y = ((h - s.height()) / 2) as f64;
                let _ = cr.set_source_surface(s, x, y);
            }
            let _ = cr.paint();
        }
    });
    overlay.set_child(Some(&base));
    if let Some(texture) = texture.filter(|_| !matches!(fit, Fit::Center | Fit::Tile)) {
        let picture = gtk::Picture::for_paintable(&texture);
        picture.set_can_shrink(true);
        picture.set_content_fit(match fit {
            Fit::Fit => gtk::ContentFit::Contain,
            Fit::Stretch => gtk::ContentFit::Fill,
            _ => gtk::ContentFit::Cover,
        });
        picture.set_hexpand(true);
        picture.set_vexpand(true);
        overlay.add_overlay(&picture);
    }
    overlay.upcast()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // the harness's two monitors: 1440 x 900 primary, 1280 x 960 right of it (part of the box lies outside both)
    fn two() -> serde_json::Value {
        json!({"monitor": 0, "monitors": [{"index": 0, "x": 0, "y": 0, "w": 1440, "h": 900},
            {"index": 1, "x": 1440, "y": 0, "w": 1280, "h": 960}]})
    }

    #[test]
    fn a_light_and_dark_pair_shows_its_dark_picture_in_dark_mode() {
        let pair = json!({"path": "/usr/share/backgrounds/a-l.jpg", "pathDark": "/usr/share/backgrounds/a-d.jpg"});
        assert_eq!(resolve_path(&pair, false), "/usr/share/backgrounds/a-l.jpg");
        assert_eq!(resolve_path(&pair, true), "/usr/share/backgrounds/a-d.jpg");
        // "Achtergrond aanpassen aan thema" off: the picture chosen, in both modes
        let mut fixed = pair.clone();
        fixed["followTheme"] = json!(false);
        assert_eq!(resolve_path(&fixed, true), "/usr/share/backgrounds/a-l.jpg");
        // no pair (an empty pathDark, or an old settings file without the key): as before
        for w in [
            json!({"path": "/home/robin/foto.jpg", "pathDark": ""}),
            json!({"path": "/home/robin/foto.jpg"}),
            json!({"path": "/home/robin/foto.jpg", "pathDark": null}),
        ] {
            assert_eq!(resolve_path(&w, true), "/home/robin/foto.jpg");
            assert_eq!(resolve_path(&w, false), "/home/robin/foto.jpg");
        }
        // the built-in twins keep following the mode, and an old id in pathDark is the new one
        assert_eq!(resolve_path(&json!({"path": "builtin:lotus-dark"}), false), "builtin:lotus-light");
        assert_eq!(resolve_path(&json!({"path": "builtin:flow-light"}), true), "builtin:flow-dark");
        assert_eq!(resolve_path(&json!({"path": "builtin:flow-light", "followTheme": false}), true), "builtin:flow-light");
        assert_eq!(resolve_path(&json!({"path": "builtin:lotus-light", "pathDark": "builtin:bloom-dark"}), true), "builtin:lotus-dark");
        // A theme replaces path with a built-in: a stale dark file must not override that theme.
        assert_eq!(resolve_path(&json!({"path": "builtin:flow-light", "pathDark": "/old-dark.jpg"}), true), "builtin:flow-dark");
        assert_eq!(resolve_path(&json!({"path": "builtin:flow-light", "pathDark": "/old-dark.jpg"}), false), "builtin:flow-light");
    }

    const SLIDESHOW: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!-- a GNOME timed slideshow (gnome-bg-slide-show.c) -->
<background>
  <starttime>
    <year>2026</year><month>11</month><day>17</day>
    <hour>00</hour><minute>00</minute><second>00</second>
  </starttime>
  <static>
    <duration>1795.0</duration>
    <file>/usr/share/backgrounds/a&amp;b.jpg</file>
  </static>
  <transition type="overlay">
    <duration>5.0</duration>
    <from>/usr/share/backgrounds/a&amp;b.jpg</from>
    <to>/usr/share/backgrounds/c.png</to>
  </transition>
  <static>
    <duration>1795.0</duration>
    <file>
      <size width="1024" height="768">/usr/share/backgrounds/c-1024.png</size>
      <size width="1920" height="1080">/usr/share/backgrounds/c.png</size>
      <size width="3840" height="2160">/usr/share/backgrounds/c-4k.png</size>
    </file>
  </static>
  <transition>
    <duration>5.0</duration>
    <from>/usr/share/backgrounds/c.png</from>
    <to>/usr/share/backgrounds/a&amp;b.jpg</to>
  </transition>
</background>
"#;

    #[test]
    fn a_slideshow_shows_its_first_picture_or_the_current_one() {
        let show = parse_slideshow(SLIDESHOW).expect("a slideshow");
        assert_eq!(show.start, [2026, 11, 17, 0, 0, 0]);
        assert_eq!(show.slides.len(), 4);
        assert_eq!(show.slides[2].from.len(), 3);
        assert_eq!(show.slides[2].from[1], SlideFile { width: 1920, height: 1080, path: "/usr/share/backgrounds/c.png".into() });
        let hd = (1920, 1080);
        // without the time: the first static picture (as host background::slideshow_preview)
        assert_eq!(show.current(None, hd).as_deref(), Some("/usr/share/backgrounds/a&b.jpg"));
        // with it: the slide of the time since the start, modulo the 3600 s of the whole show
        let start = 1_000_000.0;
        let at = |secs: f64, want| show.current(Some((start + secs, start)), want);
        assert_eq!(at(10.0, hd).as_deref(), Some("/usr/share/backgrounds/a&b.jpg"));
        assert_eq!(at(1796.0, hd).as_deref(), Some("/usr/share/backgrounds/a&b.jpg"), "early in the transition");
        assert_eq!(at(1799.0, hd).as_deref(), Some("/usr/share/backgrounds/c.png"), "late in the transition");
        assert_eq!(at(2000.0, hd).as_deref(), Some("/usr/share/backgrounds/c.png"));
        assert_eq!(at(3600.0 * 5.0 + 2000.0, hd).as_deref(), Some("/usr/share/backgrounds/c.png"), "the show loops");
        assert_eq!(at(-1600.0, hd).as_deref(), Some("/usr/share/backgrounds/c.png"), "before the start: from the end");
        // the size that suits the monitor: at least as large and of its aspect, else the closest aspect
        assert_eq!(at(2000.0, (1440, 900)).as_deref(), Some("/usr/share/backgrounds/c.png"));
        assert_eq!(at(2000.0, (1024, 768)).as_deref(), Some("/usr/share/backgrounds/c-1024.png"));
        assert_eq!(at(2000.0, (2560, 1440)).as_deref(), Some("/usr/share/backgrounds/c-4k.png"));
        assert_eq!(at(2000.0, (7680, 4320)).as_deref(), Some("/usr/share/backgrounds/c-4k.png"), "none large enough: aspect, then width");
        // no start time: the epoch
        let no_start = SLIDESHOW.replace("<year>2026</year><month>11</month><day>17</day>", "").replace(
            "<hour>00</hour><minute>00</minute><second>00</second>",
            "",
        );
        assert_eq!(parse_slideshow(&no_start).and_then(|s| s.start_unix()), Some(0.0));
        assert!(show.start_unix().is_some_and(|s| s > 1.7e9));
    }

    #[test]
    fn what_is_no_slideshow_shows_nothing() {
        assert!(is_slideshow("/usr/share/backgrounds/resolute.xml") && is_slideshow("/x/Show.XML"));
        assert!(!is_slideshow("/usr/share/backgrounds/a.jpg") && !is_slideshow("builtin:lotus-dark"));
        // not well-formed, another root, no slides
        assert_eq!(parse_slideshow("<background><static><file>/a.jpg</file></background>"), None);
        assert_eq!(parse_slideshow("<wallpapers><static><duration>5</duration><file>/a.jpg</file></static></wallpapers>"), None);
        assert_eq!(parse_slideshow("<background><starttime><year>2026</year></starttime></background>"), None);
        assert_eq!(parse_slideshow("not xml at all"), None);
        // a relative path is not taken (gnome-bg-slide-show.c needs absolute ones)
        let rel = parse_slideshow("<background><static><duration>5</duration><file>a.jpg</file></static></background>").unwrap();
        assert_eq!(rel.current(None, (1920, 1080)), None);
        // entities, CDATA and attributes
        assert_eq!(xml_text("a &amp; b &lt;c&gt; &#233;&#x20AC; &bogus; & d"), "a & b <c> \u{e9}\u{20ac} &bogus; & d");
        assert_eq!(xml_attr(r#"size width="1920" height='1080'"#, "height").as_deref(), Some("1080"));
        assert_eq!(xml_attr(r#"size xwidth="1" width = "2""#, "width").as_deref(), Some("2"));
        let cdata = parse_slideshow("<background><static><duration>5</duration><file><![CDATA[/a&b.jpg]]></file></static></background>");
        assert_eq!(cdata.unwrap().current(None, (0, 0)).as_deref(), Some("/a&b.jpg"));
    }

    #[test]
    fn span_places_every_monitor_in_the_bounding_box() {
        let both = spans(&two());
        assert_eq!(both[&0], Span { x: 0, y: 0, w: 1440, h: 900, box_w: 2720, box_h: 960 });
        assert_eq!(both[&1], Span { x: 1440, y: 0, w: 1280, h: 960, box_w: 2720, box_h: 960 });
        // one monitor (or none): no span, the picture shows as Opvullen
        assert!(spans(&json!({"monitors": [{"index": 0, "x": 0, "y": 0, "w": 1440, "h": 900}]})).is_empty());
        assert!(spans(&json!({"w": 1440, "h": 900})).is_empty());
        // a monitor left of and above the primary: the box starts at its corner
        let left = spans(&json!({"monitors": [{"index": 0, "x": 0, "y": 0, "w": 1920, "h": 1080},
            {"index": 1, "x": -1280, "y": -200, "w": 1280, "h": 1024}]}));
        assert_eq!(left[&0], Span { x: 1280, y: 200, w: 1920, h: 1080, box_w: 3200, box_h: 1280 });
        assert_eq!(left[&1], Span { x: 0, y: 0, w: 1280, h: 1024, box_w: 3200, box_h: 1280 });
    }

    #[test]
    fn span_shows_each_monitor_its_own_part_of_one_picture() {
        // the scenario's 1600 x 1000 four-colour picture: covering 2720 x 960 it is 1.7 times larger and 370 px of
        // its height (scaled) is cut, half at the top
        let both = spans(&two());
        // the primary's desktop window is its work area (852 px high): the left 847 picture pixels from y 217
        let (x, y, w, h) = span_crop((1600, 1000), both[&0], (1440, 852), 1.0);
        assert_eq!((x, y, w, h), (0, 217, 847, 501));
        // monitor 2 goes on where monitor 1 stops
        let (x2, y2, w2, h2) = span_crop((1600, 1000), both[&1], (1280, 960), 1.0);
        assert_eq!((x2, y2), (847, 217));
        assert_eq!((w2, h2), (753, 565));
        assert!(x2 + w2 <= 1600 && y2 + h2 <= 1000);
        // a picture decoded at twice the size for a monitor of scale 2: the same part, in its own pixels
        let (x, y, w, h) = span_crop((3200, 2000), both[&1], (2560, 1920), 2.0);
        assert_eq!((x, y, w, h), (1694, 435, 1506, 1129));
    }
}
