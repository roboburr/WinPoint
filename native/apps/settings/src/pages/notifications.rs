//! Instellingen › Systeem › Meldingen (6ebf169f:vendor/ui/js/apps/settings/pages/notifications.js) as Windows 11
//! lays it out, with GNOME Settings 50's Notifications panel (settings parity, batch F, amendments 4-6):
//!
//! * Meldingen (WinPoint's toasts, settings.json `notifications.showToasts`), with inside it "Meldingen weergeven
//!   op het vergrendelingsscherm" (org.gnome.desktop.notifications `show-in-lock-screen`) and "Toestaan dat
//!   meldingen geluiden afspelen" (settings.json `notifications.sounds`, which notificationsBridge.js honours).
//! * Niet storen: GNOME's Do Not Disturb, `show-banners` inverted (the mirror keeps `notifications.dnd` equal for
//!   the shell, the flyout and Start).
//! * Meldingen van apps en andere afzenders: GNOME's App Notifications (`notifications.apps`), sorted on Meest
//!   recent (the notification centre's newest per app) or Naam; per app a switch (its `enable`, written through
//!   `notifications.app.set`) and the app's own page (`app-notifications {id}`).
//! * Testmelding versturen (WinPoint's own).
use super::app_notifications::{APP_SCHEMA, app_switch};
use super::background::{remember, remembered};
use super::{Builder, Content, Page, PageFuture};
use crate::common;
use crate::gs::{self, GKey, Gs};
use kit::controls::{self as c, Chevron, Dropdown, DropdownOption, SettingsCard, SettingsExpander, SettingsGroup, Severity, Toast};
use kit::prelude::*;
use kit::t;
use std::collections::HashMap;
use std::rc::Rc;

pub const PAGES: &[(&str, Builder)] = &[("notifications", notifications)];
pub const CSS: &str = "";

const LOCK: GKey = gs::key("org.gnome.desktop.notifications", "show-in-lock-screen");
const BANNERS: GKey = gs::key("org.gnome.desktop.notifications", "show-banners");
/// gsettings.get takes at most 256 keys: the list follows the first 80 apps (three keys each).
const FOLLOWED: usize = 80;

// ------------------------------------------------------------------------------------------- pure parts

/// The newest notification time per desktop id in the notification centre's list.
pub(super) fn latest_per_app(items: &[Value]) -> HashMap<String, f64> {
    let mut out: HashMap<String, f64> = HashMap::new();
    for n in items {
        let Some(id) = n["desktopId"].as_str().filter(|s| !s.is_empty()) else { continue };
        let id = if id.ends_with(".desktop") { id.to_owned() } else { format!("{id}.desktop") };
        let time = n["time"].as_f64().unwrap_or(0.0);
        let slot = out.entry(id).or_insert(time);
        if time > *slot {
            *slot = time;
        }
    }
    out
}

/// The apps in Windows' order: Naam (by name), or Meest recent (the newest notification first, the rest by name).
pub(super) fn sorted(apps: &[Value], latest: &HashMap<String, f64>, by_name: bool) -> Vec<Value> {
    let mut out = apps.to_vec();
    let name = |a: &Value| common::s(&a["name"]).to_lowercase();
    out.sort_by(|a, b| {
        let by = name(a).cmp(&name(b));
        if by_name {
            return by;
        }
        let (ta, tb) = (latest.get(common::s(&a["id"])), latest.get(common::s(&b["id"])));
        match (ta, tb) {
            (Some(x), Some(y)) => y.partial_cmp(x).unwrap_or(std::cmp::Ordering::Equal).then(by),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => by,
        }
    });
    out
}

/// Windows' line under an app: "Banners, Geluiden" (what reaches you), nothing while it is off.
pub(super) fn summary(enabled: bool, banners: bool, sounds: bool) -> String {
    if !enabled {
        return String::new();
    }
    let mut parts = Vec::new();
    if banners {
        parts.push(t("Banners", "Banners"));
    }
    if sounds {
        parts.push(t("Geluiden", "Sounds"));
    }
    parts.join(", ")
}

