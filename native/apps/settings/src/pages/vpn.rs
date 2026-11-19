//! Instellingen › Netwerk en internet › VPN (batch C, notes/claude/settings-parity.md §2): GNOME Settings' VPN list
//! (a connect switch and the editor per VPN, Add VPN with the installed NetworkManager plugins and WireGuard, Import
//! from file…) in the layout of Windows 11's VPN page: "VPN-verbindingen" with "VPN toevoegen", one expander per
//! connection with "Verbinding maken" / "Verbinding verbreken", "Geavanceerde opties" and "Verwijderen".
//!
//! Amendment 40 (the libnma spike, recorded in notes/claude/settings-c.md): the GTK 4 plugin editors are not embedded
//! (no Dutch catalogue, GNOME's layout, libnm in the Settings process), so the forms are WinPoint's own: the common
//! OpenVPN and PPTP keys of `vpn.data` as fields, every other key as a name/value row (kept as NetworkManager has
//! it), WireGuard's own settings and peers, IPv4/IPv6 through net_editor.rs. Windows' "VPN toestaan via netwerken met
//! een datalimiet" and "VPN toestaan tijdens roaming" have no GNOME backing and are left out.
use super::net_editor as ed;
use super::{Builder, Content, Page, PageFuture};
use crate::common;
use kit::controls::{self as c, Dialog, Dropdown, DropdownOption, SettingsCard, SettingsExpander, SettingsGroup, ToggleSwitch, Variant};
use kit::prelude::*;
use kit::t;
use std::collections::BTreeMap;
use std::rc::Rc;

pub const PAGES: &[(&str, Builder)] = &[("vpn", vpn)];
pub const CSS: &str = r#"
.st-vpn-empty { color: var(--text-2); padding: 4px 0; }
"#;

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}

pub const OPENVPN: &str = "org.freedesktop.NetworkManager.openvpn";
pub const PPTP: &str = "org.freedesktop.NetworkManager.pptp";

/// (plugin name, nl, en): the providers GNOME's Add VPN list names (the plugin's own name for any other).
pub const PROVIDERS: &[(&str, &str, &str)] = &[
    ("openvpn", "OpenVPN", "OpenVPN"),
    ("pptp", "PPTP (Point-to-Point Tunneling Protocol)", "PPTP (Point-to-Point Tunnelling Protocol)"),
    ("l2tp", "L2TP/IPsec", "L2TP/IPsec"),
    ("strongswan", "IKEv2 (strongSwan)", "IKEv2 (strongSwan)"),
    ("vpnc", "Cisco-compatibele VPN (vpnc)", "Cisco-compatible VPN (vpnc)"),
    ("openconnect", "Cisco AnyConnect (OpenConnect)", "Cisco AnyConnect (OpenConnect)"),
    ("sstp", "SSTP", "SSTP"),
    ("fortisslvpn", "Fortinet SSL-VPN", "Fortinet SSL VPN"),
    ("wireguard", "WireGuard", "WireGuard"),
];

/// (connection-type, nl, en): OpenVPN's "Type aanmeldingsgegevens".
pub const OVPN_AUTH: &[(&str, &str, &str)] = &[
    ("password", "Gebruikersnaam en wachtwoord", "User name and password"),
    ("tls", "Certificaten (TLS)", "Certificates (TLS)"),
    ("password-tls", "Wachtwoord met certificaten (TLS)", "Password with certificates (TLS)"),
    ("static-key", "Statische sleutel", "Static key"),
];

/// How a vpn.data field is edited.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Field {
    Text,
    File,
    /// a check box that is on for "yes"
    Yes,
    /// a check box that is on unless the key is "yes" (PPTP's refuse-*, no-* keys: "toestaan")
    NotYes,
}

/// (key, field, nl, en): the common vpn.data keys of NetworkManager-openvpn (its editor's main and advanced tabs).
pub const OPENVPN_KEYS: &[(&str, Field, &str, &str)] = &[
    ("remote", Field::Text, "Server", "Server"),
    ("port", Field::Text, "Poort", "Port"),
    ("proto-tcp", Field::Yes, "Een TCP-verbinding gebruiken", "Use a TCP connection"),
    ("username", Field::Text, "Gebruikersnaam", "User name"),
    ("ca", Field::File, "CA-certificaat", "CA certificate"),
    ("cert", Field::File, "Gebruikerscertificaat", "User certificate"),
    ("key", Field::File, "Persoonlijke sleutel", "Private key"),
    ("static-key", Field::File, "Statische sleutel", "Static key"),
    ("static-key-direction", Field::Text, "Sleutelrichting (0 of 1)", "Key direction (0 or 1)"),
    ("local-ip", Field::Text, "Lokaal IP-adres", "Local IP address"),
    ("remote-ip", Field::Text, "Extern IP-adres", "Remote IP address"),
    ("dev-type", Field::Text, "Apparaattype (tun of tap)", "Device type (tun or tap)"),
    ("cipher", Field::Text, "Versleuteling", "Cipher"),
    ("auth", Field::Text, "HMAC-verificatie", "HMAC authentication"),
    ("ta", Field::File, "TLS-verificatiesleutel", "TLS authentication key"),
    ("ta-dir", Field::Text, "Richting van de TLS-sleutel (0 of 1)", "TLS key direction (0 or 1)"),
    ("tls-crypt", Field::File, "TLS-versleutelingssleutel", "TLS encryption key"),
    ("remote-cert-tls", Field::Text, "Certificaattype van de server controleren", "Verify the server certificate type"),
    ("tunnel-mtu", Field::Text, "MTU van de tunnel", "Tunnel MTU"),
];

/// (key, field, nl, en): the common vpn.data keys of NetworkManager-pptp (its editor and Advanced dialog).
pub const PPTP_KEYS: &[(&str, Field, &str, &str)] = &[
    ("gateway", Field::Text, "Server", "Server"),
    ("user", Field::Text, "Gebruikersnaam", "User name"),
    ("domain", Field::Text, "NT-domein", "NT domain"),
    ("require-mppe", Field::Yes, "Point-to-point-versleuteling (MPPE) gebruiken", "Use point-to-point encryption (MPPE)"),
    ("require-mppe-128", Field::Yes, "Alleen 128-bits versleuteling", "128-bit encryption only"),
    ("mppe-stateful", Field::Yes, "Stateful versleuteling toestaan", "Allow stateful encryption"),
    ("refuse-pap", Field::NotYes, "PAP toestaan", "Allow PAP"),
    ("refuse-chap", Field::NotYes, "CHAP toestaan", "Allow CHAP"),
    ("refuse-mschap", Field::NotYes, "MSCHAP toestaan", "Allow MSCHAP"),
    ("refuse-mschapv2", Field::NotYes, "MSCHAPv2 toestaan", "Allow MSCHAPv2"),
    ("refuse-eap", Field::NotYes, "EAP toestaan", "Allow EAP"),
    ("nobsdcomp", Field::NotYes, "BSD-gegevenscompressie toestaan", "Allow BSD data compression"),
    ("nodeflate", Field::NotYes, "Deflate-gegevenscompressie toestaan", "Allow Deflate data compression"),
    ("no-vj-comp", Field::NotYes, "TCP-headercompressie gebruiken", "Use TCP header compression"),
];

