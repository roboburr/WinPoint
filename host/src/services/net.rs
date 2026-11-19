//! `net.*` (batch C): NetworkManager connections, devices, Wi-Fi, the radio and connectivity switches, the mobile
//! hotspot, the VPN plugins and import, and ModemManager's modems (notes/claude/settings-parity.md §5.4), for
//! Instellingen › Netwerk en internet.
//!
//! Every profile edit is NetworkManager's own Update2 merge of what the page changed into what NM has
//! (`net/settings_map.rs`: keys the page does not name stay as NM sent them, deprecated twins are dropped so NM
//! takes the change), saved to disk; system writes go through `common::call_interactive` (polkit as GNOME Settings,
//! §5.8). The dangerous methods only check their arguments under `ctx.dry_run`, and announce a change with
//! `net.changed {what}`; a watcher on NetworkManager's signals announces the others while a page listens.
//! Methods that carry a password or key are `"secret": true` in host/src/api-methods/network.json: nothing here
//! logs their arguments, and their errors name groups and keys, never values.
mod connections;
mod hotspot;
mod modem;
mod providers;
pub(crate) mod settings_map;
#[cfg(test)]
mod tests;
mod vpn;
mod vpn_import;
mod wifi;

use super::ServiceContext;
use crate::files::{ApiError, ApiResult};
use crate::i18n::{Lang, NO_PARAMS};
use connections as nm;
use serde_json::{Value, json};
use settings_map::MapError;
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

/// The methods (name, dangerous), exactly as host/src/api-methods/network.json lists them (a test compares).
#[cfg_attr(not(test), allow(dead_code))] // the dispatch tests compare it with the api-methods files
pub(crate) const METHODS: &[(&str, bool)] = &[
    ("net.connections", false),
    ("net.connection.get", false),
    ("net.connection.update", true),
    ("net.connection.add", true),
    ("net.connection.delete", true),
    ("net.connection.activate", true),
    ("net.connection.deactivate", true),
    ("net.devices", false),
    ("net.device.setEnabled", true),
    ("net.wifi.hidden", true),
    ("net.wifi.secrets", false),
    ("net.radios.set", true),
    ("net.connectivity.set", true),
    ("net.hotspot.get", false),
    ("net.hotspot.start", true),
    ("net.hotspot.stop", true),
    ("net.vpn.plugins", false),
    ("net.vpn.import", true),
    ("net.modem.get", false),
    ("net.modem.unlock", true),
    ("net.modem.setPin", true),
    ("net.modem.changePin", true),
    ("net.modem.setModes", true),
    ("net.modem.scan", false),
    ("net.modem.register", true),
    ("net.modem.setPrimarySlot", true),
    ("net.modem.apns", false),
];

pub(crate) struct Net {
    ctx: Arc<ServiceContext>,
    stopped: Arc<AtomicBool>,
}

fn bad(lang: Lang) -> ApiError {
    ApiError::tr("bad_args", lang, "net.badArgs", NO_PARAMS)
}

fn map_error(e: MapError, lang: Lang) -> ApiError {
    match e {
        MapError::BadArgs => bad(lang),
        MapError::BadValue { group, key } => {
            let mut err = ApiError::tr("bad_args", lang, "net.badValue", NO_PARAMS);
            err.data = Some(json!({ "group": group, "key": key }));
            err
        }
    }
}

/// A connection uuid (RFC 4122 text, as NetworkManager makes them; nothing else reaches the bus).
pub(crate) fn uuid_ok(s: &str) -> bool {
    s.len() == 36
        && s.char_indices().all(|(i, c)| {
            if matches!(i, 8 | 13 | 18 | 23) {
                c == '-'
            } else {
                c.is_ascii_hexdigit()
            }
        })
}

fn uuid_arg(v: &Value, lang: Lang) -> Result<String, ApiError> {
    v.as_str()
        .filter(|s| uuid_ok(s))
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| bad(lang))
}

