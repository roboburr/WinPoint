//! Instellingen › Privacy en beveiliging › Locatie, Camera, Microfoon as Windows 11 lays them out, with GNOME
//! Settings 50's functions (settings parity, batch F):
//!
//! * the device switch: Locatieservices (org.gnome.system.location `enabled`, GNOME's Automatic Device Location),
//!   Cameratoegang (org.gnome.desktop.privacy `disable-camera` inverted), Microfoontoegang (`disable-microphone`
//!   inverted; GNOME 50 has it per app only, in Apps);
//! * "Apps toegang geven tot je ...": GNOME's Permitted Apps, the PermissionStore entry (privacy.permissions /
//!   privacy.setPermission: location/location EXACT or NONE, devices/camera and devices/microphone yes or no), named
//!   through privacy.apps.sandboxed (their portal app id), greyed while the device switch is off; it follows the
//!   store as GNOME's list does (`privacy.changed`: a portal dialog answered while the page is open, an app's
//!   Geavanceerde opties, a second window);
//! * "Desktop-apps toegang geven tot je ...": only a line, as plan.json says: unsandboxed apps use the device (or
//!   GeoClue) directly, the switches above cover the apps that ask through the portals.
use super::{Builder, Content, Page, PageFuture};
use crate::common;
use crate::gs::{self, GKey, Gs};
use kit::controls::{SettingsCard, SettingsExpander, SettingsGroup, ToggleSwitch};
use kit::format::DateFormat;
use kit::prelude::*;
use kit::t;
use std::collections::HashMap;
use std::rc::Rc;

pub const PAGES: &[(&str, Builder)] = &[("location", location), ("camera", camera), ("microphone", microphone)];
pub const CSS: &str = "";

const LOCATION_ON: GKey = gs::key("org.gnome.system.location", "enabled");
const CAMERA_OFF: GKey = gs::key("org.gnome.desktop.privacy", "disable-camera");
const MICROPHONE_OFF: GKey = gs::key("org.gnome.desktop.privacy", "disable-microphone");

/// One of the three pages.
struct Kind {
    key: GKey,
    /// the switch shows !value (disable-camera, disable-microphone)
    invert: bool,
    table: &'static str,
    id: &'static str,
    icon: &'static str,
    title: &'static str,
    desc: &'static str,
    apps_title: &'static str,
    apps_desc: &'static str,
    empty: &'static str,
    desktop_title: &'static str,
    desktop_desc: &'static str,
}

fn location(p: Page) -> PageFuture {
    let kind = Kind {
        key: LOCATION_ON,
        invert: false,
        table: "location",
        id: "location",
        icon: "location",
        title: t("Locatieservices", "Location services"),
        desc: t(
            "Wanneer dit is ingeschakeld, bepaalt het apparaat zijn locatie met bronnen zoals wifi, gps en mobiele netwerken, en kan iedereen die dit apparaat gebruikt kiezen welke apps de locatie mogen gebruiken",
            "When this is on, the device works out its location with sources like Wi-Fi, GPS and mobile networks, and anyone using this device can choose which apps may use the location",
        ),
        apps_title: t("Apps toegang geven tot je locatie", "Let apps access your location"),
        apps_desc: t("Kies welke apps toegang hebben tot je exacte locatie", "Choose which apps can access your precise location"),
        empty: t("Er zijn nog geen apps in een sandbox die om je locatie hebben gevraagd.", "No sandboxed apps have asked for your location yet."),
        desktop_title: t("Desktop-apps toegang geven tot je locatie", "Let desktop apps access your location"),
        desktop_desc: t(
            "Apps die niet in een sandbox draaien, vragen de locatie rechtstreeks op zolang Locatieservices is ingeschakeld; GNOME vraagt je eerst om toestemming.",
            "Apps that don’t run in a sandbox ask for the location directly while Location services is on; GNOME asks for your permission first.",
        ),
    };
    page(p, kind)
}

fn camera(p: Page) -> PageFuture {
    let kind = Kind {
        key: CAMERA_OFF,
        invert: true,
        table: "devices",
        id: "camera",
        icon: "camera",
        title: t("Cameratoegang", "Camera access"),
        desc: t(
            "Wanneer dit is ingeschakeld, kan iedereen die dit apparaat gebruikt kiezen of zijn apps toegang hebben tot de camera",
            "Anyone using this device can choose if their apps have camera access when this is on",
        ),
        apps_title: t("Apps toegang geven tot je camera", "Let apps access your camera"),
        apps_desc: t("Kies welke apps toegang hebben tot je camera", "Choose which apps can access your camera"),
        empty: t("Er zijn nog geen apps in een sandbox die om toegang tot je camera hebben gevraagd.", "No sandboxed apps have asked for camera access yet."),
        desktop_title: t("Desktop-apps toegang geven tot je camera", "Let desktop apps access your camera"),
        desktop_desc: t(
            "Apps die niet in een sandbox draaien, gebruiken de camera rechtstreeks. Deze instellingen gelden voor apps die de camera via de portal van GNOME vragen (Flatpak en Snap).",
            "Apps that don’t run in a sandbox use the camera directly. These settings apply to apps that ask for the camera through GNOME’s portal (Flatpak and Snap).",
        ),
    };
    page(p, kind)
}