/// The common vpn.data keys of a plugin.
pub fn known_keys(service: &str) -> &'static [(&'static str, Field, &'static str, &'static str)] {
    match service {
        OPENVPN => OPENVPN_KEYS,
        PPTP => PPTP_KEYS,
        _ => &[],
    }
}

/// The provider's text of a profile (net.connections' summary or settings).
pub fn provider_label(service_or_name: &str) -> String {
    let name = service_or_name.rsplit('.').next().unwrap_or(service_or_name);
    PROVIDERS.iter().find(|r| r.0 == name).map(|r| ed::tr(r).to_owned()).unwrap_or_else(|| name.to_owned())
}

/// A WireGuard key: base64 of 32 bytes (44 characters ending in `=`).
pub fn wg_key_ok(key: &str) -> bool {
    key.len() == 44 && key.ends_with('=') && key[..43].bytes().all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/')
}

/// "10.8.0.2/24" → (address, prefix, IPv6?).
pub fn cidr(text: &str) -> Option<(String, u32, bool)> {
    let (a, p) = text.trim().split_once('/').unwrap_or((text.trim(), ""));
    let six = a.contains(':');
    if !ed::ip_ok(a, six) {
        return None;
    }
    let prefix = if p.is_empty() { if six { 128 } else { 32 } } else { p.parse::<u32>().ok().filter(|n| *n <= if six { 128 } else { 32 })? };
    Some((a.to_owned(), prefix, six))
}

/// Allowed IPs (comma- or space-separated CIDRs).
pub fn allowed_ips(text: &str) -> Option<Vec<String>> {
    let list: Vec<String> = text.split([',', ' ']).map(str::trim).filter(|x| !x.is_empty()).map(str::to_owned).collect();
    (!list.is_empty() && list.iter().all(|x| cidr(x).is_some())).then_some(list)
}

/// An endpoint "host:port" (port 51820 added when none), or the IPv6 "[addr]:port".
pub fn endpoint(server: &str) -> Option<String> {
    let s = server.trim();
    if s.is_empty() || s.contains(char::is_whitespace) {
        return None;
    }
    let has_port = if s.starts_with('[') { s.contains("]:") } else { s.matches(':').count() == 1 };
    Some(if has_port { s.to_owned() } else if s.contains(':') && !s.starts_with('[') { format!("[{s}]:51820") } else { format!("{s}:51820") })
}

/// The fields of "Een VPN-verbinding toevoegen".
#[derive(Clone, Debug, Default)]
pub struct NewVpn {
    pub provider: String,
    pub name: String,
    pub server: String,
    pub auth: String,
    pub user: String,
    pub password: String,
    pub remember: bool,
    pub ca: String,
    pub cert: String,
    pub key: String,
    pub key_password: String,
    pub static_key: String,
    pub local_ip: String,
    pub remote_ip: String,
    pub wg_iface: String,
    pub wg_private: String,
    pub wg_address: String,
    pub wg_peer: String,
    pub wg_allowed: String,
    pub wg_psk: String,
}

/// The NetworkManager profile of a new VPN, or the text of what is missing.
pub fn new_settings(f: &NewVpn) -> Result<Value, &'static str> {
    let name = f.name.trim();
    if name.is_empty() {
        return Err(t("Typ een verbindingsnaam.", "Type a connection name."));
    }
    if f.server.trim().is_empty() {
        return Err(t("Typ de servernaam of het serveradres.", "Type the server name or address."));
    }
    let flags = if f.remember { "1" } else { "2" };
    let base = |kind: &str| json!({ "connection": { "id": name, "type": kind, "autoconnect": false }, "ipv4": { "method": "auto" }, "ipv6": { "method": "auto" } });
    match f.provider.as_str() {
        "wireguard" => {
            let iface = f.wg_iface.trim();
            if iface.is_empty() || iface.len() > 15 || !iface.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b)) {
                return Err(t("Typ een interfacenaam van maximaal 15 tekens, zoals wg0.", "Type an interface name of up to 15 characters, such as wg0."));
            }
            if !wg_key_ok(f.wg_private.trim()) || !wg_key_ok(f.wg_peer.trim()) {
                return Err(t("Een WireGuard-sleutel bestaat uit 44 tekens en eindigt op =.", "A WireGuard key has 44 characters and ends in =."));
            }
            let (address, prefix, six) = cidr(&f.wg_address).ok_or(t("Typ het IP-adres van deze computer in de tunnel, zoals 10.8.0.2/24.", "Type this computer’s IP address in the tunnel, such as 10.8.0.2/24."))?;
            let allowed = allowed_ips(if f.wg_allowed.trim().is_empty() { "0.0.0.0/0, ::/0" } else { &f.wg_allowed }).ok_or(t("Typ geldige toegestane IP-adressen, zoals 0.0.0.0/0.", "Type valid allowed IP addresses, such as 0.0.0.0/0."))?;
            let mut peer = json!({ "public-key": f.wg_peer.trim(), "endpoint": endpoint(&f.server).ok_or(t("Typ de servernaam of het serveradres.", "Type the server name or address."))?, "allowed-ips": allowed });
            if !f.wg_psk.trim().is_empty() {
                if !wg_key_ok(f.wg_psk.trim()) {
                    return Err(t("Een WireGuard-sleutel bestaat uit 44 tekens en eindigt op =.", "A WireGuard key has 44 characters and ends in =."));
                }
                peer["preshared-key"] = json!(f.wg_psk.trim());
                peer["preshared-key-flags"] = json!(0);
            }
            let mut out = base("wireguard");
            out["connection"]["interface-name"] = json!(iface);
            out["wireguard"] = json!({ "private-key": f.wg_private.trim(), "private-key-flags": 0, "peers": [peer] });
            let ip = json!({ "method": "manual", "address-data": [{ "address": address, "prefix": prefix }] });
            if six {
                out["ipv6"] = ip;
                out["ipv4"] = json!({ "method": "disabled" });
            } else {
                out["ipv4"] = ip;
                out["ipv6"] = json!({ "method": "disabled" });
            }
            Ok(out)
        }
        "pptp" => {
            let mut data = json!({ "gateway": f.server.trim(), "password-flags": flags });
            if !f.user.trim().is_empty() {
                data["user"] = json!(f.user.trim());
            }
            let mut out = base("vpn");
            out["vpn"] = json!({ "service-type": PPTP, "data": data });
            if f.remember && !f.password.is_empty() {
                out["vpn"]["secrets"] = json!({ "password": f.password });
            }
            Ok(out)
        }
        "openvpn" => {
            let auth = if OVPN_AUTH.iter().any(|r| r.0 == f.auth) { f.auth.as_str() } else { "password" };
            let mut data = json!({ "remote": f.server.trim(), "connection-type": auth });
            let mut secrets = json!({});
            let path = |p: &str| p.trim().to_owned();
            match auth {
                "static-key" => {
                    if f.static_key.trim().is_empty() || f.local_ip.trim().is_empty() || f.remote_ip.trim().is_empty() {
                        return Err(t("Kies de statische sleutel en typ het lokale en het externe IP-adres.", "Choose the static key and type the local and remote IP addresses."));
                    }
                    data["static-key"] = json!(path(&f.static_key));
                    data["local-ip"] = json!(f.local_ip.trim());
                    data["remote-ip"] = json!(f.remote_ip.trim());
                }
                _ => {
                    if !f.ca.trim().is_empty() {
                        data["ca"] = json!(path(&f.ca));
                    }
                    if auth != "password" {
                        if f.cert.trim().is_empty() || f.key.trim().is_empty() {
                            return Err(t("Kies het gebruikerscertificaat en de persoonlijke sleutel.", "Choose the user certificate and the private key."));
                        }
                        data["cert"] = json!(path(&f.cert));
                        data["key"] = json!(path(&f.key));
                        data["cert-pass-flags"] = json!(flags);
                        if f.remember && !f.key_password.is_empty() {
                            secrets["cert-pass"] = json!(f.key_password);
                        }
                    }
                    if auth != "tls" {
                        if !f.user.trim().is_empty() {
                            data["username"] = json!(f.user.trim());
                        }
                        data["password-flags"] = json!(flags);
                        if f.remember && !f.password.is_empty() {
                            secrets["password"] = json!(f.password);
                        }
                    }
                }
            }
            let mut out = base("vpn");
            out["vpn"] = json!({ "service-type": OPENVPN, "data": data });
            if secrets.as_object().is_some_and(|o| !o.is_empty()) {
                out["vpn"]["secrets"] = secrets;
            }
            Ok(out)
        }
        _ => Err(t("Deze VPN-provider kan hier niet worden ingesteld. Importeer een configuratiebestand.", "This VPN provider can’t be set up here. Import a configuration file.")),
    }
}