/// A network interface name (the kernel's rule: 1-15 bytes, no `/`, `:` or blank).
pub(crate) fn iface_ok(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 15
        && s != "."
        && s != ".."
        && s.bytes()
            .all(|b| b.is_ascii_graphic() && b != b'/' && b != b':')
}

fn iface_arg(v: &Value, lang: Lang) -> Result<String, ApiError> {
    v.as_str()
        .filter(|s| iface_ok(s))
        .map(str::to_owned)
        .ok_or_else(|| bad(lang))
}

fn s_arg(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}

fn bool_arg(v: &Value, lang: Lang) -> Result<bool, ApiError> {
    v.as_bool().ok_or_else(|| bad(lang))
}

fn opt_bool(v: &Value, lang: Lang) -> Result<Option<bool>, ApiError> {
    if v.is_null() {
        Ok(None)
    } else {
        bool_arg(v, lang).map(Some)
    }
}

/// A modem argument: its ModemManager object path.
fn modem_arg(v: &Value, lang: Lang) -> Result<String, ApiError> {
    v.as_str()
        .filter(|s| modem::modem_ok(s))
        .map(str::to_owned)
        .ok_or_else(|| bad(lang))
}

/// A SIM code argument (PIN or PUK).
fn code_arg(v: &Value, puk: bool, lang: Lang) -> Result<String, ApiError> {
    v.as_str()
        .filter(|s| {
            if puk {
                modem::puk_ok(s)
            } else {
                modem::pin_ok(s)
            }
        })
        .map(str::to_owned)
        .ok_or_else(|| ApiError::tr("bad_args", lang, "net.badPin", NO_PARAMS))
}

fn u32_arg(v: &Value, lang: Lang) -> Result<u32, ApiError> {
    v.as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .ok_or_else(|| bad(lang))
}

/// The system bus for ModemManager (NetworkManager need not run).
fn system_bus(lang: Lang) -> Result<gio::DBusConnection, ApiError> {
    crate::services::system::common::system().map_err(|_| modem::not_found(lang))
}

/// A new connection's uuid (the kernel's random uuid).
fn new_uuid() -> String {
    std::fs::read_to_string("/proc/sys/kernel/random/uuid")
        .map(|s| s.trim().to_owned())
        .unwrap_or_default()
}

impl Net {
    pub(crate) fn new(ctx: Arc<ServiceContext>) -> Result<Arc<Self>, String> {
        let net = Arc::new(Self {
            ctx,
            stopped: Arc::new(AtomicBool::new(false)),
        });
        let (weak, stopped) = (Arc::downgrade(&net), net.stopped.clone());
        std::thread::Builder::new()
            .name("net-watch".into())
            .spawn(move || watch(weak, stopped))
            .map_err(|e| e.to_string())?;
        Ok(net)
    }

    pub(crate) fn shutdown(&self) {
        self.stopped.store(true, Ordering::Release);
    }

    fn changed(&self, what: &str) {
        self.ctx.emit("net.changed", json!({ "what": what }));
    }

    pub(crate) fn call(&self, method: &str, args: &Value) -> Option<ApiResult> {
        if !METHODS.iter().any(|(m, _)| *m == method) {
            return None;
        }
        Some(self.dispatch(method, args, self.ctx.lang()))
    }

