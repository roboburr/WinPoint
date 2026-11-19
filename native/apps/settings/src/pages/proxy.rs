//! Instellingen › Netwerk en internet › Proxy (batch C, notes/claude/settings-parity.md §2): GNOME Settings' Network
//! Proxy (net-proxy.c: `org.gnome.system.proxy` mode none / auto / manual, the configuration URL, the HTTP, HTTPS,
//! FTP and SOCKS servers and the ignored hosts) in the layout of Windows 11's Proxy page: "Automatische
//! proxyinstellingen" (Instellingen automatisch detecteren = mode auto without a URL, GNOME's WPAD; Installatiescript
//! gebruiken = mode auto with the URL) and "Handmatige proxyinstellingen" (Een proxyserver gebruiken = mode manual).
//!
//! GNOME has one mode where Windows has three switches: turning one on turns the others off, and the page says so.
//! Windows' one server ("IP-adres van proxy", "Poort") is GNOME's HTTP server, written to HTTPS and FTP too while
//! "Dezelfde proxyserver voor alle protocollen gebruiken" is on (the per-protocol servers are under "Geavanceerd");
//! "De proxyserver niet gebruiken voor lokale (intranet)adressen" is the `*.local` entry of the ignored hosts. Every
//! Opslaan is one `gsettings.set {writes}` with `mode` last (amendment 51); `authentication-password` is never written.
use super::net_editor as ed;
use super::{Builder, Content, Page, PageFuture};
use crate::common;
use crate::gs::{self, GKey, Gs};
use kit::controls::{self as c, Dialog, DialogResult, Expander, SettingsCard, SettingsGroup, Severity, ToggleSwitch, Variant};
use kit::prelude::*;
use kit::t;
use std::cell::RefCell;
use std::rc::Rc;

pub const PAGES: &[(&str, Builder)] = &[("proxy", proxy)];
pub const CSS: &str = r#"
.st-proxy-port { min-width: 96px; }
"#;

const PROXY: &str = "org.gnome.system.proxy";
const MODE: GKey = gs::key(PROXY, "mode");
const URL: GKey = gs::key(PROXY, "autoconfig-url");
const IGNORE: GKey = gs::key(PROXY, "ignore-hosts");
/// The four protocols of GNOME's manual proxy: (schema, label).
pub const PROTOCOLS: &[(&str, &str)] = &[
    ("org.gnome.system.proxy.http", "HTTP"),
    ("org.gnome.system.proxy.https", "HTTPS"),
    ("org.gnome.system.proxy.ftp", "FTP"),
    ("org.gnome.system.proxy.socks", "SOCKS"),
];
/// The ignored host behind "De proxyserver niet gebruiken voor lokale (intranet)adressen".
pub const LOCAL: &str = "*.local";

fn host_key(i: usize) -> GKey {
    gs::key(PROTOCOLS[i].0, "host")
}

fn port_key(i: usize) -> GKey {
    gs::key(PROTOCOLS[i].0, "port")
}

/// Every key of the page (one gsettings.get).
pub fn keys() -> Vec<GKey> {
    let mut k = vec![MODE, URL, IGNORE];
    for i in 0..PROTOCOLS.len() {
        k.push(host_key(i));
        k.push(port_key(i));
    }
    k
}

/// The ignored hosts as Windows' text (entries separated by `;`), without the `*.local` entry its check box shows.
pub fn ignore_text(list: &[String]) -> (String, bool) {
    let local = list.iter().any(|h| h == LOCAL);
    (list.iter().filter(|h| *h != LOCAL).cloned().collect::<Vec<_>>().join(";"), local)
}

/// Windows' text (`;`, or GNOME's `,`) as the ignored hosts, `*.local` added when the check box is on.
pub fn ignore_list(text: &str, local: bool) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for h in text.split([';', ',']).map(str::trim).filter(|h| !h.is_empty() && *h != LOCAL) {
        if !out.iter().any(|o| o == h) {
            out.push(h.to_owned());
        }
    }
    if local {
        out.push(LOCAL.to_owned());
    }
    out
}