/// The vpn.data of the advanced dialog: the known fields (an empty text or an off "yes" box leaves the key out), then
/// the name/value rows; None for a row without a name or with a name twice.
pub fn data_of(service: &str, fields: &BTreeMap<String, Value>, rows: &[Vec<String>]) -> Option<BTreeMap<String, String>> {
    let mut out = BTreeMap::new();
    for (key, kind, _, _) in known_keys(service) {
        match (kind, fields.get(*key)) {
            (Field::Text | Field::File, Some(Value::String(v))) if !v.trim().is_empty() => {
                out.insert((*key).to_owned(), v.trim().to_owned());
            }
            (Field::Yes, Some(Value::Bool(true))) | (Field::NotYes, Some(Value::Bool(false))) => {
                out.insert((*key).to_owned(), "yes".to_owned());
            }
            _ => {}
        }
    }
    for row in rows {
        let (k, v) = (row.first()?.trim(), row.get(1).map(|v| v.trim()).unwrap_or(""));
        if k.is_empty() || out.contains_key(k) {
            return None;
        }
        out.insert(k.to_owned(), v.to_owned());
    }
    Some(out)
}

/// The vpn.data keys the form does not show (the name/value rows).
pub fn other_keys(service: &str, data: &Value) -> Vec<Vec<String>> {
    let known = known_keys(service);
    data.as_object()
        .map(|o| {
            o.iter()
                .filter(|(k, _)| !known.iter().any(|(kk, ..)| kk == k) && !k.ends_with("-flags") && *k != "connection-type")
                .map(|(k, v)| vec![k.clone(), s(v).to_owned()])
                .collect()
        })
        .unwrap_or_default()
}

/// The status line of a VPN card.
fn status_of(c: &Value) -> String {
    let state = match s(&c["state"]) {
        "activated" => t("Verbonden", "Connected"),
        "activating" => t("Verbinding maken…", "Connecting…"),
        "deactivating" => t("Verbinding verbreken…", "Disconnecting…"),
        _ => "",
    };
    let kind = if c["type"] == "wireguard" { "WireGuard".to_owned() } else { provider_label(s(&c["vpnType"])) };
    if state.is_empty() { kind } else { format!("{state} · {kind}") }
}

fn vpn(p: Page) -> PageFuture {
    Box::pin(async move {
        let (cr, pr) = (p.call("net.connections", json!({})), p.get("net.vpn.plugins", json!({})));
        let (cr, plugins) = (cr.await, pr.await);
        if !p.alive() {
            return Content::new();
        }
        let list = match cr {
            Ok(v) => v["connections"].as_array().cloned().unwrap_or_default(),
            Err(e) if e.code == "unknown_method" => return vec![common::notice(common::ct::unavailable_here()).widget().clone().upcast()],
            Err(e) => return vec![common::notice(&e.message).widget().clone().upcast()],
        };
        let vpns: Vec<Value> = list.into_iter().filter(|c| c["type"] == "vpn" || c["type"] == "wireguard").collect();
        let plugins = Rc::new(plugins.unwrap_or(json!({ "plugins": [], "wireguard": true })));
        let group = SettingsGroup::new("", "");
        let head = SettingsCard::new("vpn", t("VPN-verbindingen", "VPN connections"), "");
        let add = c::button(t("VPN toevoegen", "Add VPN"), Variant::Accent);
        add.set_widget_name("vpn-add");
        let import = c::button(t("Importeren uit bestand…", "Import from file…"), Variant::Standard);
        import.set_widget_name("vpn-import");
        head.set_control(&add);
        head.add_control(&import);
        group.add(head.widget());
        {
            let (pp, plugins) = (p.clone(), plugins.clone());
            add.connect_clicked(move |_| add_dialog(&pp, &plugins));
        }
        {
            let pp = p.clone();
            import.connect_clicked(move |_| import_file(&pp));
        }
        if vpns.is_empty() {
            let none = c::label(t("Er zijn nog geen VPN-verbindingen.", "There are no VPN connections yet."), "st-vpn-empty", true);
            none.set_widget_name("vpn-none");
            group.add(&none);
        }
        for (i, v) in vpns.iter().enumerate() {
            group.add(vpn_card(&p, v, i).widget());
        }
        {
            let pp = p.clone();
            p.on("net.changed", move |d| {
                if matches!(s(&d["what"]), "connections" | "active") {
                    pp.refresh();
                }
            });
        }
        let related = SettingsGroup::new(common::ct::related(), "");
        related.add(common::link_card(&p, "proxy").widget());
        related.add(common::link_card(&p, "network-advanced").widget());
        vec![group.widget().clone().upcast(), related.widget().clone().upcast()]
    })
}

