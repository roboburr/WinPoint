//! Instellingen › Tijd en taal › Typen (batch A, notes/claude/settings-parity.md §2, plan.json "typing"): Windows 11's
//! Time & language › Typing with what GNOME Settings 50 has of it.
//!
//! * Aanraaktoetsenbord (expander): "Het aanraaktoetsenbord weergeven" = GNOME's Screen Keyboard
//!   (`org.gnome.desktop.a11y.applications screen-keyboard-enabled`): Altijd = true; "Als er geen toetsenbord is
//!   aangesloten" = false, where GNOME Shell shows it when the touchscreen is used. Windows' "Nooit" has no GNOME value
//!   (windows.json), so it is not offered.
//! * Windows' spelling, text suggestion and typing insight rows have no GNOME function (GTK 4 has no system-wide spell
//!   check): left out.
//! * The sub-pages: Geavanceerde toetsenbordinstellingen and Sneltoetsen (the registry's children, caps-filtered).
//! * Gerelateerde instellingen: Taalopties (the keyboards), Toegankelijkheid › Toetsenbord.
use super::{Builder, Content, Page, PageFuture};
use crate::common::{self, ct};
use crate::gs::{self, GKey, Gs};
use kit::controls::{self as c, SettingsCard, SettingsExpander, SettingsGroup};
use kit::prelude::*;
use kit::t;

pub const PAGES: &[(&str, Builder)] = &[("typing", typing)];
pub const CSS: &str = "";

const OSK: GKey = gs::key("org.gnome.desktop.a11y.applications", "screen-keyboard-enabled");

/// "Het aanraaktoetsenbord weergeven": GNOME's two values in Windows' words.
pub(super) fn touch_keyboard_options() -> Vec<(Value, String)> {
    vec![
        (json!(false), t("Als er geen toetsenbord is aangesloten", "When no keyboard attached").into()),
        (json!(true), t("Altijd", "Always").into()),
    ]
}

fn typing(p: Page) -> PageFuture {
    Box::pin(async move {
        let gs = Gs::load(&p, &[OSK]).await;
        if !p.alive() {
            return Content::new();
        }
        let mut out: Content = Vec::new();

        if gs.available(&OSK) {
            let group = SettingsGroup::new("", "");
            let exp = SettingsExpander::new("keyboard", t("Aanraaktoetsenbord", "Touch keyboard"), t("Wanneer het aanraaktoetsenbord wordt weergegeven", "When the touch keyboard is shown"));
            exp.header_button().set_widget_name("st-typing-touch-keyboard");
            exp.set_expanded(true);
            let card = SettingsCard::new("", t("Het aanraaktoetsenbord weergeven", "Show the touch keyboard"), "");
            card.set_control(&common::sized(gs.dropdown(&OSK, touch_keyboard_options()).widget(), 320));
            exp.add(card.widget());
            let note = c::label(
                t(
                    "Zonder Altijd verschijnt het aanraaktoetsenbord wanneer je het aanraakscherm gebruikt en er in een tekstvak wordt getypt.",
                    "Without Always, the touch keyboard appears when you use the touchscreen to type in a text box.",
                ),
                "w11-text-2",
                true,
            );
            note.add_css_class("st-expander-note");
            exp.add(&note);
            group.add(exp.widget());
            out.push(group.widget().clone().upcast());
        }

        out.push(common::child_links(&p, "typing").widget().clone().upcast());

        let related = SettingsGroup::new(ct::related(), "");
        related.add(common::link_card_with(&p, "language-options", Some(t("Toetsenborden", "Keyboards")), None, None).widget());
        related.add(
            common::link_card_with(&p, "a11y-keyboard", Some(t("Toegankelijkheid van toetsenbord", "Keyboard accessibility")), Some(t("Plaktoetsen, filtertoetsen, schermtoetsenbord", "Sticky keys, filter keys, on-screen keyboard")), None)
                .widget(),
        );
        out.push(related.widget().clone().upcast());
        out
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_touch_keyboard_has_gnomes_two_values() {
        let options = touch_keyboard_options();
        assert_eq!(options.len(), 2, "Windows' Nooit has no GNOME value");
        assert_eq!(options[1].0, json!(true), "Altijd = screen-keyboard-enabled");
        assert_eq!(gs::control_name("typing", &[OSK], &OSK), "st-typing-screen-keyboard-enabled");
    }
}
