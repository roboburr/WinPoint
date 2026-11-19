//! Instellingen › Bluetooth en apparaten › Apparaten (batch B, notes/claude/settings-parity.md §2): Windows 11's
//! Devices page with GNOME's Bluetooth device dialog inside each device and GNOME's Power › Connected Devices.
//!
//! * "Een apparaat toevoegen" opens bluetooth.rs' Windows dialog.
//! * The devices in Windows' groups "Muis, toetsenbord en pen", "Audio" and "Overige apparaten": each a card with
//!   its status and battery and Windows' "⋯" menu: Verbinding maken / Verbinding verbreken, Eigenschappen and
//!   Apparaat verwijderen (system.bluetooth.connect / disconnect / remove, with Windows' question). Eigenschappen is
//!   GNOME's device dialog: Gekoppeld, Type, Adres, the battery, the settings of its kind (Muis, Toetsenbord, Geluid:
//!   GNOME's shortcuts) and "Bestanden verzenden" for a phone or a computer (devices.bluetooth.sendFiles: GNOME's
//!   bluetooth-sendto). (A button inside an expander's header button never gets its click: the header's capture
//!   gesture takes it, so the actions are in the menu, not in an expander.)
//! * The batteries of devices.batteries that belong to no Bluetooth device of the list (a wireless mouse with its
//!   own receiver, a UPS) are cards of their own in the same groups; a Bluetooth device without a BlueZ battery
//!   shows UPower's.
//! * Live: system.bluetooth.changed and devices.changed {what: batteries}.
use super::bluetooth as bt;
use super::{Builder, Page, PageFuture};
use crate::common;
use kit::controls::{self as c, MenuItem, SettingsCard, SettingsGroup, Severity, Variant};
use kit::prelude::*;
use kit::t;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

pub const PAGES: &[(&str, Builder)] = &[("bluetooth-devices", page)];
pub const CSS: &str = r#"
.st-btd-key { color: var(--text-2); }
"#;

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}

/// Windows' group of a device kind (BlueZ's bt_kind or UPower's): 0 "Muis, toetsenbord en pen", 1 "Audio",
/// 2 "Overige apparaten".
pub(super) fn group_of(kind: &str) -> usize {
    match kind {
        "mouse" | "keyboard" | "pen" | "touchpad" | "tablet" => 0,
        "headphones" | "headset" | "speaker" | "speakers" | "media" | "media-player" | "other-audio" => 1,
        _ => 2,
    }
}

/// GNOME's settings shortcut of the device dialog (cc-bluetooth-panel.c): the page for a kind.
pub(super) fn settings_page(kind: &str) -> Option<&'static str> {
    match group_of(kind) {
        0 if kind == "keyboard" => Some("keyboard-advanced"),
        0 if kind == "pen" || kind == "tablet" => None,
        0 => Some("mouse"),
        1 => Some("sound"),
        _ => None,
    }
}

fn settings_label(page: &str) -> &'static str {
    match page {
        "keyboard-advanced" => t("Toetsenbordinstellingen", "Keyboard settings"),
        "sound" => t("Geluidsinstellingen", "Sound settings"),
        _ => t("Muisinstellingen", "Mouse settings"),
    }
}

/// Bluetooth devices that take files (GNOME's Send Files, obexd's Object Push): phones and computers.
pub(super) fn takes_files(kind: &str) -> bool {
    matches!(kind, "phone" | "computer")
}

/// A UPower kind as words.
fn upower_kind(kind: &str) -> &'static str {
    match kind {
        "mouse" => t("Muis", "Mouse"),
        "keyboard" => t("Toetsenbord", "Keyboard"),
        "pen" => t("Pen", "Pen"),
        "touchpad" => t("Touchpad", "Touchpad"),
        "tablet" => t("Tablet", "Tablet"),
        "headset" => t("Headset", "Headset"),
        "headphones" => t("Koptelefoon", "Headphones"),
        "speakers" => t("Luidspreker", "Speaker"),
        "gaming-input" => t("Controller", "Controller"),
        "phone" => t("Telefoon", "Phone"),
        "computer" => t("Computer", "Computer"),
        "ups" => t("Noodstroomvoeding (UPS)", "Uninterruptible power supply (UPS)"),
        _ => t("Apparaat", "Device"),
    }
}

