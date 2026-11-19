//! Instellingen › Toegankelijkheid (batch A, notes/claude/settings-parity.md §2, plan.json "accessibility", "textsize",
//! "visualeffects", "contrast"): Windows 11's Accessibility page and three of its Vision pages, with the functions of
//! GNOME Settings 50's Accessibility panel they hold.
//!
//! * Toegankelijkheid: Windows' groups Zicht, Gehoor and Interactie (the registry's pages, caps-filtered); Snel
//!   aanpassen: Animatie-effecten (as on Visuele effecten) and, as windows.json's WinPoint addition at the bottom,
//!   Toegankelijkheidsmenu altijd weergeven = `org.gnome.desktop.a11y always-show-universal-access-status` (GNOME's
//!   accessibility menu is the Toegankelijkheid tile of Snelle instellingen in WinPoint, native/shell/src/flyouts/
//!   quick.rs).
//! * Tekstgrootte: Windows' Voorbeeld van tekstgrootte and the Tekstgrootte slider (100-225 %) with Toepassen =
//!   `org.gnome.desktop.interface text-scaling-factor` (percent / 100, GNOME Settings' Text Size dialog with its
//!   Apply). The key is Mirrored (amendment 49: the page writes GNOME's key, the mirror keeps
//!   accessibility.textScale equal); GTK scales GNOME's apps by it and the kit, the shell and the calendar scale
//!   their own text by exactly that factor (native/kit/src/theme.rs `System`), so nothing is scaled twice. A value
//!   GNOME holds outside Windows' range (0.5-3.0 is the schema's) is shown as it is and written only by Toepassen.
//! * Visuele effecten, Windows' order: Schuifbalken altijd weergeven = `overlay-scrolling` (inverted), Transparantie-
//!   effecten = WinPoint's theme.transparency, Animatie-effecten = `org.gnome.desktop.a11y.interface reduced-motion`
//!   (off = 'reduce', GNOME Settings 50's Reduced Motion) together with `org.gnome.desktop.interface
//!   enable-animations` (the mirror keeps both equal to accessibility.reduceMotion as well); WinPoint addition: Vormen
//!   voor aan/uit-status = `show-status-shapes` (GNOME's On/Off Shapes, drawn in the kit's switches). Windows'
//!   "Meldingen sluiten na deze tijd" is left out: GNOME Shell's banner time is fixed and WinPoint has no such setting.
//! * Contrastthema's: Windows' drop-down with Toepassen; GNOME has one high-contrast style, so the choices are Geen
//!   and Hoog contrast = `org.gnome.desktop.a11y.interface high-contrast` (Mirrored with accessibility.highContrast;
//!   amendment 46: while it is on, WinPoint's Fluent block leaves gtk.css). The shortcut row shows GNOME's
//!   toggle-contrast key (Sneltoetsen sets it); the tips stay as WinPoint's related settings.
use super::mouse::bind_state;
use super::{Builder, Content, Page, PageFuture};
use crate::common::{self, ct};
use crate::gs::{self, GKey, Gs};
use kit::controls::{self as c, Dropdown, DropdownOption, SettingsCard, SettingsGroup, Slider, ToggleSwitch, Variant};
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
.st-a11y-intro { margin-bottom: 4px; }
"#;

const IFACE: &str = "org.gnome.desktop.interface";
const A11Y_IFACE: &str = "org.gnome.desktop.a11y.interface";
const MENU: GKey = gs::key("org.gnome.desktop.a11y", "always-show-universal-access-status");
const MOTION: GKey = gs::key(A11Y_IFACE, "reduced-motion");
const ANIMATIONS: GKey = gs::key(IFACE, "enable-animations");
const CONTRAST: GKey = gs::key(A11Y_IFACE, "high-contrast");
const SHAPES: GKey = gs::key(A11Y_IFACE, "show-status-shapes");
const SCALE: GKey = gs::key(IFACE, "text-scaling-factor");
const OVERLAY: GKey = gs::key(IFACE, "overlay-scrolling");
const CONTRAST_SHORTCUT: GKey = gs::key("org.gnome.settings-daemon.plugins.media-keys", "toggle-contrast");

/// Windows' text size range (percent).
const TEXT_MIN: f64 = 100.0;
const TEXT_MAX: f64 = 225.0;

// ------------------------------------------------------------------------------------------ value mappings

/// Animatie-effecten is on unless GNOME's reduced-motion is 'reduce'.
fn animations_on(v: &Value) -> bool {
    v.as_str() != Some("reduce")
}

/// GNOME Settings' write for Animatie-effecten (its Reduced Motion, inverted).
fn motion_value(on: bool) -> Value {
    json!(if on { "no-preference" } else { "reduce" })
}