/// A port of the dialog: 1 to 65535.
pub fn port_of(text: &str) -> Option<i64> {
    text.trim().parse::<i64>().ok().filter(|p| (1..=65535).contains(p))
}

/// A proxy host: a name or address without blanks or slashes (a typed `http://` is taken off first).
pub fn host_of(text: &str) -> Option<String> {
    let h = text.trim();
    let h = h.strip_prefix("http://").or_else(|| h.strip_prefix("https://")).unwrap_or(h).trim_end_matches('/');
    (!h.is_empty() && h.len() <= 255 && !h.contains(char::is_whitespace) && !h.contains('/')).then(|| h.to_owned())
}

/// A setup script address: http(s) or file.
pub fn script_ok(url: &str) -> bool {
    let u = url.trim();
    (u.starts_with("http://") || u.starts_with("https://") || u.starts_with("file://")) && u.len() > 8 && !u.contains(char::is_whitespace)
}

/// One write of `gsettings.set {writes}`.
fn write(schema: &str, values: Value) -> Value {
    json!({ "schema": schema, "values": values })
}

/// The writes of "Automatisch detecteren" (on: the URL emptied, then mode auto; off: mode none).
pub fn detect_writes(on: bool) -> Value {
    if on {
        json!([write(PROXY, json!({ "autoconfig-url": "" })), write(PROXY, json!({ "mode": "auto" }))])
    } else {
        json!([write(PROXY, json!({ "mode": "none" }))])
    }
}

/// The writes of "Installatiescript gebruiken" (on: the URL, then mode auto).
pub fn script_writes(url: &str) -> Value {
    json!([write(PROXY, json!({ "autoconfig-url": url.trim() })), write(PROXY, json!({ "mode": "auto" }))])
}

/// The writes of the manual proxy: the four servers (host, port; an empty host keeps GNOME's port), the ignored
/// hosts, then mode manual.
pub fn manual_writes(servers: &[(String, Option<i64>); 4], ignore: &[String]) -> Value {
    let mut out: Vec<Value> = Vec::new();
    for (i, (host, port)) in servers.iter().enumerate() {
        let mut values = json!({ "host": host });
        if let Some(p) = port {
            values["port"] = json!(p);
        }
        out.push(write(PROTOCOLS[i].0, values));
    }
    out.push(write(PROXY, json!({ "ignore-hosts": ignore })));
    out.push(write(PROXY, json!({ "mode": "manual" })));
    Value::Array(out)
}

/// What the page shows of GNOME's keys.
#[derive(Clone, Default)]
struct State {
    mode: String,
    url: String,
    ignore: Vec<String>,
    hosts: [(String, i64); 4],
}

impl State {
    fn of(gs: &Gs) -> State {
        let mut st = State {
            mode: common::s(&gs.value(&MODE)).to_owned(),
            url: common::s(&gs.value(&URL)).to_owned(),
            ignore: gs.value(&IGNORE).as_array().map(|a| a.iter().filter_map(|h| h.as_str().map(str::to_owned)).collect()).unwrap_or_default(),
            ..State::default()
        };
        for i in 0..4 {
            st.hosts[i] = (common::s(&gs.value(&host_key(i))).to_owned(), gs.value(&port_key(i)).as_i64().unwrap_or(0));
        }
        st
    }

    fn detect(&self) -> bool {
        self.mode == "auto" && self.url.trim().is_empty()
    }

    fn script(&self) -> bool {
        self.mode == "auto" && !self.url.trim().is_empty()
    }

    fn manual(&self) -> bool {
        self.mode == "manual"
    }

    /// HTTPS and FTP use the HTTP server (or have none of their own).
    fn same_for_all(&self) -> bool {
        let http = &self.hosts[0];
        [1, 2].iter().all(|&i| self.hosts[i].0.is_empty() || self.hosts[i] == *http)
    }
}