/// One VPN: Windows' expander with Verbinding maken / verbreken, Geavanceerde opties, IP-instellingen, GNOME's "Make
/// available to other users" and Verwijderen.
fn vpn_card(p: &Page, v: &Value, i: usize) -> SettingsExpander {
    let uuid = s(&v["uuid"]).to_owned();
    let name = s(&v["id"]).to_owned();
    let exp = SettingsExpander::new("vpn", &name, &status_of(v));
    exp.header_button().set_widget_name(&format!("vpn-{i}"));
    let active = v["active"] == true;
    let connect = c::button(if active { t("Verbinding verbreken", "Disconnect") } else { t("Verbinding maken", "Connect") }, Variant::Standard);
    connect.set_widget_name(&format!("vpn-{i}-connect"));
    {
        let (pp, uuid) = (p.clone(), uuid.clone());
        connect.connect_clicked(move |b| {
            b.set_sensitive(false);
            let method = if active { "net.connection.deactivate" } else { "net.connection.activate" };
            let (wait, b, pp2) = (pp.run(method, json!({ "uuid": uuid })), b.clone(), pp.clone());
            pp.spawn(async move {
                let _ = wait.await;
                b.set_sensitive(true);
                pp2.refresh();
            });
        });
    }
    exp.header().set_control(&connect);
    // the rows that need the whole profile (net.connection.get when they are used)
    let adv = SettingsCard::new("", t("Geavanceerde opties", "Advanced options"), t("Server, aanmeldingsgegevens, certificaten en andere instellingen van de provider", "Server, sign-in info, certificates and other provider settings"));
    let adv_b = c::button(t("Bewerken", "Edit"), Variant::Standard);
    adv_b.set_widget_name(&format!("vpn-{i}-advanced"));
    adv.set_control(&adv_b);
    {
        let (pp, uuid) = (p.clone(), uuid.clone());
        adv_b.connect_clicked(move |_| with_profile(&pp, &uuid, |pp, uuid, conn| advanced_dialog(pp, uuid, conn)));
    }
    exp.add(adv.widget());
    let ip = SettingsCard::new("", t("IP-instellingen", "IP settings"), t("IPv4 en IPv6 van de tunnel", "IPv4 and IPv6 of the tunnel"));
    let ip_b = c::button(t("Bewerken", "Edit"), Variant::Standard);
    ip_b.set_widget_name(&format!("vpn-{i}-ip"));
    ip.set_control(&ip_b);
    {
        let (pp, uuid) = (p.clone(), uuid.clone());
        ip_b.connect_clicked(move |_| {
            with_profile(&pp, &uuid, |pp, uuid, conn| {
                let pp2 = pp.clone();
                ed::edit_ip(pp, uuid, &conn["settings"], Rc::new(move || pp2.refresh()));
            })
        });
    }
    exp.add(ip.widget());
    let shared = ToggleSwitch::new(v["shared"] == true);
    shared.switch().set_widget_name(&format!("st-vpn-{i}-permissions"));
    {
        let (pp, uuid, sw) = (p.clone(), uuid.clone(), shared.clone());
        shared.connect_toggled(move |on| {
            let sw = sw.clone();
            let perms = if on { json!([]) } else { ed::private_permissions() };
            ed::save_keys(&pp, &uuid, json!({ "connection": { "permissions": perms } }), move || sw.set_active(!on));
        });
    }
    let sc = SettingsCard::new("", t("Beschikbaar maken voor andere gebruikers", "Make available to other users"), "");
    sc.set_control(shared.widget());
    exp.add(sc.widget());
    let remove = SettingsCard::new("", t("Deze VPN-verbinding verwijderen", "Remove this VPN connection"), "");
    let rm = c::button(t("Verwijderen", "Remove"), Variant::Standard);
    rm.set_widget_name(&format!("vpn-{i}-remove"));
    remove.set_control(&rm);
    {
        let (pp, uuid, name) = (p.clone(), uuid.clone(), name.clone());
        rm.connect_clicked(move |_| {
            let (pp2, uuid) = (pp.clone(), uuid.clone());
            c::confirm(
                &pp.anchor(),
                &common::fill(t("VPN-verbinding “{name}” verwijderen?", "Remove the VPN connection “{name}”?"), &[("name", &name)]),
                t("De instellingen en opgeslagen aanmeldingsgegevens van deze verbinding worden verwijderd.", "The settings and saved sign-in info of this connection will be removed."),
                t("Verwijderen", "Remove"),
                true,
                move |ok| {
                    if ok {
                        let wait = pp2.run("net.connection.delete", json!({ "uuid": uuid }));
                        let pp3 = pp2.clone();
                        pp2.spawn(async move {
                            if wait.await.is_some() {
                                pp3.refresh();
                            }
                        });
                    }
                },
            );
        });
    }
    exp.add(remove.widget());
    exp
}

/// Run `f` with the profile of net.connection.get (the error toast when it is gone).
fn with_profile(p: &Page, uuid: &str, f: impl FnOnce(&Page, &str, &Value) + 'static) {
    let (pp, uuid, wait) = (p.clone(), uuid.to_owned(), p.run("net.connection.get", json!({ "uuid": uuid })));
    p.spawn(async move {
        if let Some(conn) = wait.await {
            f(&pp, &uuid, &conn);
        }
    });
}

/// "Importeren uit bestand…": a file dialog, then net.vpn.import (GNOME's Import from file).
fn import_file(p: &Page) {
    let dialog = gtk::FileDialog::new();
    dialog.set_title(t("Een VPN-configuratie importeren", "Import a VPN configuration"));
    dialog.set_modal(true);
    let filter = gtk::FileFilter::new();
    filter.set_name(Some(t("VPN-configuraties (*.ovpn, *.conf, *.pcf)", "VPN configurations (*.ovpn, *.conf, *.pcf)")));
    for pat in ["*.ovpn", "*.conf", "*.pcf"] {
        filter.add_pattern(pat);
    }
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    dialog.set_filters(Some(&filters));
    let pp = p.clone();
    dialog.open(Some(p.window().window()), gio::Cancellable::NONE, move |r| {
        let Ok(file) = r else { return };
        let Some(path) = file.path() else { return };
        let wait = pp.run_with("net.vpn.import", json!({ "path": path.to_string_lossy() }), Some(t("Kan de VPN-verbinding niet importeren", "Can’t import the VPN connection")), false);
        let pp2 = pp.clone();
        pp.spawn(async move {
            if let Some(v) = wait.await
                && v["dryRun"] != true
            {
                common::toast_success(&pp2.anchor(), t("De VPN-verbinding is geïmporteerd", "The VPN connection was imported"), 2500);
                pp2.refresh();
            }
        });
    });
}

/// The provider choices of the add dialog: the installed plugins this form can set up, and WireGuard.
pub fn provider_choices(plugins: &Value) -> Vec<String> {
    let mut out: Vec<String> = plugins["plugins"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| p["name"].as_str())
        .filter(|n| matches!(*n, "openvpn" | "pptp"))
        .map(str::to_owned)
        .collect();
    if plugins["wireguard"] != false {
        out.push("wireguard".to_owned());
    }
    out
}

