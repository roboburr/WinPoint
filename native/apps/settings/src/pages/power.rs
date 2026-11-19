//! Instellingen › Systeem › Energie en batterij (6ebf169f:vendor/ui/js/apps/settings/pages/power.js): GNOME's own
//! power settings (power mode, screen off, dimming, automatic sleep), the battery and the confirmed power actions,
//! in Windows 11's layout (settings parity, batch D, amendment 20):
//!
//! * Energie: the expander Scherm en slaapstand (GNOME's screen blank, dim and automatic suspend through
//!   system.power.settings.*, with GNOME's warning when automatic suspend is Nooit), the expander
//!   Besturingselementen voor deksel en aan/uit-knop with GNOME's one Power Button Behavior (power-button-action;
//!   Sluimerstand only where logind can hibernate; no lid rows, GNOME has none), Energiestand, and on a pc without a
//!   battery the Energiebesparing expander.
//! * Batterij (a battery): Energiebesparing (Altijd energiebesparing gebruiken = the power-saver profile, and
//!   power-saver-profile-on-low-battery), Batterijpercentage (show-battery-percentage, mirrored to
//!   taskbar.batteryPercent for the tray), Batterij opladen (GNOME's Maximise Charge / Preserve Battery Health:
//!   battery.charging.*, only when UPower supports a charge limit), Batterijniveaus (battery.history: the last 24
//!   hours or 7 days), then the health, cycles and power use.
//! * Aan/uit: the confirmed power actions.
use super::{Builder, Content, Page, PageFuture};
use crate::common::{self, UsageBar};
use crate::gs::{self, GKey, Gs};
use kit::chart::{LineChart, LineOptions, Series};
use kit::controls::{self as c, Dropdown, DropdownOption, SettingsCard, SettingsExpander, SettingsGroup, ToggleSwitch};
use kit::prelude::*;
use kit::t;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

const GSD_POWER: &str = "org.gnome.settings-daemon.plugins.power";
const BUTTON: GKey = gs::key(GSD_POWER, "power-button-action");
const LOW_BATTERY: GKey = gs::key(GSD_POWER, "power-saver-profile-on-low-battery");
const PERCENT: GKey = gs::key("org.gnome.desktop.interface", "show-battery-percentage");
fn keys() -> Vec<GKey> {
    vec![BUTTON, LOW_BATTERY, PERCENT]
}

pub const PAGES: &[(&str, Builder)] = &[("power", power)];
pub const CSS: &str = "";

struct Action {
    action: &'static str,
    icon: &'static str,
    title: (&'static str, &'static str),
    desc: (&'static str, &'static str),
    /// (question, body) when it asks first.
    confirm: Option<((&'static str, &'static str), (&'static str, &'static str))>,
}

const ACTIONS: &[Action] = &[
    Action { action: "lock", icon: "lock", title: ("Vergrendelen", "Lock"), desc: ("Het scherm vergrendelen; je apps blijven open", "Lock the screen; your apps stay open"), confirm: None },
    Action {
        action: "logout",
        icon: "signout",
        title: ("Afmelden", "Sign out"),
        desc: ("Je sessie afsluiten; open apps worden gesloten", "End your session; open apps will close"),
        confirm: Some((("Afmelden?", "Sign out?"), ("Sla eerst je werk op. Alle open apps worden gesloten.", "Save your work first. All open apps will close."))),
    },
    Action { action: "suspend", icon: "sleep", title: ("Slaapstand", "Sleep"), desc: ("Zuinig pauzeren; je werk blijft open staan", "Pause and save power; your work stays open"), confirm: None },
    Action {
        action: "reboot",
        icon: "restart",
        title: ("Opnieuw opstarten", "Restart"),
        desc: ("De computer uit- en weer aanzetten", "Turn the computer off and on again"),
        confirm: Some((
            ("Opnieuw opstarten?", "Restart?"),
            (
                "Sla eerst je werk op. Alle open apps worden gesloten en de computer start opnieuw op.",
                "Save your work first. All open apps will close and the computer will restart.",
            ),
        )),
    },
    Action {
        action: "shutdown",
        icon: "power",
        title: ("Afsluiten", "Shut down"),
        desc: ("De computer helemaal uitzetten", "Turn the computer off completely"),
        confirm: Some((
            ("Computer afsluiten?", "Shut down the computer?"),
            (
                "Sla eerst je werk op. Alle open apps worden gesloten en de computer gaat uit.",
                "Save your work first. All open apps will close and the computer will turn off.",
            ),
        )),
    },
];

/// A power action, asking first unless "Altijd eerst vragen" is off (power.js powerAction).
fn power_action(p: &Page, a: &'static Action) {
    let run = {
        let pp = p.clone();
        move || {
            let fut = pp.run("system.power.action", json!({ "action": a.action }));
            pp.spawn(async move {
                let _ = fut.await;
            });
        }
    };
    match a.confirm {
        Some((q, b)) if common::setting_bool("power.confirm", true) => {
            c::confirm(&p.anchor(), t(q.0, q.1), t(b.0, b.1), t(a.title.0, a.title.1), a.action != "logout", move |ok| {
                if ok {
                    run();
                }
            });
        }
        _ => run(),
    }
}

/// Sign out with Energie's own question (WinPoint's update card: the sign-out that finishes an update whose extension
/// changed, update_winpoint.rs).
pub fn sign_out(p: &Page) {
    if let Some(a) = ACTIONS.iter().find(|a| a.action == "logout") {
        power_action(p, a);
    }
}

const SCREEN_TIMES: &[i64] = &[60, 120, 180, 300, 600, 900, 1800, 3600];
const SLEEP_TIMES: &[i64] = &[60, 120, 180, 300, 600, 900, 1800, 3600, 7200];

fn time_label(v: i64) -> String {
    if v % 60 != 0 {
        common::plural(v, t("1 seconde", "1 second"), t("{n} seconden", "{n} seconds"))
    } else {
        kit::format::duration_long(v as f64)
    }
}

fn valid_time(v: &Value) -> Option<i64> {
    v.as_i64().filter(|n| *n >= 0)
}

