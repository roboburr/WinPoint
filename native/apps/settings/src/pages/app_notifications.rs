//! Instellingen › Systeem › Meldingen › <app> (a data page, `{id: desktop id}`, notes/claude/settings-parity.md §2):
//! Windows 11's page of one app's notifications with GNOME Settings 50's per-app settings
//! (cc-app-notifications-dialog.c), the relocatable `org.gnome.desktop.notifications.application` at
//! `/org/gnome/desktop/notifications/application/<canonical id>/`:
//!
//! * Meldingen (`enable`); every other row is greyed while it is off, as in both.
//! * Meldingsbanners weergeven (`show-banners`) with Inhoud weergeven in banners (`force-expanded`, GNOME's Show
//!   Content), the two checkboxes of Windows' banner card.
//! * Meldingen weergeven op het vergrendelingsscherm (`show-in-lock-screen`) and Inhoud verbergen wanneer meldingen
//!   op het vergrendelingsscherm staan (`details-in-lock-screen` inverted), greyed while the lock screen shows no
//!   notifications at all (Meldingen' own row).
//! * Een geluid afspelen wanneer er een melding binnenkomt (`enable-sound-alerts`).
//!
//! The keys are read and followed through `gsettings.get` (crate::gs) and written through `notifications.app.set`
//! (amendment 6: the first write also records the app's `application-id` and `application-children`).
use super::{Builder, Content, Page, PageFuture};
use crate::common;
use crate::gs::{self, GKey, Gs};
use kit::controls::{self as c, SettingsCard, SettingsGroup, ToggleSwitch};
use kit::prelude::*;
use kit::t;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

pub const PAGES: &[(&str, Builder)] = &[("app-notifications", app_notifications)];
pub const CSS: &str = ".st-app-notif-checks { padding: 4px 0; }\n";

pub(super) const APP_SCHEMA: &str = "org.gnome.desktop.notifications.application";
const APP_PREFIX: &str = "/org/gnome/desktop/notifications/application/";
const LOCK: GKey = gs::key("org.gnome.desktop.notifications", "show-in-lock-screen");
const KEYS: &[&str] = &["enable", "show-banners", "force-expanded", "show-in-lock-screen", "details-in-lock-screen", "enable-sound-alerts"];

// ------------------------------------------------------------------------------------------- pure parts

/// GNOME Shell's canonical id of an app (host/src/services/notifications.rs `canonical_id`): the desktop id
/// without ".desktop" in lower case, every other character than `[a-z0-9-]` a '-', never two '-' in a row.
pub(super) fn canonical_id(desktop_id: &str) -> String {
    let stem = desktop_id.strip_suffix(".desktop").unwrap_or(desktop_id);
    let mut out = String::new();
    for c in stem.chars().flat_map(char::to_lowercase) {
        let c = if c.is_ascii_lowercase() || c.is_ascii_digit() { c } else { '-' };
        if c == '-' && out.ends_with('-') {
            continue;
        }
        out.push(c);
    }
    out
}

/// Which rows can be changed (GNOME's sensitivity): everything needs Meldingen; Inhoud weergeven in banners needs
/// the banners; the lock-screen rows need the lock screen to show notifications (Meldingen' own row), and the
/// content row the app's lock-screen row too.
pub(super) fn sensitive(key: &str, values: &HashMap<String, bool>, lock_global: bool) -> bool {
    let on = |k: &str| values.get(k).copied().unwrap_or(true);
    if key == "enable" {
        return true;
    }
    if !on("enable") {
        return false;
    }
    match key {
        "force-expanded" => on("show-banners"),
        "show-in-lock-screen" => lock_global,
        "details-in-lock-screen" => lock_global && on("show-in-lock-screen"),
        _ => true,
    }
}

/// The desktop id with ".desktop".
fn full_id(id: &str) -> String {
    if id.ends_with(".desktop") { id.to_owned() } else { format!("{id}.desktop") }
}

// ------------------------------------------------------------------------------------------------ controls

/// The key's value a control stands for: a switch shows !value when `invert` (Inhoud verbergen ...).
pub(super) fn key_value(shown: bool, invert: bool) -> bool {
    shown != invert
}

