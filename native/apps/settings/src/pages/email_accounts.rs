//! Instellingen › Accounts › E-mail en accounts (Windows 11: Accounts › Email & accounts; batch E,
//! notes/claude/settings-parity.md §2): GNOME Online Accounts as GNOME Settings' Online Accounts panel shows them
//! (`goa.*`): the accounts with Windows' Beheren (the services an account is used for) and Verwijderen, "Opnieuw
//! aanmelden" for an account that needs attention, and "Account toevoegen" with GOA's own dialog in a process of its
//! own (`win11-gtk-settings --goa-add`). Windows' "Accounts die worden gebruikt door andere apps" has no GNOME
//! counterpart and is left out.
use super::accounts::refresh_on;
use super::{Builder, Content, Page, PageFuture};
use crate::common;
use kit::controls::{self as c, Chevron, Dialog, DialogResult, InfoBar, SettingsCard, SettingsExpander, SettingsGroup, Severity, ToggleSwitch, Variant};
use kit::prelude::*;
use kit::t;

pub const PAGES: &[(&str, Builder)] = &[("email-accounts", page)];
pub const CSS: &str = "";

/// The services of GOA's accounts (host goa.rs SERVICES) as GNOME Settings names them: (id, nl, en).
const SERVICES: &[(&str, &str, &str)] = &[
    ("mail", "E-mail", "Email"),
    ("calendar", "Agenda", "Calendar"),
    ("contacts", "Contactpersonen", "Contacts"),
    ("files", "Bestanden", "Files"),
    ("photos", "Foto's", "Photos"),
    ("documents", "Documenten", "Documents"),
    ("printers", "Printers", "Printers"),
    ("maps", "Kaarten", "Maps"),
    ("music", "Muziek", "Music"),
    ("todo", "Taken", "To Do"),
    ("chat", "Chat", "Chat"),
    // GNOME Online Accounts' own name of the switch ("Network _Resources"); Windows has no such service
    ("ticketing", "Netwerkbronnen", "Network Resources"),
    ("readLater", "Later lezen", "Read Later"),
];

/// A service's name in the UI language.
fn service_name(id: &str) -> &str {
    SERVICES.iter().find(|(s, _, _)| *s == id).map(|(_, nl, en)| t(nl, en)).unwrap_or(id)
}

/// The glyph of a provider in Windows' account list.
fn provider_glyph(provider: &str) -> &'static str {
    match provider {
        "kerberos" | "fedora" => "key",
        "owncloud" | "webdav" => "cloud",
        _ => "mail",
    }
}

/// "Beheren": the services the account is used for (GNOME: Gebruiken voor), each a switch.
fn manage(p: &Page, account: &Value) {
    let body = gtk::Box::new(gtk::Orientation::Vertical, 4);
    body.append(&c::label(t("Dit account gebruiken voor:", "Use this account for:"), "w11-body", true));
    let id = account["id"].as_str().unwrap_or("").to_owned();
    for service in account["services"].as_array().into_iter().flatten() {
        let sid = service["id"].as_str().unwrap_or("").to_owned();
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        row.set_margin_top(4);
        let label = c::label(service_name(&sid), "w11-body", false);
        label.set_hexpand(true);
        row.append(&label);
        let sw = ToggleSwitch::new(service["on"] == true);
        sw.switch().set_widget_name(&format!("st-email-accounts-{sid}"));
        sw.set_sensitive(account["locked"] != true);
        let (pp, id2, sw2) = (p.clone(), id.clone(), sw.clone());
        sw.connect_toggled(move |on| {
            let (sw3, wait) = (sw2.clone(), pp.run("goa.setService", json!({ "id": id2, "service": sid, "on": on })));
            pp.spawn(async move {
                if wait.await.is_none() {
                    sw3.set_active(!on);
                }
            });
        });
        row.append(sw.widget());
        body.append(&row);
    }
    let title = common::fill(t("Accountinstellingen voor {name}", "Account settings for {name}"), &[("name", account["presentationIdentity"].as_str().unwrap_or(""))]);
    Dialog::new(&title).body(&body).close_label(t("Gereed", "Done")).width(440).show(&p.anchor(), |_| {});
}