/// The time choices with the current value as "(aangepast)" when it is not one of them, then Nooit.
fn time_options(times: &[i64], current: Option<i64>) -> Vec<DropdownOption> {
    let mut values: Vec<i64> = times.to_vec();
    if let Some(cur) = current
        && cur > 0
        && !values.contains(&cur)
    {
        values.push(cur);
    }
    values.sort();
    let mut out: Vec<DropdownOption> = values
        .into_iter()
        .map(|v| {
            let label = if times.contains(&v) { time_label(v) } else { common::fill(t("{time} (aangepast)", "{time} (custom)"), &[("time", &time_label(v))]) };
            DropdownOption::new(&v.to_string(), &label)
        })
        .collect();
    out.push(DropdownOption::new("0", t("Nooit", "Never")));
    out
}

fn sleep_value(s: &Value) -> String {
    if s["action"] == "nothing" || s["timeout"].as_i64() == Some(0) {
        return "0".into();
    }
    if s["action"] == "suspend"
        && let Some(t) = valid_time(&s["timeout"])
    {
        return t.to_string();
    }
    format!("current:{}:{}", s["action"].as_str().unwrap_or(""), s["timeout"])
}

fn action_name(action: &str) -> Option<&'static str> {
    Some(match action {
        "hibernate" => t("sluimerstand", "hibernate"),
        "shutdown" => t("afsluiten", "shut down"),
        "blank" => t("scherm uit", "turn off the screen"),
        "interactive" => t("vragen", "ask"),
        "logout" => t("afmelden", "sign out"),
        "suspend-then-hibernate" => t("slaapstand, daarna sluimerstand", "sleep, then hibernate"),
        _ => return None,
    })
}

fn sleep_options(s: &Value) -> Vec<DropdownOption> {
    let value = sleep_value(s);
    let numeric = value.parse::<i64>().ok();
    let mut options = time_options(SLEEP_TIMES, numeric);
    if numeric.is_none() {
        let a = s["action"].as_str().unwrap_or("");
        let action = action_name(a).map(str::to_owned).unwrap_or_else(|| if a.is_empty() { "–".into() } else { a.to_owned() });
        let time = valid_time(&s["timeout"]).map(time_label).unwrap_or_else(|| "–".into());
        let mut o = DropdownOption::new(&value, &common::fill(t("Huidige instelling: {action} na {time}", "Current setting: {action} after {time}"), &[("action", &action), ("time", &time)]));
        o.disabled = true;
        options.insert(0, o);
    }
    options
}

fn profile_name(id: &str) -> String {
    match id {
        "power-saver" => t("Beste energie-efficiëntie", "Best power efficiency").into(),
        "balanced" => t("Gebalanceerd", "Balanced").into(),
        "performance" => t("Beste prestaties", "Best performance").into(),
        other => other.into(),
    }
}

fn unavailable_desc() -> &'static str {
    t("Deze instelling is op deze computer niet beschikbaar.", "This setting isn’t available on this computer.")
}

fn setting_description(setting: &Value, description: &str, supported: bool) -> String {
    if setting["available"] != true {
        unavailable_desc().into()
    } else if !supported {
        t("Deze computer ondersteunt automatisch slapen momenteel niet.", "This computer doesn’t support automatic sleep right now.").into()
    } else if setting["writable"] != true {
        t("Deze instelling wordt beheerd en kan hier niet worden gewijzigd.", "This setting is managed and can’t be changed here.").into()
    } else {
        description.into()
    }
}

/// The GNOME power settings (power.js EnergySettings): the same controls stay while the settings change
/// elsewhere; writes are serialized and followed by a read back.
struct Energy {
    page: Page,
    state: RefCell<Value>,
    pending: Cell<bool>,
    revision: Cell<u64>,
    battery: Cell<bool>,
    profile: Dropdown,
    screen: Dropdown,
    dim: ToggleSwitch,
    sleep_ac: Dropdown,
    sleep_battery: Dropdown,
    profile_card: SettingsCard,
    screen_card: SettingsCard,
    dim_card: SettingsCard,
    sleep_ac_card: SettingsCard,
    sleep_battery_card: SettingsCard,
    /// Energiebesparing's "Altijd energiebesparing gebruiken": the power-saver profile.
    saver: ToggleSwitch,
    saver_card: SettingsCard,
    /// GNOME's notice while automatic suspend is off.
    suspend_notice: gtk::Widget,
}

/// Whether "Altijd energiebesparing gebruiken" can be offered: power-profiles-daemon with a power-saver profile.
pub(crate) fn saver_offered(profiles: &Value) -> bool {
    profiles["available"] == true && profiles["choices"].as_array().is_some_and(|a| a.iter().any(|c| c["id"] == "power-saver" && c["disabled"] != true))
}

/// The profile "Altijd energiebesparing gebruiken" sets: power-saver, or back to balanced (GNOME's default) when it is
/// turned off (performance when only that and power-saver exist).
pub(crate) fn saver_profile(on: bool, profiles: &Value) -> &'static str {
    if on {
        return "power-saver";
    }
    let has = |id: &str| profiles["choices"].as_array().is_some_and(|a| a.iter().any(|c| c["id"] == id && c["disabled"] != true));
    if has("balanced") || !has("performance") { "balanced" } else { "performance" }
}

/// GNOME's warning: automatic suspend is off (Nooit) for the power source that counts (the battery's on a laptop).
/// Only where the computer can sleep at all: GNOME Settings hides its automatic suspend row (and so the warning)
/// when logind cannot suspend, and this page then says so on the sleep rows.
pub(crate) fn suspend_off(state: &Value, battery: bool) -> bool {
    let s = &state[if battery { "sleepBattery" } else { "sleepAC" }];
    state["canSuspend"] == true && s["available"] == true && sleep_value(s) == "0"
}

impl Energy {
    fn set_all_sensitive(&self, on: bool) {
        self.saver.set_sensitive(on);
        self.profile.widget().set_sensitive(on);
        self.screen.widget().set_sensitive(on);
        self.dim.set_sensitive(on);
        self.sleep_ac.widget().set_sensitive(on);
        self.sleep_battery.widget().set_sensitive(on);
    }