/// The server text of a card ("proxy.example.org:8080").
fn server_text(host: &str, port: i64) -> String {
    if port > 0 { format!("{host}:{port}") } else { host.to_owned() }
}

/// Run `gsettings.set {writes}` from a page control; `done(ok)` afterwards.
fn save(p: &Page, writes: Value, done: impl FnOnce(bool) + 'static) {
    let wait = p.run_with("gsettings.set", json!({ "writes": writes }), None, false);
    p.spawn(async move { done(wait.await.is_some()) });
}

fn proxy(p: Page) -> PageFuture {
    Box::pin(async move {
        let keys = keys();
        let gs = Gs::load(&p, &keys).await;
        if !p.alive() {
            return Content::new();
        }
        if !gs.available(&MODE) {
            return vec![common::notice(common::ct::unavailable_here()).widget().clone().upcast()];
        }
        let state = Rc::new(RefCell::new(State::of(&gs)));
        let writable = gs.writable(&MODE);
        let mut out: Content = Vec::new();

        // ---- Automatische proxyinstellingen
        let auto = SettingsGroup::new(
            t("Automatische proxyinstellingen", "Automatic proxy setup"),
            t(
                "Gebruik een proxyserver voor Ethernet- of Wi-Fi-verbindingen. Er kan maar één van deze proxyinstellingen tegelijk aan staan.",
                "Use a proxy server for Ethernet or Wi-Fi connections. Only one of these proxy settings can be on at a time.",
            ),
        );
        let detect_sw = ToggleSwitch::new(state.borrow().detect());
        detect_sw.switch().set_widget_name(&gs::control_name(p.id(), &keys, &MODE));
        let detect = SettingsCard::new("", t("Instellingen automatisch detecteren", "Automatically detect settings"), "");
        detect.set_control(detect_sw.widget());
        auto.add(detect.widget());
        let wpad = common::notice_full(
            Severity::Warning,
            "",
            t(
                "Zonder scriptadres wordt WPAD (automatische proxydetectie) gebruikt. Dit wordt niet aanbevolen voor openbare netwerken die je niet vertrouwt.",
                "Without a script address WPAD (Web Proxy Auto-Discovery) is used. This isn’t recommended for untrusted public networks.",
            ),
        );
        wpad.widget().set_widget_name("proxy-wpad");
        auto.add(wpad.widget());
        let script = SettingsCard::new("", t("Installatiescript gebruiken", "Use setup script"), "");
        let script_btn = c::button(t("Instellen", "Set up"), Variant::Standard);
        script_btn.set_widget_name("proxy-script-edit");
        script.set_control(&script_btn);
        auto.add(script.widget());

        // ---- Handmatige proxyinstellingen
        let manual = SettingsGroup::new(t("Handmatige proxyinstellingen", "Manual proxy setup"), t("Gebruik een proxyserver voor Ethernet- of Wi-Fi-verbindingen.", "Use a proxy server for Ethernet or Wi-Fi connections."));
        let server = SettingsCard::new("", t("Een proxyserver gebruiken", "Use a proxy server"), "");
        let server_btn = c::button(t("Instellen", "Set up"), Variant::Standard);
        server_btn.set_widget_name("proxy-manual-edit");
        server.set_control(&server_btn);
        manual.add(server.widget());

        let paint: Rc<dyn Fn()> = {
            let (state, detect_sw, wpad, script, server) = (state.clone(), detect_sw.clone(), wpad.clone(), script.clone(), server.clone());
            Rc::new(move || {
                let st = state.borrow();
                detect_sw.set_active(st.detect());
                wpad.set_visible(st.detect());
                script.set_description(if st.script() { st.url.as_str() } else { common::ct::off() });
                let http = &st.hosts[0];
                let text = if st.manual() { server_text(&http.0, http.1) } else { common::ct::off().to_owned() };
                server.set_description(&text);
            })
        };
        paint();
        for k in keys.iter() {
            let (gs2, state, paint) = (gs.clone(), state.clone(), paint.clone());
            gs.follow(k, move |_| {
                *state.borrow_mut() = State::of(&gs2);
                paint();
            });
        }
        if !writable {
            detect_sw.set_sensitive(false);
            for w in [detect_sw.switch().clone().upcast::<gtk::Widget>(), script_btn.clone().upcast(), server_btn.clone().upcast()] {
                w.set_sensitive(false);
                w.set_tooltip_text(Some(gs::locked_text()));
            }
        }
        {
            let (pp, sw, state, paint) = (p.clone(), detect_sw.clone(), state.clone(), paint.clone());
            detect_sw.connect_toggled(move |on| {
                // switching auto-detection off turns the proxy off only while auto-detection is what is on
                if !on && !state.borrow().detect() {
                    return;
                }
                let (sw, state, paint) = (sw.clone(), state.clone(), paint.clone());
                save(&pp, detect_writes(on), move |ok| {
                    if ok {
                        let mut st = state.borrow_mut();
                        st.mode = if on { "auto".into() } else { "none".into() };
                        if on {
                            st.url.clear();
                        }
                    } else {
                        sw.set_active(!on);
                    }
                    paint();
                });
            });
        }
        {
            let (pp, keys, state, paint) = (p.clone(), keys.clone(), state.clone(), paint.clone());
            script_btn.connect_clicked(move |_| script_dialog(&pp, &keys, &state, paint.clone()));
        }
        {
            let (pp, keys, state, paint) = (p.clone(), keys.clone(), state.clone(), paint.clone());
            server_btn.connect_clicked(move |_| manual_dialog(&pp, &keys, &state, paint.clone()));
        }
        out.push(auto.widget().clone().upcast());
        out.push(manual.widget().clone().upcast());
        let related = SettingsGroup::new(common::ct::related(), "");
        related.add(common::link_card(&p, "network-advanced").widget());
        out.push(related.widget().clone().upcast());
        out
    })
}

