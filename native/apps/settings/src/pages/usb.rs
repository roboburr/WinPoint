//! Instellingen › Bluetooth en apparaten › USB (batch B, notes/claude/settings-parity.md §2): Windows 11's USB page
//! with its "USB4-hubs en -apparaten", holding GNOME's Privacy › Thunderbolt (cc-bolt-panel.c) through bolt
//! (devices.thunderbolt.*). Shown only with a Thunderbolt / USB4 domain (`needs: thunderbolt`).
//!
//! * "Directe toegang" is GNOME's Direct Access switch: bolt's AuthMode (enabled / disabled), which asks for the
//!   administrator password (polkit org.freedesktop.bolt.manage).
//! * The devices (GNOME's Devices and Pending Devices) with GNOME's device dialog inside (open by itself for a
//!   new pending device): "Afhankelijk van" (GNOME's Depends on: the unauthorised devices it hangs behind),
//!   "Autoriseren en verbinden" (EnrollDevice for a new device, Authorize for a stored one, those parents first, as
//!   cc-bolt-device-dialog.c connects the whole chain), Status, Fabrikant, UUID and the times, and "Apparaat
//!   vergeten" (ForgetDevice), each with the shield.
//! * Live: devices.changed {what: thunderbolt}.
use super::{Builder, Page, PageFuture};
use crate::common;
use crate::pickers;
use kit::controls::{self as c, SettingsCard, SettingsExpander, SettingsGroup, ToggleSwitch, Variant};
use kit::prelude::*;
use kit::t;
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;

pub const PAGES: &[(&str, Builder)] = &[("usb", page)];
pub const CSS: &str = "";

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}

/// bolt's device status as GNOME's device dialog words it.
fn status_text(d: &Value) -> &'static str {
    match s(&d["status"]) {
        "disconnected" => t("Niet verbonden", "Not connected"),
        "connecting" => t("Verbinden…", "Connecting…"),
        "connected" => t("Verbonden, niet geautoriseerd", "Connected, not authorised"),
        "authorizing" => t("Autoriseren…", "Authorising…"),
        "auth-error" => t("Fout bij autoriseren", "Authorisation error"),
        st if st.starts_with("authorized") => t("Verbonden en geautoriseerd", "Connected and authorised"),
        _ => t("Onbekend", "Unknown"),
    }
}

/// What "Autoriseren en verbinden" does, root first: the device and its parents that are not authorised yet
/// (cc-bolt-device-dialog.c), each `(uid, stored)`: a stored device is authorised, a new one enrolled.
pub(super) fn chain(devices: &[Value], uid: &str) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut next = Some(uid.to_owned());
    while let Some(u) = next.take() {
        if !seen.insert(u.clone()) {
            break;
        }
        let Some(d) = devices.iter().find(|d| s(&d["uid"]) == u) else { break };
        if d["authorized"] != true {
            out.push((u, d["stored"] == true));
        }
        next = Some(s(&d["parent"]).to_owned()).filter(|p| !p.is_empty());
    }
    out.reverse();
    out
}

/// A device's name as GNOME shows it (bolt's name, else "Thunderbolt-apparaat").
fn device_name(d: &Value) -> &str {
    Some(s(&d["name"])).filter(|n| !n.is_empty()).unwrap_or(t("Thunderbolt-apparaat", "Thunderbolt device"))
}

/// GNOME's "Depends on" (cc-bolt-device-dialog.c): the names of the unauthorised devices `uid` hangs behind, root
/// first; "Autoriseren en verbinden" authorises them too (`chain`).
pub(super) fn depends_on(devices: &[Value], uid: &str) -> Vec<String> {
    chain(devices, uid)
        .into_iter()
        .filter(|(u, _)| u != uid)
        .filter_map(|(u, _)| devices.iter().find(|d| s(&d["uid"]) == u))
        .map(|d| device_name(d).to_owned())
        .collect()
}

/// Whether Thunderbolt connects devices without asking (no security in the firmware: none, dponly, usbonly).
pub(super) fn no_security(level: &str) -> bool {
    matches!(level, "none" | "dponly" | "usbonly")
}

struct UsbPage {
    p: Page,
    state: RefCell<Value>,
    busy: RefCell<HashSet<String>>,
    open: RefCell<HashSet<String>>,
    /// The pending devices listed before (only a new one opens by itself).
    seen: RefCell<HashSet<String>>,
    toggle: ToggleSwitch,
    /// The toggle is set by the code (its handler must not write).
    quiet: Cell<bool>,
    body: gtk::Box,
}