fn microphone(p: Page) -> PageFuture {
    let kind = Kind {
        key: MICROPHONE_OFF,
        invert: true,
        table: "devices",
        id: "microphone",
        icon: "mic",
        title: t("Microfoontoegang", "Microphone access"),
        desc: t(
            "Wanneer dit is ingeschakeld, kan iedereen die dit apparaat gebruikt kiezen of zijn apps toegang hebben tot de microfoon",
            "Anyone using this device can choose if their apps have microphone access when this is on",
        ),
        apps_title: t("Apps toegang geven tot je microfoon", "Let apps access your microphone"),
        apps_desc: t("Kies welke apps toegang hebben tot je microfoon", "Choose which apps can access your microphone"),
        empty: t("Er zijn nog geen apps in een sandbox die om toegang tot je microfoon hebben gevraagd.", "No sandboxed apps have asked for microphone access yet."),
        desktop_title: t("Desktop-apps toegang geven tot je microfoon", "Let desktop apps access your microphone"),
        desktop_desc: t(
            "Apps die niet in een sandbox draaien, gebruiken de microfoon rechtstreeks. Deze instellingen gelden alleen voor apps die de microfoon via de portal van GNOME vragen (Flatpak en Snap).",
            "Apps that don’t run in a sandbox use the microphone directly. These settings only apply to apps that ask for the microphone through GNOME’s portal (Flatpak and Snap).",
        ),
    };
    page(p, kind)
}

// ------------------------------------------------------------------------------------------- pure parts

/// One app of the PermissionStore entry, named: (portal id, name, icon, allowed, last access in ms).
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Row {
    pub app: String,
    pub name: String,
    pub icon: String,
    pub allowed: bool,
    pub last: Option<f64>,
}

/// The entry's apps that are installed (GNOME leaves out an app id without a desktop file), by name. `sandboxed` is
/// privacy.apps.sandboxed's list, `list` apps.list (a desktop app GNOME Shell stored by its desktop id).
pub(super) fn rows(entry: &[Value], sandboxed: &[Value], list: &[Value]) -> Vec<Row> {
    let by_portal: HashMap<&str, &Value> = sandboxed.iter().map(|a| (common::s(&a["portalId"]), a)).collect();
    let mut out: Vec<Row> = entry
        .iter()
        .filter_map(|e| {
            let app = common::s(&e["app"]);
            let known = by_portal.get(app).copied().or_else(|| {
                let id = format!("{app}.desktop");
                list.iter().find(|l| common::s(&l["id"]) == id)
            })?;
            // location: [accuracy, timestamp in seconds] (cc-location-panel.c "Last accessed")
            let last = e["value"].get(1).and_then(|v| v.as_str()).and_then(|s| s.parse::<f64>().ok()).filter(|s| *s > 0.0).map(|s| s * 1000.0);
            Some(Row {
                app: app.to_owned(),
                name: common::s(&known["name"]).to_owned(),
                icon: common::s(&known["icon"]).to_owned(),
                allowed: e["allowed"] == true,
                last,
            })
        })
        .collect();
    out.sort_by_key(|r| r.name.to_lowercase());
    out
}

/// Whether a `privacy.changed` event is about this page's PermissionStore entry.
pub(super) fn about(data: &Value, table: &str, id: &str) -> bool {
    data["what"] == "permissions" && data["table"] == table && data["id"] == id
}

/// Whether a new read of the entry can be shown in the rows already there (the same apps in the same order: only
/// their switches and last access change); otherwise the page is built again.
pub(super) fn same_apps(shown: &[Row], now: &[Row]) -> bool {
    shown.len() == now.len() && shown.iter().zip(now).all(|(a, b)| a.app == b.app && a.name == b.name)
}

/// A row's description: the last access (location), else nothing.
fn last_text(row: &Row) -> String {
    row.last
        .map(|ms| common::fill(t("Laatst geopend op {date}", "Last accessed {date}"), &[("date", &kit::format::date(ms, DateFormat::DateTime))]))
        .unwrap_or_default()
}

// ------------------------------------------------------------------------------------------------ the page

