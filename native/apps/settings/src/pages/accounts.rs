//! Instellingen › Accounts (Windows 11: Accounts; batch E, notes/claude/settings-parity.md §2): who you are on this PC
//! (picture, name, "Lokaal account", the account type from AccountsService through `users.self`), the Accounts pages
//! from the registry and "Nu vergrendelen".
//!
//! The helpers of the Accounts pages live here (pub(super)): the account type's text, the user's header, the
//! password form with GNOME's strength meter (users.passwordQuality) and the refresh after `users.changed`.
use super::{Builder, Content, Page, PageFuture};
use crate::common::{self, UsageBar};
use kit::controls::{self as c, SettingsCard, SettingsGroup, Variant};
use kit::prelude::*;
use kit::t;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub const PAGES: &[(&str, Builder)] = &[("accounts", page)];
pub const CSS: &str = r#"
.st-account-hero { padding: 4px 0; }
.st-account-hero .w11-avatar { box-shadow: 0 0 0 1px var(--stroke-card); }
.st-account-name {
  font-family: "Segoe UI Variable Display", "Segoe UI Variable", "Segoe UI", "Selawik", "Inter Display", "Inter", "Noto Sans", "Ubuntu Sans", sans-serif;
  font-size: var(--fs-subtitle); font-weight: 600;
}
.st-account-line { color: var(--text-2); }
.st-pw-form entry, .st-pw-form passwordentry { min-width: 320px; }
.st-pw-hint { color: var(--text-2); font-size: var(--fs-caption); }
.st-pw-error { color: var(--sys-critical); font-size: var(--fs-caption); }
.st-face { padding: 2px; border-radius: 999px; }
.st-face:checked, .st-face.st-current { box-shadow: 0 0 0 2px var(--accent); }
"#;

// ------------------------------------------------------------------------------------------------- the helpers

/// "Administrator" / "Standaardgebruiker" (Windows' account types).
pub(super) fn account_type_text(kind: &str) -> &'static str {
    if kind == "administrator" { t("Administrator", "Administrator") } else { t("Standaardgebruiker", "Standard user") }
}

/// The name a person is shown by: the full name, else the user name.
pub(super) fn display_name(u: &Value) -> String {
    let real = u["realName"].as_str().unwrap_or("").trim();
    if real.is_empty() { u["userName"].as_str().unwrap_or("").to_owned() } else { real.to_owned() }
}

/// The lines under a name in Windows' header: "Lokaal account", the account type.
pub(super) fn account_lines(u: &Value) -> Vec<String> {
    vec![t("Lokaal account", "Local account").to_owned(), account_type_text(u["accountType"].as_str().unwrap_or("")).to_owned()]
}

/// The account picture (or the initials) of a user.
pub(super) fn user_avatar(u: &Value, size: i32) -> gtk::Widget {
    common::avatar(&display_name(u), u["iconFile"].as_str().filter(|s| !s.is_empty()), size)
}

/// The header of Accounts and Je info: picture, name and the account lines.
pub(super) fn user_hero(u: &Value, size: i32) -> gtk::Box {
    let hero = gtk::Box::new(gtk::Orientation::Horizontal, 24);
    hero.add_css_class("st-account-hero");
    let avatar = user_avatar(u, size);
    avatar.set_valign(gtk::Align::Center);
    hero.append(&avatar);
    let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
    text.set_valign(gtk::Align::Center);
    text.set_hexpand(true);
    let name = c::label(&display_name(u), "st-account-name", true);
    name.set_widget_name("st-account-name");
    text.append(&name);
    for line in account_lines(u) {
        text.append(&c::label(&line, "st-account-line", true));
    }
    hero.append(&text);
    hero
}

/// Build the page again shortly after `event` (users.changed, goa.changed): a burst of them is one rebuild.
pub(super) fn refresh_on(p: &Page, event: &str) {
    let pending = Rc::new(Cell::new(false));
    let pp = p.clone();
    p.on(event, move |_| {
        if pending.replace(true) {
            return;
        }
        let (pending, p2) = (pending.clone(), pp.clone());
        pp.timeout(300, move || {
            pending.set(false);
            p2.refresh();
        });
    });
}

/// The account settings are switched off by the organisation (lockdown `user-administration-disabled`), as GNOME
/// Settings' Users panel honours it.
pub(super) fn admin_locked(gs: &crate::gs::Gs) -> bool {
    gs.value(&crate::gs::key("org.gnome.desktop.lockdown", "user-administration-disabled")) == true
}

/// A password entry in Windows' text box style.
fn password_entry(placeholder: &str, name: &str) -> gtk::PasswordEntry {
    let e = gtk::PasswordEntry::new();
    e.set_show_peek_icon(true);
    e.add_css_class("w11-entry");
    e.set_widget_name(name);
    e.set_hexpand(true);
    if !placeholder.is_empty() {
        e.set_placeholder_text(Some(placeholder));
    }
    e
}

/// The strength bar's colour for GNOME's levels 0-5.
pub(super) fn strength_color(level: u64) -> &'static str {
    match level {
        0 | 1 => "sys-critical",
        2 => "sys-caution",
        _ => "sys-success",
    }
}

