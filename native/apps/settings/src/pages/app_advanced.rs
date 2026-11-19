//! Instellingen › Apps › Geïnstalleerde apps › <app> (Windows 11's "Geavanceerde opties" of an app, a data page:
//! `{id: desktop id}`, notes/claude/settings-parity.md §2): GNOME Settings 50's per-app Apps page in Windows' order
//! ([`sections`]). Windows' own sections first: Specificaties (version, App / Gegevens / Cache / Totaal),
//! App-machtigingen (the PermissionStore rows GNOME shows for a sandboxed app: Locatie, Camera, Microfoon,
//! Schermafbeeldingen, Achtergrond wijzigen, Systeemsneltoetsen blokkeren; Meldingen, which opens the app's
//! notification page; Zoeken, the app's search provider in org.gnome.desktop.search-providers), Machtigingen voor
//! apps op de achtergrond (PermissionStore background: Altijd / Nooit), Wordt uitgevoerd bij aanmelden (GNOME's
//! Autostart, only while it may run in the background, amendment 7), Beëindigen, Opnieuw instellen (GNOME's Clear
//! Cache, which windows.json maps Windows' Reset to) and Verwijderen. Then WinPoint's additions from GNOME:
//! Algemene sneltoetsen (GNOME 50, amendment 43), Vereiste machtigingen (a Flatpak's static permissions, a snap's
//! interfaces), Standaardkoppelingen (GNOME's Files & Links, [`handlers_group`], which defaultapps.rs reuses,
//! amendment 22). The permission rows follow the PermissionStore (`privacy.changed`), the device switches of
//! Privacy en beveiliging and app search (GSettings), as GNOME's Apps panel does. Host: privacy.app and its writes
//! (host/src/services/privacy.rs), sysmon.processes / sysmon.endTask, software.* (installed.rs `software`).
use super::installed::software;
use super::{Builder, Content, Page, PageFuture};
use crate::common;
use crate::gs::{self, GKey, Gs};
use crate::pickers::{Capture, KeyCapture};
use kit::controls::{self as c, Dialog, Dropdown, DropdownOption, SettingsCard, SettingsGroup, Toast, ToggleSwitch, Variant};
use kit::prelude::*;
use kit::t;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub const PAGES: &[(&str, Builder)] = &[("app-advanced", app_advanced)];
pub const CSS: &str = r#"
.st-handler-heading { margin-top: 8px; color: var(--text-2); font-size: var(--fs-caption); font-weight: 600; }
"#;

const SEARCH_DISABLED: GKey = gs::key("org.gnome.desktop.search-providers", "disabled");
const SEARCH_ENABLED: GKey = gs::key("org.gnome.desktop.search-providers", "enabled");
const SEARCH_EXTERNAL: GKey = gs::key("org.gnome.desktop.search-providers", "disable-external");
const CAMERA_OFF: GKey = gs::key("org.gnome.desktop.privacy", "disable-camera");
const MICROPHONE_OFF: GKey = gs::key("org.gnome.desktop.privacy", "disable-microphone");
const LOCATION_ON: GKey = gs::key("org.gnome.system.location", "enabled");
const KEYS: &[GKey] = &[SEARCH_DISABLED, SEARCH_ENABLED, SEARCH_EXTERNAL, CAMERA_OFF, MICROPHONE_OFF, LOCATION_ON];

// ------------------------------------------------------------------------------------------- pure parts

fn strings(v: &Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_owned)).collect()).unwrap_or_default()
}

/// Whether the app's search provider shows results (GNOME: a DefaultDisabled provider only while it is in
/// `enabled`, any other one unless it is in `disabled`).
pub(super) fn search_on(default_disabled: bool, id: &str, disabled: &[String], enabled: &[String]) -> bool {
    if default_disabled { enabled.iter().any(|e| e == id) } else { !disabled.iter().any(|d| d == id) }
}

/// The list to write for the search switch: `enabled` (DefaultDisabled) or `disabled` with the app added or taken out.
pub(super) fn search_list(on: bool, default_disabled: bool, id: &str, current: &[String]) -> Vec<String> {
    let add = on == default_disabled;
    let mut out: Vec<String> = current.iter().filter(|x| *x != id).cloned().collect();
    if add {
        out.push(id.to_owned());
    }
    out
}

/// One section of the page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Section {
    Specifications,
    Permissions,
    Background,
    Autostart,
    Terminate,
    Reset,
    Uninstall,
    GlobalShortcuts,
    Required,
    Handlers,
}

/// The sections privacy.app's answer has, in Windows 11's order of Advanced options (Specificaties,
/// App-machtigingen, Machtigingen voor apps op de achtergrond, Wordt uitgevoerd bij aanmelden, Beëindigen, Opnieuw
/// instellen, Verwijderen), then WinPoint's additions from GNOME after Windows' own (as amendments 11 and 12 place
/// them): Algemene sneltoetsen, Vereiste machtigingen, Standaardkoppelingen.
pub(super) fn sections(app: &Value) -> Vec<Section> {
    let req = &app["required"];
    let required = match req["kind"].as_str() {
        Some("flatpak") => req["static"].as_array().is_some_and(|r| !r.is_empty()),
        Some("snap") => req["plugs"].as_array().is_some_and(|r| !r.is_empty()),
        _ => false,
    };
    [
        (Section::Specifications, true),
        (Section::Permissions, true),
        (Section::Background, app["sandboxed"] == true && app["background"].is_object()),
        (Section::Autostart, true),
        (Section::Terminate, true),
        (Section::Reset, app["canClearCache"] == true),
        (Section::Uninstall, true),
        (Section::GlobalShortcuts, app["globalShortcuts"]["shortcuts"].as_array().is_some_and(|l| !l.is_empty())),
        (Section::Required, required),
        (Section::Handlers, true),
    ]
    .into_iter()
    .filter_map(|(section, shown)| shown.then_some(section))
    .collect()
}

/// The PermissionStore rows of privacy.app's answer the page shows (GNOME: those the app's entry has), with their
/// state: (key, allowed).
pub(super) fn shown_permissions(app: &Value) -> Vec<(String, bool)> {
    app["permissions"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|r| r["set"] == true && permission_text(common::s(&r["key"])).is_some())
        .map(|r| (common::s(&r["key"]).to_owned(), r["allowed"] == true))
        .collect()
}

/// Whether the device a permission row needs is off for every app in Privacy en beveiliging, from the value of its
/// key ([`device_key`]): disable-camera / disable-microphone true, location `enabled` false.
pub(super) fn device_off(key: &str, value: &Value) -> bool {
    match key {
        "camera" | "microphone" => *value == true,
        "location" => *value == false,
        _ => false,
    }
}

