//! `battery.*` (batch D, notes/claude/settings-parity.md §5.5): GNOME Settings 50's Battery Charging (UPower's charge
//! threshold) and Windows' Batterijniveaus (UPower's history) for Energie en batterij.
//!
//! * `battery.charging.get {}` → `{available, supported, enabled, start, end, devices}`: every battery that is a
//!   power supply (UPower EnumerateDevices, Type 2), `supported` when one has ChargeThresholdSupported (GNOME shows
//!   the section only then), `enabled` when every supported one has ChargeThresholdEnabled (GNOME's "Preserve
//!   Battery Health").
//! * `battery.charging.set! {enabled}` → the same after `EnableChargeThreshold(b)` on every supported battery, as
//!   GNOME does it: through `call_interactive` (polkit org.freedesktop.UPower.enable-charging-limit, allow_active
//!   yes, so no dialog for the user at the seat). A dry run checks and sends nothing.
//! * `battery.history {type?, timespan?, resolution?}` → `{available, type, timespan, device, points: [[time, value,
//!   state]]}`: UPower's GetHistory of the first battery with HasHistory, oldest first.
//! * Event `battery.changed` (the charging state) when a battery's ChargeThreshold properties change.
use super::ServiceContext;
use super::system::common::{call, call_interactive, first, getall, system, truth, variant_json};
use crate::files::{ApiError, ApiResult};
use crate::i18n::{Lang, NO_PARAMS};
use glib::variant::ToVariant;
use serde_json::{Value, json};
use std::sync::{
    Arc, Mutex, Weak,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

/// The methods (name, dangerous), exactly as host/src/api-methods/system.json lists them (a test compares).
#[cfg_attr(not(test), allow(dead_code))] // the dispatch tests compare it with the api-methods files
pub(crate) const METHODS: &[(&str, bool)] = &[
    ("battery.charging.get", false),
    ("battery.charging.set", true),
    ("battery.history", false),
];

pub(crate) const UPOWER: &str = "org.freedesktop.UPower";
pub(crate) const UPOWER_PATH: &str = "/org/freedesktop/UPower";
pub(crate) const DEVICE: &str = "org.freedesktop.UPower.Device";
/// UP_DEVICE_KIND_BATTERY.
const KIND_BATTERY: i64 = 2;
/// The charge threshold properties GNOME follows (cc-power-panel).
const THRESHOLD_PROPS: [&str; 4] = [
    "ChargeThresholdSupported",
    "ChargeThresholdEnabled",
    "ChargeStartThreshold",
    "ChargeEndThreshold",
];

/// Where UPower is: the system bus and its name (tests: a fake on the private session bus, by its unique name).
pub(crate) struct Upower<'a> {
    pub bus: &'a gio::DBusConnection,
    pub name: &'a str,
}

/// Every battery that powers the computer: (object path, its properties).
fn batteries(up: &Upower) -> Result<Vec<(String, Value)>, String> {
    let reply = call(
        up.bus,
        up.name,
        UPOWER_PATH,
        UPOWER,
        "EnumerateDevices",
        None,
        1500,
    )?;
    let mut out = Vec::new();
    for path in first(&reply).as_array().into_iter().flatten() {
        let Some(path) = path.as_str() else { continue };
        let Ok(props) = getall(up.bus, up.name, path, DEVICE) else {
            continue;
        };
        if props["Type"].as_i64() == Some(KIND_BATTERY) && truth(&props["PowerSupply"]) {
            out.push((path.to_owned(), props));
        }
    }
    Ok(out)
}

/// battery.charging.get without UPower.
fn no_charging() -> Value {
    json!({"available": false, "supported": false, "enabled": false, "start": null, "end": null, "devices": []})
}