fn upower_glyph(kind: &str) -> &'static str {
    match kind {
        "mouse" => "mouse",
        "keyboard" => "keyboard",
        "pen" | "tablet" => "pen",
        "touchpad" => "touchpad",
        "headset" | "headphones" => "headphones",
        "speakers" | "other-audio" | "media-player" => "sound",
        "phone" => "phone",
        "gaming-input" => "gamepad",
        "computer" => "pc",
        "ups" => "battery",
        _ => "devices",
    }
}

/// The battery of a UPower device as words: "Batterij 80%" (with "wordt opgeladen" while charging), or its coarse
/// level for a device that only reports one.
fn battery_text(b: &Value) -> String {
    let base = match (b["percentage"].as_i64(), s(&b["level"])) {
        (_, "low") => t("Batterij bijna leeg", "Battery low").to_owned(),
        (_, "critical") => t("Batterij bijna op", "Battery critically low").to_owned(),
        (Some(n), _) => t("Batterij {n}%", "Battery {n}%").replace("{n}", &n.to_string()),
        (None, "full") => t("Batterij vol", "Battery full").to_owned(),
        (None, "high") => t("Batterij hoog", "Battery high").to_owned(),
        (None, "normal") => t("Batterij goed", "Battery good").to_owned(),
        _ => String::new(),
    };
    match s(&b["state"]) {
        "charging" => format!("{base} · {}", t("wordt opgeladen", "charging")),
        "full" if b["percentage"].as_i64().is_some() => format!("{base} · {}", t("volledig opgeladen", "fully charged")),
        _ => base,
    }
}

struct DevPage {
    p: Page,
    state: RefCell<Value>,
    batteries: RefCell<Vec<Value>>,
    busy: RefCell<HashMap<String, &'static str>>,
    body: gtk::Box,
}

impl DevPage {
    async fn reload(self: &Rc<Self>) {
        let (st, bats) = (self.p.get("system.bluetooth.get", json!({})), self.p.get("devices.batteries", json!({})));
        let (st, bats) = (st.await, bats.await);
        if !self.p.alive() {
            return;
        }
        if let Some(st) = st.filter(|v| v.is_object()) {
            *self.state.borrow_mut() = st;
        }
        *self.batteries.borrow_mut() = bt::battery_list(bats);
        self.paint();
    }

    fn act(self: &Rc<Self>, d: Value) {
        let addr = bt::upper_addr(&d);
        let what = if d["connected"] == true { "disconnect" } else { "connect" };
        self.busy.borrow_mut().insert(addr.clone(), what);
        self.paint();
        let me = self.clone();
        self.p.spawn(async move {
            let r = bt::toggle_device(&me.p, &d).await;
            me.busy.borrow_mut().remove(&addr);
            if !me.p.alive() {
                return;
            }
            if let Err(e) = r {
                me.p.error(&e, None);
            }
            me.reload().await;
        });
    }

    fn remove(self: &Rc<Self>, d: Value) {
        let addr = bt::upper_addr(&d);
        if self.busy.borrow().contains_key(&addr) {
            return;
        }
        let me = self.clone();
        self.p.spawn(async move {
            let (m2, a2) = (me.clone(), addr.clone());
            let r = bt::remove_device(&me.p, &d, move || {
                m2.busy.borrow_mut().insert(a2, "remove");
                if m2.p.alive() {
                    m2.paint();
                }
            })
            .await;
            let Some(r) = r else { return };
            me.busy.borrow_mut().remove(&addr);
            if !me.p.alive() {
                return;
            }
            if let Err(e) = r {
                me.p.error(&e, None);
            }
            me.reload().await;
        });
    }

    fn send_files(&self, d: &Value) {
        let wait = self.p.run_with(
            "devices.bluetooth.sendFiles",
            json!({ "address": bt::upper_addr(d), "name": bt::dev_name(d) }),
            Some(t("Bestanden verzenden lukte niet", "Couldn’t send files")),
            false,
        );
        self.p.spawn(async move {
            let _ = wait.await;
        });
    }