/// "Verwijderen", after Windows' question.
fn remove(p: &Page, account: &Value) {
    let name = account["presentationIdentity"].as_str().unwrap_or("").to_owned();
    let id = account["id"].as_str().unwrap_or("").to_owned();
    let pp = p.clone();
    c::confirm(
        &p.anchor(),
        t("Dit account verwijderen?", "Remove this account?"),
        &common::fill(
            t(
                "{name} wordt van deze pc verwijderd. E-mail, agenda, contactpersonen en bestanden van dit account zijn dan niet meer beschikbaar in je apps.",
                "{name} will be removed from this PC. Email, calendar, contacts and files of this account will then no longer be available in your apps.",
            ),
            &[("name", &name)],
        ),
        t("Verwijderen", "Remove"),
        true,
        move |ok| {
            if ok {
                let wait = pp.run("goa.remove", json!({ "id": id }));
                pp.spawn(async move {
                    let _ = wait.await;
                });
            }
        },
    );
}

/// "Een account toevoegen": the providers; GOA's own dialog opens in a window of its own.
fn add(p: &Page, providers: &[Value]) {
    let body = gtk::Box::new(gtk::Orientation::Vertical, 4);
    let dialog = Dialog::new(t("Een account toevoegen", "Add an account")).width(440);
    for provider in providers {
        let kind = provider["type"].as_str().unwrap_or("").to_owned();
        let name = provider["name"].as_str().unwrap_or("").to_owned();
        let card = SettingsCard::clickable(provider_glyph(&kind), &name, "", Chevron::Right);
        if let Some(b) = card.button() {
            b.set_widget_name(&format!("st-email-accounts-add-{kind}"));
        }
        let (pp, d) = (p.clone(), dialog.clone());
        card.connect_clicked(move || {
            d.close(DialogResult::Close);
            let wait = pp.run("goa.add", json!({ "provider": kind }));
            let (p2, name) = (pp.clone(), name.clone());
            pp.spawn(async move {
                if let Some(v) = wait.await
                    && v["dryRun"] != true
                {
                    common::toast_success(&p2.anchor(), &common::fill(t("Volg de stappen in het venster van {name}.", "Follow the steps in the {name} window."), &[("name", &name)]), 5000);
                }
            });
        });
        body.append(card.widget());
    }
    dialog.body(&body).close_label(t("Annuleren", "Cancel")).show(&p.anchor(), |_| {});
}