/// text-scaling-factor as Windows' percent (GNOME's value as it is, to a whole percent).
fn percent_of(v: &Value) -> i64 {
    (v.as_f64().filter(|f| f.is_finite() && *f > 0.0).unwrap_or(1.0) * 100.0).round() as i64
}

/// The text-scaling-factor Toepassen writes for a percent.
fn factor_of(percent: i64) -> Value {
    json!(percent as f64 / 100.0)
}

/// Contrastthema's choices: GNOME's one high-contrast style.
fn contrast_options() -> Vec<(&'static str, &'static str)> {
    vec![("none", t("Geen", "None")), ("high", t("Hoog contrast", "High contrast"))]
}

// ------------------------------------------------------------------------------------------ shared rows

/// Animatie-effecten: reduced-motion (GNOME Settings 50's key) and enable-animations (GNOME Shell's and GTK's),
/// written together; optimistic, reverted with the error toast, following both.
fn animations_card(gs: &Gs, keys: &[GKey]) -> Option<SettingsCard> {
    if !gs.available(&MOTION) {
        return None;
    }
    let sw = ToggleSwitch::new(animations_on(&gs.value(&MOTION)));
    bind_state(gs, keys, sw.switch(), &MOTION);
    if !gs.writable(&MOTION) {
        sw.set_sensitive(false);
    }
    let (g, sw2) = (gs.clone(), sw.clone());
    sw.connect_toggled(move |on| {
        let (sw3, g3) = (sw2.clone(), g.clone());
        g.set(&MOTION, motion_value(on), move || sw3.set_active(animations_on(&g3.value(&MOTION))));
        if g.writable(&ANIMATIONS) && g.value(&ANIMATIONS) != json!(on) {
            g.set(&ANIMATIONS, json!(on), || {});
        }
    });
    let sw2 = sw.clone();
    gs.follow(&MOTION, move |v| sw2.set_active(animations_on(v)));
    let card = SettingsCard::new(
        "sparkle",
        t("Animatie-effecten", "Animation effects"),
        t(
            "Vensters, menu’s en schakelaars bewegen vloeiend. Zet dit uit voor een rustiger beeld.",
            "Windows, menus and switches move smoothly. Turn this off for a calmer screen.",
        ),
    );
    card.set_control(sw.widget());
    Some(card)
}

/// A group of the cards that are there (a key that is not installed gives no card); None when none is.
fn group_of(cards: Vec<Option<SettingsCard>>, title: &str) -> Option<gtk::Widget> {
    let group = SettingsGroup::new(title, "");
    let mut any = false;
    for card in cards.into_iter().flatten() {
        group.add(card.widget());
        any = true;
    }
    any.then(|| group.widget().clone().upcast())
}

// ------------------------------------------------------------------------------------------ the pages

