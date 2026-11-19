//! Instellingen › Netwerk en internet › Wi-Fi › Wi-Fi-eigenschappen (a data page: `{uuid}` of the profile) and
//! Bekende netwerken beheren (batch C, notes/claude/settings-parity.md §2): GNOME Settings' Wi-Fi connection editor
//! and Saved Networks dialog in the layout of Windows 11's "<SSID> properties" and "Manage known networks" pages.
//! Every control writes NetworkManager's profile through `net.connection.update` (the host merges the one key into
//! NM's settings); the IP, DNS and security dialogs are net_editor.rs's.
use super::net_editor as ed;
use super::network::{ip_rows, specs};
use super::{Builder, Content, Page, PageFuture};
use crate::common;
use kit::controls::{self as c, Chevron, Dialog, Dropdown, DropdownOption, SettingsCard, SettingsExpander, SettingsGroup, Severity, Toast, ToggleSwitch, Variant};
use kit::prelude::*;
use kit::t;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub const PAGES: &[(&str, Builder)] = &[("wifi-network", wifi_network), ("wifi-known", wifi_known)];
pub const CSS: &str = "";

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}

/// "Willekeurige hardwareadressen": Windows' three choices, plus what the profile holds when it is none of them
/// ("Standaard" when unset, "Behouden", or the fixed address GNOME's Identity tab can set).
pub fn random_mac_options(current: &str) -> (Vec<DropdownOption>, String) {
    let mut opts: Vec<DropdownOption> = ed::RANDOM_MAC.iter().map(|r| DropdownOption::new(r.0, if kit::i18n::en() { r.2 } else { r.1 })).collect();
    let key = if current.is_empty() { "default".to_owned() } else { current.to_owned() };
    if !ed::RANDOM_MAC.iter().any(|r| r.0 == key) {
        let label = match key.as_str() {
            "default" => t("Standaard", "Default").to_owned(),
            "preserve" => t("Behouden", "Preserve").to_owned(),
            mac => mac.to_uppercase(),
        };
        opts.push(DropdownOption::new(&key, &label));
    }
    (opts, key)
}

/// The change of a "Willekeurige hardwareadressen" choice (`default` removes the key: NetworkManager's default).
pub fn random_mac_changes(choice: &str) -> Value {
    let v = if choice == "default" { Value::Null } else { json!(choice) };
    json!({ "802-11-wireless": { "assigned-mac-address": v, "cloned-mac-address": null } })
}

/// A switch card that writes one change through net.connection.update (optimistic, reverted after an error).
fn switch_card(p: &Page, uuid: &str, title: &str, description: &str, on: bool, name: &str, changes: impl Fn(bool) -> Value + 'static) -> SettingsCard {
    let sw = ToggleSwitch::new(on);
    sw.switch().set_widget_name(name);
    let (pp, uuid, sw2) = (p.clone(), uuid.to_owned(), sw.clone());
    sw.connect_toggled(move |on| {
        let sw = sw2.clone();
        ed::save_keys(&pp, &uuid, changes(on), move || sw.set_active(!on));
    });
    let card = SettingsCard::new("", title, description);
    card.set_control(sw.widget());
    card
}

/// A drop-down card that writes its choice through net.connection.update.
fn dropdown_card(p: &Page, uuid: &str, title: &str, description: &str, choices: (Vec<DropdownOption>, String), name: &str, changes: impl Fn(&str) -> Value + 'static) -> SettingsCard {
    let (opts, current) = choices;
    let dd = Dropdown::new(opts, Some(&current));
    dd.widget().set_widget_name(name);
    let (pp, uuid, dd2) = (p.clone(), uuid.to_owned(), dd.clone());
    let last = Rc::new(RefCell::new(current));
    dd.connect_changed(move |v| {
        let before = last.borrow().clone();
        *last.borrow_mut() = v.to_owned();
        let (dd, last2) = (dd2.clone(), last.clone());
        ed::save_keys(&pp, &uuid, changes(v), move || {
            dd.set_selected(Some(&before));
            *last2.borrow_mut() = before;
        });
    });
    let card = SettingsCard::new("", title, description);
    card.set_control(dd.widget());
    card
}

fn gone(p: &Page) -> Content {
    let group = SettingsGroup::new("", "");
    group.add(common::notice(t("Dit netwerk is niet (meer) opgeslagen op deze pc.", "This network isn’t saved on this PC (any more).")).widget());
    group.add(common::link_card(p, "wifi-known").widget());
    vec![group.widget().clone().upcast()]
}

