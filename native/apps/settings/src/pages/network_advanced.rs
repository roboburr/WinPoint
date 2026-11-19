//! Instellingen › Netwerk en internet › Geavanceerde netwerkinstellingen (› Geavanceerde instellingen voor delen), batch
//! C of the settings parity (notes/claude/settings-parity.md §2): Windows 11's Advanced network settings with GNOME's
//! functions behind it.
//!
//! * **network-advanced**: "Netwerkadapters", one expander per adapter with Windows' Uitschakelen / Inschakelen
//!   (GNOME's per-device switch, `net.device.setEnabled`) and "Aanvullende eigenschappen weergeven" (the adapter's own
//!   page); "Meer instellingen": "Hardware- en verbindingseigenschappen" (every adapter's properties with
//!   Kopiëren), GNOME's "Controle op internetverbinding" (Privacy › Connectivity Checking, `net.connectivity.set`) and
//!   the link to Geavanceerde instellingen voor delen. Windows' Netwerk opnieuw instellen (it would delete every
//!   profile) and Gegevensgebruik (no GNOME counterpart) are left out.
//! * **sharing-advanced**: Windows' Advanced sharing settings with GNOME's Sharing panel: Netwerkdetectie (Avahi,
//!   always on, information) and Mediastreaming (GNOME's Media Sharing: the switch for the current network through gsd
//!   Sharing, the folders in rygel.conf, the networks it is shared on), `sharing.media.*`. File Sharing needs
//!   gnome-user-share, which GNOME also hides when it is not installed.
use super::net_editor as ed;
use super::{Builder, Content, Page, PageFuture};
use crate::common::{self, ct};
use kit::controls::{self as c, SettingsCard, SettingsExpander, SettingsGroup, Severity, ToggleSwitch, Variant};
use kit::prelude::*;
use kit::t;
use std::rc::Rc;

pub const PAGES: &[(&str, Builder)] = &[("network-advanced", network_advanced), ("sharing-advanced", sharing_advanced)];
pub const CSS: &str = r#"
.st-na-section { font-weight: 600; }
"#;

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}

/// The adapter kinds Windows lists (NetworkManager's virtual devices only when NetworkManager manages them, never
/// Wi-Fi Direct or veth).
pub fn listed(dev: &Value) -> bool {
    match s(&dev["type"]) {
        "ethernet" | "wifi" | "modem" | "bluetooth" | "wireguard" => true,
        "loopback" | "wifi-p2p" | "veth" => false,
        _ => dev["managed"] != false && s(&dev["state"]) != "unmanaged",
    }
}

/// The adapters of net.devices that the page lists.
pub fn adapters(devs: &Value) -> Vec<Value> {
    devs["devices"].as_array().map(|a| a.iter().filter(|d| listed(d)).cloned().collect()).unwrap_or_default()
}

/// The adapter's glyph.
pub fn adapter_glyph(kind: &str) -> &'static str {
    match kind {
        "ethernet" => "ethernet",
        "wifi" => "wifi-3",
        "modem" => "phone",
        "bluetooth" => "bluetooth",
        "wireguard" | "tun" => "vpn",
        _ => "network",
    }
}

/// The adapter's name as Windows shows it: its kind, with the interface when several have that kind.
pub fn adapter_title(dev: &Value, several: bool) -> String {
    let kind = match s(&dev["type"]) {
        "ethernet" => t("Ethernet", "Ethernet"),
        "wifi" => t("Wi-Fi", "Wi-Fi"),
        "modem" => t("Mobiel netwerk", "Mobile network"),
        "bluetooth" => t("Bluetooth-netwerkverbinding", "Bluetooth Network Connection"),
        "wireguard" => "WireGuard",
        _ => "",
    };
    let iface = s(&dev["iface"]);
    match (kind.is_empty(), several) {
        (true, _) => iface.to_owned(),
        (false, true) => format!("{kind} ({iface})"),
        (false, false) => kind.to_owned(),
    }
}

/// Whether the adapter is turned on as Windows means it: connected or connecting, or waiting with autoconnect allowed
/// (NetworkManager's Device.Disconnect, which GNOME's switch and Windows' Uitschakelen send, blocks autoconnect).
pub fn adapter_on(dev: &Value) -> bool {
    match s(&dev["state"]) {
        "connected" | "connecting" => true,
        "disconnected" | "unplugged" | "unavailable" | "failed" => dev["autoconnect"] != false,
        _ => false,
    }
}

