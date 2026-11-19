//! Instellingen › Accounts › Aanmeldingsopties (Windows 11: Accounts › Sign-in options; batch E,
//! notes/claude/settings-parity.md §2). Windows' two sections:
//!
//! * Manieren om je aan te melden: Vingerafdrukherkenning (fprintd through `users.fingerprint.*`, shown with a reader
//!   as GNOME does: `caps.fingerprint`) and Wachtwoord ("Wijzigen": passwd through `users.changePassword`, GNOME's
//!   strength meter). Windows Hello's face, PIN and security key have no GNOME counterpart and are left out.
//! * Aanvullende instellingen (amendment 23): "Als je weg bent geweest, wanneer moet WinPoint vereisen dat je je
//!   opnieuw aanmeldt?" is GNOME's Automatic Screen Lock and its delay (`lock-enabled` + `lock-delay`, one write),
//!   "Ook vergrendelen wanneer de pc in slaapstand gaat" Ubuntu's `ubuntu-lock-on-suspend`, "Nieuwe USB-apparaten
//!   blokkeren" `usb-protection` (only with usbguard, as GNOME), and GNOME's Automatic Login (`users.setAutoLogin`) as a
//!   WinPoint addition.
use super::accounts::{PasswordForm, refresh_on};
use super::{Builder, Content, Page, PageFuture};
use crate::common;
use crate::gs::{self, GKey, Gs};
use kit::controls::{self as c, Dialog, DialogResult, Dropdown, DropdownOption, ProgressBar, SettingsCard, SettingsExpander, SettingsGroup, Variant};
use kit::prelude::*;
use kit::t;
use std::cell::Cell;
use std::rc::Rc;

pub const PAGES: &[(&str, Builder)] = &[("signin", page)];
pub const CSS: &str = "";

const SCREENSAVER: &str = "org.gnome.desktop.screensaver";
const LOCK_ENABLED: GKey = gs::key(SCREENSAVER, "lock-enabled");
const LOCK_DELAY: GKey = gs::key(SCREENSAVER, "lock-delay");
const LOCK_ON_SUSPEND: GKey = gs::key(SCREENSAVER, "ubuntu-lock-on-suspend");
const USB_PROTECTION: GKey = gs::key("org.gnome.desktop.privacy", "usb-protection");
const NO_LOCK_SCREEN: GKey = gs::key("org.gnome.desktop.lockdown", "disable-lock-screen");
const NO_USER_ADMIN: GKey = gs::key("org.gnome.desktop.lockdown", "user-administration-disabled");

/// GNOME's lock delays (Privacy › Screen Lock: Screen Turns Off, 30 s, 1, 2, 3, 5, 30 minutes, 1 hour).
pub(crate) const LOCK_DELAYS: &[u64] = &[0, 30, 60, 120, 180, 300, 1800, 3600];

/// The dropdown's value of GNOME's two keys: "never" while automatic locking is off, else the delay.
pub(crate) fn lock_choice(enabled: bool, delay: u64) -> String {
    if enabled { delay.to_string() } else { "never".into() }
}

/// What a choice writes: `lock-enabled`, and the delay unless it is "never" (GNOME keeps the delay then).
pub(crate) fn lock_values(choice: &str) -> Option<Value> {
    if choice == "never" {
        return Some(json!({ "lock-enabled": false }));
    }
    let delay: u64 = choice.parse().ok()?;
    Some(json!({ "lock-enabled": true, "lock-delay": delay }))
}

fn delay_label(seconds: u64) -> String {
    match seconds {
        0 => t("Wanneer het scherm wordt uitgeschakeld", "When the screen turns off").into(),
        s if s % 60 != 0 => common::fill(t("Na {n} seconden", "After {n} seconds"), &[("n", &s.to_string())]),
        3600 => t("Na 1 uur", "After 1 hour").into(),
        60 => t("Na 1 minuut", "After 1 minute").into(),
        s => common::fill(t("Na {n} minuten", "After {n} minutes"), &[("n", &(s / 60).to_string())]),
    }
}