fn wifi_network(p: Page) -> PageFuture {
    Box::pin(async move {
        let uuid = p.param("uuid").unwrap_or_default();
        let (cr, dr) = (p.call("net.connection.get", json!({ "uuid": uuid })), p.get("net.devices", json!({})));
        let (cr, devs) = (cr.await, dr.await);
        if !p.alive() {
            return Content::new();
        }
        let conn = match cr {
            Ok(v) if v["settings"].is_object() => v,
            Err(e) if e.code == "unknown_method" || e.code == "unsupported" => {
                let text = if e.code == "unknown_method" { common::ct::unavailable_here().to_owned() } else { e.message };
                return vec![common::notice(&text).widget().clone().upcast()];
            }
            _ => return gone(&p),
        };
        let settings = conn["settings"].clone();
        let ssid = ed::ssid_text(&settings["802-11-wireless"]["ssid"]);
        p.set_title(if ssid.is_empty() { s(&settings["connection"]["id"]) } else { ssid.as_str() });
        let wifi_devs: Vec<Value> = devs
            .as_ref()
            .and_then(|d| d["devices"].as_array().cloned())
            .unwrap_or_default()
            .into_iter()
            .filter(|d| d["type"] == "wifi")
            .collect();
        let dev = wifi_devs.iter().find(|d| d["active"]["uuid"] == json!(uuid)).cloned();
        let saved: Rc<dyn Fn()> = {
            let pp = p.clone();
            Rc::new(move || pp.refresh())
        };
        let mut out: Content = Vec::new();

        // ---- connecting (Windows' top of the page, with GNOME's "Make available to other users")
        let group = SettingsGroup::new("", "");
        group.add(
            switch_card(
                &p,
                &uuid,
                t("Automatisch verbinding maken als ik binnen bereik ben", "Connect automatically when in range"),
                "",
                settings["connection"]["autoconnect"] != false,
                "st-wifi-network-autoconnect",
                |on| json!({ "connection": { "autoconnect": on } }),
            )
            .widget(),
        );
        group.add(
            switch_card(
                &p,
                &uuid,
                t("Beschikbaar maken voor andere gebruikers", "Make available to other users"),
                t("Andere gebruikers van deze pc kunnen dit netwerk ook gebruiken", "Other users of this PC can use this network too"),
                ed::shared(&settings),
                "st-wifi-network-permissions",
                |on| json!({ "connection": { "permissions": if on { json!([]) } else { ed::private_permissions() } } }),
            )
            .widget(),
        );
        let mac_now = s(&settings["802-11-wireless"]["assigned-mac-address"]).to_owned();
        let mac_now = if mac_now.is_empty() { ed::mac_text(&settings["802-11-wireless"]["cloned-mac-address"]) } else { mac_now };
        group.add(
            dropdown_card(
                &p,
                &uuid,
                t("Willekeurige hardwareadressen", "Random hardware addresses"),
                t(
                    "Help je privacy te beschermen door het moeilijker te maken om de locatie van je apparaat te volgen wanneer je verbinding maakt met dit netwerk",
                    "Help protect your privacy by making it harder for people to track your device location when you connect to this network",
                ),
                random_mac_options(&mac_now),
                "st-wifi-network-assigned-mac-address",
                random_mac_changes,
            )
            .widget(),
        );
        out.push(group.widget().clone().upcast());

        // ---- metered, IP and DNS (shared with Ethernet)
        let group = SettingsGroup::new("", "");
        for w in ip_rows(&p, "wifi-network", &uuid, &settings, saved.clone()) {
            group.add(&w);
        }
        out.push(group.widget().clone().upcast());

        // ---- security: the type with Bewerken, the key with Weergeven (and its QR code)
        let group = SettingsGroup::new("", "");
        let sec = SettingsCard::new("", t("Beveiligingstype:", "Security type:"), &ed::security_text(&settings));
        let edit = c::button(t("Bewerken", "Edit"), Variant::Standard);
        edit.set_widget_name("wifi-network-security-edit");
        {
            let (pp, u, st, sv) = (p.clone(), uuid.clone(), settings.clone(), saved.clone());
            edit.connect_clicked(move |_| ed::edit_security(&pp, &u, &st, sv.clone()));
        }
        sec.set_control(&edit);
        group.add(sec.widget());
        if matches!(ed::security_kind(&settings), "wpa-psk" | "sae" | "wep" | "wep-passphrase" | "leap") {
            let key = SettingsCard::new("", t("Wi-Fi-beveiligingssleutel weergeven", "View Wi-Fi security key"), t("Ook als QR-code om met een telefoon te delen", "Also as a QR code to share with a phone"));
            let view = c::button(t("Weergeven", "View"), Variant::Standard);
            view.set_widget_name("wifi-network-key-view");
            let (pp, u) = (p.clone(), uuid.clone());
            view.connect_clicked(move |_| ed::show_key(&pp, &u));
            key.set_control(&view);
            group.add(key.widget());
        }
        out.push(group.widget().clone().upcast());

        // ---- Eigenschappen with Kopiëren (GNOME's Details)
        let mut rows = match &dev {
            Some(d) => ed::properties(d, Some(&settings)),
            None => vec![(t("SSID:", "SSID:").to_owned(), ssid.clone()), (t("Beveiligingstype:", "Security type:").to_owned(), ed::security_text(&settings))],
        };
        let used = ed::last_used(&settings);
        if !used.is_empty() {
            rows.push((t("Laatst gebruikt:", "Last used:").to_owned(), used));
        }
        let props = SettingsExpander::new("", t("Eigenschappen", "Properties"), if dev.is_some() { t("Verbonden", "Connected") } else { t("Niet verbonden", "Not connected") });
        props.header().set_control(&ed::copy_button(&p, rows.clone(), "wifi-network-copy"));
        props.set_expanded(true);
        props.add(&specs(&rows, true));
        let group = SettingsGroup::new("", "");
        group.add(props.widget());

        // ---- Geavanceerd (GNOME's Identity: BSSID, the adapter's MAC address)
        let adv = SettingsExpander::new("", t("Geavanceerd", "Advanced"), t("BSSID en de Wi-Fi-adapter van dit netwerk", "BSSID and the Wi-Fi adapter of this network"));
        let none = || DropdownOption::new("", t("Geen", "None"));
        let bssid_now = ed::mac_text(&settings["802-11-wireless"]["bssid"]);
        let mut seen: Vec<String> = settings["802-11-wireless"]["seen-bssids"].as_array().map(|a| a.iter().filter_map(|b| b.as_str().map(str::to_uppercase)).collect()).unwrap_or_default();
        if !bssid_now.is_empty() && !seen.contains(&bssid_now) {
            seen.push(bssid_now.clone());
        }
        let mut bssids = vec![none()];
        bssids.extend(seen.iter().map(|b| DropdownOption::new(b, b)));
        let only_bssid = |v: &str| json!({ "802-11-wireless": { "bssid": if v.is_empty() { Value::Null } else { json!(v) } } });
        adv.add(dropdown_card(&p, &uuid, t("BSSID", "BSSID"), t("Alleen verbinden met dit toegangspunt", "Only connect to this access point"), (bssids, bssid_now), "st-wifi-network-bssid", only_bssid).widget());
        let mac = ed::mac_text(&settings["802-11-wireless"]["mac-address"]);
        let mut macs = vec![none()];
        for d in &wifi_devs {
            let m = s(if s(&d["permHwAddress"]).is_empty() { &d["hwAddress"] } else { &d["permHwAddress"] }).to_uppercase();
            if !m.is_empty() {
                macs.push(DropdownOption::new(&m, &format!("{m} ({})", s(&d["iface"]))));
            }
        }
        if !mac.is_empty() && !macs.iter().any(|o| o.value == mac) {
            macs.push(DropdownOption::new(&mac, &mac));
        }
        let only_adapter = |v: &str| json!({ "802-11-wireless": { "mac-address": if v.is_empty() { Value::Null } else { json!(v) } } });
        adv.add(
            dropdown_card(&p, &uuid, t("MAC-adres van apparaat", "Device MAC address"), t("De Wi-Fi-adapter die dit netwerk gebruikt", "The Wi-Fi adapter that uses this network"), (macs, mac), "st-wifi-network-mac-address", only_adapter)
                .widget(),
        );
        group.add(adv.widget());

        // ---- Vergeten (GNOME's Forget Connection)
        let forget = SettingsCard::new(
            "",
            t("Dit netwerk vergeten", "Forget this network"),
            t("De opgeslagen instellingen en de beveiligingssleutel van dit netwerk verwijderen", "Remove the saved settings and the security key of this network"),
        );
        let fb = c::button(t("Vergeten", "Forget"), Variant::Standard);
        fb.set_widget_name("wifi-network-forget");
        {
            let (pp, u, name) = (p.clone(), uuid.clone(), ssid.clone());
            fb.connect_clicked(move |_| {
                let (pp2, u2) = (pp.clone(), u.clone());
                c::confirm(
                    &pp.anchor(),
                    &common::fill(t("“{ssid}” vergeten?", "Forget “{ssid}”?"), &[("ssid", &name)]),
                    t("Je moet de beveiligingssleutel opnieuw typen om weer verbinding te maken.", "You’ll have to type the security key again to connect again."),
                    t("Vergeten", "Forget"),
                    true,
                    move |ok| {
                        if !ok {
                            return;
                        }
                        let wait = pp2.run("net.connection.delete", json!({ "uuid": u2 }));
                        let pp3 = pp2.clone();
                        pp2.spawn(async move {
                            if wait.await.is_some() {
                                pp3.go("wifi");
                            }
                        });
                    },
                );
            });
        }
        forget.set_control(&fb);
        group.add(forget.widget());
        out.push(group.widget().clone().upcast());

        // follow NetworkManager: another program changed the profile
        let state = Rc::new(RefCell::new(settings));
        let pending = Rc::new(Cell::new(false));
        {
            let pp = p.clone();
            p.on("net.changed", move |d| {
                if pending.get() || d["what"] != "connections" {
                    return;
                }
                pending.set(true);
                let (pp2, state, pending, uuid) = (pp.clone(), state.clone(), pending.clone(), uuid.clone());
                pp.timeout(600, move || {
                    let wait = pp2.get("net.connection.get", json!({ "uuid": uuid }));
                    let pp3 = pp2.clone();
                    pp2.spawn(async move {
                        pending.set(false);
                        match wait.await {
                            // this page's own switch or drop-down: the controls show it already
                            Some(v) if v["settings"] != *state.borrow() && ed::own_write_recent(&uuid) => *state.borrow_mut() = v["settings"].clone(),
                            Some(v) if v["settings"] != *state.borrow() => pp3.refresh(),
                            None => pp3.refresh(),
                            _ => {}
                        }
                    });
                });
            });
        }
        out
    })
}

