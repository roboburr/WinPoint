//! The small pictures of common.js: `DesktopPreview` (a mini Windows 11 desktop: wallpaper, a small window with
//! "Aa", lines and an accent button, and the taskbar, in a given mode and accent; `.st-dp`) and `DeviceArt` (a
//! monitor showing the wallpaper; `.st-device`), drawn with GtkSnapshot so the wallpaper texture is scaled by
//! GSK instead of being copied per frame.
use crate::common;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib, graphene, gsk, pango};
use serde_json::Value;
use std::cell::{Cell, RefCell};

/// What a desktop preview shows (DesktopPreview update(o)); None takes the value of the current settings.
#[derive(Clone, Debug, Default)]
pub struct PreviewOpts {
    /// Dark or light (o.mode); None: the theme in effect.
    pub dark: Option<bool>,
    /// The accent colour (o.accent); None: theme.accent.
    pub accent: Option<String>,
    /// The wallpaper path (o.url): Some("") is none (a solid colour); None: the settings' wallpaper.
    pub path: Option<String>,
    /// The background colour (o.color); None: wallpaper.color.
    pub color: Option<String>,
    /// fill, fit, stretch, tile, center or span (o.fit); None: wallpaper.fit.
    pub fit: Option<String>,
    /// Hide the small window (o.window === false).
    pub no_window: bool,
    /// The accent-coloured taskbar (o.accentTaskbar); None: theme.accentOnTaskbar in dark mode.
    pub accent_taskbar: Option<bool>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Look {
    dark: bool,
    accent_fill: gdk::RGBA,
    color: gdk::RGBA,
    path: String,
    fit: String,
    window: bool,
    accent_taskbar: bool,
}

impl Default for Look {
    fn default() -> Look {
        Look {
            dark: true,
            accent_fill: gdk::RGBA::new(0.3, 0.76, 1.0, 1.0),
            color: gdk::RGBA::new(0.043, 0.165, 0.29, 1.0),
            path: String::new(),
            fit: "fill".into(),
            window: true,
            accent_taskbar: false,
        }
    }
}

fn rgba(s: &str) -> Option<gdk::RGBA> {
    gdk::RGBA::parse(s).ok()
}

fn resolve(opts: &PreviewOpts, s: &Value) -> Look {
    let dark = opts.dark.unwrap_or_else(kit::theme::dark);
    let accent = opts.accent.clone().or_else(|| s["theme"]["accent"].as_str().map(str::to_owned)).unwrap_or_else(|| "#0078d4".into());
    let (fill, _) = common::accent_for(&accent, dark);
    let path = match &opts.path {
        Some(p) => p.clone(),
        None => common::resolve_wallpaper_path(s, dark),
    };
    let color = opts
        .color
        .clone()
        .or_else(|| s["wallpaper"]["color"].as_str().map(str::to_owned))
        .and_then(|c| rgba(&c))
        .unwrap_or(gdk::RGBA::new(0.043, 0.165, 0.29, 1.0));
    let fit = opts.fit.clone().or_else(|| s["wallpaper"]["fit"].as_str().map(str::to_owned)).unwrap_or_else(|| "fill".into());
    let accent_taskbar = opts.accent_taskbar.unwrap_or(s["theme"]["accentOnTaskbar"] == true && dark);
    Look {
        dark,
        accent_fill: rgba(&fill).unwrap_or(gdk::RGBA::new(0.0, 0.47, 0.83, 1.0)),
        color,
        path,
        fit,
        window: !opts.no_window,
        accent_taskbar,
    }
}

fn rect(x: f32, y: f32, w: f32, h: f32) -> graphene::Rect {
    graphene::Rect::new(x, y, w.max(0.0), h.max(0.0))
}

fn rounded(x: f32, y: f32, w: f32, h: f32, r: f32) -> gsk::RoundedRect {
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    gsk::RoundedRect::from_rect(rect(x, y, w, h), r)
}

fn fill_rounded(sn: &gtk::Snapshot, x: f32, y: f32, w: f32, h: f32, r: f32, color: &gdk::RGBA) {
    sn.push_rounded_clip(&rounded(x, y, w, h, r));
    sn.append_color(color, &rect(x, y, w, h));
    sn.pop();
}

fn with_alpha(c: &gdk::RGBA, a: f32) -> gdk::RGBA {
    gdk::RGBA::new(c.red(), c.green(), c.blue(), c.alpha() * a)
}

/// The wallpaper `texture` in (x, y, w, h) the way wallpaper.fit places it (FIT_CSS of common.js).
fn draw_wallpaper(sn: &gtk::Snapshot, texture: &gdk::Texture, fit: &str, x: f32, y: f32, w: f32, h: f32) {
    let (tw, th) = (texture.width() as f32, texture.height() as f32);
    if tw <= 0.0 || th <= 0.0 {
        return;
    }
    let filter = gsk::ScalingFilter::Trilinear;
    match fit {
        "stretch" => sn.append_scaled_texture(texture, filter, &rect(x, y, w, h)),
        "fit" => {
            let s = (w / tw).min(h / th);
            let (dw, dh) = (tw * s, th * s);
            sn.append_scaled_texture(texture, filter, &rect(x + (w - dw) / 2.0, y + (h - dh) / 2.0, dw, dh));
        }
        "center" => {
            let dw = w * 0.46;
            let dh = dw * th / tw;
            sn.append_scaled_texture(texture, filter, &rect(x + (w - dw) / 2.0, y + (h - dh) / 2.0, dw, dh));
        }
        "tile" => {
            let dw = w * 0.22;
            let dh = dw * th / tw;
            if dw < 1.0 || dh < 1.0 {
                return;
            }
            // centred like background-position: center
            let ox = x + ((w / 2.0 - dw / 2.0) % dw) - dw;
            let oy = y + ((h / 2.0 - dh / 2.0) % dh) - dh;
            let mut ty = oy;
            while ty < y + h {
                let mut tx = ox;
                while tx < x + w {
                    sn.append_scaled_texture(texture, filter, &rect(tx, ty, dw, dh));
                    tx += dw;
                }
                ty += dh;
            }
        }
        _ => {
            let s = (w / tw).max(h / th);
            let (dw, dh) = (tw * s, th * s);
            sn.append_scaled_texture(texture, filter, &rect(x + (w - dw) / 2.0, y + (h - dh) / 2.0, dw, dh));
        }
    }
}

// -------------------------------------------------------------------------------------- DesktopPreview

mod dp_imp {
    use super::*;