    fn sync(&self) {
        if !self.page.alive() || self.pending.get() {
            return;
        }
        let s = self.state.borrow().clone();
        let p = &s["profiles"];
        let mut options: Vec<DropdownOption> = p["choices"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|ch| {
                        let id = common::value_key(&ch["id"]);
                        let mut o = DropdownOption::new(&id, &profile_name(&id));
                        o.disabled = ch["disabled"] == true;
                        o
                    })
                    .collect()
            })
            .unwrap_or_default();
        let active = p["active"].as_str().unwrap_or("").to_owned();
        if p["available"] == true && !active.is_empty() && !options.iter().any(|o| o.value == active) {
            let mut o = DropdownOption::new(&active, &active);
            o.disabled = true;
            options.push(o);
        }
        let empty = options.is_empty();
        self.profile.set_options(options);
        self.profile.set_selected(if active.is_empty() { None } else { Some(active.as_str()) });
        self.profile.widget().set_sensitive(p["available"] == true && p["writable"] == true && !empty);
        let mut desc = if p["available"] != true {
            t("Op deze computer kan de energiestand niet worden aangepast.", "The power mode can’t be changed on this computer.").to_owned()
        } else if p["writable"] != true {
            t("Deze instelling wordt beheerd en kan hier niet worden gewijzigd.", "This setting is managed and can’t be changed here.").to_owned()
        } else {
            t("Kies de balans tussen energieverbruik en prestaties", "Choose the balance between power use and performance").to_owned()
        };
        let reason = p["inhibited"].as_str().filter(|r| !r.is_empty()).or(p["degraded"].as_str().filter(|r| !r.is_empty()));
        if p["available"] == true
            && let Some(reason) = reason
        {
            desc.push(' ');
            desc.push_str(if p["inhibited"].as_str().is_some_and(|r| !r.is_empty()) {
                t("Beste prestaties is momenteel niet beschikbaar.", "Best performance is currently unavailable.")
            } else {
                t("De prestaties zijn momenteel beperkt door de computer.", "Performance is currently limited by the computer.")
            });
            if reason == "high-operating-temperature" {
                desc.push(' ');
                desc.push_str(t("De computer is te warm.", "The computer is too hot."));
            }
            if reason == "lap-detected" {
                desc.push(' ');
                desc.push_str(t("De computer staat op schoot.", "The computer is on your lap."));
            }
        }
        self.profile_card.set_description(&desc);

        let sc = &s["screen"];
        let delay = valid_time(&sc["idleDelay"]);
        self.screen.set_options(if sc["available"] == true { time_options(SCREEN_TIMES, delay) } else { Vec::new() });
        self.screen.set_selected(delay.map(|d| d.to_string()).as_deref());
        self.screen.widget().set_sensitive(sc["available"] == true && sc["writable"] == true);
        self.screen_card.set_description(&setting_description(sc, t("Als je de computer niet gebruikt", "When you’re not using the computer"), true));

        let dim = &s["dim"];
        self.dim.set_active(dim["enabled"] == true);
        self.dim.set_sensitive(dim["available"] == true && dim["writable"] == true);
        self.dim_card.widget().set_visible(dim["available"] == true);
        self.dim_card.set_description(&setting_description(
            dim,
            t("De helderheid van het ingebouwde scherm verlagen om stroom te besparen", "Lower the brightness of the built-in screen to save power"),
            true,
        ));

        let can_suspend = s["canSuspend"] == true;
        for (key, dd, card, desc) in [
            ("sleepAC", &self.sleep_ac, &self.sleep_ac_card, t("Automatisch slapen als je de computer niet gebruikt", "Sleep automatically when you’re not using the computer")),
            ("sleepBattery", &self.sleep_battery, &self.sleep_battery_card, t("Automatisch slapen om de batterij te sparen", "Sleep automatically to save battery power")),
        ] {
            let st = &s[key];
            dd.set_options(if st["available"] == true { sleep_options(st) } else { Vec::new() });
            dd.set_selected(if st["available"] == true { Some(sleep_value(st)) } else { None }.as_deref());
            dd.widget().set_sensitive(st["available"] == true && st["writable"] == true && can_suspend);
            card.set_description(&setting_description(st, desc, can_suspend));
        }
        self.sleep_battery_card.widget().set_visible(self.battery.get());
        self.suspend_notice.set_visible(suspend_off(&s, self.battery.get()));

        let offered = saver_offered(p);
        self.saver_card.widget().set_visible(offered);
        self.saver.set_active(active == "power-saver");
        self.saver.set_sensitive(offered && p["writable"] == true);
    }

    fn save(self: &Rc<Self>, patch: Value, writable: bool) {
        if !self.page.alive() {
            return;
        }
        if self.pending.get() || !writable {
            self.sync();
            return;
        }
        self.pending.set(true);
        self.set_all_sensitive(false);
        let me = self.clone();
        self.page.spawn(async move {
            let p = me.page.clone();
            let before_write = me.revision.get();
            let result = p.call("system.power.settings.set", patch).await;
            if !p.alive() {
                return;
            }
            match result {
                Err(e) => p.error(&e, None),
                Ok(v) => {
                    common::note_dry_run(&p.anchor(), &v);
                    if me.revision.get() == before_write && v.is_object() {
                        *me.state.borrow_mut() = v;
                    }
                }
            }
            let before_read = me.revision.get();
            let fresh = p.call("system.power.settings.get", json!({})).await;
            if !p.alive() {
                return;
            }
            if let Ok(v) = fresh
                && me.revision.get() == before_read
            {
                *me.state.borrow_mut() = if v.is_object() { v } else { json!({}) };
            }
            me.pending.set(false);
            me.sync();
        });
    }

    fn accept(&self, value: &Value) {
        if !self.page.alive() || !value.is_object() {
            return;
        }
        *self.state.borrow_mut() = value.clone();
        self.revision.set(self.revision.get() + 1);
        if !self.pending.get() {
            self.sync();
        }
    }

    fn set_battery_present(&self, present: bool) {
        self.battery.set(present);
        self.sync();
    }
}

