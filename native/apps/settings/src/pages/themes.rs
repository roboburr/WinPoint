//! Instellingen › Persoonlijke instellingen › Thema’s (6ebf169f:vendor/ui/js/apps/settings/pages/themes.js): the
//! current theme with its background, colour and mode, and the six presets (wallpaper, mode and accent in one go).
//! Shared with Startpagina and Persoonlijke instellingen: THEMES, `current_theme`, `apply_theme`, `theme_card`,
//! `theme_grid`; with Achtergrond: WALLPAPERS and `wallpaper_name`.
use super::background::FillGrid;
use super::{Builder, Page, PageFuture};
use crate::common;
use crate::preview::{DesktopPreview, PreviewOpts};
use crate::widgets::{Clamp, Wrap};
use kit::controls::{self as c, SettingsGroup};
use kit::prelude::*;
use kit::t;
use std::cell::RefCell;
use std::rc::Rc;

pub const PAGES: &[(&str, Builder)] = &[("themes", page)];
pub const CSS: &str = r#"
.st-theme-facts { margin: 10px 0 0 -10px; }
button.st-fact {
  min-height: 36px; min-width: 0; margin: 0; padding: 0 10px;
  border: 0 solid transparent; border-radius: var(--radius-control);
  background-color: transparent; background-image: none; box-shadow: none;
  text-shadow: none; -gtk-icon-shadow: none; outline: 0 solid transparent;
  color: var(--text-1); font-weight: 400;
  transition: background-color 83ms linear;
}
button.st-fact:hover { background-color: var(--fill-subtle-2); }
button.st-fact:active { background-color: var(--fill-subtle-3); }
button.st-fact:focus-visible { outline: 2px solid var(--focus-outer); outline-offset: -2px; }
.st-fact-icon { color: var(--text-2); }
.st-fact-k { color: var(--text-2); }
.st-card-pad {
  padding: 16px;
  border: 1px solid var(--stroke-card);
  border-radius: var(--radius-card);
  background-color: var(--bg-card);
}
"#;

/// A theme preset: wallpaper, mode and accent in one go.
pub struct Theme {
    pub id: &'static str,
    pub name: (&'static str, &'static str),
    pub wallpaper: &'static str,
    pub mode: &'static str,
    pub accent: &'static str,
}

impl Theme {
    pub fn name(&self) -> &'static str {
        t(self.name.0, self.name.1)
    }
}

pub static THEMES: &[Theme] = &[
    Theme { id: "lotus-light", name: ("Lotus (licht)", "Lotus (light)"), wallpaper: "builtin:lotus-light", mode: "light", accent: "#0078D4" },
    Theme { id: "lotus-dark", name: ("Lotus (donker)", "Lotus (dark)"), wallpaper: "builtin:lotus-dark", mode: "dark", accent: "#0078D4" },
    Theme { id: "flow", name: ("Stroming", "Flow"), wallpaper: "builtin:flow-dark", mode: "dark", accent: "#8764B8" },
    Theme { id: "glow", name: ("Gloed", "Glow"), wallpaper: "builtin:glow", mode: "dark", accent: "#E3008C" },
    Theme { id: "sunrise", name: ("Zonsopgang", "Sunrise"), wallpaper: "builtin:sunrise", mode: "light", accent: "#CA5010" },
    Theme { id: "aurora", name: ("Noorderlicht", "Aurora"), wallpaper: "builtin:aurora", mode: "dark", accent: "#00B294" },
];

/// A built-in wallpaper and its friendly name.
pub struct Wallpaper {
    pub id: &'static str,
    pub name: (&'static str, &'static str),
}

impl Wallpaper {
    pub fn name(&self) -> &'static str {
        t(self.name.0, self.name.1)
    }
}

