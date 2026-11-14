//! Instellingen › Persoonlijke instellingen › Start (last-webkit:vendor/ui/js/apps/settings/pages/start.js): the layout
//! (start.layout: morePins, default, moreRecommendations), recently added apps, recommended files, and the
//! folders next to the power button (start.folders).
use super::{Builder, Page, PageFuture};
use crate::common;
use crate::widgets::Wrap;
use kit::controls::{self as c, SettingsCard, SettingsExpander, SettingsGroup, ToggleSwitch};
use kit::prelude::*;
use kit::t;
use std::rc::Rc;

pub const PAGES: &[(&str, Builder)] = &[("start", page)];
pub const CSS: &str = r#"
.st-layout-card {
  border: 1px solid var(--stroke-card);
  border-radius: var(--radius-card);
  background-color: var(--bg-card);
}
.st-layout-card > .w11-settings-card { border-width: 0; background-color: transparent; }
.st-layout-row { padding: 0 16px 16px 58px; }
.st-narrow .st-layout-row { padding-left: 16px; }
button.st-layout {
  min-height: 0; min-width: 0; margin: 0; padding: 12px 10px 10px 10px;
  border: 1px solid transparent; border-radius: var(--radius-card);
  background-color: transparent; background-image: none; box-shadow: none;
  text-shadow: none; -gtk-icon-shadow: none; outline: 0 solid transparent;
  color: var(--text-1); font-weight: 400;
  transition: background-color 83ms linear;
}
button.st-layout:hover { background-color: var(--fill-subtle-2); }
button.st-layout.w11-selected { background-color: var(--fill-subtle-2); border-color: var(--stroke-card); }
button.st-layout:focus-visible { outline: 2px solid var(--focus-outer); outline-offset: -2px; }
.st-start-art {
  border-radius: 6px;
  background-color: var(--bg-solid-4);
  box-shadow: 0 0 0 1px var(--stroke-card), 0 2px 6px rgba(0, 0, 0, 0.12);
}
.st-radio-dot {
  min-width: 18px; min-height: 18px; border-radius: 999px;
  border: 1px solid var(--stroke-control-strong);
  background-color: var(--fill-control-alt-2);
}
button.st-layout.w11-selected .st-radio-dot { border-width: 0; min-width: 18px; min-height: 18px; background-color: var(--accent); }
.st-radio-inner { min-width: 7px; min-height: 7px; border-radius: 999px; background-color: transparent; }
button.st-layout.w11-selected .st-radio-inner { background-color: var(--accent-text); }
"#;

/// A start.layout value with how many rows of pins and of recommendations its picture shows.
struct Layout {
    value: &'static str,
    label: (&'static str, &'static str),
    pins: usize,
    recs: usize,
}

const LAYOUTS: &[Layout] = &[
    Layout { value: "morePins", label: ("Meer vastgemaakte items", "More pins"), pins: 3, recs: 1 },
    Layout { value: "default", label: ("Standaard", "Default"), pins: 2, recs: 2 },
    Layout { value: "moreRecommendations", label: ("Meer aanbevelingen", "More recommendations"), pins: 1, recs: 3 },
];

/// (id, glyph, nl, en) of the folders next to the power button.
const FOLDERS: &[(&str, &str, &str, &str)] = &[
    ("settings", "settings", "Instellingen", "Settings"),
    ("explorer", "folder", "Verkenner", "File Explorer"),
    ("documents", "document", "Documenten", "Documents"),
    ("downloads", "download", "Downloads", "Downloads"),
    ("music", "music", "Muziek", "Music"),
    ("pictures", "image", "Afbeeldingen", "Pictures"),
    ("videos", "video", "Video’s", "Videos"),
    ("home", "home", "Persoonlijke map", "Personal folder"),
];

fn cur_layout() -> String {
    common::setting_str("start.layout").unwrap_or_else(|| "default".into())
}

fn folders() -> Vec<String> {
    kit::ctx().setting("start.folders").as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect()).unwrap_or_default()
}

