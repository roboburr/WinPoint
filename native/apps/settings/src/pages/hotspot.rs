//! Instellingen › Netwerk en internet › Mobiele hotspot (batch C, notes/claude/settings-parity.md §2): GNOME Settings'
//! Wi-Fi hotspot (Turn On Wi-Fi Hotspot…: Network Name, Password, Generate Random Password; the active view with its
//! QR code, security type and password; Turn Off Hotspot… with its question) in the layout of Windows 11's Mobile
//! hotspot page: the switch, "Mijn internetverbinding delen vanaf" and "Delen via" (what is shared, as information:
//! GNOME shares the primary connection over Wi-Fi), and "Eigenschappen" with Bewerken (name, password, band). The
//! host is `net.hotspot.*` (NetworkManager); the stored key is read only when "Weergeven" asks for it.
use super::net_editor as ed;
use super::{Builder, Content, Page, PageFuture};
use crate::common;
use crate::pickers::QrCode;
use kit::controls::{self as c, Dialog, Dropdown, DropdownOption, SettingsCard, SettingsExpander, SettingsGroup, Severity, ToggleSwitch, Variant};
use kit::prelude::*;
use kit::t;
use std::cell::RefCell;
use std::rc::Rc;

pub const PAGES: &[(&str, Builder)] = &[("hotspot", hotspot)];
pub const CSS: &str = "";

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}

/// The bands of the edit dialog: (NetworkManager's band, nl, en).
pub const BANDS: &[(&str, &str, &str)] = &[("", "Elke beschikbare", "Any available"), ("a", "5 GHz", "5 GHz"), ("bg", "2,4 GHz", "2.4 GHz")];

/// The band's text.
pub fn band_label(band: &str) -> &'static str {
    BANDS.iter().find(|b| b.0 == band).map(|b| if kit::i18n::en() { b.2 } else { b.1 }).unwrap_or(t("Elke beschikbare", "Any available"))
}

/// The bands the adapter offers (Elke beschikbare always; 5 GHz and 2,4 GHz when net.hotspot.get says so).
pub fn band_options(h: &Value) -> Vec<&'static str> {
    BANDS
        .iter()
        .filter(|b| match b.0 {
            "a" => h["bands"]["5"] == true || h["band"] == "a",
            "bg" => h["bands"]["2.4"] == true || h["band"] == "bg",
            _ => true,
        })
        .map(|b| b.0)
        .collect()
}

/// The security type as Windows names it.
pub fn security_label(security: &str) -> &'static str {
    match security {
        "wpa3" => "WPA3-Personal",
        "wpa2" => "WPA2-Personal",
        "wpa" => "WPA-Personal",
        "wep" => "WEP",
        "open" => t("Open", "Open"),
        _ => t("Onbekend", "Unknown"),
    }
}

/// A key the dialog accepts: WPA 8 to 63 characters (or 64 hex digits); WEP 5 or 13 characters, 10 or 26 hex digits.
pub fn password_ok(key_mgmt: &str, pw: &str) -> bool {
    if key_mgmt == "none" { ed::wep_key_ok(pw) } else { ed::psk_ok(pw) }
}

/// net.hotspot.start's arguments from the dialog (an empty password keeps the stored one).
pub fn start_args(ssid: &str, password: &str, band: &str, activate: bool) -> Value {
    let mut a = json!({ "ssid": ssid, "band": band });
    a["password"] = if password.is_empty() { Value::Null } else { json!(password) };
    if !activate {
        a["activate"] = json!(false);
    }
    a
}

/// GNOME's "Willekeurig wachtwoord genereren": 8 letters and digits (10 hex digits for WEP), from GLib's random.
fn random_password(wep: bool) -> String {
    const ALNUM: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    const HEX: &[u8] = b"0123456789abcdef";
    let (set, len) = if wep { (HEX, 10) } else { (ALNUM, 8) };
    (0..len).map(|_| set[glib::random_int_range(0, set.len() as i32) as usize] as char).collect()
}