    /// GNOME's device dialog (cc-bluetooth-panel.c): Gekoppeld, Type, Adres, the battery, the settings shortcut of
    /// its kind and "Bestanden verzenden"; Windows' "⋯" › Eigenschappen opens it.
    fn properties(self: &Rc<Self>, d: &Value) {
        let addr = bt::upper_addr(d);
        let kind = s(&d["kind"]).to_owned();
        let body = gtk::Box::new(gtk::Orientation::Vertical, 10);
        body.set_widget_name("btd-properties");
        let line = |title: &str, value: &str| {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            let k = c::label(title, "st-btd-key", false);
            k.set_size_request(120, -1);
            row.append(&k);
            let v = c::label(value, "", false);
            v.set_selectable(true);
            v.set_hexpand(true);
            row.append(&v);
            row
        };
        body.append(&line(t("Gekoppeld", "Paired"), if d["paired"] == true { t("Ja", "Yes") } else { t("Nee", "No") }));
        let kind_text = bt::kind_label(&kind);
        if !kind_text.is_empty() {
            body.append(&line(t("Type", "Type"), kind_text));
        }
        body.append(&line(t("Adres", "Address"), &addr));
        if let Some(n) = d["battery"].as_i64() {
            body.append(&line(t("Batterij", "Battery"), &format!("{n}%")));
        }
        let dialog = c::Dialog::new(&bt::dev_name(d)).body(&body).close_label(t("Sluiten", "Close"));
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        actions.set_margin_top(6);
        if let Some(page) = settings_page(&kind) {
            let b = c::button(settings_label(page), Variant::Standard);
            b.set_widget_name(&format!("btd-settings-{page}"));
            let (pp, dlg) = (self.p.clone(), dialog.clone());
            b.connect_clicked(move |_| {
                dlg.close(c::DialogResult::Close);
                pp.go(page);
            });
            actions.append(&b);
        }
        if takes_files(&kind) {
            let b = c::button(t("Bestanden verzenden", "Send files"), Variant::Standard);
            b.set_widget_name("btd-send");
            b.set_tooltip_text(Some(t("Bestanden verzenden via Bluetooth", "Send files via Bluetooth")));
            let (me, dev) = (self.clone(), d.clone());
            b.connect_clicked(move |_| me.send_files(&dev));
            actions.append(&b);
        }
        if actions.first_child().is_some() {
            body.append(&actions);
        }
        dialog.show(&self.p.anchor(), |_| {});
    }

    /// One Bluetooth device: Windows' card with its status and battery and the "⋯" menu (Verbinding maken /
    /// Verbinding verbreken, Eigenschappen: GNOME's device dialog, Apparaat verwijderen).
    fn bt_row(self: &Rc<Self>, d: &Value) -> gtk::Widget {
        let addr = bt::upper_addr(d);
        let busy = self.busy.borrow().get(&addr).copied();
        let status = match busy {
            Some("connect") => t("Verbinden…", "Connecting…").to_owned(),
            Some("disconnect") => t("Verbinding verbreken…", "Disconnecting…").to_owned(),
            Some(_) => t("Verwijderen…", "Removing…").to_owned(),
            None => bt::device_status(d),
        };
        let card = SettingsCard::new("", &bt::dev_name(d), &status);
        card.set_icon_widget(&bt::device_icon(d, 20));
        card.widget().set_widget_name(&format!("btd-dev-{addr}"));
        let more = c::icon_button("more", t("Meer opties", "More options"), 32);
        more.set_widget_name(&format!("btd-more-{addr}"));
        more.set_sensitive(busy.is_none());
        {
            let (me, dev) = (self.clone(), d.clone());
            more.connect_clicked(move |b| {
                let connected = dev["connected"] == true;
                let (m1, d1, m2, d2, m3, d3) = (me.clone(), dev.clone(), me.clone(), dev.clone(), me.clone(), dev.clone());
                c::show_menu_for(
                    b,
                    vec![
                        MenuItem::new(if connected { t("Verbinding verbreken", "Disconnect") } else { t("Verbinding maken", "Connect") }, move || m1.act(d1.clone()))
                            .glyph(if connected { "close" } else { "link" })
                            .name("btd-toggle"),
                        MenuItem::new(t("Eigenschappen", "Properties"), move || m3.properties(&d3)).glyph("properties").name("btd-properties-item"),
                        MenuItem::separator(),
                        MenuItem::new(t("Apparaat verwijderen", "Remove device"), move || m2.remove(d2.clone())).glyph("trash").name("btd-remove"),
                    ],
                );
            });
        }
        card.set_control(&more);
        card.widget().clone()
    }