/// Whether Inschakelen can work now (GNOME's switch is insensitive for these too).
pub fn can_enable(dev: &Value) -> bool {
    !matches!(s(&dev["state"]), "unplugged" | "unavailable" | "unmanaged")
}

/// What a change of net.devices means for the page: the adapters' kind, state and autoconnect, and the switches.
pub fn digest(devs: &Value) -> String {
    let mut out: Vec<String> = adapters(devs).iter().map(|d| format!("{}|{}|{}|{}", s(&d["iface"]), s(&d["type"]), s(&d["state"]), d["autoconnect"])).collect();
    out.push(format!("{}|{}", devs["connectivityCheckEnabled"], devs["connectivityCheckAvailable"]));
    out.join(";")
}

/// The full property list of one adapter for "Hardware- en verbindingseigenschappen".
pub fn adapter_rows(dev: &Value) -> Vec<(String, String)> {
    let mut rows = vec![
        (t("Naam:", "Name:").to_owned(), s(&dev["iface"]).to_owned()),
        (t("Status:", "Status:").to_owned(), super::network::device_status(dev)),
    ];
    if let Some(id) = dev["active"]["id"].as_str() {
        rows.push((t("Verbinding:", "Connection:").to_owned(), id.to_owned()));
    }
    if let Some(mtu) = dev["mtu"].as_u64().filter(|m| *m > 0) {
        rows.push(("MTU:".to_owned(), mtu.to_string()));
    }
    rows.extend(ed::properties(dev, None));
    rows
}

/// A key / value block with its adapter's name above it.
fn titled_specs(title: &str, rows: &[(String, String)]) -> gtk::Widget {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 8);
    b.append(&c::label(title, "st-na-section", false));
    b.append(&super::network::specs(rows, true));
    b.upcast()
}

/// "Aanvullende eigenschappen weergeven": the adapter's own page (Windows opens its properties).
fn open_properties(p: &Page, dev: &Value) {
    match (s(&dev["type"]), dev["active"]["uuid"].as_str()) {
        ("wifi", Some(uuid)) => p.go_with("wifi-network", json!({ "uuid": uuid })),
        ("wifi", None) => p.go("wifi"),
        ("modem", _) => p.go("mobile"),
        ("wireguard", _) => p.go("vpn"),
        _ => p.go("ethernet"),
    }
}

