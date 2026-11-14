//! Instellingen › Update en herstel › Geavanceerde opties
//! (`git show last-webkit:vendor/ui/js/apps/settings/pages/update-advanced.js`): Windows 11 24H2's Advanced options.
//! The toggle rows first, without icons (downloads over metered connections and the restart notification, settings
//! update.*), then Ubuntu's "Software-updater van Ubuntu automatisch openen voor gewone updates" (update.setAutoOpen;
//! hidden without update-notifier's schema), and under "Aanvullende opties" the automatic security updates and Snap
//! Store rows (read-only, update.options) and the Software Updater ("Openen"). The page is there at once: the rows that
//! wait for update.options say "Even geduld…" until it answers.
use super::update_release::{err_message, reply};
use super::{Builder, Content, Page, PageFuture};
use crate::common::{self, ct};
use kit::controls::{self as c, Dialog, DialogResult, SettingsCard, SettingsGroup, ToggleSwitch};
use kit::format::DateFormat;
use kit::prelude::*;
use kit::t;
use std::cell::RefCell;
use std::rc::Rc;

pub const PAGES: &[(&str, Builder)] = &[("update-advanced", update_advanced)];
pub const CSS: &str = "";

/// "7 uur geleden (25-10-2026 07:35)", as the Update page writes {when}; "–" when snapd has no time.
fn when_text(at: &Value) -> String {
    match at.as_f64().filter(|v| v.is_finite()) {
        Some(at) => format!(
            "{} ({} {})",
            kit::format::rel_time(at, kit::format::now_ms()),
            kit::format::date(at, DateFormat::Short),
            kit::format::time(at, false, true)
        ),
        None => "–".into(),
    }
}

/// The end of a hold, "28-10-2026 15:00" (after "tot": no relative time).
fn until_text(at: f64) -> String {
    format!("{} {}", kit::format::date(at, DateFormat::Short), kit::format::time(at, false, true))
}

/// The Snap Store row: snapd's timer, or the hold (snaps.held: `next` is the hold's end, null = until you resume).
fn snaps_text(s: &Value) -> String {
    if !s.is_object() {
        return ct::unavailable().into();
    }
    if s["held"] == true {
        return match s["next"].as_f64().filter(|v| v.is_finite()) {
            Some(next) => common::fill(
                t("Updates zijn onderbroken tot {until}. Laatst: {when}.", "Updates are paused until {until}. Last: {when}."),
                &[("until", &until_text(next)), ("when", &when_text(&s["last"]))],
            ),
            None => common::fill(t("Updates zijn onderbroken. Laatst: {when}.", "Updates are paused. Last: {when}."), &[("when", &when_text(&s["last"]))]),
        };
    }
    common::fill(
        t("Worden automatisch bijgewerkt door Ubuntu. Laatst: {when}, volgende: {next}.", "Updated automatically by Ubuntu. Last: {when}, next: {next}."),
        &[("when", &when_text(&s["last"])), ("next", &when_text(&s["next"]))],
    )
}

/// The auto-open row's description: the interval sentence while on, then the security-update exception.
fn auto_text(a: &Value) -> String {
    let desc = t(
        "Voor beveiligingsupdates opent Ubuntu hem altijd meteen. Updates kun je ook hier installeren.",
        "For security updates, Ubuntu always opens it right away. You can also install updates here.",
    );
    if a["on"] != true {
        return desc.into();
    }
    let n = a["days"].as_f64().filter(|v| v.is_finite()).map(|v| v as i64).unwrap_or(7);
    let days = match n {
        0 => t("Ubuntu opent de Software-updater meteen als er gewone updates zijn.", "Ubuntu opens Software Updater right away when regular updates are available.").to_owned(),
        // "om de 1 dag" reads as "every other day"
        1 => t("Ubuntu opent de Software-updater elke dag als er gewone updates zijn.", "Ubuntu opens Software Updater every day when regular updates are available.").to_owned(),
        n => common::fill(
            t("Ubuntu opent de Software-updater om de {n} dagen als er gewone updates zijn.", "Ubuntu opens Software Updater every {n} days when regular updates are available."),
            &[("n", &n.to_string())],
        ),
    };
    format!("{days} {desc}")
}