/// The controls of the GNOME power settings, for the page to place.
struct EnergyCards {
    profile: SettingsCard,
    screen_sleep: SettingsExpander,
    saver: SettingsCard,
}

fn energy_settings(p: &Page, initial: Value, battery: bool) -> (Rc<Energy>, EnergyCards) {
    let dd = |name: &str| {
        let d = Dropdown::new(Vec::new(), None);
        d.set_placeholder(t("Niet beschikbaar", "Not available"));
        d.widget().set_widget_name(name);
        d
    };
    let profile = dd("power-mode");
    let screen = dd("power-screen");
    let sleep_ac = dd("power-sleep-ac");
    let sleep_battery = dd("power-sleep-battery");
    let dim = ToggleSwitch::new(false);
    dim.switch().set_widget_name("power-dim");
    let profile_card = SettingsCard::new("lightning", t("Energiestand", "Power mode"), "");
    profile_card.set_control(&common::sized(profile.widget(), 230));
    let screen_card = SettingsCard::new("display", t("Scherm uitschakelen na", "Turn off my screen after"), "");
    screen_card.set_control(&common::sized(screen.widget(), 200));
    let dim_card = SettingsCard::new("brightness", t("Scherm dimmen bij inactiviteit", "Dim the screen when inactive"), "");
    dim_card.set_control(dim.widget());
    let sleep_ac_card = SettingsCard::new("sleep", t("Op netstroom in slaapstand na", "When plugged in, put my device to sleep after"), "");
    sleep_ac_card.set_control(&common::sized(sleep_ac.widget(), 200));
    let sleep_battery_card = SettingsCard::new("sleep", t("Op batterij in slaapstand na", "On battery, put my device to sleep after"), "");
    sleep_battery_card.set_control(&common::sized(sleep_battery.widget(), 200));
    let saver = ToggleSwitch::new(false);
    saver.switch().set_widget_name("st-power-saver");
    let saver_card = SettingsCard::new("", t("Altijd energiebesparing gebruiken", "Always use energy saver"), "");
    saver_card.set_control(saver.widget());
    // gnome-control-center's suspend notice
    let notice = common::notice(t(
        "Als je automatisch slapen uitschakelt, verbruikt de pc meer energie. Het is beter om automatisch slapen ingeschakeld te laten.",
        "Disabling automatic sleep will result in higher power consumption. It’s recommended to keep automatic sleep turned on.",
    ));
    notice.widget().set_widget_name("st-power-suspend-notice");
    let e = Rc::new(Energy {
        page: p.clone(),
        state: RefCell::new(if initial.is_object() { initial } else { json!({}) }),
        pending: Cell::new(false),
        revision: Cell::new(0),
        battery: Cell::new(battery),
        profile: profile.clone(),
        screen: screen.clone(),
        dim: dim.clone(),
        sleep_ac: sleep_ac.clone(),
        sleep_battery: sleep_battery.clone(),
        profile_card: profile_card.clone(),
        screen_card: screen_card.clone(),
        dim_card: dim_card.clone(),
        sleep_ac_card: sleep_ac_card.clone(),
        sleep_battery_card: sleep_battery_card.clone(),
        saver: saver.clone(),
        saver_card: saver_card.clone(),
        suspend_notice: notice.widget().clone().upcast(),
    });
    let weak = Rc::downgrade(&e);
    profile.connect_changed(move |value| {
        let Some(e) = weak.upgrade() else { return };
        let s = e.state.borrow()["profiles"].clone();
        let ok = s["available"] == true
            && s["writable"] == true
            && s["choices"].as_array().is_some_and(|a| a.iter().any(|ch| common::value_key(&ch["id"]) == value && ch["disabled"] != true));
        e.save(json!({ "powerProfile": value }), ok);
    });
    let weak = Rc::downgrade(&e);
    saver.connect_toggled(move |on| {
        let Some(e) = weak.upgrade() else { return };
        let s = e.state.borrow()["profiles"].clone();
        e.save(json!({ "powerProfile": saver_profile(on, &s) }), saver_offered(&s) && s["writable"] == true);
    });
    let weak = Rc::downgrade(&e);
    screen.connect_changed(move |value| {
        let Some(e) = weak.upgrade() else { return };
        let s = e.state.borrow()["screen"].clone();
        let v: i64 = value.parse().unwrap_or(0);
        e.save(json!({ "idleDelay": v }), s["available"] == true && s["writable"] == true);
    });
    let weak = Rc::downgrade(&e);
    dim.connect_toggled(move |on| {
        let Some(e) = weak.upgrade() else { return };
        let s = e.state.borrow()["dim"].clone();
        e.save(json!({ "idleDim": on }), s["available"] == true && s["writable"] == true);
    });
    for (key, d) in [("sleepAC", &sleep_ac), ("sleepBattery", &sleep_battery)] {
        let weak = Rc::downgrade(&e);
        d.connect_changed(move |value| {
            let Some(e) = weak.upgrade() else { return };
            let (current, can) = {
                let s = e.state.borrow();
                (s[key].clone(), s["canSuspend"] == true)
            };
            let numeric = value.parse::<i64>().ok();
            let v = numeric.unwrap_or(0);
            let timeout = if v == 0 { valid_time(&current["timeout"]).unwrap_or(1800) } else { v };
            let mut patch = json!({});
            patch[key] = json!({ "action": if v == 0 { "nothing" } else { "suspend" }, "timeout": timeout });
            e.save(patch, numeric.is_some() && can && current["available"] == true && current["writable"] == true);
        });
    }
    e.sync();
    // Windows 23H2's "Scherm en slaapstand" (24H2: Time-outs voor scherm, slaapstand en sluimerstand)
    let screen_sleep = SettingsExpander::new("sleep", t("Scherm en slaapstand", "Screen & sleep"), t("Scherm uitschakelen, dimmen en automatisch slapen", "Turn off the screen, dim it and sleep automatically"));
    screen_sleep.header_button().set_widget_name("st-power-screen-sleep");
    screen_sleep.add(screen_card.widget());
    screen_sleep.add(dim_card.widget());
    screen_sleep.add(sleep_ac_card.widget());
    screen_sleep.add(sleep_battery_card.widget());
    screen_sleep.add(notice.widget());
    (e, EnergyCards { profile: profile_card, screen_sleep, saver: saver_card })
}