fn network_advanced(p: Page) -> PageFuture {
    Box::pin(async move {
        let devs = p.get("net.devices", json!({})).await;
        if !p.alive() {
            return Content::new();
        }
        let Some(devs) = devs.filter(Value::is_object) else {
            return vec![super::network::nm_missing()];
        };
        let list = adapters(&devs);
        let several = |dev: &Value| list.iter().filter(|d| d["type"] == dev["type"]).count() > 1;
        let mut out: Content = Vec::new();

        // Netwerkadapters
        let group = SettingsGroup::new(t("Netwerkadapters", "Network adapters"), "");
        if list.is_empty() {
            group.add(SettingsCard::new("network", t("Geen netwerkadapters gevonden", "No network adapters found"), "").widget());
        }
        for (i, dev) in list.iter().enumerate() {
            let kind = s(&dev["type"]);
            let model = [s(&dev["vendor"]), s(&dev["model"])].iter().filter(|x| !x.is_empty()).copied().collect::<Vec<_>>().join(" ");
            let status = super::network::device_status(dev);
            let desc = if model.is_empty() { status } else { format!("{model} · {status}") };
            let exp = SettingsExpander::new(adapter_glyph(kind), &adapter_title(dev, several(dev)), &desc);
            exp.widget().set_widget_name(&format!("network-advanced-adapter-{i}"));
            let on = adapter_on(dev);
            let b = c::button(if on { t("Uitschakelen", "Disable") } else { t("Inschakelen", "Enable") }, Variant::Standard);
            b.set_widget_name(&format!("network-advanced-toggle-{i}"));
            b.set_sensitive(on || can_enable(dev));
            if !on && !can_enable(dev) {
                b.set_tooltip_text(Some(t("Deze adapter kan nu niet worden ingeschakeld.", "This adapter can’t be enabled right now.")));
            }
            {
                let (pp, iface) = (p.clone(), s(&dev["iface"]).to_owned());
                b.connect_clicked(move |b| {
                    b.set_sensitive(false);
                    let wait = pp.run("net.device.setEnabled", json!({ "iface": iface, "on": !on }));
                    let (pp2, b) = (pp.clone(), b.clone());
                    pp.spawn(async move {
                        if wait.await.is_some() {
                            pp2.refresh();
                        } else {
                            b.set_sensitive(true);
                        }
                    });
                });
            }
            exp.header().set_control(&b);
            let more = SettingsCard::clickable("", t("Aanvullende eigenschappen weergeven", "View additional properties"), "", c::Chevron::Right);
            if let Some(btn) = more.button() {
                btn.set_widget_name(&format!("network-advanced-properties-{i}"));
            }
            {
                let (pp, d) = (p.clone(), dev.clone());
                more.connect_clicked(move || open_properties(&pp, &d));
            }
            exp.add(more.widget());
            group.add(exp.widget());
        }
        out.push(group.widget().clone().upcast());

        // Meer instellingen
        let more = SettingsGroup::new(t("Meer instellingen", "More settings"), "");
        let hw = SettingsExpander::new(
            "properties",
            t("Hardware- en verbindingseigenschappen", "Hardware and connection properties"),
            t("De eigenschappen van alle netwerkadapters", "The properties of every network adapter"),
        );
        hw.widget().set_widget_name("network-advanced-hardware");
        let mut copy_rows: Vec<(String, String)> = Vec::new();
        let body = gtk::Box::new(gtk::Orientation::Vertical, 18);
        for dev in &list {
            let rows = adapter_rows(dev);
            let title = adapter_title(dev, several(dev));
            copy_rows.push((title.clone(), String::new()));
            copy_rows.extend(rows.iter().cloned());
            body.append(&titled_specs(&title, &rows));
        }
        hw.header().set_control(&ed::copy_button(&p, copy_rows, "network-advanced-copy"));
        hw.add(&body);
        more.add(hw.widget());
        // GNOME's Privacy › Connectivity Checking (hidden as GNOME hides it when NetworkManager cannot check)
        if devs["connectivityCheckAvailable"] == true {
            let sw = ToggleSwitch::new(devs["connectivityCheckEnabled"] == true);
            sw.switch().set_widget_name("network-advanced-connectivity");
            let (pp, sw2) = (p.clone(), sw.clone());
            sw.connect_toggled(move |on| {
                let wait = pp.run("net.connectivity.set", json!({ "on": on }));
                let sw = sw2.clone();
                pp.spawn(async move {
                    if wait.await.is_none() {
                        sw.set_active(!on);
                    }
                });
            });
            let card = SettingsCard::new(
                "globe",
                t("Controle op internetverbinding", "Connectivity checking"),
                t(
                    "Verbindingsproblemen detecteren, zodat je online blijft. Hiervoor maakt deze pc af en toe verbinding met een server van je distributie.",
                    "Detect connection problems to help you stay online. To do this, this PC now and then connects to a server of your distribution.",
                ),
            );
            card.set_control(sw.widget());
            more.add(card.widget());
        }
        more.add(common::link_card(&p, "sharing-advanced").widget());
        out.push(more.widget().clone().upcast());

        // follow NetworkManager: rebuilt when an adapter's state or a switch changed
        {
            let (pp, last) = (p.clone(), Rc::new(std::cell::RefCell::new(digest(&devs))));
            p.on("net.changed", move |d| {
                if !matches!(s(&d["what"]), "devices" | "active" | "connectivity" | "state" | "radios") {
                    return;
                }
                let (pp2, last) = (pp.clone(), last.clone());
                let wait = pp.get("net.devices", json!({}));
                pp.spawn(async move {
                    if let Some(v) = wait.await
                        && digest(&v) != *last.borrow()
                    {
                        *last.borrow_mut() = digest(&v);
                        pp2.refresh();
                    }
                });
            });
        }
        out
    })
}

// ------------------------------------------------------------------------------------- sharing-advanced

