//! Instellingen › Netwerk en internet (last-webkit:vendor/ui/js/apps/settings/pages/network.js) with Wi-Fi (the
//! networks, connecting with a password or to an open network, disconnecting) and Ethernet (the adapters' addresses).
use super::{Builder, Content, Page, PageFuture};
use crate::common::{self, ct};
use crate::widgets::Wrap;
use kit::controls::{self as c, Chevron, Dialog, DialogResult, ProgressRing, SettingsCard, SettingsExpander, SettingsGroup, Severity, Toast, ToggleSwitch, Variant};
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

/// The password / open-network flow; true when connected (network.js connectWifi). `error` pre-fills the
/// password box (the router's password changed, so the saved one no longer works).
pub fn connect_wifi(p: &Page, n: Value, error: Option<String>) -> Pin<Box<dyn Future<Output = bool>>> {
    let p = p.clone();
    Box::pin(async move {
        let ssid = s(&n["ssid"]).to_owned();
        let secure = n["secure"] == true;
        let saved = n["saved"] == true;
        if secure && (!saved || error.is_some()) {
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
            let (entry2, err2, ssid2, dry2) = (entry.clone(), err.clone(), ssid.clone(), dry.clone());
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
                    let wait = super::call("system.network.connect", json!({ "ssid": ssid2, "password": pw }));
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
        let r = p.call("system.network.connect", json!({ "ssid": ssid })).await;
        let (ok, dry_run, code, msg) = connect_outcome(&r);
        if ok {
            connected_toast(&p, &ssid, dry_run);
            return true;
        }
        // a saved network whose password no longer works (changed on the router): ask for the new one, like Windows
        if secure && (code == "needs_password" || code == "wrong_password") {
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

fn network(p: Page) -> PageFuture {
    Box::pin(async move {
        let (nr, eths) = (p.call("system.network.get", json!({})), ethernet_ifaces(&p));
        let (nr, eths) = (nr.await, eths.await);
        if !p.alive() {
            return Content::new();
        }
        let net = match nr {
            Ok(v) if v.is_object() => v,
            _ => return not_available(),
        };
        let state = Rc::new(RefCell::new(net));

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
                Some(Rc::new(move || pp.go(if s(&st.borrow()["primary"]["type"]) == "ethernet" { "ethernet" } else { "wifi" }))),
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

        // ---- Wi-Fi, Ethernet, Vliegtuigmodus
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
                if eth_active { t("Verbonden", "Connected") } else { t("Verbinding via een netwerkkabel", "Connection through a network cable") },
                Chevron::Right,
            );
            if let Some(b) = card.button() {
                b.set_widget_name("network-ethernet");
            }
            let pp = p.clone();
            card.connect_clicked(move || pp.go("ethernet"));
            group.add(card.widget());
        }
        let air = SettingsCard::new(
            "airplane",
            t("Vliegtuigmodus", "Flight mode"),
            t("Alle draadloze verbindingen uitzetten, zoals Wi-Fi en Bluetooth", "Stop all wireless communication, such as Wi-Fi and Bluetooth"),
        );
        air.set_control(air_sw.widget());
        group.add(air.widget());
        vec![hero.upcast(), group.widget().clone().upcast()]
    })
}

// ------------------------------------------------------------------------------------------------- Wi-Fi

struct WifiPage {
    p: Page,
    state: RefCell<Value>,
    networks: RefCell<Vec<Value>>,
    busy: Cell<bool>,
    list: gtk::Box,
    refresh: gtk::Button,
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

    fn load(self: &Rc<Self>, rescan: bool) {
        if self.busy.get() {
            return;
        }
        self.busy.set(true);
        self.refresh.set_sensitive(false);
        if rescan || self.networks.borrow().is_empty() {
            self.scanning_row();
        }
        let wait = self.p.get("system.network.wifiList", json!({ "rescan": rescan }));
        let me = self.clone();
        self.p.spawn(async move {
            let r = wait.await;
            me.busy.set(false);
            me.refresh.set_sensitive(true);
            if !me.p.alive() {
                return;
            }
            *me.networks.borrow_mut() = r.and_then(|v| v.as_array().cloned()).unwrap_or_default();
            me.paint_list();
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
            return;
        }
        for (i, n) in sorted.into_iter().enumerate() {
            let row = self.row(n, i);
            self.list.append(&row);
        }
    }

    fn row(self: &Rc<Self>, n: Value, i: usize) -> gtk::Box {
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
        let glyph = gtk::Overlay::new();
        glyph.add_css_class("st-wifi-icon");
        glyph.set_size_request(24, -1);
        glyph.set_valign(gtk::Align::Center);
        let g = kit::icons::glyph(wifi_glyph(n["strength"].as_f64(), 50.0), 20);
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
        row.append(&glyph);
        let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
        text.set_hexpand(true);
        text.set_valign(gtk::Align::Center);
        let ssid = s(&n["ssid"]);
        text.append(&c::label(if ssid.is_empty() { "–" } else { ssid }, "st-wifi-name", false));
        text.append(&c::label(&status, "st-wifi-status", false));
        row.append(&text);
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
                        let wait = me2.p.run_with("system.network.disconnect", json!({}), None, true);
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
}

fn wifi(p: Page) -> PageFuture {
    Box::pin(async move {
        let nr = p.call("system.network.get", json!({})).await;
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
        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        list.add_css_class("st-wifi-list");
        list.set_widget_name("wifi-networks");
        let refresh = c::icon_button("refresh", t("Vernieuwen", "Refresh"), 32);
        refresh.set_widget_name("wifi-refresh");
        let page = Rc::new(WifiPage { p: p.clone(), state: RefCell::new(net.clone()), networks: RefCell::new(Vec::new()), busy: Cell::new(false), list: list.clone(), refresh: refresh.clone() });

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
        group.add(exp.widget());
        vec![group.widget().clone().upcast()]
    })
}

// ---------------------------------------------------------------------------------------------- Ethernet

fn ethernet(p: Page) -> PageFuture {
    Box::pin(async move {
        let (nr, eths) = (p.call("system.network.get", json!({})), ethernet_ifaces(&p));
        let (nr, eths) = (nr.await, eths.await);
        if !p.alive() {
            return Content::new();
        }
        let net = nr.unwrap_or(Value::Null);
        let active = s(&net["primary"]["type"]) == "ethernet";
        let group = SettingsGroup::new("", "");
        if eths.is_empty() && !active {
            group.add(
                SettingsCard::new(
                    "ethernet",
                    t("Geen netwerkkabel gevonden", "No network cable found"),
                    t("Sluit een netwerkkabel aan om via Ethernet te verbinden.", "Plug in a network cable to connect over Ethernet."),
                )
                .widget(),
            );
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
            let copy = c::button(t("Kopiëren", "Copy"), Variant::Standard);
            copy.set_widget_name(&format!("ethernet-copy-{i}"));
            {
                let (pp, rows) = (p.clone(), rows.clone());
                copy.connect_clicked(move |_| {
                    let text = rows.iter().filter(|(_, v)| !v.is_empty()).map(|(k, v)| format!("{k}\t{v}")).collect::<Vec<_>>().join("\n");
                    let ok = common::copy_text(&text);
                    pp.toast(Toast::new(if ok { ct::copied() } else { ct::copy_failed() }).severity(if ok { Severity::Success } else { Severity::Error }).timeout(2000));
                });
            }
            let title = if !name.is_empty() && name != iface { format!("{name} ({iface})") } else { iface.to_owned() };
            let exp = SettingsExpander::new("ethernet", &title, state);
            exp.header().set_control(&copy);
            exp.set_expanded(true);
            exp.add(&specs(&rows, true));
            group.add(exp.widget());
        }
        vec![group.widget().clone().upcast()]
    })
}
