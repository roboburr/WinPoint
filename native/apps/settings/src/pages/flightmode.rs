//! Instellingen › Netwerk en internet › Vliegtuigmodus (batch C, notes/claude/settings-parity.md §2): GNOME Settings'
//! Airplane Mode (gsd's Rfkill AirplaneMode through `system.network.setAirplane`, amendment 57) and its "Hardware
//! Airplane Mode is On", with Windows 11's "Draadloze apparaten": the Wi-Fi switch (NetworkManager's
//! WirelessEnabled), Bluetooth (`system.bluetooth.setPowered`, batch B's) and Mobiel netwerk (`net.radios.set
//! {wwan}`, only with a modem). Follows the system and net events.
use super::{Builder, Content, Page, PageFuture};
use crate::common;
use kit::controls::{SettingsCard, SettingsGroup, Severity, ToggleSwitch};
use kit::prelude::*;
use kit::t;
use std::rc::Rc;

pub const PAGES: &[(&str, Builder)] = &[("flightmode", flightmode)];
pub const CSS: &str = "";

/// What the switches show: (flight mode, hardware switch, Wi-Fi there, Wi-Fi on, Bluetooth there, Bluetooth on).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Radios {
    pub airplane: bool,
    pub hardware: bool,
    pub wifi_available: bool,
    pub wifi: bool,
    pub bluetooth_available: bool,
    pub bluetooth: bool,
}

/// The switches' state from `system.network.get` and `system.bluetooth.get`.
pub fn radios(net: &Value, bt: &Value) -> Radios {
    Radios {
        airplane: net["airplane"] == true,
        hardware: net["airplaneHardware"] == true,
        wifi_available: net["wifiAvailable"] == true,
        wifi: net["wifiEnabled"] == true,
        bluetooth_available: bt["available"] == true,
        bluetooth: bt["powered"] == true,
    }
}

/// The Mobiel netwerk switch's state from net.devices (NetworkManager's WwanEnabled).
pub fn wwan_enabled(devices: &Value) -> bool {
    devices["wwanEnabled"] == true
}

/// The net.changed kinds after which the Mobiel netwerk switch reads WwanEnabled again: WinPoint's own net.radios.set
/// (radios), NetworkManager's root object (state: WwanEnabled, also after gsd's Rfkill applied flight mode) and
/// ModemManager (modem).
pub fn wwan_follows(what: &Value) -> bool {
    matches!(what.as_str(), Some("radios" | "state" | "modem"))
}

/// A card with a switch that calls `method {arg: on}`; reverted when it fails.
fn radio_card(p: &Page, icon: &str, title: &str, description: &str, on: bool, name: &str, method: &'static str, arg: &'static str) -> (SettingsCard, ToggleSwitch) {
    let sw = ToggleSwitch::new(on);
    sw.switch().set_widget_name(name);
    let (pp, sw2) = (p.clone(), sw.clone());
    sw.connect_toggled(move |on| {
        let wait = pp.run_with(method, json!({ arg: on }), None, true);
        let sw = sw2.clone();
        pp.spawn(async move {
            if wait.await.is_none() {
                sw.set_active(!on);
            }
        });
    });
    let card = SettingsCard::new(icon, title, description);
    card.set_control(sw.widget());
    (card, sw)
}