    /// A device of UPower's Connected Devices that is no Bluetooth device of the list: the battery in the status
    /// line only, as Windows and the Bluetooth cards show it (the harness saw "Batterij 55%" and "55%" side by side).
    fn battery_row(b: &Value) -> gtk::Widget {
        let kind = s(&b["kind"]);
        let model = s(&b["model"]).trim();
        let title = if model.is_empty() { upower_kind(kind) } else { model };
        let card = SettingsCard::new(upower_glyph(kind), title, &battery_text(b));
        card.widget().set_widget_name(&format!("btd-battery-{}", s(&b["id"])));
        card.widget().clone()
    }

    fn paint(self: &Rc<Self>) {
        let st = self.state.borrow().clone();
        let batteries = self.batteries.borrow().clone();
        let available = st["available"] == true;
        let powered = st["powered"] == true;
        let mut out: Vec<gtk::Widget> = Vec::new();

        if !available {
            let n = common::notice(t("Er is geen Bluetooth gevonden op deze computer.", "No Bluetooth was found on this computer."));
            n.widget().set_widget_name("btd-no-bluetooth");
            out.push(n.widget().clone().upcast());
        } else {
            if !powered {
                let n = common::notice_full(
                    Severity::Info,
                    t("Bluetooth staat uit", "Bluetooth is off"),
                    t("Zet Bluetooth aan om je apparaten te verbinden.", "Turn on Bluetooth to connect your devices."),
                );
                n.set_action_below(true);
                let pp = self.p.clone();
                n.set_action(Some((t("Bluetooth en apparaten", "Bluetooth & devices"), Rc::new(move || pp.go("bluetooth")))));
                n.widget().set_widget_name("btd-off");
                out.push(n.widget().clone().upcast());
            }
            let add = SettingsCard::new("plus", t("Een apparaat toevoegen", "Add a device"), "");
            let b = c::button(t("Apparaat toevoegen", "Add device"), Variant::Standard);
            b.set_widget_name("btd-add");
            let me = self.clone();
            b.connect_clicked(move |_| {
                let m2 = me.clone();
                bt::open_add_device_then(
                    &me.p,
                    Some(Box::new(move || {
                        let m3 = m2.clone();
                        m2.p.spawn(async move { m3.reload().await });
                    })),
                );
            });
            add.set_control(&b);
            let g = SettingsGroup::new("", "");
            g.add(add.widget());
            out.push(g.widget().clone().upcast());
        }

        let mut groups: [Vec<gtk::Widget>; 3] = [Vec::new(), Vec::new(), Vec::new()];
        let listed: Vec<Value> = if available && powered { st["devices"].as_array().cloned().unwrap_or_default() } else { Vec::new() };
        let mut devs: Vec<Value> = listed.iter().map(|d| bt::with_battery(d, &batteries)).collect();
        devs.sort_by(|a, b| (b["connected"] == true).cmp(&(a["connected"] == true)).then_with(|| kit::format::compare_natural(&bt::dev_name(a), &bt::dev_name(b))));
        let bt_addresses: HashSet<String> = devs.iter().map(bt::upper_addr).collect();
        for d in &devs {
            groups[group_of(s(&d["kind"]))].push(self.bt_row(d));
        }
        for b in &batteries {
            if b["address"].as_str().is_some_and(|a| bt_addresses.contains(&a.to_uppercase())) {
                continue;
            }
            groups[group_of(s(&b["kind"]))].push(Self::battery_row(b));
        }
        let titles = [t("Muis, toetsenbord en pen", "Mouse, keyboard & pen"), t("Audio", "Audio"), t("Overige apparaten", "Other devices")];
        let mut any = false;
        for (i, rows) in groups.iter().enumerate() {
            if rows.is_empty() {
                continue;
            }
            any = true;
            let g = SettingsGroup::new(titles[i], "");
            g.widget().set_widget_name(&format!("btd-group-{i}"));
            for r in rows {
                g.add(r);
            }
            out.push(g.widget().clone().upcast());
        }
        if !any && (!available || powered) {
            let n = common::notice_full(
                Severity::Info,
                t("Nog geen apparaten", "No devices yet"),
                if available {
                    t(
                        "Kies “Apparaat toevoegen” om een koptelefoon, muis, toetsenbord of ander Bluetooth-apparaat te koppelen.",
                        "Choose “Add device” to pair headphones, a mouse, a keyboard or another Bluetooth device.",
                    )
                } else {
                    t("Apparaten met een batterij verschijnen hier zodra ze zijn aangesloten.", "Devices with a battery appear here once they’re connected.")
                },
            );
            n.widget().set_widget_name("btd-empty");
            out.push(n.widget().clone().upcast());
        }
        common::set_slot(&self.body, &out);
    }
}