fn page(p: Page) -> PageFuture {
    Box::pin(async move {
        let (list, providers, net) = (p.call("goa.list", json!({})), p.call("goa.providers", json!({})), p.call("system.network.get", json!({})));
        let (list, providers, net) = (list.await, providers.await, net.await);
        if !p.alive() {
            return Content::new();
        }
        let mut out: Content = Vec::new();
        let list = list.unwrap_or(Value::Null);
        if list["available"] != true {
            out.push(common::notice(t("Onlineaccounts zijn op deze computer niet beschikbaar.", "Online accounts aren’t available on this computer.")).widget().clone().upcast());
        }
        // GNOME's offline banner: accounts can't be added or signed in to without a connection
        let offline = net.ok().is_some_and(|n| n["connectivity"] == "none");
        if offline {
            let bar = common::notice_full(Severity::Warning, t("Geen internetverbinding", "No internet connection"), t("Accounts kunnen pas worden toegevoegd als je weer verbinding hebt.", "Accounts can only be added once you’re connected again."));
            bar.widget().set_widget_name("st-email-accounts-offline");
            out.push(bar.widget().clone().upcast());
        }
        let providers: Vec<Value> = providers.ok().and_then(|v| v["providers"].as_array().cloned()).unwrap_or_default();
        let group = SettingsGroup::new(t("Accounts die worden gebruikt door e-mail, agenda en contactpersonen", "Accounts used by email, calendar and contacts"), "");
        let add_card = SettingsCard::new("plus", t("Een account toevoegen", "Add an account"), t("Google, Microsoft 365, Nextcloud, IMAP en meer", "Google, Microsoft 365, Nextcloud, IMAP and more"));
        let add_btn = c::button(t("Account toevoegen", "Add account"), Variant::Standard);
        add_btn.set_widget_name("st-email-accounts-add");
        add_btn.set_sensitive(list["canAdd"] == true && !offline && !providers.is_empty());
        {
            let (pp, providers) = (p.clone(), providers.clone());
            add_btn.connect_clicked(move |_| add(&pp, &providers));
        }
        add_card.set_control(&add_btn);
        group.add(add_card.widget());
        let accounts = list["accounts"].as_array().cloned().unwrap_or_default();
        for account in &accounts {
            let id = account["id"].as_str().unwrap_or("");
            let mut desc = account["providerName"].as_str().unwrap_or("").to_owned();
            let attention = account["attentionNeeded"] == true;
            if attention {
                desc = format!("{desc} · {}", t("Aanmelden vereist", "Sign-in required"));
            }
            let exp = SettingsExpander::new(provider_glyph(account["provider"].as_str().unwrap_or("")), account["presentationIdentity"].as_str().unwrap_or(""), &desc);
            exp.header_button().set_widget_name(&format!("st-email-accounts-{id}"));
            let locked = account["locked"] == true;
            if attention {
                let bar = InfoBar::new(Severity::Warning, t("Aanmelden vereist", "Sign-in required"), t("Meld je opnieuw aan om dit account te blijven gebruiken.", "Sign in again to keep using this account."));
                bar.set_closable(false);
                let again = c::button(t("Opnieuw aanmelden", "Sign in again"), Variant::Standard);
                again.set_widget_name(&format!("st-email-accounts-signin-{id}"));
                again.set_sensitive(!offline);
                {
                    let (pp, id) = (p.clone(), id.to_owned());
                    again.connect_clicked(move |_| {
                        let wait = pp.run("goa.signIn", json!({ "id": id }));
                        pp.spawn(async move {
                            let _ = wait.await;
                        });
                    });
                }
                bar.set_action_widget(&again);
                exp.add(bar.widget());
            }
            let manage_card = SettingsCard::new("", t("Accountinstellingen", "Account settings"), t("Kies waarvoor je dit account gebruikt", "Choose what you use this account for"));
            let manage_btn = c::button(t("Beheren", "Manage"), Variant::Standard);
            manage_btn.set_widget_name(&format!("st-email-accounts-manage-{id}"));
            manage_btn.set_sensitive(account["services"].as_array().is_some_and(|s| !s.is_empty()));
            {
                let (pp, account) = (p.clone(), account.clone());
                manage_btn.connect_clicked(move |_| manage(&pp, &account));
            }
            manage_card.set_control(&manage_btn);
            exp.add(manage_card.widget());
            let remove_card = SettingsCard::new("", t("Dit account verwijderen", "Remove this account"), "");
            let remove_btn = c::button(t("Verwijderen", "Remove"), Variant::Standard);
            remove_btn.set_widget_name(&format!("st-email-accounts-remove-{id}"));
            if locked {
                remove_btn.set_sensitive(false);
                remove_card.set_description(t("Dit account wordt beheerd door je organisatie.", "This account is managed by your organisation."));
            }
            {
                let (pp, account) = (p.clone(), account.clone());
                remove_btn.connect_clicked(move |_| remove(&pp, &account));
            }
            remove_card.set_control(&remove_btn);
            exp.add(remove_card.widget());
            if attention {
                exp.set_expanded(true);
            }
            group.add(exp.widget());
        }
        if accounts.is_empty() && list["available"] == true {
            group.add(SettingsCard::new("", t("Er zijn nog geen accounts toegevoegd.", "No accounts have been added yet."), "").widget());
        }
        out.push(group.widget().clone().upcast());
        let related = SettingsGroup::new(common::ct::related(), "");
        related.add(common::link_card(&p, "defaultapps").widget());
        out.push(related.widget().clone().upcast());
        refresh_on(&p, "goa.changed");
        out
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn every_goa_service_has_a_name_and_providers_a_glyph() {
        let ids: Vec<&str> = SERVICES.iter().map(|(id, _, _)| *id).collect();
        assert_eq!(ids, ["mail", "calendar", "contacts", "files", "photos", "documents", "printers", "maps", "music", "todo", "chat", "ticketing", "readLater"], "host goa.rs SERVICES, in order");
        assert_eq!(service_name("unknown"), "unknown");
        assert_eq!(provider_glyph("google"), "mail");
        assert_eq!(provider_glyph("kerberos"), "key");
        assert_eq!(provider_glyph("owncloud"), "cloud");
    }

    /// The quoted strings of a Rust source between `start` and the next `];`.
    pub(crate) fn quoted_after(source: &str, start: &str) -> Vec<String> {
        let at = source.find(start).expect("the table");
        let body = &source[at..at + source[at..].find("];").expect("its end")];
        body.split('"').skip(1).step_by(2).map(str::to_owned).collect()
    }

    #[test]
    fn the_names_cover_the_hosts_services() {
        // host/src/services/goa.rs SERVICES: (id, "<Service>Disabled", interface)
        let host = include_str!("../../../../../host/src/services/goa.rs");
        let strings = quoted_after(host, "pub(crate) const SERVICES");
        let ids: Vec<&str> = strings.windows(2).filter(|w| w[1].ends_with("Disabled")).map(|w| w[0].as_str()).collect();
        let ours: Vec<&str> = SERVICES.iter().map(|(id, _, _)| *id).collect();
        assert_eq!(ids, ours);
    }
}