/// battery.charging.get.
pub(crate) fn charging(up: &Upower) -> Value {
    let Ok(list) = batteries(up) else {
        return no_charging();
    };
    let supported: Vec<&(String, Value)> = list
        .iter()
        .filter(|(_, p)| p["ChargeThresholdSupported"] == true)
        .collect();
    let enabled = !supported.is_empty()
        && supported
            .iter()
            .all(|(_, p)| p["ChargeThresholdEnabled"] == true);
    let threshold = |key: &str| {
        supported
            .first()
            .and_then(|(_, p)| p[key].as_u64())
            .filter(|n| *n <= 100)
    };
    json!({
        "available": !list.is_empty(),
        "supported": !supported.is_empty(),
        "enabled": enabled,
        "start": threshold("ChargeStartThreshold"),
        "end": threshold("ChargeEndThreshold"),
        "devices": supported.iter().map(|(path, _)| path.clone()).collect::<Vec<_>>(),
    })
}

fn bad(lang: Lang) -> ApiError {
    ApiError::tr("bad_args", lang, "core.badArgs", NO_PARAMS)
}

/// battery.charging.set: EnableChargeThreshold on every battery that supports it (GNOME's Maximise Charge /
/// Preserve Battery Health radios), then the state as UPower reports it.
pub(crate) fn set_charging(up: &Upower, args: &Value, lang: Lang, dry_run: bool) -> ApiResult {
    let enabled = args
        .get("enabled")
        .and_then(Value::as_bool)
        .ok_or_else(|| bad(lang))?;
    let state = charging(up);
    if state["available"] != true {
        return Err(ApiError::tr(
            "unsupported",
            lang,
            "battery.unavailable",
            NO_PARAMS,
        ));
    }
    if state["supported"] != true {
        return Err(ApiError::tr(
            "unsupported",
            lang,
            "battery.noChargeLimit",
            NO_PARAMS,
        ));
    }
    if dry_run {
        let mut out = state;
        out["dryRun"] = json!(true);
        return Ok(out);
    }
    for path in state["devices"].as_array().into_iter().flatten() {
        let Some(path) = path.as_str() else { continue };
        call_interactive(
            up.bus,
            up.name,
            path,
            DEVICE,
            "EnableChargeThreshold",
            Some(&(enabled,).to_variant()),
            lang,
        )?;
    }
    Ok(charging(up))
}

/// The arguments of battery.history: (type, timespan s, resolution points).
pub(crate) fn history_args(args: &Value, lang: Lang) -> Result<(String, u32, u32), ApiError> {
    let kind = match args.get("type") {
        None | Some(Value::Null) => "charge",
        Some(v) => v
            .as_str()
            .filter(|s| matches!(*s, "charge" | "rate"))
            .ok_or_else(|| bad(lang))?,
    };
    let number = |key: &str, default: u64, range: std::ops::RangeInclusive<u64>| match args.get(key)
    {
        None | Some(Value::Null) => Ok(default as u32),
        Some(v) => v
            .as_u64()
            .filter(|n| range.contains(n))
            .map(|n| n as u32)
            .ok_or_else(|| bad(lang)),
    };
    let timespan = number("timespan", 86_400, 60..=30 * 86_400)?;
    let resolution = number("resolution", 200, 2..=2000)?;
    Ok((kind.to_owned(), timespan, resolution))
}

/// GetHistory's a(udu) as [[time, value, state]], oldest first, without empty or repeated samples.
pub(crate) fn history_points(reply: &glib::Variant) -> Vec<Value> {
    let mut points: Vec<(u32, f64, u32)> = reply
        .child_value(0)
        .get::<Vec<(u32, f64, u32)>>()
        .unwrap_or_default()
        .into_iter()
        .filter(|(time, value, _)| *time > 0 && value.is_finite())
        .collect();
    points.sort_by_key(|p| p.0);
    points.dedup_by_key(|p| p.0);
    points
        .into_iter()
        .map(|(time, value, state)| json!([time, (value * 10.0).round() / 10.0, state]))
        .collect()
}

/// battery.history without a battery history.
fn no_history(kind: &str, timespan: u32) -> Value {
    json!({"available": false, "type": kind, "timespan": timespan, "device": null, "points": []})
}