impl UsbPage {
    async fn reload(self: &Rc<Self>) {
        let st = self.p.get("devices.thunderbolt.get", json!({})).await;
        if let Some(st) = st.filter(|v| v.is_object())
            && self.p.alive()
        {
            *self.state.borrow_mut() = st;
            self.paint();
        }
    }

    fn set_mode(self: &Rc<Self>, on: bool) {
        let me = self.clone();
        let wait = self.p.run_with(
            "devices.thunderbolt.setAuthMode",
            json!({ "mode": if on { "enabled" } else { "disabled" } }),
            Some(t("Directe toegang wijzigen lukte niet", "Couldn’t change direct access")),
            false,
        );
        self.toggle.set_sensitive(false);
        self.p.spawn(async move {
            let r = wait.await;
            if !me.p.alive() {
                return;
            }
            me.toggle.set_sensitive(true);
            if r.is_none() {
                me.quiet.set(true);
                me.toggle.set_active(s(&me.state.borrow()["authMode"]) == "enabled");
                me.quiet.set(false);
            }
            me.reload().await;
        });
    }

    fn connect(self: &Rc<Self>, uid: String) {
        let devices = self.state.borrow()["devices"].as_array().cloned().unwrap_or_default();
        let steps = chain(&devices, &uid);
        if steps.is_empty() {
            return;
        }
        self.busy.borrow_mut().insert(uid.clone());
        self.paint();
        let me = self.clone();
        self.p.spawn(async move {
            for (u, stored) in steps {
                let (method, args) = if stored {
                    ("devices.thunderbolt.authorize", json!({ "uid": u }))
                } else {
                    ("devices.thunderbolt.enroll", json!({ "uid": u, "policy": "default" }))
                };
                let r = me.p.run_with(method, args, Some(t("Autoriseren lukte niet", "Couldn’t authorise")), false).await;
                if r.is_none() || !me.p.alive() {
                    break;
                }
            }
            me.busy.borrow_mut().remove(&uid);
            me.reload().await;
        });
    }

    fn forget(self: &Rc<Self>, d: &Value) {
        let uid = s(&d["uid"]).to_owned();
        let name = s(&d["name"]).to_owned();
        let me = self.clone();
        self.p.spawn(async move {
            let (fill, wait) = super::later::<bool>();
            c::Dialog::new(t("Dit apparaat vergeten?", "Forget this device?"))
                .text(&common::fill(
                    t(
                        "“{name}” wordt niet meer automatisch geautoriseerd. Sluit je het later weer aan, dan moet je het opnieuw autoriseren.",
                        "“{name}” will no longer be authorised automatically. If you connect it later, you’ll need to authorise it again.",
                    ),
                    &[("name", &name)],
                ))
                .primary(t("Vergeten", "Forget"))
                .close_label(t("Annuleren", "Cancel"))
                .show(&me.p.anchor(), move |r| fill(r == c::DialogResult::Primary));
            if !wait.await || !me.p.alive() {
                return;
            }
            me.busy.borrow_mut().insert(uid.clone());
            me.paint();
            let _ = me.p.run_with("devices.thunderbolt.forget", json!({ "uid": uid }), Some(t("Vergeten lukte niet", "Couldn’t forget the device")), false).await;
            me.busy.borrow_mut().remove(&uid);
            me.reload().await;
        });
    }

    fn detail(title: &str, value: &str) -> SettingsCard {
        let card = SettingsCard::new("", title, "");
        let l = common::value_label(value);
        l.set_selectable(true);
        card.set_control(&l);
        card
    }