/// The notice under Mediastreaming for gsd's SharingStatus, None while sharing is possible.
pub fn status_text(media: &Value) -> Option<&'static str> {
    if media["running"] != true {
        return Some(t(
            "Delen in het netwerk is niet beschikbaar: gnome-settings-daemon wordt niet uitgevoerd.",
            "Network sharing isn’t available: gnome-settings-daemon isn’t running.",
        ));
    }
    match s(&media["status"]) {
        "available" => None,
        "offline" => Some(t("Deze pc is niet verbonden met een netwerk.", "This PC isn’t connected to a network.")),
        "disabled-mobile-broadband" => Some(t("Op een mobiel netwerk kan niet worden gedeeld.", "Sharing isn’t possible on a mobile network.")),
        "disabled-low-security" => Some(t("Op een netwerk zonder beveiliging kan niet worden gedeeld.", "Sharing isn’t possible on a network without security.")),
        _ => Some(t("Delen in het netwerk is nu niet beschikbaar.", "Network sharing isn’t available right now.")),
    }
}

/// The folder's glyph: GNOME's special folders by their place, else a folder.
fn folder_glyph(path: &str) -> &'static str {
    let special = |d: glib::UserDirectory| glib::user_special_dir(d).is_some_and(|p| p.to_string_lossy() == path);
    if special(glib::UserDirectory::Music) {
        "music"
    } else if special(glib::UserDirectory::Pictures) {
        "image"
    } else if special(glib::UserDirectory::Videos) {
        "video"
    } else {
        "folder"
    }
}

/// The folder list with one folder added (not twice) or removed.
pub fn folders_with(folders: &[String], add: Option<&str>, remove: Option<&str>) -> Vec<String> {
    let mut out: Vec<String> = folders.iter().filter(|f| Some(f.as_str()) != remove).cloned().collect();
    if let Some(a) = add
        && !out.iter().any(|f| f == a)
    {
        out.push(a.to_owned());
    }
    out
}

/// sharing.media.folders.set, then the page again.
fn save_folders(p: &Page, folders: Vec<String>) {
    let wait = p.run("sharing.media.folders.set", json!({ "folders": folders }));
    let pp = p.clone();
    p.spawn(async move {
        if let Some(v) = wait.await {
            common::note_dry_run(&pp.anchor(), &v);
            pp.refresh();
        }
    });
}