/// Each app's name as apps.list gives it (WinPoint's language); the host's own name otherwise.
fn with_list_names(apps: &mut [Value], list: &[Value]) {
    for a in apps.iter_mut() {
        if let Some(name) = list.iter().find(|l| l["id"] == a["id"]).and_then(|l| l["name"].as_str()).filter(|n| !n.is_empty()) {
            a["name"] = json!(name);
        }
    }
}

// ------------------------------------------------------------------------------------------------ the page

fn notifications(p: Page) -> PageFuture {
    Box::pin(async move {
        let (lst, apps, names) =
            (p.call("notifications.list", json!({})), p.call("notifications.apps", json!({})), p.call("apps.list", json!({})));
        let master = Gs::load(&p, &[LOCK, BANNERS]).await;
        let (lst, apps, names) = (lst.await, apps.await, names.await);
        if !p.alive() {
            return Content::new();
        }
        let supported = match &lst {
            Ok(v) => v["supported"] != false,
            Err(_) => true,
        };
        let items: Vec<Value> = lst.as_ref().ok().and_then(|v| v["items"].as_array().cloned()).unwrap_or_default();
        let mut apps: Vec<Value> = apps.ok().and_then(|v| v["apps"].as_array().cloned()).unwrap_or_default();
        let names: Vec<Value> = names.ok().and_then(|v| v.as_array().cloned()).unwrap_or_default();
        with_list_names(&mut apps, &names);

        // ---- Meldingen, with the lock screen and the sounds inside (amendment 5)
        let sounds = c::checkbox(t("Toestaan dat meldingen geluiden afspelen", "Allow notifications to play sounds"));
        sounds.set_active(common::setting_bool("notifications.sounds", true));
        sounds.set_widget_name("notifications-sounds");
        let silent = Rc::new(std::cell::Cell::new(false));
        {
            let (pp, s2, silent) = (p.clone(), sounds.clone(), silent.clone());
            sounds.connect_toggled(move |b| {
                if silent.get() {
                    return;
                }
                let on = b.is_active();
                let (s3, silent) = (s2.clone(), silent.clone());
                common::set_setting(&pp, "notifications.sounds", json!(on), move |ok| {
                    if !ok {
                        silent.set(true);
                        s3.set_active(!on);
                        silent.set(false);
                    }
                });
            });
        }
        let toasts = common::setting_toggle(&p, "notifications.showToasts", false);
        toasts.switch().set_widget_name("notifications-toggle");
        let exp = SettingsExpander::new(
            "notification",
            t("Meldingen", "Notifications"),
            t("Meldingen ontvangen van apps en andere afzenders", "Get notifications from apps and other senders"),
        );
        exp.header().set_control(toasts.widget());
        if master.available(&LOCK) {
            exp.add(&master.checkbox(&LOCK, t("Meldingen weergeven op het vergrendelingsscherm", "Show notifications on the lock screen"), false));
        }
        exp.add(&sounds);
        exp.set_expanded(true);

        // ---- Niet storen: GNOME's Do Not Disturb (show-banners inverted, mirrored to notifications.dnd)
        let dnd_title = t("Niet storen", "Do not disturb");
        let dnd_desc = t("Meldingen worden rechtstreeks naar het meldingencentrum gestuurd", "Notifications will go directly to notification centre");
        let dnd_card = match master.toggle_card(&BANNERS, "notification-off", dnd_title, dnd_desc, true) {
            Some(card) => card,
            None => {
                let (card, sw) = common::toggle_card(&p, "notification-off", dnd_title, dnd_desc, "notifications.dnd", false);
                sw.switch().set_widget_name("notifications-dnd");
                card
            }
        };
        let dnd_note = common::notice(t("Niet storen staat aan: je ziet geen meldingen in beeld.", "Do not disturb is on: notifications won’t pop up."));
        let dnd_on = {
            let master = master.clone();
            move || if master.available(&BANNERS) { master.value(&BANNERS) == false } else { common::setting_bool("notifications.dnd", false) }
        };
        dnd_note.widget().set_visible(dnd_on());
        {
            let (note, on) = (dnd_note.clone(), dnd_on.clone());
            master.follow(&BANNERS, move |_| note.widget().set_visible(on()));
        }
        {
            let (note, s2) = (dnd_note.clone(), sounds.clone());
            p.on_settings(move |_, _| {
                note.widget().set_visible(dnd_on());
                silent.set(true);
                s2.set_active(common::setting_bool("notifications.sounds", true));
                silent.set(false);
            });
        }

        let mut out: Content = Vec::new();
        if !supported {
            out.push(
                common::notice(t(
                    "Onder GNOME toont GNOME zelf de meldingen. Deze instellingen gelden voor WinPoint.",
                    "In GNOME, notifications are shown by GNOME itself. These settings apply to WinPoint.",
                ))
                .widget()
                .clone()
                .upcast(),
            );
        }
        let g1 = SettingsGroup::new("", "");
        g1.add(exp.widget());
        g1.add(dnd_card.widget());
        g1.add(dnd_note.widget());
        out.push(g1.widget().clone().upcast());
        if let Some(g) = apps_group(&p, &apps, &items).await {
            out.push(g.widget().clone().upcast());
        }
        out.push(test_group(&p).widget().clone().upcast());
        out
    })
}