/// A dialog's own on/off switch (Windows' "Installatiescript gebruiken · Aan").
fn switch_row(title: &str, on: bool, name: &str) -> (gtk::Box, ToggleSwitch) {
    let row = gtk::Box::new(gtk::Orientation::Vertical, 4);
    row.append(&c::label(title, "st-net-form-section", true));
    let sw = ToggleSwitch::new(on);
    sw.switch().set_widget_name(name);
    sw.widget().set_halign(gtk::Align::Start);
    row.append(sw.widget());
    (row, sw)
}

/// "Installatiescript bewerken": the switch and "Scriptadres" (GNOME's Configuration URL).
fn script_dialog(p: &Page, keys: &[GKey], state: &Rc<RefCell<State>>, paint: Rc<dyn Fn()>) {
    let st = state.borrow().clone();
    let form = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let (row, sw) = switch_row(t("Installatiescript gebruiken", "Use setup script"), st.script(), "proxy-script-on");
    form.append(&row);
    let url = ed::entry(&st.url, "http://wpad.example.org/wpad.dat", &gs::control_name(p.id(), keys, &URL));
    form.append(&c::field(t("Scriptadres", "Script address"), &url, None));
    let err = ed::error_label();
    form.append(&err);
    let (anchor, state, was_script) = (p.anchor(), state.clone(), st.script());
    Dialog::new(t("Installatiescript bewerken", "Edit setup script"))
        .body(&form)
        .primary(t("Opslaan", "Save"))
        .width(480)
        .on_primary(move |d| {
            let on = sw.active();
            let text = url.text().trim().to_owned();
            let writes = if on {
                if !script_ok(&text) {
                    ed::show_error(&err, t("Typ een scriptadres dat begint met http://, https:// of file://.", "Type a script address that starts with http://, https:// or file://."));
                    return false;
                }
                script_writes(&text)
            } else if was_script {
                json!([write(PROXY, json!({ "mode": "none" }))])
            } else {
                d.close(DialogResult::Primary);
                return false;
            };
            let (state, paint, anchor) = (state.clone(), paint.clone(), anchor.clone());
            ed::dialog_call(d, &err, "gsettings.set", json!({ "writes": writes }), move |v| {
                common::note_dry_run(&anchor, &v);
                if v["dryRun"] != true {
                    let mut st = state.borrow_mut();
                    st.mode = if on { "auto".into() } else { "none".into() };
                    if on {
                        st.url = text;
                    }
                }
                paint();
            });
            false
        })
        .show(&p.anchor(), |_| {});
}