/// Mediastreaming: the switch, the folders and the networks (rygel installed).
fn media_group(p: &Page, media: &Value, out: &mut Content) {
    let group = SettingsGroup::new(
        t("Mediastreaming", "Media streaming"),
        t(
            "Met mediastreaming kunnen apparaten met DLNA in het huidige netwerk je afbeeldingen, muziek en video’s afspelen.",
            "Media streaming lets DLNA devices on the current network play your pictures, music and videos.",
        ),
    );
    let notice = status_text(media);
    let current = s(&media["currentNetworkName"]).to_owned();
    let desc = if current.is_empty() {
        t("Muziek, foto’s en video’s streamen naar apparaten in het huidige netwerk", "Stream music, photos and videos to devices on the current network").to_owned()
    } else {
        common::fill(t("Muziek, foto’s en video’s streamen naar apparaten in {network}", "Stream music, photos and videos to devices on {network}"), &[("network", &current)])
    };
    let sw = ToggleSwitch::new(media["enabled"] == true);
    sw.switch().set_widget_name("sharing-advanced-media");
    sw.set_sensitive(notice.is_none());
    {
        let (pp, sw2) = (p.clone(), sw.clone());
        sw.connect_toggled(move |on| {
            let wait = pp.run("sharing.media.set", json!({ "on": on }));
            let (pp2, sw) = (pp.clone(), sw2.clone());
            pp.spawn(async move {
                match wait.await {
                    Some(_) => pp2.refresh(),
                    None => sw.set_active(!on),
                }
            });
        });
    }
    let main = SettingsCard::new("cast", t("Mediastreaming", "Media streaming"), &desc);
    main.set_control(sw.widget());
    group.add(main.widget());
    if let Some(text) = notice {
        let bar = common::notice_full(Severity::Info, "", text);
        bar.widget().set_widget_name("sharing-advanced-status");
        out.push(bar.widget().clone().upcast());
    }
    // Mappen (rygel.conf)
    let folders: Vec<String> = media["folders"].as_array().map(|a| a.iter().filter_map(|f| f.as_str().map(str::to_owned)).collect()).unwrap_or_default();
    let fx = SettingsExpander::new("folder", t("Mappen", "Folders"), t("De mappen die worden gedeeld", "The folders that are shared"));
    fx.set_expanded(true);
    let add = c::button(t("Map toevoegen", "Add folder"), Variant::Standard);
    add.set_widget_name("sharing-advanced-folder-add");
    {
        let (pp, list) = (p.clone(), folders.clone());
        add.connect_clicked(move |_| {
            let dialog = gtk::FileDialog::new();
            dialog.set_title(t("Een map kiezen", "Choose a folder"));
            dialog.set_modal(true);
            let (pp2, list) = (pp.clone(), list.clone());
            dialog.select_folder(Some(pp.window().window()), gio::Cancellable::NONE, move |r| {
                if let Ok(f) = r
                    && let Some(path) = f.path()
                {
                    save_folders(&pp2, folders_with(&list, Some(&path.to_string_lossy()), None));
                }
            });
        });
    }
    fx.header().set_control(&add);
    if folders.is_empty() {
        fx.add(SettingsCard::new("", t("Er worden geen mappen gedeeld", "No folders are shared"), "").widget());
    }
    for (i, f) in folders.iter().enumerate() {
        let name = std::path::Path::new(f).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| f.clone());
        let row = SettingsCard::new(folder_glyph(f), &name, f);
        let rm = c::button(t("Verwijderen", "Remove"), Variant::Standard);
        rm.set_widget_name(&format!("sharing-advanced-folder-remove-{i}"));
        let (pp, list, f2) = (p.clone(), folders.clone(), f.clone());
        rm.connect_clicked(move |_| save_folders(&pp, folders_with(&list, None, Some(&f2))));
        row.set_control(&rm);
        fx.add(row.widget());
    }
    group.add(fx.widget());
    // Netwerken (gsd Sharing's list)
    let networks: Vec<Value> = media["networks"].as_array().cloned().unwrap_or_default();
    let nx = SettingsExpander::new("network", t("Netwerken", "Networks"), t("De netwerken waarop mediastreaming is ingeschakeld", "The networks media streaming is turned on for"));
    nx.widget().set_widget_name("sharing-advanced-networks");
    if networks.is_empty() {
        nx.add(SettingsCard::new("", t("Op geen enkel netwerk ingeschakeld", "Not turned on for any network"), "").widget());
    }
    for (i, n) in networks.iter().enumerate() {
        let is_current = n["uuid"] == media["currentNetwork"];
        let glyph = if s(&n["carrier"]) == "802-11-wireless" { "wifi-3" } else { "ethernet" };
        let row = SettingsCard::new(glyph, s(&n["name"]), if is_current { t("Huidig netwerk", "Current network") } else { "" });
        let rm = c::button(t("Verwijderen", "Remove"), Variant::Standard);
        rm.set_widget_name(&format!("sharing-advanced-network-remove-{i}"));
        let (pp, uuid) = (p.clone(), s(&n["uuid"]).to_owned());
        rm.connect_clicked(move |_| {
            let wait = pp.run("sharing.media.networks.remove", json!({ "uuid": uuid }));
            let pp2 = pp.clone();
            pp.spawn(async move {
                if wait.await.is_some() {
                    pp2.refresh();
                }
            });
        });
        row.set_control(&rm);
        nx.add(row.widget());
    }
    group.add(nx.widget());
    out.push(group.widget().clone().upcast());
}