// ------------------------------------------------------------------------------- the GNOME rows of amendment 20

/// GNOME's Power Button Behavior (one value for every power source): Windows' four choices, Sluimerstand only where
/// logind can hibernate (or when it is set already).
pub(crate) fn button_options(hibernate: bool, current: &str) -> Vec<(Value, String)> {
    let mut out = vec![
        (json!("nothing"), t("Niets doen", "Do nothing").to_owned()),
        (json!("suspend"), t("Slaapstand", "Sleep").to_owned()),
    ];
    if hibernate || current == "hibernate" {
        out.push((json!("hibernate"), t("Sluimerstand", "Hibernate").to_owned()));
    }
    // GNOME's Power Off: the power-off question with its countdown
    out.push((json!("interactive"), t("Afsluiten", "Shut down").to_owned()));
    out
}

fn lid_and_button(gs: &Gs) -> Option<SettingsExpander> {
    if !gs.available(&BUTTON) {
        return None;
    }
    let exp = SettingsExpander::new("power", t("Besturingselementen voor deksel en aan/uit-knop", "Lid & power button controls"), t("Wat er gebeurt als je op de aan/uit-knop drukt", "What happens when you press the power button"));
    exp.header_button().set_widget_name("st-power-lid-button");
    let current = gs.value(&BUTTON).as_str().unwrap_or("").to_owned();
    let card = SettingsCard::new("", t("Als ik op de aan/uit-knop druk, gaat mijn pc naar", "Pressing the power button will make my PC"), "");
    card.set_control(&common::sized(gs.dropdown(&BUTTON, button_options(common::caps()["hibernate"] == true, &current)).widget(), 200));
    exp.add(card.widget());
    Some(exp)
}

/// Energiebesparing (amendment 20): both rows inside the expander; the automatic one only with a battery and
/// power-profiles-daemon, as GNOME's Automatic Power Saver.
fn energy_saver(gs: &Gs, saver: &SettingsCard, battery: bool) -> SettingsExpander {
    let exp = SettingsExpander::new("battery-saver", t("Energiebesparing", "Energy saver"), t("Energie besparen door de prestaties te beperken", "Save energy by limiting performance"));
    exp.header_button().set_widget_name("st-power-energy-saver");
    exp.add(saver.widget());
    // without power-profiles-daemon's power-saver profile there is nothing to turn on (GNOME hides both rows too)
    saver.widget().bind_property("visible", exp.widget(), "visible").sync_create().build();
    if battery && gs.available(&LOW_BATTERY) {
        let card = SettingsCard::new("", t("Energiebesparing automatisch inschakelen bij een laag batterijniveau", "Turn energy saver on automatically when the battery is low"), "");
        card.set_control(gs.toggle(&LOW_BATTERY, false).widget());
        exp.add(card.widget());
    }
    exp
}

/// Batterij opladen (GNOME's Battery Charging): two radio rows on battery.charging.*.
struct Charging {
    p: Page,
    exp: SettingsExpander,
    maximize: gtk::CheckButton,
    preserve: gtk::CheckButton,
    preserve_card: SettingsCard,
    silent: Cell<bool>,
}

impl Charging {
    fn apply(&self, state: &Value) {
        self.exp.widget().set_visible(state["supported"] == true);
        self.silent.set(true);
        if state["enabled"] == true {
            self.preserve.set_active(true);
        } else {
            self.maximize.set_active(true);
        }
        self.silent.set(false);
        let mut desc = t("Verlengt de levensduur van de batterij door een lager laadniveau aan te houden", "Increases battery longevity by maintaining lower charge levels").to_owned();
        if let Some(end) = state["end"].as_u64() {
            desc.push_str(". ");
            desc.push_str(&common::fill(t("De batterij wordt tot {n}% opgeladen.", "The battery is charged up to {n}%."), &[("n", &end.to_string())]));
        }
        self.preserve_card.set_description(&desc);
    }
}

fn charging(p: &Page, state: &Value) -> Rc<Charging> {
    let exp = SettingsExpander::new("battery-charging", t("Batterij opladen", "Battery charging"), t("Kiezen hoe ver de batterij wordt opgeladen", "Choose how far the battery is charged"));
    exp.header_button().set_widget_name("st-power-charging");
    let maximize = gtk::CheckButton::new();
    maximize.add_css_class("w11-radio");
    maximize.set_widget_name("st-power-charging-maximize");
    let preserve = gtk::CheckButton::new();
    preserve.add_css_class("w11-radio");
    preserve.set_group(Some(&maximize));
    preserve.set_widget_name("st-power-charging-preserve");
    let max_card = SettingsCard::new("", t("Maximale lading", "Maximise charge"), t("Gebruikt de volledige capaciteit van de batterij. De batterij gaat sneller achteruit.", "Uses all battery capacity. Degrades batteries more quickly."));
    max_card.set_icon_widget(&maximize);
    let preserve_card = SettingsCard::new("", t("Batterijgezondheid behouden", "Preserve battery health"), "");
    preserve_card.set_icon_widget(&preserve);
    exp.add(max_card.widget());
    exp.add(preserve_card.widget());
    let ch = Rc::new(Charging { p: p.clone(), exp, maximize: maximize.clone(), preserve: preserve.clone(), preserve_card, silent: Cell::new(false) });
    ch.apply(state);
    for (button, enabled) in [(&maximize, false), (&preserve, true)] {
        let weak = Rc::downgrade(&ch);
        button.connect_toggled(move |b| {
            let Some(ch) = weak.upgrade() else { return };
            if ch.silent.get() || !b.is_active() {
                return;
            }
            let wait = ch.p.run_with("battery.charging.set", json!({ "enabled": enabled }), Some(t("Het opladen kon niet worden gewijzigd", "Couldn’t change battery charging")), false);
            let ch2 = ch.clone();
            ch.p.spawn(async move {
                let answer = wait.await;
                if !ch2.p.alive() {
                    return;
                }
                match answer {
                    Some(v) if v["dryRun"] != true => ch2.apply(&v),
                    _ => {
                        let fresh = ch2.p.get("battery.charging.get", json!({})).await;
                        if let Some(v) = fresh
                            && ch2.p.alive()
                        {
                            ch2.apply(&v);
                        }
                    }
                }
            });
        });
    }
    {
        let weak = Rc::downgrade(&ch);
        p.on("battery.changed", move |v| {
            if let Some(ch) = weak.upgrade() {
                ch.apply(v);
            }
        });
    }
    ch
}