/// The options: GNOME's delays, a current value that is none of them, and Nooit.
pub(crate) fn lock_options(current: &str) -> Vec<(String, String)> {
    let mut values: Vec<u64> = LOCK_DELAYS.to_vec();
    if let Ok(cur) = current.parse::<u64>()
        && !values.contains(&cur)
    {
        values.push(cur);
        values.sort();
    }
    let mut out: Vec<(String, String)> = values.into_iter().map(|v| (v.to_string(), delay_label(v))).collect();
    out.push(("never".into(), t("Nooit", "Never").into()));
    out
}

/// The lock question's dropdown, bound to both keys (one gsettings.set, optimistic, following later changes).
fn lock_dropdown(p: &Page, gs: &Gs) -> Dropdown {
    let current = || lock_choice(gs.value(&LOCK_ENABLED) != false, gs.value(&LOCK_DELAY).as_u64().unwrap_or(0));
    let now = current();
    let options = |cur: &str| lock_options(cur).into_iter().map(|(v, l)| DropdownOption::new(&v, &l)).collect::<Vec<_>>();
    let dd = Dropdown::new(options(&now), Some(&now));
    dd.widget().set_widget_name("st-signin-lock-delay");
    let writable = gs.writable(&LOCK_ENABLED) && gs.writable(&LOCK_DELAY) && gs.value(&NO_LOCK_SCREEN) != true;
    dd.widget().set_sensitive(writable);
    if !writable {
        dd.widget().set_tooltip_text(Some(gs::locked_text()));
    }
    {
        let (pp, gs2, dd2) = (p.clone(), gs.clone(), dd.clone());
        dd.connect_changed(move |choice| {
            let Some(values) = lock_values(choice) else { return };
            let wait = pp.run_with("gsettings.set", json!({ "schema": SCREENSAVER, "values": values }), None, false);
            let (gs3, dd3) = (gs2.clone(), dd2.clone());
            pp.spawn(async move {
                if wait.await.is_none() {
                    let back = lock_choice(gs3.value(&LOCK_ENABLED) != false, gs3.value(&LOCK_DELAY).as_u64().unwrap_or(0));
                    dd3.set_selected(Some(&back));
                }
            });
        });
    }
    // a change made elsewhere (GNOME Settings, gsettings) moves the dropdown along
    for k in [&LOCK_ENABLED, &LOCK_DELAY] {
        let (gs2, dd2) = (gs.clone(), dd.clone());
        gs.follow(k, move |_| {
            let cur = lock_choice(gs2.value(&LOCK_ENABLED) != false, gs2.value(&LOCK_DELAY).as_u64().unwrap_or(0));
            dd2.set_options(lock_options(&cur).into_iter().map(|(v, l)| DropdownOption::new(&v, &l)).collect());
            dd2.set_selected(Some(&cur));
        });
    }
    dd
}

/// "Je wachtwoord wijzigen" (Windows' dialog; passwd answers through users.changePassword).
fn change_password(p: &Page, user_name: Option<String>) {
    let form = PasswordForm::new(p, true, user_name, "st-signin-password");
    let pp = p.clone();
    let form2 = form.clone();
    let dialog = Dialog::new(t("Je wachtwoord wijzigen", "Change your password")).body(&form.widget).primary(t("Wijzigen", "Change")).width(440).on_primary(move |d| {
        if !form2.valid() {
            return false;
        }
        form2.set_error("");
        d.set_busy(true);
        let wait = pp.call("users.changePassword", json!({ "current": form2.current(), "new": form2.password() }));
        let (d2, f3, p2) = (d.clone(), form2.clone(), pp.clone());
        pp.spawn(async move {
            let answer = wait.await;
            d2.set_busy(false);
            match answer {
                Ok(v) => {
                    f3.clear();
                    d2.close(DialogResult::Primary);
                    if v["dryRun"] == true {
                        common::note_dry_run(&p2.anchor(), &v);
                    } else {
                        common::toast_success(&p2.anchor(), t("Je wachtwoord is gewijzigd", "Your password has been changed"), 4000);
                    }
                }
                Err(e) => {
                    f3.set_error(&e.message);
                    d2.set_primary_enabled(f3.valid());
                }
            }
        });
        false
    });
    {
        let d = dialog.clone();
        form.connect_changed(move |ok| d.set_primary_enabled(ok));
    }
    let form3 = form.clone();
    dialog.show(&p.anchor(), move |_| form3.clear());
    dialog.set_primary_enabled(false);
}