    fn device_row(self: &Rc<Self>, d: &Value, devices: &[Value], direct: bool) -> gtk::Widget {
        let uid = s(&d["uid"]).to_owned();
        let busy = self.busy.borrow().contains(&uid);
        let name = device_name(d);
        let vendor = s(&d["vendor"]);
        let status = if busy { t("Even geduld…", "Just a moment…") } else { status_text(d) };
        let desc = if vendor.is_empty() { status.to_owned() } else { format!("{vendor} · {status}") };
        let exp = SettingsExpander::new("usb", name, &desc);
        exp.header_button().set_widget_name(&format!("usb-dev-{uid}"));
        let needs_auth = d["connected"] == true && d["authorized"] != true;
        // the devices "Autoriseren en verbinden" authorises first, named before the button (GNOME's Depends on)
        let parents = depends_on(devices, &uid);
        if !parents.is_empty() {
            let card = Self::detail(t("Afhankelijk van", "Depends on"), &parents.join(", "));
            card.widget().set_widget_name(&format!("usb-dev-{uid}-depends"));
            exp.add(card.widget());
        }
        if needs_auth {
            // in the opened dialog, not on the header: a button inside the header button never gets its click
            let card = SettingsCard::new(
                "",
                t("Nog niet geautoriseerd", "Not authorised yet"),
                if direct {
                    t("Het apparaat werkt pas nadat je het hebt geautoriseerd", "The device works once you’ve authorised it")
                } else {
                    t("Zet eerst Directe toegang aan.", "Turn on direct access first.")
                },
            );
            let b = pickers::admin_button(t("Autoriseren en verbinden", "Authorise and connect"), Variant::Accent);
            b.set_widget_name("usb-authorize");
            b.set_sensitive(direct && !busy);
            let (me, u) = (self.clone(), uid.clone());
            b.connect_clicked(move |_| me.connect(u.clone()));
            card.set_control(&b);
            exp.add(card.widget());
        }
        exp.add(Self::detail(t("Status", "Status"), status_text(d)).widget());
        if !vendor.is_empty() {
            exp.add(Self::detail(t("Fabrikant", "Manufacturer"), vendor).widget());
        }
        exp.add(Self::detail(t("UUID", "UUID"), &uid).widget());
        let time = |key: &str| d[key].as_f64().map(|secs| kit::format::file_time(secs * 1000.0));
        if let Some(at) = time("authorizeTime") {
            exp.add(Self::detail(t("Geautoriseerd op", "Authorised on"), &at).widget());
        } else if let Some(at) = time("connectTime") {
            exp.add(Self::detail(t("Verbonden op", "Connected on"), &at).widget());
        }
        if let Some(at) = time("storeTime") {
            exp.add(Self::detail(t("Onthouden sinds", "Remembered since"), &at).widget());
        }
        if d["stored"] == true {
            let card = SettingsCard::new("", t("Apparaat vergeten", "Forget device"), t("Het apparaat wordt dan niet meer automatisch geautoriseerd", "The device is then no longer authorised automatically"));
            let b = pickers::admin_button(t("Vergeten", "Forget"), Variant::Standard);
            b.set_widget_name("usb-forget");
            b.set_sensitive(!busy);
            let (me, dev) = (self.clone(), d.clone());
            b.connect_clicked(move |_| me.forget(&dev));
            card.set_control(&b);
            exp.add(card.widget());
        }
        // a device that waits for authorisation shows its dialog open the first time it is listed
        if needs_auth && self.seen.borrow_mut().insert(uid.clone()) {
            self.open.borrow_mut().insert(uid.clone());
        }
        if self.open.borrow().contains(&uid) {
            exp.set_expanded(true);
        }
        let me = self.clone();
        exp.connect_toggled(move |open| {
            if open {
                me.open.borrow_mut().insert(uid.clone());
            } else {
                me.open.borrow_mut().remove(&uid);
            }
        });
        exp.widget().clone().upcast()
    }

    fn paint(self: &Rc<Self>) {
        let st = self.state.borrow().clone();
        let direct = s(&st["authMode"]) == "enabled";
        self.quiet.set(true);
        self.toggle.set_active(direct);
        self.quiet.set(false);
        let mut out: Vec<gtk::Widget> = Vec::new();
        if no_security(s(&st["securityLevel"])) {
            let n = common::notice(t(
                "De firmware van deze computer verbindt Thunderbolt-apparaten zonder toestemming te vragen.",
                "This computer’s firmware connects Thunderbolt devices without asking for permission.",
            ));
            n.widget().set_widget_name("usb-no-security");
            out.push(n.widget().clone().upcast());
        }
        let devices = st["devices"].as_array().cloned().unwrap_or_default();
        let group = SettingsGroup::new(t("USB4-hubs en -apparaten", "USB4 hubs and devices"), "");
        group.widget().set_widget_name("usb-devices");
        if devices.is_empty() {
            let text = if st["probing"] == true {
                t("Zoeken naar apparaten…", "Looking for devices…")
            } else {
                t("Er zijn geen USB4- of Thunderbolt-apparaten aangesloten.", "No USB4 or Thunderbolt devices are connected.")
            };
            let card = SettingsCard::new("usb", text, "");
            card.widget().set_widget_name("usb-empty");
            group.add(card.widget());
        }
        // GNOME's Pending Devices first, then the others
        let mut sorted = devices.clone();
        sorted.sort_by_key(|d| d["pending"] != true);
        for d in &sorted {
            group.add(&self.device_row(d, &devices, direct));
        }
        out.push(group.widget().clone().upcast());
        common::set_slot(&self.body, &out);
    }
}