fn accessibility(p: Page) -> PageFuture {
    Box::pin(async move {
        let keys = [MOTION, ANIMATIONS, MENU];
        let gs = Gs::load(&p, &keys).await;
        if !p.alive() {
            return Content::new();
        }
        // Windows' three groups, from the registry (settings parity, B0; caps-filtered)
        let groups = [
            common::links_group(
                &p,
                t("Zicht", "Vision"),
                &["textsize", "visualeffects", "mousepointer", "textcursor", "magnifier", "contrast", "narrator"],
            ),
            common::links_group(&p, t("Gehoor", "Hearing"), &["a11y-audio"]),
            common::links_group(&p, t("Interactie", "Interaction"), &["a11y-keyboard", "a11y-mouse"]),
        ];
        let mut out: Content = groups.into_iter().flatten().map(|g| g.widget().clone().upcast()).collect();
        let menu = gs.toggle_card(
            &MENU,
            "accessibility",
            t("Toegankelijkheidsmenu altijd weergeven", "Always show accessibility menu"),
            t("De tegel Toegankelijkheid weergeven in Snelle instellingen", "Show the Accessibility tile in Quick settings"),
            false,
        );
        out.extend(group_of(vec![animations_card(&gs, &keys), menu], t("Snel aanpassen", "Quick adjustments")));
        out
    })
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
        let keys = [SCALE];
        let gs = Gs::load(&p, &keys).await;
        if !p.alive() {
            return Content::new();
        }
        if !gs.available(&SCALE) {
            let g = SettingsGroup::new("", "");
            g.add(common::unavailable_card("zoom-in", t("Tekstgrootte", "Text size"), None).widget());
            return vec![g.widget().clone().upcast()];
        }
        let current = Rc::new(Cell::new(percent_of(&gs.value(&SCALE))));
        let pending = Rc::new(Cell::new(current.get()));
        let saving = Rc::new(Cell::new(false));
        let sample = c::label(
            t(
                "Sleep de schuifregelaar tot je deze tekst goed kunt lezen. Klik daarna op Toepassen.",
                "Drag the slider until you can read this text comfortably. Then select Apply.",
            ),
            "st-ts-sample-text",
            true,
        );
        sample.set_max_width_chars(90);
        sample.set_widget_name("st-textsize-sample");
        let value_l = common::value_label("");
        value_l.add_css_class("w11-tabular");
        // a width in characters: it grows with the text scale, never with the number, so the slider stays put
        value_l.set_width_chars(4);
        value_l.set_xalign(1.0);
        value_l.set_widget_name("st-textsize-value");
        let apply = c::button(t("Toepassen", "Apply"), Variant::Accent);
        apply.set_widget_name("st-textsize-apply");
        let slider = Slider::new(TEXT_MIN, TEXT_MAX, 1.0, (current.get() as f64).clamp(TEXT_MIN, TEXT_MAX));
        slider.set_value_visible(false);
        slider.set_format(|v| format!("{}%", v.round() as i64));
        bind_state(&gs, &keys, slider.scale(), &SCALE);
        let writable = gs.writable(&SCALE);
        if !writable {
            slider.set_sensitive(false);
        }
        slider.widget().set_size_request(260, -1);
        let paint: Rc<dyn Fn()> = {
            let (pending, current, saving, sample, value_l, apply) = (pending.clone(), current.clone(), saving.clone(), sample.clone(), value_l.clone(), apply.clone());
            Rc::new(move || {
                let v = pending.get();
                sample_size(&sample, v);
                value_l.set_label(&format!("{v}%"));
                apply.set_sensitive(writable && !saving.get() && v != current.get());
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
            let (pp, pending, current, saving, paint) = (p.clone(), pending.clone(), current.clone(), saving.clone(), paint.clone());
            apply.connect_clicked(move |_| {
                if saving.replace(true) { return; }
                let v = pending.get();
                paint();
                let wait = pp.run("gsettings.set", json!({ "schema": SCALE.schema, "values": { SCALE.key: factor_of(v) } }));
                let (pp2, current, saving, paint) = (pp.clone(), current.clone(), saving.clone(), paint.clone());
                pp.spawn(async move {
                    if let Some(answer) = wait.await {
                        if answer["dryRun"] != true {
                            current.set(v);
                            if pp2.alive() {
                                common::toast_success(&pp2.anchor(), &common::fill(t("Tekstgrootte ingesteld op {v}%", "Text size set to {v}%"), &[("v", &v.to_string())]), 2200);
                            }
                        }
                    }
                    saving.set(false);
                    paint();
                });
            });
        }
        {
            let (pending, current, paint, slider) = (pending.clone(), current.clone(), paint.clone(), slider.clone());
            gs.follow(&SCALE, move |v| {
                // a change from elsewhere (GNOME Settings, the mirror), unless the slider is being used
                let now = percent_of(v);
                let untouched = pending.get() == current.get() || !slider.scale().has_focus();
                current.set(now);
                if untouched {
                    pending.set(now);
                    slider.set_value((now as f64).clamp(TEXT_MIN, TEXT_MAX));
                }
                paint();
            });
        }

        let sample_box = gtk::Box::new(gtk::Orientation::Vertical, 8);
        sample_box.add_css_class("st-ts-sample");
        sample_box.append(&c::label(t("Voorbeeld van tekstgrootte", "Text size preview"), "st-ts-sample-label", false));
        sample_box.append(&sample);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        row.append(slider.widget());
        row.append(&value_l);
        row.append(&apply);
        let card = SettingsCard::new("zoom-in", t("Tekstgrootte", "Text size"), t("Tekst in WinPoint en in apps groter maken", "Make text bigger in WinPoint and in apps"));
        card.set_control(&row);
        let group = SettingsGroup::new("", "");
        group.add(card.widget());
        vec![sample_box.upcast(), group.widget().clone().upcast()]
    })
}