/// fprintd's finger names (host users/fprint.rs FINGERS) as GNOME Settings names them: (id, nl, en).
const FINGERS: &[(&str, &str, &str)] = &[
    ("right-index-finger", "Rechterwijsvinger", "Right index finger"),
    ("left-index-finger", "Linkerwijsvinger", "Left index finger"),
    ("right-thumb", "Rechterduim", "Right thumb"),
    ("right-middle-finger", "Rechtermiddelvinger", "Right middle finger"),
    ("right-ring-finger", "Rechterringvinger", "Right ring finger"),
    ("right-little-finger", "Rechterpink", "Right little finger"),
    ("left-thumb", "Linkerduim", "Left thumb"),
    ("left-middle-finger", "Linkermiddelvinger", "Left middle finger"),
    ("left-ring-finger", "Linkerringvinger", "Left ring finger"),
    ("left-little-finger", "Linkerpink", "Left little finger"),
];

/// What the reader asks for after an EnrollStatus result (fprintd's enroll-* results).
pub(crate) fn enroll_text(result: &str, swipe: bool) -> &'static str {
    match result {
        "enroll-completed" => t("Je vingerafdruk is ingesteld.", "Your fingerprint has been set up."),
        "enroll-stage-passed" if swipe => t("Veeg nog een keer met dezelfde vinger over de lezer.", "Swipe the same finger over the reader again."),
        "enroll-stage-passed" => t("Til je vinger op en leg hem opnieuw op de lezer.", "Lift your finger and place it on the reader again."),
        "enroll-swipe-too-short" => t("Je veegde te kort. Probeer het opnieuw.", "The swipe was too short. Try again."),
        "enroll-finger-not-centered" => t("Leg je vinger in het midden van de lezer.", "Place your finger in the middle of the reader."),
        "enroll-remove-and-retry" | "enroll-retry-scan" => t("Til je vinger op en probeer het opnieuw.", "Lift your finger and try again."),
        "enroll-duplicate" => t("Deze vingerafdruk is al ingesteld.", "This fingerprint is already set up."),
        "enroll-data-full" => t("De lezer heeft geen ruimte meer voor vingerafdrukken.", "The reader has no room for more fingerprints."),
        "enroll-disconnected" => t("De vingerafdruklezer is losgekoppeld.", "The fingerprint reader was disconnected."),
        _ if swipe => t("Veeg met je vinger over de lezer.", "Swipe your finger over the reader."),
        _ => t("Leg je vinger op de vingerafdruklezer.", "Place your finger on the fingerprint reader."),
    }
}

/// Where an enrolment is after an EnrollStatus: still running, set up, or ended without a fingerprint (enroll-duplicate,
/// -data-full, -disconnected, -failed), when GNOME's dialog lets you try again or pick another finger.
#[derive(Debug, PartialEq)]
pub(crate) enum EnrollStep {
    Running,
    Completed,
    Retry,
}

pub(crate) fn enroll_step(result: &str, done: bool) -> EnrollStep {
    match (done, result) {
        (false, _) => EnrollStep::Running,
        (true, "enroll-completed") => EnrollStep::Completed,
        (true, _) => EnrollStep::Retry,
    }
}

