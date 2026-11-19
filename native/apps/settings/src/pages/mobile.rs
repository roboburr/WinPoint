//! Instellingen › Netwerk en internet › Mobiel netwerk (batch C, notes/claude/settings-parity.md §2): GNOME Settings'
//! Mobile Network panel (the switch and its status pages, SIM unlock with PIN or PUK, Mobile Data and Data Roaming,
//! Data Connection between modems, Primary SIM Slot, Network Mode, Network with its scan, Access Point Names with the
//! operator presets, SIM Lock, Modem Details) in the layout of Windows 11's Mobile (Cellular) page: the switch, the
//! data and roaming rows, "Instellingen van mobiele provider" (preferred network type, network selection, the access
//! points, the SIM PIN) and "Eigenschappen" with Kopiëren.
//!
//! ModemManager is `net.modem.*`; the data connection itself is NetworkManager's gsm profile of the SIM
//! (`net.connection.*`: autoconnect and activate for Mobiele gegevens, `gsm.home-only` for Roaming, apn, username,
//! password and the ppp sign-in flags for an access point).
use super::net_editor as ed;
use super::{Builder, Content, Page, PageFuture};
use crate::common;
use kit::controls::{self as c, Dialog, Dropdown, DropdownOption, ProgressRing, RadioGroup, SettingsCard, SettingsExpander, SettingsGroup, Severity, ToggleSwitch, Variant};
use kit::prelude::*;
use kit::t;
use std::rc::Rc;

pub const PAGES: &[(&str, Builder)] = &[("mobile", mobile)];
pub const CSS: &str = "";

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}

/// (auth, nl, en): an access point's "Type aanmeldingsgegevens" (ppp refuse flags, net.modem's apn_auth).
pub const APN_AUTH: &[(&str, &str, &str)] = &[
    ("auto", "Automatisch", "Automatic"),
    ("none", "Geen", "None"),
    ("pap", "PAP", "PAP"),
    ("chap", "CHAP", "CHAP"),
    ("mschapv2", "MS-CHAP v2", "MS-CHAP v2"),
];

/// (type, nl, en): GNOME's APN Type (Internet, or also the LTE attach bearer).
pub const APN_TYPE: &[(&str, &str, &str)] = &[("internet", "Internet", "Internet"), ("attach", "Internet en koppelen", "Internet and attach")];

/// The ppp group of a sign-in method (as the host's net/modem.rs apn_auth_ppp; `auto` removes the group).
pub fn auth_ppp(auth: &str) -> Value {
    let all = ["pap", "chap", "mschap", "mschapv2", "eap"];
    match auth {
        "none" => json!(all.iter().map(|m| (format!("refuse-{m}"), json!(true))).collect::<serde_json::Map<_, _>>()),
        m if all.contains(&m) => json!(all.iter().map(|x| (format!("refuse-{x}"), json!(*x != m))).collect::<serde_json::Map<_, _>>()),
        _ => Value::Null,
    }
}

/// A network mode pair as Windows' "Voorkeursnetwerktype" names it ("4G, 3G, 2G (voorkeur: 4G)").
pub fn mode_label(m: &Value) -> String {
    let names = |v: &Value| -> Vec<String> { v.as_array().into_iter().flatten().filter_map(Value::as_str).map(|n| if n == "any" { t("Alle", "All").to_owned() } else { n.to_uppercase() }).collect() };
    let allowed = names(&m["allowedNames"]).join(", ");
    let preferred = names(&m["preferredNames"]).join(", ");
    if preferred.is_empty() { allowed } else { common::fill(t("{allowed} (voorkeur: {preferred})", "{allowed} (preferred: {preferred})"), &[("allowed", &allowed), ("preferred", &preferred)]) }
}

/// The value of a mode pair in the drop-down.
fn mode_key(m: &Value) -> String {
    format!("{}:{}", m["allowed"].as_u64().unwrap_or(0), m["preferred"].as_u64().unwrap_or(0))
}

/// The registration as text.
pub fn registration_text(r: &str) -> &'static str {
    match r {
        "home" => t("Geregistreerd", "Registered"),
        "roaming" => t("Roaming", "Roaming"),
        "searching" => t("Zoeken", "Searching"),
        "denied" => t("Geweigerd", "Denied"),
        "emergency-only" => t("Alleen noodoproepen", "Emergency calls only"),
        "idle" => t("Niet geregistreerd", "Not registered"),
        _ => t("Onbekend", "Unknown"),
    }
}

/// The SIM or modem state that keeps the data rows away: (title, text) of the notice, or None.
pub fn blocker(m: &Value) -> Option<(&'static str, String)> {
    let retries = |k: &str| m["unlockRetries"][k].as_u64();
    match s(&m["unlockRequired"]) {
        "sim-pin" | "sim-pin2" => Some((
            t("De simkaart is vergrendeld", "The SIM card is locked"),
            match retries("sim-pin") {
                Some(n) => common::fill(t("Typ de pincode van de simkaart om hem te ontgrendelen. Je hebt nog {n} pogingen.", "Type the SIM PIN to unlock it. You have {n} attempts left."), &[("n", &n.to_string())]),
                None => t("Typ de pincode van de simkaart om hem te ontgrendelen.", "Type the SIM PIN to unlock it.").to_owned(),
            },
        )),
        "sim-puk" | "sim-puk2" => Some((
            t("De simkaart is geblokkeerd", "The SIM card is blocked"),
            match retries("sim-puk") {
                Some(n) => common::fill(t("Typ de PUK-code van je provider en een nieuwe pincode. Je hebt nog {n} pogingen.", "Type the PUK from your operator and a new PIN. You have {n} attempts left."), &[("n", &n.to_string())]),
                None => t("Typ de PUK-code van je provider en een nieuwe pincode.", "Type the PUK from your operator and a new PIN.").to_owned(),
            },
        )),
        _ if m["failedReason"] == "sim-missing" || m["sim"].is_null() => Some((t("Geen simkaart", "No SIM card"), t("Plaats een simkaart om mobiele gegevens te gebruiken.", "Insert a SIM card to use mobile data.").to_owned())),
        _ if m["failedReason"] == "sim-error" => Some((t("Simkaartfout", "SIM card error"), t("De simkaart werkt niet. Plaats hem opnieuw of neem contact op met je provider.", "The SIM card isn’t working. Insert it again or contact your operator.").to_owned())),
        _ => None,
    }
}

/// The status line of the switch ("Verbonden met KPN, 4G").
pub fn status_text(m: &Value, on: bool) -> String {
    if !on {
        return t("Uit", "Off").to_owned();
    }
    if let Some((title, _)) = blocker(m) {
        return title.to_owned();
    }
    let op = s(&m["operatorName"]);
    let kind = s(&m["networkType"]);
    let connected = m["connections"].as_array().is_some_and(|a| a.iter().any(|c| c["state"] == "activated"));
    match (connected, op.is_empty()) {
        (true, false) if !kind.is_empty() => common::fill(t("Verbonden met {op}, {kind}", "Connected to {op}, {kind}"), &[("op", op), ("kind", kind)]),
        (true, false) => common::fill(t("Verbonden met {op}", "Connected to {op}"), &[("op", op)]),
        (true, true) => t("Verbonden", "Connected").to_owned(),
        (false, false) => common::fill(t("Niet verbonden ({op})", "Not connected ({op})"), &[("op", op)]),
        (false, true) => t("Niet verbonden", "Not connected").to_owned(),
    }
}