/// UPower's history points [[time, value, state]] on a grid of `n` times from `now - span` to `now`: the last value
/// at or before each time, starting at the first sample (the chart draws them from the right).
pub(crate) fn resample(points: &[(f64, f64)], now: f64, span: f64, n: usize) -> Vec<f64> {
    let n = n.max(2);
    let step = span / (n - 1) as f64;
    let mut out = Vec::new();
    let mut j = 0;
    let mut last = None;
    for i in 0..n {
        let at = now - span + i as f64 * step;
        while j < points.len() && points[j].0 <= at {
            last = Some(points[j].1);
            j += 1;
        }
        if let Some(v) = last {
            out.push(v.clamp(0.0, 100.0));
        }
    }
    out
}

/// Batterijniveaus: Windows' battery levels chart for the last 24 hours or 7 days (battery.history).
fn levels(p: &Page) -> SettingsExpander {
    let exp = SettingsExpander::new("chart", t("Batterijniveaus", "Battery levels"), t("Het batterijniveau in de loop van de tijd", "The battery level over time"));
    exp.header_button().set_widget_name("st-power-levels");
    let span = Dropdown::new(
        vec![DropdownOption::new("86400", t("Afgelopen 24 uur", "Last 24 hours")), DropdownOption::new("604800", t("Afgelopen 7 dagen", "Last 7 days"))],
        Some("86400"),
    );
    span.widget().set_widget_name("st-power-levels-span");
    let chart = LineChart::new(LineOptions { min: 0.0, max: 100.0, color: "accent".into(), points: 97, border: false, grid_rows: 4, scroll_grid: false, line_width: 1.5, ..Default::default() });
    chart.widget().set_size_request(-1, 150);
    chart.widget().set_widget_name("st-power-levels-chart");
    let axis = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    let from = c::label("", "w11-text-3", false);
    from.set_hexpand(true);
    from.set_xalign(0.0);
    let to = c::label(t("Nu", "Now"), "w11-text-3", false);
    axis.append(&from);
    axis.append(&to);
    let body = gtk::Box::new(gtk::Orientation::Vertical, 6);
    // in the expander, not on its header: a drop-down inside the header's button never gets the pointer's click
    let range = common::sized(span.widget(), 180);
    range.set_halign(gtk::Align::End);
    body.append(&range);
    let empty = c::label(t("Er is nog geen batterijgeschiedenis.", "There’s no battery history yet."), "w11-text-2", true);
    body.append(&empty);
    body.append(chart.widget());
    body.append(&axis);
    exp.add(&body);
    let load = {
        let (pp, chart, from, empty, span) = (p.clone(), chart.clone(), from.clone(), empty.clone(), span.clone());
        Rc::new(move || {
            let seconds: u64 = span.selected().and_then(|s| s.parse().ok()).unwrap_or(86_400);
            let wait = pp.get("battery.history", json!({ "type": "charge", "timespan": seconds, "resolution": 200 }));
            let (p2, chart, from, empty) = (pp.clone(), chart.clone(), from.clone(), empty.clone());
            pp.spawn(async move {
                let Some(v) = wait.await else { return };
                if !p2.alive() {
                    return;
                }
                let points: Vec<(f64, f64)> = v["points"].as_array().into_iter().flatten().filter_map(|p| Some((p[0].as_f64()?, p[1].as_f64()?))).collect();
                let now = kit::format::now_ms() / 1000.0;
                let n = if seconds > 86_400 { 169 } else { 97 };
                let values = resample(&points, now, seconds as f64, n);
                chart.set_points(n);
                chart.update(vec![Series::new(values.clone())]);
                empty.set_visible(values.is_empty());
                chart.widget().set_visible(!values.is_empty());
                let start = (now - seconds as f64) * 1000.0;
                from.set_text(&format!("{} {}", kit::format::date(start, kit::format::DateFormat::DayShort), kit::format::time(start, false, common::setting_bool("time.h24", true))));
            });
        })
    };
    load();
    {
        let l = load.clone();
        span.connect_changed(move |_| l());
    }
    {
        // a new sample every few minutes (UPower records on every change of the level)
        let l = load.clone();
        p.interval(300_000, move || {
            l();
            true
        });
    }
    exp
}