/// The GSettings key of Privacy en beveiliging's device switch a permission row needs.
fn device_key(key: &str) -> Option<GKey> {
    match key {
        "camera" => Some(CAMERA_OFF),
        "microphone" => Some(MICROPHONE_OFF),
        "location" => Some(LOCATION_ON),
        _ => None,
    }
}

fn device_off_text() -> &'static str {
    t("Uitgeschakeld voor alle apps in Privacy en beveiliging", "Turned off for all apps in Privacy & security")
}

/// Zoeken: whether the switch can be used (a writable list, app search not turned off) and its description.
pub(super) fn search_state(writable: bool, external_off: bool) -> (bool, &'static str) {
    let text = if external_off {
        t("Zoeken in apps is uitgeschakeld", "Searching in apps is turned off")
    } else {
        t("Resultaten van deze app opnemen in de zoekresultaten", "Include results from this app in search results")
    };
    (writable && !external_off, text)
}

/// "12,3 MB", or "Onbekend" when the host could not measure it.
fn size_text(v: &Value) -> String {
    match v.as_f64() {
        Some(n) => kit::format::bytes(n),
        None => t("Onbekend", "Unknown").to_owned(),
    }
}

/// The title and description of a PermissionStore row privacy.app gives.
pub(super) fn permission_text(key: &str) -> Option<(&'static str, &'static str, &'static str)> {
    Some(match key {
        "location" => ("location", t("Locatie", "Location"), t("Je exacte locatie gebruiken", "Use your precise location")),
        "camera" => ("camera", t("Camera", "Camera"), t("Je camera gebruiken", "Use your camera")),
        "microphone" => ("mic", t("Microfoon", "Microphone"), t("Je microfoon gebruiken", "Use your microphone")),
        "screenshot" => ("image", t("Schermafbeeldingen", "Screenshots"), t("Schermafbeeldingen maken zonder het te vragen", "Take screenshots without asking")),
        "wallpaper" => ("image", t("Achtergrond wijzigen", "Change background"), t("Je bureaubladachtergrond instellen", "Set your desktop background")),
        "shortcutsInhibit" => (
            "keyboard",
            t("Systeemsneltoetsen blokkeren", "Block system shortcuts"),
            t("De sneltoetsen van het systeem tijdelijk overnemen", "Take over the system’s shortcuts for a while"),
        ),
        _ => return None,
    })
}

/// A Flatpak static permission row: (title, access text).
pub(super) fn static_text(key: &str, access: &str) -> Option<(&'static str, &'static str)> {
    let full = t("Volledige toegang", "Full access");
    let read = t("Alleen-lezen", "Read-only");
    let access_text = if access == "read" { read } else { full };
    Some(match key {
        "systemBus" => (t("Systeembus", "System bus"), full),
        "sessionBus" => (t("Sessiebus", "Session bus"), full),
        "devices" => (t("Apparaten", "Devices"), t("Volledige toegang tot /dev", "Full access to /dev")),
        "network" => (t("Netwerk", "Network"), t("Heeft netwerktoegang", "Has network access")),
        "home" => (t("Thuismap", "Home folder"), access_text),
        "fileSystem" => (t("Bestandssysteem", "File system"), access_text),
        "settings" => (t("Instellingen", "Settings"), t("Kan instellingen wijzigen", "Can change settings")),
        "sound" => (t("Geluid", "Sound"), t("Kan geluid afspelen en opnemen", "Can play and record sound")),
        _ => return None,
    })
}

/// A snap interface's name as a person reads it (GNOME's cc-snap-row labels for the common ones), else the
/// interface itself.
pub(super) fn interface_label(interface: &str) -> String {
    match interface {
        "camera" => t("Camera", "Camera"),
        "audio-playback" => t("Audio afspelen", "Play audio"),
        "audio-record" => t("Audio opnemen", "Record audio"),
        "network" => t("Netwerk", "Network"),
        "network-bind" => t("Netwerkdiensten aanbieden", "Provide network services"),
        "home" => t("Thuismap", "Home folder"),
        "removable-media" => t("Verwisselbare media", "Removable media"),
        "bluez" | "bluetooth-control" => t("Bluetooth", "Bluetooth"),
        "location-observe" | "location-control" => t("Locatie", "Location"),
        "password-manager-service" => t("Opgeslagen wachtwoorden", "Saved passwords"),
        "screen-inhibit-control" => t("Slaapstand van het scherm voorkomen", "Prevent the screen from sleeping"),
        "joystick" => t("Gamecontrollers", "Game controllers"),
        "cups-control" | "cups" => t("Afdrukken", "Printing"),
        "hardware-observe" => t("Hardware bekijken", "View hardware"),
        "system-observe" => t("Systeem bekijken", "View the system"),
        "raw-usb" => t("USB-apparaten", "USB devices"),
        _ => return interface.to_owned(),
    }
    .to_owned()
}

/// A type row's title: its extensions (".html, .htm") for a file type, else its name.
pub(super) fn handler_title(h: &Value) -> String {
    let ext = strings(&h["extensions"]);
    if h["kind"] == "file" && !ext.is_empty() { ext.join(", ") } else { common::s(&h["name"]).to_owned() }
}

/// The text of a GTK accelerator ("<Super>r" → "Super+R"), "Geen" without one.
fn accel_text(accels: &[String]) -> String {
    let labels: Vec<String> = accels
        .iter()
        .filter_map(|a| gtk::accelerator_parse(a).map(|(k, m)| gtk::accelerator_get_label(k, m).to_string()))
        .filter(|l| !l.is_empty())
        .collect();
    if labels.is_empty() { t("Geen", "None").to_owned() } else { labels.join(", ") }
}

// ------------------------------------------------------------------------------------------------ the page

fn no_app(p: &Page) -> Content {
    let g = SettingsGroup::new("", t("Kies een app in Geïnstalleerde apps.", "Choose an app in Installed apps."));
    g.add(common::link_card(p, "installed").widget());
    vec![g.widget().clone().upcast()]
}