/// The Software Updater's confirmation, then system.openUpdater (update.js openUpdater).
pub(super) fn open_updater(p: &Page, done: impl FnOnce() + 'static) {
    let pp = p.clone();
    Dialog::new(t("Software-updater openen?", "Open Software Updater?"))
        .text(t(
            "De Software-updater van Ubuntu wordt geopend. Als je updates installeert, vraagt Ubuntu om je wachtwoord. Dat is normaal: updates veranderen het systeem. Sla je werk op, want soms moet de computer daarna opnieuw opstarten.",
            "Ubuntu’s Software Updater will open. When you install updates, Ubuntu asks for your password. That’s normal: updates change the system. Save your work, because the computer sometimes needs to restart afterwards.",
        ))
        .icon("info")
        .primary(t("Openen", "Open"))
        .show(&p.anchor(), move |r| {
            if r != DialogResult::Primary || !pp.alive() {
                done();
                return;
            }
            let fut = pp.run("system.openUpdater", json!({}));
            pp.spawn(async move {
                let _ = fut.await;
                done();
            });
        });
}

struct State {
    p: Page,
    /// update.options; None: not answered yet, an older host without it, or a failure
    o: RefCell<Option<Value>>,
    auto_sw: ToggleSwitch,
    auto: SettingsCard,
    security: SettingsCard,
    snaps: SettingsCard,
}

fn apply(st: &State, v: Option<Value>) {
    if let Some(v) = v.filter(|v| !v.is_null()) {
        *st.o.borrow_mut() = Some(v);
    }
    let o = st.o.borrow().clone().unwrap_or(Value::Null);
    let a = &o["autoOpen"];
    // without update-notifier's schema the row goes (Ubuntu would not open the Software Updater by itself)
    if a["supported"] != true {
        st.auto.widget().set_visible(false);
    } else {
        st.auto_sw.set_active(a["on"] == true);
        st.auto_sw.widget().set_visible(true);
        st.auto.set_description(&auto_text(a));
    }
    st.security.set_description(match o["unattended"].as_bool() {
        Some(true) => t("Aan: Ubuntu installeert ze elke dag", "On: Ubuntu installs them every day"),
        Some(false) => t("Uit", "Off"),
        None => ct::unavailable(),
    });
    st.snaps.set_description(&snaps_text(&o["snaps"]));
}

async fn set_auto(st: Rc<State>, on: bool) {
    st.auto_sw.set_sensitive(false);
    let r = st.p.call("update.setAutoOpen", json!({ "on": on })).await;
    if !st.p.alive() {
        return;
    }
    let mut fresh: Option<Value> = None;
    let mut lost = false;
    match r {
        Err(e) => st.p.error(&e, None),
        Ok(v) if v["forwarded"] == true => match reply(v["nonce"].clone(), 5000).await {
            None => lost = true,
            Some(d) if d["ok"] == false && d["error"].is_object() => {
                common::show_error_text(&st.p.anchor(), &err_message(&d["error"]).unwrap_or_default(), None);
            }
            Some(_) => {}
        },
        Ok(v) if v["dryRun"] == true => common::note_dry_run(&st.p.anchor(), &v),
        // the answer's autoOpen only: its other rows are the host's last ones
        Ok(v) if v["autoOpen"].is_object() => {
            let mut o = st.o.borrow().clone().filter(Value::is_object).unwrap_or_else(|| json!({}));
            o["autoOpen"] = v["autoOpen"].clone();
            fresh = Some(o);
        }
        Ok(_) => {}
    }
    // the real state, also after a refusal, a dry run or a forwarded write
    if fresh.is_none() {
        fresh = st.p.get("update.options", json!({})).await.or_else(|| st.o.borrow().clone());
    }
    if !st.p.alive() {
        return;
    }
    // the owning window did not answer (it closed): say so unless the change arrived anyway
    if lost && fresh.as_ref().is_none_or(|f| f["autoOpen"]["on"] != on) {
        common::show_error_text(&st.p.anchor(), "", None);
    }
    apply(&st, fresh);
    st.auto_sw.set_sensitive(true);
}