/// The battery hero (power.js batteryHero): glyph, percentage, state and time, a coloured bar.
fn battery_hero(b: &Value) -> gtk::Widget {
    let pct = b["percent"].as_f64().unwrap_or(0.0).round();
    let st = b["state"].as_str().unwrap_or("");
    let on_ac = b["onAC"] == true;
    let state = if st == "charging" {
        t("Wordt opgeladen", "Charging")
    } else if st == "fully-charged" || st == "full" {
        t("Volledig opgeladen", "Fully charged")
    } else if on_ac {
        t("Aangesloten op netstroom", "Plugged in")
    } else {
        t("Op batterij", "On battery")
    };
    let time = match (st == "charging", b["timeToFull"].as_f64().filter(|v| *v > 0.0), b["timeToEmpty"].as_f64().filter(|v| *v > 0.0)) {
        (true, Some(full), _) => common::fill(t("Vol over ongeveer {d}", "Full in about {d}"), &[("d", &kit::format::duration_long(full))]),
        (_, _, Some(empty)) if !on_ac => common::fill(t("Nog ongeveer {d}", "About {d} left"), &[("d", &kit::format::duration_long(empty))]),
        _ => String::new(),
    };
    let color = if pct <= 15.0 {
        "sys-critical"
    } else if pct <= 30.0 {
        "sys-caution"
    } else {
        "sys-success"
    };
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 20);
    row.add_css_class("st-battery-hero");
    let icon = kit::icons::glyph(if st == "charging" { "battery-charging" } else { "battery" }, 40);
    icon.add_css_class("st-battery-icon");
    row.append(&icon);
    let text = gtk::Box::new(gtk::Orientation::Vertical, 6);
    text.set_hexpand(true);
    text.append(&c::label(&format!("{}%", pct as i64), "st-battery-pct", false));
    let line = [state.to_owned(), time].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ");
    text.append(&c::label(&line, "st-battery-state", true));
    let bar = UsageBar::new(pct, Some(color), 101.0);
    bar.set_thickness(6);
    let clamp = crate::widgets::Clamp::new(bar.widget(), 420);
    text.append(&clamp);
    row.append(&text);
    row.upcast()
}