/// A switch for one of an app's keys, written through `notifications.app.set` (optimistic, reverted with the
/// error toast). `invert`: the switch shows !value (Inhoud verbergen ...). `changed(value)` hears the key's value
/// the switch stands for: at once on a click, and the old value again when the write is refused (the revert is
/// silent, so a page that keeps an optimistic copy of the values learns it only here).
pub(super) fn app_switch(
    p: &Page,
    app_id: &str,
    key: &'static str,
    value: bool,
    invert: bool,
    writable: bool,
    changed: impl Fn(bool) + 'static,
) -> ToggleSwitch {
    let sw = ToggleSwitch::new(value != invert);
    sw.set_sensitive(writable);
    if !writable {
        sw.switch().set_tooltip_text(Some(gs::locked_text()));
    }
    let (pp, sw2, id, changed) = (p.clone(), sw.clone(), app_id.to_owned(), Rc::new(changed));
    sw.connect_toggled(move |on| {
        changed(key_value(on, invert));
        let fut = pp.run("notifications.app.set", json!({ "appId": id, "values": { key: key_value(on, invert) } }));
        let (sw3, changed) = (sw2.clone(), changed.clone());
        pp.spawn(async move {
            if fut.await.is_none() {
                sw3.set_active(!on);
                changed(key_value(!on, invert));
            }
        });
    });
    sw
}

/// A check box for one of an app's keys, written through `notifications.app.set`; `changed` as [`app_switch`].
fn app_checkbox(
    p: &Page,
    app_id: &str,
    key: &'static str,
    label: &str,
    value: bool,
    writable: bool,
    changed: impl Fn(bool) + 'static,
) -> (gtk::CheckButton, Rc<std::cell::Cell<bool>>) {
    let cb = c::checkbox(label);
    cb.set_active(value);
    cb.set_sensitive(writable);
    if !writable {
        cb.set_tooltip_text(Some(gs::locked_text()));
    }
    let silent = Rc::new(std::cell::Cell::new(false));
    let (pp, id, s2, changed) = (p.clone(), app_id.to_owned(), silent.clone(), Rc::new(changed));
    cb.connect_toggled(move |b| {
        if s2.get() {
            return;
        }
        let on = b.is_active();
        changed(on);
        let fut = pp.run("notifications.app.set", json!({ "appId": id, "values": { key: on } }));
        let (b2, s3, changed) = (b.clone(), s2.clone(), changed.clone());
        pp.spawn(async move {
            if fut.await.is_none() {
                s3.set(true);
                b2.set_active(!on);
                s3.set(false);
                changed(!on);
            }
        });
    });
    (cb, silent)
}

// ------------------------------------------------------------------------------------------------ the page

fn no_app(p: &Page) -> Content {
    let g = SettingsGroup::new("", t("Kies een app in Meldingen.", "Choose an app in Notifications."));
    g.add(common::link_card(p, "notifications").widget());
    vec![g.widget().clone().upcast()]
}

/// One row of the page: a switch (with its invert) or a check box (with its guard).
enum Control {
    Switch(ToggleSwitch, bool),
    Check(gtk::CheckButton, Rc<std::cell::Cell<bool>>),
}