fn sharing_advanced(p: Page) -> PageFuture {
    Box::pin(async move {
        let media = p.get("sharing.media.get", json!({})).await.unwrap_or(Value::Null);
        if !p.alive() {
            return Content::new();
        }
        let mut out: Content = Vec::new();

        // Netwerkdetectie: Avahi is always on (information, where Windows has a switch)
        let discovery = SettingsGroup::new("", "");
        let card = SettingsCard::new(
            "network",
            t("Netwerkdetectie", "Network discovery"),
            t(
                "Je pc kan andere apparaten in het netwerk vinden en door ze worden gevonden. Dit staat in WinPoint altijd aan.",
                "Your PC can find other devices on the network and be found by them. This is always on in WinPoint.",
            ),
        );
        card.set_control(&common::value_label(ct::on()));
        card.widget().set_widget_name("sharing-advanced-discovery");
        discovery.add(card.widget());
        out.push(discovery.widget().clone().upcast());

        // Mediastreaming (GNOME's Media Sharing)
        if media["available"] == true {
            media_group(&p, &media, &mut out);
        } else {
            let group = SettingsGroup::new(t("Mediastreaming", "Media streaming"), "");
            let card = SettingsCard::new("cast", t("Mediastreaming", "Media streaming"), t("Hiervoor is rygel nodig. Installeer het pakket rygel om media te streamen.", "This needs rygel. Install the rygel package to stream media."));
            card.widget().set_widget_name("sharing-advanced-no-rygel");
            group.add(card.widget());
            out.push(group.widget().clone().upcast());
        }
        {
            let pp = p.clone();
            p.on("sharing.changed", move |d| {
                if d["what"] == "media" {
                    pp.refresh();
                }
            });
        }
        let related = SettingsGroup::new(ct::related(), "");
        for id in ["remotedesktop", "system-advanced"] {
            if crate::registry::page(id).is_some_and(common::page_visible) {
                related.add(common::link_card(&p, id).widget());
            }
        }
        out.push(related.widget().clone().upcast());
        out
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_adapters_windows_lists_and_their_state() {
        let devs = json!({ "devices": [
            { "iface": "enp3s0", "type": "ethernet", "state": "unplugged", "autoconnect": true, "managed": true },
            { "iface": "wlan0", "type": "wifi", "state": "connected", "autoconnect": true, "managed": true, "active": { "uuid": "u1", "id": "Thuis" } },
            { "iface": "p2p-dev-wlan0", "type": "wifi-p2p", "state": "disconnected", "managed": true },
            { "iface": "veth1", "type": "veth", "state": "unmanaged", "managed": false },
            { "iface": "docker0", "type": "bridge", "state": "unmanaged", "managed": false },
            { "iface": "br0", "type": "bridge", "state": "connected", "managed": true },
            { "iface": "enp4s0", "type": "ethernet", "state": "disconnected", "autoconnect": false, "managed": true },
        ]});
        let a = adapters(&devs);
        let names: Vec<&str> = a.iter().map(|d| s(&d["iface"])).collect();
        assert_eq!(names, ["enp3s0", "wlan0", "br0", "enp4s0"], "no Wi-Fi Direct, veth or unmanaged bridge");
        assert!(adapter_on(&a[0]) && !can_enable(&a[0]), "a cable unplugged: the adapter stays enabled");
        assert!(adapter_on(&a[1]));
        assert!(!adapter_on(&a[3]) && can_enable(&a[3]), "Disconnect blocked autoconnect: disabled");
        assert_eq!(adapter_title(&a[0], true), "Ethernet (enp3s0)");
        assert_eq!(adapter_title(&a[1], false), "Wi-Fi");
        assert_eq!(adapter_title(&a[2], false), "br0");
        assert_eq!(adapter_glyph("wifi"), "wifi-3");
        let rows = adapter_rows(&a[1]);
        assert_eq!(rows[0].1, "wlan0");
        assert!(rows.iter().any(|(_, v)| v == "Thuis"), "the connection's name");
        let before = digest(&devs);
        let mut changed = devs.clone();
        changed["devices"][6]["autoconnect"] = json!(true);
        assert_ne!(digest(&changed), before, "an adapter turned on rebuilds the page");
        let mut same = devs.clone();
        same["devices"][1]["bitrate"] = json!(866_000);
        assert_eq!(digest(&same), before, "a new bit rate does not");
    }

    #[test]
    fn mediastreaming_texts_and_folders() {
        assert!(status_text(&json!({ "running": true, "status": "available" })).is_none());
        assert!(status_text(&json!({ "running": false })).is_some());
        assert!(status_text(&json!({ "running": true, "status": "offline" })).is_some());
        let f = vec!["/a".to_owned(), "/b".to_owned()];
        assert_eq!(folders_with(&f, Some("/c"), None), ["/a", "/b", "/c"]);
        assert_eq!(folders_with(&f, Some("/a"), None), ["/a", "/b"], "never twice");
        assert_eq!(folders_with(&f, None, Some("/a")), ["/b"]);
    }
}
