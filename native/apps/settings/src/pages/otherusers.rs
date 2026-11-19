//! Instellingen › Accounts › Andere gebruikers (Windows 11: Accounts › Other users; batch E,
//! notes/claude/settings-parity.md §2): the other people with an account on this PC and GNOME Settings' Users panel
//! functions for them, in Windows' order: "Een andere gebruiker toevoegen · Account toevoegen" (Volledige naam,
//! Gebruikersnaam, Accounttype, a password now or at the first sign-in), and per user Accountopties ("Accounttype
//! wijzigen"), GNOME's Wachtwoord opnieuw instellen, Automatisch aanmelden and Taal, and "Account en gegevens ·
//! Verwijderen" with GNOME's choice to keep or delete the files. Every change asks for the administrator password
//! (polkit `org.freedesktop.accounts.user-administration`), so the buttons carry Windows' shield. Windows' kiosk
//! has no GNOME counterpart and is left out.
use super::accounts::{PasswordForm, account_type_text, admin_locked, display_name, refresh_on, user_avatar};
use super::{Builder, Content, Page, PageFuture};
use crate::common;
use crate::gs::{self, Gs};
use crate::pickers::admin_button;
use kit::controls::{self as c, Dialog, DialogResult, Dropdown, DropdownOption, RadioGroup, SettingsCard, SettingsExpander, SettingsGroup, ToggleSwitch, Variant};
use kit::prelude::*;
use kit::t;
use std::rc::Rc;

pub const PAGES: &[(&str, Builder)] = &[("otherusers", page)];
pub const CSS: &str = "";

/// GNOME's user name suggestion from a full name (user-utils.c generate_username_choices, its first choice): the
/// first word in lowercase ASCII letters and digits; accented letters lose their accent.
pub(crate) fn suggest_user_name(full: &str) -> String {
    let word = full.split_whitespace().next().unwrap_or("");
    let mut out = String::new();
    for ch in word.chars().flat_map(char::to_lowercase) {
        let base = match ch {
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' => 'a',
            'ç' => 'c',
            'è' | 'é' | 'ê' | 'ë' => 'e',
            'ì' | 'í' | 'î' | 'ï' => 'i',
            'ñ' => 'n',
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' => 'o',
            'ù' | 'ú' | 'û' | 'ü' => 'u',
            'ý' | 'ÿ' => 'y',
            c if c.is_ascii_lowercase() || c.is_ascii_digit() => c,
            _ => continue,
        };
        out.push(base);
    }
    // a user name starts with a letter (host accounts.rs valid_user_name)
    let start = out.find(|c: char| c.is_ascii_lowercase()).unwrap_or(out.len());
    out[start..].chars().take(32).collect()
}

/// The line under a user's name: "Administrator · Lokaal account".
fn user_line(u: &Value) -> String {
    format!("{} · {}", account_type_text(u["accountType"].as_str().unwrap_or("")), t("Lokaal account", "Local account"))
}

