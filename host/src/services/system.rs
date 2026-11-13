//! Native system API. Payloads and user-visible errors follow the retained service oracle.
mod agent;
mod audio;
mod audio_monitor;
mod bluetooth;
pub(super) mod common;
mod energy;
mod hardware;
mod info;
mod network;
mod power;
use super::ServiceContext;
use crate::files::{ApiError, ApiResult};
use crate::i18n::Lang;
use common::*;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
pub(crate) struct System {
    ctx: Arc<ServiceContext>,
    audio: Arc<audio::Audio>,
    audio_monitor: Mutex<Option<Arc<audio_monitor::AudioMonitor>>>,
    network: network::Network,
    bluetooth: Arc<bluetooth::Bluetooth>,
    battery: hardware::Battery,
    info: info::Info,
    updates: Arc<info::Updates>,
    night: power::NightLight,
    stopped: AtomicBool,
    last: Mutex<BTreeMap<String, Value>>,
    dirty: Mutex<BTreeSet<String>>,
}
const METHODS: &[&str] = &[
    "system.audio.get",
    "system.audio.set",
    "system.audio.setSink",
    "system.brightness.get",
    "system.brightness.set",
    "system.battery.get",
    "system.network.get",
    "system.network.wifiList",
    "system.network.setWifi",
    "system.network.setAirplane",
    "system.network.connect",
    "system.network.disconnect",
    "system.bluetooth.get",
    "system.bluetooth.setPowered",
    "system.bluetooth.connect",
    "system.bluetooth.disconnect",
    "system.bluetooth.remove",
    "system.bluetooth.discover",
    "system.bluetooth.pair",
    "system.bluetooth.pairReply",
    "system.bluetooth.cancelPair",
    "system.bluetooth.reset",
    "system.power.action",
    "system.power.settings.get",
    "system.power.settings.set",
    "system.nightLight.get",
    "system.nightLight.set",
    "system.info",
    "system.updates",
    "system.openUpdater",
    "system.time",
];
impl System {
    pub(crate) fn new(ctx: Arc<ServiceContext>) -> Result<Arc<Self>, String> {
        let audio = Arc::new(audio::Audio::new());
        let service = Arc::new(Self {
            network: network::Network::new(ctx.clone()),
            bluetooth: bluetooth::Bluetooth::new(ctx.clone(), audio.clone()),
            battery: hardware::Battery::new(),
            info: info::Info::new(),
            updates: info::Updates::new(ctx.clone()),
            night: power::NightLight::new(),
            ctx,
            audio,
            audio_monitor: Mutex::new(None),
            stopped: AtomicBool::new(false),
            last: Mutex::new(BTreeMap::new()),
            dirty: Mutex::new(BTreeSet::new()),
        });
        let audio_weak = Arc::downgrade(&service);
        let monitor = audio_monitor::AudioMonitor::start(Arc::new(move || {
            if let Some(s) = audio_weak.upgrade() {
                s.dirty
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert("audio".into());
            }
        }));
        *service
            .audio_monitor
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = monitor;
        let weak = Arc::downgrade(&service);
        thread::spawn(move || {
            let context = glib::MainContext::new();
            let _ = context.with_thread_default(|| {
                let energy_preferences = energy::Preferences::new();
                let energy_weak = weak.clone();
                energy_preferences.watch(move || {
                    if let Some(s) = energy_weak.upgrade() {
                        s.dirty
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .insert("power.settings".into());
                    }
                });
                let bus = system().ok();
                let mut subs = vec![];
                if let Some(bus) = bus.as_ref() {
                    for (name, domain) in [
                        ("org.freedesktop.NetworkManager", "network"),
                        ("org.bluez", "bluetooth"),
                        ("org.freedesktop.UPower", "battery"),
                        ("org.freedesktop.UPower.PowerProfiles", "power.settings"),
                        ("net.hadess.PowerProfiles", "power.settings"),
                    ] {
                        let w = weak.clone();
                        let domain = domain.to_owned();
                        subs.push(bus.signal_subscribe(
                            Some(name),
                            Some("org.freedesktop.DBus.Properties"),
                            Some("PropertiesChanged"),
                            None,
                            None,
                            gio::DBusSignalFlags::NONE,
                            move |_, _, _, _, _, _| {
                                if let Some(s) = w.upgrade() {
                                    s.dirty
                                        .lock()
                                        .unwrap_or_else(|e| e.into_inner())
                                        .insert(domain.clone());
                                }
                            },
                        ));
                    }
                    let power_weak = weak.clone();
                    subs.push(bus.signal_subscribe(
                        Some("org.freedesktop.DBus"),
                        Some("org.freedesktop.DBus"),
                        Some("NameOwnerChanged"),
                        Some("/org/freedesktop/DBus"),
                        None,
                        gio::DBusSignalFlags::NONE,
                        move |_, _, _, _, _, args| {
                            if args
                                .get::<(String, String, String)>()
                                .is_some_and(|(name, _, _)| {
                                    energy::PROFILE_ENDPOINTS.iter().any(|(n, _)| *n == name)
                                })
                            {
                                if let Some(s) = power_weak.upgrade() {
                                    s.dirty
                                        .lock()
                                        .unwrap_or_else(|e| e.into_inner())
                                        .insert("power.settings".into());
                                }
                            }
                        },
                    ));
                    let w = weak.clone();
                    subs.push(bus.signal_subscribe(
                        Some("org.bluez"),
                        Some("org.freedesktop.DBus.ObjectManager"),
                        None,
                        Some("/"),
                        None,
                        gio::DBusSignalFlags::NONE,
                        move |_, _, _, _, _, _| {
                            if let Some(s) = w.upgrade() {
                                s.dirty
                                    .lock()
                                    .unwrap_or_else(|e| e.into_inner())
                                    .insert("bluetooth".into());
                            }
                        },
                    ));
                    for signal in ["DeviceAdded", "DeviceRemoved"] {
                        let w = weak.clone();
                        subs.push(bus.signal_subscribe(
                            Some("org.freedesktop.NetworkManager"),
                            Some("org.freedesktop.NetworkManager"),
                            Some(signal),
                            None,
                            None,
                            gio::DBusSignalFlags::NONE,
                            move |_, _, _, _, _, _| {
                                if let Some(s) = w.upgrade() {
                                    s.network.invalidate();
                                    s.dirty
                                        .lock()
                                        .unwrap_or_else(|e| e.into_inner())
                                        .insert("network".into());
                                }
                            },
                        ));
                    }
                }
                let mut tick = 0u64;
                let mut audio_reads = 0u64;
                let mut next = Instant::now();
                let mut last_runs = BTreeMap::<String, Instant>::new();
                let mut settings_night = Value::Null;
                loop {
                    while context.pending() {
                        context.iteration(false);
                    }
                    let Some(s) = weak.upgrade() else { break };
                    if s.stopped.load(Ordering::Relaxed) {
                        break;
                    }
                    if s.ctx.subscriber_count() == 0 {
                        drop(s);
                        thread::sleep(Duration::from_millis(100));
                        continue;
                    }
                    let monitored = s
                        .audio_monitor
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .as_ref()
                        .is_some_and(|m| m.alive());
                    let now = Instant::now();
                    let mut domains =
                        std::mem::take(&mut *s.dirty.lock().unwrap_or_else(|e| e.into_inner()));
                    if now >= next {
                        next = now + Duration::from_millis(2500);
                        tick += 1;
                        if tick == 1 || !monitored || tick % 8 == 0 {
                            domains.insert("audio".into());
                        }
                        if tick == 1 || hardware::brightness_get()["available"] == true {
                            domains.insert("brightness".into());
                        }
                        if tick % 2 == 1 {
                            domains.extend(["battery", "network", "bluetooth"].map(str::to_owned));
                        }
                    }
                    for domain in domains {
                        if last_runs
                            .get(&domain)
                            .is_some_and(|at| at.elapsed() < Duration::from_millis(500))
                        {
                            s.dirty
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .insert(domain);
                            continue;
                        }
                        let value = match domain.as_str() {
                            "audio" => {
                                audio_reads += 1;
                                s.audio.get(monitored || audio_reads % 4 == 1)
                            }
                            "brightness" => hardware::brightness_get(),
                            "battery" => s.battery.get(),
                            "power.settings" => energy_preferences.snapshot(bus.as_ref()),
                            "network" => s.network.get(),
                            "bluetooth" => s.bluetooth.get(),
                            _ => continue,
                        };
                        last_runs.insert(domain.clone(), Instant::now());
                        s.publish(&domain, value, false);
                    }
                    if s.night.supported() && !s.ctx.dry_run {
                        let settings = s.ctx.settings()["nightLight"].clone();
                        if settings != settings_night {
                            settings_night = settings.clone();
                            let _ = s.night.set(
                                truth(&settings["enabled"]),
                                &settings["temperature"],
                                s.ctx.lang(),
                            );
                        }
                    }
                    drop(s);
                    thread::sleep(Duration::from_millis(50));
                }
                if let Some(bus) = bus {
                    for sub in subs {
                        bus.signal_unsubscribe(sub);
                    }
                }
            });
        });
        Ok(service)
    }
    pub(crate) fn shutdown(&self) {
        self.stopped.store(true, Ordering::Relaxed);
        if let Some(monitor) = self
            .audio_monitor
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            monitor.stop();
        }
        self.bluetooth.shutdown();
        self.night.stop();
    }
    fn publish(&self, domain: &str, value: Value, explicit: bool) {
        let mut last = self.last.lock().unwrap_or_else(|e| e.into_inner());
        let previous = last.insert(domain.into(), value.clone());
        if explicit || previous.as_ref().is_some_and(|old| old != &value) {
            drop(last);
            self.ctx.emit(&format!("system.{domain}.changed"), value);
        }
    }
    pub(crate) fn call(self: &Arc<Self>, method: &str, args: &Value) -> Option<ApiResult> {
        if !METHODS.contains(&method) {
            return None;
        }
        let mutating = matches!(
            method,
            "system.audio.set"
                | "system.audio.setSink"
                | "system.brightness.set"
                | "system.network.setWifi"
                | "system.network.setAirplane"
                | "system.network.connect"
                | "system.network.disconnect"
                | "system.bluetooth.setPowered"
                | "system.bluetooth.connect"
                | "system.bluetooth.disconnect"
                | "system.bluetooth.remove"
                | "system.bluetooth.discover"
                | "system.bluetooth.pair"
                | "system.bluetooth.pairReply"
                | "system.bluetooth.cancelPair"
                | "system.bluetooth.reset"
                | "system.power.action"
                | "system.power.settings.set"
                | "system.nightLight.set"
                | "system.openUpdater"
        );
        if mutating && self.ctx.dry_run {
            if method == "system.power.settings.set" {
                if let Err(error) = energy::validate(args, self.ctx.lang()) {
                    return Some(Err(error));
                }
            }
            let mut result = match method {
                "system.audio.set" => self.audio.get(false),
                "system.brightness.set" => hardware::brightness_get(),
                "system.bluetooth.reset" => self.bluetooth.get(),
                "system.power.settings.set" => energy::get(),
                _ => json!({}),
            };
            result["ok"] = json!(true);
            result["dryRun"] = json!(true);
            return Some(Ok(result));
        }
        let lang = self.ctx.lang();
        let result = (|| -> ApiResult {
            match method {
                "system.audio.get" => Ok(self.audio.get(true)),
                "system.audio.set" => {
                    let v = self.audio.set(args, lang)?;
                    self.publish("audio", v.clone(), true);
                    Ok(v)
                }
                "system.audio.setSink" => {
                    let v = self.audio.set_sink(&args["id"], lang)?;
                    self.publish("audio", v.clone(), true);
                    Ok(v)
                }
                "system.brightness.get" => Ok(hardware::brightness_get()),
                "system.brightness.set" => {
                    let v = hardware::brightness_set(args, lang)?;
                    self.publish("brightness", v.clone(), true);
                    Ok(v)
                }
                "system.battery.get" => Ok(self.battery.get()),
                "system.network.get" => Ok(self.network.get()),
                "system.network.wifiList" => Ok(self.network.wifi_list(truth(&args["rescan"]))),
                "system.network.setWifi" => {
                    let v = self
                        .network
                        .set_wifi(bool_arg_exact(&args["enabled"], lang)?)?;
                    self.publish("network", v.clone(), true);
                    Ok(v)
                }
                "system.network.setAirplane" => {
                    let v = self
                        .network
                        .set_airplane(bool_arg_exact(&args["enabled"], lang)?, &self.bluetooth)?;
                    self.publish("network", v.clone(), true);
                    self.publish("bluetooth", self.bluetooth.get(), true);
                    Ok(v)
                }
                "system.network.connect" => {
                    let v = self.network.connect(args)?;
                    self.publish("network", self.network.get(), true);
                    Ok(v)
                }
                "system.network.disconnect" => {
                    let v = self.network.disconnect(&args["iface"])?;
                    self.publish("network", self.network.get(), true);
                    Ok(v)
                }
                "system.bluetooth.get" => Ok(self.bluetooth.get()),
                "system.bluetooth.setPowered" => {
                    let v = self
                        .bluetooth
                        .set_powered(bool_arg_exact(&args["powered"], lang)?)?;
                    self.publish("bluetooth", v.clone(), true);
                    Ok(v)
                }
                "system.bluetooth.connect" | "system.bluetooth.disconnect" => {
                    let v = self
                        .bluetooth
                        .connect(&args["address"], method.ends_with(".connect"))?;
                    self.publish("bluetooth", self.bluetooth.get(), true);
                    Ok(v)
                }
                "system.bluetooth.remove" => {
                    let v = self.bluetooth.remove(&args["address"])?;
                    self.publish("bluetooth", self.bluetooth.get(), true);
                    Ok(v)
                }
                "system.bluetooth.discover" => {
                    let on = if args["on"].is_null() {
                        true
                    } else {
                        bool_arg_exact(&args["on"], lang)?
                    };
                    let v = self.bluetooth.discover(on)?;
                    self.publish("bluetooth", self.bluetooth.get(), true);
                    Ok(v)
                }
                "system.bluetooth.pair" => {
                    let v = self.bluetooth.pair(&args["address"])?;
                    self.publish("bluetooth", self.bluetooth.get(), true);
                    Ok(v)
                }
                "system.bluetooth.pairReply" => self.bluetooth.pair_reply(args),
                "system.bluetooth.cancelPair" => self.bluetooth.cancel_pair(&args["address"]),
                "system.bluetooth.reset" => {
                    let v = self.bluetooth.reset()?;
                    self.publish("bluetooth", v.clone(), true);
                    Ok(v)
                }
                "system.power.settings.get" => {
                    let v = energy::get();
                    // Prime event comparison when the energy page is first opened.
                    self.last
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .insert("power.settings".into(), v.clone());
                    Ok(v)
                }
                "system.power.settings.set" => {
                    let v = energy::set(args, lang)?;
                    self.publish("power.settings", v.clone(), true);
                    Ok(v)
                }
                "system.power.action" => {
                    power::action(args["action"].as_str().unwrap_or(""), &self.ctx)
                }
                "system.nightLight.get" => Ok(self.night.get()),
                "system.nightLight.set" => self.night.set(
                    bool_arg_exact(&args["enabled"], lang)?,
                    &args["temperature"],
                    lang,
                ),
                "system.info" => Ok(self.info.get()),
                "system.updates" => Ok(self.updates.get(truth(&args["refresh"]))),
                "system.openUpdater" => power::open_updater(lang),
                "system.time" => Ok(info::time_info(
                    self.ctx.settings()["time"]["h24"].as_bool(),
                )),
                _ => unreachable!("validated system method"),
            }
        })();
        Some(result)
    }
}
impl Drop for System {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn text(ctx: &ServiceContext, key: &str) -> String {
    static STRINGS: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    STRINGS.get_or_init(|| {
        serde_json::from_str(include_str!("system/strings.json"))
            .expect("embedded system translations")
    })["system"][ctx.language()][key]
        .as_str()
        .unwrap_or(key)
        .into()
}
/// An API error with a text of strings.json (D-G8 keeps that table in v1). Its C2 `key` is `system.<entry>`, the
/// table's own name for the text, so the UI can match it: bluetooth.js and quicksettings.js test
/// `system.bt_not_responding` (with code `not_ready`). Codes stay exactly as they were.
fn text_error(ctx: &ServiceContext, code: &str, entry: &str) -> ApiError {
    let mut error = ApiError::new(code, &text(ctx, entry));
    error.key = Some(format!("system.{entry}"));
    error.params = Some(json!({}));
    error
}
fn number_arg(v: &Value, lang: Lang) -> Result<f64, ApiError> {
    if v.is_null() || v.is_boolean() {
        return Err(ApiError::tr(
            "bad_args",
            lang,
            "system.invalidValue",
            crate::i18n::NO_PARAMS,
        ));
    }
    number(v).filter(|n| n.is_finite()).ok_or_else(|| {
        ApiError::tr(
            "bad_args",
            lang,
            "system.invalidValue",
            crate::i18n::NO_PARAMS,
        )
    })
}
fn pct_arg(v: &Value, lang: Lang) -> Result<i64, ApiError> {
    let mut n = number_arg(v, lang)?;
    if v.as_number().is_some_and(|v| v.is_f64()) && n > 0. && n < 1. {
        n *= 100.
    }
    Ok(round(n, 0).clamp(0., 100.) as i64)
}
fn bool_arg_exact(v: &Value, lang: Lang) -> Result<bool, ApiError> {
    if let Some(b) = v.as_bool() {
        return Ok(b);
    }
    if v.is_number() {
        return Ok(truth(v));
    }
    if let Some(s) = v.as_str() {
        match s.to_lowercase().as_str() {
            "1" | "true" | "yes" | "on" | "aan" => return Ok(true),
            "0" | "false" | "no" | "off" | "uit" | "" => return Ok(false),
            _ => {}
        }
    }
    Err(ApiError::tr(
        "bad_args",
        lang,
        "system.invalidValue",
        crate::i18n::NO_PARAMS,
    ))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn argument_coercion_is_reference_exact() {
        assert_eq!(pct_arg(&json!(0.5), Lang::Nl).unwrap(), 50);
        assert_eq!(pct_arg(&json!("0.5"), Lang::Nl).unwrap(), 0);
        assert!(number_arg(&json!(true), Lang::Nl).is_err());
        assert!(number_arg(&json!("NaN"), Lang::Nl).is_err());
        assert!(bool_arg_exact(&json!(null), Lang::Nl).is_err());
        assert_eq!(bool_arg_exact(&json!("aan"), Lang::Nl).unwrap(), true);
        assert_eq!(bool_arg_exact(&json!("uit"), Lang::Nl).unwrap(), false);
    }
    #[test]
    fn contract_inventory_complete() {
        assert_eq!(METHODS.len(), 31);
        assert_eq!(METHODS.iter().collect::<BTreeSet<_>>().len(), METHODS.len());
        assert!(METHODS.contains(&"system.power.settings.get"));
        assert!(METHODS.contains(&"system.power.settings.set"));
    }
}

#[cfg(test)]
mod i18n_tests {
    // R2-A: the system stem in English through its real call sites. The Dutch bytes are checked by
    // i18n::tests::every_converted_site_renders_its_3b898e6_dutch and by reference_contracts below.
    use super::*;
    use common::LangContext;