fn visualeffects(p: Page) -> PageFuture {
    Box::pin(async move {
        let keys = [OVERLAY, MOTION, ANIMATIONS, SHAPES];
        let gs = Gs::load(&p, &keys).await;
        if !p.alive() {
            return Content::new();
        }
        let scrollbars = gs.toggle_card(&OVERLAY, "view", t("Schuifbalken altijd weergeven", "Always show scrollbars"), "", true);
        let (trans, sw) = common::toggle_card(
            &p,
            "color",
            t("Transparantie-effecten", "Transparency effects"),
            t("Vensters en oppervlakken zijn een beetje doorschijnend", "Windows and surfaces appear slightly translucent"),
            "theme.transparency",
            false,
        );
        sw.switch().set_widget_name("st-visualeffects-transparency");
        let shapes = gs.toggle_card(
            &SHAPES,
            "check",
            t("Vormen voor aan/uit-status", "On/off shapes"),
            t("Vormen gebruiken om de status aan te geven, naast of in plaats van kleur", "Use shapes to indicate state in addition to or instead of colour"),
            false,
        );
        let mut out: Content = Vec::new();
        out.extend(group_of(vec![scrollbars, Some(trans), animations_card(&gs, &keys), shapes], ""));
        let related = SettingsGroup::new(ct::related(), "");
        related.add(common::link_card(&p, "contrast").widget());
        related.add(common::link_card(&p, "textsize").widget());
        out.push(related.widget().clone().upcast());
        out
    })
}

/// The first accelerator of GNOME's toggle-contrast (None: no shortcut).
fn contrast_accel(v: &Value) -> Option<String> {
    v.as_array().into_iter().flatten().filter_map(|a| a.as_str()).find(|a| !a.is_empty()).map(str::to_owned)
}

/// The shortcut that switches high contrast as key caps, or "Uitgeschakeld".
fn shortcut_control(gs: &Gs) -> gtk::Widget {
    let en = kit::ctx().lang() == "en";
    match contrast_accel(&gs.value(&CONTRAST_SHORTCUT)).and_then(|a| super::shortcuts::accel_keys(&a, en)) {
        Some(keys) => {
            let refs: Vec<&str> = keys.iter().map(String::as_str).collect();
            common::kbd(&refs).upcast()
        }
        None => c::label(t("Uitgeschakeld", "Disabled"), "w11-text-2", false).upcast(),
    }
}

/// Contrastthema's: the drop-down with Toepassen, the shortcut row; None when high-contrast is not installed.
fn contrast_group(p: &Page, gs: &Gs, keys: &[GKey]) -> Option<gtk::Widget> {
    if !gs.available(&CONTRAST) {
        return None;
    }
    let on = gs.value(&CONTRAST) == true;
    let options: Vec<DropdownOption> = contrast_options().into_iter().map(|(v, l)| DropdownOption::new(v, l)).collect();
    let dd = Dropdown::new(options, Some(if on { "high" } else { "none" }));
    bind_state(gs, keys, dd.widget(), &CONTRAST);
    let apply = c::button(t("Toepassen", "Apply"), Variant::Accent);
    apply.set_widget_name("st-contrast-apply");
    let writable = gs.writable(&CONTRAST);
    let chosen = Rc::new(Cell::new(on));
    let sync: Rc<dyn Fn()> = {
        let (g, chosen, apply) = (gs.clone(), chosen.clone(), apply.clone());
        Rc::new(move || apply.set_sensitive(writable && chosen.get() != (g.value(&CONTRAST) == true)))
    };
    sync();
    {
        let (chosen, sync) = (chosen.clone(), sync.clone());
        dd.connect_changed(move |v| {
            chosen.set(v == "high");
            sync();
        });
    }
    {
        let (g, chosen, sync) = (gs.clone(), chosen.clone(), sync.clone());
        apply.connect_clicked(move |_| {
            let sync2 = sync.clone();
            g.set(&CONTRAST, json!(chosen.get()), move || sync2());
            sync();
        });
    }
    {
        let (dd, chosen, sync) = (dd.clone(), chosen.clone(), sync.clone());
        gs.follow(&CONTRAST, move |v| {
            // GNOME's shortcut, the quick settings or GNOME Settings changed it
            chosen.set(*v == true);
            dd.set_selected(Some(if *v == true { "high" } else { "none" }));
            sync();
        });
    }
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row.append(&common::sized(dd.widget(), 200));
    row.append(&apply);
    let card = SettingsCard::new("eye", t("Contrastthema’s", "Contrast themes"), "");
    card.set_control(&row);
    let group = SettingsGroup::new("", "");
    group.add(card.widget());
    if gs.available(&CONTRAST_SHORTCUT) {
        let key = SettingsCard::clickable(
            "keyboard",
            t("Sneltoets voor contrastthema’s", "Keyboard shortcut for contrast themes"),
            t("Wijzigen in Sneltoetsen", "Change it in Keyboard shortcuts"),
            c::Chevron::Right,
        );
        key.set_control(&shortcut_control(gs));
        if let Some(b) = key.button() {
            b.set_widget_name("st-contrast-toggle-contrast");
        }
        let pp = p.clone();
        key.connect_clicked(move || pp.go_with("shortcuts", json!({ "section": "accessibility" })));
        group.add(key.widget());
    }
    Some(group.widget().clone().upcast())
}

