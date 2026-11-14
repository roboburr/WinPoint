//! Instellingen › Toegankelijkheid (last-webkit:vendor/ui/js/apps/settings/pages/accessibility.js) with its sub-pages
//! Tekstgrootte (a sample, the slider and Toepassen), Visuele effecten and Contrastthema’s.
use super::{Builder, Page, PageFuture};
use crate::common;
use kit::controls::{self as c, SettingsCard, SettingsGroup, Slider, Variant};
use kit::prelude::*;
use kit::t;
use std::cell::Cell;
use std::rc::Rc;

pub const PAGES: &[(&str, Builder)] = &[("accessibility", accessibility), ("textsize", textsize), ("visualeffects", visualeffects), ("contrast", contrast)];
pub const CSS: &str = r#"
.st-ts-sample {
  min-height: 92px;
  padding: 20px 24px;
  border: 1px solid var(--stroke-card);
  border-radius: var(--radius-card);
  background-color: var(--bg-card);
}
.st-ts-sample-label { color: var(--text-2); font-size: 12px; }
.st-ts-sample-text { color: var(--text-1); }
"#;

fn animations_card(p: &Page) -> SettingsCard {
    let (card, sw) = common::toggle_card(
        p,
        "sparkle",
        t("Animatie-effecten", "Animation effects"),
        t(
            "Vensters, menu’s en schakelaars bewegen vloeiend. Zet dit uit voor een rustiger beeld.",
            "Windows, menus and switches move smoothly. Turn this off for a calmer screen.",
        ),
        "accessibility.reduceMotion",
        true,
    );
    sw.switch().set_widget_name("a11y-animations");
    card
}

fn accessibility(p: Page) -> PageFuture {
    Box::pin(async move {
        let vision = SettingsGroup::new(t("Zicht", "Vision"), "");
        for id in ["textsize", "visualeffects", "contrast"] {
            vision.add(common::link_card(&p, id).widget());
        }
        let quick = SettingsGroup::new(t("Snel aanpassen", "Quick adjustments"), "");
        quick.add(animations_card(&p).widget());
        vec![vision.widget().clone().upcast(), quick.widget().clone().upcast()]
    })
}

fn cur_scale() -> i64 {
    // JS Number(getSetting(..)) || 100: a number or numeric text, 0 and junk count as 100
    let raw = kit::ctx().setting("accessibility.textScale");
    let v = raw.as_f64().or_else(|| raw.as_str().and_then(|s| s.trim().parse::<f64>().ok())).filter(|v| v.is_finite() && *v != 0.0).unwrap_or(100.0);
    v.round() as i64
}

/// The sample's size: 14 px (line 20 px) at 100 %, absolute, whatever scale the page itself has.
fn sample_size(label: &gtk::Label, pct: i64) {
    let attrs = gtk::pango::AttrList::new();
    let px = 14.0 * pct as f64 / 100.0;
    attrs.insert(gtk::pango::AttrSize::new_size_absolute((px * gtk::pango::SCALE as f64).round() as i32));
    attrs.insert(gtk::pango::AttrFloat::new_line_height(20.0 / 14.0 / 1.2));
    label.set_attributes(Some(&attrs));
}

