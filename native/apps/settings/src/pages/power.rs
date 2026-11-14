//! Instellingen › Systeem › Energie en batterij (last-webkit:vendor/ui/js/apps/settings/pages/power.js): GNOME's own
//! power settings (power mode, screen off, dimming, automatic sleep), the battery and the confirmed power actions.
use super::{Builder, Content, Page, PageFuture};
use crate::common::{self, UsageBar};
use kit::controls::{self as c, Dropdown, DropdownOption, SettingsCard, SettingsGroup, ToggleSwitch};
use kit::prelude::*;
use kit::t;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

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
}

impl Energy {
    fn set_all_sensitive(&self, on: bool) {
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

fn energy_settings(p: &Page, initial: Value, battery: bool) -> (Rc<Energy>, Vec<gtk::Widget>) {
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
    let g1 = SettingsGroup::new(t("Energie", "Power"), "");
    g1.add(profile_card.widget());
    let g2 = SettingsGroup::new(t("Scherm en slaapstand", "Screen & sleep"), "");
    g2.add(screen_card.widget());
    g2.add(dim_card.widget());
    g2.add(sleep_ac_card.widget());
    g2.add(sleep_battery_card.widget());
    (e, vec![g1.widget().clone().upcast(), g2.widget().clone().upcast()])
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
        let (br, er) = (p.call("system.battery.get", json!({})), p.call("system.power.settings.get", json!({})));
        let (br, er) = (br.await, er.await);
        if !p.alive() {
            return Content::new();
        }
        let b = br.ok().unwrap_or(Value::Null);
        let mut out: Content = Vec::new();
        let battery_slot = common::slot();
        let mains_slot = common::slot();
        out.push(battery_slot.clone().upcast());
        let q = queued.borrow_mut().take();
        match (er, q) {
            (_, Some(v)) | (Ok(v), None) => {
                let (e, groups) = energy_settings(&p, v, b["present"] == true);
                *energy.borrow_mut() = Some(e);
                out.extend(groups);
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
                let g = SettingsGroup::new(t("Energie", "Power"), "");
                g.add(card.widget());
                out.push(g.widget().clone().upcast());
            }
        }

        let update_battery = {
            let (energy, battery_slot, mains_slot) = (energy.clone(), battery_slot.clone(), mains_slot.clone());
            Rc::new(move |b: &Value| {
                let present = b["present"] == true;
                if let Some(e) = energy.borrow().as_ref() {
                    e.set_battery_present(present);
                }
                if !present {
                    common::set_slot(&battery_slot, &[]);
                    let g = SettingsGroup::new(t("Batterij", "Battery"), "");
                    g.add(SettingsCard::new("lightning", t("Netstroom", "Mains power"), t("Deze computer heeft geen batterij en werkt altijd op netstroom.", "This computer has no battery and always runs on mains power.")).widget());
                    common::set_slot(&mains_slot, &[g.widget().clone().upcast()]);
                    return;
                }
                common::set_slot(&mains_slot, &[]);
                let mut blocks: Vec<gtk::Widget> = vec![battery_hero(b)];
                let g = SettingsGroup::new(t("Batterij", "Battery"), "");
                let mut any = false;
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
                    g.add(card.widget());
                    any = true;
                }
                if let Some(cy) = b["cycles"].as_f64() {
                    let card = SettingsCard::new("refresh", t("Laadcycli", "Charge cycles"), t("Hoe vaak de batterij volledig is opgeladen", "How often the battery has been fully charged"));
                    card.set_control(&common::value_label(&kit::format::number(cy, 0)));
                    g.add(card.widget());
                    any = true;
                }
                if let Some(rate) = b["energyRate"].as_f64() {
                    let card = SettingsCard::new("lightning", t("Huidig stroomverbruik", "Current power use"), t("Hoeveel energie de computer nu gebruikt", "How much energy the computer uses right now"));
                    card.set_control(&common::value_label(&format!("{} W", kit::format::number(rate, 1))));
                    g.add(card.widget());
                    any = true;
                }
                if any {
                    blocks.push(g.widget().clone().upcast());
                }
                common::set_slot(&battery_slot, &blocks);
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