/// battery.history.
pub(crate) fn history(up: &Upower, args: &Value, lang: Lang) -> ApiResult {
    let (kind, timespan, resolution) = history_args(args, lang)?;
    let Ok(list) = batteries(up) else {
        return Ok(no_history(&kind, timespan));
    };
    // UPower keeps a history only for real batteries (HasHistory); an older UPower without the property: the first
    let Some((path, _)) = list
        .iter()
        .find(|(_, p)| p["HasHistory"] == true)
        .or_else(|| list.iter().find(|(_, p)| p["HasHistory"].is_null()))
    else {
        return Ok(no_history(&kind, timespan));
    };
    match call(
        up.bus,
        up.name,
        path,
        DEVICE,
        "GetHistory",
        Some(&(kind.as_str(), timespan, resolution).to_variant()),
        3000,
    ) {
        Ok(reply) => Ok(
            json!({"available": true, "type": kind, "timespan": timespan, "device": path,
            "points": history_points(&reply)}),
        ),
        Err(_) => Ok(no_history(&kind, timespan)),
    }
}

/// Whether a PropertiesChanged of a UPower device touches the charge threshold.
pub(crate) fn threshold_changed(params: &glib::Variant) -> bool {
    let changed = variant_json(&params.child_value(1));
    let invalidated = variant_json(&params.child_value(2));
    THRESHOLD_PROPS.iter().any(|k| {
        changed.get(*k).is_some()
            || invalidated
                .as_array()
                .is_some_and(|a| a.iter().any(|v| v == *k))
    })
}

pub(crate) struct Battery {
    ctx: Arc<ServiceContext>,
    stopped: AtomicBool,
    /// The watcher's context (UPower's PropertiesChanged), woken by `shutdown`.
    context: Mutex<Option<glib::MainContext>>,
    /// How often the watcher woke up (tests: it sleeps until something happens).
    #[cfg(test)]
    wakeups: std::sync::atomic::AtomicUsize,
}

impl Battery {
    pub(crate) fn new(ctx: Arc<ServiceContext>) -> Result<Arc<Self>, String> {
        Ok(Self::watching(ctx, || {
            system().ok().map(|bus| Watched {
                bus,
                sender: UPOWER.into(),
                name: UPOWER.into(),
            })
        }))
    }

    /// The service with its watcher on `at` (UPower on the system bus; tests: a private bus).
    fn watching(
        ctx: Arc<ServiceContext>,
        at: impl FnOnce() -> Option<Watched> + Send + 'static,
    ) -> Arc<Self> {
        let s = Arc::new(Self {
            ctx,
            stopped: AtomicBool::new(false),
            context: Mutex::new(None),
            #[cfg(test)]
            wakeups: std::sync::atomic::AtomicUsize::new(0),
        });
        watch(Arc::downgrade(&s), at);
        s
    }

    pub(crate) fn call(&self, method: &str, args: &Value) -> Option<ApiResult> {
        if !METHODS.iter().any(|(name, _)| *name == method) {
            return None;
        }
        let lang = self.ctx.lang();
        let Ok(bus) = system() else {
            return Some(match method {
                "battery.charging.get" => Ok(no_charging()),
                "battery.history" => {
                    history_args(args, lang).map(|(kind, timespan, _)| no_history(&kind, timespan))
                }
                _ => Err(ApiError::tr(
                    "unsupported",
                    lang,
                    "battery.unavailable",
                    NO_PARAMS,
                )),
            });
        };
        let up = Upower {
            bus: &bus,
            name: UPOWER,
        };
        Some(match method {
            "battery.charging.get" => Ok(charging(&up)),
            "battery.charging.set" => {
                let result = set_charging(&up, args, lang, self.ctx.dry_run);
                if let Ok(state) = &result
                    && !self.ctx.dry_run
                {
                    self.ctx.emit("battery.changed", state.clone());
                }
                result
            }
            _ => history(&up, args, lang),
        })
    }

    pub(crate) fn shutdown(&self) {
        self.stopped.store(true, Ordering::Relaxed);
        if let Some(context) = self
            .context
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            context.wakeup();
        }
    }
}