fn textsize(p: Page) -> PageFuture {
    Box::pin(async move {
        let pending = Rc::new(Cell::new(cur_scale()));
        let sample = c::label(
            t(
                "Sleep de schuifregelaar tot je deze tekst goed kunt lezen. Klik daarna op Toepassen.",
                "Drag the slider until you can read this text comfortably. Then select Apply.",
            ),
            "st-ts-sample-text",
            true,
        );
        sample.set_max_width_chars(90);
        let value_l = common::value_label("");
        value_l.add_css_class("w11-tabular");
        value_l.set_size_request(44, -1);
        value_l.set_xalign(1.0);
        let apply = c::button(t("Toepassen", "Apply"), Variant::Accent);
        apply.set_widget_name("textsize-apply");
        let slider = Slider::new(100.0, 225.0, 5.0, pending.get() as f64);
        slider.set_value_visible(false);
        slider.set_format(|v| format!("{}%", v.round() as i64));
        slider.scale().set_widget_name("textsize-slider");
        slider.widget().set_size_request(260, -1);
        let paint: Rc<dyn Fn()> = {
            let (pending, sample, value_l, apply) = (pending.clone(), sample.clone(), value_l.clone(), apply.clone());
            Rc::new(move || {
                let v = pending.get();
                sample_size(&sample, v);
                value_l.set_label(&format!("{v}%"));
                apply.set_sensitive(v != cur_scale());
            })
        };
        paint();
        {
            let (pending, paint) = (pending.clone(), paint.clone());
            slider.connect_changed(move |v| {
                pending.set(v.round() as i64);
                paint();
            });
        }
        {
            let (pending, paint) = (pending.clone(), paint.clone());
            slider.connect_committed(move |v| {
                pending.set(v.round() as i64);
                paint();
            });
        }
        {
            let (pp, pending, paint) = (p.clone(), pending.clone(), paint.clone());
            apply.connect_clicked(move |_| {
                let v = pending.get();
                let (p2, paint) = (pp.clone(), paint.clone());
                common::set_setting(&pp, "accessibility.textScale", json!(v), move |ok| {
                    if ok {
                        common::toast_success(&p2.anchor(), &common::fill(t("Tekstgrootte ingesteld op {v}%", "Text size set to {v}%"), &[("v", &v.to_string())]), 2200);
                    }
                    paint();
                });
            });
        }
        {
            let (pending, paint, slider) = (pending.clone(), paint.clone(), slider.clone());
            p.on_settings(move |_, _| {
                // a change from elsewhere, unless the slider is being used
                if pending.get() == cur_scale() || !slider.scale().has_focus() {
                    pending.set(cur_scale());
                    slider.set_value(pending.get() as f64);
                    paint();
                }
            });
        }

        let sample_box = gtk::Box::new(gtk::Orientation::Vertical, 8);
        sample_box.add_css_class("st-ts-sample");
        sample_box.append(&c::label(t("Voorbeeldtekst", "Sample text"), "st-ts-sample-label", false));
        sample_box.append(&sample);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        row.append(slider.widget());
        row.append(&value_l);
        row.append(&apply);
        let card = SettingsCard::new("zoom-in", t("Tekstgrootte", "Text size"), t("Hoe groot tekst is in Win11 Shell", "How big text is in Win11 Shell"));
        card.set_control(&row);
        let group = SettingsGroup::new("", "");
        group.add(card.widget());
        vec![sample_box.upcast(), group.widget().clone().upcast()]
    })
}

fn visualeffects(p: Page) -> PageFuture {
    Box::pin(async move {
        let group = SettingsGroup::new("", "");
        let (trans, sw) = common::toggle_card(
            &p,
            "color",
            t("Transparantie-effecten", "Transparency effects"),
            t("Vensters en oppervlakken zijn een beetje doorschijnend", "Windows and surfaces appear slightly translucent"),
            "theme.transparency",
            false,
        );
        sw.switch().set_widget_name("a11y-transparency");
        group.add(trans.widget());
        group.add(animations_card(&p).widget());
        vec![group.widget().clone().upcast()]
    })
}

fn contrast(p: Page) -> PageFuture {
    Box::pin(async move {
        let info = common::notice(t(
            "Speciale contrastthema’s zijn nog niet beschikbaar in Win11 Shell.",
            "Special contrast themes aren’t available in Win11 Shell yet.",
        ));
        let tips = SettingsGroup::new(t("Tips voor beter leesbare tekst", "Tips for easier reading"), "");
        let mode = common::setting_dropdown(
            &p,
            "theme.mode",
            vec![(json!("light"), t("Licht", "Light").into()), (json!("dark"), t("Donker", "Dark").into())],
            None,
        );
        mode.widget().set_widget_name("contrast-mode");
        let mode_card = SettingsCard::new("theme", t("Licht of donker", "Light or dark"), t("Kies de modus die voor jou het prettigst leest", "Choose the mode that reads best for you"));
        mode_card.set_control(&common::sized(mode.widget(), 160));
        tips.add(mode_card.widget());
        let (trans, sw) = common::toggle_card(
            &p,
            "color",
            t("Transparantie uitzetten", "Turn off transparency"),
            t("Effen achtergronden geven meer contrast", "Solid backgrounds give more contrast"),
            "theme.transparency",
            true,
        );
        sw.switch().set_widget_name("contrast-transparency");
        tips.add(trans.widget());
        tips.add(common::link_card(&p, "textsize").widget());
        vec![info.widget().clone().upcast(), tips.widget().clone().upcast()]
    })
}