fn page(p: Page) -> PageFuture {
    Box::pin(async move {
        let st = p.get("devices.thunderbolt.get", json!({})).await;
        if !p.alive() {
            return Vec::new();
        }
        let st = st.filter(|v| v.is_object()).unwrap_or_else(|| json!({ "available": false }));
        if st["available"] != true {
            let empty = c::empty_state(
                "usb",
                t("Thunderbolt-beheer is niet beschikbaar", "Thunderbolt management isn’t available"),
                t(
                    "De Thunderbolt-service (bolt) draait niet op deze computer, dus USB4- en Thunderbolt-apparaten kunnen hier niet worden beheerd.",
                    "The Thunderbolt service (bolt) isn’t running on this computer, so USB4 and Thunderbolt devices can’t be managed here.",
                ),
                None,
            );
            empty.set_widget_name("usb-unavailable");
            return vec![empty.upcast()];
        }
        let toggle = ToggleSwitch::new(s(&st["authMode"]) == "enabled");
        toggle.switch().set_widget_name("usb-direct-access");
        toggle.switch().set_tooltip_text(Some(t("Hiervoor is een beheerderswachtwoord nodig.", "This needs an administrator password.")));
        let card = SettingsCard::new("shield", t("Directe toegang", "Direct access"), t("Directe toegang tot apparaten zoals docks en externe GPU’s toestaan", "Allow direct access to devices such as docks and external GPUs"));
        card.set_control(toggle.widget());
        let body = common::slot();
        let page = Rc::new(UsbPage {
            p: p.clone(),
            state: RefCell::new(st),
            busy: RefCell::new(HashSet::new()),
            open: RefCell::new(HashSet::new()),
            seen: RefCell::new(HashSet::new()),
            toggle: toggle.clone(),
            quiet: Cell::new(false),
            body: body.clone(),
        });
        {
            let pg = Rc::downgrade(&page);
            toggle.connect_toggled(move |on| {
                if let Some(pg) = pg.upgrade()
                    && !pg.quiet.get()
                {
                    pg.set_mode(on);
                }
            });
        }
        {
            let pg = page.clone();
            p.on("devices.changed", move |d| {
                if d["what"] == "thunderbolt" {
                    let me = pg.clone();
                    pg.p.spawn(async move { me.reload().await });
                }
            });
        }
        page.paint();
        let main = SettingsGroup::new("", "");
        main.add(card.widget());
        vec![main.widget().clone().upcast(), body.upcast()]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(uid: &str, parent: &str, authorized: bool, stored: bool) -> Value {
        json!({"uid": uid, "parent": parent, "authorized": authorized, "stored": stored})
    }

    #[test]
    fn authorising_connects_the_whole_chain_root_first() {
        let devices = vec![
            dev("dock", "host", false, true),
            dev("gpu", "dock", false, false),
            dev("disk", "gpu", true, true),
        ];
        assert_eq!(chain(&devices, "gpu"), vec![("dock".to_owned(), true), ("gpu".to_owned(), false)]);
        assert_eq!(chain(&devices, "disk"), vec![("dock".to_owned(), true), ("gpu".to_owned(), false)], "an authorised device needs only its parents");
        assert!(chain(&devices, "unknown").is_empty());
        // a loop in the parents ends
        let looped = vec![dev("a", "b", false, false), dev("b", "a", false, false)];
        assert_eq!(chain(&looped, "a").len(), 2);
    }

    #[test]
    fn a_device_names_the_unauthorised_devices_it_depends_on() {
        // review B finding 11: the dock the eGPU hangs behind is named before "Autoriseren en verbinden"
        let mut dock = dev("dock", "host", false, true);
        dock["name"] = json!("Lenovo Dock");
        let devices = vec![dock, dev("gpu", "dock", false, false), dev("disk", "gpu", true, true)];
        assert_eq!(depends_on(&devices, "gpu"), ["Lenovo Dock"]);
        assert_eq!(depends_on(&devices, "disk"), ["Lenovo Dock".to_owned(), t("Thunderbolt-apparaat", "Thunderbolt device").to_owned()]);
        assert!(depends_on(&devices, "dock").is_empty(), "the host is authorised (and not listed)");
    }

    #[test]
    fn firmware_without_security_is_named() {
        assert!(no_security("none") && no_security("dponly") && no_security("usbonly"));
        assert!(!no_security("user") && !no_security("secure") && !no_security(""));
    }
}