/// The gsm profile Mobiele gegevens and Roaming act on: the active one, else the one that connects by itself, else
/// the first.
pub fn data_profile(m: &Value) -> Option<Value> {
    let list = m["connections"].as_array()?;
    list.iter().find(|c| c["active"] == true).or_else(|| list.iter().find(|c| c["autoconnect"] == true)).or_else(|| list.first()).cloned()
}

/// Windows' properties with GNOME's Modem Details.
pub fn properties(m: &Value) -> Vec<(String, String)> {
    let numbers: Vec<&str> = m["ownNumbers"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    let signal = m["signal"].as_u64().filter(|n| *n > 0).map(|n| format!("{n}%")).unwrap_or_default();
    vec![
        (t("Provider:", "Operator:").to_owned(), s(&m["operatorName"]).to_owned()),
        (t("Netwerktype:", "Network type:").to_owned(), s(&m["networkType"]).to_owned()),
        (t("Signaalsterkte:", "Signal strength:").to_owned(), signal),
        (t("Netwerkstatus:", "Network status:").to_owned(), registration_text(s(&m["registration"])).to_owned()),
        (t("Telefoonnummer:", "Phone number:").to_owned(), numbers.join(", ")),
        (t("Fabrikant:", "Manufacturer:").to_owned(), s(&m["manufacturer"]).to_owned()),
        (t("Model:", "Model:").to_owned(), s(&m["model"]).to_owned()),
        (t("Firmwareversie:", "Firmware version:").to_owned(), s(&m["firmware"]).to_owned()),
        (t("IMEI:", "IMEI:").to_owned(), s(&m["imei"]).to_owned()),
        (t("ICCID van simkaart:", "SIM ICCID:").to_owned(), s(&m["sim"]["iccid"]).to_owned()),
    ]
    .into_iter()
    .filter(|(_, v)| !v.is_empty())
    .collect()
}

/// The fields of the access point dialog.
#[derive(Clone, Debug, Default)]
pub struct Apn {
    pub name: String,
    pub apn: String,
    pub user: String,
    pub password: String,
    pub auth: String,
    pub kind: String,
}

/// The changes of an access point (a new profile has `connection` too): name, APN, sign-in, attach bearer; an
/// empty password keeps the stored one.
pub fn apn_changes(a: &Apn) -> Result<Value, &'static str> {
    let name = a.name.trim();
    let apn = a.apn.trim();
    if name.is_empty() {
        return Err(t("Typ een profielnaam.", "Type a profile name."));
    }
    if apn.is_empty() || apn.len() > 100 || apn.contains(char::is_whitespace) {
        return Err(t(
            "Typ de naam van het toegangspunt (APN), zonder spaties.",
            "Type the access point name (APN), without spaces.",
        ));
    }
    let attach = a.kind == "attach";
    let user = a.user.trim();
    let mut gsm = json!({
        "apn": apn,
        "username": if user.is_empty() { Value::Null } else { json!(user) },
        "initial-eps-bearer-configure": if attach { json!(true) } else { Value::Null },
        "initial-eps-bearer-apn": if attach { json!(apn) } else { Value::Null },
    });
    gsm["initial-eps-bearer-username"] = if attach && !user.is_empty() {
        json!(user)
    } else {
        Value::Null
    };
    let auth = auth_ppp(&a.auth);
    // NOAUTH governs authentication of the remote peer, independently of the client's PAP/CHAP method.
    // Mobile networks normally do not authenticate themselves to this client (NetworkManager's default).
    gsm["initial-eps-bearer-noauth"] = if attach { json!(true) } else { Value::Null };
    for method in ["pap", "chap", "eap", "mschap", "mschapv2"] {
        gsm[format!("initial-eps-bearer-refuse-{method}")] = if attach {
            auth.get(format!("refuse-{method}"))
                .cloned()
                .unwrap_or(Value::Null)
        } else {
            Value::Null
        };
    }
    if !attach {
        // Clear every attach-only credential when returning to an internet-only APN. Never retain an old password
        // that would make NetworkManager infer an attach bearer despite configure having been removed.
        gsm["initial-eps-bearer-password"] = Value::Null;
        gsm["initial-eps-bearer-password-flags"] = Value::Null;
        gsm["initial-eps-bearer-noauth"] = Value::Null;
    }
    if !a.password.is_empty() {
        gsm["password"] = json!(a.password);
        gsm["password-flags"] = json!(0);
        if attach {
            gsm["initial-eps-bearer-password"] = json!(a.password);
            gsm["initial-eps-bearer-password-flags"] = json!(0);
        }
    }
    Ok(json!({ "connection": { "id": name }, "gsm": gsm, "ppp": auth }))
}

/// A new gsm profile for the modem's SIM (GNOME: the APN's name, the SIM's identifier, roaming as now).
pub fn new_apn_settings(a: &Apn, sim_id: &str, home_only: bool, autoconnect: bool) -> Result<Value, &'static str> {
    let mut ch = apn_changes(a)?;
    ch["connection"]["type"] = json!("gsm");
    ch["connection"]["autoconnect"] = json!(autoconnect);
    if !sim_id.is_empty() {
        ch["gsm"]["sim-id"] = json!(sim_id);
    }
    ch["gsm"]["home-only"] = json!(home_only);
    // a new profile has nothing to remove: the nulls go
    for g in ["gsm", "ppp"] {
        if let Some(o) = ch[g].as_object_mut() {
            o.retain(|_, v| !v.is_null());
        }
    }
    if ch["ppp"].is_null() || ch["ppp"].as_object().is_some_and(|o| o.is_empty()) {
        ch.as_object_mut().map(|o| o.remove("ppp"));
    }
    ch["ipv4"] = json!({ "method": "auto" });
    ch["ipv6"] = json!({ "method": "auto" });
    Ok(ch)
}

