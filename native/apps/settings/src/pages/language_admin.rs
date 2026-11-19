//! Instellingen › Tijd en taal › Taal en regio › Beheerinstellingen voor taal (Windows 11: Administrative language
//! settings, the Region dialog's "Welkomstscherm en nieuwe gebruikersaccounts"; batch E, notes/claude/settings-parity.md
//! §2): GNOME Settings' Region & Language › Login Screen. The sign-in screen's language and formats are the system
//! locale (systemd-localed, `machine.login.setLocale`, which sends what GNOME sends: LANG, and LC_TIME, LC_NUMERIC,
//! LC_MONETARY, LC_MEASUREMENT and LC_PAPER for the formats); new user accounts start with it too. "Mijn instellingen
//! kopiëren" copies the own display language and regional format there. locale1 asks polkit (`set-locale`, yes for
//! an administrator on Ubuntu), so the controls carry the shield.
use super::formats::{self, REGION};
use super::{Builder, Content, Page, PageFuture};
use crate::common;
use crate::gs::Gs;
use crate::pickers::admin_button;
use kit::controls::{Dropdown, DropdownOption, SettingsCard, SettingsGroup, Variant};
use kit::prelude::*;
use kit::t;
use std::cell::RefCell;
use std::rc::Rc;

pub const PAGES: &[(&str, Builder)] = &[("language-admin", page)];
pub const CSS: &str = "";

/// The locales a sign-in screen drop-down offers: every generated locale by name, `first` (the formats' "follow the
/// language") on top, and the current value when the list lacks it.
pub fn login_options(locales: &Value, current: &str, first: Option<(&str, &str)>) -> Vec<DropdownOption> {
    let mut out: Vec<DropdownOption> = first.map(|(v, l)| DropdownOption::new(v, l)).into_iter().collect();
    for l in locales["locales"].as_array().into_iter().flatten() {
        if let (Some(id), Some(name)) = (l["id"].as_str(), l["name"].as_str()) {
            out.push(DropdownOption::new(id, name));
        }
    }
    if !out.iter().any(|o| o.value == current) {
        out.insert(0, DropdownOption::new(current, if current.is_empty() { t("Niet ingesteld", "Not set") } else { current }));
    }
    out
}

/// What "Mijn instellingen kopiëren" sends: the own display language (the account's, else the session's) and the own
/// regional format ("" follows the language).
pub fn own_settings(me: &Value, locales: &Value, region: &str) -> (String, String) {
    let language = [me["language"].as_str().unwrap_or(""), locales["session"]["language"].as_str().unwrap_or("")]
        .into_iter()
        .find(|s| !s.is_empty())
        .unwrap_or("")
        .to_owned();
    let formats = if region.is_empty() { me["formatsLocale"].as_str().unwrap_or("").to_owned() } else { region.to_owned() };
    (language, formats)
}

/// A drop-down that sends `machine.login.setLocale {<key>: value}` and goes back to the old value when that fails.
fn login_dropdown(p: &Page, options: Vec<DropdownOption>, current: &str, key: &'static str, name: &str) -> Dropdown {
    let dd = Dropdown::new(options, Some(current));
    dd.widget().set_widget_name(name);
    let before = Rc::new(RefCell::new(current.to_owned()));
    let (pp, dd2) = (p.clone(), dd.clone());
    dd.connect_changed(move |v| {
        let (wait, dd3, before2, v2) = (pp.run("machine.login.setLocale", json!({ key: v })), dd2.clone(), before.clone(), v.to_owned());
        pp.spawn(async move {
            if wait.await.is_some() {
                *before2.borrow_mut() = v2;
            } else {
                dd3.set_selected(Some(before2.borrow().as_str()));
            }
        });
    });
    dd
}