/// The friendly names of the built-in wallpapers (themes.js WALLPAPERS).
pub static WALLPAPERS: &[Wallpaper] = &[
    Wallpaper { id: "lotus-light", name: ("Lotus licht", "Lotus light") },
    Wallpaper { id: "lotus-dark", name: ("Lotus donker", "Lotus dark") },
    Wallpaper { id: "flow-light", name: ("Stroming licht", "Flow light") },
    Wallpaper { id: "flow-dark", name: ("Stroming donker", "Flow dark") },
    Wallpaper { id: "glow", name: ("Gloed", "Glow") },
    Wallpaper { id: "sunrise", name: ("Zonsopgang", "Sunrise") },
    Wallpaper { id: "aurora", name: ("Noorderlicht", "Aurora") },
    Wallpaper { id: "dunes", name: ("Duinen", "Dunes") },
];

fn same_accent(a: &str, b: &str) -> bool {
    let n = |x: &str| x.to_lowercase().replace("#0067c0", "#0078d4");
    n(a) == n(b)
}

/// The preset the settings match, if any (themes.js currentTheme).
pub fn current_theme(s: &Value) -> Option<&'static Theme> {
    if s["wallpaper"]["type"] == "color" {
        return None;
    }
    let mode = s["theme"]["mode"].as_str().unwrap_or("");
    let accent = s["theme"]["accent"].as_str().unwrap_or("");
    let follow = s["wallpaper"]["followTheme"] != false;
    let path = s["wallpaper"]["path"].as_str().unwrap_or("");
    THEMES.iter().find(|th| {
        mode == th.mode
            && same_accent(accent, th.accent)
            && s["theme"]["accentFromWallpaper"] != true
            && common::twin_for(path, th.mode == "dark", follow) == common::twin_for(th.wallpaper, th.mode == "dark", true)
    })
}

/// The wallpaper's friendly name: "Effen kleur", a built-in's name, or the photo's file name (themes.js
/// wallpaperName).
pub fn wallpaper_name(s: &Value) -> String {
    let w = &s["wallpaper"];
    if w["type"] == "color" {
        return t("Effen kleur", "Solid colour").into();
    }
    let p = w["path"].as_str().unwrap_or("");
    if p.starts_with("builtin:") {
        let twin = common::twin_for(p, kit::theme::dark(), w["followTheme"] != false);
        let id = twin.strip_prefix("builtin:").unwrap_or(&twin);
        return WALLPAPERS.iter().find(|x| x.id == id).map(|x| x.name().to_owned()).unwrap_or_else(|| id.to_owned());
    }
    p.rsplit('/').next().filter(|n| !n.is_empty()).unwrap_or(t("Eigen foto", "Own photo")).to_owned()
}

/// Apply a preset (themes.js applyTheme): one settings.patch, then "Thema “..” toegepast".
pub fn apply_theme(p: &Page, th: &'static Theme) {
    let pp = p.clone();
    let args = json!({ "patch": {
        "theme": { "mode": th.mode, "accent": th.accent, "accentFromWallpaper": false },
        "wallpaper": { "type": "image", "path": th.wallpaper, "pathDark": "", "followTheme": true },
    } });
    p.spawn(async move {
        match pp.call("settings.patch", args).await {
            Ok(_) => common::toast_success(&pp.anchor(), &common::fill(t("Thema “{name}” toegepast", "Theme “{name}” applied"), &[("name", th.name())]), 2200),
            Err(e) => pp.error(&e, None),
        }
    });
}

/// The thumbnail card of one preset (themes.js ThemeCard).
pub fn theme_card(p: &Page, th: &'static Theme, compact: bool) -> gtk::Button {
    let dark = th.mode == "dark";
    let prev = DesktopPreview::new(
        &PreviewOpts {
            dark: Some(dark),
            accent: Some(th.accent.into()),
            path: Some(common::twin_for(th.wallpaper, dark, true)),
            fit: Some("fill".into()),
            no_window: compact,
            ..Default::default()
        },
        false,
    );
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&prev));
    let check = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    check.add_css_class("st-theme-check");
    let size = if compact { 16 } else { 20 };
    check.set_size_request(size, size);
    check.set_halign(gtk::Align::End);
    check.set_valign(gtk::Align::Start);
    let g = kit::icons::glyph_stroke("check", 12, 2.2);
    g.set_hexpand(true);
    g.set_halign(gtk::Align::Center);
    check.append(&g);
    overlay.add_overlay(&check);
    let col = gtk::Box::new(gtk::Orientation::Vertical, 8);
    col.append(&overlay);
    if !compact {
        col.append(&c::label(th.name(), "st-theme-name", false));
    }
    let b = gtk::Button::new();
    b.add_css_class("st-theme");
    if compact {
        b.add_css_class("st-compact");
        b.set_tooltip_text(Some(th.name()));
    }
    b.set_widget_name(&format!("theme-{}", th.id));
    b.set_child(Some(&col));
    let pp = p.clone();
    b.connect_clicked(move |_| apply_theme(&pp, th));
    b
}