fn rounded(cr: &gtk::cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(w / 2.0).min(h / 2.0);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -std::f64::consts::FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, std::f64::consts::FRAC_PI_2);
    cr.arc(x + r, y + h - r, r, std::f64::consts::FRAC_PI_2, std::f64::consts::PI);
    cr.arc(x + r, y + r, r, std::f64::consts::PI, 1.5 * std::f64::consts::PI);
    cr.close_path();
}

fn set_rgba(cr: &gtk::cairo::Context, c: &gdk::RGBA, alpha: f64) {
    cr.set_source_rgba(c.red() as f64, c.green() as f64, c.blue() as f64, c.alpha() as f64 * alpha);
}

/// The little Start menu of a layout (`.st-start-art`, 104 x 116): search box, pins, recommendations, foot.
fn layout_art(l: &'static Layout) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::new();
    area.add_css_class("st-start-art");
    area.set_content_width(104);
    area.set_content_height(116);
    area.set_halign(gtk::Align::Center);
    area.set_draw_func(move |_, cr, w, h| {
        let (w, h) = (w as f64, h as f64);
        let (pad, gap) = (8.0, 5.0);
        let inner = w - 2.0 * pad;
        let mut y = pad;
        // search: 7 px, fill-control-strong at 35 %
        rounded(cr, pad, y, inner, 7.0, 4.0);
        set_rgba(cr, &kit::theme::token("fill-control-strong"), 0.35);
        let _ = cr.fill();
        y += 7.0 + gap;
        // pins: 6 columns of squares, 4 px apart; every third one grey
        let cell = (inner - 4.0 * 5.0) / 6.0;
        let accent = kit::theme::token("accent");
        let text3 = kit::theme::token("text-3");
        for row in 0..l.pins {
            for col in 0..6 {
                let i = row * 6 + col + 1;
                rounded(cr, pad + col as f64 * (cell + 4.0), y, cell, cell, 2.0);
                if i % 3 == 0 {
                    set_rgba(cr, &text3, 0.5);
                } else {
                    set_rgba(cr, &accent, 0.85);
                }
                let _ = cr.fill();
            }
            y += cell + 4.0;
        }
        y += gap - 4.0 + 3.0;
        // recommendations: two columns of 6 px bars, 4 px rows and 6 px columns apart
        let bar_w = (inner - 6.0) / 2.0;
        for _ in 0..l.recs {
            for col in 0..2 {
                rounded(cr, pad + col as f64 * (bar_w + 6.0), y, bar_w, 6.0, 3.0);
                set_rgba(cr, &text3, 0.5);
                let _ = cr.fill();
            }
            y += 6.0 + 4.0;
        }
        // foot: a divider 6 px above the bottom padding
        let fy = h - pad - 6.0;
        set_rgba(cr, &kit::theme::token("stroke-divider"), 1.0);
        cr.rectangle(pad, fy, inner, 1.0);
        let _ = cr.fill();
    });
    kit::theme::track(&area);
    area
}

fn layout_option(p: &Page, l: &'static Layout) -> gtk::Button {
    let col = gtk::Box::new(gtk::Orientation::Vertical, 10);
    col.append(&layout_art(l));
    let label_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    label_row.set_halign(gtk::Align::Center);
    let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    dot.add_css_class("st-radio-dot");
    dot.set_valign(gtk::Align::Center);
    dot.set_hexpand(false);
    let inner = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    inner.add_css_class("st-radio-inner");
    inner.set_halign(gtk::Align::Center);
    inner.set_valign(gtk::Align::Center);
    inner.set_hexpand(true);
    dot.append(&inner);
    label_row.append(&dot);
    label_row.append(&c::label(t(l.label.0, l.label.1), "", true));
    col.append(&label_row);
    let b = gtk::Button::new();
    b.add_css_class("st-layout");
    b.set_size_request(200, -1);
    b.set_child(Some(&col));
    b.set_widget_name(&format!("start-layout-{}", l.value));
    let pp = p.clone();
    b.connect_clicked(move |_| common::set_setting(&pp, "start.layout", json!(l.value), |_| {}));
    b
}