/// "Meldingen van apps en andere afzenders": Sorteren op, then one row per app.
async fn apps_group(p: &Page, apps: &[Value], items: &[Value]) -> Option<SettingsGroup> {
    if apps.is_empty() {
        return None;
    }
    let keys: Vec<GKey> = apps
        .iter()
        .take(FOLLOWED)
        .flat_map(|a| {
            let path = common::s(&a["path"]);
            [gs::at(APP_SCHEMA, path, "enable"), gs::at(APP_SCHEMA, path, "show-banners"), gs::at(APP_SCHEMA, path, "enable-sound-alerts")]
        })
        .collect();
    let gs = Gs::load(p, &keys).await;
    if !p.alive() {
        return None;
    }
    // Gs follows gsettings.changed through a weak reference: the page keeps it while it is shown
    let keep = gs.clone();
    p.on_cleanup(move || drop(keep));
    let g = SettingsGroup::new(t("Meldingen van apps en andere afzenders", "Notifications from apps and other senders"), "");
    let by_name = remembered("notificationsSort").first().map(String::as_str) == Some("name");
    let sort = Dropdown::new(
        vec![DropdownOption::new("recent", t("Meest recent", "Most recent")), DropdownOption::new("name", t("Naam", "Name"))],
        Some(if by_name { "name" } else { "recent" }),
    );
    sort.widget().set_widget_name("notifications-sort");
    let sort_card = SettingsCard::new("", t("Sorteren op", "Sort by"), "");
    sort_card.set_control(&common::sized(sort.widget(), 180));
    let rows: Rc<HashMap<String, gtk::Widget>> = Rc::new(apps.iter().map(|a| (common::s(&a["id"]).to_owned(), app_row(p, &gs, &keys, a))).collect());
    let latest = Rc::new(latest_per_app(items));
    let apps = Rc::new(apps.to_vec());
    let fill = {
        let (g, sort_card, rows, apps, latest) = (g.clone(), sort_card.clone(), rows.clone(), apps.clone(), latest.clone());
        move |by_name: bool| {
            g.clear();
            g.add(sort_card.widget());
            for a in sorted(&apps, &latest, by_name) {
                if let Some(row) = rows.get(common::s(&a["id"])) {
                    g.add(row);
                }
            }
        }
    };
    fill(by_name);
    sort.connect_changed(move |v| {
        remember("notificationsSort", vec![v.to_owned()]);
        fill(v == "name");
    });
    Some(g)
}

