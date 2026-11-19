//! Instellingen › Netwerk en internet (6ebf169f:vendor/ui/js/apps/settings/pages/network.js) with Wi-Fi (the
//! networks, connecting with a password or to an open or hidden network, disconnecting, the adapter's hardware) and
//! Ethernet (each adapter on or off, its profile's 802.1X, metered, IP and DNS settings, its profiles, the Bluetooth
//! network), batch C of the settings parity (notes/claude/settings-parity.md §2): GNOME Settings' Network and Wi-Fi
//! panels in Windows 11's layout. The profile edits are `net.*` (NetworkManager through the host); the Wi-Fi list and
//! connect flow stay `system.network.*`.
use super::net_editor as ed;
use super::{Builder, Content, Page, PageFuture};
use crate::common::{self, ct};
use crate::widgets::Wrap;
use kit::controls::{
    self as c, Chevron, Dialog, DialogResult, Dropdown, DropdownOption, ProgressRing, SettingsCard, SettingsExpander, SettingsGroup, Severity, Toast,
    ToggleSwitch, Variant,
};
use kit::prelude::*;
use kit::t;
use std::cell::{Cell, RefCell};
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;

pub const PAGES: &[(&str, Builder)] = &[("network", network), ("wifi", wifi), ("ethernet", ethernet)];
pub const CSS: &str = r#"
.st-net-hero {
  padding: 20px;
  border: 1px solid var(--stroke-card);
  border-radius: var(--radius-card);
  background-color: var(--bg-card);
}
.st-net-hero-icon { min-width: 64px; min-height: 64px; border-radius: 999px; background-color: var(--accent-subtle); color: var(--accent-fg-3); }
.st-net-hero.st-offline .st-net-hero-icon { background-color: var(--fill-control-alt-3); color: var(--text-2); }
.st-net-hero-name {
  font-family: "Segoe UI Variable Display", "Segoe UI Variable", "Segoe UI", "Selawik", "Inter Display", "Inter", "Noto Sans", "Ubuntu Sans", sans-serif;
  font-size: var(--fs-subtitle); font-weight: 600;
}
.st-net-hero-status { color: var(--text-2); }
.st-wifi { min-height: 40px; padding: 8px 0; }
.st-wifi.st-sep { border-top: 1px solid var(--stroke-divider); }
.st-wifi-icon { color: var(--text-1); }
.st-wifi.st-active .st-wifi-icon { color: var(--accent-fg-3); }
.st-wifi-lock { margin: 0 -4px -3px 0; min-width: 13px; min-height: 13px; border-radius: 999px; background-color: var(--bg-solid-3); color: var(--text-1); }
.st-wifi.st-active .st-wifi-name { font-weight: 600; }
.st-wifi-status { color: var(--text-2); font-size: var(--fs-caption); }
.st-wifi-scan { min-height: 48px; color: var(--text-2); }
.st-pw-error { color: var(--sys-critical); font-size: var(--fs-caption); }
"#;

/// A two-column key / value list (`.st-specs`); `selectable` values can be copied with the mouse.
pub fn specs(rows: &[(String, String)], selectable: bool) -> gtk::Widget {
    let grid = gtk::Grid::new();
    grid.add_css_class("st-specs");
    grid.set_column_spacing(24);
    grid.set_row_spacing(10);
    for (r, (k, v)) in rows.iter().filter(|(_, v)| !v.is_empty()).enumerate() {
        let key = c::label(k, "st-spec-key", true);
        key.set_size_request(130, -1);
        key.set_max_width_chars(28);
        key.set_valign(gtk::Align::Start);
        let val = c::label(v, "st-spec-value", true);
        val.set_selectable(selectable);
        val.set_hexpand(true);
        grid.attach(&key, 0, r as i32, 1, 1);
        grid.attach(&val, 1, r as i32, 1, 1);
    }
    let wrap = gtk::Box::new(gtk::Orientation::Vertical, 0);
    wrap.append(&grid);
    wrap.upcast()
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}

/// The hero's status line (network.js statusText).
fn status_text(net: &Value) -> &'static str {
    let ty = s(&net["primary"]["type"]);
    if ty.is_empty() || ty == "none" {
        return t("Niet verbonden", "Not connected");
    }
    match s(&net["connectivity"]) {
        "limited" | "none" => t("Verbonden, geen internet", "Connected, no internet"),
        "portal" => t("Aanmelden vereist", "Sign-in required"),
        _ if ty == "wifi" => t("Verbonden, beveiligd", "Connected, secured"),
        _ => t("Verbonden", "Connected"),
    }
}

fn wifi_glyph(strength: Option<f64>, fallback: f64) -> &'static str {
    kit::icons::wifi_icon(Some(strength.unwrap_or(fallback)))
}

/// The hero's glyph (network.js netGlyph).
fn net_glyph(net: &Value) -> &'static str {
    if net["airplane"] == true {
        return "airplane";
    }
    match s(&net["primary"]["type"]) {
        "wifi" => wifi_glyph(net["primary"]["strength"].as_f64(), 70.0),
        "ethernet" => "ethernet",
        _ => "globe",
    }
}

/// "Verbonden met …", or the Testmodus notice when the host only pretended (--dry-run).
fn connected_toast(p: &Page, ssid: &str, dry_run: bool) {
    if dry_run {
        p.toast(Toast::new(ct::dry_run()).severity(Severity::Info).timeout(3500));
    } else {
        common::toast_success(&p.anchor(), &common::fill(t("Verbonden met {ssid}", "Connected to {ssid}"), &[("ssid", ssid)]), 2500);
    }
}

/// The host's answer of system.network.connect as (ok, dryRun, code, message).
fn connect_outcome(r: &Result<Value, kit::CallError>) -> (bool, bool, String, String) {
    match r {
        Ok(v) if v["ok"] != false => (true, v["dryRun"] == true, String::new(), String::new()),
        Ok(v) => (false, false, s(&v["code"]).to_owned(), s(&v["error"]).to_owned()),
        Err(e) => (false, false, e.code.clone(), e.message.clone()),
    }
}

/// system.network.connect's arguments: the SSID, the password, and the adapter when the page chose one.
fn connect_args(n: &Value, password: Option<&str>) -> Value {
    let mut a = json!({ "ssid": n["ssid"] });
    if let Some(pw) = password {
        a["password"] = json!(pw);
    }
    if n["iface"].is_string() {
        a["iface"] = n["iface"].clone();
    }
    a
}

/// The security form's kind for a network that needs more than a key: WPA/WPA2-Enterprise (net.devices' `wpa-eap`,
/// nmcli's `802.1X`) or WEP. None for the open, personal (WPA/WPA3) and OWE networks of the key prompt.
pub fn editor_kind(n: &Value) -> Option<&'static str> {
    let sec = s(&n["security"]);
    if matches!(sec, "wpa-eap" | "enterprise" | "dynamic-wep") || sec.contains("802.1X") {
        Some("wpa-eap")
    } else if sec == "wep" || sec.split_whitespace().any(|w| w == "WEP") {
        Some("wep")
    } else {
        None
    }
}

/// GNOME's Wi-Fi dialog for an access point that needs the 802.1X or WEP fields (NMAWifiDialog): the network's name
/// fixed, the security form preset to its kind; Verbinden adds the profile and connects (net.wifi.hidden with
/// `hidden: false`). True when connected.
fn connect_with_fields(p: &Page, n: &Value, kind: &'static str) -> Pin<Box<dyn Future<Output = bool>>> {
    let p = p.clone();
    let n = n.clone();
    Box::pin(async move {
        let ssid = s(&n["ssid"]).to_owned();
        let (fill, wait) = super::later::<bool>();
        let fill = Rc::new(RefCell::new(Some(fill)));
        let dry = Rc::new(Cell::new(false));
        let done: Rc<dyn Fn(&Dialog, &gtk::Label, ed::NewNetwork)> = {
            let (n, dry) = (n.clone(), dry.clone());
            Rc::new(move |d: &Dialog, err: &gtk::Label, (ssid, kind, groups, auto, _): ed::NewNetwork| {
                let mut secrets = json!({});
                for (g, v) in groups.as_object().into_iter().flatten() {
                    if !v.is_null() {
                        secrets[g.as_str()] = v.clone();
                    }
                }
                let mut args = json!({ "ssid": ssid, "security": kind, "secrets": secrets, "autoconnect": auto, "hidden": false });
                if n["iface"].is_string() {
                    args["iface"] = n["iface"].clone();
                }
                let dry = dry.clone();
                ed::dialog_call(d, err, "net.wifi.hidden", args, move |v| dry.set(v["dryRun"] == true));
            })
        };
        let title = common::fill(t("Verbinden met “{ssid}”", "Connect to “{ssid}”"), &[("ssid", &ssid)]);
        ed::new_network_with(&p, &title, t("Verbinden", "Connect"), (&ssid, kind), false, done, move |r| {
            if let Some(f) = fill.borrow_mut().take() {
                f(r == DialogResult::Primary);
            }
        });
        let ok = wait.await;
        if ok {
            connected_toast(&p, &ssid, dry.get());
        }
        ok
    })
}