/// "Een gebruiker voor deze pc maken" (Windows' title; GNOME's Add User fields).
fn add_user(p: &Page) {
    let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let full = c::text_box("");
    full.set_widget_name("st-otherusers-add-name");
    let user = c::text_box("");
    user.set_widget_name("st-otherusers-add-username");
    user.set_max_length(32);
    body.append(&c::field(t("Volledige naam", "Full name"), &full, None));
    body.append(&c::field(t("Gebruikersnaam", "User name"), &user, Some(t("Kleine letters, cijfers, - en _", "Lowercase letters, digits, - and _"))));
    let kind = Dropdown::new(vec![DropdownOption::new("standard", t("Standaardgebruiker", "Standard user")), DropdownOption::new("administrator", t("Administrator", "Administrator"))], Some("standard"));
    kind.widget().set_widget_name("st-otherusers-add-type");
    body.append(&c::field(t("Accounttype", "Account type"), kind.widget(), None));
    let mode = RadioGroup::new(
        &[("atLogin", t("De gebruiker stelt een wachtwoord in bij de eerste aanmelding", "The user sets a password at the first sign-in")), ("now", t("Nu een wachtwoord instellen", "Set a password now"))],
        Some("atLogin"),
        Some(t("Wachtwoord", "Password")),
        false,
    );
    mode.widget().set_widget_name("st-otherusers-add-mode");
    body.append(mode.widget());
    let form = PasswordForm::new(p, false, None, "st-otherusers-add-password");
    // GNOME shows the password fields only with "Nu een wachtwoord instellen" (the dialog stays within the window)
    form.widget.set_visible(false);
    body.append(&form.widget);
    // the user name follows the full name until it is edited by hand (GNOME's behaviour)
    let edited = Rc::new(std::cell::Cell::new(false));
    {
        let (user, edited) = (user.clone(), edited.clone());
        full.connect_changed(move |e| {
            if !edited.get() {
                user.set_text(&suggest_user_name(&e.text()));
            }
        });
    }
    {
        let (edited, full) = (edited.clone(), full.clone());
        user.connect_changed(move |e| edited.set(e.text() != suggest_user_name(&full.text()).as_str()));
    }
    let pp = p.clone();
    let (full2, user2, kind2, mode2, form2) = (full.clone(), user.clone(), kind.clone(), mode.clone(), form.clone());
    let dialog = Dialog::new(t("Een gebruiker voor deze pc maken", "Create a user for this PC")).body(&body).primary(t("Toevoegen", "Add")).width(480).on_primary(move |d| {
        let now = mode2.value().as_deref() == Some("now");
        if full2.text().trim().is_empty() || user2.text().is_empty() || (now && !form2.valid()) {
            return false;
        }
        let mut args = json!({ "realName": full2.text().trim(), "userName": user2.text().as_str(), "accountType": kind2.selected().unwrap_or_else(|| "standard".into()) });
        if now {
            args["password"] = json!(form2.password());
        }
        form2.set_error("");
        d.set_busy(true);
        let wait = pp.call("users.create", args);
        let (d2, f3, p2) = (d.clone(), form2.clone(), pp.clone());
        pp.spawn(async move {
            let answer = wait.await;
            d2.set_busy(false);
            match answer {
                Ok(v) => {
                    f3.clear();
                    d2.close(DialogResult::Primary);
                    common::note_dry_run(&p2.anchor(), &v);
                }
                Err(e) => f3.set_error(&e.message),
            }
        });
        false
    });
    let ready = {
        let (d, full, user, mode, form) = (dialog.clone(), full.clone(), user.clone(), mode.clone(), form.clone());
        Rc::new(move || {
            let now = mode.value().as_deref() == Some("now");
            d.set_primary_enabled(!full.text().trim().is_empty() && !user.text().is_empty() && (!now || form.valid()));
        })
    };
    {
        let r = ready.clone();
        full.connect_changed(move |_| r());
    }
    {
        let r = ready.clone();
        user.connect_changed(move |_| r());
    }
    {
        let (r, form3) = (ready.clone(), form.clone());
        mode.connect_changed(move |v| {
            form3.widget.set_visible(v == "now");
            r();
        });
    }
    {
        let r = ready.clone();
        form.connect_changed(move |_| r());
    }
    let form4 = form.clone();
    dialog.show(&p.anchor(), move |_| form4.clear());
    dialog.set_primary_enabled(false);
}

/// "Accounttype wijzigen" (Windows' dialog: Administrator / Standaardgebruiker, OK).
fn change_type(p: &Page, u: &Value) {
    let current = u["accountType"].as_str().unwrap_or("standard").to_owned();
    let options = RadioGroup::new(&[("administrator", t("Administrator", "Administrator")), ("standard", t("Standaardgebruiker", "Standard user"))], Some(&current), Some(t("Accounttype", "Account type")), false);
    options.widget().set_widget_name("st-otherusers-type");
    let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
    body.append(&c::label(&display_name(u), "w11-body-strong", true));
    body.append(options.widget());
    let (pp, uid) = (p.clone(), u["uid"].clone());
    Dialog::new(t("Accounttype wijzigen", "Change account type")).body(&body).primary("OK").width(400).show(&p.anchor(), move |r| {
        let chosen = options.value().unwrap_or_default();
        if r == DialogResult::Primary && chosen != current {
            let wait = pp.run("users.setAccountType", json!({ "uid": uid, "type": chosen }));
            pp.spawn(async move {
                let _ = wait.await;
            });
        }
    });
}