fn app_advanced(p: Page) -> PageFuture {
    Box::pin(async move {
        let Some(id) = p.param("id").filter(|s| !s.is_empty()) else {
            return no_app(&p);
        };
        let (info, list) = (p.call("privacy.app", json!({ "id": id })), p.call("apps.list", json!({})));
        let gs = Gs::load(&p, KEYS).await;
        let (info, list) = (info.await, list.await);
        if !p.alive() {
            return Content::new();
        }
        let list: Vec<Value> = list.ok().and_then(|v| v.as_array().cloned()).unwrap_or_default();
        let info = match info {
            Ok(v) => with_list_name(v, &list),
            Err(e) => {
                let mut out = vec![common::notice(&e.message).widget().clone().upcast()];
                out.extend(no_app(&p));
                return out;
            }
        };
        // Gs follows gsettings.changed through a weak reference: the page keeps it while it is shown, also when no
        // control holds a clone of it (an app without a search provider still follows the device switches)
        let keep = gs.clone();
        p.on_cleanup(move || drop(keep));
        let name = common::s(&info["name"]).to_owned();
        p.set_title(&name);
        let app = Rc::new(info);
        let all = Rc::new(super::installed::ids(&list));
        let mut out: Content = vec![header(&p, &app, all)];
        let (perm_group, perm_switches) = permissions(&p, &app, &gs);
        let (mut background, autostart, background_state) = background_and_autostart(&p, &app);
        let (mut perm_group, mut autostart) = (Some(perm_group), Some(autostart));
        for section in sections(&app) {
            let group = match section {
                Section::Specifications => Some(specifications(&app)),
                Section::Permissions => perm_group.take(),
                Section::Background => background.take(),
                Section::Autostart => autostart.take(),
                Section::Terminate => Some(terminate(&p, &app)),
                Section::Reset => reset(&p, &app),
                Section::Uninstall => Some(uninstall(&p, &app)),
                Section::GlobalShortcuts => global_shortcuts(&p, &app),
                Section::Required => required(&p, &app),
                Section::Handlers => Some(handlers_group(&p, &app)),
            };
            if let Some(g) = group {
                out.push(g.widget().clone().upcast());
            }
        }
        follow_store(&p, &app, perm_switches, background_state);
        out
    })
}

/// GNOME's Apps panel follows the PermissionStore: on `privacy.changed {what: "permissions"}` (a portal dialog
/// answered while the page is open, Locatie / Camera / Microfoon, a second window) the app's rows are read again
/// and their switches and Achtergrond's choice set in place; a row that appeared or went builds the page again.
fn follow_store(p: &Page, app: &Rc<Value>, switches: Vec<(String, ToggleSwitch)>, background: Option<BackgroundState>) {
    if app["portalId"].as_str().is_none_or(str::is_empty) {
        return;
    }
    let (pp, id) = (p.clone(), common::s(&app["id"]).to_owned());
    let shown: Vec<String> = switches.iter().map(|(k, _)| k.clone()).collect();
    let (switches, background) = (Rc::new(switches), Rc::new(background));
    p.on("privacy.changed", move |data| {
        if data["what"] != "permissions" {
            return;
        }
        let fut = pp.call("privacy.app", json!({ "id": id, "sizes": false }));
        let (p2, shown, switches, background) = (pp.clone(), shown.clone(), switches.clone(), background.clone());
        pp.spawn(async move {
            let Ok(info) = fut.await else { return };
            if !p2.alive() {
                return;
            }
            let now = shown_permissions(&info);
            let has_background = sections(&info).contains(&Section::Background);
            if now.iter().map(|(k, _)| k).ne(shown.iter()) || has_background != background.is_some() {
                p2.refresh();
                return;
            }
            for (key, allowed) in now {
                if let Some((_, sw)) = switches.iter().find(|(k, _)| *k == key) {
                    sw.set_active(allowed);
                }
            }
            if let Some(state) = background.as_ref() {
                state.show(info["background"]["allowed"] == true);
            }
        });
    });
}

/// privacy.app's answer with the app's name as apps.list gives it (in WinPoint's language, as Geïnstalleerde apps
/// shows it; the host's desktop file name follows the process's locale).
pub(super) fn with_list_name(mut info: Value, list: &[Value]) -> Value {
    if let Some(name) = list.iter().find(|a| a["id"] == info["id"]).and_then(|a| a["name"].as_str()).filter(|n| !n.is_empty()) {
        info["name"] = json!(name);
    }
    info
}

/// The app: its icon, name and kind, Openen and Openen in App Center.
fn header(p: &Page, app: &Rc<Value>, all: Rc<Vec<String>>) -> gtk::Widget {
    let id = common::s(&app["id"]).to_owned();
    let icon = app["icon"].as_str().filter(|s| !s.is_empty()).unwrap_or("application-x-executable");
    let art = kit::icons::app_icon(icon, 64);
    let kind = match app["kind"].as_str() {
        Some("flatpak") => "Flatpak",
        Some("snap") => "Snap",
        _ => t("Systeempakket", "System package"),
    };
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let open = c::button(t("Openen", "Open"), Variant::Accent);
    open.set_widget_name("st-app-advanced-open");
    {
        let (pp, id) = (p.clone(), id.clone());
        open.connect_clicked(move |_| {
            let fut = pp.run("apps.launch", json!({ "id": id }));
            pp.spawn(async move {
                let _ = fut.await;
            });
        });
    }
    actions.append(&open);
    let store = c::button("", Variant::Standard);
    store.set_widget_name("st-app-advanced-store");
    store.set_visible(false);
    actions.append(&store);
    // only when a store that can show it is installed (apps.list has it)
    if let Some((target, _)) = super::installed::store_target(app, &all) {
        store.set_label(&common::fill(t("Openen in {store}", "Open in {store}"), &[("store", super::installed::store_name(&target))]));
        store.set_visible(true);
        let (pp, id) = (p.clone(), id.clone());
        store.connect_clicked(move |_| super::installed::open_in_store(&pp, &id, all.clone()));
    }
    let version = app["version"].as_str().filter(|v| !v.is_empty()).map(|v| common::fill(t("Versie {version}", "Version {version}"), &[("version", v)]));
    let lines: Vec<String> = [Some(kind.to_owned()), version].into_iter().flatten().collect();
    common::hero(Some(art.upcast_ref()), common::s(&app["name"]), &lines, Some(actions.upcast_ref()), None).upcast()
}

fn value_card(icon: &str, title: &str, value: &str) -> SettingsCard {
    let card = SettingsCard::new(icon, title, "");
    card.set_control(&common::value_label(value));
    card
}

/// Specificaties: the version and the disk use (GNOME's Storage section).
fn specifications(app: &Rc<Value>) -> SettingsGroup {
    let g = SettingsGroup::new(t("Specificaties", "Specifications"), "");
    if let Some(v) = app["version"].as_str().filter(|v| !v.is_empty()) {
        g.add(value_card("info", t("Versie", "Version"), v).widget());
    }
    let storage = &app["storage"];
    if storage.is_object() {
        g.add(value_card("apps", t("App", "App"), &size_text(&storage["app"])).widget());
        if !storage["data"].is_null() {
            g.add(value_card("folder", t("Gegevens", "Data"), &size_text(&storage["data"])).widget());
        }
        if !storage["cache"].is_null() {
            g.add(value_card("clean", t("Cache", "Cache"), &size_text(&storage["cache"])).widget());
        }
        g.add(value_card("storage", t("Totaal", "Total"), &size_text(&storage["total"])).widget());
    }
    if g.items().first_child().is_none() {
        g.add(SettingsCard::new("info", t("Er zijn geen specificaties van deze app bekend.", "No specifications of this app are known."), "").widget());
    }
    g
}