/// GNOME's rule for the password dialogs' button: the new password is typed twice alike and its level is above 1
/// (libpwquality's error levels block it); a current password when one is asked for.
pub(super) fn password_ok(current: Option<&str>, new: &str, confirm: &str, level: u64) -> bool {
    current.is_none_or(|c| !c.is_empty()) && !new.is_empty() && new == confirm && level > 1
}

/// The password fields of the Accounts dialogs (Windows: Huidig wachtwoord, Nieuw wachtwoord, Wachtwoord bevestigen)
/// with GNOME's strength meter and "Een wachtwoord genereren".
pub(super) struct PasswordForm {
    pub widget: gtk::Box,
    current: Option<gtk::PasswordEntry>,
    new: gtk::PasswordEntry,
    confirm: gtk::PasswordEntry,
    error: gtk::Label,
    level: Cell<u64>,
    changed: RefCell<Vec<Rc<dyn Fn(bool)>>>,
    seq: Cell<u64>,
}

impl PasswordForm {
    /// `with_current`: the own password's dialog; `user_name` goes to libpwquality's user check.
    pub(super) fn new(p: &Page, with_current: bool, user_name: Option<String>, prefix: &str) -> Rc<PasswordForm> {
        let widget = gtk::Box::new(gtk::Orientation::Vertical, 12);
        widget.add_css_class("st-pw-form");
        let current = with_current.then(|| password_entry("", &format!("{prefix}-current")));
        if let Some(cur) = &current {
            widget.append(&c::field(t("Huidig wachtwoord", "Current password"), cur, None));
        }
        let new = password_entry("", &format!("{prefix}-new"));
        let confirm = password_entry("", &format!("{prefix}-confirm"));
        widget.append(&c::field(t("Nieuw wachtwoord", "New password"), &new, None));
        let bar = UsageBar::new(0.0, Some("sys-critical"), 101.0);
        bar.widget().set_widget_name(&format!("{prefix}-strength"));
        let hint = c::label("", "st-pw-hint", true);
        hint.set_widget_name(&format!("{prefix}-hint"));
        widget.append(bar.widget());
        widget.append(&hint);
        widget.append(&c::field(t("Wachtwoord bevestigen", "Confirm password"), &confirm, None));
        let generate = c::hyperlink(t("Een wachtwoord genereren", "Generate a password"), None);
        generate.set_widget_name(&format!("{prefix}-generate"));
        generate.set_halign(gtk::Align::Start);
        widget.append(&generate);
        let error = c::label("", "st-pw-error", true);
        error.set_visible(false);
        error.set_widget_name(&format!("{prefix}-error"));
        widget.append(&error);
        let form = Rc::new(PasswordForm { widget, current, new, confirm, error, level: Cell::new(0), changed: RefCell::new(Vec::new()), seq: Cell::new(0) });
        // the strength of the new password, asked 250 ms after the last key (the answers of older ones are dropped)
        let check = {
            let (weak, pp, bar, hint) = (Rc::downgrade(&form), p.clone(), bar.clone(), hint.clone());
            move || {
                let Some(f) = weak.upgrade() else { return };
                let seq = f.seq.get() + 1;
                f.seq.set(seq);
                let password = f.new.text().to_string();
                let old = f.current.as_ref().map(|c| c.text().to_string());
                let mut args = json!({ "password": password });
                if let Some(old) = old.filter(|o| !o.is_empty()) {
                    args["old"] = json!(old);
                }
                if let Some(u) = &user_name {
                    args["userName"] = json!(u);
                }
                let (weak, pp2, bar, hint) = (Rc::downgrade(&f), pp.clone(), bar.clone(), hint.clone());
                // not the page's timeout: a page rebuilt meanwhile (users.changed) keeps its open dialog working
                glib::timeout_add_local_once(std::time::Duration::from_millis(250), move || {
                    let Some(f) = weak.upgrade() else { return };
                    if f.seq.get() != seq {
                        return;
                    }
                    let wait = pp2.call("users.passwordQuality", args);
                    pp2.spawn(async move {
                        let answer = wait.await.unwrap_or(Value::Null);
                        if f.seq.get() != seq {
                            return;
                        }
                        let level = answer["level"].as_u64().unwrap_or(if f.new.text().is_empty() { 0 } else { 3 });
                        f.level.set(level);
                        bar.set(level as f64 * 20.0, Some(strength_color(level)));
                        hint.set_label(answer["hint"].as_str().unwrap_or(""));
                        f.emit();
                    });
                });
            }
        };
        let check: Rc<dyn Fn()> = Rc::new(check);
        {
            let check = check.clone();
            form.new.connect_changed(move |_| check());
        }
        if let Some(cur) = &form.current {
            let check = check.clone();
            cur.connect_changed(move |_| check());
        }
        {
            let weak = Rc::downgrade(&form);
            form.confirm.connect_changed(move |_| {
                if let Some(f) = weak.upgrade() {
                    f.emit();
                }
            });
        }
        {
            let (weak, pp) = (Rc::downgrade(&form), p.clone());
            generate.connect_clicked(move |_| {
                let (weak, wait) = (weak.clone(), pp.call("users.passwordQuality", json!({ "generate": true })));
                pp.spawn(async move {
                    let Some(f) = weak.upgrade() else { return };
                    if let Ok(v) = wait.await
                        && let Some(pw) = v["password"].as_str()
                    {
                        // a generated password is shown, so it can be written down (GNOME does the same)
                        f.new.set_show_peek_icon(true);
                        f.new.set_text(pw);
                        f.confirm.set_text(pw);
                    }
                });
            });
        }
        form
    }