/// The hotspot switch's flow, GNOME's (net-device-wifi.c): on = the shown properties (`password` None keeps the stored
/// key, or a new hotspot's proposed one), first the question when the adapter's own Wi-Fi connection would drop; off
/// = "Mobiele hotspot uitschakelen?" first. A cancelled question or a failed call puts `sw` back; `after(ok)` runs
/// once the call answered. Used by this page and by the switch on Netwerk en internet (network.rs).
pub(super) fn toggle(p: &Page, h: &Value, password: Option<String>, on: bool, sw: &ToggleSwitch, after: Rc<dyn Fn(bool)>) {
    let run = {
        let (p, sw) = (p.clone(), sw.clone());
        Rc::new(move |method: &'static str, args: Value| {
            let (sw, after, wait) = (sw.clone(), after.clone(), p.run(method, args));
            p.spawn(async move {
                let ok = wait.await.is_some();
                if !ok {
                    sw.set_active(!on);
                }
                after(ok);
            });
        })
    };
    let sw = sw.clone();
    if on {
        let mut args = start_args(s(&h["ssid"]), &password.unwrap_or_default(), s(&h["band"]), true);
        // Keep the adapter whose connection the confirmation warned would be interrupted.
        if h["iface"].is_string() {
            args["iface"] = h["iface"].clone();
        }
        if h["sameAdapter"] != true {
            run("net.hotspot.start", args);
            return;
        }
        let name = s(&h["uplink"]["id"]).to_owned();
        c::confirm(
            &p.anchor(),
            t("Mobiele hotspot inschakelen?", "Turn on mobile hotspot?"),
            &common::fill(
                t(
                    "De verbinding met {name} wordt verbroken: deze Wi-Fi-adapter kan niet tegelijk verbonden zijn en een hotspot delen.",
                    "You’ll be disconnected from {name}: this Wi-Fi adapter can’t stay connected and share a hotspot at the same time.",
                ),
                &[("name", &name)],
            ),
            t("Inschakelen", "Turn on"),
            false,
            move |ok| {
                if ok {
                    run("net.hotspot.start", args.clone());
                } else {
                    sw.set_active(false);
                }
            },
        );
    } else {
        c::confirm(
            &p.anchor(),
            t("Mobiele hotspot uitschakelen?", "Turn off mobile hotspot?"),
            t("Apparaten die de hotspot gebruiken, worden losgekoppeld.", "Devices that use the hotspot will be disconnected."),
            t("Uitschakelen", "Turn off"),
            true,
            move |ok| {
                if ok {
                    run("net.hotspot.stop", json!({}));
                } else {
                    sw.set_active(true);
                }
            },
        );
    }
}

/// The page's state: net.hotspot.get, the key once it was shown.
struct Hotspot {
    p: Page,
    state: RefCell<Value>,
    revealed: RefCell<Option<String>>,
    switch: ToggleSwitch,
    status: SettingsCard,
    from: gtk::Label,
    props: gtk::Box,
    qr: gtk::Box,
    warning: kit::controls::InfoBar,
}

impl Hotspot {
    fn password(&self) -> Option<String> {
        self.revealed.borrow().clone().or_else(|| self.state.borrow()["password"].as_str().map(str::to_owned))
    }

    fn paint(self: &Rc<Self>) {
        let h = self.state.borrow().clone();
        let available = h["available"] == true;
        self.switch.set_active(h["active"] == true);
        self.switch.set_sensitive(available);
        self.warning.set_visible(!available);
        self.status.set_description(match s(&h["state"]) {
            "activating" => t("Wordt ingeschakeld…", "Turning on…"),
            "activated" => t("Aan: andere apparaten kunnen verbinding maken", "On: other devices can connect"),
            _ => t("Je internetverbinding delen met andere apparaten", "Share your internet connection with other devices"),
        });
        let uplink = &h["uplink"];
        self.from.set_label(&if uplink.is_object() {
            let kind = match s(&uplink["type"]) {
                "802-11-wireless" => t("Wi-Fi", "Wi-Fi"),
                "802-3-ethernet" => t("Ethernet", "Ethernet"),
                "gsm" | "cdma" => t("Mobiel netwerk", "Mobile network"),
                "vpn" | "wireguard" => t("VPN", "VPN"),
                _ => "",
            };
            if kind.is_empty() { s(&uplink["id"]).to_owned() } else { format!("{} ({kind})", s(&uplink["id"])) }
        } else {
            t("Geen internetverbinding", "No internet connection").to_owned()
        });
        // Eigenschappen: Windows' name, password and band, GNOME's security type
        let pw = self.password();
        let rows = vec![
            (t("Naam:", "Name:").to_owned(), s(&h["ssid"]).to_owned()),
            (t("Wachtwoord:", "Password:").to_owned(), pw.clone().unwrap_or_else(|| "••••••••".to_owned())),
            (t("Band:", "Band:").to_owned(), band_label(s(&h["band"])).to_owned()),
            (t("Beveiligingstype:", "Security type:").to_owned(), security_label(s(&h["security"])).to_owned()),
        ];
        common::set_slot(&self.props, &[super::network::specs(&rows, true)]);
        match shown_qr(&h, pw.as_deref()).and_then(|q| QrCode::new(&q, 160)) {
            Some(code) => {
                let col = gtk::Box::new(gtk::Orientation::Vertical, 6);
                let frame = gtk::Box::new(gtk::Orientation::Vertical, 0);
                frame.add_css_class("st-net-qr");
                frame.set_widget_name("hotspot-qr");
                frame.set_halign(gtk::Align::Start);
                frame.append(&code);
                col.append(&frame);
                col.append(&c::label(t("Scan de QR-code met je telefoon om verbinding te maken met de hotspot.", "Scan the QR code with your phone to connect to the hotspot."), "st-spec-key", true));
                common::set_slot(&self.qr, &[col.upcast()]);
            }
            None => common::set_slot(&self.qr, &[]),
        }
    }