/// Opnieuw instellen: Windows' Reset, which is GNOME's Clear Cache here (a Flatpak's `~/.var/app/<id>/cache`; the
/// app's data stays). Only for an app whose cache the host can empty.
fn reset(p: &Page, app: &Rc<Value>) -> Option<SettingsGroup> {
    if app["canClearCache"] != true {
        return None;
    }
    let g = SettingsGroup::new(t("Opnieuw instellen", "Reset"), "");
    {
        let card = SettingsCard::new(
            "broom",
            t("Cache wissen", "Clear cache"),
            t("De tijdelijke bestanden van deze app verwijderen. Je gegevens blijven behouden.", "Delete this app’s temporary files. Your data will be kept."),
        );
        let b = c::button(t("Cache wissen", "Clear cache"), Variant::Standard);
        b.set_widget_name("st-app-advanced-clear-cache");
        let (pp, id) = (p.clone(), common::s(&app["id"]).to_owned());
        b.connect_clicked(move |_| {
            let (pp, id) = (pp.clone(), id.clone());
            c::confirm(
                &pp.anchor(),
                t("Cache wissen?", "Clear cache?"),
                t("De tijdelijke bestanden van deze app worden verwijderd. Je gegevens blijven behouden.", "This app’s temporary files will be deleted. Your data will be kept."),
                t("Cache wissen", "Clear cache"),
                false,
                move |ok| {
                    if !ok {
                        return;
                    }
                    let fut = pp.run("privacy.app.clearCache", json!({ "id": id }));
                    let p2 = pp.clone();
                    pp.spawn(async move {
                        if let Some(r) = fut.await
                            && r["dryRun"] != true
                        {
                            let freed = kit::format::bytes(r["freed"].as_f64().unwrap_or(0.0));
                            common::toast_success(&p2.anchor(), &common::fill(t("{size} vrijgemaakt", "{size} freed"), &[("size", &freed)]), 2500);
                        }
                    });
                },
            );
        });
        card.set_control(&b);
        g.add(card.widget());
    }
    Some(g)
}

/// A switch whose writes go to a host method: optimistic, reverted with the error toast.
fn method_switch(p: &Page, name: &str, on: bool, write: impl Fn(bool) -> (String, Value) + 'static) -> ToggleSwitch {
    let sw = ToggleSwitch::new(on);
    sw.switch().set_widget_name(name);
    let (pp, sw2) = (p.clone(), sw.clone());
    sw.connect_toggled(move |on| {
        let (method, args) = write(on);
        let fut = pp.run(&method, args);
        let sw3 = sw2.clone();
        pp.spawn(async move {
            if fut.await.is_none() {
                sw3.set_active(!on);
            }
        });
    });
    sw
}

/// App-machtigingen: the PermissionStore rows GNOME shows (those the app asked for), Meldingen and Zoeken; also
/// the rows' switches by key (for [`follow_store`]).
fn permissions(p: &Page, app: &Rc<Value>, gs: &Gs) -> (SettingsGroup, Vec<(String, ToggleSwitch)>) {
    let g = SettingsGroup::new(t("App-machtigingen", "App permissions"), "");
    let id = common::s(&app["id"]).to_owned();
    let portal = common::s(&app["portalId"]).to_owned();
    let mut switches = Vec::new();
    for row in app["permissions"].as_array().into_iter().flatten().filter(|r| r["set"] == true) {
        let key = common::s(&row["key"]);
        let Some((icon, title, desc)) = permission_text(key) else { continue };
        let (table, pid) = (common::s(&row["table"]).to_owned(), common::s(&row["id"]).to_owned());
        let portal = portal.clone();
        let sw = method_switch(p, &format!("st-app-advanced-{key}"), row["allowed"] == true, move |on| {
            ("privacy.setPermission".to_owned(), json!({ "table": table, "id": pid, "app": portal, "value": on }))
        });
        // GNOME: "Uitgeschakeld" while the whole device is off in Privacy en beveiliging, now and after a change
        // made there (or anywhere else)
        let device = device_key(key);
        let off = device.as_ref().is_some_and(|k| device_off(key, &gs.value(k)));
        let card = SettingsCard::new(icon, title, if off { device_off_text() } else { desc });
        sw.set_sensitive(!off);
        if let Some(k) = device {
            let (sw, card, key) = (sw.clone(), card.clone(), key.to_owned());
            gs.follow(&k, move |value| {
                let off = device_off(&key, value);
                sw.set_sensitive(!off);
                card.set_description(if off { device_off_text() } else { desc });
            });
        }
        card.set_control(sw.widget());
        g.add(card.widget());
        switches.push((key.to_owned(), sw));
    }
    {
        // the app's own notification page (a data page of batch F: {id: desktop id})
        let (pp, id) = (p.clone(), id.clone());
        let card = SettingsCard::clickable(
            "notification",
            t("Meldingen", "Notifications"),
            t("Meldingen en geluiden van deze app", "Notifications and sounds from this app"),
            c::Chevron::Right,
        );
        if let Some(b) = card.button() {
            b.set_widget_name("st-app-advanced-notifications");
        }
        card.connect_clicked(move || pp.go_with("app-notifications", json!({ "id": id })));
        g.add(card.widget());
    }
    if let Some(provider) = app["searchProvider"].as_object() {
        let default_disabled = provider.get("defaultDisabled") == Some(&json!(true));
        let list_key = if default_disabled { SEARCH_ENABLED } else { SEARCH_DISABLED };
        if gs.available(&list_key) {
            let on = search_on(default_disabled, &id, &strings(&gs.value(&SEARCH_DISABLED)), &strings(&gs.value(&SEARCH_ENABLED)));
            let sw = ToggleSwitch::new(on);
            sw.switch().set_widget_name(&gs::control_name(p.id(), KEYS, &list_key));
            let writable = gs.writable(&list_key);
            let (sensitive, desc) = search_state(writable, gs.value(&SEARCH_EXTERNAL) == true);
            sw.set_sensitive(sensitive);
            if !writable {
                sw.switch().set_tooltip_text(Some(gs::locked_text()));
            }
            let (gs2, sw2, id2, key2) = (gs.clone(), sw.clone(), id.clone(), list_key.clone());
            sw.connect_toggled(move |on| {
                let current = strings(&gs2.value(&key2));
                let list = search_list(on, default_disabled, &id2, &current);
                let sw3 = sw2.clone();
                gs2.set(&key2, json!(list), move || sw3.set_active(!on));
            });
            let (gs3, sw4, id3) = (gs.clone(), sw.clone(), id.clone());
            gs.follow(&list_key, move |_| {
                sw4.set_active(search_on(default_disabled, &id3, &strings(&gs3.value(&SEARCH_DISABLED)), &strings(&gs3.value(&SEARCH_ENABLED))));
            });
            let card = SettingsCard::new("search", t("Zoeken", "Search"), desc);
            card.set_control(sw.widget());
            // GNOME greys the row while app search is off for every app (Zoekmachtigingen, or anywhere else)
            let (sw5, card2) = (sw.clone(), card.clone());
            gs.follow(&SEARCH_EXTERNAL, move |value| {
                let (sensitive, desc) = search_state(writable, *value == true);
                sw5.set_sensitive(sensitive);
                card2.set_description(desc);
            });
            g.add(card.widget());
        }
    }
    (g, switches)
}