    fn dispatch(&self, method: &str, args: &Value, lang: Lang) -> ApiResult {
        let dry = self.ctx.dry_run;
        match method {
            "net.connections" => {
                let filter = match &args["type"] {
                    Value::Null => None,
                    Value::String(t) if t.len() <= 40 => Some(t.as_str()),
                    _ => return Err(bad(lang)),
                };
                let bus = nm::bus(lang)?;
                nm::connections(&bus, filter).map_err(|_| nm::unavailable(lang))
            }
            "net.connection.get" => {
                let uuid = uuid_arg(&args["uuid"], lang)?;
                nm::get(&nm::bus(lang)?, &uuid, lang)
            }
            "net.connection.update" => self.update(args, lang, dry),
            "net.connection.add" => self.add(args, lang, dry),
            "net.connection.delete" => {
                let uuid = uuid_arg(&args["uuid"], lang)?;
                if !dry {
                    let bus = nm::bus(lang)?;
                    let path = nm::path_of(&bus, &uuid, lang)?;
                    nm::delete(&bus, &path, lang)?;
                    self.changed("connections");
                }
                Ok(json!({ "uuid": uuid }))
            }
            "net.connection.activate" => {
                let uuid = uuid_arg(&args["uuid"], lang)?;
                let device = match &args["device"] {
                    Value::Null => None,
                    v => Some(iface_arg(v, lang)?),
                };
                if dry {
                    return Ok(json!({ "uuid": uuid }));
                }
                let bus = nm::bus(lang)?;
                let path = nm::path_of(&bus, &uuid, lang)?;
                let dev = match device {
                    Some(d) => nm::device_of(&bus, &d).ok_or_else(|| nm::no_device(lang))?,
                    None => "/".to_owned(),
                };
                let active = nm::activate(&bus, &path, &dev, lang)?;
                self.changed("active");
                Ok(json!({ "uuid": uuid, "active": active }))
            }
            "net.connection.deactivate" => {
                let uuid = uuid_arg(&args["uuid"], lang)?;
                if !dry {
                    let bus = nm::bus(lang)?;
                    let actives = nm::actives(&bus);
                    let (active, _) = actives
                        .iter()
                        .find(|(_, a)| a["Uuid"] == json!(uuid))
                        .ok_or_else(|| {
                            ApiError::tr("not_found", lang, "net.notActive", NO_PARAMS)
                        })?;
                    nm::deactivate(&bus, active, lang)?;
                    self.changed("active");
                }
                Ok(json!({ "uuid": uuid }))
            }
            "net.devices" => {
                let bus = nm::bus(lang)?;
                if args["scan"] == true {
                    nm::request_scan(&bus);
                }
                nm::devices(&bus, args["accessPoints"] == true).map_err(|_| nm::unavailable(lang))
            }
            "net.device.setEnabled" => {
                let name = iface_arg(&args["iface"], lang)?;
                let on = bool_arg(&args["on"], lang)?;
                if !dry {
                    let bus = nm::bus(lang)?;
                    let dev = nm::device_of(&bus, &name).ok_or_else(|| nm::no_device(lang))?;
                    if on {
                        nm::activate(&bus, "/", &dev, lang)?;
                    } else {
                        nm::disconnect(&bus, &dev, lang)?;
                    }
                    self.changed("devices");
                }
                Ok(json!({ "iface": name, "on": on }))
            }
            "net.wifi.hidden" => self.hidden(args, lang, dry),
            "net.wifi.secrets" => {
                let uuid = uuid_arg(&args["uuid"], lang)?;
                wifi::secrets(&nm::bus(lang)?, &uuid, lang)
            }
            "net.radios.set" => {
                let (wifi, wwan) = (
                    opt_bool(&args["wifi"], lang)?,
                    opt_bool(&args["wwan"], lang)?,
                );
                if wifi.is_none() && wwan.is_none() {
                    return Err(bad(lang));
                }
                if !dry {
                    let bus = nm::bus(lang)?;
                    if let Some(on) = wifi {
                        nm::set_switch(&bus, "WirelessEnabled", on, lang)?;
                    }
                    if let Some(on) = wwan {
                        nm::set_switch(&bus, "WwanEnabled", on, lang)?;
                    }
                    self.changed("radios");
                }
                Ok(json!({ "wifi": wifi, "wwan": wwan }))
            }
            "net.connectivity.set" => {
                let on = bool_arg(&args["on"], lang)?;
                if !dry {
                    nm::set_switch(&nm::bus(lang)?, "ConnectivityCheckEnabled", on, lang)?;
                    self.changed("connectivity");
                }
                Ok(json!({ "on": on }))
            }
            "net.hotspot.get" => {
                let secrets = opt_bool(&args["secrets"], lang)?.unwrap_or(false);
                hotspot::get(&nm::bus(lang)?, secrets, lang)
            }
            "net.hotspot.start" => {
                let a = hotspot::start_args(args, lang)?;
                if dry {
                    return Ok(json!({ "ssid": a.ssid, "band": a.band, "active": a.activate }));
                }
                let r = hotspot::start(&nm::bus(lang)?, &a, &new_uuid(), lang)?;
                self.changed("hotspot");
                Ok(r)
            }
            "net.hotspot.stop" => {
                if dry {
                    return Ok(json!({ "active": false }));
                }
                let r = hotspot::stop(&nm::bus(lang)?, lang)?;
                self.changed("hotspot");
                Ok(r)
            }
            "net.vpn.plugins" => {
                let dirs: Vec<&std::path::Path> =
                    vpn::DIRS.iter().map(std::path::Path::new).collect();
                Ok(vpn::plugins(&dirs))
            }
            "net.vpn.import" => {
                let (path, kind) = vpn::import_args(args, lang)?;
                if dry {
                    return Ok(json!({ "type": kind }));
                }
                let r = vpn::import(&path, kind, lang)?;
                self.changed("connections");
                Ok(r)
            }
            "net.modem.get" => Ok(modem::get(&system_bus(lang)?)),
            "net.modem.apns" => {
                let (mcc, mnc) = (s_arg(&args["mcc"]), s_arg(&args["mnc"]));
                if !providers::code_ok(mcc, true) || !providers::code_ok(mnc, false) {
                    return Err(bad(lang));
                }
                let xml = std::fs::metadata(providers::FILE)
                    .ok()
                    .filter(|m| m.len() <= 16 * 1024 * 1024)
                    .and_then(|_| std::fs::read_to_string(providers::FILE).ok())
                    .unwrap_or_default();
                Ok(providers::apns(&xml, mcc, mnc))
            }
            "net.modem.scan" => {
                let m = modem_arg(&args["modem"], lang)?;
                modem::scan(&system_bus(lang)?, &m, lang)
            }
            "net.modem.unlock"
            | "net.modem.setPin"
            | "net.modem.changePin"
            | "net.modem.setModes"
            | "net.modem.register"
            | "net.modem.setPrimarySlot" => self.modem_write(method, args, lang, dry),
            _ => Err(bad(lang)),
        }
    }