    #[derive(Default)]
    pub struct DesktopPreview {
        pub look: RefCell<Look>,
        pub texture: RefCell<Option<gdk::Texture>>,
        pub loaded: RefCell<String>,
        pub large: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for DesktopPreview {
        const NAME: &'static str = "StSettingsDesktopPreview";
        type Type = super::DesktopPreview;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for DesktopPreview {}

    impl WidgetImpl for DesktopPreview {
        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::HeightForWidth
        }

        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            match orientation {
                gtk::Orientation::Horizontal => (48, 384, -1, -1),
                _ => {
                    let w = if for_size > 0 { for_size } else { 384 };
                    let h = (w as f64 / 1.6).round() as i32;
                    (h, h, -1, -1)
                }
            }
        }

        fn snapshot(&self, sn: &gtk::Snapshot) {
            let w = self.obj().width() as f32;
            let h = self.obj().height() as f32;
            if w < 2.0 || h < 2.0 {
                return;
            }
            let look = self.look.borrow().clone();
            let q = w / 100.0; // 1cqw
            let radius = if self.large.get() { 8.0 } else { 6.0 };
            // .st-dp.is-large: ring and flyout shadow
            if self.large.get() {
                let shadow = gdk::RGBA::new(0.0, 0.0, 0.0, if look.dark { 0.26 } else { 0.14 });
                sn.append_outset_shadow(&rounded(0.0, 0.0, w, h, radius), &shadow, 0.0, 8.0, 0.0, 16.0);
            }
            sn.push_rounded_clip(&rounded(0.0, 0.0, w, h, radius));
            sn.append_color(&look.color, &rect(0.0, 0.0, w, h));
            if let Some(t) = self.texture.borrow().as_ref() {
                draw_wallpaper(sn, t, &look.fit, 0.0, 0.0, w, h);
            }
            let (base, text1, text2, text3, stroke) = if look.dark {
                (
                    gdk::RGBA::new(0.125, 0.125, 0.125, 1.0),
                    gdk::RGBA::new(1.0, 1.0, 1.0, 1.0),
                    gdk::RGBA::new(1.0, 1.0, 1.0, 0.786),
                    gdk::RGBA::new(1.0, 1.0, 1.0, 0.5442),
                    gdk::RGBA::new(1.0, 1.0, 1.0, 0.08),
                )
            } else {
                (
                    gdk::RGBA::new(0.953, 0.953, 0.953, 1.0),
                    gdk::RGBA::new(0.0, 0.0, 0.0, 0.8956),
                    gdk::RGBA::new(0.0, 0.0, 0.0, 0.6063),
                    gdk::RGBA::new(0.0, 0.0, 0.0, 0.4458),
                    gdk::RGBA::new(0.0, 0.0, 0.0, 0.12),
                )
            };
            // ---- the small window
            if look.window {
                let (wx, wy, ww, wh) = (w * 0.18, h * 0.13, w * 0.52, h * 0.58);
                let wr = 2.4 * q;
                sn.append_outset_shadow(&rounded(wx, wy, ww, wh, wr), &gdk::RGBA::new(0.0, 0.0, 0.0, 0.3), 0.0, 1.6 * q, 0.0, 4.0 * q);
                fill_rounded(sn, wx - 1.0, wy - 1.0, ww + 2.0, wh + 2.0, wr + 1.0, &stroke);
                fill_rounded(sn, wx, wy, ww, wh, wr, &base);
                // caption dashes, right aligned in a 9 % bar
                let bar_h = wh * 0.09;
                let dash_w = 1.8 * q;
                let dash_h = (0.35 * q).max(1.0);
                for i in 0..3 {
                    let dx = wx + ww - 2.6 * q - dash_w - i as f32 * (dash_w + 2.2 * q);
                    fill_rounded(sn, dx, wy + (bar_h - dash_h) / 2.0, dash_w, dash_h, 1.0, &with_alpha(&text2, 0.8));
                }
                // body: "Aa", three lines, the accent button
                let bx = wx + 5.0 * q;
                let by = wy + bar_h + 3.0 * q;
                let bw = ww - 10.0 * q;
                let layout = self.obj().create_pango_layout(Some("Aa"));
                let mut fd = pango::FontDescription::from_string("Segoe UI Variable Display, Inter Display, Inter, Ubuntu Sans, sans-serif Semibold");
                fd.set_absolute_size((8.5 * q) as f64 * pango::SCALE as f64);
                layout.set_font_description(Some(&fd));
                let (_, logical) = layout.pixel_extents();
                sn.save();
                sn.translate(&graphene::Point::new(bx, by - 0.15 * 8.5 * q));
                sn.append_layout(&layout, &text1);
                sn.restore();
                let mut ly = by + (logical.height() as f32).min(8.5 * q * 1.3) * 0.85 + 2.4 * q;
                let line_h = (1.4 * q).max(2.0);
                for frac in [0.82f32, 0.64, 0.72] {
                    fill_rounded(sn, bx, ly, bw * frac, line_h, q, &with_alpha(&text3, 0.6));
                    ly += line_h + 1.6 * q;
                }
                let btn_h = 6.0 * q;
                fill_rounded(sn, bx, wy + wh - 4.4 * q - btn_h, bw * 0.34, btn_h, 1.2 * q, &look.accent_fill);
            }
            // ---- the taskbar: tint (no blur here) with a 1 px top stroke
            let tb_h = h * 0.095;
            let tb_y = h - tb_h;
            let tint = if look.accent_taskbar {
                let a = &look.accent_fill;
                let k = 0.55f32;
                let (r, g, b) = (16.0 / 255.0, 16.0 / 255.0, 16.0 / 255.0);
                gdk::RGBA::new(a.red() * k + r * (1.0 - k), a.green() * k + g * (1.0 - k), a.blue() * k + b * (1.0 - k), 0.9)
            } else if look.dark {
                gdk::RGBA::new(0.11, 0.11, 0.11, 0.86)
            } else {
                gdk::RGBA::new(0.933, 0.933, 0.933, 0.86)
            };
            sn.append_color(&tint, &rect(0.0, tb_y, w, tb_h));
            let top = if look.dark { gdk::RGBA::new(1.0, 1.0, 1.0, 0.075) } else { gdk::RGBA::new(0.0, 0.0, 0.0, 0.075) };
            sn.append_color(&top, &rect(0.0, tb_y, w, 1.0));
            let icon = 4.2 * q;
            let gap = 2.0 * q;
            let total = icon * 5.0 + gap * 4.0;
            let mut ix = (w - total) / 2.0;
            let iy = tb_y + (tb_h - icon) / 2.0;
            // the Start button: four accent squares
            let pad = 0.3 * q;
            let cell = (icon - 2.0 * pad - 0.45 * q) / 2.0;
            for (cx, cy) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)] {
                fill_rounded(
                    sn,
                    ix + pad + cx * (cell + 0.45 * q),
                    iy + pad + cy * (cell + 0.45 * q),
                    cell,
                    cell,
                    0.35 * q,
                    &look.accent_fill,
                );
            }
            ix += icon + gap;
            for _ in 0..4 {
                fill_rounded(sn, ix, iy, icon, icon, q, &with_alpha(&text3, 0.55));
                ix += icon + gap;
            }
            sn.pop();
        }
    }
}