fn flightmode(p: Page) -> PageFuture {
    Box::pin(async move {
        let (nr, br, dr) = (p.call("system.network.get", json!({})), p.get("system.bluetooth.get", json!({})), p.get("net.devices", json!({})));
        let (nr, bt, devs) = (nr.await, br.await, dr.await);
        if !p.alive() {
            return Content::new();
        }
        let net = match nr {
            Ok(v) if v.is_object() => v,
            _ => return vec![common::notice(common::ct::unavailable()).widget().clone().upcast()],
        };
        let bt = bt.unwrap_or(Value::Null);
        let r = radios(&net, &bt);
        let mut out: Content = Vec::new();

        let hardware = common::notice_full(
            Severity::Warning,
            t("Vliegtuigmodus via hardwareschakelaar staat aan", "Hardware flight mode is on"),
            t(
                "Zet de schakelaar voor draadloze verbindingen op je pc uit om draadloze apparaten weer te gebruiken.",
                "Turn off the wireless switch on your PC to use wireless devices again.",
            ),
        );
        hardware.widget().set_widget_name("flightmode-hardware");
        hardware.set_visible(r.hardware);
        out.push(hardware.widget().clone().upcast());

        let main = SettingsGroup::new("", "");
        let (air, air_sw) = radio_card(
            &p,
            "airplane",
            t("Vliegtuigmodus", "Flight mode"),
            t("Alle draadloze communicatie stoppen", "Stop all wireless communication"),
            r.airplane,
            "flightmode-airplane",
            "system.network.setAirplane",
            "enabled",
        );
        air_sw.set_sensitive(!r.hardware);
        main.add(air.widget());
        out.push(main.widget().clone().upcast());

        let devices = SettingsGroup::new(t("Draadloze apparaten", "Wireless devices"), "");
        let (wifi, wifi_sw) = radio_card(&p, "wifi-3", t("Wi-Fi", "Wi-Fi"), "", r.wifi, "flightmode-wifi", "system.network.setWifi", "enabled");
        wifi.widget().set_visible(r.wifi_available);
        devices.add(wifi.widget());
        let (blue, blue_sw) = radio_card(&p, "bluetooth", t("Bluetooth", "Bluetooth"), "", r.bluetooth, "flightmode-bluetooth", "system.bluetooth.setPowered", "powered");
        blue.widget().set_visible(r.bluetooth_available);
        devices.add(blue.widget());
        // mobile broadband: only with a modem (GNOME's WWAN switch)
        let wwan_on = devs.as_ref().is_some_and(wwan_enabled);
        let (mobile, mobile_sw) = radio_card(&p, "phone", t("Mobiel netwerk", "Mobile network"), "", wwan_on, "flightmode-mobile", "net.radios.set", "wwan");
        mobile.widget().set_visible(common::caps_ok(Some("modem")) && devs.is_some());
        devices.add(mobile.widget());
        let any = r.wifi_available || r.bluetooth_available || mobile.widget().is_visible();
        devices.widget().set_visible(any);
        out.push(devices.widget().clone().upcast());

        let paint = {
            let (hardware, air_sw, wifi, wifi_sw, blue, blue_sw) = (hardware.clone(), air_sw.clone(), wifi.clone(), wifi_sw.clone(), blue.clone(), blue_sw.clone());
            Rc::new(move |r: Radios| {
                hardware.set_visible(r.hardware);
                air_sw.set_active(r.airplane);
                air_sw.set_sensitive(!r.hardware);
                wifi.widget().set_visible(r.wifi_available);
                wifi_sw.set_active(r.wifi);
                blue.widget().set_visible(r.bluetooth_available);
                blue_sw.set_active(r.bluetooth);
            })
        };
        // the Mobiel netwerk switch reads NetworkManager's WwanEnabled again (debounced): after its own write (radios),
        // NetworkManager's own change (state: also after this page's flight mode switch, which gsd's Rfkill applies),
        // ModemManager's (modem) and every system.network.changed
        let wwan_again: Rc<dyn Fn()> = {
            let (pp, mobile_sw, pending) = (p.clone(), mobile_sw.clone(), Rc::new(std::cell::Cell::new(false)));
            Rc::new(move || {
                if pending.replace(true) {
                    return;
                }
                let (pp2, sw, pending) = (pp.clone(), mobile_sw.clone(), pending.clone());
                pp.timeout(500, move || {
                    pending.set(false);
                    let wait = pp2.get("net.devices", json!({}));
                    pp2.spawn(async move {
                        if let Some(d) = wait.await {
                            sw.set_active(wwan_enabled(&d));
                        }
                    });
                });
            })
        };
        let last = Rc::new(std::cell::RefCell::new((net, bt)));
        {
            let (last, paint, wwan_again) = (last.clone(), paint.clone(), wwan_again.clone());
            p.on("system.network.changed", move |d| {
                if d.is_object() {
                    last.borrow_mut().0 = d.clone();
                    let (n, b) = &*last.borrow();
                    paint(radios(n, b));
                    wwan_again();
                }
            });
        }
        {
            let (last, paint) = (last.clone(), paint.clone());
            p.on("system.bluetooth.changed", move |d| {
                if d.is_object() {
                    last.borrow_mut().1 = d.clone();
                    let (n, b) = &*last.borrow();
                    paint(radios(n, b));
                }
            });
        }
        p.on("net.changed", move |d| {
            if wwan_follows(&d["what"]) {
                wwan_again();
            }
        });
        let related = SettingsGroup::new(common::ct::related(), "");
        related.add(common::link_card(&p, "wifi").widget());
        related.add(common::link_card(&p, "bluetooth").widget());
        out.push(related.widget().clone().upcast());
        out
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_switches_follow_gnomes_state() {
        let net = json!({ "airplane": true, "airplaneHardware": false, "wifiAvailable": true, "wifiEnabled": false });
        let bt = json!({ "available": true, "powered": false });
        assert_eq!(radios(&net, &bt), Radios { airplane: true, hardware: false, wifi_available: true, wifi: false, bluetooth_available: true, bluetooth: false });
        let r = radios(&json!({ "airplaneHardware": true }), &Value::Null);
        assert!(r.hardware && !r.airplane && !r.bluetooth_available, "no Bluetooth answer: its card is hidden");
    }

    #[test]
    fn the_mobile_switch_follows_networkmanager_and_modemmanager() {
        for what in ["radios", "state", "modem"] {
            assert!(wwan_follows(&json!(what)), "{what}");
        }
        for what in ["connections", "accessPoints", "hotspot"] {
            assert!(!wwan_follows(&json!(what)), "{what}");
        }
        assert!(wwan_enabled(&json!({ "wwanEnabled": true })));
        assert!(!wwan_enabled(&json!({ "wwanEnabled": false })) && !wwan_enabled(&json!({})));
    }
}