/// Achtergrond's choice and what depends on it, for [`follow_store`].
struct BackgroundState {
    dropdown: Dropdown,
    allowed: Rc<Cell<bool>>,
    sync: Rc<dyn Fn()>,
}

impl BackgroundState {
    /// Show a value read from the store (silently: nothing is written).
    fn show(&self, allowed: bool) {
        if self.allowed.replace(allowed) != allowed {
            self.dropdown.set_selected(Some(if allowed { "always" } else { "never" }));
            (self.sync)();
        }
    }
}

/// Machtigingen voor apps op de achtergrond (a sandboxed app only) and Wordt uitgevoerd bij aanmelden, which needs
/// the first (amendment 7).
fn background_and_autostart(p: &Page, app: &Rc<Value>) -> (Option<SettingsGroup>, SettingsGroup, Option<BackgroundState>) {
    let id = common::s(&app["id"]).to_owned();
    let sandboxed = app["sandboxed"] == true && app["background"].is_object();
    let allowed = Rc::new(Cell::new(!sandboxed || app["background"]["allowed"] == true));
    let auto = method_switch(p, "st-app-advanced-autostart", app["autostart"]["enabled"] == true, {
        let id = id.clone();
        move |on| ("privacy.app.setAutostart".to_owned(), json!({ "id": id, "on": on }))
    });
    let name = common::s(&app["name"]).to_owned();
    let on_desc = common::fill(t("{app} starten wanneer je je aanmeldt", "Start {app} when you sign in"), &[("app", &name)]);
    let auto_card = SettingsCard::new("startup", t("Wordt uitgevoerd bij aanmelden", "Runs at log-in"), &on_desc);
    auto_card.set_control(auto.widget());
    let sync: Rc<dyn Fn()> = {
        let (auto, card, allowed) = (auto.clone(), auto_card.clone(), allowed.clone());
        Rc::new(move || {
            auto.set_sensitive(allowed.get());
            card.set_description(&if allowed.get() {
                on_desc.clone()
            } else {
                t("Hiervoor moet de app op de achtergrond mogen worden uitgevoerd", "The app must be allowed to run in the background for this").to_owned()
            });
        })
    };
    sync();
    let auto_group = SettingsGroup::new("", "");
    auto_group.add(auto_card.widget());
    if !sandboxed {
        return (None, auto_group, None);
    }
    let dd = Dropdown::new(
        vec![DropdownOption::new("always", t("Altijd", "Always")), DropdownOption::new("never", t("Nooit", "Never"))],
        Some(if allowed.get() { "always" } else { "never" }),
    );
    dd.widget().set_widget_name("st-app-advanced-background");
    let portal = common::s(&app["portalId"]).to_owned();
    {
        let (pp, dd2, allowed, sync) = (p.clone(), dd.clone(), allowed.clone(), sync.clone());
        dd.connect_changed(move |v| {
            let on = v == "always";
            let before = allowed.replace(on);
            sync();
            let fut = pp.run("privacy.setPermission", json!({ "table": "background", "id": "background", "app": portal, "value": on }));
            let (dd3, allowed, sync) = (dd2.clone(), allowed.clone(), sync.clone());
            pp.spawn(async move {
                if fut.await.is_none() {
                    allowed.set(before);
                    dd3.set_selected(Some(if before { "always" } else { "never" }));
                    sync();
                }
            });
        });
    }
    let card = SettingsCard::new("process", t("Deze app op de achtergrond laten uitvoeren", "Let this app run in the background"), "");
    card.set_control(&common::sized(dd.widget(), 180));
    let g = SettingsGroup::new(t("Machtigingen voor apps op de achtergrond", "Background apps permissions"), "");
    g.add(card.widget());
    (Some(g), auto_group, Some(BackgroundState { dropdown: dd, allowed, sync }))
}

/// Algemene sneltoetsen (GNOME 50): the actions the app registered, Wijzigen and Alles verwijderen in a dialog.
fn global_shortcuts(p: &Page, app: &Rc<Value>) -> Option<SettingsGroup> {
    let list = app["globalShortcuts"]["shortcuts"].as_array().filter(|l| !l.is_empty())?.clone();
    let name = common::s(&app["name"]).to_owned();
    let g = SettingsGroup::new(t("Algemene sneltoetsen", "Global shortcuts"), "");
    let desc = common::plural(list.len() as i64, t("1 sneltoets geregistreerd", "1 shortcut registered"), t("{n} sneltoetsen geregistreerd", "{n} shortcuts registered"));
    let card = SettingsCard::new("keyboard", t("Sneltoetsen voor het hele systeem", "Shortcuts for the whole system"), &desc);
    let b = c::button(t("Weergeven", "View"), Variant::Standard);
    b.set_widget_name("st-app-advanced-global-shortcuts");
    let (pp, id) = (p.clone(), common::s(&app["id"]).to_owned());
    b.connect_clicked(move |_| shortcuts_dialog(&pp, &id, &name, &list));
    card.set_control(&b);
    g.add(card.widget());
    Some(g)
}