impl Drop for Battery {
    /// The watcher sleeps until something happens: wake it so it ends with the service.
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// How long a burst of UPower's signals settles before battery.changed.
const BURST: Duration = Duration::from_millis(200);

/// Where the watcher listens: the bus, the signals' sender and the name to read the state from (UPower's own name on
/// the system bus; tests: a private bus).
struct Watched {
    bus: gio::DBusConnection,
    sender: String,
    name: String,
}

/// UPower's PropertiesChanged on its own GLib context: a change of a charge threshold (here, in GNOME Settings or by
/// UPower itself) emits battery.changed with the new state, once per burst (200 ms after its last signal: every
/// signal of the burst restarts a one-shot timer). Between signals the thread sleeps in its context: no periodic tick
/// wakes it, only a signal, the burst's timer or `shutdown`.
fn watch(weak: Weak<Battery>, at: impl FnOnce() -> Option<Watched> + Send + 'static) {
    std::thread::spawn(move || {
        let context = glib::MainContext::new();
        let _ = context.with_thread_default(|| {
            let Some(battery) = weak.upgrade() else {
                return;
            };
            *battery.context.lock().unwrap_or_else(|e| e.into_inner()) = Some(context.clone());
            drop(battery);
            let Some(Watched { bus, sender, name }) = at() else {
                return;
            };
            let timer: Arc<Mutex<Option<glib::Source>>> = Arc::new(Mutex::new(None));
            let (t, w, b, c) = (timer.clone(), weak.clone(), bus.clone(), context.clone());
            let sub = bus.signal_subscribe(
                Some(&sender),
                Some("org.freedesktop.DBus.Properties"),
                Some("PropertiesChanged"),
                None,
                Some(DEVICE),
                gio::DBusSignalFlags::NONE,
                move |_, _, _, _, _, params| {
                    if !threshold_changed(params) {
                        return;
                    }
                    let mut pending = t.lock().unwrap_or_else(|e| e.into_inner());
                    if let Some(old) = pending.take() {
                        old.destroy();
                    }
                    let (t2, w2, b2, n2) = (t.clone(), w.clone(), b.clone(), name.clone());
                    let source =
                        glib::timeout_source_new(BURST, None, glib::Priority::DEFAULT, move || {
                            t2.lock().unwrap_or_else(|e| e.into_inner()).take();
                            if let Some(s) = w2.upgrade()
                                && !s.stopped.load(Ordering::Relaxed)
                            {
                                let state = charging(&Upower {
                                    bus: &b2,
                                    name: &n2,
                                });
                                s.ctx.emit("battery.changed", state);
                            }
                            glib::ControlFlow::Break
                        });
                    source.attach(Some(&c));
                    *pending = Some(source);
                },
            );
            while let Some(s) = weak.upgrade() {
                if s.stopped.load(Ordering::Relaxed) {
                    break;
                }
                #[cfg(test)]
                s.wakeups.fetch_add(1, Ordering::Relaxed);
                drop(s);
                context.iteration(true);
            }
            if let Some(source) = timer.lock().unwrap_or_else(|e| e.into_inner()).take() {
                source.destroy();
            }
            bus.signal_unsubscribe(sub);
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::storage::test_bus::{self, Object, Props, TestBus};

    const UPOWER_XML: &str = r#"<interface name="org.freedesktop.UPower">
<method name="EnumerateDevices"><arg name="devices" type="ao" direction="out"/></method>
<property name="OnBattery" type="b" access="read"/></interface>"#;
    // UPower 1.91's Device: the properties battery.rs reads, EnableChargeThreshold and GetHistory
    const DEVICE_XML: &str = r#"<interface name="org.freedesktop.UPower.Device">
<method name="EnableChargeThreshold"><arg name="charge_threshold" type="b" direction="in"/></method>
<method name="GetHistory"><arg name="type" type="s" direction="in"/><arg name="timespan" type="u" direction="in"/>
<arg name="resolution" type="u" direction="in"/><arg name="data" type="a(udu)" direction="out"/></method>
<property name="Type" type="u" access="read"/><property name="PowerSupply" type="b" access="read"/>
<property name="HasHistory" type="b" access="read"/><property name="Percentage" type="d" access="read"/>
<property name="ChargeThresholdSupported" type="b" access="read"/>
<property name="ChargeThresholdEnabled" type="b" access="read"/>
<property name="ChargeStartThreshold" type="u" access="read"/>
<property name="ChargeEndThreshold" type="u" access="read"/></interface>"#;
    const BAT: &str = "/org/freedesktop/UPower/devices/battery_BAT0";
    const MOUSE: &str = "/org/freedesktop/UPower/devices/mouse_hidpp_battery_0";
    const AC: &str = "/org/freedesktop/UPower/devices/line_power_AC";

    fn device(props: &Props, path: &str, kind: u32, supply: bool, supported: bool) {
        props.set(path, "Type", kind.to_variant());
        props.set(path, "PowerSupply", supply.to_variant());
        props.set(path, "HasHistory", (kind == 2).to_variant());
        props.set(path, "Percentage", 80.0f64.to_variant());
        props.set(path, "ChargeThresholdSupported", supported.to_variant());
        props.set(path, "ChargeThresholdEnabled", false.to_variant());
        props.set(path, "ChargeStartThreshold", 75u32.to_variant());
        props.set(path, "ChargeEndThreshold", 80u32.to_variant());
    }

    /// A laptop battery (`supported`: with a charge limit), a mouse battery and the mains adapter.
    fn upower(supported: bool) -> Option<TestBus> {
        let props = Props::default();
        device(&props, BAT, 2, true, supported);
        device(&props, MOUSE, 2, false, true);
        device(&props, AC, 1, true, false);
        let objects = vec![
            Object {
                path: UPOWER_PATH.into(),
                xml: UPOWER_XML.into(),
            },
            Object {
                path: BAT.into(),
                xml: DEVICE_XML.into(),
            },
            Object {
                path: MOUSE.into(),
                xml: DEVICE_XML.into(),
            },
            Object {
                path: AC.into(),
                xml: DEVICE_XML.into(),
            },
        ];
        TestBus::start(
            objects,
            props,
            Arc::new(|props, path, method, params| match method {
                "EnumerateDevices" => {
                    let paths: Vec<glib::variant::ObjectPath> = [AC, BAT, MOUSE]
                        .iter()
                        .map(|p| crate::services::system::common::object_path(p).unwrap())
                        .collect();
                    Ok(Some((paths,).to_variant()))
                }
                "EnableChargeThreshold" => {
                    let on = params.child_value(0).get::<bool>().unwrap_or(false);
                    props.set(path, "ChargeThresholdEnabled", on.to_variant());
                    Ok(None)
                }
                "GetHistory" => {
                    // UPower answers newest first; one empty sample and a repeated one
                    let data: Vec<(u32, f64, u32)> = vec![
                        (1_700_003_600, 64.0, 2),
                        (1_700_003_600, 64.0, 2),
                        (1_700_000_000, 80.04, 1),
                        (0, 0.0, 0),
                    ];
                    Ok(Some((data,).to_variant()))
                }
                _ => Err((
                    "org.freedesktop.DBus.Error.UnknownMethod".into(),
                    method.into(),
                )),
            }),
        )
    }

    fn skip() {
        eprintln!("skipped: no private session bus (run through tests/rust-sandbox/run-tests.sh)");
    }

    #[test]
    fn charging_reads_the_batteries_that_power_the_computer() {
        let Some(fake) = upower(true) else {
            return skip();
        };
        let bus = test_bus::client();
        let up = Upower {
            bus: &bus,
            name: &fake.name,
        };
        let state = charging(&up);
        assert_eq!(state["available"], true);
        assert_eq!(state["supported"], true);
        assert_eq!(state["enabled"], false);
        assert_eq!(
            (state["start"].as_u64(), state["end"].as_u64()),
            (Some(75), Some(80))
        );
        assert_eq!(
            state["devices"],
            json!([BAT]),
            "not the mouse (no power supply) nor the adapter"
        );
        let none = upower(false).unwrap();
        let up = Upower {
            bus: &bus,
            name: &none.name,
        };
        let state = charging(&up);
        assert_eq!(
            (state["available"].clone(), state["supported"].clone()),
            (json!(true), json!(false))
        );
        let e = set_charging(&up, &json!({"enabled": true}), Lang::En, false).unwrap_err();
        assert_eq!(e.key.as_deref(), Some("battery.noChargeLimit"));
        assert_eq!(e.message, "This battery doesn’t support a charge limit.");
        assert!(none.calls_of("EnableChargeThreshold").is_empty());
    }

    #[test]
    fn set_charging_asks_upower_with_interactive_authorization_and_a_dry_run_sends_nothing() {
        let Some(fake) = upower(true) else {
            return skip();
        };
        let bus = test_bus::client();
        let up = Upower {
            bus: &bus,
            name: &fake.name,
        };
        let dry = set_charging(&up, &json!({"enabled": true}), Lang::Nl, true).unwrap();
        assert_eq!(
            (dry["dryRun"].clone(), dry["enabled"].clone()),
            (json!(true), json!(false))
        );
        assert!(
            fake.calls_of("EnableChargeThreshold").is_empty(),
            "a dry run sends nothing"
        );
        let state = set_charging(&up, &json!({"enabled": true}), Lang::Nl, false).unwrap();
        assert_eq!(
            state["enabled"], true,
            "the state as UPower reports it afterwards"
        );
        let calls = fake.calls_of("EnableChargeThreshold");
        assert_eq!(calls.len(), 1, "only the battery that supports it");
        assert_eq!(
            (calls[0].path.as_str(), calls[0].args.clone()),
            (BAT, json!([true]))
        );
        assert!(calls[0].interactive, "polkit may ask (call_interactive)");
        set_charging(&up, &json!({"enabled": false}), Lang::Nl, false).unwrap();
        assert_eq!(
            fake.calls_of("EnableChargeThreshold")[1].args,
            json!([false])
        );
        for bad in [json!({}), json!({"enabled": "yes"}), json!({"enabled": 1})] {
            assert_eq!(
                set_charging(&up, &bad, Lang::Nl, false).unwrap_err().code,
                "bad_args",
                "{bad}"
            );
        }
        assert_eq!(fake.calls_of("EnableChargeThreshold").len(), 2);
    }

    #[test]
    fn history_is_upowers_oldest_first_and_checks_its_arguments() {
        let Some(fake) = upower(true) else {
            return skip();
        };
        let bus = test_bus::client();
        let up = Upower {
            bus: &bus,
            name: &fake.name,
        };
        let h = history(
            &up,
            &json!({"type": "charge", "timespan": 604800}),
            Lang::Nl,
        )
        .unwrap();
        assert_eq!(h["available"], true);
        assert_eq!(h["device"], BAT);
        assert_eq!(
            h["points"],
            json!([[1_700_000_000u32, 80.0, 1], [1_700_003_600u32, 64.0, 2]])
        );
        let call = &fake.calls_of("GetHistory")[0];
        assert_eq!(
            (call.path.as_str(), call.args.clone()),
            (BAT, json!(["charge", 604800, 200]))
        );
        assert!(!call.interactive);
        assert_eq!(
            history_args(&json!({}), Lang::Nl).unwrap(),
            ("charge".into(), 86_400, 200)
        );
        for bad in [
            json!({"type": "energy"}),
            json!({"timespan": 5}),
            json!({"timespan": -1}),
            json!({"resolution": 1}),
        ] {
            assert_eq!(
                history_args(&bad, Lang::Nl).unwrap_err().code,
                "bad_args",
                "{bad}"
            );
        }
    }

    #[test]
    fn a_threshold_change_is_told_apart_from_other_properties() {
        let changed = |keys: &[&str], invalidated: Vec<&str>| {
            let dict: std::collections::HashMap<String, glib::Variant> = keys
                .iter()
                .map(|k| ((*k).to_owned(), true.to_variant()))
                .collect();
            (DEVICE, dict, invalidated).to_variant()
        };
        assert!(threshold_changed(&changed(
            &["ChargeThresholdEnabled"],
            vec![]
        )));
        assert!(threshold_changed(&changed(&[], vec!["ChargeEndThreshold"])));
        assert!(!threshold_changed(&changed(
            &["Percentage"],
            vec!["TimeToEmpty"]
        )));
    }

    /// A service context in a temporary directory, with the battery.changed events it emits.
    fn context(root: &std::path::Path) -> (Arc<ServiceContext>, Arc<Mutex<Vec<Value>>>) {
        let config = root.join("config");
        std::fs::create_dir_all(&config).unwrap();
        std::fs::write(config.join("settings.json"), r#"{"language":"en"}"#).unwrap();
        let events = crate::events::Events::new(&root.join("events")).unwrap();
        let ctx = ServiceContext::new(
            events,
            config,
            root.join("cache"),
            root.to_path_buf(),
            "settings".into(),
            false,
        )
        .unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s2 = seen.clone();
        ctx.events.on(
            "battery.changed",
            Arc::new(move |v: &Value| s2.lock().unwrap().push(v.clone())),
        );
        (ctx, seen)
    }

    fn eventually(what: impl Fn() -> bool) -> bool {
        let until = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < until {
            if what() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        what()
    }

    #[test]
    fn the_watcher_sleeps_between_signals_and_tells_one_burst_once() {
        let Some(fake) = upower(true) else {
            return skip();
        };
        let address = crate::services::display::fake::private_bus_address().unwrap();
        // UPower's signals come from a connection of their own, as from upowerd
        let upowerd = gio::DBusConnection::for_address_sync(
            &address,
            gio::DBusConnectionFlags::AUTHENTICATION_CLIENT
                | gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION,
            None,
            gio::Cancellable::NONE,
        )
        .unwrap();
        let root = std::env::temp_dir().join(format!("win11-battery-{}", crate::events::nonce()));
        let (ctx, seen) = context(&root);
        let (sender, name) = (
            upowerd.unique_name().unwrap().to_string(),
            fake.name.clone(),
        );
        let battery = Battery::watching(ctx.clone(), move || {
            Some(Watched {
                bus: test_bus::client(),
                sender,
                name,
            })
        });
        assert!(
            eventually(|| battery.wakeups.load(Ordering::Relaxed) >= 1),
            "the watcher runs"
        );
        std::thread::sleep(Duration::from_millis(300));
        // review D-3: no 200 ms tick; nothing happens, nothing wakes the thread
        let idle = battery.wakeups.load(Ordering::Relaxed);
        std::thread::sleep(Duration::from_millis(1200));
        assert!(
            battery.wakeups.load(Ordering::Relaxed) - idle <= 1,
            "the watcher woke {} times in 1.2 s without a signal",
            battery.wakeups.load(Ordering::Relaxed) - idle
        );
        let emit = |keys: &[&str]| {
            let dict: std::collections::HashMap<String, glib::Variant> = keys
                .iter()
                .map(|k| ((*k).to_owned(), true.to_variant()))
                .collect();
            upowerd
                .emit_signal(
                    None,
                    BAT,
                    "org.freedesktop.DBus.Properties",
                    "PropertiesChanged",
                    Some(&(DEVICE, dict, Vec::<String>::new()).to_variant()),
                )
                .unwrap();
            upowerd.flush_sync(gio::Cancellable::NONE).unwrap();
        };
        // a burst: three threshold changes 30 ms apart are one battery.changed
        for _ in 0..3 {
            emit(&["ChargeThresholdEnabled"]);
            std::thread::sleep(Duration::from_millis(30));
        }
        assert!(
            eventually(|| seen.lock().unwrap().len() == 1),
            "the burst is told"
        );
        std::thread::sleep(Duration::from_millis(600));
        assert_eq!(seen.lock().unwrap().len(), 1, "once");
        assert_eq!(
            seen.lock().unwrap()[0]["supported"],
            true,
            "the state as UPower has it"
        );
        // a change of anything else is not told
        emit(&["Percentage"]);
        std::thread::sleep(Duration::from_millis(600));
        assert_eq!(seen.lock().unwrap().len(), 1);
        emit(&["ChargeEndThreshold"]);
        assert!(
            eventually(|| seen.lock().unwrap().len() == 2),
            "the next burst"
        );
        drop(battery);
        ctx.events.shutdown();
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn without_upower_nothing_is_offered() {
        let Some(bus) = test_bus::client_or_none() else {
            return skip();
        };
        // a unique name nobody owns: EnumerateDevices fails
        let up = Upower {
            bus: &bus,
            name: ":1.999999",
        };
        assert_eq!(charging(&up)["available"], false);
        assert_eq!(
            history(&up, &json!({}), Lang::Nl).unwrap()["available"],
            false
        );
        assert_eq!(
            set_charging(&up, &json!({"enabled": true}), Lang::Nl, false)
                .unwrap_err()
                .key
                .as_deref(),
            Some("battery.unavailable")
        );
    }
}