/// GNOME's "Wachtwoord" of another user: at the next sign-in, or now.
fn reset_password(p: &Page, u: &Value) {
    let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let mode = RadioGroup::new(
        &[("atLogin", t("De gebruiker stelt een wachtwoord in bij de volgende aanmelding", "The user sets a password at the next sign-in")), ("now", t("Nu een wachtwoord instellen", "Set a password now"))],
        Some("atLogin"),
        None,
        false,
    );
    mode.widget().set_widget_name("st-otherusers-password-mode");
    body.append(mode.widget());
    let form = PasswordForm::new(p, false, u["userName"].as_str().map(str::to_owned), "st-otherusers-password");
    // GNOME shows the password fields only with "Nu een wachtwoord instellen" (the dialog stays within the window)
    form.widget.set_visible(false);
    body.append(&form.widget);
    let (pp, uid, mode2, form2) = (p.clone(), u["uid"].clone(), mode.clone(), form.clone());
    let title = common::fill(t("Wachtwoord van {name} opnieuw instellen", "Reset the password of {name}"), &[("name", &display_name(u))]);
    let dialog = Dialog::new(&title).body(&body).primary(t("Opslaan", "Save")).width(460).on_primary(move |d| {
        let now = mode2.value().as_deref() == Some("now");
        if now && !form2.valid() {
            return false;
        }
        let mut args = json!({ "uid": uid, "mode": if now { "now" } else { "atLogin" } });
        if now {
            args["password"] = json!(form2.password());
        }
        d.set_busy(true);
        let wait = pp.call("users.setPassword", args);
        let (d2, f3, p2) = (d.clone(), form2.clone(), pp.clone());
        pp.spawn(async move {
            let answer = wait.await;
            d2.set_busy(false);
            match answer {
                Ok(v) => {
                    f3.clear();
                    d2.close(DialogResult::Primary);
                    common::note_dry_run(&p2.anchor(), &v);
                }
                Err(e) => f3.set_error(&e.message),
            }
        });
        false
    });
    {
        let (d, form3) = (dialog.clone(), form.clone());
        mode.connect_changed(move |v| {
            form3.widget.set_visible(v == "now");
            d.set_primary_enabled(v != "now" || form3.valid());
        });
    }
    {
        let (d, mode3) = (dialog.clone(), mode.clone());
        form.connect_changed(move |ok| d.set_primary_enabled(mode3.value().as_deref() != Some("now") || ok));
    }
    let form4 = form.clone();
    dialog.show(&p.anchor(), move |_| form4.clear());
}

/// GNOME's "%s is still logged in" (cc-user-page.c): a user with a running session is not deleted, the dialog has
/// only Annuleren (users.delete refuses it too: AccountsService's `userdel -f` would not).
fn still_signed_in(p: &Page, name: &str) {
    Dialog::new(&common::fill(t("{name} is nog aangemeld", "{name} is still signed in"), &[("name", name)]))
        .text(t(
            "Een gebruiker verwijderen terwijl die is aangemeld, kan het systeem in een inconsistente toestand achterlaten.",
            "Deleting a user while they’re signed in can leave the system in an inconsistent state.",
        ))
        .close_label(t("Annuleren", "Cancel"))
        .width(440)
        .show(&p.anchor(), |_| {});
}

/// "Account en gegevens verwijderen?" with GNOME's choice: Bestanden verwijderen or Bestanden bewaren.
fn remove_user(p: &Page, u: &Value) {
    let name = display_name(u);
    if u["loggedIn"] == true {
        still_signed_in(p, &name);
        return;
    }
    let (pp, uid) = (p.clone(), u["uid"].clone());
    Dialog::new(t("Account en gegevens verwijderen?", "Delete account and data?"))
        .text(&common::fill(
            t(
                "Het account van {name} wordt van deze pc verwijderd. Met Bestanden verwijderen gaan ook de persoonlijke map, de documenten, foto’s, muziek en andere bestanden van {name} verloren. Dit kan niet ongedaan worden gemaakt.",
                "The account of {name} will be removed from this PC. Delete files also removes the home folder, documents, photos, music and other files of {name}. This can’t be undone.",
            ),
            &[("name", &name)],
        ))
        .primary(t("Bestanden verwijderen", "Delete files"))
        .secondary(t("Bestanden bewaren", "Keep files"))
        .danger()
        .width(460)
        .show(&p.anchor(), move |r| {
            let remove_files = match r {
                DialogResult::Primary => true,
                DialogResult::Secondary => false,
                DialogResult::Close => return,
            };
            let wait = pp.run("users.delete", json!({ "uid": uid, "removeFiles": remove_files }));
            pp.spawn(async move {
                let _ = wait.await;
            });
        });
}