fn shortcuts_dialog(p: &Page, id: &str, name: &str, list: &[Value]) {
    let body = gtk::Box::new(gtk::Orientation::Vertical, 8);
    body.append(&c::label(&common::fill(t("{app} heeft de volgende algemene sneltoetsen geregistreerd", "{app} has registered the following global shortcuts"), &[("app", name)]), "w11-body", true));
    let dialog: Rc<RefCell<Option<Dialog>>> = Rc::default();
    for s in list {
        let action = common::s(&s["id"]).to_owned();
        let label = Some(common::s(&s["description"])).filter(|d| !d.is_empty()).unwrap_or(&action).to_owned();
        let card = SettingsCard::new("", &label, &accel_text(&strings(&s["accels"])));
        let change = c::button(t("Wijzigen", "Change"), Variant::Standard);
        change.set_widget_name(&format!("st-app-advanced-shortcut-{}", action.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>()));
        let (pp, id, card2) = (p.clone(), id.to_owned(), card.clone());
        change.connect_clicked(move |_| {
            let (pp2, id, action, card3) = (pp.clone(), id.clone(), action.clone(), card2.clone());
            KeyCapture::run(&pp, &common::fill(t("Sneltoets voor {action}", "Shortcut for {action}"), &[("action", &label)]), move |capture| {
                let accels: Vec<String> = match capture {
                    Capture::Accel(a) => vec![a],
                    Capture::Disabled => vec![],
                    Capture::Cancelled => return,
                };
                let fut = pp2.run("privacy.app.globalShortcuts.set", json!({ "id": id, "shortcuts": [{ "id": action, "accels": accels.clone() }] }));
                pp2.spawn(async move {
                    if fut.await.is_some() {
                        card3.set_description(&accel_text(&accels));
                    }
                });
            });
        });
        card.set_control(&change);
        body.append(card.widget());
    }
    let remove = c::button(t("Alles verwijderen", "Remove all"), Variant::Standard);
    remove.set_widget_name("st-app-advanced-shortcuts-remove-all");
    remove.set_halign(gtk::Align::Start);
    {
        let (pp, id, name, dialog) = (p.clone(), id.to_owned(), name.to_owned(), dialog.clone());
        remove.connect_clicked(move |_| {
            let (pp2, id, dialog) = (pp.clone(), id.clone(), dialog.clone());
            c::confirm(
                &pp.anchor(),
                t("Alle sneltoetsen verwijderen?", "Remove all shortcuts?"),
                &common::fill(
                    t("Alle acties van {app} die voor algemene sneltoetsen zijn geregistreerd, worden verwijderd.", "All actions from {app} that have been registered for global shortcuts will be removed."),
                    &[("app", &name)],
                ),
                t("Verwijderen", "Remove"),
                true,
                move |ok| {
                    if !ok {
                        return;
                    }
                    let fut = pp2.run("privacy.app.globalShortcuts.set", json!({ "id": id, "clear": true }));
                    pp2.spawn(async move {
                        if fut.await.is_some() {
                            let d = dialog.borrow().clone();
                            if let Some(d) = d {
                                d.close(kit::controls::DialogResult::Close);
                            }
                        }
                    });
                },
            );
        });
    }
    body.append(&remove);
    let d = Dialog::new(t("Algemene sneltoetsen", "Global shortcuts")).body(&body).close_label(t("Sluiten", "Close")).width(480);
    *dialog.borrow_mut() = Some(d.clone());
    d.show(&p.anchor(), |_| {});
}

/// Vereiste machtigingen: a Flatpak's static permissions (to read), a snap's interfaces (to switch).
fn required(p: &Page, app: &Rc<Value>) -> Option<SettingsGroup> {
    let req = &app["required"];
    let name = common::s(&app["name"]).to_owned();
    match req["kind"].as_str()? {
        "flatpak" => {
            let rows = req["static"].as_array().filter(|r| !r.is_empty())?;
            let g = SettingsGroup::new(
                t("Vereiste machtigingen", "Required permissions"),
                &common::fill(
                    t(
                        "{app} heeft toegang nodig tot de volgende systeembronnen. Als je deze toegang wilt stoppen, moet de app worden verwijderd.",
                        "{app} requires access to the following system resources. To stop this access, the app must be removed.",
                    ),
                    &[("app", &name)],
                ),
            );
            for r in rows {
                if let Some((title, access)) = static_text(common::s(&r["key"]), common::s(&r["access"])) {
                    g.add(value_card("shield", title, access).widget());
                }
            }
            Some(g)
        }
        "snap" => {
            let plugs = req["plugs"].as_array().filter(|r| !r.is_empty())?;
            let g = SettingsGroup::new(
                t("Vereiste machtigingen", "Required permissions"),
                &common::fill(t("Kies waartoe {app} toegang heeft. Een wijziging vraagt om je wachtwoord.", "Choose what {app} can access. A change asks for your password."), &[("app", &name)]),
            );
            for plug in plugs {
                g.add(snap_row(p, app, plug).widget());
            }
            Some(g)
        }
        _ => None,
    }
}

fn slot_key(slot: &Value) -> String {
    format!("{}:{}", common::s(&slot["snap"]), common::s(&slot["slot"]))
}

fn snap_row(p: &Page, app: &Rc<Value>, plug: &Value) -> SettingsCard {
    let id = common::s(&app["id"]).to_owned();
    let plug_name = common::s(&plug["plug"]).to_owned();
    let interface = common::s(&plug["interface"]);
    let label = interface_label(interface);
    let card = SettingsCard::new("shield", &label, if plug_name != interface { plug_name.as_str() } else { "" });
    let slots: Vec<Value> = plug["slots"].as_array().cloned().unwrap_or_default();
    let chosen = Rc::new(RefCell::new(if plug["slot"].is_object() { plug["slot"].clone() } else { slots.first().cloned().unwrap_or(Value::Null) }));
    let controls = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    if slots.len() > 1 {
        let options: Vec<DropdownOption> = slots.iter().map(|s| DropdownOption::new(&slot_key(s), &slot_key(s))).collect();
        let dd = Dropdown::new(options, Some(&slot_key(&chosen.borrow())));
        dd.widget().set_widget_name(&format!("st-app-advanced-slot-{plug_name}"));
        dd.widget().set_sensitive(plug["connected"] != true);
        let (chosen, slots) = (chosen.clone(), slots.clone());
        dd.connect_changed(move |v| {
            if let Some(s) = slots.iter().find(|s| slot_key(s) == v) {
                *chosen.borrow_mut() = s.clone();
            }
        });
        controls.append(dd.widget());
    }
    let sw = ToggleSwitch::new(plug["connected"] == true);
    sw.switch().set_widget_name(&format!("st-app-advanced-plug-{plug_name}"));
    let (pp, sw2) = (p.clone(), sw.clone());
    sw.connect_toggled(move |on| {
        let mut args = json!({ "id": id, "plug": plug_name, "connect": on });
        if chosen.borrow().is_object() {
            args["slot"] = chosen.borrow().clone();
        }
        let fut = pp.run("privacy.app.snapConnect", args);
        let sw3 = sw2.clone();
        pp.spawn(async move {
            if fut.await.is_none() {
                sw3.set_active(!on);
            }
        });
    });
    controls.append(sw.widget());
    card.set_control(&controls);
    card
}