fn page(p: Page, kind: Kind) -> PageFuture {
    Box::pin(async move {
        let (entry, sandboxed, list) = (
            p.call("privacy.permissions", json!({ "table": kind.table, "id": kind.id })),
            p.call("privacy.apps.sandboxed", json!({})),
            p.call("apps.list", json!({})),
        );
        let gs = Gs::load(&p, std::slice::from_ref(&kind.key)).await;
        let (entry, sandboxed, list) = (entry.await, sandboxed.await, list.await);
        if !p.alive() {
            return Content::new();
        }
        let keep = gs.clone();
        p.on_cleanup(move || drop(keep));
        let mut out: Content = Vec::new();
        if kind.table == "location" && common::caps()["geoclue"] == false {
            out.push(
                common::notice(t("Locatieservices zijn op dit apparaat niet beschikbaar: GeoClue is niet geïnstalleerd.", "Location services aren’t available on this device: GeoClue isn’t installed."))
                    .widget()
                    .clone()
                    .upcast(),
            );
        }
        let g = SettingsGroup::new("", "");
        match gs.toggle_card(&kind.key, kind.icon, kind.title, kind.desc, kind.invert) {
            Some(card) => g.add(card.widget()),
            None => g.add(common::unavailable_card(kind.icon, kind.title, None).widget()),
        }

        // ---- Apps toegang geven tot je ...: GNOME's Permitted Apps
        let exp = SettingsExpander::new("apps", kind.apps_title, kind.apps_desc);
        exp.header_button().set_widget_name(&format!("{}-apps", p.id()));
        let entry = entry.unwrap_or_else(|_| json!({ "available": false }));
        let sandboxed: Vec<Value> = sandboxed.ok().and_then(|v| v["apps"].as_array().cloned()).unwrap_or_default();
        let list: Vec<Value> = list.ok().and_then(|v| v.as_array().cloned()).unwrap_or_default();
        let device_on = {
            let (gs, key, invert) = (gs.clone(), kind.key.clone(), kind.invert);
            Rc::new(move || (gs.value(&key) == true) != invert || !gs.available(&key))
        };
        let switches: Rc<std::cell::RefCell<Vec<ToggleSwitch>>> = Rc::default();
        let shown: Rc<std::cell::RefCell<Vec<(Row, SettingsCard, ToggleSwitch)>>> = Rc::default();
        let available = entry["available"] == true;
        if !available {
            exp.add(&common::notice(t("De machtigingen van apps zijn nu niet beschikbaar.", "App permissions aren’t available right now.")).widget().clone());
        } else {
            let found = rows(entry["apps"].as_array().map(Vec::as_slice).unwrap_or(&[]), &sandboxed, &list);
            if found.is_empty() {
                exp.add(&SettingsCard::new("info", kind.empty, "").widget().clone());
            }
            for row in found {
                let (card, sw) = app_row(&p, &kind, &row);
                switches.borrow_mut().push(sw.clone());
                exp.add(card.widget());
                shown.borrow_mut().push((row, card, sw));
            }
        }
        // the store changed (GNOME follows its Changed signal): read the entry again; the same apps are updated in
        // place, an app that asked for the first time (or went) builds the page again
        {
            let (pp, table, id) = (p.clone(), kind.table, kind.id);
            let (sandboxed, list) = (Rc::new(sandboxed), Rc::new(list));
            p.on("privacy.changed", move |data| {
                if !about(data, table, id) {
                    return;
                }
                let fut = pp.call("privacy.permissions", json!({ "table": table, "id": id }));
                let (p2, shown, sandboxed, list) = (pp.clone(), shown.clone(), sandboxed.clone(), list.clone());
                pp.spawn(async move {
                    let Ok(entry) = fut.await else { return };
                    if !p2.alive() {
                        return;
                    }
                    let now = rows(entry["apps"].as_array().map(Vec::as_slice).unwrap_or(&[]), &sandboxed, &list);
                    let current: Vec<Row> = shown.borrow().iter().map(|(r, _, _)| r.clone()).collect();
                    if (entry["available"] == true) != available || !same_apps(&current, &now) {
                        p2.refresh();
                        return;
                    }
                    for ((row, card, sw), new) in shown.borrow_mut().iter_mut().zip(now) {
                        sw.set_active(new.allowed);
                        card.set_description(&last_text(&new));
                        *row = new;
                    }
                });
            });
        }
        exp.set_expanded(true);
        let sensitive = {
            let (switches, device_on, exp) = (switches.clone(), device_on.clone(), exp.clone());
            move || {
                let on = device_on();
                for sw in switches.borrow().iter() {
                    sw.set_sensitive(on);
                }
                exp.header().set_description(if on { kind.apps_desc } else { t("Schakel dit hierboven eerst in", "Turn this on above first") });
            }
        };
        sensitive();
        gs.follow(&kind.key, move |_| sensitive());
        g.add(exp.widget());
        out.push(g.widget().clone().upcast());

        // ---- Desktop-apps toegang geven tot je ...: what the switches do not cover
        let g2 = SettingsGroup::new("", "");
        g2.add(SettingsCard::new("pc", kind.desktop_title, kind.desktop_desc).widget());
        out.push(g2.widget().clone().upcast());
        out
    })
}

