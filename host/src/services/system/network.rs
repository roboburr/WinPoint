use super::{bluetooth::Bluetooth, common::*};
use crate::{
    files::{ApiError, ApiResult},
    services::ServiceContext,
};
use glib::variant::ToVariant;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};
const NM: &str = "org.freedesktop.NetworkManager";
const PATH: &str = "/org/freedesktop/NetworkManager";
const VIRTUAL: &[&str] = &[
    "lo",
    "veth",
    "docker",
    "br-",
    "virbr",
    "vnet",
    "tap",
    "tun",
    "tailscale",
    "wg",
    "zt",
    "vmnet",
    "vboxnet",
    "lxc",
    "lxd",
    "podman",
    "cni",
    "flannel",
];
pub(super) struct Network {
    ctx: Arc<ServiceContext>,
    devices: Mutex<Option<(Instant, Vec<Value>)>>,
    saved: Mutex<Option<(Instant, BTreeMap<String, Vec<String>>)>>,
}
impl Network {
    pub fn new(ctx: Arc<ServiceContext>) -> Self {
        Self {
            ctx,
            devices: Mutex::new(None),
            saved: Mutex::new(None),
        }
    }
    pub fn invalidate(&self) {
        *self.devices.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
    fn devices(&self) -> Result<Vec<Value>, String> {
        if let Some((at, devices)) = self
            .devices
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            if at.elapsed() < Duration::from_secs(20) && !devices.is_empty() {
                return Ok(devices.clone());
            }
        }
        let bus = system()?;
        let paths = first(&call(&bus, NM, PATH, NM, "GetDevices", None, 1500)?);
        let mut devices = vec![];
        for p in paths.as_array().unwrap_or(&vec![]) {
            let Some(path) = p.as_str() else { continue };
            if let Ok(d) = getall(&bus, NM, path, &format!("{NM}.Device")) {
                devices.push(json!({"path":path,"type":integer(&d["DeviceType"]).unwrap_or(0),"iface":d["Interface"].as_str().unwrap_or(""),"state":integer(&d["State"]).unwrap_or(0),"active":d["ActiveConnection"].as_str().unwrap_or("/")}));
            }
        }
        *self.devices.lock().unwrap_or_else(|e| e.into_inner()) =
            Some((Instant::now(), devices.clone()));
        Ok(devices)
    }
    fn active(bus: &gio::DBusConnection, path: &str, devices: &[Value]) -> Option<Value> {
        if path.is_empty() || path == "/" {
            return None;
        }
        let p = getall(bus, NM, path, &format!("{NM}.Connection.Active")).ok()?;
        let iface = devices
            .iter()
            .find(|d| {
                p["Devices"]
                    .as_array()
                    .is_some_and(|a| a.contains(&d["path"]))
            })
            .map(|d| d["iface"].clone())
            .unwrap_or(json!(""));
        let ctype = p["Type"].as_str().unwrap_or("");
        Some(
            json!({"ctype":ctype,"id":p["Id"].as_str().unwrap_or(""),"state":integer(&p["State"]).unwrap_or(0),"iface":iface,"default":truth(&p["Default"])||truth(&p["Default6"]),"ap":p["SpecificObject"].as_str().unwrap_or("/"),"type":match ctype{"802-11-wireless"=>"wifi","802-3-ethernet"|"pppoe"=>"ethernet",_=>"other"}}),
        )
    }
    pub fn get(&self) -> Value {
        let airplane = airplane_state();
        let hardware = airplane_hardware();
        let unavailable = || json!({"connectivity":"unknown","wifiAvailable":false,"wifiEnabled":false,"airplane":airplane,"airplaneHardware":hardware,"primary":{"type":"none","name":"","strength":null},"available":false});
        let Ok(bus) = system() else {
            return unavailable();
        };
        let Ok(p) = getall(&bus, NM, PATH, NM) else {
            return unavailable();
        };
        let Ok(devices) = self.devices() else {
            return unavailable();
        };
        let wifi = devices.iter().any(|d| d["type"] == 2);
        let mut primary = Self::active(
            &bus,
            p["PrimaryConnection"].as_str().unwrap_or("/"),
            &devices,
        )
        .filter(|i| matches!(i["type"].as_str(), Some("wifi" | "ethernet")) && i["state"] == 2);
        if primary.is_none() {
            let mut candidates = p["ActiveConnections"]
                .as_array()
                .unwrap_or(&vec![])
                .iter()
                .filter_map(|v| Self::active(&bus, v.as_str().unwrap_or("/"), &devices))
                .filter(|i| {
                    matches!(i["type"].as_str(), Some("wifi" | "ethernet"))
                        && i["state"] == 2
                        && !VIRTUAL.iter().any(|x| string(&i["iface"]).starts_with(x))
                })
                .collect::<Vec<_>>();
            candidates.sort_by_key(|i| (!truth(&i["default"]), i["type"] != "wifi"));
            primary = candidates.into_iter().next();
        }
        let mut out = json!({"type":"none","name":"","strength":null});
        if let Some(pr) = primary {
            if pr["type"] == "wifi" {
                let ap = getall(
                    &bus,
                    NM,
                    pr["ap"].as_str().unwrap_or("/"),
                    &format!("{NM}.AccessPoint"),
                )
                .unwrap_or(json!({}));
                let ssid = ssid(&ap["Ssid"]);
                out = json!({"type":"wifi","name":if ssid.is_empty(){string(&pr["id"])}else{ssid},"strength":if ap.is_object()&&!ap.as_object().unwrap().is_empty(){Some(integer(&ap["Strength"]).unwrap_or(0))}else{None}});
            } else {
                let id = string(&pr["id"]);
                let name = if id.is_empty() || pr["id"] == pr["iface"] || generic_connection(&id) {
                    "Ethernet".into()
                } else {
                    id
                };
                out = json!({"type":"ethernet","name":name,"strength":null});
            }
            out["device"] = pr["iface"].clone();
            out["connection"] = pr["id"].clone();
        }
        let connectivity = match integer(&p["Connectivity"]).unwrap_or(0) {
            1 => "none",
            2 => "portal",
            3 => "limited",
            4 => "full",
            _ => "unknown",
        };
        json!({"connectivity":connectivity,"wifiAvailable":wifi,"wifiEnabled":truth(&p["WirelessEnabled"])&&wifi,"airplane":airplane,"airplaneHardware":hardware,"primary":out,"available":true,"wifiHardwareEnabled":truth(&p["WirelessHardwareEnabled"])})
    }
    fn saved(&self, max_age: u64) -> BTreeMap<String, Vec<String>> {
        if let Some((at, saved)) = self
            .saved
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            if at.elapsed() < Duration::from_secs(max_age) {
                return saved.clone();
            }
        }
        let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
        if let Ok(bus) = system() {
            if let Ok(r) = call(
                &bus,
                NM,
                &format!("{PATH}/Settings"),
                &format!("{NM}.Settings"),
                "ListConnections",
                None,
                1500,
            ) {
                for path in first(&r).as_array().unwrap_or(&vec![]) {
                    if let Some(path) = path.as_str() {
                        if let Ok(r) = call(
                            &bus,
                            NM,
                            path,
                            &format!("{NM}.Settings.Connection"),
                            "GetSettings",
                            None,
                            5000,
                        ) {
                            let settings = first(&r);
                            if !settings["802-11-wireless"]["ssid"].is_null() {
                                out.entry(ssid(&settings["802-11-wireless"]["ssid"]))
                                    .or_default()
                                    .push(path.into());
                            }
                        }
                    }
                }
            }
        }
        *self.saved.lock().unwrap_or_else(|e| e.into_inner()) = Some((Instant::now(), out.clone()));
        out
    }
    pub fn wifi_list(&self, rescan: bool) -> Value {
        if !available("nmcli") {
            return json!([]);
        }
        if self
            .devices()
            .is_ok_and(|ds| !ds.iter().any(|d| d["type"] == 2))
        {
            return json!([]);
        }
        let result = run(
            &[
                "nmcli",
                "-t",
                "-f",
                "IN-USE,SSID,SIGNAL,SECURITY",
                "device",
                "wifi",
                "list",
                "--rescan",
                if rescan { "yes" } else { "no" },
            ],
            if rescan { 20 } else { 6 },
            true,
        );
        if !result.ok {
            return json!([]);
        }
        let saved = self.saved(30);
        let mut best: BTreeMap<String, Value> = BTreeMap::new();
        for line in result.out.lines() {
            let fields = split_terse(line);
            if fields.len() < 4 || fields[1].is_empty() || fields[1] == "--" {
                continue;
            }
            let sec = fields[3].trim();
            let item = json!({"ssid":fields[1],"strength":fields[2].parse::<i64>().unwrap_or(0),"secure":!sec.is_empty()&&sec!="--","active":fields[0].trim()=="*","saved":saved.contains_key(&fields[1]),"security":if sec=="--"{""}else{sec}});
            let replace = best.get(&fields[1]).is_none_or(|old| {
                (item["active"] == true && old["active"] != true)
                    || (old["active"] != true
                        && integer(&item["strength"]) > integer(&old["strength"]))
            });
            if replace {
                best.insert(fields[1].clone(), item);
            }
        }
        let mut out = best.into_values().collect::<Vec<_>>();
        out.sort_by(|a, b| {
            (!truth(&a["active"]))
                .cmp(&!truth(&b["active"]))
                .then_with(|| integer(&b["strength"]).cmp(&integer(&a["strength"])))
                .then_with(|| {
                    string(&a["ssid"])
                        .to_lowercase()
                        .cmp(&string(&b["ssid"]).to_lowercase())
                })
        });
        json!(out)
    }
    /// GNOME Settings' Wi-Fi switch (cc-wifi-panel.c): NetworkManager's WirelessEnabled through Properties.Set
    /// (polkit's enable-disable-wifi, so with the password dialog allowed); nmcli only while NetworkManager is not
    /// on the system bus.
    pub fn set_wifi(&self, enabled: bool) -> ApiResult {
        if let Ok(bus) = system()
            && getall(&bus, NM, PATH, NM).is_ok()
        {
            let args = (NM, "WirelessEnabled", enabled.to_variant()).to_variant();
            call_interactive(
                &bus,
                NM,
                PATH,
                "org.freedesktop.DBus.Properties",
                "Set",
                Some(&args),
                self.ctx.lang(),
            )?;
            self.invalidate();
            return Ok(self.get());
        }
        if !run(
            &["nmcli", "radio", "wifi", if enabled { "on" } else { "off" }],
            8,
            true,
        )
        .ok
        {
            return Err(ApiError::tr(
                "failed",
                self.ctx.lang(),
                "system.network.wifiFailed",
                crate::i18n::NO_PARAMS,
            ));
        }
        Ok(self.get())
    }
    pub fn disconnect(&self, iface: &Value) -> ApiResult {
        let devices = self.devices().map_err(|_| {
            ApiError::tr(
                "internal",
                self.ctx.lang(),
                "system.network.internalError",
                crate::i18n::NO_PARAMS,
            )
        })?;
        let wifi = if truth(iface) {
            let found = devices
                .iter()
                .filter(|d| d["iface"] == *iface)
                .collect::<Vec<_>>();
            if found.is_empty() {
                return Err(ApiError::tr(
                    "not_found",
                    self.ctx.lang(),
                    "system.network.noAdapter",
                    crate::i18n::NO_PARAMS,
                ));
            }
            found
        } else {
            devices
                .iter()
                .filter(|d| d["type"] == 2 && d["active"] != "/" && d["active"] != "")
                .collect::<Vec<_>>()
        };
        let Some(dev) = wifi.first() else {
            return Ok(json!({"ok":true}));
        };
        let r = run(
            &[
                "nmcli",
                "device",
                "disconnect",
                dev["iface"].as_str().unwrap_or(""),
            ],
            10,
            true,
        );
        self.invalidate();
        if !r.ok {
            return Err(ApiError::tr(
                "failed",
                self.ctx.lang(),
                "system.network.disconnectFailed",
                crate::i18n::NO_PARAMS,
            ));
        }
        Ok(json!({"ok":true}))
    }
    pub fn connect(&self, args: &Value) -> ApiResult {
        let name = args["ssid"]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 32)
            .ok_or_else(|| {
                ApiError::tr(
                    "bad_args",
                    self.ctx.lang(),
                    "system.network.badName",
                    crate::i18n::NO_PARAMS,
                )
            })?;
        let password = if args["password"].is_null() {
            None
        } else {
            Some(
                args["password"]
                    .as_str()
                    .filter(|p| p.chars().count() <= 64 && !p.contains('\0'))
                    .ok_or_else(|| {
                        ApiError::tr(
                            "bad_args",
                            self.ctx.lang(),
                            "system.network.badPassword",
                            crate::i18n::NO_PARAMS,
                        )
                    })?,
            )
        }
        .filter(|p| !p.is_empty());
        let Ok(bus) = system() else {
            return Ok(wifi_fail(&self.ctx, "failed", Some("unavailable")));
        };
        let Ok(devices) = self.devices() else {
            return Ok(wifi_fail(&self.ctx, "failed", Some("unavailable")));
        };
        // optional `iface` (batch C): the adapter the Wi-Fi page shows when there are several
        let only = args["iface"].as_str().filter(|s| !s.is_empty());
        let wifi = devices
            .iter()
            .filter(|d| d["type"] == 2 && only.is_none_or(|i| d["iface"] == i))
            .collect::<Vec<_>>();
        let Some(dev) = wifi
            .iter()
            .find(|d| integer(&d["state"]).unwrap_or(0) >= 30)
            .copied()
            .or_else(|| wifi.first().copied())
        else {
            return Ok(wifi_fail(&self.ctx, "failed", Some("no_adapter")));
        };
        let path = dev["path"].as_str().unwrap_or("/");
        let mut ap = find_ap(&bus, path, name);
        if ap.is_none() {
            let _ = call(
                &bus,
                NM,
                path,
                &format!("{NM}.Device.Wireless"),
                "RequestScan",
                Some(&(BTreeMap::<String, glib::Variant>::new(),).to_variant()),
                5000,
            );
            thread::sleep(Duration::from_secs(3));
            ap = find_ap(&bus, path, name);
        }
        let Some(ap) = ap else {
            return Ok(wifi_fail(&self.ctx, "not_found", None));
        };
        let sec = ap["security"].as_str().unwrap_or("open");
        // a saved profile first: an enterprise (802.1X) or WEP network the page added with its own fields connects
        // like any other saved one (ActivateConnection; GNOME Shell's secret agent asks for what is missing), as
        // GNOME's Wi-Fi panel does; only a new one needs those fields (the page's dialog, net.wifi.hidden)
        let saved = self.saved(0).remove(name).unwrap_or_default();
        if matches!(sec, "enterprise" | "wep") && saved.is_empty() {
            return Ok(wifi_fail(&self.ctx, "failed", Some(sec)));
        }
        let needs_key = matches!(sec, "wpa-psk" | "sae");
        if needs_key && password.is_none() && saved.is_empty() {
            return Ok(wifi_fail(&self.ctx, "needs_password", None));
        }
        if needs_key && password.is_some_and(|p| !psk_valid(p)) {
            return Ok(wifi_fail(&self.ctx, "wrong_password", Some("psk_format")));
        }
        let context = glib::MainContext::new();
        let mut created: Option<String> = None;
        let outcome = context
            .with_thread_default(|| {
                let events = Arc::new(Mutex::new(VecDeque::<(i64, i64)>::new()));
                let copy = events.clone();
                let sid = bus.signal_subscribe(
                    Some(NM),
                    Some(&format!("{NM}.Device")),
                    Some("StateChanged"),
                    Some(path),
                    None,
                    gio::DBusSignalFlags::NONE,
                    move |_, _, _, _, _, v| {
                        let j = variant_json(v);
                        if let Ok(mut q) = copy.lock() {
                            q.push_back((integer(&j[0]).unwrap_or(0), integer(&j[2]).unwrap_or(0)));
                        }
                    },
                );
                let activation = (|| -> Result<String, String> {
                    let device = object_path(path)?;
                    let access = object_path(ap["path"].as_str().unwrap_or("/"))?;
                    if let Some(conn) = saved.first() {
                        if let Some(password) = password.filter(|_| needs_key) {
                            update_psk(&bus, conn, sec, password)?;
                        }
                        let r = call(
                            &bus,
                            NM,
                            PATH,
                            NM,
                            "ActivateConnection",
                            Some(&(object_path(conn)?, device, access).to_variant()),
                            15000,
                        )?;
                        Ok(string(&first(&r)))
                    } else {
                        let settings = wifi_settings(name, sec, password);
                        let r = call(
                            &bus,
                            NM,
                            PATH,
                            NM,
                            "AddAndActivateConnection",
                            Some(&(settings, device, access).to_variant()),
                            15000,
                        )?;
                        let j = variant_json(&r);
                        created = j[0].as_str().map(str::to_owned);
                        Ok(string(&j[1]))
                    }
                })();
                let result = match activation {
                    Ok(active) => {
                        let until = Instant::now() + Duration::from_secs(45);
                        let mut started = false;
                        let mut last_poll = Instant::now() - Duration::from_secs(2);
                        loop {
                            while context.pending() {
                                context.iteration(false);
                            }
                            let mut state = None;
                            {
                                let mut q = events.lock().unwrap_or_else(|e| e.into_inner());
                                while let Some((new, reason)) = q.pop_front() {
                                    if (40..=90).contains(&new) {
                                        started = true
                                    } else if new == 100 && started {
                                        state = Some((true, None));
                                        break;
                                    } else if new == 120 || (new == 30 && started) {
                                        state = Some((false, Some(reason)));
                                        break;
                                    }
                                }
                            }
                            if let Some(state) = state {
                                break Ok(state);
                            }
                            if last_poll.elapsed() > Duration::from_secs(1) {
                                last_poll = Instant::now();
                                match getall(&bus, NM, &active, &format!("{NM}.Connection.Active"))
                                {
                                    Ok(p) if p["State"] == 2 => break Ok((true, None)),
                                    Err(_) if started => break Ok((false, None)),
                                    _ => {}
                                }
                            }
                            if Instant::now() >= until {
                                break Ok((false, Some(-1)));
                            }
                            thread::sleep(Duration::from_millis(50));
                        }
                    }
                    Err(e) => Err(e),
                };
                bus.signal_unsubscribe(sid);
                result
            })
            .map_err(|e| e.to_string())
            .and_then(|v| v);
        self.invalidate();
        *self.saved.lock().unwrap_or_else(|e| e.into_inner()) = None;
        match outcome {
            Ok((true, _)) => Ok(json!({"ok":true})),
            Ok((false, reason)) => {
                if let Some(created) = created {
                    let _ = call(
                        &bus,
                        NM,
                        &created,
                        &format!("{NM}.Settings.Connection"),
                        "Delete",
                        None,
                        1500,
                    );
                }
                let (code, why) = reason_code(reason, password.is_some(), !saved.is_empty());
                Ok(wifi_fail(&self.ctx, code, why))
            }
            Err(e) => {
                let (code, why) =
                    if e.contains("InvalidProperty") || e.to_lowercase().contains("psk") {
                        ("wrong_password", Some("invalid_property"))
                    } else if ["PermissionDenied", "NotAuthorized", "AccessDenied"]
                        .iter()
                        .any(|s| e.contains(s))
                    {
                        ("failed", Some("denied"))
                    } else {
                        ("failed", Some("dbus"))
                    };
                Ok(wifi_fail(&self.ctx, code, why))
            }
        }
    }
    /// Flight mode (amendment 57): through gnome-settings-daemon's Rfkill when it runs (GNOME Settings' own switch:
    /// AirplaneMode blocks or unblocks Wi-Fi, Bluetooth and mobile broadband together), else nmcli and rfkill.
    pub fn set_airplane(&self, enabled: bool, bt: &Bluetooth) -> ApiResult {
        if let Some(result) = gsd_set_airplane(enabled) {
            if result.is_err() {
                return Err(ApiError::tr(
                    "failed",
                    self.ctx.lang(),
                    "system.network.airplaneFailed",
                    crate::i18n::NO_PARAMS,
                ));
            }
            thread::sleep(Duration::from_millis(300));
            self.invalidate();
            return Ok(self.get());
        }
        let mut ok = false;
        if available("nmcli") {
            ok = run(
                &["nmcli", "radio", "all", if enabled { "off" } else { "on" }],
                8,
                true,
            )
            .ok;
        }
        if available("rfkill") && !radios().is_empty() {
            ok = run(
                &["rfkill", if enabled { "block" } else { "unblock" }, "all"],
                5,
                false,
            )
            .ok || ok;
        } else if enabled {
            ok = bt.set_powered(false).is_ok() || ok;
        }
        if !ok {
            return Err(ApiError::tr(
                "failed",
                self.ctx.lang(),
                "system.network.airplaneFailed",
                crate::i18n::NO_PARAMS,
            ));
        }
        thread::sleep(Duration::from_millis(300));
        self.invalidate();
        Ok(self.get())
    }
}
fn generic_connection(id: &str) -> bool {
    let s = id.to_lowercase();
    if s.starts_with("netplan-") || s.starts_with("ethernet") || s.starts_with("auto") {
        return true;
    }
    let rest = s
        .trim_start_matches("bekabelde")
        .trim_start_matches("wired")
        .trim_start_matches("kabel")
        .trim();
    let rest = rest
        .trim_start_matches("connection")
        .trim_start_matches("verbinding")
        .trim();
    rest.chars().all(|c| c.is_ascii_digit())
}
pub(super) fn split_terse(line: &str) -> Vec<String> {
    let mut out = vec![];
    let mut s = String::new();
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(n) = chars.next() {
                    s.push(n)
                } else {
                    s.push(c)
                }
            }
            ':' => {
                out.push(std::mem::take(&mut s));
            }
            _ => s.push(c),
        }
    }
    out.push(s);
    out
}
fn ssid(v: &Value) -> String {
    if let Some(a) = v.as_array() {
        String::from_utf8_lossy(
            &a.iter()
                .filter_map(|v| integer(v).map(|i| i as u8))
                .collect::<Vec<_>>(),
        )
        .into_owned()
    } else {
        string(v)
    }
}
pub(super) fn security(flags: i64, wpa: i64, rsn: i64) -> &'static str {
    let k = wpa | rsn;
    if k & (0x200 | 0x2000) != 0 {
        "enterprise"
    } else if k & 0x100 != 0 {
        "wpa-psk"
    } else if k & 0x400 != 0 {
        "sae"
    } else if k & (0x800 | 0x1000) != 0 {
        "owe"
    } else if flags & 1 != 0 {
        "wep"
    } else {
        "open"
    }
}
pub(super) fn psk_valid(s: &str) -> bool {
    if s.len() == 64 {
        s.bytes().all(|b| b.is_ascii_hexdigit())
    } else {
        (8..=63).contains(&s.len()) && s.bytes().all(|b| (32..127).contains(&b))
    }
}
fn find_ap(bus: &gio::DBusConnection, path: &str, name: &str) -> Option<Value> {
    let a = first(
        &call(
            bus,
            NM,
            path,
            &format!("{NM}.Device.Wireless"),
            "GetAllAccessPoints",
            None,
            1500,
        )
        .ok()?,
    );
    let mut best = None;
    let mut strength = -1;
    for path in a.as_array()? {
        let path = path.as_str()?;
        if let Ok(p) = getall(bus, NM, path, &format!("{NM}.AccessPoint")) {
            let current = integer(&p["Strength"]).unwrap_or(0);
            if ssid(&p["Ssid"]) == name && current > strength {
                strength = current;
                best = Some(
                    json!({"path":path,"strength":current,"security":security(integer(&p["Flags"]).unwrap_or(0),integer(&p["WpaFlags"]).unwrap_or(0),integer(&p["RsnFlags"]).unwrap_or(0))}),
                );
            }
        }
    }
    best
}
fn wifi_settings(
    ssid: &str,
    security: &str,
    password: Option<&str>,
) -> BTreeMap<String, BTreeMap<String, glib::Variant>> {
    let mut s = BTreeMap::from([
        (
            "connection".into(),
            BTreeMap::from([
                ("id".into(), ssid.to_variant()),
                ("type".into(), "802-11-wireless".to_variant()),
                ("autoconnect".into(), true.to_variant()),
            ]),
        ),
        (
            "802-11-wireless".into(),
            BTreeMap::from([
                ("ssid".into(), ssid.as_bytes().to_variant()),
                ("mode".into(), "infrastructure".to_variant()),
            ]),
        ),
        (
            "ipv4".into(),
            BTreeMap::from([("method".into(), "auto".to_variant())]),
        ),
        (
            "ipv6".into(),
            BTreeMap::from([("method".into(), "auto".to_variant())]),
        ),
    ]);
    if matches!(security, "wpa-psk" | "sae" | "owe") {
        let mut sec = BTreeMap::from([("key-mgmt".into(), security.to_variant())]);
        if security != "owe" {
            sec.insert("psk".into(), password.unwrap_or("").to_variant());
        }
        s.insert("802-11-wireless-security".into(), sec);
    }
    s
}
fn update_psk(
    bus: &gio::DBusConnection,
    path: &str,
    security: &str,
    password: &str,
) -> Result<(), String> {
    let r = call(
        bus,
        NM,
        path,
        &format!("{NM}.Settings.Connection"),
        "GetSettings",
        None,
        5000,
    )?;
    let raw = r.child_value(0);
    let mut groups = BTreeMap::<String, BTreeMap<String, glib::Variant>>::new();
    for i in 0..raw.n_children() {
        let e = raw.child_value(i);
        if let Some(k) = e.child_value(0).str() {
            groups.insert(k.into(), dictionary(&e.child_value(1)));
        }
    }
    let sec = groups.entry("802-11-wireless-security".into()).or_default();
    sec.entry("key-mgmt".into())
        .or_insert_with(|| security.to_variant());
    sec.insert("psk".into(), password.to_variant());
    call(
        bus,
        NM,
        path,
        &format!("{NM}.Settings.Connection"),
        "Update",
        Some(&(groups,).to_variant()),
        10000,
    )?;
    Ok(())
}
fn wifi_fail(ctx: &ServiceContext, code: &str, why: Option<&str>) -> Value {
    let strings: Value =
        serde_json::from_str(include_str!("strings.json")).expect("embedded system translations");
    let key = why
        .filter(|w| strings["wifi"][w].is_array())
        .unwrap_or(code);
    let key = if strings["wifi"][key].is_array() {
        key
    } else {
        "failed"
    };
    let text = strings["wifi"][key][if ctx.language() == "en" { 1 } else { 0 }]
        .as_str()
        .unwrap_or("");
    let mut v = json!({"ok":false,"code":code,"error":text});
    if let Some(why) = why {
        v["reason"] = json!(why)
    }
    v
}
fn reason_code(
    r: Option<i64>,
    password: bool,
    saved: bool,
) -> (&'static str, Option<&'static str>) {
    match r.unwrap_or(0) {
        7 => {
            if password {
                ("wrong_password", None)
            } else {
                ("needs_password", None)
            }
        }
        8 | 11 => {
            if password || saved {
                ("wrong_password", None)
            } else {
                ("failed", None)
            }
        }
        53 => ("not_found", None),
        5 | 17 | 18 => ("failed", Some("ip")),
        -1 => ("failed", Some("timeout")),
        _ => ("failed", None),
    }
}
pub(super) fn radios() -> Vec<Value> {
    let mut out = vec![];
    if let Ok(ds) = std::fs::read_dir("/sys/class/rfkill") {
        for d in ds.flatten() {
            let p = d.path();
            let read = |n| {
                std::fs::read_to_string(p.join(n))
                    .unwrap_or_default()
                    .trim()
                    .to_owned()
            };
            out.push(json!({"name":d.file_name().to_string_lossy(),"type":read("type"),"soft":read("soft")=="1","hard":read("hard")=="1"}));
        }
    }
    out
}
const RFKILL: &str = "org.gnome.SettingsDaemon.Rfkill";
const RFKILL_PATH: &str = "/org/gnome/SettingsDaemon/Rfkill";
/// gnome-settings-daemon's Rfkill on the session bus (its rfkill plugin; GNOME Settings' Wi-Fi panel and Flight
/// mode switch use it): its properties, None when it is not there.
pub(super) fn gsd_rfkill(bus: &gio::DBusConnection) -> Option<Value> {
    let v = bus
        .call_sync(
            Some(RFKILL),
            RFKILL_PATH,
            "org.freedesktop.DBus.Properties",
            "GetAll",
            Some(&(RFKILL,).to_variant()),
            None,
            gio::DBusCallFlags::NONE,
            500,
            gio::Cancellable::NONE,
        )
        .ok()?;
    let props = variant_json(&v.child_value(0));
    props.get("AirplaneMode").is_some().then_some(props)
}
/// Set gsd's AirplaneMode (no polkit: gsd owns /dev/rfkill for the session); None without gsd's Rfkill.
pub(super) fn gsd_set_airplane(enabled: bool) -> Option<Result<(), String>> {
    let bus = session().ok()?;
    gsd_rfkill(&bus)?;
    Some(setprop(
        &bus,
        RFKILL,
        RFKILL_PATH,
        RFKILL,
        "AirplaneMode",
        enabled.to_variant(),
        5000,
    ))
}
/// GNOME's "Vliegtuigmodus via hardwareschakelaar staat aan" (gsd's HardwareAirplaneMode; without gsd: every radio
/// hard-blocked): the flight mode page's notice, and its switch cannot change it.
fn airplane_hardware() -> bool {
    if let Some(props) = session().ok().and_then(|bus| gsd_rfkill(&bus)) {
        return truth(&props["HardwareAirplaneMode"]);
    }
    let radios = radios();
    !radios.is_empty() && radios.iter().all(|r| truth(&r["hard"]))
}
fn airplane_state() -> bool {
    if let Some(props) = session().ok().and_then(|bus| gsd_rfkill(&bus)) {
        return truth(&props["AirplaneMode"]) || truth(&props["HardwareAirplaneMode"]);
    }
    let radios = radios();
    !radios.is_empty()
        && radios
            .iter()
            .all(|r| truth(&r["soft"]) || truth(&r["hard"]))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nmcli_escapes() {
        assert_eq!(
            split_terse(r"*:a\:b:90:WPA2"),
            vec!["*", "a:b", "90", "WPA2"]
        );
        assert_eq!(split_terse("a\\"), vec!["a\\"]);
    }
    #[test]
    fn security_precedence() {
        assert_eq!(security(1, 0x100, 0x400), "wpa-psk");
        assert_eq!(security(1, 0x200, 0x100), "enterprise");
        assert_eq!(security(1, 0, 0), "wep");
        assert_eq!(security(0, 0, 0x800), "owe");
    }
    /// Flight mode through gsd's Rfkill (amendment 57), against tests/dbus-fakes/fakes/c/networkmanager.py.
    #[test]
    #[ignore = "batch C bus test: only tests/dbus-fakes/run.sh --batch c"]
    fn bus_flight_mode_goes_through_gsd_rfkill() {
        let Ok(file) = std::env::var("WIN11_FAKE_CALLS_NETWORKMANAGER") else {
            eprintln!("skipped: run inside tests/dbus-fakes/run.sh --batch c");
            return;
        };
        let calls = || -> Vec<Value> {
            std::fs::read_to_string(&file)
                .unwrap_or_default()
                .lines()
                .filter_map(|l| serde_json::from_str(l).ok())
                .collect()
        };
        let n = calls().len();
        assert!(gsd_rfkill(&session().unwrap()).is_some());
        assert_eq!(gsd_set_airplane(true), Some(Ok(())));
        assert!(airplane_state(), "AirplaneMode follows");
        let set = calls()
            .into_iter()
            .skip(n)
            .find(|c| c["method"] == "Set")
            .expect("a Set call");
        assert_eq!(set["bus"], "session");
        assert_eq!(
            set["args"],
            "('org.gnome.SettingsDaemon.Rfkill', 'AirplaneMode', <true>)"
        );
        assert_eq!(gsd_set_airplane(false), Some(Ok(())));
        assert!(!airplane_state());
    }
    /// The Wi-Fi switch is GNOME's: Properties.Set of NetworkManager's WirelessEnabled with the password dialog
    /// allowed, no nmcli (the private settings harness found the switch failing on its nmcli stub).
    #[test]
    #[ignore = "batch C bus test: only tests/dbus-fakes/run.sh --batch c"]
    fn bus_wifi_switch_sets_wireless_enabled() {
        let Ok(file) = std::env::var("WIN11_FAKE_CALLS_NETWORKMANAGER") else {
            eprintln!("skipped: run inside tests/dbus-fakes/run.sh --batch c");
            return;
        };
        let calls = || -> Vec<Value> {
            std::fs::read_to_string(&file)
                .unwrap_or_default()
                .lines()
                .filter_map(|l| serde_json::from_str(l).ok())
                .collect()
        };
        let context = super::super::common::LangContext::new("en", "settings");
        let network = Network::new(context.ctx.clone());
        let n = calls().len();
        let state = network.set_wifi(false).expect("Wi-Fi off");
        assert_eq!(
            state["wifiEnabled"], false,
            "the answer reads NetworkManager again"
        );
        let set = calls()
            .into_iter()
            .skip(n)
            .find(|c| c["method"] == "Set")
            .expect("a Set call");
        assert_eq!(set["bus"], "system");
        assert_eq!(
            set["args"],
            "('org.freedesktop.NetworkManager', 'WirelessEnabled', <false>)"
        );
        assert_eq!(set["interactive"], true);
        assert_eq!(
            network.set_wifi(true).expect("Wi-Fi on")["wifiEnabled"],
            true
        );
    }
    /// An enterprise (802.1X) network in the list: refused while it has no profile (the page then asks for its fields
    /// and adds it with net.wifi.hidden {hidden: false}); once saved, connecting activates that profile, as GNOME's
    /// Wi-Fi panel does, instead of the "zakelijk netwerk" refusal.
    #[test]
    #[ignore = "batch C bus test: only tests/dbus-fakes/run.sh --batch c"]
    fn bus_a_saved_enterprise_network_connects() {
        let Ok(file) = std::env::var("WIN11_FAKE_CALLS_NETWORKMANAGER") else {
            eprintln!("skipped: run inside tests/dbus-fakes/run.sh --batch c");
            return;
        };
        let calls = || -> Vec<Value> {
            std::fs::read_to_string(&file)
                .unwrap_or_default()
                .lines()
                .filter_map(|l| serde_json::from_str(l).ok())
                .collect()
        };
        let context = super::super::common::LangContext::new("en", "settings");
        let network = Network::new(context.ctx.clone());
        let r = network.connect(&json!({ "ssid": "eduroam" })).unwrap();
        assert_eq!(
            (r["ok"].clone(), r["reason"].clone()),
            (json!(false), json!("enterprise")),
            "no profile yet: the page's dialog first"
        );
        // the page's dialog: WPA/WPA2-Enterprise with PEAP, a network in the list (not hidden)
        let net = crate::services::net::Net::new(context.ctx.clone()).unwrap();
        let n = calls().len();
        net.call(
            "net.wifi.hidden",
            &json!({ "ssid": "eduroam", "security": "wpa-eap", "hidden": false, "iface": "wlan0",
                "secrets": { "802-1x": { "eap": ["peap"], "identity": "alice", "phase2-auth": "mschapv2", "password": "x" } } }),
        )
        .unwrap()
        .unwrap();
        let add = calls()
            .into_iter()
            .skip(n)
            .find(|c| c["method"] == "AddAndActivateConnection2")
            .expect("AddAndActivateConnection2");
        let body = add["args"].as_str().unwrap();
        assert!(body.contains("'hidden': <false>"), "{body}");
        assert!(body.contains("'key-mgmt': <'wpa-eap'>"), "{body}");
        assert!(body.contains("'identity': <'alice'>"), "{body}");
        net.shutdown();
        let n = calls().len();
        let r = network.connect(&json!({ "ssid": "eduroam" })).unwrap();
        assert_eq!(r["ok"], true, "{r}");
        let activate = calls()
            .into_iter()
            .skip(n)
            .find(|c| c["method"] == "ActivateConnection")
            .expect("ActivateConnection of the saved profile");
        assert!(
            activate["args"]
                .as_str()
                .is_some_and(|a| a.contains("/org/freedesktop/NetworkManager/AccessPoint/2")),
            "{activate}"
        );
    }
    #[test]
    fn secrets_and_variant_types() {
        let s = wifi_settings("é", "wpa-psk", Some("a$`12345"));
        assert_eq!(s["802-11-wireless"]["ssid"].type_().as_str(), "ay");
        assert_eq!(s["connection"]["autoconnect"].type_().as_str(), "b");
        assert!(psk_valid("12345678"));
        assert!(!psk_valid("1234567"));
        assert!(!psk_valid("é12345678"));
    }
}