fn app_notifications(p: Page) -> PageFuture {
    Box::pin(async move {
        let Some(raw) = p.param("id").filter(|s| !s.is_empty()) else {
            return no_app(&p);
        };
        let id = full_id(&raw);
        let (apps, names) = (p.call("notifications.apps", json!({})), p.call("apps.list", json!({})));
        let master = Gs::load(&p, &[LOCK]).await;
        let (apps, names) = (apps.await, names.await);
        if !p.alive() {
            return Content::new();
        }
        let listed = apps.ok().and_then(|v| v["apps"].as_array().and_then(|a| a.iter().find(|x| x["id"] == id.as_str()).cloned()));
        let names: Vec<Value> = names.ok().and_then(|v| v.as_array().cloned()).unwrap_or_default();
        let named = names.iter().find(|a| a["id"] == id.as_str()).cloned();
        if listed.is_none() && named.is_none() {
            let mut out = vec![common::notice(t("Deze app is niet gevonden.", "This app wasn’t found.")).widget().clone().upcast()];
            out.extend(no_app(&p));
            return out;
        }
        let name = named
            .as_ref()
            .and_then(|a| a["name"].as_str())
            .filter(|n| !n.is_empty())
            .or_else(|| listed.as_ref().and_then(|a| a["name"].as_str()))
            .unwrap_or(&id)
            .to_owned();
        p.set_title(&name);
        let canonical = listed.as_ref().and_then(|a| a["canonical"].as_str().map(str::to_owned)).unwrap_or_else(|| canonical_id(&id));
        let path = format!("{APP_PREFIX}{canonical}/");
        let keys: Vec<GKey> = KEYS.iter().map(|k| gs::at(APP_SCHEMA, &path, k)).collect();
        let gs = Gs::load(&p, &keys).await;
        if !p.alive() {
            return Content::new();
        }
        // Gs follows gsettings.changed through a weak reference: the page keeps it while it is shown
        let keep = gs.clone();
        p.on_cleanup(move || drop(keep));
        let values: Rc<RefCell<HashMap<String, bool>>> =
            Rc::new(RefCell::new(KEYS.iter().zip(&keys).map(|(k, gk)| ((*k).to_owned(), gs.value(gk) != false)).collect()));
        let writable: Vec<bool> = keys.iter().map(|gk| gs.available(gk) && gs.item(gk)["writable"] != false).collect();
        let icon = named
            .as_ref()
            .and_then(|a| a["icon"].as_str())
            .or_else(|| listed.as_ref().and_then(|a| a["icon"].as_str()))
            .filter(|s| !s.is_empty() && !s.starts_with('/'))
            .unwrap_or("application-x-executable")
            .to_owned();
        let v = |k: &str| values.borrow().get(k).copied().unwrap_or(true);
        let mut controls: Vec<(&'static str, Control)> = Vec::new();
        // the page's own writes: the optimistic value at once, the old one back when a write is refused (before
        // gsettings.changed, which a refused write never sends); then the sensitivity again
        let sync_later: Rc<std::cell::OnceCell<Rc<dyn Fn()>>> = Rc::default();
        let note = |key: &'static str| {
            let (values, sync_later) = (values.clone(), sync_later.clone());
            move |value: bool| {
                values.borrow_mut().insert(key.to_owned(), value);
                if let Some(sync) = sync_later.get() {
                    sync();
                }
            }
        };

        // ---- Meldingen
        let enable = app_switch(&p, &id, "enable", v("enable"), false, writable[0], note("enable"));
        enable.switch().set_widget_name(&gs::control_name(p.id(), &keys, &keys[0]));
        let head = SettingsCard::new(&icon, t("Meldingen", "Notifications"), t("Meldingen van deze app ontvangen", "Get notifications from this app"));
        head.set_control(enable.widget());
        controls.push(("enable", Control::Switch(enable, false)));

        // ---- the banner card: Meldingsbanners weergeven, Inhoud weergeven in banners
        let checks = gtk::Box::new(gtk::Orientation::Vertical, 4);
        checks.add_css_class("st-app-notif-checks");
        for (i, key, label) in [
            (1usize, "show-banners", t("Meldingsbanners weergeven", "Show notification banners")),
            (2, "force-expanded", t("Inhoud weergeven in banners", "Show content in banners")),
        ] {
            let (cb, silent) = app_checkbox(&p, &id, key, label, v(key), writable[i], note(key));
            cb.set_widget_name(&gs::control_name(p.id(), &keys, &keys[i]));
            checks.append(&cb);
            controls.push((key, Control::Check(cb, silent)));
        }
        let banners = SettingsCard::new(
            "notification",
            t("Meldingsbanners", "Notification banners"),
            t("Meldingen rechtsonder in beeld, boven de taakbalk", "Notifications at the bottom right, above the taskbar"),
        );
        common::card_extra(&banners, &checks);

        // ---- the lock screen and the sound
        let mut cards = Vec::new();
        for (i, key, icon, title, invert) in [
            (3usize, "show-in-lock-screen", "lock", t("Meldingen weergeven op het vergrendelingsscherm", "Show notifications on the lock screen"), false),
            (
                4,
                "details-in-lock-screen",
                "lock",
                t("Inhoud verbergen wanneer meldingen op het vergrendelingsscherm staan", "Hide content when notifications are on the lock screen"),
                true,
            ),
            (5, "enable-sound-alerts", "volume", t("Een geluid afspelen wanneer er een melding binnenkomt", "Play a sound when a notification arrives"), false),
        ] {
            let sw = app_switch(&p, &id, key, v(key), invert, writable[i], note(key));
            sw.switch().set_widget_name(&gs::control_name(p.id(), &keys, &keys[i]));
            let card = SettingsCard::new(icon, title, "");
            card.set_control(sw.widget());
            cards.push(card);
            controls.push((key, Control::Switch(sw, invert)));
        }

        // sensitivity and the description lines, now and after every change (also one made elsewhere)
        let controls = Rc::new(controls);
        let sync: Rc<dyn Fn()> = {
            let (values, controls, master) = (values.clone(), controls.clone(), master.clone());
            let (lock_card, sound_card) = (cards[0].clone(), cards[2].clone());
            let writable = writable.clone();
            Rc::new(move || {
                let lock_global = master.value(&LOCK) != false;
                let vals = values.borrow().clone();
                for (key, control) in controls.iter() {
                    let index = KEYS.iter().position(|k| k == key).unwrap_or(0);
                    let on = sensitive(key, &vals, lock_global) && writable[index];
                    match control {
                        Control::Switch(sw, _) => sw.set_sensitive(on),
                        Control::Check(cb, _) => cb.set_sensitive(on),
                    }
                }
                lock_card.set_description(if lock_global {
                    ""
                } else {
                    t("Meldingen op het vergrendelingsscherm staan uit voor alle apps", "Notifications on the lock screen are turned off for all apps")
                });
                sound_card.set_description(if common::setting_bool("notifications.sounds", true) {
                    ""
                } else {
                    t("Meldingsgeluiden staan uit in Meldingen", "Notification sounds are turned off in Notifications")
                });
            })
        };
        sync();
        let _ = sync_later.set(sync.clone());
        for (key, gk) in KEYS.iter().zip(&keys) {
            let (values, controls, sync, key) = (values.clone(), controls.clone(), sync.clone(), *key);
            gs.follow(gk, move |value| {
                let on = *value != false;
                values.borrow_mut().insert(key.to_owned(), on);
                if let Some((_, control)) = controls.iter().find(|(k, _)| *k == key) {
                    match control {
                        Control::Switch(sw, invert) => sw.set_active(on != *invert),
                        Control::Check(cb, silent) => {
                            silent.set(true);
                            cb.set_active(on);
                            silent.set(false);
                        }
                    }
                }
                sync();
            });
        }
        {
            let sync = sync.clone();
            master.follow(&LOCK, move |_| sync());
        }
        p.on_settings(move |_, _| sync());

        let g = SettingsGroup::new("", "");
        g.add(head.widget());
        g.add(banners.widget());
        for card in &cards {
            g.add(card.widget());
        }
        let related = SettingsGroup::new(common::ct::related(), "");
        related.add(common::link_card(&p, "notifications").widget());
        if named.is_some() {
            let (pp, id2) = (p.clone(), id.clone());
            let card = SettingsCard::clickable(
                "apps",
                t("Geavanceerde opties", "Advanced options"),
                t("Machtigingen, opslag en standaardkoppelingen van deze app", "This app’s permissions, storage and default links"),
                c::Chevron::Right,
            );
            if let Some(b) = card.button() {
                b.set_widget_name("st-app-notifications-advanced");
            }
            card.connect_clicked(move || pp.go_with("app-advanced", json!({ "id": id2 })));
            related.add(card.widget());
        }
        vec![g.widget().clone().upcast(), related.widget().clone().upcast()]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_ids_match_the_host() {
        assert_eq!(canonical_id("org.gnome.Nautilus.desktop"), "org-gnome-nautilus");
        assert_eq!(canonical_id("firefox_firefox.desktop"), "firefox-firefox");
        assert_eq!(canonical_id("snap.app--x"), "snap-app-x");
        assert_eq!(full_id("org.gnome.Nautilus"), "org.gnome.Nautilus.desktop");
    }

    #[test]
    fn a_control_stands_for_its_keys_value() {
        // Inhoud verbergen ... shows details-in-lock-screen inverted; a refused write reverts the shown state, and
        // the page's copy gets the key's old value back (not the shown one)
        assert!(key_value(true, false) && !key_value(false, false));
        assert!(!key_value(true, true) && key_value(false, true));
        for (on, invert) in [(true, false), (false, false), (true, true), (false, true)] {
            assert_ne!(key_value(on, invert), key_value(!on, invert), "the revert restores the other value");
        }
    }

    #[test]
    fn rows_grey_out_as_gnome_settings() {
        let all_on: HashMap<String, bool> = KEYS.iter().map(|k| ((*k).to_owned(), true)).collect();
        for k in KEYS {
            assert!(sensitive(k, &all_on, true), "{k}");
        }
        let mut off = all_on.clone();
        off.insert("enable".into(), false);
        assert!(sensitive("enable", &off, true));
        for k in &KEYS[1..] {
            assert!(!sensitive(k, &off, true), "{k} needs Meldingen");
        }
        let mut no_banners = all_on.clone();
        no_banners.insert("show-banners".into(), false);
        assert!(!sensitive("force-expanded", &no_banners, true));
        assert!(sensitive("enable-sound-alerts", &no_banners, true));
        assert!(!sensitive("show-in-lock-screen", &all_on, false), "the lock screen shows no notifications at all");
        assert!(!sensitive("details-in-lock-screen", &all_on, false));
        let mut no_lock = all_on.clone();
        no_lock.insert("show-in-lock-screen".into(), false);
        assert!(!sensitive("details-in-lock-screen", &no_lock, true));
    }
}