fn update_advanced(p: Page) -> PageFuture {
    Box::pin(async move {
        let (metered, _) = common::toggle_card(
            &p,
            "",
            t("Updates downloaden via verbindingen met een datalimiet", "Download updates over metered connections"),
            t("Er kunnen kosten in rekening worden gebracht", "Data charges may apply"),
            "update.metered",
            false,
        );
        let (notify, _) = common::toggle_card(
            &p,
            "",
            t("Melden wanneer opnieuw opstarten vereist is om het bijwerken te voltooien", "Notify me when a restart is required to finish updating"),
            t(
                "Een melding weergeven wanneer je pc opnieuw moet worden opgestart om het bijwerken te voltooien",
                "Show a notification when your PC requires a restart to finish updating",
            ),
            "update.notifyRestart",
            false,
        );
        let auto_title = t("Software-updater van Ubuntu automatisch openen voor gewone updates", "Open Ubuntu’s Software Updater automatically for regular updates");
        let auto_sw = ToggleSwitch::new(false);
        auto_sw.switch().set_widget_name("advanced-autoopen");
        auto_sw.switch().update_property(&[gtk::accessible::Property::Label(auto_title)]);
        // the switch appears with its real state (not a false "Uit" while update.options runs)
        auto_sw.set_sensitive(false);
        auto_sw.widget().set_visible(false);
        let auto = SettingsCard::new("", auto_title, ct::loading());
        auto.set_control(auto_sw.widget());
        let security = SettingsCard::new("shield", t("Automatische beveiligingsupdates", "Automatic security updates"), ct::loading());
        let snaps = SettingsCard::new("apps", t("Apps uit de Snap Store", "Snap Store apps"), ct::loading());
        let open = c::button(t("Openen", "Open"), c::Variant::Standard);
        open.set_widget_name("advanced-updater");
        let pp = p.clone();
        open.connect_clicked(move |_| open_updater(&pp, || {}));
        let updater = SettingsCard::new("update", t("Software-updater van Ubuntu", "Ubuntu Software Updater"), "");
        updater.set_control(&open);

        let st = Rc::new(State { p: p.clone(), o: RefCell::new(None), auto_sw, auto, security, snaps });
        let weak = Rc::downgrade(&st);
        st.auto_sw.connect_toggled(move |on| {
            if let Some(st) = weak.upgrade() {
                st.p.clone().spawn(set_auto(st, on));
            }
        });
        // the page first, then the rows that wait for update.options' programs
        let fut = p.get("update.options", json!({}));
        let s2 = st.clone();
        p.spawn(async move {
            let v = fut.await;
            if !s2.p.alive() {
                return;
            }
            apply(&s2, v);
            s2.auto_sw.set_sensitive(true);
        });

        let g1 = SettingsGroup::new("", "");
        g1.add(metered.widget());
        g1.add(notify.widget());
        g1.add(st.auto.widget());
        // Windows 11 24H2's "Additional options": the rows with icons under their own header
        let g2 = SettingsGroup::new(t("Aanvullende opties", "Additional options"), "");
        g2.add(st.security.widget());
        g2.add(st.snaps.widget());
        g2.add(updater.widget());
        p.on_cleanup(move || drop(st));
        let out: Content = vec![g1.widget().clone().upcast(), g2.widget().clone().upcast()];
        out
    })
}