    #[test]
    fn argument_errors_carry_their_key_and_render_in_both_languages() {
        assert_eq!(
            number_arg(&json!(true), Lang::Nl).unwrap_err().json(),
            json!({"code":"bad_args","message":"Ongeldige waarde.","key":"system.invalidValue","params":{}})
        );
        let en = bool_arg_exact(&json!("maybe"), Lang::En).unwrap_err();
        assert_eq!(
            (en.code.as_str(), en.message.as_str()),
            ("bad_args", "Invalid value.")
        );
        assert_eq!(
            pct_arg(&json!("x"), Lang::En).unwrap_err().message,
            "Invalid value."
        );
        assert_eq!(
            energy::validate(&json!({}), Lang::En).unwrap_err().message,
            "Specify exactly one valid power setting."
        );
        assert_eq!(
            energy::validate(&json!({}), Lang::Nl).unwrap_err().message,
            "Geef één geldige energie-instelling op."
        );
    }

    #[test]
    fn power_network_and_night_light_errors_follow_the_context_language() {
        let en = LangContext::new("en", "desktop");
        assert_eq!(
            power::action("bogus", &en.ctx).unwrap_err().json(),
            json!({"code":"bad_args","message":"Unknown action.","key":"system.power.unknownAction","params":{}})
        );
        let network = network::Network::new(en.ctx.clone());
        assert_eq!(
            network.connect(&json!({"ssid":""})).unwrap_err().message,
            "Invalid network name."
        );
        let e = network
            .connect(&json!({"ssid":"Home","password":5}))
            .unwrap_err();
        // the code opens the password field in the UI (quicksettings.js): it stays bad_args
        assert_eq!(
            (e.code.as_str(), e.message.as_str()),
            ("bad_args", "Invalid password.")
        );
        // D-G2: GNOME's night light is set in Ubuntu's Settings app (the UI's display.js nightNA, both languages).
        // The error NightLight::set returns without hyprsunset, on every machine (review m4) ...
        let (night_en, night_nl) = (
            "Night light can’t be set up here. Use Ubuntu’s Settings app instead.",
            "Nachtlampje kan hier niet worden ingesteld. Gebruik daarvoor de app Instellingen van Ubuntu.",
        );
        assert_eq!(
            power::NightLight::unsupported(en.ctx.lang()).json(),
            json!({"code":"unsupported","message":night_en,"key":"system.nightLight.unsupported","params":{}})
        );
        assert_eq!(power::NightLight::unsupported(Lang::Nl).message, night_nl);
        // ... is the UI's own nightNA ...
        if let Some((nl, en)) = common::ui_display_text("nightNA") {
            assert_eq!((night_nl, night_en), (nl.as_str(), en.as_str()));
        }
        // ... and set() really returns it here (a GNOME session has no hyprsunset under Hyprland)
        let night = power::NightLight::new();
        if !night.supported() {
            let e = night.set(true, &Value::Null, en.ctx.lang()).unwrap_err();
            assert_eq!(e.json(), power::NightLight::unsupported(Lang::En).json());
            assert_eq!(
                night.set(true, &Value::Null, Lang::Nl).unwrap_err().message,
                night_nl
            );
        }
        let nl = LangContext::new("nl", "desktop");
        assert_eq!(
            power::action("bogus", &nl.ctx).unwrap_err().message,
            "Onbekende actie."
        );
        assert_eq!(
            network::Network::new(nl.ctx.clone())
                .connect(&json!({"ssid":""}))
                .unwrap_err()
                .message,
            "Ongeldige netwerknaam."
        );
    }