/// "Vingerafdrukherkenning instellen": the finger, then the stages with the reader's prompts.
fn enroll(p: &Page, info: &Value) {
    let swipe = info["device"]["scanType"] == "swipe";
    let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let finger = Dropdown::new(FINGERS.iter().map(|(id, nl, en)| DropdownOption::new(id, t(nl, en))).collect(), Some(FINGERS[0].0));
    finger.widget().set_widget_name("st-signin-finger");
    body.append(&c::field(t("Vinger", "Finger"), finger.widget(), None));
    let prompt = c::label(enroll_text("", swipe), "w11-body", true);
    prompt.set_widget_name("st-signin-enroll-prompt");
    let bar = ProgressBar::new();
    bar.set_fraction(0.0);
    body.append(&prompt);
    body.append(bar.widget());
    let running = Rc::new(Cell::new(false));
    let pp = p.clone();
    let (finger2, running2, prompt2, bar2) = (finger.clone(), running.clone(), prompt.clone(), bar.clone());
    let dialog = Dialog::new(t("Vingerafdrukherkenning instellen", "Set up fingerprint recognition")).body(&body).primary(t("Beginnen", "Start")).width(440);
    let d_for_events = dialog.clone();
    let dialog = dialog.on_primary(move |d| {
        if running2.get() {
            return false;
        }
        let f = finger2.selected().unwrap_or_else(|| FINGERS[0].0.to_owned());
        let wait = pp.run("users.fingerprint.enroll", json!({ "finger": f }));
        let (d2, running3, finger3) = (d.clone(), running2.clone(), finger2.clone());
        running2.set(true);
        d.set_primary_enabled(false);
        finger2.widget().set_sensitive(false);
        pp.spawn(async move {
            if wait.await.is_none() {
                running3.set(false);
                finger3.widget().set_sensitive(true);
                d2.set_primary_enabled(true);
            }
        });
        false
    });
    // the dialog's own listener, not the page's: the end of an enrolment sends users.changed, which builds the page
    // again (and would stop a page handler) while the dialog stays open for another try
    let (running5, finger5) = (running.clone(), finger.clone());
    let listener = kit::ctx().on("users.fingerprint.progress", move |_, data| {
        let stages = data["stages"].as_f64().unwrap_or(5.0).max(1.0);
        let result = data["result"].as_str().unwrap_or("");
        bar2.set_fraction((data["stage"].as_f64().unwrap_or(0.0) / stages).clamp(0.0, 1.0));
        prompt2.set_label(enroll_text(result, swipe));
        match enroll_step(result, data["done"] == true) {
            EnrollStep::Running => {}
            EnrollStep::Completed => d_for_events.close(DialogResult::Primary),
            // GNOME's dialog: try again, or with another finger
            EnrollStep::Retry => {
                running5.set(false);
                bar2.set_fraction(0.0);
                finger5.widget().set_sensitive(true);
                d_for_events.set_primary_enabled(true);
                d_for_events.set_primary_label(t("Opnieuw proberen", "Try again"));
            }
        }
    });
    let (pp, running4) = (p.clone(), running.clone());
    dialog.show(&p.anchor(), move |_| {
        kit::ctx().off(listener);
        // closed while the reader still waits: stop the enrolment and release the reader
        if running4.get() {
            let wait = pp.call("users.fingerprint.cancel", json!({}));
            pp.spawn(async move {
                let _ = wait.await;
            });
        }
    });
}

fn delete_fingerprints(p: &Page) {
    let pp = p.clone();
    c::confirm(
        &p.anchor(),
        t("Vingerafdrukken verwijderen?", "Remove fingerprints?"),
        t("Je kunt je dan niet meer met je vingerafdruk aanmelden.", "You will no longer be able to sign in with your fingerprint."),
        t("Verwijderen", "Remove"),
        true,
        move |ok| {
            if ok {
                let wait = pp.run("users.fingerprint.delete", json!({}));
                pp.spawn(async move {
                    let _ = wait.await;
                });
            }
        },
    );
}

