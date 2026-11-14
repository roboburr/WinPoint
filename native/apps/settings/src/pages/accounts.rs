//! Instellingen › Accounts (vendor/ui/js/apps/settings/pages/accounts.js): who you are on this PC (name,
//! picture) and where to change it, and "Nu vergrendelen".
use super::{Builder, Content, Page, PageFuture};
use crate::common;
use kit::controls::{self as c, Chevron, SettingsCard, SettingsGroup, Variant};
use kit::prelude::*;
use kit::t;

pub const PAGES: &[(&str, Builder)] = &[("accounts", page)];
pub const CSS: &str = r#"
.st-account-hero { padding: 4px 0; }
.st-account-hero .w11-avatar { box-shadow: 0 0 0 1px var(--stroke-card); }
.st-account-name {
  font-family: "Segoe UI Variable Display", "Segoe UI Variable", "Segoe UI", "Selawik", "Inter Display", "Inter", "Noto Sans", "Ubuntu Sans", sans-serif;
  font-size: var(--fs-subtitle); font-weight: 600;
}
.st-account-line { color: var(--text-2); }
"#;

/// A clickable card that explains where something is changed (Dialog.alert).
fn info_card(p: &Page, icon: &str, title: &str, desc: &str, dialog_title: &'static str, dialog_body: &'static str, name: &str) -> SettingsCard {
    let card = SettingsCard::clickable(icon, title, desc, Chevron::Right);
    if let Some(b) = card.button() {
        b.set_widget_name(name);
    }
    let pp = p.clone();
    card.connect_clicked(move || c::alert(&pp.anchor(), dialog_title, dialog_body, || {}));
    card
}

fn page(p: Page) -> PageFuture {
    Box::pin(async move {
        let info = p.get("system.info", json!({})).await;
        if !p.alive() {
            return Content::new();
        }
        let info = info.unwrap_or(Value::Null);
        let sh = common::shell_info();
        let u = if info["user"].is_object() { info["user"].clone() } else { sh["user"].clone() };
        let real = u["realName"].as_str().filter(|s| !s.is_empty());
        let login = u["name"].as_str().filter(|s| !s.is_empty());
        let name = real.or(login).unwrap_or("");

        let hero = gtk::Box::new(gtk::Orientation::Horizontal, 24);
        hero.add_css_class("st-account-hero");
        let avatar = common::avatar(name, u["avatar"].as_str(), 96);
        avatar.set_valign(gtk::Align::Center);
        hero.append(&avatar);
        let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
        text.set_valign(gtk::Align::Center);
        text.set_hexpand(true);
        text.append(&c::label(name, "st-account-name", true));
        text.append(&c::label(t("Lokaal account", "Local account"), "st-account-line", true));
        if let Some(login) = login {
            text.append(&c::label(&common::fill(t("Gebruikersnaam: {u}", "User name: {u}"), &[("u", login)]), "st-account-line", true));
        }
        hero.append(&text);

        let settings = SettingsGroup::new(t("Accountinstellingen", "Account settings"), "");
        settings.add(
            info_card(
                &p,
                "user",
                t("Je gegevens", "Your info"),
                t("Je naam en profielfoto", "Your name and profile picture"),
                t("Naam en profielfoto wijzigen", "Change name and picture"),
                t(
                    "Win11 Shell gebruikt de naam en foto van je Ubuntu-account. Je wijzigt ze in de app Instellingen van Ubuntu: Systeem › Gebruikers. Daarna zie je de nieuwe naam en foto ook hier.",
                    "Win11 Shell uses the name and picture of your Ubuntu account. Change them in Ubuntu’s Settings app: System › Users. The new name and picture then appear here too.",
                ),
                "accounts-info",
            )
            .widget(),
        );
        settings.add(
            info_card(
                &p,
                "key",
                t("Aanmeldingsopties", "Sign-in options"),
                t("Wachtwoord wijzigen, automatisch aanmelden", "Change password, automatic sign-in"),
                t("Aanmelden en wachtwoord", "Sign-in and password"),
                t(
                    "Je wachtwoord wijzig je in de app Instellingen van Ubuntu: Systeem › Gebruikers › Wachtwoord. Kies een wachtwoord dat je goed onthoudt; Win11 Shell bewaart nooit je wachtwoord.",
                    "Change your password in Ubuntu’s Settings app: System › Users › Password. Choose a password you remember well; Win11 Shell never stores your password.",
                ),
                "accounts-signin",
            )
            .widget(),
        );
        let lock_btn = c::button(t("Nu vergrendelen", "Lock now"), Variant::Standard);
        lock_btn.set_widget_name("accounts-lock");
        {
            let pp = p.clone();
            lock_btn.connect_clicked(move |_| {
                // power.js powerAction({action: 'lock', confirm: false})
                let wait = pp.run("system.power.action", json!({ "action": "lock" }));
                pp.spawn(async move {
                    let _ = wait.await;
                });
            });
        }
        let lock = SettingsCard::new(
            "lock",
            t("Vergrendelen", "Lock"),
            t("Het scherm vergrendelen als je even weggaat (Windows-logotoets + L)", "Lock the screen when you step away (Windows logo key + L)"),
        );
        lock.set_control(&lock_btn);
        settings.add(lock.widget());

        let privacy = SettingsGroup::new("", "");
        privacy.add(
            SettingsCard::new(
                "shield",
                t(
                    "Je account blijft op deze computer. Win11 Shell stuurt geen gegevens naar internet.",
                    "Your account stays on this computer. Win11 Shell doesn’t send any data to the internet.",
                ),
                "",
            )
            .widget(),
        );
        vec![hero.upcast(), settings.widget().clone().upcast(), privacy.widget().clone().upcast()]
    })
}
