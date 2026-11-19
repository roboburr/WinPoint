//! BlueZ state, discovery, temporary-agent pairing and adapter recovery.
use super::{
    agent::PairAgent,
    audio::{Audio, property},
    common::*,
    network::radios,
    text, text_error,
};
use crate::{
    files::{ApiError, ApiResult},
    services::ServiceContext,
};
use glib::variant::ToVariant;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs::OpenOptions,
    io::Write,
    sync::{
        Arc, Condvar, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
const BZ: &str = "org.bluez";
const ADAPTER: &str = "org.bluez.Adapter1";
const DEVICE: &str = "org.bluez.Device1";
struct Pairing {
    address: String,
    path: Option<String>,
    agent: Option<Arc<PairAgent>>,
    cancelled: bool,
    stage: String,
    new_bond: bool,
}
#[derive(Default)]
struct State {
    transition: Option<(String, Instant)>,
    last_state: Option<String>,
    notready: bool,
    wanted: bool,
    active: bool,
    adapter: Option<String>,
    deadline: Option<Instant>,
    nearby_until: Option<Instant>,
    seen: BTreeMap<String, Instant>,
    pairing: Option<Pairing>,
}
#[derive(Default)]
struct Reset {
    active: bool,
    error: Option<ApiError>,
}
pub(super) struct Bluetooth {
    ctx: Arc<ServiceContext>,
    audio: Arc<Audio>,
    state: Mutex<State>,
    pair_lock: Mutex<()>,
    reset: Mutex<Reset>,
    reset_done: Condvar,
    stopped: AtomicBool,
}
impl Bluetooth {
    pub fn new(ctx: Arc<ServiceContext>, audio: Arc<Audio>) -> Arc<Self> {
        let b = Arc::new(Self {
            ctx,
            audio,
            state: Mutex::new(State::default()),
            pair_lock: Mutex::new(()),
            reset: Mutex::new(Reset::default()),
            reset_done: Condvar::new(),
            stopped: AtomicBool::new(false),
        });
        let weak = Arc::downgrade(&b);
        thread::spawn(move || {
            loop {
                thread::sleep(Duration::from_millis(100));
                let Some(b) = weak.upgrade() else { break };
                if b.stopped.load(Ordering::Relaxed) {
                    break;
                }
                let expired = {
                    let s = b.state.lock().unwrap_or_else(|e| e.into_inner());
                    s.wanted && s.deadline.is_some_and(|at| Instant::now() >= at)
                };
                if expired {
                    b.stop_discovery();
                    b.notify();
                }
            }
        });
        b
    }
    fn unavailable(blocked: bool) -> Value {
        json!({"available":false,"powered":false,"powerState":"off","stuck":false,"discovering":false,"name":"","blocked":blocked,"devices":[],"nearby":[]})
    }
    /// A state of [`Self::get`] with gsd's flight mode (amendment 57): `airplane` (Rfkill AirplaneMode, the
    /// "Vliegtuigmodus staat aan" of the page) and `hardwareAirplane` (BluetoothHardwareAirplaneMode, the switch on
    /// the computer), both null without gsd's Rfkill.
    fn with_gsd(mut state: Value) -> Value {
        let gsd = session().ok().and_then(|bus| gsd_rfkill(&bus, RFKILL));
        state["airplane"] = gsd
            .as_ref()
            .map_or(Value::Null, |p| json!(truth(&p["AirplaneMode"])));
        state["hardwareAirplane"] = gsd.as_ref().map_or(Value::Null, |p| {
            json!(truth(&p["BluetoothHardwareAirplaneMode"]))
        });
        state
    }
    fn objects() -> Result<Value, String> {
        let bus = system()?;
        call(
            &bus,
            BZ,
            "/",
            "org.freedesktop.DBus.ObjectManager",
            "GetManagedObjects",
            None,
            2000,
        )
        .map(|v| first(&v))
    }
    fn adapter(objects: &Value) -> Option<(String, Value)> {
        let mut paths = objects
            .as_object()?
            .iter()
            .filter(|(_, v)| v[ADAPTER].is_object())
            .collect::<Vec<_>>();
        paths.sort_by_key(|(k, _)| *k);
        paths
            .first()
            .map(|(k, v)| (k.to_string(), v[ADAPTER].clone()))
    }
    fn blocked() -> (bool, bool) {
        let radios = radios();
        let bt = radios
            .iter()
            .filter(|r| r["type"] == "bluetooth")
            .collect::<Vec<_>>();
        (
            bt.iter().any(|r| truth(&r["soft"]) || truth(&r["hard"])),
            bt.iter().any(|r| truth(&r["hard"])),
        )
    }
    pub fn get(&self) -> Value {
        let (blocked, _) = Self::blocked();
        let Ok(objects) = Self::objects() else {
            return Self::with_gsd(Self::unavailable(blocked));
        };
        let Some((path, props)) = Self::adapter(&objects) else {
            return Self::with_gsd(Self::unavailable(blocked));
        };
        let now = Instant::now();
        let powered = truth(&props["Powered"]);
        let power_state = power_state(&props, blocked);
        let discovering = truth(&props["Discovering"]);
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.last_state.as_deref() != Some(&power_state) {
            if state.last_state.is_some() {
                state.notready = false
            }
            state.last_state = Some(power_state.clone())
        }
        if matches!(power_state.as_str(), "on-disabling" | "off-enabling") {
            if state
                .transition
                .as_ref()
                .is_none_or(|(s, _)| s != &power_state)
            {
                state.transition = Some((power_state.clone(), now));
            }
        } else {
            state.transition = None
        }
        let stuck = state
            .transition
            .as_ref()
            .is_some_and(|(_, at)| at.elapsed() > Duration::from_secs(5))
            || (powered && state.notready);
        if discovering || state.wanted {
            let wanted = now + Duration::from_secs(60);
            state.nearby_until = Some(
                state
                    .nearby_until
                    .map(|old| old.max(wanted))
                    .unwrap_or(wanted),
            );
        }
        let nearby =
            powered && (discovering || state.wanted || state.nearby_until.is_some_and(|t| now < t));
        if !nearby {
            state.seen.clear()
        }
        let mut devices = vec![];
        let mut found = vec![];
        for (p, ifs) in objects.as_object().into_iter().flatten() {
            let d = &ifs[DEVICE];
            if !d.is_object() || !p.starts_with(&(path.clone() + "/")) {
                continue;
            }
            let address = string(&d["Address"]);
            let kind = kind(d);
            if truth(&d["Paired"]) || truth(&d["Connected"]) || truth(&d["Bonded"]) {
                let mut item = json!({"address":address,"name":friendly_name(d).unwrap_or_else(||address.clone()),"icon":d["Icon"].as_str().filter(|s|!s.is_empty()).unwrap_or("bluetooth"),"paired":truth(&d["Paired"])||truth(&d["Bonded"]),"connected":truth(&d["Connected"]),"kind":kind});
                if !ifs["org.bluez.Battery1"]["Percentage"].is_null() {
                    item["battery"] =
                        json!(integer(&ifs["org.bluez.Battery1"]["Percentage"]).unwrap_or(0))
                }
                devices.push(item);
                continue;
            }
            if !nearby {
                continue;
            }
            let Some(name) = friendly_name(d) else {
                continue;
            };
            let rssi = if d["RSSI"].is_number() {
                integer(&d["RSSI"])
            } else {
                None
            };
            if rssi.is_some() {
                state.seen.insert(address.to_uppercase(), now);
            } else if !state.seen.contains_key(&address.to_uppercase()) {
                continue;
            }
            found.push(json!({"address":address,"name":name,"icon":d["Icon"].as_str().filter(|s|!s.is_empty()).unwrap_or("bluetooth"),"kind":kind,"rssi":rssi}));
        }
        devices.sort_by(|a, b| {
            (!truth(&a["connected"]))
                .cmp(&!truth(&b["connected"]))
                .then_with(|| {
                    string(&a["name"])
                        .to_lowercase()
                        .cmp(&string(&b["name"]).to_lowercase())
                })
        });
        found.sort_by(|a, b| {
            a["rssi"]
                .is_null()
                .cmp(&b["rssi"].is_null())
                .then_with(|| {
                    integer(&b["rssi"])
                        .unwrap_or(0)
                        .cmp(&integer(&a["rssi"]).unwrap_or(0))
                })
                .then_with(|| {
                    string(&a["name"])
                        .to_lowercase()
                        .cmp(&string(&b["name"]).to_lowercase())
                })
        });
        Self::with_gsd(
            json!({"available":true,"powered":powered,"powerState":power_state,"stuck":stuck,"discovering":discovering,"name":props["Alias"].as_str().filter(|s|!s.is_empty()).or_else(||props["Name"].as_str()).unwrap_or(""),"blocked":blocked,"devices":devices,"nearby":found}),
        )
    }
    fn adapter_or_error(&self) -> Result<(String, Value), ApiError> {
        let objs =
            Self::objects().map_err(|_| text_error(&self.ctx, "unavailable", "bt_unavailable"))?;
        Self::adapter(&objs).ok_or_else(|| text_error(&self.ctx, "unavailable", "bt_no_adapter"))
    }
    fn check(&self, address: &Value) -> Result<String, ApiError> {
        address
            .as_str()
            .filter(|s| valid_address(s, ':'))
            .map(str::to_uppercase)
            .ok_or_else(|| text_error(&self.ctx, "bad_args", "bt_bad_device"))
    }
    fn find(objects: &Value, address: &str) -> Option<String> {
        let mut pairs = objects.as_object()?.iter().collect::<Vec<_>>();
        pairs.sort_by_key(|(p, _)| *p);
        pairs
            .into_iter()
            .find(|(_, i)| {
                i[DEVICE].is_object() && string(&i[DEVICE]["Address"]).eq_ignore_ascii_case(address)
            })
            .map(|(p, _)| p.clone())
    }
    fn device_path(&self, address: &Value) -> Result<String, ApiError> {
        let address = self.check(address)?;
        let objs =
            Self::objects().map_err(|_| text_error(&self.ctx, "unavailable", "bt_unavailable"))?;
        Self::find(&objs, &address)
            .ok_or_else(|| text_error(&self.ctx, "not_found", "bt_not_found"))
    }
    fn props(path: &str) -> Value {
        Self::objects()
            .map(|o| o[path][DEVICE].clone())
            .unwrap_or(json!({}))
    }
    fn powered(&self) -> bool {
        Self::objects()
            .ok()
            .and_then(|o| Self::adapter(&o))
            .map(|(_, p)| truth(&p["Powered"]))
            .unwrap_or(true)
    }
    fn note_error(&self, error: &str, powered: bool) {
        if powered && error.contains("NotReady") {
            self.state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .notready = true;
            self.notify()
        }
    }
    fn note_ok(&self) {
        let had = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            std::mem::replace(&mut state.notready, false)
        };
        if had {
            self.notify()
        }
    }
    fn stuck(&self) -> bool {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.notready
            || s.transition
                .as_ref()
                .is_some_and(|(_, t)| t.elapsed() > Duration::from_secs(5))
    }
    fn notify(&self) {
        self.ctx.emit("system.bluetooth.changed", self.get());
    }
    fn setprop(
        path: &str,
        iface: &str,
        key: &str,
        value: glib::Variant,
        timeout: i32,
    ) -> Result<(), String> {
        setprop(&system()?, BZ, path, iface, key, value, timeout)
    }
    pub fn set_powered(&self, on: bool) -> ApiResult {
        if self.set_powered_gsd(on)? {
            thread::sleep(Duration::from_millis(200));
            return Ok(self.get());
        }
        let (path, props) = self.adapter_or_error()?;
        if !on {
            self.stop_discovery()
        }
        for attempt in 0..2 {
            match Self::setprop(&path, ADAPTER, "Powered", on.to_variant(), 5000) {
                Ok(()) => break,
                Err(error) => {
                    if attempt == 0 && on && error.contains("Blocked") && !Self::blocked().1 {
                        rfkill(false);
                        thread::sleep(Duration::from_millis(400));
                        continue;
                    }
                    let was = truth(&props["Powered"]);
                    self.note_error(&error, was);
                    let (code, key) = if was && (error.contains("NotReady") || self.stuck()) {
                        ("not_ready", "bt_not_responding")
                    } else if error.contains("Blocked") {
                        ("blocked", "bt_hard_blocked")
                    } else if error.contains("Error.Busy") || error.contains("InProgress") {
                        ("busy", "bt_busy")
                    } else {
                        ("failed", "bt_power_failed")
                    };
                    return Err(text_error(&self.ctx, code, key));
                }
            }
        }
        self.note_ok();
        thread::sleep(Duration::from_millis(200));
        Ok(self.get())
    }
    /// Amendment 57: Bluetooth on or off as GNOME Settings switches it, through gsd's Rfkill (BluetoothAirplaneMode
    /// = !on), so the state is GNOME's own after the next login too. Ok(true) when that did it all; Ok(false) to go on
    /// with BlueZ's Powered and rfkill as before: no gsd (Hyprland, the sandboxes), Bluetooth not blocked by gsd (the
    /// adapter was only powered off), gsd refused the write, or the adapter stayed off after gsd unblocked it.
    fn set_powered_gsd(&self, on: bool) -> Result<bool, ApiError> {
        let Ok(bus) = session() else {
            return Ok(false);
        };
        let props = gsd_rfkill(&bus, RFKILL);
        let step = gsd_step(props.as_ref(), on);
        if !on && step != GsdStep::None {
            self.stop_discovery()
        }
        match step {
            GsdStep::None => Ok(false),
            GsdStep::HardBlocked => Err(text_error(&self.ctx, "blocked", "bt_hard_blocked")),
            GsdStep::Already => Ok(!on
                && Self::objects()
                    .ok()
                    .and_then(|o| Self::adapter(&o))
                    .is_none_or(|(_, p)| !truth(&p["Powered"]))),
            GsdStep::Write(airplane) => {
                if gsd_write(&bus, RFKILL, airplane).is_err() {
                    return Ok(false);
                }
                // gsd blocks or unblocks the radio; BlueZ follows (an unblocked adapter is powered when BlueZ
                // does not do it by itself)
                Ok(self.wait_power(4, on))
            }
        }
    }
    fn start_session(&self, path: &str, props: &Value) -> Result<(), String> {
        let bus = system()?;
        let transport = if props["Roles"].as_array().is_some_and(|a| a.is_empty()) {
            "bredr"
        } else {
            "auto"
        };
        let filter = BTreeMap::from([
            ("Transport".to_owned(), transport.to_variant()),
            ("DuplicateData".to_owned(), false.to_variant()),
        ]);
        let _ = call(
            &bus,
            BZ,
            path,
            ADAPTER,
            "SetDiscoveryFilter",
            Some(&(filter,).to_variant()),
            3000,
        );
        if let Err(e) = call(&bus, BZ, path, ADAPTER, "StartDiscovery", None, 5000) {
            if !e.contains("InProgress") && !e.contains("Error.Busy") {
                return Err(e);
            }
        }
        let still = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state.active = true;
            state.adapter = Some(path.into());
            state.wanted
        };
        if !still {
            self.stop_session()
        }
        Ok(())
    }
    fn stop_session(&self) {
        let path = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if std::mem::replace(&mut state.active, false) {
                state.adapter.clone()
            } else {
                None
            }
        };
        if let Some(path) = path {
            if let Ok(bus) = system() {
                let _ = call(&bus, BZ, &path, ADAPTER, "StopDiscovery", None, 3000);
            }
        }
    }
    fn stop_discovery(&self) {
        {
            let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let was = s.wanted || s.active;
            s.wanted = false;
            s.deadline = None;
            if was {
                s.nearby_until = Some(Instant::now() + Duration::from_secs(60));
            }
        }
        self.stop_session()
    }
    pub fn discover(&self, on: bool) -> ApiResult {
        if !on {
            self.stop_discovery();
            self.notify();
            return Ok(json!({"ok":true}));
        }
        let (path, props) = self.adapter_or_error()?;
        if !truth(&props["Powered"]) {
            return Ok(self.failure("bt_off", None));
        }
        let start = {
            let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
            s.wanted = true;
            s.deadline = Some(Instant::now() + Duration::from_secs(60));
            if s.active && (s.adapter.as_deref() != Some(&path) || !truth(&props["Discovering"])) {
                s.active = false
            }
            !s.active && s.pairing.is_none()
        };
        if start {
            if let Err(e) = self.start_session(&path, &props) {
                let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
                s.wanted = false;
                s.deadline = None;
                drop(s);
                let powered = truth(&props["Powered"]);
                self.note_error(&e, powered);
                return Ok(self.failure(error_key(&e, "discover", powered), None));
            }
            self.note_ok()
        }
        self.notify();
        Ok(json!({"ok":true}))
    }
    fn failure(&self, key: &str, cancelled: Option<bool>) -> Value {
        let mut result = json!({"ok":false,"error":text(&self.ctx,key),"code":error_code(key)});
        if let Some(b) = cancelled {
            result["cancelled"] = json!(b)
        }
        result
    }
    fn emit_pair(&self, address: &str, stage: &str, extra: Value) {
        let mut data = json!({"address":address,"stage":stage});
        if let (Some(d), Some(extra)) = (data.as_object_mut(), extra.as_object()) {
            for (k, v) in extra {
                if !v.is_null() {
                    d.insert(k.clone(), v.clone());
                }
            }
        }
        self.ctx.emit("system.bluetooth.pairing", data)
    }
    fn pair_end(&self, address: &str, stage: &str, key: &str) -> Value {
        self.emit_pair(
            address,
            stage,
            json!({"error":text(&self.ctx,key),"code":error_code(key)}),
        );
        self.failure(key, Some(stage == "cancelled"))
    }
    fn pair_agent(&self, address: &str) -> Option<Arc<PairAgent>> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pairing
            .as_ref()
            .filter(|p| p.address.eq_ignore_ascii_case(address))
            .and_then(|p| p.agent.clone())
    }
    fn pair_slot(&self, address: &str) -> Option<MutexGuard<'_, ()>> {
        if let Ok(guard) = self.pair_lock.try_lock() {
            return Some(guard);
        }
        let stage = {
            let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
            s.pairing
                .as_ref()
                .filter(|p| {
                    p.address == address
                        && !self.reset.lock().unwrap_or_else(|e| e.into_inner()).active
                })
                .map(|p| p.stage.clone())
        }?;
        let connecting = matches!(stage.as_str(), "connecting" | "done");
        if !connecting {
            let _ = self.cancel_pair(&json!(address));
        }
        lock_until(
            &self.pair_lock,
            Duration::from_secs(if connecting { 60 } else { 10 }),
        )
    }
    fn cancelled(&self, stage: Option<&str>) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(p) = state.pairing.as_mut() else {
            return true;
        };
        if p.cancelled {
            return true;
        }
        if let Some(stage) = stage {
            p.stage = stage.into()
        }
        false
    }
    pub fn pair(self: &Arc<Self>, address: &Value) -> ApiResult {
        let addr = self.check(address)?;
        let Some(_guard) = self.pair_slot(&addr) else {
            let key = if self.reset.lock().unwrap_or_else(|e| e.into_inner()).active {
                "bt_busy"
            } else {
                "bt_busy_pairing"
            };
            return Ok(self.failure(key, Some(false)));
        };
        self.state.lock().unwrap_or_else(|e| e.into_inner()).pairing = Some(Pairing {
            address: addr.clone(),
            path: None,
            agent: None,
            cancelled: false,
            stage: "setup".into(),
            new_bond: false,
        });
        let start = Instant::now();
        let mut agent: Option<Arc<PairAgent>> = None;
        let mut pairable: Option<String> = None;
        let mut success = false;
        let result = (|| -> Value {
            self.emit_pair(&addr, "pairing", json!({}));
            let Ok(objects) = Self::objects() else {
                return self.pair_end(&addr, "failed", "bt_unavailable");
            };
            let Some(path) = Self::find(&objects, &addr) else {
                return self.pair_end(&addr, "failed", "bt_gone");
            };
            let apath = path.rsplit_once('/').map(|(p, _)| p).unwrap_or("/");
            if !truth(&objects[apath][ADAPTER]["Powered"]) {
                return self.pair_end(&addr, "failed", "bt_off");
            }
            if let Some(p) = self
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .pairing
                .as_mut()
            {
                p.path = Some(path.clone())
            }
            self.stop_session();
            let new_agent = match PairAgent::start(self.ctx.clone(), &addr, &path) {
                Ok(a) => a,
                Err(_) => return self.pair_end(&addr, "failed", "bt_pair_failed"),
            };
            agent = Some(new_agent.clone());
            if let Some(p) = self
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .pairing
                .as_mut()
            {
                p.agent = Some(new_agent.clone())
            }
            if self.cancelled(None) {
                return self.pair_end(&addr, "cancelled", "bt_canceled");
            }
            if ensure_pairable(apath) {
                pairable = Some(apath.into())
            }
            let had_bond = truth(&objects[&path][DEVICE]["Paired"])
                || truth(&objects[&path][DEVICE]["Bonded"]);
            if self.cancelled(Some("pair")) {
                return self.pair_end(&addr, "cancelled", "bt_canceled");
            }
            let paired = system().and_then(|b| call(&b, BZ, &path, DEVICE, "Pair", None, 45000));
            let new_bond = match paired {
                Ok(_) => !had_bond,
                Err(e) if e.contains("AlreadyExists") => false,
                Err(e) => {
                    let cancelled =
                        self.cancelled(None) || new_agent.declined.load(Ordering::Relaxed);
                    let key = if cancelled {
                        "bt_canceled"
                    } else if new_agent.timed_out.load(Ordering::Relaxed) {
                        "bt_reply_timeout"
                    } else {
                        error_key(&e, "pair", self.powered())
                    };
                    self.note_error(&e, self.powered());
                    if e.contains("Timeout was reached") || e.contains("NoReply") {
                        device_call(&path, "CancelPairing", 3000);
                    }
                    return self.pair_end(
                        &addr,
                        if key == "bt_canceled" {
                            "cancelled"
                        } else {
                            "failed"
                        },
                        key,
                    );
                }
            };
            self.note_ok();
            if let Some(p) = self
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .pairing
                .as_mut()
            {
                p.new_bond = new_bond
            }
            if self.cancelled(Some("connecting")) {
                return self.undo_pair(&addr, &path, new_bond, false);
            }
            let _ = Self::setprop(&path, DEVICE, "Trusted", true.to_variant(), 3000);
            self.emit_pair(&addr, "connecting", json!({}));
            if self.cancelled(None) {
                return self.undo_pair(&addr, &path, new_bond, false);
            }
            let left = (60f64 - start.elapsed().as_secs_f64()).max(10.);
            let connect = system().and_then(|b| {
                call(
                    &b,
                    BZ,
                    &path,
                    DEVICE,
                    "Connect",
                    None,
                    (left * 1000.) as i32,
                )
            });
            let connected = match connect.as_ref() {
                Ok(_) => true,
                Err(e) => e.contains("AlreadyConnected"),
            };
            if self.cancelled(Some("done")) {
                return self.undo_pair(&addr, &path, new_bond, connected);
            }
            let mut res = json!({"ok":true,"paired":true,"connected":connected});
            if !connected {
                res["error"] = json!(text(&self.ctx, "bt_paired_not_connected"));
                res["code"] = json!("connect_failed");
            }
            success = true;
            self.emit_pair(
                &addr,
                "done",
                json!({"connected":connected,"error":res["error"],"code":res["code"]}),
            );
            if connected {
                self.follow_audio(&addr, kind(&Self::props(&path)));
            }
            res
        })();
        if let Some(a) = agent {
            a.stop()
        }
        if let Some(apath) = pairable {
            let _ = Self::setprop(&apath, ADAPTER, "Pairable", false.to_variant(), 3000);
        }
        let resume = {
            let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
            s.pairing = None;
            !success && s.wanted && s.deadline.is_some_and(|t| Instant::now() < t)
        };
        drop(_guard);
        if resume {
            if let Ok((p, props)) = self.adapter_or_error() {
                if truth(&props["Powered"]) {
                    let _ = self.start_session(&p, &props);
                }
            }
        }
        self.notify();
        Ok(result)
    }
    fn undo_pair(&self, address: &str, path: &str, new_bond: bool, connected: bool) -> Value {
        if new_bond {
            forget_bond(path)
        } else if connected {
            device_call(path, "Disconnect", 5000)
        }
        self.pair_end(address, "cancelled", "bt_canceled")
    }
    pub fn pair_reply(&self, args: &Value) -> ApiResult {
        let addr = self.check(&args["address"])?;
        let Some(agent) = self.pair_agent(&addr) else {
            return Ok(json!({"ok":false,"error":text(&self.ctx,"bt_no_request")}));
        };
        agent.reply(
            super::bool_arg_exact(&args["accept"], self.ctx.lang())?,
            &args["passkey"],
            &args["pin"],
            &args["qid"],
        )
    }
    pub fn cancel_pair(&self, address: &Value) -> ApiResult {
        let addr = self.check(address)?;
        let current = {
            let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
            s.pairing
                .as_mut()
                .filter(|p| p.address == addr && p.stage != "done")
                .map(|p| {
                    p.cancelled = true;
                    (p.path.clone(), p.agent.clone(), p.stage.clone())
                })
        };
        if let Some((path, agent, stage)) = current {
            if let Some(agent) = agent.as_ref() {
                agent.cancel()
            }
            if let Some(path) = path {
                if stage == "connecting" {
                    device_call(&path, "Disconnect", 5000)
                } else if agent.is_some() {
                    device_call(&path, "CancelPairing", 3000)
                }
            }
        }
        Ok(json!({"ok":true}))
    }
    pub fn remove(&self, address: &Value) -> ApiResult {
        let addr = self.check(address)?;
        let path = match self.device_path(&json!(addr)) {
            Ok(p) => p,
            Err(e) if e.code == "not_found" => return Ok(json!({"ok":true})),
            Err(e) => return Ok(json!({"ok":false,"error":e.message,"code":e.code})),
        };
        if self.pair_agent(&addr).is_some() {
            let _ = self.cancel_pair(&json!(addr));
        }
        let result = system().and_then(|b| {
            call(
                &b,
                BZ,
                path.rsplit_once('/').map(|(a, _)| a).unwrap_or("/"),
                ADAPTER,
                "RemoveDevice",
                Some(&(object_path(&path)?,).to_variant()),
                10000,
            )
        });
        match result {
            Ok(_) => {
                self.note_ok();
                Ok(json!({"ok":true}))
            }
            Err(e) if e.contains("DoesNotExist") || e.contains("UnknownObject") => {
                Ok(json!({"ok":true}))
            }
            Err(e) => {
                let powered = self.powered();
                self.note_error(&e, powered);
                Ok(self.failure(error_key(&e, "remove", powered), None))
            }
        }
    }
    pub fn connect(self: &Arc<Self>, address: &Value, on: bool) -> ApiResult {
        let path = self.device_path(address)?;
        let op = if on { "connect" } else { "disconnect" };
        match system().and_then(|b| {
            call(
                &b,
                BZ,
                &path,
                DEVICE,
                if on { "Connect" } else { "Disconnect" },
                None,
                30000,
            )
        }) {
            Ok(_) => {}
            Err(e) if e.contains("AlreadyConnected") || (!on && e.contains("NotConnected")) => {
                return Ok(json!({"ok":true}));
            }
            Err(e) => {
                let powered = self.powered();
                self.note_error(&e, powered);
                return Ok(self.failure(error_key(&e, op, powered), None));
            }
        }
        self.note_ok();
        if on {
            self.follow_audio(&string(address).to_uppercase(), kind(&Self::props(&path)));
        }
        Ok(json!({"ok":true}))
    }
    fn follow_audio(self: &Arc<Self>, address: &str, kind: &str) {
        if !["headphones", "headset", "speaker"].contains(&kind) || !available("wpctl") {
            return;
        }
        let weak = Arc::downgrade(self);
        let address = address.to_uppercase();
        thread::spawn(move || {
            let end = Instant::now() + Duration::from_secs(12);
            let mut first = None;
            let mut cache = BTreeMap::new();
            while Instant::now() < end {
                let Some(bt) = weak.upgrade() else { break };
                if bt.stopped.load(Ordering::Relaxed) {
                    break;
                }
                let hit = bt.audio.sinks().iter().find_map(|s| {
                    let id = integer(&s["id"])?;
                    let matched = if let Some(b) = cache.get(&id) {
                        *b
                    } else {
                        let ins = run(&["wpctl", "inspect", &id.to_string()], 2, false);
                        if !ins.ok {
                            return None;
                        }
                        let a = property(&ins.out, "api.bluez5.address")
                            .unwrap_or_default()
                            .to_uppercase();
                        let n = property(&ins.out, "node.name")
                            .unwrap_or_default()
                            .to_uppercase();
                        let b = a == address
                            || n.contains(&address)
                            || n.contains(&address.replace(':', "_"));
                        cache.insert(id, b);
                        b
                    };
                    matched.then(|| (id, truth(&s["default"])))
                });
                if let Some((id, default)) = hit {
                    if default {
                        return;
                    }
                    let at = first.get_or_insert_with(Instant::now);
                    if at.elapsed() >= Duration::from_millis(1500) {
                        if let Ok(state) = bt.audio.set_sink(&json!(id), bt.ctx.lang()) {
                            bt.ctx.emit("system.audio.changed", state);
                        }
                        return;
                    }
                }
                drop(bt);
                thread::sleep(Duration::from_millis(400));
            }
        });
    }
    fn wait_power(&self, seconds: u64, on: bool) -> bool {
        let end = Instant::now() + Duration::from_secs(seconds);
        let mut asked: Option<Instant> = None;
        loop {
            if let Ok(o) = Self::objects() {
                if let Some((path, p)) = Self::adapter(&o) {
                    let state = power_state(&p, Self::blocked().0);
                    if (on && state == "on" && truth(&p["Powered"]))
                        || (!on && matches!(state.as_str(), "off" | "off-blocked"))
                    {
                        return true;
                    }
                    if on
                        && state == "off"
                        && asked.is_none_or(|t| t.elapsed() > Duration::from_millis(2500))
                    {
                        asked = Some(Instant::now());
                        let _ = Self::setprop(&path, ADAPTER, "Powered", true.to_variant(), 5000);
                    }
                }
            }
            if Instant::now() >= end {
                return false;
            }
            thread::sleep(Duration::from_millis(250));
        }
    }
    pub fn reset(&self) -> ApiResult {
        let mut reset = self.reset.lock().unwrap_or_else(|e| e.into_inner());
        if reset.active {
            let end = Instant::now() + Duration::from_secs(35);
            while reset.active {
                let left = end.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    return Err(text_error(&self.ctx, "busy", "bt_busy"));
                }
                let (r, timeout) = self
                    .reset_done
                    .wait_timeout(reset, left)
                    .unwrap_or_else(|e| e.into_inner());
                reset = r;
                if timeout.timed_out() && reset.active {
                    return Err(text_error(&self.ctx, "busy", "bt_busy"));
                }
            }
            return match reset.error.clone() {
                Some(e) => Err(e),
                None => Ok(self.get()),
            };
        }
        reset.active = true;
        reset.error = None;
        drop(reset);
        let result = self.reset_inner();
        let mut reset = self.reset.lock().unwrap_or_else(|e| e.into_inner());
        reset.active = false;
        reset.error = result.as_ref().err().cloned();
        drop(reset);
        self.reset_done.notify_all();
        result
    }
    fn reset_inner(&self) -> ApiResult {
        let radios = radios()
            .into_iter()
            .filter(|r| r["type"] == "bluetooth")
            .collect::<Vec<_>>();
        if radios.iter().any(|r| truth(&r["hard"])) {
            return Err(text_error(&self.ctx, "blocked", "bt_hard_blocked"));
        }
        let pair = {
            let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
            s.pairing.as_ref().map(|p| p.address.clone())
        };
        if let Some(pair) = pair.as_ref() {
            let _ = self.cancel_pair(&json!(pair));
        }
        let _pair = lock_until(
            &self.pair_lock,
            Duration::from_secs(if pair.is_some() { 8 } else { 2 }),
        );
        let state = self.get();
        if state["available"] != true {
            Self::objects().map_err(|_| text_error(&self.ctx, "unavailable", "bt_unavailable"))?;
            if radios.is_empty() {
                return Err(text_error(&self.ctx, "unavailable", "bt_no_adapter"));
            }
        }
        self.stop_discovery();
        let ps = state["powerState"].as_str().unwrap_or("missing");
        let mut use_rf = state["available"] != true
            || truth(&state["stuck"])
            || matches!(ps, "on-disabling" | "off-enabling" | "off-blocked");
        if !use_rf && truth(&state["powered"]) {
            let (path, _) = self.adapter_or_error()?;
            if Self::setprop(&path, ADAPTER, "Powered", false.to_variant(), 5000).is_err()
                || !self.wait_power(5, false)
            {
                use_rf = true
            }
        }
        if use_rf {
            if !rfkill(true) {
                return Err(text_error(&self.ctx, "failed", "bt_reset_failed"));
            }
            thread::sleep(Duration::from_secs(1));
            if !rfkill(false) {
                thread::sleep(Duration::from_millis(500));
                if !rfkill(false) {
                    return Err(text_error(&self.ctx, "failed", "bt_reset_failed"));
                }
            }
        }
        {
            let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
            s.notready = false;
            s.transition = None;
        }
        if !self.wait_power(12, true) {
            self.notify();
            return Err(text_error(&self.ctx, "failed", "bt_reset_failed"));
        }
        thread::sleep(Duration::from_millis(200));
        Ok(self.get())
    }
    pub fn shutdown(&self) {
        self.stopped.store(true, Ordering::Relaxed);
        self.stop_discovery();
        let addr = self
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pairing
            .as_ref()
            .map(|p| p.address.clone());
        if let Some(addr) = addr {
            let _ = self.cancel_pair(&json!(addr));
        }
    }
}
fn lock_until<T>(mutex: &Mutex<T>, duration: Duration) -> Option<MutexGuard<'_, T>> {
    let end = Instant::now() + duration;
    loop {
        if let Ok(g) = mutex.try_lock() {
            return Some(g);
        }
        if Instant::now() >= end {
            return None;
        }
        thread::sleep(Duration::from_millis(20));
    }
}
fn device_call(path: &str, method: &str, timeout: i32) {
    if let Ok(bus) = system() {
        let _ = call(&bus, BZ, path, DEVICE, method, None, timeout);
    }
}
fn forget_bond(path: &str) {
    if let (Ok(bus), Ok(p)) = (system(), object_path(path)) {
        let _ = call(
            &bus,
            BZ,
            path.rsplit_once('/').map(|(p, _)| p).unwrap_or("/"),
            ADAPTER,
            "RemoveDevice",
            Some(&(p,).to_variant()),
            10000,
        );
    }
}
fn ensure_pairable(path: &str) -> bool {
    let Ok(bus) = system() else { return false };
    for _ in 0..4 {
        match call(
            &bus,
            BZ,
            path,
            "org.freedesktop.DBus.Properties",
            "Get",
            Some(&(ADAPTER, "Pairable").to_variant()),
            2000,
        ) {
            Ok(r) if truth(&first(&r)) => return false,
            Err(_) => return false,
            _ => {}
        }
        thread::sleep(Duration::from_millis(150));
    }
    Bluetooth::setprop(path, ADAPTER, "Pairable", true.to_variant(), 3000).is_ok()
}
fn valid_address(s: &str, separator: char) -> bool {
    let p = s.split(separator).collect::<Vec<_>>();
    p.len() == 6
        && p.iter()
            .all(|p| p.len() == 2 && p.bytes().all(|b| b.is_ascii_hexdigit()))
}
fn friendly_name(d: &Value) -> Option<String> {
    for key in ["Alias", "Name"] {
        let s = string(&d[key]).trim().to_owned();
        if !s.is_empty() && ![':', '_', '-'].iter().any(|sep| valid_address(&s, *sep)) {
            return Some(s);
        }
    }
    None
}
pub(super) fn power_state(props: &Value, blocked: bool) -> String {
    props["PowerState"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| {
            if truth(&props["Powered"]) {
                "on"
            } else if blocked {
                "off-blocked"
            } else {
                "off"
            }
            .into()
        })
}
pub(super) fn kind(d: &Value) -> &'static str {
    match d["Icon"].as_str().unwrap_or("") {
        "audio-headphones" => return "headphones",
        "audio-headset" => return "headset",
        "audio-card" | "audio-speakers" => return "speaker",
        "phone" => return "phone",
        "computer" => return "computer",
        "input-keyboard" => return "keyboard",
        "input-mouse" => return "mouse",
        "input-gaming" => return "gamepad",
        "input-tablet" => return "pen",
        "video-display" => return "display",
        "camera-video" | "camera-photo" => return "camera",
        "printer" | "scanner" => return "printer",
        "multimedia-player" => return "media",
        "modem" | "network-wireless" => return "network",
        _ => {}
    }
    let class = integer(&d["Class"]).unwrap_or(0);
    let major = (class >> 8) & 31;
    let minor = (class >> 2) & 63;
    if class != 0 && major != 0 {
        return match major {
            1 => "computer",
            2 => "phone",
            3 => "network",
            4 => match minor {
                1 | 2 | 4 | 16 => "headset",
                5 | 7 | 8 | 10 => "speaker",
                6 => "headphones",
                12 | 13 => "camera",
                14 | 15 => "display",
                _ => "media",
            },
            5 => match (class >> 6) & 3 {
                2 => "mouse",
                1 | 3 => "keyboard",
                _ => match minor & 15 {
                    1 | 2 => "gamepad",
                    5 => "pen",
                    _ => "other",
                },
            },
            6 => {
                if class & 0x10 != 0 {
                    "display"
                } else if class & 0x20 != 0 {
                    "camera"
                } else {
                    "printer"
                }
            }
            7 => "watch",
            _ => "other",
        };
    }
    let app = integer(&d["Appearance"]).unwrap_or(0);
    if app != 0 {
        let cat = app >> 6;
        let sub = app & 63;
        match cat {
            1 => return "phone",
            2 => return "computer",
            3 => return "watch",
            5 => return "display",
            15 => {
                return match sub {
                    1 => "keyboard",
                    2 => "mouse",
                    3 | 4 => "gamepad",
                    5 => "pen",
                    _ => "other",
                };
            }
            33 => return "speaker",
            37 => return if sub == 2 { "headset" } else { "headphones" },
            _ => {}
        }
    }
    let uuid = d["UUIDs"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|v| string(v).to_lowercase().chars().take(8).collect::<String>())
        .collect::<Vec<_>>();
    if uuid.iter().any(|u| u == "00001108" || u == "0000111e") {
        "headset"
    } else if uuid.iter().any(|u| u == "0000110b") {
        "speaker"
    } else {
        "other"
    }
}
pub(super) fn error_key<'a>(error: &str, op: &str, powered: bool) -> &'a str {
    let low = error.to_lowercase();
    if error.contains("NotReady") {
        return if powered {
            "bt_not_responding"
        } else {
            "bt_off"
        };
    }
    if error.contains("Error.Busy") {
        return "bt_busy";
    }
    if error.contains("DoesNotExist") || error.contains("UnknownObject") {
        return if op == "pair" {
            "bt_gone"
        } else {
            "bt_not_found"
        };
    }
    if op == "pair" {
        if error.contains("AuthenticationFailed") {
            "bt_auth_failed"
        } else if error.contains("AuthenticationCanceled") || error.contains("Error.Canceled") {
            "bt_canceled"
        } else if error.contains("AuthenticationRejected") || error.contains("Error.Rejected") {
            "bt_rejected"
        } else if [
            "AuthenticationTimeout",
            "Timeout",
            "NoReply",
            "ConnectionAttemptFailed",
        ]
        .iter()
        .any(|s| error.contains(s))
            || [
                "timed out",
                "page-timeout",
                "page timeout",
                "host is down",
                "temporarily unavailable",
            ]
            .iter()
            .any(|s| low.contains(s))
        {
            "bt_timeout"
        } else if error.contains("InProgress") {
            "bt_busy_pairing"
        } else {
            "bt_pair_failed"
        }
    } else if op == "connect" {
        if error.contains("InProgress") {
            "bt_in_progress"
        } else if low.contains("profile-unavailable")
            || error.contains("NotAvailable")
            || error.contains("NotSupported")
        {
            "bt_profile_unavailable"
        } else {
            "bt_connect_failed"
        }
    } else {
        match op {
            "disconnect" => "bt_disconnect_failed",
            "remove" => "bt_remove_failed",
            "discover" => "bt_discover_failed",
            _ => "bt_power_failed",
        }
    }
}
pub(super) fn error_code(key: &str) -> &'static str {
    match key {
        "bt_auth_failed" => "auth_failed",
        "bt_canceled" => "cancelled",
        "bt_rejected" => "rejected",
        "bt_timeout" | "bt_reply_timeout" => "timeout",
        "bt_not_responding" => "not_ready",
        "bt_off" => "off",
        "bt_busy" | "bt_busy_pairing" | "bt_in_progress" => "busy",
        "bt_gone" | "bt_not_found" => "not_found",
        "bt_profile_unavailable" => "unsupported",
        "bt_paired_not_connected" | "bt_connect_failed" => "connect_failed",
        "bt_unavailable" | "bt_no_adapter" => "unavailable",
        _ => "failed",
    }
}
/// gsd's rfkill plugin on the session bus (GNOME's flight mode and Bluetooth switch, amendment 57).
const RFKILL: &str = "org.gnome.SettingsDaemon.Rfkill";
const RFKILL_PATH: &str = "/org/gnome/SettingsDaemon/Rfkill";
/// What switching Bluetooth asks of gsd's Rfkill ([`gsd_step`]).
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(super) enum GsdStep {
    /// gsd has no Bluetooth rfkill here: BlueZ and rfkill as before.
    None,
    /// On asked while the hardware switch blocks Bluetooth.
    HardBlocked,
    /// gsd's BluetoothAirplaneMode is the wanted one already.
    Already,
    /// Write BluetoothAirplaneMode = this (!on).
    Write(bool),
}
/// GNOME Settings' Bluetooth switch (cc-bluetooth-panel.c): with BluetoothHasAirplaneMode it writes
/// BluetoothAirplaneMode = !on; the hardware switch (BluetoothHardwareAirplaneMode) leaves on impossible.
pub(super) fn gsd_step(props: Option<&Value>, on: bool) -> GsdStep {
    let Some(p) = props.filter(|p| truth(&p["BluetoothHasAirplaneMode"])) else {
        return GsdStep::None;
    };
    if on && truth(&p["BluetoothHardwareAirplaneMode"]) {
        return GsdStep::HardBlocked;
    }
    if truth(&p["BluetoothAirplaneMode"]) == !on {
        GsdStep::Already
    } else {
        GsdStep::Write(!on)
    }
}
/// gsd's Rfkill properties, None without gsd's rfkill plugin.
pub(super) fn gsd_rfkill(bus: &gio::DBusConnection, name: &str) -> Option<Value> {
    getall(bus, name, RFKILL_PATH, RFKILL)
        .ok()
        .filter(|v| v.is_object() && !v["BluetoothAirplaneMode"].is_null())
}
pub(super) fn gsd_write(
    bus: &gio::DBusConnection,
    name: &str,
    airplane: bool,
) -> Result<(), String> {
    setprop(
        bus,
        name,
        RFKILL_PATH,
        RFKILL,
        "BluetoothAirplaneMode",
        airplane.to_variant(),
        3000,
    )
}
fn rfkill(block: bool) -> bool {
    if available("rfkill")
        && run(
            &[
                "rfkill",
                if block { "block" } else { "unblock" },
                "bluetooth",
            ],
            5,
            false,
        )
        .ok
    {
        return true;
    }
    let Ok(mut fd) = OpenOptions::new().write(true).open("/dev/rfkill") else {
        return false;
    };
    fd.write_all(&[0, 0, 0, 0, 2, 3, u8::from(block), 0])
        .is_ok()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn device_categories() {
        assert_eq!(
            kind(&json!({"Icon":"audio-headphones","Class":0x100})),
            "headphones"
        );
        assert_eq!(kind(&json!({"Class":0x580})), "mouse");
        assert_eq!(kind(&json!({"Appearance":(37<<6)|2})), "headset");
        assert!(
            friendly_name(&json!({"Alias":"AA_BB_CC_DD_EE_FF","Name":"Headset"})).as_deref()
                == Some("Headset")
        );
    }
    #[test]
    fn bluez_errors() {
        assert_eq!(
            error_key("org.bluez.Error.NotReady", "pair", true),
            "bt_not_responding"
        );
        assert_eq!(
            error_key("org.bluez.Error.AuthenticationCanceled", "pair", true),
            "bt_canceled"
        );
        assert_eq!(
            error_code(error_key("org.bluez.Error.NotSupported", "connect", true)),
            "unsupported"
        );
    }
    #[test]
    fn the_switch_goes_through_gsd_as_gnome_settings_does() {
        let gsd = |airplane: bool, hardware: bool| {
            json!({"BluetoothHasAirplaneMode": true, "BluetoothAirplaneMode": airplane,
                "BluetoothHardwareAirplaneMode": hardware, "AirplaneMode": airplane})
        };
        assert_eq!(
            gsd_step(None, true),
            GsdStep::None,
            "no gsd: BlueZ as before"
        );
        assert_eq!(
            gsd_step(Some(&json!({"BluetoothHasAirplaneMode": false})), false),
            GsdStep::None
        );
        assert_eq!(
            gsd_step(Some(&gsd(false, false)), false),
            GsdStep::Write(true)
        );
        assert_eq!(
            gsd_step(Some(&gsd(true, false)), true),
            GsdStep::Write(false)
        );
        assert_eq!(gsd_step(Some(&gsd(false, false)), true), GsdStep::Already);
        assert_eq!(gsd_step(Some(&gsd(true, true)), true), GsdStep::HardBlocked);
        assert_eq!(gsd_step(Some(&gsd(true, true)), false), GsdStep::Already);
    }
    #[test]
    fn gsd_rfkill_is_read_and_written_on_the_session_bus() {
        use crate::services::devices::fake_bus::{Fake, Object};
        const XML: &str = r#"<node><interface name="org.gnome.SettingsDaemon.Rfkill">
          <property name="AirplaneMode" type="b" access="readwrite"/>
          <property name="BluetoothAirplaneMode" type="b" access="readwrite"/>
          <property name="BluetoothHasAirplaneMode" type="b" access="read"/>
          <property name="BluetoothHardwareAirplaneMode" type="b" access="read"/>
        </interface></node>"#;
        let p = "/org/gnome/SettingsDaemon/Rfkill";
        let Some(fake) = Fake::start(
            vec![Object {
                path: p.into(),
                xml: XML.into(),
                handler: std::sync::Arc::new(|_, _, _| Ok(None)),
            }],
            vec![
                (p, RFKILL, "AirplaneMode", false.to_variant()),
                (p, RFKILL, "BluetoothAirplaneMode", false.to_variant()),
                (p, RFKILL, "BluetoothHasAirplaneMode", true.to_variant()),
                (
                    p,
                    RFKILL,
                    "BluetoothHardwareAirplaneMode",
                    false.to_variant(),
                ),
            ],
        ) else {
            eprintln!(
                "skipped: no private session bus (run through tests/rust-sandbox/run-tests.sh)"
            );
            return;
        };
        let bus = session().unwrap();
        let props = gsd_rfkill(&bus, &fake.name).unwrap();
        assert_eq!(gsd_step(Some(&props), false), GsdStep::Write(true));
        gsd_write(&bus, &fake.name, true).unwrap();
        let set = fake.calls_of("Set");
        assert_eq!(set.len(), 1);
        assert_eq!(
            set[0].args,
            "('org.gnome.SettingsDaemon.Rfkill', 'BluetoothAirplaneMode', <true>)"
        );
        assert_eq!(
            fake.prop(p, RFKILL, "BluetoothAirplaneMode")
                .unwrap()
                .get::<bool>(),
            Some(true)
        );
        assert!(gsd_rfkill(&bus, "org.example.NoGsd").is_none());
    }
}