fn contrast(p: Page) -> PageFuture {
    Box::pin(async move {
        let keys = [CONTRAST, CONTRAST_SHORTCUT];
        let gs = Gs::load(&p, &keys).await;
        if !p.alive() {
            return Content::new();
        }
        let mut out: Content = Vec::new();
        let intro = c::label(
            t(
                "Contrastthema’s gebruiken eigen kleuren voor tekst en apps, zodat ze beter te zien zijn.",
                "Contrast themes use distinct colours for text and apps to make them easier to see.",
            ),
            "w11-text-1 st-a11y-intro",
            true,
        );
        intro.set_xalign(0.0);
        out.push(intro.upcast());
        match contrast_group(&p, &gs, &keys) {
            Some(group) => {
                out.push(group);
                let note = c::label(
                    t(
                        "GNOME- en GTK-apps hebben één stijl voor hoog contrast. WinPoint gebruikt zijn eigen contrastkleuren.",
                        "GNOME and GTK apps have one high contrast style. WinPoint uses its own contrast colours.",
                    ),
                    "w11-text-2",
                    true,
                );
                note.set_xalign(0.0);
                out.push(note.upcast());
            }
            None => {
                let g = SettingsGroup::new("", "");
                g.add(common::unavailable_card("eye", t("Contrastthema’s", "Contrast themes"), None).widget());
                out.push(g.widget().clone().upcast());
            }
        }

        let tips = SettingsGroup::new(t("Tips voor beter leesbare tekst", "Tips for easier reading"), "");
        let mode = common::setting_dropdown(
            &p,
            "theme.mode",
            vec![(json!("light"), t("Licht", "Light").into()), (json!("dark"), t("Donker", "Dark").into())],
            None,
        );
        mode.widget().set_widget_name("st-contrast-mode");
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
        sw.switch().set_widget_name("st-contrast-transparency");
        tips.add(trans.widget());
        tips.add(common::link_card(&p, "textsize").widget());
        out.push(tips.widget().clone().upcast());
        out
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn animation_effects_are_gnomes_reduced_motion_inverted() {
        assert!(animations_on(&json!("no-preference")));
        assert!(!animations_on(&json!("reduce")));
        assert_eq!(motion_value(false), json!("reduce"));
        assert_eq!(motion_value(true), json!("no-preference"));
        let keys = [OVERLAY, MOTION, ANIMATIONS, SHAPES];
        assert_eq!(gs::control_name("visualeffects", &keys, &MOTION), "st-visualeffects-reduced-motion");
        assert_eq!(gs::control_name("visualeffects", &keys, &OVERLAY), "st-visualeffects-overlay-scrolling");
        assert_eq!(gs::control_name("visualeffects", &keys, &SHAPES), "st-visualeffects-show-status-shapes");
        let keys = [MOTION, ANIMATIONS, MENU];
        assert_eq!(gs::control_name("accessibility", &keys, &MENU), "st-accessibility-always-show-universal-access-status");
    }

    #[test]
    fn text_size_is_text_scaling_factor_in_percent() {
        assert_eq!(percent_of(&json!(1.0)), 100);
        assert_eq!(percent_of(&json!(1.25)), 125);
        assert_eq!(percent_of(&json!(2.25)), 225);
        assert_eq!(percent_of(&json!(0.8)), 80, "a GNOME value outside Windows' range is shown as it is");
        assert_eq!(percent_of(&Value::Null), 100);
        for pct in [100, 137, 225] {
            assert_eq!(percent_of(&factor_of(pct)), pct, "round trip {pct}");
        }
        assert_eq!(factor_of(150), json!(1.5));
        assert_eq!(gs::control_name("textsize", &[SCALE], &SCALE), "st-textsize-text-scaling-factor");
    }

    #[test]
    fn contrast_themes_are_none_and_gnomes_high_contrast() {
        let options = contrast_options();
        assert_eq!(options.iter().map(|(v, _)| *v).collect::<Vec<_>>(), ["none", "high"]);
        assert_eq!(gs::control_name("contrast", &[CONTRAST, CONTRAST_SHORTCUT], &CONTRAST), "st-contrast-high-contrast");
        assert_eq!(contrast_accel(&json!([""])), None, "GNOME's default: no shortcut");
        assert_eq!(contrast_accel(&json!(["", "<Alt><Shift>Print"])).as_deref(), Some("<Alt><Shift>Print"));
    }
}