// ---------------------------------------------------------------------------------------- known networks

/// Forget with Windows' undo (GNOME's Saved Networks): the row goes at once, the profile after the toast.
fn forget_later(p: &Page, uuid: &str, ssid: &str, row: &gtk::Widget) {
    row.set_visible(false);
    let undone = Rc::new(Cell::new(false));
    let (u2, row2) = (undone.clone(), row.clone());
    let handle = Toast::new(&common::fill(t("“{ssid}” vergeten", "“{ssid}” forgotten"), &[("ssid", ssid)]))
        .severity(Severity::Info)
        .action(t("Ongedaan maken", "Undo"), move || {
            u2.set(true);
            row2.set_visible(true);
        })
        .timeout(5000)
        .show(&p.anchor());
    // not tied to the page: leaving it does not keep the network
    let (uuid, anchor) = (uuid.to_owned(), p.anchor());
    glib::timeout_add_local_once(std::time::Duration::from_millis(5200), move || {
        handle.close();
        if undone.get() {
            return;
        }
        kit::ctx().call("net.connection.delete", json!({ "uuid": uuid }), move |r| match r {
            Ok(v) => common::note_dry_run(&anchor, &v),
            Err(e) => common::show_error(&anchor, &e, None),
        });
    });
}

fn wifi_known(p: Page) -> PageFuture {
    Box::pin(async move {
        let r = p.call("net.connections", json!({ "type": "802-11-wireless" })).await;
        if !p.alive() {
            return Content::new();
        }
        let list: Vec<Value> = match r {
            Ok(v) => v["connections"].as_array().cloned().unwrap_or_default().into_iter().filter(|c| c["mode"] != "ap").collect(),
            Err(e) => {
                let text = if e.code == "unknown_method" { common::ct::unavailable_here().to_owned() } else { e.message };
                return vec![common::notice(&text).widget().clone().upcast()];
            }
        };
        let mut out: Content = Vec::new();
        // ---- Een nieuw netwerk toevoegen
        let group = SettingsGroup::new("", "");
        let add = SettingsCard::new("plus", t("Een nieuw netwerk toevoegen", "Add a new network"), "");
        let ab = c::button(t("Netwerk toevoegen", "Add network"), Variant::Accent);
        ab.set_widget_name("wifi-known-add");
        {
            let pp = p.clone();
            ab.connect_clicked(move |_| {
                let pp2 = pp.clone();
                let done: Rc<dyn Fn(&Dialog, &gtk::Label, ed::NewNetwork)> = Rc::new(move |d: &Dialog, err: &gtk::Label, n: ed::NewNetwork| {
                    let pp3 = pp2.clone();
                    ed::dialog_call(d, err, "net.connection.add", json!({ "settings": ed::new_wifi_settings(&n) }), move |v| {
                        common::note_dry_run(&pp3.anchor(), &v);
                        pp3.refresh();
                    });
                });
                ed::new_network(&pp, t("Een nieuw netwerk toevoegen", "Add a new network"), t("Opslaan", "Save"), true, done);
            });
        }
        add.set_control(&ab);
        group.add(add.widget());
        out.push(group.widget().clone().upcast());

        // ---- the list, with Windows' search
        let group = SettingsGroup::new(t("Bekende netwerken", "Known networks"), "");
        if list.is_empty() {
            group.add(SettingsCard::new("wifi", t("Er zijn geen bekende netwerken", "There are no known networks"), t("Netwerken waarmee je verbinding maakt, worden hier opgeslagen.", "Networks you connect to are saved here.")).widget());
            out.push(group.widget().clone().upcast());
            return out;
        }
        let search = c::text_box(t("Zoeken in deze lijst", "Search this list"));
        search.set_widget_name("wifi-known-search");
        search.set_hexpand(true);
        group.add(&search);
        let rows: Rc<RefCell<Vec<(String, gtk::Widget)>>> = Rc::default();
        for (i, k) in list.iter().enumerate() {
            let uuid = s(&k["uuid"]).to_owned();
            let ssid = if s(&k["ssid"]).is_empty() { s(&k["id"]).to_owned() } else { s(&k["ssid"]).to_owned() };
            let desc = if k["active"] == true {
                t("Verbonden", "Connected").to_owned()
            } else {
                let used = ed::last_used(&json!({ "connection": { "timestamp": k["timestamp"] } }));
                if used.is_empty() { String::new() } else { common::fill(t("Laatst gebruikt: {date}", "Last used: {date}"), &[("date", &used)]) }
            };
            let card = SettingsCard::clickable("wifi", &ssid, &desc, Chevron::Right);
            if let Some(b) = card.button() {
                b.set_widget_name(&format!("wifi-known-{i}"));
            }
            {
                let (pp, u) = (p.clone(), uuid.clone());
                card.connect_clicked(move || pp.go_with("wifi-network", json!({ "uuid": u })));
            }
            let fb = c::button(t("Vergeten", "Forget"), Variant::Standard);
            fb.set_widget_name(&format!("wifi-known-forget-{i}"));
            {
                let (pp, u, name, w) = (p.clone(), uuid.clone(), ssid.clone(), card.widget().clone());
                fb.connect_clicked(move |_| forget_later(&pp, &u, &name, &w));
            }
            card.set_control(&fb);
            group.add(card.widget());
            rows.borrow_mut().push((ssid.to_lowercase(), card.widget().clone()));
        }
        {
            let rows = rows.clone();
            search.connect_changed(move |e| {
                let q = e.text().trim().to_lowercase();
                for (name, w) in rows.borrow().iter() {
                    w.set_visible(q.is_empty() || name.contains(&q));
                }
            });
        }
        out.push(group.widget().clone().upcast());
        // another program added or forgot a network (after the undo time of a forget of our own)
        {
            let (pp, pending) = (p.clone(), Rc::new(Cell::new(false)));
            p.on("net.changed", move |d| {
                if d["what"] == "connections" && !pending.get() {
                    pending.set(true);
                    let pp2 = pp.clone();
                    pp.timeout(5600, move || pp2.refresh());
                }
            });
        }
        out
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_hardware_addresses_offer_what_the_profile_holds() {
        kit::i18n::set_forced_lang(Some("nl"));
        let (opts, key) = random_mac_options("stable");
        assert_eq!((opts.len(), key.as_str()), (3, "stable"));
        let (opts, key) = random_mac_options("");
        assert_eq!((opts.len(), key.as_str()), (4, "default"));
        assert_eq!((opts[3].value.as_str(), opts[3].label.as_str()), ("default", "Standaard"));
        let (opts, key) = random_mac_options("02:00:00:00:00:01");
        assert_eq!(key, "02:00:00:00:00:01");
        assert_eq!(opts.last().unwrap().value, "02:00:00:00:00:01");
        assert_eq!(random_mac_changes("random"), json!({ "802-11-wireless": { "assigned-mac-address": "random", "cloned-mac-address": null } }));
        assert_eq!(random_mac_changes("default")["802-11-wireless"]["assigned-mac-address"], Value::Null);
    }
}