/// Windows' "Een VPN-verbinding toevoegen" with GNOME's providers.
fn add_dialog(p: &Page, plugins: &Value) {
    let providers = provider_choices(plugins);
    if providers.is_empty() {
        c::alert(&p.anchor(), t("VPN toevoegen", "Add VPN"), t("Er is geen VPN-plug-in geïnstalleerd.", "No VPN plug-in is installed."), || {});
        return;
    }
    let form = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let provider = Dropdown::new(providers.iter().map(|n| DropdownOption::new(n, &provider_label(n))).collect(), providers.first().map(String::as_str));
    provider.widget().set_widget_name("vpn-new-provider");
    form.append(&c::field(t("VPN-provider", "VPN provider"), provider.widget(), None));
    let name = ed::entry("", "", "vpn-new-name");
    form.append(&c::field(t("Verbindingsnaam", "Connection name"), &name, None));
    let server = ed::entry("", "", "vpn-new-server");
    form.append(&c::field(t("Servernaam of -adres", "Server name or address"), &server, None));
    // OpenVPN / PPTP: Windows' sign-in fields
    let classic = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let auth = Dropdown::new(ed::options(OVPN_AUTH), Some("password"));
    auth.widget().set_widget_name("vpn-new-auth");
    let auth_field = c::field(t("Type aanmeldingsgegevens", "Type of sign-in info"), auth.widget(), None);
    classic.append(&auth_field);
    let (ca_box, ca) = ed::file_field(p, t("CA-certificaat", "CA certificate"), "", "vpn-new-ca");
    let (cert_box, cert) = ed::file_field(p, t("Gebruikerscertificaat", "User certificate"), "", "vpn-new-cert");
    let (key_box, key) = ed::file_field(p, t("Persoonlijke sleutel", "Private key"), "", "vpn-new-key");
    let key_pw = ed::password_entry("", "vpn-new-key-password");
    let key_pw_box = c::field(t("Wachtwoord voor persoonlijke sleutel", "Private key password"), &key_pw, None);
    let (sk_box, static_key) = ed::file_field(p, t("Statische sleutel", "Static key"), "", "vpn-new-static-key");
    let local_ip = ed::entry("", "", "vpn-new-local-ip");
    let remote_ip = ed::entry("", "", "vpn-new-remote-ip");
    let local_box = c::field(t("Lokaal IP-adres", "Local IP address"), &local_ip, None);
    let remote_box = c::field(t("Extern IP-adres", "Remote IP address"), &remote_ip, None);
    let user = ed::entry("", "", "vpn-new-user");
    let user_box = c::field(t("Gebruikersnaam (optioneel)", "User name (optional)"), &user, None);
    let password = ed::password_entry("", "vpn-new-password");
    let password_box = c::field(t("Wachtwoord (optioneel)", "Password (optional)"), &password, None);
    for w in [&ca_box, &cert_box, &key_box, &key_pw_box, &sk_box, &local_box, &remote_box, &user_box, &password_box] {
        classic.append(w);
    }
    let remember = c::checkbox(t("Mijn aanmeldingsgegevens onthouden", "Remember my sign-in info"));
    remember.set_active(true);
    remember.set_widget_name("vpn-new-remember");
    classic.append(&remember);
    form.append(&classic);
    // WireGuard: NetworkManager's own type
    let wg = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let wg_iface = ed::entry("wg0", "", "vpn-new-wg-iface");
    wg.append(&c::field(t("Interfacenaam", "Interface name"), &wg_iface, None));
    let wg_private = ed::password_entry("", "vpn-new-wg-private");
    wg.append(&c::field(t("Privésleutel", "Private key"), &wg_private, None));
    let wg_address = ed::entry("", "10.8.0.2/24", "vpn-new-wg-address");
    wg.append(&c::field(t("IP-adres van deze computer", "This computer’s IP address"), &wg_address, None));
    let wg_peer = ed::entry("", "", "vpn-new-wg-peer");
    wg.append(&c::field(t("Openbare sleutel van de server", "Server public key"), &wg_peer, None));
    let wg_allowed = ed::entry("0.0.0.0/0, ::/0", "", "vpn-new-wg-allowed");
    wg.append(&c::field(t("Toegestane IP-adressen", "Allowed IP addresses"), &wg_allowed, None));
    let wg_psk = ed::password_entry("", "vpn-new-wg-psk");
    wg.append(&c::field(t("Vooraf gedeelde sleutel (optioneel)", "Pre-shared key (optional)"), &wg_psk, None));
    form.append(&wg);
    form.append(&ed::show_characters(&[password.clone(), key_pw.clone(), wg_private.clone(), wg_psk.clone()]));
    let show = {
        let (provider, auth, auth_field, classic, wg) = (provider.clone(), auth.clone(), auth_field.clone(), classic.clone(), wg.clone());
        let boxes = (ca_box.clone(), cert_box.clone(), key_box.clone(), key_pw_box.clone(), sk_box.clone(), local_box.clone(), remote_box.clone(), user_box.clone(), password_box.clone());
        Rc::new(move || {
            let prov = provider.selected().unwrap_or_default();
            let a = auth.selected().unwrap_or_else(|| "password".into());
            let ovpn = prov == "openvpn";
            classic.set_visible(prov != "wireguard");
            wg.set_visible(prov == "wireguard");
            auth_field.set_visible(ovpn);
            let (ca_b, cert_b, key_b, kp_b, sk_b, l_b, r_b, u_b, p_b) = &boxes;
            let certs = ovpn && matches!(a.as_str(), "tls" | "password-tls");
            ca_b.set_visible(ovpn && a != "static-key");
            for b in [cert_b, key_b, kp_b] {
                b.set_visible(certs);
            }
            for b in [sk_b, l_b, r_b] {
                b.set_visible(ovpn && a == "static-key");
            }
            let pw = !ovpn || matches!(a.as_str(), "password" | "password-tls");
            u_b.set_visible(pw);
            p_b.set_visible(pw);
        })
    };
    show();
    {
        let show = show.clone();
        provider.connect_changed(move |_| show());
    }
    {
        let show = show.clone();
        auth.connect_changed(move |_| show());
    }
    let err = ed::error_label();
    form.append(&err);
    let pp = p.clone();
    Dialog::new(t("Een VPN-verbinding toevoegen", "Add a VPN connection"))
        .body(&ed::scroller(&form))
        .primary(t("Opslaan", "Save"))
        .width(520)
        .on_primary(move |d| {
            let f = NewVpn {
                provider: provider.selected().unwrap_or_default(),
                name: name.text().to_string(),
                server: server.text().to_string(),
                auth: auth.selected().unwrap_or_default(),
                user: user.text().to_string(),
                password: password.text().to_string(),
                remember: remember.is_active(),
                ca: ca.text().to_string(),
                cert: cert.text().to_string(),
                key: key.text().to_string(),
                key_password: key_pw.text().to_string(),
                static_key: static_key.text().to_string(),
                local_ip: local_ip.text().to_string(),
                remote_ip: remote_ip.text().to_string(),
                wg_iface: wg_iface.text().to_string(),
                wg_private: wg_private.text().to_string(),
                wg_address: wg_address.text().to_string(),
                wg_peer: wg_peer.text().to_string(),
                wg_allowed: wg_allowed.text().to_string(),
                wg_psk: wg_psk.text().to_string(),
            };
            match new_settings(&f) {
                Err(m) => ed::show_error(&err, m),
                Ok(settings) => {
                    let pp2 = pp.clone();
                    ed::dialog_call(d, &err, "net.connection.add", json!({ "settings": settings }), move |v| {
                        common::note_dry_run(&pp2.anchor(), &v);
                        pp2.refresh();
                    });
                }
            }
            false
        })
        .show(&p.anchor(), |_| {});
}