    /// The ModemManager writes: every argument checked first, nothing sent under `ctx.dry_run`.
    fn modem_write(&self, method: &str, args: &Value, lang: Lang, dry: bool) -> ApiResult {
        let m = modem_arg(&args["modem"], lang)?;
        type Write = Box<dyn FnOnce(&gio::DBusConnection) -> Result<(), ApiError>>;
        let write: Write = match method {
            "net.modem.unlock" => {
                let puk = match &args["puk"] {
                    Value::Null => None,
                    v => Some(code_arg(v, true, lang)?),
                };
                let pin = match puk {
                    Some(_) => code_arg(&args["newPin"], false, lang)?,
                    None => code_arg(&args["pin"], false, lang)?,
                };
                let m = m.clone();
                Box::new(move |bus| modem::unlock(bus, &m, &pin, puk.as_deref(), lang))
            }
            "net.modem.setPin" => {
                let enabled = bool_arg(&args["enabled"], lang)?;
                let pin = code_arg(&args["pin"], false, lang)?;
                let m = m.clone();
                Box::new(move |bus| modem::enable_pin(bus, &m, &pin, enabled, lang))
            }
            "net.modem.changePin" => {
                let old = code_arg(&args["old"], false, lang)?;
                let new = code_arg(&args["new"], false, lang)?;
                let m = m.clone();
                Box::new(move |bus| modem::change_pin(bus, &m, &old, &new, lang))
            }
            "net.modem.setModes" => {
                let (allowed, preferred) = (
                    u32_arg(&args["allowed"], lang)?,
                    u32_arg(&args["preferred"], lang)?,
                );
                if !modem::modes_ok(allowed as u64, preferred as u64) {
                    return Err(bad(lang));
                }
                let m = m.clone();
                Box::new(move |bus| modem::set_modes(bus, &m, allowed, preferred, lang))
            }
            "net.modem.register" => {
                let operator = match &args["operator"] {
                    Value::Null => String::new(),
                    Value::String(o) if modem::operator_ok(o) => o.clone(),
                    _ => return Err(bad(lang)),
                };
                let m = m.clone();
                Box::new(move |bus| modem::register(bus, &m, &operator, lang))
            }
            _ => {
                let slot = u32_arg(&args["slot"], lang)?;
                if slot == 0 {
                    return Err(bad(lang));
                }
                let m = m.clone();
                Box::new(move |bus| modem::set_primary_slot(bus, &m, slot, lang))
            }
        };
        if !dry {
            write(&system_bus(lang)?)?;
            self.changed("modem");
        }
        Ok(json!({ "modem": m }))
    }

