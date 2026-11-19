//! Instellingen › Persoonlijke instellingen › Kleuren (6ebf169f:vendor/ui/js/apps/settings/pages/colors.js): the
//! mode, transparency, the accent colour (manual or automatic from the wallpaper, Windows’ 48 swatches, recent and
//! custom colours), the accent on Start and the taskbar, the GTK theme of other apps (theme.gtkFamily) and the
//! Windows 11 caption buttons in other apps (theme.captionButtons).
//!
//! GNOME Settings 50's Appearance (settings parity, batch F): Kies je modus is GNOME's Style (theme.mode, written to
//! `color-scheme` by WinPoint's look); the accent goes to GNOME's `accent-color` as the nearest of GNOME's names
//! (B0's mirror, while theme.syncSystem), which the row "Accentkleur in Ubuntu- en GNOME-apps" shows and follows.
use super::background::{color_block, remember, remembered};
use super::{Builder, Page, PageFuture};
use crate::common;
use crate::gs::{self, GKey, Gs};
use kit::controls::{self as c, ColorSwatches, Dropdown, DropdownOption, SettingsCard, SettingsExpander, SettingsGroup, Variant};
use kit::prelude::*;
use kit::t;
use std::cell::RefCell;
use std::rc::Rc;

pub const PAGES: &[(&str, Builder)] = &[("colors", page)];
pub const CSS: &str = "";

const GNOME_ACCENT: GKey = gs::key("org.gnome.desktop.interface", "accent-color");

/// GNOME's accent colour names (and Ubuntu's brown) as Settings shows them.
pub(super) fn gnome_accent_name(name: &str) -> String {
    match name {
        "blue" => t("Blauw", "Blue"),
        "teal" => t("Groenblauw", "Teal"),
        "green" => t("Groen", "Green"),
        "yellow" => t("Geel", "Yellow"),
        "orange" => t("Oranje", "Orange"),
        "red" => t("Rood", "Red"),
        "pink" => t("Roze", "Pink"),
        "purple" => t("Paars", "Purple"),
        "slate" => t("Leigrijs", "Slate"),
        "brown" => t("Bruin", "Brown"),
        other => other,
    }
    .to_owned()
}

/// The line under "Accentkleur in Ubuntu- en GNOME-apps": WinPoint passes its accent on (theme.syncSystem) or not.
fn gnome_accent_text(sync: bool) -> &'static str {
    if sync {
        t(
            "Ubuntu- en GNOME-apps gebruiken de dichtstbijzijnde GNOME-accentkleur",
            "Ubuntu and GNOME apps use the nearest GNOME accent colour",
        )
    } else {
        t("Ubuntu- en GNOME-apps houden hun eigen accentkleur", "Ubuntu and GNOME apps keep their own accent colour")
    }
}

fn recent_colors() -> Vec<String> {
    let mut l: Vec<String> = remembered("recentColors").into_iter().filter(|c| kit::theme::normalize_hex(c).is_some()).collect();
    l.truncate(5);
    l
}

fn push_recent(hex: &str) {
    let mut list = vec![hex.to_owned()];
    list.extend(recent_colors().into_iter().filter(|x| !x.eq_ignore_ascii_case(hex)));
    list.truncate(5);
    remember("recentColors", list);
}

/// Set the accent (colors.js setAccent): an automatic accent becomes manual in the same write; the colour goes
/// to the recent colours once it is saved. `after` runs when it is saved.
fn set_accent(p: &Page, hex: &str, after: Rc<dyn Fn()>) {
    let Some(n) = kit::theme::normalize_hex(hex).map(|h| h.to_uppercase()) else { return };
    if common::setting_bool("theme.accentFromWallpaper", false) {
        let wait = p.call("settings.patch", json!({ "patch": { "theme": { "accent": n, "accentFromWallpaper": false } } }));
        let pp = p.clone();
        p.spawn(async move {
            match wait.await {
                Ok(_) => {
                    push_recent(&n);
                    after();
                }
                Err(e) => pp.error(&e, None),
            }
        });
    } else {
        let n2 = n.clone();
        common::set_setting(p, "theme.accent", json!(n), move |ok| {
            if ok {
                push_recent(&n2);
                after();
            }
        });
    }
}