/// "Geavanceerde opties": the provider's settings of a VPN (OpenVPN, PPTP, another plugin's name/value rows) or of
/// a WireGuard connection, saved with net.connection.update.
fn advanced_dialog(p: &Page, uuid: &str, conn: &Value) {
    let settings = &conn["settings"];
    if settings["connection"]["type"] == "wireguard" {
        return wireguard_dialog(p, uuid, settings);
    }
    let service = s(&settings["vpn"]["service-type"]).to_owned();
    let data = settings["vpn"]["data"].clone();
    let form = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let name = ed::entry(s(&settings["connection"]["id"]), "", "vpn-adv-name");
    form.append(&c::field(t("Verbindingsnaam", "Connection name"), &name, None));
    form.append(&c::field(t("VPN-provider", "VPN provider"), &common::value_label(&provider_label(&service)), None));
    // the known fields
    type Getter = Box<dyn Fn() -> Value>;
    let mut getters: Vec<(&'static str, Getter)> = Vec::new();
    let ovpn = service == OPENVPN;
    let auth = Dropdown::new(ed::options(OVPN_AUTH), Some(s(&data["connection-type"])).filter(|a| !a.is_empty()).or(Some("password")));
    auth.widget().set_widget_name("vpn-adv-connection-type");
    if ovpn {
        form.append(&c::field(t("Type aanmeldingsgegevens", "Type of sign-in info"), auth.widget(), None));
    }
    for (key, kind, nl, en) in known_keys(&service) {
        let label = if kit::i18n::en() { *en } else { *nl };
        let wname = format!("vpn-adv-{key}");
        match kind {
            Field::Text => {
                let e = ed::entry(s(&data[*key]), "", &wname);
                form.append(&c::field(label, &e, None));
                getters.push((*key, Box::new(move || json!(e.text().to_string()))));
            }
            Field::File => {
                let (b, e) = ed::file_field(p, label, s(&data[*key]), &wname);
                form.append(&b);
                getters.push((*key, Box::new(move || json!(e.text().to_string()))));
            }
            Field::Yes | Field::NotYes => {
                let cb = c::checkbox(label);
                cb.set_active((data[*key] == "yes") == (*kind == Field::Yes));
                cb.set_widget_name(&wname);
                form.append(&cb);
                getters.push((*key, Box::new(move || json!(cb.is_active()))));
            }
        }
    }
    // the password (a secret: empty keeps the stored one) and how it is kept
    let uses_password = !ovpn || matches!(s(&data["connection-type"]), "" | "password" | "password-tls");
    let password = ed::password_entry(t("Ongewijzigd laten", "Leave unchanged"), "vpn-adv-password");
    let remember = c::checkbox(t("Mijn aanmeldingsgegevens onthouden", "Remember my sign-in info"));
    remember.set_active(s(&data["password-flags"]) != "2");
    remember.set_widget_name("vpn-adv-remember");
    if uses_password {
        form.append(&c::field(t("Wachtwoord", "Password"), &password, None));
        form.append(&remember);
    }
    // everything else NetworkManager keeps for the plugin
    form.append(&ed::section(t("Andere instellingen", "Other settings")));
    let rows = ed::Rows::new(vec![t("Naam", "Name"), t("Waarde", "Value")], other_keys(&service, &data), t("Instelling toevoegen", "Add setting"), "vpn-adv-other");
    form.append(&rows.root);
    let err = ed::error_label();
    form.append(&err);
    let (pp, uuid) = (p.clone(), uuid.to_owned());
    // the flag keys the form does not show stay as they are
    let kept_flags: Vec<(String, String)> = data
        .as_object()
        .map(|o| o.iter().filter(|(k, _)| k.ends_with("-flags") && *k != "password-flags").map(|(k, v)| (k.clone(), s(v).to_owned())).collect())
        .unwrap_or_default();
    Dialog::new(t("Geavanceerde opties", "Advanced options"))
        .body(&ed::scroller(&form))
        .primary(t("Opslaan", "Save"))
        .width(560)
        .on_primary(move |d| {
            let id = name.text().trim().to_owned();
            if id.is_empty() {
                ed::show_error(&err, t("Typ een verbindingsnaam.", "Type a connection name."));
                return false;
            }
            let fields: BTreeMap<String, Value> = getters.iter().map(|(k, g)| ((*k).to_owned(), g())).collect();
            let Some(mut data) = data_of(&service, &fields, &rows.values()) else {
                ed::show_error(&err, t("Elke andere instelling heeft een eigen naam nodig.", "Every other setting needs a name of its own."));
                return false;
            };
            if ovpn {
                data.insert("connection-type".into(), auth.selected().unwrap_or_else(|| "password".into()));
            }
            for (k, v) in &kept_flags {
                data.entry(k.clone()).or_insert_with(|| v.clone());
            }
            if uses_password {
                data.insert("password-flags".into(), if remember.is_active() { "1".into() } else { "2".into() });
            }
            let mut vpn = json!({ "data": data });
            let pw = password.text().to_string();
            if uses_password && !pw.is_empty() && remember.is_active() {
                vpn["secrets"] = json!({ "password": pw });
            }
            let pp2 = pp.clone();
            ed::dialog_call(d, &err, "net.connection.update", json!({ "uuid": uuid, "settings": { "connection": { "id": id }, "vpn": vpn } }), move |v| {
                common::note_dry_run(&pp2.anchor(), &v);
                pp2.refresh();
            });
            false
        })
        .show(&p.anchor(), |_| {});
}

/// The WireGuard peers of the dialog as NetworkManager's `wireguard.peers` (an empty pre-shared key keeps the
/// stored one: the host keeps it by the peer's public key); Err names the first bad row.
pub fn peers_of(rows: &[Vec<String>]) -> Result<Vec<Value>, String> {
    let mut out = Vec::new();
    for (n, r) in rows.iter().enumerate() {
        let get = |i: usize| r.get(i).map(|x| x.trim()).unwrap_or("");
        let bad = || common::fill(t("Peer {n}: controleer de openbare sleutel, het eindpunt en de toegestane IP-adressen.", "Peer {n}: check the public key, endpoint and allowed IP addresses."), &[("n", &(n + 1).to_string())]);
        if !wg_key_ok(get(0)) {
            return Err(bad());
        }
        let mut peer = json!({ "public-key": get(0) });
        if !get(1).is_empty() {
            peer["endpoint"] = json!(endpoint(get(1)).ok_or_else(bad)?);
        }
        peer["allowed-ips"] = json!(if get(2).is_empty() { Vec::new() } else { allowed_ips(get(2)).ok_or_else(bad)? });
        if !get(3).is_empty() {
            peer["persistent-keepalive"] = json!(get(3).parse::<u32>().ok().filter(|k| *k <= 65535).ok_or_else(bad)?);
        }
        if !get(4).is_empty() {
            if !wg_key_ok(get(4)) {
                return Err(bad());
            }
            peer["preshared-key"] = json!(get(4));
        }
        out.push(peer);
    }
    Ok(out)
}

/// GNOME's WireGuard page: name, interface, private key, listen port, fwmark, MTU, peer routes and the peers.
fn wireguard_dialog(p: &Page, uuid: &str, settings: &Value) {
    let w = &settings["wireguard"];
    let form = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let name = ed::entry(s(&settings["connection"]["id"]), "", "vpn-wg-name");
    form.append(&c::field(t("Verbindingsnaam", "Connection name"), &name, None));
    let iface = ed::entry(s(&settings["connection"]["interface-name"]), "", "vpn-wg-iface");
    form.append(&c::field(t("Interfacenaam", "Interface name"), &iface, None));
    let private = ed::password_entry(t("Ongewijzigd laten", "Leave unchanged"), "vpn-wg-private");
    form.append(&c::field(t("Privésleutel", "Private key"), &private, None));
    let number = |key: &str, label: &'static str, max: f64, hint: &'static str| {
        let n = c::number_box(0.0, max, 1.0, w[key].as_f64().unwrap_or(0.0), 0);
        n.set_widget_name(&format!("vpn-wg-{key}"));
        (c::field(label, &n, Some(hint)), n)
    };
    let (port_f, port) = number("listen-port", t("Luisterpoort", "Listen port"), 65535.0, t("0 is automatisch", "0 is automatic"));
    form.append(&port_f);
    let (mark_f, fwmark) = number("fwmark", t("Firewallmarkering (fwmark)", "Firewall mark (fwmark)"), 4_294_967_295.0, t("0 is uit", "0 is off"));
    form.append(&mark_f);
    let (mtu_f, mtu) = number("mtu", t("MTU", "MTU"), 65535.0, t("0 is automatisch", "0 is automatic"));
    form.append(&mtu_f);
    let routes = c::checkbox(t("Routes voor de toegestane IP-adressen van de peers toevoegen", "Add routes for the peers’ allowed IP addresses"));
    routes.set_active(w["peer-routes"] != false);
    routes.set_widget_name("vpn-wg-peer-routes");
    form.append(&routes);
    form.append(&ed::section(t("Peers", "Peers")));
    let values: Vec<Vec<String>> = w["peers"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|peer| {
            let allowed: Vec<&str> = peer["allowed-ips"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
            let keepalive = peer["persistent-keepalive"].as_u64().filter(|k| *k > 0).map(|k| k.to_string()).unwrap_or_default();
            vec![s(&peer["public-key"]).to_owned(), s(&peer["endpoint"]).to_owned(), allowed.join(", "), keepalive, String::new()]
        })
        .collect();
    let peers = ed::Rows::new(
        vec![t("Openbare sleutel", "Public key"), t("Eindpunt", "Endpoint"), t("Toegestane IP-adressen", "Allowed IP addresses"), t("Keepalive (s)", "Keepalive (s)"), t("Vooraf gedeelde sleutel", "Pre-shared key")],
        values,
        t("Peer toevoegen", "Add peer"),
        "vpn-wg-peer",
    );
    form.append(&peers.root);
    let err = ed::error_label();
    form.append(&err);
    let (pp, uuid) = (p.clone(), uuid.to_owned());
    Dialog::new(t("Geavanceerde opties", "Advanced options"))
        .body(&ed::scroller(&form))
        .primary(t("Opslaan", "Save"))
        .width(720)
        .on_primary(move |d| {
            let id = name.text().trim().to_owned();
            let ifn = iface.text().trim().to_owned();
            if id.is_empty() || ifn.is_empty() || ifn.len() > 15 {
                ed::show_error(&err, t("Typ een verbindingsnaam en een interfacenaam van maximaal 15 tekens.", "Type a connection name and an interface name of up to 15 characters."));
                return false;
            }
            let list = match peers_of(&peers.values()) {
                Ok(l) => l,
                Err(m) => {
                    ed::show_error(&err, &m);
                    return false;
                }
            };
            let mut wg = json!({
                "listen-port": port.value() as u64, "fwmark": fwmark.value() as u64, "mtu": mtu.value() as u64,
                "peer-routes": routes.is_active(), "peers": list,
            });
            let key = private.text().trim().to_owned();
            if !key.is_empty() {
                if !wg_key_ok(&key) {
                    ed::show_error(&err, t("Een WireGuard-sleutel bestaat uit 44 tekens en eindigt op =.", "A WireGuard key has 44 characters and ends in =."));
                    return false;
                }
                wg["private-key"] = json!(key);
            }
            let pp2 = pp.clone();
            ed::dialog_call(d, &err, "net.connection.update", json!({ "uuid": uuid, "settings": { "connection": { "id": id, "interface-name": ifn }, "wireguard": wg } }), move |v| {
                common::note_dry_run(&pp2.anchor(), &v);
                pp2.refresh();
            });
            false
        })
        .show(&p.anchor(), |_| {});
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "cHJpdmF0ZWtleWZvcnRoZWZha2Vjb25uZWN0aW9uMDA=";

    fn form(provider: &str) -> NewVpn {
        NewVpn { provider: provider.into(), name: "Kantoor".into(), server: "vpn.example.org".into(), remember: true, ..NewVpn::default() }
    }

    #[test]
    fn a_new_openvpn_or_pptp_profile_has_the_plugins_keys() {
        let mut f = form("openvpn");
        f.user = "robin".into();
        f.password = "geheim".into();
        f.ca = "/etc/ca.crt".into();
        let v = new_settings(&f).unwrap();
        assert_eq!(v["connection"], json!({ "id": "Kantoor", "type": "vpn", "autoconnect": false }));
        assert_eq!(v["vpn"]["service-type"], OPENVPN);
        assert_eq!(v["vpn"]["data"], json!({ "remote": "vpn.example.org", "connection-type": "password", "ca": "/etc/ca.crt", "username": "robin", "password-flags": "1" }));
        assert_eq!(v["vpn"]["secrets"], json!({ "password": "geheim" }));
        f.remember = false;
        let v = new_settings(&f).unwrap();
        assert_eq!(v["vpn"]["data"]["password-flags"], "2", "not remembered: asked each time");
        assert!(v["vpn"]["secrets"].is_null());
        let mut tls = form("openvpn");
        tls.auth = "tls".into();
        assert!(new_settings(&tls).is_err(), "TLS needs a certificate and a key");
        tls.cert = "/c".into();
        tls.key = "/k".into();
        tls.key_password = "pw".into();
        let v = new_settings(&tls).unwrap();
        assert_eq!(v["vpn"]["secrets"], json!({ "cert-pass": "pw" }));
        assert!(v["vpn"]["data"]["username"].is_null() && v["vpn"]["data"]["password-flags"].is_null());
        let mut sk = form("openvpn");
        sk.auth = "static-key".into();
        assert!(new_settings(&sk).is_err());
        sk.static_key = "/s.key".into();
        sk.local_ip = "10.0.0.2".into();
        sk.remote_ip = "10.0.0.1".into();
        assert_eq!(new_settings(&sk).unwrap()["vpn"]["data"]["static-key"], "/s.key");
        let mut pptp = form("pptp");
        pptp.user = "robin".into();
        let v = new_settings(&pptp).unwrap();
        assert_eq!(v["vpn"]["data"], json!({ "gateway": "vpn.example.org", "password-flags": "1", "user": "robin" }));
        assert!(new_settings(&NewVpn { name: String::new(), ..form("pptp") }).is_err());
        assert!(new_settings(&form("l2tp")).is_err());
    }

    #[test]
    fn a_new_wireguard_profile_is_networkmanagers_own_type() {
        let mut f = form("wireguard");
        f.wg_iface = "wg0".into();
        f.wg_private = KEY.into();
        f.wg_peer = KEY.into();
        f.wg_address = "10.8.0.2/24".into();
        let v = new_settings(&f).unwrap();
        assert_eq!((v["connection"]["type"].as_str(), v["connection"]["interface-name"].as_str()), (Some("wireguard"), Some("wg0")));
        assert_eq!(v["wireguard"]["peers"][0], json!({ "public-key": KEY, "endpoint": "vpn.example.org:51820", "allowed-ips": ["0.0.0.0/0", "::/0"] }));
        assert_eq!(v["ipv4"], json!({ "method": "manual", "address-data": [{ "address": "10.8.0.2", "prefix": 24 }] }));
        assert_eq!(v["ipv6"]["method"], "disabled");
        f.wg_address = "fd00::2/64".into();
        f.wg_psk = KEY.into();
        let v = new_settings(&f).unwrap();
        assert_eq!(v["ipv6"]["address-data"][0]["prefix"], 64);
        assert_eq!(v["wireguard"]["peers"][0]["preshared-key"], KEY);
        f.wg_private = "kort".into();
        assert!(new_settings(&f).is_err());
        assert!(wg_key_ok(KEY) && !wg_key_ok("AAAA") && !wg_key_ok(&KEY.replace('=', "A")));
        assert_eq!(endpoint("wg.example.org").as_deref(), Some("wg.example.org:51820"));
        assert_eq!(endpoint("wg.example.org:443").as_deref(), Some("wg.example.org:443"));
        assert_eq!(endpoint("fd00::1").as_deref(), Some("[fd00::1]:51820"));
        assert_eq!(endpoint("[fd00::1]:51821").as_deref(), Some("[fd00::1]:51821"));
        assert_eq!(cidr("10.0.0.1"), Some(("10.0.0.1".into(), 32, false)));
        assert_eq!(cidr("10.0.0.1/33"), None);
        assert_eq!(allowed_ips("0.0.0.0/0 ::/0"), Some(vec!["0.0.0.0/0".to_owned(), "::/0".to_owned()]));
        assert_eq!(allowed_ips("x"), None);
    }

    #[test]
    fn the_advanced_dialog_keeps_every_other_key() {
        let data = json!({ "remote": "vpn.example.org", "connection-type": "password", "password-flags": "1", "x-extra": "1", "ping": "10" });
        let mut others = other_keys(OPENVPN, &data);
        others.sort();
        assert_eq!(others, vec![vec!["ping".to_owned(), "10".to_owned()], vec!["x-extra".to_owned(), "1".to_owned()]]);
        let fields: BTreeMap<String, Value> = BTreeMap::from([
            ("remote".into(), json!(" vpn2.example.org ")),
            ("port".into(), json!("")),
            ("proto-tcp".into(), json!(true)),
            ("cipher".into(), json!("AES-256-GCM")),
        ]);
        let d = data_of(OPENVPN, &fields, &others).unwrap();
        assert_eq!(d.get("remote").map(String::as_str), Some("vpn2.example.org"));
        assert_eq!(d.get("proto-tcp").map(String::as_str), Some("yes"));
        assert!(!d.contains_key("port"), "an empty field leaves its key out");
        assert_eq!(d.get("x-extra").map(String::as_str), Some("1"));
        assert!(data_of(OPENVPN, &fields, &[vec!["remote".into(), "x".into()]]).is_none(), "a row may not repeat a field");
        assert!(data_of(OPENVPN, &fields, &[vec![String::new(), "x".into()]]).is_none());
        // PPTP's "toestaan" boxes are the refuse-* keys turned round
        let d = data_of(PPTP, &BTreeMap::from([("refuse-eap".into(), json!(false)), ("refuse-pap".into(), json!(true)), ("require-mppe".into(), json!(true))]), &[]).unwrap();
        assert_eq!(d.get("refuse-eap").map(String::as_str), Some("yes"));
        assert!(!d.contains_key("refuse-pap"));
        assert_eq!(d.get("require-mppe").map(String::as_str), Some("yes"));
    }

    #[test]
    fn peers_providers_and_labels() {
        let rows = vec![vec![KEY.to_owned(), "wg.example.org".to_owned(), "10.0.0.0/8, 192.168.0.0/16".to_owned(), "25".to_owned(), String::new()]];
        let p = peers_of(&rows).unwrap();
        assert_eq!(p[0], json!({ "public-key": KEY, "endpoint": "wg.example.org:51820", "allowed-ips": ["10.0.0.0/8", "192.168.0.0/16"], "persistent-keepalive": 25 }));
        assert!(p[0]["preshared-key"].is_null(), "an empty key keeps the stored one");
        assert!(peers_of(&[vec!["x".into()]]).is_err());
        let plugins = json!({ "plugins": [{ "name": "openvpn" }, { "name": "vpnc" }, { "name": "pptp" }], "wireguard": true });
        assert_eq!(provider_choices(&plugins), vec!["openvpn", "pptp", "wireguard"]);
        assert_eq!(provider_label(OPENVPN), "OpenVPN");
        assert_eq!(provider_label("org.freedesktop.NetworkManager.fortisslvpn"), ed::tr(&PROVIDERS[7]));
        assert_eq!(provider_label("org.example.unknown"), "unknown");
        assert_eq!(status_of(&json!({ "type": "wireguard", "state": "activated" })), format!("{} · WireGuard", t("Verbonden", "Connected")));
    }
}