glib::wrapper! {
    /// A mini Windows 11 desktop (common.js DesktopPreview, `.st-dp`), 16:10, as wide as it is given.
    pub struct DesktopPreview(ObjectSubclass<dp_imp::DesktopPreview>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl DesktopPreview {
    /// `large`: the 8 px corners and flyout shadow of `.st-dp.is-large` (Thema's, Persoonlijke instellingen).
    pub fn new(opts: &PreviewOpts, large: bool) -> DesktopPreview {
        let p: DesktopPreview = glib::Object::new();
        p.add_css_class("st-dp");
        p.imp().large.set(large);
        p.set_hexpand(true);
        p.update(opts);
        p
    }

    /// Show these options (missing ones from the current settings).
    pub fn update(&self, opts: &PreviewOpts) {
        let look = resolve(opts, &kit::ctx().settings());
        let path = look.path.clone();
        let changed = *self.imp().look.borrow() != look;
        *self.imp().look.borrow_mut() = look;
        if *self.imp().loaded.borrow() != path {
            *self.imp().loaded.borrow_mut() = path.clone();
            *self.imp().texture.borrow_mut() = None;
            if !path.is_empty() {
                let weak = self.downgrade();
                let key = path.clone();
                common::wallpaper_texture(&key, 640, move |t| {
                    if let Some(me) = weak.upgrade()
                        && *me.imp().loaded.borrow() == path
                    {
                        *me.imp().texture.borrow_mut() = t;
                        me.queue_draw();
                    }
                });
            }
            self.queue_draw();
        } else if changed {
            self.queue_draw();
        }
    }
}

// ------------------------------------------------------------------------------------------- DeviceArt

mod dev_imp {
    use super::*;

    #[derive(Default)]
    pub struct DeviceArt {
        pub small: Cell<bool>,
        pub color: RefCell<Option<gdk::RGBA>>,
        pub texture: RefCell<Option<gdk::Texture>>,
        pub loaded: RefCell<String>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for DeviceArt {
        const NAME: &'static str = "StSettingsDeviceArt";
        type Type = super::DeviceArt;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for DeviceArt {}

    impl WidgetImpl for DeviceArt {
        fn snapshot(&self, sn: &gtk::Snapshot) {
            let small = self.small.get();
            let w = self.obj().width() as f32;
            let bezel_h = w / 1.6;
            let pad = if small { 4.0 } else { 5.0 };
            let r = if small { 6.0 } else { 7.0 };
            let neck_h = if small { 10.0 } else { 14.0 };
            // filter: drop-shadow(0 6px 10px rgba(0, 0, 0, 0.18))
            sn.append_outset_shadow(&rounded(0.0, 0.0, w, bezel_h, r), &gdk::RGBA::new(0.0, 0.0, 0.0, 0.18), 0.0, 6.0, 0.0, 10.0);
            sn.push_rounded_clip(&rounded(0.0, 0.0, w, bezel_h, r));
            sn.append_linear_gradient(
                &rect(0.0, 0.0, w, bezel_h),
                &graphene::Point::new(0.0, 0.0),
                &graphene::Point::new(0.0, bezel_h),
                &[
                    gsk::ColorStop::new(0.0, gdk::RGBA::new(0.227, 0.227, 0.227, 1.0)),
                    gsk::ColorStop::new(1.0, gdk::RGBA::new(0.086, 0.086, 0.086, 1.0)),
                ],
            );
            sn.append_inset_shadow(&rounded(0.0, 0.0, w, bezel_h, r), &gdk::RGBA::new(1.0, 1.0, 1.0, 0.08), 0.0, 0.0, 1.0, 0.0);
            sn.pop();
            let (sx, sy, sw, sh) = (pad, pad, w - 2.0 * pad, bezel_h - 2.0 * pad);
            sn.push_rounded_clip(&rounded(sx, sy, sw, sh, 3.0));
            let color = self.color.borrow().unwrap_or(gdk::RGBA::new(0.043, 0.165, 0.29, 1.0));
            sn.append_color(&color, &rect(sx, sy, sw, sh));
            if let Some(t) = self.texture.borrow().as_ref() {
                draw_wallpaper(sn, t, "fill", sx, sy, sw, sh);
            }
            sn.pop();
            // neck: linear-gradient(90deg, #8e8e8e, #c9c9c9 45%, #8e8e8e)
            let nw = w * 0.16;
            let nx = (w - nw) / 2.0;
            sn.append_linear_gradient(
                &rect(nx, bezel_h, nw, neck_h),
                &graphene::Point::new(nx, 0.0),
                &graphene::Point::new(nx + nw, 0.0),
                &[
                    gsk::ColorStop::new(0.0, gdk::RGBA::new(0.557, 0.557, 0.557, 1.0)),
                    gsk::ColorStop::new(0.45, gdk::RGBA::new(0.788, 0.788, 0.788, 1.0)),
                    gsk::ColorStop::new(1.0, gdk::RGBA::new(0.557, 0.557, 0.557, 1.0)),
                ],
            );
            // foot: 42 %, 5 px, rounded 3 3 2 2, linear-gradient(180deg, #d0d0d0, #8d8d8d)
            let fw = w * 0.42;
            let fx = (w - fw) / 2.0;
            let fy = bezel_h + neck_h;
            sn.push_rounded_clip(&rounded(fx, fy, fw, 5.0, 2.5));
            sn.append_linear_gradient(
                &rect(fx, fy, fw, 5.0),
                &graphene::Point::new(0.0, fy),
                &graphene::Point::new(0.0, fy + 5.0),
                &[
                    gsk::ColorStop::new(0.0, gdk::RGBA::new(0.816, 0.816, 0.816, 1.0)),
                    gsk::ColorStop::new(1.0, gdk::RGBA::new(0.553, 0.553, 0.553, 1.0)),
                ],
            );
            sn.pop();
        }
    }
}

glib::wrapper! {
    /// A monitor showing the wallpaper (common.js DeviceArt, `.st-device`): 196 px wide, 132 when small.
    pub struct DeviceArt(ObjectSubclass<dev_imp::DeviceArt>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl DeviceArt {
    pub fn new(small: bool) -> DeviceArt {
        let d: DeviceArt = glib::Object::new();
        d.add_css_class("st-device");
        d.imp().small.set(small);
        let w = if small { 132 } else { 196 };
        let h = (w as f64 / 1.6).round() as i32 + if small { 10 } else { 14 } + 5;
        d.set_size_request(w, h);
        d.set_halign(gtk::Align::Start);
        d.set_valign(gtk::Align::Center);
        d.update(&kit::ctx().settings());
        d
    }

    /// Show the wallpaper of these settings (DeviceArt update(s)).
    pub fn update(&self, s: &Value) {
        let path = common::resolve_wallpaper_path(s, kit::theme::dark());
        let color = s["wallpaper"]["color"].as_str().and_then(rgba);
        *self.imp().color.borrow_mut() = color;
        if *self.imp().loaded.borrow() != path {
            *self.imp().loaded.borrow_mut() = path.clone();
            *self.imp().texture.borrow_mut() = None;
            if !path.is_empty() {
                let weak = self.downgrade();
                let key = path.clone();
                common::wallpaper_texture(&key, 512, move |t| {
                    if let Some(me) = weak.upgrade()
                        && *me.imp().loaded.borrow() == path
                    {
                        *me.imp().texture.borrow_mut() = t;
                        me.queue_draw();
                    }
                });
            }
        }
        self.queue_draw();
    }
}