/// "Recente kleuren": rebuilt only when the list changes (a swatch grid owns a style sheet).
struct Recent {
    p: Page,
    slot: gtk::Box,
    block: gtk::Box,
    shown: RefCell<Option<Vec<String>>>,
    grid: RefCell<Option<ColorSwatches>>,
}

impl Recent {
    fn paint(self: &Rc<Self>) {
        let list = recent_colors();
        self.block.set_visible(!list.is_empty());
        let accent = common::setting_str("theme.accent").unwrap_or_default();
        if self.shown.borrow().as_ref() == Some(&list) {
            if let Some(g) = self.grid.borrow().as_ref() {
                g.set_selected(&accent);
            }
            return;
        }
        c::clear(&self.slot);
        *self.shown.borrow_mut() = Some(list.clone());
        *self.grid.borrow_mut() = None;
        if list.is_empty() {
            return;
        }
        let refs: Vec<&str> = list.iter().map(String::as_str).collect();
        let grid = ColorSwatches::new(Some(&refs), Some(&accent), 5, 40);
        grid.widget().set_widget_name("colors-recent");
        let weak = Rc::downgrade(self);
        grid.connect_selected(move |c| {
            if let Some(me) = weak.upgrade() {
                let again = me.clone();
                set_accent(&me.p, c, Rc::new(move || again.paint()));
            }
        });
        self.slot.append(grid.widget());
        *self.grid.borrow_mut() = Some(grid);
    }
}