    /// net.connection.update {uuid, settings, reapply?}: NM's settings with the page's changes merged in (every other
    /// key kept), saved with Update2; `reapply` (default true) applies an active profile's change at once
    /// (Device.Reapply, or a reconnect when the device cannot reapply it: Windows applies its properties at once).
    fn update(&self, args: &Value, lang: Lang, dry: bool) -> ApiResult {
        let uuid = uuid_arg(&args["uuid"], lang)?;
        let changes = &args["settings"];
        if !changes.is_object() {
            return Err(bad(lang));
        }
        if dry {
            // the shape and the types of the change only: no bus, nothing written
            let mut probe = settings_map::Groups::new();
            let connection = [("id", "x"), ("type", "x")]
                .map(|(k, v)| (k.to_owned(), glib::variant::ToVariant::to_variant(v)));
            probe.insert("connection".into(), connection.into_iter().collect());
            settings_map::merge(&mut probe, changes).map_err(|e| map_error(e, lang))?;
            return Ok(json!({ "uuid": uuid }));
        }
        let bus = nm::bus(lang)?;
        let path = nm::path_of(&bus, &uuid, lang)?;
        let mut base = nm::base_for_update(&bus, &path, changes, lang)?;
        settings_map::merge(&mut base, changes).map_err(|e| map_error(e, lang))?;
        nm::update2(&bus, &path, &base, lang)?;
        let mut reapplied = false;
        if args["reapply"] != false {
            for (_, a) in nm::actives(&bus)
                .iter()
                .filter(|(_, a)| a["Uuid"] == json!(uuid))
            {
                for dev in a["Devices"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                {
                    reapplied = nm::reapply(&bus, dev, lang)
                        || nm::activate(&bus, &path, dev, lang).is_ok();
                }
            }
        }
        self.changed("connections");
        Ok(json!({ "uuid": uuid, "reapplied": reapplied }))
    }

    /// net.connection.add {settings}: a new profile (AddConnection2, to disk); a uuid is made when it has none.
    fn add(&self, args: &Value, lang: Lang, dry: bool) -> ApiResult {
        let mut settings = args["settings"].clone();
        if !settings.is_object() || !settings["connection"].is_object() {
            return Err(bad(lang));
        }
        if settings["connection"]["uuid"].is_null() {
            settings["connection"]["uuid"] = json!(new_uuid());
        }
        let uuid = uuid_arg(&settings["connection"]["uuid"], lang)?;
        let groups = settings_map::build(&settings).map_err(|e| map_error(e, lang))?;
        if dry {
            return Ok(json!({ "uuid": uuid }));
        }
        let bus = nm::bus(lang)?;
        let path = nm::add(&bus, &groups, lang)?;
        self.changed("connections");
        Ok(json!({ "uuid": uuid, "path": path }))
    }

    /// net.wifi.hidden {ssid, security, secrets?, autoconnect?, iface?}: a hidden network added and connected
    /// (AddAndActivateConnection2 with 802-11-wireless.hidden, as GNOME's Connect to Hidden Network).
    fn hidden(&self, args: &Value, lang: Lang, dry: bool) -> ApiResult {
        let settings = wifi::hidden_settings(args, &new_uuid()).ok_or_else(|| bad(lang))?;
        let uuid = settings["connection"]["uuid"]
            .as_str()
            .unwrap_or("")
            .to_owned();
        let groups = settings_map::build(&settings).map_err(|e| map_error(e, lang))?;
        let name = match &args["iface"] {
            Value::Null => None,
            v => Some(iface_arg(v, lang)?),
        };
        if dry {
            return Ok(json!({ "uuid": uuid }));
        }
        let bus = nm::bus(lang)?;
        let dev = wifi::wifi_device(&bus, name.as_deref())
            .ok_or_else(|| ApiError::tr("not_found", lang, "net.noWifi", NO_PARAMS))?;
        let (path, active) = nm::add_and_activate(&bus, &groups, &dev, lang)?;
        self.changed("connections");
        Ok(json!({ "uuid": uuid, "path": path, "active": active }))
    }
}

/// What a NetworkManager signal changed, by its object path.
pub(crate) fn signal_kind(path: &str) -> &'static str {
    let rest = path.strip_prefix(nm::PATH).unwrap_or("");
    let under = |p: &str| rest.starts_with(p);
    if under("/Settings") {
        "connections"
    } else if under("/ActiveConnection") {
        "active"
    } else if under("/AccessPoint") {
        "accessPoints"
    } else if under("/Devices") || under("/IP4Config") || under("/IP6Config") || under("/DHCP") {
        "devices"
    } else {
        "state"
    }
}