/// Standaardkoppelingen (GNOME's Files & Links): the app's file types and link types, whether it opens each of them
/// now (Als standaard instellen otherwise), removing one, and Opnieuw instellen. Also defaultapps.rs's app view.
pub(super) fn handlers_group(p: &Page, app: &Rc<Value>) -> SettingsGroup {
    let name = common::s(&app["name"]).to_owned();
    let id = common::s(&app["id"]).to_owned();
    let handlers: Vec<Value> = app["handlers"].as_array().cloned().unwrap_or_default();
    let g = SettingsGroup::new(
        t("Standaardkoppelingen", "Default handlers"),
        &if handlers.is_empty() {
            common::fill(t("{app} opent geen bestandstypen of koppelingen.", "{app} doesn’t open any file or link types."), &[("app", &name)])
        } else {
            common::fill(t("{app} wordt gebruikt om de volgende typen bestanden en koppelingen te openen", "{app} is used to open the following types of files and links"), &[("app", &name)])
        },
    );
    for (kind, heading) in [("file", t("Bestandstypen", "File types")), ("link", t("Koppelingstypen", "Link types"))] {
        let rows: Vec<&Value> = handlers.iter().filter(|h| h["kind"] == kind).collect();
        if rows.is_empty() {
            continue;
        }
        g.add(&c::label(heading, "st-handler-heading", false));
        for h in rows {
            g.add(handler_row(p, &id, h).widget());
        }
    }
    if !handlers.is_empty() {
        let card = SettingsCard::new("refresh", t("Opnieuw instellen", "Reset"), t("Je eigen keuzes voor deze typen verwijderen", "Remove your own choices for these types"));
        let b = c::button(t("Opnieuw instellen", "Reset"), Variant::Standard);
        b.set_widget_name("st-app-advanced-handlers-reset");
        let (pp, id) = (p.clone(), id.clone());
        b.connect_clicked(move |_| {
            let fut = pp.run("privacy.app.resetHandlers", json!({ "id": id }));
            let p2 = pp.clone();
            pp.spawn(async move {
                if fut.await.is_some() {
                    p2.refresh();
                }
            });
        });
        card.set_control(&b);
        g.add(card.widget());
    }
    g
}

fn handler_row(p: &Page, id: &str, h: &Value) -> SettingsCard {
    let kind = common::s(&h["type"]).to_owned();
    let title = handler_title(h);
    let desc = if h["kind"] == "file" && title != common::s(&h["name"]) { common::s(&h["name"]).to_owned() } else { kind.clone() };
    let card = SettingsCard::new(if h["kind"] == "link" { "link" } else { "file" }, &title, &desc);
    let controls = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    controls.set_valign(gtk::Align::Center);
    let safe: String = kind.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    if h["isDefault"] == true {
        controls.append(&common::value_label(t("Standaard", "Default")));
    } else {
        let set = c::button(t("Als standaard instellen", "Set as default"), Variant::Standard);
        set.set_widget_name(&format!("st-app-advanced-default-{safe}"));
        let (pp, id, kind) = (p.clone(), id.to_owned(), kind.clone());
        set.connect_clicked(move |_| {
            let fut = pp.run("apps.setDefaultForType", json!({ "mime": kind, "id": id }));
            let p2 = pp.clone();
            pp.spawn(async move {
                if fut.await.is_some() {
                    p2.refresh();
                }
            });
        });
        controls.append(&set);
    }
    let remove = c::icon_button("delete", t("Type verwijderen", "Remove type"), 32);
    remove.set_widget_name(&format!("st-app-advanced-remove-{safe}"));
    let (pp, id) = (p.clone(), id.to_owned());
    remove.connect_clicked(move |_| {
        let fut = pp.run("privacy.app.removeHandler", json!({ "id": id, "type": kind }));
        let p2 = pp.clone();
        pp.spawn(async move {
            if let Some(r) = fut.await {
                if r["dryRun"] != true {
                    p2.toast(Toast::new(t("Type verwijderd", "Type removed")).timeout(2500));
                }
                p2.refresh();
            }
        });
    });
    controls.append(&remove);
    card.set_control(&controls);
    card
}

/// Beëindigen: every process of the app (Taakbeheer's app group, sysmon.processes) through sysmon.endTask.
fn terminate(p: &Page, app: &Rc<Value>) -> SettingsGroup {
    let g = SettingsGroup::new(t("Beëindigen", "Terminate"), "");
    let card = SettingsCard::new("process", t("Beëindigen", "Terminate"), t("Deze app en de bijbehorende processen onmiddellijk beëindigen.", "Immediately terminate this app and its related processes."));
    let b = c::button(t("Beëindigen", "Terminate"), Variant::Standard);
    b.set_widget_name("st-app-advanced-terminate");
    let (pp, id, name) = (p.clone(), common::s(&app["id"]).to_owned(), common::s(&app["name"]).to_owned());
    b.connect_clicked(move |_| {
        let fut = pp.call("sysmon.processes", json!({}));
        let (p2, id, name) = (pp.clone(), id.clone(), name.clone());
        pp.spawn(async move {
            let pids = fut.await.ok().map(|v| app_pids(&v, &id)).unwrap_or_default();
            if pids.is_empty() {
                p2.toast(Toast::new(&common::fill(t("{app} wordt niet uitgevoerd.", "{app} isn’t running."), &[("app", &name)])).timeout(3000));
                return;
            }
            if let Some(r) = p2.run("sysmon.endTask", json!({ "pids": pids })).await
                && r["dryRun"] != true
            {
                common::toast_success(&p2.anchor(), &common::fill(t("{app} is beëindigd.", "{app} was terminated."), &[("app", &name)]), 2500);
            }
        });
    });
    card.set_control(&b);
    g.add(card.widget());
    g
}

/// The pids of the app's group in sysmon.processes (`apps[].desktopId`, with or without ".desktop").
pub(super) fn app_pids(processes: &Value, id: &str) -> Vec<Value> {
    let bare = id.trim_end_matches(".desktop");
    processes["apps"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|a| a["desktopId"].as_str().is_some_and(|d| d == id || d.trim_end_matches(".desktop") == bare))
        .flat_map(|a| a["pids"].as_array().cloned().unwrap_or_default())
        .collect()
}