fn page(p: Page) -> PageFuture {
    Box::pin(async move {
        let gs = Gs::load(&p, &[GNOME_ACCENT]).await;
        if !p.alive() {
            return Vec::new();
        }
        // Gs follows gsettings.changed through a weak reference: the page keeps it while it is shown
        let keep = gs.clone();
        p.on_cleanup(move || drop(keep));
        let mode_dd = common::setting_dropdown(
            &p,
            "theme.mode",
            vec![(json!("light"), t("Licht", "Light").into()), (json!("dark"), t("Donker", "Dark").into())],
            None,
        );
        mode_dd.widget().set_widget_name("colors-mode");
        let mode_card = SettingsCard::new(
            "theme",
            t("Kies je modus", "Choose your mode"),
            t("De kleuren van WinPoint en je apps: licht of donker", "The colours of WinPoint and your apps: light or dark"),
        );
        mode_card.set_control(&common::sized(mode_dd.widget(), 160));
        let (trans_card, trans_sw) = common::toggle_card(
            &p,
            "sparkle",
            t("Transparantie-effecten", "Transparency effects"),
            t("Vensters en oppervlakken zijn een beetje doorschijnend", "Windows and surfaces appear slightly translucent"),
            "theme.transparency",
            false,
        );
        trans_sw.switch().set_widget_name("colors-transparency");

        // ---- the accent expander
        let auto = || common::setting_bool("theme.accentFromWallpaper", false);
        let accent_now = || common::setting_str("theme.accent").unwrap_or_default();
        let how = Dropdown::new(
            vec![DropdownOption::new("manual", t("Handmatig", "Manual")), DropdownOption::new("auto", t("Automatisch", "Automatic"))],
            Some(if auto() { "auto" } else { "manual" }),
        );
        how.widget().set_widget_name("colors-accent-mode");
        {
            let (pp, how2) = (p.clone(), how.clone());
            how.connect_changed(move |v| {
                let how3 = how2.clone();
                common::set_setting(&pp, "theme.accentFromWallpaper", json!(v == "auto"), move |ok| {
                    if !ok {
                        how3.set_selected(Some(if common::setting_bool("theme.accentFromWallpaper", false) { "auto" } else { "manual" }));
                    }
                });
            });
        }
        let presets = ColorSwatches::new(None, Some(&accent_now()), 8, 40);
        presets.widget().set_widget_name("colors-accents");
        let recent_slot = gtk::Box::new(gtk::Orientation::Vertical, 0);
        recent_slot.add_css_class("st-recent-colors");
        let auto_note = c::label(
            t("De accentkleur wordt gekozen uit je bureaubladachtergrond.", "The accent colour is picked from your desktop background."),
            "st-sub-note",
            true,
        );
        let recent_block = color_block(t("Recente kleuren", "Recent colours"), &recent_slot);
        let preset_block = color_block(t("Kleuren", "Colours"), presets.widget());
        let custom_btn = c::button(t("Kleuren weergeven", "View colours"), Variant::Standard);
        custom_btn.set_widget_name("colors-custom");
        let custom_card = SettingsCard::new("", t("Aangepaste kleuren", "Custom colours"), "");
        custom_card.set_control(&custom_btn);
        let exp = SettingsExpander::new("color", t("Accentkleur", "Accent colour"), t("De kleur van knoppen, schakelaars en de selectie", "The colour of buttons, switches and selection"));
        exp.widget().add_css_class("st-accent-exp");
        exp.header().set_control(&common::sized(how.widget(), 160));
        exp.set_expanded(true);
        exp.add(&auto_note);
        exp.add(&recent_block);
        exp.add(&preset_block);
        exp.add(custom_card.widget());
        // GNOME's accent-color: WinPoint's look sets it, so here it is only shown (and followed)
        let gnome_card = gs.available(&GNOME_ACCENT).then(|| {
            // a row of the expander, as Aangepaste kleuren: no icon of its own
            let card = SettingsCard::new(
                "",
                t("Accentkleur in Ubuntu- en GNOME-apps", "Accent colour in Ubuntu and GNOME apps"),
                gnome_accent_text(common::setting_bool("theme.syncSystem", true)),
            );
            let value = common::value_label(&gnome_accent_name(common::s(&gs.value(&GNOME_ACCENT))));
            value.set_widget_name("st-colors-accent-color");
            card.set_control(&value);
            gs.follow(&GNOME_ACCENT, move |v| value.set_label(&gnome_accent_name(common::s(v))));
            exp.add(card.widget());
            card
        });

        // the recent colours: rebuilt only when the list changes (a swatch grid owns a style sheet)
        let recent = Rc::new(Recent { p: p.clone(), slot: recent_slot.clone(), block: recent_block.clone(), shown: RefCell::new(None), grid: RefCell::new(None) });
        let paint_recent: Rc<dyn Fn()> = {
            let r = recent.clone();
            Rc::new(move || r.paint())
        };
        {
            let (pp, pr) = (p.clone(), paint_recent.clone());
            presets.connect_selected(move |c| set_accent(&pp, c, pr.clone()));
        }
        {
            let (pp, pr) = (p.clone(), paint_recent.clone());
            custom_btn.connect_clicked(move |_| {
                let cur = common::setting_str("theme.accent").unwrap_or_else(|| "#0078d4".into());
                let (p2, pr) = (pp.clone(), pr.clone());
                crate::colorpicker::pick_color(&pp.anchor(), &cur, None, true, move |hex| {
                    if let Some(h) = hex {
                        set_accent(&p2, &h, pr);
                    }
                });
            });
        }

        let (tb_card, tb_sw) = common::toggle_card(
            &p,
            "taskbar",
            t("Accentkleur weergeven op Start en taakbalk", "Show accent colour on Start and taskbar"),
            t("Start, de taakbalk en het meldingencentrum krijgen je accentkleur", "Start, the taskbar and the notification centre use your accent colour"),
            "theme.accentOnTaskbar",
            false,
        );
        tb_sw.switch().set_widget_name("colors-accent-taskbar");
        let (app_card, app_dd) = common::dropdown_card(
            &p,
            "brush",
            t("Thema voor apps", "Theme for apps"),
            t(
                "Hoe andere apps eruitzien; licht en donker volgen je modus. Zonder Fluent-thema houden apps het Ubuntu-thema.",
                "How other apps look; light and dark follow your mode. Without the Fluent theme, apps keep the Ubuntu theme.",
            ),
            "theme.gtkFamily",
            vec![(json!("fluent"), t("Windows 11 (Fluent)", "Windows 11 (Fluent)").into()), (json!("system"), t("Ubuntu-standaard (Yaru)", "Ubuntu default (Yaru)").into())],
            Some(json!("fluent")),
        );
        app_dd.widget().set_widget_name("colors-app-theme");
        // the bound drop-down at its CSS width: out of the card first, then back in a fixed-width slot
        app_card.clear_control();
        app_card.set_control(&common::sized(app_dd.widget(), 220));
        let (caption_card, caption_sw) = common::toggle_card(
            &p,
            "window",
            t("Windows 11-vensterknoppen in apps", "Windows 11 caption buttons in apps"),
            t(
                "Minimaliseren, maximaliseren en sluiten zoals in Windows 11, in licht en donker. Geldt voor apps die je hierna opent; Snap- en Flatpak-apps houden hun eigen knoppen.",
                "Minimise, maximise and close like Windows 11, in light and dark. Applies to apps you open after this; Snap and Flatpak apps keep their own buttons.",
            ),
            "theme.captionButtons",
            false,
        );
        caption_sw.switch().set_widget_name("colors-caption-buttons");

        let sync = {
            let (how, presets, auto_note, pr) = (how.clone(), presets.clone(), auto_note.clone(), paint_recent.clone());
            let gnome_card = gnome_card.clone();
            let dimmed: Vec<gtk::Widget> = vec![recent_block.clone().upcast(), preset_block.clone().upcast(), custom_card.widget().clone()];
            let (tb_card, tb_sw) = (tb_card.clone(), tb_sw.clone());
            move || {
                let a = common::setting_bool("theme.accentFromWallpaper", false);
                how.set_selected(Some(if a { "auto" } else { "manual" }));
                presets.set_selected(&common::setting_str("theme.accent").unwrap_or_default());
                // .st-accent-exp.is-auto: the manual choices fade and take no clicks
                for w in &dimmed {
                    w.set_opacity(if a { 0.45 } else { 1.0 });
                    w.set_can_target(!a);
                    w.set_can_focus(!a);
                }
                auto_note.set_visible(a);
                if let Some(card) = &gnome_card {
                    card.set_description(gnome_accent_text(common::setting_bool("theme.syncSystem", true)));
                }
                pr();
                let light = !kit::theme::dark();
                tb_card.set_sensitive(!light);
                tb_sw.set_sensitive(!light);
                tb_card.set_description(if light {
                    t("Alleen beschikbaar in de donkere modus", "Only available in dark mode")
                } else {
                    t("Start, de taakbalk en het meldingencentrum krijgen je accentkleur", "Start, the taskbar and the notification centre use your accent colour")
                });
            }
        };
        sync();
        p.on_settings(move |_, _| sync());

        let main = SettingsGroup::new("", "");
        for w in [mode_card.widget(), trans_card.widget()] {
            main.add(w);
        }
        main.add(exp.widget());
        for w in [tb_card.widget(), app_card.widget(), caption_card.widget()] {
            main.add(w);
        }
        let related = SettingsGroup::new(common::ct::related(), "");
        related.add(common::link_card(&p, "contrast").widget());
        related.add(common::link_card(&p, "themes").widget());
        vec![main.widget().clone().upcast(), related.widget().clone().upcast()]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gnome_accent_names_read_in_both_languages() {
        kit::i18n::set_forced_lang(Some("nl"));
        assert_eq!(gnome_accent_name("teal"), "Groenblauw");
        assert_eq!(gnome_accent_name("brown"), "Bruin");
        assert_eq!(gnome_accent_name("magenta"), "magenta", "a name GNOME may add later stays readable");
        kit::i18n::set_forced_lang(Some("en"));
        assert_eq!(gnome_accent_name("slate"), "Slate");
        assert_eq!(gnome_accent_text(false), "Ubuntu and GNOME apps keep their own accent colour");
        kit::i18n::set_forced_lang(None);
    }
}