    fn emit(&self) {
        let ok = self.valid();
        let list: Vec<Rc<dyn Fn(bool)>> = self.changed.borrow().clone();
        for f in list {
            f(ok);
        }
    }

    /// Run `f(valid)` whenever the fields or the strength change.
    pub(super) fn connect_changed(&self, f: impl Fn(bool) + 'static) {
        self.changed.borrow_mut().push(Rc::new(f));
    }

    pub(super) fn valid(&self) -> bool {
        let current = self.current.as_ref().map(|c| c.text().to_string());
        password_ok(current.as_deref(), &self.new.text(), &self.confirm.text(), self.level.get())
    }

    pub(super) fn current(&self) -> String {
        self.current.as_ref().map(|c| c.text().to_string()).unwrap_or_default()
    }

    pub(super) fn password(&self) -> String {
        self.new.text().to_string()
    }

    /// Show an error under the fields (empty: none).
    pub(super) fn set_error(&self, text: &str) {
        self.error.set_label(text);
        self.error.set_visible(!text.is_empty());
    }

    /// Empty the fields (the passwords do not stay in the widgets after the dialog).
    pub(super) fn clear(&self) {
        if let Some(c) = &self.current {
            c.set_text("");
        }
        self.new.set_text("");
        self.confirm.set_text("");
    }
}

// ----------------------------------------------------------------------------------------------------- the page

fn page(p: Page) -> PageFuture {
    Box::pin(async move {
        let me = p.get("users.self", json!({})).await;
        if !p.alive() {
            return Content::new();
        }
        // AccountsService answers; without it the shell's own user (system.info) as before
        let u = match me {
            Some(u) if u["available"] == true => u,
            other => {
                let sh = common::shell_info();
                let shell = &sh["user"];
                json!({
                    "realName": shell["realName"].as_str().or(other.as_ref().and_then(|o| o["realName"].as_str())).unwrap_or(""),
                    "userName": shell["name"].as_str().or(other.as_ref().and_then(|o| o["userName"].as_str())).unwrap_or(""),
                    "iconFile": shell["avatar"].as_str().unwrap_or(""),
                    "accountType": other.as_ref().and_then(|o| o["accountType"].as_str()).unwrap_or("standard"),
                })
            }
        };
        let hero = user_hero(&u, 96);
        hero.set_widget_name("st-accounts-hero");

        // Windows: Accountinstellingen (Je info, E-mail en accounts, Aanmeldingsopties, Andere gebruikers)
        let links = common::child_links(&p, "accounts");
        links.set_title(t("Accountinstellingen", "Account settings"));

        let lock_group = SettingsGroup::new("", "");
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
        lock_group.add(lock.widget());

        let privacy = SettingsGroup::new("", "");
        privacy.add(
            SettingsCard::new(
                "shield",
                t(
                    "Je account blijft op deze computer. WinPoint stuurt geen gegevens over jou naar internet; alleen de dagelijkse updatecontrole vraagt GitHub naar de nieuwste versie van WinPoint.",
                    "Your account stays on this computer. WinPoint doesn’t send any data about you to the internet; only the daily update check asks GitHub for WinPoint’s latest version.",
                ),
                "",
            )
            .widget(),
        );
        refresh_on(&p, "users.changed");
        vec![hero.upcast(), links.widget().clone().upcast(), lock_group.widget().clone().upcast(), privacy.widget().clone().upcast()]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_password_button_follows_gnomes_rule() {
        assert!(password_ok(None, "Lang genoeg 42!", "Lang genoeg 42!", 3));
        assert!(!password_ok(None, "Lang genoeg 42!", "Lang genoeg 43!", 3), "typed twice alike");
        assert!(!password_ok(None, "abc", "abc", 1), "libpwquality refused it");
        assert!(!password_ok(Some(""), "Lang genoeg 42!", "Lang genoeg 42!", 4), "the current password is needed");
        assert!(password_ok(Some("oud"), "Lang genoeg 42!", "Lang genoeg 42!", 2));
        assert!(!password_ok(None, "", "", 0));
        assert_eq!(strength_color(1), "sys-critical");
        assert_eq!(strength_color(2), "sys-caution");
        assert_eq!(strength_color(5), "sys-success");
    }

    #[test]
    fn a_person_is_shown_by_the_full_name_else_the_user_name() {
        assert_eq!(display_name(&json!({"realName": " Anna de Vries ", "userName": "anna"})), "Anna de Vries");
        assert_eq!(display_name(&json!({"realName": "", "userName": "anna"})), "anna");
        assert_eq!(display_name(&json!({})), "");
    }
}