/// The expander of one other user, Windows' rows first, GNOME's after them.
fn user_expander(p: &Page, u: &Value, locales: &[Value], locked: bool) -> SettingsExpander {
    let uid = u["uid"].as_u64().unwrap_or(0);
    let exp = SettingsExpander::new("", &display_name(u), &user_line(u));
    exp.header().set_icon_widget(&user_avatar(u, 32));
    exp.header_button().set_widget_name(&format!("st-otherusers-user-{uid}"));
    let lock = |w: &gtk::Widget| {
        if locked {
            w.set_sensitive(false);
            w.set_tooltip_text(Some(gs::locked_text()));
        }
    };
    // Accountopties · Accounttype wijzigen
    let options = SettingsCard::new("", t("Accountopties", "Account options"), "");
    let type_btn = admin_button(t("Accounttype wijzigen", "Change account type"), Variant::Standard);
    type_btn.set_widget_name(&format!("st-otherusers-type-{uid}"));
    lock(type_btn.upcast_ref());
    {
        let (pp, u) = (p.clone(), u.clone());
        type_btn.connect_clicked(move |_| change_type(&pp, &u));
    }
    options.set_control(&type_btn);
    exp.add(options.widget());
    // GNOME: Wachtwoord (other users)
    let password = SettingsCard::new("", t("Wachtwoord", "Password"), if u["passwordMode"] == "setAtLogin" { t("Wordt ingesteld bij de volgende aanmelding", "Set at the next sign-in") } else { "" });
    let pw_btn = admin_button(t("Wachtwoord opnieuw instellen", "Reset password"), Variant::Standard);
    pw_btn.set_widget_name(&format!("st-otherusers-password-{uid}"));
    lock(pw_btn.upcast_ref());
    {
        let (pp, u) = (p.clone(), u.clone());
        pw_btn.connect_clicked(move |_| reset_password(&pp, &u));
    }
    password.set_control(&pw_btn);
    exp.add(password.widget());
    // GNOME: Automatic Login
    let auto = SettingsCard::new("", t("Automatisch aanmelden", "Automatic sign-in"), t("Deze gebruiker wordt aangemeld zonder wachtwoord wanneer de pc opstart", "This user is signed in without a password when the PC starts"));
    let sw = ToggleSwitch::new(u["automaticLogin"] == true);
    sw.switch().set_widget_name(&format!("st-otherusers-autologin-{uid}"));
    lock(sw.switch().upcast_ref());
    sw.set_sensitive(!locked);
    {
        let (pp, sw2) = (p.clone(), sw.clone());
        sw.connect_toggled(move |on| {
            let (sw3, wait) = (sw2.clone(), pp.run("users.setAutoLogin", json!({ "uid": uid, "on": on })));
            pp.spawn(async move {
                if wait.await.is_none() {
                    sw3.set_active(!on);
                }
            });
        });
    }
    auto.set_control(sw.widget());
    exp.add(auto.widget());
    // GNOME: Language (other users), once machine.locales lists the generated locales (batch E, slice 2)
    if !locales.is_empty() {
        let current = u["language"].as_str().unwrap_or("").to_owned();
        let mut opts: Vec<DropdownOption> = vec![DropdownOption::new("", t("Systeemstandaard", "System default"))];
        opts.extend(locales.iter().filter_map(|l| Some(DropdownOption::new(l["id"].as_str()?, l["name"].as_str().or(l["id"].as_str())?))));
        if !current.is_empty() && !opts.iter().any(|o| o.value == current) {
            opts.push(DropdownOption::new(&current, &current));
        }
        let dd = Dropdown::new(opts, Some(&current));
        dd.widget().set_widget_name(&format!("st-otherusers-language-{uid}"));
        lock(dd.widget().upcast_ref());
        {
            let (pp, dd2, before) = (p.clone(), dd.clone(), current.clone());
            dd.connect_changed(move |v| {
                let (dd3, before, wait) = (dd2.clone(), before.clone(), pp.run("users.setLanguage", json!({ "uid": uid, "language": v })));
                pp.spawn(async move {
                    if wait.await.is_none() {
                        dd3.set_selected(Some(&before));
                    }
                });
            });
        }
        let lang = SettingsCard::new("", t("Taal", "Language"), t("Geldt vanaf de volgende aanmelding van deze gebruiker", "Applies from this user’s next sign-in"));
        lang.set_control(dd.widget());
        exp.add(lang.widget());
    }
    // Account en gegevens · Verwijderen
    let data = SettingsCard::new("", t("Account en gegevens", "Account and data"), "");
    let remove_btn = admin_button(t("Verwijderen", "Remove"), Variant::Standard);
    remove_btn.set_widget_name(&format!("st-otherusers-remove-{uid}"));
    lock(remove_btn.upcast_ref());
    {
        let (pp, u) = (p.clone(), u.clone());
        remove_btn.connect_clicked(move |_| remove_user(&pp, &u));
    }
    data.set_control(&remove_btn);
    exp.add(data.widget());
    exp
}