fn page(p: Page) -> PageFuture {
    Box::pin(async move {
        let (login, locales, me) = (p.call("machine.login.get", json!({})), p.call("machine.locales", json!({})), p.call("users.self", json!({})));
        let gs = Gs::load(&p, &[REGION]).await;
        let (login, locales, me) = (login.await.unwrap_or(Value::Null), locales.await.unwrap_or(Value::Null), me.await.unwrap_or(Value::Null));
        if !p.alive() {
            return Content::new();
        }
        let mut out: Content = Vec::new();
        let available = login["available"] == true;
        if !available {
            out.push(common::notice(t("De taal van het aanmeldingsscherm kan hier niet worden gewijzigd.", "The sign-in screen’s language can’t be changed here.")).widget().clone().upcast());
        }
        let group = SettingsGroup::new(
            t("Welkomstscherm en nieuwe gebruikersaccounts", "Welcome screen and new user accounts"),
            t("Het aanmeldingsscherm, de systeemaccounts en nieuwe gebruikersaccounts gebruiken deze taal en notaties.", "The sign-in screen, the system accounts and new user accounts use this language and these formats."),
        );
        let language = login["language"].as_str().unwrap_or("").to_owned();
        let lang_card = SettingsCard::new("shield", t("Taal van het aanmeldingsscherm", "Sign-in screen language"), "");
        let lang_dd = login_dropdown(&p, login_options(&locales, &language, None), &language, "locale", "st-language-admin-language");
        lang_dd.widget().set_sensitive(available);
        lang_card.set_control(&common::sized(lang_dd.widget(), 280));
        group.add(lang_card.widget());
        let fmt = login["formats"].as_str().unwrap_or("").to_owned();
        let fmt_card = SettingsCard::new("shield", t("Notaties van het aanmeldingsscherm", "Sign-in screen formats"), "");
        let follow = t("Hetzelfde als de taal", "Same as the language");
        let fmt_dd = login_dropdown(&p, login_options(&locales, &fmt, Some(("", follow))), &fmt, "formats", "st-language-admin-formats");
        fmt_dd.widget().set_sensitive(available);
        fmt_card.set_control(&common::sized(fmt_dd.widget(), 280));
        group.add(fmt_card.widget());
        let (own_language, own_formats) = own_settings(&me, &locales, gs.value(&REGION).as_str().unwrap_or(""));
        let copy_card = SettingsCard::new(
            "copy",
            t("Mijn instellingen kopiëren", "Copy my settings"),
            &common::fill(
                t("Je weergavetaal ({language}) en regionale notatie gebruiken voor het aanmeldingsscherm en nieuwe gebruikersaccounts", "Use your display language ({language}) and regional format for the sign-in screen and new user accounts"),
                &[("language", &formats::locale_name(&locales, &own_language))],
            ),
        );
        let copy = admin_button(t("Instellingen kopiëren", "Copy settings"), Variant::Standard);
        copy.set_widget_name("st-language-admin-copy");
        copy.set_sensitive(available && !own_language.is_empty());
        {
            let pp = p.clone();
            copy.connect_clicked(move |_| {
                let wait = pp.run("machine.login.setLocale", json!({ "locale": own_language, "formats": own_formats }));
                let p2 = pp.clone();
                pp.spawn(async move {
                    if wait.await.is_some() {
                        p2.refresh();
                    }
                });
            });
        }
        copy_card.set_control(&copy);
        group.add(copy_card.widget());
        out.push(group.widget().clone().upcast());

        let related = SettingsGroup::new(common::ct::related(), "");
        related.add(common::link_card(&p, "language").widget());
        related.add(common::link_card(&p, "otherusers").widget());
        out.push(related.widget().clone().upcast());
        // a change made elsewhere (another Settings, localectl)
        let pp = p.clone();
        p.on("machine.changed", move |d| {
            if d["what"] == "login" {
                pp.refresh();
            }
        });
        out
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sign_in_screen_gets_my_language_and_formats() {
        let locales = json!({"locales": [{"id": "en_GB.UTF-8", "name": "English (United Kingdom)"}], "session": {"language": "en_GB.UTF-8"}});
        assert_eq!(own_settings(&json!({"language": "nl_NL.UTF-8", "formatsLocale": "de_DE.UTF-8"}), &locales, ""), ("nl_NL.UTF-8".into(), "de_DE.UTF-8".into()));
        assert_eq!(own_settings(&json!({"language": ""}), &locales, "nl_BE.UTF-8"), ("en_GB.UTF-8".into(), "nl_BE.UTF-8".into()), "the region key wins");
        let opts = login_options(&locales, "", Some(("", "Same")));
        assert_eq!(opts.iter().map(|o| o.value.as_str()).collect::<Vec<_>>(), ["", "en_GB.UTF-8"]);
        let opts = login_options(&locales, "fr_FR.UTF-8", None);
        assert_eq!(opts[0].value, "fr_FR.UTF-8", "the current value stays");
    }
}