/// The password / open-network flow; true when connected (network.js connectWifi). `error` pre-fills the
/// password box (the router's password changed, so the saved one no longer works). A new enterprise or WEP network
/// gets GNOME's dialog with its fields; a saved one connects like any other saved network.
pub fn connect_wifi(p: &Page, n: Value, error: Option<String>) -> Pin<Box<dyn Future<Output = bool>>> {
    let p = p.clone();
    Box::pin(async move {
        let ssid = s(&n["ssid"]).to_owned();
        let secure = n["secure"] == true;
        let saved = n["saved"] == true;
        let fields = editor_kind(&n);
        if let Some(kind) = fields.filter(|_| !saved) {
            return connect_with_fields(&p, &n, kind).await;
        }
        if secure && fields.is_none() && (!saved || error.is_some()) {
            let entry = c::text_box("");
            entry.set_visibility(false);
            entry.set_input_purpose(gtk::InputPurpose::Password);
            entry.set_hexpand(true);
            entry.set_widget_name("wifi-password");
            let err = c::label(error.as_deref().unwrap_or(""), "st-pw-error", true);
            err.set_visible(error.is_some());
            let col = gtk::Box::new(gtk::Orientation::Vertical, 4);
            col.append(&entry);
            col.append(&err);
            let body = c::field(
                t("Netwerkbeveiligingssleutel", "Network security key"),
                &col,
                Some(t("Het wachtwoord van je wifi staat vaak op een sticker op je router.", "Your Wi-Fi password is often on a sticker on your router.")),
            );
            let dry = Rc::new(Cell::new(false));
            let (fill, wait) = super::later::<bool>();
            let fill = Rc::new(RefCell::new(Some(fill)));
            let (entry2, err2, n2, dry2) = (entry.clone(), err.clone(), n.clone(), dry.clone());
            Dialog::new(&common::fill(t("Verbinden met “{ssid}”", "Connect to “{ssid}”"), &[("ssid", &ssid)]))
                .body(&body)
                .primary(t("Verbinden", "Connect"))
                .width(420)
                .on_primary(move |d| {
                    let pw = entry2.text().to_string();
                    let show = |m: &str| {
                        err2.set_label(m);
                        err2.set_visible(!m.is_empty());
                    };
                    if pw.is_empty() {
                        show(t("Typ het wachtwoord.", "Type the password."));
                        entry2.grab_focus();
                        return false;
                    }
                    d.set_busy(true);
                    let wait = super::call("system.network.connect", connect_args(&n2, Some(&pw)));
                    let (d, err3, entry3, dry3) = (d.clone(), err2.clone(), entry2.clone(), dry2.clone());
                    glib::spawn_future_local(async move {
                        let r = wait.await;
                        let (ok, dry_run, _, msg) = connect_outcome(&r);
                        if ok {
                            dry3.set(dry_run);
                            d.close(DialogResult::Primary);
                            return;
                        }
                        d.set_busy(false);
                        err3.set_label(if msg.is_empty() { t("Verbinden lukte niet", "Couldn’t connect") } else { &msg });
                        err3.set_visible(true);
                        entry3.grab_focus();
                    });
                    false
                })
                .show(&p.anchor(), move |r| {
                    if let Some(f) = fill.borrow_mut().take() {
                        f(r == DialogResult::Primary);
                    }
                });
            let ok = wait.await;
            if ok {
                connected_toast(&p, &ssid, dry.get());
            }
            return ok;
        }
        if !secure && !saved {
            let (fill, wait) = super::later::<bool>();
            c::confirm(
                &p.anchor(),
                t("Verbinden met een open netwerk?", "Connect to an open network?"),
                &common::fill(
                    t(
                        "“{ssid}” is niet beveiligd. Anderen op dit netwerk kunnen mogelijk zien wat je doet. Gebruik het liever niet voor bankzaken.",
                        "“{ssid}” isn’t secured. Other people on this network might see what you do. It’s best not to use it for banking.",
                    ),
                    &[("ssid", &ssid)],
                ),
                t("Verbinden", "Connect"),
                false,
                fill,
            );
            if !wait.await {
                return false;
            }
        }
        let r = p.call("system.network.connect", connect_args(&n, None)).await;
        let (ok, dry_run, code, msg) = connect_outcome(&r);
        if ok {
            connected_toast(&p, &ssid, dry_run);
            return true;
        }
        // a saved network whose password no longer works (changed on the router): ask for the new one, like Windows
        // (a key prompt only for a network with a key; an enterprise or WEP profile is changed on its own page)
        if secure && fields.is_none() && (code == "needs_password" || code == "wrong_password") {
            let mut again = n.clone();
            again["saved"] = json!(false);
            return connect_wifi(&p, again, Some(t("Het opgeslagen wachtwoord werkt niet meer. Typ het nieuwe wachtwoord.", "The saved password no longer works. Type the new password.").into())).await;
        }
        p.toast(Toast::new(t("Verbinden lukte niet", "Couldn’t connect")).message(&msg).severity(Severity::Error).timeout(5000));
        false
    })
}

/// The Ethernet interfaces of sysmon.performance (network.js ethernetIfaces); the call starts now.
fn ethernet_ifaces(p: &Page) -> impl Future<Output = Vec<Value>> + 'static {
    let wait = p.get("sysmon.performance", json!({}));
    async move {
        let perf = wait.await;
        perf.and_then(|v| v["network"].as_array().cloned()).unwrap_or_default().into_iter().filter(|n| n["kind"] == "ethernet").collect()
    }
}

fn not_available() -> Content {
    vec![common::notice(t("Netwerkinformatie is nu niet beschikbaar. Probeer het later opnieuw.", "Network information isn’t available right now. Try again later.")).widget().clone().upcast()]
}

/// GNOME's "Network Unavailable": NetworkManager does not run (also network_advanced.rs).
pub(super) fn nm_missing() -> gtk::Widget {
    let bar = common::notice_full(
        Severity::Warning,
        t("Netwerk niet beschikbaar", "Network unavailable"),
        t("NetworkManager wordt niet uitgevoerd. Netwerkinstellingen kunnen pas worden gewijzigd als het weer wordt uitgevoerd.", "NetworkManager isn’t running. Network settings can only be changed once it runs again."),
    );
    bar.widget().set_widget_name("network-unavailable");
    bar.widget().clone().upcast()
}

/// The devices of net.devices of one type (no unmanaged ones).
fn devices_of(devs: &Value, kind: &str) -> Vec<Value> {
    devs["devices"].as_array().map(|a| a.iter().filter(|d| d["type"] == kind && d["managed"] != false).cloned().collect()).unwrap_or_default()
}

/// A link card when its page is offered (caps, data pages), with an own description.
fn link(p: &Page, id: &str, description: Option<&str>) -> Option<SettingsCard> {
    let info = crate::registry::page(id)?;
    common::page_visible(info).then(|| common::link_card_with(p, id, None, description, None))
}