/// Mark the card of the current preset, now and after every settings change (themes.js ThemeGrid sync).
fn track_selection(p: &Page, cards: Vec<gtk::Button>) {
    let sync = move |s: &Value| {
        let cur = current_theme(s);
        for (i, c) in cards.iter().enumerate() {
            if cur.is_some_and(|th| std::ptr::eq(th, &THEMES[i])) {
                c.add_css_class("w11-selected");
            } else {
                c.remove_css_class("w11-selected");
            }
        }
    };
    sync(&kit::ctx().settings());
    p.on_settings(move |s, _| sync(s));
}

/// The grid of preset cards that follows the current selection (themes.js ThemeGrid); compact: Startpagina.
pub fn theme_grid(p: &Page, compact: bool) -> gtk::Widget {
    let grid = gtk::Grid::new();
    grid.add_css_class("st-theme-grid");
    grid.set_column_homogeneous(true);
    grid.set_column_spacing(if compact { 8 } else { 12 });
    grid.set_row_spacing(if compact { 8 } else { 16 });
    let cards: Vec<gtk::Button> = THEMES.iter().map(|th| theme_card(p, th, compact)).collect();
    for (i, c) in cards.iter().enumerate() {
        grid.attach(c, i as i32 % 3, i as i32 / 3, 1, 1);
    }
    track_selection(p, cards);
    grid.upcast()
}

/// The Thema’s page's grid in its card (`.st-card-pad .st-theme-grid`: auto-fill, at least 140 px).
fn padded_theme_grid(p: &Page) -> gtk::Widget {
    let grid = FillGrid::new(140, 12, 16, 0);
    grid.add_css_class("st-theme-grid");
    let cards: Vec<gtk::Button> = THEMES.iter().map(|th| theme_card(p, th, false)).collect();
    for c in &cards {
        c.set_valign(gtk::Align::Start);
        grid.append(c);
    }
    track_selection(p, cards);
    let pad = gtk::Box::new(gtk::Orientation::Vertical, 0);
    pad.add_css_class("st-card-pad");
    pad.append(&grid);
    pad.upcast()
}

/// The 16 px accent dot (`.st-swatch-dot`).
fn swatch_dot(color: Rc<RefCell<gdk::RGBA>>) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::new();
    area.set_content_width(16);
    area.set_content_height(16);
    area.set_valign(gtk::Align::Center);
    area.add_css_class("st-swatch-dot");
    area.set_draw_func(move |_, cr, w, h| {
        let r = (w.min(h) as f64) / 2.0;
        let c = *color.borrow();
        cr.arc(w as f64 / 2.0, h as f64 / 2.0, r, 0.0, std::f64::consts::TAU);
        cr.set_source_rgba(c.red() as f64, c.green() as f64, c.blue() as f64, c.alpha() as f64);
        let _ = cr.fill();
        // box-shadow: inset 0 0 0 1px rgba(0, 0, 0, 0.15)
        cr.arc(w as f64 / 2.0, h as f64 / 2.0, r - 0.5, 0.0, std::f64::consts::TAU);
        cr.set_source_rgba(0.0, 0.0, 0.0, 0.15);
        cr.set_line_width(1.0);
        let _ = cr.stroke();
    });
    area
}