/// One app: its icon and name, Windows' summary line, the switch and the way to its own page.
fn app_row(p: &Page, gs: &Gs, keys: &[GKey], app: &Value) -> gtk::Widget {
    let (id, path, canonical) = (common::s(&app["id"]).to_owned(), common::s(&app["path"]).to_owned(), common::s(&app["canonical"]).to_owned());
    let icon = app["icon"].as_str().filter(|s| !s.is_empty() && !s.starts_with('/')).unwrap_or("application-x-executable");
    let value = |k: &str| app["values"][k] != false;
    let card = SettingsCard::clickable(icon, common::s(&app["name"]), &summary(value("enable"), value("show-banners"), value("enable-sound-alerts")), Chevron::Right);
    if let Some(b) = card.button() {
        b.set_widget_name(&format!("notifications-app-{canonical}"));
    }
    {
        let (pp, id) = (p.clone(), id.clone());
        card.connect_clicked(move || pp.go_with("app-notifications", json!({ "id": id })));
    }
    let enable = gs::at(APP_SCHEMA, &path, "enable");
    let followed = keys.contains(&enable);
    let locked = app["locked"].as_array().is_some_and(|l| l.iter().any(|k| k == "enable"));
    // the summary line follows gsettings.changed only, which a refused write never sends: nothing to put back
    let sw = app_switch(p, &id, "enable", value("enable"), false, !locked, |_| {});
    sw.switch().set_widget_name(&gs::control_name(p.id(), keys, &enable));
    card.set_control(sw.widget());
    if followed {
        let state = Rc::new(std::cell::RefCell::new([value("enable"), value("show-banners"), value("enable-sound-alerts")]));
        for (i, key) in ["enable", "show-banners", "enable-sound-alerts"].into_iter().enumerate() {
            let (card, sw, state) = (card.clone(), sw.clone(), state.clone());
            gs.follow(&gs::at(APP_SCHEMA, &path, key), move |v| {
                let mut s = state.borrow_mut();
                s[i] = *v != false;
                if i == 0 {
                    sw.set_active(s[0]);
                }
                card.set_description(&summary(s[0], s[1], s[2]));
            });
        }
    }
    card.widget().clone()
}

/// Testmelding versturen (WinPoint's own).
fn test_group(p: &Page) -> SettingsGroup {
    let send = c::button(t("Versturen", "Send"), c::Variant::Standard);
    send.set_widget_name("notifications-test");
    let pp = p.clone();
    send.connect_clicked(move |b| {
        b.set_sensitive(false);
        let (pp, b) = (pp.clone(), b.clone());
        let wait = pp.call("notifications.test", json!({}));
        pp.clone().spawn(async move {
            let r = wait.await;
            b.set_sensitive(true);
            if r.is_ok() {
                pp.toast(Toast::new(t("Testmelding verstuurd", "Test notification sent")).severity(Severity::Success).timeout(2500));
            } else {
                pp.toast(
                    Toast::new(t("Testmelding", "Test notification"))
                        .glyph("notification")
                        .message(t("Zo ziet een melding eruit.", "This is what a notification looks like."))
                        .timeout(4000),
                );
            }
        });
    });
    let test = SettingsCard::new("play", t("Testmelding versturen", "Send a test notification"), t("Bekijk hoe een melding eruitziet", "See what a notification looks like"));
    test.set_control(&send);
    let g = SettingsGroup::new("", "");
    g.add(test.widget());
    g
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apps_sort_as_windows_does() {
        let apps = vec![
            json!({"id": "b.desktop", "name": "Bestanden"}),
            json!({"id": "a.desktop", "name": "agenda"}),
            json!({"id": "c.desktop", "name": "Chat"}),
        ];
        let items = vec![
            json!({"desktopId": "c.desktop", "time": 10.0}),
            json!({"desktopId": "b", "time": 30.0}),
            json!({"desktopId": "c.desktop", "time": 20.0}),
            json!({"desktopId": null, "time": 99.0}),
        ];
        let latest = latest_per_app(&items);
        assert_eq!(latest.get("c.desktop"), Some(&20.0));
        assert_eq!(latest.get("b.desktop"), Some(&30.0), "a desktop id without .desktop counts");
        let ids = |v: Vec<Value>| v.iter().map(|a| a["id"].as_str().unwrap().to_owned()).collect::<Vec<_>>();
        assert_eq!(ids(sorted(&apps, &latest, false)), ["b.desktop", "c.desktop", "a.desktop"], "Meest recent");
        assert_eq!(ids(sorted(&apps, &latest, true)), ["a.desktop", "b.desktop", "c.desktop"], "Naam, case-insensitive");
    }

    #[test]
    fn the_summary_names_what_reaches_you() {
        kit::i18n::set_forced_lang(Some("nl"));
        assert_eq!(summary(true, true, true), "Banners, Geluiden");
        assert_eq!(summary(true, false, true), "Geluiden");
        assert_eq!(summary(true, false, false), "");
        assert_eq!(summary(false, true, true), "", "an app that is off shows only its switch");
        kit::i18n::set_forced_lang(Some("en"));
        assert_eq!(summary(true, true, true), "Banners, Sounds");
        kit::i18n::set_forced_lang(None);
    }
}