fn network(p: Page) -> PageFuture {
    Box::pin(async move {
        let (nr, dr, hr) = (p.call("system.network.get", json!({})), p.get("net.devices", json!({})), p.get("net.hotspot.get", json!({})));
        let (nr, devs, hotspot) = (nr.await, dr.await, hr.await);
        let eths = match &devs {
            Some(d) => devices_of(d, "ethernet"),
            None => ethernet_ifaces(&p).await,
        };
        if !p.alive() {
            return Content::new();
        }
        let net = match nr {
            Ok(v) if v.is_object() => v,
            _ => return not_available(),
        };
        let state = Rc::new(RefCell::new(net));
        let mut out: Content = Vec::new();
        if state.borrow()["available"] == false {
            out.push(nm_missing());
        }
        // the active Wi-Fi profile, for the hero's Eigenschappen
        let active_wifi: Option<String> =
            devs.as_ref().map(|d| devices_of(d, "wifi")).and_then(|w| w.iter().find_map(|d| d["active"]["uuid"].as_str().map(str::to_owned)));

        // ---- the hero (`.st-net-hero`)
        let hero = Wrap::new(20, 16);
        hero.add_css_class("st-net-hero");
        let icon_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        icon_box.add_css_class("st-net-hero-icon");
        icon_box.set_size_request(64, 64);
        icon_box.set_hexpand(false);
        icon_box.set_valign(gtk::Align::Center);
        let icon = kit::icons::glyph("globe", 36);
        icon.set_hexpand(true);
        icon.set_halign(gtk::Align::Center);
        icon_box.append(&icon);
        hero.append(&icon_box);
        let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
        text.set_hexpand(true);
        text.set_size_request(180, -1);
        text.set_valign(gtk::Align::Center);
        let name_l = c::label("", "st-net-hero-name", false);
        let status_l = c::label("", "st-net-hero-status", true);
        text.append(&name_l);
        text.append(&status_l);
        hero.append(&text);
        let tiles = Wrap::new(4, 4);
        tiles.set_valign(gtk::Align::Center);
        let props = {
            let (pp, st) = (p.clone(), state.clone());
            common::hero_tile(
                "properties",
                t("Eigenschappen", "Properties"),
                &[t("IP-adres en details", "IP address and details").into()],
                Some(Rc::new(move || match (s(&st.borrow()["primary"]["type"]), &active_wifi) {
                    ("wifi", Some(uuid)) => pp.go_with("wifi-network", json!({ "uuid": uuid })),
                    ("ethernet", _) => pp.go("ethernet"),
                    _ => pp.go("wifi"),
                })),
            )
        };
        props.set_widget_name("network-properties");
        tiles.append(&props);
        let usage = {
            let pp = p.clone();
            common::hero_tile(
                "chart",
                t("Netwerkgebruik", "Network usage"),
                &[t("Live in Taakbeheer", "Live in Task Manager").into()],
                Some(Rc::new(move || {
                    let wait = pp.run_with("shell.openApp", json!({ "app": "taskmgr", "params": { "page": "performance" } }), None, true);
                    pp.spawn(async move {
                        let _ = wait.await;
                    });
                })),
            )
        };
        usage.set_widget_name("network-usage");
        tiles.append(&usage);
        hero.append(&tiles);

        // ---- Wi-Fi, Ethernet, VPN, Mobiele hotspot, Vliegtuigmodus, Proxy, Mobiel netwerk, Geavanceerd (Windows' order)
        let st0 = state.borrow().clone();
        let wifi_sw = ToggleSwitch::new(st0["wifiEnabled"] == true);
        wifi_sw.switch().set_widget_name("network-wifi-toggle");
        {
            let (pp, sw) = (p.clone(), wifi_sw.clone());
            wifi_sw.connect_toggled(move |on| {
                let wait = pp.run_with("system.network.setWifi", json!({ "enabled": on }), None, true);
                let sw = sw.clone();
                pp.spawn(async move {
                    if wait.await.is_none() {
                        sw.set_active(!on);
                    }
                });
            });
        }
        let wifi_card = SettingsCard::clickable("wifi", t("Wi-Fi", "Wi-Fi"), "", Chevron::Right);
        if let Some(b) = wifi_card.button() {
            b.set_widget_name("network-wifi");
        }
        wifi_card.set_control(wifi_sw.widget());
        {
            let pp = p.clone();
            wifi_card.connect_clicked(move || pp.go("wifi"));
        }
        let air_sw = ToggleSwitch::new(st0["airplane"] == true);
        air_sw.switch().set_widget_name("network-airplane");
        {
            let (pp, sw) = (p.clone(), air_sw.clone());
            air_sw.connect_toggled(move |on| {
                let wait = pp.run_with("system.network.setAirplane", json!({ "enabled": on }), None, true);
                let sw = sw.clone();
                pp.spawn(async move {
                    if wait.await.is_none() {
                        sw.set_active(!on);
                    }
                });
            });
        }

        let paint = {
            let (state, icon, name_l, status_l, hero, wifi_sw, wifi_card, air_sw) =
                (state.clone(), icon.clone(), name_l.clone(), status_l.clone(), hero.clone(), wifi_sw.clone(), wifi_card.clone(), air_sw.clone());
            move || {
                let st = state.borrow().clone();
                kit::icons::set_glyph(&icon, net_glyph(&st));
                let ty = s(&st["primary"]["type"]);
                let connected = !ty.is_empty() && ty != "none";
                let pname = s(&st["primary"]["name"]);
                name_l.set_label(if connected {
                    if !pname.is_empty() {
                        pname
                    } else if ty == "wifi" {
                        t("Wi-Fi", "Wi-Fi")
                    } else {
                        t("Ethernet", "Ethernet")
                    }
                } else {
                    t("Niet verbonden", "Not connected")
                });
                status_l.set_label(if st["airplane"] == true { t("Vliegtuigmodus", "Flight mode") } else { status_text(&st) });
                if connected {
                    hero.remove_css_class("st-offline");
                } else {
                    hero.add_css_class("st-offline");
                }
                wifi_sw.set_active(st["wifiEnabled"] == true);
                wifi_sw.set_sensitive(st["wifiAvailable"] == true && st["airplane"] != true);
                let desc = if st["wifiAvailable"] != true {
                    t("Er is geen Wi-Fi-adapter gevonden op deze computer.", "No Wi-Fi adapter was found on this computer.").to_owned()
                } else if st["wifiEnabled"] == true {
                    if ty == "wifi" { format!("{}: {pname}", t("Verbonden", "Connected")) } else { t("Verbinden met draadloze netwerken", "Connect to wireless networks").to_owned() }
                } else {
                    t("Uit", "Off").to_owned()
                };
                wifi_card.set_description(&desc);
                air_sw.set_active(st["airplane"] == true);
            }
        };
        paint();
        {
            let (state, paint) = (state.clone(), paint.clone());
            p.on("system.network.changed", move |d| {
                if d.is_object() {
                    *state.borrow_mut() = d.clone();
                    paint();
                }
            });
        }

        let group = SettingsGroup::new("", "");
        if st0["wifiAvailable"] == true {
            group.add(wifi_card.widget());
        }
        let eth_active = s(&st0["primary"]["type"]) == "ethernet";
        if !eths.is_empty() || eth_active {
            let card = SettingsCard::clickable(
                "ethernet",
                t("Ethernet", "Ethernet"),
                if eth_active { t("Verbonden", "Connected") } else { t("Verificatie, IP- en DNS-instellingen, verbinding met een datalimiet", "Authentication, IP and DNS settings, metered network") },
                Chevron::Right,
            );
            if let Some(b) = card.button() {
                b.set_widget_name("network-ethernet");
            }
            let pp = p.clone();
            card.connect_clicked(move || pp.go("ethernet"));
            group.add(card.widget());
        }
        if let Some(card) = link(&p, "mobile", None) {
            group.add(card.widget());
        }
        if let Some(card) = link(&p, "vpn", Some(t("Toevoegen, verbinden, beheren", "Add, connect, manage"))) {
            group.add(card.widget());
        }
        if let Some(card) = link(&p, "hotspot", None) {
            // the switch once the hotspot service answers and there is a Wi-Fi adapter: the stored name, key (null: kept)
            // and band, or for a new hotspot the computer's name and the key net.hotspot.get proposed; the hotspot
            // page's flow (its questions first, hotspot::toggle), and it follows NetworkManager (GNOME Shell's quick
            // settings, the hotspot page)
            if let Some(h) = hotspot.filter(|h| h.is_object() && h["available"] == true) {
                let sw = ToggleSwitch::new(h["active"] == true);
                sw.switch().set_widget_name("network-hotspot");
                let state = Rc::new(RefCell::new(h));
                let reread: Rc<dyn Fn(bool)> = {
                    let (pp, state, sw) = (p.clone(), state.clone(), sw.clone());
                    Rc::new(move |_| {
                        let (wait, state, sw) = (pp.get("net.hotspot.get", json!({})), state.clone(), sw.clone());
                        pp.spawn(async move {
                            if let Some(h) = wait.await.filter(|h| h.is_object()) {
                                sw.set_active(h["active"] == true);
                                *state.borrow_mut() = h;
                            }
                        });
                    })
                };
                {
                    let (pp, state, sw2, reread) = (p.clone(), state.clone(), sw.clone(), reread.clone());
                    sw.connect_toggled(move |on| {
                        let h = state.borrow().clone();
                        super::hotspot::toggle(&pp, &h, h["password"].as_str().map(str::to_owned), on, &sw2, reread.clone());
                    });
                }
                let pending = Rc::new(Cell::new(false));
                let pp = p.clone();
                p.on("net.changed", move |d| {
                    if pending.get() || !matches!(s(&d["what"]), "hotspot" | "active" | "connections") {
                        return;
                    }
                    pending.set(true);
                    let (pending, reread) = (pending.clone(), reread.clone());
                    pp.timeout(600, move || {
                        pending.set(false);
                        reread(true);
                    });
                });
                card.set_control(sw.widget());
            }
            group.add(card.widget());
        }
        if let Some(card) = link(&p, "flightmode", None) {
            card.set_control(air_sw.widget());
            if let Some(b) = card.button() {
                b.set_widget_name("network-flightmode");
            }
            group.add(card.widget());
        }
        if let Some(card) = link(&p, "proxy", Some(t("Proxyserver voor Wi-Fi- en Ethernet-verbindingen", "Proxy server for Wi-Fi and Ethernet connections"))) {
            group.add(card.widget());
        }
        if let Some(card) = link(&p, "network-advanced", None) {
            group.add(card.widget());
        }
        out.push(hero.upcast());
        out.push(group.widget().clone().upcast());
        out
    })
}

// ------------------------------------------------------------------------------------------------- Wi-Fi

struct WifiPage {
    p: Page,
    state: RefCell<Value>,
    networks: RefCell<Vec<Value>>,
    /// what the list shows (`list_shape`): it is painted again only when that changed
    shape: RefCell<Vec<(String, bool, bool, bool, &'static str)>>,
    /// net.connections of type Wi-Fi: SSID → uuid for the Eigenschappen buttons
    known: RefCell<Vec<Value>>,
    /// the connected profile the page was built with (its "Eigenschappen van {ssid}" card)
    built_active: Option<String>,
    /// the chosen Wi-Fi adapter when there are several (net.devices)
    iface: RefCell<Option<String>>,
    busy: Cell<bool>,
    list: gtk::Box,
    refresh: gtk::Button,
}

/// What a row of the list shows: (SSID, connected, saved, secured, the strength's glyph). The access points' signal
/// changes all the time; the list is painted again only when one of these changed (a click on a row that is being
/// rebuilt would be lost).
fn list_shape(networks: &[Value]) -> Vec<(String, bool, bool, bool, &'static str)> {
    let mut out: Vec<_> = networks
        .iter()
        .map(|n| (s(&n["ssid"]).to_owned(), n["active"] == true, n["saved"] == true, n["secure"] == true, wifi_glyph(n["strength"].as_f64(), 50.0)))
        .collect();
    out.sort();
    out
}

/// The connected Wi-Fi profile among net.connections (not a hotspot).
fn active_profile(known: &[Value]) -> Option<&Value> {
    known.iter().find(|k| k["active"] == true && k["mode"] != "ap")
}

/// The "Eigenschappen van {ssid}" card's line: secured unless the profile has no key management (open) or OWE.
fn active_status(k: &Value) -> &'static str {
    if matches!(s(&k["keyMgmt"]), "" | "owe") { t("Verbonden", "Connected") } else { t("Verbonden, beveiligd", "Connected, secured") }
}

/// The networks of one adapter from net.devices' access points, in system.network.wifiList's shape.
fn networks_of(dev: &Value, known: &[Value]) -> Vec<Value> {
    let mut best: Vec<Value> = Vec::new();
    for ap in dev["accessPoints"].as_array().into_iter().flatten() {
        let ssid = s(&ap["ssid"]);
        if ssid.is_empty() {
            continue;
        }
        let item = json!({
            "ssid": ssid, "strength": ap["strength"], "secure": ap["security"] != "open" && ap["security"] != "owe",
            "active": ap["active"], "saved": known.iter().any(|k| k["ssid"] == ssid), "security": ap["security"], "iface": dev["iface"],
        });
        match best.iter_mut().find(|b| b["ssid"] == ssid) {
            Some(b) if (item["active"] == true && b["active"] != true) || item["strength"].as_u64() > b["strength"].as_u64() && b["active"] != true => *b = item,
            Some(_) => {}
            None => best.push(item),
        }
    }
    best
}

impl WifiPage {
    fn scanning_row(&self) {
        c::clear(&self.list);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        row.add_css_class("st-wifi-scan");
        let ring = ProgressRing::new(16, None);
        ring.widget().set_valign(gtk::Align::Center);
        row.append(ring.widget());
        row.append(&c::label(t("Zoeken naar netwerken…", "Looking for networks…"), "", false));
        self.list.append(&row);
    }

    fn uuid_of(&self, ssid: &str) -> Option<String> {
        self.known.borrow().iter().find(|k| k["ssid"] == ssid && k["mode"] != "ap").and_then(|k| k["uuid"].as_str().map(str::to_owned))
    }