/// The internet access points of net.modem.apns, as (label, Apn).
pub fn presets(apns: &Value) -> Vec<(String, Apn)> {
    let mut out = Vec::new();
    for provider in apns["providers"].as_array().into_iter().flatten() {
        for a in provider["apns"].as_array().into_iter().flatten() {
            let usage: Vec<&str> = a["usage"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
            if !usage.is_empty() && !usage.contains(&"internet") {
                continue;
            }
            let name = if s(&a["name"]).is_empty() { format!("{} ({})", s(&provider["name"]), s(&a["apn"])) } else { s(&a["name"]).to_owned() };
            let auth = match s(&a["auth"]) {
                "pap" => "pap",
                "chap" => "chap",
                _ => "auto",
            };
            out.push((name.clone(), Apn { name, apn: s(&a["apn"]).to_owned(), user: s(&a["username"]).to_owned(), password: s(&a["password"]).to_owned(), auth: auth.into(), kind: "internet".into() }));
        }
    }
    out
}

fn modem_args(m: &Value, extra: Value) -> Value {
    let mut a = extra;
    a["modem"] = m["path"].clone();
    a
}

fn mobile(p: Page) -> PageFuture {
    Box::pin(async move {
        let (mr, nr) = (p.call("net.modem.get", json!({})), p.get("system.network.get", json!({})));
        let (mr, net) = (mr.await, nr.await);
        if !p.alive() {
            return Content::new();
        }
        let state = match mr {
            Ok(v) => v,
            Err(e) if e.code == "unknown_method" => return vec![common::notice(common::ct::unavailable_here()).widget().clone().upcast()],
            Err(e) => return vec![common::notice(&e.message).widget().clone().upcast()],
        };
        let modems: Vec<Value> = state["modems"].as_array().cloned().unwrap_or_default();
        // follow ModemManager and NetworkManager in every state the page can show (no adapter, the switch off, a locked
        // or missing SIM, the full page): the page again when something it shows changed, as GNOME's WWAN panel
        // follows a modem coming or going, the WWAN switch and the SIM
        {
            let (pp, shape, pending) = (p.clone(), shape_of(&state), Rc::new(std::cell::Cell::new(false)));
            p.on("net.changed", move |d| {
                if !follows(&d["what"]) || pending.replace(true) {
                    return;
                }
                let (pp2, shape, pending) = (pp.clone(), shape.clone(), pending.clone());
                pp.timeout(500, move || {
                    let (wait, pp3) = (pp2.get("net.modem.get", json!({})), pp2.clone());
                    pp2.spawn(async move {
                        pending.set(false);
                        if let Some(v) = wait.await
                            && shape_of(&v) != shape
                        {
                            pp3.refresh();
                        }
                    });
                });
            });
        }
        {
            // flight mode turned on or off elsewhere: its notice (GNOME's Airplane Mode page)
            let (pp, airplane) = (p.clone(), net.as_ref().is_some_and(|n| n["airplane"] == true));
            p.on("system.network.changed", move |d| {
                if d.is_object() && (d["airplane"] == true) != airplane {
                    pp.refresh();
                }
            });
        }
        let mut out: Content = Vec::new();
        // GNOME's status pages: Airplane Mode On (with its button), No WWAN Adapter Found
        if net.as_ref().is_some_and(|n| n["airplane"] == true) {
            let bar = common::notice_full(Severity::Warning, t("Vliegtuigmodus staat aan", "Flight mode is on"), t("Schakel Vliegtuigmodus uit om het mobiele netwerk te gebruiken.", "Turn off flight mode to use the mobile network."));
            let off = c::button(t("Vliegtuigmodus uitschakelen", "Turn off flight mode"), Variant::Standard);
            off.set_widget_name("mobile-airplane-off");
            let pp = p.clone();
            off.connect_clicked(move |_| {
                let (wait, pp2) = (pp.run("system.network.setAirplane", json!({ "enabled": false })), pp.clone());
                pp.spawn(async move {
                    if wait.await.is_some() {
                        pp2.refresh();
                    }
                });
            });
            bar.set_action_widget(&off);
            bar.widget().set_widget_name("mobile-airplane");
            out.push(bar.widget().clone().upcast());
        }
        if state["available"] != true || modems.is_empty() {
            let bar = common::notice_full(Severity::Info, t("Er is geen mobiele breedbandadapter gevonden", "No mobile broadband adapter found"), t("Sluit een modem aan of plaats er een simkaart in om een mobiel netwerk te gebruiken.", "Connect a modem or insert a SIM card to use a mobile network."));
            bar.widget().set_widget_name("mobile-none");
            out.push(bar.widget().clone().upcast());
            return out;
        }
        let chosen = p.param("modem").and_then(|m| modems.iter().position(|x| x["path"] == json!(m))).unwrap_or(0);
        let m = modems[chosen].clone();
        let wwan = state["wwanEnabled"] == true;

        // ---- the switch, the modem switcher, the SIM notices
        let top = SettingsGroup::new("", "");
        let sw = ToggleSwitch::new(wwan);
        sw.switch().set_widget_name("mobile-switch");
        sw.set_sensitive(state["wwanHardwareEnabled"] != false);
        {
            let (pp, sw2) = (p.clone(), sw.clone());
            sw.connect_toggled(move |on| {
                let (wait, sw, pp2) = (pp.run("net.radios.set", json!({ "wwan": on })), sw2.clone(), pp.clone());
                pp.spawn(async move {
                    if wait.await.is_none() {
                        sw.set_active(!on);
                    } else {
                        pp2.refresh();
                    }
                });
            });
        }
        let card = SettingsCard::new("phone", t("Mobiel netwerk", "Mobile network"), &status_text(&m, wwan));
        card.set_control(sw.widget());
        top.add(card.widget());
        if modems.len() > 1 {
            // GNOME's per-modem switcher and its Data Connection
            let opts: Vec<DropdownOption> = modems.iter().map(|x| DropdownOption::new(s(&x["path"]), &format!("{} {}", s(&x["manufacturer"]), s(&x["model"])))).collect();
            let dd = Dropdown::new(opts, Some(s(&m["path"])));
            dd.widget().set_widget_name("mobile-modem");
            let pp = p.clone();
            dd.connect_changed(move |path| pp.go_with("mobile", json!({ "modem": path })));
            let c1 = SettingsCard::new("", t("Modem", "Modem"), "");
            c1.set_control(dd.widget());
            top.add(c1.widget());
            top.add(data_sim_card(&p, &modems).widget());
        }
        if !wwan {
            out.push(top.widget().clone().upcast());
            return out;
        }
        if let Some((title, text)) = blocker(&m) {
            let bar = common::notice_full(Severity::Warning, title, &text);
            bar.widget().set_widget_name("mobile-sim");
            if matches!(s(&m["unlockRequired"]), "sim-pin" | "sim-pin2" | "sim-puk" | "sim-puk2") {
                let unlock = c::button(t("Ontgrendelen", "Unlock"), Variant::Standard);
                unlock.set_widget_name("mobile-unlock");
                let (pp, m2) = (p.clone(), m.clone());
                unlock.connect_clicked(move |_| unlock_dialog(&pp, &m2));
                bar.set_action_widget(&unlock);
            }
            out.push(bar.widget().clone().upcast());
            if m["unlockRequired"] != "none" || m["sim"].is_null() {
                out.push(top.widget().clone().upcast());
                out.push(details(&p, &m).widget().clone().upcast());
                return out;
            }
        }

        // ---- Mobiele gegevens, Roaming, Primaire simsleuf
        let profile = data_profile(&m);
        let data_sw = ToggleSwitch::new(profile.as_ref().is_some_and(|c| c["active"] == true || c["autoconnect"] == true));
        data_sw.switch().set_widget_name("st-mobile-autoconnect");
        {
            let (pp, m2, profile, sw2) = (p.clone(), m.clone(), profile.clone(), data_sw.clone());
            data_sw.connect_toggled(move |on| set_data(&pp, &m2, profile.as_ref(), on, sw2.clone()));
        }
        let data = SettingsCard::new("", t("Mobiele gegevens", "Mobile data"), t("Toegang tot gegevens via het mobiele netwerk", "Access data using the mobile network"));
        data.set_control(data_sw.widget());
        top.add(data.widget());
        let roam = Dropdown::new(
            vec![DropdownOption::new("home", t("Niet roamen", "Don’t roam")), DropdownOption::new("roam", t("Roamen", "Roam"))],
            Some(if profile.as_ref().is_none_or(|c| c["homeOnly"] == true) { "home" } else { "roam" }),
        );
        roam.widget().set_widget_name("st-mobile-home-only");
        roam.widget().set_sensitive(profile.is_some());
        if let Some(c0) = &profile {
            let (pp, uuid, roam2) = (p.clone(), s(&c0["uuid"]).to_owned(), roam.clone());
            roam.connect_changed(move |v| {
                let home = v == "home";
                let r = roam2.clone();
                ed::save_keys(&pp, &uuid, json!({ "gsm": { "home-only": home } }), move || r.set_selected(Some(if home { "roam" } else { "home" })));
            });
        }
        let rc = SettingsCard::new("", t("Roaming", "Roaming"), t("Mobiele gegevens gebruiken buiten het netwerk van je provider", "Use mobile data outside your operator’s network"));
        rc.set_control(roam.widget());
        top.add(rc.widget());
        let slots = m["simSlots"].as_array().cloned().unwrap_or_default();
        if slots.len() > 1 {
            let opts: Vec<DropdownOption> = slots
                .iter()
                .map(|sl| {
                    let n = sl["slot"].as_u64().unwrap_or(0).to_string();
                    let op = s(&sl["sim"]["operatorName"]);
                    let base = common::fill(t("Simkaart {n}", "SIM {n}"), &[("n", &n)]);
                    let label = if sl["sim"].is_null() { format!("{base} ({})", t("leeg", "empty")) } else if op.is_empty() { base } else { format!("{base} ({op})") };
                    DropdownOption::new(&n, &label)
                })
                .collect();
            let current = m["primarySimSlot"].as_u64().unwrap_or(1).to_string();
            let dd = Dropdown::new(opts, Some(&current));
            dd.widget().set_widget_name("mobile-primary-slot");
            let (pp, m2, dd2, cur) = (p.clone(), m.clone(), dd.clone(), current.clone());
            dd.connect_changed(move |v| {
                let Ok(slot) = v.parse::<u32>() else { return };
                let (pp2, m3, dd3, cur2) = (pp.clone(), m2.clone(), dd2.clone(), cur.clone());
                c::confirm(
                    &pp.anchor(),
                    t("Een andere simsleuf gebruiken?", "Use another SIM slot?"),
                    t("De modem start opnieuw en het mobiele netwerk is even niet beschikbaar.", "The modem restarts and the mobile network is unavailable for a moment."),
                    t("Wijzigen", "Change"),
                    false,
                    move |ok| {
                        if !ok {
                            dd3.set_selected(Some(&cur2));
                            return;
                        }
                        let wait = pp2.run("net.modem.setPrimarySlot", modem_args(&m3, json!({ "slot": slot })));
                        let pp3 = pp2.clone();
                        pp2.spawn(async move {
                            if wait.await.is_none() {
                                dd3.set_selected(Some(&cur2));
                            } else {
                                pp3.refresh();
                            }
                        });
                    },
                );
            });
            let sc = SettingsCard::new("", t("Primaire simsleuf", "Primary SIM slot"), "");
            sc.set_control(dd.widget());
            top.add(sc.widget());
        }
        out.push(top.widget().clone().upcast());

        // ---- Instellingen van mobiele provider
        let provider = SettingsGroup::new(t("Instellingen van mobiele provider", "Mobile operator settings"), "");
        let supported = m["supportedModes"].as_array().cloned().unwrap_or_default();
        if supported.len() > 1 {
            let opts: Vec<DropdownOption> = supported.iter().map(|x| DropdownOption::new(&mode_key(x), &mode_label(x))).collect();
            let current = mode_key(&m["currentModes"]);
            let dd = Dropdown::new(opts, Some(&current));
            dd.widget().set_widget_name("mobile-modes");
            let (pp, m2, dd2, cur) = (p.clone(), m.clone(), dd.clone(), Rc::new(std::cell::RefCell::new(current)));
            dd.connect_changed(move |v| {
                let mut it = v.split(':').map(|n| n.parse::<u64>().unwrap_or(0));
                let (a, pr) = (it.next().unwrap_or(0), it.next().unwrap_or(0));
                let before = cur.borrow().clone();
                *cur.borrow_mut() = v.to_owned();
                let (wait, dd3, cur2) = (pp.run("net.modem.setModes", modem_args(&m2, json!({ "allowed": a, "preferred": pr }))), dd2.clone(), cur.clone());
                pp.spawn(async move {
                    if wait.await.is_none() {
                        dd3.set_selected(Some(&before));
                        *cur2.borrow_mut() = before;
                    }
                });
            });
            let mc = SettingsCard::new("", t("Voorkeursnetwerktype", "Preferred network type"), "");
            mc.set_control(dd.widget());
            provider.add(mc.widget());
        }
        if m["is3gpp"] == true {
            let op = s(&m["operatorName"]);
            let nc = SettingsCard::new("", t("Netwerk selecteren", "Select network"), if op.is_empty() { t("Automatisch", "Automatic") } else { op });
            let b = c::button(t("Kiezen", "Choose"), Variant::Standard);
            b.set_widget_name("mobile-network-choose");
            let (pp, m2) = (p.clone(), m.clone());
            b.connect_clicked(move |_| network_dialog(&pp, &m2));
            nc.set_control(&b);
            provider.add(nc.widget());
        }
        provider.add(apn_expander(&p, &m).widget());
        provider.add(sim_lock_card(&p, &m).widget());
        out.push(provider.widget().clone().upcast());
        out.push(details(&p, &m).widget().clone().upcast());
        let related = SettingsGroup::new(common::ct::related(), "");
        related.add(common::link_card(&p, "flightmode").widget());
        out.push(related.widget().clone().upcast());

        out
    })
}

/// The net.changed kinds the page reads ModemManager again after: the modem itself (state, SIM, a modem plugged in
/// or out), NetworkManager's root object (state: WwanEnabled), its devices and profiles, WinPoint's own switch.
pub fn follows(what: &Value) -> bool {
    matches!(what.as_str(), Some("modem" | "state" | "connections" | "active" | "radios" | "devices"))
}

/// What the page is built from (not the signal, which changes all the time).
fn shape_of(state: &Value) -> String {
    let mut v = state.clone();
    for m in v["modems"].as_array_mut().into_iter().flatten() {
        if let Some(o) = m.as_object_mut() {
            o.remove("signal");
        }
    }
    v.to_string()
}

/// GNOME's Data Connection: the SIM whose gsm profile connects by itself.
fn data_sim_card(p: &Page, modems: &[Value]) -> SettingsCard {
    let opts: Vec<DropdownOption> = modems
        .iter()
        .map(|x| {
            let op = s(&x["sim"]["operatorName"]);
            DropdownOption::new(s(&x["path"]), if op.is_empty() { s(&x["model"]) } else { op })
        })
        .collect();
    let current = modems.iter().find(|x| data_profile(x).is_some_and(|c| c["autoconnect"] == true)).map(|x| s(&x["path"]).to_owned()).unwrap_or_default();
    let dd = Dropdown::new(opts, Some(&current));
    dd.widget().set_widget_name("mobile-data-sim");
    let (pp, list) = (p.clone(), modems.to_vec());
    dd.connect_changed(move |path| {
        for x in &list {
            if let Some(c0) = data_profile(x) {
                let on = x["path"] == json!(path);
                ed::save_keys(&pp, s(&c0["uuid"]), json!({ "connection": { "autoconnect": on } }), || {});
            }
        }
    });
    let card = SettingsCard::new("", t("Deze simkaart gebruiken voor mobiele gegevens", "Use this SIM for mobile data"), "");
    card.set_control(dd.widget());
    card
}

/// Mobiele gegevens (GNOME's Mobile Data): the profile connects by itself and now, or neither; without a profile, the
/// operator's first internet access point becomes one (or the access point dialog opens).
fn set_data(p: &Page, m: &Value, profile: Option<&Value>, on: bool, sw: ToggleSwitch) {
    let iface = m["iface"].clone();
    match profile {
        Some(c0) => {
            let uuid = s(&c0["uuid"]).to_owned();
            let (pp, active) = (p.clone(), c0["active"] == true);
            let wait = p.run("net.connection.update", json!({ "uuid": uuid, "settings": { "connection": { "autoconnect": on } }, "reapply": false }));
            p.spawn(async move {
                if wait.await.is_none() {
                    sw.set_active(!on);
                    return;
                }
                let next = if on {
                    let mut a = json!({ "uuid": uuid });
                    if iface.is_string() {
                        a["device"] = iface;
                    }
                    Some(pp.run("net.connection.activate", a))
                } else if active {
                    Some(pp.run("net.connection.deactivate", json!({ "uuid": uuid })))
                } else {
                    None
                };
                if let Some(w) = next {
                    let _ = w.await;
                }
                pp.refresh();
            });
        }
        None if on => {
            let (pp, m2) = (p.clone(), m.clone());
            let wait = p.get("net.modem.apns", json!({ "mcc": m["sim"]["mcc"], "mnc": m["sim"]["mnc"] }));
            p.spawn(async move {
                let first = wait.await.map(|v| presets(&v)).and_then(|list| list.into_iter().next());
                match first.and_then(|(_, a)| new_apn_settings(&a, s(&m2["sim"]["iccid"]), true, true).ok()) {
                    Some(settings) => {
                        let ok = pp.run("net.connection.add", json!({ "settings": settings })).await.is_some();
                        if !ok {
                            sw.set_active(false);
                        }
                        pp.refresh();
                    }
                    None => {
                        sw.set_active(false);
                        apn_dialog(&pp, &m2, None);
                    }
                }
            });
        }
        None => {}
    }
}

/// GNOME's SIM Unlock: the PIN, or after too many tries the PUK and a new PIN.
fn unlock_dialog(p: &Page, m: &Value) {
    let puk_mode = s(&m["unlockRequired"]).contains("puk");
    let form = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let puk = ed::password_entry("", "mobile-unlock-puk");
    let pin = ed::password_entry("", "mobile-unlock-pin");
    let again = ed::password_entry("", "mobile-unlock-again");
    for e in [&puk, &pin, &again] {
        e.set_input_purpose(gtk::InputPurpose::Pin);
    }
    if puk_mode {
        form.append(&c::field(t("PUK-code", "PUK"), &puk, None));
        form.append(&c::field(t("Nieuwe pincode", "New PIN"), &pin, None));
        form.append(&c::field(t("Nieuwe pincode bevestigen", "Confirm new PIN"), &again, None));
    } else {
        form.append(&c::field(t("Pincode van simkaart", "SIM PIN"), &pin, None));
    }
    if let Some((_, text)) = blocker(m) {
        form.append(&c::label(&text, "st-spec-key", true));
    }
    let err = ed::error_label();
    form.append(&err);
    let (pp, m) = (p.clone(), m.clone());
    Dialog::new(t("Simkaart ontgrendelen", "Unlock SIM card"))
        .body(&form)
        .primary(t("Ontgrendelen", "Unlock"))
        .width(420)
        .on_primary(move |d| {
            let digits = |e: &gtk::Entry, min: usize, max: usize| {
                let v = e.text().to_string();
                ((min..=max).contains(&v.len()) && v.bytes().all(|b| b.is_ascii_digit())).then_some(v)
            };
            let args = if puk_mode {
                let (Some(k), Some(n)) = (digits(&puk, 8, 8), digits(&pin, 4, 8)) else {
                    ed::show_error(&err, t("Een PUK-code bestaat uit 8 cijfers, een pincode uit 4 tot 8 cijfers.", "A PUK has 8 digits, a PIN 4 to 8 digits."));
                    return false;
                };
                if again.text() != pin.text() {
                    ed::show_error(&err, t("De pincodes komen niet overeen.", "The PINs don’t match."));
                    return false;
                }
                modem_args(&m, json!({ "puk": k, "newPin": n }))
            } else {
                let Some(n) = digits(&pin, 4, 8) else {
                    ed::show_error(&err, t("Een pincode bestaat uit 4 tot 8 cijfers.", "A PIN has 4 to 8 digits."));
                    return false;
                };
                modem_args(&m, json!({ "pin": n }))
            };
            let pp2 = pp.clone();
            ed::dialog_call(d, &err, "net.modem.unlock", args, move |v| {
                common::note_dry_run(&pp2.anchor(), &v);
                pp2.refresh();
            });
            false
        })
        .show(&p.anchor(), |_| {});
}

/// GNOME's Network dialog: Automatisch, or a network of the scan.
fn network_dialog(p: &Page, m: &Value) {
    let form = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let list = common::slot();
    list.set_visible(true);
    let ring = ProgressRing::new(32, None);
    list.append(ring.widget());
    list.append(&c::label(t("Zoeken naar netwerken… Dit kan een minuut duren.", "Searching for networks… This can take a minute."), "st-spec-key", true));
    form.append(&list);
    let err = ed::error_label();
    form.append(&err);
    let choice: Rc<std::cell::RefCell<String>> = Rc::new(std::cell::RefCell::new(String::new()));
    let scan = p.call("net.modem.scan", modem_args(m, json!({})));
    let (list2, choice2, err2) = (list.clone(), choice.clone(), err.clone());
    let current = s(&m["operatorCode"]).to_owned();
    p.spawn(async move {
        match scan.await {
            Ok(v) => {
                let mut opts: Vec<(String, String)> = vec![(String::new(), t("Automatisch", "Automatic").to_owned())];
                for n in v["networks"].as_array().into_iter().flatten() {
                    if n["status"] == "forbidden" {
                        continue;
                    }
                    let label = if s(&n["networkType"]).is_empty() { s(&n["name"]).to_owned() } else { format!("{} ({})", s(&n["name"]), s(&n["networkType"])) };
                    opts.push((s(&n["code"]).to_owned(), label));
                }
                let refs: Vec<(&str, &str)> = opts.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
                let selected = if opts.iter().any(|(k, _)| *k == current) && !current.is_empty() { current.clone() } else { String::new() };
                *choice2.borrow_mut() = selected.clone();
                let rg = RadioGroup::new(&refs, Some(&selected), None, false);
                rg.widget().set_widget_name("mobile-networks");
                let ch = choice2.clone();
                rg.connect_changed(move |v| *ch.borrow_mut() = v.to_owned());
                common::set_slot(&list2, &[rg.widget().clone().upcast()]);
            }
            Err(e) => {
                common::set_slot(&list2, &[]);
                ed::show_error(&err2, &e.message);
            }
        }
    });
    let (pp, m) = (p.clone(), m.clone());
    Dialog::new(t("Netwerk selecteren", "Select network"))
        .body(&form)
        .primary(t("Selecteren", "Select"))
        .width(440)
        .on_primary(move |d| {
            let op = choice.borrow().clone();
            let pp2 = pp.clone();
            ed::dialog_call(d, &err, "net.modem.register", modem_args(&m, json!({ "operator": op })), move |v| {
                common::note_dry_run(&pp2.anchor(), &v);
                pp2.refresh();
            });
            false
        })
        .show(&p.anchor(), |_| {});
}

/// Windows' "Toegangspunt": the SIM's access points (GNOME's Access Point Names), the one in use first.
fn apn_expander(p: &Page, m: &Value) -> SettingsExpander {
    let list = m["connections"].as_array().cloned().unwrap_or_default();
    let in_use = data_profile(m);
    let exp = SettingsExpander::new("", t("Toegangspunt", "Access point"), in_use.as_ref().map(|c| s(&c["id"])).unwrap_or(t("Geen", "None")));
    exp.header_button().set_widget_name("mobile-apn");
    let add = c::button(t("APN toevoegen", "Add APN"), Variant::Standard);
    add.set_widget_name("mobile-apn-add");
    {
        let (pp, m2) = (p.clone(), m.clone());
        add.connect_clicked(move |_| apn_dialog(&pp, &m2, None));
    }
    exp.header().set_control(&add);
    for (i, c0) in list.iter().enumerate() {
        let used = in_use.as_ref().is_some_and(|u| u["uuid"] == c0["uuid"]);
        let text = if used { format!("{} · {}", s(&c0["apn"]), t("In gebruik", "In use")) } else { s(&c0["apn"]).to_owned() };
        let card = SettingsCard::new("", s(&c0["id"]), &text);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        if !used {
            let apply = c::button(t("Toepassen", "Apply"), Variant::Standard);
            apply.set_widget_name(&format!("mobile-apn-{i}-apply"));
            let (pp, all, uuid) = (p.clone(), list.clone(), s(&c0["uuid"]).to_owned());
            apply.connect_clicked(move |_| {
                // the chosen one connects by itself, the others no longer (GNOME's default APN)
                for other in &all {
                    let on = other["uuid"] == json!(uuid);
                    if on || other["autoconnect"] == true {
                        ed::save_keys(&pp, s(&other["uuid"]), json!({ "connection": { "autoconnect": on } }), || {});
                    }
                }
                pp.refresh();
            });
            row.append(&apply);
        }
        let edit = c::button(t("Bewerken", "Edit"), Variant::Standard);
        edit.set_widget_name(&format!("mobile-apn-{i}-edit"));
        {
            let (pp, m2, c1) = (p.clone(), m.clone(), c0.clone());
            edit.connect_clicked(move |_| apn_dialog(&pp, &m2, Some(&c1)));
        }
        row.append(&edit);
        let rm = c::button(t("Verwijderen", "Remove"), Variant::Standard);
        rm.set_widget_name(&format!("mobile-apn-{i}-remove"));
        {
            let (pp, uuid, name) = (p.clone(), s(&c0["uuid"]).to_owned(), s(&c0["id"]).to_owned());
            rm.connect_clicked(move |_| {
                let (pp2, uuid) = (pp.clone(), uuid.clone());
                c::confirm(&pp.anchor(), &common::fill(t("Toegangspunt “{name}” verwijderen?", "Remove the access point “{name}”?"), &[("name", &name)]), "", t("Verwijderen", "Remove"), true, move |ok| {
                    if ok {
                        let (wait, pp3) = (pp2.run("net.connection.delete", json!({ "uuid": uuid })), pp2.clone());
                        pp2.spawn(async move {
                            if wait.await.is_some() {
                                pp3.refresh();
                            }
                        });
                    }
                });
            });
        }
        row.append(&rm);
        card.set_control(&row);
        exp.add(card.widget());
    }
    exp
}

/// Windows' "APN toevoegen" / "APN bewerken" with GNOME's presets of the operator (serviceproviders.xml).
fn apn_dialog(p: &Page, m: &Value, existing: Option<&Value>) {
    let form = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let preset_slot = common::slot();
    form.append(&preset_slot);
    let get = |k: &str| existing.map(|c| s(&c[k]).to_owned()).unwrap_or_default();
    let name = ed::entry(&existing.map(|c| s(&c["id"]).to_owned()).unwrap_or_default(), "", "mobile-apn-name");
    form.append(&c::field(t("Profielnaam", "Profile name"), &name, None));
    let apn = ed::entry(&get("apn"), "", "st-mobile-apn");
    form.append(&c::field(t("APN", "APN"), &apn, None));
    let user = ed::entry(&get("username"), "", "st-mobile-username");
    form.append(&c::field(t("Gebruikersnaam", "User name"), &user, None));
    let password = ed::password_entry(if existing.is_some() { t("Ongewijzigd laten", "Leave unchanged") } else { "" }, "st-mobile-password");
    form.append(&c::field(t("Wachtwoord", "Password"), &password, None));
    let auth_now = existing.map(|c| s(&c["auth"]).to_owned()).filter(|a| APN_AUTH.iter().any(|r| r.0 == a)).unwrap_or_else(|| "auto".into());
    let auth = Dropdown::new(ed::options(APN_AUTH), Some(&auth_now));
    auth.widget().set_widget_name("mobile-apn-auth");
    form.append(&c::field(t("Type aanmeldingsgegevens", "Type of sign-in info"), auth.widget(), None));
    let kind = Dropdown::new(ed::options(APN_TYPE), Some(if existing.is_some_and(|c| c["attach"] == true) { "attach" } else { "internet" }));
    kind.widget().set_widget_name("mobile-apn-type");
    form.append(&c::field(t("APN-type", "APN type"), kind.widget(), None));
    // the operator's presets fill the fields
    if existing.is_none() && !s(&m["sim"]["mcc"]).is_empty() {
        let wait = p.get("net.modem.apns", json!({ "mcc": m["sim"]["mcc"], "mnc": m["sim"]["mnc"] }));
        let (slot, name, apn, user, password, auth) = (preset_slot.clone(), name.clone(), apn.clone(), user.clone(), password.clone(), auth.clone());
        p.spawn(async move {
            let list = wait.await.map(|v| presets(&v)).unwrap_or_default();
            if list.is_empty() {
                return;
            }
            let mut opts = vec![DropdownOption::new("", t("Zelf invullen", "Fill in myself"))];
            opts.extend(list.iter().enumerate().map(|(i, (label, _))| DropdownOption::new(&i.to_string(), label)));
            let dd = Dropdown::new(opts, Some(""));
            dd.widget().set_widget_name("mobile-apn-preset");
            dd.connect_changed(move |v| {
                if let Some((_, a)) = v.parse::<usize>().ok().and_then(|i| list.get(i)) {
                    name.set_text(&a.name);
                    apn.set_text(&a.apn);
                    user.set_text(&a.user);
                    password.set_text(&a.password);
                    auth.set_selected(Some(&a.auth));
                }
            });
            common::set_slot(&slot, &[c::field(t("Instellingen van je provider", "Your operator’s settings"), dd.widget(), None).upcast()]);
        });
    }
    let err = ed::error_label();
    form.append(&err);
    let (pp, m, existing) = (p.clone(), m.clone(), existing.cloned());
    let title = if existing.is_some() { t("APN bewerken", "Edit APN") } else { t("APN toevoegen", "Add APN") };
    Dialog::new(title)
        .body(&form)
        .primary(t("Opslaan", "Save"))
        .width(460)
        .on_primary(move |d| {
            let a = Apn {
                name: name.text().to_string(),
                apn: apn.text().to_string(),
                user: user.text().to_string(),
                password: password.text().to_string(),
                auth: auth.selected().unwrap_or_else(|| "auto".into()),
                kind: kind.selected().unwrap_or_else(|| "internet".into()),
            };
            let pp2 = pp.clone();
            let done = move |v: Value| {
                common::note_dry_run(&pp2.anchor(), &v);
                pp2.refresh();
            };
            match &existing {
                Some(c0) => match apn_changes(&a) {
                    Ok(ch) => ed::dialog_call(d, &err, "net.connection.update", json!({ "uuid": c0["uuid"], "settings": ch }), done),
                    Err(e) => ed::show_error(&err, e),
                },
                None => {
                    // the first access point of the SIM is the one it uses
                    let first = m["connections"].as_array().is_none_or(|l| l.is_empty());
                    let home = data_profile(&m).is_none_or(|c| c["homeOnly"] == true);
                    match new_apn_settings(&a, s(&m["sim"]["iccid"]), home, first) {
                        Ok(settings) => ed::dialog_call(d, &err, "net.connection.add", json!({ "settings": settings }), done),
                        Err(e) => ed::show_error(&err, e),
                    }
                }
            }
            false
        })
        .show(&p.anchor(), |_| {});
}

/// Windows' "Pincode van simkaart gebruiken" and "Pincode van simkaart wijzigen" (GNOME's SIM Lock).
fn sim_lock_card(p: &Page, m: &Value) -> SettingsExpander {
    let exp = SettingsExpander::new("lock", t("Beveiliging van simkaart", "SIM security"), "");
    let on = m["pinEnabled"] == true;
    let sw = ToggleSwitch::new(on);
    sw.switch().set_widget_name("mobile-sim-pin");
    {
        let (pp, m2, sw2) = (p.clone(), m.clone(), sw.clone());
        sw.connect_toggled(move |enable| pin_dialog(&pp, &m2, Some(enable), sw2.clone()));
    }
    let use_pin = SettingsCard::new("", t("Pincode van simkaart gebruiken", "Use SIM PIN"), t("De pincode is nodig als je de pc opstart of de simkaart plaatst", "The PIN is needed when you start the PC or insert the SIM card"));
    use_pin.set_control(sw.widget());
    exp.add(use_pin.widget());
    let change = SettingsCard::new("", t("Pincode van simkaart wijzigen", "Change SIM PIN"), "");
    let b = c::button(t("Wijzigen", "Change"), Variant::Standard);
    b.set_widget_name("mobile-sim-pin-change");
    b.set_sensitive(on);
    {
        let (pp, m2, sw2) = (p.clone(), m.clone(), sw.clone());
        b.connect_clicked(move |_| pin_dialog(&pp, &m2, None, sw2.clone()));
    }
    change.set_control(&b);
    exp.add(change.widget());
    exp
}

/// The PIN dialog: `enable` Some(on) turns the SIM lock on or off (the current PIN), None changes the PIN.
fn pin_dialog(p: &Page, m: &Value, enable: Option<bool>, sw: ToggleSwitch) {
    let form = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let current = ed::password_entry("", "mobile-pin-current");
    let new = ed::password_entry("", "mobile-pin-new");
    let again = ed::password_entry("", "mobile-pin-again");
    form.append(&c::field(t("Huidige pincode", "Current PIN"), &current, None));
    if enable.is_none() {
        form.append(&c::field(t("Nieuwe pincode", "New PIN"), &new, None));
        form.append(&c::field(t("Nieuwe pincode bevestigen", "Confirm new PIN"), &again, None));
    }
    let err = ed::error_label();
    form.append(&err);
    let (pp, m, sw2) = (p.clone(), m.clone(), sw.clone());
    let title = match enable {
        Some(true) => t("Pincode van simkaart inschakelen", "Turn on SIM PIN"),
        Some(false) => t("Pincode van simkaart uitschakelen", "Turn off SIM PIN"),
        None => t("Pincode van simkaart wijzigen", "Change SIM PIN"),
    };
    Dialog::new(title)
        .body(&form)
        .primary(t("OK", "OK"))
        .width(420)
        .on_primary(move |d| {
            let pin = |e: &gtk::Entry| {
                let v = e.text().to_string();
                ((4..=8).contains(&v.len()) && v.bytes().all(|b| b.is_ascii_digit())).then_some(v)
            };
            let Some(old) = pin(&current) else {
                ed::show_error(&err, t("Een pincode bestaat uit 4 tot 8 cijfers.", "A PIN has 4 to 8 digits."));
                return false;
            };
            let (method, args) = match enable {
                Some(on) => ("net.modem.setPin", modem_args(&m, json!({ "enabled": on, "pin": old }))),
                None => {
                    let Some(n) = pin(&new) else {
                        ed::show_error(&err, t("Een pincode bestaat uit 4 tot 8 cijfers.", "A PIN has 4 to 8 digits."));
                        return false;
                    };
                    if again.text() != new.text() {
                        ed::show_error(&err, t("De pincodes komen niet overeen.", "The PINs don’t match."));
                        return false;
                    }
                    ("net.modem.changePin", modem_args(&m, json!({ "old": old, "new": n })))
                }
            };
            let pp2 = pp.clone();
            ed::dialog_call(d, &err, method, args, move |v| {
                common::note_dry_run(&pp2.anchor(), &v);
                pp2.refresh();
            });
            false
        })
        .show(&p.anchor(), move |r| {
            if r != kit::controls::DialogResult::Primary
                && let Some(on) = enable
            {
                sw2.set_active(!on);
            }
        });
    let _ = sw;
}

/// "Eigenschappen" with Kopiëren (GNOME's Modem Details).
fn details(p: &Page, m: &Value) -> SettingsGroup {
    let group = SettingsGroup::new("", "");
    let exp = SettingsExpander::new("properties", t("Eigenschappen", "Properties"), &format!("{} {}", s(&m["manufacturer"]), s(&m["model"])));
    exp.header_button().set_widget_name("mobile-properties");
    let rows = properties(m);
    let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
    body.append(&super::network::specs(&rows, true));
    let copy = ed::copy_button(p, rows, "mobile-copy");
    copy.set_halign(gtk::Align::Start);
    body.append(&copy);
    exp.add(&body);
    group.add(exp.widget());
    group
}

#[cfg(test)]
mod tests {
    use super::*;

    fn modem() -> Value {
        json!({
            "path": "/org/freedesktop/ModemManager1/Modem/0", "manufacturer": "Quectel", "model": "EM05-G",
            "firmware": "EM05GFAR07A07M1G", "imei": "861536030196001", "ownNumbers": ["+31612345678"],
            "unlockRequired": "none", "unlockRetries": {}, "failedReason": "", "networkType": "4G", "signal": 72,
            "registration": "home", "operatorName": "KPN", "sim": { "iccid": "8931", "mcc": "204", "mnc": "08" },
            "connections": [
                { "uuid": "a", "id": "MMS", "autoconnect": false, "active": false, "homeOnly": true },
                { "uuid": "b", "id": "Internet", "autoconnect": true, "active": false, "homeOnly": false },
            ],
        })
    }

    #[test]
    fn the_page_follows_modemmanager_in_every_state() {
        for what in ["modem", "state", "devices", "radios", "connections", "active"] {
            assert!(follows(&json!(what)), "{what}");
        }
        assert!(!follows(&json!("accessPoints")) && !follows(&json!("hotspot")));
        // the signal moves all the time: not a reason to build the page again; a modem or the SIM is
        let none = json!({ "available": true, "modems": [], "wwanEnabled": true });
        let one = json!({ "available": true, "modems": [modem()], "wwanEnabled": true });
        assert_ne!(shape_of(&none), shape_of(&one), "a modem plugged in");
        let mut weaker = one.clone();
        weaker["modems"][0]["signal"] = json!(12);
        assert_eq!(shape_of(&one), shape_of(&weaker));
        let mut locked = one.clone();
        locked["modems"][0]["unlockRequired"] = json!("sim-pin");
        assert_ne!(shape_of(&locked), shape_of(&one), "the SIM unlocked elsewhere");
        let mut off = one.clone();
        off["wwanEnabled"] = json!(false);
        assert_ne!(shape_of(&one), shape_of(&off), "the switch turned off elsewhere");
    }

    #[test]
    fn the_status_and_the_sim_notices() {
        let mut m = modem();
        assert_eq!(status_text(&m, false), t("Uit", "Off"));
        assert_eq!(status_text(&m, true), common::fill(t("Niet verbonden ({op})", "Not connected ({op})"), &[("op", "KPN")]));
        m["connections"][1]["state"] = json!("activated");
        assert_eq!(status_text(&m, true), common::fill(t("Verbonden met {op}, {kind}", "Connected to {op}, {kind}"), &[("op", "KPN"), ("kind", "4G")]));
        assert!(blocker(&m).is_none());
        m["unlockRequired"] = json!("sim-pin");
        m["unlockRetries"] = json!({ "sim-pin": 2 });
        let (title, text) = blocker(&m).unwrap();
        assert_eq!(title, t("De simkaart is vergrendeld", "The SIM card is locked"));
        assert!(text.contains('2'));
        m["unlockRequired"] = json!("sim-puk");
        assert_eq!(blocker(&m).unwrap().0, t("De simkaart is geblokkeerd", "The SIM card is blocked"));
        let mut none = modem();
        none["sim"] = Value::Null;
        assert_eq!(blocker(&none).unwrap().0, t("Geen simkaart", "No SIM card"));
    }

    #[test]
    fn the_data_profile_modes_and_properties() {
        let mut m = modem();
        assert_eq!(data_profile(&m).unwrap()["uuid"], "b", "the one that connects by itself");
        m["connections"][0]["active"] = json!(true);
        assert_eq!(data_profile(&m).unwrap()["uuid"], "a", "the active one first");
        assert!(data_profile(&json!({ "connections": [] })).is_none());
        let mode = json!({ "allowed": 14, "preferred": 8, "allowedNames": ["4g", "3g", "2g"], "preferredNames": ["4g"] });
        assert_eq!(mode_label(&mode), common::fill(t("{allowed} (voorkeur: {preferred})", "{allowed} (preferred: {preferred})"), &[("allowed", "4G, 3G, 2G"), ("preferred", "4G")]));
        assert_eq!(mode_label(&json!({ "allowedNames": ["4g"], "preferredNames": [] })), "4G");
        assert_eq!(mode_key(&mode), "14:8");
        let props = properties(&m);
        assert!(props.iter().any(|(_, v)| v == "861536030196001"));
        assert!(props.iter().any(|(_, v)| v == "72%"));
        assert_eq!(registration_text("roaming"), t("Roaming", "Roaming"));
    }

    #[test]
    fn an_access_point_is_a_gsm_profile() {
        let a = Apn {
            name: "KPN Internet".into(),
            apn: "portalmmm.nl".into(),
            user: "KPN".into(),
            password: "gprs".into(),
            auth: "chap".into(),
            kind: "internet".into(),
        };
        let ch = apn_changes(&a).unwrap();
        assert_eq!(ch["gsm"]["apn"], "portalmmm.nl");
        assert_eq!(
            (
                ch["gsm"]["password"].as_str(),
                ch["gsm"]["password-flags"].as_u64()
            ),
            (Some("gprs"), Some(0))
        );
        assert_eq!(ch["ppp"]["refuse-chap"], false);
        assert_eq!(ch["ppp"]["refuse-pap"], true);
        assert!(
            ch["gsm"]["initial-eps-bearer-configure"].is_null(),
            "an update clears the attach bearer of an internet APN"
        );
        for key in [
            "initial-eps-bearer-username",
            "initial-eps-bearer-password",
            "initial-eps-bearer-password-flags",
            "initial-eps-bearer-refuse-chap",
            "initial-eps-bearer-noauth",
        ] {
            assert!(ch["gsm"][key].is_null(), "internet-only clears {key}");
        }
        let attach = apn_changes(&Apn {
            kind: "attach".into(),
            ..a.clone()
        })
        .unwrap();
        assert_eq!(attach["gsm"]["initial-eps-bearer-username"], "KPN");
        assert_eq!(attach["gsm"]["initial-eps-bearer-password"], "gprs");
        assert_eq!(attach["gsm"]["initial-eps-bearer-password-flags"], 0);
        assert_eq!(attach["gsm"]["initial-eps-bearer-noauth"], true);
        assert_eq!(attach["gsm"]["initial-eps-bearer-refuse-chap"], false);
        assert_eq!(attach["gsm"]["initial-eps-bearer-refuse-pap"], true);
        let keep_attach = apn_changes(&Apn {
            kind: "attach".into(),
            password: String::new(),
            ..a.clone()
        })
        .unwrap();
        assert!(keep_attach["gsm"].get("password").is_none());
        assert!(
            keep_attach["gsm"]
                .get("initial-eps-bearer-password")
                .is_none(),
            "blank edits retain both stored passwords"
        );
        let new = new_apn_settings(
            &Apn {
                kind: "attach".into(),
                auth: "auto".into(),
                ..a.clone()
            },
            "8931",
            true,
            true,
        )
        .unwrap();
        assert_eq!(
            new["connection"],
            json!({ "id": "KPN Internet", "type": "gsm", "autoconnect": true })
        );
        assert_eq!(
            (
                new["gsm"]["sim-id"].as_str(),
                new["gsm"]["home-only"].as_bool()
            ),
            (Some("8931"), Some(true))
        );
        assert_eq!(
            (
                new["gsm"]["initial-eps-bearer-configure"].as_bool(),
                new["gsm"]["initial-eps-bearer-apn"].as_str()
            ),
            (Some(true), Some("portalmmm.nl"))
        );
        assert!(new.get("ppp").is_none(), "automatic sign-in: no ppp group");
        assert!(
            new["gsm"]
                .as_object()
                .unwrap()
                .values()
                .all(|v| !v.is_null()),
            "a new profile has no nulls"
        );
        assert!(
            apn_changes(&Apn {
                apn: "two words".into(),
                ..a.clone()
            })
            .is_err()
        );
        assert!(
            apn_changes(&Apn {
                name: " ".into(),
                ..a.clone()
            })
            .is_err()
        );
        let keep = apn_changes(&Apn {
            password: String::new(),
            ..a
        })
        .unwrap();
        assert!(
            keep["gsm"].get("password").is_none(),
            "an empty password keeps the stored one"
        );
        assert_eq!(auth_ppp("none")["refuse-eap"], true);
        assert!(auth_ppp("auto").is_null());
    }

    #[test]
    fn the_operators_presets_fill_the_dialog() {
        let apns = json!({ "providers": [{ "name": "Hi", "apns": [
            { "apn": "portalmmm.nl", "name": "", "usage": ["internet"], "username": "", "password": "", "auth": "" },
            { "apn": "mms.nl", "name": "KPN MMS", "usage": ["mms"], "username": "KPN", "password": "gprs", "auth": "chap" },
            { "apn": "x", "name": "Plain", "usage": [], "username": "u", "password": "p", "auth": "pap" },
        ] }] });
        let list = presets(&apns);
        assert_eq!(list.len(), 2, "the MMS access point is no data access point");
        assert_eq!(list[0].0, "Hi (portalmmm.nl)");
        assert_eq!((list[1].1.auth.as_str(), list[1].1.user.as_str()), ("pap", "u"));
    }
}