fn power(p: Page) -> PageFuture {
    Box::pin(async move {
        let energy: Rc<RefCell<Option<Rc<Energy>>>> = Rc::new(RefCell::new(None));
        let queued: Rc<RefCell<Option<Value>>> = Rc::new(RefCell::new(None));
        {
            let (en, q) = (energy.clone(), queued.clone());
            p.on("system.power.settings.changed", move |v| {
                let e = en.borrow().clone();
                match e {
                    Some(e) => e.accept(v),
                    None => *q.borrow_mut() = Some(v.clone()),
                }
            });
        }
        let (br, er, cr) = (p.call("system.battery.get", json!({})), p.call("system.power.settings.get", json!({})), p.call("battery.charging.get", json!({})));
        let gs = Gs::load(&p, &keys()).await;
        let (br, er, cr) = (br.await, er.await, cr.await);
        if !p.alive() {
            return Content::new();
        }
        let b = br.ok().unwrap_or(Value::Null);
        let present = b["present"] == true;
        let mut out: Content = Vec::new();
        let battery_slot = common::slot();
        let mains_slot = common::slot();
        out.push(battery_slot.clone().upcast());

        // ---- Energie: Scherm en slaapstand, Besturingselementen voor deksel en aan/uit-knop, Energiestand
        let power_group = SettingsGroup::new(t("Energie", "Power"), "");
        let mut saver_card: Option<SettingsCard> = None;
        let q = queued.borrow_mut().take();
        match (er, q) {
            (_, Some(v)) | (Ok(v), None) => {
                let (e, cards) = energy_settings(&p, v, present);
                *energy.borrow_mut() = Some(e);
                power_group.add(cards.screen_sleep.widget());
                if let Some(exp) = lid_and_button(&gs) {
                    power_group.add(exp.widget());
                }
                power_group.add(cards.profile.widget());
                saver_card = Some(cards.saver);
            }
            (Err(err), None) => {
                let card = SettingsCard::new(
                    "lightning",
                    t("De energie-instellingen konden niet worden geladen.", "The power settings couldn’t be loaded."),
                    if err.message.is_empty() { unavailable_desc() } else { &err.message },
                );
                let retry = c::button(t("Opnieuw proberen", "Try again"), c::Variant::Standard);
                let pp = p.clone();
                retry.connect_clicked(move |_| pp.refresh());
                card.set_control(&retry);
                power_group.add(card.widget());
                if let Some(exp) = lid_and_button(&gs) {
                    power_group.add(exp.widget());
                }
            }
        }
        // a pc without a battery: Energiebesparing is a row of Energie (Windows 24H2)
        if !present && let Some(saver) = &saver_card {
            power_group.add(energy_saver(&gs, saver, false).widget());
        }
        out.push(power_group.widget().clone().upcast());

        // ---- Batterij: built once, shown while there is a battery; the figures below it follow the battery
        let battery_group = SettingsGroup::new(t("Batterij", "Battery"), "");
        battery_group.widget().set_widget_name("st-power-battery");
        if present && let Some(saver) = &saver_card {
            battery_group.add(energy_saver(&gs, saver, true).widget());
        }
        if let Some(card) = gs.toggle_card(
            &PERCENT,
            "battery",
            t("Batterijpercentage", "Battery percentage"),
            t("Het batterijpercentage naast het batterijpictogram in het systeemvak weergeven", "Show battery percentage next to battery icon in the system tray"),
            false,
        ) {
            battery_group.add(card.widget());
        }
        let ch = charging(&p, &cr.unwrap_or(Value::Null));
        battery_group.add(ch.exp.widget());
        // the radios' handlers hold it weakly: the page keeps it while it is shown
        p.on_cleanup(move || drop(ch));
        battery_group.add(levels(&p).widget());
        let info_slot = common::slot();
        battery_group.add(&info_slot);
        battery_group.widget().set_visible(present);
        out.push(battery_group.widget().clone().upcast());

        let update_battery = {
            let (energy, battery_slot, mains_slot, group, info_slot) = (energy.clone(), battery_slot.clone(), mains_slot.clone(), battery_group.widget().clone(), info_slot.clone());
            Rc::new(move |b: &Value| {
                let present = b["present"] == true;
                if let Some(e) = energy.borrow().as_ref() {
                    e.set_battery_present(present);
                }
                group.set_visible(present);
                if !present {
                    common::set_slot(&battery_slot, &[]);
                    common::set_slot(&info_slot, &[]);
                    let g = SettingsGroup::new(t("Batterij", "Battery"), "");
                    g.add(SettingsCard::new("lightning", t("Netstroom", "Mains power"), t("Deze computer heeft geen batterij en werkt altijd op netstroom.", "This computer has no battery and always runs on mains power.")).widget());
                    common::set_slot(&mains_slot, &[g.widget().clone().upcast()]);
                    return;
                }
                common::set_slot(&mains_slot, &[]);
                common::set_slot(&battery_slot, &[battery_hero(b)]);
                let mut cards: Vec<gtk::Widget> = Vec::new();
                if let Some(h) = b["health"].as_f64() {
                    let hv = h.round() as i64;
                    let word = if hv >= 80 {
                        t("Goed", "Good")
                    } else if hv >= 60 {
                        t("Redelijk", "Fair")
                    } else {
                        t("Versleten", "Worn")
                    };
                    let card = SettingsCard::new("heart", t("Batterijconditie", "Battery health"), t("Hoeveel de batterij nog kan opslaan vergeleken met nieuw", "How much the battery can still hold compared to new"));
                    card.set_control(&common::value_label(&format!("{hv}% · {word}")));
                    cards.push(card.widget().clone());
                }
                if let Some(cy) = b["cycles"].as_f64() {
                    let card = SettingsCard::new("refresh", t("Laadcycli", "Charge cycles"), t("Hoe vaak de batterij volledig is opgeladen", "How often the battery has been fully charged"));
                    card.set_control(&common::value_label(&kit::format::number(cy, 0)));
                    cards.push(card.widget().clone());
                }
                if let Some(rate) = b["energyRate"].as_f64() {
                    let card = SettingsCard::new("lightning", t("Huidig stroomverbruik", "Current power use"), t("Hoeveel energie de computer nu gebruikt", "How much energy the computer uses right now"));
                    card.set_control(&common::value_label(&format!("{} W", kit::format::number(rate, 1))));
                    cards.push(card.widget().clone());
                }
                let col = gtk::Box::new(gtk::Orientation::Vertical, 4);
                for w in &cards {
                    col.append(w);
                }
                let list: Vec<gtk::Widget> = if cards.is_empty() { Vec::new() } else { vec![col.upcast()] };
                common::set_slot(&info_slot, &list);
            })
        };
        update_battery(&b);
        {
            let ub = update_battery.clone();
            p.on("system.battery.changed", move |v| ub(v));
        }

        let actions = SettingsGroup::new(t("Aan/uit", "Shut down or sign out"), "");
        for a in ACTIONS {
            let card = SettingsCard::new(a.icon, t(a.title.0, a.title.1), t(a.desc.0, a.desc.1));
            let btn = c::button(t(a.title.0, a.title.1), c::Variant::Standard);
            btn.set_widget_name(&format!("power-{}", a.action));
            let pp = p.clone();
            btn.connect_clicked(move |_| power_action(&pp, a));
            card.set_control(&btn);
            actions.add(card.widget());
        }
        let (confirm_card, _) = common::toggle_card(
            &p,
            "shield",
            t("Altijd eerst vragen", "Always ask first"),
            t("Vragen voordat de computer wordt afgesloten, opnieuw opgestart of afgemeld", "Ask before the computer shuts down, restarts or signs out"),
            "power.confirm",
            false,
        );
        actions.add(confirm_card.widget());
        out.push(actions.widget().clone().upcast());
        out.push(mains_slot.upcast());
        out
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_power_button_offers_gnomes_values_in_windows_words() {
        let values = |o: Vec<(Value, String)>| o.into_iter().map(|(v, _)| v.as_str().unwrap_or("").to_owned()).collect::<Vec<_>>();
        assert_eq!(values(button_options(false, "interactive")), ["nothing", "suspend", "interactive"]);
        assert_eq!(values(button_options(true, "suspend")), ["nothing", "suspend", "hibernate", "interactive"]);
        assert_eq!(values(button_options(false, "hibernate")), ["nothing", "suspend", "hibernate", "interactive"], "a value already set stays visible");
    }

    #[test]
    fn energy_saver_is_the_power_saver_profile() {
        let three = json!({"available": true, "writable": true, "active": "balanced",
            "choices": [{"id": "power-saver"}, {"id": "balanced"}, {"id": "performance"}]});
        assert!(saver_offered(&three));
        assert_eq!(saver_profile(true, &three), "power-saver");
        assert_eq!(saver_profile(false, &three), "balanced");
        let two = json!({"available": true, "choices": [{"id": "power-saver"}, {"id": "performance"}]});
        assert_eq!(saver_profile(false, &two), "performance");
        assert!(!saver_offered(&json!({"available": false, "choices": []})));
        assert!(!saver_offered(&json!({"available": true, "choices": [{"id": "power-saver", "disabled": true}]})));
    }

    #[test]
    fn gnomes_warning_shows_while_automatic_sleep_is_off() {
        let state = json!({"canSuspend": true, "sleepAC": {"available": true, "action": "nothing", "timeout": 0},
            "sleepBattery": {"available": true, "action": "suspend", "timeout": 900}});
        assert!(suspend_off(&state, false), "a desktop: plugged in");
        assert!(!suspend_off(&state, true), "a laptop: on battery");
        let off = json!({"canSuspend": true, "sleepBattery": {"available": true, "action": "suspend", "timeout": 0}});
        assert!(suspend_off(&off, true));
        assert!(!suspend_off(&json!({}), true));
        // the harness's session without logind: the sleep rows say it can't sleep, no warning on top of that
        let cannot = json!({"canSuspend": false, "sleepAC": {"available": true, "action": "nothing", "timeout": 0}});
        assert!(!suspend_off(&cannot, false));
    }

    #[test]
    fn battery_levels_are_sampled_from_the_right() {
        let points = [(100.0, 80.0), (160.0, 70.0), (400.0, 50.0)];
        // 500 s on a grid of 6: 0, 100, ..., 500 → from the first sample at 100 on
        assert_eq!(resample(&points, 500.0, 500.0, 6), [80.0, 70.0, 70.0, 50.0, 50.0]);
        assert_eq!(resample(&[], 500.0, 500.0, 6), Vec::<f64>::new());
        assert_eq!(resample(&[(0.0, 120.0)], 10.0, 10.0, 2), [100.0, 100.0], "clamped to 100 %");
        assert_eq!(gs::control_name("power", &keys(), &BUTTON), "st-power-power-button-action");
    }
}