    fn load(self: &Rc<Self>, rescan: bool) {
        if self.busy.get() {
            return;
        }
        self.busy.set(true);
        self.refresh.set_sensitive(false);
        let scanning = rescan || self.networks.borrow().is_empty();
        if scanning {
            self.scanning_row();
        }
        let me = self.clone();
        let known = self.p.get("net.connections", json!({ "type": "802-11-wireless" }));
        // NetworkManager's access points over D-Bus, as GNOME's Wi-Fi panel has them (RequestScan, then the list a
        // moment later): the chosen adapter's when there are several, else the first one's. nmcli's list only when
        // net.devices has no Wi-Fi adapter (the settings harness found the page empty on its nmcli stub while
        // NetworkManager listed Lab-5G).
        let iface = self.iface.borrow().clone();
        let first = self.p.get("net.devices", json!({ "accessPoints": true, "scan": rescan }));
        self.p.spawn(async move {
            let (known, mut devs) = (known.await, first.await);
            if rescan && devs.is_some() && me.p.alive() {
                super::sleep(4000).await;
                devs = me.p.get("net.devices", json!({ "accessPoints": true })).await.or(devs);
            }
            let known: Vec<Value> = known.and_then(|v| v["connections"].as_array().cloned()).unwrap_or_default();
            let dev = devs
                .as_ref()
                .map(|d| devices_of(d, "wifi"))
                .and_then(|w| w.into_iter().find(|d| iface.as_deref().is_none_or(|i| d["iface"] == i)));
            let networks = match dev {
                Some(dev) => networks_of(&dev, &known),
                None if me.p.alive() => me.p.get("system.network.wifiList", json!({ "rescan": rescan })).await.and_then(|v| v.as_array().cloned()).unwrap_or_default(),
                None => Vec::new(),
            };
            me.busy.set(false);
            me.refresh.set_sensitive(true);
            if !me.p.alive() {
                return;
            }
            // another network connected (here, the taskbar's flyout, GNOME Shell): the page again, so the
            // "Eigenschappen van {ssid}" card and the list's Verbinding verbreken name the network that is on now
            if active_profile(&known).and_then(|k| k["uuid"].as_str()) != me.built_active.as_deref() {
                me.p.refresh();
                return;
            }
            let shape = list_shape(&networks);
            let changed = scanning || *me.shape.borrow() != shape;
            *me.known.borrow_mut() = known;
            *me.networks.borrow_mut() = networks;
            *me.shape.borrow_mut() = shape;
            if changed {
                me.paint_list();
            }
        });
    }

    fn paint_list(self: &Rc<Self>) {
        c::clear(&self.list);
        if self.state.borrow()["wifiEnabled"] != true {
            return;
        }
        let mut sorted = self.networks.borrow().clone();
        let key = |n: &Value| (n["active"] == true, n["saved"] == true, n["strength"].as_f64().unwrap_or(0.0));
        sorted.sort_by(|a, b| {
            let (ka, kb) = (key(a), key(b));
            kb.0.cmp(&ka.0).then(kb.1.cmp(&ka.1)).then(kb.2.partial_cmp(&ka.2).unwrap_or(std::cmp::Ordering::Equal))
        });
        if sorted.is_empty() {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            row.add_css_class("st-wifi-scan");
            row.append(&c::label(t("Geen netwerken gevonden", "No networks found"), "", false));
            self.list.append(&row);
        }
        let count = sorted.len();
        for (i, n) in sorted.into_iter().enumerate() {
            let row = self.row(n, i);
            self.list.append(&row);
        }
        self.list.append(&self.hidden_row(count));
    }

    fn glyph_box(strength: Option<f64>, secure: bool) -> gtk::Overlay {
        let glyph = gtk::Overlay::new();
        glyph.add_css_class("st-wifi-icon");
        glyph.set_size_request(24, -1);
        glyph.set_valign(gtk::Align::Center);
        let g = kit::icons::glyph(wifi_glyph(strength, 50.0), 20);
        g.set_halign(gtk::Align::Center);
        glyph.set_child(Some(&g));
        if secure {
            let lock = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            lock.add_css_class("st-wifi-lock");
            lock.set_halign(gtk::Align::End);
            lock.set_valign(gtk::Align::End);
            let lg = kit::icons::glyph("lock", 10);
            lg.set_hexpand(true);
            lg.set_halign(gtk::Align::Center);
            lock.append(&lg);
            glyph.add_overlay(&lock);
        }
        glyph
    }

    fn row(self: &Rc<Self>, mut n: Value, i: usize) -> gtk::Box {
        if let Some(iface) = self.iface.borrow().as_ref() {
            n["iface"] = json!(iface);
        }
        let active = n["active"] == true;
        let secure = n["secure"] == true;
        let status = if active {
            if secure { t("Verbonden, beveiligd", "Connected, secured").to_owned() } else { t("Verbonden", "Connected").to_owned() }
        } else {
            let mut parts = vec![if secure { t("Beveiligd", "Secured") } else { t("Open", "Open") }];
            if n["saved"] == true {
                parts.push(t("Opgeslagen", "Saved"));
            }
            parts.join(" · ")
        };
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
        row.add_css_class("st-wifi");
        if active {
            row.add_css_class("st-active");
        }
        if i > 0 {
            row.add_css_class("st-sep");
        }
        row.append(&Self::glyph_box(n["strength"].as_f64(), secure));
        let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
        text.set_hexpand(true);
        text.set_valign(gtk::Align::Center);
        let ssid = s(&n["ssid"]).to_owned();
        text.append(&c::label(if ssid.is_empty() { "–" } else { ssid.as_str() }, "st-wifi-name", false));
        text.append(&c::label(&status, "st-wifi-status", false));
        row.append(&text);
        // GNOME's network options (gear): this network's Wi-Fi properties
        if let Some(uuid) = self.uuid_of(&ssid) {
            let props = c::icon_button("properties", t("Eigenschappen", "Properties"), 32);
            props.set_valign(gtk::Align::Center);
            props.set_widget_name(&format!("wifi-properties-{i}"));
            let pp = self.p.clone();
            props.connect_clicked(move |_| pp.go_with("wifi-network", json!({ "uuid": uuid })));
            row.append(&props);
        }
        let btn = c::button(if active { t("Verbinding verbreken", "Disconnect") } else { t("Verbinden", "Connect") }, Variant::Standard);
        btn.set_valign(gtk::Align::Center);
        btn.set_widget_name(&format!("wifi-connect-{i}"));
        let me = self.clone();
        btn.connect_clicked(move |b| {
            let (me, n, b) = (me.clone(), n.clone(), b.clone());
            let ssid = s(&n["ssid"]).to_owned();
            if n["active"] == true {
                let me2 = me.clone();
                c::confirm(
                    &me.p.anchor(),
                    &common::fill(t("Verbinding met “{ssid}” verbreken?", "Disconnect from “{ssid}”?"), &[("ssid", &ssid)]),
                    t("Je bent dan niet meer verbonden met internet via dit netwerk.", "You’ll no longer be connected to the internet through this network."),
                    t("Verbinding verbreken", "Disconnect"),
                    false,
                    move |ok| {
                        if !ok {
                            return;
                        }
                        let args = if n["iface"].is_string() { json!({ "iface": n["iface"] }) } else { json!({}) };
                        let wait = me2.p.run_with("system.network.disconnect", args, None, true);
                        let me3 = me2.clone();
                        me2.p.spawn(async move {
                            if wait.await.is_some() {
                                me3.load(false);
                            }
                        });
                    },
                );
                return;
            }
            b.set_sensitive(false);
            b.set_label(t("Verbinden…", "Connecting…"));
            let p = me.p.clone();
            p.spawn(async move {
                let ok = connect_wifi(&me.p, n, None).await;
                b.set_sensitive(true);
                b.set_label(t("Verbinden", "Connect"));
                if ok && me.p.alive() {
                    me.load(false);
                }
            });
        });
        row.append(&btn);
        row
    }

    /// Windows' "Verborgen netwerk" entry at the end of the list (GNOME's Connect to Hidden Network).
    fn hidden_row(self: &Rc<Self>, i: usize) -> gtk::Box {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
        row.add_css_class("st-wifi");
        if i > 0 {
            row.add_css_class("st-sep");
        }
        row.append(&Self::glyph_box(Some(100.0), true));
        let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
        text.set_hexpand(true);
        text.set_valign(gtk::Align::Center);
        text.append(&c::label(t("Verborgen netwerk", "Hidden network"), "st-wifi-name", false));
        text.append(&c::label(t("Een netwerk dat zijn naam niet uitzendt", "A network that doesn’t broadcast its name"), "st-wifi-status", false));
        row.append(&text);
        let btn = c::button(t("Verbinden", "Connect"), Variant::Standard);
        btn.set_valign(gtk::Align::Center);
        btn.set_widget_name("wifi-hidden-connect");
        let me = self.clone();
        btn.connect_clicked(move |_| {
            let me2 = me.clone();
            let done: Rc<dyn Fn(&Dialog, &gtk::Label, ed::NewNetwork)> = Rc::new(move |d: &Dialog, err: &gtk::Label, (ssid, kind, groups, auto, _): ed::NewNetwork| {
                let mut secrets = json!({});
                for (g, v) in groups.as_object().into_iter().flatten() {
                    if !v.is_null() {
                        secrets[g.as_str()] = v.clone();
                    }
                }
                let mut args = json!({ "ssid": ssid, "security": kind, "secrets": secrets, "autoconnect": auto });
                if let Some(iface) = me2.iface.borrow().as_ref() {
                    args["iface"] = json!(iface);
                }
                let (me3, ssid2) = (me2.clone(), ssid.clone());
                ed::dialog_call(d, err, "net.wifi.hidden", args, move |v| {
                    connected_toast(&me3.p, &ssid2, v["dryRun"] == true);
                    me3.load(false);
                });
            });
            ed::new_network(&me.p, t("Verbinding maken met een verborgen netwerk", "Connect to a hidden network"), t("Verbinden", "Connect"), false, done);
        });
        row.append(&btn);
        row
    }
}

/// The adapter's hardware (Windows' "Hardware-eigenschappen"; GNOME's Wi-Fi details): driver, MAC, bands.
fn hardware_rows(dev: &Value) -> Vec<(String, String)> {
    let mut bands = Vec::new();
    if dev["bands"]["2.4"] == true {
        bands.push(ed::band_text("2.4"));
    }
    if dev["bands"]["5"] == true {
        bands.push(ed::band_text("5"));
    }
    vec![
        (t("Fabrikant:", "Manufacturer:").into(), s(&dev["vendor"]).into()),
        (t("Beschrijving:", "Description:").into(), s(&dev["model"]).into()),
        (t("Stuurprogramma:", "Driver:").into(), s(&dev["driver"]).into()),
        (t("Versie van stuurprogramma:", "Driver version:").into(), s(&dev["driverVersion"]).into()),
        (t("Firmwareversie:", "Firmware version:").into(), s(&dev["firmware"]).into()),
        (t("Ondersteunde banden:", "Supported bands:").into(), bands.join(", ")),
        (t("Fysiek adres (MAC):", "Physical address (MAC):").into(), s(if s(&dev["permHwAddress"]).is_empty() { &dev["hwAddress"] } else { &dev["permHwAddress"] }).into()),
        (t("Netwerkadapter:", "Network adapter:").into(), s(&dev["iface"]).into()),
    ]
    .into_iter()
    .filter(|(_, v): &(String, String)| !v.is_empty())
    .collect()
}