fn page(p: Page) -> PageFuture {
    Box::pin(async move {
        let (st, bats) = (p.get("system.bluetooth.get", json!({})), p.get("devices.batteries", json!({})));
        let (st, bats) = (st.await, bats.await);
        if !p.alive() {
            return Vec::new();
        }
        let body = common::slot();
        let page = Rc::new(DevPage {
            p: p.clone(),
            state: RefCell::new(st.filter(|v| v.is_object()).unwrap_or_else(|| json!({ "available": false }))),
            batteries: RefCell::new(bt::battery_list(bats)),
            busy: RefCell::new(HashMap::new()),
            body: body.clone(),
        });
        {
            let pg = page.clone();
            p.on("system.bluetooth.changed", move |d| {
                if !d.is_object() || d["available"].is_null() {
                    return;
                }
                *pg.state.borrow_mut() = d.clone();
                pg.paint();
            });
        }
        {
            let pg = page.clone();
            p.on("devices.changed", move |d| {
                if d["what"] == "batteries" {
                    let me = pg.clone();
                    pg.p.spawn(async move { me.reload().await });
                }
            });
        }
        page.paint();
        let related = SettingsGroup::new(common::ct::related(), "");
        related.add(common::link_card_with(&p, "sound", None, Some(t("Kies je koptelefoon of speaker als geluidsapparaat", "Choose your headphones or speaker as the sound device")), None).widget());
        related.add(common::link_card(&p, "printers").widget());
        vec![body.upcast(), related.widget().clone().upcast()]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn devices_go_in_windows_groups() {
        assert_eq!(group_of("mouse"), 0);
        assert_eq!(group_of("keyboard"), 0);
        assert_eq!(group_of("pen"), 0);
        assert_eq!(group_of("headset"), 1);
        assert_eq!(group_of("speakers"), 1, "UPower's kind");
        assert_eq!(group_of("speaker"), 1, "BlueZ's kind");
        assert_eq!(group_of("phone"), 2);
        assert_eq!(group_of("ups"), 2);
        assert_eq!(group_of("gamepad"), 2);
    }

    #[test]
    fn the_device_dialog_links_the_settings_of_its_kind() {
        assert_eq!(settings_page("mouse"), Some("mouse"));
        assert_eq!(settings_page("touchpad"), Some("mouse"));
        assert_eq!(settings_page("keyboard"), Some("keyboard-advanced"));
        assert_eq!(settings_page("headphones"), Some("sound"));
        assert_eq!(settings_page("pen"), None);
        assert_eq!(settings_page("phone"), None);
        assert!(takes_files("phone") && takes_files("computer") && !takes_files("mouse"));
    }
}