    /// net.hotspot.get again (no key), then paint.
    fn reload(self: &Rc<Self>) {
        let (me, wait) = (self.clone(), self.p.get("net.hotspot.get", json!({})));
        self.p.spawn(async move {
            if let Some(h) = wait.await
                && h.is_object()
            {
                *me.state.borrow_mut() = h;
                me.paint();
            }
        });
    }
}

/// The QR code's text the page shows: GNOME's active view only, once the hotspot is on and its key known (a phone that
/// scans it while the hotspot is off finds no such network).
pub fn shown_qr(h: &Value, password: Option<&str>) -> Option<String> {
    let pw = password.filter(|_| h["active"] == true)?;
    let mut v = h.clone();
    v["password"] = json!(pw);
    Some(qr_string(&v))
}

/// The WIFI: text of the hotspot's QR code (gnome-control-center's `get_qr_string`).
pub fn qr_string(h: &Value) -> String {
    let esc = |x: &str| x.chars().flat_map(|c| if matches!(c, '\\' | ';' | ',' | ':' | '"') { vec!['\\', c] } else { vec![c] }).collect::<String>();
    let kind = match s(&h["keyMgmt"]) {
        "wpa-psk" => "WPA",
        "sae" => "SAE",
        "none" => "WEP",
        _ => "nopass",
    };
    match h["password"].as_str().filter(|p| !p.is_empty() && kind != "nopass") {
        Some(pw) => format!("WIFI:S:{};T:{kind};P:{};;", esc(s(&h["ssid"])), esc(pw)),
        None => format!("WIFI:S:{};T:nopass;;", esc(s(&h["ssid"]))),
    }
}