/// Verwijderen: the reviewed plan of installed.rs (software.uninstallInfo decides whether it can).
fn uninstall(p: &Page, app: &Rc<Value>) -> SettingsGroup {
    let g = SettingsGroup::new(t("Verwijderen", "Uninstall"), "");
    let card = SettingsCard::new("delete", t("Verwijderen", "Uninstall"), t("Deze app en de instellingen ervan verwijderen. Je documenten worden niet beïnvloed.", "Uninstall this app and its settings. Your documents will not be affected."));
    let b = c::button(t("Verwijderen", "Uninstall"), Variant::Standard);
    b.set_widget_name("st-app-advanced-uninstall");
    b.set_sensitive(false);
    let id = common::s(&app["id"]).to_owned();
    {
        let fut = p.call("software.uninstallInfo", json!({ "id": id }));
        let (b, card) = (b.clone(), card.clone());
        p.spawn(async move {
            match fut.await {
                Ok(info) if info["supported"] == true => b.set_sensitive(true),
                Ok(info) => {
                    if let Some(reason) = info["reason"].as_str().filter(|r| !r.is_empty()) {
                        card.set_description(reason);
                    }
                }
                Err(_) => {}
            }
        });
    }
    let anchor = p.anchor();
    b.connect_clicked(move |_| software::review_and_track(&anchor, json!({ "action": "uninstall", "id": id })));
    card.set_control(&b);
    g.add(card.widget());
    g
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_search_switch_follows_gnomes_two_lists() {
        let id = "org.gnome.Notes.desktop";
        assert!(search_on(false, id, &[], &[]), "a provider is on unless it is disabled");
        assert!(!search_on(false, id, &list(&[id]), &[]));
        assert!(!search_on(true, id, &[], &[]), "a DefaultDisabled provider is off unless it is enabled");
        assert!(search_on(true, id, &[], &list(&[id])));
        assert_eq!(search_list(false, false, id, &list(&["a.desktop"])), list(&["a.desktop", id]), "off: into disabled");
        assert_eq!(search_list(true, false, id, &list(&[id, "a.desktop"])), list(&["a.desktop"]), "on: out of disabled");
        assert_eq!(search_list(true, true, id, &[]), list(&[id]), "on: into enabled");
        assert_eq!(search_list(false, true, id, &list(&[id])), Vec::<String>::new(), "off: out of enabled");
        assert_eq!(search_list(false, false, id, &list(&[id])), list(&[id]), "never twice");
    }

    #[test]
    fn the_sections_follow_windows_order_then_winpoints_additions() {
        use Section::*;
        // a desktop app: no background choice, no cache to clear, nothing required
        let host = json!({"kind": "host", "sandboxed": false, "background": null});
        assert_eq!(sections(&host), [Specifications, Permissions, Autostart, Terminate, Uninstall, Handlers]);
        // a Flatpak with everything: Windows' Opnieuw instellen (Clear Cache) between Beëindigen and Verwijderen,
        // the GNOME additions after Verwijderen
        let flatpak = json!({
            "kind": "flatpak", "sandboxed": true, "background": {"set": true, "allowed": true}, "canClearCache": true,
            "globalShortcuts": {"shortcuts": [{"id": "a"}]},
            "required": {"kind": "flatpak", "static": [{"key": "network"}]},
        });
        assert_eq!(
            sections(&flatpak),
            [Specifications, Permissions, Background, Autostart, Terminate, Reset, Uninstall, GlobalShortcuts, Required, Handlers]
        );
        let snap = json!({"kind": "snap", "sandboxed": true, "background": {"set": false, "allowed": true},
            "required": {"kind": "snap", "plugs": []}, "globalShortcuts": {"shortcuts": []}});
        assert_eq!(sections(&snap), [Specifications, Permissions, Background, Autostart, Terminate, Uninstall, Handlers], "empty lists add nothing");
        let order = sections(&flatpak);
        let at = |s: Section| order.iter().position(|x| *x == s).unwrap();
        assert!(at(Terminate) < at(Reset) && at(Reset) < at(Uninstall));
        assert!([GlobalShortcuts, Required, Handlers].iter().all(|s| at(*s) > at(Uninstall)), "WinPoint's additions come after Windows' rows");
    }

    #[test]
    fn permission_rows_follow_the_store_and_the_device_switches() {
        let info = json!({"permissions": [
            {"key": "location", "set": true, "allowed": false},
            {"key": "camera", "set": true, "allowed": true},
            {"key": "microphone", "set": false, "allowed": true},
            {"key": "unknown", "set": true, "allowed": true},
        ]});
        assert_eq!(shown_permissions(&info), [("location".to_owned(), false), ("camera".to_owned(), true)], "GNOME: only the rows the entry has");
        assert!(shown_permissions(&json!({})).is_empty());
        // the device switches of Privacy en beveiliging: disable-camera / disable-microphone, location enabled
        assert!(device_off("camera", &json!(true)) && !device_off("camera", &json!(false)));
        assert!(device_off("microphone", &json!(true)) && !device_off("microphone", &json!(false)));
        assert!(device_off("location", &json!(false)) && !device_off("location", &json!(true)));
        assert!(!device_off("screenshot", &json!(true)), "no device switch");
        assert!(device_key("camera").is_some() && device_key("wallpaper").is_none());
        // Zoeken: greyed (with GNOME's text) while app search is off, also when the list is writable
        assert!(search_state(true, false).0);
        assert!(!search_state(true, true).0 && !search_state(false, false).0);
        assert_ne!(search_state(true, true).1, search_state(true, false).1);
    }

    #[test]
    fn rows_have_texts_and_the_processes_their_app() {
        for key in ["location", "camera", "microphone", "screenshot", "wallpaper", "shortcutsInhibit"] {
            assert!(permission_text(key).is_some(), "{key}");
        }
        assert!(permission_text("background").is_none(), "background has a section of its own");
        for key in ["systemBus", "sessionBus", "devices", "network", "home", "fileSystem", "settings", "sound"] {
            assert!(static_text(key, "full").is_some(), "{key}");
        }
        assert_ne!(static_text("home", "read"), static_text("home", "full"));
        assert_eq!(interface_label("x11"), "x11", "an unknown interface keeps its name");
        assert_ne!(interface_label("camera"), "camera");
        let html = json!({"type":"text/html","kind":"file","name":"HTML document","extensions":[".html",".htm"]});
        assert_eq!(handler_title(&html), ".html, .htm");
        let mail = json!({"type":"x-scheme-handler/mailto","kind":"link","name":"mailto links","extensions":[]});
        assert_eq!(handler_title(&mail), "mailto links");
        let processes = json!({"apps":[
            {"desktopId":"org.gnome.Maps.desktop","pids":[10, 11]},
            {"desktopId":"org.gnome.Maps","pids":[12]},
            {"desktopId":"org.gnome.Mapsx.desktop","pids":[13]}]});
        assert_eq!(app_pids(&processes, "org.gnome.Maps.desktop"), [json!(10), json!(11), json!(12)]);
        assert!(app_pids(&json!({}), "a.desktop").is_empty());
        let list = [json!({"id": "org.gnome.Nautilus.desktop", "name": "Bestanden"})];
        assert_eq!(with_list_name(json!({"id": "org.gnome.Nautilus.desktop", "name": "Files"}), &list)["name"], "Bestanden");
        assert_eq!(with_list_name(json!({"id": "x.desktop", "name": "X"}), &list)["name"], "X");
    }
}