fn page(p: Page) -> PageFuture {
    Box::pin(async move {
        // ---- Indeling: the card with its three pictures (`.st-layout-card`)
        let opts: Vec<(&'static str, gtk::Button)> = LAYOUTS.iter().map(|l| (l.value, layout_option(&p, l))).collect();
        let sync_layout = {
            let opts = opts.clone();
            move || {
                let cur = cur_layout();
                for (v, b) in &opts {
                    if *v == cur {
                        b.add_css_class("w11-selected");
                    } else {
                        b.remove_css_class("w11-selected");
                    }
                }
            }
        };
        sync_layout();
        let head = SettingsCard::new(
            "start-menu",
            t("Indeling", "Layout"),
            t("Hoeveel ruimte Start geeft aan vastgemaakte apps en aan aanbevelingen", "How much room Start gives to pinned apps and to recommendations"),
        );
        let row = Wrap::new(8, 8);
        row.add_css_class("st-layout-row");
        for (_, b) in &opts {
            row.append(b);
        }
        let layout_card = gtk::Box::new(gtk::Orientation::Vertical, 0);
        layout_card.add_css_class("st-layout-card");
        layout_card.append(head.widget());
        layout_card.append(&row);

        let (apps_card, apps_sw) = common::toggle_card(
            &p,
            "apps",
            t("Onlangs toegevoegde apps weergeven", "Show recently added apps"),
            t("Nieuwe apps verschijnen bovenaan Aanbevolen", "New apps appear at the top of Recommended"),
            "start.showRecentApps",
            false,
        );
        apps_sw.switch().set_widget_name("start-recent-apps");
        let (rec_card, rec_sw) = common::toggle_card(
            &p,
            "history",
            t("Aanbevolen bestanden weergeven in Start", "Show recommended files in Start"),
            t("Je recente bestanden in het deel Aanbevolen", "Your recent files in the Recommended section"),
            "start.showRecommended",
            false,
        );
        rec_sw.switch().set_widget_name("start-recommended");
        let recent_note = common::notice(t(
            "Recente bestanden staan uit bij Privacy en beveiliging. Aanbevolen blijft dan leeg.",
            "Recent files are turned off in Privacy & security, so Recommended stays empty.",
        ));
        let sync_note = {
            let n = recent_note.clone();
            move || n.set_visible(kit::ctx().setting("privacy.recentFiles") == false)
        };
        sync_note();

        // ---- Mappen
        let folders_exp = SettingsExpander::new(
            "folder",
            t("Mappen", "Folders"),
            t("Deze mappen staan naast de aan/uit-knop", "These folders appear next to the power button"),
        );
        let mut switches: Vec<(&'static str, ToggleSwitch)> = Vec::new();
        for (id, glyph, nl, en) in FOLDERS {
            let id: &'static str = id;
            let sw = ToggleSwitch::new(folders().iter().any(|f| f == id));
            sw.switch().set_widget_name(&format!("start-folder-{id}"));
            let (pp, sw2) = (p.clone(), sw.clone());
            sw.connect_toggled(move |on| {
                let mut list = folders();
                list.retain(|f| f != id);
                if on {
                    list.push(id.to_owned());
                }
                // the folders' own order, whatever order they were switched on in
                let ordered: Vec<&str> = FOLDERS.iter().map(|f| f.0).filter(|f| list.iter().any(|x| x == f)).collect();
                let sw3 = sw2.clone();
                common::set_setting(&pp, "start.folders", json!(ordered), move |ok| {
                    if !ok {
                        sw3.set_active(!on);
                    }
                });
            });
            let card = SettingsCard::new(glyph, t(nl, en), "");
            card.set_control(sw.widget());
            folders_exp.add(card.widget());
            switches.push((id, sw));
        }
        let switches = Rc::new(switches);

        {
            let sw = switches.clone();
            p.on_settings(move |_, _| {
                sync_layout();
                sync_note();
                let list = folders();
                for (id, s) in sw.iter() {
                    s.set_active(list.iter().any(|f| f == id));
                }
            });
        }

        let main = SettingsGroup::new("", "");
        main.add(&layout_card);
        main.add(apps_card.widget());
        main.add(rec_card.widget());
        main.add(recent_note.widget());
        main.add(folders_exp.widget());
        let links = SettingsGroup::new("", "");
        links.add(common::link_card(&p, "privacy").widget());
        vec![main.widget().clone().upcast(), links.widget().clone().upcast()]
    })
}