/// A "Kopiëren" button for a property list that can change (another adapter chosen): it copies the rows it holds
/// when clicked.
fn copy_rows_button(p: &Page, rows: Rc<RefCell<Vec<(String, String)>>>, name: &str) -> gtk::Button {
    let copy = c::button(t("Kopiëren", "Copy"), Variant::Standard);
    copy.set_widget_name(name);
    let pp = p.clone();
    copy.connect_clicked(move |_| {
        let text = rows.borrow().iter().map(|(k, v)| format!("{k}\t{v}")).collect::<Vec<_>>().join("\n");
        let ok = common::copy_text(&text);
        pp.toast(Toast::new(if ok { ct::copied() } else { ct::copy_failed() }).severity(if ok { Severity::Success } else { Severity::Error }).timeout(2000));
    });
    copy
}

fn wifi(p: Page) -> PageFuture {
    Box::pin(async move {
        let (nr, dr, kr) = (p.call("system.network.get", json!({})), p.get("net.devices", json!({})), p.get("net.connections", json!({ "type": "802-11-wireless" })));
        let (nr, devs, known) = (nr.await, dr.await, kr.await);
        if !p.alive() {
            return Content::new();
        }
        let net = match nr {
            Ok(v) if v.is_object() => v,
            _ => return not_available(),
        };
        if net["wifiAvailable"] != true {
            return vec![common::notice(t("Er is geen Wi-Fi-adapter gevonden op deze computer.", "No Wi-Fi adapter was found on this computer.")).widget().clone().upcast()];
        }
        let adapters = devs.as_ref().map(|d| devices_of(d, "wifi")).unwrap_or_default();
        let known: Vec<Value> = known.and_then(|v| v["connections"].as_array().cloned()).unwrap_or_default();
        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        list.add_css_class("st-wifi-list");
        list.set_widget_name("wifi-networks");
        let refresh = c::icon_button("refresh", t("Vernieuwen", "Refresh"), 32);
        refresh.set_widget_name("wifi-refresh");
        let iface = (adapters.len() > 1).then(|| s(&adapters[0]["iface"]).to_owned());
        let page = Rc::new(WifiPage {
            p: p.clone(),
            state: RefCell::new(net.clone()),
            networks: RefCell::new(Vec::new()),
            shape: RefCell::new(Vec::new()),
            known: RefCell::new(known.clone()),
            built_active: active_profile(&known).and_then(|k| k["uuid"].as_str()).map(str::to_owned),
            iface: RefCell::new(iface),
            busy: Cell::new(false),
            list: list.clone(),
            refresh: refresh.clone(),
        });

        let exp = SettingsExpander::new("wifi-3", t("Beschikbare netwerken weergeven", "Show available networks"), t("Kies een netwerk om verbinding te maken", "Choose a network to connect"));
        exp.widget().add_css_class("st-wifi-exp");
        exp.header().set_control(&refresh);
        exp.set_expanded(true);
        exp.add(&list);
        exp.widget().set_visible(net["wifiEnabled"] == true);
        {
            let pg = page.clone();
            refresh.connect_clicked(move |_| pg.load(true));
        }

        let toggle = ToggleSwitch::new(net["wifiEnabled"] == true);
        toggle.set_sensitive(net["airplane"] != true);
        toggle.switch().set_widget_name("wifi-toggle");
        {
            let (pg, sw, exp) = (page.clone(), toggle.clone(), exp.clone());
            toggle.connect_toggled(move |on| {
                let wait = pg.p.run_with("system.network.setWifi", json!({ "enabled": on }), None, true);
                let (pg, sw, exp) = (pg.clone(), sw.clone(), exp.clone());
                let p = pg.p.clone();
                p.spawn(async move {
                    if wait.await.is_none() {
                        sw.set_active(!on);
                        return;
                    }
                    pg.state.borrow_mut()["wifiEnabled"] = json!(on);
                    exp.widget().set_visible(on);
                    if on {
                        pg.load(true);
                    } else {
                        pg.paint_list();
                    }
                });
            });
        }
        {
            let (pg, sw, exp) = (page.clone(), toggle.clone(), exp.clone());
            p.on("system.network.changed", move |d| {
                if !d.is_object() {
                    return;
                }
                let was = pg.state.borrow()["wifiEnabled"] == true;
                *pg.state.borrow_mut() = d.clone();
                let on = d["wifiEnabled"] == true;
                sw.set_active(on);
                sw.set_sensitive(d["airplane"] != true);
                exp.widget().set_visible(on);
                if on && !was {
                    pg.load(true);
                }
            });
        }
        if net["wifiEnabled"] == true {
            page.load(false);
        }
        let card = SettingsCard::new(
            "wifi",
            t("Wi-Fi", "Wi-Fi"),
            if net["airplane"] == true { t("Vliegtuigmodus", "Flight mode") } else { t("Verbinden met draadloze netwerken", "Connect to wireless networks") },
        );
        card.set_control(toggle.widget());
        let group = SettingsGroup::new("", "");
        group.add(card.widget());
        // the adapter's hardware (GNOME's per-adapter details): the chosen adapter's, refilled when another is chosen
        let hw_rows: Rc<RefCell<Vec<(String, String)>>> = Rc::new(RefCell::new(adapters.first().map(hardware_rows).unwrap_or_default()));
        let hw_slot = common::slot();
        common::set_slot(&hw_slot, &[specs(&hw_rows.borrow(), true)]);
        // GNOME's adapter switcher (several Wi-Fi adapters)
        if adapters.len() > 1 {
            let opts = adapters
                .iter()
                .map(|d| {
                    let model = s(&d["model"]);
                    DropdownOption::new(s(&d["iface"]), &if model.is_empty() { s(&d["iface"]).to_owned() } else { format!("{model} ({})", s(&d["iface"])) })
                })
                .collect();
            let dd = Dropdown::new(opts, page.iface.borrow().as_deref());
            dd.widget().set_widget_name("wifi-adapter");
            let (pg, adapters2, hw_rows, hw_slot) = (page.clone(), adapters.clone(), hw_rows.clone(), hw_slot.clone());
            dd.connect_changed(move |v| {
                *pg.iface.borrow_mut() = Some(v.to_owned());
                pg.networks.borrow_mut().clear();
                if let Some(dev) = adapters2.iter().find(|d| d["iface"] == v) {
                    *hw_rows.borrow_mut() = hardware_rows(dev);
                    common::set_slot(&hw_slot, &[specs(&hw_rows.borrow(), true)]);
                }
                pg.load(true);
            });
            let ac = SettingsCard::new("network", t("Adapter", "Adapter"), t("De Wi-Fi-adapter van deze lijst", "The Wi-Fi adapter of this list"));
            ac.set_control(dd.widget());
            group.add(ac.widget());
        }
        group.add(exp.widget());
        // the connected network's properties (Windows: "<SSID>-eigenschappen")
        if let Some(k) = active_profile(&known) {
            let uuid = s(&k["uuid"]).to_owned();
            let pc = SettingsCard::clickable(
                "properties",
                &common::fill(t("Eigenschappen van {ssid}", "{ssid} properties"), &[("ssid", s(&k["ssid"]))]),
                active_status(k),
                Chevron::Right,
            );
            if let Some(b) = pc.button() {
                b.set_widget_name("wifi-active-properties");
            }
            let pp = p.clone();
            pc.connect_clicked(move || pp.go_with("wifi-network", json!({ "uuid": uuid })));
            group.add(pc.widget());
        }
        group.add(common::link_card(&p, "wifi-known").widget());
        if !adapters.is_empty() {
            let hw = SettingsExpander::new("network", t("Hardware-eigenschappen", "Hardware properties"), t("Stuurprogramma, MAC-adres en banden van de Wi-Fi-adapter", "Driver, MAC address and bands of the Wi-Fi adapter"));
            hw.header().set_control(&copy_rows_button(&p, hw_rows.clone(), "wifi-hardware-copy"));
            hw.add(&hw_slot);
            group.add(hw.widget());
        }
        // follow NetworkManager as GNOME's list does: another network connected (active), access points that come and
        // go (devices: the adapter's AccessPointAdded / Removed; accessPoints), profiles added or forgotten
        {
            let (pg, pending) = (page.clone(), Rc::new(Cell::new(false)));
            p.on("net.changed", move |d| {
                if pending.get() || !matches!(s(&d["what"]), "connections" | "active" | "devices" | "accessPoints") {
                    return;
                }
                pending.set(true);
                let (pg2, pending) = (pg.clone(), pending.clone());
                pg.p.timeout(1000, move || {
                    pending.set(false);
                    if !pg2.busy.get() && pg2.state.borrow()["wifiEnabled"] == true {
                        pg2.load(false);
                    }
                });
            });
        }
        vec![group.widget().clone().upcast()]
    })
}

// ---------------------------------------------------------------------------------------------- Ethernet

/// The state of an adapter as Windows writes it (also network_advanced.rs).
pub(super) fn device_status(dev: &Value) -> String {
    let speed = dev["speed"].as_u64().unwrap_or(0);
    match s(&dev["state"]) {
        "connected" if speed > 0 => common::fill(t("Verbonden, {speed} Mbps", "Connected, {speed} Mbps"), &[("speed", &speed.to_string())]),
        "connected" => t("Verbonden", "Connected").to_owned(),
        "connecting" => t("Verbinding maken…", "Connecting…").to_owned(),
        "deactivating" => t("Verbinding verbreken…", "Disconnecting…").to_owned(),
        "unplugged" => t("Netwerkkabel losgekoppeld", "Network cable unplugged").to_owned(),
        "unmanaged" => t("Niet beheerd door NetworkManager", "Not managed by NetworkManager").to_owned(),
        "failed" => t("Verbinden lukte niet", "Couldn’t connect").to_owned(),
        "unavailable" => ct::unavailable().to_owned(),
        _ => t("Niet verbonden", "Not connected").to_owned(),
    }
}

/// The profile an adapter's settings show: its active one, else the first it can use.
fn device_profile(dev: &Value, profiles: &[Value]) -> Option<String> {
    dev["active"]["uuid"]
        .as_str()
        .map(str::to_owned)
        .or_else(|| dev["available"].as_array().and_then(|a| a.first()).and_then(|u| u.as_str().map(str::to_owned)))
        .or_else(|| profiles.iter().find(|c| c["interface"] == dev["iface"]).and_then(|c| c["uuid"].as_str().map(str::to_owned)))
}