fn hotspot(p: Page) -> PageFuture {
    Box::pin(async move {
        let r = p.call("net.hotspot.get", json!({})).await;
        if !p.alive() {
            return Content::new();
        }
        let h = match r {
            Ok(v) if v.is_object() => v,
            Err(e) if e.code == "unknown_method" => return vec![common::notice(common::ct::unavailable_here()).widget().clone().upcast()],
            Err(e) => return vec![common::notice(&e.message).widget().clone().upcast()],
            _ => return vec![common::notice(common::ct::unavailable()).widget().clone().upcast()],
        };
        let warning = common::notice_full(Severity::Warning, "", t("Er is geen Wi-Fi-adapter gevonden op deze computer.", "No Wi-Fi adapter was found on this computer."));
        warning.widget().set_widget_name("hotspot-no-wifi");
        let switch = ToggleSwitch::new(h["active"] == true);
        switch.switch().set_widget_name("hotspot-switch");
        let status = SettingsCard::new("hotspot", t("Mobiele hotspot", "Mobile hotspot"), "");
        status.set_control(switch.widget());
        let from = common::value_label("");
        from.set_widget_name("hotspot-from");
        let props = common::slot();
        let qr = common::slot();
        let me = Rc::new(Hotspot { p: p.clone(), state: RefCell::new(h), revealed: RefCell::new(None), switch: switch.clone(), status: status.clone(), from: from.clone(), props: props.clone(), qr: qr.clone(), warning: warning.clone() });

        let group = SettingsGroup::new("", "");
        group.add(status.widget());
        let from_card = SettingsCard::new("", t("Mijn internetverbinding delen vanaf", "Share my internet connection from"), "");
        from_card.set_control(&from);
        group.add(from_card.widget());
        let over = SettingsCard::new("", t("Delen via", "Share over"), "");
        over.set_control(&common::value_label(t("Wi-Fi", "Wi-Fi")));
        group.add(over.widget());
        let exp = SettingsExpander::new("properties", t("Eigenschappen", "Properties"), "");
        exp.set_expanded(true);
        let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
        body.append(&props);
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let edit = c::button(t("Bewerken", "Edit"), Variant::Standard);
        edit.set_widget_name("hotspot-edit");
        buttons.append(&edit);
        let show = c::button(t("Wachtwoord weergeven", "Show password"), Variant::Standard);
        show.set_widget_name("hotspot-show");
        buttons.append(&show);
        body.append(&buttons);
        body.append(&qr);
        exp.add(&body);
        group.add(exp.widget());
        me.paint();
        show.set_visible(me.password().is_none());

        // on: the shown properties (an unknown stored key is kept); off: GNOME's question first
        {
            let me = me.clone();
            switch.connect_toggled(move |on| {
                let h = me.state.borrow().clone();
                let (me2, used) = (me.clone(), me.password());
                // the key the hotspot was turned on with is its key now: GNOME's active view shows it and the QR code
                // without reading the secrets back
                toggle(
                    &me.p,
                    &h,
                    used.clone(),
                    on,
                    &me.switch,
                    Rc::new(move |ok| {
                        if ok && on && me2.revealed.borrow().is_none() {
                            *me2.revealed.borrow_mut() = used.clone();
                        }
                        me2.reload();
                    }),
                );
            });
        }
        {
            let (me, show2) = (me.clone(), show.clone());
            show.connect_clicked(move |_| {
                let (me, show, wait) = (me.clone(), show2.clone(), me.p.run("net.hotspot.get", json!({ "secrets": true })));
                me.p.clone().spawn(async move {
                    if let Some(h) = wait.await
                        && let Some(pw) = h["password"].as_str()
                    {
                        *me.revealed.borrow_mut() = Some(pw.to_owned());
                        show.set_visible(false);
                        me.paint();
                    }
                });
            });
        }
        {
            let me = me.clone();
            edit.connect_clicked(move |_| edit_dialog(&me));
        }
        {
            let me = me.clone();
            p.on("net.changed", move |d| {
                if matches!(s(&d["what"]), "hotspot" | "active" | "devices" | "connections" | "radios") {
                    me.reload();
                }
            });
        }
        let related = SettingsGroup::new(common::ct::related(), "");
        related.add(common::link_card(&p, "wifi").widget());
        vec![warning.widget().clone().upcast(), group.widget().clone().upcast(), related.widget().clone().upcast()]
    })
}