/// One app: its icon and name, the last access (location), the switch.
fn app_row(p: &Page, kind: &Kind, row: &Row) -> (SettingsCard, ToggleSwitch) {
    let icon = if row.icon.is_empty() || row.icon.starts_with('/') { "application-x-executable" } else { row.icon.as_str() };
    let card = SettingsCard::new(icon, &row.name, &last_text(row));
    let sw = ToggleSwitch::new(row.allowed);
    let part: String = row.app.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
    sw.switch().set_widget_name(&format!("{}-app-{part}", p.id()));
    let (pp, sw2, app, table, id) = (p.clone(), sw.clone(), row.app.clone(), kind.table, kind.id);
    sw.connect_toggled(move |on| {
        let fut = pp.run("privacy.setPermission", json!({ "table": table, "id": id, "app": app, "value": on }));
        let sw3 = sw2.clone();
        pp.spawn(async move {
            if fut.await.is_none() {
                sw3.set_active(!on);
            }
        });
    });
    card.set_control(sw.widget());
    (card, sw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permitted_apps_are_named_and_sorted_as_gnome_lists_them() {
        let entry = vec![
            json!({"app": "org.gnome.Maps", "value": ["EXACT", "1700000000"], "allowed": true}),
            json!({"app": "snap.firefox", "value": ["NONE", "0"], "allowed": false}),
            json!({"app": "org.gnome.Weather", "value": ["EXACT", "0"], "allowed": true}),
            json!({"app": "com.example.Gone", "value": ["yes"], "allowed": true}),
        ];
        let sandboxed = vec![
            json!({"portalId": "org.gnome.Maps", "id": "org.gnome.Maps.desktop", "name": "Kaarten", "icon": "org.gnome.Maps"}),
            json!({"portalId": "snap.firefox", "id": "firefox_firefox.desktop", "name": "Firefox", "icon": "/snap/firefox/icon.png"}),
        ];
        let list = vec![json!({"id": "org.gnome.Weather.desktop", "name": "Weer", "icon": "org.gnome.Weather"})];
        let found = rows(&entry, &sandboxed, &list);
        let names: Vec<&str> = found.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["Firefox", "Kaarten", "Weer"], "by name; an app no desktop file names is left out");
        assert_eq!(found[1].last, Some(1_700_000_000_000.0), "location's timestamp is the last access");
        assert_eq!(found[2].last, None, "0 is no access");
        assert!(!found[0].allowed && found[1].allowed);
        assert_eq!(found[0].app, "snap.firefox", "a snap by its portal id");
    }

    #[test]
    fn a_change_in_the_store_updates_the_rows_or_builds_the_page_again() {
        let sandboxed = vec![
            json!({"portalId": "org.gnome.Maps", "name": "Kaarten", "icon": ""}),
            json!({"portalId": "snap.firefox", "name": "Firefox", "icon": ""}),
        ];
        let read = |entry: &[Value]| rows(entry, &sandboxed, &[]);
        let before = read(&[json!({"app": "org.gnome.Maps", "value": ["yes"], "allowed": true})]);
        // the portal's dialog answered for Maps: the same app, another value, updated in place
        let switched = read(&[json!({"app": "org.gnome.Maps", "value": ["no"], "allowed": false})]);
        assert!(same_apps(&before, &switched) && !switched[0].allowed);
        // Firefox asked for the first time: a new row, the page is built again
        let added = read(&[
            json!({"app": "org.gnome.Maps", "value": ["yes"], "allowed": true}),
            json!({"app": "snap.firefox", "value": ["yes"], "allowed": true}),
        ]);
        assert!(!same_apps(&before, &added));
        assert!(!same_apps(&before, &[]), "the entry was deleted");
        // only this page's entry counts
        let camera = json!({"what": "permissions", "table": "devices", "id": "camera"});
        assert!(about(&camera, "devices", "camera"));
        assert!(!about(&camera, "devices", "microphone") && !about(&camera, "location", "location"));
        assert!(!about(&json!({"what": "handlers", "table": "devices", "id": "camera"}), "devices", "camera"));
    }
}