/// A card with a value under its title and "Bewerken" on the right (Windows' IP assignment rows).
fn edit_row(title: &str, value: &str, name: &str, on_edit: impl Fn() + 'static) -> SettingsCard {
    let card = SettingsCard::new("", title, value);
    let b = c::button(t("Bewerken", "Edit"), Variant::Standard);
    b.set_widget_name(name);
    b.connect_clicked(move |_| on_edit());
    card.set_control(&b);
    card
}

/// The rows of one profile shared by Ethernet (and the Wi-Fi properties page in wifi_network.rs): metered, IP and DNS.
pub(super) fn ip_rows(p: &Page, page: &str, uuid: &str, settings: &Value, saved: Rc<dyn Fn()>) -> Vec<gtk::Widget> {
    let mut out: Vec<gtk::Widget> = Vec::new();
    let metered = ToggleSwitch::new(ed::metered_on(settings));
    metered.switch().set_widget_name(&format!("st-{page}-metered"));
    {
        let (pp, uuid, sw) = (p.clone(), uuid.to_owned(), metered.clone());
        metered.connect_toggled(move |on| {
            let sw = sw.clone();
            ed::save_keys(&pp, &uuid, json!({ "connection": { "metered": if on { 1 } else { 2 } } }), move || sw.set_active(!on));
        });
    }
    let mc = SettingsCard::new(
        "",
        t("Verbinding met een datalimiet", "Metered connection"),
        t(
            "Sommige apps werken mogelijk anders om het gegevensgebruik te beperken wanneer je met dit netwerk bent verbonden",
            "Some apps might work differently to reduce data usage when you’re connected to this network",
        ),
    );
    mc.set_control(metered.widget());
    out.push(mc.widget().clone());
    let (pp, u, st, sv) = (p.clone(), uuid.to_owned(), settings.clone(), saved.clone());
    out.push(edit_row(t("IP-toewijzing:", "IP assignment:"), &ed::ip_assignment_text(settings), &format!("{page}-ip-edit"), move || ed::edit_ip(&pp, &u, &st, sv.clone())).widget().clone());
    let (pp, u, st, sv) = (p.clone(), uuid.to_owned(), settings.clone(), saved);
    out.push(edit_row(t("DNS-servertoewijzing:", "DNS server assignment:"), &ed::dns_text(settings), &format!("{page}-dns-edit"), move || ed::edit_dns(&pp, &u, &st, sv.clone())).widget().clone());
    out
}

/// GNOME's Ethernet Identity tab as a dialog: Naam, MAC-adres, Gekloond adres, MTU.
fn edit_identity(p: &Page, uuid: &str, settings: &Value, dev: &Value, saved: Rc<dyn Fn()>) {
    let form = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let name = ed::entry(s(&settings["connection"]["id"]), "", "ethernet-identity-name");
    form.append(&c::field(t("Naam", "Name"), &name, None));
    let eth = &settings["802-3-ethernet"];
    let mac_now = ed::mac_text(&eth["mac-address"]);
    let dev_mac = s(if s(&dev["permHwAddress"]).is_empty() { &dev["hwAddress"] } else { &dev["permHwAddress"] }).to_uppercase();
    let mut mac_opts = vec![DropdownOption::new("", t("Geen (elke adapter)", "None (any adapter)"))];
    if !dev_mac.is_empty() {
        mac_opts.push(DropdownOption::new(&dev_mac, &format!("{dev_mac} ({})", s(&dev["iface"]))));
    }
    if !mac_now.is_empty() && mac_now != dev_mac {
        mac_opts.push(DropdownOption::new(&mac_now, &mac_now));
    }
    let mac = Dropdown::new(mac_opts, Some(&mac_now));
    mac.widget().set_widget_name("ethernet-identity-mac");
    form.append(&c::field(t("MAC-adres", "MAC address"), mac.widget(), Some(t("De adapter waarvoor dit profiel geldt", "The adapter this profile is for"))));
    let cloned_now = s(&eth["assigned-mac-address"]).to_owned();
    let cloned_now = if cloned_now.is_empty() { ed::mac_text(&eth["cloned-mac-address"]) } else { cloned_now };
    let cloned_key = if cloned_now.is_empty() { "default".to_owned() } else { cloned_now.clone() };
    let mut cloned_opts: Vec<DropdownOption> = ed::CLONED_MAC.iter().map(|r| DropdownOption::new(r.0, if kit::i18n::en() { r.2 } else { r.1 })).collect();
    if !ed::CLONED_MAC.iter().any(|r| r.0 == cloned_key) {
        cloned_opts.push(DropdownOption::new(&cloned_key, &cloned_key.to_uppercase()));
    }
    let cloned = Dropdown::new(cloned_opts, Some(&cloned_key));
    cloned.widget().set_widget_name("ethernet-identity-cloned");
    form.append(&c::field(t("Gekloond adres", "Cloned address"), cloned.widget(), None));
    let mtu = c::number_box(0.0, 65536.0, 1.0, eth["mtu"].as_f64().unwrap_or(0.0), 0);
    mtu.set_widget_name("ethernet-identity-mtu");
    form.append(&c::field(t("MTU", "MTU"), &mtu, Some(t("0 is automatisch", "0 is automatic"))));
    let err = ed::error_label();
    form.append(&err);
    let uuid = uuid.to_owned();
    Dialog::new(t("Geavanceerde eigenschappen bewerken", "Edit advanced properties"))
        .body(&form)
        .primary(t("Opslaan", "Save"))
        .width(460)
        .on_primary(move |d| {
            let id = name.text().trim().to_owned();
            if id.is_empty() {
                ed::show_error(&err, t("Typ een naam.", "Type a name."));
                return false;
            }
            let mac_v = mac.selected().filter(|m| !m.is_empty()).map(|m| json!(m)).unwrap_or(Value::Null);
            let cloned_v = match cloned.selected().as_deref() {
                None | Some("default") => Value::Null,
                Some(v) => json!(v),
            };
            let changes = json!({
                "connection": { "id": id },
                "802-3-ethernet": { "mac-address": mac_v, "assigned-mac-address": cloned_v, "cloned-mac-address": null, "mtu": mtu.value() as u64 },
            });
            let sv = saved.clone();
            ed::dialog_call(d, &err, "net.connection.update", json!({ "uuid": uuid, "settings": changes }), move |_| sv());
            false
        })
        .show(&p.anchor(), |_| {});
}

/// "Profiel toevoegen": a new wired profile for this adapter (GNOME's Add Ethernet Connection).
fn add_profile(p: &Page, iface: &str) {
    let name = ed::entry("", "", "ethernet-profile-name");
    let err = ed::error_label();
    let form = gtk::Box::new(gtk::Orientation::Vertical, 8);
    form.append(&c::field(t("Naam", "Name"), &name, None));
    form.append(&err);
    let (pp, iface) = (p.clone(), iface.to_owned());
    Dialog::new(t("Een profiel toevoegen", "Add a profile"))
        .body(&form)
        .primary(t("Opslaan", "Save"))
        .width(420)
        .on_primary(move |d| {
            let id = name.text().trim().to_owned();
            if id.is_empty() {
                ed::show_error(&err, t("Typ een naam.", "Type a name."));
                return false;
            }
            let settings = json!({
                "connection": { "id": id, "type": "802-3-ethernet", "interface-name": iface, "autoconnect": true },
                "802-3-ethernet": {}, "ipv4": { "method": "auto" }, "ipv6": { "method": "auto" },
            });
            let pp2 = pp.clone();
            ed::dialog_call(d, &err, "net.connection.add", json!({ "settings": settings }), move |v| {
                common::note_dry_run(&pp2.anchor(), &v);
                pp2.refresh();
            });
            false
        })
        .show(&p.anchor(), |_| {});
}

/// Forget a profile after Windows' question.
fn remove_profile(p: &Page, uuid: &str, name: &str) {
    let (pp, uuid) = (p.clone(), uuid.to_owned());
    c::confirm(
        &p.anchor(),
        &common::fill(t("Profiel “{name}” verwijderen?", "Remove the profile “{name}”?"), &[("name", name)]),
        t("De instellingen van dit profiel worden verwijderd.", "The settings of this profile will be removed."),
        t("Verwijderen", "Remove"),
        true,
        move |ok| {
            if ok {
                let wait = pp.run("net.connection.delete", json!({ "uuid": uuid }));
                let pp2 = pp.clone();
                pp.spawn(async move {
                    if wait.await.is_some() {
                        pp2.refresh();
                    }
                });
            }
        },
    );
}

/// Rename a profile (connection.id): the Bluetooth network's "Naam".
fn rename_profile(p: &Page, uuid: &str, name: &str) {
    let entry = ed::entry(name, "", "network-rename");
    let err = ed::error_label();
    let form = gtk::Box::new(gtk::Orientation::Vertical, 8);
    form.append(&c::field(t("Naam", "Name"), &entry, None));
    form.append(&err);
    let (pp, uuid) = (p.clone(), uuid.to_owned());
    Dialog::new(t("Naam wijzigen", "Rename"))
        .body(&form)
        .primary(t("Opslaan", "Save"))
        .width(420)
        .on_primary(move |d| {
            let id = entry.text().trim().to_owned();
            if id.is_empty() {
                ed::show_error(&err, t("Typ een naam.", "Type a name."));
                return false;
            }
            let pp2 = pp.clone();
            ed::dialog_call(d, &err, "net.connection.update", json!({ "uuid": uuid, "settings": { "connection": { "id": id } } }), move |_| pp2.refresh());
            false
        })
        .show(&p.anchor(), |_| {});
}