fn page(p: Page) -> PageFuture {
    Box::pin(async move {
        let (list, locales) = (p.call("users.list", json!({})), p.call("machine.locales", json!({})));
        let gs = Gs::load(&p, &[gs::key("org.gnome.desktop.lockdown", "user-administration-disabled")]).await;
        let (list, locales) = (list.await, locales.await);
        if !p.alive() {
            return Content::new();
        }
        let mut out: Content = Vec::new();
        let locked = admin_locked(&gs);
        let list = match list {
            Ok(v) => v,
            Err(_) => {
                out.push(common::notice(t("Gebruikersaccounts zijn op deze computer niet beschikbaar.", "User accounts aren’t available on this computer.")).widget().clone().upcast());
                Value::Null
            }
        };
        if locked {
            out.push(common::notice(gs::locked_text()).widget().clone().upcast());
        }
        let locales: Vec<Value> = locales.ok().and_then(|v| v["locales"].as_array().cloned()).unwrap_or_default();
        let group = SettingsGroup::new(t("Andere gebruikers", "Other users"), "");
        let add = SettingsCard::new("people", t("Een andere gebruiker toevoegen", "Add other user"), "");
        let add_btn = admin_button(t("Account toevoegen", "Add account"), Variant::Standard);
        add_btn.set_widget_name("st-otherusers-add");
        add_btn.set_sensitive(list.is_object() && !locked);
        {
            let pp = p.clone();
            add_btn.connect_clicked(move |_| add_user(&pp));
        }
        add.set_control(&add_btn);
        group.add(add.widget());
        let others: Vec<Value> = list["users"].as_array().into_iter().flatten().filter(|u| u["self"] != true).cloned().collect();
        for u in &others {
            group.add(user_expander(&p, u, &locales, locked).widget());
        }
        if others.is_empty() && list.is_object() {
            group.add(SettingsCard::new("", t("Er zijn geen andere gebruikers op deze pc.", "There are no other users on this PC."), "").widget());
        }
        out.push(group.widget().clone().upcast());
        let related = SettingsGroup::new(common::ct::related(), "");
        related.add(common::link_card(&p, "signin").widget());
        out.push(related.widget().clone().upcast());
        refresh_on(&p, "users.changed");
        out
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_user_name_is_suggested_from_the_full_name() {
        assert_eq!(suggest_user_name("Anna de Vries"), "anna");
        assert_eq!(suggest_user_name("  Émile Zola"), "emile");
        assert_eq!(suggest_user_name("Jürgen"), "jurgen");
        assert_eq!(suggest_user_name("O'Brien"), "obrien");
        assert_eq!(suggest_user_name("2Pac Shakur"), "pac", "a user name starts with a letter");
        assert_eq!(suggest_user_name(""), "");
        assert_eq!(suggest_user_name(&"a".repeat(40)).len(), 32);
    }
}