/// The watcher: NetworkManager's signals become `net.changed {what}` while someone listens (at most every 400 ms per
/// kind; access points, whose strength changes every few seconds, every 5 s).
fn watch(weak: Weak<Net>, stopped: Arc<AtomicBool>) {
    let context = glib::MainContext::new();
    let _ = context.with_thread_default(|| {
        let Ok(bus) = crate::services::system::common::system() else {
            return;
        };
        let dirty: Arc<Mutex<BTreeSet<&'static str>>> = Arc::default();
        let sink = dirty.clone();
        let sub = bus.signal_subscribe(
            Some(nm::NM),
            None,
            None,
            None,
            None,
            gio::DBusSignalFlags::NONE,
            move |_, _, path, _, _, _| {
                sink.lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(signal_kind(path));
            },
        );
        // ModemManager's signals (state, signal quality, SIM): `modem`, at most every 5 s
        let modem_sink = dirty.clone();
        let modem_sub = bus.signal_subscribe(
            Some(modem::MM),
            None,
            None,
            None,
            None,
            gio::DBusSignalFlags::NONE,
            move |_, _, _, _, _, _| {
                modem_sink
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert("modem");
            },
        );
        let mut last_aps = Instant::now() - Duration::from_secs(10);
        let mut last_modem = last_aps;
        let mut last = Instant::now();
        while !stopped.load(Ordering::Acquire) {
            while context.pending() {
                context.iteration(false);
            }
            if last.elapsed() >= Duration::from_millis(400) {
                last = Instant::now();
                let Some(net) = weak.upgrade() else { break };
                let kinds = std::mem::take(&mut *dirty.lock().unwrap_or_else(|e| e.into_inner()));
                if net.ctx.subscriber_count() > 0 {
                    for kind in kinds {
                        let throttled = match kind {
                            "accessPoints" => Some(&mut last_aps),
                            "modem" => Some(&mut last_modem),
                            _ => None,
                        };
                        if let Some(at) = throttled {
                            if at.elapsed() < Duration::from_secs(5) {
                                dirty.lock().unwrap_or_else(|e| e.into_inner()).insert(kind);
                                continue;
                            }
                            *at = Instant::now();
                        }
                        net.changed(kind);
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        bus.signal_unsubscribe(sub);
        bus.signal_unsubscribe(modem_sub);
    });
}
