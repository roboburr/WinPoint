//! The wallpaper (vendor/ui/js/desktop/wallpaper.js with theme.js resolveWallpaper): a picture (fill, fit,
//! stretch, center, tile, span) or a plain colour, cross-faded in 450 ms when it changes. Pictures are decoded off
//! the main thread at the size they are shown, so a 4K photo costs what the monitor shows. A picture that cannot
//! be loaded leaves a calm gradient in the wallpaper colour instead of a black screen.
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

pub struct Wallpaper {
    pub widget: gtk::Stack,
    key: RefCell<String>,
    ticket: Cell<u64>,
    /// The device-pixel size the current picture was decoded for.
    decoded: Cell<(i32, i32)>,
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
        Rc::new(Wallpaper { widget: stack, key: RefCell::new(String::new()), ticket: Cell::new(0), decoded: Cell::new((0, 0)) })
    }

    /// Show the wallpaper of the current settings. `size` is the logical size of the desktop, `scale` its device
    /// scale; nothing happens when it is already shown, unless the desktop grew beyond what it was decoded for.
    pub fn apply(self: &Rc<Self>, shell: &Rc<Shell>, size: (i32, i32), scale: i32, instant: bool) {
        let spec = spec(shell);
        let key = format!("{}|{:?}|{}", spec.image, spec.fit, spec.color);
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
            let texture = gio::spawn_blocking(move || load(&backend, &image, fit, device)).await.ok().flatten();
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
/// current light/dark mode. An old builtin:bloom-... id means the lotus picture.
fn resolve_path(w: &serde_json::Value, dark: bool) -> String {
    let mut path = builtin_alias(w["path"].as_str().filter(|p| !p.is_empty()).unwrap_or("builtin:lotus-dark"));
    if w["followTheme"] != false
        && let Some(id) = path.strip_prefix("builtin:")
    {
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

fn read(backend: &Backend, image: &str) -> Option<Vec<u8>> {
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

fn load(backend: &Backend, image: &str, fit: Fit, device: (i32, i32)) -> Option<gdk::Texture> {
    let bytes = read(backend, image)?;
    let target = match fit {
        Fit::Center | Fit::Tile => None,
        Fit::Fit => Some((device, false)),
        _ => Some((device, true)),
    };
    super::decode(&bytes, target)
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