/// The ethernet page without net.* (an older host): the adapters' addresses from sysmon.
async fn ethernet_legacy(p: Page) -> Content {
    let (nr, eths) = (p.call("system.network.get", json!({})), ethernet_ifaces(&p));
    let (nr, eths) = (nr.await, eths.await);
    if !p.alive() {
        return Content::new();
    }
    let net = nr.unwrap_or(Value::Null);
    let active = s(&net["primary"]["type"]) == "ethernet";
    let group = SettingsGroup::new("", "");
    if eths.is_empty() && !active {
        group.add(SettingsCard::new("ethernet", t("Geen netwerkkabel gevonden", "No network cable found"), t("Sluit een netwerkkabel aan om via Ethernet te verbinden.", "Plug in a network cable to connect over Ethernet.")).widget());
        return vec![group.widget().clone().upcast()];
    }
    let list = if eths.is_empty() {
        let iface = net["primary"]["name"].as_str().filter(|s| !s.is_empty()).unwrap_or("eth0");
        vec![json!({ "iface": iface, "name": t("Ethernet", "Ethernet"), "ipv4": "", "ipv6": "" })]
    } else {
        eths
    };
    for (i, e) in list.iter().enumerate() {
        let (iface, name, v4, v6) = (s(&e["iface"]), s(&e["name"]), s(&e["ipv4"]), s(&e["ipv6"]));
        let connected = !v4.is_empty() || !v6.is_empty() || active;
        let state = if connected { t("Verbonden", "Connected") } else { t("Niet verbonden", "Not connected") };
        let rows: Vec<(String, String)> = vec![
            (t("Status", "Status").into(), state.into()),
            (t("Netwerkadapter", "Network adapter").into(), iface.into()),
            (t("IPv4-adres", "IPv4 address").into(), v4.into()),
            (t("IPv6-adres", "IPv6 address").into(), v6.into()),
        ];
        let title = if !name.is_empty() && name != iface { format!("{name} ({iface})") } else { iface.to_owned() };
        let exp = SettingsExpander::new("ethernet", &title, state);
        exp.header().set_control(&ed::copy_button(&p, rows.iter().filter(|(_, v)| !v.is_empty()).cloned().collect(), &format!("ethernet-copy-{i}")));
        exp.set_expanded(true);
        exp.add(&specs(&rows, true));
        group.add(exp.widget());
    }
    vec![group.widget().clone().upcast()]
}

fn ethernet(p: Page) -> PageFuture {
    Box::pin(async move {
        let (dr, cr, br) = (
            p.get("net.devices", json!({})),
            p.get("net.connections", json!({ "type": "802-3-ethernet" })),
            p.get("net.connections", json!({ "type": "bluetooth" })),
        );
        let (devs, profiles, bluetooth) = (dr.await, cr.await, br.await);
        let Some(devs) = devs else { return ethernet_legacy(p).await };
        let profiles: Vec<Value> = profiles.and_then(|v| v["connections"].as_array().cloned()).unwrap_or_default();
        let bluetooth: Vec<Value> = bluetooth.and_then(|v| v["connections"].as_array().cloned()).unwrap_or_default();
        let adapters = devices_of(&devs, "ethernet");
        // the profile of each adapter, read in full
        let mut shown = Vec::new();
        for dev in &adapters {
            let uuid = device_profile(dev, &profiles);
            let settings = match &uuid {
                Some(u) => p.get("net.connection.get", json!({ "uuid": u })).await.map(|v| v["settings"].clone()),
                None => None,
            };
            shown.push((dev.clone(), uuid, settings));
        }
        if !p.alive() {
            return Content::new();
        }
        let mut out: Content = Vec::new();
        if adapters.is_empty() {
            let group = SettingsGroup::new("", "");
            group.add(SettingsCard::new("ethernet", t("Geen netwerkkabel gevonden", "No network cable found"), t("Sluit een netwerkkabel aan om via Ethernet te verbinden.", "Plug in a network cable to connect over Ethernet.")).widget());
            out.push(group.widget().clone().upcast());
        }
        let saved: Rc<dyn Fn()> = {
            let pp = p.clone();
            Rc::new(move || pp.refresh())
        };
        let several = adapters.len() > 1;
        for (i, (dev, uuid, settings)) in shown.iter().enumerate() {
            let iface = s(&dev["iface"]).to_owned();
            let title = match (&dev["active"]["id"], several) {
                (Value::String(id), false) => id.clone(),
                (Value::String(id), true) => format!("{id} ({iface})"),
                _ if several => format!("{} ({iface})", t("Ethernet", "Ethernet")),
                _ => t("Ethernet", "Ethernet").to_owned(),
            };
            let group = SettingsGroup::new(if several { iface.as_str() } else { "" }, "");
            // GNOME's wired switch and status ("Verbonden, 1000 Mbps")
            let state = s(&dev["state"]);
            let on = matches!(state, "connected" | "connecting");
            let sw = ToggleSwitch::new(on);
            sw.switch().set_widget_name(&format!("st-ethernet-{iface}-enabled"));
            sw.set_sensitive(!matches!(state, "unplugged" | "unavailable" | "unmanaged"));
            {
                let (pp, sw2, iface) = (p.clone(), sw.clone(), iface.clone());
                sw.connect_toggled(move |on| {
                    let wait = pp.run("net.device.setEnabled", json!({ "iface": iface, "on": on }));
                    let sw = sw2.clone();
                    pp.spawn(async move {
                        if wait.await.is_none() {
                            sw.set_active(!on);
                        }
                    });
                });
            }
            let head = SettingsCard::new("ethernet", &title, &device_status(dev));
            head.set_control(sw.widget());
            head.widget().set_widget_name(&format!("ethernet-adapter-{i}"));
            group.add(head.widget());
            match (uuid, settings) {
                (Some(uuid), Some(settings)) => {
                    let (pp, u, st, sv) = (p.clone(), uuid.clone(), settings.clone(), saved.clone());
                    let auth = if settings["802-1x"].is_object() {
                        let eap = settings["802-1x"]["eap"][0].as_str().and_then(|e| ed::EAP.iter().find(|r| r.0 == e)).map(|r| if kit::i18n::en() { r.2 } else { r.1 }).unwrap_or("");
                        common::fill(t("Ingeschakeld ({method})", "Enabled ({method})"), &[("method", eap)])
                    } else {
                        t("Uitgeschakeld", "Disabled").to_owned()
                    };
                    group.add(edit_row(t("Verificatie-instellingen", "Authentication settings"), &auth, "ethernet-8021x-edit", move || ed::edit_8021x(&pp, &u, &st, sv.clone())).widget());
                    for w in ip_rows(&p, "ethernet", uuid, settings, saved.clone()) {
                        group.add(&w);
                    }
                    let (pp, u, st, d, sv) = (p.clone(), uuid.clone(), settings.clone(), dev.clone(), saved.clone());
                    group.add(
                        edit_row(t("Geavanceerd:", "Advanced:"), t("Naam, MAC-adres, gekloond adres, MTU", "Name, MAC address, cloned address, MTU"), "ethernet-identity-edit", move || {
                            edit_identity(&pp, &u, &st, &d, sv.clone())
                        })
                        .widget(),
                    );
                }
                _ => {
                    let card = SettingsCard::new("", t("Geen profiel", "No profile"), t("Deze adapter heeft nog geen profiel met instellingen.", "This adapter doesn’t have a profile with settings yet."));
                    let b = c::button(t("Profiel toevoegen", "Add profile"), Variant::Standard);
                    let (pp, ifc) = (p.clone(), iface.clone());
                    b.connect_clicked(move |_| add_profile(&pp, &ifc));
                    card.set_control(&b);
                    group.add(card.widget());
                }
            }
            // the properties with Kopiëren
            let rows = ed::properties(dev, None);
            let props = SettingsExpander::new("", t("Eigenschappen", "Properties"), &iface);
            props.header().set_control(&ed::copy_button(&p, rows.clone(), &format!("ethernet-copy-{i}")));
            props.set_expanded(true);
            props.add(&specs(&rows, true));
            group.add(props.widget());
            // GNOME's wired profiles: activate, add, remove
            let mine: Vec<&Value> = profiles.iter().filter(|c| c["interface"] == dev["iface"] || s(&c["interface"]).is_empty()).collect();
            let pe = SettingsExpander::new("", t("Profielen", "Profiles"), t("Kies welk profiel deze adapter gebruikt", "Choose which profile this adapter uses"));
            let add = c::button(t("Profiel toevoegen", "Add profile"), Variant::Standard);
            add.set_widget_name(&format!("ethernet-profile-add-{i}"));
            {
                let (pp, ifc) = (p.clone(), iface.clone());
                add.connect_clicked(move |_| add_profile(&pp, &ifc));
            }
            pe.header().set_control(&add);
            for (j, prof) in mine.iter().enumerate() {
                let (pu, pn) = (s(&prof["uuid"]).to_owned(), s(&prof["id"]).to_owned());
                let active = dev["active"]["uuid"] == prof["uuid"];
                let row = SettingsCard::new("", &pn, if active { t("Actief", "Active") } else { "" });
                let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                if !active {
                    let ab = c::button(t("Activeren", "Activate"), Variant::Standard);
                    ab.set_widget_name(&format!("ethernet-profile-activate-{i}-{j}"));
                    let (pp, pu2, ifc) = (p.clone(), pu.clone(), iface.clone());
                    ab.connect_clicked(move |_| {
                        let wait = pp.run("net.connection.activate", json!({ "uuid": pu2, "device": ifc }));
                        let pp2 = pp.clone();
                        pp.spawn(async move {
                            if wait.await.is_some() {
                                pp2.refresh();
                            }
                        });
                    });
                    buttons.append(&ab);
                }
                let rb = c::button(t("Verwijderen", "Remove"), Variant::Standard);
                rb.set_widget_name(&format!("ethernet-profile-remove-{i}-{j}"));
                let pp = p.clone();
                rb.connect_clicked(move |_| remove_profile(&pp, &pu, &pn));
                buttons.append(&rb);
                row.set_control(&buttons);
                pe.add(row.widget());
            }
            group.add(pe.widget());
            out.push(group.widget().clone().upcast());
        }
        // GNOME's Bluetooth network (PAN / DUN) profiles
        if !bluetooth.is_empty() {
            let group = SettingsGroup::new(t("Bluetooth-netwerk", "Bluetooth network"), "");
            for (i, b) in bluetooth.iter().enumerate() {
                let (uuid, name) = (s(&b["uuid"]).to_owned(), s(&b["id"]).to_owned());
                let kind = if b["bluetoothType"] == "dun" { t("Inbelnetwerk (DUN)", "Dial-up network (DUN)") } else { t("Persoonlijk netwerk (PAN)", "Personal area network (PAN)") };
                let card = SettingsCard::new("bluetooth", &name, kind);
                let sw = ToggleSwitch::new(b["active"] == true);
                sw.switch().set_widget_name(&format!("ethernet-bluetooth-{i}"));
                {
                    let (pp, sw2, uuid) = (p.clone(), sw.clone(), uuid.clone());
                    sw.connect_toggled(move |on| {
                        let wait = pp.run(if on { "net.connection.activate" } else { "net.connection.deactivate" }, json!({ "uuid": uuid }));
                        let sw = sw2.clone();
                        pp.spawn(async move {
                            if wait.await.is_none() {
                                sw.set_active(!on);
                            }
                        });
                    });
                }
                let rename = c::button(t("Naam wijzigen", "Rename"), Variant::Standard);
                {
                    let (pp, uuid, name) = (p.clone(), uuid.clone(), name.clone());
                    rename.connect_clicked(move |_| rename_profile(&pp, &uuid, &name));
                }
                let controls = gtk::Box::new(gtk::Orientation::Horizontal, 12);
                controls.append(&rename);
                controls.append(sw.widget());
                card.set_control(&controls);
                group.add(card.widget());
            }
            out.push(group.widget().clone().upcast());
        }
        // follow NetworkManager (a cable, another profile, the shown profile changed elsewhere: nmcli, GNOME Settings'
        // wired editor): build again when what the page shows changed; this page's own switch or dialog a moment ago
        // is taken as the new state (as wifi_network.rs)
        let shown_state = |d: &Value| -> Vec<(String, String, String)> {
            devices_of(d, "ethernet").iter().map(|x| (s(&x["iface"]).to_owned(), s(&x["state"]).to_owned(), s(&x["active"]["uuid"]).to_owned())).collect()
        };
        let before = Rc::new(RefCell::new(shown_state(&devs)));
        let profiles_shown: Rc<RefCell<Vec<(String, Value)>>> =
            Rc::new(RefCell::new(shown.iter().filter_map(|(_, u, st)| Some((u.clone()?, profile_shape(st.as_ref()?)))).collect()));
        let pending = Rc::new(Cell::new(false));
        let profiles_dirty = Rc::new(Cell::new(false));
        {
            let pp = p.clone();
            p.on("net.changed", move |d| {
                if !matches!(d["what"].as_str(), Some("devices" | "active" | "connections")) {
                    return;
                }
                if d["what"] == "connections" {
                    profiles_dirty.set(true);
                }
                if pending.get() {
                    return;
                }
                pending.set(true);
                let (pp2, before, pending, profiles_shown, profiles_dirty) = (pp.clone(), before.clone(), pending.clone(), profiles_shown.clone(), profiles_dirty.clone());
                pp.timeout(700, move || {
                    let wait = pp2.get("net.devices", json!({}));
                    let reads: Vec<(String, _)> = if profiles_dirty.replace(false) {
                        profiles_shown.borrow().iter().map(|(u, _)| (u.clone(), pp2.get("net.connection.get", json!({ "uuid": u })))).collect()
                    } else {
                        Vec::new()
                    };
                    let pp3 = pp2.clone();
                    pp2.spawn(async move {
                        let now = wait.await;
                        let mut profile_changed = false;
                        for (uuid, read) in reads {
                            let Some(v) = read.await else {
                                profile_changed = true;
                                continue;
                            };
                            let shape = profile_shape(&v["settings"]);
                            let mut shown = profiles_shown.borrow_mut();
                            let Some(entry) = shown.iter_mut().find(|(u, _)| *u == uuid) else { continue };
                            if entry.1 != shape {
                                if !ed::own_write_recent(&uuid) {
                                    profile_changed = true;
                                }
                                entry.1 = shape;
                            }
                        }
                        pending.set(false);
                        if !pp3.alive() {
                            return;
                        }
                        let device_changed = now.as_ref().is_some_and(|n| shown_state(n) != *before.borrow());
                        if let Some(n) = &now {
                            *before.borrow_mut() = shown_state(n);
                        }
                        if device_changed || profile_changed {
                            pp3.refresh();
                        }
                    });
                });
            });
        }
        out
    })
}