/// The host and port entries of one protocol, named after their keys.
fn host_port(p: &Page, keys: &[GKey], i: usize, host: &str, port: i64) -> (gtk::Entry, gtk::Entry) {
    let h = ed::entry(host, t("Adres", "Address"), &gs::control_name(p.id(), keys, &host_key(i)));
    let port_text = if host.is_empty() || port <= 0 { String::new() } else { port.to_string() };
    let pt = ed::entry(&port_text, t("Poort", "Port"), &gs::control_name(p.id(), keys, &port_key(i)));
    pt.add_css_class("st-proxy-port");
    pt.set_hexpand(false);
    pt.set_input_purpose(gtk::InputPurpose::Digits);
    (h, pt)
}

/// "Proxyserver bewerken": Windows' one server and its exceptions, GNOME's four servers under "Geavanceerd".
fn manual_dialog(p: &Page, keys: &[GKey], state: &Rc<RefCell<State>>, paint: Rc<dyn Fn()>) {
    let st = state.borrow().clone();
    let form = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let (row, sw) = switch_row(t("Een proxyserver gebruiken", "Use a proxy server"), st.manual(), "proxy-manual-on");
    form.append(&row);
    let fields = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let (http_host, http_port) = host_port(p, keys, 0, &st.hosts[0].0, st.hosts[0].1);
    let host_field = c::field(t("IP-adres van proxy", "Proxy IP address"), &http_host, None);
    host_field.set_hexpand(true);
    fields.append(&host_field);
    fields.append(&c::field(t("Poort", "Port"), &http_port, None));
    form.append(&fields);
    let (ignore, local) = ignore_text(&st.ignore);
    let exceptions = ed::entry(&ignore, "", &gs::control_name(p.id(), keys, &IGNORE));
    // Windows shows this long header in full above the box (the harness saw kit::field's one-line header cut off)
    let exceptions_field = gtk::Box::new(gtk::Orientation::Vertical, 8);
    exceptions_field.add_css_class("w11-field");
    exceptions_field.append(&c::label(
        t(
            "Gebruik de proxyserver behalve voor adressen die beginnen met de volgende vermeldingen. Gebruik puntkomma’s (;) om vermeldingen te scheiden.",
            "Use the proxy server except for addresses that start with the following entries. Use semicolons (;) to separate entries.",
        ),
        "w11-field-header",
        true,
    ));
    exceptions_field.append(&exceptions);
    form.append(&exceptions_field);
    let local_cb = c::checkbox(t("De proxyserver niet gebruiken voor lokale (intranet)adressen", "Don’t use the proxy server for local (intranet) addresses"));
    local_cb.set_active(local);
    local_cb.set_widget_name("proxy-local");
    form.append(&local_cb);
    // GNOME's servers per protocol
    let adv = Expander::new("", t("Geavanceerd", "Advanced"), t("Een andere proxyserver per protocol", "Another proxy server per protocol"));
    let adv_box = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let same = c::checkbox(t("Dezelfde proxyserver voor alle protocollen gebruiken", "Use the same proxy server for all protocols"));
    same.set_active(st.same_for_all());
    same.set_widget_name("proxy-same");
    adv_box.append(&same);
    let mirrored = gtk::Box::new(gtk::Orientation::Vertical, 8);
    adv_box.append(&mirrored);
    let mut rows: Vec<(gtk::Entry, gtk::Entry)> = Vec::new();
    for (i, (_, label)) in PROTOCOLS.iter().enumerate().skip(1) {
        let (h, pt) = host_port(p, keys, i, &st.hosts[i].0, st.hosts[i].1);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.append(&h);
        row.append(&pt);
        let f = c::field(label, &row, None);
        if i < 3 {
            mirrored.append(&f);
        } else {
            adv_box.append(&f);
        }
        rows.push((h, pt));
    }
    mirrored.set_sensitive(!same.is_active());
    {
        let mirrored = mirrored.clone();
        same.connect_toggled(move |b| mirrored.set_sensitive(!b.is_active()));
    }
    adv.set_content(&adv_box);
    form.append(adv.widget());
    let err = ed::error_label();
    form.append(&err);
    let (anchor, state, was_manual) = (p.anchor(), state.clone(), st.manual());
    Dialog::new(t("Proxyserver bewerken", "Edit proxy server"))
        .body(&ed::scroller(&form))
        .primary(t("Opslaan", "Save"))
        .width(560)
        .on_primary(move |d| {
            if !sw.active() {
                if !was_manual {
                    d.close(DialogResult::Primary);
                    return false;
                }
                let (state, paint, anchor) = (state.clone(), paint.clone(), anchor.clone());
                ed::dialog_call(d, &err, "gsettings.set", json!({ "writes": [write(PROXY, json!({ "mode": "none" }))] }), move |v| {
                    common::note_dry_run(&anchor, &v);
                    if v["dryRun"] != true {
                        state.borrow_mut().mode = "none".into();
                    }
                    paint();
                });
                return false;
            }
            let Some(host) = host_of(&http_host.text()) else {
                ed::show_error(&err, t("Typ het IP-adres of de naam van de proxyserver.", "Type the IP address or name of the proxy server."));
                return false;
            };
            let Some(port) = port_of(&http_port.text()) else {
                ed::show_error(&err, t("Typ een poort van 1 tot 65535.", "Type a port from 1 to 65535."));
                return false;
            };
            let mut servers: [(String, Option<i64>); 4] = [(host.clone(), Some(port)), (String::new(), None), (String::new(), None), (String::new(), None)];
            for (n, (h, pt)) in rows.iter().enumerate() {
                let slot = n + 1;
                if slot < 3 && same.is_active() {
                    servers[slot] = (host.clone(), Some(port));
                    continue;
                }
                let text = h.text().trim().to_owned();
                if text.is_empty() {
                    continue;
                }
                let protocol = [("protocol", PROTOCOLS[slot].1)];
                let Some(h) = host_of(&text) else {
                    ed::show_error(&err, &common::fill(t("{protocol}: typ het adres van de proxyserver.", "{protocol}: type the address of the proxy server."), &protocol));
                    return false;
                };
                let Some(port) = port_of(&pt.text()) else {
                    ed::show_error(&err, &common::fill(t("{protocol}: typ een poort van 1 tot 65535.", "{protocol}: type a port from 1 to 65535."), &protocol));
                    return false;
                };
                servers[slot] = (h, Some(port));
            }
            let list = ignore_list(&exceptions.text(), local_cb.is_active());
            let writes = manual_writes(&servers, &list);
            let (state, paint, anchor) = (state.clone(), paint.clone(), anchor.clone());
            ed::dialog_call(d, &err, "gsettings.set", json!({ "writes": writes }), move |v| {
                common::note_dry_run(&anchor, &v);
                if v["dryRun"] != true {
                    let mut st = state.borrow_mut();
                    st.mode = "manual".into();
                    st.ignore = list;
                    for (i, (h, pt)) in servers.iter().enumerate() {
                        let port = pt.unwrap_or(st.hosts[i].1);
                        st.hosts[i] = (h.clone(), port);
                    }
                }
                paint();
            });
            false
        })
        .show(&p.anchor(), |_| {});
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_exceptions_are_gnomes_ignored_hosts() {
        let gnome = vec!["localhost".to_owned(), "127.0.0.0/8".to_owned(), "::1".to_owned()];
        assert_eq!(ignore_text(&gnome), ("localhost;127.0.0.0/8;::1".to_owned(), false));
        let list = ignore_list("localhost; 127.0.0.0/8;;::1, intranet.example ;localhost", true);
        assert_eq!(list, ["localhost", "127.0.0.0/8", "::1", "intranet.example", "*.local"]);
        assert_eq!(ignore_text(&list), ("localhost;127.0.0.0/8;::1;intranet.example".to_owned(), true));
        assert_eq!(ignore_list("*.local;a", false), ["a"], "the check box decides about *.local");
    }

    #[test]
    fn hosts_ports_and_scripts() {
        assert_eq!(port_of(" 8080 "), Some(8080));
        assert_eq!(port_of("0"), None);
        assert_eq!(port_of("65536"), None);
        assert_eq!(port_of("x"), None);
        assert_eq!(host_of(" http://proxy.example.org/ ").as_deref(), Some("proxy.example.org"));
        assert_eq!(host_of("10.0.0.1").as_deref(), Some("10.0.0.1"));
        assert_eq!(host_of("a b"), None);
        assert_eq!(host_of(""), None);
        assert!(script_ok("http://wpad/wpad.dat") && script_ok("file:///etc/proxy.pac") && !script_ok("wpad.dat") && !script_ok("http://"));
    }

    #[test]
    fn every_save_writes_the_mode_last() {
        let servers = [("proxy".to_owned(), Some(3128)), ("proxy".to_owned(), Some(3128)), (String::new(), None), ("socks".to_owned(), Some(1080))];
        let w = manual_writes(&servers, &["localhost".to_owned()]);
        let list = w.as_array().unwrap();
        assert_eq!(list.len(), 6);
        assert_eq!(list[0], json!({ "schema": "org.gnome.system.proxy.http", "values": { "host": "proxy", "port": 3128 } }));
        assert_eq!(list[2], json!({ "schema": "org.gnome.system.proxy.ftp", "values": { "host": "" } }), "an empty server keeps GNOME's port");
        assert_eq!(list[4]["values"]["ignore-hosts"], json!(["localhost"]));
        assert_eq!(list[5], json!({ "schema": "org.gnome.system.proxy", "values": { "mode": "manual" } }));
        for w in [detect_writes(true), script_writes(" http://wpad/wpad.dat ")] {
            let a = w.as_array().unwrap();
            assert_eq!(a.last().unwrap()["values"]["mode"], "auto", "{w}");
        }
        assert_eq!(script_writes(" http://wpad/wpad.dat ")[0]["values"]["autoconfig-url"], "http://wpad/wpad.dat");
        assert_eq!(detect_writes(true)[0]["values"]["autoconfig-url"], "");
        assert_eq!(detect_writes(false), json!([{ "schema": "org.gnome.system.proxy", "values": { "mode": "none" } }]));
        // nothing writes the password GNOME keeps in plain text
        assert!(!w.to_string().contains("authentication"));
    }

    #[test]
    fn the_three_windows_switches_are_one_gnome_mode() {
        let mut st = State { mode: "auto".into(), ..State::default() };
        assert!(st.detect() && !st.script() && !st.manual());
        st.url = "http://wpad/wpad.dat".into();
        assert!(!st.detect() && st.script());
        st.mode = "manual".into();
        assert!(st.manual() && !st.script());
        st.hosts[0] = ("p".into(), 8080);
        assert!(st.same_for_all());
        st.hosts[1] = ("p".into(), 8080);
        assert!(st.same_for_all());
        st.hosts[2] = ("q".into(), 21);
        assert!(!st.same_for_all());
        assert_eq!(server_text("p", 8080), "p:8080");
        let names: Vec<String> = keys().iter().map(|k| gs::control_name("proxy", &keys(), k)).collect();
        assert!(names.contains(&"st-proxy-https-host".to_owned()) && names.contains(&"st-proxy-mode".to_owned()));
    }
}