    #[test]
    fn bluetooth_table_errors_carry_the_key_the_ui_matches() {
        // bluetooth.js and quicksettings.js test e.key === 'system.bt_not_responding' (or code not_ready)
        let en = LangContext::new("en", "desktop");
        assert_eq!(
            text_error(&en.ctx, "not_ready", "bt_not_responding").json(),
            json!({"code":"not_ready","message":"Bluetooth is not responding. Choose “Restart Bluetooth”.","key":"system.bt_not_responding","params":{}})
        );
        let nl = LangContext::new("nl", "desktop");
        assert_eq!(
            text_error(&nl.ctx, "not_ready", "bt_not_responding").message,
            "Bluetooth reageert niet. Kies “Bluetooth opnieuw starten”."
        );
        assert_eq!(bluetooth::error_code("bt_not_responding"), "not_ready");
        // every Bluetooth text of strings.json exists in both languages, so no error shows its key
        let table: Value = serde_json::from_str(include_str!("system/strings.json")).unwrap();
        for (entry, dutch) in table["system"]["nl"].as_object().unwrap() {
            if entry.starts_with("bt_") {
                let error = text_error(&nl.ctx, "failed", entry);
                assert_eq!(error.message, dutch.as_str().unwrap());
                assert_eq!(
                    error.key.as_deref(),
                    Some(format!("system.{entry}").as_str())
                );
                let english = text_error(&en.ctx, "failed", entry).message;
                assert!(
                    !english.is_empty() && english != *entry && english != error.message,
                    "{entry}"
                );
            }
        }
    }
}

#[cfg(test)]
mod reference_contracts {
    use super::*;
    #[test]
    fn retained_python_oracle_contracts_match_native_helpers() {
        let cases: Value =
            serde_json::from_str(include_str!("system/reference-tests.json")).unwrap();
        for (n, case) in cases.as_array().unwrap().iter().enumerate() {
            let a = &case["args"];
            let name = case["name"].as_str().unwrap();
            let result: ApiResult = match name {
                "number" => number_arg(&a[0], Lang::Nl).map(|v| json!(v)),
                "pct" => pct_arg(&a[0], Lang::Nl).map(|v| json!(v)),
                "bool" => bool_arg_exact(&a[0], Lang::Nl).map(|v| json!(v)),
                "wifi_security" => Ok(json!(network::security(
                    a[0].as_i64().unwrap(),
                    a[1].as_i64().unwrap(),
                    a[2].as_i64().unwrap()
                ))),
                "psk_valid" => Ok(json!(network::psk_valid(a[0].as_str().unwrap()))),
                "split_terse" => Ok(json!(network::split_terse(a[0].as_str().unwrap()))),
                "norm_mode" => Ok(json!(crate::services::display::norm_mode(
                    a[0].as_str().unwrap()
                ))),
                "monitor_dict" => Ok(crate::services::display::monitor_dict(&a[0])),
                "bt_kind" => Ok(json!(bluetooth::kind(&a[0]))),
                "bt_error_key" => Ok(json!(bluetooth::error_key(
                    a[0].as_str().unwrap(),
                    a[1].as_str().unwrap(),
                    a[2].as_bool().unwrap()
                ))),
                "player_dict" => {
                    let mut p =
                        crate::services::media::player_dict(a[0].as_str().unwrap(), &a[1], &a[2]);
                    p.as_object_mut().unwrap().remove("updated");
                    Ok(p)
                }
                _ => panic!("Unknown oracle case {name}"),
            };
            let actual = match result {
                Ok(v) => json!({"ok":v}),
                Err(e) => json!({"error":{"code":e.code,"message":e.message}}),
            };
            assert_eq!(
                actual, case["result"],
                "Reference case {n}: {name} args={a}"
            );
        }
    }
}

#[cfg(test)]
mod private_bus_tests;