/// Windows' "Netwerkinformatie bewerken" with GNOME's random password: name, password, band; saved (and, while the
/// hotspot is on, activated again) through net.hotspot.start.
fn edit_dialog(me: &Rc<Hotspot>) {
    let h = me.state.borrow().clone();
    let key_mgmt = s(&h["keyMgmt"]).to_owned();
    let form = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let name = ed::entry(s(&h["ssid"]), "", "hotspot-edit-name");
    form.append(&c::field(t("Netwerknaam", "Network name"), &name, None));
    let known = me.password();
    let pw = ed::entry(known.as_deref().unwrap_or(""), if known.is_none() { t("Ongewijzigd laten", "Leave unchanged") } else { "" }, "hotspot-edit-password");
    let pw_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    pw_row.append(&pw);
    let random = c::icon_button("refresh", t("Willekeurig wachtwoord genereren", "Generate random password"), 32);
    random.set_widget_name("hotspot-edit-random");
    {
        let (pw, wep) = (pw.clone(), key_mgmt == "none");
        random.connect_clicked(move |_| pw.set_text(&random_password(wep)));
    }
    pw_row.append(&random);
    let pw_label = if key_mgmt == "none" { t("Netwerkwachtwoord (WEP-sleutel)", "Network password (WEP key)") } else { t("Netwerkwachtwoord (minimaal 8 tekens)", "Network password (at least 8 characters)") };
    form.append(&c::field(pw_label, &pw_row, None));
    let opts: Vec<DropdownOption> = band_options(&h).into_iter().map(|b| DropdownOption::new(b, band_label(b))).collect();
    let band = Dropdown::new(opts, Some(s(&h["band"])));
    band.widget().set_widget_name("hotspot-edit-band");
    form.append(&c::field(t("Netwerkband", "Network band"), band.widget(), None));
    let err = ed::error_label();
    form.append(&err);
    let active = h["active"] == true;
    let anchor = me.p.anchor();
    let me = me.clone();
    Dialog::new(t("Netwerkinformatie bewerken", "Edit network info"))
        .body(&form)
        .primary(t("Opslaan", "Save"))
        .width(460)
        .on_primary(move |d| {
            let ssid = name.text().trim().to_owned();
            if ssid.is_empty() || ssid.len() > 32 {
                ed::show_error(&err, t("De netwerknaam moet uit 1 tot 32 tekens bestaan.", "The network name must be 1 to 32 characters."));
                return false;
            }
            let password = pw.text().to_string();
            if !(password.is_empty() && known.is_none()) && !password_ok(&key_mgmt, &password) {
                ed::show_error(&err, t("Het netwerkwachtwoord is niet geldig. Gebruik 8 tot 63 tekens.", "The network password isn’t valid. Use 8 to 63 characters."));
                return false;
            }
            let args = start_args(&ssid, &password, &band.selected().unwrap_or_default(), active);
            let me = me.clone();
            ed::dialog_call(d, &err, "net.hotspot.start", args, move |v| {
                common::note_dry_run(&me.p.anchor(), &v);
                if !password.is_empty() && v["dryRun"] != true {
                    *me.revealed.borrow_mut() = Some(password);
                }
                me.reload();
            });
            false
        })
        .show(&anchor, |_| {});
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dialog_sends_what_net_hotspot_start_takes() {
        assert_eq!(start_args("WinPoint", "abcd1234", "bg", true), json!({ "ssid": "WinPoint", "band": "bg", "password": "abcd1234" }));
        assert_eq!(start_args("WinPoint", "", "", false), json!({ "ssid": "WinPoint", "band": "", "password": null, "activate": false }));
        assert!(password_ok("wpa-psk", "abcd1234") && !password_ok("wpa-psk", "kort"));
        assert!(password_ok("none", "0123456789") && !password_ok("none", "abcd1234"));
        let k = random_password(false);
        assert!(k.len() == 8 && password_ok("wpa-psk", &k));
        assert!(password_ok("none", &random_password(true)));
    }

    #[test]
    fn bands_security_and_the_qr_text() {
        let h = json!({ "bands": { "2.4": true, "5": false }, "band": "" });
        assert_eq!(band_options(&h), vec!["", "bg"]);
        let h = json!({ "bands": { "2.4": true, "5": false }, "band": "a" });
        assert_eq!(band_options(&h), vec!["", "a", "bg"], "the stored band stays offered");
        assert_eq!(security_label("wpa2"), "WPA2-Personal");
        let qr = qr_string(&json!({ "ssid": "Win;Point", "keyMgmt": "wpa-psk", "password": "a:b" }));
        assert_eq!(qr, "WIFI:S:Win\\;Point;T:WPA;P:a\\:b;;");
        assert_eq!(qr_string(&json!({ "ssid": "x", "keyMgmt": "", "password": "y" })), "WIFI:S:x;T:nopass;;");
        // GNOME's active view only: no code for a hotspot that is off (a new one shows its proposed key without it)
        let off = json!({ "ssid": "WinPoint", "keyMgmt": "wpa-psk", "active": false, "configured": false, "password": "irsoNBEG" });
        assert_eq!(shown_qr(&off, Some("irsoNBEG")), None);
        let mut on = off.clone();
        on["active"] = json!(true);
        assert_eq!(shown_qr(&on, Some("irsoNBEG")).as_deref(), Some("WIFI:S:WinPoint;T:WPA;P:irsoNBEG;;"));
        assert_eq!(shown_qr(&on, None), None, "the stored key not read yet");
    }
}