/// What the Ethernet page shows of a profile: its settings without the last-used time (NetworkManager moves that while
/// the profile is up, which changes nothing on the page).
fn profile_shape(settings: &Value) -> Value {
    let mut v = settings.clone();
    if let Some(c) = v.get_mut("connection").and_then(Value::as_object_mut) {
        c.remove("timestamp");
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn several_adapters_list_their_own_networks() {
        let dev = json!({ "iface": "wlan1", "accessPoints": [
            { "ssid": "Thuis", "strength": 40, "security": "wpa-psk", "active": false },
            { "ssid": "Thuis", "strength": 70, "security": "wpa-psk", "active": false },
            { "ssid": "Cafe", "strength": 90, "security": "open", "active": false },
            { "ssid": "", "strength": 99, "security": "open", "active": false },
        ] });
        let known = vec![json!({ "ssid": "Thuis", "uuid": "u" })];
        let list = networks_of(&dev, &known);
        assert_eq!(list.len(), 2);
        let thuis = list.iter().find(|n| n["ssid"] == "Thuis").unwrap();
        assert_eq!((thuis["strength"].as_u64(), thuis["saved"].as_bool(), thuis["secure"].as_bool()), (Some(70), Some(true), Some(true)));
        assert_eq!(thuis["iface"], "wlan1");
        assert_eq!(list.iter().find(|n| n["ssid"] == "Cafe").unwrap()["secure"], false);
        assert_eq!(connect_args(thuis, Some("pw")), json!({ "ssid": "Thuis", "password": "pw", "iface": "wlan1" }));
        assert_eq!(connect_args(&json!({ "ssid": "A" }), None), json!({ "ssid": "A" }));
    }

    #[test]
    fn enterprise_and_wep_networks_get_their_fields() {
        let n = |sec: &str| json!({ "ssid": "x", "security": sec, "secure": sec != "open" });
        // net.devices' kinds and nmcli's
        assert_eq!(editor_kind(&n("wpa-eap")), Some("wpa-eap"));
        assert_eq!(editor_kind(&n("enterprise")), Some("wpa-eap"));
        assert_eq!(editor_kind(&n("WPA2 802.1X")), Some("wpa-eap"));
        assert_eq!(editor_kind(&n("wep")), Some("wep"));
        assert_eq!(editor_kind(&n("WEP")), Some("wep"));
        for key_only in ["wpa-psk", "sae", "WPA2", "WPA1 WPA2", "open", "owe", ""] {
            assert_eq!(editor_kind(&n(key_only)), None, "{key_only}");
        }
    }

    #[test]
    fn the_list_follows_networkmanager_without_repainting_for_the_signal() {
        let a = vec![
            json!({ "ssid": "Lab-5G", "strength": 80, "secure": true, "active": true, "saved": true }),
            json!({ "ssid": "eduroam", "strength": 55, "secure": true, "active": false, "saved": false }),
        ];
        let mut b = a.clone();
        b[0]["strength"] = json!(82);
        assert_eq!(list_shape(&a), list_shape(&b), "a signal change within the same glyph");
        b.reverse();
        assert_eq!(list_shape(&a), list_shape(&b), "the order NetworkManager lists them in");
        let mut c = a.clone();
        c[0]["active"] = json!(false);
        c[1]["active"] = json!(true);
        assert_ne!(list_shape(&a), list_shape(&c), "another network connected");
        let mut d = a.clone();
        d.push(json!({ "ssid": "Kantoor", "strength": 40, "secure": true }));
        assert_ne!(list_shape(&a), list_shape(&d), "a network came into range");
        let mut e = a.clone();
        e[1]["strength"] = json!(5);
        assert_ne!(list_shape(&a), list_shape(&e), "another strength glyph");
        let known = vec![json!({ "uuid": "h", "ssid": "Hotspot", "active": true, "mode": "ap" }), json!({ "uuid": "u", "ssid": "Lab-5G", "active": true })];
        assert_eq!(active_profile(&known).map(|k| k["uuid"].clone()), Some(json!("u")), "the hotspot is no connected network");
        kit::i18n::set_forced_lang(Some("en"));
        assert_eq!(active_status(&json!({ "keyMgmt": "" })), "Connected");
        assert_eq!(active_status(&json!({ "keyMgmt": "owe" })), "Connected");
        assert_eq!(active_status(&json!({ "keyMgmt": "wpa-psk" })), "Connected, secured");
        assert_eq!(active_status(&json!({ "keyMgmt": "wpa-eap" })), "Connected, secured");
    }

    #[test]
    fn the_ethernet_page_compares_its_profile_without_the_last_used_time() {
        let a = json!({ "connection": { "id": "Wired connection 1", "timestamp": 1_700_000_000u64, "metered": 0 }, "ipv4": { "method": "auto" } });
        let mut b = a.clone();
        b["connection"]["timestamp"] = json!(1_700_000_300u64);
        assert_eq!(profile_shape(&a), profile_shape(&b));
        let mut c = a.clone();
        c["connection"]["metered"] = json!(1);
        assert_ne!(profile_shape(&a), profile_shape(&c), "metered set elsewhere");
        let mut d = a.clone();
        d["ipv4"]["method"] = json!("manual");
        assert_ne!(profile_shape(&a), profile_shape(&d), "IP assignment changed elsewhere");
    }

    #[test]
    fn an_adapter_shows_its_active_or_first_profile() {
        let profiles = vec![json!({ "uuid": "b", "interface": "eth1" })];
        assert_eq!(device_profile(&json!({ "active": { "uuid": "a" }, "available": ["c"] }), &profiles).as_deref(), Some("a"));
        assert_eq!(device_profile(&json!({ "active": null, "available": ["c"] }), &profiles).as_deref(), Some("c"));
        assert_eq!(device_profile(&json!({ "iface": "eth1", "available": [] }), &profiles).as_deref(), Some("b"));
        assert_eq!(device_profile(&json!({ "iface": "eth2", "available": [] }), &profiles), None);
        kit::i18n::set_forced_lang(Some("en"));
        assert_eq!(device_status(&json!({ "state": "connected", "speed": 1000 })), "Connected, 1000 Mbps");
        assert_eq!(device_status(&json!({ "state": "unplugged" })), "Network cable unplugged");
        let hw = hardware_rows(&json!({ "iface": "wlan0", "driver": "iwlwifi", "bands": { "2.4": true, "5": true }, "permHwAddress": "AA:BB:CC:00:11:22" }));
        assert!(hw.iter().any(|(k, v)| k == "Supported bands:" && v == "2.4 GHz, 5 GHz"));
        assert!(hw.iter().all(|(_, v)| !v.is_empty()));
        // the Adapter drop-down's other adapter: its own rows (driver, MAC), not the first adapter's
        let usb = hardware_rows(&json!({ "iface": "wlan1", "driver": "mt7921u", "hwAddress": "02:00:00:00:00:01" }));
        assert!(usb.iter().any(|(k, v)| k == "Driver:" && v == "mt7921u"));
        assert!(usb.iter().any(|(k, v)| k == "Physical address (MAC):" && v == "02:00:00:00:00:01"));
        assert!(!usb.iter().any(|(_, v)| v == "iwlwifi" || v == "wlan0"));
    }
}