/// One fact row of the hero (`.st-fact`): icon, key, value; opens a page.
fn fact(p: &Page, icon: &gtk::Widget, key: &str, value: &gtk::Label, target: &'static str, name: &str) -> gtk::Button {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    icon.add_css_class("st-fact-icon");
    icon.set_valign(gtk::Align::Center);
    row.append(icon);
    let k = c::label(key, "st-fact-k", false);
    k.set_size_request(92, -1);
    row.append(&k);
    value.set_hexpand(true);
    row.append(value);
    let b = gtk::Button::new();
    b.add_css_class("st-fact");
    b.set_child(Some(&row));
    b.set_widget_name(name);
    let pp = p.clone();
    b.connect_clicked(move |_| pp.go(target));
    b
}

fn page(p: Page) -> PageFuture {
    Box::pin(async move {
        let prev = DesktopPreview::new(&PreviewOpts::default(), true);
        let name_l = c::label("", "st-hero-title", true);
        let bg_l = c::label("", "", false);
        let color_l = c::label("", "", false);
        let mode_l = c::label("", "", false);
        let dot_color = Rc::new(RefCell::new(gdk::RGBA::new(0.0, 0.47, 0.83, 1.0)));
        let dot = swatch_dot(dot_color.clone());
        let sync = {
            let (prev, name_l, bg_l, color_l, mode_l, dot) = (prev.clone(), name_l.clone(), bg_l.clone(), color_l.clone(), mode_l.clone(), dot.clone());
            move |s: &Value| {
                prev.update(&PreviewOpts::default());
                let cur = current_theme(s);
                name_l.set_label(&common::fill(t("Huidig thema: {name}", "Current theme: {name}"), &[("name", cur.map(|c| c.name()).unwrap_or(t("Aangepast", "Custom")))]));
                bg_l.set_label(&wallpaper_name(s));
                let accent = s["theme"]["accent"].as_str().unwrap_or("#0078d4");
                *dot_color.borrow_mut() = gdk::RGBA::parse(accent).unwrap_or(gdk::RGBA::new(0.0, 0.47, 0.83, 1.0));
                dot.queue_draw();
                color_l.set_label(&common::color_name(accent));
                color_l.set_tooltip_text(Some(&accent.to_uppercase()));
                mode_l.set_label(if kit::theme::dark() { t("Donker", "Dark") } else { t("Licht", "Light") });
            }
        };
        sync(&kit::ctx().settings());
        p.on_settings(move |s, _| sync(s));

        let facts = gtk::Box::new(gtk::Orientation::Vertical, 2);
        facts.add_css_class("st-theme-facts");
        facts.set_halign(gtk::Align::Start);
        facts.append(&fact(&p, kit::icons::glyph("image", 16).upcast_ref(), t("Achtergrond", "Background"), &bg_l, "background", "themes-background"));
        facts.append(&fact(&p, dot.upcast_ref(), t("Kleur", "Colour"), &color_l, "colors", "themes-color"));
        facts.append(&fact(&p, kit::icons::glyph("theme", 16).upcast_ref(), t("Modus", "Mode"), &mode_l, "colors", "themes-mode"));

        // the hero (`.st-hero.st-theme-hero`): the preview 320 px wide, the name and the facts
        let hero = Wrap::new(24, 16);
        hero.add_css_class("st-hero");
        hero.add_css_class("st-theme-hero");
        let art = Clamp::new(&prev, 320);
        art.set_size_request(200, -1);
        art.set_hexpand(false);
        art.set_valign(gtk::Align::Center);
        hero.append(&art);
        let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
        text.add_css_class("st-hero-text");
        text.set_hexpand(true);
        text.set_size_request(200, -1);
        text.set_valign(gtk::Align::Center);
        text.append(&name_l);
        text.append(&facts);
        hero.append(&text);

        let apply = SettingsGroup::new(
            t("Een thema toepassen", "Apply a theme"),
            t(
                "Een thema verandert in één keer de achtergrond, de kleurmodus en de accentkleur.",
                "A theme changes the background, colour mode and accent colour in one go.",
            ),
        );
        apply.add(&padded_theme_grid(&p));
        let related = SettingsGroup::new(common::ct::related(), "");
        related.add(common::link_card(&p, "desktopicons").widget());
        related.add(common::link_card(&p, "contrast").widget());
        vec![hero.upcast(), apply.widget().clone().upcast(), related.widget().clone().upcast()]
    })
}