fn page(p: Page) -> PageFuture {
    Box::pin(async move {
        let keys = [LOCK_ENABLED, LOCK_DELAY, LOCK_ON_SUSPEND, USB_PROTECTION, NO_LOCK_SCREEN, NO_USER_ADMIN];
        // this page's caps (cached, or asked now on a cold start): a reader and usbguard hide no page of their own
        let caps = common::caps_now(&p).await;
        let want_fp = caps["fingerprint"] != false;
        let (me, fp) = (p.call("users.self", json!({})), want_fp.then(|| p.call("users.fingerprint.get", json!({}))));
        let gs = Gs::load(&p, &keys).await;
        let me = me.await.unwrap_or(Value::Null);
        let fp = match fp {
            Some(w) => w.await.unwrap_or(Value::Null),
            None => Value::Null,
        };
        if !p.alive() {
            return Content::new();
        }
        let mut out: Content = Vec::new();

        // ---- Manieren om je aan te melden
        let ways = SettingsGroup::new(t("Manieren om je aan te melden", "Ways to sign in"), "");
        if fp["available"] == true {
            let enrolled = fp["enrolled"].as_array().map(Vec::len).unwrap_or(0);
            let exp = SettingsExpander::new("shield", t("Vingerafdrukherkenning", "Fingerprint recognition"), t("Aanmelden met je vingerafdruklezer", "Sign in with your fingerprint reader"));
            exp.header_button().set_widget_name("st-signin-fingerprint");
            let state = if enrolled == 0 {
                t("Er is nog geen vingerafdruk ingesteld.", "No fingerprint has been set up yet.").to_owned()
            } else {
                common::plural(enrolled as i64, t("Er is {n} vingerafdruk ingesteld.", "{n} fingerprint has been set up."), t("Er zijn {n} vingerafdrukken ingesteld.", "{n} fingerprints have been set up."))
            };
            let setup = SettingsCard::new("", &state, fp["device"]["name"].as_str().unwrap_or(""));
            let setup_btn = c::button(if enrolled == 0 { t("Instellen", "Set up") } else { t("Nog een vinger toevoegen", "Add another finger") }, Variant::Standard);
            setup_btn.set_widget_name("st-signin-fingerprint-setup");
            {
                let (pp, info) = (p.clone(), fp.clone());
                setup_btn.connect_clicked(move |_| enroll(&pp, &info));
            }
            setup.set_control(&setup_btn);
            exp.add(setup.widget());
            if enrolled > 0 {
                let remove = SettingsCard::new("", t("Vingerafdrukken verwijderen", "Remove fingerprints"), "");
                let remove_btn = c::button(t("Verwijderen", "Remove"), Variant::Standard);
                remove_btn.set_widget_name("st-signin-fingerprint-remove");
                {
                    let pp = p.clone();
                    remove_btn.connect_clicked(move |_| delete_fingerprints(&pp));
                }
                remove.set_control(&remove_btn);
                exp.add(remove.widget());
            }
            ways.add(exp.widget());
        }
        let password = SettingsExpander::new("key", t("Wachtwoord", "Password"), t("Aanmelden met het wachtwoord van je account", "Sign in with your account password"));
        password.header_button().set_widget_name("st-signin-password");
        let change = SettingsCard::new("", t("Je kunt je aanmelden met het wachtwoord van je account.", "You can sign in with your account password."), "");
        let change_btn = c::button(t("Wijzigen", "Change"), Variant::Standard);
        change_btn.set_widget_name("st-signin-password-change");
        change_btn.set_sensitive(me["available"] != false);
        {
            let (pp, user) = (p.clone(), me["userName"].as_str().map(str::to_owned));
            change_btn.connect_clicked(move |_| change_password(&pp, user.clone()));
        }
        change.set_control(&change_btn);
        password.add(change.widget());
        ways.add(password.widget());
        out.push(ways.widget().clone().upcast());

        // ---- Aanvullende instellingen
        let more = SettingsGroup::new(t("Aanvullende instellingen", "Additional settings"), "");
        if gs.available(&LOCK_ENABLED) && gs.available(&LOCK_DELAY) {
            let card = SettingsCard::new(
                "lock",
                t("Als je weg bent geweest, wanneer moet WinPoint vereisen dat je je opnieuw aanmeldt?", "If you’ve been away, when should WinPoint require you to sign in again?"),
                "",
            );
            card.set_control(lock_dropdown(&p, &gs).widget());
            if gs.available(&LOCK_ON_SUSPEND) {
                let cb = gs.checkbox(&LOCK_ON_SUSPEND, t("Ook vergrendelen wanneer de pc in slaapstand gaat", "Also lock when the PC goes to sleep"), false);
                cb.set_margin_top(6);
                if gs.value(&NO_LOCK_SCREEN) == true {
                    cb.set_sensitive(false);
                }
                common::card_extra(&card, &cb);
            }
            more.add(card.widget());
        }
        if caps["usbguard"] == true
            && let Some(card) = gs.toggle_card(&USB_PROTECTION, "usb", t("Nieuwe USB-apparaten blokkeren wanneer de pc is vergrendeld", "Block new USB devices while the PC is locked"), t("Nieuwe USB-apparaten werken pas nadat je je hebt aangemeld", "New USB devices only work after you sign in"), false)
        {
            more.add(card.widget());
        }
        // WinPoint addition (Windows keeps it in netplwiz): GNOME's Automatic Login of the own account
        let auto = SettingsCard::new("user", t("Automatisch aanmelden", "Automatic sign-in"), t("Aanmelden zonder wachtwoord wanneer de pc opstart", "Sign in without a password when the PC starts"));
        let sw = c::ToggleSwitch::new(me["automaticLogin"] == true);
        sw.switch().set_widget_name("st-signin-automatic-login");
        sw.switch().set_tooltip_text(Some(t("Hiervoor is een beheerderswachtwoord nodig.", "This needs an administrator password.")));
        let auto_ok = me["available"] == true && gs.value(&NO_USER_ADMIN) != true;
        sw.set_sensitive(auto_ok);
        {
            let (pp, sw2) = (p.clone(), sw.clone());
            sw.connect_toggled(move |on| {
                let (sw3, wait) = (sw2.clone(), pp.run("users.setAutoLogin", json!({ "on": on })));
                pp.spawn(async move {
                    if wait.await.is_none() {
                        sw3.set_active(!on);
                    }
                });
            });
        }
        auto.set_control(sw.widget());
        more.add(auto.widget());
        out.push(more.widget().clone().upcast());

        let related = SettingsGroup::new(common::ct::related(), "");
        related.add(common::link_card_with(&p, "power", None, Some(t("Wanneer het scherm wordt uitgeschakeld en de pc in slaapstand gaat", "When the screen turns off and the PC goes to sleep")), None).widget());
        related.add(common::link_card(&p, "otherusers").widget());
        out.push(related.widget().clone().upcast());
        refresh_on(&p, "users.changed");
        out
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lock_question_maps_to_gnomes_two_keys() {
        assert_eq!(lock_choice(true, 0), "0");
        assert_eq!(lock_choice(true, 300), "300");
        assert_eq!(lock_choice(false, 300), "never", "the delay stays as it is");
        assert_eq!(lock_values("never"), Some(json!({ "lock-enabled": false })));
        assert_eq!(lock_values("1800"), Some(json!({ "lock-enabled": true, "lock-delay": 1800 })));
        assert_eq!(lock_values("x"), None);
        let keys: Vec<String> = lock_options("300").into_iter().map(|(v, _)| v).collect();
        assert_eq!(keys, ["0", "30", "60", "120", "180", "300", "1800", "3600", "never"]);
        let custom: Vec<String> = lock_options("90").into_iter().map(|(v, _)| v).collect();
        assert_eq!(custom, ["0", "30", "60", "90", "120", "180", "300", "1800", "3600", "never"], "a value of its own stays choosable");
        assert_eq!(lock_options("never").len(), LOCK_DELAYS.len() + 1);
    }

    #[test]
    fn an_enrolment_that_ends_without_a_fingerprint_can_be_tried_again() {
        assert_eq!(enroll_step("enroll-stage-passed", false), EnrollStep::Running);
        assert_eq!(enroll_step("enroll-retry-scan", false), EnrollStep::Running);
        assert_eq!(enroll_step("enroll-completed", true), EnrollStep::Completed);
        for end in ["enroll-duplicate", "enroll-data-full", "enroll-disconnected", "enroll-failed", "enroll-unknown-error"] {
            assert_eq!(enroll_step(end, true), EnrollStep::Retry, "{end}");
        }
    }

    #[test]
    fn enrolment_texts_follow_the_reader() {
        assert_ne!(enroll_text("enroll-stage-passed", true), enroll_text("enroll-stage-passed", false), "swipe and press readers");
        assert_eq!(enroll_text("enroll-retry-scan", false), enroll_text("enroll-remove-and-retry", false));
        assert_eq!(FINGERS.len(), 10);
        assert_eq!(FINGERS[0].0, "right-index-finger", "GNOME's first choice");
        // the host's fprintd names (host/src/services/users/fprint.rs FINGERS), in the same order
        let host = include_str!("../../../../../host/src/services/users/fprint.rs");
        let names = super::super::email_accounts::tests::quoted_after(host, "pub(crate) const FINGERS");
        let ours: Vec<&str> = FINGERS.iter().map(|(id, _, _)| *id).collect();
        assert_eq!(names, ours);
    }
}
